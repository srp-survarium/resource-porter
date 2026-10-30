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
use super::{BinaryConfig, ConfigValue, NamedValue, Record, ResourceDb, SourceBuild, Map};
use super::{array, check_hash, emit, string};
use super::lighting::{DiffuseRecipe, DonorLight, DonorProbe, LightEffect, LightScale, Shape, Term, Volume};
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
    /// What is ported: a map (`map`) or a lobby scene (`lobby`); exactly one
    /// (`Manifest::port`).
    #[serde(default)]
    pub map: Option<MapSpec>,
    #[serde(default)]
    pub lobby: Option<LobbySpec>,
    /// Where imported resources go, under each family (`models/<it>/...`);
    /// default `ported/<build prefix>`. Ports of one build that may ship side
    /// by side take their own, so no file of one overwrites another's.
    #[serde(default)]
    pub namespace: Option<String>,
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
    /// Donor materials drawn with a native 0.26 material as it is, by donor
    /// name (a material whose category's template would draw it wrongly:
    /// 0.69d's window panes made of a still-water material).
    #[serde(default)]
    pub native_materials: BTreeMap<String, String>,
    /// G-stage constants set on converted materials whose donor name starts
    /// with `prefix` (every matching rule applies, in order), where the
    /// template has the field: e.g. the ground's Fresnel, which 0.26's own
    /// ground materials keep at 0.
    #[serde(default)]
    pub material_constants: Vec<MaterialConstants>,
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
    /// structures at full detail across the map. Without it they keep the
    /// donor's switching and merge only with models alike.
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
    /// Material clusters: the imported render instances of a square cell
    /// become one render-only model with one surface per material and LOD.
    /// 0.26 picks a model's LOD by the distance to its bounding-box centre,
    /// so the cluster switches `max distance from its centre to a member's`
    /// later than `lod_distances`: no member is drawn coarser than alone.
    #[serde(default)]
    pub material_clusters: Option<MaterialClusters>,
    /// Render instances of imported models whose LOD0 equals another's up to
    /// a translation draw that one, moved (memory for repeated structures).
    #[serde(default)]
    pub dedup_translated_copies: bool,
    /// Imported models draw LOD0 where they would draw LOD1, and LOD1
    /// geometry is not imported: it is a third of the static geometry, which
    /// 0.26 keeps in an 88 MB vertex pool ("No enough memory in vertices
    /// pool" beyond it).
    #[serde(default)]
    pub lod1_shares_lod0: bool,
    /// Terrain render meshes (`decimate_terrain`) simplified with their open
    /// borders locked, so neighbouring tiles stay sealed whatever each one
    /// sheds; collision is unchanged. Without it grid tiles keep every second
    /// grid line.
    #[serde(default)]
    pub terrain_simplify: Option<TerrainSimplify>,
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialClusters {
    /// Cell size (metres, x and z).
    pub cell: f32,
    /// Instances of larger models (bounding radius with instance scale,
    /// metres) merge only with models alike.
    pub max_member_radius: f32,
    /// Cell size for clusters of models alike (same LOD switching, same
    /// material set; copies of one model or e.g. native trees sharing one
    /// material), formed first. Foliage, grass (whose shading may depend on
    /// the instance), native models and models above `max_member_radius`
    /// only ever merge this way.
    pub model_cell: f32,
    /// Models whose render geometry (all LODs, bytes) exceeds this stay
    /// instanced: baking copies of heavy models multiplies their memory
    /// (0.26 maps load 25-54 MiB of static geometry).
    pub max_member_bytes: usize,
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainSimplify {
    /// Target triangle ratio.
    pub ratio: f32,
    /// Relative error bound (fraction of the tile extent).
    pub max_error: f32,
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
    /// Error bound of LOD2 surfaces (drawn beyond the second LOD distance),
    /// when looser than `max_error`.
    #[serde(default)]
    pub lod2_max_error: Option<f32>,
    /// LOD2 surfaces may collapse across UV and normal seams and drop small
    /// disconnected parts (meshoptimizer `Permissive | Prune`): far geometry
    /// is otherwise held at its seams.
    #[serde(default)]
    pub lod2_permissive: bool,
    /// The same for LOD1 surfaces (drawn from the first LOD distance on).
    #[serde(default)]
    pub lod1_permissive: bool,
    /// Keep every open edge of LOD0 and LOD-less surfaces in place
    /// (meshoptimizer `LockBorder`): models
    /// meet other models and the terrain along open edges (cave mouths,
    /// walls on the ground), and a moved edge opens a crack the sky shows
    /// through.
    #[serde(default)]
    pub lock_borders: bool,
    /// Target ratio for LOD0 and LOD-less surfaces (None: kept as authored).
    #[serde(default)]
    pub lod0_ratio: Option<f32>,
    /// Their error bound (fraction of the mesh extent); default `max_error`.
    #[serde(default)]
    pub lod0_max_error: Option<f32>,
    /// Their size threshold; default `min_triangles`.
    #[serde(default)]
    pub lod0_min_triangles: Option<usize>,
    /// Models without LODs whose LOD0 exceeds this many triangles get
    /// generated LOD1/LOD2 surfaces at the two ratios.
    pub generate_above: usize,
}

impl Simplify {
    fn options(&self, lod: usize) -> meshopt::SimplifyOptions {
        let mut options = if (lod >= 2 && self.lod2_permissive) || (lod == 1 && self.lod1_permissive) {
            meshopt::SimplifyOptions::Permissive | meshopt::SimplifyOptions::Prune
        } else {
            meshopt::SimplifyOptions::None
        };
        // Near geometry only: far LODs (beyond the second distance) would
        // keep most of their triangles at their seams.
        if self.lock_borders && lod == 0 {
            options |= meshopt::SimplifyOptions::LockBorder;
        }
        options
    }
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialConstants {
    pub prefix: String,
    pub g_stage: BTreeMap<String, f32>,
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
    /// Centre and radius of the map-wide sphere probe 0.26 lights the open
    /// ground with (0.69d has none: its outdoor probes are boxes).
    pub global_probe_position: [f32; 3],
    pub global_probe_radius: f32,
    /// What the probes draw.
    pub probes: MapProbes,
    /// Which donor respawn folder supplies the spawns (TDM).
    pub spawn_folder: String,
    /// Donor mode options whose `ui.map_name` names the map.
    pub donor_mode: String,
    /// `mode.ui.loading_screen_id`: the lobby_menu.swf slot that shows the
    /// donor's loading screen (filled by the UI bundle).
    #[serde(default)]
    pub loading_screen_id: Option<u8>,
    /// `mode.match_making_params.map_id`, the level number the lobby keys
    /// names, room tiles and play counts by; default the template's. A port
    /// of a map 0.26 already has (0.69d Rudnya over level_01) takes its own.
    #[serde(default)]
    pub level_id: Option<u8>,
    /// `mode.ui.map_name`: a key the target's dictionaries already have (the
    /// server's content adds it), instead of the donor's key with its texts
    /// copied in. For a map whose donor key 0.26 uses for another map.
    #[serde(default)]
    pub map_name: Option<String>,
}

/// A lobby scene: the project the lobby menu loads behind the character (0.26
/// loads `lobby_scene/day`; another worker picks among these). It is a client
/// project only: no server project, mode or registration. The donor's
/// character stands where 0.26's does (the origin, facing -z: both builds'
/// `gameplay/lobby_menu.options` put `local_player` at (0, 0, 0), rotation
/// 180), so the scene keeps the donor's coordinates.
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LobbySpec {
    /// The donor's lobby project.
    pub donor_project: String,
    /// The native lobby whose objects are the templates (environment, probes,
    /// lights), and whose ambient lights' median the donor's are scaled to.
    pub template_project: String,
    /// Output project directory under `projects/` (`lobby_scene/<name>`).
    pub output_project: String,
    /// Folder under the level's `vegetation/` for this lobby's instance
    /// files: every lobby is in `projects/lobby_scene/`, and the donors name
    /// theirs alike (`vegetation/green.veg`).
    pub vegetation_folder: String,
    /// How the donor's environment probes are drawn.
    pub probes: LobbyProbes,
    /// Donor model-name prefixes whose instances are left out (a seasonal
    /// event's dressing).
    #[serde(default)]
    pub exclude_models: Vec<String>,
    /// Where a weapon's middle floats in its upgrade window (skhid-srp's
    /// client patch 45 reads the project's `srp_lobby.weapon`): in the clear
    /// above a table.
    pub weapon: [f32; 3],
    /// The camera's yaw there, in the 0.26 lobby camera's convention
    /// (forward = (-sin yaw, 0, cos yaw)): `atan2(-dx, dz)` of the direction
    /// it looks along (`srp_lobby.weapon_view`).
    pub weapon_view: f32,
    /// Invisible walls for the lobby camera: its distance (1.5..4.6 m) is
    /// cut by a ray against the project's static collision, so a wall whose
    /// collision has an opening (a window, a doorway) lets the eye out of a
    /// small room. Each is a box of 0.26's unit primitive
    /// (`editor/walker_collision`) scaled to these half extents.
    #[serde(default)]
    pub camera_walls: Vec<CameraWall>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CameraWall {
    pub centre: [f32; 3],
    pub half_extents: [f32; 3],
}

/// 0.26's unit box collision (half extents 1), scaled per instance.
const UNIT_BOX_COLLISION: &str = "resources/models/editor/walker_collision.model/collision";

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LobbyProbes {
    /// The donor's own cubemaps, imported with their volumes. 0.46c2 bakes
    /// probes as 0.26 does: a DXT5 cubemap and its `_diffuse` irradiance.
    Donor,
    /// The donor's volumes, drawn with this template probe's (`full_name`)
    /// cubemaps: 0.69d probes are encoded for its exposure pipeline and carry
    /// their irradiance as SH coefficients, with no `_diffuse` cubemap.
    Template { native: String },
}

/// What one manifest ports, parsed from its `map` / `lobby` keys.
#[derive(Clone)]
pub enum Port {
    Map(MapSpec),
    Lobby(LobbySpec),
}

impl Port {
    fn donor_project(&self) -> &str {
        match self {
            Self::Map(m) => &m.donor_project,
            Self::Lobby(l) => &l.donor_project,
        }
    }
    fn template_project(&self) -> &str {
        match self {
            Self::Map(m) => &m.template_project,
            Self::Lobby(l) => &l.template_project,
        }
    }
    fn output_project(&self) -> &str {
        match self {
            Self::Map(m) => &m.output_project,
            Self::Lobby(l) => &l.output_project,
        }
    }
    fn excluded(&self, model: &str) -> bool {
        match self {
            Self::Map(_) => false,
            Self::Lobby(l) => l.exclude_models.iter().any(|p| model.starts_with(p.as_str())),
        }
    }
}

impl Manifest {
    pub fn port(&self) -> Result<Port> {
        match (&self.map, &self.lobby) {
            (Some(map), None) => Ok(Port::Map(map.clone())),
            (None, Some(lobby)) => Ok(Port::Lobby(lobby.clone())),
            _ => bail!("a manifest ports one map (`map`) or one lobby (`lobby`)"),
        }
    }
    /// The folder imported resources take under each family.
    pub fn namespace(&self) -> String {
        self.namespace.clone().unwrap_or_else(|| format!("ported/{}", self.source_build.prefix()))
    }
    /// 0.69d stores light in photometric units (lux, lumen); 0.46c2 in 0.26's
    /// unitless intensities.
    fn photometric(&self) -> bool {
        self.source_build == SourceBuild::V069d
    }
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
/// not stored by 0.69d and take the canonical values. 0.46c2 already packs
/// the basis as 0.26 does (the same mesh is byte-identical in both), so only
/// its chunks change. Chunks 110, 113 (0.46c2) and 114 (0.69d) are mesh
/// statistics and are dropped; any other unknown chunk is an error.
pub fn geometry(bytes: &[u8], build: SourceBuild) -> Result<Vec<u8>> {
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
    let float_basis = build == SourceBuild::V069d;
    for vertex in result[12..8 + size].chunks_exact_mut(stride).filter(|_| float_basis) {
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
            110 | 113 | 114 => {}
            other => bail!("unknown {} geometry chunk {other}", build.label()),
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
/// A surface name as a path component without spaces or signs: 0.69d names
/// some by their Maya material (`material #25`), which 0.26's own surfaces
/// never are and skhid-srp's release tools (space-separated lists) cannot
/// carry. Letters, digits and `_` stay; every other run becomes one `_`.
fn plain_surface(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out
}

/// A texture name (or path) without whitespace: 0.69d has `tiled/floor/floor_plate_tile `,
/// which skhid-srp's release tools (space-separated lists) cannot carry. Each
/// whitespace character becomes `_`.
fn plain_texture(name: &str) -> String {
    name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect()
}

/// A model file's path with its surface folder (`.model/render/<surface>/`) plain.
fn plain_render_path(path: &str) -> String {
    let Some((model, rest)) = path.split_once(".model/render/") else { return path.to_owned() };
    match rest.split_once('/') {
        Some((surface, file)) => format!("{model}.model/render/{}/{file}", plain_surface(surface)),
        None => path.to_owned(),
    }
}

/// The surfaces a model's settings name (materials, LOD lists), plain as
/// their folders (`plain_render_path`).
fn plain_surfaces(settings: &mut ConfigValue) {
    if let Some(ConfigValue::Named(entries)) = settings.named_mut("material_settings") {
        for entry in entries {
            entry.name = plain_surface(&entry.name);
        }
    }
    if let Some(ConfigValue::Named(lods)) = settings.named_mut("lod_hierrarchy") {
        for lod in lods {
            if let Some(ConfigValue::Indexed(surfaces)) = lod.value.named_mut("surfaces") {
                for surface in surfaces {
                    if let ConfigValue::String(name) = surface {
                        *name = plain_surface(name);
                    }
                }
            }
        }
    }
}

/// A donor model's settings, its LOD surface lists as 0.26 writes them.
fn model_settings(db: &mut ResourceDb, path: &str, build: SourceBuild) -> Result<ConfigValue> {
    let mut c = cfg(db, path)?;
    if build == SourceBuild::V046c2 {
        lod_lists(&mut c);
    }
    Ok(c)
}

/// 0.46c2 writes a LOD's surfaces as a table of names with empty values
/// (`LOD0.surfaces.bushsg = ""`); 0.26 and 0.69d as a list of names, which
/// 0.26 requires. Lists are left as they are.
fn lod_lists(settings: &mut ConfigValue) {
    let Some(ConfigValue::Named(lods)) = settings.named_mut("lod_hierrarchy") else { return };
    for lod in lods {
        let Some(surfaces) = lod.value.named_mut("surfaces") else { continue };
        if let ConfigValue::Named(names) = surfaces {
            let list = names.iter().map(|n| ConfigValue::String(n.name.clone())).collect();
            *surfaces = ConfigValue::Indexed(list);
        }
    }
}

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

/// Largest merged surface. 0.26 loads a static surface's buffers on the
/// main thread's stack (`static_render_surface::load` overflows it with a
/// 1.2 MB surface); native surfaces stop at 35 001 vertices, 980 084 bytes
/// of vertex data and 103 935 indices.
pub const MAX_SURFACE_VERTICES: usize = 32_000;
pub const MAX_SURFACE_VERTEX_BYTES: usize = 900_000;
pub const MAX_SURFACE_INDICES: usize = 96_000;

/// Concatenate static meshes of one stride (0.26 layout, world space) into
/// as few meshes as the native surface limits allow. Returns each merged
/// mesh with its bounds.
pub fn concat_meshes(meshes: &[Vec<u8>]) -> Result<Vec<(Vec<u8>, [f32; 3], [f32; 3])>> {
    let mut out = Vec::new();
    let mut current: Option<(usize, Vec<u8>, usize, Vec<u16>, [f32; 3], [f32; 3])> = None;
    let finish = |c: (usize, Vec<u8>, usize, Vec<u16>, [f32; 3], [f32; 3]), out: &mut Vec<(Vec<u8>, [f32; 3], [f32; 3])>| -> Result<()> {
        let (_, vertices, count, indices, lo, hi) = c;
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend(u32::try_from(4 + vertices.len())?.to_le_bytes());
        b.extend(u32::try_from(count)?.to_le_bytes());
        b.extend(vertices);
        b.extend(4u32.to_le_bytes());
        b.extend(u32::try_from(4 + indices.len() * 2)?.to_le_bytes());
        b.extend(u32::try_from(indices.len())?.to_le_bytes());
        for i in indices {
            b.extend(i.to_le_bytes());
        }
        out.push((b, lo, hi));
        Ok(())
    };
    for mesh in meshes {
        let s = stride(mesh).context("not a static mesh")?;
        let u = |o: usize| u32::from_le_bytes(mesh[o..o + 4].try_into().unwrap()) as usize;
        let (size, count) = (u(4), u(8));
        let vertices = &mesh[12..8 + size];
        let at = 8 + size;
        ensure!(u(at) == 4, "expected index chunk");
        let n = u(at + 8);
        // A mesh at the native maximum (0.69d splits like 0.26 does) stays a
        // surface of its own; merged ones stay under the limits.
        ensure!(count <= 35_001 && count * s <= 980_084 && n <= 103_935, "a {count}-vertex, {n}-index mesh exceeds the largest native surface");
        if current.as_ref().is_some_and(|c| {
            c.0 != s || c.2 + count > MAX_SURFACE_VERTICES || (c.2 + count) * s > MAX_SURFACE_VERTEX_BYTES || c.3.len() + n > MAX_SURFACE_INDICES
        }) {
            finish(current.take().unwrap(), &mut out)?;
        }
        let c = current.get_or_insert_with(|| (s, Vec::new(), 0, Vec::new(), [f32::MAX; 3], [f32::MIN; 3]));
        for v in vertices.chunks_exact(s) {
            for i in 0..3 {
                let x = f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap());
                c.4[i] = c.4[i].min(x);
                c.5[i] = c.5[i].max(x);
            }
        }
        c.1.extend_from_slice(vertices);
        for k in 0..n {
            let i = u16::from_le_bytes([mesh[at + 12 + 2 * k], mesh[at + 13 + 2 * k]]) as usize;
            c.3.push(u16::try_from(c.2 + i)?);
        }
        c.2 += count;
    }
    if let Some(c) = current {
        finish(c, &mut out)?;
    }
    Ok(out)
}

/// Geometry key of a model's LOD0 surfaces (0.26 layout, sorted by surface
/// name) up to a translation: positions relative to the first vertex at
/// 1 mm, the remaining vertex bytes except the packed basis (exports
/// differ by one in its rounding), and the set of triangles (their order
/// differs between exports). Returns the key and the first vertex.
pub fn translation_key(surfaces: &[(String, Vec<u8>)]) -> Option<(Vec<u8>, [f32; 3])> {
    let mut key = Vec::new();
    let mut origin = None;
    for (name, mesh) in surfaces {
        let s = stride(mesh)?;
        let u = |o: usize| u32::from_le_bytes(mesh[o..o + 4].try_into().unwrap()) as usize;
        let (size, count) = (u(4), u(8));
        let at = 8 + size;
        if mesh.get(at..at + 4)? != 4u32.to_le_bytes() {
            return None;
        }
        let n = u(at + 8);
        let o = *origin.get_or_insert_with(|| std::array::from_fn::<f32, 3, _>(|i| f32::from_le_bytes(mesh[12 + 4 * i..16 + 4 * i].try_into().unwrap())));
        key.extend(name.as_bytes());
        key.extend((count as u32).to_le_bytes());
        for v in mesh[12..8 + size].chunks_exact(s) {
            for i in 0..3 {
                let x = f32::from_le_bytes(v[4 * i..4 * i + 4].try_into().unwrap()) - o[i];
                key.extend(((x * 1000.0).round() as i32).to_le_bytes());
            }
            key.extend(&v[24..]);
        }
        let mut triangles: Vec<[u16; 3]> = (0..n / 3)
            .map(|t| {
                let mut tri: [u16; 3] = std::array::from_fn(|k| u16::from_le_bytes([mesh[at + 12 + 2 * (3 * t + k)], mesh[at + 13 + 2 * (3 * t + k)]]));
                // Same winding, any rotation.
                let r = (0..3).min_by_key(|&k| tri[k]).unwrap();
                tri.rotate_left(r);
                tri
            })
            .collect();
        triangles.sort();
        for t in triangles {
            for i in t {
                key.extend(i.to_le_bytes());
            }
        }
    }
    Some((key, origin?))
}

/// Bounds of a static mesh's positions.
fn mesh_bounds(mesh: &[u8]) -> Option<([f32; 3], [f32; 3])> {
    let s = stride(mesh)?;
    let size = u32::from_le_bytes(mesh[4..8].try_into().ok()?) as usize;
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in mesh.get(12..8 + size)?.chunks_exact(s) {
        for i in 0..3 {
            let x = f32::from_le_bytes(v[4 * i..4 * i + 4].try_into().unwrap());
            lo[i] = lo[i].min(x);
            hi[i] = hi[i].max(x);
        }
    }
    Some((lo, hi))
}

/// One render instance offered to a material cluster: its placement and,
/// per LOD (0..3, an empty LOD falls back to the one below), its surfaces as
/// (material, model-space mesh).
pub struct ClusterMember {
    pub placement: Placement,
    pub lods: [Vec<(String, Vec<u8>)>; 3],
}

/// Bake cluster members into world space and merge their surfaces by
/// material per LOD. With `lods` false every member is LOD-less and only
/// LOD0 is produced.
pub fn merge_cluster(members: &[ClusterMember], lods: bool) -> Result<Vec<Vec<(String, Vec<u8>, [f32; 3], [f32; 3])>>> {
    let levels = if lods { 3 } else { 1 };
    let mut out = Vec::new();
    for level in 0..levels {
        let mut by_material: BTreeMap<(String, usize), Vec<Vec<u8>>> = BTreeMap::new();
        for member in members {
            let surfaces = (0..=level).rev().map(|l| &member.lods[l]).find(|s| !s.is_empty()).context("member without LOD0")?;
            for (material, mesh) in surfaces {
                let (baked, _, _) = bake_instances(mesh, &[member.placement])?;
                by_material.entry((material.clone(), stride(mesh).context("not static")?)).or_default().push(baked);
            }
        }
        let mut merged = Vec::new();
        for ((material, _), meshes) in by_material {
            for (mesh, lo, hi) in concat_meshes(&meshes)? {
                merged.push((material.clone(), mesh, lo, hi));
            }
        }
        out.push(merged);
    }
    Ok(out)
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
    set(&mut result, 28, mips - 1);
    result.extend_from_slice(&bytes[128 + top..]);
    // Native pitch: every byte after the header (mips + colour-space byte).
    let after = u32::try_from(result.len() - 128)?;
    set(&mut result, 20, after);
    Ok(result)
}

/// Retail 0.26 textures: flags 0xA100F and dwPitchOrLinearSize = every byte
/// after the 128-byte header, which is the mip chain (all faces) plus one
/// trailing colour-space byte. 0.26's streamer locates mips from that field
/// and reads the tail one byte early otherwise (colour noise on distant mips).
/// Checks the layout and writes the native flags and pitch.
pub fn canonical_dds(bytes: &mut [u8]) -> Result<()> {
    ensure!(bytes.len() > 128 && &bytes[..4] == b"DDS ", "not a DDS");
    let (width, height, mips, fourcc) = dds_info(bytes)?;
    let block = match &fourcc {
        b"DXT1" => 8,
        b"DXT3" | b"DXT5" => 16,
        other => bail!("unsupported texture format {:?}", String::from_utf8_lossy(other)),
    };
    let caps2 = u32::from_le_bytes(bytes[112..116].try_into().unwrap());
    let faces = if caps2 & 0xfe00 != 0 { 6 } else { 1 };
    let chain: usize = (0..mips.max(1)).map(|m| (((width >> m).max(1).div_ceil(4)) * ((height >> m).max(1).div_ceil(4)) * block) as usize).sum();
    let after = bytes.len() - 128;
    ensure!(after == chain * faces + 1, "DDS layout: {after} bytes after the header, the mip chain needs {} plus the colour-space byte", chain * faces);
    bytes[8..12].copy_from_slice(&0xA100Fu32.to_le_bytes());
    bytes[20..24].copy_from_slice(&u32::try_from(after)?.to_le_bytes());
    Ok(())
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

fn effect_value<'a>(root: &'a ConfigValue, stage: &str, name: &str) -> Option<&'a ConfigValue> {
    value_of(root.named("material")?.named(stage)?.named("effect")?, name)
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
    // A metalness map (0.69d's `_fmap`, as 0.26's metals name theirs) takes
    // the category's `_metal` template where the manifest has one: 0.26's
    // Fresnel map, which darkens the diffuse by f0 and tints the specular by
    // the albedo's hue above f0 0.5 (`composition.ps`).
    let metalness = g_stage(donor)
        .and_then(|s| s.named("effect"))
        .filter(|e| flag(e, "use_t_metalness"))
        .and_then(|e| text(e, "t_metalness"))
        .map(str::to_owned);
    let metal_key = format!("{}_metal", if usage.vertex_colour { vc_key.as_str() } else { category.key() });
    let metal_template = metalness.as_ref().and_then(|_| templates.get(&metal_key));
    let (template_path, template) = metal_template
        .or_else(|| if usage.vertex_colour { templates.get(&vc_key) } else { None })
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
            match (&metalness, metal_template) {
                (Some(map), Some(_)) if effect_flag(&root, stage, "use_tfresnel") => {
                    set_effect(&mut root, stage, "texture_fresnel", ConfigValue::String(map.clone()))?;
                    set_effect(&mut root, stage, "constant_fresnel_min", ConfigValue::Float(0.04))?;
                    set_effect(&mut root, stage, "constant_fresnel_max", ConfigValue::Float(1.0))?;
                    notes.push(format!("metalness map {map} as 0.26's Fresnel map (f0 0.04 to 1)"));
                }
                _ if flag(effect, "use_t_metalness") => {
                    notes.push("metalness map has no 0.26 counterpart for this template; the albedo is drawn as a dielectric".into())
                }
                _ => {}
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

/// 0.46c2 materials still name their maps as 0.26 does (`texture_diffuse`,
/// `use_nmap`, `texture_roughness`: 0.26's gloss channel); 0.69d renamed them
/// (`t_albedo`, `use_t_normal`, `t_smoothness`). Every field 0.69d reads is
/// added under its 0.69d name, so `categorize` and `convert_material` read
/// both builds alike; the manifest's `roughness_scale` is then 1 (the values
/// are 0.26's already).
pub fn vocabulary_046(root: &mut ConfigValue) {
    const NAMES: [(&str, &str); 14] = [
        ("texture_diffuse", "t_albedo"),
        ("use_tdiffuse", "use_t_albedo"),
        ("texture_normal", "t_normal"),
        ("use_nmap", "use_t_normal"),
        ("texture_roughness", "t_smoothness"),
        ("use_troughness", "use_t_smoothness"),
        ("constant_roughness", "smoothness"),
        ("constant_roughness_min", "min_smoothness"),
        ("constant_roughness_max", "max_smoothness"),
        ("texture_translucency", "t_translucency"),
        ("use_ttranslucency", "use_t_translucency"),
        ("constant_diffuse", "albedo_color"),
        ("use_tditail_diffuse", "use_t_albedo_detail"),
        ("use_tparallax", "use_parallax"),
    ];
    let Some(ConfigValue::Named(stages)) = root.named_mut("material") else { return };
    for stage in stages {
        let Some(ConfigValue::Named(fields)) = stage.value.named_mut("effect") else { continue };
        for (old, new) in NAMES {
            if fields.iter().any(|f| f.name == new) {
                continue;
            }
            if let Some(field) = fields.iter().find(|f| f.name == old).cloned() {
                fields.push(NamedValue { name: new.into(), value: field.value });
            }
        }
    }
}

/// 0.46c2 exports surfaces without a game material (render-only background
/// models) with an empty name and id 0; 0.26 writes its 65535 sentinel.
fn unassigned_046(settings: &mut ConfigValue) {
    let Some(ConfigValue::Named(entries)) = settings.named_mut("game_material_settings") else { return };
    for entry in entries {
        if matches!(entry.value.named("game_material_name"), Some(ConfigValue::String(n)) if n.is_empty()) {
            if let Some(id) = entry.value.named_mut("game_material_id") {
                *id = ConfigValue::Integer(65_535);
            }
        }
    }
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

/// A map's environment probes.
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MapProbes {
    /// The donor's volumes on native cubemaps: the template's largest sphere
    /// probe over the map, and per donor folder a template probe
    /// (`classes`, first prefix wins). Matte water (0.69d smoothness below
    /// 0.5: cave puddles) reflects the template probe `matte_water`.
    Native {
        classes: Vec<ProbeClass>,
        #[serde(default)]
        matte_water: Option<String>,
        /// Keep 0.69d ambient lights, scaled so their median matches the
        /// template's (off by default: 0.26's only darken).
        #[serde(default)]
        ambient_lights: bool,
    },
    /// The donor's own probes, re-encoded for 0.26 (`DonorProbe`): every
    /// volume with its cubemap, the irradiance cubemap 0.26 reads evaluated
    /// from its SH coefficients, both in one scale with the sun
    /// (`LightScale`). 0.26 has no sky light, so the donor probe `global`
    /// (`full_name`) is also the map-wide sphere. Matte water reflects the
    /// donor probe `matte_water`.
    Donor {
        global: String,
        #[serde(default)]
        matte_water: Option<String>,
    },
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeClass {
    pub prefix: String,
    /// `full_name` of a probe in the template project.
    pub native: String,
}

/// A 0.69d probe's lighting data (`lighting::DonorProbe`).
fn donor_probe(object: &ConfigValue) -> Result<DonorProbe> {
    let full_name = string(object, "full_name").unwrap_or("environment_probe");
    let texture = string(object, "texture").with_context(|| format!("{full_name}: no texture"))?.replace('\\', "/");
    let mut sh = [[0.0; 3]; 9];
    for (i, c) in sh.iter_mut().enumerate() {
        *c = float3(object, &format!("sh_coefficient_{i}")).with_context(|| format!("{full_name}: SH coefficient {i}"))?;
    }
    Ok(DonorProbe {
        texture,
        capture_exposure_value: real(object, "capture_exposure_value").unwrap_or(13.0),
        directional_factor: real(object, "directional_factor").unwrap_or(0.5),
        sh,
    })
}

/// A probe cubemap pair made for 0.26: `textures/<ns>/<texture>.dds` from
/// the donor probe `specular` (its cube from mip `specular_mip`, so a fill
/// light's volume reuses its host's small), and `<texture>_diffuse.dds`
/// from `diffuse`.
#[derive(Clone, Debug)]
pub struct GeneratedProbe {
    /// The volume's `full_name` in the scene.
    pub full_name: String,
    pub texture: String,
    pub specular: DonorProbe,
    pub specular_mip: usize,
    pub diffuse: DiffuseRecipe,
    pub scale: LightScale,
}

/// One 0.26 probe volume of a donor-lit map.
struct LitVolume {
    full_name: String,
    volume: Volume,
    texture: String,
    specular: DonorProbe,
    specular_mip: usize,
    specular_multiplier: f32,
    diffuse: DiffuseRecipe,
}

struct DonorLighting {
    /// In 0.26's drawing order (largest first).
    volumes: Vec<LitVolume>,
    /// Darkening multiplicative lights: (full name, light, sRGB colour,
    /// attenuation power), which 0.26's ambient lights reproduce.
    darkeners: Vec<(String, DonorLight, [f32; 3], f32)>,
    light_volumes: usize,
}

/// A fill light's volume reuses its host probe's cube from this mip (32
/// texels): its own cube only carries its irradiance.
const FILL_SPECULAR_MIP: usize = 3;

/// The volume of a 0.69d probe or ambient light (scale = half extents).
fn donor_volume(object: &ConfigValue, side_width_field: &str) -> Result<Volume> {
    let scale = float3(object, "scale")?;
    let shape = match integer(object, "geometry").unwrap_or(0) {
        0 | 3 => Shape::Sphere { radius: scale.iter().cloned().fold(0.0, f32::max) },
        2 => Shape::Cylinder { radius: scale[0].max(scale[2]), half_height: scale[1] },
        _ => Shape::Box { half: scale },
    };
    Ok(Volume {
        centre: float3(object, "position")?,
        yaw: float3(object, "rotation")?[1],
        shape,
        side_width: real(object, side_width_field).unwrap_or(0.5),
    })
}

/// A 0.69d ambient light: type 0 adds `illuminance` (klux) as
/// `illuminance * 1000 / pi * colour^2.2` (cd/m^2 off a white surface),
/// type 1 multiplies by `multiplier * colour^2.2` (`ambient_light_*.ps`).
fn donor_light(object: &ConfigValue) -> Result<(DonorLight, [f32; 3])> {
    let colour = float3(object, "color").unwrap_or([1.0; 3]);
    let linear = colour.map(|c| c.max(0.0).powf(2.2));
    let effect = if integer(object, "type") == Some(1) {
        let m = real(object, "multiplier").unwrap_or(1.0);
        LightEffect::Multiply { factor: linear.map(|c| c * m) }
    } else {
        let e = real(object, "illuminance").unwrap_or(1.0) * 1000.0 / std::f32::consts::PI;
        LightEffect::Add { radiance: linear.map(|c| c * e) }
    };
    let facing = matches!(object.named("dot_normal"), Some(ConfigValue::Boolean(true)))
        .then(|| (real(object, "dot_normal_cutoff").unwrap_or(0.0), real(object, "dot_normal_power").unwrap_or(1.0)));
    let light = DonorLight { volume: donor_volume(object, "side_width")?, effect, attenuation_power: real(object, "attenuation_power").unwrap_or(1.0), facing };
    Ok((light, colour))
}

/// Put a 0.26 probe or ambient light on `volume`.
fn place_volume(native: &mut ConfigValue, volume: &Volume, full_name: &str) -> Result<()> {
    native.set_path("position", ConfigValue::Float3(volume.centre))?;
    native.set_path("rotation", ConfigValue::Float3([0.0, volume.yaw, 0.0]))?;
    native.set_path("pivot", ConfigValue::Float3([0.0; 3]))?;
    let (geometry, scale, radius) = match volume.shape {
        Shape::Sphere { radius } => (0, [1.0; 3], radius),
        Shape::Box { half } => (1, half, volume.sort_radius()),
        Shape::Cylinder { radius, half_height } => (1, [radius, half_height, radius], volume.sort_radius()),
    };
    native.set_path("geometry", ConfigValue::Integer(geometry))?;
    native.set_path("scale", ConfigValue::Float3(scale))?;
    native.set_path("radius", ConfigValue::Float(radius))?;
    if native.named("probe_side_width").is_some() {
        native.set_path("probe_side_width", ConfigValue::Float(volume.side_width))?;
    }
    let name = full_name.rsplit('/').next().unwrap_or(full_name);
    native.set_path("name", ConfigValue::String(name.into()))?;
    native.set_path("full_name", ConfigValue::String(full_name.into()))?;
    if native.named("is_temporary").is_some() {
        native.set_path("is_temporary", ConfigValue::Boolean(false))?;
    }
    Ok(())
}

/// A texture name without signs (the porter's surface folders avoid them).
fn plain_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// 0.69d's probes and ambient lights as 0.26 probe volumes in drawing
/// order. Every donor probe keeps its volume and cube; the map-wide sphere
/// takes `global`'s. An additive (or brightening) light becomes a volume
/// with its host's cube (the last volume drawn under its centre). 0.26
/// draws each volume over the ones before it, so a volume's irradiance is
/// 0.69d's light at its centre: the donor probe there (its own, or its
/// host's) and every light drawn so far, itself included, by their falloff
/// there; lights drawn later add themselves over it.
fn donor_lighting(probes: &[&ConfigValue], objects: &[ConfigValue], global: &str, map: &MapSpec) -> Result<DonorLighting> {
    enum Source {
        Probe(DonorProbe, f32),
        Light(DonorLight),
    }
    let mut pending: Vec<(String, Volume, Source)> = Vec::new();
    let global_object = probes
        .iter()
        .find(|o| string(o, "full_name") == Some(global))
        .with_context(|| format!("donor has no probe {global}"))?;
    let map_wide = Volume { centre: map.global_probe_position, yaw: 0.0, shape: Shape::Sphere { radius: map.global_probe_radius }, side_width: 0.5 };
    pending.push(("env_and_lights/probes/global".into(), map_wide, Source::Probe(donor_probe(global_object)?, 1.0)));
    for object in probes {
        let full_name = string(object, "full_name").unwrap_or("environment_probe").to_owned();
        // 0.69d: specular_multiplier -1 turns the cube's reflections off.
        let specular = real(object, "specular_multiplier").unwrap_or(1.0).max(0.0);
        pending.push((full_name, donor_volume(object, "probe_side_width")?, Source::Probe(donor_probe(object)?, specular)));
    }
    let mut darkeners = Vec::new();
    for object in objects.iter().filter(|o| kind(o) == "ambient_light") {
        if matches!(object.named("is_enabled"), Some(ConfigValue::Boolean(false))) {
            continue;
        }
        let full_name = string(object, "full_name").unwrap_or("ambient_light").to_owned();
        let (light, colour) = donor_light(object)?;
        match light.effect {
            LightEffect::Multiply { factor } if factor.iter().all(|f| *f <= 1.0) => {
                darkeners.push((full_name, light, colour, light.attenuation_power));
            }
            _ => pending.push((full_name, light.volume, Source::Light(light))),
        }
    }
    // 0.26's order: larger first; the map-wide sphere stays first.
    pending[1..].sort_by(|a, b| b.1.sort_radius().total_cmp(&a.1.sort_radius()));
    let mut volumes: Vec<LitVolume> = Vec::new();
    let mut lights_drawn: Vec<DonorLight> = Vec::new();
    let mut names = BTreeSet::new();
    let mut light_volumes = 0;
    for (full_name, volume, source) in pending {
        // What 0.69d lights the volume's centre with: the donor probe there
        // and every light drawn so far (larger ones, and the light itself).
        let terms = |lights: &[DonorLight]| -> Vec<Term> {
            lights
                .iter()
                .filter_map(|l| {
                    let weight = l.volume.weight(volume.centre, l.attenuation_power);
                    (weight > 0.0).then_some(Term { light: *l, at: volume.centre, weight })
                })
                .collect()
        };
        let lit = match source {
            Source::Probe(probe, specular_multiplier) => {
                let base = probe.texture.rsplit('/').next().unwrap_or(&probe.texture);
                let texture = if volumes.is_empty() { "environment_probes/map_wide".to_owned() } else { format!("environment_probes/{}", plain_name(base)) };
                let terms = terms(&lights_drawn);
                LitVolume { full_name, volume, texture, specular: probe.clone(), specular_mip: 0, specular_multiplier, diffuse: DiffuseRecipe { base: probe, terms } }
            }
            Source::Light(light) => {
                let host = volumes
                    .iter()
                    .rev()
                    .find(|v| v.volume.contains(light.volume.centre))
                    .context("a light outside the map-wide probe")?;
                lights_drawn.push(light);
                light_volumes += 1;
                let texture = format!("environment_probes/fill/{}", plain_name(full_name.trim_start_matches("environment_and_lights/")));
                LitVolume {
                    full_name,
                    volume,
                    texture,
                    specular: host.specular.clone(),
                    specular_mip: FILL_SPECULAR_MIP,
                    specular_multiplier: host.specular_multiplier,
                    diffuse: DiffuseRecipe { base: host.diffuse.base.clone(), terms: terms(&lights_drawn) },
                }
            }
        };
        let mut lit = lit;
        let stem = lit.texture.clone();
        let mut n = 1;
        while !names.insert(lit.texture.clone()) {
            n += 1;
            lit.texture = format!("{stem}_{n}");
        }
        volumes.push(lit);
    }
    Ok(DonorLighting { volumes, darkeners, light_volumes })
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
    /// Vegetation instance files relative to the level directory: the
    /// donor's name -> the emitted one.
    pub vegetation_files: BTreeMap<String, String>,
    /// Donor cubemaps the scene's probes name (lobbies with `LobbyProbes::Donor`).
    pub probe_textures: BTreeSet<String>,
    /// Probe cubemaps made for 0.26 (maps with `MapProbes::Donor`).
    pub generated_probes: Vec<GeneratedProbe>,
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

/// Build the 0.26 scene from the donor's scene (a 0.69d map's TDM scene, or a
/// lobby) and the native template. `photometric`: the donor's lights are in
/// lux and lumen (0.69d), not 0.26's unitless intensities (0.46c2).
pub fn convert_scene(
    donor: &ConfigValue,
    template: &ConfigValue,
    port: &Port,
    photometric: bool,
    particle_replacements: &BTreeMap<String, String>,
) -> Result<SceneReport> {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut notes = Vec::new();
    let mut models = BTreeSet::new();
    let mut count = |k: &str| *counts.entry(k.to_owned()).or_default() += 1;

    // Render and collision instances.
    let mut visuals = Vec::new();
    let mut excluded = 0usize;
    for visual in array(donor.named("render_visuals").context("no render_visuals")?)? {
        let model = string(visual, "model_name").context("render visual without model_name")?;
        if port.excluded(&model_of_reference(model)) {
            excluded += 1;
            continue;
        }
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
        if port.excluded(&model_of_reference(model)) {
            continue;
        }
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
    let mut vegetation_files = BTreeMap::new();
    let mut probe_textures = BTreeSet::new();
    let mut generated_probes = Vec::<GeneratedProbe>::new();
    let mut dropped = BTreeMap::<String, usize>::new();
    if excluded > 0 {
        notes.push(format!("left out: {excluded} render instances of the excluded models and their collision"));
    }

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
        "environment: native {} sky, clouds, fog and post-process; the donor's sun direction (elevation {elevation}, azimuth {azimuth}); its exposure, colour grading, light shafts and fog volumes have no 0.26 counterpart",
        port.template_project()
    ));
    // A donor-lit map: the sun in the probes' scale. 0.26 values are 0.69d's
    // exposed values at its outdoor exposure (the auto exposure's ceiling,
    // where a sunlit view sits): the sky the native atmosphere draws and the
    // probes then agree within a few percent, and 0.26's eye adaptation and
    // curve show sunlit and shaded grey as 0.69d's curve does.
    let light_scale = match port {
        Port::Map(MapSpec { probes: MapProbes::Donor { .. }, .. }) => {
            let ev = real(donor_environment, "max_auto_exposure_value").context("max_auto_exposure_value")?;
            let scale = LightScale(1.0 / (1.2 * 2f32.powf(ev)));
            let sun = super::lighting::sun_radiance(
                real(donor_environment, "sun_intensity_r2").context("sun_intensity_r2")?,
                real(donor_environment, "air_density").context("air_density")?,
                real(donor_environment, "air_turbidity").context("air_turbidity")?,
                elevation,
            )
            .map(|v| v * scale.0);
            let intensity = sun.iter().cloned().fold(0.0, f32::max);
            environment.set_path("properties.sun_color", ConfigValue::Float4([sun[0] / intensity, sun[1] / intensity, sun[2] / intensity, 1.0]))?;
            environment.set_path("properties.sun_intensity", ConfigValue::Float(intensity))?;
            // 0.69d has no height-based ambient; 0.26's multiplies the
            // probes' light (purple below y 10 in native Rudnya).
            environment.set_path("properties.use_height_based_ambient", ConfigValue::Boolean(false))?;
            notes.push(format!(
                "sun: 0.69d's (intensity {}, its atmosphere's transmittance at {elevation} degrees) at exposure EV {ev}: colour {:?}, intensity {intensity:.3}; no height-based ambient",
                real(donor_environment, "sun_intensity_r2").unwrap_or(0.0),
                sun.map(|v| (v / intensity * 1000.0).round() / 1000.0)
            ));
            Some((scale, ev))
        }
        _ => None,
    };
    objects.push(environment);
    count("environment");

    match port {
        Port::Map(map) => {
            // The map-wide sphere: 0.26 has no sky light, so open ground is lit
            // by its largest probe.
            let mut global = templates_of(template, "environment_probe")
                .into_iter()
                .filter(|p| integer(p, "geometry") == Some(0))
                .max_by(|a, b| real(a, "radius").unwrap_or(0.0).total_cmp(&real(b, "radius").unwrap_or(0.0)))
                .context("template has no sphere probe")?
                .clone();
            global.set_path("position", ConfigValue::Float3(map.global_probe_position))?;
            global.set_path("radius", ConfigValue::Float(map.global_probe_radius))?;
            global.set_path("name", ConfigValue::String("global".into()))?;
            global.set_path("full_name", ConfigValue::String("env_and_lights/probes/global".into()))?;
            let donor_probe_objects: Vec<&ConfigValue> = donor_objects
                .iter()
                .filter(|o| kind(o) == "environment_probe")
                .filter(|o| !matches!(o.named("is_enabled"), Some(ConfigValue::Boolean(false))))
                .collect();
            // Each donor volume on a clone of `native`, drawn with `texture`.
            let volume = |native: &ConfigValue, object: &ConfigValue, texture: &str| -> Result<ConfigValue> {
                let full_name = string(object, "full_name").unwrap_or("");
                let mut p = native.clone();
                place(&mut p, object, full_name)?;
                set_probe_texture(&mut p, texture)?;
                let scale = float3(object, "scale")?;
                // 0.69d shapes: 0 sphere, 1 box, 2/3 boxes with other falloffs.
                // 0.26 draws the larger probes first (by radius), each
                // smaller one over them by its falloff.
                let sphere = integer(object, "geometry") == Some(0);
                p.set_path("geometry", ConfigValue::Integer(if sphere { 0 } else { 1 }))?;
                if sphere {
                    p.set_path("radius", ConfigValue::Float(scale.iter().cloned().fold(0.0, f32::max)))?;
                    p.set_path("scale", ConfigValue::Float3([1.0; 3]))?;
                } else {
                    p.set_path("radius", ConfigValue::Float(scale.iter().map(|v| v * v).sum::<f32>().sqrt()))?;
                }
                copy_shared(&mut p, object, &["probe_side_width"])?;
                Ok(p)
            };
            match &map.probes {
                MapProbes::Native { classes, .. } => {
                    let global_texture = probe_texture(&global).context("global probe texture")?;
                    set_probe_texture(&mut global, &global_texture)?;
                    objects.push(global);
                    count("environment_probe");
                    let mut class_probes = Vec::new();
                    for class in classes {
                        let native = templates_of(template, "environment_probe")
                            .into_iter()
                            .find(|p| string(p, "full_name") == Some(class.native.as_str()))
                            .with_context(|| format!("template has no probe {}", class.native))?;
                        class_probes.push((class.prefix.clone(), native.clone(), probe_texture(native).context("probe texture")?));
                    }
                    for object in donor_probe_objects {
                        let full_name = string(object, "full_name").unwrap_or("");
                        let Some((_, native, texture)) = class_probes.iter().find(|(prefix, _, _)| full_name.starts_with(prefix.as_str())) else {
                            continue;
                        };
                        objects.push(volume(native, object, texture)?);
                        count("environment_probe");
                    }
                }
                MapProbes::Donor { global: global_name, .. } => {
                    let (scale, outdoor_ev) = light_scale.context("donor lighting needs the donor's exposure")?;
                    let lit = donor_lighting(&donor_probe_objects, &donor_objects, global_name, map)?;
                    // 0.26 draws probes largest first, each over the ones
                    // before it by its falloff (`environment_probe_lighting`
                    // blends with the weight in alpha). A volume's irradiance
                    // is what lies under its centre plus itself.
                    for v in &lit.volumes {
                        let mut p = global.clone();
                        place_volume(&mut p, &v.volume, &v.full_name)?;
                        set_probe_texture(&mut p, &v.texture)?;
                        p.set_path("diffuse_multiplier", ConfigValue::Float(1.0))?;
                        p.set_path("specular_multiplier", ConfigValue::Float(v.specular_multiplier))?;
                        objects.push(p);
                        count("environment_probe");
                        generated_probes.push(GeneratedProbe {
                            full_name: v.full_name.clone(),
                            texture: v.texture.clone(),
                            specular: v.specular.clone(),
                            specular_mip: v.specular_mip,
                            diffuse: v.diffuse.clone(),
                            scale,
                        });
                    }
                    let ambient_template = first_template(template, "ambient_light")?;
                    for (full_name, light, colour, power) in &lit.darkeners {
                        let mut a = ambient_template.clone();
                        place_volume(&mut a, &light.volume, full_name)?;
                        let LightEffect::Multiply { factor } = light.effect else { unreachable!() };
                        let intensity = (0..3).map(|c| factor[c] / colour[c].powf(2.2).max(1e-4)).fold(0.0f32, f32::max);
                        a.set_path("color", ConfigValue::Integer(packed_colour(*colour)))?;
                        a.set_path("intensity", ConfigValue::Float(intensity))?;
                        a.set_path("attenuation_power", ConfigValue::Float(*power))?;
                        a.set_path("side_width", ConfigValue::Float(light.volume.side_width))?;
                        a.set_path("dot_normal", ConfigValue::Boolean(light.facing.is_some()))?;
                        a.set_path("affect_specular", ConfigValue::Boolean(false))?;
                        objects.push(a);
                        count("ambient_light");
                    }
                    notes.push(format!(
                        "environment probes: the donor's {} volumes with their own cubemaps re-encoded for 0.26 (scale {:.4e}: 0.69d's exposure at EV {}) and irradiance from their SH, {global_name}'s also as the map-wide sphere; {} additive or brightening ambient lights as probe volumes over the probe under them (0.26's ambient lights only darken), {} darkening ones as 0.26 ambient lights",
                        donor_probe_objects.len(),
                        scale.0,
                        outdoor_ev,
                        lit.light_volumes,
                        lit.darkeners.len()
                    ));
                }
            }
        }
        Port::Lobby(lobby) => {
            // The donor's volumes on the template's probe; their cubemaps the
            // donor's own or the template's.
            let native = match &lobby.probes {
                LobbyProbes::Donor => templates_of(template, "environment_probe").into_iter().next(),
                LobbyProbes::Template { native } => templates_of(template, "environment_probe")
                    .into_iter()
                    .find(|p| string(p, "full_name") == Some(native.as_str())),
            }
            .context("template has no such environment probe")?
            .clone();
            let native_texture = probe_texture(&native).context("template probe texture")?;
            for object in donor_objects.iter().filter(|o| kind(o) == "environment_probe") {
                if matches!(object.named("is_enabled"), Some(ConfigValue::Boolean(false))) {
                    continue;
                }
                let full_name = string(object, "full_name").unwrap_or("environment_probe");
                let mut p = native.clone();
                place(&mut p, object, full_name)?;
                let texture = match &lobby.probes {
                    LobbyProbes::Donor => {
                        let t = string(object, "texture").context("donor probe without texture")?.replace('\\', "/");
                        probe_textures.insert(t.clone());
                        // Same model of probe: every value of both builds' schema.
                        copy_shared(
                            &mut p,
                            object,
                            &[
                                "radius", "geometry", "probe_side_width", "diffuse_multiplier", "specular_multiplier", "ambient_multiplier",
                                "affect_diffuse", "smart_attenuation", "use_parallax_correction", "priority", "cubemap_resolution_index",
                            ],
                        )?;
                        t
                    }
                    LobbyProbes::Template { .. } => {
                        let scale = float3(object, "scale")?;
                        let sphere = integer(object, "geometry") == Some(0);
                        p.set_path("geometry", ConfigValue::Integer(if sphere { 0 } else { 1 }))?;
                        if sphere {
                            p.set_path("radius", ConfigValue::Float(scale.iter().cloned().fold(0.0, f32::max)))?;
                            p.set_path("scale", ConfigValue::Float3([1.0; 3]))?;
                        }
                        native_texture.clone()
                    }
                };
                set_probe_texture(&mut p, &texture)?;
                objects.push(p);
                count("environment_probe");
            }
            notes.push(match &lobby.probes {
                LobbyProbes::Donor => format!("environment probes: the donor's volumes and cubemaps ({} cubemaps)", probe_textures.len()),
                LobbyProbes::Template { native } => format!("environment probes: the donor's volumes drawn with {native} of {} ({native_texture})", lobby.template_project),
            });
        }
    }

    let ambient_template = first_template(template, "ambient_light")?;
    let decal_template = first_template(template, "decal")?;
    // Asked for only when the donor has lights (native Rudnya has none).
    let light_template = || {
        templates_of(template, "light")
            .into_iter()
            .find(|l| integer(l, "light_type") == Some(1))
            .or_else(|| templates_of(template, "light").into_iter().next())
            .cloned()
            .context("template has no light")
    };
    // Only maps have resupply zones (a lobby's template has none).
    let resupply_template = || first_template(template, "resupply_zone");
    let collision_geometry_template = || {
        templates_of(template, "collision_geometry")
            .into_iter()
            .find(|c| array(c.named("meshes").unwrap_or(&ConfigValue::Indexed(vec![]))).is_ok_and(|m| m.len() == 1))
            .cloned()
            .context("template has no single-mesh collision geometry")
    };
    /// What becomes of the donor's ambient lights.
    enum AmbientLights {
        Drop,
        Keep,
        /// Already in the probe volumes (`donor_lighting`).
        InProbes,
    }
    let ambient_lights = match port {
        Port::Map(MapSpec { probes: MapProbes::Native { ambient_lights: true, .. }, .. }) | Port::Lobby(_) => AmbientLights::Keep,
        Port::Map(MapSpec { probes: MapProbes::Native { .. }, .. }) => AmbientLights::Drop,
        Port::Map(MapSpec { probes: MapProbes::Donor { .. }, .. }) => AmbientLights::InProbes,
    };
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
            "ambient_light" if matches!(ambient_lights, AmbientLights::Drop) => {
                *dropped.entry("ambient_light (photometric units and shapes 2/3 have no 0.26 equivalent)".into()).or_default() += 1;
            }
            "ambient_light" if matches!(ambient_lights, AmbientLights::InProbes) => {}
            "environment_probe" => {}
            // 0.69d's `type` 1 ambient lights multiply the light under them
            // (the scene's painted contact shadows, a multiplier below 1);
            // 0.26's only add.
            "ambient_light" if photometric && integer(object, "type") == Some(1) => {
                *dropped.entry("multiplying ambient light (0.69d type 1, contact shadows; 0.26's ambient lights only add)".into()).or_default() += 1;
            }
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
                // 0.46c2's are 0.26's intensities.
                let intensity = if photometric {
                    real(object, "illuminance").unwrap_or(1.0)
                        * real(object, "multiplier").unwrap_or(1.0)
                        * real(object, "intensity").unwrap_or(1.0)
                } else {
                    real(object, "intensity").unwrap_or(1.0)
                };
                a.set_path("color", ConfigValue::Integer(packed_colour(colour)))?;
                a.set_path("intensity", ConfigValue::Float(intensity))?;
                // 0.69d sizes a sphere by its scale (radius 5 times the
                // largest scale); 0.26 by its radius at scale 1.
                if photometric && integer(object, "geometry") == Some(0) {
                    let scale = float3(object, "scale")?;
                    let radius = real(object, "radius").unwrap_or(1.0) * scale.iter().cloned().fold(0.0, f32::max);
                    a.set_path("radius", ConfigValue::Float(radius))?;
                    a.set_path("scale", ConfigValue::Float3([1.0; 3]))?;
                }
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
                let mut l = light_template()?;
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
                if photometric {
                    // 0.69d photometric power (lumen); 0.26 intensity is unitless.
                    let power = real(object, "luminous_power").unwrap_or(30.0) * real(object, "intensity").unwrap_or(1.0);
                    l.set_path("intensity", ConfigValue::Float((power / 30.0).clamp(0.2, 3.0)))?;
                    notes.push(format!("{full_name}: point light intensity {} from luminous power {power}", (power / 30.0).clamp(0.2, 3.0)));
                } else {
                    l.set_path("intensity", ConfigValue::Float(real(object, "intensity").unwrap_or(1.0)))?;
                }
                objects.push(l);
                count("light");
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
                let resupply_template = resupply_template()?;
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
                let native_layer = array(v.named("layers").context("template vegetation layers")?)?.first().cloned().context("template vegetation has no layer")?;
                place(&mut v, object, &full_name)?;
                let mut layers = Vec::new();
                for layer in array(object.named("layers").context("vegetation layers")?)? {
                    let mut l = layer.clone();
                    // 0.26's grass cook reads every native layer key (a layer
                    // without wind_factor crashes the client on match setup):
                    // keys the donor lacks come from the template's layer.
                    if let (ConfigValue::Named(fields), ConfigValue::Named(native)) = (&mut l, &native_layer) {
                        for field in native {
                            if field.name != "model_def" && !fields.iter().any(|f| f.name == field.name) {
                                fields.push(field.clone());
                            }
                        }
                    }
                    for def in array(layer.named("model_def").context("model_def")?)? {
                        vegetation_models.insert(model_of_reference(string(def, "name").context("veg model")?));
                    }
                    let file = string(layer, "filename").context("veg filename")?.replace('\\', "/");
                    let emitted = match port {
                        Port::Map(_) => file.clone(),
                        Port::Lobby(lobby) => {
                            let name = file.strip_prefix("vegetation/").unwrap_or(&file);
                            let emitted = format!("vegetation/{}/{name}", lobby.vegetation_folder);
                            l.set_path("filename", ConfigValue::String(emitted.clone()))?;
                            emitted
                        }
                    };
                    vegetation_files.insert(file, emitted);
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
            // A hanging cable: the same object in both builds (0.46c2 adds a
            // texture tile 0.26 does not read).
            "wire_set" if templates_of(template, "wire_set").first().is_some() => {
                let mut w = first_template(template, "wire_set")?;
                place(&mut w, object, &full_name)?;
                copy_shared(
                    &mut w,
                    object,
                    &["fragments_count", "iterations", "margin", "offset1", "points_count", "stiftness", "timescale", "wire_width"],
                )?;
                w.set_path("points", object.named("points").context("wire_set without points")?.clone())?;
                objects.push(w);
                count("wire_set");
            }
            "collision_geometry" => {} // linked ones are added below
            "environment" => {}
            other => *dropped.entry(other.to_owned()).or_default() += 1,
        }
    }
    for link in &kept_collision_geometry {
        let donor_geometry = by_name[link.as_str()];
        let mut c = collision_geometry_template()?;
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

    for (what, n) in &dropped {
        let why = match port {
            Port::Map(_) => "no 0.26 counterpart or not part of TDM",
            Port::Lobby(_) => "no 0.26 counterpart",
        };
        notes.push(format!("dropped {n} {what} ({why})"));
    }

    let mut project = match port {
        Port::Map(map) => {
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
                if !full_name.starts_with(&map.spawn_folder) {
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
            notes.push(format!("spawns: {} from {} (team 0: {}, team 1: {})", respawns.len(), map.spawn_folder, teams[&0], teams[&1]));
            // project_name is the HUD minimap key, not a directory. 0.26's
            // Minimap.setProps sends keys it does not know to its level_09 slot,
            // which no 0.26 map uses; the UI bundle (build_069d_ui.py) puts the
            // donor minimap there.
            notes.push(format!("HUD minimap key: the donor's {:?}; 0.26 resolves unknown keys to its unused level_09 minimap slot", map.level));
            named(vec![
                ("project_name", ConfigValue::String(map.level.clone())),
                ("client_objects", ConfigValue::Indexed(objects)),
                ("render_visuals", ConfigValue::Indexed(visuals)),
                ("collision_objects", ConfigValue::Indexed(collisions)),
                (
                    "server_objects",
                    named(vec![("respawn_points", ConfigValue::Indexed(respawns))]),
                ),
            ])
        }
        // A lobby is a client project: no server objects.
        Port::Lobby(lobby) => {
            // The lobby menu plays the project's camera track from the start
            // and refuses a project without one: the 0.26 lobby's.
            objects.push(first_template(template, "track")?);
            count("track");
            for wall in &lobby.camera_walls {
                let [x, y, z] = wall.half_extents;
                collisions.push(named(vec![
                    ("lib_name", ConfigValue::String(format!("{UNIT_BOX_COLLISION}#[{x:.2}][{y:.2}][{z:.2}]"))),
                    ("position", ConfigValue::Float3(wall.centre)),
                    ("rotation", ConfigValue::Float3([0.0; 3])),
                    ("scale", ConfigValue::Float3(wall.half_extents)),
                    // The groups of the 0.26 lobby's static collision.
                    ("cmask", ConfigValue::Integer(16)),
                    ("cgroup", ConfigValue::Integer(8)),
                ]));
                count("camera_wall");
            }
            named(vec![
                (
                    "project_name",
                    ConfigValue::String(string(template, "project_name").unwrap_or("lobby_scene").to_owned()),
                ),
                ("client_objects", ConfigValue::Indexed(objects)),
                ("render_visuals", ConfigValue::Indexed(visuals)),
                ("collision_objects", ConfigValue::Indexed(collisions)),
                (
                    "srp_lobby",
                    named(vec![
                        ("weapon", ConfigValue::Float3(lobby.weapon)),
                        ("weapon_view", ConfigValue::Float(lobby.weapon_view)),
                    ]),
                ),
            ])
        }
    };
    if let Some(camera) = template.named("camera") {
        let mut camera = camera.clone();
        if let Some(c) = donor.named("camera") {
            copy_shared(&mut camera, c, &["position", "rotation", "direction", "fov"])?;
        }
        project.set_path("camera", camera)?;
    }
    // Groups every native project carries (victory_item_spawn_points,
    // temporary_* instances) are present, empty: the 0.26 client reads them
    // on match setup whatever the mode is. Scalars (levelmap bounds) are not
    // copied; level_07 ships without them.
    if let (ConfigValue::Named(fields), ConfigValue::Named(native)) = (&mut project, template) {
        for field in native {
            let empty = match field.value {
                ConfigValue::Indexed(_) => ConfigValue::Indexed(Vec::new()),
                ConfigValue::Named(_) => ConfigValue::Named(Vec::new()),
                _ => continue,
            };
            if !fields.iter().any(|f| f.name == field.name) {
                fields.push(NamedValue { name: field.name.clone(), value: empty });
            }
        }
    }
    Ok(SceneReport {
        project,
        models,
        vegetation_models,
        vegetation_files,
        probe_textures,
        generated_probes,
        decal_materials,
        counts,
        notes,
    })
}

/// The brightest 0.26 ambient light a scaled 0.69d one becomes (0.26's own
/// stop at 2.2 in the lobby and 4 in 0.46c2's).
const AMBIENT_MAX: f32 = 3.0;

/// Normalize ambient light intensities so their median matches
/// `target_median`, at most `AMBIENT_MAX`.
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
    // 0.69d's brightest (lamp shades, painted window light) are tens of times
    // its median under its auto exposure; 0.26 has none and draws them white.
    for o in objects.iter_mut().filter(|o| kind(o) == "ambient_light") {
        let v = (real(o, "intensity").unwrap_or(0.0) * scale).min(AMBIENT_MAX);
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

fn relocate(path: &str, namespace: &str) -> String {
    let (family, tail) = path.split_once('/').expect("resource has a family");
    format!("{family}/{namespace}/{tail}")
}

pub fn build(
    manifest_path: &Path,
    donor_path: &Path,
    target_path: &Path,
    output: &Path,
) -> Result<Record> {
    let m: Manifest = serde_json::from_reader(fs::File::open(manifest_path)?)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    ensure!(
        matches!(m.source_build, SourceBuild::V069d | SourceBuild::V046c2),
        "this builder ports 0.69d and 0.46c2 scenes"
    );
    let port = m.port()?;
    let ns = m.namespace();
    let relocate = |path: &str| relocate(path, &ns);
    eprintln!("Checking donor and target fingerprints...");
    check_hash(donor_path, &m.donor_fingerprint)?;
    check_hash(target_path, &m.target_sha256)?;
    let mut donor = ResourceDb::open(donor_path)?;
    let mut target = ResourceDb::open(target_path)?;

    let mut dictionary = cfg(&mut target, DICTIONARY)?;
    if let Port::Map(spec) = &port {
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
    }
    let template = cfg(&mut target, port.template_project())?;
    let donor_project = cfg(&mut donor, port.donor_project())?;
    eprintln!("Converting the scene...");
    let mut scene = convert_scene(&donor_project, &template, &port, m.photometric(), &m.particle_replacements)?;
    let donor_lit = matches!(port, Port::Map(MapSpec { probes: MapProbes::Donor { .. }, .. }));
    let ambient_scale = if m.photometric() && !donor_lit {
        Some(calibrate_ambient(&mut scene.project, median_ambient(&template))?)
    } else {
        None
    };
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
        scene.notes.push(format!("ambience: {added} native object_sound emitters from {} (the donor's sound environments have no 0.26 counterpart)", sounds.project));
    }
    let mut notes = std::mem::take(&mut scene.notes);
    match ambient_scale {
        Some(scale) => notes.push(format!("ambient lights: 0.69d lux scaled by {scale:.4} so their median intensity matches the template's")),
        None if donor_lit => {}
        None => notes.push(format!("ambient lights and lights: {}'s intensities, which are 0.26's", m.source_build.label())),
    }

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
        "models: {} native 0.26 models reused (same collision, render bounds within 5 cm), {} imported from {}",
        native_models.len(),
        donor_models.len(),
        m.source_build.label()
    ));

    // Donor files: the imported models' files, their materials and textures.
    let mut donor_files = BTreeSet::new();
    let mut material_usage: BTreeMap<String, Usage> = BTreeMap::new();
    let mut surface_strides: BTreeMap<String, usize> = BTreeMap::new();
    let mut surface_lod: BTreeMap<String, usize> = BTreeMap::new();
    let mut lod1_only: BTreeSet<String> = BTreeSet::new();
    // Surface directory -> (stride, vertices, indices) of the mesh as emitted.
    let mut emitted_meshes: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
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
        let settings_probe = model_settings(&mut donor, &format!("{root}settings"), m.source_build)?;
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
        let settings = model_settings(&mut donor, &settings_path, m.source_build)?;
        let lods = surface_lods(&settings);
        for (surface, lod) in &lods {
            surface_lod.insert(format!("{root}render/{surface}/converted_model"), *lod);
        }
        // Surfaces listed only in LOD1 (some models also list one in LOD2).
        let listed = |lod: &str| -> BTreeSet<String> {
            match settings.named("lod_hierrarchy").and_then(|h| h.named(lod)).and_then(|l| l.named("surfaces")) {
                Some(ConfigValue::Indexed(v)) => v.iter().filter_map(|x| match x { ConfigValue::String(s) => Some(s.clone()), _ => None }).collect(),
                _ => BTreeSet::new(),
            }
        };
        let (l0, l2) = (listed("LOD0"), listed("LOD2"));
        for surface in listed("LOD1") {
            if !l0.contains(&surface) && !l2.contains(&surface) {
                lod1_only.insert(format!("{root}render/{surface}/converted_model"));
            }
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
    // A native probe stands in for reflection cubemaps that ship nowhere: the
    // map's global one, or a lobby template's (a lobby's may be the donor's).
    let cube = match &port {
        // The template's largest probe (the map's global one when its probes
        // are native).
        Port::Map(_) => templates_of(&template, "environment_probe")
            .into_iter()
            .max_by(|a, b| real(a, "radius").unwrap_or(0.0).total_cmp(&real(b, "radius").unwrap_or(0.0)))
            .and_then(probe_texture)
            .context("template has no probe")?,
        Port::Lobby(_) => {
            let Some(ConfigValue::Indexed(objects)) = template.named("client_objects") else { bail!("no objects") };
            let probe = objects.iter().find(|o| kind(o) == "environment_probe").context("no probe")?;
            string(probe, "texture").context("probe texture")?.to_owned()
        }
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
    // Probe cubemaps this build makes (`MapProbes::Donor`).
    let generated_textures: BTreeSet<String> = scene.generated_probes.iter().map(|g| format!("{ns}/{}", g.texture)).collect();
    let mut material_notes = BTreeMap::<String, Vec<String>>::new();
    for (material, usage) in &material_usage {
        let path = format!("material_instances/{material}.material");
        if let Some(native) = m.native_materials.get(material) {
            ensure!(target.is_file(&format!("material_instances/{native}.material")), "native material {native} missing");
            notes.push(format!("{material}: drawn with the native {native} (manifest)"));
            continue;
        }
        if !donor.is_file(&path) {
            notes.push(format!(
                "{material}: named by a 0.69d surface but shipped by 0.69d {}; the name is kept",
                if target.is_file(&path) { "not, 0.26 has it" } else { "nor 0.26 (editor-only surface)" }
            ));
            continue;
        }
        let mut root = cfg(&mut donor, &path).with_context(|| format!("decal or model material {material}"))?;
        if m.source_build == SourceBuild::V046c2 {
            vocabulary_046(&mut root);
        }
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
                let settings = model_settings(&mut donor, &format!("{model_root}.model/settings"), m.source_build)?;
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
        if converted.category == Category::Water {
            // 0.26's water reflects a fixed sky probe where its screen-space
            // reflection finds nothing: in the caves it drew bright flat sheets.
            let effect = root.named("material").and_then(|m| m.named("forward_water")).and_then(|s| s.named("effect"));
            if let Some(ConfigValue::Float4(c)) = effect.and_then(|e| value_of(e, "scattering_color")) {
                set_effect(&mut converted.root, "forward", "water_fog_color", ConfigValue::Float4(*c))?;
                converted.notes.push("water fog colour: the 0.69d scattering colour".into());
            }
            let smoothness = effect.and_then(|e| number(e, "smoothness")).unwrap_or(1.0);
            let matte_water_probe = match &port {
                Port::Map(MapSpec { probes: MapProbes::Native { matte_water, .. } | MapProbes::Donor { matte_water, .. }, .. }) => matte_water.as_ref(),
                Port::Lobby(_) => None,
            };
            if let (true, Some(probe)) = (smoothness < 0.5, matte_water_probe) {
                let cube = match &port {
                    Port::Map(MapSpec { probes: MapProbes::Donor { .. }, .. }) => {
                        let made = scene.generated_probes.iter().find(|g| g.full_name == *probe).with_context(|| format!("no probe {probe}"))?;
                        format!("{}/{}", m.namespace(), made.texture)
                    }
                    _ => {
                        let native = templates_of(&template, "environment_probe").into_iter().find(|p| string(p, "full_name") == Some(probe.as_str())).with_context(|| format!("template has no probe {probe}"))?;
                        probe_texture(native).context("matte water probe texture")?
                    }
                };
                set_effect(&mut converted.root, "forward", "texture_reflection", ConfigValue::String(cube.clone()))?;
                set_effect(&mut converted.root, "forward", "use_local_reflections", ConfigValue::Boolean(false))?;
                converted.notes.push(format!("matte 0.69d water (smoothness {smoothness}): reflects {cube} ({probe}), no local reflections"));
            }
        }
        let exists = |t: &str| donor.is_file(&format!("textures/{t}.dds")) || target.is_file(&format!("textures/{t}.dds")) || generated_textures.contains(t);
        let replaced = fill_missing_textures(&mut converted.root, &exists, &cube).with_context(|| format!("checking textures of {material}"))?;
        if !replaced.is_empty() {
            converted.notes.push(format!("unused template texture names that ship nowhere now name the diffuse: {}", replaced.join(", ")));
        }
        for t in bound_textures(&converted.root) {
            textures.insert(t);
        }
        material_notes.entry(material.clone()).or_default().extend(converted.notes.iter().cloned());
        for rule in m.material_constants.iter().filter(|r| material.starts_with(&r.prefix)) {
            for (field, value) in &rule.g_stage {
                if effect_value(&converted.root, "g_stage", field).is_some() {
                    set_effect(&mut converted.root, "g_stage", field, ConfigValue::Float(*value))?;
                    material_notes.entry(material.clone()).or_default().push(format!("{field} {value} (manifest, {})", rule.prefix));
                }
            }
        }
        converted_materials.insert(material.clone(), converted);
    }

    // Textures: donor ones are imported, everything else must be native.
    let mut donor_textures = BTreeSet::new();
    for t in textures.iter().filter(|t| !generated_textures.contains(*t)) {
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
    let donor_level_dir = parent_dir(parent_dir(port.donor_project()));
    let output_level_dir = parent_dir(&format!("projects/{}/client_project", port.output_project())).to_owned();
    let output_level_dir = parent_dir(&output_level_dir).to_owned();
    // Donor files that keep their place in the level (vegetation instances).
    let mut level_files = BTreeMap::new();
    for (veg, emitted) in &scene.vegetation_files {
        let path = format!("{donor_level_dir}/{veg}");
        ensure!(donor.is_file(&path), "vegetation file {path} missing");
        level_files.insert(path.clone(), format!("{output_level_dir}/{emitted}"));
        donor_files.insert(path);
    }
    // The donor's probe cubemaps (a cubemap, its irradiance and their options).
    let mut probe_textures = BTreeSet::new();
    for t in &scene.probe_textures {
        for suffix in [".dds", ".options", "_diffuse.dds", "_diffuse.options"] {
            let path = format!("textures/{t}{suffix}");
            ensure!(donor.is_file(&path) || suffix.ends_with(".options"), "probe cubemap {path} missing");
            if donor.is_file(&path) {
                donor_files.insert(path);
            }
        }
        probe_textures.insert(t.clone());
    }

    // References to rewrite: imported donor resources move to the namespace.
    let mut substitutions = BTreeMap::new();
    for model in &donor_models {
        let dest = format!("{ns}/{model}");
        substitutions.insert(model.clone(), dest.clone());
        substitutions.insert(format!("models/{model}.model"), format!("models/{dest}.model"));
        substitutions.insert(format!("resources/models/{model}.model"), format!("resources/models/{dest}.model"));
    }
    let material_dest = |name: &str| format!("{ns}/{name}");
    for t in &donor_textures {
        substitutions.insert(format!("texture:{t}"), format!("{ns}/{t}"));
    }

    crate::weapon::prepare_empty_output(output)?;
    let mut files: Vec<BuiltFile> = Vec::new();
    // Probe cubemaps made from the donor's (`MapProbes::Donor`), with the
    // options native probes carry.
    if !scene.generated_probes.is_empty() {
        let native = templates_of(&template, "environment_probe")
            .into_iter()
            .find_map(probe_texture)
            .context("template has no probe cubemap")?;
        let options = target.read(&format!("textures/{native}.options"))?;
        let diffuse_options = target.read(&format!("textures/{native}_diffuse.options"))?;
        let mut cubes = BTreeMap::<(String, usize), Vec<u8>>::new();
        let mut clamped = 0;
        for g in &scene.generated_probes {
            let key = (g.specular.texture.clone(), g.specular_mip);
            if !cubes.contains_key(&key) {
                // 0.69d probes name their cube under `environment_probes/`.
                let bytes = donor.read(&format!("textures/environment_probes/{}.dds", g.specular.texture))?;
                let (cube, over) = super::lighting::specular_cube(&bytes, &g.specular, g.scale, g.specular_mip)?;
                clamped += over;
                cubes.insert(key.clone(), cube);
            }
            let base = format!("textures/{ns}/{}", g.texture);
            let what = format!("0.69d probe {} re-encoded for 0.26 ({})", g.specular.texture, g.full_name);
            emit(output, &format!("{base}.dds"), &cubes[&key], &what, &mut files)?;
            emit(output, &format!("{base}.options"), &options, &what, &mut files)?;
            let diffuse = super::lighting::diffuse_cube(&g.diffuse, g.scale);
            emit(output, &format!("{base}_diffuse.dds"), &diffuse, &what, &mut files)?;
            emit(output, &format!("{base}_diffuse.options"), &diffuse_options, &what, &mut files)?;
            probe_textures.insert(g.texture.clone());
        }
        notes.push(format!("probe cubemaps: {} made, {clamped} texels above 0.26's range clamped", scene.generated_probes.len()));
    }
    let game_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let mut converted_geometry = BTreeSet::new();
    let mut repaired_model_settings = BTreeSet::new();
    let mut vegetation_instances = BTreeMap::new();
    let source = format!("{}+0.26g-adapter", m.source_build.label());
    let source = source.as_str();
    let mut halved = 0usize;
    let mut decimated = 0usize;
    let mut terrain_triangles = (0usize, 0usize);
    let mut lod0_triangles = (0usize, 0usize);
    let mut simplified_triangles = (0usize, 0usize);
    let mut blended_terrain = 0usize;
    let mut generated: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new();
    let mut lod_switched = 0usize;
    let mut lods_generated = 0usize;
    eprintln!("Emitting {} donor files...", donor_files.len());
    for path in &donor_files {
        if m.lod1_shares_lod0 {
            let surface_file = path.rsplit_once('/').map(|(dir, _)| format!("{dir}/converted_model"));
            if surface_file.is_some_and(|f| lod1_only.contains(&f)) {
                continue;
            }
        }
        let mut bytes = donor.read(path)?;
        let dest = plain_render_path(&level_files.get(path).cloned().unwrap_or_else(|| relocate(path)));
        let dest = if dest.starts_with("textures/") { plain_texture(&dest) } else { dest };
        if path.ends_with("/converted_model") {
            bytes = geometry(&bytes, m.source_build).with_context(|| format!("repacking {path}"))?;
            let terrain = m.decimate_terrain.iter().any(|p| path.starts_with(&format!("models/{p}")));
            // Blend overlays (`*_vc`) keep every vertex: their vertex alpha
            // gates the alpha test, and simplifying averages it away.
            let decimate = terrain && !path.trim_end_matches("/converted_model").ends_with("_vc");
            let locked = match (decimate, m.terrain_simplify) {
                (true, Some(t)) => simplify_mesh_with(&bytes, t.ratio, t.max_error, meshopt::SimplifyOptions::LockBorder)?,
                _ => None,
            };
            if let Some(smaller) = locked {
                terrain_triangles.0 += triangles(&bytes);
                terrain_triangles.1 += triangles(&smaller);
                decimated += 1;
                bytes = smaller;
            } else if decimate {
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
            // LOD0 (and LOD-less) surfaces: 0.69d meshes are several times
            // denser than 0.26's, whose maps load 25-54 MiB of static geometry.
            if let (Some(simplify), Some(ratio), false) = (m.simplify, m.simplify.and_then(|s| s.lod0_ratio), terrain) {
                if surface_lod.get(path).copied().unwrap_or(0) == 0 && triangles(&bytes) >= simplify.lod0_min_triangles.unwrap_or(simplify.min_triangles) {
                    if let Some(smaller) = simplify_mesh_with(&bytes, ratio, simplify.lod0_max_error.unwrap_or(simplify.max_error), simplify.options(0)).with_context(|| format!("simplifying {path}"))? {
                        lod0_triangles.0 += triangles(&bytes);
                        lod0_triangles.1 += triangles(&smaller);
                        bytes = smaller;
                    }
                }
            }
            if let (Some(simplify), Some(&lod)) = (m.simplify, surface_lod.get(path)) {
                if lod > 0 && triangles(&bytes) >= simplify.min_triangles {
                    let ratio = if lod == 1 { simplify.lod1_ratio } else { simplify.lod2_ratio };
                    let error = if lod == 1 { simplify.max_error } else { simplify.lod2_max_error.unwrap_or(simplify.max_error) };
                    if let Some(smaller) = simplify_mesh_with(&bytes, ratio, error, simplify.options(lod)).with_context(|| format!("simplifying {path}"))? {
                        simplified_triangles.0 += triangles(&bytes);
                        simplified_triangles.1 += triangles(&smaller);
                        bytes = smaller;
                    }
                }
            }
            if let (Some(simplify), Some((model_root, surface))) = (m.simplify, path.strip_suffix("/converted_model").and_then(|p| p.split_once("/render/"))) {
                if generate_lods.get(model_root).is_some_and(|l| l.iter().any(|x| x == surface)) {
                    for (level, ratio) in [(1, simplify.lod1_ratio), (2, simplify.lod2_ratio)] {
                        if level == 1 && m.lod1_shares_lod0 {
                            continue;
                        }
                        let error = if level == 1 { simplify.max_error } else { simplify.lod2_max_error.unwrap_or(simplify.max_error) };
                        let lod = simplify_mesh_with(&bytes, ratio, error, simplify.options(level))?.unwrap_or_else(|| bytes.clone());
                        let name = format!("srp_lod{level}_{surface}");
                        let gen_dest = plain_render_path(&relocate(&format!("{model_root}/render/{name}/converted_model")));
                        emit(output, &gen_dest, &lod, "0.69d LOD0 simplified by meshoptimizer", &mut files)?;
                        let properties = donor.read(&format!("{model_root}/render/{surface}/export_properties"))?;
                        let mut properties = BinaryConfig::parse(&properties)?;
                        if properties.root.named("source").is_some() {
                            let u = |o: usize| u32::from_le_bytes(lod[o..o + 4].try_into().unwrap()) as usize;
                            let (vertices, indices) = (u(8), u(8 + u(4) + 8));
                            properties.root.set_path("source.vertices", ConfigValue::Integer(i32::try_from(vertices)?))?;
                            properties.root.set_path("source.indices", ConfigValue::Integer(i32::try_from(indices)?))?;
                            properties.root.set_path("source.primitives", ConfigValue::Integer(i32::try_from(indices / 3)?))?;
                        }
                        let properties = properties.to_bytes()?;
                        emit(output, &plain_render_path(&relocate(&format!("{model_root}/render/{name}/export_properties"))), &properties, source, &mut files)?;
                        generated.entry(model_root.to_owned()).or_insert_with(Vec::new).push((level, surface.to_owned(), name));
                    }
                }
            }
            converted_geometry.insert(path.clone());
            if let Some(s) = stride(&bytes) {
                let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
                emitted_meshes.insert(path.trim_end_matches("/converted_model").to_owned(), (s, u(8), u(8 + u(4) + 8)));
            }
        } else if path.ends_with(".model/settings") {
            let mut c = BinaryConfig::parse(&bytes)?.root;
            lod_lists(&mut c);
            let model_root = path.trim_end_matches("/settings");
            if c.named("game_material_settings").is_some() {
                if m.source_build == SourceBuild::V046c2 {
                    unassigned_046(&mut c);
                }
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
                    } else if let Some(native) = m.native_materials.get(&normalized) {
                        *name = native.clone();
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
            if m.lod1_shares_lod0 {
                let list = |c: &ConfigValue, lod: &str| -> Vec<ConfigValue> {
                    c.named("lod_hierrarchy").and_then(|h| h.named(lod)).and_then(|l| l.named("surfaces")).and_then(|s| array(s).ok()).map(<[ConfigValue]>::to_vec).unwrap_or_default()
                };
                let (lod0, lod1, lod2) = (list(&c, "LOD0"), list(&c, "LOD1"), list(&c, "LOD2"));
                if !lod0.is_empty() && (!lod1.is_empty() || !lod2.is_empty()) {
                    c.set_path("lod_hierrarchy.LOD1", named(vec![("surfaces", ConfigValue::Indexed(lod0.clone()))]))?;
                    if let Some(ConfigValue::Named(entries)) = c.named_mut("material_settings") {
                        entries.retain(|e| !lod1.contains(&ConfigValue::String(e.name.clone())) || lod0.contains(&ConfigValue::String(e.name.clone())) || lod2.contains(&ConfigValue::String(e.name.clone())));
                    }
                }
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
            plain_surfaces(&mut c);
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
            canonical_dds(&mut bytes).with_context(|| format!("{path}: native DDS header"))?;
        } else if super::is_config_resource(path) {
            // export_properties, exported_primitives and texture options are
            // schema-identical; re-encode in the 24-byte layout.
            let mut c = BinaryConfig::parse(&bytes).with_context(|| format!("decoding {path}"))?;
            // A surface's properties follow its mesh as emitted: 0.26 reads the
            // vertex layout from `type` (2: with vertex colour, 32 bytes) and
            // the counts from `source` (blended terrain gains a colour, and
            // simplified meshes shrink).
            if let Some(&(s, vertices, indices)) = path.strip_suffix("/export_properties").and_then(|d| emitted_meshes.get(d)) {
                if let Some(ConfigValue::Integer(t)) = c.root.named("type") {
                    let t = match (s, *t) {
                        (32, _) => 2,
                        (_, 2) => 0,
                        (_, t) => t,
                    };
                    c.root.set_path("type", ConfigValue::Integer(t))?;
                }
                if c.root.named("source").is_some() {
                    c.root.set_path("source.vertices", ConfigValue::Integer(i32::try_from(vertices)?))?;
                    c.root.set_path("source.indices", ConfigValue::Integer(i32::try_from(indices)?))?;
                    c.root.set_path("source.primitives", ConfigValue::Integer(i32::try_from(indices / 3)?))?;
                }
            }
            bytes = c.to_bytes()?;
        }
        ensure!(!target.contains(&dest), "would overwrite native resource {dest}");
        emit(output, &dest, &bytes, source, &mut files)?;
    }
    if let (true, Some(t)) = (decimated > 0, m.terrain_simplify) {
        notes.push(format!("{decimated} terrain render meshes simplified with locked borders from {} to {} triangles (error bound {} of the tile extent; collision unchanged)", terrain_triangles.0, terrain_triangles.1, t.max_error));
    } else if decimated > 0 {
        notes.push(format!("{decimated} terrain render meshes reduced to about a quarter of their triangles (every second grid line, or border-locked simplification for re-cut tiles; collision unchanged)"));
    }
    if blended_terrain > 0 {
        notes.push(format!("{blended_terrain} terrain meshes carry two-layer blend weights in a new vertex colour (stride 28 -> 32)"));
    }
    if lod0_triangles.0 > 0 {
        let s = m.simplify.unwrap();
        notes.push(format!("LOD0 and LOD-less surfaces simplified from {} to {} triangles (meshoptimizer, error bound {} of the mesh extent; collision unchanged)", lod0_triangles.0, lod0_triangles.1, s.lod0_max_error.unwrap_or(s.max_error)));
    }
    if simplified_triangles.0 > 0 {
        notes.push(format!("authored LOD1/LOD2 surfaces simplified from {} to {} triangles (meshoptimizer, error bound {}, LOD2 {})", simplified_triangles.0, simplified_triangles.1, m.simplify.unwrap().max_error, m.simplify.unwrap().lod2_max_error.unwrap_or(m.simplify.unwrap().max_error)));
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
        rewrite_textures(&mut root, &donor_textures, &ns);
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
            let root = format!("models/{ns}/{model}.model");
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
                let dest = format!("models/{ns}/{name}.model");
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
                    ("lib_name", ConfigValue::String(format!("{ns}/{name}"))),
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

    // Imported models that are translated copies of another (0.69d exports
    // the Duga antenna sections as eight models at their own offsets): their
    // render instances draw the first copy, moved by the offset. Collision
    // instances keep their own models; the copies' files stay emitted.
    if m.dedup_translated_copies {
        // Per model: LOD structure and materials, then per LOD0 surface its
        // translation key, first vertex, vertex count and bounds.
        struct Shape {
            model: String,
            structure: String,
            origin: [f32; 3],
            surfaces: Vec<(Vec<u8>, [f32; 3], usize, [f32; 3], [f32; 3])>,
        }
        let mut shapes: Vec<Shape> = Vec::new();
        for model in &donor_models {
            if scene.vegetation_models.contains(model) || m.decimate_terrain.iter().any(|p| model.starts_with(p.as_str())) {
                continue;
            }
            let root = format!("models/{model}.model");
            let settings = cfg(&mut donor, &format!("{root}/settings"))?;
            let Some(ConfigValue::Indexed(list)) = settings.named("lod_hierrarchy").and_then(|h| h.named("LOD0")).and_then(|l| l.named("surfaces")) else { continue };
            let mut surfaces = Vec::new();
            let mut materials = Vec::new();
            for surface in list {
                let ConfigValue::String(surface) = surface else { continue };
                let path = format!("{root}/render/{surface}/converted_model");
                if !donor.is_file(&path) {
                    continue;
                }
                surfaces.push((surface.clone(), geometry(&donor.read(&path)?, m.source_build)?));
                materials.push(settings.named("material_settings").and_then(|t| t.named(surface)).and_then(|e| string(e, "material_name")).unwrap_or("").to_owned());
            }
            surfaces.sort();
            let mut shape = Shape {
                model: model.clone(),
                structure: format!("{:?}{materials:?}{:?}", settings.named("lod_hierrarchy"), surfaces.iter().map(|s| &s.0).collect::<Vec<_>>()),
                origin: [0.0; 3],
                surfaces: Vec::new(),
            };
            let mut ok = !surfaces.is_empty();
            for (k, (name, mesh)) in surfaces.iter().enumerate() {
                let Some((key, first)) = translation_key(&[(name.clone(), mesh.clone())]) else { ok = false; break };
                let Some((lo, hi)) = mesh_bounds(mesh) else { ok = false; break };
                if k == 0 {
                    shape.origin = first;
                }
                let count = u32::from_le_bytes(mesh[8..12].try_into().unwrap()) as usize;
                shape.surfaces.push((key, first, count, lo, hi));
            }
            if ok {
                shapes.push(shape);
            }
        }
        // A copy matches surface by surface: the same geometry at the model's
        // offset, or (re-exported wires and the like) a vertex count within
        // 0.2% and the same bounds within 1 cm; at least half of the vertices
        // in exactly equal surfaces.
        let same_copy = |a: &Shape, b: &Shape| -> bool {
            if a.structure != b.structure {
                return false;
            }
            let offset: [f32; 3] = std::array::from_fn(|i| b.origin[i] - a.origin[i]);
            let close = |x: [f32; 3], y: [f32; 3], tolerance: f32| (0..3).all(|i| (y[i] - x[i] - offset[i]).abs() <= tolerance);
            let (mut exact, mut total) = (0, 0);
            for (sa, sb) in a.surfaces.iter().zip(&b.surfaces) {
                total += sa.2;
                if sa.0 == sb.0 && close(sa.1, sb.1, 0.001) {
                    exact += sa.2;
                } else if sa.2.abs_diff(sb.2) * 500 > sa.2 || !close(sa.3, sb.3, 0.01) || !close(sa.4, sb.4, 0.01) {
                    return false;
                }
            }
            exact * 2 >= total
        };
        let mut alias: BTreeMap<String, (String, [f32; 3])> = BTreeMap::new();
        let mut canonical: Vec<usize> = Vec::new();
        for (i, shape) in shapes.iter().enumerate() {
            match canonical.iter().find(|&&c| same_copy(&shapes[c], shape)) {
                Some(&c) => {
                    alias.insert(shape.model.clone(), (shapes[c].model.clone(), std::array::from_fn(|k| shape.origin[k] - shapes[c].origin[k])));
                }
                None => canonical.push(i),
            }
        }
        let Some(ConfigValue::Indexed(visuals)) = scene.project.named_mut("render_visuals") else { bail!("no render visuals") };
        let mut moved = 0;
        for v in visuals.iter_mut() {
            let lib = string(v, "lib_name").unwrap_or("").to_owned();
            let model = model_of_reference(&lib);
            let Some((canonical, offset)) = alias.get(&model) else { continue };
            let placement = Placement { position: float3(v, "position")?, rotation: float3(v, "rotation")?, scale: float3(v, "scale")? };
            let d = placement.apply(&instance_rotation(placement.rotation), *offset, placement.scale);
            v.set_path("position", ConfigValue::Float3(std::array::from_fn(|i| placement.position[i] + d[i])))?;
            v.set_path("lib_name", ConfigValue::String(lib.replacen(&model, canonical, 1)))?;
            moved += 1;
        }
        notes.push(format!("{} imported models are translated copies of others; {moved} render instances draw the original moved by the offset", alias.len()));
    }

    // Material clusters of render instances.
    if let Some(cluster) = m.material_clusters {
        let overlay = output.join("overlay");
        // A model's files: imported ones from the overlay, native ones from 0.26.
        fn read_model_file(target: &mut ResourceDb, overlay: &Path, native: bool, rel: &str) -> Result<Vec<u8>> {
            if native { target.read(rel) } else { fs::read(overlay.join(rel)).with_context(|| format!("reading {rel}")) }
        }
        fn read_model_config(target: &mut ResourceDb, overlay: &Path, native: bool, rel: &str) -> Result<ConfigValue> {
            Ok(BinaryConfig::parse(&read_model_file(target, overlay, native, rel)?)?.root)
        }
        // Blended and terrain materials stay per instance (draw order, tile
        // borders); foliage and grass merge only with models of the same
        // material set.
        let category = |material: &str| {
            let name = material.strip_prefix(&format!("{ns}/")).unwrap_or(material);
            converted_materials.get(name).map(|c| c.category)
        };
        let alone = |material: &str| matches!(category(material), Some(Category::Glass | Category::Water | Category::Terrain | Category::ProjectedDecal));
        let own_model = |material: &str| matches!(category(material), Some(Category::Foliage | Category::Grass));
        let imported_distances = m.lod_distances;
        let Some(ConfigValue::Indexed(visuals)) = scene.project.named_mut("render_visuals") else { bail!("no render visuals") };
        struct Prepared {
            settings: ConfigValue,
            root: String,
            native: bool,
            /// Per LOD: (material, surface).
            lods: [Vec<(String, String)>; 3],
            centre: [f32; 3],
            radius: f32,
            /// Merges only with models of the same material set.
            own: bool,
            /// Type-0 switching distances; None for a LOD-less model.
            distances: Option<[f32; 3]>,
            /// Distances and material set: models alike merge first.
            signature: String,
        }
        let mut prepared: BTreeMap<String, Option<Prepared>> = BTreeMap::new();
        let mut cells: BTreeMap<(i64, i64, String), Vec<usize>> = BTreeMap::new();
        // (instance, world centre, merges only with models alike)
        let mut alike: BTreeMap<(i64, i64, String), Vec<(usize, [f32; 3], bool)>> = BTreeMap::new();
        for (i, v) in visuals.iter().enumerate() {
            let model = model_of_reference(string(v, "lib_name").unwrap_or(""));
            let native = native_models.contains(&model);
            if !(native || donor_models.contains(&model))
                || scene.vegetation_models.contains(&model)
                || m.decimate_terrain.iter().any(|p| model.starts_with(p.as_str()))
            {
                continue;
            }
            if !prepared.contains_key(&model) {
                let root = if native { format!("models/{model}.model") } else { format!("models/{ns}/{model}.model") };
                let settings = read_model_config(&mut target, &overlay, native, &format!("{root}/settings"))?;
                let materials = settings.named("material_settings");
                let mut lods: [Vec<(String, String)>; 3] = Default::default();
                let (mut ok, mut own, mut geometry_bytes) = (true, native, 0usize);
                for (level, lod) in ["LOD0", "LOD1", "LOD2"].iter().enumerate() {
                    let Some(ConfigValue::Indexed(list)) = settings.named("lod_hierrarchy").and_then(|h| h.named(lod)).and_then(|l| l.named("surfaces")) else { continue };
                    for surface in list {
                        let ConfigValue::String(surface) = surface else { continue };
                        let material = materials.and_then(|m| m.named(surface)).and_then(|e| string(e, "material_name")).unwrap_or("");
                        own |= own_model(material);
                        ok &= !material.is_empty() && !alone(material);
                        match read_model_file(&mut target, &overlay, native, &format!("{root}/render/{surface}/converted_model")) {
                            // Only the 0.26 static layouts bake (basis words at 12..24).
                            Ok(mesh) => {
                                ok &= matches!(stride(&mesh), Some(28 | 32));
                                geometry_bytes += mesh.len();
                            }
                            Err(_) => ok = false,
                        }
                        lods[level].push((material.to_owned(), surface.clone()));
                    }
                }
                ok &= settings.named("lod_hierrarchy").and_then(|h| h.named("LOD3")).is_none() && !lods[0].is_empty();
                ok &= geometry_bytes <= cluster.max_member_bytes;
                let has_lods = !lods[1].is_empty() || !lods[2].is_empty();
                let switching = settings.named("lod_switching");
                let distances = match (has_lods, switching.and_then(|s| integer(s, "type")), switching.and_then(|s| s.named("default_params"))) {
                    (false, _, _) => None,
                    // 0.26 select_lod defaults (see docs: 20/40/240).
                    (true, None | Some(0), None | Some(ConfigValue::Boolean(true))) => Some([20.0, 40.0, 240.0]),
                    (true, Some(0), _) => {
                        let s = switching.unwrap();
                        Some([real(s, "param0").unwrap_or(20.0), real(s, "param1").unwrap_or(40.0), real(s, "param2").unwrap_or(240.0)])
                    }
                    _ => {
                        ok = false;
                        None
                    }
                };
                let (lo, hi) = bounding_box(&read_model_config(&mut target, &overlay, native, &format!("{root}/render/export_properties"))?).context("model bounds")?;
                let centre: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
                let radius = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt() / 2.0;
                let materials: BTreeSet<&str> = lods.iter().flatten().map(|(m, _)| m.as_str()).collect();
                let signature = format!("{distances:?} {}", materials.into_iter().collect::<Vec<_>>().join(" "));
                prepared.insert(model.clone(), ok.then_some(Prepared { settings, root, native, lods, centre, radius, own, distances, signature }));
            }
            let Some(p) = &prepared[&model] else { continue };
            let scale = float3(v, "scale")?;
            // Larger instances, and models keeping the donor's own LOD
            // distances, merge only with models alike.
            let own = p.own || (p.distances.is_some() && m.lod_distances.is_none()) || p.radius * scale.iter().fold(0.0f32, |a, s| a.max(s.abs())) > cluster.max_member_radius;
            // Cells hold the instance's world bounding-box centre (some models
            // are authored far from their origin).
            let placement = Placement { position: float3(v, "position")?, rotation: float3(v, "rotation")?, scale };
            let c = placement.apply(&instance_rotation(placement.rotation), p.centre, scale);
            let c: [f32; 3] = std::array::from_fn(|k| c[k] + placement.position[k]);
            let size = cluster.model_cell;
            alike.entry(((c[0] / size).floor() as i64, (c[2] / size).floor() as i64, p.signature.clone())).or_default().push((i, c, own));
        }
        // Instances of models alike (same switching, same materials) in a
        // model cell merge first; what is left of the small imported ones
        // merges across models in the finer cells.
        let mut signatures = Vec::new();
        for ((cx, cz, signature), members) in alike {
            if members.len() >= 2 {
                signatures.push(signature);
                cells.insert((cx, cz, format!("alike{}", signatures.len() - 1)), members.iter().map(|m| m.0).collect());
                continue;
            }
            for (i, c, own) in members {
                if !own {
                    cells.entry(((c[0] / cluster.cell).floor() as i64, (c[2] / cluster.cell).floor() as i64, String::new())).or_default().push(i);
                }
            }
        }
        let mut removed = BTreeSet::new();
        let mut added = Vec::new();
        let (mut clusters, mut clustered, mut native_clustered, mut draws_before, mut draws_after) = (0usize, 0usize, 0usize, [0usize; 3], [0usize; 3]);
        for ((cx, cz, group), members) in cells {
            if members.len() < 2 {
                continue;
            }
            let mut offered = Vec::new();
            let mut centres = Vec::new();
            let mut first: Option<(ConfigValue, String, bool)> = None;
            // (material, stride) -> export_properties of a source surface (its
            // mesh `type` says whether the surface carries vertex colour).
            let mut surface_properties: BTreeMap<(String, usize), ConfigValue> = BTreeMap::new();
            let mut distances: Option<[f32; 3]> = None;
            for &i in &members {
                let model = model_of_reference(string(&visuals[i], "lib_name").unwrap_or(""));
                let p = prepared[&model].as_ref().unwrap();
                let placement = Placement { position: float3(&visuals[i], "position")?, rotation: float3(&visuals[i], "rotation")?, scale: float3(&visuals[i], "scale")? };
                let r = instance_rotation(placement.rotation);
                let c = placement.apply(&r, p.centre, placement.scale);
                centres.push(std::array::from_fn::<f32, 3, _>(|k| c[k] + placement.position[k]));
                if let Some(d) = p.distances {
                    ensure!(distances.is_none_or(|x| x == d), "cluster {group} {cx},{cz} mixes LOD distances");
                    distances = Some(d);
                }
                let mut member = ClusterMember { placement, lods: Default::default() };
                for level in 0..3 {
                    for (material, surface) in &p.lods[level] {
                        let mesh = read_model_file(&mut target, &overlay, p.native, &format!("{}/render/{surface}/converted_model", p.root))?;
                        let key = (material.clone(), stride(&mesh).context("not static")?);
                        if !surface_properties.contains_key(&key) {
                            surface_properties.insert(key, read_model_config(&mut target, &overlay, p.native, &format!("{}/render/{surface}/export_properties", p.root))?);
                        }
                        member.lods[level].push((material.clone(), mesh));
                    }
                }
                for level in 0..3 {
                    draws_before[level] += (0..=level).rev().map(|l| member.lods[l].len()).find(|n| *n > 0).unwrap_or(0);
                }
                native_clustered += p.native as usize;
                first.get_or_insert_with(|| (p.settings.clone(), p.root.clone(), p.native));
                offered.push(member);
            }
            if group.is_empty() && distances.is_some() {
                ensure!(distances == imported_distances, "mixed cluster {cx},{cz} has non-imported LOD distances");
            }
            let lods = distances.is_some();
            // Members whose LOD1 is their LOD0 (or LOD-less ones) let the cluster share it too.
            let share1 = members.iter().all(|&i| {
                let p = prepared[&model_of_reference(string(&visuals[i], "lib_name").unwrap_or(""))].as_ref().unwrap();
                p.lods[1].is_empty() || p.lods[1] == p.lods[0]
            });
            let merged = merge_cluster(&offered, lods).with_context(|| format!("clustering cell {cx},{cz}"))?;
            let (settings, member_root, member_native) = first.unwrap();
            let name = if group.is_empty() { format!("srp_cluster/cell_{cx}_{cz}") } else { format!("srp_cluster/{group}_{cx}_{cz}") };
            let dest = format!("models/{ns}/{name}.model");
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut hierarchy: Vec<NamedValue> = Vec::new();
            let mut material_settings = Vec::new();
            for (level, surfaces) in merged.iter().enumerate() {
                let mut names = Vec::new();
                if level == 1 && share1 {
                    names = hierarchy[0].value.named("surfaces").and_then(|s| array(s).ok()).map(<[ConfigValue]>::to_vec).unwrap_or_default();
                    draws_after[level] += names.len();
                    hierarchy.push(NamedValue { name: "LOD1".into(), value: named(vec![("surfaces", ConfigValue::Indexed(names))]) });
                    continue;
                }
                for (k, (material, mesh, a, b)) in surfaces.iter().enumerate() {
                    let surface = format!("lod{level}_{k}");
                    for j in 0..3 {
                        lo[j] = lo[j].min(a[j]);
                        hi[j] = hi[j].max(b[j]);
                    }
                    emit(output, &format!("{dest}/render/{surface}/converted_model"), mesh, "instances merged by material", &mut files)?;
                    let mut properties = surface_properties[&(material.clone(), stride(mesh).context("not static")?)].clone();
                    set_bounds(&mut properties, *a, *b)?;
                    if properties.named("source").is_some() {
                        // Counts of the merged mesh, as the STK2 builder writes them.
                        let u = |o: usize| u32::from_le_bytes(mesh[o..o + 4].try_into().unwrap()) as usize;
                        let (vertices, indices) = (u(8), u(8 + u(4) + 8));
                        properties.set_path("source.vertices", ConfigValue::Integer(i32::try_from(vertices)?))?;
                        properties.set_path("source.indices", ConfigValue::Integer(i32::try_from(indices)?))?;
                        properties.set_path("source.primitives", ConfigValue::Integer(i32::try_from(indices / 3)?))?;
                    }
                    emit(output, &format!("{dest}/render/{surface}/export_properties"), &BinaryConfig { root: properties }.to_bytes()?, "instances merged by material", &mut files)?;
                    material_settings.push(NamedValue { name: surface.clone(), value: named(vec![("material_name", ConfigValue::String(material.clone()))]) });
                    names.push(ConfigValue::String(surface));
                }
                draws_after[level] += names.len();
                hierarchy.push(NamedValue { name: format!("LOD{level}"), value: named(vec![("surfaces", ConfigValue::Indexed(names))]) });
            }
            if !lods {
                for level in 1..3 {
                    draws_after[level] += merged[0].len();
                }
            }
            let centre: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
            let reach = centres.iter().map(|c| (0..3).map(|k| (c[k] - centre[k]).powi(2)).sum::<f32>().sqrt()).fold(0.0f32, f32::max);
            let mut settings = settings;
            settings.set_path("lod_hierrarchy", ConfigValue::Named(hierarchy))?;
            settings.set_path("material_settings", ConfigValue::Named(material_settings))?;
            if settings.named("game_material_settings").is_some() {
                settings.set_path("game_material_settings", ConfigValue::Named(Vec::new()))?;
            }
            for field in ["collision_enabled", "walker_collision_enabled"] {
                if settings.named(field).is_some() {
                    settings.set_path(field, ConfigValue::Boolean(false))?;
                }
            }
            if let Some(d) = distances {
                settings.set_path(
                    "lod_switching",
                    named(vec![
                        ("type", ConfigValue::Integer(0)),
                        ("default_params", ConfigValue::Boolean(false)),
                        ("param0", ConfigValue::Float(d[0] + reach)),
                        ("param1", ConfigValue::Float(d[1] + reach)),
                        ("param2", ConfigValue::Float(d[2] + reach)),
                    ]),
                )?;
            }
            emit(output, &format!("{dest}/settings"), &BinaryConfig { root: settings }.to_bytes()?, "instances merged by material", &mut files)?;
            let mut properties = read_model_config(&mut target, &overlay, member_native, &format!("{member_root}/render/export_properties"))?;
            set_bounds(&mut properties, lo, hi)?;
            emit(output, &format!("{dest}/render/export_properties"), &BinaryConfig { root: properties }.to_bytes()?, "instances merged by material", &mut files)?;
            added.push(named(vec![
                ("lib_name", ConfigValue::String(format!("{ns}/{name}"))),
                ("position", ConfigValue::Float3([0.0; 3])),
                ("rotation", ConfigValue::Float3([0.0; 3])),
                ("scale", ConfigValue::Float3([1.0; 3])),
            ]));
            removed.extend(members.iter().copied());
            clusters += 1;
            clustered += members.len();
        }
        let mut index = 0;
        visuals.retain(|_| {
            index += 1;
            !removed.contains(&(index - 1))
        });
        visuals.extend(added);
        notes.push(format!(
            "material clusters (instances of models alike, same LOD switching and material set, per {2} m cell first, then the remaining imported instances up to {1} m radius across models per {0} m cell; foliage, native and larger models only with models alike; models above {3} bytes of geometry stay instanced): {clustered} render instances ({native_clustered} of native models) merged into {clusters} render-only models, one surface per material and LOD; surfaces per LOD {draws_before:?} -> {draws_after:?}; each cluster switches LOD later by its centre-to-member reach; collision instances unchanged",
            cluster.cell, cluster.max_member_radius, cluster.model_cell, cluster.max_member_bytes
        ));
    }

    // Scene references.
    let mut project = scene.project;
    rewrite_scene(&mut project, &donor_models, &converted_materials, &probe_textures, &ns)?;
    let output_project = format!("projects/{}/client_project", port.output_project());
    ensure!(!target.contains(&output_project), "project exists");
    let what = format!("{} scene on 0.26g templates", m.source_build.label());
    emit(output, &output_project, &BinaryConfig { root: project }.to_bytes()?, &what, &mut files)?;

    if let Port::Map(spec) = &port {
        // Mode and registration.
        let mut mode = cfg(&mut target, &spec.template_mode)?;
        mode.set_path("mode.name", ConfigValue::String(spec.mode_name.trim_end_matches(".options").into()))?;
        mode.set_path("mode.match_making_params.map_name", ConfigValue::String(spec.output_project.clone()))?;
        if let Some(level) = spec.level_id {
            mode.set_path("mode.match_making_params.map_id", ConfigValue::Integer(level.into()))?;
            notes.push(format!("level number (match_making_params.map_id): {level}"));
        }
        if let Some(id) = spec.loading_screen_id {
            mode.set_path("mode.ui.loading_screen_id", ConfigValue::Integer(id.into()))?;
            notes.push(format!("loading screen: slot {id} of lobby_menu.swf (the UI bundle puts the donor's image there)"));
        }
        if let Some(name_key) = &spec.map_name {
            mode.set_path("mode.ui.map_name", ConfigValue::String(name_key.clone()))?;
            notes.push(format!("map name: {name_key}, its texts the target's dictionaries'"));
        } else {
            add_donor_map_name(&mut mode, spec, &mut donor, &mut target, output, &mut files, &mut notes)?;
        }
        let mode_path = format!("gameplay/game_modes/{}", spec.mode_name);
        ensure!(!target.contains(&mode_path), "mode exists");
        emit(output, &mode_path, &BinaryConfig { root: mode }.to_bytes()?, "0.26g mode template", &mut files)?;
        let ConfigValue::Indexed(entries) = dictionary.named_mut("maps_dict").unwrap() else { unreachable!() };
        entries.push(named(vec![
            ("id", ConfigValue::Integer(spec.id.into())),
            ("cfg_mode_name", ConfigValue::String(spec.mode_name.clone())),
        ]));
        emit(output, DICTIONARY, &BinaryConfig { root: dictionary }.to_bytes()?, "0.26g+additive-map-registration", &mut files)?;
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let record = Record {
        format_version: 1,
        asset_id: match &port {
            Port::Map(spec) => format!("retail-{}-{}-to-026g", m.source_build.prefix(), spec.output_project.replace('/', "-")),
            Port::Lobby(l) => format!("retail-{}-{}-to-026g", m.source_build.prefix(), l.output_project.replace('/', "-")),
        },
        donor_build: m.source_build.label().into(),
        target_build: "0.26g0-2777".into(),
        target_db_sha256: m.target_sha256.clone(),
        server_project_fallback: None,
        // The server learns the map from this (the content pack's `server.maps`).
        maps: match &port {
            Port::Map(spec) => vec![Map {
                id: spec.id,
                level: spec.level.clone(),
                weather: "day".into(),
                environment_look: Default::default(),
                native_ambient_fill: false,
            }],
            Port::Lobby(_) => Vec::new(),
        },
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
    let summary = match &port {
        Port::Map(spec) => serde_json::json!({ "map": spec, "counts": scene.counts, "native_models": native_models, "donor_models": donor_models }),
        Port::Lobby(spec) => serde_json::json!({ "lobby": spec, "counts": scene.counts, "native_models": native_models, "donor_models": donor_models }),
    };
    serde_json::to_writer_pretty(fs::File::create(output.join("scene-summary.json"))?, &summary)?;
    Ok(record)
}

/// The donor's own map name (`ui.map_name`) on the mode, its texts added to
/// every 0.26 localization that lacks the key.
fn add_donor_map_name(
    mode: &mut ConfigValue,
    spec: &MapSpec,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    output: &Path,
    files: &mut Vec<BuiltFile>,
    notes: &mut Vec<String>,
) -> Result<()> {
    {
        let donor_mode = cfg(donor, &spec.donor_mode)?;
        let name_key = string(&donor_mode, "mode.ui.map_name").or_else(|| donor_mode.named("mode").and_then(|m| m.named("ui")).and_then(|u| string(u, "map_name"))).context("donor mode has no ui.map_name")?.to_owned();
        mode.set_path("mode.ui.map_name", ConfigValue::String(name_key.clone()))?;
        let mut added_strings = Vec::new();
        for path in target.paths_with_prefix("localization/").filter(|p| p.ends_with("/localization")).map(str::to_owned).collect::<Vec<_>>() {
            let mut strings = cfg(target, &path)?;
            let ConfigValue::Named(entries) = strings.named("strings").context("localization without strings")? else { bail!("{path}: strings is not a table") };
            if entries.iter().any(|e| e.name == name_key) {
                continue;
            }
            // The same language from the donor, else its English text.
            let donor_path = if donor.is_file(&path) { path.clone() } else { "localization/english/localization".to_owned() };
            let donor_strings = cfg(donor, &donor_path)?;
            let text = donor_strings.named("strings").and_then(|s| string(s, &name_key)).with_context(|| format!("{donor_path} has no {name_key}"))?.to_owned();
            let Some(ConfigValue::Named(entries)) = strings.named_mut("strings") else { unreachable!() };
            entries.push(NamedValue { name: name_key.clone(), value: ConfigValue::String(text) });
            emit(output, &path, &BinaryConfig { root: strings }.to_bytes()?, &format!("0.26g localization + {name_key} from {donor_path}"), files)?;
            added_strings.push(path.split('/').nth(1).unwrap_or("").to_owned());
        }
        ensure!(!added_strings.is_empty() || target.is_file("localization/english/localization"), "no localization to extend");
        notes.push(format!("map name: the donor's {name_key} added to 0.26 localization ({})", added_strings.join(", ")));
    }
    Ok(())
}

fn rewrite_textures(root: &mut ConfigValue, donor_textures: &BTreeSet<String>, ns: &str) {
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
                            *s = format!("{ns}/{}", plain_texture(&n));
                        }
                    }
                } else {
                    rewrite_textures(&mut field.value, donor_textures, ns);
                }
            }
        }
        ConfigValue::Indexed(values) => values.iter_mut().for_each(|v| rewrite_textures(v, donor_textures, ns)),
        _ => {}
    }
}

fn rewrite_scene(
    project: &mut ConfigValue,
    donor_models: &BTreeSet<String>,
    materials: &BTreeMap<String, ConvertedMaterial>,
    probe_textures: &BTreeSet<String>,
    ns: &str,
) -> Result<()> {
    let relocate_model = |reference: &str| -> String {
        let model = model_of_reference(reference);
        if !donor_models.contains(&model) {
            return reference.to_owned();
        }
        reference.replacen(&model, &format!("{ns}/{model}"), 1)
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
                        *s = format!("{ns}/{n}");
                    }
                }
            }
            "environment_probe" => {
                let Some(texture) = string(object, "texture").map(|t| t.replace('\\', "/")) else { continue };
                if probe_textures.contains(&texture) {
                    set_probe_texture(object, &format!("{ns}/{texture}"))?;
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
        let out = geometry(&mesh, SourceBuild::V069d).unwrap();
        let v = &out[12..40];
        assert_eq!(&v[12..16], &[0x80, 0x00, 0x80, 0x7f]);
        assert_eq!(&v[16..20], &[0x00, 0x80, 0x7f, 0xff]);
        assert_eq!(&v[20..24], &[0x7f, 0x80, 0xff, 0xff]);
        assert_eq!(&v[24..28], &[0x00, 0x3c, 0x00, 0x38], "UV untouched");
        // Vertex chunk + index chunk only.
        assert_eq!(out.len(), 8 + 4 + 28 + 8 + 10);
        // Vertex colour survives.
        let vc = geometry(&static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 32, &[]), SourceBuild::V069d).unwrap();
        assert_eq!(&vc[12 + 28..12 + 32], &[1, 2, 3, 4]);
    }

    #[test]
    fn a_026_packed_basis_or_unknown_chunk_is_rejected() {
        assert!(geometry(&static_mesh([0x7f7f007f, 0xff7e7f00, 0xfffe7f7e], 28, &[]), SourceBuild::V069d).is_err());
        assert!(geometry(&static_mesh([0x3f000080, 0x3eff0000, 0x3f7f807f], 28, &[(77, vec![0; 4])]), SourceBuild::V069d).is_err());
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

    /// `count` vertices along +X at x = 0..count, triangles (i, i+1, i+2).
    fn strip(count: usize, stride: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(3u32.to_le_bytes());
        b.extend(((4 + stride * count) as u32).to_le_bytes());
        b.extend((count as u32).to_le_bytes());
        for i in 0..count {
            for v in [i as f32, 0.0, 0.0] {
                b.extend(v.to_le_bytes());
            }
            b.extend(pack_basis([0.0, 1.0, 0.0], 0x7f).to_le_bytes());
            b.extend(pack_basis([1.0, 0.0, 0.0], 0xff).to_le_bytes());
            b.extend(pack_basis([0.0, 0.0, 1.0], 0xff).to_le_bytes());
            b.extend(vec![0u8; stride - 24]);
        }
        let tris = count - 2;
        b.extend(4u32.to_le_bytes());
        b.extend(((4 + tris * 6) as u32).to_le_bytes());
        b.extend(((tris * 3) as u32).to_le_bytes());
        for t in 0..tris {
            for i in [t, t + 1, t + 2] {
                b.extend((i as u16).to_le_bytes());
            }
        }
        b
    }

    fn mesh_counts(m: &[u8]) -> (usize, Vec<u16>) {
        let u = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap()) as usize;
        let at = 8 + u(4);
        (u(8), (0..u(at + 8)).map(|k| u16::from_le_bytes([m[at + 12 + 2 * k], m[at + 13 + 2 * k]])).collect())
    }

    #[test]
    fn concatenated_meshes_rebase_indices_and_split_at_the_16_bit_limit() {
        let merged = concat_meshes(&[strip(4, 28), strip(3, 28)]).unwrap();
        assert_eq!(merged.len(), 1);
        let (count, indices) = mesh_counts(&merged[0].0);
        assert_eq!(count, 7);
        assert_eq!(indices, [0, 1, 2, 1, 2, 3, 4, 5, 6], "second mesh indexes its own vertices");
        assert_eq!((merged[0].1[0], merged[0].2[0]), (0.0, 3.0));
        // 20000 + 15000 vertices exceed a native-sized surface; a stride change starts another.
        let parts = concat_meshes(&[strip(20_000, 28), strip(15_000, 28), strip(3, 32)]).unwrap();
        assert_eq!(parts.iter().map(|p| mesh_counts(&p.0).0).collect::<Vec<_>>(), [20_000, 15_000, 3]);
        assert_eq!(stride(&parts[2].0), Some(32));
        // 900 kB of vertex data at most: 28 000 vertices of stride 32 do not fit with 4 000 more.
        let parts = concat_meshes(&[strip(25_000, 32), strip(4_000, 32)]).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(concat_meshes(&[strip(3, 28), strip(34_000, 28)]).unwrap().len(), 2, "a native-sized mesh stays alone");
        assert!(concat_meshes(&[strip(36_000, 28)]).is_err(), "a mesh above the native maximum is an error");
    }

    #[test]
    fn a_cluster_merges_surfaces_by_material_per_lod_and_lod_less_members_fill_every_level() {
        let at = |x: f32| Placement { position: [x, 0.0, 0.0], rotation: [0.0; 3], scale: [1.0; 3] };
        let with_lods = ClusterMember {
            placement: at(0.0),
            lods: [
                vec![("brick".into(), strip(4, 28)), ("metal".into(), strip(3, 28))],
                vec![("brick".into(), strip(3, 28))],
                vec![],
            ],
        };
        let lod_less = ClusterMember { placement: at(100.0), lods: [vec![("brick".into(), strip(3, 28))], vec![], vec![]] };
        let merged = merge_cluster(&[with_lods, lod_less], true).unwrap();
        let summary: Vec<Vec<(String, usize)>> = merged
            .iter()
            .map(|level| level.iter().map(|(m, mesh, _, _)| (m.clone(), mesh_counts(mesh).0)).collect())
            .collect();
        assert_eq!(summary, [
            vec![("brick".into(), 7), ("metal".into(), 3)],
            vec![("brick".into(), 6)],
            vec![("brick".into(), 6)],
        ], "an empty LOD2 falls back to LOD1, as 0.26 does");
        let (_, _, lo, hi) = &merged[0][0];
        assert_eq!((lo[0], hi[0]), (0.0, 102.0), "members are baked at their placements");
        assert_eq!(merge_cluster(&[ClusterMember { placement: at(0.0), lods: [vec![("a".into(), strip(3, 28))], vec![], vec![]] }], false).unwrap().len(), 1);
    }

    #[test]
    fn translated_copies_share_a_key_whatever_their_triangle_order() {
        let a = strip(5, 28);
        let mut b = a.clone();
        // Move every vertex by (25, 0, 0) and reverse the triangle order.
        for k in 0..5 {
            let o = 12 + 28 * k;
            let x = f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) + 25.0;
            b[o..o + 4].copy_from_slice(&x.to_le_bytes());
        }
        let at = 8 + u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize + 12;
        let tris: Vec<[u8; 6]> = b[at..at + 18].chunks(6).map(|c| c.try_into().unwrap()).rev().collect();
        b[at..at + 18].copy_from_slice(&tris.concat());
        let (ka, oa) = translation_key(&[("s".into(), a.clone())]).unwrap();
        let (kb, ob) = translation_key(&[("s".into(), b)]).unwrap();
        assert_eq!(ka, kb);
        assert_eq!(ob[0] - oa[0], 25.0);
        let mut c = a;
        c[12 + 28 + 24] ^= 1; // a different UV
        assert_ne!(translation_key(&[("s".into(), c)]).unwrap().0, ka);
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
        assert_eq!(&half[20..24], &9u32.to_le_bytes(), "pitch: every byte after the header");
    }

    #[test]
    fn dds_headers_take_the_native_flags_and_pitch() {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[8..12].copy_from_slice(&0xA1007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&8u32.to_le_bytes());
        dds[16..20].copy_from_slice(&8u32.to_le_bytes());
        dds[20..24].copy_from_slice(&32u32.to_le_bytes()); // top mip only
        dds[28..32].copy_from_slice(&2u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend([0u8; 40]);
        dds.push(1);
        canonical_dds(&mut dds).unwrap();
        assert_eq!(&dds[8..12], &0xA100Fu32.to_le_bytes());
        assert_eq!(&dds[20..24], &41u32.to_le_bytes());
        dds.pop();
        assert!(canonical_dds(&mut dds).is_err(), "a chain without the colour-space byte is refused");
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
            ("texture_fresnel", ConfigValue::String("native/f".into())),
            ("constant_fresnel_min", ConfigValue::Float(0.04)),
            ("constant_fresnel_max", ConfigValue::Integer(1)),
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
        // With a metalness map and an `opaque_metal` template, the map is
        // 0.26's Fresnel map.
        let mut donor = donor;
        donor.set_path("material.g_stage.effect.t_metalness", ConfigValue::String("tiled/a_fmap".into())).unwrap();
        let mut templates = templates;
        templates.insert(
            "opaque_metal".to_owned(),
            ("props/box/props_boxes_metal".to_owned(), template(&[("use_nmap", true), ("use_troughness", true), ("use_tfresnel", true)])),
        );
        let c = convert_material(&donor, Usage::default(), &templates, 0.2, "editor/default_nmap").unwrap();
        assert_eq!(c.template, "props/box/props_boxes_metal");
        let e = c.root.named("material").unwrap().named("g_stage").unwrap().named("effect").unwrap();
        assert_eq!(text(e, "texture_fresnel"), Some("tiled/a_fmap"));
        assert_eq!((number(e, "constant_fresnel_min"), number(e, "constant_fresnel_max")), (Some(0.04), Some(1.0)));
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
            ("project_name", ConfigValue::String("level_03".into())),
            ("levelmap_min", ConfigValue::Float3([1.0; 3])),
            ("victory_item_spawn_points", obj(vec![])),
            ("temporary_render_visuals", obj(vec![])),
            ("client_objects", ConfigValue::Indexed(vec![
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment".into())), ("properties", obj(vec![("sun_angle", ConfigValue::Float(1.0)), ("sun_position_azimut", ConfigValue::Float(2.0))]))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment_probe".into())), ("geometry", ConfigValue::Integer(0)), ("radius", ConfigValue::Float(600.0)), ("texture", ConfigValue::String("environment_probes/level_03/global".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("ambient_light".into())), ("color", ConfigValue::Integer(0)), ("intensity", ConfigValue::Integer(1))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("decal".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("light".into())), ("light_type", ConfigValue::Integer(1)), ("intensity", ConfigValue::Float(1.0))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("resupply_zone".into())), ("team", ConfigValue::Integer(0)), ("resupply_time_in_ms", ConfigValue::Integer(1))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("collision_geometry".into())), ("meshes", ConfigValue::Indexed(vec![obj(vec![("type", ConfigValue::Integer(1)), ("position", ConfigValue::Float3([0.0; 3])), ("rotation", ConfigValue::Float3([0.0; 3])), ("scale", ConfigValue::Float3([1.0; 3]))])]))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("vegetation".into())), ("layers", ConfigValue::Indexed(vec![obj(vec![
                    ("name", ConfigValue::String("a_high".into())), ("wind_factor", ConfigValue::Integer(1)), ("model_def", ConfigValue::Indexed(vec![obj(vec![("name", ConfigValue::String("level/03/flora/a".into()))])])),
                ])]))]),
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
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("vegetation".into())), ("full_name", ConfigValue::String("env/vegetation".into())), ("layers", ConfigValue::Indexed(vec![obj(vec![
                    ("name", ConfigValue::String("green".into())), ("filename", ConfigValue::String("vegetation/green.veg".into())),
                    ("model_def", ConfigValue::Indexed(vec![obj(vec![("name", ConfigValue::String("flora/v2/grass".into()))])])),
                ])]))]),
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
            probes: MapProbes::Native { classes: Vec::new(), matte_water: None, ambient_lights: false },
            donor_mode: "d".into(), loading_screen_id: None, level_id: None, map_name: None,
        };
        let scene = convert_scene(&donor, &template, &Port::Map(spec), true, &BTreeMap::new()).unwrap();
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
        let veg = objects.iter().find(|o| kind(o) == "vegetation").unwrap();
        let layer = &array(veg.named("layers").unwrap()).unwrap()[0];
        assert_eq!(integer(layer, "wind_factor"), Some(1), "native layer keys the donor lacks are filled");
        assert_eq!(array(layer.named("model_def").unwrap()).unwrap().len(), 1, "the donor's models, not the template's");
        assert!(scene.vegetation_models.contains("flora/v2/grass"));
        // Native groups present and empty; the HUD key is the donor's; scalars not copied.
        assert_eq!(string(&scene.project, "project_name"), Some("level_11"), "the donor's HUD key");
        assert!(matches!(scene.project.named("victory_item_spawn_points"), Some(ConfigValue::Named(v)) if v.is_empty()));
        assert!(matches!(scene.project.named("temporary_render_visuals"), Some(ConfigValue::Named(v)) if v.is_empty()));
        assert!(scene.project.named("levelmap_min").is_none());
    }

    #[test]
    fn a_0_46c2_mesh_keeps_its_basis_and_loses_its_statistics() {
        // 0.46c2 packs the basis as 0.26 does: bytes z, y, x, alpha.
        let mesh = static_mesh([0x7f80_0080, 0xff7f_8000, 0xffff_807f], 28, &[(110, vec![0; 4]), (113, vec![0; 72])]);
        let out = geometry(&mesh, SourceBuild::V046c2).unwrap();
        assert_eq!(&out[12..8 + 4 + 28], &mesh[12..8 + 4 + 28], "vertices untouched");
        assert_eq!(out.len(), 8 + 4 + 28 + 8 + 10, "vertex and index chunks only");
        assert!(geometry(&static_mesh([0; 3], 28, &[(77, vec![0; 4])]), SourceBuild::V046c2).is_err());
    }

    #[test]
    fn a_0_46c2_material_reads_with_the_0_69d_names() {
        let field = |v: ConfigValue| named(vec![("value", v), ("is_model_parameter", ConfigValue::Boolean(true))]);
        let mut root = named(vec![("material", named(vec![("g_stage", named(vec![("effect", named(vec![
            ("effect_id", ConfigValue::String("g_stage_default".into())),
            ("texture_diffuse", field(ConfigValue::String("tiled/metal/metal_truba".into()))),
            ("texture_normal", field(ConfigValue::String("tiled/metal/metal_truba_nmap".into()))),
            ("use_nmap", field(ConfigValue::Boolean(true))),
            ("texture_roughness", field(ConfigValue::String("tiled/metal/metal_truba_spec".into()))),
            ("use_troughness", field(ConfigValue::Boolean(true))),
            ("constant_roughness_max", field(ConfigValue::Float(0.6))),
        ]))]))]))]);
        vocabulary_046(&mut root);
        let effect = g_stage(&root).unwrap().named("effect").unwrap();
        assert_eq!(text(effect, "t_albedo"), Some("tiled/metal/metal_truba"));
        assert_eq!(text(effect, "t_normal"), Some("tiled/metal/metal_truba_nmap"));
        assert!(flag(effect, "use_t_normal") && flag(effect, "use_t_smoothness"));
        assert_eq!(text(effect, "t_smoothness"), Some("tiled/metal/metal_truba_spec"));
        assert_eq!(number(effect, "max_smoothness"), Some(0.6));
        assert_eq!(text(effect, "texture_diffuse"), Some("tiled/metal/metal_truba"), "0.26 names kept");
    }

    #[test]
    fn surface_folders_and_names_lose_spaces_and_signs() {
        assert_eq!(plain_surface("lod1_material #25"), "lod1_material_25");
        assert_eq!(plain_surface("metal_truba"), "metal_truba");
        assert_eq!(
            plain_render_path("models/x/notebook.model/render/material #25/converted_model"),
            "models/x/notebook.model/render/material_25/converted_model"
        );
        assert_eq!(plain_render_path("models/x/a.model/settings"), "models/x/a.model/settings");
        let mut settings = named(vec![
            ("material_settings", named(vec![("material #25", named(vec![("material_name", ConfigValue::String("m".into()))]))])),
            ("lod_hierrarchy", named(vec![("LOD0", named(vec![("surfaces", ConfigValue::Indexed(vec![ConfigValue::String("material #25".into())]))]))])),
        ]);
        plain_surfaces(&mut settings);
        assert!(settings.named("material_settings").unwrap().named("material_25").is_some());
        assert_eq!(surface_lods(&settings), BTreeMap::from([("material_25".to_owned(), 0)]));
    }

    #[test]
    fn a_0_46c2_lod_table_becomes_a_list() {
        let mut settings = named(vec![("lod_hierrarchy", named(vec![
            ("LOD0", named(vec![("surfaces", named(vec![("bushsg", ConfigValue::String(String::new()))]))])),
            ("LOD1", named(vec![("surfaces", ConfigValue::Indexed(vec![ConfigValue::String("lod1_bushsg".into())]))])),
        ]))]);
        lod_lists(&mut settings);
        assert_eq!(surface_lods(&settings), BTreeMap::from([("bushsg".to_owned(), 0), ("lod1_bushsg".to_owned(), 1)]));
    }

    #[test]
    fn a_lobby_is_a_client_project_with_its_own_vegetation_folder() {
        let obj = |fields: Vec<(&str, ConfigValue)>| named(fields);
        let tf = |p: [f32; 3]| vec![("position", ConfigValue::Float3(p)), ("rotation", ConfigValue::Float3([0.0; 3])), ("scale", ConfigValue::Float3([1.0; 3]))];
        let with = |mut base: Vec<(&'static str, ConfigValue)>, more: Vec<(&'static str, ConfigValue)>| { base.extend(more); obj(base) };
        let model = |name: &str, p: [f32; 3]| with(tf(p), vec![("model_name", ConfigValue::String(name.into()))]);
        let template = obj(vec![
            ("project_name", ConfigValue::String("lobby_scene".into())),
            ("client_objects", ConfigValue::Indexed(vec![
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment".into())), ("properties", obj(vec![("sun_angle", ConfigValue::Float(90.0)), ("sun_position_azimut", ConfigValue::Float(1.0))]))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment_probe".into())), ("geometry", ConfigValue::Integer(0)), ("radius", ConfigValue::Float(15.3)), ("diffuse_multiplier", ConfigValue::Float(2.6)), ("texture", ConfigValue::String("environment_probes/lobby_scene/probe_day".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("ambient_light".into())), ("color", ConfigValue::Integer(0)), ("intensity", ConfigValue::Integer(1)), ("radius", ConfigValue::Float(3.0))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("decal".into()))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("light".into())), ("light_type", ConfigValue::Integer(1)), ("intensity", ConfigValue::Integer(2))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("vegetation".into())), ("layers", ConfigValue::Indexed(vec![obj(vec![
                    ("name", ConfigValue::String("default".into())), ("wind_factor", ConfigValue::Integer(1)), ("model_def", ConfigValue::Indexed(vec![])),
                ])]))]),
                with(tf([0.988, 0.0, -0.605]), vec![("game_object_type", ConfigValue::String("track".into())), ("cyclic", ConfigValue::Boolean(true))]),
            ])),
        ]);
        let donor = obj(vec![
            ("render_visuals", ConfigValue::Indexed(vec![model("level/lobby/hangar_lobby", [0.0; 3]), model("level/new_year_event/snow_event_level1", [1.0; 3])])),
            ("collision_objects", ConfigValue::Indexed(vec![])),
            ("client_objects", ConfigValue::Indexed(vec![
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("environment".into())), ("sun_elevation", ConfigValue::Integer(55)), ("sun_azimuth", ConfigValue::Integer(150))]),
                with(tf([1.3, 3.0, 3.5]), vec![("game_object_type", ConfigValue::String("environment_probe".into())), ("full_name", ConfigValue::String("environment_and_lights/environment_probe".into())),
                    ("geometry", ConfigValue::Integer(1)), ("radius", ConfigValue::Integer(9)), ("diffuse_multiplier", ConfigValue::Integer(1)),
                    ("texture", ConfigValue::String("environment_probes/lobby_scene/probe_day_default_1".into()))]),
                with(tf([2.0, 2.6, 0.0]), vec![("game_object_type", ConfigValue::String("ambient_light".into())), ("full_name", ConfigValue::String("lights/ambient_light(38)".into())), ("intensity", ConfigValue::Float(1.3)), ("color", ConfigValue::Float3([1.0, 0.5, 0.0]))]),
                with(tf([0.0, 1.8, 0.0]), vec![("game_object_type", ConfigValue::String("light".into())), ("full_name", ConfigValue::String("lights/point_light(6)".into())), ("intensity", ConfigValue::Float(0.8))]),
                with(tf([0.0; 3]), vec![("game_object_type", ConfigValue::String("vegetation".into())), ("full_name", ConfigValue::String("vegetation".into())), ("layers", ConfigValue::Indexed(vec![obj(vec![
                    ("name", ConfigValue::String("green".into())), ("filename", ConfigValue::String("vegetation/green.veg".into())),
                    ("model_def", ConfigValue::Indexed(vec![obj(vec![("name", ConfigValue::String("level/03/flora/pra_big_01".into()))])])),
                ])]))]),
            ])),
        ]);
        let lobby = LobbySpec {
            donor_project: "projects/lobby_scene/day/client_project".into(), template_project: "projects/lobby_scene/day/client_project".into(),
            output_project: "lobby_scene/hangar_046".into(), vegetation_folder: "hangar_046".into(), probes: LobbyProbes::Donor,
            exclude_models: vec!["level/new_year_event/".into()], weapon: [-9.9, 1.3, 6.95], weapon_view: 0.6545,
            camera_walls: vec![CameraWall { centre: [0.0, 1.4, -1.9], half_extents: [2.0, 1.4, 0.05] }],
        };
        let scene = convert_scene(&donor, &template, &Port::Lobby(lobby), false, &BTreeMap::new()).unwrap();
        assert!(scene.project.named("server_objects").is_none(), "no server project");
        let srp = scene.project.named("srp_lobby").unwrap();
        assert_eq!(float3(srp, "weapon").unwrap(), [-9.9, 1.3, 6.95]);
        assert_eq!(real(srp, "weapon_view"), Some(0.6545));
        let walls = array(scene.project.named("collision_objects").unwrap()).unwrap();
        assert_eq!(string(&walls[0], "lib_name"), Some("resources/models/editor/walker_collision.model/collision#[2.00][1.40][0.05]"));
        assert_eq!(float3(&walls[0], "scale").unwrap(), [2.0, 1.4, 0.05]);
        assert_eq!(string(&scene.project, "project_name"), Some("lobby_scene"));
        assert_eq!(scene.models, BTreeSet::from(["level/lobby/hangar_lobby".to_owned()]), "the excluded model is left out");
        assert_eq!(scene.vegetation_files, BTreeMap::from([("vegetation/green.veg".to_owned(), "vegetation/hangar_046/green.veg".to_owned())]));
        let objects = array(scene.project.named("client_objects").unwrap()).unwrap();
        let veg = objects.iter().find(|o| kind(o) == "vegetation").unwrap();
        assert_eq!(string(&array(veg.named("layers").unwrap()).unwrap()[0], "filename"), Some("vegetation/hangar_046/green.veg"));
        assert_eq!(objects.iter().filter(|o| kind(o) == "track").count(), 1, "the template's camera track");
        let probes: Vec<_> = objects.iter().filter(|o| kind(o) == "environment_probe").collect();
        assert_eq!(probes.len(), 1, "the donor's volumes only, no global probe");
        assert_eq!(string(probes[0], "texture"), Some("environment_probes/lobby_scene/probe_day_default_1"));
        assert_eq!(integer(probes[0], "geometry"), Some(1));
        assert_eq!(real(probes[0], "diffuse_multiplier"), Some(1.0), "the donor's multiplier");
        assert!(scene.probe_textures.contains("environment_probes/lobby_scene/probe_day_default_1"));
        let ambient = objects.iter().find(|o| kind(o) == "ambient_light").unwrap();
        assert_eq!(real(ambient, "intensity"), Some(1.3), "0.46c2 intensities are 0.26's");
        assert_eq!(integer(ambient, "color"), Some(packed_colour([1.0, 0.5, 0.0])));
        let light = objects.iter().find(|o| kind(o) == "light").unwrap();
        assert_eq!(real(light, "intensity"), Some(0.8));
        let env = objects.iter().find(|o| kind(o) == "environment").unwrap();
        assert_eq!(real(env.named("properties").unwrap(), "sun_angle"), Some(55.0));
    }

    #[test]
    fn a_manifest_ports_one_map_or_one_lobby() {
        let lobby = r#""lobby": {"donor_project": "d", "template_project": "t", "output_project": "lobby_scene/x", "vegetation_folder": "x", "probes": {"template": {"native": "p"}}, "weapon": [0, 1, 0], "weapon_view": 0}"#;
        let manifest = |body: &str| format!(r#"{{"source_build": "0.69d", "donor_fingerprint": "f", "target_sha256": "t", {body}"game_material_names": {{}}, "material_templates": {{}}, "roughness_scale": 0.2}}"#);
        let m: Manifest = serde_json::from_str(&manifest(&format!("{lobby}, "))).unwrap();
        assert!(matches!(m.port().unwrap(), Port::Lobby(LobbySpec { probes: LobbyProbes::Template { .. }, .. })));
        assert_eq!(m.namespace(), "ported/v069d");
        let none: Manifest = serde_json::from_str(&manifest("")).unwrap();
        assert!(none.port().is_err());
    }
}
