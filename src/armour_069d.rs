// SPDX-License-Identifier: GPL-3.0-or-later
//! Armour models only Survarium 0.69d has, as v0.26g equipment models.
//!
//! A piece of 0.69d armour is a skinned model on the player's rig with a
//! physically based material. This builder turns each piece of a manifest
//! (`ports/survarium-069d-*-to-026g.json`) into the files a v0.26g item
//! config can draw, at the donor's own paths:
//!
//! - **Surfaces.** The LOD0 surfaces in the v0.26g four-bone layout
//!   ([`crate::v069d_skinned`]); 0.26g's skinned models draw LOD0 only. The
//!   bones a surface is skinned to must bind where the native rig binds them
//!   ([`same_rig_for`]), so the vertices carry over unmoved.
//! - **Model folder.** The bind pose, skeleton, settings and properties of a
//!   native model of the same slot (`template_model`), with the piece's
//!   surfaces and materials, LOD1 and LOD2 empty, and bounds of the converted
//!   vertices. The properties store bounds turned half a turn about the up
//!   axis (`-x, y, -z`), as every native and 0.69d model does.
//! - **Materials.** A native character material (`template_material`) with
//!   the donor's four maps: albedo as diffuse, normal map, smoothness as the
//!   `texture_roughness` channel (which 0.26g reads as gloss) and metalness
//!   as the Fresnel map. 0.69d's smoothness and metalness are no 0.26g gloss
//!   and Fresnel maps, so the material's ranges ([`GLOSS`], [`FRESNEL`]) map
//!   them onto native values. The donor's fuzziness and detail normal map
//!   carry over; the result must be a switch combination a released
//!   character material draws (0.26g ships a compiled shader per
//!   combination).
//! - **Textures.** The donor's DDS in the native container
//!   ([`crate::duty_026::texture_dds`]), larger maps down to
//!   `max_texture_size` by their own mips, a DXT5 colour map as DXT1 (0.26g's
//!   character colour maps carry no alpha), the options of the template's
//!   map of the same role.
//! - **Fit.** A back piece is measured against every stock torso that leaves
//!   the back slot free ([`fit_back`]) and must sink into none of them more
//!   than the native back pieces do.
//!
//! What the item is (its config, level, price, texts, icon) is not this
//! builder's: the pack that ships the models writes it.
use crate::face_items_069d::same_rig_for;
use crate::manifest::BuiltFile;
use crate::mask_fit::{Mesh, closest_on_triangle, cross, dot, parse_mesh, read_u32, sub};
use crate::outfit_026::Writer;
use crate::soc::source::parse_dds;
use crate::v069d_skinned::{parse_069d, to_026_four_bone};
use crate::weapon::parse_config;
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// The pieces to build.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub asset_id: String,
    /// Maps larger than this edge lose their top mips down to it (native
    /// character maps are 1024).
    pub max_texture_size: u32,
    pub pieces: Vec<Piece>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        ensure!(!manifest.pieces.is_empty(), "{}: no pieces", path.display());
        ensure!(
            manifest.max_texture_size.is_power_of_two() && manifest.max_texture_size >= 64,
            "max_texture_size must be a power of two"
        );
        Ok(manifest)
    }
}

/// One 0.69d piece.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    /// The 0.69d item, for provenance.
    pub item: String,
    /// The 0.69d model, under `models/`, without `.skinned_model`.
    pub model: String,
    /// A native model of the same slot, under `models/`.
    pub template_model: String,
    /// The native material the piece's materials start from, under
    /// `material_instances/`, without `.material`.
    pub template_material: String,
    pub fit: Fit,
}

/// What a piece is measured against.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Worn on the back: against the stock torsos ([`fit_back`]).
    Back,
}

/// The gloss range a 0.69d smoothness map is drawn in: the G-buffer writes
/// `min + (max - min) * texel`. The medians of per-texel linear fits of the
/// native `_smap` against 0.69d's over seven backpacks both builds ship
/// (sniper 06, assault 05, combat, scout 07, hunter 01, VDV, Marodeur):
/// native = 0.07 + 0.20 x 0.69d.
pub const GLOSS: (f32, f32) = (0.07, 0.27);
/// The Fresnel range a 0.69d metalness map is drawn in, from the same fits
/// of the native `_fmap`: native = 0.11 + 0.48 x 0.69d (cloth keeps a
/// dielectric's reflectance, bare metal about half the scale).
pub const FRESNEL: (f32, f32) = (0.11, 0.59);

/// A map of a material, by role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Map {
    Albedo,
    Normal,
    Smoothness,
    Metalness,
}

impl Map {
    const ALL: [Self; 4] = [
        Self::Albedo,
        Self::Normal,
        Self::Smoothness,
        Self::Metalness,
    ];

    /// The 0.69d texture field and its switch.
    fn donor(self) -> (&'static str, &'static str) {
        match self {
            Self::Albedo => ("t_albedo", "use_t_albedo"),
            Self::Normal => ("t_normal", "use_t_normal"),
            Self::Smoothness => ("t_smoothness", "use_t_smoothness"),
            Self::Metalness => ("t_metalness", "use_t_metalness"),
        }
    }

    /// The v0.26g texture field and its switch.
    fn native(self) -> (&'static str, &'static str) {
        match self {
            Self::Albedo => ("texture_diffuse", "use_tdiffuse"),
            Self::Normal => ("texture_normal", "use_nmap"),
            Self::Smoothness => ("texture_roughness", "use_troughness"),
            Self::Metalness => ("texture_fresnel", "use_tfresnel"),
        }
    }

    fn srgb(self) -> bool {
        self == Self::Albedo
    }

    /// The block format 0.26g's character maps of this role use.
    fn four_cc(self) -> &'static [u8; 4] {
        match self {
            Self::Normal => b"DXT5",
            _ => b"DXT1",
        }
    }
}

const STAGE: &str = "material.g_stage.effect";
/// The 0.69d fuzziness and the matching 0.26g fields (the same names).
const FUZZINESS: [&str; 3] = [
    "fuzziness_multiplier",
    "fuzziness_power",
    "fuzziness_saturation",
];
/// The 0.69d detail normal map's fields and their 0.26g names.
const DETAIL_NORMAL: [(&str, &str); 3] = [
    ("t_normal_detail", "texture_detail_normal"),
    ("tile_detail_normal", "tile_detail_normal"),
    ("scale_detail_normal", "scale_detail_normal"),
];

fn field<'a>(effect: &'a V, key: &str) -> Option<&'a V> {
    effect.named(key)?.named("value")
}

fn flag(effect: &V, key: &str) -> bool {
    field(effect, key) == Some(&V::Boolean(true))
}

fn text(effect: &V, key: &str) -> Option<String> {
    match field(effect, key) {
        Some(V::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn effect(material: &BinaryConfig) -> Result<&V> {
    material
        .root
        .named("material")
        .and_then(|m| m.named("g_stage"))
        .and_then(|g| g.named("effect"))
        .context("material without a g_stage effect")
}

/// The g_stage switches a material turns on.
fn switches(material: &BinaryConfig) -> Result<BTreeSet<String>> {
    let V::Named(fields) = effect(material)? else {
        bail!("g_stage effect is not a table");
    };
    Ok(fields
        .iter()
        .filter(|f| f.name.starts_with("use_") && f.value.named("value") == Some(&V::Boolean(true)))
        .map(|f| f.name.clone())
        .collect())
}

/// The g_stage switch combinations the released character materials draw.
fn released_combinations(target: &mut ResourceDb) -> Result<BTreeSet<BTreeSet<String>>> {
    let paths: Vec<String> = target
        .paths_with_prefix("material_instances/characters/")
        .filter(|p| p.ends_with(".material"))
        .map(str::to_owned)
        .collect();
    let mut out = BTreeSet::new();
    for path in paths {
        let material = parse_config(target, &path, "material")?;
        if effect(&material).is_ok() {
            out.insert(switches(&material)?);
        }
    }
    ensure!(!out.is_empty(), "no released character material");
    Ok(out)
}

fn set(c: &mut BinaryConfig, path: &str, value: V) -> Result<()> {
    c.root.set_path(path, value)
}

fn source_config(source: &Path, path: &str) -> Result<BinaryConfig> {
    let bytes = fs::read(source.join(path)).with_context(|| format!("0.69d {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing 0.69d {path}"))
}

/// Bytes per 4x4 block of the colour formats a material map may use.
fn block_bytes(four_cc: &[u8; 4]) -> Result<usize> {
    Ok(match four_cc {
        b"DXT1" => 8,
        b"DXT5" => 16,
        other => bail!("map format {:?}", String::from_utf8_lossy(other)),
    })
}

fn blocks(width: u32, height: u32) -> usize {
    (width as usize).div_ceil(4) * (height as usize).div_ceil(4)
}

/// A DXT5 payload's colour blocks as DXT1. DXT5 always decodes its colour
/// block in four-colour mode; DXT1 does only when the first endpoint is the
/// greater, so a block with them the other way round has its endpoints and
/// indices swapped (0 <-> 1, 2 <-> 3), and one with equal endpoints takes
/// index 0 everywhere. The decoded colours are the same.
fn dxt5_to_dxt1(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        payload.len().is_multiple_of(16),
        "DXT5 payload is not whole blocks"
    );
    let mut out = Vec::with_capacity(payload.len() / 2);
    for block in payload.chunks_exact(16) {
        let mut colour: [u8; 8] = block[8..16].try_into()?;
        let c0 = u16::from_le_bytes([colour[0], colour[1]]);
        let c1 = u16::from_le_bytes([colour[2], colour[3]]);
        if c0 == c1 {
            colour[4..8].fill(0);
        } else if c0 < c1 {
            colour.swap(0, 2);
            colour.swap(1, 3);
            let indices = u32::from_le_bytes(colour[4..8].try_into()?) ^ 0x5555_5555;
            colour[4..8].copy_from_slice(&indices.to_le_bytes());
        }
        out.extend_from_slice(&colour);
    }
    Ok(out)
}

/// A donor map in the native container: down to `max` by its mips, in the
/// role's format.
fn convert_texture(bytes: &[u8], label: &str, map: Map, max: u32) -> Result<Vec<u8>> {
    let mut info = parse_dds(bytes, label)?;
    let block = block_bytes(&info.four_cc)?;
    let mut at = 128;
    while info.width > max || info.height > max {
        ensure!(info.mip_levels > 1, "{label}: no mip to shrink to");
        at += blocks(info.width, info.height) * block;
        info.width /= 2;
        info.height /= 2;
        info.mip_levels -= 1;
    }
    let chain: usize = (0..info.mip_levels)
        .map(|level| blocks((info.width >> level).max(1), (info.height >> level).max(1)) * block)
        .sum();
    let payload = bytes
        .get(at..at + chain)
        .with_context(|| format!("{label}: shorter than its mips"))?;
    let (four_cc, payload) = match (&info.four_cc, map.four_cc()) {
        (have, want) if have == want => (*want, payload.to_vec()),
        (b"DXT5", b"DXT1") => (*b"DXT1", dxt5_to_dxt1(payload)?),
        (have, want) => bail!(
            "{label}: {} where 0.26g has {}",
            String::from_utf8_lossy(have),
            String::from_utf8_lossy(want)
        ),
    };
    let mut out = bytes[..128].to_vec();
    out[12..16].copy_from_slice(&info.height.to_le_bytes());
    out[16..20].copy_from_slice(&info.width.to_le_bytes());
    out[28..32].copy_from_slice(&info.mip_levels.to_le_bytes());
    out[84..88].copy_from_slice(&four_cc);
    out.extend_from_slice(&payload);
    crate::duty_026::texture_dds(&out, label, map.srgb())
}

/// Native-convention bounds of converted vertices: half a turn about the up
/// axis (`-x, y, -z`).
fn stored_bounds(min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    ([-max[0], min[1], -max[2]], [-min[0], max[1], -min[2]])
}

fn vertex_bounds(surface: &[u8]) -> Result<([f32; 3], [f32; 3])> {
    crate::outfit_026::converted_bounds(surface)
}

/// A converted surface and the material it is drawn with.
struct Surface {
    name: String,
    bytes: Vec<u8>,
    material: String,
    /// The donor's surface properties (mesh name, texture name).
    donor_properties: BinaryConfig,
}

/// The piece's LOD0 surfaces in the four-bone layout; each may be skinned
/// only to bones that bind where `native_pose` binds them.
fn convert_surfaces(source: &Path, piece: &Piece, native_pose: &[u8]) -> Result<Vec<Surface>> {
    let root = format!("models/{}.skinned_model", piece.model);
    let settings = source_config(source, &format!("{root}/settings"))?;
    let Some(V::Indexed(lod0)) = settings
        .root
        .named("lod_hierrarchy")
        .and_then(|l| l.named("LOD0"))
        .and_then(|l| l.named("surfaces"))
    else {
        bail!("{}: no LOD0 surfaces", piece.model);
    };
    let donor_pose = fs::read(source.join(format!("{root}/render/bind_pose")))?;
    let mut out = vec![];
    for name in lod0 {
        let V::String(name) = name else {
            bail!("{}: a LOD0 surface is not a name", piece.model);
        };
        let Some(V::String(material)) = settings
            .root
            .named("material_settings")
            .and_then(|m| m.named(name))
            .and_then(|m| m.named("material_name"))
        else {
            bail!("{}: surface {name} has no material", piece.model);
        };
        let label = format!("{} {name}", piece.model);
        let donor = fs::read(source.join(format!("{root}/render/{name}/converted_model")))
            .with_context(|| label.clone())?;
        let parsed = parse_069d(&donor, &label)?;
        let used: BTreeSet<usize> = parsed
            .vertices
            .iter()
            .flat_map(|v| v.bones.iter().zip(v.weights))
            .filter(|(_, w)| *w > 0.0)
            .map(|(&b, _)| usize::from(b))
            .collect();
        same_rig_for(&donor_pose, native_pose, &label, Some(&used))?;
        out.push(Surface {
            name: name.clone(),
            bytes: to_026_four_bone(&parsed)?,
            material: material.clone(),
            donor_properties: source_config(
                source,
                &format!("{root}/render/{name}/export_properties"),
            )?,
        });
    }
    ensure!(!out.is_empty(), "{}: no LOD0 surface", piece.model);
    Ok(out)
}

/// What a material conversion did, for the record.
#[derive(Debug, Serialize)]
pub struct MaterialRecord {
    pub material: String,
    pub template: String,
    pub switches: Vec<String>,
    pub textures: Vec<String>,
}

/// What every material conversion reads: the donor's files, the size
/// limit, and the switch combinations the released characters draw.
struct MaterialInputs<'a> {
    source: &'a Path,
    max_texture_size: u32,
    released: &'a BTreeSet<BTreeSet<String>>,
}

/// One donor material as a native one, its maps written beside it.
fn convert_material(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    inputs: &MaterialInputs<'_>,
    name: &str,
    piece: &Piece,
    files: &mut Vec<BuiltFile>,
) -> Result<MaterialRecord> {
    let MaterialInputs {
        source,
        max_texture_size,
        released,
    } = *inputs;
    let path = format!("material_instances/{name}.material");
    ensure!(!target.contains(&path), "{path} is in the released game");
    let donor = source_config(source, &path)?;
    let donor_effect = effect(&donor)?.clone();
    let template_path = format!("material_instances/{}.material", piece.template_material);
    let mut material = parse_config(target, &template_path, "template material")?;
    let template_effect = effect(&material)?.clone();
    let mut textures = vec![];
    for map in Map::ALL {
        let (donor_key, donor_switch) = map.donor();
        let (native_key, native_switch) = map.native();
        ensure!(
            flag(&donor_effect, donor_switch),
            "{name}: {donor_switch} is off; the template draws {native_key}"
        );
        ensure!(
            flag(&template_effect, native_switch),
            "{template_path}: {native_switch} is off"
        );
        let texture =
            text(&donor_effect, donor_key).with_context(|| format!("{name}: no {donor_key}"))?;
        let template_texture = text(&template_effect, native_key)
            .with_context(|| format!("{template_path}: no {native_key}"))?;
        let dds = format!("textures/{texture}.dds");
        ensure!(!target.contains(&dds), "{dds} is in the released game");
        let bytes = fs::read(source.join(&dds)).with_context(|| format!("0.69d {dds}"))?;
        out.raw(
            &dds,
            &convert_texture(&bytes, &dds, map, max_texture_size)?,
            files,
        )?;
        let options = parse_config(
            target,
            &format!("textures/{template_texture}.options"),
            "template texture options",
        )?;
        out.config(&format!("textures/{texture}.options"), &options, files)?;
        set(
            &mut material,
            &format!("{STAGE}.{native_key}.value"),
            V::String(texture.clone()),
        )?;
        textures.push(texture);
    }
    for (key, value) in [
        ("constant_roughness_min", GLOSS.0),
        ("constant_roughness_max", GLOSS.1),
        ("constant_fresnel_min", FRESNEL.0),
        ("constant_fresnel_max", FRESNEL.1),
    ] {
        set(
            &mut material,
            &format!("{STAGE}.{key}.value"),
            V::Float(value),
        )?;
    }
    let fuzzy = flag(&donor_effect, "use_fuzziness");
    set(
        &mut material,
        &format!("{STAGE}.use_fuzziness.value"),
        V::Boolean(fuzzy),
    )?;
    if fuzzy {
        for key in FUZZINESS {
            let value = field(&donor_effect, key)
                .with_context(|| format!("{name}: fuzziness without {key}"))?
                .clone();
            set(&mut material, &format!("{STAGE}.{key}.value"), value)?;
        }
    }
    let detail = flag(&donor_effect, "use_t_normal_detail");
    set(
        &mut material,
        &format!("{STAGE}.use_detail_nmap.value"),
        V::Boolean(detail),
    )?;
    if detail {
        for (donor_key, native_key) in DETAIL_NORMAL {
            let value = field(&donor_effect, donor_key)
                .with_context(|| format!("{name}: detail normal map without {donor_key}"))?
                .clone();
            if let V::String(texture) = &value {
                ensure!(
                    target.contains(&format!("textures/{texture}.dds")),
                    "{name}: detail normal map {texture} is not in the released game"
                );
            }
            set(&mut material, &format!("{STAGE}.{native_key}.value"), value)?;
        }
    }
    let on = switches(&material)?;
    ensure!(
        released.contains(&on),
        "{name}: no released character material draws {on:?} (no shader for it)"
    );
    out.config(&path, &material, files)?;
    Ok(MaterialRecord {
        material: name.to_owned(),
        template: piece.template_material.clone(),
        switches: on.into_iter().collect(),
        textures,
    })
}

fn named(entries: Vec<(String, V)>) -> V {
    V::Named(
        entries
            .into_iter()
            .map(|(name, value)| NamedValue { name, value })
            .collect(),
    )
}

/// The model folder: the template's bind pose, skeleton, settings and
/// properties around the piece's surfaces.
fn model_files(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    source: &Path,
    piece: &Piece,
    surfaces: &[Surface],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let root = format!("models/{}.skinned_model", piece.model);
    let template = format!("models/{}.skinned_model", piece.template_model);
    ensure!(
        target
            .paths_with_prefix(&format!("{root}/"))
            .next()
            .is_none(),
        "{root} is in the released game"
    );
    out.raw(
        &format!("{root}/render/bind_pose"),
        &target.read(&format!("{template}/render/bind_pose"))?,
        files,
    )?;
    out.raw(
        &format!("{root}/skeleton"),
        &target.read(&format!("{template}/skeleton"))?,
        files,
    )?;
    let mut settings = parse_config(target, &format!("{template}/settings"), "template settings")?;
    let Some(V::String(template_surface)) = settings
        .root
        .named("lod_hierrarchy")
        .and_then(|l| l.named("LOD0"))
        .and_then(|l| l.named("surfaces"))
        .and_then(|s| match s {
            V::Indexed(list) => list.first(),
            _ => None,
        })
        .cloned()
    else {
        bail!("{template}: no LOD0 surface");
    };
    set(
        &mut settings,
        "material_settings",
        named(
            surfaces
                .iter()
                .map(|s| {
                    (
                        s.name.clone(),
                        named(vec![(
                            "material_name".to_owned(),
                            V::String(s.material.clone()),
                        )]),
                    )
                })
                .collect(),
        ),
    )?;
    set(
        &mut settings,
        "lod_hierrarchy.LOD0.surfaces",
        V::Indexed(surfaces.iter().map(|s| V::String(s.name.clone())).collect()),
    )?;
    for lod in ["LOD1", "LOD2"] {
        set(
            &mut settings,
            &format!("lod_hierrarchy.{lod}"),
            V::Named(vec![]),
        )?;
    }
    out.config(&format!("{root}/settings"), &settings, files)?;

    let (mut all_min, mut all_max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for surface in surfaces {
        let (min, max) = vertex_bounds(&surface.bytes)?;
        for k in 0..3 {
            all_min[k] = all_min[k].min(min[k]);
            all_max[k] = all_max[k].max(max[k]);
        }
        let (stored_min, stored_max) = stored_bounds(min, max);
        let vertices = read_u32(&surface.bytes, 8)?;
        let indices = read_u32(&surface.bytes, 12 + vertices * 48 + 8)?;
        out.raw(
            &format!("{root}/render/{}/converted_model", surface.name),
            &surface.bytes,
            files,
        )?;
        let mut props = parse_config(
            target,
            &format!("{template}/render/{template_surface}/export_properties"),
            "template surface properties",
        )?;
        for (key, value) in [
            ("source.vertices", vertices),
            ("source.indices", indices),
            ("source.primitives", indices / 3),
        ] {
            set(&mut props, key, V::Integer(i32::try_from(value)?))?;
        }
        // 43 + bones a vertex: the four-bone layout.
        set(&mut props, "type", V::Integer(47))?;
        for key in ["mesh_name", "texture_name"] {
            if let Some(value) = surface
                .donor_properties
                .root
                .named("source")
                .and_then(|s| s.named(key))
            {
                set(&mut props, &format!("source.{key}"), value.clone())?;
            }
        }
        set(&mut props, "bounding_box.min", V::Float3(stored_min))?;
        set(&mut props, "bounding_box.max", V::Float3(stored_max))?;
        out.config(
            &format!("{root}/render/{}/export_properties", surface.name),
            &props,
            files,
        )?;
    }
    let mut render = parse_config(
        target,
        &format!("{template}/render/export_properties"),
        "template render properties",
    )?;
    let donor_render = source_config(source, &format!("{root}/render/export_properties"))?;
    if let Some(value) = donor_render.root.named("src_filename") {
        set(&mut render, "src_filename", value.clone())?;
    }
    let (stored_min, stored_max) = stored_bounds(all_min, all_max);
    set(&mut render, "bounding_box.min", V::Float3(stored_min))?;
    set(&mut render, "bounding_box.max", V::Float3(stored_max))?;
    out.config(&format!("{root}/render/export_properties"), &render, files)
}

/// Within this of a torso's surface a back piece's vertex counts (m).
pub const NEAR: f64 = 0.05;
/// Deeper than this under it, the vertex is sunk into the torso (m).
pub const SUNK: f64 = 0.005;

/// A mesh's triangles by cubes of [`NEAR`]: every triangle within `NEAR` of
/// a point is listed in one of the 27 cubes around it.
struct Grid {
    mesh: Mesh,
    normals: Vec<[f64; 3]>,
    cells: HashMap<[i64; 3], Vec<usize>>,
}

fn cell_of(p: [f64; 3]) -> [i64; 3] {
    p.map(|v| (v / NEAR).floor() as i64)
}

impl Grid {
    fn new(mesh: Mesh) -> Self {
        let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        let mut normals = Vec::with_capacity(mesh.triangles.len());
        for (index, tri) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|v| mesh.positions[v]);
            normals.push(cross(sub(b, a), sub(c, a)));
            let lo = cell_of([0, 1, 2].map(|k| a[k].min(b[k]).min(c[k])));
            let hi = cell_of([0, 1, 2].map(|k| a[k].max(b[k]).max(c[k])));
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        cells.entry([x, y, z]).or_default().push(index);
                    }
                }
            }
        }
        Self {
            mesh,
            normals,
            cells,
        }
    }

    /// The distance to the closest surface point within [`NEAR`], negative
    /// behind it (by that triangle's facing).
    fn signed_distance(&self, p: [f64; 3]) -> Option<f64> {
        let centre = cell_of(p);
        let mut best: Option<(f64, f64)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = [centre[0] + dx, centre[1] + dy, centre[2] + dz];
                    for &t in self.cells.get(&key).into_iter().flatten() {
                        let [a, b, c] = self.mesh.triangles[t].map(|v| self.mesh.positions[v]);
                        let w = closest_on_triangle(p, a, b, c);
                        let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * c[k]);
                        let d = sub(p, q);
                        let squared = dot(d, d);
                        if best.is_none_or(|(s, _)| squared < s) {
                            best = Some((squared, dot(d, self.normals[t])));
                        }
                    }
                }
            }
        }
        best.filter(|(s, _)| s.sqrt() <= NEAR)
            .map(|(s, facing)| if facing < 0.0 { -s.sqrt() } else { s.sqrt() })
    }

    /// The share of `points` sunk into the mesh: within [`NEAR`] of it and
    /// more than [`SUNK`] behind it.
    fn sunk_share(&self, points: &[[f64; 3]]) -> f64 {
        let sunk = points
            .iter()
            .filter(|&&p| self.signed_distance(p).is_some_and(|d| d < -SUNK))
            .count();
        sunk as f64 / points.len().max(1) as f64
    }
}

fn stride(surface: &[u8]) -> Result<usize> {
    let count = read_u32(surface, 8)?;
    ensure!(count > 0, "a surface without vertices");
    Ok((read_u32(surface, 4)? - 4) / count)
}

fn surface_mesh(bytes: &[u8], label: &str) -> Result<Mesh> {
    parse_mesh(bytes, stride(bytes)?, label)
}

/// The surfaces an item config draws (its model and additional models).
fn item_meshes(
    target: &mut ResourceDb,
    config: &BinaryConfig,
    label: &str,
) -> Result<Vec<(String, Mesh)>> {
    let data = config
        .root
        .named("data")
        .with_context(|| format!("{label}: no data"))?;
    let mut parts = vec![data.clone()];
    if let Some(V::Indexed(extra)) = data.named("additional_models") {
        parts.extend(extra.iter().cloned());
    }
    let mut out = vec![];
    for part in parts {
        let (Some(V::String(model)), Some(V::String(surface))) =
            (part.named("base_model"), part.named("part_name"))
        else {
            bail!("{label}: a model part without base_model and part_name");
        };
        let path = format!("models/{model}.skinned_model/render/{surface}/converted_model");
        out.push((
            format!("{model} {surface}"),
            surface_mesh(&target.read(&path)?, &path)?,
        ));
    }
    Ok(out)
}

fn configs_in(target: &mut ResourceDb, folder: &str) -> Result<Vec<(String, BinaryConfig)>> {
    let paths: Vec<String> = target
        .paths_with_prefix(folder)
        .filter(|p| p.ends_with(".options"))
        .map(str::to_owned)
        .collect();
    paths
        .into_iter()
        .map(|p| Ok((p.clone(), parse_config(target, &p, "item config")?)))
        .collect()
}

fn covers_back(config: &BinaryConfig) -> bool {
    matches!(
        config.root.named("data").and_then(|d| d.named("additional_model_slots")),
        Some(V::Indexed(slots)) if slots.contains(&V::String("back".to_owned()))
    )
}

/// One torso of the fit measure.
#[derive(Debug, Serialize)]
pub struct TorsoFit {
    pub torso: String,
    /// The share of the piece's vertices sunk into it.
    pub share: f64,
    /// The same of the native back pieces: their median and the largest.
    pub native_median: f64,
    pub native_max: f64,
    pub native_max_piece: String,
}

/// The stock torsos a back piece is measured against, and the native back
/// pieces' vertices.
pub struct BackReference {
    torsos: Vec<(String, Grid)>,
    natives: Vec<(String, Vec<[f64; 3]>)>,
}

impl BackReference {
    /// Every torso item's surfaces whose item leaves the back slot free (the
    /// exoskeletons take it), and every back item's.
    pub fn new(target: &mut ResourceDb) -> Result<Self> {
        let mut seen = BTreeSet::new();
        let mut torsos = vec![];
        for (path, config) in configs_in(target, "gameplay/items/armour/torso/")? {
            if covers_back(&config) {
                continue;
            }
            for (label, mesh) in item_meshes(target, &config, &path)? {
                if seen.insert(label.clone()) {
                    torsos.push((label, Grid::new(mesh)));
                }
            }
        }
        let mut natives = vec![];
        for (path, config) in configs_in(target, "gameplay/items/armour/back/")? {
            let points = item_meshes(target, &config, &path)?
                .into_iter()
                .flat_map(|(_, mesh)| mesh.positions)
                .collect();
            natives.push((path, points));
        }
        ensure!(
            !torsos.is_empty() && !natives.is_empty(),
            "no stock torsos or back pieces"
        );
        Ok(Self { torsos, natives })
    }
}

/// The back piece against every torso, beside the native back pieces; the
/// piece may sink into no torso more than the deepest native one does.
pub fn fit_back(
    reference: &BackReference,
    label: &str,
    points: &[[f64; 3]],
) -> Result<Vec<TorsoFit>> {
    let mut out = vec![];
    for (torso, grid) in &reference.torsos {
        let mut natives: Vec<(f64, &str)> = reference
            .natives
            .iter()
            .map(|(name, p)| (grid.sunk_share(p), name.as_str()))
            .collect();
        natives.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (native_max, native_max_piece) = *natives.last().context("no native back piece")?;
        let fit = TorsoFit {
            torso: torso.clone(),
            share: grid.sunk_share(points),
            native_median: natives[natives.len() / 2].0,
            native_max,
            native_max_piece: native_max_piece.to_owned(),
        };
        ensure!(
            fit.share <= fit.native_max,
            "{label} sinks into {torso} deeper than any native back piece: {:.1} % of its vertices (native at most {:.1} %, {})",
            fit.share * 100.0,
            fit.native_max * 100.0,
            fit.native_max_piece
        );
        out.push(fit);
    }
    Ok(out)
}

/// What the builder made of a piece.
#[derive(Debug, Serialize)]
pub struct PieceRecord {
    pub item: String,
    pub model: String,
    pub surfaces: Vec<String>,
    pub vertices: usize,
    pub materials: Vec<MaterialRecord>,
    pub fit: Vec<TorsoFit>,
}

/// Builds the manifest's pieces into `output` (`overlay/`,
/// `port-manifest.json`, `fit-report.json`). `patches`: the released
/// `resources.db.patch_<n>` over `db`, as players have them.
pub fn build(
    manifest: &Manifest,
    source: &Path,
    db: &Path,
    patches: &[PathBuf],
    output: &Path,
) -> Result<Vec<PieceRecord>> {
    crate::weapon::prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Armour-069d",
    };
    let mut target = crate::novice_026::game_view(db, patches)?;
    let released = released_combinations(&mut target)?;
    let reference = BackReference::new(&mut target)?;
    let inputs = MaterialInputs {
        source,
        max_texture_size: manifest.max_texture_size,
        released: &released,
    };
    let mut files = vec![];
    let mut records = vec![];
    let mut materials = BTreeSet::new();
    for piece in &manifest.pieces {
        let native_pose = target.read(&format!(
            "models/{}.skinned_model/render/bind_pose",
            piece.template_model
        ))?;
        let surfaces = convert_surfaces(source, piece, &native_pose)?;
        let mut piece_materials = vec![];
        for name in surfaces
            .iter()
            .map(|s| s.material.clone())
            .collect::<BTreeSet<_>>()
        {
            ensure!(
                materials.insert(name.clone()),
                "material {name} is used by two pieces"
            );
            piece_materials.push(convert_material(
                &out,
                &mut target,
                &inputs,
                &name,
                piece,
                &mut files,
            )?);
        }
        model_files(&out, &mut target, source, piece, &surfaces, &mut files)?;
        let mut points = vec![];
        for surface in &surfaces {
            points.extend(surface_mesh(&surface.bytes, &surface.name)?.positions);
        }
        let fit = match piece.fit {
            Fit::Back => fit_back(&reference, &piece.item, &points)?,
        };
        let above_median = fit.iter().filter(|f| f.share > f.native_median).count();
        let worst = fit
            .iter()
            .max_by(|a, b| a.share.total_cmp(&b.share))
            .context("no torso measured")?;
        println!(
            "{} ({}): {} surfaces, {} vertices; sunk into {} torsos above the native median, at most {:.1} % ({}; native up to {:.1} %)",
            piece.item,
            piece.model,
            surfaces.len(),
            points.len(),
            above_median,
            worst.share * 100.0,
            worst.torso,
            worst.native_max * 100.0
        );
        records.push(PieceRecord {
            item: piece.item.clone(),
            model: piece.model.clone(),
            surfaces: surfaces.iter().map(|s| s.name.clone()).collect(),
            vertices: points.len(),
            materials: piece_materials,
            fit,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    crate::weapon::reject_duplicate_outputs(&files)?;
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": manifest.asset_id,
            "donor_build": "0.69d",
            "target_build": "0.26g",
            "gloss_range": [GLOSS.0, GLOSS.1],
            "fresnel_range": [FRESNEL.0, FRESNEL.1],
            "files": files,
        }))?,
    )?;
    fs::write(
        output.join("fit-report.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {} 0.69d pieces: {} resources",
        records.len(),
        files.len()
    );
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxt5_block(c0: u16, c1: u16, indices: u32) -> Vec<u8> {
        let mut b = vec![0xff, 0xff, 0, 0, 0, 0, 0, 0];
        b.extend_from_slice(&c0.to_le_bytes());
        b.extend_from_slice(&c1.to_le_bytes());
        b.extend_from_slice(&indices.to_le_bytes());
        b
    }

    #[test]
    fn dxt5_colour_blocks_decode_the_same_as_dxt1() {
        // In order: kept as it is.
        let kept = dxt5_to_dxt1(&dxt5_block(0xf800, 0x001f, 0x1b1b_1b1b)).unwrap();
        assert_eq!(kept, [0x00, 0xf8, 0x1f, 0x00, 0x1b, 0x1b, 0x1b, 0x1b]);
        // The other way round: endpoints and indices swapped, 0 <-> 1, 2 <-> 3.
        let swapped = dxt5_to_dxt1(&dxt5_block(0x001f, 0xf800, 0b11_10_01_00)).unwrap();
        assert_eq!(&swapped[..4], &[0x00, 0xf8, 0x1f, 0x00]);
        assert_eq!(
            u32::from_le_bytes(swapped[4..8].try_into().unwrap()) & 0xff,
            0b10_11_00_01
        );
        // Equal endpoints: one colour, index 0 (index 3 would be transparent).
        let flat = dxt5_to_dxt1(&dxt5_block(0x1234, 0x1234, 0xffff_ffff)).unwrap();
        assert_eq!(&flat[4..], &[0, 0, 0, 0]);
        assert!(dxt5_to_dxt1(&[0; 15]).is_err());
    }

    fn dds(width: u32, height: u32, mips: u32, four_cc: &[u8; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..4].copy_from_slice(b"DDS ");
        b[4..8].copy_from_slice(&124u32.to_le_bytes());
        b[12..16].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&width.to_le_bytes());
        b[28..32].copy_from_slice(&mips.to_le_bytes());
        b[76..80].copy_from_slice(&32u32.to_le_bytes());
        b[80..84].copy_from_slice(&4u32.to_le_bytes());
        b[84..88].copy_from_slice(four_cc);
        let block = block_bytes(four_cc).unwrap();
        for level in 0..mips {
            let n = blocks((width >> level).max(1), (height >> level).max(1));
            // Each level filled with its own number, to see which ones stay.
            b.extend(std::iter::repeat_n(level as u8 + 1, n * block));
        }
        b.push(1);
        b
    }

    #[test]
    fn large_maps_shrink_by_their_own_mips() {
        let source = dds(64, 64, 5, b"DXT1");
        let out = convert_texture(&source, "test", Map::Smoothness, 32).unwrap();
        let info = parse_dds(&out, "out").unwrap();
        assert_eq!((info.width, info.height), (32, 32));
        // 32 down to 4: four levels; the first is the source's second.
        assert_eq!(info.mip_levels, 4);
        assert_eq!(out[128], 2);
        assert_eq!(out.len(), 128 + (64 + 16 + 4 + 1) * 8 + 1);
        // Linear data: the trailing colour-space byte is 0.
        assert_eq!(*out.last().unwrap(), 0);
    }

    #[test]
    fn a_dxt5_colour_map_becomes_dxt1_and_a_normal_map_must_be_dxt5() {
        let source = dds(16, 16, 3, b"DXT5");
        let out = convert_texture(&source, "test", Map::Albedo, 1024).unwrap();
        assert_eq!(&out[84..88], b"DXT1");
        assert_eq!(out.len(), 128 + (16 + 4 + 1) * 8 + 1);
        assert_eq!(*out.last().unwrap(), 1);
        assert!(convert_texture(&dds(16, 16, 3, b"DXT1"), "test", Map::Normal, 1024).is_err());
    }

    #[test]
    fn stored_bounds_turn_half_about_the_up_axis() {
        // The native Wanderer container: vertices x -0.179..0.074, z -0.315..-0.125;
        // stored x -0.074..0.179, z 0.125..0.315.
        let (min, max) = stored_bounds([-0.179, 1.056, -0.315], [0.074, 1.476, -0.125]);
        assert_eq!(min, [-0.074, 1.056, 0.125]);
        assert_eq!(max, [0.179, 1.476, 0.315]);
    }

    #[test]
    fn sunk_vertices_are_the_near_ones_behind_the_surface() {
        // A square in z = 0 facing +z, cut in two triangles.
        let mesh = Mesh {
            positions: vec![
                [-1.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        let grid = Grid::new(mesh);
        assert!(grid.signed_distance([0.2, 0.1, 0.01]).unwrap() > 0.0);
        assert!(grid.signed_distance([0.2, 0.1, -0.01]).unwrap() < 0.0);
        // Beyond NEAR: not measured.
        assert_eq!(grid.signed_distance([0.2, 0.1, -0.2]), None);
        let points = [
            [0.0, 0.0, 0.01],
            [0.0, 0.0, -0.002],
            [0.0, 0.0, -0.02],
            [0.0, 0.0, -0.3],
        ];
        assert_eq!(grid.sunk_share(&points), 0.25);
    }

    #[test]
    fn a_manifest_names_its_fit() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"asset_id": "a", "max_texture_size": 1024, "pieces": [{"item": "i", "model": "m",
               "template_model": "t", "template_material": "tm", "fit": "back"}]}"#,
        )
        .unwrap();
        assert_eq!(manifest.pieces[0].fit, Fit::Back);
        assert!(
            serde_json::from_str::<Manifest>(
                r#"{"asset_id": "a", "max_texture_size": 1024, "pieces": [{"item": "i", "model": "m",
                   "template_model": "t", "template_material": "tm", "fit": "head"}]}"#
            )
            .is_err()
        );
    }
}
