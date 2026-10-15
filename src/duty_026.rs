// SPDX-License-Identifier: GPL-3.0-or-later
//! The six pre-split Duty models use the native 69-bone rig, rotated 180°
//! around Y. Equipment is additive; native 0.26 item/shader schemas are retained.
use crate::manifest::{BuiltFile, TextureCompression};
use crate::weapon::{
    add_localization_strings, parse_config, prepare_empty_output, record_file,
    write_generated_config,
};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const BASE: &str = "models/character/human/base/base_character_boot.skinned_model";
const NS: &str = "ported/duty_026";
const DICT: &str = "gameplay/db_static_dictionaries";
const MATERIAL: &str =
    "material_instances/characters/human/renesanse_tank/renesanse_tank_torso.material";

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}
fn string(s: impl Into<String>) -> V {
    V::String(s.into())
}
fn table(fields: Vec<NamedValue>) -> V {
    V::Named(fields)
}
fn raw(out: &Path, path: &str, bytes: &[u8], files: &mut Vec<BuiltFile>) -> Result<()> {
    let dest = crate::archive::safe_join(out, path)?;
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(dest, bytes)?;
    files.push(record_file(path, "converted:Duty-026", bytes));
    Ok(())
}
fn config(out: &Path, path: &str, root: V, files: &mut Vec<BuiltFile>) -> Result<()> {
    write_generated_config(
        out,
        path,
        &BinaryConfig { root },
        "generated:Duty-026",
        files,
    )
}
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated u32")?.try_into()?,
    ))
}
fn f32_at(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}

/// Match the 0.26 compressor's header, including its colour-space byte in
/// dwPitchOrLinearSize and retaining BOTH PITCH and LINEARSIZE flags.
/// Omitting these produced corrupt colours in the native lobby renderer even
/// though ordinary DDS readers displayed the images correctly.
pub(crate) fn texture_dds(bytes: &[u8], name: &str, srgb: bool) -> Result<Vec<u8>> {
    let mut out = crate::soc::source::canonicalize_dds(
        bytes,
        crate::soc::source::parse_dds(bytes, name)?,
        srgb,
    )?;
    let flags = u32_at(&out, 8)? | 8;
    let size = u32::try_from(out.len() - 128)?;
    out[8..12].copy_from_slice(&flags.to_le_bytes());
    out[20..24].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

/// The supplied editor configs store name before data; database configs store
/// data before name. Only visited nodes are swapped; string/float payloads stay put.
fn editor_config(input: &[u8]) -> Result<BinaryConfig> {
    fn visit(b: &mut [u8], at: usize, seen: &mut BTreeSet<usize>) -> Result<()> {
        ensure!(seen.insert(at), "cyclic editor config");
        let n = b.get(at..at + 24).context("truncated editor config node")?;
        let name = u64::from_le_bytes(n[..8].try_into()?);
        let data = u64::from_le_bytes(n[8..16].try_into()?);
        let kind = u16::from_le_bytes(n[20..22].try_into()?);
        let count = u16::from_le_bytes(n[22..24].try_into()?);
        b[at..at + 8].copy_from_slice(&data.to_le_bytes());
        b[at + 8..at + 16].copy_from_slice(&name.to_le_bytes());
        if kind == 3 || kind == 4 {
            for i in 0..usize::from(count) {
                visit(
                    b,
                    usize::try_from(data)?
                        .checked_add(i * 24)
                        .context("node overflow")?,
                    seen,
                )?;
            }
        }
        Ok(())
    }
    let mut b = input.to_vec();
    visit(&mut b, 0, &mut BTreeSet::new())?;
    BinaryConfig::parse(&b)
}

fn bind_frames(b: &[u8]) -> Result<Vec<(String, [f32; 9])>> {
    let count = u16::from_le_bytes(b.get(..2).context("missing bone count")?.try_into()?);
    let mut at = 2;
    let mut result = vec![];
    for _ in 0..count {
        let len = b
            .get(at..)
            .context("missing bone")?
            .iter()
            .position(|&v| v == 0)
            .context("missing bone terminator")?;
        let name = std::str::from_utf8(&b[at..at + len])?.to_owned();
        at += len + 1;
        let mut frame = [0.; 9];
        for (i, v) in frame.iter_mut().enumerate() {
            *v = f32_at(b, at + i * 4)?;
            ensure!(v.is_finite(), "nonfinite bone");
        }
        at += 36;
        result.push((name, frame));
    }
    ensure!(at == b.len(), "trailing bind data");
    Ok(result)
}
fn basis(f: &[f32; 9]) -> [[f32; 3]; 3] {
    let (xs, xc) = f[3].sin_cos();
    let (ys, yc) = f[4].sin_cos();
    let (zs, zc) = f[5].sin_cos();
    let mut m = [
        [yc * zc, -yc * zs, ys],
        [ys * xs * zc + xc * zs, -xs * ys * zs + xc * zc, -xs * yc],
        [-xc * ys * zc + xs * zs, xc * ys * zs + xs * zc, xc * yc],
    ];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] *= f[6 + r];
        }
    }
    m
}
fn check_bind(source: &[u8], target: &[u8]) -> Result<usize> {
    let s = bind_frames(source)?;
    let t = bind_frames(target)?;
    ensure!(
        s.len() == 69 && s.len() == t.len(),
        "expected the native 69-bone rig"
    );
    for ((sn, sf), (tn, tf)) in s.iter().zip(&t) {
        ensure!(sn == tn, "bone order differs: {sn}/{tn}");
        let sm = basis(sf);
        let tm = basis(tf);
        for c in 0..3 {
            let sign = if c == 1 { 1. } else { -1. };
            ensure!(
                (sf[c] * sign - tf[c]).abs() < 0.0001,
                "bone translation differs: {sn}"
            );
            for r in 0..3 {
                ensure!(
                    (sm[r][c] * sign - tm[r][c]).abs() < 0.0001,
                    "bone rotation differs: {sn}"
                );
            }
        }
    }
    Ok(s.len())
}

/// Preserve all four influences, UVs and index ordering. A Y half-turn has
/// positive determinant, so winding and tangent handedness do not change.
fn rotate_mesh(input: &[u8], bones: usize) -> Result<(Vec<u8>, [f32; 3], [f32; 3], usize, usize)> {
    ensure!(u32_at(input, 0)? == 3, "missing vertex chunk");
    let n = u32_at(input, 8)? as usize;
    let size = u32_at(input, 4)? as usize;
    ensure!(
        n > 0 && size == 4 + n * 48,
        "expected 48-byte four-weight vertices"
    );
    let index_at = 8 + size;
    ensure!(u32_at(input, index_at)? == 4, "missing index chunk");
    let ni = u32_at(input, index_at + 8)? as usize;
    ensure!(
        ni > 0
            && ni % 3 == 0
            && u32_at(input, index_at + 4)? as usize == 4 + ni * 2
            && input.len() == index_at + 12 + ni * 2,
        "invalid index chunk"
    );
    for v in input[index_at + 12..].chunks_exact(2) {
        ensure!(
            (u16::from_le_bytes(v.try_into()?) as usize) < n,
            "index outside vertex array"
        );
    }
    let mut b = input.to_vec();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for i in 0..n {
        let at = 12 + i * 48;
        let mut sum = 0.;
        for k in 0..3 {
            let w = f32_at(input, at + 16 + k * 4)?;
            ensure!(
                w.is_finite() && (-0.0001..=1.0001).contains(&w),
                "invalid weight"
            );
            sum += w;
        }
        ensure!(sum <= 1.0001, "weights exceed one");
        for k in 0..4 {
            ensure!((input[at + 12 + k] as usize) < bones, "invalid bone index");
        }
        for k in 0..3 {
            let p = f32_at(input, at + k * 4)? * if k == 1 { 1. } else { -1. };
            ensure!(p.is_finite(), "nonfinite position");
            min[k] = min[k].min(p);
            max[k] = max[k].max(p);
            b[at + k * 4..at + k * 4 + 4].copy_from_slice(&p.to_le_bytes());
        }
        // Packed basis bytes are B, G, R, A (Z, Y, X, auxiliary).
        for offset in [28, 32, 36] {
            for axis in [0, 2] {
                b[at + offset + axis] = 255 - b[at + offset + axis];
            }
        }
        for offset in [40, 44] {
            ensure!(f32_at(input, at + offset)?.is_finite(), "nonfinite UV");
        }
    }
    Ok((b, min, max, n, ni))
}

/// Separate whole welded components of the backpack from the limb frame.
/// Bounds are in the supplied model's authoring coordinates. No triangle is cut.
fn split_back(input: &[u8], back: bool, wire_anchors: &[usize]) -> Result<Vec<u8>> {
    split_components(input, back, wire_anchors, true)
}

/// Partition whole welded components, preserving all original vertex bytes.
fn split_components(
    input: &[u8],
    selected_half: bool,
    anchors: &[usize],
    back_bounds: bool,
) -> Result<Vec<u8>> {
    rotate_mesh(input, 69)?; // validate layout before indexing
    let n = u32_at(input, 8)? as usize;
    let index_at = 12 + n * 48;
    let indices: Vec<usize> = input[index_at + 12..]
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()) as usize)
        .collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut positions = BTreeMap::new();
    for i in 0..n {
        let p = [
            f32_at(input, 12 + i * 48)?,
            f32_at(input, 16 + i * 48)?,
            f32_at(input, 20 + i * 48)?,
        ];
        let key = p.map(|v| (v * 100_000.).round() as i32);
        if let Some(j) = positions.insert(key, i) {
            let a = root(&mut parent, i);
            let b = root(&mut parent, j);
            parent[a] = b;
        }
    }
    for tri in indices.chunks_exact(3) {
        for &v in &tri[1..] {
            let a = root(&mut parent, tri[0]);
            let b = root(&mut parent, v);
            parent[a] = b;
        }
    }
    let mut selected = BTreeMap::new();
    for i in 0..n {
        let p = [
            f32_at(input, 12 + i * 48)?,
            f32_at(input, 16 + i * 48)?,
            f32_at(input, 20 + i * 48)?,
        ];
        let inside = back_bounds && p[0].abs() < 0.20 && p[1] > 1.03 && p[2] > 0.11;
        *selected.entry(root(&mut parent, i)).or_insert(true) &= inside;
    }
    for &vertex in anchors {
        ensure!(vertex < n, "component anchor outside mesh");
        selected.insert(root(&mut parent, vertex), true);
    }
    let mut vertices = vec![];
    let mut remap = BTreeMap::new();
    let mut out_indices = vec![];
    for tri in indices.chunks_exact(3) {
        if selected[&root(&mut parent, tri[0])] != selected_half {
            continue;
        }
        for &v in tri {
            let next = u16::try_from(remap.len())?;
            let idx = *remap.entry(v).or_insert_with(|| {
                vertices.extend_from_slice(&input[12 + v * 48..12 + (v + 1) * 48]);
                next
            });
            out_indices.extend_from_slice(&idx.to_le_bytes());
        }
    }
    ensure!(!vertices.is_empty(), "empty component split");
    let mut out = vec![];
    for v in [3u32, 4 + vertices.len() as u32, remap.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    out.extend(vertices);
    for v in [
        4u32,
        4 + out_indices.len() as u32,
        (out_indices.len() / 2) as u32,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(out_indices);
    Ok(out)
}

pub fn build(
    source: &Path,
    textures: &Path,
    db: &Path,
    icons_swf: Option<&Path>,
    output: &Path,
) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open(db)?;
    let bind = target.read(&format!("{BASE}/render/bind_pose"))?;
    let mut files = vec![];
    let mut provenance = BTreeMap::new();
    let parts = [
        (
            "torso_s1",
            vec![
                ("torso_s", "torso"),
                ("exoskeleton_s", "attach"),
                ("back_s", "attach"),
                ("sign_s", "sign"),
            ],
        ),
        ("head", vec![("helmet_s", "helmet")]),
        ("legs_s1", vec![("legs_s", "legs")]),
        ("boot", vec![("legs_s", "legs")]),
        ("hand", vec![("torso_s", "torso")]),
        ("hand_hud", vec![("torso_s", "torso")]),
    ];
    for (part, surfaces) in &parts {
        let name = format!("duty_03_{part}.skinned_model");
        let exported = source
            .join("resources/exported/models/character/human/duty")
            .join(&name);
        let mut donor = ResourceDb::open(&exported)?;
        let bones = check_bind(&donor.read("render/bind_pose")?, &bind)?;
        let platform = source
            .join("resources/platforms/pc_dx11/models/character/human/duty")
            .join(&name);
        let root = format!("models/{NS}/{name}");
        let mut settings =
            parse_config(&mut target, &format!("{BASE}/settings"), "model template")?;
        settings.root.set_path(
            "lod_hierrarchy",
            table(vec![field(
                "LOD0",
                table(vec![field(
                    "surfaces",
                    V::Indexed(surfaces.iter().map(|(s, _)| string(*s)).collect()),
                )]),
            )]),
        )?;
        settings.root.set_path(
            "material_settings",
            table(
                surfaces
                    .iter()
                    .map(|(s, m)| {
                        field(
                            s,
                            table(vec![
                                field("material_name", string(format!("{NS}/{m}"))),
                                field("used_in_portals", V::Boolean(false)),
                            ]),
                        )
                    })
                    .collect(),
            ),
        )?;
        config(
            &overlay,
            &format!("{root}/settings"),
            settings.root,
            &mut files,
        )?;
        raw(
            &overlay,
            &format!("{root}/render/bind_pose"),
            &bind,
            &mut files,
        )?;
        raw(
            &overlay,
            &format!("{root}/skeleton"),
            &target.read(&format!("{BASE}/skeleton"))?,
            &mut files,
        )?;
        let mut all_min = [f32::INFINITY; 3];
        let mut all_max = [f32::NEG_INFINITY; 3];
        for (surface, _) in surfaces {
            let source_surface = if *surface == "back_s" {
                "exoskeleton_s"
            } else {
                surface
            };
            let input = platform.join(format!("render/{source_surface}/converted_model"));
            let source_bytes = fs::read(&input)?;
            provenance.insert(
                input.display().to_string(),
                record_file("", "", &source_bytes).sha256,
            );
            let split;
            let bytes = if *surface == "back_s" || *surface == "exoskeleton_s" {
                // Reviewed flexible hoses/cables, including their end cuffs.
                // Anchor vertices select entire welded components; the hash
                // prevents applying those indices to a different source mesh.
                ensure!(
                    record_file("", "", &source_bytes).sha256
                        == "0c52447877ee36c4d928d2922fc359f60cecbf00913cb082bef0fc17459b8ffe",
                    "Duty attachment mesh changed; review wire anchors"
                );
                split = split_back(
                    &source_bytes,
                    *surface == "back_s",
                    &[
                        7366, 7461, 5388, 5234, 5292, 5450, 212, 35, 67, 3800, 4840, 247, 167,
                        3238, 4278, 854, 2288, 0,
                    ],
                )?;
                &split
            } else {
                &source_bytes
            };
            let (mesh, min, max, n, ni) = rotate_mesh(bytes, bones)?;
            for k in 0..3 {
                all_min[k] = all_min[k].min(min[k]);
                all_max[k] = all_max[k].max(max[k]);
            }
            let mut properties = editor_config(&fs::read(
                platform.join(format!("render/{source_surface}/export_properties")),
            )?)?;
            ensure!(
                properties.root.named("type") == Some(&V::Integer(47)),
                "source is not four-weight skinned"
            );
            properties
                .root
                .set_path("bounding_box.min", V::Float3(min))?;
            properties
                .root
                .set_path("bounding_box.max", V::Float3(max))?;
            // Replace authoring metadata with the actual emitted counts.
            properties
                .root
                .set_path("source.vertices", V::Integer(n.try_into()?))?;
            properties
                .root
                .set_path("source.indices", V::Integer(ni.try_into()?))?;
            config(
                &overlay,
                &format!("{root}/render/{surface}/export_properties"),
                properties.root,
                &mut files,
            )?;
            raw(
                &overlay,
                &format!("{root}/render/{surface}/converted_model"),
                &mesh,
                &mut files,
            )?;
        }
        let mut properties = parse_config(
            &mut target,
            &format!("{BASE}/render/export_properties"),
            "render template",
        )?;
        properties
            .root
            .set_path("bounding_box.min", V::Float3(all_min))?;
        properties
            .root
            .set_path("bounding_box.max", V::Float3(all_max))?;
        properties
            .root
            .set_path("src_filename", string(format!("___surva_char/{name}")))?;
        config(
            &overlay,
            &format!("{root}/render/export_properties"),
            properties.root,
            &mut files,
        )?;
    }
    let texture_sets = [
        ("torso", "dolg_exo_torso", "dolg_exo_torso_nmap"),
        ("legs", "dolg_exo_legs", "dolg_exo_legs_nmap"),
        ("helmet", "dolg_exo_helmet", "dolg_exo_helmet_nmap"),
        ("attach", "dold_exo_attach", "dolg_exo_attach_nmap"),
        ("sign", "dol_sign", "dol_sign _nmap"),
    ];
    for (part, diffuse, normal) in texture_sets {
        let mut material = parse_config(&mut target, MATERIAL, "native equipped armor material")?;
        for (role, source_name, compression, srgb) in [
            ("diffuse", diffuse, TextureCompression::Dxt1, true),
            ("normal", normal, TextureCompression::Dxt5Normal, false),
        ] {
            let input = textures.join(format!("{source_name}.tga"));
            provenance.insert(
                input.display().to_string(),
                record_file("", "", &fs::read(&input)?).sha256,
            );
            let path = format!("textures/{NS}/{part}_{role}");
            let dest = overlay.join(format!("{path}.dds"));
            fs::create_dir_all(dest.parent().unwrap())?;
            crate::armor::convert_texture(&input, &dest, compression)?;
            let dds = fs::read(&dest)?;
            let dds = texture_dds(&dds, &path, srgb)?;
            raw(&overlay, &format!("{path}.dds"), &dds, &mut files)?;
            let mut options = parse_config(
                &mut target,
                "textures/character/human/renesanse_tank/renesanse_tank_torso.options",
                "native texture options",
            )?;
            options
                .root
                .set_path("options.srgb", V::Integer(i32::from(srgb)))?;
            options.root.set_path(
                "options.format",
                string(if role == "normal" {
                    "NORMAL_MAP"
                } else {
                    "DXT1"
                }),
            )?;
            config(
                &overlay,
                &format!("{path}.options"),
                options.root,
                &mut files,
            )?;
            material.root.set_path(
                &format!("material.g_stage.effect.texture_{role}.value"),
                string(format!("{NS}/{part}_{role}")),
            )?;
        }
        // The donor's *_fmap is an old specular-power map, not the modern
        // Fresnel input. Do not interpret it as metallic reflectance. Keep the
        // native shader permutation but use an explicit matte approximation.
        for (name, value) in [
            ("constant_fresnel_min", 0.008),
            ("constant_fresnel_max", 0.008),
            ("constant_roughness_min", 0.98),
            ("constant_roughness_max", 0.98),
        ] {
            material.root.set_path(
                &format!("material.g_stage.effect.{name}.value"),
                V::Float(value),
            )?;
        }
        material.root.set_path(
            "material.g_stage.effect.constant_specular_color_multiplier.value",
            V::Float3([0.025; 3]),
        )?;
        let material_path = format!("material_instances/{NS}/{part}.material");
        // A revision can reuse an identical installed material. Do not mark it
        // as replaced: that would require auditing every unrelated stock model.
        if !target.is_file(&material_path) || target.read(&material_path)? != material.to_bytes()? {
            config(&overlay, &material_path, material.root, &mut files)?;
        }
    }
    // Native energy-cost variants skip -5%; add a default-only variant.
    let modification_path = "gameplay/items/modifications/modifications.options";
    let mut modification_config = parse_config(&mut target, modification_path, "native modifiers")?;
    let V::Indexed(mods) = modification_config
        .root
        .named_mut("modifications")
        .context("modifications missing")?
    else {
        bail!("expected indexed modifications")
    };
    let mut energy = mods
        .iter()
        .find(|m| m.named("id") == Some(&V::Integer(1162)))
        .context("energy modifier template missing")?
        .clone();
    energy.set_path("id", V::Integer(1165))?;
    energy.set_path("drop_weight", V::Integer(0))?;
    energy.set_path(
        "lobby_info.stamina_spending_speed_modifier",
        V::Float(-0.05),
    )?;
    if let Some(V::Indexed(values)) = energy.named_mut("modifiers") {
        ensure!(values.len() == 1, "unexpected energy modifier shape");
        values[0].set_path("value", V::Float(-0.05))?;
    } else {
        bail!("energy modifier values missing")
    }
    if let Some(V::Indexed(props)) = energy
        .named_mut("ui_desc")
        .and_then(|v| v.named_mut("props_list"))
    {
        ensure!(props.len() == 1, "unexpected energy tooltip shape");
        props[0].set_path("prop_value", string("-0.05"))?;
    } else {
        bail!("energy tooltip missing")
    }
    if let Some(existing) = mods
        .iter()
        .find(|m| m.named("id") == Some(&V::Integer(1165)))
    {
        ensure!(
            existing == &energy,
            "modifier 1165 is occupied by another definition"
        );
    } else {
        mods.push(energy);
    }
    config(
        &overlay,
        modification_path,
        modification_config.root,
        &mut files,
    )?;
    let oxygen_template = parse_config(
        &mut target,
        "gameplay/items/armour/back/scavenger_oxygen_1.options",
        "native oxygen behavior",
    )?;
    let mut dictionary = parse_config(&mut target, DICT, "0.26 dictionary")?;
    let V::Indexed(items) = dictionary
        .root
        .named_mut("items_dict")
        .context("items_dict missing")?
    else {
        bail!("expected the 0.26 indexed dictionary");
    };
    // Retire the split preview items, but never remove another mod's IDs.
    for (i, category) in ["helmet", "torso", "legs", "gloves", "boots", "back", "mask"]
        .iter()
        .enumerate()
    {
        let id = 1800 + i as i32;
        if let Some(existing) = items
            .iter()
            .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
        {
            ensure!(
                existing.named("cfg_name")
                    == Some(&string(format!(
                        "gameplay/items/armour/{category}/duty_exo_026.options"
                    )))
                    && existing.named("name") == Some(&string(format!("duty_exo_{category}"))),
                "item ID {id} belongs to other content"
            );
        }
    }
    items.retain(
        |v| !matches!(v.named("dict_id"), Some(V::Integer(id)) if (1800..=1806).contains(id)),
    );
    let id = 1805;
    let path = "gameplay/items/armour/back/duty_exo_026.options";
    let template = "gameplay/items/armour/back/ren_back_1.options";
    let mut item = items
        .iter()
        .find(|v| v.named("cfg_name") == Some(&string(template)))
        .context("native back item missing")?
        .clone();
    item.set_path("dict_id", V::Integer(id))?;
    item.set_path("cfg_name", string(path))?;
    item.set_path("name", string("duty_exo_back"))?;
    // Keep the aggregate purchase cost of the six original pieces.
    let mut cost = 0;
    for category in ["helmet", "torso", "legs", "gloves", "boots", "back"] {
        let cfg_path = if category == "back" {
            template.to_owned()
        } else {
            format!("gameplay/items/armour/{category}/renesanse_{category}_6.options")
        };
        let native = items
            .iter()
            .find(|v| v.named("cfg_name") == Some(&string(&cfg_path)))
            .context("native armor item missing")?;
        let Some(V::Integer(value)) = native.named("base_cost") else {
            bail!("native cost missing");
        };
        cost += value * 5 / 4;
    }
    item.set_path("base_cost", V::Integer(cost))?;
    items.push(item.clone());
    let mut cfg = parse_config(&mut target, template, "native back config")?;
    let modifications = [1031, 1050, 1170, 1040, 1114, 1165];
    for (key, value) in [("weight", 27.0), ("armor", 0.8), ("ano_armor", 0.7)] {
        cfg.root
            .set_path(&format!("parameters.{key}"), V::Float(value))?;
    }
    cfg.root
        .set_path("parameters.max_carried_weight_modifier", V::Integer(35))?;
    cfg.root
        .set_path("parameters.oxygen_amount", V::Integer(30))?;
    cfg.root
        .set_path("parameters.camo_available", V::Boolean(false))?;
    cfg.root
        .set_path("parameters.decal_available", V::Boolean(false))?;
    cfg.root.set_path(
        "default_modifications",
        V::Indexed(modifications.iter().map(|&m| V::Integer(m)).collect()),
    )?;
    // One item protects the union of the native body regions. Each region is
    // authored exactly once, avoiding duplicate armor/isolation application.
    let mut zones = BTreeMap::new();
    for category in ["helmet", "torso", "legs", "gloves", "boots"] {
        let native = parse_config(
            &mut target,
            &format!("gameplay/items/armour/{category}/renesanse_{category}_6.options"),
            "native protection",
        )?;
        let Some(V::Named(parts)) = native.root.named("hit_params") else {
            bail!("native hit parameters missing");
        };
        for part in parts {
            let mut value = part.value.clone();
            let Some(V::Named(types)) = value.named_mut("hit_types") else {
                bail!("native hit types missing");
            };
            for hit in types {
                let protection = if matches!(hit.name.as_str(), "slash" | "explosive" | "ranged") {
                    if matches!(part.name.as_str(), "brain" | "face") {
                        0.65
                    } else {
                        0.80
                    }
                } else {
                    0.70
                };
                hit.value.set_path("armor", V::Float(protection))?;
            }
            zones.insert(part.name.clone(), value);
        }
    }
    cfg.root.set_path(
        "hit_params",
        table(zones.into_iter().map(|(k, v)| field(&k, v)).collect()),
    )?;
    // Oxygen activation targets the real back slot. Other armor slots are
    // occupied through the same additional-model mechanism as Zubr's mask.
    for key in ["type", "amount_time_sec", "influences"] {
        let value = oxygen_template
            .root
            .named("data")
            .and_then(|v| v.named(key))
            .context("oxygen data missing")?
            .clone();
        cfg.root.set_path(&format!("data.{key}"), value)?;
    }
    cfg.root
        .set_path("data.base_model", string(format!("{NS}/duty_03_torso_s1")))?;
    cfg.root.set_path("data.part_name", string("torso_s"))?;
    cfg.root
        .set_path("data.remove_head_model", V::Boolean(true))?;
    cfg.root.set_path(
        "data.base_model_hud",
        string(format!("{NS}/duty_03_hand_hud")),
    )?;
    cfg.root.set_path("data.part_name_hud", string("torso_s"))?;
    let additional = [
        ("torso_s1", "exoskeleton_s"),
        ("torso_s1", "back_s"),
        ("torso_s1", "sign_s"),
        ("head", "helmet_s"),
        ("legs_s1", "legs_s"),
        ("hand", "torso_s"),
        ("boot", "legs_s"),
    ]
    .into_iter()
    .map(|(model, surface)| {
        table(vec![
            field("base_model", string(format!("{NS}/duty_03_{model}"))),
            field("part_name", string(surface)),
        ])
    })
    .collect();
    cfg.root
        .set_path("data.additional_models", V::Indexed(additional))?;
    cfg.root.set_path(
        "data.additional_model_slots",
        V::Indexed(
            ["helmet", "mask", "torso", "pants", "gloves", "boots"]
                .into_iter()
                .map(string)
                .collect(),
        ),
    )?;
    let props = [(6, "80"), (8, "70"), (15, "27.0"), (106, "35"), (11, "30")]
        .into_iter()
        .map(|(id, value)| {
            table(vec![
                field("prop_id", V::Integer(id)),
                field("prop_value", string(value)),
            ])
        })
        .collect();
    cfg.root.set_path("ui_desc.props_list", V::Indexed(props))?;
    cfg.root.set_path(
        "ui_desc.text_descriptions.name",
        string("st_duty_026_suit_name"),
    )?;
    cfg.root.set_path(
        "ui_desc.text_descriptions.description",
        string("st_duty_026_suit_desc"),
    )?;
    if icons_swf.is_some() {
        cfg.root.set_path(
            "ui_desc.icon",
            V::Indexed(vec![V::Integer(1), V::Integer(6)]),
        )?;
    }
    let strings: BTreeMap<String, String> = BTreeMap::from([
        ("st_duty_026_suit_name".into(), "Duty exoskeleton".into()),
        ("st_duty_026_suit_desc".into(), "An experimental sample of a military exoskeleton. Was never mass-produced due to extraordinary cost and some design flaws. Despite this, it is in demand due to its ability to take on the weight of all carried equipment, and therefore small batches are made in underground facilities outside Ukraine. Comes with a built-in artifact container.".into()),
    ]);
    config(&overlay, path, cfg.root, &mut files)?;
    let registration = vec![
        json!({"dict_id":id,"slot":3,"additional_slots":[0,1,2,4,5,6],"name":"Duty exoskeleton","template":template,"weight":27.0,"default_modifications":modifications,"dictionary_entry":format!("{item:?}"),"config":path}),
    ];
    if let Some(path) = icons_swf {
        let bytes = fs::read(path)?;
        provenance.insert(
            path.display().to_string(),
            record_file("", "", &bytes).sha256,
        );
        raw(&overlay, "flash_movies/lobby_menu.swf", &bytes, &mut files)?;
    }
    config(&overlay, DICT, dictionary.root, &mut files)?;
    for lang in [
        "english",
        "original",
        "russian",
        "polish",
        "german",
        "ukrainian",
    ] {
        let path = format!("localization/{lang}/localization");
        let mut loc = parse_config(&mut target, &path, "localization")?;
        let missing = strings
            .iter()
            .filter_map(|(key, text)| {
                (loc.root.named("strings").and_then(|v| v.named(key)) != Some(&string(text)))
                    .then(|| (key.clone(), text.clone()))
            })
            .collect();
        add_localization_strings(&mut loc.root, &missing)?;
        config(&overlay, &path, loc.root, &mut files)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"format_version":1,"asset_id":"duty-exoskeleton-026g","files":files,"inputs":provenance,"items":registration,"base_db":db,"limitations":["Custom experimental balance; native Renaissance tier requirements","Approximate matte material response; source diffuse and normal textures"]}),
        )?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(&registration)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!(
        "Built complete Duty exoskeleton item 1805: {} resources",
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backpack_split_keeps_whole_components_and_vertex_attributes() {
        // Two rear triangles, one joined across a UV seam to a frame triangle.
        let positions = [
            [0., 1.2, 0.2],
            [0.01, 1.2, 0.2],
            [0., 1.21, 0.2],
            [0.1, 1.4, 0.3],
            [0.11, 1.4, 0.3],
            [0.1, 1.41, 0.3],
            [0., 1.2, 0.2],
            [0.4, 1.2, 0.2],
            [0.4, 1.3, 0.2],
        ];
        let mut b = vec![];
        for v in [3u32, 4 + 9 * 48, 9] {
            b.extend(v.to_le_bytes());
        }
        for (i, p) in positions.iter().enumerate() {
            let mut v = [0u8; 48];
            for (axis, &x) in p.iter().enumerate() {
                v[axis * 4..axis * 4 + 4].copy_from_slice(&f32::to_le_bytes(x));
            }
            v[16..20].copy_from_slice(&1f32.to_le_bytes());
            v[40..44].copy_from_slice(&(i as f32 / 10.).to_le_bytes());
            b.extend(v);
        }
        for v in [4u32, 22, 9] {
            b.extend(v.to_le_bytes());
        }
        for v in 0..9u16 {
            b.extend(v.to_le_bytes());
        }
        let back = split_back(&b, true, &[]).unwrap();
        let frame = split_back(&b, false, &[]).unwrap();
        assert_eq!(u32_at(&back, 8).unwrap(), 3);
        assert_eq!(u32_at(&frame, 8).unwrap(), 6);
        assert_eq!(&back[12..156], &b[12 + 3 * 48..12 + 6 * 48]);
        assert_eq!(&frame[12..156], &b[12..156]);
        assert_eq!(&frame[156..300], &b[12 + 6 * 48..12 + 9 * 48]);
        // An anchor on the duplicate vertex also selects the triangle across
        // the UV seam. Future anchored partitions use this same contract.
        assert_eq!(split_components(&b, true, &[6], false).unwrap(), frame);
        assert_eq!(split_components(&b, false, &[6], false).unwrap(), back);
        assert!(split_components(&b, true, &[9], false).is_err());
        rotate_mesh(&back, 69).unwrap();
        rotate_mesh(&frame, 69).unwrap();
    }
    #[test]
    fn texture_header_matches_026_without_changing_compressed_pixels() {
        let mut source = vec![0u8; 128 + 40];
        source[..4].copy_from_slice(b"DDS ");
        for (at, value) in [
            (4, 124u32),
            (8, 0xa1007),
            (12, 8),
            (16, 8),
            (20, 32),
            (28, 2),
            (76, 32),
            (80, 4),
            (108, 0x401008),
        ] {
            source[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        source[84..88].copy_from_slice(b"DXT1");
        source[128..].fill(0x5a);
        let out = texture_dds(&source, "fixture", true).unwrap();
        assert_eq!(u32_at(&out, 8).unwrap(), 0xa100f);
        assert_eq!(u32_at(&out, 20).unwrap(), 41);
        assert_eq!(&out[128..168], &source[128..]);
        assert_eq!(out[168], 1);
    }
    #[test]
    fn editor_order_is_converted_without_swapping_payloads() {
        let cfg = BinaryConfig {
            root: table(vec![
                field("value", string("kept")),
                field("v", V::Float3([1., 2., 3.])),
            ]),
        };
        let mut b = cfg.to_bytes().unwrap();
        // Root and its two scalar children are the only nodes.
        for at in [0, 24, 48] {
            for i in 0..8 {
                b.swap(at + i, at + i + 8);
            }
        }
        let decoded = editor_config(&b).unwrap();
        assert_eq!(decoded.root.named("value"), cfg.root.named("value"));
        assert_eq!(decoded.root.named("v"), cfg.root.named("v"));
        assert!(editor_config(&b[..25]).is_err());
    }
    #[test]
    fn mesh_rotation_keeps_weights_uv_and_winding() {
        let mut b = vec![0u8; 12 + 3 * 48 + 12 + 6];
        for (at, v) in [(0, 3u32), (4, 148), (8, 3), (156, 4), (160, 10), (164, 3)] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        for i in 0..3 {
            let at = 12 + i * 48;
            for (o, v) in [(0, 1f32), (4, 2.), (8, 3.), (16, 1.), (40, 0.2), (44, 0.7)] {
                b[at + o..at + o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        for i in 0..3 {
            b[168 + i * 2..170 + i * 2].copy_from_slice(&(i as u16).to_le_bytes());
        }
        let (out, min, max, _, _) = rotate_mesh(&b, 69).unwrap();
        assert_eq!(min, [-1., 2., -3.]);
        assert_eq!(max, min);
        assert_eq!(&out[24..40], &b[24..40]);
        assert_eq!(&out[52..60], &b[52..60]);
        assert_eq!(&out[156..], &b[156..]);
        let (twice, _, _, _, _) = rotate_mesh(&out, 69).unwrap();
        assert_eq!(twice, b);
        b[24] = 69;
        assert!(rotate_mesh(&b, 69).is_err());
    }
}
