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
    /// A native `.options` the made textures' options are cloned from.
    pub texture_options_template: String,
    pub max_texture_size: u32,
    pub scene: SceneSpec,
    pub models: Vec<ModelSpec>,
    pub ground: GroundSpec,
    /// Keyed by the STK2 material instance name the models' settings use.
    pub materials: BTreeMap<String, MaterialSpec>,
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
    /// `rotation = (-pi, r, -pi)` is the yaw `pi - r`).
    #[serde(default)]
    pub yaw: f32,
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
    },
    /// A native material, as it is.
    Native { material: String },
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

/// A yaw about +y then a translation: `p' = rot(yaw) p + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rigid {
    yaw: f32,
    translation: [f32; 3],
}

impl Rigid {
    fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        let (s, c) = self.yaw.sin_cos();
        [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = self.rotate(p);
        [
            r[0] + self.translation[0],
            r[1] + self.translation[1],
            r[2] + self.translation[2],
        ]
    }

    /// `outer` after `self`.
    fn then(self, outer: Rigid) -> Rigid {
        Rigid {
            yaw: self.yaw + outer.yaw,
            translation: outer.apply(self.translation),
        }
    }

    /// The scene's frame: the character at the origin, the view along +z.
    fn scene(spec: &SceneSpec) -> Rigid {
        let turn = Rigid {
            yaw: spec.view_yaw,
            translation: [0.0; 3],
        };
        let back = turn.rotate(spec.character);
        Rigid {
            yaw: spec.view_yaw,
            translation: [-back[0], -back[1], -back[2]],
        }
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
    };

    // The materials every model needs, made once.
    let mut used_materials = BTreeSet::new();
    let mut models = Vec::new();
    for model in &manifest.models {
        let surfaces = stk2.model_surfaces(&model.settings)?;
        for material in surfaces.values() {
            used_materials.insert(material.clone());
        }
        models.push((model, surfaces));
    }
    used_materials.insert(manifest.ground.material.clone());
    let mut textures = TextureMaker {
        stk2: &stk2,
        prefix: &manifest.texture_prefix,
        dir: &manifest.textures_dir,
        max_size: manifest.max_texture_size,
        work: options.work,
        options_template: parse(&mut target, &manifest.texture_options_template)?,
        made: BTreeMap::new(),
    };
    let mut material_names = BTreeMap::new();
    let mut diffuse_of = BTreeMap::new();
    for key in &used_materials {
        let spec = manifest
            .materials
            .get(key)
            .with_context(|| format!("no material spec for the STK2 material {key}"))?;
        let (name, diffuse) = make_material(
            key,
            spec,
            &manifest.material_prefix,
            &stk2,
            &mut target,
            &mut textures,
            &mut pack,
        )?;
        material_names.insert(key.clone(), name);
        diffuse_of.insert(key.clone(), diffuse.unwrap_or_default());
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
        let placement = Rigid {
            yaw: model.yaw,
            translation: model.position,
        }
        .then(scene);
        let mut archive = ResourceDb::open(safe_join(options.stk2_root, &model.archive)?)
            .with_context(|| format!("opening {}", model.archive))?;
        let mut built = Vec::new();
        for (surface, material) in surfaces {
            let vertices = archive.read(&format!("render/{surface}/vertices"))?;
            let indices = archive.read(&format!("render/{surface}/indices"))?;
            validate_static_geometry(&vertices, &indices, surface)?;
            let (mesh, bounds) = convert_surface(&vertices, &indices, placement, surface)?;
            built.push(BuiltSurface {
                name: surface.clone(),
                material: material_names[material].clone(),
                mesh,
                bounds,
                vertices: read_u32(&vertices, 0)?,
                indices: read_u32(&indices, 0)?,
                mesh_name: format!("|{}|{surface}", model.name),
                texture_name: diffuse_of[material].clone(),
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

    // The ground: one flat plane, the character in its middle.
    let ground = &manifest.ground;
    let ground_name = format!("{}/{}", manifest.model_prefix, ground.name);
    let height = scene.apply([0.0, ground.height, 0.0])[1];
    let (mesh, bounds, vertices, indices) = ground_plane(height, ground.half_size, ground.tile)?;
    write_static_model(
        &ground_name,
        "generated: flat ground in place of the STK2 lobby terrain (four 64 m cells, height 0)",
        &[BuiltSurface {
            name: "ground_s".to_owned(),
            material: material_names[&ground.material].clone(),
            mesh,
            bounds,
            vertices,
            indices,
            mesh_name: format!("|{}|ground", ground.name),
            texture_name: diffuse_of[&ground.material].clone(),
        }],
        None,
        &templates,
        &mut pack,
    )?;
    visuals.push(ground_name);

    // The project.
    let mut project = parse(&mut target, &manifest.template_project)?;
    replace_project(
        &mut project,
        &visuals,
        &collisions,
        &manifest.objects,
        scene,
    )?;
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
        notes: spot_notes(&manifest, &spots),
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
        let lod0 = lua_block(&text, "LOD0")
            .and_then(|lod| lua_block(lod, "surfaces"))
            .with_context(|| format!("{settings}: no LOD0 surfaces"))?;
        let listed: BTreeSet<&str> = lua_children_flat(lod0).into_iter().collect();
        let materials: BTreeSet<&str> = surfaces.keys().map(String::as_str).collect();
        ensure!(
            listed == materials,
            "{settings}: LOD0 surfaces {listed:?} are not the material_settings {materials:?}"
        );
        Ok(surfaces)
    }

    /// The source TGA and `.options` of an STK2 texture.
    fn texture(&self, name: &str, dir: &str) -> Result<(PathBuf, String)> {
        for extension in ["tga", "TGA"] {
            let path = safe_join(self.root, &format!("{dir}/{name}.{extension}"))?;
            if path.is_file() {
                let options = self.text(&format!("{dir}/{name}.options"))?;
                return Ok((path, options));
            }
        }
        bail!("STK2 texture {name}: no {dir}/{name}.tga")
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
    max_size: u32,
    work: &'a Path,
    options_template: BinaryConfig,
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
    /// The made texture's name (`<prefix>/<stk2 name>`), made once.
    fn make(&mut self, name: &str, kind: TextureKind, pack: &mut PackWriter) -> Result<String> {
        let key = (name.to_owned(), kind_key(kind));
        if let Some(done) = self.made.get(&key) {
            return Ok(done.clone());
        }
        let target = format!("{}/{name}", self.prefix);
        ensure!(
            self.made.values().all(|made| *made != target),
            "{name} is used both as colour and as data"
        );
        let (source, source_options) = self.stk2.texture(name, self.dir)?;
        let stem = self.work.join(name.replace('/', "__"));
        let scaled = stem.with_extension("scaled.tga");
        magick(&[
            source.as_os_str(),
            "-resize".as_ref(),
            format!("{0}x{0}>", self.max_size).as_ref(),
            "-type".as_ref(),
            "TrueColorAlpha".as_ref(),
            "-compress".as_ref(),
            "none".as_ref(),
            scaled.as_os_str(),
        ])?;
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
        let bytes = crate::duty_026::texture_dds(&fs::read(&dds)?, &target, srgb)?;
        pack.write(&format!("textures/{target}.dds"), &bytes)?;
        let mut options = self.options_template.clone();
        options
            .root
            .set_path("options.srgb", ConfigValue::Integer(i32::from(srgb)))?;
        options
            .root
            .set_path("options.format", ConfigValue::String(format.to_owned()))?;
        pack.config(&format!("textures/{target}.options"), &options)?;
        fs::remove_file(&scaled)?;
        fs::remove_file(&dds)?;
        self.made.insert(key, target.clone());
        Ok(target)
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
/// diffuse slot binds.
fn make_material(
    key: &str,
    spec: &MaterialSpec,
    prefix: &str,
    stk2: &Stk2Tree<'_>,
    target: &mut ResourceDb,
    textures: &mut TextureMaker<'_>,
    pack: &mut PackWriter,
) -> Result<(String, Option<String>)> {
    let (template, source, explicit) = match spec {
        MaterialSpec::Native { material } => {
            ensure!(
                target.contains(&format!("material_instances/{material}.material")),
                "{key}: native material {material} is not in the target"
            );
            return Ok((material.clone(), None));
        }
        MaterialSpec::Converted {
            template,
            source,
            textures,
        } => (template, source, textures),
    };
    let roles = material_textures(key, source.as_deref(), explicit, stk2)?;
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
        let made = textures.make(texture, *kind, pack)?;
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
    let name = format!("{prefix}/{key}");
    pack.config(&format!("material_instances/{name}.material"), &material)?;
    Ok((name, diffuse))
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
    ensure!(
        roles.contains_key("texture_diffuse"),
        "{key}: {source} has no diffuse texture"
    );
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

/// An STK2 surface (u32 n, n 56-byte vertices; u32 m, m u16 indices) as a
/// 0.26g `converted_model` with its vertices moved by `placement`.
fn convert_surface(
    vertices: &[u8],
    indices: &[u8],
    placement: Rigid,
    label: &str,
) -> Result<(Vec<u8>, Bounds)> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    ensure!(
        count <= MAX_STATIC_SURFACE_VERTICES,
        "{label}: {count} vertices, more than 0.26g loads ({MAX_STATIC_SURFACE_VERTICES})"
    );
    let bases = repaired_static_bases(vertices, indices, label)?;
    let mut out = Vec::with_capacity(12 + count * VERTEX_SIZE + indices.len() + 8);
    push_u32(&mut out, 3);
    push_u32(&mut out, u32::try_from(4 + count * VERTEX_SIZE)?);
    push_u32(&mut out, u32::try_from(count)?);
    let mut bounds = Bounds::empty();
    for (index, basis) in bases.iter().enumerate() {
        let source = &vertices[4 + index * STK2_VERTEX_SIZE..][..STK2_VERTEX_SIZE];
        let position = placement.apply(read_float3(source, 0)?);
        bounds.add(position);
        for value in position {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for (vector, alpha) in basis.iter().zip([127, 255, 255]) {
            push_u32(&mut out, packed_basis(placement.rotate(*vector), alpha));
        }
        for offset in [48, 52] {
            out.extend_from_slice(&half(read_f32(source, offset)?)?.to_le_bytes());
        }
    }
    push_u32(&mut out, 4);
    push_u32(&mut out, u32::try_from(indices.len())?);
    out.extend_from_slice(indices);
    Ok((out, bounds))
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
        for (path, value) in &spec.set {
            let hint = crate::content_pack::value::value_at(&object, path).cloned();
            let value = crate::content_pack::value::to_config(value, hint.as_ref())
                .with_context(|| format!("{name}: {path}"))?;
            object.set_path(path, value)?;
        }
        made.push(object);
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

fn parse(db: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    crate::weapon::parse_config(db, path, "template")
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
        let object = Rigid {
            yaw: 1.5811,
            translation: [5.27, 1.1, 26.2],
        };
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
        let r = Rigid {
            yaw: std::f32::consts::PI - 1.560_489_3,
            translation: [5.272_343, 0.0, 26.225_277],
        };
        let corners = [[-0.76, 0.0, -0.89], [0.79, 0.0, 0.03]];
        let a = r.apply(corners[0]);
        let b = r.apply(corners[1]);
        assert!((a[0].min(b[0]) - 4.38).abs() < 0.02);
        assert!((a[2].max(b[2]) - 26.99).abs() < 0.02);
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
        let placement = Rigid {
            yaw: std::f32::consts::FRAC_PI_2,
            translation: [10.0, 0.0, 0.0],
        };
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
        let tree = Stk2Tree { root: dir.path() };
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
