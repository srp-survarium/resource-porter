// SPDX-License-Identifier: GPL-3.0-or-later
//! The cancelled S.T.A.L.K.E.R. 2's own lobby scene (2011-12,
//! `resources/sources/projects/lobby_scene/project.xprj`: the a02 blockpost
//! guard hut, its sentry bunk, mattress, stool and campfire on a flat grass
//! terrain) as a 0.26g lobby project, written as a content pack.
//!
//! The 0.26g lobby puts its character at the world origin facing -z (yaw pi)
//! and its camera looks at (0, 1.2, 0) from -z (yaw 0, distance 1.96 by
//! default, 1.5..4.6 with the wheel, shortened by a collision ray); nothing of
//! that comes from the project. So the scene is moved, not the character:
//! every STK2 mesh is baked into the frame `rot(view_yaw) * (p - character)`,
//! with the character's spot on the hut floor at the origin and the chosen
//! view direction along +z, and placed at the identity in the project.
//!
//! What is made (all new paths, nothing of the base replaced):
//!
//! - static models: per surface a 0.26g `converted_model` (28-byte vertices:
//!   position, packed normal/tangent/binormal as the 0.10b static layout,
//!   half-float uv; the u16 index block as it is), the surface and model
//!   `export_properties` and `settings` on native templates, and the STK2
//!   low collision mesh (moved the same way) with a one-material face table;
//! - materials: a native 0.26g material per STK2 material family (cloned, its
//!   texture bindings replaced) or a native material used as it is;
//! - textures: the STK2 source TGAs, scaled to at most `max_texture_size`,
//!   compressed (DXT1 or DXT5 colour, DXT5nm normals) with the 0.26
//!   compressor's header, and their `.options`;
//! - a flat ground plane in place of the STK2 terrain (four 64 m cells, all
//!   height 0, all `terrain/grass_dry`);
//! - the project and its `.lua` from the classic lobby's: the render visuals
//!   and collision objects replaced, the client objects the manifest names
//!   kept or cloned (lights, sounds, the fire particle) at moved positions.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value as Json;

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::content_pack::{Operation, PackManifest, TARGET_BUILD};
use crate::manifest::TextureCompression;
use crate::stk2_content::{
    config_to_lua, make_stk2_collision_face_data, packed_basis, repaired_static_bases,
    validate_static_geometry, validate_stk2_collision,
};

/// Largest static surface 0.26g loads (its loader copies the vertices
/// through a 20-byte-per-vertex `_alloca` on a 1 MiB stack; native surfaces
/// stop at 35,003 vertices).
const MAX_STATIC_SURFACE_VERTICES: usize = 35_003;
const STK2_VERTEX_SIZE: usize = 56;
const VERTEX_SIZE: usize = 28;
/// What the classic lobby gives the collision its camera ray tests
/// (`level/lobby/base.model/collision`).
const CAMERA_COLLISION_MASK: i32 = 20;
const CAMERA_COLLISION_GROUP: i32 = 10;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LobbyManifest {
    pub pack: PackSpec,
    /// The classic lobby project the new one is made from.
    pub template_project: String,
    /// `projects/lobby_scene/<name>/client_project`; its `.lua` beside it.
    pub target_project: String,
    /// Model names are `<model_prefix>/<name>`.
    pub model_prefix: String,
    /// Materials are `<material_prefix>/<stk2 material>`, textures
    /// `<texture_prefix>/<stk2 texture>`.
    pub material_prefix: String,
    pub texture_prefix: String,
    pub static_templates: StaticTemplates,
    /// The STK2 source textures' folder, relative to the STK2 root.
    pub textures_dir: String,
    /// More texture folders, looked in after it.
    #[serde(default)]
    pub more_texture_dirs: Vec<String>,
    /// The template of a model surface that names only its diffuse texture
    /// (an exported model without source settings) when no material
    /// instance binds that texture.
    #[serde(default)]
    pub fallback_template: Option<String>,
    /// A native `.options` the made textures' options are cloned from.
    pub texture_options_template: String,
    pub max_texture_size: u32,
    /// Smaller caps for textures under a prefix (`props/`, ...): the
    /// first that matches.
    #[serde(default)]
    pub texture_caps: Vec<TextureCap>,
    pub scene: SceneSpec,
    #[serde(default)]
    pub models: Vec<ModelSpec>,
    /// Corners of STK2 projects, each merged into one model.
    #[serde(default)]
    pub slices: Vec<SliceSpec>,
    #[serde(default)]
    pub ground: Option<GroundSpec>,
    /// Keyed by the STK2 material instance name the models' settings use.
    #[serde(default)]
    pub materials: BTreeMap<String, MaterialSpec>,
    /// For a material `materials` does not name: the first rule matching
    /// its STK2 base material and flags.
    #[serde(default)]
    pub material_rules: Vec<MaterialRule>,
    /// Where an STK2 material instance named by a model is looked for (its
    /// `material_instances/<name>.material`, or `.material.orig`), in order.
    #[serde(default)]
    pub material_roots: Vec<String>,
    /// The STK2 environment's post-process mapped onto the 0.26g
    /// environment object.
    #[serde(default)]
    pub environment: Option<EnvironmentSpec>,
    /// Client objects of the project, in order: kept or cloned from the
    /// template's (by name).
    pub objects: Vec<ObjectSpec>,
    /// Named points (STK2 world) reported in the new frame, for the exe's
    /// displays.
    #[serde(default)]
    pub spots: BTreeMap<String, [f32; 3]>,
    /// The project's root `srp_lobby` table, which skhid-srp's lobby
    /// patches read (patch 45's weapon spot and view).
    pub srp_lobby: SrpLobbySpec,
    /// Invisible boxes in the lobby frame for the lobby camera's collision
    /// ray alone: a cage inside the room's walls, floor and ceiling (the eye
    /// stops short of them, never on them) and around the props the eye
    /// would get into (skhid-srp lobby-scenes.md, "What a lobby project
    /// must give").
    #[serde(default)]
    pub camera_boxes: Vec<CameraBox>,
    /// The target build's own models placed in the lobby frame (a workbench
    /// under the weapon's spot), with their collision.
    #[serde(default)]
    pub native_props: Vec<NativeProp>,
    /// Fields of the environment object's `properties` set after the STK2
    /// post-process (0.26g has no counterpart of STK2's ambient colour or
    /// tone curve, so the scene is balanced by eye): numbers become the
    /// field's type, 3 or 4 numbers a vector.
    #[serde(default)]
    pub environment_overrides: BTreeMap<String, Json>,
    /// Converted lights (`stk2_light_<n>`, `sun`) set by eye; intensity 0
    /// leaves the light out.
    #[serde(default)]
    pub light_overrides: BTreeMap<String, LightOverride>,
    /// Environment probes of `objects` given cubemaps made here: 0.26g's
    /// diffuse light away from the lamps comes from probes (and ambient
    /// lights), STK2's from the post-process's flat ambient colour.
    #[serde(default)]
    pub probe_cubes: Vec<ProbeCube>,
    /// STK2 materials whose kept emission (a rule's or a material's
    /// `emission`) is scaled further.
    #[serde(default)]
    pub emission_scale: BTreeMap<String, f32>,
}

/// A converted light's fields set by eye.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightOverride {
    #[serde(default)]
    pub intensity: Option<f32>,
    #[serde(default)]
    pub range: Option<f32>,
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    #[serde(default)]
    pub cast_shadow: Option<bool>,
}

/// The cubemaps of a probe in `objects`: radiance from a gradient over the
/// directions' height (`up` straight up, `side` at the horizon, `down`
/// straight down; linear colour, at most 1), and its `_diffuse` irradiance
/// made from it. The probe's `diffuse_multiplier` sets the strength.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeCube {
    /// The client object's name (an `environment_probe` of `objects`).
    pub object: String,
    pub up: [f32; 3],
    pub side: [f32; 3],
    pub down: [f32; 3],
}

/// An invisible box of the camera's collision: 0.26's unit box primitive
/// (`editor/walker_collision`, half extents 1), scaled, turned about y.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraBox {
    /// What it keeps the eye from (for the notes).
    pub what: String,
    pub centre: [f32; 3],
    pub half_extents: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
}

/// A model of the target build in the lobby frame.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProp {
    /// The model, `models/<lib>.model`, as the target has it.
    pub lib: String,
    pub position: [f32; 3],
    /// Its turn about y (the engine's create_rotation(0, yaw, 0)).
    #[serde(default)]
    pub yaw: f32,
}

/// 0.26's unit box collision (half extents 1), scaled per instance.
const UNIT_BOX_COLLISION: &str = "resources/models/editor/walker_collision.model/collision";
/// The groups of the 0.26 lobby's prop collision, which the lobby camera's
/// ray (group 10h, mask 8: 0x2844E4) meets.
const PROP_COLLISION_MASK: i32 = 16;
const PROP_COLLISION_GROUP: i32 = 8;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureCap {
    pub prefix: String,
    pub max: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceSpec {
    /// The merged model is `<model_prefix>/<name>`.
    pub name: String,
    /// The STK2 `project.xprj` and the composite library, relative to the
    /// STK2 root.
    pub project: String,
    pub composites: String,
    /// The `resources` trees the objects' models are looked for in, in
    /// order (`<root>/exported/models/<lib>.model`,
    /// `<root>/sources/models/<lib>.model/settings`).
    pub model_roots: Vec<String>,
    /// The STK2 world box kept: objects' triangles with a corner inside it,
    /// lights inside it.
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    /// Library name prefixes left out (weapons on tables, static
    /// characters, ...).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The project's lights inside the box as copies of a template light.
    #[serde(default)]
    pub lights: Option<SliceLights>,
    /// Give the camera ray the kept objects' collision.
    #[serde(default)]
    pub collision: bool,
    /// Take the objects' models (false: only the lights).
    #[serde(default = "yes")]
    pub geometry: bool,
    /// Surfaces of a library given another STK2 material: a lamp STK2
    /// placed unlit (its plain material) lit by its own emissive variant,
    /// so that it reads as the light source the scene's lights stand for.
    #[serde(default)]
    pub material_swaps: Vec<MaterialSwap>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialSwap {
    /// The library (`props/light/lamp_street_04`).
    pub lib: String,
    pub from: String,
    pub to: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSpec {
    /// The template project's environment object (kept by `objects`).
    pub object: String,
    /// The STK2 project environment's `post_effect` material instance,
    /// relative to the STK2 root; its base material is read from
    /// `<...>/materials/` beside its `material_instances/`.
    pub post_process: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceLights {
    /// The template project's client objects the lights are cloned from:
    /// point, sphere and box lights as the point template, spot lights as
    /// the spot template (point when absent), the sun as the sun template
    /// (the project's sun left out when absent).
    pub point_template: String,
    #[serde(default)]
    pub spot_template: Option<String>,
    #[serde(default)]
    pub sun_template: Option<String>,
    #[serde(default = "one")]
    pub intensity_scale: f32,
    #[serde(default = "one")]
    pub range_scale: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialRule {
    /// STK2 base materials (`concrete`, `wood`, ...); `*` any.
    pub bases: Vec<String>,
    #[serde(default)]
    pub alpha_test: Option<bool>,
    #[serde(default)]
    pub two_sided: Option<bool>,
    pub then: RuleTarget,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuleTarget {
    Converted {
        template: String,
        /// Keep the STK2 material's emission (its colour and multiplier,
        /// times this) on the converted one: 0.26g lights a lamp's glass
        /// only through its emission.
        #[serde(default)]
        emission: Option<f32>,
    },
    Native {
        material: String,
    },
    /// The surfaces are left out.
    Hidden,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SrpLobbySpec {
    /// The spot (a key of `spots`) the weapon's middle floats at:
    /// `srp_lobby.weapon`.
    pub weapon_spot: String,
    /// `srp_lobby.weapon_view`: the lobby camera's yaw looking at it
    /// (forward = (-sin yaw, 0, cos yaw); 0 looks along +z).
    pub weapon_view: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackSpec {
    pub id: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticTemplates {
    pub settings: String,
    pub render_properties: String,
    pub surface_properties: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    /// The character's spot in the STK2 project's world, on the floor.
    pub character: [f32; 3],
    /// Yaw (radians, `x' = x cos + z sin`, `z' = -x sin + z cos`) that turns
    /// the chosen view direction onto +z.
    pub view_yaw: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    /// The target model is `<model_prefix>/<name>`.
    pub name: String,
    /// The STK2 exported model archive, relative to the STK2 root.
    pub archive: String,
    /// The STK2 source model settings (lua), relative to the STK2 root: the
    /// surfaces and their material instances are read from it.
    pub settings: String,
    /// The object's placement in the STK2 project.
    pub position: [f32; 3],
    /// Same convention as [`SceneSpec::view_yaw`] (the project's
    /// `rotation = (-pi, r, -pi)` is the yaw `pi + r`).
    #[serde(default)]
    pub yaw: f32,
    /// Or the project's rotation and scale as they are (the engine's
    /// `create_scale * create_rotation * create_translation`).
    #[serde(default)]
    pub rotation: Option<[f32; 3]>,
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
    /// Give the camera ray the model's low collision mesh.
    #[serde(default)]
    pub collision: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundSpec {
    pub name: String,
    /// STK2 world height of the terrain.
    pub height: f32,
    /// Half the plane's edge, metres, about the character.
    pub half_size: f32,
    /// One texture repeat every `tile` metres.
    pub tile: f32,
    /// The STK2 material instance key in `materials`.
    pub material: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialSpec {
    /// A native material cloned, its textures the STK2 instance's (read from
    /// `source`, a material instance relative to the STK2 root, and its base
    /// material for the roles the instance inherits) or, with `textures`,
    /// these STK2 textures by role.
    Converted {
        template: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        textures: BTreeMap<String, String>,
        /// As [`RuleTarget::Converted`]'s (it needs `source`).
        #[serde(default)]
        emission: Option<f32>,
    },
    /// A native material, as it is.
    Native { material: String },
    /// The surfaces are left out.
    Hidden,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectSpec {
    /// The template client object (its `name`).
    pub from: String,
    /// The new object's name; the template's when absent.
    #[serde(default)]
    pub name: Option<String>,
    /// STK2 world position; the template's (unmoved) when absent.
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    /// Fields set on the object (dotted paths, JSON values).
    #[serde(default)]
    pub set: BTreeMap<String, Json>,
}

/// An affine move in the engine's row-vector form: `p' = p m + t` (the
/// rows of `m` are the images of x, y and z); normals go by `n`, the
/// inverse transpose of `m`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rigid {
    m: [[f32; 3]; 3],
    n: [[f32; 3]; 3],
    t: [f32; 3],
}

fn mul3(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (row, a_row) in out.iter_mut().zip(a) {
        for (column, value) in row.iter_mut().enumerate() {
            *value = a_row[0] * b[0][column] + a_row[1] * b[1][column] + a_row[2] * b[2][column];
        }
    }
    out
}

fn row_times(v: [f32; 3], m: [[f32; 3]; 3]) -> [f32; 3] {
    [0, 1, 2].map(|c| v[0] * m[0][c] + v[1] * m[1][c] + v[2] * m[2][c])
}

/// The angles the engine's `create_rotation` makes `m` (rows) from; `m`
/// a rotation.
fn euler_of(m: [[f32; 3]; 3]) -> [f32; 3] {
    let y = m[0][2].clamp(-1.0, 1.0).asin();
    if m[0][2].abs() < 0.999_99 {
        [(-m[1][2]).atan2(m[2][2]), y, (-m[0][1]).atan2(m[0][0])]
    } else {
        // gimbal: x and z turn about the same axis; all of it on x
        [m[1][0].atan2(m[1][1]), y, 0.0]
    }
}

impl Rigid {
    /// A yaw about +y (`x' = x cos + z sin`, `z' = -x sin + z cos`), then
    /// a translation.
    fn yaw(yaw: f32, translation: [f32; 3]) -> Rigid {
        let (s, c) = yaw.sin_cos();
        let m = [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]];
        Rigid {
            m,
            n: m,
            t: translation,
        }
    }

    /// An STK2 project object's transform: the engine's
    /// `create_scale(scale) * create_rotation(rotation) *
    /// create_translation(position)` (vostok `base_project.cpp`, the
    /// matrix of `math_float4x4_inline.h`).
    fn stk2(position: [f32; 3], rotation: [f32; 3], scale: [f32; 3]) -> Result<Rigid> {
        ensure!(
            scale.iter().all(|s| s.is_finite() && s.abs() > 1e-6),
            "scale {scale:?} is degenerate"
        );
        let (xs, xc) = rotation[0].sin_cos();
        let (ys, yc) = rotation[1].sin_cos();
        let (zs, zc) = rotation[2].sin_cos();
        let r = [
            [yc * zc, -yc * zs, ys],
            [ys * xs * zc + xc * zs, -xs * ys * zs + xc * zc, -xs * yc],
            [-xc * ys * zc + xs * zs, xc * ys * zs + xs * zc, xc * yc],
        ];
        let m = [0, 1, 2].map(|i| r[i].map(|v| v * scale[i]));
        let n = [0, 1, 2].map(|i| r[i].map(|v| v / scale[i]));
        Ok(Rigid { m, n, t: position })
    }

    fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        row_times(v, self.m)
    }

    fn normal(&self, v: [f32; 3]) -> [f32; 3] {
        row_times(v, self.n)
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = self.rotate(p);
        [r[0] + self.t[0], r[1] + self.t[1], r[2] + self.t[2]]
    }

    /// Whether it mirrors (the triangles' turn must then be flipped).
    fn mirrors(&self) -> bool {
        let m = self.m;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        det < 0.0
    }

    /// `outer` after `self`.
    fn then(self, outer: Rigid) -> Rigid {
        Rigid {
            m: mul3(self.m, outer.m),
            n: mul3(self.n, outer.n),
            t: outer.apply(self.t),
        }
    }

    /// The inverse of a rotation and translation (`m` orthonormal).
    fn inverse(&self) -> Rigid {
        let mut m = [[0.0; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, value) in row.iter_mut().enumerate() {
                *value = self.m[c][r];
            }
        }
        let back = row_times(self.t, m);
        Rigid {
            m,
            n: m,
            t: [-back[0], -back[1], -back[2]],
        }
    }

    /// The scene's frame: the character at the origin, the view along +z.
    fn scene(spec: &SceneSpec) -> Rigid {
        let turn = Rigid::yaw(spec.view_yaw, [0.0; 3]);
        let back = turn.rotate(spec.character);
        Rigid::yaw(spec.view_yaw, [-back[0], -back[1], -back[2]])
    }
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl Bounds {
    fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    fn add(&mut self, p: [f32; 3]) {
        for (axis, value) in p.into_iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    fn union(&mut self, other: &Bounds) {
        self.add(other.min);
        self.add(other.max);
    }

    fn centre(&self) -> [f32; 3] {
        [0, 1, 2].map(|a| (self.min[a] + self.max[a]) * 0.5)
    }
}

/// The colour space and block format of a made texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureKind {
    /// sRGB colour (diffuse, emissive): DXT5 when the source has alpha.
    Colour,
    /// Tangent-space normals as DXT5nm.
    Normal,
    /// Linear data (specular intensity): DXT1.
    Linear,
}

/// The 0.26g texture slots the STK2 roles fill. `use_flag` names the
/// template's switch that must be on for the slot to be read; an unread
/// slot keeps the template's binding (nothing is made for it).
const ROLES: &[(&str, &str, TextureKind, Option<&str>)] = &[
    (
        "texture_diffuse",
        "texture_diffuse",
        TextureKind::Colour,
        None,
    ),
    (
        "texture_normal",
        "texture_normal",
        TextureKind::Normal,
        Some("use_nmap"),
    ),
    (
        "texture_specular_intensity",
        "texture_specular_intensity",
        TextureKind::Linear,
        Some("use_tspecular_intensity"),
    ),
    (
        "texture_emissive",
        "texture_emissive",
        TextureKind::Colour,
        Some("use_emissive_map"),
    ),
];

/// One file of the pack (its resource path, its bytes on disk under `files/`).
struct PackWriter {
    files: PathBuf,
    paths: BTreeSet<String>,
}

impl PackWriter {
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        crate::content_pack::validate_resource_path(path)?;
        ensure!(self.paths.insert(path.to_owned()), "{path}: made twice");
        let dest = safe_join(&self.files, path)?;
        fs::create_dir_all(dest.parent().context("resource path has no parent")?)?;
        fs::write(&dest, bytes).with_context(|| format!("writing {}", dest.display()))
    }

    fn config(&mut self, path: &str, config: &BinaryConfig) -> Result<()> {
        self.write(path, &config.to_bytes()?)
    }
}

pub struct BuildOptions<'a> {
    pub manifest: &'a Path,
    /// The STK2 tree holding `STK2/xr2-*/resources/...` as the manifest names them.
    pub stk2_root: &'a Path,
    /// The 0.26g resources.db (templates, and the check that no path exists).
    pub target_db: &'a Path,
    /// A new, empty folder: the pack (`pack.json`, `files/`).
    pub output: &'a Path,
    /// Scratch space for the texture conversions.
    pub work: &'a Path,
    /// Write the STK2 textures the materials need to this file, one a
    /// line, and make nothing (to know what to extract).
    pub list_textures: Option<&'a Path>,
}

/// What a build made, for the report.
pub struct BuildReport {
    pub files: usize,
    pub spots: BTreeMap<String, [f32; 3]>,
    pub manifest_sha256: String,
}

pub fn build(options: &BuildOptions<'_>) -> Result<BuildReport> {
    let manifest: LobbyManifest = serde_json::from_slice(
        &fs::read(options.manifest)
            .with_context(|| format!("reading {}", options.manifest.display()))?,
    )
    .with_context(|| format!("parsing {}", options.manifest.display()))?;
    crate::weapon::prepare_empty_output(options.output)?;
    fs::create_dir_all(options.work)?;
    let mut target = ResourceDb::open(options.target_db)?;
    let mut pack = PackWriter {
        files: options.output.join("files"),
        paths: BTreeSet::new(),
    };
    let scene = Rigid::scene(&manifest.scene);
    let stk2 = Stk2Tree {
        root: options.stk2_root,
        dry: options.list_textures.is_some(),
    };

    // What every model and slice needs: their surfaces' STK2 materials.
    let mut used_materials = BTreeSet::new();
    let mut models = Vec::new();
    for model in &manifest.models {
        let surfaces = stk2.model_surfaces(&model.settings)?;
        used_materials.extend(surfaces.values().cloned());
        models.push((model, surfaces));
    }
    let mut slices = Vec::new();
    for slice in &manifest.slices {
        let objects = slice_objects(slice, &stk2, scene)?;
        for object in &objects.models {
            used_materials.extend(object.surfaces.values().cloned());
        }
        slices.push((slice, objects));
    }
    if let Some(ground) = &manifest.ground {
        used_materials.insert(ground.material.clone());
    }
    let mut textures = TextureMaker {
        stk2: &stk2,
        prefix: &manifest.texture_prefix,
        dir: &manifest.textures_dir,
        more_dirs: &manifest.more_texture_dirs,
        max_size: manifest.max_texture_size,
        caps: &manifest.texture_caps,
        work: options.work,
        options_template: parse(&mut target, &manifest.texture_options_template)?,
        made: BTreeMap::new(),
        failed: Vec::new(),
    };
    // STK2 material -> the 0.26 material and its diffuse; absent: hidden.
    let mut material_names: BTreeMap<String, Option<(String, String)>> = BTreeMap::new();
    let mut rule_notes = Vec::new();
    for key in &used_materials {
        let resolved;
        let spec = match manifest.materials.get(key) {
            Some(spec) => spec,
            None => {
                resolved = material_by_rule(key, &manifest, &stk2)?;
                rule_notes.push(format!("{key}: {}", resolved.1));
                &resolved.0
            }
        };
        let made = make_material(
            key,
            spec,
            &manifest.emission_scale,
            &manifest.material_prefix,
            &stk2,
            &mut target,
            &mut textures,
            &mut pack,
        )?;
        material_names.insert(
            key.clone(),
            made.map(|(name, diffuse)| (name, diffuse.unwrap_or_default())),
        );
    }

    if let Some(list) = options.list_textures {
        let names: BTreeSet<&str> = textures
            .made
            .keys()
            .map(|(name, _)| name.as_str())
            .collect();
        let mut text = String::new();
        for name in names {
            text.push_str(name);
            text.push('\n');
        }
        fs::write(list, text)?;
        for note in &rule_notes {
            eprintln!("material by rule {note}");
        }
        return Ok(BuildReport {
            files: 0,
            spots: BTreeMap::new(),
            manifest_sha256: String::new(),
        });
    }

    let templates = StaticModelTemplates {
        settings: parse(&mut target, &manifest.static_templates.settings)?,
        render_properties: parse(&mut target, &manifest.static_templates.render_properties)?,
        surface_properties: parse(&mut target, &manifest.static_templates.surface_properties)?,
    };
    let mut visuals = Vec::new();
    let mut collisions = Vec::new();
    for (model, surfaces) in &models {
        let name = format!("{}/{}", manifest.model_prefix, model.name);
        let placement = match model.rotation {
            Some(rotation) => {
                ensure!(model.yaw == 0.0, "{}: a yaw and a rotation", model.name);
                Rigid::stk2(model.position, rotation, model.scale.unwrap_or([1.0; 3]))?
            }
            None => {
                ensure!(
                    model.scale.is_none(),
                    "{}: a scale without a rotation",
                    model.name
                );
                Rigid::yaw(model.yaw, model.position)
            }
        }
        .then(scene);
        let mut archive = ResourceDb::open(safe_join(options.stk2_root, &model.archive)?)
            .with_context(|| format!("opening {}", model.archive))?;
        let mut built = Vec::new();
        for (surface, material) in surfaces {
            let Some((material_name, texture_name)) = material_names[material].clone() else {
                continue;
            };
            let vertices = archive.read(&format!("render/{surface}/vertices"))?;
            let indices = archive.read(&format!("render/{surface}/indices"))?;
            validate_static_geometry(&vertices, &indices, surface)?;
            let (mesh, bounds) = convert_surface(&vertices, &indices, placement, surface)?;
            built.push(BuiltSurface {
                name: surface.clone(),
                material: material_name,
                mesh,
                bounds,
                vertices: read_u32(&vertices, 0)?,
                indices: read_u32(&indices, 0)?,
                mesh_name: format!("|{}|{surface}", model.name),
                texture_name,
            });
        }
        let collision = if model.collision {
            let vertices = archive.read("collision/vertices")?;
            let indices = archive.read("collision/indices")?;
            let triangles = validate_stk2_collision(&vertices, &indices, &model.archive)?;
            Some((move_collision(&vertices, placement)?, indices, triangles))
        } else {
            None
        };
        write_static_model(
            &name,
            &format!("STK2/{}", model.archive),
            &built,
            collision,
            &templates,
            &mut pack,
        )?;
        visuals.push(name.clone());
        if model.collision {
            collisions.push(name);
        }
    }

    // The slices: each one merged model of its kept objects.
    let mut slice_lights = Vec::new();
    let mut slice_notes = Vec::new();
    for (slice, objects) in &slices {
        let name = format!("{}/{}", manifest.model_prefix, slice.name);
        let made = write_slice(
            &name,
            slice,
            objects,
            &material_names,
            &templates,
            scene,
            &mut pack,
        )?;
        slice_notes.push(format!(
            "slice {}: {} objects kept of {} ({} without a model: {}), {} surfaces, {} vertices, {} lights",
            slice.name,
            made.objects,
            objects.total,
            objects.missing.len(),
            objects.missing.join(", "),
            made.surfaces,
            made.vertices,
            objects.lights.len()
        ));
        if made.surfaces > 0 {
            visuals.push(name.clone());
            if made.collision {
                collisions.push(name);
            }
        }
        if let Some(lights) = &slice.lights {
            for light in &objects.lights {
                slice_lights.push((lights, light.clone()));
            }
        }
    }

    // The ground: one flat plane, the character in its middle.
    if let Some(ground) = &manifest.ground {
        let ground_name = format!("{}/{}", manifest.model_prefix, ground.name);
        let height = scene.apply([0.0, ground.height, 0.0])[1];
        let (mesh, bounds, vertices, indices) =
            ground_plane(height, ground.half_size, ground.tile)?;
        let (material, texture_name) = material_names[&ground.material]
            .clone()
            .context("the ground's material is hidden")?;
        write_static_model(
            &ground_name,
            "generated: flat ground in place of the STK2 lobby terrain (four 64 m cells, height 0)",
            &[BuiltSurface {
                name: "ground_s".to_owned(),
                material,
                mesh,
                bounds,
                vertices,
                indices,
                mesh_name: format!("|{}|ground", ground.name),
                texture_name,
            }],
            None,
            &templates,
            &mut pack,
        )?;
        visuals.push(ground_name);
    }

    // The project.
    let mut project = parse(&mut target, &manifest.template_project)?;
    replace_project(
        &mut project,
        &visuals,
        &collisions,
        &manifest.objects,
        &slice_lights,
        &manifest.light_overrides,
        scene,
    )?;
    let mut extra_notes = Vec::new();
    add_camera_boxes(&mut project, &manifest.camera_boxes, &mut extra_notes)?;
    add_native_props(&mut project, &manifest.native_props, &mut target, &mut extra_notes)?;
    let mut environment_notes = Vec::new();
    if let Some(environment) = &manifest.environment {
        let values = material_values(&stk2, &environment.post_process)?;
        let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("client_objects") else {
            bail!("the project has no client_objects");
        };
        let object = objects
            .iter_mut()
            .find(|o| matches!(o.named("name"), Some(ConfigValue::String(n)) if *n == environment.object))
            .with_context(|| format!("no client object {:?} to take the post-process", environment.object))?;
        environment_notes = apply_post_process(object, &values)?;
    }
    if !manifest.environment_overrides.is_empty() {
        let name = manifest
            .environment
            .as_ref()
            .map(|e| e.object.as_str())
            .context("environment_overrides without an environment")?;
        let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("client_objects") else {
            bail!("the project has no client_objects");
        };
        let object = objects
            .iter_mut()
            .find(|o| matches!(o.named("name"), Some(ConfigValue::String(n)) if n == name))
            .with_context(|| format!("no client object {name:?} for environment_overrides"))?;
        for (field, value) in &manifest.environment_overrides {
            let path = format!("properties.{field}");
            let hint = crate::content_pack::value::value_at(object, &path)
                .cloned()
                .with_context(|| format!("environment_overrides: no field {field}"))?;
            // a number over the template's integer stays fractional
            let hint = match hint {
                ConfigValue::Integer(v) => ConfigValue::Float(v as f32),
                other => other,
            };
            let value = crate::content_pack::value::to_config(value, Some(&hint))
                .with_context(|| format!("environment_overrides: {field}"))?;
            object.set_path(&path, value)?;
        }
        extra_notes.push(format!(
            "environment overrides (by eye, in the game): {}",
            manifest.environment_overrides.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    for cube in &manifest.probe_cubes {
        add_probe_cube(&mut project, cube, &manifest.texture_prefix, &mut target, &mut pack, &mut extra_notes)?;
    }
    let weapon = manifest
        .spots
        .get(&manifest.srp_lobby.weapon_spot)
        .with_context(|| format!("srp_lobby: no spot {:?}", manifest.srp_lobby.weapon_spot))?;
    ensure!(
        manifest.srp_lobby.weapon_view.is_finite(),
        "srp_lobby: weapon_view is not a number"
    );
    project.root.set_path(
        "srp_lobby",
        ConfigValue::Named(vec![
            named(
                "weapon",
                ConfigValue::Float3(scene.apply(*weapon).map(|v| (v * 1000.0).round() / 1000.0)),
            ),
            named(
                "weapon_view",
                ConfigValue::Float(manifest.srp_lobby.weapon_view),
            ),
        ]),
    )?;
    pack.config(&manifest.target_project, &project)?;
    pack.write(
        &format!("{}.lua", manifest.target_project),
        config_to_lua(&project.root)?.as_bytes(),
    )?;

    for path in &pack.paths {
        ensure!(
            !target.contains(path),
            "{path} is in the target already: the pack adds new files only"
        );
    }
    let spots: BTreeMap<String, [f32; 3]> = manifest
        .spots
        .iter()
        .map(|(name, p)| (name.clone(), scene.apply(*p)))
        .collect();
    let pack_manifest = PackManifest {
        format_version: 1,
        id: manifest.pack.id.clone(),
        version: manifest.pack.version.clone(),
        target_build: TARGET_BUILD.to_owned(),
        description: Some(manifest.pack.description.clone()),
        requires: Vec::new(),
        base_db_sha256: None,
        operations: pack
            .paths
            .iter()
            .map(|path| Operation::AddFile {
                path: path.clone(),
                source: None,
            })
            .collect(),
        server: None,
        notes: spot_notes(&manifest, &spots)
            .into_iter()
            .chain(slice_notes)
            .chain(extra_notes)
            .chain(
                environment_notes
                    .iter()
                    .map(|n| format!("STK2 post-process without a 0.26g counterpart: {n}")),
            )
            .chain(
                textures
                    .failed
                    .iter()
                    .map(|f| format!("texture not read, left out: {f}")),
            )
            .chain(
                rule_notes
                    .into_iter()
                    .map(|n| format!("material by rule {n}")),
            )
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&pack_manifest)?;
    fs::write(options.output.join("pack.json"), &bytes)?;
    Ok(BuildReport {
        files: pack.paths.len(),
        spots,
        manifest_sha256: crate::content_pack::sha256_hex(&bytes),
    })
}

fn spot_notes(manifest: &LobbyManifest, spots: &BTreeMap<String, [f32; 3]>) -> Vec<String> {
    let mut notes = vec![format!(
        "made by vostok-port build-stk2-lobby-026 from the STK2 lobby_scene; the scene baked into the 0.26g lobby frame: the character's spot {:?} (STK2 world) at the origin, the view yaw {}",
        manifest.scene.character, manifest.scene.view_yaw
    )];
    for (name, p) in spots {
        notes.push(format!(
            "spot {name}: ({:.3}, {:.3}, {:.3})",
            p[0], p[1], p[2]
        ));
    }
    notes.push(format!(
        "the project's srp_lobby: weapon = spot {}, weapon_view = {}; one track object (the lobby plays it)",
        manifest.srp_lobby.weapon_spot, manifest.srp_lobby.weapon_view
    ));
    notes
}

struct Stk2Tree<'a> {
    root: &'a Path,
    /// Only list the textures: every one is taken to be there.
    dry: bool,
}

impl Stk2Tree<'_> {
    fn text(&self, path: &str) -> Result<String> {
        let full = safe_join(self.root, path)?;
        Ok(fs::read_to_string(&full)
            .with_context(|| format!("reading {}", full.display()))?
            .replace('\r', ""))
    }

    /// The LOD0 surfaces of an STK2 source model's settings and their
    /// material instances.
    fn model_surfaces(&self, settings: &str) -> Result<BTreeMap<String, String>> {
        let text = self.text(settings)?;
        let block = lua_block(&text, "material_settings")
            .with_context(|| format!("{settings}: no material_settings"))?;
        let mut surfaces = BTreeMap::new();
        for (surface, body) in lua_children(block) {
            let material = lua_string(body, "material_name")
                .with_context(|| format!("{settings}: {surface} has no material_name"))?;
            surfaces.insert(surface.to_owned(), material.replace('\\', "/"));
        }
        // some settings list no LOD0: the material_settings are the surfaces
        if let Some(lod0) = lua_block(&text, "LOD0").and_then(|lod| lua_block(lod, "surfaces")) {
            let listed: BTreeSet<&str> = lua_children_flat(lod0).into_iter().collect();
            let materials: BTreeSet<&str> = surfaces.keys().map(String::as_str).collect();
            ensure!(
                listed == materials,
                "{settings}: LOD0 surfaces {listed:?} are not the material_settings {materials:?}"
            );
        }
        Ok(surfaces)
    }

    /// The source TGA and `.options` of an STK2 texture.
    fn texture(&self, name: &str, dir: &str, more: &[String]) -> Result<(PathBuf, String)> {
        for dir in std::iter::once(dir).chain(more.iter().map(String::as_str)) {
            for extension in ["tga", "TGA"] {
                let path = safe_join(self.root, &format!("{dir}/{name}.{extension}"))?;
                if path.is_file() {
                    let options = self
                        .text(&format!("{dir}/{name}.options"))
                        .unwrap_or_default();
                    return Ok((path, options));
                }
            }
        }
        bail!("STK2 texture {name}: no {dir}/{name}.tga")
    }

    fn has_texture(&self, name: &str, dir: &str, more: &[String]) -> bool {
        self.dry || self.texture(name, dir, more).is_ok()
    }
}

/// The text of `name = { ... }` or `name = create_property_table ( { ... }`
/// up to its `-- name` end marker.
fn lua_block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0;
    while let Some(found) = text[search..].find(&format!("{name} = ")) {
        let start = search + found;
        let before = text[..start].chars().next_back();
        let line_start = before.is_none_or(|c| c == '\t' || c == '\n' || c == ' ');
        let rest = &text[start + name.len() + 3..];
        let opens = rest.starts_with('{') || rest.starts_with("create_property_table");
        if line_start && opens {
            let end_marker = format!("-- {name}\n");
            let end = rest
                .find(&end_marker)
                .or_else(|| rest.find(&format!("-- {name}")))?;
            return Some(&rest[..end]);
        }
        search = start + name.len();
    }
    None
}

/// The `name = { ... }` children of a block, one level down.
fn lua_children(block: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(marker) = rest.find("}, -- ") {
        let name_end = rest[marker + 6..]
            .find('\n')
            .map_or(rest.len(), |n| marker + 6 + n);
        let name = rest[marker + 6..name_end].trim();
        let opening = format!("{name} = {{");
        if let Some(start) = rest[..marker].rfind(&opening) {
            out.push((name, &rest[start + opening.len()..marker]));
        }
        rest = &rest[name_end..];
    }
    out
}

/// The keys of a flat table `{ a = "", b = "", }`.
fn lua_children_flat(block: &str) -> Vec<&str> {
    block
        .lines()
        .filter_map(|line| line.trim().split_once(" = "))
        .map(|(key, _)| key.trim())
        .filter(|key| !key.is_empty() && !key.starts_with('{') && !key.starts_with('}'))
        .collect()
}

fn lua_string<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let start = text.find(&format!("{key} = \""))? + key.len() + 4;
    let end = text[start..].find('"')?;
    Some(&text[start..start + end])
}

/// A texture role's value in an STK2 material: `Some(Some(texture))` set,
/// `Some(None)` set empty, `None` not set here (a reference to the base).
fn material_role(text: &str, role: &str) -> Option<Option<String>> {
    let block = lua_block(text, role)?;
    let value = lua_string(block, "value")?;
    Some((!value.is_empty()).then(|| value.replace('\\', "/")))
}

struct TextureMaker<'a> {
    stk2: &'a Stk2Tree<'a>,
    prefix: &'a str,
    dir: &'a str,
    more_dirs: &'a [String],
    max_size: u32,
    caps: &'a [TextureCap],
    work: &'a Path,
    options_template: BinaryConfig,
    /// Textures that could not be read (their surfaces left out).
    failed: Vec<String>,
    /// STK2 texture and kind -> the made texture's name.
    made: BTreeMap<(String, TextureKindKey), String>,
}

type TextureKindKey = u8;

fn kind_key(kind: TextureKind) -> TextureKindKey {
    match kind {
        TextureKind::Colour => 0,
        TextureKind::Normal => 1,
        TextureKind::Linear => 2,
    }
}

impl TextureMaker<'_> {
    fn cap(&self, name: &str) -> u32 {
        self.caps
            .iter()
            .find(|cap| name.starts_with(&cap.prefix))
            .map_or(self.max_size, |cap| cap.max.min(self.max_size))
    }

    /// The made texture's name (`<prefix>/<stk2 name>`), made once.
    fn make(&mut self, name: &str, kind: TextureKind, pack: &mut PackWriter) -> Result<String> {
        let key = (name.to_owned(), kind_key(kind));
        if let Some(done) = self.made.get(&key) {
            return Ok(done.clone());
        }
        // a resource path keeps to names without spaces (STK2 has a few)
        let target = format!("{}/{}", self.prefix, name.replace(' ', "_"));
        ensure!(
            self.made.values().all(|made| *made != target),
            "{name} is used both as colour and as data"
        );
        if self.stk2.dry {
            self.made.insert(key, target.clone());
            return Ok(target);
        }
        let (source, source_options) = self.stk2.texture(name, self.dir, self.more_dirs)?;
        let stem = self.work.join(name.replace('/', "__"));
        // a texture made before in the same work folder (same kind and
        // size) is taken again: the conversions are the slow part
        let cache = stem.with_extension(format!("k{}.c{}.cache", kind_key(kind), self.cap(name)));
        let cache_format = cache.with_extension("format");
        let (bytes, format, srgb) = if let (Ok(bytes), Ok(format)) =
            (fs::read(&cache), fs::read_to_string(&cache_format))
        {
            let (format, srgb) = format.split_once(' ').context("bad texture cache")?;
            (bytes, format.to_owned(), srgb == "1")
        } else {
            let (bytes, format, srgb) =
                self.convert(name, &target, kind, &source, &source_options, &stem)?;
            fs::write(&cache, &bytes)?;
            fs::write(&cache_format, format!("{format} {}", u8::from(srgb)))?;
            (bytes, format.to_owned(), srgb)
        };
        pack.write(&format!("textures/{target}.dds"), &bytes)?;
        let mut options = self.options_template.clone();
        options
            .root
            .set_path("options.srgb", ConfigValue::Integer(i32::from(srgb)))?;
        options
            .root
            .set_path("options.format", ConfigValue::String(format))?;
        pack.config(&format!("textures/{target}.options"), &options)?;
        self.made.insert(key, target.clone());
        Ok(target)
    }

    /// The texture's canonical DDS bytes, its options format and colour space.
    fn convert(
        &self,
        name: &str,
        target: &str,
        kind: TextureKind,
        source: &Path,
        source_options: &str,
        stem: &Path,
    ) -> Result<(Vec<u8>, &'static str, bool)> {
        let scaled = stem.with_extension("scaled.tga");
        // some STK2 TGAs are run-length packed in a way ImageMagick refuses
        // (Pillow reads them): those go through an unpacked copy
        let unpacked = stem.with_extension("unpacked.tga");
        let resize = |from: &Path| {
            magick(&[
                from.as_os_str(),
                "-resize".as_ref(),
                format!("{0}x{0}>", self.cap(name)).as_ref(),
                "-type".as_ref(),
                "TrueColorAlpha".as_ref(),
                "-compress".as_ref(),
                "none".as_ref(),
                scaled.as_os_str(),
            ])
        };
        if resize(source).is_err() {
            let output = Command::new("python3")
                .args([
                    "-I",
                    "-c",
                    "import sys; from PIL import Image; Image.open(sys.argv[1]).save(sys.argv[2], compression=None)",
                ])
                .arg(source)
                .arg(&unpacked)
                .output()
                .context("starting python3 (with Pillow) for a TGA ImageMagick refuses")?;
            ensure!(
                output.status.success(),
                "{}: neither ImageMagick nor Pillow reads it: {}",
                source.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            resize(&unpacked)?;
        }
        let has_alpha = tga_has_alpha(&scaled)?;
        let dds = stem.with_extension("dds");
        let (format, srgb) = match kind {
            TextureKind::Normal => {
                crate::armor::convert_texture(&scaled, &dds, TextureCompression::Dxt5Normal)?;
                ("NORMAL_MAP", false)
            }
            TextureKind::Colour | TextureKind::Linear => {
                let dxt5 =
                    kind == TextureKind::Colour && has_alpha && source_options.contains("\"DXT5\"");
                magick(&[
                    "-define".as_ref(),
                    if dxt5 {
                        "dds:compression=dxt5"
                    } else {
                        "dds:compression=dxt1"
                    }
                    .as_ref(),
                    scaled.as_os_str(),
                    dds.as_os_str(),
                ])?;
                (
                    if dxt5 { "DXT5" } else { "DXT1" },
                    kind == TextureKind::Colour,
                )
            }
        };
        let bytes = crate::duty_026::texture_dds(&fs::read(&dds)?, target, srgb)?;
        fs::remove_file(&scaled)?;
        fs::remove_file(&dds)?;
        let _ = fs::remove_file(&unpacked);
        Ok((bytes, format, srgb))
    }
}

fn magick(args: &[&std::ffi::OsStr]) -> Result<()> {
    let output = Command::new("magick")
        .args(args)
        .output()
        .context("starting ImageMagick (`magick`)")?;
    ensure!(
        output.status.success(),
        "ImageMagick {args:?}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// Whether an uncompressed TGA has an alpha channel that is not all opaque.
fn tga_has_alpha(path: &Path) -> Result<bool> {
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() >= 18 && bytes[2] == 2,
        "{}: not an uncompressed TGA",
        path.display()
    );
    if bytes[16] != 32 {
        return Ok(false);
    }
    let start = 18 + usize::from(bytes[0]);
    Ok(bytes[start..].chunks_exact(4).any(|pixel| pixel[3] != 255))
}

/// The material's name in the models' settings, and the texture its
/// diffuse slot binds; `None` for a hidden one.
#[allow(clippy::too_many_arguments)]
fn make_material(
    key: &str,
    spec: &MaterialSpec,
    emission_scale: &BTreeMap<String, f32>,
    prefix: &str,
    stk2: &Stk2Tree<'_>,
    target: &mut ResourceDb,
    textures: &mut TextureMaker<'_>,
    pack: &mut PackWriter,
) -> Result<Option<(String, Option<String>)>> {
    let (template, source, explicit, emission) = match spec {
        MaterialSpec::Hidden => return Ok(None),
        MaterialSpec::Native { material } => {
            ensure!(
                target.contains(&format!("material_instances/{material}.material")),
                "{key}: native material {material} is not in the target"
            );
            return Ok(Some((material.clone(), None)));
        }
        MaterialSpec::Converted {
            template,
            source,
            textures,
            emission,
        } => (template, source, textures, emission),
    };
    let roles = material_textures(key, source.as_deref(), explicit, stk2)?;
    // an instance with no diffuse of its own or its base's (a decal's
    // projector, ...): its surfaces are left out
    if !roles.contains_key("texture_diffuse") {
        textures.failed.push(format!("{key}: no diffuse texture"));
        return Ok(None);
    }
    let mut material = parse(target, &format!("material_instances/{template}.material"))?;
    let effect = "material.g_stage.effect";
    let mut diffuse = None;
    for (role, slot, kind, flag) in ROLES {
        let Some(texture) = roles.get(*role) else {
            continue;
        };
        if let Some(flag) = flag {
            let on = matches!(
                crate::content_pack::value::value_at(
                    &material.root,
                    &format!("{effect}.{flag}.value")
                ),
                Some(ConfigValue::Boolean(true))
            );
            if !on {
                continue;
            }
        }
        // a texture the STK2 trees lack: a surface without its diffuse is
        // left out, another slot keeps the template's binding
        if !stk2.has_texture(texture, textures.dir, textures.more_dirs) {
            if *slot == "texture_diffuse" {
                return Ok(None);
            }
            continue;
        }
        let made = match textures.make(texture, *kind, pack) {
            Ok(made) => made,
            Err(error) => {
                textures.failed.push(format!("{texture}: {error:#}"));
                if *slot == "texture_diffuse" {
                    return Ok(None);
                }
                continue;
            }
        };
        if *slot == "texture_diffuse" {
            diffuse = Some(made.clone());
        }
        material.root.set_path(
            &format!("{effect}.{slot}.value"),
            ConfigValue::String(made.clone()),
        )?;
        if *slot == "texture_normal"
            && crate::content_pack::value::value_at(
                &material.root,
                &format!("{effect}.texture_detail_normal.value"),
            )
            .is_some()
        {
            material.root.set_path(
                &format!("{effect}.texture_detail_normal.value"),
                ConfigValue::String(made),
            )?;
        }
    }
    if let Some(scale) = emission {
        let source = source
            .as_deref()
            .with_context(|| format!("{key}: emission kept without an STK2 instance"))?;
        let values = material_values(stk2, source)?;
        let colour = values
            .get("constant_emissive")
            .and_then(|v| lua_float4(v))
            .unwrap_or([1.0; 4]);
        let multiplier = values
            .get("constant_emissive_multiplier")
            .and_then(|v| lua_number(v))
            .unwrap_or(1.0);
        let scale = scale * emission_scale.get(key).copied().unwrap_or(1.0);
        set_emission(&mut material.root, colour, multiplier * scale, diffuse.as_deref())
            .with_context(|| format!("{key}: emission"))?;
    }
    // `tex:<texture>` keys (a surface known by its texture) as a folder
    let name = format!("{prefix}/{}", key.replace(':', "/").replace(' ', "_"));
    pack.config(&format!("material_instances/{name}.material"), &material)?;
    Ok(Some((name, diffuse)))
}

/// The spec of a material `materials` does not name: its STK2 instance
/// (found in `material_roots`) read for its base material and flags, the
/// first rule that matches. Also says why, for the pack's notes.
fn material_by_rule(
    key: &str,
    manifest: &LobbyManifest,
    stk2: &Stk2Tree<'_>,
) -> Result<(MaterialSpec, String)> {
    if key.is_empty() {
        return Ok((MaterialSpec::Hidden, "no material -> hidden".to_owned()));
    }
    let key = match key.strip_prefix("tex:") {
        None => key.to_owned(),
        Some("") => return Ok((MaterialSpec::Hidden, "no texture -> hidden".to_owned())),
        Some(texture) => match instance_with_diffuse(manifest, stk2, texture)? {
            Some(instance) => instance,
            None => {
                let template = manifest.fallback_template.clone().with_context(|| {
                    format!("{key}: no instance binds the texture and no fallback_template")
                })?;
                let mut textures = BTreeMap::new();
                textures.insert("texture_diffuse".to_owned(), texture.to_owned());
                let normal = format!("{texture}_nmap");
                if stk2.has_texture(&normal, &manifest.textures_dir, &manifest.more_texture_dirs) {
                    textures.insert("texture_normal".to_owned(), normal);
                }
                if !stk2.has_texture(texture, &manifest.textures_dir, &manifest.more_texture_dirs) {
                    return Ok((
                        MaterialSpec::Hidden,
                        format!("{texture}: no source -> hidden"),
                    ));
                }
                return Ok((
                    MaterialSpec::Converted {
                        template: template.clone(),
                        source: None,
                        textures,
                        emission: None,
                    },
                    format!("its texture alone -> {template}"),
                ));
            }
        },
    };
    let key = key.as_str();
    let source = manifest
        .material_roots
        .iter()
        .flat_map(|root| {
            ["material", "material.orig"]
                .map(|ext| format!("{root}/material_instances/{key}.{ext}"))
        })
        .find(|path| safe_join(stk2.root, path).is_ok_and(|p| p.is_file()))
        .with_context(|| format!("STK2 material {key}: no instance in the material roots"))?;
    let text = stk2.text(&source)?;
    let base = lua_string(&text, "base_material").unwrap_or("").to_owned();
    let base_text = Path::new(&source)
        .ancestors()
        .find(|p| p.ends_with("material_instances"))
        .map(|dir| {
            dir.with_file_name("materials")
                .join(format!("{base}.material"))
        })
        .and_then(|path| stk2.text(&path.to_string_lossy()).ok());
    let flag = |name: &str| {
        let own = lua_block(&text, name).and_then(|block| lua_bool(block, "value"));
        own.or_else(|| {
            base_text
                .as_deref()
                .and_then(|base| lua_block(base, name))
                .and_then(|block| lua_bool(block, "value"))
        })
        .unwrap_or(false)
    };
    let (alpha_test, two_sided) = (flag("use_alpha_test"), flag("two_sided"));
    let rule = manifest
        .material_rules
        .iter()
        .find(|rule| {
            (rule.bases.iter().any(|b| b == "*" || *b == base))
                && rule.alpha_test.is_none_or(|v| v == alpha_test)
                && rule.two_sided.is_none_or(|v| v == two_sided)
        })
        .with_context(|| {
            format!(
                "STK2 material {key} ({base}, alpha {alpha_test}, two-sided {two_sided}): no rule"
            )
        })?;
    let why = format!("{base}, alpha {alpha_test}, two-sided {two_sided}");
    Ok(match &rule.then {
        RuleTarget::Converted { template, emission } => (
            MaterialSpec::Converted {
                template: template.clone(),
                source: Some(source),
                textures: BTreeMap::new(),
                emission: *emission,
            },
            match emission {
                Some(k) => format!("{why} -> {template}, its emission x{k}"),
                None => format!("{why} -> {template}"),
            },
        ),
        RuleTarget::Native { material } => (
            MaterialSpec::Native {
                material: material.clone(),
            },
            format!("{why} -> native {material}"),
        ),
        RuleTarget::Hidden => (MaterialSpec::Hidden, format!("{why} -> hidden")),
    })
}

/// The first material instance (by name) of the roots whose own
/// `texture_diffuse` is `texture`.
fn instance_with_diffuse(
    manifest: &LobbyManifest,
    stk2: &Stk2Tree<'_>,
    texture: &str,
) -> Result<Option<String>> {
    for root in &manifest.material_roots {
        let dir = safe_join(stk2.root, &format!("{root}/material_instances"))?;
        let mut paths = Vec::new();
        collect_materials(&dir, &mut paths)?;
        paths.sort();
        for path in paths {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let text = text.replace('\r', "");
            if material_role(&text, "texture_diffuse") == Some(Some(texture.to_owned())) {
                let name = path
                    .strip_prefix(&dir)?
                    .to_string_lossy()
                    .trim_end_matches(".orig")
                    .trim_end_matches(".material")
                    .replace('\\', "/");
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

fn collect_materials(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    // names the file system cannot give (a stray encoding) are skipped
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_materials(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "material") {
            out.push(path);
        }
    }
    Ok(())
}

/// An STK2 material's `value = ...` leaves by the name of the table
/// holding them (`environment_far_fog_color`, ...): its base material's
/// first, its own over them.
fn material_values(stk2: &Stk2Tree<'_>, instance: &str) -> Result<BTreeMap<String, String>> {
    fn leaves(text: &str, out: &mut BTreeMap<String, String>) {
        let mut stack: Vec<String> = Vec::new();
        for line in text.lines() {
            let depth = line.len() - line.trim_start_matches('\t').len();
            let body = line.trim();
            if let Some(name) = body
                .strip_suffix(" = {")
                .or_else(|| body.strip_suffix(" = create_property_table ( {"))
            {
                stack.truncate(depth);
                stack.push(name.to_owned());
            } else if let Some(value) = body
                .strip_prefix("value = ")
                .and_then(|v| v.strip_suffix(','))
            {
                if let Some(name) = stack.iter().rev().find(|n| *n != "__super_tables") {
                    out.insert(name.clone(), value.to_owned());
                }
            } else if body.starts_with('}') {
                stack.truncate(depth);
            }
        }
    }
    let text = stk2.text(instance)?;
    let mut values = BTreeMap::new();
    if let Some(base) = lua_string(&text, "base_material") {
        let dir = Path::new(instance)
            .ancestors()
            .find(|p| p.ends_with("material_instances"))
            .with_context(|| format!("{instance} is not under material_instances"))?
            .with_file_name("materials");
        let base_path = dir.join(format!("{base}.material"));
        leaves(&stk2.text(&base_path.to_string_lossy())?, &mut values);
    }
    leaves(&text, &mut values);
    Ok(values)
}

fn lua_number(value: &str) -> Option<f32> {
    value.trim().parse().ok()
}

fn lua_float4(value: &str) -> Option<[f32; 4]> {
    let inner = value.trim().strip_prefix("float4(")?.strip_suffix(')')?;
    let parts: Vec<f32> = inner
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]])
}

/// The STK2 post-process onto a 0.26g environment object's `properties`:
/// what has a counterpart is set, the effects STK2's post-process lacks are
/// off; says what could not be mapped.
fn apply_post_process(
    object: &mut ConfigValue,
    values: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let set = |object: &mut ConfigValue, key: &str, value: ConfigValue| -> Result<()> {
        let path = format!("properties.{key}");
        if crate::content_pack::value::value_at(object, &path).is_some() {
            let value = match value {
                ConfigValue::Float(v) => number_like(object, &path, v),
                other => other,
            };
            object.set_path(&path, value)?;
        }
        Ok(())
    };
    let get = |key: &str| values.get(key).map(String::as_str);
    if let Some(color) = get("environment_far_fog_color").and_then(lua_float4) {
        set(object, "mie_fog_color", ConfigValue::Float4(color))?;
    }
    for (from, to) in [
        ("environment_near_fog_distance", "mie_fog_near_distance"),
        ("environment_far_fog_distance", "mie_fog_far_distance"),
        ("adaptation_speed", "eye_adaptation_speed"),
        ("bloom_scale", "bloom_ratio"),
    ] {
        if let Some(v) = get(from).and_then(lua_number) {
            set(object, to, ConfigValue::Float(v))?;
        }
    }
    if let Some(color) = get("environment_sun_color").and_then(lua_float4) {
        set(object, "sun_color", ConfigValue::Float4(color))?;
    }
    if get("enable_bloom") == Some("false") {
        set(object, "bloom_intensity", ConfigValue::Float(0.0))?;
    }
    if get("use_color_grading_lut") != Some("true") {
        set(
            object,
            "use_color_grading_texture",
            ConfigValue::Boolean(false),
        )?;
    }
    // effects STK2's post-process has no counterpart of: off
    set(object, "image_grain_intensity", ConfigValue::Float(0.0))?;
    set(object, "aberration_amount", ConfigValue::Float(0.0))?;
    set(object, "vignette_effect_power", ConfigValue::Float(0.0))?;
    set(object, "god_rays_intensity", ConfigValue::Float(0.0))?;
    set(
        object,
        "use_dynamic_lens_flare",
        ConfigValue::Boolean(false),
    )?;
    for key in [
        "environment_ambient_color",
        "environment_skylight_upper_color",
        "environment_skylight_lower_power",
        "environment_skylight_upper_power",
        "tonemap_median",
        "tonemap_bright_threshold",
        "tonemap_darkness_threshold",
        "frame_height_lights",
        "frame_shadows",
        "environment_shadow_transparency",
        "bloom_max_color",
        "dof_focus_distance",
    ] {
        if let Some(value) = get(key) {
            notes.push(format!("{key} = {value}"));
        }
    }
    Ok(notes)
}

fn lua_bool(text: &str, key: &str) -> Option<bool> {
    let start = text.find(&format!("{key} = "))? + key.len() + 3;
    let rest = &text[start..];
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// The STK2 textures of a material by role: `explicit` first, then the
/// instance's own values, then its base material's for what it inherits.
fn material_textures(
    key: &str,
    source: Option<&str>,
    explicit: &BTreeMap<String, String>,
    stk2: &Stk2Tree<'_>,
) -> Result<BTreeMap<String, String>> {
    let mut roles = explicit.clone();
    let Some(source) = source else {
        ensure!(!roles.is_empty(), "{key}: neither a source nor textures");
        return Ok(roles);
    };
    let text = stk2.text(source)?;
    let base = lua_string(&text, "base_material").map(str::to_owned);
    let base_text = match &base {
        Some(base) => {
            let dir = Path::new(source)
                .ancestors()
                .find(|p| p.ends_with("material_instances"))
                .with_context(|| format!("{source} is not under material_instances"))?
                .with_file_name("materials");
            Some(stk2.text(&format!("{}/{base}.material", dir.display()))?)
        }
        None => None,
    };
    for (role, ..) in ROLES {
        if roles.contains_key(*role) {
            continue;
        }
        let value = match material_role(&text, role) {
            Some(value) => value,
            None => base_text
                .as_deref()
                .and_then(|base| material_role(base, role))
                .flatten(),
        };
        // STK2's editor placeholders (`editor/default_spec`, ...) are
        // nothing to port: the template keeps its own binding.
        if let Some(texture) = value.filter(|t| !t.starts_with("editor/")) {
            roles.insert((*role).to_owned(), texture);
        }
    }
    Ok(roles)
}

struct StaticModelTemplates {
    settings: BinaryConfig,
    render_properties: BinaryConfig,
    surface_properties: BinaryConfig,
}

struct BuiltSurface {
    name: String,
    material: String,
    mesh: Vec<u8>,
    bounds: Bounds,
    vertices: u32,
    indices: u32,
    mesh_name: String,
    texture_name: String,
}

/// Converted 0.26g vertices (28 bytes each) and their triangles.
#[derive(Clone, Debug, Default)]
struct Mesh {
    vertices: Vec<[u8; VERTEX_SIZE]>,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// An STK2 surface (u32 n, n 56-byte vertices; u32 m, m u16 indices)
    /// moved by `placement` (a mirroring one turns the triangles over).
    fn stk2(vertices: &[u8], indices: &[u8], placement: Rigid, label: &str) -> Result<Mesh> {
        let count = usize::try_from(read_u32(vertices, 0)?)?;
        let bases = repaired_static_bases(vertices, indices, label)?;
        let mut mesh = Mesh::default();
        for (index, basis) in bases.iter().enumerate() {
            let source = &vertices[4 + index * STK2_VERTEX_SIZE..][..STK2_VERTEX_SIZE];
            let position = placement.apply(read_float3(source, 0)?);
            let mut vertex = [0u8; VERTEX_SIZE];
            for (axis, value) in position.iter().enumerate() {
                vertex[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            let moved = [
                placement.normal(basis[0]),
                placement.rotate(basis[1]),
                placement.rotate(basis[2]),
            ];
            for (slot, (vector, alpha)) in moved.iter().zip([127, 255, 255]).enumerate() {
                vertex[12 + slot * 4..16 + slot * 4]
                    .copy_from_slice(&packed_basis(*vector, alpha).to_le_bytes());
            }
            for (slot, offset) in [48, 52].into_iter().enumerate() {
                vertex[24 + slot * 2..26 + slot * 2]
                    .copy_from_slice(&half(read_f32(source, offset)?)?.to_le_bytes());
            }
            mesh.vertices.push(vertex);
            mesh.positions.push(position);
        }
        let index_count = usize::try_from(read_u32(indices, 0)?)?;
        let flip = placement.mirrors();
        for triangle in 0..index_count / 3 {
            let mut corners = [0u32; 3];
            for (corner, value) in corners.iter_mut().enumerate() {
                let at = 4 + (triangle * 3 + corner) * 2;
                *value = u32::from(u16::from_le_bytes([indices[at], indices[at + 1]]));
                ensure!(
                    (*value as usize) < count,
                    "{label}: index outside its vertices"
                );
            }
            if flip {
                corners.swap(1, 2);
            }
            mesh.triangles.push(corners);
        }
        Ok(mesh)
    }

    /// Only the triangles with a corner `inside`, their vertices
    /// renumbered.
    fn clipped(&self, inside: impl Fn([f32; 3]) -> bool) -> Mesh {
        let mut out = Mesh::default();
        let mut renumbered = vec![u32::MAX; self.vertices.len()];
        for triangle in &self.triangles {
            if !triangle.iter().any(|&i| inside(self.positions[i as usize])) {
                continue;
            }
            let corners = triangle.map(|i| {
                let slot = &mut renumbered[i as usize];
                if *slot == u32::MAX {
                    *slot = out.vertices.len() as u32;
                    out.vertices.push(self.vertices[i as usize]);
                    out.positions.push(self.positions[i as usize]);
                }
                *slot
            });
            out.triangles.push(corners);
        }
        out
    }

    fn append(&mut self, other: &Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.positions.extend_from_slice(&other.positions);
        self.triangles
            .extend(other.triangles.iter().map(|t| t.map(|i| i + base)));
    }

    fn bounds(&self) -> Bounds {
        let mut bounds = Bounds::empty();
        for p in &self.positions {
            bounds.add(*p);
        }
        bounds
    }

    /// Pieces of at most `limit` vertices (whole triangles).
    fn split(&self, limit: usize) -> Vec<Mesh> {
        let mut pieces = Vec::new();
        let mut piece = Mesh::default();
        let mut renumbered: BTreeMap<u32, u32> = BTreeMap::new();
        for triangle in &self.triangles {
            let new = triangle
                .iter()
                .filter(|i| !renumbered.contains_key(i))
                .count();
            if piece.vertices.len() + new > limit {
                pieces.push(std::mem::take(&mut piece));
                renumbered.clear();
            }
            let corners = triangle.map(|i| {
                *renumbered.entry(i).or_insert_with(|| {
                    piece.vertices.push(self.vertices[i as usize]);
                    piece.positions.push(self.positions[i as usize]);
                    (piece.vertices.len() - 1) as u32
                })
            });
            piece.triangles.push(corners);
        }
        if !piece.triangles.is_empty() {
            pieces.push(piece);
        }
        pieces
    }

    /// The 0.26g `converted_model`: the vertex block (3) and the u16 index
    /// block (4).
    fn encode(&self, label: &str) -> Result<Vec<u8>> {
        let count = self.vertices.len();
        ensure!(
            count <= MAX_STATIC_SURFACE_VERTICES,
            "{label}: {count} vertices, more than 0.26g loads ({MAX_STATIC_SURFACE_VERTICES})"
        );
        let indices = self.triangles.len() * 3;
        let mut out = Vec::with_capacity(24 + count * VERTEX_SIZE + indices * 2);
        push_u32(&mut out, 3);
        push_u32(&mut out, u32::try_from(4 + count * VERTEX_SIZE)?);
        push_u32(&mut out, u32::try_from(count)?);
        for vertex in &self.vertices {
            out.extend_from_slice(vertex);
        }
        push_u32(&mut out, 4);
        push_u32(&mut out, u32::try_from(4 + indices * 2)?);
        push_u32(&mut out, u32::try_from(indices)?);
        for triangle in &self.triangles {
            for &index in triangle {
                out.extend_from_slice(&u16::try_from(index)?.to_le_bytes());
            }
        }
        Ok(out)
    }
}

/// An STK2 surface as a 0.26g `converted_model` moved by `placement`.
fn convert_surface(
    vertices: &[u8],
    indices: &[u8],
    placement: Rigid,
    label: &str,
) -> Result<(Vec<u8>, Bounds)> {
    let mesh = Mesh::stk2(vertices, indices, placement, label)?;
    Ok((mesh.encode(label)?, mesh.bounds()))
}

/// A collision vertex chunk (0x19, float3s) moved by `placement`.
fn move_collision(vertices: &[u8], placement: Rigid) -> Result<Vec<u8>> {
    let size = usize::try_from(read_u32(vertices, 4)?)?;
    let mut out = vertices[..8].to_vec();
    for offset in (8..8 + size).step_by(12) {
        for value in placement.apply(read_float3(vertices, offset)?) {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&vertices[8 + size..]);
    Ok(out)
}

/// A square of `2 half` metres at `height`, about the origin, facing up,
/// one uv unit every `tile` metres, as a 0.26g `converted_model`: a grid of
/// 4 m cells so that the vertex light and fog interpolate.
fn ground_plane(height: f32, half: f32, tile: f32) -> Result<(Vec<u8>, Bounds, u32, u32)> {
    ensure!(
        half > 0.0 && tile > 0.0,
        "ground: half size and tile must be positive"
    );
    let cells = ((2.0 * half / 4.0).ceil() as usize).clamp(1, 64);
    let step = 2.0 * half / cells as f32;
    let side = cells + 1;
    let mut out = Vec::new();
    push_u32(&mut out, 3);
    push_u32(&mut out, u32::try_from(4 + side * side * VERTEX_SIZE)?);
    push_u32(&mut out, u32::try_from(side * side)?);
    let mut bounds = Bounds::empty();
    // the tangent along +x, the binormal along -z (uv v grows towards -z),
    // as the native floors.
    let normal = packed_basis([0.0, 1.0, 0.0], 127);
    let tangent = packed_basis([1.0, 0.0, 0.0], 255);
    let binormal = packed_basis([0.0, 0.0, -1.0], 255);
    for row in 0..side {
        for column in 0..side {
            let x = -half + column as f32 * step;
            let z = -half + row as f32 * step;
            bounds.add([x, height, z]);
            for value in [x, height, z] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for packed in [normal, tangent, binormal] {
                push_u32(&mut out, packed);
            }
            for value in [x / tile, -z / tile] {
                out.extend_from_slice(&half_float(value)?.to_le_bytes());
            }
        }
    }
    let mut indices = Vec::new();
    for row in 0..cells {
        for column in 0..cells {
            let a = row * side + column;
            let b = a + 1;
            let c = a + side;
            let d = c + 1;
            // the turn of the STK2 hut floor's triangles
            indices.extend([a, c, b, b, c, d]);
        }
    }
    let count = u32::try_from(indices.len())?;
    push_u32(&mut out, 4);
    push_u32(&mut out, u32::try_from(4 + indices.len() * 2)?);
    push_u32(&mut out, count);
    for index in indices {
        out.extend_from_slice(&u16::try_from(index)?.to_le_bytes());
    }
    Ok((out, bounds, u32::try_from(side * side)?, count))
}

fn write_static_model(
    name: &str,
    source: &str,
    surfaces: &[BuiltSurface],
    collision: Option<(Vec<u8>, Vec<u8>, usize)>,
    templates: &StaticModelTemplates,
    pack: &mut PackWriter,
) -> Result<()> {
    let root = format!("models/{name}.model");
    let mut all = Bounds::empty();
    for surface in surfaces {
        all.union(&surface.bounds);
        pack.write(
            &format!("{root}/render/{}/converted_model", surface.name),
            &surface.mesh,
        )?;
        let mut properties = templates.surface_properties.clone();
        let b = surface.bounds;
        let centre = b.centre();
        let radius = [0, 1, 2]
            .map(|a| (b.max[a] - b.min[a]) * 0.5)
            .iter()
            .map(|h| h * h)
            .sum::<f32>()
            .sqrt();
        for (path, value) in [
            ("bounding_box.min", ConfigValue::Float3(b.min)),
            ("bounding_box.max", ConfigValue::Float3(b.max)),
            ("bounding_sphere.center", ConfigValue::Float3(centre)),
            ("bounding_sphere.radius", ConfigValue::Float(radius)),
            (
                "source.vertices",
                ConfigValue::Integer(i32::try_from(surface.vertices)?),
            ),
            (
                "source.indices",
                ConfigValue::Integer(i32::try_from(surface.indices)?),
            ),
            (
                "source.primitives",
                ConfigValue::Integer(i32::try_from(surface.indices / 3)?),
            ),
            (
                "source.mesh_name",
                ConfigValue::String(surface.mesh_name.clone()),
            ),
            (
                "source.texture_name",
                ConfigValue::String(surface.texture_name.clone()),
            ),
        ] {
            properties.root.set_path(path, value)?;
        }
        pack.config(
            &format!("{root}/render/{}/export_properties", surface.name),
            &properties,
        )?;
    }
    let mut properties = templates.render_properties.clone();
    properties
        .root
        .set_path("bounding_box.min", ConfigValue::Float3(all.min))?;
    properties
        .root
        .set_path("bounding_box.max", ConfigValue::Float3(all.max))?;
    properties
        .root
        .set_path("src_filename", ConfigValue::String(source.to_owned()))?;
    pack.config(&format!("{root}/render/export_properties"), &properties)?;

    let mut settings = templates.settings.clone();
    settings.root.set_path(
        "lod_hierrarchy",
        ConfigValue::Named(vec![named(
            "LOD0",
            ConfigValue::Named(vec![named(
                "surfaces",
                ConfigValue::Indexed(
                    surfaces
                        .iter()
                        .map(|s| ConfigValue::String(s.name.clone()))
                        .collect(),
                ),
            )]),
        )]),
    )?;
    settings.root.set_path(
        "material_settings",
        ConfigValue::Named(
            surfaces
                .iter()
                .map(|s| {
                    named(
                        &s.name,
                        ConfigValue::Named(vec![
                            named("material_name", ConfigValue::String(s.material.clone())),
                            named("used_in_portals", ConfigValue::Boolean(false)),
                        ]),
                    )
                })
                .collect(),
        ),
    )?;
    pack.config(&format!("{root}/settings"), &settings)?;

    if let Some((vertices, indices, triangles)) = collision {
        pack.write(&format!("{root}/collision/vertices"), &vertices)?;
        pack.write(&format!("{root}/collision/indices"), &indices)?;
        pack.write(
            &format!("{root}/collision/face_data"),
            &make_stk2_collision_face_data(triangles)?,
        )?;
    }
    Ok(())
}

/// The template project with its visuals, collision and client objects
/// replaced.
fn replace_project(
    project: &mut BinaryConfig,
    visuals: &[String],
    collisions: &[String],
    objects: &[ObjectSpec],
    slice_lights: &[(&SliceLights, SliceLight)],
    light_overrides: &BTreeMap<String, LightOverride>,
    scene: Rigid,
) -> Result<()> {
    let identity = |lib: ConfigValue, extra: Vec<NamedValue>| {
        let mut fields = vec![
            named("rotation", ConfigValue::Float3([0.0; 3])),
            named("position", ConfigValue::Float3([0.0; 3])),
        ];
        fields.extend(extra);
        fields.insert(2, named("lib_name", lib));
        fields.push(named("scale", ConfigValue::Float3([1.0; 3])));
        ConfigValue::Named(fields)
    };
    project.root.set_path(
        "render_visuals",
        ConfigValue::Indexed(
            visuals
                .iter()
                .map(|v| {
                    identity(
                        ConfigValue::String(v.clone()),
                        vec![named("visual_name", ConfigValue::String(v.clone()))],
                    )
                })
                .collect(),
        ),
    )?;
    project.root.set_path(
        "collision_objects",
        ConfigValue::Indexed(
            collisions
                .iter()
                .map(|v| {
                    let mut value = identity(
                        ConfigValue::String(format!("resources/models/{v}.model/collision")),
                        Vec::new(),
                    );
                    if let ConfigValue::Named(fields) = &mut value {
                        fields.insert(
                            2,
                            named("cmask", ConfigValue::Integer(CAMERA_COLLISION_MASK)),
                        );
                        let scale = fields.len() - 1;
                        fields.insert(
                            scale,
                            named("cgroup", ConfigValue::Integer(CAMERA_COLLISION_GROUP)),
                        );
                    }
                    value
                })
                .collect(),
        ),
    )?;
    let template: Vec<ConfigValue> = match project.root.named("client_objects") {
        Some(ConfigValue::Indexed(objects)) => objects.clone(),
        _ => bail!("the template project has no client_objects table"),
    };
    let by_name = |name: &str| {
        template
            .iter()
            .find(|o| matches!(o.named("name"), Some(ConfigValue::String(n)) if n == name))
    };
    let mut names = BTreeSet::new();
    let mut made = Vec::new();
    for spec in objects {
        let mut object = by_name(&spec.from)
            .with_context(|| format!("the template project has no client object {:?}", spec.from))?
            .clone();
        let name = spec.name.clone().unwrap_or_else(|| spec.from.clone());
        ensure!(
            names.insert(name.clone()),
            "two client objects named {name:?}"
        );
        if spec.name.is_some() {
            object.set_path("name", ConfigValue::String(name.clone()))?;
            object.set_path(
                "full_name",
                ConfigValue::String(format!("stk2_lobby/{name}")),
            )?;
        }
        if let Some(position) = spec.position {
            object.set_path("position", ConfigValue::Float3(scene.apply(position)))?;
        }
        let ambient = matches!(object.named("lib_name"), Some(ConfigValue::String(l)) if l == "ambient_light");
        for (path, value) in &spec.set {
            let hint = crate::content_pack::value::value_at(&object, path).cloned();
            let value = match (ambient, path.as_str(), value) {
                // an ambient light's colour is one i32, red in the low byte
                (true, "color", Json::Array(rgb)) => {
                    let rgb: Vec<f32> = rgb
                        .iter()
                        .map(|v| v.as_f64().map(|v| v as f32))
                        .collect::<Option<_>>()
                        .filter(|v: &Vec<f32>| v.len() == 3)
                        .with_context(|| format!("{name}: color is not 3 numbers"))?;
                    ConfigValue::Integer(packed_colour([rgb[0], rgb[1], rgb[2]]))
                }
                _ => crate::content_pack::value::to_config(value, hint.as_ref())
                    .with_context(|| format!("{name}: {path}"))?,
            };
            object.set_path(path, value)?;
        }
        made.push(object);
    }
    for (index, (spec, light)) in slice_lights.iter().enumerate() {
        let template = match light.kind {
            LightKind::Point => Some(&spec.point_template),
            LightKind::Spot => spec.spot_template.as_ref().or(Some(&spec.point_template)),
            LightKind::Sun => spec.sun_template.as_ref(),
        };
        let Some(template) = template else {
            continue;
        };
        let mut object = by_name(template)
            .with_context(|| format!("the template project has no client object {template:?}"))?
            .clone();
        let name = match light.kind {
            LightKind::Sun => "sun".to_owned(),
            _ => format!("stk2_light_{index:03}"),
        };
        ensure!(
            names.insert(name.clone()),
            "two client objects named {name:?}"
        );
        let set = light_overrides.get(&name);
        if set.and_then(|o| o.intensity) == Some(0.0) {
            continue;
        }
        object.set_path("name", ConfigValue::String(name.clone()))?;
        object.set_path(
            "full_name",
            ConfigValue::String(format!("stk2_lobby/{}", light.source)),
        )?;
        object.set_path("position", ConfigValue::Float3(light.position))?;
        object.set_path("rotation", ConfigValue::Float3(light.rotation))?;
        object.set_path("color", ConfigValue::Float3(light.color))?;
        let value = number_like(&object, "intensity", light.intensity * spec.intensity_scale);
        object.set_path("intensity", value)?;
        if light.kind != LightKind::Sun {
            let value = number_like(&object, "range", light.range * spec.range_scale);
            object.set_path("range", value)?;
            let value = number_like(&object, "attenuation_power", light.attenuation_power);
            object.set_path("attenuation_power", value)?;
        }
        object.set_path("is_cast_shadow", ConfigValue::Boolean(light.cast_shadow))?;
        let value = number_like(&object, "diffuse_influence_factor", light.influence[0]);
        object.set_path("diffuse_influence_factor", value)?;
        let value = number_like(&object, "specular_influence_factor", light.influence[1]);
        object.set_path("specular_influence_factor", value)?;
        if let Some([umbra, penumbra, falloff]) = light.cone {
            let value = number_like(&object, "spot_umbra_angle", umbra);
            object.set_path("spot_umbra_angle", value)?;
            let value = number_like(&object, "spot_penumbra_angle", penumbra);
            object.set_path("spot_penumbra_angle", value)?;
            let value = number_like(&object, "spot_falloff", falloff);
            object.set_path("spot_falloff", value)?;
        }
        object.set_path("layer", ConfigValue::String("default".to_owned()))?;
        if let Some(set) = set {
            if let Some(v) = set.intensity {
                let value = number_like(&object, "intensity", v);
                object.set_path("intensity", value)?;
            }
            if let Some(v) = set.range {
                let value = number_like(&object, "range", v);
                object.set_path("range", value)?;
            }
            if let Some(c) = set.color {
                object.set_path("color", ConfigValue::Float3(c))?;
            }
            if let Some(v) = set.cast_shadow {
                object.set_path("is_cast_shadow", ConfigValue::Boolean(v))?;
            }
        }
        made.push(object);
    }
    if let Some(unknown) = light_overrides.keys().find(|k| !names.contains(*k)) {
        bail!("light_overrides: no converted light {unknown:?}");
    }
    project
        .root
        .set_path("client_objects", ConfigValue::Indexed(made))?;
    // the editor's camera: where the lobby's camera starts
    project.root.set_path(
        "camera",
        ConfigValue::Named(vec![
            named("distance", ConfigValue::Float(1.96)),
            named("direction", ConfigValue::Float3([0.0, 0.0, 1.0])),
            named("position", ConfigValue::Float3([0.0, 1.2, -1.96])),
        ]),
    )?;
    Ok(())
}

/// A kept object of a slice: its model, surfaces and placement in the
/// lobby frame.
struct SliceModel {
    library: String,
    archive: PathBuf,
    surfaces: BTreeMap<String, String>,
    placement: Rigid,
}

/// A project light inside a slice's box, in the lobby frame.
#[derive(Clone, Debug)]
pub(crate) struct SliceLight {
    source: String,
    kind: LightKind,
    position: [f32; 3],
    /// Euler angles in the lobby frame (the engine's create_rotation).
    rotation: [f32; 3],
    color: [f32; 3],
    intensity: f32,
    range: f32,
    attenuation_power: f32,
    cast_shadow: bool,
    /// How much it lights the diffuse and the specular (STK2's sun lights
    /// a scene's every surface unshadowed through these).
    influence: [f32; 2],
    /// Spot cone fields as authored (umbra, penumbra, falloff).
    cone: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightKind {
    Point,
    Spot,
    Sun,
}

struct SliceObjects {
    models: Vec<SliceModel>,
    lights: Vec<SliceLight>,
    /// solid visuals of the project
    total: usize,
    /// libraries with no exported model in the roots
    missing: Vec<String>,
}

/// The project's solid visuals (composites' children too) whose model is
/// found and not excluded, and its point, spot and sphere lights inside the
/// box. Whether an object's triangles fall in the box is the mesh's to
/// tell (write_slice clips them).
fn slice_objects(slice: &SliceSpec, stk2: &Stk2Tree<'_>, scene: Rigid) -> Result<SliceObjects> {
    let (objects, _) = crate::stk2_project::parse_project_with_composite_lights(
        &safe_join(stk2.root, &slice.project)?,
        &safe_join(stk2.root, &slice.composites)?,
    )?;
    let inside = |p: [f32; 3]| (0..3).all(|a| p[a] >= slice.box_min[a] && p[a] <= slice.box_max[a]);
    let mut out = SliceObjects {
        models: Vec::new(),
        lights: Vec::new(),
        total: 0,
        missing: Vec::new(),
    };
    let mut missing = BTreeSet::new();
    let mut swapped = vec![0usize; slice.material_swaps.len()];
    for object in &objects {
        match object.kind.as_str() {
            "solid_visual" if slice.geometry => {
                out.total += 1;
                let Some(library) = &object.library_name else {
                    continue;
                };
                if slice
                    .exclude
                    .iter()
                    .any(|prefix| library.starts_with(prefix))
                {
                    continue;
                }
                let archive = slice.model_roots.iter().find_map(|root| {
                    let archive = safe_join(
                        stk2.root,
                        &format!("{root}/exported/models/{library}.model"),
                    )
                    .ok()?;
                    archive.is_file().then_some(archive)
                });
                let Some(archive) = archive else {
                    missing.insert(library.clone());
                    continue;
                };
                let settings = slice.model_roots.iter().find_map(|root| {
                    let settings = format!("{root}/sources/models/{library}.model/settings");
                    safe_join(stk2.root, &settings)
                        .ok()?
                        .is_file()
                        .then_some(settings)
                });
                // without its source settings a surface is known by the
                // diffuse texture its export names: `tex:<texture>`
                let surfaces = match settings {
                    Some(settings) => stk2.model_surfaces(&settings),
                    None => crate::stk2_content::discover_static_model_surface_textures(&archive)
                        .map(|surfaces| {
                            surfaces
                                .into_iter()
                                .map(|(surface, texture)| {
                                    let texture = texture.canonical.unwrap_or_default();
                                    (surface, format!("tex:{texture}"))
                                })
                                .collect()
                        }),
                };
                // an archive the reader refuses is reported, not fatal
                let mut surfaces = match surfaces {
                    Ok(surfaces) => surfaces,
                    Err(error) => {
                        missing.insert(format!("{library} ({error:#})"));
                        continue;
                    }
                };
                for (k, swap) in slice.material_swaps.iter().enumerate() {
                    if swap.lib != *library {
                        continue;
                    }
                    for material in surfaces.values_mut().filter(|m| **m == swap.from) {
                        material.clone_from(&swap.to);
                        swapped[k] += 1;
                    }
                }
                out.models.push(SliceModel {
                    library: library.clone(),
                    archive,
                    surfaces,
                    placement: Rigid::stk2(object.position, object.rotation, object.scale)?
                        .then(scene),
                });
            }
            "light" => {
                use crate::stk2_project::ProjectProperty as P;
                let number = |key: &str, default: f32| match object.properties.get(key) {
                    Some(P::Float(v)) => *v,
                    Some(P::Integer(v)) => *v as f32,
                    _ => default,
                };
                // 0 point, 2 box, 5 sphere: points; 1 spot, 6 plane spot
                // (an area spot 0.26g lacks: a spot from its centre); 4 the
                // sun (kept wherever it is: it lights the whole scene)
                let kind = match number("light_type", 0.0) as i32 {
                    0 | 2 | 5 => LightKind::Point,
                    1 | 6 => LightKind::Spot,
                    4 => LightKind::Sun,
                    _ => continue,
                };
                if kind != LightKind::Sun && !inside(object.position) {
                    continue;
                }
                let color = match object.properties.get("color") {
                    Some(P::Float3(c)) => *c,
                    _ => [1.0; 3],
                };
                let turned = Rigid::stk2(object.position, object.rotation, [1.0; 3])?.then(scene);
                out.lights.push(SliceLight {
                    source: object.id.clone(),
                    kind,
                    position: scene.apply(object.position),
                    rotation: euler_of(turned.m),
                    color,
                    intensity: number("intensity", 1.0),
                    range: number("range", 5.0),
                    attenuation_power: number("attenuation_power", 2.0),
                    cast_shadow: matches!(
                        object.properties.get("is_cast_shadow"),
                        Some(P::Boolean(true))
                    ),
                    influence: [
                        number("diffuse_influence_factor", 1.0),
                        number("specular_influence_factor", 1.0),
                    ],
                    cone: (kind == LightKind::Spot).then(|| {
                        [
                            number("spot_umbra_angle", 0.393),
                            number("spot_penumbra_angle", 0.785),
                            number("spot_falloff", 2.0),
                        ]
                    }),
                });
            }
            _ => {}
        }
    }
    if let Some((swap, _)) = slice.material_swaps.iter().zip(&swapped).find(|(_, n)| **n == 0) {
        bail!("slice {}: material swap {} {} -> {} meets no surface", slice.name, swap.lib, swap.from, swap.to);
    }
    out.missing = missing.into_iter().collect();
    Ok(out)
}

struct SliceMade {
    objects: usize,
    surfaces: usize,
    vertices: usize,
    collision: bool,
}

/// One model of a slice's objects: their surfaces clipped to the box and
/// merged by material (in pieces the 0.26g loader takes), and their low
/// collision clipped the same way.
fn write_slice(
    name: &str,
    slice: &SliceSpec,
    objects: &SliceObjects,
    material_names: &BTreeMap<String, Option<(String, String)>>,
    templates: &StaticModelTemplates,
    scene: Rigid,
    pack: &mut PackWriter,
) -> Result<SliceMade> {
    // the box is the STK2 world's: a point is moved back to test it
    let back = scene.inverse();
    let in_box = |p: [f32; 3]| {
        let w = back.apply(p);
        (0..3).all(|a| w[a] >= slice.box_min[a] && w[a] <= slice.box_max[a])
    };
    let mut merged: BTreeMap<String, Mesh> = BTreeMap::new();
    let mut collision_positions: Vec<[f32; 3]> = Vec::new();
    let mut collision_triangles: Vec<[u32; 3]> = Vec::new();
    let mut kept = 0;
    for object in &objects.models {
        let mut archive = ResourceDb::open(&object.archive)
            .with_context(|| format!("opening {}", object.archive.display()))?;
        let mut any = false;
        for (surface, material) in &object.surfaces {
            if material_names[material].is_none() {
                continue;
            }
            let label = format!("{}:{surface}", object.library);
            let (Ok(vertices), Ok(indices)) = (
                archive.read(&format!("render/{surface}/vertices")),
                archive.read(&format!("render/{surface}/indices")),
            ) else {
                continue;
            };
            validate_static_geometry(&vertices, &indices, &label)?;
            let mesh = Mesh::stk2(&vertices, &indices, object.placement, &label)?;
            let clipped = mesh.clipped(in_box);
            if clipped.triangles.is_empty() {
                continue;
            }
            any = true;
            merged.entry(material.clone()).or_default().append(&clipped);
        }
        if !any {
            continue;
        }
        kept += 1;
        if slice.collision
            && let (Ok(vertices), Ok(indices)) = (
                archive.read("collision/vertices"),
                archive.read("collision/indices"),
            )
        {
            validate_stk2_collision(&vertices, &indices, &object.library)?;
            let count = usize::try_from(read_u32(&vertices, 4)?)? / 12;
            let base = collision_positions.len() as u32;
            for i in 0..count {
                collision_positions
                    .push(object.placement.apply(read_float3(&vertices, 8 + i * 12)?));
            }
            let index_count = usize::try_from(read_u32(&indices, 4)?)? / 4;
            for t in 0..index_count / 3 {
                let mut corners = [0u32; 3];
                for (c, value) in corners.iter_mut().enumerate() {
                    *value = read_u32(&indices, 8 + (t * 3 + c) * 4)? + base;
                }
                if object.placement.mirrors() {
                    corners.swap(1, 2);
                }
                collision_triangles.push(corners);
            }
        }
    }
    let mut built = Vec::new();
    let mut vertices = 0;
    for (index, (material, mesh)) in merged.iter().enumerate() {
        let (material_name, texture_name) = material_names[material]
            .clone()
            .expect("hidden materials are not merged");
        for (piece_index, piece) in mesh.split(MAX_STATIC_SURFACE_VERTICES).iter().enumerate() {
            let surface = format!("m{index:03}_{piece_index}_s");
            vertices += piece.vertices.len();
            built.push(BuiltSurface {
                name: surface.clone(),
                material: material_name.clone(),
                mesh: piece.encode(&surface)?,
                bounds: piece.bounds(),
                vertices: u32::try_from(piece.vertices.len())?,
                indices: u32::try_from(piece.triangles.len() * 3)?,
                mesh_name: format!("|{}|{material}", slice.name),
                texture_name: texture_name.clone(),
            });
        }
    }
    let collision = if collision_triangles.is_empty() {
        None
    } else {
        let mut v = Vec::new();
        push_u32(&mut v, 0x19);
        push_u32(&mut v, u32::try_from(collision_positions.len() * 12)?);
        for p in &collision_positions {
            for value in p {
                v.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut i = Vec::new();
        push_u32(&mut i, 0x1a);
        push_u32(&mut i, u32::try_from(collision_triangles.len() * 12)?);
        for t in &collision_triangles {
            for value in t {
                push_u32(&mut i, *value);
            }
        }
        Some((v, i, collision_triangles.len()))
    };
    let has_collision = collision.is_some();
    if !built.is_empty() {
        write_static_model(
            name,
            &format!("STK2/{} (a corner, merged)", slice.project),
            &built,
            collision,
            templates,
            pack,
        )?;
    }
    Ok(SliceMade {
        objects: kept,
        surfaces: built.len(),
        vertices,
        collision: has_collision && !built.is_empty(),
    })
}

/// The 0.26 ambient light colour: an i32 with red in the low byte (A, B, G,
/// R from the top), as resource-porter's later-scene lobbies found in a game
/// (feat/lobby-scenes-046-069, c9940e1).
fn packed_colour(rgb: [f32; 3]) -> i32 {
    let c = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 255;
    (0xff00_0000 | (c(rgb[2]) << 16) | (c(rgb[1]) << 8) | c(rgb[0])) as i32
}

/// A converted material's emission on: its colour and multiplier, emitting
/// the emissive texture the STK2 one bound, or else its diffuse.
fn set_emission(
    root: &mut ConfigValue,
    colour: [f32; 4],
    multiplier: f32,
    diffuse: Option<&str>,
) -> Result<()> {
    let effect = "material.g_stage.effect";
    for field in [
        "use_emissive",
        "texture_emissive",
        "constant_emissive",
        "constant_emissive_multiplier",
    ] {
        ensure!(
            crate::content_pack::value::value_at(root, &format!("{effect}.{field}.value")).is_some(),
            "the template has no {field}"
        );
    }
    root.set_path(&format!("{effect}.use_emissive.value"), ConfigValue::Boolean(true))?;
    let bound = matches!(
        crate::content_pack::value::value_at(root, &format!("{effect}.texture_emissive.value")),
        Some(ConfigValue::String(t)) if !t.is_empty()
    );
    if !bound {
        let diffuse = diffuse.context("no texture to emit")?;
        root.set_path(
            &format!("{effect}.texture_emissive.value"),
            ConfigValue::String(diffuse.to_owned()),
        )?;
    }
    root.set_path(&format!("{effect}.constant_emissive.value"), ConfigValue::Float4(colour))?;
    root.set_path(
        &format!("{effect}.constant_emissive_multiplier.value"),
        ConfigValue::Float(multiplier),
    )?;
    Ok(())
}

/// Radiance of a probe cube's gradient in direction `d`.
fn gradient(cube: &ProbeCube, d: [f32; 3]) -> [f32; 3] {
    let (t, end) = if d[1] >= 0.0 { (d[1], cube.up) } else { (-d[1], cube.down) };
    std::array::from_fn(|c| cube.side[c] * (1.0 - t) + end[c] * t)
}

/// Size and mips of a made probe's radiance cubemap: 0.26g's smallest
/// (its own are 128..512 px, their mips down to 4 px).
const PROBE_CUBE_SIZE: usize = 128;
const PROBE_CUBE_MIPS: usize = 6;

/// A cubemap as 0.26g's compressor writes it: the header's linear size the
/// whole payload, which ends in one zero byte.
fn native_cube_dds(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.push(0);
    let payload = (bytes.len() - 128) as u32;
    bytes[20..24].copy_from_slice(&payload.to_le_bytes());
    bytes
}

/// A probe of the project given the cube's cubemaps: written beside the
/// template probe's options as `environment_probes/<prefix>/<object>`.
fn add_probe_cube(
    project: &mut BinaryConfig,
    cube: &ProbeCube,
    prefix: &str,
    target: &mut ResourceDb,
    pack: &mut PackWriter,
    notes: &mut Vec<String>,
) -> Result<()> {
    use crate::probe_bake::{Cube, cube_header, diffuse_of, direction, encode_026};
    for c in [cube.up, cube.side, cube.down] {
        ensure!(
            c.iter().all(|v| (0.0..=1.0).contains(v)),
            "probe cube {}: colours are 0..1 (the probe's diffuse_multiplier scales them)",
            cube.object
        );
    }
    let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("client_objects") else {
        bail!("the project has no client_objects");
    };
    let object = objects
        .iter_mut()
        .find(|o| matches!(o.named("name"), Some(ConfigValue::String(n)) if *n == cube.object))
        .with_context(|| format!("probe cube: no client object {:?}", cube.object))?;
    ensure!(
        matches!(object.named("lib_name"), Some(ConfigValue::String(l)) if l == "environment_probe"),
        "probe cube: {:?} is not an environment_probe",
        cube.object
    );
    let Some(ConfigValue::String(native)) = object.named("texture").cloned() else {
        bail!("probe cube: {:?} has no texture", cube.object);
    };
    let texture = format!("environment_probes/{prefix}/{}", cube.object);
    object.set_path("texture", ConfigValue::String(texture.clone()))?;
    if let Some(ConfigValue::Named(layers)) = object.named_mut("textures") {
        for layer in layers {
            layer.value = ConfigValue::String(texture.clone());
        }
    }
    let faces = (0..6)
        .map(|f| {
            (0..PROBE_CUBE_MIPS)
                .map(|m| {
                    let n = (PROBE_CUBE_SIZE >> m).max(1);
                    (0..n * n).map(|i| gradient(cube, direction(f, n, i % n, i / n))).collect()
                })
                .collect()
        })
        .collect();
    let radiance = Cube { size: PROBE_CUBE_SIZE, mips: PROBE_CUBE_MIPS, faces };
    let diffuse = diffuse_of(&radiance);
    let template = target.read(&format!("textures/{native}.dds"))?;
    let template_diffuse = target.read(&format!("textures/{native}_diffuse.dds"))?;
    pack.write(
        &format!("textures/{texture}.dds"),
        &native_cube_dds(encode_026(&radiance, &cube_header(&template, radiance.size, radiance.mips)?)?),
    )?;
    pack.write(
        &format!("textures/{texture}_diffuse.dds"),
        &native_cube_dds(encode_026(&diffuse, &cube_header(&template_diffuse, diffuse.size, diffuse.mips)?)?),
    )?;
    for suffix in ["", "_diffuse"] {
        let options = parse(target, &format!("textures/{native}{suffix}.options"))?;
        pack.config(&format!("textures/{texture}{suffix}.options"), &options)?;
    }
    let mean = crate::probe_bake::mean(&diffuse, 0);
    notes.push(format!(
        "probe {}: cubemaps made from a gradient (up {:?}, side {:?}, down {:?}); mean irradiance over pi ({:.3}, {:.3}, {:.3})",
        cube.object, cube.up, cube.side, cube.down, mean[0], mean[1], mean[2]
    ));
    Ok(())
}

/// A number in the type the object already has there: an integer where
/// 0.26g keeps one (and the value is whole), a float otherwise.
fn number_like(object: &ConfigValue, path: &str, value: f32) -> ConfigValue {
    match crate::content_pack::value::value_at(object, path) {
        Some(ConfigValue::Integer(_)) if value.fract() == 0.0 && value.abs() < 1e9 => {
            ConfigValue::Integer(value as i32)
        }
        _ => ConfigValue::Float(value),
    }
}

fn parse(db: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    crate::weapon::parse_config(db, path, "template")
}

fn float3_of(v: [f32; 3]) -> ConfigValue {
    ConfigValue::Float3(v.map(|x| (x * 1000.0).round() / 1000.0))
}

/// The camera boxes as collision objects of the project (the lobby frame).
fn add_camera_boxes(project: &mut BinaryConfig, boxes: &[CameraBox], notes: &mut Vec<String>) -> Result<()> {
    let Some(ConfigValue::Indexed(collisions)) = project.root.named_mut("collision_objects") else {
        bail!("the project has no collision_objects");
    };
    for b in boxes {
        ensure!(
            b.half_extents.iter().all(|h| h.is_finite() && *h > 0.0) && b.centre.iter().all(|c| c.is_finite())
                && b.yaw.is_finite(),
            "camera box {:?}: a centre, a turn and half extents above 0",
            b.what
        );
        let [x, y, z] = b.half_extents;
        collisions.push(ConfigValue::Named(vec![
            named("lib_name", ConfigValue::String(format!("{UNIT_BOX_COLLISION}#[{x:.2}][{y:.2}][{z:.2}]"))),
            named("position", float3_of(b.centre)),
            named("rotation", ConfigValue::Float3([0.0, b.yaw, 0.0])),
            named("scale", ConfigValue::Float3(b.half_extents)),
            named("cmask", ConfigValue::Integer(PROP_COLLISION_MASK)),
            named("cgroup", ConfigValue::Integer(PROP_COLLISION_GROUP)),
        ]));
        notes.push(format!(
            "camera box ({}): centre ({:.2}, {:.2}, {:.2}), half extents ({x:.2}, {y:.2}, {z:.2}), yaw {:.3}",
            b.what, b.centre[0], b.centre[1], b.centre[2], b.yaw
        ));
    }
    Ok(())
}

/// The target's own models placed in the lobby frame: a render visual each
/// and its collision (the camera's ray meets it), the model checked there.
fn add_native_props(
    project: &mut BinaryConfig,
    props: &[NativeProp],
    target: &mut ResourceDb,
    notes: &mut Vec<String>,
) -> Result<()> {
    for prop in props {
        let settings = format!("models/{}.model/settings", prop.lib);
        ensure!(target.is_file(&settings), "native prop {}: {settings} is not in the target", prop.lib);
        let collision = format!("models/{}.model/collision", prop.lib);
        let has_collision = ["vertices", "exported_primitives"]
            .iter()
            .any(|leaf| target.is_file(&format!("{collision}/{leaf}")));
        let place = |extra: Vec<NamedValue>, lib: ConfigValue| {
            let mut fields = vec![
                named("rotation", ConfigValue::Float3([0.0, prop.yaw, 0.0])),
                named("position", float3_of(prop.position)),
                named("lib_name", lib),
            ];
            fields.extend(extra);
            fields.push(named("scale", ConfigValue::Float3([1.0; 3])));
            ConfigValue::Named(fields)
        };
        let Some(ConfigValue::Indexed(visuals)) = project.root.named_mut("render_visuals") else {
            bail!("the project has no render_visuals");
        };
        visuals.push(place(
            vec![named("visual_name", ConfigValue::String(prop.lib.clone()))],
            ConfigValue::String(prop.lib.clone()),
        ));
        if has_collision {
            let Some(ConfigValue::Indexed(collisions)) = project.root.named_mut("collision_objects") else {
                bail!("the project has no collision_objects");
            };
            collisions.push(place(
                vec![
                    named("cmask", ConfigValue::Integer(PROP_COLLISION_MASK)),
                    named("cgroup", ConfigValue::Integer(PROP_COLLISION_GROUP)),
                ],
                ConfigValue::String(format!("resources/{collision}")),
            ));
        }
        notes.push(format!(
            "native prop {} at ({:.2}, {:.2}, {:.2}), yaw {:.3}{}",
            prop.lib,
            prop.position[0],
            prop.position[1],
            prop.position[2],
            prop.yaw,
            if has_collision { ", with its collision" } else { ", no collision of its own" }
        ));
    }
    Ok(())
}

fn named(name: &str, value: ConfigValue) -> NamedValue {
    NamedValue {
        name: name.to_owned(),
        value,
    }
}

/// An f32 as an IEEE half, round to nearest even (the uv format of 0.26g
/// static vertices).
fn half(value: f32) -> Result<u16> {
    half_float(value)
}

fn half_float(value: f32) -> Result<u16> {
    ensure!(
        value.is_finite() && value.abs() <= 65504.0,
        "uv {value} outside the half range"
    );
    let sign = ((value.to_bits() >> 16) & 0x8000) as u16;
    let value = value.abs();
    if value < 2f32.powi(-14) {
        return Ok(sign | (value * 2f32.powi(24)).round_ties_even() as u16);
    }
    let mut exponent = ((value.to_bits() >> 23) & 255) as i32 - 127;
    let mut mantissa = ((value / 2f32.powi(exponent) - 1.0) * 1024.0).round_ties_even() as u16;
    if mantissa == 1024 {
        exponent += 1;
        mantissa = 0;
    }
    Ok(sign | (((exponent + 15) as u16) << 10) | mantissa)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated u32")?
            .try_into()?,
    ))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

fn read_float3(bytes: &[u8], offset: usize) -> Result<[f32; 3]> {
    Ok([
        read_f32(bytes, offset)?,
        read_f32(bytes, offset + 4)?,
        read_f32(bytes, offset + 8)?,
    ])
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn the_scene_puts_the_character_at_the_origin_and_the_view_along_z() {
        let scene = Rigid::scene(&SceneSpec {
            character: [2.0, 1.0, 27.0],
            view_yaw: -std::f32::consts::FRAC_PI_2,
        });
        assert!(near(scene.apply([2.0, 1.0, 27.0]), [0.0; 3]));
        // one metre further along STK2's +x is one metre along +z
        assert!(near(scene.apply([3.0, 1.0, 27.0]), [0.0, 0.0, 1.0]));
        // and STK2's +z goes to -x
        assert!(near(scene.apply([2.0, 1.0, 28.0]), [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_placement_then_the_scene_is_one_rigid_move() {
        let object = Rigid::yaw(1.5811, [5.27, 1.1, 26.2]);
        let scene = Rigid::scene(&SceneSpec {
            character: [2.0, 1.1, 27.0],
            view_yaw: -0.7,
        });
        let both = object.then(scene);
        for p in [[0.0, 0.0, 0.0], [0.79, 2.5, -0.89], [-0.3, 1.0, 0.2]] {
            assert!(near(both.apply(p), scene.apply(object.apply(p))));
        }
    }

    #[test]
    fn the_sentry_yaw_reproduces_its_stk2_bounding_box() {
        // sentry_a: rotation (-pi, -1.5605, -pi) in the project, local box
        // x -0.76..0.79, z -0.89..0.03; the project's world box x
        // 4.376..5.315, z 25.432..26.992.
        let r = Rigid::yaw(
            std::f32::consts::PI - 1.560_489_3,
            [5.272_343, 0.0, 26.225_277],
        );
        let corners = [[-0.76, 0.0, -0.89], [0.79, 0.0, 0.03]];
        let a = r.apply(corners[0]);
        let b = r.apply(corners[1]);
        assert!((a[0].min(b[0]) - 4.38).abs() < 0.02);
        assert!((a[2].max(b[2]) - 26.99).abs() < 0.02);
    }

    #[test]
    fn the_engine_rotation_is_the_sentry_yaw_and_turns_back_into_its_angles() {
        // the project's (-pi, r, -pi) is the yaw pi + r
        let r = -1.560_489_3_f32;
        let engine = Rigid::stk2(
            [0.0; 3],
            [-std::f32::consts::PI, r, -std::f32::consts::PI],
            [1.0; 3],
        )
        .unwrap();
        let yaw = Rigid::yaw(std::f32::consts::PI + r, [0.0; 3]);
        for p in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -0.2, 0.9]] {
            assert!(near(engine.apply(p), yaw.apply(p)));
        }
        for angles in [[0.3, -0.7, 1.1], [-1.2, 0.4, -2.5], [0.0, 1.0, 0.0]] {
            let m = Rigid::stk2([0.0; 3], angles, [1.0; 3]).unwrap().m;
            let back = Rigid::stk2([0.0; 3], euler_of(m), [1.0; 3]).unwrap().m;
            for (a, b) in m.iter().zip(back) {
                assert!(near(*a, b));
            }
        }
    }

    #[test]
    fn a_scaled_mirror_turns_normals_by_the_inverse_and_flips_triangles() {
        let r = Rigid::stk2([1.0, 2.0, 3.0], [0.0; 3], [2.0, 1.0, -1.0]).unwrap();
        assert!(r.mirrors());
        assert!(near(r.apply([1.0, 1.0, 1.0]), [3.0, 3.0, 2.0]));
        // a normal across x keeps its direction (scaled the other way)
        assert!(near(r.normal([1.0, 0.0, 0.0]), [0.5, 0.0, 0.0]));
        assert!(near(r.normal([0.0, 0.0, 1.0]), [0.0, 0.0, -1.0]));
    }

    #[test]
    fn a_mesh_is_clipped_to_whole_triangles_and_split_under_the_limit() {
        let vertex = |x: f32| {
            let mut v = [0u8; VERTEX_SIZE];
            v[..4].copy_from_slice(&x.to_le_bytes());
            v
        };
        let mesh = Mesh {
            vertices: (0..6).map(|i| vertex(i as f32)).collect(),
            positions: (0..6).map(|i| [i as f32, 0.0, 0.0]).collect(),
            triangles: vec![[0, 1, 2], [3, 4, 5]],
        };
        let kept = mesh.clipped(|p| p[0] < 1.0);
        assert_eq!(kept.triangles, vec![[0, 1, 2]]);
        assert_eq!(kept.vertices.len(), 3);
        let pieces = mesh.split(4);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.vertices.len() == 3));
    }

    #[test]
    fn half_floats_round_and_refuse_the_out_of_range() {
        assert_eq!(half(0.0).unwrap(), 0);
        assert_eq!(half(1.0).unwrap(), 0x3c00);
        assert_eq!(half(-2.0).unwrap(), 0xc000);
        assert_eq!(half(0.5).unwrap(), 0x3800);
        assert!(half(70000.0).is_err());
        assert!(half(f32::NAN).is_err());
    }

    #[test]
    fn a_surface_is_converted_to_the_026_layout_and_moved() {
        // one triangle: positions, normals up, tangents +x, binormals +z
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 3);
        for (p, uv) in [
            ([0.0f32, 0.0, 0.0], [0.0f32, 0.0]),
            ([1.0, 0.0, 0.0], [1.0, 0.0]),
            ([0.0, 0.0, 1.0], [0.0, 1.0]),
        ] {
            for v in p
                .iter()
                .chain(&[0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0])
                .chain(&uv)
            {
                vertices.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        for i in [0u16, 2, 1] {
            indices.extend_from_slice(&i.to_le_bytes());
        }
        let placement = Rigid::yaw(std::f32::consts::FRAC_PI_2, [10.0, 0.0, 0.0]);
        let (mesh, bounds) = convert_surface(&vertices, &indices, placement, "test").unwrap();
        assert_eq!(read_u32(&mesh, 0).unwrap(), 3);
        assert_eq!(read_u32(&mesh, 4).unwrap() as usize, 4 + 3 * VERTEX_SIZE);
        assert_eq!(read_u32(&mesh, 8).unwrap(), 3);
        // the second vertex (1, 0, 0) turned a quarter: (0, 0, -1), then +10 x
        let second = read_float3(&mesh, 12 + VERTEX_SIZE).unwrap();
        assert!(near(second, [10.0, 0.0, -1.0]));
        // its uv as halves
        let uv = &mesh[12 + VERTEX_SIZE + 24..12 + VERTEX_SIZE + 28];
        assert_eq!(uv, &[0x00, 0x3c, 0x00, 0x00]);
        // the normal stays up (packed y 255 at byte 1... z, y, x, alpha order)
        assert_eq!(
            read_u32(&mesh, 12 + 12).unwrap(),
            packed_basis([0.0, 1.0, 0.0], 127)
        );
        // the index block follows unchanged
        let at = 12 + 3 * VERTEX_SIZE;
        assert_eq!(read_u32(&mesh, at).unwrap(), 4);
        assert_eq!(&mesh[at + 8..], &indices[..]);
        assert!(near(bounds.min, [10.0, 0.0, -1.0]));
    }

    #[test]
    fn lua_settings_give_the_surfaces_and_their_materials() {
        let text = "lod_hierrarchy = {\n\tLOD0 = {\n\t\tsurfaces = {\n\t\t\ta_s = \"\",\n\t\t\tb_s = \"\",\n\t\t}, -- surfaces\n\n\t}, -- LOD0\n\n} -- lod_hierrarchy\n\nmaterial_settings = {\n\ta_s = {\n\t\tmaterial_name = \"tiled/wood/x\",\n\t}, -- a_s\n\n\tb_s = {\n\t\tmaterial_name = \"props/y\",\n\t}, -- b_s\n\n} -- material_settings\n";
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings"), text).unwrap();
        let tree = Stk2Tree {
            root: dir.path(),
            dry: false,
        };
        let surfaces = tree.model_surfaces("settings").unwrap();
        assert_eq!(surfaces["a_s"], "tiled/wood/x");
        assert_eq!(surfaces["b_s"], "props/y");
    }

    #[test]
    fn material_roles_tell_set_empty_and_inherited_apart() {
        let text = "\t\t\ttexture_diffuse = create_property_table ( {\n\t\t\t\tvalue = \"tiled/wood/b\",\n\t\t\t}, this ), -- texture_diffuse\n\n\t\t\ttexture_bump = create_property_table ( {\n\t\t\t\tvalue = \"\",\n\t\t\t}, this ), -- texture_bump\n\n\t\t\ttexture_normal = create_reference( \"modules['materials/wood.material'].material.g_stage.effect.texture_normal\", this ),\n";
        assert_eq!(
            material_role(text, "texture_diffuse"),
            Some(Some("tiled/wood/b".to_owned()))
        );
        assert_eq!(material_role(text, "texture_bump"), Some(None));
        assert_eq!(material_role(text, "texture_normal"), None);
        let base = "\t\t\ttexture_normal = {\n\t\t\t\tis_model_parameter = true,\n\t\t\t\tvalue = \"props/fire/campfire_nmap\",\n\t\t\t}, -- texture_normal\n";
        assert_eq!(
            material_role(base, "texture_normal"),
            Some(Some("props/fire/campfire_nmap".to_owned()))
        );
    }

    #[test]
    fn an_ambient_colour_packs_red_in_the_low_byte() {
        assert_eq!(packed_colour([1.0, 0.0, 0.0]) as u32, 0xff00_00ff);
        assert_eq!(packed_colour([0.0, 0.0, 1.0]) as u32, 0xffff_0000);
        // the classic lobby's bluish fill
        let c = |v: u32| v as f32 / 255.0;
        assert_eq!(packed_colour([c(0x6b), c(0x85), c(0xb3)]) as u32, 0xffb3_856b);
        assert_eq!(packed_colour([2.0, -1.0, 0.5]) as u32, 0xff80_00ff);
    }

    #[test]
    fn a_probe_gradient_runs_from_the_floor_to_the_sky() {
        let cube = ProbeCube { object: String::new(), up: [0.1; 3], side: [0.2; 3], down: [0.4; 3] };
        assert!(near(gradient(&cube, [0.0, 1.0, 0.0]), [0.1; 3]));
        assert!(near(gradient(&cube, [1.0, 0.0, 0.0]), [0.2; 3]));
        assert!(near(gradient(&cube, [0.0, -1.0, 0.0]), [0.4; 3]));
        assert!(near(gradient(&cube, [0.0, -0.5, 0.866]), [0.3; 3]));
    }

    #[test]
    fn a_cubemap_ends_as_the_compressor_writes_it() {
        let mut bytes = vec![0u8; 128 + 16];
        bytes[20..24].copy_from_slice(&16u32.to_le_bytes());
        let out = native_cube_dds(bytes);
        assert_eq!(out.len(), 128 + 17);
        assert_eq!(u32::from_le_bytes(out[20..24].try_into().unwrap()), 17);
        assert_eq!(out[out.len() - 1], 0);
    }

    #[test]
    fn emission_needs_the_template_fields_and_emits_the_diffuse() {
        let leaf = |v: ConfigValue| ConfigValue::Named(vec![named("value", v)]);
        let effect = |texture: &str| {
            ConfigValue::Named(vec![
                named("use_emissive", leaf(ConfigValue::Boolean(false))),
                named("texture_emissive", leaf(ConfigValue::String(texture.to_owned()))),
                named("constant_emissive", leaf(ConfigValue::Float4([0.0, 0.0, 0.0, 1.0]))),
                named("constant_emissive_multiplier", leaf(ConfigValue::Integer(1))),
            ])
        };
        let material = |e: ConfigValue| {
            ConfigValue::Named(vec![named(
                "material",
                ConfigValue::Named(vec![named("g_stage", ConfigValue::Named(vec![named("effect", e)]))]),
            )])
        };
        let at = |root: &ConfigValue, f: &str| {
            crate::content_pack::value::value_at(root, &format!("material.g_stage.effect.{f}.value")).cloned()
        };
        let mut root = material(effect(""));
        set_emission(&mut root, [1.0, 0.5, 0.1, 1.0], 6.0, Some("t/diffuse")).unwrap();
        assert_eq!(at(&root, "use_emissive"), Some(ConfigValue::Boolean(true)));
        assert_eq!(at(&root, "texture_emissive"), Some(ConfigValue::String("t/diffuse".into())));
        assert_eq!(at(&root, "constant_emissive_multiplier"), Some(ConfigValue::Float(6.0)));
        // an emissive mask the STK2 material bound is kept
        let mut root = material(effect("t/mask"));
        set_emission(&mut root, [1.0; 4], 1.0, Some("t/diffuse")).unwrap();
        assert_eq!(at(&root, "texture_emissive"), Some(ConfigValue::String("t/mask".into())));
        assert!(set_emission(&mut material(ConfigValue::Named(Vec::new())), [1.0; 4], 1.0, None).is_err());
    }

    #[test]
    fn the_ground_is_a_grid_facing_up_at_its_height() {
        let (mesh, bounds, vertices, indices) = ground_plane(-1.1, 8.0, 4.0).unwrap();
        assert_eq!(vertices, 25);
        assert_eq!(indices, 4 * 4 * 6);
        assert!(near(bounds.min, [-8.0, -1.1, -8.0]));
        assert!(near(bounds.max, [8.0, -1.1, 8.0]));
        assert_eq!(read_u32(&mesh, 4).unwrap() as usize, 4 + 25 * VERTEX_SIZE);
        let at = 12 + 25 * VERTEX_SIZE + 12;
        let corner = |k: usize| {
            let i = u16::from_le_bytes([mesh[at + 2 * k], mesh[at + 2 * k + 1]]) as usize;
            read_float3(&mesh, 12 + i * VERTEX_SIZE).unwrap()
        };
        let (a, b, c) = (corner(0), corner(1), corner(2));
        let u = [b[0] - a[0], b[2] - a[2]];
        let v = [c[0] - a[0], c[2] - a[2]];
        // the same turn as the STK2 hut floor's triangles (crete_floor_1sg)
        assert!(u[0] * v[1] - u[1] * v[0] < 0.0);
    }
}
