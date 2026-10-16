// SPDX-License-Identifier: GPL-3.0-or-later
//! Bring a map of a later build (0.69d0, 2022) down to 0.26g.
//!
//! 0.69d ships its resources as 76 patch volumes (see `ResourceDb` split sets)
//! with 16-byte config records (see `binary_config`). Static geometry keeps the
//! 0.26 vertex layout except for the tangent basis; materials use a
//! metalness/smoothness effect that 0.26 has no shader for, and the scene uses
//! object types and photometric lighting 0.26 does not know.
//!
//! The conversion is template-first: every emitted scene object and material
//! starts from a native 0.26 one and takes only values whose meaning is the
//! same in both builds. Native shader permutations therefore never change.
//! What has no 0.26 equivalent keeps the native look of the template map.
use super::{BinaryConfig, ConfigValue, NamedValue, Record, ResourceDb, SourceBuild};
use super::{array, check_hash, emit, string};
use crate::manifest::BuiltFile;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::{fs, path::Path};

const DICTIONARY: &str = "gameplay/db_static_dictionaries";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub source_build: SourceBuild,
    /// `archive::split_fingerprint` of the donor volume list.
    pub donor_fingerprint: String,
    pub target_sha256: String,
    pub map: MapSpec,
    /// 0.69d-only game materials (collision surfaces) -> the 0.26 material
    /// that behaves the same for bullets and footsteps.
    pub game_material_names: BTreeMap<String, String>,
    /// 0.26 template material per converted-material category.
    pub material_templates: BTreeMap<String, String>,
    /// Scale from 0.69d smoothness to the value range of 0.26's
    /// `t_roughness` channel (which the gbuffer pass consumes as gloss).
    pub roughness_scale: f32,
    /// Models never taken from 0.26 even when an equivalent ships there.
    #[serde(default)]
    pub force_donor_models: BTreeSet<String>,
    /// Native particles that stand in for 0.69d particles, by donor name.
    #[serde(default)]
    pub particle_replacements: BTreeMap<String, String>,
    /// Halve every imported texture larger than this many texels on a side.
    #[serde(default)]
    pub max_texture_size: Option<u32>,
    /// Tighter limits for texture names ending with a suffix (e.g. `_smap`,
    /// smoothness maps 0.26 only samples as a low-range gloss channel).
    #[serde(default)]
    pub texture_limits: BTreeMap<String, u32>,
    /// Distance LOD switching (0.26 type 0: LOD0 below the first distance,
    /// LOD1 below the second, LOD2 below the third) for imported models with
    /// LODs. 0.69d authors most of them for projected size, which keeps big
    /// structures at full detail across the map.
    #[serde(default)]
    pub lod_distances: Option<[f32; 3]>,
    /// Model-name prefixes of regular-grid terrain whose render mesh keeps
    /// every second grid line (collision is unchanged).
    #[serde(default)]
    pub decimate_terrain: Vec<String>,
    /// Mesh simplification of imported far LODs (meshoptimizer, error-bounded).
    #[serde(default)]
    pub simplify: Option<Simplify>,
    /// Native ambient emitters (`object_sound`) borrowed from another native
    /// map: 0.69d's sound environments have no 0.26 counterpart.
    #[serde(default)]
    pub ambient_sounds: Option<AmbientSounds>,
    /// Static batching: render instances of one imported model within a
    /// square cell of this size (metres) become one render-only model with
    /// the instance transforms baked in (one draw per surface instead of one
    /// per instance). Collision instances are unchanged.
    #[serde(default)]
    pub batch_cell: Option<f32>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AmbientSounds {
    pub project: String,
    /// `full_name` or `sound_name` prefixes of the emitters to copy.
    pub include: Vec<String>,
    /// Added to every copied position (the two maps' ground heights differ).
    pub offset: [f32; 3],
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Simplify {
    /// Target index-count ratio of authored LOD1 / LOD2 surfaces.
    pub lod1_ratio: f32,
    pub lod2_ratio: f32,
    /// Surfaces with fewer triangles stay as they are.
    pub min_triangles: usize,
    /// Relative error bound (fraction of the mesh extent).
    pub max_error: f32,
    /// Models without LODs whose LOD0 exceeds this many triangles get
    /// generated LOD1/LOD2 surfaces at the two ratios.
    pub generate_above: usize,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapSpec {
    pub id: u8,
    /// HUD key; 0.26 has no minimap for it.
    pub level: String,
    pub donor_project: String,
    /// Native project the scene schema, environment and global probe come from.
    pub template_project: String,
    /// Native mode options to clone (its `mode.mode` selects the rules).
    pub template_mode: String,
    /// Output project directory under `projects/`.
    pub output_project: String,
    pub mode_name: String,
    /// Centre and radius of the global sphere probe taken from the template.
    pub global_probe_position: [f32; 3],
    pub global_probe_radius: f32,
    /// 0.69d probe folders (full-name prefixes, first match wins) and the
    /// native template probe whose cubemap stands in for them. 0.69d probe
    /// cubemaps do not decode in 0.26; the volumes keep their placement.
    #[serde(default)]
    pub probe_classes: Vec<ProbeClass>,
    /// Keep 0.69d ambient lights (their photometric units and shapes 2/3 have
    /// no 0.26 equivalent; off by default).
    #[serde(default)]
    pub ambient_lights: bool,
    /// Which donor respawn folder supplies the spawns (TDM).
    pub spawn_folder: String,
}

fn value_of<'a>(effect: &'a ConfigValue, name: &str) -> Option<&'a ConfigValue> {
    let field = effect.named(name)?;
    Some(field.named("value").unwrap_or(field))
}

fn flag(effect: &ConfigValue, name: &str) -> bool {
    matches!(value_of(effect, name), Some(ConfigValue::Boolean(true)))
}

fn text<'a>(effect: &'a ConfigValue, name: &str) -> Option<&'a str> {
    match value_of(effect, name) {
        Some(ConfigValue::String(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

fn number(effect: &ConfigValue, name: &str) -> Option<f32> {
    match value_of(effect, name)? {
        ConfigValue::Float(v) => Some(*v),
        ConfigValue::Integer(v) => Some(*v as f32),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Repack a 0.69d static `converted_model` for 0.26.
///
/// Both builds store position (float3), a 12-byte tangent basis, half2 UV and
/// an optional colour. 0.26 packs each basis vector as bytes `z, y, x, alpha`;
/// 0.69d stores each as one float whose value is `(x << 16 | y << 8 | z) /
/// 2^24` - the same three bytes, carried exactly by the float mantissa. The
/// alpha bytes (normal 127, tangent/binormal 255 on 0.26's static meshes) are
/// not stored by 0.69d and take the canonical values. Chunks 110 and 114
/// (0.69d mesh statistics) are dropped; any other unknown chunk is an error.
pub fn geometry(bytes: &[u8]) -> Result<Vec<u8>> {
    let u32_at = |offset: usize| -> Result<usize> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .context("truncated geometry")?
                .try_into()
                .unwrap(),
        ) as usize)
    };
    ensure!(u32_at(0)? == 3, "expected a static vertex chunk first");
    let size = u32_at(4)?;
    let count = u32_at(8)?;
    ensure!(count > 0 && size >= 4, "empty vertex chunk");
    let stride = (size - 4) / count;
    ensure!(
        matches!(stride, 28 | 32) && 4 + stride * count == size && bytes.len() >= 8 + size,
        "unsupported static vertex chunk (stride {stride})"
    );
    let mut result = bytes[..8 + size].to_vec();
    for vertex in result[12..8 + size].chunks_exact_mut(stride) {
        for (slot, alpha) in [(12, 0x7f_u32), (16, 0xff), (20, 0xff)] {
            let raw = u32::from_le_bytes(vertex[slot..slot + 4].try_into().unwrap());
            let value = f32::from_bits(raw);
            ensure!(
                value.is_finite() && (0.0..1.0).contains(&value),
                "basis value {raw:#010x} is not a 0.69d packed float"
            );
            let packed = (value as f64 * 16_777_216.0).round() as u32;
            ensure!(packed <= 0xff_ffff, "packed basis overflow");
            vertex[slot..slot + 4].copy_from_slice(&(alpha << 24 | packed).to_le_bytes());
        }
    }
    let mut offset = 8 + size;
    let mut indices = false;
    while offset < bytes.len() {
        let id = u32_at(offset)?;
        let length = u32_at(offset + 4)?;
        let end = offset + 8 + length;
        ensure!(end <= bytes.len(), "truncated chunk {id}");
        match id {
            4 => {
                ensure!(!indices, "duplicate index chunk");
                indices = true;
                result.extend_from_slice(&bytes[offset..end]);
            }
            110 | 114 => {}
            other => bail!("unknown 0.69d geometry chunk {other}"),
        }
        offset = end;
    }
    ensure!(indices, "geometry has no index chunk");
    Ok(result)
}

/// Keep every second line of a regular-grid static mesh (0.26 layout).
/// Returns `None` when the vertices do not form a full grid.
pub fn decimate_grid(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let s = stride(bytes).context("not a static mesh")?;
    let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let size = u(4);
    let count = u(8);
    let vertices = &bytes[12..8 + size];
    let position = |i: usize| -> [f32; 3] {
        let v = &vertices[i * s..];
        [0, 4, 8].map(|o| f32::from_le_bytes(v[o..o + 4].try_into().unwrap()))
    };
    let key = |x: f32| (x * 1000.0).round() as i64;
    let xs: BTreeSet<i64> = (0..count).map(|i| key(position(i)[0])).collect();
    let zs: BTreeSet<i64> = (0..count).map(|i| key(position(i)[2])).collect();
    if xs.len() * zs.len() != count || xs.len() < 3 || zs.len() < 3 {
        return Ok(None);
    }
    let xi: BTreeMap<i64, usize> = xs.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let zi: BTreeMap<i64, usize> = zs.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let mut grid = vec![usize::MAX; count];
    for i in 0..count {
        let p = position(i);
        let cell = zi[&key(p[2])] * xs.len() + xi[&key(p[0])];
        if grid[cell] != usize::MAX {
            return Ok(None);
        }
        grid[cell] = i;
    }
    // Index chunk and the winding the mesh uses (sign of the XZ cross product).
    let at = 8 + size;
    ensure!(u(at) == 4, "expected index chunk");
    let n = u(at + 8);
    let index = |k: usize| u16::from_le_bytes([bytes[at + 12 + 2 * k], bytes[at + 13 + 2 * k]]) as usize;
    ensure!(n >= 3, "no triangles");
    let winding = {
        let (a, b, c) = (position(index(0)), position(index(1)), position(index(2)));
        ((b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0])).signum()
    };
    let keep = |len: usize| -> Vec<usize> {
        let mut v: Vec<usize> = (0..len).step_by(2).collect();
        if *v.last().unwrap() != len - 1 {
            v.push(len - 1);
        }
        v
    };
    let (kx, kz) = (keep(xs.len()), keep(zs.len()));
    let mut remap = BTreeMap::new();
    let mut out_vertices = Vec::new();
    for &z in &kz {
        for &x in &kx {
            let old = grid[z * xs.len() + x];
            remap.insert(old, remap.len());
            out_vertices.extend_from_slice(&vertices[old * s..(old + 1) * s]);
        }
    }
    let mut out_indices: Vec<u16> = Vec::new();
    for zw in kz.windows(2) {
        for xw in kx.windows(2) {
            let corner = |x: usize, z: usize| grid[z * xs.len() + x];
            let (a, b, c, d) = (corner(xw[0], zw[0]), corner(xw[1], zw[0]), corner(xw[1], zw[1]), corner(xw[0], zw[1]));
            for tri in [[a, b, c], [a, c, d]] {
                let (p, q, r) = (position(tri[0]), position(tri[1]), position(tri[2]));
                let sign = ((q[0] - p[0]) * (r[2] - p[2]) - (q[2] - p[2]) * (r[0] - p[0])).signum();
                let tri = if sign == winding { tri } else { [tri[0], tri[2], tri[1]] };
                for v in tri {
                    out_indices.push(u16::try_from(remap[&v])?);
                }
            }
        }
    }
    let mut out = Vec::new();
    out.extend(3u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_vertices.len())?.to_le_bytes());
    out.extend(u32::try_from(remap.len())?.to_le_bytes());
    out.extend(out_vertices);
    out.extend(4u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_indices.len() * 2)?.to_le_bytes());
    out.extend(u32::try_from(out_indices.len())?.to_le_bytes());
    for i in out_indices {
        out.extend(i.to_le_bytes());
    }
    Ok(Some(out))
}

/// Simplify a converted static mesh (0.26 layout) to about `ratio` of its
/// triangles within `max_error` of its extent; unused vertices are dropped.
/// `None` when it would not shed at least a tenth of the triangles.
pub fn simplify_mesh(bytes: &[u8], ratio: f32, max_error: f32) -> Result<Option<Vec<u8>>> {
    simplify_mesh_with(bytes, ratio, max_error, meshopt::SimplifyOptions::None)
}

/// `simplify_mesh` with meshoptimizer options (`LockBorder` keeps open
/// edges, so adjacent pieces such as terrain tiles stay closed).
pub fn simplify_mesh_with(bytes: &[u8], ratio: f32, max_error: f32, options: meshopt::SimplifyOptions) -> Result<Option<Vec<u8>>> {
    let s = stride(bytes).context("not a static mesh")?;
    let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let size = u(4);
    let count = u(8);
    let vertices = &bytes[12..8 + size];
    let at = 8 + size;
    ensure!(u(at) == 4, "expected index chunk");
    let n = u(at + 8);
    let indices: Vec<u32> = (0..n)
        .map(|k| u16::from_le_bytes([bytes[at + 12 + 2 * k], bytes[at + 13 + 2 * k]]) as u32)
        .collect();
    ensure!(indices.iter().all(|&i| (i as usize) < count), "index outside vertices");
    let adapter = meshopt::VertexDataAdapter::new(vertices, s, 0).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let target = ((n as f32 * ratio) as usize / 3 * 3).max(3);
    let simplified = meshopt::simplify(&indices, &adapter, target, max_error, options, None);
    if simplified.len() * 10 > n * 9 || simplified.is_empty() {
        return Ok(None);
    }
    let mut remap = BTreeMap::new();
    let mut out_vertices = Vec::new();
    let mut out_indices = Vec::with_capacity(simplified.len());
    for i in simplified {
        let next = remap.len();
        let j = *remap.entry(i).or_insert_with(|| {
            out_vertices.extend_from_slice(&vertices[i as usize * s..(i as usize + 1) * s]);
            next
        });
        out_indices.push(u16::try_from(j)?);
    }
    let mut out = Vec::new();
    out.extend(3u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_vertices.len())?.to_le_bytes());
    out.extend(u32::try_from(remap.len())?.to_le_bytes());
    out.extend(out_vertices);
    out.extend(4u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_indices.len() * 2)?.to_le_bytes());
    out.extend(u32::try_from(out_indices.len())?.to_le_bytes());
    for i in out_indices {
        out.extend(i.to_le_bytes());
    }
    Ok(Some(out))
}

fn triangles(bytes: &[u8]) -> usize {
    let u = |o: usize| bytes.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    u(8 + u(4) + 8) / 3
}

/// LOD index of each surface (the lowest LOD naming it).
fn surface_lods(settings: &ConfigValue) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for (index, lod) in ["LOD0", "LOD1", "LOD2"].iter().enumerate() {
        if let Some(ConfigValue::Indexed(surfaces)) = settings.named("lod_hierrarchy").and_then(|h| h.named(lod)).and_then(|l| l.named("surfaces")) {
            for surface in surfaces {
                if let ConfigValue::String(name) = surface {
                    let e = result.entry(name.clone()).or_insert(index);
                    *e = (*e).min(index);
                }
            }
        }
    }
    result
}

/// Euler rotation of scene instances (as `audit_collision_region.rotation`;
/// row vectors: world = (v * scale) * R + position).
fn instance_rotation(a: [f32; 3]) -> [[f32; 3]; 3] {
    let (sx, cx) = a[0].sin_cos();
    let (sy, cy) = a[1].sin_cos();
    let (sz, cz) = a[2].sin_cos();
    [
        [cy * cz, -cy * sz, sy],
        [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy],
        [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy],
    ]
}

#[derive(Clone, Copy)]
pub struct Placement {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Placement {
    fn apply(&self, r: &[[f32; 3]; 3], v: [f32; 3], scale: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|j| (0..3).map(|i| v[i] * scale[i] * r[i][j]).sum())
    }
}

fn unpack_basis(word: u32) -> [f32; 3] {
    [(word >> 16) & 255, (word >> 8) & 255, word & 255].map(|c| c as f32 / 127.5 - 1.0)
}

fn pack_basis(v: [f32; 3], alpha: u32) -> u32 {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    let c = |x: f32| (((x / length + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
    alpha << 24 | c(v[0]) << 16 | c(v[1]) << 8 | c(v[2])
}

/// Bake several placements of one static mesh (0.26 layout) into one mesh.
/// Returns the mesh and its world bounds.
pub fn bake_instances(bytes: &[u8], placements: &[Placement]) -> Result<(Vec<u8>, [f32; 3], [f32; 3])> {
    let s = stride(bytes).context("not a static mesh")?;
    let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let size = u(4);
    let count = u(8);
    let vertices = &bytes[12..8 + size];
    let at = 8 + size;
    ensure!(u(at) == 4, "expected index chunk");
    let n = u(at + 8);
    ensure!(count * placements.len() <= 65_536, "batch exceeds 16-bit indices");
    let mut out_vertices = Vec::with_capacity(vertices.len() * placements.len());
    let mut out_indices: Vec<u16> = Vec::with_capacity(n * placements.len());
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (k, p) in placements.iter().enumerate() {
        let r = instance_rotation(p.rotation);
        let inverse: [f32; 3] = p.scale.map(|x| if x.abs() > 1e-9 { 1.0 / x } else { 0.0 });
        let flip = p.scale[0] * p.scale[1] * p.scale[2] < 0.0;
        for v in vertices.chunks_exact(s) {
            let mut v = v.to_vec();
            let position: [f32; 3] = [0, 4, 8].map(|o| f32::from_le_bytes(v[o..o + 4].try_into().unwrap()));
            let world = p.apply(&r, position, p.scale);
            let world: [f32; 3] = std::array::from_fn(|i| world[i] + p.position[i]);
            for i in 0..3 {
                lo[i] = lo[i].min(world[i]);
                hi[i] = hi[i].max(world[i]);
                v[i * 4..i * 4 + 4].copy_from_slice(&world[i].to_le_bytes());
            }
            for (slot, normal) in [(12, true), (16, false), (20, false)] {
                let word = u32::from_le_bytes(v[slot..slot + 4].try_into().unwrap());
                let rotated = p.apply(&r, unpack_basis(word), if normal { inverse } else { p.scale });
                v[slot..slot + 4].copy_from_slice(&pack_basis(rotated, word >> 24).to_le_bytes());
            }
            out_vertices.extend(v);
        }
        let base = (k * count) as u32;
        for t in 0..n / 3 {
            let tri: [u32; 3] = std::array::from_fn(|i| u16::from_le_bytes([bytes[at + 12 + 2 * (3 * t + i)], bytes[at + 13 + 2 * (3 * t + i)]]) as u32);
            let tri = if flip { [tri[0], tri[2], tri[1]] } else { tri };
            for i in tri {
                out_indices.push(u16::try_from(base + i)?);
            }
        }
    }
    let mut out = Vec::new();
    out.extend(3u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_vertices.len())?.to_le_bytes());
    out.extend(u32::try_from(count * placements.len())?.to_le_bytes());
    out.extend(out_vertices);
    out.extend(4u32.to_le_bytes());
    out.extend(u32::try_from(4 + out_indices.len() * 2)?.to_le_bytes());
    out.extend(u32::try_from(out_indices.len())?.to_le_bytes());
    for i in out_indices {
        out.extend(i.to_le_bytes());
    }
    Ok((out, lo, hi))
}

fn set_bounds(properties: &mut ConfigValue, lo: [f32; 3], hi: [f32; 3]) -> Result<()> {
    properties.set_path("bounding_box", named(vec![("min", ConfigValue::Float3(lo)), ("max", ConfigValue::Float3(hi))]))?;
    if properties.named("bounding_sphere").is_some() {
        let center: [f32; 3] = std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0);
        let radius = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f32>().sqrt() / 2.0;
        properties.set_path("bounding_sphere", named(vec![("center", ConfigValue::Float3(center)), ("radius", ConfigValue::Float(radius))]))?;
    }
    Ok(())
}

/// Vertex stride of a converted static model, if it is one.
fn stride(bytes: &[u8]) -> Option<usize> {
    let u = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as usize);
    (u(0)? == 3).then_some(())?;
    let count = u(8)?;
    let size = u(4)?;
    (count > 0 && size >= 4).then(|| (size - 4) / count)
}

/// Bounding box of render geometry from its `export_properties`.
fn bounding_box(properties: &ConfigValue) -> Option<([f32; 3], [f32; 3])> {
    let b = properties.named("bounding_box")?;
    match (b.named("min")?, b.named("max")?) {
        (ConfigValue::Float3(lo), ConfigValue::Float3(hi)) => Some((*lo, *hi)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Textures
// ---------------------------------------------------------------------------

fn dds_info(bytes: &[u8]) -> Result<(u32, u32, u32, [u8; 4])> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS");
    let get = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let mut fourcc = [0; 4];
    fourcc.copy_from_slice(&bytes[84..88]);
    Ok((get(16), get(12), get(28).max(1), fourcc))
}

/// Drop the top mip of a 2D BC1/BC3 texture (halving it), keeping the
/// trailing byte the archives append after the pixel data.
fn drop_top_mip(bytes: &[u8]) -> Result<Vec<u8>> {
    let (width, height, mips, fourcc) = dds_info(bytes)?;
    let block = match &fourcc {
        b"DXT1" => 8,
        b"DXT5" => 16,
        _ => bail!("cannot halve {:?}", String::from_utf8_lossy(&fourcc)),
    };
    let caps2 = u32::from_le_bytes(bytes[112..116].try_into().unwrap());
    ensure!(caps2 & 0xfe00 == 0, "cannot halve a cubemap");
    ensure!(mips >= 2 && width >= 8 && height >= 8, "texture too small to halve");
    let level = |w: u32, h: u32| ((w / 4).max(1) * (h / 4).max(1) * block) as usize;
    let mut total = 0;
    for mip in 0..mips {
        total += level(width >> mip, height >> mip);
    }
    let trailer = bytes.len() - 128 - total;
    ensure!(trailer <= 1, "unexpected DDS length");
    let top = level(width, height);
    let mut result = bytes[..128].to_vec();
    let set = |r: &mut Vec<u8>, o: usize, v: u32| r[o..o + 4].copy_from_slice(&v.to_le_bytes());
    set(&mut result, 12, height / 2);
    set(&mut result, 16, width / 2);
    set(&mut result, 20, level(width / 2, height / 2) as u32);
    set(&mut result, 28, mips - 1);
    result.extend_from_slice(&bytes[128 + top..]);
    Ok(result)
}

// ---------------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------------

/// How a donor material is drawn in 0.26.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Opaque,
    OpaqueTwoSided,
    VertexColour,
    VertexBlend,
    AlphaTest,
    AlphaTestTwoSided,
    Foliage,
    Grass,
    /// g-stage glass (order-independent transparency in 0.69d), cut out.
    GlassCutout,
    /// Forward-lit glass: a native glass material, used as it is.
    Glass,
    /// Forward water: a native water material, used as it is.
    Water,
    /// Mesh surface multiplying the albedo below it.
    Decal,
    /// Projected `decal` scene object.
    ProjectedDecal,
    Terrain,
}

impl Category {
    fn key(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::OpaqueTwoSided => "opaque_two_sided",
            Self::VertexColour => "vertex_colour",
            Self::VertexBlend => "vertex_blend",
            Self::AlphaTest => "alpha_test",
            Self::AlphaTestTwoSided => "alpha_test_two_sided",
            Self::Foliage => "foliage",
            Self::Grass => "grass",
            Self::GlassCutout => "glass_cutout",
            Self::Glass => "glass",
            Self::Water => "water",
            Self::Decal => "decal",
            Self::ProjectedDecal => "projected_decal",
            Self::Terrain => "terrain",
        }
    }
}

/// How a material is used by the meshes that reference it.
#[derive(Clone, Copy, Default, Debug)]
pub struct Usage {
    pub vertex_colour: bool,
    pub vegetation: bool,
}

fn stages(root: &ConfigValue) -> Vec<String> {
    match root
        .named("material")
        .and_then(|m| m.named("common"))
        .and_then(|c| c.named("stages"))
    {
        Some(ConfigValue::Named(v)) => v.iter().map(|s| s.name.clone()).collect(),
        _ => Vec::new(),
    }
}

fn g_stage(root: &ConfigValue) -> Option<&ConfigValue> {
    root.named("material")?.named("g_stage")
}

pub fn categorize(root: &ConfigValue, usage: Usage) -> Result<Category> {
    let stages = stages(root);
    if stages.iter().any(|s| s == "decals") {
        return Ok(Category::ProjectedDecal);
    }
    if stages.iter().any(|s| s.contains("decal")) {
        return Ok(Category::Decal);
    }
    if stages.iter().any(|s| s == "forward_water") {
        return Ok(Category::Water);
    }
    if g_stage(root).is_none() && stages.iter().any(|s| s == "forward") {
        return Ok(Category::Glass);
    }
    let stage = g_stage(root).context("material has neither g_stage nor decal stage")?;
    let effect = stage.named("effect").context("g_stage has no effect")?;
    let effect_id = text(effect, "effect_id").unwrap_or("");
    if effect_id == "g_stage_terrain_layered" {
        return Ok(Category::Terrain);
    }
    ensure!(
        effect_id == "g_stage_default",
        "unsupported 0.69d effect {effect_id}"
    );
    let two_sided = flag(stage, "two_sided") || flag(effect, "two_sided");
    let alpha = flag(effect, "use_alpha_test");
    let base = root
        .named("material")
        .and_then(|m| m.named("instance"))
        .and_then(|i| string(i, "base_material"))
        .unwrap_or("");
    if usage.vegetation {
        return Ok(Category::Grass);
    }
    if flag(effect, "use_olta") {
        return Ok(Category::GlassCutout);
    }
    if alpha && (matches!(base, "flora" | "grass") || flag(effect, "use_t_translucency")) {
        return Ok(Category::Foliage);
    }
    if alpha {
        return Ok(if two_sided { Category::AlphaTestTwoSided } else { Category::AlphaTest });
    }
    if usage.vertex_colour {
        let blends = text(effect, "texture_vertex_blended_diffuse").is_some()
            && (flag(effect, "use_tiled_vertex_blended_mask")
                || flag(effect, "use_vertex_blended_normal"));
        return Ok(if blends { Category::VertexBlend } else { Category::VertexColour });
    }
    Ok(if two_sided { Category::OpaqueTwoSided } else { Category::Opaque })
}

/// Texture names a donor material binds that the converted material uses.
pub fn donor_textures(root: &ConfigValue) -> Vec<String> {
    let mut result = Vec::new();
    let mut visit = |effect: &ConfigValue, names: &[&str]| {
        for name in names {
            if let Some(t) = text(effect, name) {
                result.push(t.replace('\\', "/"));
            }
        }
    };
    if let Some(material) = root.named("material") {
        if let ConfigValue::Named(stages) = material {
            for stage in stages {
                if let Some(effect) = stage.value.named("effect") {
                    visit(
                        effect,
                        &[
                            "t_albedo",
                            "t_normal",
                            "t_smoothness",
                            "t_translucency",
                            "texture_vertex_blended_diffuse",
                            "texture_vertex_blended_normal",
                            "texture_vertex_blended_tiled_mask",
                        ],
                    );
                    if text(effect, "effect_id") == Some("g_stage_terrain_layered") {
                        for layer in 0..5 {
                            visit(
                                effect,
                                &[
                                    &format!("layer_{layer}_texture_albedo"),
                                    &format!("layer_{layer}_texture_normal"),
                                ],
                            );
                        }
                    }
                }
            }
        }
    }
    result.sort();
    result.dedup();
    result
}

fn set_effect(root: &mut ConfigValue, stage: &str, name: &str, value: ConfigValue) -> Result<()> {
    let path = format!("material.{stage}.effect.{name}");
    let field = root
        .named("material")
        .and_then(|m| m.named(stage))
        .and_then(|s| s.named("effect"))
        .and_then(|e| e.named(name))
        .with_context(|| format!("template has no {stage}.{name}"))?;
    // Material fields are {value, is_model_parameter} tables; keep the shape.
    if field.named("value").is_some() {
        root.set_path(&format!("{path}.value"), value)
    } else {
        root.set_path(&path, value)
    }
}

fn effect_text(root: &ConfigValue, stage: &str, name: &str) -> Option<String> {
    let effect = root.named("material")?.named(stage)?.named("effect")?;
    text(effect, name).map(str::to_owned)
}

fn effect_flag(root: &ConfigValue, stage: &str, name: &str) -> bool {
    root.named("material")
        .and_then(|m| m.named(stage))
        .and_then(|s| s.named("effect"))
        .is_some_and(|e| flag(e, name))
}

pub struct ConvertedMaterial {
    pub root: ConfigValue,
    pub template: String,
    pub category: Category,
    pub notes: Vec<String>,
}

/// Convert one 0.69d material onto its category's native 0.26 template.
///
/// Only texture names and constants whose meaning is the same in both builds
/// change; every switch stays the template's, so the shader permutation is a
/// shipped one. `default_normal` stands in where the donor binds no normal
/// map but the template samples one.
pub fn convert_material(
    donor: &ConfigValue,
    usage: Usage,
    templates: &BTreeMap<String, (String, ConfigValue)>,
    roughness_scale: f32,
    default_normal: &str,
) -> Result<ConvertedMaterial> {
    let category = categorize(donor, usage)?;
    // Vertex-colour meshes need a template compiled for that vertex input.
    let vc_key = format!("{}_vc", category.key());
    let (template_path, template) = (if usage.vertex_colour { templates.get(&vc_key) } else { None })
        .or_else(|| templates.get(category.key()))
        .with_context(|| format!("no template for category {}", category.key()))?;
    if usage.vertex_colour {
        ensure!(
            matches!(
                template.named("material").and_then(|m| m.named("g_stage")).and_then(|g| g.named("with_static_vertex_color_meshes")).map(|v| v.named("value").unwrap_or(v)),
                Some(ConfigValue::Boolean(true))
            ) || !matches!(category, Category::Opaque | Category::OpaqueTwoSided | Category::VertexColour | Category::VertexBlend | Category::AlphaTest | Category::AlphaTestTwoSided | Category::Foliage | Category::GlassCutout),
            "template {template_path} for vertex-colour {} meshes is not compiled for them; declare {vc_key}",
            category.key()
        );
    }
    let mut root = template.clone();
    let mut notes = Vec::new();
    match category {
        Category::Decal | Category::ProjectedDecal => {
            let stage = stages(donor)
                .into_iter()
                .find(|s| s.contains("decal"))
                .unwrap();
            let effect = donor
                .named("material")
                .and_then(|m| m.named(&stage))
                .and_then(|s| s.named("effect"))
                .context("decal has no effect")?;
            let albedo = text(effect, "t_albedo")
                .or_else(|| text(effect, "texture_diffuse"))
                .context("decal binds no albedo")?;
            if category == Category::Decal {
                // The native multiply decal draws its texture as emissive
                // through a multiply blend.
                set_effect(&mut root, "forward", "texture_emissive", ConfigValue::String(albedo.into()))?;
            } else {
                set_effect(&mut root, "decals", "texture_diffuse", ConfigValue::String(albedo.into()))?;
                let normal = text(effect, "t_normal")
                    .filter(|_| flag(effect, "use_t_normal"))
                    .unwrap_or(default_normal);
                set_effect(&mut root, "decals", "texture_normal", ConfigValue::String(normal.into()))?;
                if let Some(ConfigValue::Float4(c)) = value_of(effect, "albedo_color") {
                    set_effect(&mut root, "decals", "constant_diffuse", ConfigValue::Float4(*c))?;
                }
            }
        }
        Category::Terrain => bail!("terrain materials are converted per layer"),
        Category::Glass | Category::Water => {
            notes.push(format!("drawn with native {template_path} as it is (0.69d forward shading has no 0.26 counterpart)"));
        }
        _ => {
            let effect = g_stage(donor)
                .and_then(|s| s.named("effect"))
                .context("material has no g_stage effect")?;
            let albedo = text(effect, "t_albedo").context("material binds no albedo")?;
            let albedo = albedo.to_owned();
            let normal = text(effect, "t_normal")
                .filter(|_| flag(effect, "use_t_normal"))
                .map(str::to_owned);
            let stage = "g_stage";
            set_effect(&mut root, stage, "texture_diffuse", ConfigValue::String(albedo.clone()))?;
            if effect_flag(&root, stage, "use_nmap") {
                let n = normal.clone().unwrap_or_else(|| default_normal.to_owned());
                set_effect(&mut root, stage, "texture_normal", ConfigValue::String(n.clone()))?;
                // Unused unless a template switch enables them; keep them on
                // this material's own maps rather than another material's.
                for field in ["texture_detail_normal", "texture_alphablended_normal"] {
                    if effect_text(&root, stage, field).is_some() {
                        set_effect(&mut root, stage, field, ConfigValue::String(n.clone()))?;
                    }
                }
            }
            let smoothness_map = text(effect, "t_smoothness")
                .filter(|_| flag(effect, "use_t_smoothness"))
                .map(str::to_owned);
            let max = number(effect, "max_smoothness").unwrap_or(1.0);
            let min = number(effect, "min_smoothness").unwrap_or(0.0);
            let constant = number(effect, "smoothness").unwrap_or(0.5);
            if effect_flag(&root, stage, "use_troughness") {
                match &smoothness_map {
                    Some(map) => {
                        set_effect(&mut root, stage, "texture_roughness", ConfigValue::String(map.clone()))?;
                        set_effect(&mut root, stage, "constant_roughness_min", ConfigValue::Float(min * roughness_scale))?;
                        set_effect(&mut root, stage, "constant_roughness_max", ConfigValue::Float(max * roughness_scale))?;
                    }
                    None => {
                        // A flat channel: min == max makes the tap irrelevant.
                        let v = constant * roughness_scale;
                        set_effect(&mut root, stage, "texture_roughness", ConfigValue::String(albedo.clone()))?;
                        set_effect(&mut root, stage, "constant_roughness_min", ConfigValue::Float(v))?;
                        set_effect(&mut root, stage, "constant_roughness_max", ConfigValue::Float(v))?;
                    }
                }
            } else if root
                .named("material")
                .and_then(|m| m.named(stage))
                .and_then(|s| s.named("effect"))
                .and_then(|e| e.named("constant_roughness"))
                .is_some()
            {
                set_effect(&mut root, stage, "constant_roughness", ConfigValue::Float(constant * roughness_scale))?;
            }
            if effect_flag(&root, stage, "use_tspecular_intensity") {
                let map = smoothness_map.clone().unwrap_or_else(|| albedo.clone());
                set_effect(&mut root, stage, "texture_specular_intensity", ConfigValue::String(map))?;
            }
            if effect_flag(&root, stage, "use_ttranslucency") {
                let map = text(effect, "t_translucency")
                    .filter(|_| flag(effect, "use_t_translucency"))
                    .map(str::to_owned)
                    .unwrap_or_else(|| albedo.clone());
                set_effect(&mut root, stage, "texture_translucency", ConfigValue::String(map))?;
            }
            if effect_flag(&root, stage, "use_alpha_test") {
                if let Some(r) = number(effect, "alpha_ref") {
                    set_effect(&mut root, stage, "alpha_ref", ConfigValue::Float(r))?;
                }
            }
            if matches!(category, Category::VertexBlend | Category::VertexColour) {
                let blend_diffuse = text(effect, "texture_vertex_blended_diffuse")
                    .filter(|_| category == Category::VertexBlend)
                    .map(str::to_owned)
                    .unwrap_or_else(|| albedo.clone());
                let blend_normal = text(effect, "texture_vertex_blended_normal")
                    .filter(|_| category == Category::VertexBlend)
                    .map(str::to_owned)
                    .or_else(|| normal.clone())
                    .unwrap_or_else(|| default_normal.to_owned());
                for (field, value) in [
                    ("texture_vertex_blended_diffuse", blend_diffuse),
                    ("texture_vertex_blended_normal", blend_normal),
                ] {
                    if effect_text(&root, stage, field).is_some() {
                        set_effect(&mut root, stage, field, ConfigValue::String(value))?;
                    }
                }
                if effect_text(&root, stage, "texture_vertex_blended_tiled_mask").is_some() {
                    let mask = text(effect, "texture_vertex_blended_tiled_mask")
                        .filter(|_| category == Category::VertexBlend)
                        .map(str::to_owned)
                        .unwrap_or_else(|| albedo.clone());
                    set_effect(&mut root, stage, "texture_vertex_blended_tiled_mask", ConfigValue::String(mask))?;
                }
            }
            if let Some(ConfigValue::Float4(c)) = value_of(effect, "albedo_color") {
                if root
                    .named("material")
                    .and_then(|m| m.named(stage))
                    .and_then(|s| s.named("effect"))
                    .and_then(|e| e.named("constant_diffuse"))
                    .is_some()
                {
                    set_effect(&mut root, stage, "constant_diffuse", ConfigValue::Float4(*c))?;
                }
            }
            if flag(effect, "use_t_metalness") {
                notes.push("metalness map has no 0.26 counterpart; the albedo is drawn as a dielectric".into());
            }
            for (switch, what) in [
                ("use_parallax", "parallax"),
                ("use_displacement", "displacement"),
                ("use_fuzziness", "fuzziness"),
                ("use_t_normal_detail", "detail normal"),
                ("use_t_albedo_detail", "detail albedo"),
                ("use_subsurface_scattering", "subsurface scattering"),
                ("use_flicker", "flicker"),
                ("use_emission_flicker", "emission flicker"),
                ("use_uv_sequence", "UV sequence"),
            ] {
                if flag(effect, switch) {
                    notes.push(format!("{what} is not reproduced (template switch kept)"));
                }
            }
        }
    }
    Ok(ConvertedMaterial {
        root,
        template: template_path.clone(),
        category,
        notes,
    })
}

/// Every texture-valued effect field of a converted material.
fn bound_textures(root: &ConfigValue) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(ConfigValue::Named(stages)) = root.named("material") {
        for stage in stages {
            if let Some(ConfigValue::Named(fields)) = stage.value.named("effect") {
                for field in fields {
                    if field.name.starts_with("texture_") || field.name.ends_with("_texture") {
                        if let Some(ConfigValue::String(s)) =
                            Some(field.value.named("value").unwrap_or(&field.value))
                        {
                            if !s.is_empty() {
                                result.push(s.replace('\\', "/"));
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

/// Template fields naming textures that ship in neither build (native
/// templates keep such names in fields their switches leave unused) are
/// pointed at the material's own diffuse, so every bound name resolves.
fn fill_missing_textures(root: &mut ConfigValue, exists: &dyn Fn(&str) -> bool, cube: &str) -> Result<Vec<String>> {
    let mut fallback = None;
    if let Some(ConfigValue::Named(stages)) = root.named("material") {
        for stage in stages {
            if let Some(effect) = stage.value.named("effect") {
                for field in ["texture_diffuse", "texture_emissive"] {
                    if let Some(t) = text(effect, field).filter(|t| exists(t)) {
                        fallback.get_or_insert_with(|| t.to_owned());
                    }
                }
            }
        }
    }
    let fallback = fallback.unwrap_or_else(|| cube.to_owned());
    ensure!(exists(&fallback), "diffuse {fallback} exists in neither build");
    let mut replaced = Vec::new();
    if let Some(ConfigValue::Named(stages)) = root.named_mut("material") {
        for stage in stages {
            if let Some(ConfigValue::Named(fields)) = stage.value.named_mut("effect") {
                for field in fields {
                    if !(field.name.starts_with("texture_") || field.name.ends_with("_texture")) {
                        continue;
                    }
                    let slot = if matches!(field.value, ConfigValue::Named(_)) {
                        field.value.named_mut("value")
                    } else {
                        Some(&mut field.value)
                    };
                    if let Some(ConfigValue::String(t)) = slot {
                        if !t.is_empty() && !exists(&t.replace('\\', "/")) {
                            replaced.push(format!("{}={t}", field.name));
                            *t = if field.name.contains("reflection") || field.name.contains("cubemap") {
                                cube.to_owned()
                            } else {
                                fallback.clone()
                            };
                        }
                    }
                }
            }
        }
    }
    Ok(replaced)
}

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// Decode one BC1/BC3 texel block row-major into RGBA (top mip only).
fn decode_bc(bytes: &[u8]) -> Result<(usize, usize, Vec<[u8; 4]>)> {
    let (width, height, _, fourcc) = dds_info(bytes)?;
    let (w, h) = (width as usize, height as usize);
    let block = match &fourcc {
        b"DXT1" => 8,
        b"DXT5" => 16,
        _ => bail!("cannot decode {:?}", String::from_utf8_lossy(&fourcc)),
    };
    ensure!(w % 4 == 0 && h % 4 == 0, "odd texture size");
    ensure!(bytes.len() >= 128 + (w / 4) * (h / 4) * block, "truncated texture");
    let mut out = vec![[0u8; 4]; w * h];
    let c565 = |c: u16| {
        let r = ((c >> 11) & 31) as u32;
        let g = ((c >> 5) & 63) as u32;
        let b = (c & 31) as u32;
        [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
    };
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            let o = 128 + (by * (w / 4) + bx) * block;
            let mut alpha = [255u8; 16];
            let co = if block == 16 {
                let a0 = bytes[o] as u32;
                let a1 = bytes[o + 1] as u32;
                let bits = u64::from_le_bytes([
                    bytes[o + 2], bytes[o + 3], bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7], 0, 0,
                ]);
                let palette: [u32; 8] = if a0 > a1 {
                    [a0, a1, (6 * a0 + a1) / 7, (5 * a0 + 2 * a1) / 7, (4 * a0 + 3 * a1) / 7, (3 * a0 + 4 * a1) / 7, (2 * a0 + 5 * a1) / 7, (a0 + 6 * a1) / 7]
                } else {
                    [a0, a1, (4 * a0 + a1) / 5, (3 * a0 + 2 * a1) / 5, (2 * a0 + 3 * a1) / 5, (a0 + 4 * a1) / 5, 0, 255]
                };
                for (i, a) in alpha.iter_mut().enumerate() {
                    *a = palette[((bits >> (3 * i)) & 7) as usize] as u8;
                }
                o + 8
            } else {
                o
            };
            let c0 = u16::from_le_bytes([bytes[co], bytes[co + 1]]);
            let c1 = u16::from_le_bytes([bytes[co + 2], bytes[co + 3]]);
            let bits = u32::from_le_bytes(bytes[co + 4..co + 8].try_into().unwrap());
            let (p0, p1) = (c565(c0), c565(c1));
            let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
            let palette: [[u8; 3]; 4] = if c0 > c1 || block == 16 {
                [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 2, 1)), std::array::from_fn(|i| mix(p0[i], p1[i], 1, 2))]
            } else {
                [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 1, 1)), [0, 0, 0]]
            };
            for i in 0..16 {
                let p = palette[((bits >> (2 * i)) & 3) as usize];
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                out[y * w + x] = [p[0], p[1], p[2], alpha[i]];
            }
        }
    }
    Ok((w, h, out))
}

/// Layer weights of the 0.69d layered terrain at a mask texel: channels
/// R, G, B, A weigh layers 1..4 over the base layer 0.
fn layer_weights(texel: [u8; 4]) -> [f32; 5] {
    let mut w = [0.0f32; 5];
    let mut remaining = 1.0f32;
    // Later layers paint over earlier ones.
    for layer in (1..5).rev() {
        let a = texel[layer - 1] as f32 / 255.0;
        w[layer] = remaining * a;
        remaining *= 1.0 - a;
    }
    w[0] = remaining;
    w
}

/// Dominant terrain layer of a terrain mesh, sampling the blend mask at every
/// vertex (mask UV = mesh UV / tile factor).
fn dominant_layer(
    geometry: &[u8],
    mask: &(usize, usize, Vec<[u8; 4]>),
    tile: f32,
    invert_v: bool,
) -> Result<(usize, [f32; 5])> {
    let s = stride(geometry).context("terrain is not static geometry")?;
    let count = u32::from_le_bytes(geometry[8..12].try_into().unwrap()) as usize;
    let mut total = [0.0f32; 5];
    let (w, h, texels) = mask;
    for v in geometry[12..12 + count * s].chunks_exact(s) {
        let uv = [24, 26].map(|o| half_to_f32(u16::from_le_bytes([v[o], v[o + 1]])));
        let mut u = uv[0] / tile;
        let mut t = uv[1] / tile;
        if invert_v {
            t = 1.0 - t;
        }
        u = u.clamp(0.0, 0.999_999);
        t = t.clamp(0.0, 0.999_999);
        let texel = texels[(t * *h as f32) as usize * w + (u * *w as f32) as usize];
        for (acc, x) in total.iter_mut().zip(layer_weights(texel)) {
            *acc += x;
        }
    }
    let n = count.max(1) as f32;
    let share = total.map(|x| x / n);
    let best = (0..5)
        .max_by(|a, b| share[*a].total_cmp(&share[*b]))
        .unwrap();
    Ok((best, share))
}

/// Add a vertex colour to a static mesh whose alpha carries the weight of
/// the `base` layer against `blended` (0.26 colored-static vertex blending
/// draws the blend map with weight 1 - occlusion, occlusion being the
/// colour's fourth component). RGB stays white.
fn blend_colours(geometry: &[u8], mask: &(usize, usize, Vec<[u8; 4]>), tile: f32, base: usize, blended: usize) -> Result<Vec<u8>> {
    let s = stride(geometry).context("terrain is not static geometry")?;
    ensure!(s == 28, "terrain already has vertex colours");
    let u = |o: usize| u32::from_le_bytes(geometry[o..o + 4].try_into().unwrap()) as usize;
    let (size, count) = (u(4), u(8));
    let (w, h, texels) = mask;
    let mut out = Vec::with_capacity(geometry.len() + count * 4);
    out.extend(3u32.to_le_bytes());
    out.extend(u32::try_from(4 + count * 32)?.to_le_bytes());
    out.extend(u32::try_from(count)?.to_le_bytes());
    for v in geometry[12..8 + size].chunks_exact(28) {
        let uv = [24, 26].map(|o| half_to_f32(u16::from_le_bytes([v[o], v[o + 1]])));
        let (x, y) = ((uv[0] / tile).clamp(0.0, 0.999_999), (uv[1] / tile).clamp(0.0, 0.999_999));
        let weights = layer_weights(texels[(y * *h as f32) as usize * w + (x * *w as f32) as usize]);
        let (a, b) = (weights[base], weights[blended]);
        let occlusion = if a + b > 1e-6 { a / (a + b) } else { 1.0 };
        out.extend_from_slice(v);
        out.extend([255, 255, 255, (occlusion * 255.0).round() as u8]);
    }
    out.extend_from_slice(&geometry[8 + size..]);
    Ok(out)
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((h >> 10) & 31) as i32;
    let mantissa = (h & 1023) as f32;
    if exponent == 0 {
        sign * mantissa * 2f32.powi(-24)
    } else if exponent == 31 {
        f32::NAN
    } else {
        sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
    }
}

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
    ConfigValue::Named(
        fields
            .into_iter()
            .map(|(name, value)| NamedValue {
                name: name.into(),
                value,
            })
            .collect(),
    )
}

fn float3(object: &ConfigValue, field: &str) -> Result<[f32; 3]> {
    match object.named(field) {
        Some(ConfigValue::Float3(v)) if v.iter().all(|x| x.is_finite()) => Ok(*v),
        _ => bail!("object field {field} is not a finite float3"),
    }
}

fn integer(object: &ConfigValue, field: &str) -> Option<i32> {
    match object.named(field)? {
        ConfigValue::Integer(v) => Some(*v),
        ConfigValue::Float(v) => Some(*v as i32),
        _ => None,
    }
}

fn real(object: &ConfigValue, field: &str) -> Option<f32> {
    match object.named(field)? {
        ConfigValue::Integer(v) => Some(*v as f32),
        ConfigValue::Float(v) => Some(*v),
        _ => None,
    }
}

fn kind(object: &ConfigValue) -> &str {
    string(object, "game_object_type").unwrap_or("")
}

/// Native objects of a type in the template project (client and server).
fn templates_of<'a>(template: &'a ConfigValue, wanted: &str) -> Vec<&'a ConfigValue> {
    let mut result = Vec::new();
    if let Some(ConfigValue::Indexed(objects)) = template.named("client_objects") {
        result.extend(objects.iter().filter(|o| kind(o) == wanted));
    }
    if let Some(ConfigValue::Named(groups)) = template.named("server_objects") {
        for group in groups {
            if let ConfigValue::Indexed(objects) = &group.value {
                result.extend(objects.iter().filter(|o| kind(o) == wanted));
            }
        }
    }
    result
}

fn first_template(template: &ConfigValue, wanted: &str) -> Result<ConfigValue> {
    templates_of(template, wanted)
        .first()
        .map(|o| (*o).clone())
        .with_context(|| format!("template project has no {wanted}"))
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeClass {
    pub prefix: String,
    /// `full_name` of a probe in the template project.
    pub native: String,
}

/// Point every weather variant of a probe at one cubemap.
fn set_probe_texture(probe: &mut ConfigValue, texture: &str) -> Result<()> {
    probe.set_path("texture", ConfigValue::String(texture.into()))?;
    if let Some(ConfigValue::Named(variants)) = probe.named_mut("textures") {
        for v in variants {
            v.value = ConfigValue::String(texture.into());
        }
    }
    Ok(())
}

fn probe_texture(probe: &ConfigValue) -> Option<String> {
    probe
        .named("textures")
        .and_then(|t| string(t, "day_4"))
        .or_else(|| string(probe, "texture"))
        .map(str::to_owned)
}

/// Copy the transform and naming of a donor object onto a native clone.
fn place(native: &mut ConfigValue, donor: &ConfigValue, full_name: &str) -> Result<()> {
    for field in ["position", "rotation", "scale"] {
        native.set_path(field, ConfigValue::Float3(float3(donor, field)?))?;
    }
    native.set_path("pivot", ConfigValue::Float3([0.0; 3]))?;
    let name = full_name.rsplit('/').next().unwrap_or(full_name);
    native.set_path("name", ConfigValue::String(name.into()))?;
    native.set_path("full_name", ConfigValue::String(full_name.into()))?;
    if native.named("is_temporary").is_some() {
        native.set_path("is_temporary", ConfigValue::Boolean(false))?;
    }
    Ok(())
}

/// Copy donor values of fields the native object also has, with the same
/// value kind (numbers may change between integer and float encodings).
fn copy_shared(native: &mut ConfigValue, donor: &ConfigValue, fields: &[&str]) -> Result<()> {
    for field in fields {
        let (Some(old), Some(new)) = (native.named(field), donor.named(field)) else {
            continue;
        };
        let value = match (old, new) {
            (ConfigValue::Float(_), ConfigValue::Integer(v)) => ConfigValue::Float(*v as f32),
            (ConfigValue::Integer(_), ConfigValue::Float(v)) if v.fract() == 0.0 => {
                ConfigValue::Integer(*v as i32)
            }
            (ConfigValue::Float(_), ConfigValue::Float(_))
            | (ConfigValue::Integer(_), ConfigValue::Integer(_))
            | (ConfigValue::Boolean(_), ConfigValue::Boolean(_))
            | (ConfigValue::String(_), ConfigValue::String(_))
            | (ConfigValue::Float3(_), ConfigValue::Float3(_))
            | (ConfigValue::Float4(_), ConfigValue::Float4(_)) => new.clone(),
            _ => continue,
        };
        native.set_path(field, value)?;
    }
    Ok(())
}

/// The 0.26 ambient light colour is packed ARGB in an i32.
fn packed_colour(rgb: [f32; 3]) -> i32 {
    let c = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 255;
    (0xff00_0000 | c(rgb[0]) << 16 | c(rgb[1]) << 8 | c(rgb[2])) as i32
}

pub struct SceneReport {
    pub project: ConfigValue,
    /// Donor model names (without `models/`) referenced by render and
    /// collision instances.
    pub models: BTreeSet<String>,
    /// Model names referenced by vegetation layers.
    pub vegetation_models: BTreeSet<String>,
    /// Vegetation instance files, project-relative.
    pub vegetation_files: BTreeSet<String>,
    pub decal_materials: BTreeSet<String>,
    pub counts: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

fn model_of_reference(reference: &str) -> String {
    let normalized = reference.replace('\\', "/");
    let base = normalized.split('#').next().unwrap();
    let base = base.trim_start_matches("resources/");
    if let Some((model, _)) = base.split_once(".model/") {
        model.trim_start_matches("models/").to_owned()
    } else {
        base.trim_start_matches("models/")
            .trim_end_matches(".model")
            .to_owned()
    }
}

/// Build the 0.26 scene from the 0.69d TDM scene and the native template.
pub fn convert_scene(
    donor: &ConfigValue,
    template: &ConfigValue,
    spec: &MapSpec,
    particle_replacements: &BTreeMap<String, String>,
) -> Result<SceneReport> {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut notes = Vec::new();
    let mut models = BTreeSet::new();
    let mut count = |k: &str| *counts.entry(k.to_owned()).or_default() += 1;

    // Render and collision instances.
    let mut visuals = Vec::new();
    for visual in array(donor.named("render_visuals").context("no render_visuals")?)? {
        let model = string(visual, "model_name").context("render visual without model_name")?;
        models.insert(model_of_reference(model));
        visuals.push(named(vec![
            ("lib_name", ConfigValue::String(model.replace('\\', "/"))),
            ("position", ConfigValue::Float3(float3(visual, "position")?)),
            ("rotation", ConfigValue::Float3(float3(visual, "rotation")?)),
            ("scale", ConfigValue::Float3(float3(visual, "scale")?)),
        ]));
        count("render_visual");
    }
    let mut collisions = Vec::new();
    for collision in array(donor.named("collision_objects").context("no collision_objects")?)? {
        let model = string(collision, "model_name").context("collision without model_name")?;
        models.insert(model_of_reference(model));
        collisions.push(named(vec![
            ("lib_name", ConfigValue::String(model.replace('\\', "/"))),
            ("position", ConfigValue::Float3(float3(collision, "position")?)),
            ("rotation", ConfigValue::Float3(float3(collision, "rotation")?)),
            ("scale", ConfigValue::Float3(float3(collision, "scale")?)),
            ("cmask", ConfigValue::Integer(integer(collision, "cmask").context("cmask")?)),
            ("cgroup", ConfigValue::Integer(integer(collision, "cgroup").context("cgroup")?)),
        ]));
        count("collision_object");
    }

    let donor_objects = array(donor.named("client_objects").context("no client_objects")?)?;
    let by_name: BTreeMap<&str, &ConfigValue> = donor_objects
        .iter()
        .filter_map(|o| Some((string(o, "full_name")?, o)))
        .collect();
    let mut objects = Vec::new();
    let mut decal_materials = BTreeSet::new();
    let mut vegetation_models = BTreeSet::new();
    let mut vegetation_files = BTreeSet::new();
    let mut dropped = BTreeMap::<String, usize>::new();

    // Environment: the native one, lit from the donor's sun direction.
    let mut environment = first_template(template, "environment")?;
    let donor_environment = donor
        .named("server_objects")
        .and_then(|s| s.named("logic_objects"))
        .and_then(|l| match l {
            ConfigValue::Indexed(v) => v.iter().find(|o| kind(o) == "environment"),
            _ => None,
        })
        .or_else(|| donor_objects.iter().find(|o| kind(o) == "environment"))
        .context("donor has no environment")?;
    let elevation = real(donor_environment, "sun_elevation").context("sun_elevation")?;
    let azimuth = real(donor_environment, "sun_azimuth").context("sun_azimuth")?;
    environment.set_path("properties.sun_angle", ConfigValue::Float(elevation))?;
    environment.set_path("properties.sun_position_azimut", ConfigValue::Float(azimuth))?;
    notes.push(format!(
        "environment: native {} sky, clouds, fog and post-process; 0.69d sun direction (elevation {elevation}, azimuth {azimuth}); 0.69d exposure, colour grading, light shafts and fog volumes have no 0.26 counterpart",
        spec.template_project
    ));
    objects.push(environment);
    count("environment");

    // One native outdoor probe over the whole map. 0.69d probes are encoded
    // for its exposure pipeline (RGBM-like alpha, SH irradiance) and do not
    // decode in 0.26.
    let mut probe = templates_of(template, "environment_probe")
        .into_iter()
        .filter(|p| integer(p, "geometry") == Some(0))
        .max_by(|a, b| {
            real(a, "radius")
                .unwrap_or(0.0)
                .total_cmp(&real(b, "radius").unwrap_or(0.0))
        })
        .context("template has no sphere probe")?
        .clone();
    probe.set_path("position", ConfigValue::Float3(spec.global_probe_position))?;
    probe.set_path("radius", ConfigValue::Float(spec.global_probe_radius))?;
    probe.set_path("name", ConfigValue::String("global".into()))?;
    probe.set_path(
        "full_name",
        ConfigValue::String("env_and_lights/probes/global".into()),
    )?;
    let global_texture = probe_texture(&probe).context("global probe texture")?;
    set_probe_texture(&mut probe, &global_texture)?;
    objects.push(probe);
    count("environment_probe");
    // Local probes: 0.69d volumes with native cubemaps by folder.
    let mut class_probes = Vec::new();
    for class in &spec.probe_classes {
        let native = templates_of(template, "environment_probe")
            .into_iter()
            .find(|p| string(p, "full_name") == Some(class.native.as_str()))
            .with_context(|| format!("template has no probe {}", class.native))?;
        class_probes.push((class.prefix.clone(), native.clone(), probe_texture(native).context("probe texture")?));
    }
    for object in donor_objects.iter().filter(|o| kind(o) == "environment_probe") {
        let full_name = string(object, "full_name").unwrap_or("");
        if matches!(object.named("is_enabled"), Some(ConfigValue::Boolean(false))) {
            continue;
        }
        let Some((_, native, texture)) = class_probes.iter().find(|(prefix, _, _)| full_name.starts_with(prefix.as_str())) else {
            continue;
        };
        let mut p = native.clone();
        place(&mut p, object, full_name)?;
        set_probe_texture(&mut p, texture)?;
        let scale = float3(object, "scale")?;
        // 0.69d shapes: 0 sphere, 1 box, 2/3 boxes with other falloffs.
        let sphere = integer(object, "geometry") == Some(0);
        p.set_path("geometry", ConfigValue::Integer(if sphere { 0 } else { 1 }))?;
        if sphere {
            p.set_path("radius", ConfigValue::Float(scale.iter().cloned().fold(0.0, f32::max)))?;
            p.set_path("scale", ConfigValue::Float3([1.0; 3]))?;
        }
        copy_shared(&mut p, object, &["probe_side_width"])?;
        objects.push(p);
        count("environment_probe");
    }

    let ambient_template = first_template(template, "ambient_light")?;
    let decal_template = first_template(template, "decal")?;
    let light_template = templates_of(template, "light")
        .into_iter()
        .find(|l| integer(l, "light_type") == Some(1))
        .or_else(|| templates_of(template, "light").into_iter().next())
        .context("template has no light")?
        .clone();
    let resupply_template = first_template(template, "resupply_zone")?;
    let collision_geometry_template = templates_of(template, "collision_geometry")
        .into_iter()
        .find(|c| array(c.named("meshes").unwrap_or(&ConfigValue::Indexed(vec![]))).is_ok_and(|m| m.len() == 1))
        .context("template has no single-mesh collision geometry")?
        .clone();
    let particle_template = templates_of(template, "particle").first().map(|p| (*p).clone());

    let mut kept_collision_geometry = BTreeSet::new();
    for object in donor_objects {
        let t = kind(object);
        let full_name = string(object, "full_name").unwrap_or("").to_owned();
        if matches!(object.named("is_enabled"), Some(ConfigValue::Boolean(false))) {
            *dropped.entry(format!("{t} (disabled)")).or_default() += 1;
            continue;
        }
        match t {
            "ambient_light" if !spec.ambient_lights => {
                *dropped.entry("ambient_light (photometric units and shapes 2/3 have no 0.26 equivalent)".into()).or_default() += 1;
            }
            "environment_probe" => {}
            "ambient_light" => {
                let mut a = ambient_template.clone();
                place(&mut a, object, &full_name)?;
                copy_shared(
                    &mut a,
                    object,
                    &["attenuation_power", "side_width", "affect_specular", "geometry", "smart_attenuation", "dot_normal", "radius", "enabled"],
                )?;
                let colour = float3(object, "color").unwrap_or([1.0; 3]);
                // 0.69d lights in lux (illuminance x multiplier); 0.26 in a
                // unitless intensity on an sRGB colour. The scale makes the
                // map's median ambient light match the template's median.
                let lux = real(object, "illuminance").unwrap_or(1.0)
                    * real(object, "multiplier").unwrap_or(1.0)
                    * real(object, "intensity").unwrap_or(1.0);
                a.set_path("color", ConfigValue::Integer(packed_colour(colour)))?;
                a.set_path("intensity", ConfigValue::Float(lux))?;
                objects.push(a);
                count("ambient_light");
            }
            "decal" => {
                let material = string(object, "decal_material").context("decal material")?;
                let mut d = decal_template.clone();
                place(&mut d, object, &full_name)?;
                copy_shared(
                    &mut d,
                    object,
                    &[
                        "alpha_angle", "clip_angle", "decal_far_distance", "decal_height", "decal_width", "draw_priority",
                        "projection_on_particle_geometry", "projection_on_skeleton_geometry", "projection_on_static_geometry",
                        "projection_on_terrain_geometry",
                    ],
                )?;
                d.set_path("decal_material", ConfigValue::String(material.into()))?;
                decal_materials.insert(material.replace('\\', "/"));
                objects.push(d);
                count("decal");
            }
            "light" => {
                let mut l = light_template.clone();
                place(&mut l, object, &full_name)?;
                copy_shared(
                    &mut l,
                    object,
                    &[
                        "attenuation_power", "color", "diffuse_influence_factor", "specular_influence_factor", "is_cast_shadow",
                        "light_type", "range", "spot_falloff", "spot_penumbra_angle", "spot_umbra_angle", "z_bias",
                        "static_shadows", "lighting_model", "shadow_map_size_index",
                    ],
                )?;
                // 0.69d photometric power (lumen); 0.26 intensity is unitless.
                let power = real(object, "luminous_power").unwrap_or(30.0) * real(object, "intensity").unwrap_or(1.0);
                l.set_path("intensity", ConfigValue::Float((power / 30.0).clamp(0.2, 3.0)))?;
                objects.push(l);
                count("light");
                notes.push(format!("{full_name}: point light intensity {} from luminous power {power}", (power / 30.0).clamp(0.2, 3.0)));
            }
            "resupply_zone" => {
                // Team 2 is neutral in both builds. 0.26 maps only author team
                // zones, so a neutral zone becomes one zone per team.
                let team = integer(object, "team").unwrap_or(2);
                let links: Vec<String> = match object.named("collision_geometries") {
                    Some(ConfigValue::Named(links)) => links
                        .iter()
                        .filter_map(|l| string(&l.value, "resolved_path").map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                };
                ensure!(!links.is_empty(), "{full_name}: resupply zone links no collision");
                for link in &links {
                    ensure!(by_name.contains_key(link.as_str()), "{full_name}: missing {link}");
                    kept_collision_geometry.insert(link.clone());
                }
                let teams: Vec<i32> = if team == 2 { vec![0, 1] } else { vec![team] };
                for (i, team) in teams.iter().enumerate() {
                    let mut r = resupply_template.clone();
                    let name = if i == 0 { full_name.clone() } else { format!("{full_name}_team{team}") };
                    place(&mut r, object, &name)?;
                    copy_shared(&mut r, object, &["resupply_time_in_ms"])?;
                    r.set_path("team", ConfigValue::Integer(*team))?;
                    r.set_path(
                        "collision_geometries",
                        ConfigValue::Indexed(
                            links
                                .iter()
                                .map(|l| named(vec![("name", ConfigValue::String(l.clone()))]))
                                .collect(),
                        ),
                    )?;
                    objects.push(r);
                    count("resupply_zone");
                }
            }
            "vegetation" => {
                let mut v = first_template(template, "vegetation")?;
                place(&mut v, object, &full_name)?;
                let mut layers = Vec::new();
                for layer in array(object.named("layers").context("vegetation layers")?)? {
                    let mut l = layer.clone();
                    if let ConfigValue::Named(fields) = &mut l {
                        // 0.26 reads the same keys; wind_factor is 0.69d-only.
                        fields.retain(|f| f.name != "wind_factor");
                    }
                    for def in array(layer.named("model_def").context("model_def")?)? {
                        vegetation_models.insert(model_of_reference(string(def, "name").context("veg model")?));
                    }
                    vegetation_files.insert(string(layer, "filename").context("veg filename")?.replace('\\', "/"));
                    layers.push(l);
                }
                v.set_path("layers", ConfigValue::Indexed(layers))?;
                objects.push(v);
                count("vegetation");
            }
            "particle" => {
                let name = string(object, "particle_name").or_else(|| string(object, "lib_name")).unwrap_or("");
                match (particle_replacements.get(name), &particle_template) {
                    (Some(native), Some(template)) => {
                        let mut p = template.clone();
                        place(&mut p, object, &full_name)?;
                        p.set_path("lib_name", ConfigValue::String(native.clone()))?;
                        if p.named("path").is_some() {
                            p.set_path("path", ConfigValue::String(native.clone()))?;
                        }
                        objects.push(p);
                        count("particle");
                    }
                    _ => *dropped.entry(format!("particle {name}")).or_default() += 1,
                }
            }
            "collision_geometry" => {} // linked ones are added below
            "environment" => {}
            other => *dropped.entry(other.to_owned()).or_default() += 1,
        }
    }
    for link in &kept_collision_geometry {
        let donor_geometry = by_name[link.as_str()];
        let mut c = collision_geometry_template.clone();
        place(&mut c, donor_geometry, link)?;
        copy_shared(&mut c, donor_geometry, &["collision_filter_group", "collision_filter_mask", "filter_group", "filter_mask", "mode"])?;
        let meshes = array(donor_geometry.named("meshes").context("collision meshes")?)?;
        let native_mesh = array(c.named("meshes").unwrap())?[0].clone();
        let mut out = Vec::new();
        for mesh in meshes {
            let mut m = native_mesh.clone();
            for field in ["position", "rotation", "scale"] {
                m.set_path(field, ConfigValue::Float3(float3(mesh, field)?))?;
            }
            m.set_path("type", ConfigValue::Integer(integer(mesh, "type").context("mesh type")?))?;
            out.push(m);
        }
        c.set_path("meshes", ConfigValue::Indexed(out))?;
        objects.push(c);
        count("collision_geometry");
    }

    // Spawns: the donor's TDM folder, on native respawn points.
    let respawn_template = first_template(template, "respawn_point")?;
    let mut respawns = Vec::new();
    let mut teams = BTreeMap::<i32, usize>::new();
    let donor_respawns = donor
        .named("server_objects")
        .and_then(|s| s.named("respawn_points"))
        .context("donor has no respawn points")?;
    for spawn in array(donor_respawns)? {
        let full_name = string(spawn, "full_name").unwrap_or("");
        if !full_name.starts_with(&spec.spawn_folder) {
            continue;
        }
        let mut r = respawn_template.clone();
        place(&mut r, spawn, full_name)?;
        let team = integer(spawn, "team").context("spawn team")?;
        ensure!(matches!(team, 0 | 1), "{full_name}: unexpected team {team}");
        copy_shared(&mut r, spawn, &["team", "priority", "point_id", "invulnerability_time"])?;
        // 0.26's loader treats absent check radii as no check; the donor's
        // radii are authored for its spawns, keep them.
        for field in ["ally_check_radius", "enemy_check_radius"] {
            if let Some(v) = real(spawn, field) {
                r.set_path(field, ConfigValue::Float(v))?;
            }
        }
        *teams.entry(team).or_default() += 1;
        respawns.push(r);
    }
    ensure!(
        teams.get(&0).copied().unwrap_or(0) > 0 && teams.get(&1).copied().unwrap_or(0) > 0,
        "both teams need spawns, got {teams:?}"
    );
    counts.insert("respawn_point".into(), respawns.len());
    notes.push(format!("spawns: {} from {} (team 0: {}, team 1: {})", respawns.len(), spec.spawn_folder, teams[&0], teams[&1]));
    for (what, n) in &dropped {
        notes.push(format!("dropped {n} {what} (no 0.26 counterpart or not part of TDM)"));
    }

    let mut project = named(vec![
        ("project_name", ConfigValue::String(spec.level.clone())),
        ("client_objects", ConfigValue::Indexed(objects)),
        ("render_visuals", ConfigValue::Indexed(visuals)),
        ("collision_objects", ConfigValue::Indexed(collisions)),
        (
            "server_objects",
            named(vec![("respawn_points", ConfigValue::Indexed(respawns))]),
        ),
    ]);
    if let Some(camera) = template.named("camera") {
        let mut camera = camera.clone();
        if let Some(c) = donor.named("camera") {
            copy_shared(&mut camera, c, &["position", "rotation", "direction", "fov"])?;
        }
        project.set_path("camera", camera)?;
    }
    Ok(SceneReport {
        project,
        models,
        vegetation_models,
        vegetation_files,
        decal_materials,
        counts,
        notes,
    })
}

/// Normalize ambient light intensities so their median matches `target_median`.
fn calibrate_ambient(project: &mut ConfigValue, target_median: f32) -> Result<f32> {
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("no client objects")
    };
    let mut values: Vec<f32> = objects
        .iter()
        .filter(|o| kind(o) == "ambient_light")
        .filter_map(|o| real(o, "intensity"))
        .filter(|v| *v > 0.0)
        .collect();
    if values.is_empty() {
        return Ok(1.0);
    }
    values.sort_by(f32::total_cmp);
    let median = values[values.len() / 2];
    let scale = target_median / median;
    for o in objects.iter_mut().filter(|o| kind(o) == "ambient_light") {
        let v = real(o, "intensity").unwrap_or(0.0) * scale;
        o.set_path("intensity", ConfigValue::Float(v))?;
    }
    Ok(scale)
}

fn median_ambient(template: &ConfigValue) -> f32 {
    let mut v: Vec<f32> = templates_of(template, "ambient_light")
        .into_iter()
        .filter_map(|o| real(o, "intensity"))
        .collect();
    if v.is_empty() {
        return 1.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

fn cfg(db: &mut ResourceDb, path: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(&db.read(path)?)
        .with_context(|| format!("decoding {path}"))?
        .root)
}

/// Whether the 0.26 model at the same path draws and collides like the donor:
/// identical collision payloads and render bounds within 5 cm.
fn native_equivalent(model: &str, donor: &mut ResourceDb, target: &mut ResourceDb) -> Result<bool> {
    let root = format!("models/{model}.model/");
    if !target.is_file(&format!("{root}settings")) {
        return Ok(false);
    }
    for kind in ["collision", "walker_collision"] {
        for member in ["vertices", "indices", "exported_primitives"] {
            let path = format!("{root}{kind}/{member}");
            match (donor.is_file(&path), target.is_file(&path)) {
                (false, false) => {}
                (true, true) => {
                    if donor.read(&path)? != target.read(&path)? {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
    }
    let properties = format!("{root}render/export_properties");
    if !(donor.is_file(&properties) && target.is_file(&properties)) {
        return Ok(false);
    }
    let (Some(a), Some(b)) = (
        bounding_box(&cfg(donor, &properties)?),
        bounding_box(&cfg(target, &properties)?),
    ) else {
        return Ok(false);
    };
    Ok(a.0.iter().chain(&a.1).zip(b.0.iter().chain(&b.1)).all(|(x, y)| (x - y).abs() <= 0.05))
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(a, _)| a)
}

fn relocate(path: &str) -> String {
    let (family, tail) = path.split_once('/').expect("resource has a family");
    format!("{family}/ported/v069d/{tail}")
}

pub fn build(
    manifest_path: &Path,
    donor_path: &Path,
    target_path: &Path,
    output: &Path,
) -> Result<Record> {
    let m: Manifest = serde_json::from_reader(fs::File::open(manifest_path)?)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    ensure!(m.source_build == SourceBuild::V069d, "this builder ports 0.69d maps");
    eprintln!("Checking donor and target fingerprints...");
    check_hash(donor_path, &m.donor_fingerprint)?;
    check_hash(target_path, &m.target_sha256)?;
    let mut donor = ResourceDb::open(donor_path)?;
    let mut target = ResourceDb::open(target_path)?;
    let spec = m.map.clone();

    let mut dictionary = cfg(&mut target, DICTIONARY)?;
    if let Some(ConfigValue::Indexed(entries)) = dictionary.named("maps_dict") {
        for entry in entries {
            ensure!(
                integer(entry, "id") != Some(spec.id.into()),
                "map ID {} is already registered",
                spec.id
            );
        }
    } else {
        bail!("0.26 dictionary has no maps_dict");
    }
    let template = cfg(&mut target, &spec.template_project)?;
    let donor_project = cfg(&mut donor, &spec.donor_project)?;
    eprintln!("Converting the scene...");
    let mut scene = convert_scene(&donor_project, &template, &spec, &m.particle_replacements)?;
    let ambient_scale = calibrate_ambient(&mut scene.project, median_ambient(&template))?;
    if let Some(sounds) = &m.ambient_sounds {
        let source = cfg(&mut target, &sounds.project)?;
        let Some(ConfigValue::Indexed(objects)) = scene.project.named_mut("client_objects") else { bail!("no objects") };
        let mut added = 0;
        for sound in templates_of(&source, "object_sound") {
            let names = [string(sound, "full_name").unwrap_or(""), string(sound, "sound_name").unwrap_or("")];
            if !sounds.include.iter().any(|p| names.iter().any(|n| n.starts_with(p.as_str()))) {
                continue;
            }
            let mut sound = sound.clone();
            let p = float3(&sound, "position")?;
            sound.set_path("position", ConfigValue::Float3(std::array::from_fn(|i| p[i] + sounds.offset[i])))?;
            objects.push(sound);
            added += 1;
        }
        scene.counts.insert("object_sound".into(), added);
        scene.notes.push(format!("ambience: {added} native object_sound emitters from {} (0.69d sound environments have no 0.26 counterpart)", sounds.project));
    }
    let mut notes = std::mem::take(&mut scene.notes);
    notes.push(format!("ambient lights: 0.69d lux scaled by {ambient_scale:.4} so their median intensity matches the template's"));

    // Which models come from 0.26 and which from the donor.
    let mut native_models = BTreeSet::new();
    let mut donor_models = BTreeSet::new();
    let all_models: BTreeSet<String> = scene.models.iter().chain(&scene.vegetation_models).cloned().collect();
    for model in &all_models {
        let root = format!("models/{model}.model/");
        let in_donor = donor.paths_with_prefix(&root).next().is_some();
        let use_native = !m.force_donor_models.contains(model)
            && !scene.vegetation_models.contains(model)
            && (!in_donor || native_equivalent(model, &mut donor, &mut target)?);
        if use_native {
            ensure!(
                target.paths_with_prefix(&root).next().is_some(),
                "model {model} exists in neither build"
            );
            native_models.insert(model.clone());
        } else {
            donor_models.insert(model.clone());
        }
    }
    notes.push(format!(
        "models: {} native 0.26 models reused (same collision, render bounds within 5 cm), {} imported from 0.69d",
        native_models.len(),
        donor_models.len()
    ));

    // Donor files: the imported models' files, their materials and textures.
    let mut donor_files = BTreeSet::new();
    let mut material_usage: BTreeMap<String, Usage> = BTreeMap::new();
    let mut surface_strides: BTreeMap<String, usize> = BTreeMap::new();
    let mut surface_lod: BTreeMap<String, usize> = BTreeMap::new();
    // Model root -> LOD0 surfaces that get generated LOD1/LOD2 copies.
    let mut generate_lods: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for model in &donor_models {
        let root = format!("models/{model}.model/");
        let mut files: Vec<String> = donor
            .paths_with_prefix(&root)
            .filter(|p| donor.is_file(p))
            .map(str::to_owned)
            .collect();
        ensure!(!files.is_empty(), "donor model {model} has no files");
        // Render geometry no material or LOD names is not drawn by 0.69d
        // either (stale export leftovers); it is not imported.
        let settings_probe = cfg(&mut donor, &format!("{root}settings"))?;
        let mut referenced = BTreeSet::new();
        if let Some(ConfigValue::Named(entries)) = settings_probe.named("material_settings") {
            referenced.extend(entries.iter().map(|e| e.name.clone()));
        }
        let stale: Vec<String> = files
            .iter()
            .filter_map(|f| f.strip_prefix(&format!("{root}render/"))?.strip_suffix("/converted_model").map(str::to_owned))
            .filter(|surface| !referenced.contains(surface))
            .collect();
        if !stale.is_empty() {
            files.retain(|f| !stale.iter().any(|s| f.starts_with(&format!("{root}render/{s}/"))));
            notes.push(format!("{root}: unreferenced render geometry not imported: {}", stale.join(", ")));
        }
        for file in &files {
            if file.ends_with("/converted_model") {
                let bytes = donor.read(file)?;
                surface_strides.insert(file.clone(), stride(&bytes).context("not static")?);
            }
        }
        let settings_path = format!("{root}settings");
        let settings = cfg(&mut donor, &settings_path)?;
        let lods = surface_lods(&settings);
        for (surface, lod) in &lods {
            surface_lod.insert(format!("{root}render/{surface}/converted_model"), *lod);
        }
        if let Some(simplify) = m.simplify {
            let has_lod1 = lods.values().any(|l| *l > 0);
            let lod0: Vec<&String> = lods.iter().filter(|(_, l)| **l == 0).map(|(s, _)| s).collect();
            let terrain = m.decimate_terrain.iter().any(|p| model.starts_with(p.as_str()));
            if !has_lod1 && !lod0.is_empty() && !scene.vegetation_models.contains(model) && !terrain {
                let mut total = 0;
                for surface in &lod0 {
                    let path = format!("{root}render/{surface}/converted_model");
                    if donor.is_file(&path) {
                        total += triangles(&donor.read(&path)?);
                    }
                }
                if total > simplify.generate_above {
                    generate_lods.insert(root.trim_end_matches('/').to_owned(), lod0.iter().map(|s| (*s).clone()).collect::<Vec<_>>());
                }
            }
        }
        if let Some(ConfigValue::Named(entries)) = settings.named("material_settings") {
            for entry in entries {
                let Some(material) = string(&entry.value, "material_name") else { continue };
                if material.is_empty() {
                    continue;
                }
                let material = material.replace('\\', "/");
                let surface = format!("{root}render/{}/converted_model", entry.name);
                let usage = material_usage.entry(material).or_default();
                if surface_strides.get(&surface) == Some(&32) {
                    usage.vertex_colour = true;
                }
                if scene.vegetation_models.contains(model) {
                    usage.vegetation = true;
                }
            }
        }
        donor_files.extend(files);
    }
    for material in &scene.decal_materials {
        material_usage.entry(material.clone()).or_default();
    }

    // Material templates.
    let mut templates = BTreeMap::new();
    for (category, path) in &m.material_templates {
        let config = cfg(&mut target, &format!("material_instances/{path}.material"))
            .with_context(|| format!("loading template {path}"))?;
        templates.insert(category.clone(), (path.clone(), config));
    }
    let default_normal = "editor/default_nmap";
    // The native global probe stands in for reflection cubemaps that ship nowhere.
    let cube = {
        let Some(ConfigValue::Indexed(objects)) = scene.project.named("client_objects") else { bail!("no objects") };
        let probe = objects.iter().find(|o| kind(o) == "environment_probe").context("no probe")?;
        string(probe, "texture").context("probe texture")?.to_owned()
    };
    ensure!(target.is_file(&format!("textures/{cube}.dds")), "global probe texture {cube} missing");
    ensure!(target.is_file(&format!("textures/{default_normal}.dds")), "no default normal map");

    // Terrain: one material per layer; segments take their dominant layer.
    let mut converted_materials: BTreeMap<String, ConvertedMaterial> = BTreeMap::new();
    let mut terrain_layers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut terrain_surface_material: BTreeMap<String, String> = BTreeMap::new();
    // Terrain surfaces drawn as a two-layer vertex blend: (base, blended) layer.
    let mut terrain_blend: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut terrain_mask: Option<((usize, usize, Vec<[u8; 4]>), f32)> = None;
    let mut textures = BTreeSet::new();
    let mut material_notes = BTreeMap::<String, Vec<String>>::new();
    for (material, usage) in &material_usage {
        let path = format!("material_instances/{material}.material");
        if !donor.is_file(&path) {
            notes.push(format!(
                "{material}: named by a 0.69d surface but shipped by 0.69d {}; the name is kept",
                if target.is_file(&path) { "not, 0.26 has it" } else { "nor 0.26 (editor-only surface)" }
            ));
            continue;
        }
        let root = cfg(&mut donor, &path).with_context(|| format!("decal or model material {material}"))?;
        let category = categorize(&root, *usage).with_context(|| format!("categorizing {material}"))?;
        if category == Category::Terrain {
            let effect = g_stage(&root).and_then(|s| s.named("effect")).context("terrain effect")?;
            let layer_count = number(effect, "layer_count").unwrap_or(5.0) as usize;
            let mut names = Vec::new();
            let mut layer_maps: Vec<(String, Option<String>, Option<String>)> = Vec::new();
            for layer in 0..layer_count {
                let albedo = text(effect, &format!("layer_{layer}_texture_albedo"))
                    .with_context(|| format!("terrain layer {layer} albedo"))?;
                let normal = text(effect, &format!("layer_{layer}_texture_normal"));
                let smoothness = text(effect, &format!("layer_{layer}_texture_smoothness"));
                // A synthetic donor material for the layer, then the normal path.
                let synthetic = named(vec![(
                    "material",
                    named(vec![
                        ("instance", named(vec![("base_material", ConfigValue::String("dielectric".into()))])),
                        ("common", named(vec![("stages", named(vec![("g_stage", ConfigValue::Integer(1))]))])),
                        (
                            "g_stage",
                            named(vec![(
                                "effect",
                                named(vec![
                                    ("effect_id", ConfigValue::String("g_stage_default".into())),
                                    ("t_albedo", ConfigValue::String(albedo.into())),
                                    ("use_t_albedo", ConfigValue::Boolean(true)),
                                    ("t_normal", ConfigValue::String(normal.unwrap_or("").into())),
                                    ("use_t_normal", ConfigValue::Boolean(normal.is_some())),
                                    ("t_smoothness", ConfigValue::String(smoothness.unwrap_or("").into())),
                                    ("use_t_smoothness", ConfigValue::Boolean(smoothness.is_some())),
                                ]),
                            )]),
                        ),
                    ]),
                )]);
                let mut converted = convert_material(&synthetic, Usage::default(), &templates, m.roughness_scale, default_normal)?;
                if let Some((t, template)) = templates.get("terrain") {
                    // Prefer the native terrain template's switches.
                    let mut root = template.clone();
                    for (field, value) in [("texture_diffuse", albedo), ("texture_normal", normal.unwrap_or(default_normal))] {
                        set_effect(&mut root, "g_stage", field, ConfigValue::String(value.into()))?;
                    }
                    if effect_flag(&root, "g_stage", "use_troughness") {
                        set_effect(&mut root, "g_stage", "texture_roughness", ConfigValue::String(smoothness.unwrap_or(albedo).into()))?;
                        let s = if smoothness.is_some() { m.roughness_scale } else { 0.1 * m.roughness_scale };
                        set_effect(&mut root, "g_stage", "constant_roughness_min", ConfigValue::Float(0.0))?;
                        set_effect(&mut root, "g_stage", "constant_roughness_max", ConfigValue::Float(s))?;
                    }
                    converted = ConvertedMaterial { root, template: t.clone(), category: Category::Terrain, notes: Vec::new() };
                }
                layer_maps.push((albedo.to_owned(), normal.map(str::to_owned), smoothness.map(str::to_owned)));
                let layer_material = format!("{material}_layer_{layer}");
                let exists = |t: &str| donor.is_file(&format!("textures/{t}.dds")) || target.is_file(&format!("textures/{t}.dds"));
                fill_missing_textures(&mut converted.root, &exists, &cube)?;
                for t in bound_textures(&converted.root) {
                    textures.insert(t);
                }
                converted_materials.insert(layer_material.clone(), converted);
                names.push(layer_material);
            }
            terrain_layers.insert(material.clone(), names);
            material_notes.entry(material.clone()).or_default().push(format!(
                "layered terrain ({layer_count} layers): each segment blends its two dominant layers by vertex colour sampled from the 0.69d blend mask (0.26 vertex blend); further layers, height blending and the per-texel mask have no 0.26 counterpart"
            ));
            let mask = text(effect, "texture_layer_mask").context("terrain mask")?;
            let tile = number(effect, "layer_mask_tile_factor").unwrap_or(1.0);
            let invert = flag(effect, "invert_layer_mask_tile_factor");
            let mask_bytes = donor.read(&format!("textures/{mask}.dds"))?;
            let decoded = decode_bc(&mask_bytes)?;
            // Assign every terrain surface its dominant layer.
            for (surface, _) in surface_strides.iter() {
                let Some((model_root, surface_name)) = surface.strip_suffix("/converted_model").and_then(|p| p.split_once(".model/render/")) else { continue };
                let settings = cfg(&mut donor, &format!("{model_root}.model/settings"))?;
                let uses = settings
                    .named("material_settings")
                    .and_then(|s| s.named(surface_name))
                    .and_then(|e| string(e, "material_name"))
                    .is_some_and(|n| n.replace('\\', "/") == *material);
                if !uses {
                    continue;
                }
                let bytes = donor.read(surface)?;
                let mask_tile = if invert { tile } else { 1.0 / tile };
                let (layer, share) = dominant_layer(&bytes, &decoded, mask_tile, false)?;
                let second = (0..share.len())
                    .filter(|l| *l != layer)
                    .max_by(|a, b| share[*a].total_cmp(&share[*b]))
                    .unwrap();
                match templates.get("terrain_blend") {
                    Some((template_path, template)) if share[second] >= 0.03 => {
                        let blend = format!("{material}_blend_{layer}_{second}");
                        if !converted_materials.contains_key(&blend) {
                            let mut root = template.clone();
                            let (a, b) = (&layer_maps[layer], &layer_maps[second]);
                            let normal_a = a.1.clone().unwrap_or_else(|| default_normal.to_owned());
                            let normal_b = b.1.clone().unwrap_or_else(|| default_normal.to_owned());
                            for (field, value) in [
                                ("texture_diffuse", a.0.clone()),
                                ("texture_normal", normal_a.clone()),
                                ("texture_vertex_blended_diffuse", b.0.clone()),
                                ("texture_vertex_blended_normal", normal_b),
                            ] {
                                set_effect(&mut root, "g_stage", field, ConfigValue::String(value))?;
                            }
                            for field in ["texture_detail_normal", "texture_alphablended_normal"] {
                                if effect_text(&root, "g_stage", field).is_some() {
                                    set_effect(&mut root, "g_stage", field, ConfigValue::String(normal_a.clone()))?;
                                }
                            }
                            if effect_text(&root, "g_stage", "texture_vertex_blended_specular").is_some() {
                                set_effect(&mut root, "g_stage", "texture_vertex_blended_specular", ConfigValue::String(b.2.clone().unwrap_or_else(|| b.0.clone())))?;
                            }
                            if effect_flag(&root, "g_stage", "use_troughness") {
                                set_effect(&mut root, "g_stage", "texture_roughness", ConfigValue::String(a.2.clone().unwrap_or_else(|| a.0.clone())))?;
                                set_effect(&mut root, "g_stage", "constant_roughness_min", ConfigValue::Float(0.0))?;
                                set_effect(&mut root, "g_stage", "constant_roughness_max", ConfigValue::Float(m.roughness_scale))?;
                            }
                            let exists = |t: &str| donor.is_file(&format!("textures/{t}.dds")) || target.is_file(&format!("textures/{t}.dds"));
                            fill_missing_textures(&mut root, &exists, &cube)?;
                            for t in bound_textures(&root) {
                                textures.insert(t);
                            }
                            converted_materials.insert(blend.clone(), ConvertedMaterial { root, template: template_path.clone(), category: Category::Terrain, notes: Vec::new() });
                        }
                        terrain_surface_material.insert(surface.clone(), blend);
                        terrain_blend.insert(surface.clone(), (layer, second));
                    }
                    _ => {
                        terrain_surface_material.insert(surface.clone(), terrain_layers[material][layer].clone());
                    }
                }
            }
            terrain_mask = Some((decoded, if invert { tile } else { 1.0 / tile }));
            continue;
        }
        if matches!(category, Category::Glass | Category::Water) && target.is_file(&path) {
            notes.push(format!("{material}: the native 0.26 material of the same name is used"));
            continue;
        }
        let mut converted = convert_material(&root, *usage, &templates, m.roughness_scale, default_normal)
            .with_context(|| format!("converting {material}"))?;
        let exists = |t: &str| donor.is_file(&format!("textures/{t}.dds")) || target.is_file(&format!("textures/{t}.dds"));
        let replaced = fill_missing_textures(&mut converted.root, &exists, &cube).with_context(|| format!("checking textures of {material}"))?;
        if !replaced.is_empty() {
            converted.notes.push(format!("unused template texture names that ship nowhere now name the diffuse: {}", replaced.join(", ")));
        }
        for t in bound_textures(&converted.root) {
            textures.insert(t);
        }
        material_notes.entry(material.clone()).or_default().extend(converted.notes.iter().cloned());
        converted_materials.insert(material.clone(), converted);
    }

    // Textures: donor ones are imported, everything else must be native.
    let mut donor_textures = BTreeSet::new();
    for t in &textures {
        let dds = format!("textures/{t}.dds");
        if donor.is_file(&dds) {
            donor_textures.insert(t.clone());
            donor_files.insert(dds);
            let options = format!("textures/{t}.options");
            if donor.is_file(&options) {
                donor_files.insert(options);
            }
        } else {
            ensure!(target.is_file(&dds), "texture {t} exists in neither build");
        }
    }
    // Vegetation files are relative to the project's level directory.
    let donor_level_dir = parent_dir(parent_dir(&spec.donor_project));
    let output_level_dir = parent_dir(&format!("projects/{}/client_project", spec.output_project)).to_owned();
    let output_level_dir = parent_dir(&output_level_dir).to_owned();
    for veg in &scene.vegetation_files {
        let path = format!("{donor_level_dir}/{veg}");
        ensure!(donor.is_file(&path), "vegetation file {path} missing");
        donor_files.insert(path);
    }

    // References to rewrite: imported donor resources move to the namespace.
    let mut substitutions = BTreeMap::new();
    for model in &donor_models {
        let dest = format!("ported/v069d/{model}");
        substitutions.insert(model.clone(), dest.clone());
        substitutions.insert(format!("models/{model}.model"), format!("models/{dest}.model"));
        substitutions.insert(format!("resources/models/{model}.model"), format!("resources/models/{dest}.model"));
    }
    let material_dest = |name: &str| format!("ported/v069d/{name}");
    for t in &donor_textures {
        substitutions.insert(format!("texture:{t}"), format!("ported/v069d/{t}"));
    }

    crate::weapon::prepare_empty_output(output)?;
    let mut files: Vec<BuiltFile> = Vec::new();
    let game_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let mut converted_geometry = BTreeSet::new();
    let mut repaired_model_settings = BTreeSet::new();
    let mut vegetation_instances = BTreeMap::new();
    let source = "0.69d0+0.26g-adapter";
    let mut halved = 0usize;
    let mut decimated = 0usize;
    let mut simplified_triangles = (0usize, 0usize);
    let mut blended_terrain = 0usize;
    let mut generated: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new();
    let mut lod_switched = 0usize;
    let mut lods_generated = 0usize;
    eprintln!("Emitting {} donor files...", donor_files.len());
    for path in &donor_files {
        let mut bytes = donor.read(path)?;
        let dest = if let Some(rest) = path.strip_prefix(&format!("{donor_level_dir}/")) {
            format!("{output_level_dir}/{rest}")
        } else {
            relocate(path)
        };
        if path.ends_with("/converted_model") {
            bytes = geometry(&bytes).with_context(|| format!("repacking {path}"))?;
            if m.decimate_terrain.iter().any(|p| path.starts_with(&format!("models/{p}"))) {
                match decimate_grid(&bytes) {
                    Ok(Some(smaller)) => {
                        decimated += 1;
                        bytes = smaller;
                    }
                    Ok(None) => {
                        // Tiles re-cut around buildings are not grids; keep
                        // their borders so neighbouring tiles stay sealed.
                        match simplify_mesh_with(&bytes, 0.25, 0.005, meshopt::SimplifyOptions::LockBorder)? {
                            Some(smaller) => {
                                decimated += 1;
                                bytes = smaller;
                            }
                            None => notes.push(format!("{path}: terrain kept at full density")),
                        }
                    }
                    Err(e) => return Err(e.context(format!("decimating {path}"))),
                }
            }
            if let (Some(&(base, blended)), Some((mask, tile))) = (terrain_blend.get(path), &terrain_mask) {
                bytes = blend_colours(&bytes, mask, *tile, base, blended).with_context(|| format!("blending {path}"))?;
                blended_terrain += 1;
            }
            if let (Some(simplify), Some(&lod)) = (m.simplify, surface_lod.get(path)) {
                if lod > 0 && triangles(&bytes) >= simplify.min_triangles {
                    let ratio = if lod == 1 { simplify.lod1_ratio } else { simplify.lod2_ratio };
                    if let Some(smaller) = simplify_mesh(&bytes, ratio, simplify.max_error).with_context(|| format!("simplifying {path}"))? {
                        simplified_triangles.0 += triangles(&bytes);
                        simplified_triangles.1 += triangles(&smaller);
                        bytes = smaller;
                    }
                }
            }
            if let (Some(simplify), Some((model_root, surface))) = (m.simplify, path.strip_suffix("/converted_model").and_then(|p| p.split_once("/render/"))) {
                if generate_lods.get(model_root).is_some_and(|l| l.iter().any(|x| x == surface)) {
                    for (level, ratio) in [(1, simplify.lod1_ratio), (2, simplify.lod2_ratio)] {
                        let lod = simplify_mesh(&bytes, ratio, simplify.max_error)?.unwrap_or_else(|| bytes.clone());
                        let name = format!("srp_lod{level}_{surface}");
                        let gen_dest = relocate(&format!("{model_root}/render/{name}/converted_model"));
                        emit(output, &gen_dest, &lod, "0.69d LOD0 simplified by meshoptimizer", &mut files)?;
                        let properties = donor.read(&format!("{model_root}/render/{surface}/export_properties"))?;
                        let properties = BinaryConfig::parse(&properties)?.to_bytes()?;
                        emit(output, &relocate(&format!("{model_root}/render/{name}/export_properties")), &properties, source, &mut files)?;
                        generated.entry(model_root.to_owned()).or_insert_with(Vec::new).push((level, surface.to_owned(), name));
                    }
                }
            }
            converted_geometry.insert(path.clone());
        } else if path.ends_with(".model/settings") {
            let mut c = BinaryConfig::parse(&bytes)?.root;
            let model_root = path.trim_end_matches("/settings");
            if c.named("game_material_settings").is_some() {
                crate::target_native_map::remap_game_material_settings(
                    &mut c,
                    path,
                    &game_materials,
                    &m.game_material_names,
                    None,
                )?;
            }
            if let Some(ConfigValue::Named(entries)) = c.named_mut("material_settings") {
                for entry in entries {
                    let surface = format!("{model_root}/render/{}/converted_model", entry.name);
                    let Some(ConfigValue::String(name)) = entry.value.named_mut("material_name") else { continue };
                    let normalized = name.replace('\\', "/");
                    if let Some(layer) = terrain_surface_material.get(&surface) {
                        *name = material_dest(layer);
                        repaired_model_settings.insert(path.clone());
                    } else if let Some(first) = terrain_layers.get(&normalized) {
                        *name = material_dest(&first[0]);
                    } else if converted_materials.contains_key(&normalized) {
                        *name = material_dest(&normalized);
                    }
                }
            }
            if let Some(list) = generated.get(model_root) {
                for (level, surface, name) in list {
                    let material = c
                        .named("material_settings")
                        .and_then(|ms| ms.named(surface))
                        .cloned()
                        .with_context(|| format!("{path}: no material for {surface}"))?;
                    c.set_path(&format!("material_settings.{name}"), material)?;
                    let lod_path = format!("lod_hierrarchy.LOD{level}");
                    let existing = c.named("lod_hierrarchy").and_then(|h| h.named(&format!("LOD{level}"))).and_then(|l| l.named("surfaces")).cloned();
                    match existing {
                        Some(ConfigValue::Indexed(_)) => {}
                        Some(other) if array(&other).is_ok_and(|a| a.is_empty()) => {
                            c.set_path(&format!("{lod_path}.surfaces"), ConfigValue::Indexed(Vec::new()))?;
                        }
                        Some(_) => bail!("{path}: LOD{level} surfaces are not a list"),
                        None => c.set_path(&lod_path, named(vec![("surfaces", ConfigValue::Indexed(Vec::new()))]))?,
                    }
                    let Some(ConfigValue::Indexed(list)) = c.named_mut("lod_hierrarchy").and_then(|h| h.named_mut(&format!("LOD{level}"))).and_then(|l| l.named_mut("surfaces")) else {
                        bail!("{path}: LOD{level} surfaces are not a list");
                    };
                    list.push(ConfigValue::String(name.clone()));
                }
                lods_generated += 1;
            }
            let prefix = format!("{model_root}/render/");
            let mut actual: BTreeSet<String> = donor_files
                .range(prefix.clone()..)
                .take_while(|p| p.starts_with(&prefix))
                .filter_map(|p| p.strip_prefix(&prefix)?.strip_suffix("/converted_model").map(str::to_owned))
                .collect();
            if let Some(list) = generated.get(model_root) {
                actual.extend(list.iter().map(|(_, _, name)| name.clone()));
            }
            if !actual.is_empty() && super::repair_settings(&mut c, &actual, &BTreeMap::new()).with_context(|| format!("repairing {path}"))? {
                repaired_model_settings.insert(path.clone());
            }
            let has_lod1 = c
                .named("lod_hierrarchy")
                .and_then(|h| h.named("LOD1"))
                .and_then(|l| l.named("surfaces"))
                .is_some_and(|s| array(s).is_ok_and(|a| !a.is_empty()));
            if let (Some(d), true) = (m.lod_distances, has_lod1) {
                c.set_path(
                    "lod_switching",
                    named(vec![
                        ("type", ConfigValue::Integer(0)),
                        ("default_params", ConfigValue::Boolean(false)),
                        ("param0", ConfigValue::Float(d[0])),
                        ("param1", ConfigValue::Float(d[1])),
                        ("param2", ConfigValue::Float(d[2])),
                    ]),
                )?;
                lod_switched += 1;
            }
            bytes = BinaryConfig { root: c }.to_bytes()?;
        } else if path.ends_with(".veg") {
            vegetation_instances.insert(path.clone(), super::early::vegetation_instances(&bytes).with_context(|| format!("checking {path}"))?);
        } else if path.starts_with("textures/") && path.ends_with(".dds") {
            let (w, h, _, fourcc) = dds_info(&bytes).with_context(|| format!("reading {path}"))?;
            ensure!(matches!(&fourcc, b"DXT1" | b"DXT5"), "{path}: unsupported texture format {:?}", String::from_utf8_lossy(&fourcc));
            let stem = path.trim_end_matches(".dds");
            let limit = m
                .texture_limits
                .iter()
                .filter(|(suffix, _)| stem.ends_with(suffix.as_str()))
                .map(|(_, l)| *l)
                .min()
                .or(m.max_texture_size);
            if let Some(limit) = limit {
                let caps2 = u32::from_le_bytes(bytes[112..116].try_into().unwrap());
                let (mut w, mut h) = (w, h);
                while w.max(h) > limit && caps2 & 0xfe00 == 0 && w >= 8 && h >= 8 {
                    bytes = drop_top_mip(&bytes).with_context(|| format!("halving {path}"))?;
                    (w, h) = (w / 2, h / 2);
                    halved += 1;
                }
            }
        } else if super::is_config_resource(path) {
            // export_properties, exported_primitives and texture options are
            // schema-identical; re-encode in the 24-byte layout.
            let c = BinaryConfig::parse(&bytes).with_context(|| format!("decoding {path}"))?;
            bytes = c.to_bytes()?;
        }
        ensure!(!target.contains(&dest), "would overwrite native resource {dest}");
        emit(output, &dest, &bytes, source, &mut files)?;
    }
    if decimated > 0 {
        notes.push(format!("{decimated} terrain render meshes reduced to about a quarter of their triangles (every second grid line, or border-locked simplification for re-cut tiles; collision unchanged)"));
    }
    if blended_terrain > 0 {
        notes.push(format!("{blended_terrain} terrain meshes carry two-layer blend weights in a new vertex colour (stride 28 -> 32)"));
    }
    if simplified_triangles.0 > 0 {
        notes.push(format!("authored LOD1/LOD2 surfaces simplified from {} to {} triangles (meshoptimizer, error bound {})", simplified_triangles.0, simplified_triangles.1, m.simplify.unwrap().max_error));
    }
    if lods_generated > 0 {
        notes.push(format!("{lods_generated} imported models without LODs above {} LOD0 triangles got generated LOD1/LOD2 surfaces", m.simplify.unwrap().generate_above));
    }
    if lod_switched > 0 {
        notes.push(format!("{lod_switched} imported models with LODs switch by distance {:?} m instead of projected size", m.lod_distances.unwrap()));
    }
    if halved > 0 {
        notes.push(format!("{halved} top mips dropped from imported textures above {:?} texels (per-suffix limits {:?}) for 0.26 memory budgets", m.max_texture_size, m.texture_limits));
    }

    // Materials.
    for (name, converted) in &converted_materials {
        let mut root = converted.root.clone();
        rewrite_textures(&mut root, &donor_textures);
        let dest = format!("material_instances/{}.material", material_dest(name));
        ensure!(!target.contains(&dest), "would overwrite native {dest}");
        emit(output, &dest, &BinaryConfig { root }.to_bytes()?, &format!("0.26g template {}", converted.template), &mut files)?;
    }
    for (name, list) in &material_notes {
        for note in list {
            notes.push(format!("{name}: {note}"));
        }
    }

    // Static batching of repeated imported models.
    if let Some(cell) = m.batch_cell {
        let overlay = output.join("overlay");
        let read_config = |rel: &str| -> Result<ConfigValue> {
            Ok(BinaryConfig::parse(&fs::read(overlay.join(rel)).with_context(|| format!("reading {rel}"))?)?.root)
        };
        let Some(ConfigValue::Indexed(visuals)) = scene.project.named_mut("render_visuals") else { bail!("no render visuals") };
        let mut groups: BTreeMap<(String, i64, i64), Vec<usize>> = BTreeMap::new();
        for (i, v) in visuals.iter().enumerate() {
            let model = model_of_reference(string(v, "lib_name").unwrap_or(""));
            if !donor_models.contains(&model) || scene.vegetation_models.contains(&model) {
                continue;
            }
            let p = float3(v, "position")?;
            groups.entry((model, (p[0] / cell).floor() as i64, (p[2] / cell).floor() as i64)).or_default().push(i);
        }
        let mut removed = BTreeSet::new();
        let mut added = Vec::new();
        let (mut batches, mut batched) = (0usize, 0usize);
        for ((model, cx, cz), members) in groups {
            if members.len() < 2 {
                continue;
            }
            let root = format!("models/ported/v069d/{model}.model");
            let mut surfaces = Vec::new();
            for entry in fs::read_dir(overlay.join(&root).join("render"))? {
                let entry = entry?;
                if entry.path().join("converted_model").is_file() {
                    surfaces.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            surfaces.sort();
            let mut largest = 1;
            for surface in &surfaces {
                let bytes = fs::read(overlay.join(&root).join("render").join(surface).join("converted_model"))?;
                largest = largest.max(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize);
            }
            let per_batch = 65_536 / largest;
            if per_batch < 2 {
                continue;
            }
            let settings = read_config(&format!("{root}/settings"))?;
            let model_properties = read_config(&format!("{root}/render/export_properties"))?;
            for (chunk_index, chunk) in members.chunks(per_batch).enumerate() {
                if chunk.len() < 2 {
                    continue;
                }
                let placements: Vec<Placement> = chunk
                    .iter()
                    .map(|&i| Ok(Placement { position: float3(&visuals[i], "position")?, rotation: float3(&visuals[i], "rotation")?, scale: float3(&visuals[i], "scale")? }))
                    .collect::<Result<_>>()?;
                let name = format!("srp_batch/{}_{cx}_{cz}_{chunk_index}", model.replace('/', "__"));
                let dest = format!("models/ported/v069d/{name}.model");
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for surface in &surfaces {
                    let bytes = fs::read(overlay.join(&root).join("render").join(surface).join("converted_model"))?;
                    let (baked, a, b) = bake_instances(&bytes, &placements).with_context(|| format!("batching {model}/{surface}"))?;
                    for i in 0..3 {
                        lo[i] = lo[i].min(a[i]);
                        hi[i] = hi[i].max(b[i]);
                    }
                    emit(output, &format!("{dest}/render/{surface}/converted_model"), &baked, "0.69d instances baked into one static batch", &mut files)?;
                    let mut properties = read_config(&format!("{root}/render/{surface}/export_properties"))?;
                    set_bounds(&mut properties, a, b)?;
                    emit(output, &format!("{dest}/render/{surface}/export_properties"), &BinaryConfig { root: properties }.to_bytes()?, source, &mut files)?;
                }
                let mut properties = model_properties.clone();
                set_bounds(&mut properties, lo, hi)?;
                emit(output, &format!("{dest}/render/export_properties"), &BinaryConfig { root: properties }.to_bytes()?, source, &mut files)?;
                let mut batch_settings = settings.clone();
                for field in ["collision_enabled", "walker_collision_enabled"] {
                    if batch_settings.named(field).is_some() {
                        batch_settings.set_path(field, ConfigValue::Boolean(false))?;
                    }
                }
                emit(output, &format!("{dest}/settings"), &BinaryConfig { root: batch_settings }.to_bytes()?, source, &mut files)?;
                added.push(named(vec![
                    ("lib_name", ConfigValue::String(format!("ported/v069d/{name}"))),
                    ("position", ConfigValue::Float3([0.0; 3])),
                    ("rotation", ConfigValue::Float3([0.0; 3])),
                    ("scale", ConfigValue::Float3([1.0; 3])),
                ]));
                removed.extend(chunk.iter().copied());
                batches += 1;
                batched += chunk.len();
            }
        }
        let mut index = 0;
        visuals.retain(|_| {
            index += 1;
            !removed.contains(&(index - 1))
        });
        visuals.extend(added);
        notes.push(format!("static batching ({cell} m cells): {batched} render instances of repeated imported models baked into {batches} render-only batch models; collision instances unchanged"));
    }

    // Scene references.
    let mut project = scene.project;
    rewrite_scene(&mut project, &donor_models, &converted_materials)?;
    let output_project = format!("projects/{}/client_project", spec.output_project);
    ensure!(!target.contains(&output_project), "project exists");
    emit(output, &output_project, &BinaryConfig { root: project }.to_bytes()?, "0.69d scene on 0.26g templates", &mut files)?;

    // Mode and registration.
    let mut mode = cfg(&mut target, &spec.template_mode)?;
    mode.set_path("mode.name", ConfigValue::String(spec.mode_name.trim_end_matches(".options").into()))?;
    mode.set_path("mode.match_making_params.map_name", ConfigValue::String(spec.output_project.clone()))?;
    mode.set_path("mode.ui.map_name", ConfigValue::String(format!("st_{}_name", spec.level)))?;
    let mode_path = format!("gameplay/game_modes/{}", spec.mode_name);
    ensure!(!target.contains(&mode_path), "mode exists");
    emit(output, &mode_path, &BinaryConfig { root: mode }.to_bytes()?, "0.26g mode template", &mut files)?;
    let ConfigValue::Indexed(entries) = dictionary.named_mut("maps_dict").unwrap() else { unreachable!() };
    entries.push(named(vec![
        ("id", ConfigValue::Integer(spec.id.into())),
        ("cfg_mode_name", ConfigValue::String(spec.mode_name.clone())),
    ]));
    emit(output, DICTIONARY, &BinaryConfig { root: dictionary }.to_bytes()?, "0.26g+additive-map-registration", &mut files)?;

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let record = Record {
        format_version: 1,
        asset_id: "retail-v069d-duga-to-026g".into(),
        donor_build: SourceBuild::V069d.label().into(),
        target_build: "0.26g0-2777".into(),
        target_db_sha256: m.target_sha256.clone(),
        server_project_fallback: None,
        maps: Vec::new(),
        files,
        converted_materials: converted_materials.iter().map(|(k, v)| (k.clone(), v.template.clone())).collect(),
        shared_dependencies: native_models.iter().map(|m| format!("models/{m}.model")).collect(),
        repaired_model_settings,
        adapted_particles: BTreeSet::new(),
        collision_repairs: BTreeMap::new(),
        converted_geometry,
        compatibility_notes: notes,
        vegetation_instances,
    };
    serde_json::to_writer_pretty(fs::File::create(output.join("port-manifest.json"))?, &record)?;
    let summary = serde_json::json!({ "map": spec, "counts": scene.counts, "native_models": native_models, "donor_models": donor_models });
    serde_json::to_writer_pretty(fs::File::create(output.join("scene-summary.json"))?, &summary)?;
    Ok(record)
}

fn rewrite_textures(root: &mut ConfigValue, donor_textures: &BTreeSet<String>) {
    match root {
        ConfigValue::Named(fields) => {
            for field in fields {
                let is_texture = field.name.starts_with("texture_") || field.name.ends_with("_texture");
                if is_texture {
                    let slot = if let ConfigValue::Named(_) = field.value {
                        field.value.named_mut("value")
                    } else {
                        Some(&mut field.value)
                    };
                    if let Some(ConfigValue::String(s)) = slot {
                        let n = s.replace('\\', "/");
                        if donor_textures.contains(&n) {
                            *s = format!("ported/v069d/{n}");
                        }
                    }
                } else {
                    rewrite_textures(&mut field.value, donor_textures);
                }
            }
        }
        ConfigValue::Indexed(values) => values.iter_mut().for_each(|v| rewrite_textures(v, donor_textures)),
        _ => {}
    }
}

fn rewrite_scene(
    project: &mut ConfigValue,
    donor_models: &BTreeSet<String>,
    materials: &BTreeMap<String, ConvertedMaterial>,
) -> Result<()> {
    let relocate_model = |reference: &str| -> String {
        let model = model_of_reference(reference);
        if !donor_models.contains(&model) {
            return reference.to_owned();
        }
        reference.replacen(&model, &format!("ported/v069d/{model}"), 1)
    };
    for group in ["render_visuals", "collision_objects"] {
        let Some(ConfigValue::Indexed(items)) = project.named_mut(group) else { bail!("no {group}") };
        for item in items {
            let Some(ConfigValue::String(s)) = item.named_mut("lib_name") else { bail!("no lib_name") };
            *s = relocate_model(s);
        }
    }
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else { bail!("no objects") };
    for object in objects {
        match kind(object) {
            "decal" => {
                if let Some(ConfigValue::String(s)) = object.named_mut("decal_material") {
                    let n = s.replace('\\', "/");
                    if materials.contains_key(&n) {
                        *s = format!("ported/v069d/{n}");
                    }
                }
            }
            "vegetation" => {
                if let Some(ConfigValue::Indexed(layers)) = object.named_mut("layers") {
                    for layer in layers {
                        if let Some(ConfigValue::Indexed(defs)) = layer.named_mut("model_def") {
                            for def in defs {
                                if let Some(ConfigValue::String(s)) = def.named_mut("name") {
                                    *s = relocate_model(s);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn static_mesh(basis: [u32; 3], stride: usize, extra: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend(((4 + stride) as u32).to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend([0u8; 12]);
        for w in basis {
            b.extend(w.to_le_bytes());
        }
        b.extend([0x00, 0x3c, 0x00, 0x38]);
        if stride == 32 {
            b.extend([1, 2, 3, 4]);
        }
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend([0, 0, 0, 0, 0, 0]);
        for (id, payload) in extra {
            b.extend(id.to_le_bytes());
            b.extend((payload.len() as u32).to_le_bytes());
            b.extend(payload);
        }
        b
    }

    #[test]
    fn basis_floats_become_packed_bytes_and_statistics_chunks_are_dropped() {
        // From the same mesh in both builds (metal_floor_03): 0.69d
        // 0x3f000080 0x3eff0000 0x3f7f807f is N = (0,-1,0), T = (0,0,-1),
        // B = (1,0,0); bytes x=0x80 y=0x00 z=0x80 etc.
        let mesh = static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 28, &[(110, vec![0; 4]), (114, vec![0; 72])]);
        let out = geometry(&mesh).unwrap();
        let v = &out[12..40];
        assert_eq!(&v[12..16], &[0x80, 0x00, 0x80, 0x7f]);
        assert_eq!(&v[16..20], &[0x00, 0x80, 0x7f, 0xff]);
        assert_eq!(&v[20..24], &[0x7f, 0x80, 0xff, 0xff]);
        assert_eq!(&v[24..28], &[0x00, 0x3c, 0x00, 0x38], "UV untouched");
        // Vertex chunk + index chunk only.
        assert_eq!(out.len(), 8 + 4 + 28 + 8 + 10);
        // Vertex colour survives.
        let vc = geometry(&static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 32, &[])).unwrap();
        assert_eq!(&vc[12 + 28..12 + 32], &[1, 2, 3, 4]);
    }

    #[test]
    fn a_026_packed_basis_or_unknown_chunk_is_rejected() {
        assert!(geometry(&static_mesh([0x7f7f007f, 0xff7e7f00, 0xfffe7f7e], 28, &[])).is_err());
        assert!(geometry(&static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 28, &[(77, vec![0; 4])])).is_err());
    }

    #[test]
    fn a_regular_grid_keeps_every_second_line_and_its_winding() {
        // 5 x 5 grid, two triangles per cell, x-major rows.
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend((4 + 25 * 28u32).to_le_bytes());
        b.extend(25u32.to_le_bytes());
        for z in 0..5 {
            for x in 0..5 {
                for v in [x as f32, (x * z) as f32 * 0.1, z as f32] {
                    b.extend(v.to_le_bytes());
                }
                b.extend([0u8; 16]);
            }
        }
        let mut idx = Vec::new();
        for z in 0..4u16 {
            for x in 0..4u16 {
                let (a, bb, c, d) = (z * 5 + x, z * 5 + x + 1, (z + 1) * 5 + x + 1, (z + 1) * 5 + x);
                idx.extend([a, c, bb, a, d, c]);
            }
        }
        b.extend(4u32.to_le_bytes());
        b.extend((4 + idx.len() as u32 * 2).to_le_bytes());
        b.extend((idx.len() as u32).to_le_bytes());
        for i in &idx {
            b.extend(i.to_le_bytes());
        }
        let out = decimate_grid(&b).unwrap().unwrap();
        assert_eq!(u32::from_le_bytes(out[8..12].try_into().unwrap()), 9, "3 x 3 vertices");
        let at = 8 + u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(u32::from_le_bytes(out[at + 8..at + 12].try_into().unwrap()), 8 * 3, "eight triangles");
        // Corners keep their exact heights (x * z * 0.1 at (4, 4) = 1.6).
        let last = 12 + 8 * 28;
        assert_eq!(f32::from_le_bytes(out[last + 4..last + 8].try_into().unwrap()), 1.6);
        // Same winding as the source (a, c, b): negative XZ cross product.
        let p = |i: usize| [f32::from_le_bytes(out[12 + i * 28..16 + i * 28].try_into().unwrap()), f32::from_le_bytes(out[20 + i * 28..24 + i * 28].try_into().unwrap())];
        let t: Vec<usize> = (0..3).map(|k| u16::from_le_bytes([out[at + 12 + 2 * k], out[at + 13 + 2 * k]]) as usize).collect();
        let (a, bb, c) = (p(t[0]), p(t[1]), p(t[2]));
        assert!((bb[0] - a[0]) * (c[1] - a[1]) - (bb[1] - a[1]) * (c[0] - a[0]) < 0.0);
        // A mesh that is not a grid is left alone.
        assert!(decimate_grid(&static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 28, &[])).unwrap().is_none());
    }

    #[test]
    fn baked_instances_follow_the_collision_transform_convention() {
        // One vertex at (1, 0, 0) with normal +X, one triangle (0, 0, 0).
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend((4 + 28u32).to_le_bytes());
        b.extend(1u32.to_le_bytes());
        for v in [1.0f32, 0.0, 0.0] {
            b.extend(v.to_le_bytes());
        }
        b.extend(pack_basis([1.0, 0.0, 0.0], 0x7f).to_le_bytes());
        b.extend(pack_basis([0.0, 0.0, 1.0], 0xff).to_le_bytes());
        b.extend(pack_basis([0.0, 1.0, 0.0], 0xff).to_le_bytes());
        b.extend([0u8; 4]);
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend([0u8; 6]);
        let quarter = std::f32::consts::FRAC_PI_2;
        let placements = [
            Placement { position: [10.0, 0.0, 0.0], rotation: [0.0; 3], scale: [2.0, 1.0, 1.0] },
            Placement { position: [0.0, 5.0, 0.0], rotation: [0.0, quarter, 0.0], scale: [1.0; 3] },
        ];
        let (out, lo, hi) = bake_instances(&b, &placements).unwrap();
        let position = |k: usize| -> [f32; 3] { [0, 4, 8].map(|o| f32::from_le_bytes(out[12 + k * 28 + o..16 + k * 28 + o].try_into().unwrap())) };
        assert_eq!(position(0), [12.0, 0.0, 0.0], "scale then translate");
        // Row-vector rotation about Y by +90 degrees: x -> (cos, 0, sin) row of R = (0, 0, 1).
        let p1 = position(1);
        assert!((p1[0]).abs() < 1e-6 && (p1[1] - 5.0).abs() < 1e-6 && (p1[2] - 1.0).abs() < 1e-6, "{p1:?}");
        let n1 = unpack_basis(u32::from_le_bytes(out[12 + 28 + 12..12 + 28 + 16].try_into().unwrap()));
        assert!(n1[2] > 0.98, "normal follows the rotation: {n1:?}");
        assert_eq!(out[12 + 28 + 15], 0x7f, "basis alpha kept");
        assert!(hi[0] >= 12.0 && lo[1] <= 0.0);
        // Second copy's triangle indexes its own vertex.
        let at = 8 + u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(&out[at + 12 + 6..at + 12 + 12], &[1, 0, 1, 0, 1, 0]);
    }

    #[test]
    fn halving_drops_the_top_mip_and_keeps_the_trailer() {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&8u32.to_le_bytes());
        dds[16..20].copy_from_slice(&8u32.to_le_bytes());
        dds[28..32].copy_from_slice(&2u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend([1u8; 32]); // 8x8: 4 blocks
        dds.extend([2u8; 8]); // 4x4: 1 block
        dds.push(9);
        let half = drop_top_mip(&dds).unwrap();
        assert_eq!(&half[12..16], &4u32.to_le_bytes());
        assert_eq!(&half[28..32], &1u32.to_le_bytes());
        assert_eq!(&half[128..], &[2, 2, 2, 2, 2, 2, 2, 2, 9]);
    }

    #[test]
    fn terrain_layer_weights_paint_later_layers_over_earlier_ones() {
        assert_eq!(layer_weights([0, 0, 0, 0]), [1.0, 0.0, 0.0, 0.0, 0.0]);
        let w = layer_weights([255, 0, 0, 255]);
        assert_eq!(w[4], 1.0);
        assert_eq!(w[1], 0.0);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    fn material(stage_fields: Vec<(&str, ConfigValue)>, base: &str) -> ConfigValue {
        let effect: Vec<(&str, ConfigValue)> = stage_fields
            .into_iter()
            .map(|(k, v)| (k, named(vec![("value", v), ("is_model_parameter", ConfigValue::Boolean(true))])))
            .collect();
        named(vec![(
            "material",
            named(vec![
                ("instance", named(vec![("base_material", ConfigValue::String(base.into()))])),
                ("common", named(vec![("stages", named(vec![("g_stage", ConfigValue::Integer(1))]))])),
                ("g_stage", named(vec![("effect", named(effect))])),
            ]),
        )])
    }

    fn template(switches: &[(&str, bool)]) -> ConfigValue {
        let mut effect = vec![
            ("effect_id", ConfigValue::String("g_stage_default".into())),
            ("texture_diffuse", ConfigValue::String("native/d".into())),
            ("texture_normal", ConfigValue::String("native/n".into())),
            ("texture_roughness", ConfigValue::String("native/s".into())),
            ("constant_roughness_min", ConfigValue::Integer(0)),
            ("constant_roughness_max", ConfigValue::Integer(1)),
            ("alpha_ref", ConfigValue::Float(0.25)),
        ];
        for (k, v) in switches {
            effect.push((k, ConfigValue::Boolean(*v)));
        }
        material(effect, "dielectric")
    }

    #[test]
    fn converted_material_keeps_template_switches_and_takes_donor_maps() {
        let donor = material(
            vec![
                ("effect_id", ConfigValue::String("g_stage_default".into())),
                ("t_albedo", ConfigValue::String("tiled/a".into())),
                ("use_t_albedo", ConfigValue::Boolean(true)),
                ("t_normal", ConfigValue::String("tiled/a_nmap".into())),
                ("use_t_normal", ConfigValue::Boolean(true)),
                ("t_smoothness", ConfigValue::String("tiled/a_smap".into())),
                ("use_t_smoothness", ConfigValue::Boolean(true)),
                ("max_smoothness", ConfigValue::Integer(1)),
                ("use_t_metalness", ConfigValue::Boolean(true)),
            ],
            "complex",
        );
        let templates = BTreeMap::from([(
            "opaque".to_owned(),
            ("tiled/concrete/concrete_01".to_owned(), template(&[("use_nmap", true), ("use_troughness", true)])),
        )]);
        let c = convert_material(&donor, Usage::default(), &templates, 0.2, "editor/default_nmap").unwrap();
        assert_eq!(c.category, Category::Opaque);
        let e = c.root.named("material").unwrap().named("g_stage").unwrap().named("effect").unwrap();
        assert_eq!(text(e, "texture_diffuse"), Some("tiled/a"));
        assert_eq!(text(e, "texture_normal"), Some("tiled/a_nmap"));
        assert_eq!(text(e, "texture_roughness"), Some("tiled/a_smap"));
        assert_eq!(number(e, "constant_roughness_max"), Some(0.2));
        assert!(flag(e, "use_troughness") && flag(e, "use_nmap"));
        assert!(c.notes.iter().any(|n| n.contains("metalness")));
    }

    #[test]
    fn vertex_colour_meshes_and_alpha_select_their_categories() {
        let base = |alpha: bool, two: bool| {
            material(
                vec![
                    ("effect_id", ConfigValue::String("g_stage_default".into())),
                    ("t_albedo", ConfigValue::String("a".into())),
                    ("use_alpha_test", ConfigValue::Boolean(alpha)),
                    ("two_sided", ConfigValue::Boolean(two)),
                ],
                "dielectric",
            )
        };
        let vc = Usage { vertex_colour: true, vegetation: false };
        assert_eq!(categorize(&base(false, false), vc).unwrap(), Category::VertexColour);
        assert_eq!(categorize(&base(true, true), Usage::default()).unwrap(), Category::AlphaTestTwoSided);
        assert_eq!(categorize(&base(false, true), Usage::default()).unwrap(), Category::OpaqueTwoSided);
        assert_eq!(categorize(&base(true, true), Usage { vertex_colour: false, vegetation: true }).unwrap(), Category::Grass);
    }

    #[test]
    fn neutral_resupply_becomes_one_zone_per_team_and_spawns_follow_the_folder() {
        let obj = |fields: Vec<(&str, ConfigValue)>| named(fields);
        let tf = |p: [f32; 3]| vec![("position", ConfigValue::Float3(p)), ("rotation", ConfigValue::Float3([0.0; 3])), ("scale", ConfigValue::Float3([1.0; 3]))];
        let with = |mut base: Vec<(&'static str, ConfigValue)>, more: Vec<(&'static str, ConfigValue)>| { base.extend(more); obj(base) };
        let template = obj(vec![
            ("client_objects", ConfigValue::Indexed(vec![
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment".into())), ("properties", obj(vec![("sun_angle", ConfigValue::Float(1.0)), ("sun_position_azimut", ConfigValue::Float(2.0))]))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment_probe".into())), ("geometry", ConfigValue::Integer(0)), ("radius", ConfigValue::Float(600.0)), ("texture", ConfigValue::String("environment_probes/level_03/global".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("ambient_light".into())), ("color", ConfigValue::Integer(0)), ("intensity", ConfigValue::Integer(1))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("decal".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("light".into())), ("light_type", ConfigValue::Integer(1)), ("intensity", ConfigValue::Float(1.0))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("resupply_zone".into())), ("team", ConfigValue::Integer(0)), ("resupply_time_in_ms", ConfigValue::Integer(1))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("collision_geometry".into())), ("meshes", ConfigValue::Indexed(vec![obj(vec![("type", ConfigValue::Integer(1)), ("position", ConfigValue::Float3([0.0; 3])), ("rotation", ConfigValue::Float3([0.0; 3])), ("scale", ConfigValue::Float3([1.0; 3]))])]))]),
            ])),
            ("server_objects", obj(vec![("respawn_points", ConfigValue::Indexed(vec![with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("respawn_point".into())), ("team", ConfigValue::Integer(0)), ("invulnerability_time", ConfigValue::Integer(6))])]))])),
        ]);
        let spawn = |team: i32, folder: &str, x: f32| with(tf([x, 0.0, 0.0]), vec![("game_object_type", ConfigValue::String("respawn_point".into())), ("team", ConfigValue::Integer(team)), ("full_name", ConfigValue::String(format!("{folder}/respawn_point({x})")))]);
        let donor = obj(vec![
            ("render_visuals", ConfigValue::Indexed(vec![])),
            ("collision_objects", ConfigValue::Indexed(vec![])),
            ("client_objects", ConfigValue::Indexed(vec![
                with(tf([5.0, 0.0, 5.0]), vec![("game_object_type", ConfigValue::String("resupply_zone".into())), ("team", ConfigValue::Integer(2)), ("full_name", ConfigValue::String("logic/resupply/a/resupply_zone".into())),
                    ("collision_geometries", obj(vec![("<g>", obj(vec![("resolved_path", ConfigValue::String("logic/resupply/a/resupply_collision".into()))]))]))]),
                with(tf([5.0, 0.0, 5.0]), vec![("game_object_type", ConfigValue::String("collision_geometry".into())), ("full_name", ConfigValue::String("logic/resupply/a/resupply_collision".into())),
                    ("meshes", ConfigValue::Indexed(vec![obj(vec![("type", ConfigValue::Integer(1)), ("position", ConfigValue::Float3([0.0; 3])), ("rotation", ConfigValue::Float3([0.0; 3])), ("scale", ConfigValue::Float3([2.0; 3]))])]))]),
                with(tf([9.0; 3]), vec![("game_object_type", ConfigValue::String("winner_point".into()))]),
            ])),
            ("server_objects", obj(vec![
                ("logic_objects", ConfigValue::Indexed(vec![obj(vec![("game_object_type", ConfigValue::String("environment".into())), ("sun_elevation", ConfigValue::Integer(35)), ("sun_azimuth", ConfigValue::Integer(20))])])),
                ("respawn_points", ConfigValue::Indexed(vec![spawn(0, "logic/modes/tdm/spawns/team1", 1.0), spawn(1, "logic/modes/tdm/spawns/team2", 2.0), spawn(0, "logic/modes/br/spawns", 3.0)])),
            ])),
        ]);
        let spec = MapSpec {
            id: 205, level: "level_11".into(), donor_project: "p".into(), template_project: "t".into(), template_mode: "m".into(),
            output_project: "v069d_level_11/day_4".into(), mode_name: "x.options".into(), global_probe_position: [0.0; 3],
            global_probe_radius: 300.0, spawn_folder: "logic/modes/tdm/spawns/".into(),
            probe_classes: Vec::new(), ambient_lights: false,
        };
        let scene = convert_scene(&donor, &template, &spec, &BTreeMap::new()).unwrap();
        let objects = array(scene.project.named("client_objects").unwrap()).unwrap();
        let zones: Vec<_> = objects.iter().filter(|o| kind(o) == "resupply_zone").collect();
        assert_eq!(zones.iter().map(|z| integer(z, "team").unwrap()).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(objects.iter().filter(|o| kind(o) == "collision_geometry").count(), 1);
        assert!(!objects.iter().any(|o| kind(o) == "winner_point"));
        let env = objects.iter().find(|o| kind(o) == "environment").unwrap();
        assert_eq!(real(env.named("properties").unwrap(), "sun_angle"), Some(35.0));
        let spawns = array(scene.project.named("server_objects").unwrap().named("respawn_points").unwrap()).unwrap();
        assert_eq!(spawns.len(), 2, "only the TDM folder");
        assert_eq!(integer(&spawns[0], "invulnerability_time"), Some(6), "native rule kept where the donor has none");
        assert_eq!(float3(&spawns[1], "position").unwrap(), [2.0, 0.0, 0.0]);
    }
}
