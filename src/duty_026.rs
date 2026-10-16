// SPDX-License-Identifier: GPL-3.0-or-later
//! The Duty exoskeleton set (body 1805, helmet 1800). The six pre-split Duty
//! models use the native 69-bone rig, rotated 180° around Y. Equipment is
//! additive; native 0.26 item/shader schemas are retained.
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

/// One item of the Duty set. Like the native level 15 exoskeleton sets
/// (Zubr UM-4, Pilgrim) the pieces are separate items of one faction and
/// level; 0.26g has no set id or set bonus. Each piece is worn in its
/// category's own slot and, as Zubr's helmet covers the mask, covers the
/// other slots its model replaces (`data.additional_model_slots`).
struct SetPiece {
    id: i32,
    /// The dictionary's internal name.
    name: &'static str,
    /// English name, for the registration.
    label: &'static str,
    path: &'static str,
    template: &'static str,
    /// Profile slot: helmet 0, mask 1, torso 2, back 3, legs 4, gloves 5,
    /// boots 6 (the exe's name table at 0xA79870; srp `model_slot`).
    slot: u8,
    covers: &'static [&'static str],
    weight: f32,
    cost: i32,
    icon: [i32; 2],
    modifications: &'static [i32],
}

/// The body suit: torso, frame, back unit, legs, hands and boots. Worn in
/// the torso slot (category 3), as the user expects of body armour. Its
/// oxygen still works there: 0.26g's quick-bar keys act on the n-th usable
/// item of quick slots 13..=18 and then armour slots 3, 0, 1, 2, 4, 5, 6
/// (0x2FE7F0, table 0x925FD8; called from 0x2D8D88), not on the back slot
/// alone as 0.100b's `player.cpp` did.
const BODY: SetPiece = SetPiece {
    id: 1805,
    name: "duty_exo_suit",
    label: "Duty exoskeleton",
    path: "gameplay/items/armour/torso/duty_exo_026.options",
    template: "gameplay/items/armour/torso/renesanse_torso_6.options",
    slot: 2,
    covers: &["back", "pants", "gloves", "boots"],
    weight: 30.0,
    // Owner's balance (2026-10-15): 300 000 for the set, between "Zubr
    // UM-4" (287 500 for five pieces) and "Pilgrim" (310 000 for six). The
    // helmet takes the average of Zubr's (77 500) and Pilgrim's (72 500).
    cost: 225_000,
    icon: [1, 6],
    // Zubr's ammo bag and pocket, Pilgrim's artefact slot, and no speed,
    // stamina or carry bonus, which no level 15 equipment has.
    modifications: &[1030, 1050, 1170, 1040],
};

/// The helmet with its mask: the original combined STK2 head, worn alone
/// over the hidden native head, covering the mask slot like Zubr UM-4's.
/// ID, path, name and icon of the v10 split helmet (1800); bundle v10 never
/// shipped to players.
const HELMET: SetPiece = SetPiece {
    id: 1800,
    name: "duty_exo_helmet",
    label: "Duty exoskeleton helmet",
    path: "gameplay/items/armour/helmet/duty_exo_026.options",
    template: "gameplay/items/armour/helmet/renesanse_helmet_6.options",
    slot: 0,
    covers: &["mask"],
    weight: 5.0,
    cost: 75_000,
    icon: [3, 6],
    modifications: &[],
};

static SET: [SetPiece; 2] = [BODY, HELMET];

/// The profile slot a `data.additional_model_slots` name stands for.
fn profile_slot(name: &str) -> Option<u8> {
    Some(match name {
        "helmet" => 0,
        "mask" => 1,
        "torso" => 2,
        "back" => 3,
        "pants" => 4,
        "gloves" => 5,
        "boots" => 6,
        _ => return None,
    })
}

impl SetPiece {
    fn covered_slots(&self) -> Result<Vec<u8>> {
        self.covers
            .iter()
            .map(|name| profile_slot(name).with_context(|| format!("no profile slot {name}")))
            .collect()
    }
}

/// The native configs the set is made of.
struct Natives {
    /// `scavenger_oxygen_1`.
    oxygen: V,
    /// Zubr UM-4's pieces by category: helmet, torso, legs, gloves, boots.
    protection: Vec<(String, V)>,
    /// Each [`SET`] piece's template: its dictionary row and config.
    templates: Vec<(V, V)>,
}

/// An earlier Duty row, which the builder may replace: a split preview
/// item, the one-item suit worn in the back slot, or a piece of this set.
fn check_reused_id(id: i32, row: &V) -> Result<()> {
    let is = |path: &str, name: &str| {
        row.named("cfg_name") == Some(&string(path)) && row.named("name") == Some(&string(name))
    };
    let split = ["helmet", "torso", "legs", "gloves", "boots", "back", "mask"]
        .get(usize::try_from(id - 1800)?)
        .is_some_and(|category| {
            is(
                &format!("gameplay/items/armour/{category}/duty_exo_026.options"),
                &format!("duty_exo_{category}"),
            )
        });
    let ours = SET
        .iter()
        .any(|piece| piece.id == id && is(piece.path, piece.name));
    ensure!(split || ours, "item ID {id} belongs to other content");
    Ok(())
}

fn remove_field(value: &mut V, key: &str) {
    if let V::Named(fields) = value {
        fields.retain(|field| field.name != key);
    }
}

/// Zubr UM-4's hit regions with the set's armour: the helmet's regions
/// (brain, face) for the helmet, every other region for the body, each
/// region once.
fn split_protection(natives: &[(String, V)]) -> Result<(V, V)> {
    let mut head = BTreeMap::new();
    let mut body = BTreeMap::new();
    for (category, native) in natives {
        let Some(V::Named(parts)) = native.named("hit_params") else {
            bail!("native hit parameters missing");
        };
        for part in parts {
            let mut value = part.value.clone();
            let Some(V::Named(types)) = value.named_mut("hit_types") else {
                bail!("native hit types missing");
            };
            for hit in types {
                let protection = if matches!(hit.name.as_str(), "slash" | "explosive" | "ranged") {
                    // Zubr 0.75/0.90 and Pilgrim 0.70/0.85 (head/body).
                    if matches!(part.name.as_str(), "brain" | "face") {
                        0.72
                    } else {
                        0.87
                    }
                } else {
                    // Zubr 0.5, Pilgrim 0.9.
                    0.70
                };
                hit.value.set_path("armor", V::Float(protection))?;
            }
            if category == "helmet" {
                &mut head
            } else {
                &mut body
            }
            .insert(part.name.clone(), value);
        }
    }
    body.retain(|name, _| !head.contains_key(name));
    ensure!(
        !head.is_empty() && !body.is_empty(),
        "native hit regions missing"
    );
    let to_table =
        |zones: BTreeMap<String, V>| table(zones.into_iter().map(|(k, v)| field(&k, v)).collect());
    Ok((to_table(head), to_table(body)))
}

fn props(values: &[(i32, String)]) -> V {
    V::Indexed(
        values
            .iter()
            .map(|(id, value)| {
                table(vec![
                    field("prop_id", V::Integer(*id)),
                    field("prop_value", string(value.as_str())),
                ])
            })
            .collect(),
    )
}

/// The dictionary row and config of each [`SET`] piece.
fn duty_set(natives: &Natives, icons: bool) -> Result<Vec<(&'static SetPiece, V, V)>> {
    ensure!(
        natives.templates.len() == SET.len(),
        "set templates missing"
    );
    let (head_zones, body_zones) = split_protection(&natives.protection)?;
    let mut out = vec![];
    for (piece, (row, template)) in SET.iter().zip(&natives.templates) {
        let body = piece.id == BODY.id;
        let mut row = row.clone();
        row.set_path("dict_id", V::Integer(piece.id))?;
        row.set_path("cfg_name", string(piece.path))?;
        row.set_path("name", string(piece.name))?;
        row.set_path("base_cost", V::Integer(piece.cost))?;
        let mut cfg = template.clone();
        let (armor, protection) = if body {
            (0.87, &body_zones)
        } else {
            (0.72, &head_zones)
        };
        let mut values = vec![
            ("weight", piece.weight),
            ("armor", armor),
            ("ano_armor", 0.7),
        ];
        if body {
            values.extend([("arms_armor", armor), ("back_armor", armor)]);
        }
        for (key, value) in values {
            cfg.set_path(&format!("parameters.{key}"), V::Float(value))?;
        }
        for key in ["item_level", "match_making_level"] {
            cfg.set_path(&format!("parameters.{key}"), V::Integer(15))?;
        }
        cfg.set_path("parameters.max_carried_weight_modifier", V::Integer(0))?;
        // Owner's decision (2026-10-15): sold only, never dropped from a match.
        cfg.set_path("parameters.drop_weight", V::Integer(0))?;
        // Imported textures have no customization masks.
        cfg.set_path("parameters.camo_available", V::Boolean(false))?;
        cfg.set_path("parameters.decal_available", V::Boolean(false))?;
        cfg.set_path("hit_params", protection.clone())?;
        if !piece.modifications.is_empty() {
            cfg.set_path(
                "default_modifications",
                V::Indexed(piece.modifications.iter().map(|&m| V::Integer(m)).collect()),
            )?;
        }
        // 0.26g reads only `data`; drop the template's legacy variants so
        // no native Zubr model stays referenced.
        for key in ["data_full", "data_long", "data_short"] {
            remove_field(&mut cfg, key);
        }
        let covers = V::Indexed(piece.covers.iter().map(|&s| string(s)).collect());
        let model = |name: &str, surface: &str| {
            table(vec![
                field("base_model", string(format!("{NS}/duty_03_{name}"))),
                field("part_name", string(surface)),
            ])
        };
        let mut shown = vec![
            (6, format!("{}", (armor * 100.0f32).round())),
            (8, "70".into()),
            (15, format!("{:.1}", piece.weight)),
        ];
        let data = if body {
            // Native oxygen behaviour (`data.type` 1 makes an oxygen tank of
            // the item), with the displayed capacity.
            cfg.set_path("parameters.oxygen_amount", V::Integer(30))?;
            shown.push((11, "30".into()));
            let mut data = vec![];
            for key in ["type", "amount_time_sec", "influences"] {
                let value = natives
                    .oxygen
                    .named("data")
                    .and_then(|v| v.named(key))
                    .context("oxygen data missing")?;
                data.push(field(key, value.clone()));
            }
            data.extend([
                field("base_model", string(format!("{NS}/duty_03_torso_s1"))),
                field("part_name", string("torso_s")),
                field("base_model_hud", string(format!("{NS}/duty_03_hand_hud"))),
                field("part_name_hud", string("torso_s")),
                field(
                    "additional_models",
                    V::Indexed(vec![
                        model("torso_s1", "exoskeleton_s"),
                        model("torso_s1", "back_s"),
                        model("torso_s1", "sign_s"),
                        model("legs_s1", "legs_s"),
                        model("hand", "torso_s"),
                        model("boot", "legs_s"),
                    ]),
                ),
                field("additional_model_slots", covers),
            ]);
            data
        } else {
            vec![
                field(
                    "type",
                    template
                        .named("data")
                        .and_then(|v| v.named("type"))
                        .context("native helmet type missing")?
                        .clone(),
                ),
                field("base_model", string(format!("{NS}/duty_03_head"))),
                field("part_name", string("helmet_s")),
                // The combined helmet and mask cover the whole head.
                field("remove_head_model", V::Boolean(true)),
                field("additional_model_slots", covers),
            ]
        };
        cfg.set_path("data", table(data))?;
        cfg.set_path("ui_desc.props_list", props(&shown))?;
        let (name, description) = if body {
            ("st_duty_026_suit_name", "st_duty_026_suit_desc")
        } else {
            ("st_duty_026_helmet_name", "st_duty_026_head_desc")
        };
        cfg.set_path("ui_desc.text_descriptions.name", string(name))?;
        cfg.set_path("ui_desc.text_descriptions.description", string(description))?;
        if icons {
            cfg.set_path(
                "ui_desc.icon",
                V::Indexed(piece.icon.iter().map(|&v| V::Integer(v)).collect()),
            )?;
        }
        out.push((piece, row, cfg));
    }
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
    // The suit and its patch are padded cloth; the helmet and the
    // exoskeleton frame and back unit (`attach`) are painted metal.
    use crate::outfit_026::Finish;
    let texture_sets = [
        (
            "torso",
            "dolg_exo_torso",
            "dolg_exo_torso_nmap",
            Finish::CLOTH,
        ),
        ("legs", "dolg_exo_legs", "dolg_exo_legs_nmap", Finish::CLOTH),
        (
            "helmet",
            "dolg_exo_helmet",
            "dolg_exo_helmet_nmap",
            Finish::ARMOUR,
        ),
        (
            "attach",
            "dold_exo_attach",
            "dolg_exo_attach_nmap",
            Finish::ARMOUR,
        ),
        ("sign", "dol_sign", "dol_sign _nmap", Finish::CLOTH),
    ];
    for (part, diffuse, normal, finish) in texture_sets {
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
        // native shader permutation with a constant stock-like finish.
        crate::outfit_026::set_finish(&mut material, finish)?;
        let material_path = format!("material_instances/{NS}/{part}.material");
        // A revision can reuse an identical installed material. Do not mark it
        // as replaced: that would require auditing every unrelated stock model.
        if !target.is_file(&material_path) || target.read(&material_path)? != material.to_bytes()? {
            config(&overlay, &material_path, material.root, &mut files)?;
        }
    }
    let mut natives = Natives {
        oxygen: parse_config(
            &mut target,
            "gameplay/items/armour/back/scavenger_oxygen_1.options",
            "native oxygen behavior",
        )?
        .root,
        protection: vec![],
        templates: vec![],
    };
    for category in ["helmet", "torso", "legs", "gloves", "boots"] {
        let native = parse_config(
            &mut target,
            &format!("gameplay/items/armour/{category}/renesanse_{category}_6.options"),
            "native protection",
        )?;
        natives.protection.push((category.into(), native.root));
    }
    let mut dictionary = parse_config(&mut target, DICT, "0.26 dictionary")?;
    let V::Indexed(items) = dictionary
        .root
        .named_mut("items_dict")
        .context("items_dict missing")?
    else {
        bail!("expected the 0.26 indexed dictionary");
    };
    for piece in &SET {
        let row = items
            .iter()
            .find(|v| v.named("cfg_name") == Some(&string(piece.template)))
            .with_context(|| format!("native {} template missing", piece.template))?
            .clone();
        let config = parse_config(&mut target, piece.template, "native set template")?;
        natives.templates.push((row, config.root));
    }
    // Retire earlier Duty rows (the split preview items and the one-item
    // suit in the back slot), but never remove another mod's IDs.
    for id in 1800..=1806 {
        if let Some(existing) = items
            .iter()
            .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
        {
            check_reused_id(id, existing)?;
        }
    }
    items.retain(
        |v| !matches!(v.named("dict_id"), Some(V::Integer(id)) if (1800..=1806).contains(id)),
    );
    let pieces = duty_set(&natives, icons_swf.is_some())?;
    for (piece, row, _) in &pieces {
        ensure!(
            !items
                .iter()
                .any(|v| v.named("cfg_name") == Some(&string(piece.path))),
            "{} already belongs to another item",
            piece.path
        );
        items.push(row.clone());
    }
    let strings: BTreeMap<String, String> = BTreeMap::from([
        ("st_duty_026_suit_name".into(), "Duty exoskeleton".into()),
        ("st_duty_026_suit_desc".into(), "An experimental sample of a military exoskeleton. Was never mass-produced due to extraordinary cost and some design flaws. Small batches are still made in underground facilities outside Ukraine. Comes with a built-in artifact container.".into()),
        ("st_duty_026_helmet_name".into(), "Duty exoskeleton helmet".into()),
        ("st_duty_026_head_desc".into(), "The sealed helmet of the experimental Duty exoskeleton, with a built-in respirator. Covers the whole head and face.".into()),
    ]);
    let mut registration = vec![];
    for (piece, row, cfg) in pieces {
        config(&overlay, piece.path, cfg, &mut files)?;
        registration.push(json!({
            "dict_id": piece.id,
            "slot": piece.slot,
            "additional_slots": piece.covered_slots()?,
            "name": piece.label,
            "template": piece.template,
            "weight": piece.weight,
            "item_level": 15,
            "match_making_level": 15,
            "default_modifications": piece.modifications,
            "dictionary_entry": format!("{row:?}"),
            "config": piece.path,
        }));
    }
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
        "Built the Duty set, body 1805 and helmet 1800: {} resources",
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A native armour config with its regions, as the 0.26g files are.
    fn native(kind: &str, regions: &[&str], data: V) -> V {
        let hit = |names: &[&str]| {
            table(
                names
                    .iter()
                    .map(|n| field(n, table(vec![field("armor", V::Float(0.5))])))
                    .collect(),
            )
        };
        table(vec![
            field(
                "parameters",
                table(vec![
                    field("type", string(kind)),
                    field("weight", V::Integer(20)),
                    field("drop_weight", V::Integer(10)),
                ]),
            ),
            field(
                "hit_params",
                table(
                    regions
                        .iter()
                        .map(|r| {
                            field(
                                r,
                                table(vec![field("hit_types", hit(&["ranged", "toxic"]))]),
                            )
                        })
                        .collect(),
                ),
            ),
            field(
                "ui_desc",
                table(vec![field(
                    "text_descriptions",
                    table(vec![field("name", string("native"))]),
                )]),
            ),
            field("data", data),
            field(
                "data_full",
                table(vec![field("base_model", string("zubr"))]),
            ),
        ])
    }

    fn natives() -> Natives {
        let model = |type_: i32| {
            table(vec![
                field("type", V::Integer(type_)),
                field("base_model", string("character/human/renesanse/zubr")),
                field(
                    "additional_models",
                    V::Indexed(vec![table(vec![field("base_model", string("balaclava"))])]),
                ),
            ])
        };
        let row = |category: i32| {
            table(vec![
                field("cfg_name", string("native")),
                field("name", string("native")),
                field("item_category", V::Integer(category)),
                field("dict_id", V::Integer(316)),
                field("base_cost", V::Integer(105000)),
            ])
        };
        let oxygen = table(vec![field(
            "data",
            table(vec![
                field("type", V::Integer(1)),
                field("amount_time_sec", V::Integer(30)),
                field("influences", V::Indexed(vec![string("face toxic")])),
            ]),
        )]);
        let zubr = |category: &str, regions: &[&str]| {
            (category.to_owned(), native("arm", regions, model(4)))
        };
        Natives {
            oxygen,
            protection: vec![
                zubr("helmet", &["brain", "face"]),
                zubr("torso", &["chest", "stomach", "left_arm"]),
                zubr("legs", &["left_leg", "right_leg"]),
                zubr("gloves", &["left_hand"]),
                zubr("boots", &["left_foot"]),
            ],
            templates: vec![
                (row(3), native("arm_tors", &["chest"], model(4))),
                (row(1), native("arm_hlmt", &["brain"], model(4))),
            ],
        }
    }

    fn data_of<'a>(cfg: &'a V, key: &str) -> Option<&'a V> {
        cfg.named("data").and_then(|data| data.named(key))
    }

    fn zones(cfg: &V) -> Vec<String> {
        let Some(V::Named(zones)) = cfg.named("hit_params") else {
            panic!("no hit_params")
        };
        zones.iter().map(|zone| zone.name.clone()).collect()
    }

    #[test]
    fn duty_body_is_torso_armour_and_keeps_its_oxygen() {
        let set = duty_set(&natives(), true).unwrap();
        let (piece, row, cfg) = &set[0];
        assert_eq!(piece.id, 1805);
        // The user's report: the suit was a backpack (category 4, slot 3).
        assert_eq!(row.named("item_category"), Some(&V::Integer(3)));
        assert_eq!(piece.slot, 2);
        assert_eq!(row.named("cfg_name"), Some(&string(BODY.path)));
        assert!(BODY.path.starts_with("gameplay/items/armour/torso/"));
        assert_eq!(
            cfg.named("parameters").and_then(|p| p.named("type")),
            Some(&string("arm_tors"))
        );
        // Back, legs, gloves and boots are linked cells; helmet and mask
        // stay free for the set's helmet.
        assert_eq!(piece.covered_slots().unwrap(), [3, 4, 5, 6]);
        assert_eq!(
            data_of(cfg, "additional_model_slots"),
            Some(&V::Indexed(
                ["back", "pants", "gloves", "boots"].map(string).to_vec()
            ))
        );
        // `data.type` 1 makes an oxygen tank of the item; 0.26g's quick bar
        // finds it in any armour slot.
        assert_eq!(data_of(cfg, "type"), Some(&V::Integer(1)));
        assert_eq!(data_of(cfg, "amount_time_sec"), Some(&V::Integer(30)));
        assert!(data_of(cfg, "influences").is_some());
        // The native head shows without the helmet; no head model is worn.
        assert_eq!(data_of(cfg, "remove_head_model"), None);
        let Some(V::Indexed(models)) = data_of(cfg, "additional_models") else {
            panic!("no additional models")
        };
        assert_eq!(models.len(), 6);
        assert!(!format!("{models:?}").contains("duty_03_head"));
        assert!(cfg.named("data_full").is_none());
        assert_eq!(
            cfg.named("default_modifications"),
            Some(&V::Indexed(
                [1030, 1050, 1170, 1040].map(V::Integer).to_vec()
            ))
        );
        assert_eq!(
            cfg.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(1), V::Integer(6)]))
        );
    }

    #[test]
    fn duty_helmet_covers_the_mask_like_zubrs() {
        let set = duty_set(&natives(), true).unwrap();
        let (piece, row, cfg) = &set[1];
        assert_eq!((piece.id, piece.slot), (1800, 0));
        assert_eq!(row.named("item_category"), Some(&V::Integer(1)));
        assert_eq!(row.named("name"), Some(&string("duty_exo_helmet")));
        assert_eq!(piece.covered_slots().unwrap(), [1]);
        assert_eq!(data_of(cfg, "type"), Some(&V::Integer(4)));
        assert_eq!(data_of(cfg, "remove_head_model"), Some(&V::Boolean(true)));
        assert_eq!(
            data_of(cfg, "base_model"),
            Some(&string("ported/duty_026/duty_03_head"))
        );
        // Zubr's balaclava head would show inside the Duty head.
        assert_eq!(data_of(cfg, "additional_models"), None);
        assert_eq!(
            cfg.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(3), V::Integer(6)]))
        );
    }

    #[test]
    fn duty_set_splits_the_suit_without_overlap() {
        let set = duty_set(&natives(), false).unwrap();
        let (body, head) = (&set[0], &set[1]);
        // Worn together: neither covers the other's slots.
        let body_slots: Vec<u8> = [vec![body.0.slot], body.0.covered_slots().unwrap()].concat();
        let head_slots: Vec<u8> = [vec![head.0.slot], head.0.covered_slots().unwrap()].concat();
        assert!(body_slots.iter().all(|slot| !head_slots.contains(slot)));
        let mut all: Vec<u8> = [body_slots, head_slots].concat();
        all.sort_unstable();
        assert_eq!(all, [0, 1, 2, 3, 4, 5, 6]);
        // Each region is protected by one piece.
        assert_eq!(zones(&head.2), ["brain", "face"]);
        let body_zones = zones(&body.2);
        assert_eq!(body_zones.len(), 7);
        assert!(
            body_zones
                .iter()
                .all(|zone| zone != "brain" && zone != "face")
        );
        // The one-item suit's totals: 35 kg and 300 000.
        assert_eq!(body.0.weight + head.0.weight, 35.0);
        let cost = |row: &V| match row.named("base_cost") {
            Some(V::Integer(cost)) => *cost,
            _ => panic!("no cost"),
        };
        assert_eq!(cost(&body.1) + cost(&head.1), 300_000);
        for (_, _, cfg) in &set {
            let p = cfg.named("parameters").unwrap();
            assert_eq!(p.named("drop_weight"), Some(&V::Integer(0)));
            assert_eq!(p.named("item_level"), Some(&V::Integer(15)));
        }
        // No icon swf: the template's own icon stays.
        assert!(body.2.named("ui_desc").unwrap().named("icon").is_none());
    }

    #[test]
    fn only_earlier_duty_rows_are_replaced() {
        let row = |path: &str, name: &str| {
            table(vec![
                field("cfg_name", string(path)),
                field("name", string(name)),
            ])
        };
        let back = row(
            "gameplay/items/armour/back/duty_exo_026.options",
            "duty_exo_back",
        );
        // The released one-item suit, and this set's own rows, are replaced.
        check_reused_id(1805, &back).unwrap();
        check_reused_id(1805, &row(BODY.path, BODY.name)).unwrap();
        check_reused_id(1800, &row(HELMET.path, HELMET.name)).unwrap();
        // Another mod's item, or a Duty row under a foreign ID, is not.
        assert!(check_reused_id(1805, &row("gameplay/items/other.options", "other")).is_err());
        assert!(check_reused_id(1803, &row(BODY.path, BODY.name)).is_err());
        assert!(check_reused_id(1804, &back).is_err());
    }
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
