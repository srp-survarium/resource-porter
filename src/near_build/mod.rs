// SPDX-License-Identifier: GPL-3.0-or-later
//! A map of a build near 0.26g (0.29 to 0.45: the same config, model and
//! surface formats) as a content pack for 0.26g, in one pass over the
//! donor's own projects:
//!
//! 1. the project in the form 0.26g reads (`schema`: the editor form 0.32
//!    introduced), without the kinds the manifest leaves out;
//! 2. its models, materials and textures against the files players have
//!    (`files`): theirs where they are the same, imported under the map's
//!    namespace where they differ, added where players have none;
//! 3. each object with the fields 0.26g's own projects give its kind
//!    (`fields`);
//! 4. whatever else the project needs that players lack (particles, sounds,
//!    animations, sky textures), found by the dependency scan and added as the
//!    donor has it; a file of those players have in another form stays theirs.
//!
//! Every weather and mode is one of the donor's own projects, so a map's
//! projects are one consistent build. What could not be carried over is in
//! the pack's `report.json`, by rule, never by place.

pub mod fields;
pub mod files;
pub mod image;
pub mod programs;
pub mod schema;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use crate::{
    ResourceDb,
    binary_config::{BinaryConfig, ConfigValue, NamedValue},
    content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex},
    dependency,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub pack: Pack,
    /// The donor build, for the pack's notes (`0.44a5`).
    pub donor_build: String,
    /// The stock 0.26g resources.db the port is made for.
    pub stock_sha256: String,
    /// The folder imported files go under (`ported/v044a`).
    pub namespace: String,
    /// The donor's level folder (`level_09`) and the port's (`v044a_level_09`).
    pub donor_level: String,
    pub level: String,
    /// For a map of its own (`Project::map`): the level's number in the game
    /// data (`match_making_params.map_id`) and its name key
    /// (`st_level_32_name`).
    #[serde(default)]
    pub map_id: Option<i32>,
    #[serde(default)]
    pub map_name_key: Option<String>,
    /// Kinds left out by decision, with why. Each takes its group along
    /// (`schema::without_kinds`).
    #[serde(default)]
    pub leave_out: BTreeMap<String, String>,
    /// The two builds' shader key orders, for supplying programs
    /// (`programs`): files beside the manifest.
    #[serde(default)]
    pub shader_keys: Option<ShaderKeys>,
    pub projects: Vec<Project>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderKeys {
    /// 0.26g's key order.
    pub target: PathBuf,
    /// The donor's.
    pub donor: PathBuf,
    /// The families `--donor-programs` supplies: the g-buffer's pixel and
    /// vertex shaders, what a material and its mesh select.
    #[serde(default)]
    pub families: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    pub version: String,
    pub description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The donor's project folder (`evn_3`) and the port's (`evening_3`).
    pub donor: String,
    pub project: String,
    /// The environment's layer, as 0.26g names its weathers (`day`, `rain`,
    /// `evening`).
    pub layer: String,
    /// A new map: its maps_dict row and mode config. Without it the project
    /// replaces one players have, whose row and config stay as they are.
    #[serde(default)]
    pub map: Option<NewMap>,
    /// Server lists left out of this project, with the client objects and
    /// statics their objects own (a mode derived from another: the battery
    /// mode without the charging stations is the stations' project without
    /// `victory_item_spawn_points`).
    #[serde(default)]
    pub without_lists: Vec<String>,
    /// Environment settings written after the build (a shader profile's
    /// marker); each must be a setting the project has.
    #[serde(default)]
    pub environment_overrides: BTreeMap<String, f32>,
    /// A probe the donor box-projects (`use_parallax_correction`, a field
    /// 0.26g has not) is marked by `smart_attenuation`: the channel pack
    /// london-probes reads on
    /// a scene that carries London's marker (`environment_overrides`),
    /// where its smart box programs project the reflection and draw the
    /// plain falloff. Only for a project with that marker. A probe that
    /// uses smart attenuation itself draws the plain falloff there and is
    /// counted.
    #[serde(default)]
    pub parallax_probes_by_smart_attenuation: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewMap {
    /// The maps_dict id and the mode config's name (without `.options`).
    pub id: u16,
    pub options: String,
    /// A mode config players have, taken with this map's identity.
    pub options_from: String,
    pub weather_id: i32,
    pub loading_screen_id: i32,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ProjectReport {
    pub left_out_by_decision: BTreeMap<String, usize>,
    pub schema: schema::SchemaReport,
    pub fields: fields::FieldReport,
    pub client_objects: usize,
    pub render_visuals: usize,
    pub collision_objects: usize,
    /// Probes marked as box-projected (`parallax_probes_by_smart_attenuation`).
    pub parallax_probes: usize,
    /// Of the project's probes, those that used smart attenuation themselves
    /// and draw the plain falloff on the marked scene.
    pub smart_probes_lost: usize,
    /// Donor files added as they are, by kind of path.
    pub added: BTreeMap<String, usize>,
    /// Files players have in another form than the donor's and keep, by kind.
    pub kept_as_players_have: BTreeMap<String, usize>,
    pub unresolved: Vec<String>,
}

fn text<'a>(object: &'a ConfigValue, key: &str) -> Option<&'a str> {
    match object.named(key) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}

fn cfg(db: &mut ResourceDb, path: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(&db.read(path).with_context(|| format!("reading {path}"))?)
        .with_context(|| format!("parsing {path}"))?
        .root)
}

fn kind_of_path(path: &str) -> &'static str {
    match path.split('/').next().unwrap_or_default() {
        "textures" => "texture",
        "models" => "model",
        "material_instances" => "material",
        "particles" => "particle",
        "sounds" => "sound",
        "animations" => "animation",
        "projects" => "project",
        _ => "other",
    }
}

/// Every string of an object that names a material (a field called
/// `*material*`), by the name to draw it by.
fn materials_of(value: &mut ConfigValue, importer: &mut files::Importer<'_>) -> Result<()> {
    match value {
        ConfigValue::Named(fields) => {
            for field in fields {
                match &mut field.value {
                    ConfigValue::String(name) if field.name.contains("material") && !name.is_empty() => {
                        let path = format!("material_instances/{}.material", name.replace('\\', "/"));
                        if importer.donor.is_file(&path) {
                            *name = importer.material(&name.replace('\\', "/"))?;
                        }
                    }
                    other => materials_of(other, importer)?,
                }
            }
        }
        ConfigValue::Indexed(list) => {
            for v in list {
                materials_of(v, importer)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The old-form project with every model, material and probe cubemap by the
/// name the port places it by.
fn place_files(
    project: &mut ConfigValue,
    m: &Manifest,
    p: &Project,
    importer: &mut files::Importer<'_>,
    parallax_probes: &mut usize,
    smart_probes_lost: &mut usize,
) -> Result<()> {
    let ConfigValue::Named(top) = project else { bail!("a project is not a table") };
    for field in top {
        match (field.name.as_str(), &mut field.value) {
            ("render_visuals" | "temporary_render_visuals", ConfigValue::Indexed(list)) => {
                for entry in list {
                    if let Some(ConfigValue::String(model)) = entry.named_mut("lib_name") {
                        *model = importer.model(&model.replace('\\', "/"), "model")?;
                    }
                    if let Some(ConfigValue::String(visual)) = entry.named_mut("visual_name") {
                        *visual = importer.visual(&visual.replace('\\', "/"))?;
                    }
                }
            }
            ("collision_objects" | "temporary_collision_objects", ConfigValue::Indexed(list)) => {
                for entry in list {
                    if let Some(ConfigValue::String(collision)) = entry.named_mut("lib_name") {
                        *collision = importer.collision(&collision.replace('\\', "/"))?;
                    }
                }
            }
            ("client_objects", ConfigValue::Indexed(objects)) => {
                for object in objects {
                    match text(object, "game_object_type").unwrap_or_default() {
                        "skeleton_visual" => {
                            if let Some(ConfigValue::String(model)) = object.named_mut("lib_name") {
                                *model = importer.model(&model.replace('\\', "/"), "skinned_model")?;
                            }
                        }
                        "vegetation" => {
                            if let Some(ConfigValue::Indexed(layers)) = object.named_mut("layers") {
                                for layer in layers {
                                    if let Some(ConfigValue::Indexed(models)) = layer.named_mut("model_def") {
                                        for model in models {
                                            if let Some(ConfigValue::String(name)) = model.named_mut("name") {
                                                *name = importer.model(&name.replace('\\', "/"), "model")?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "environment_probe" => {
                            if let Some(ConfigValue::String(texture)) = object.named_mut("texture") {
                                *texture = probe_cubemap(texture, m, importer)?;
                            }
                            if p.parallax_probes_by_smart_attenuation {
                                let projected = object.named("use_parallax_correction") == Some(&ConfigValue::Boolean(true));
                                let smart = object.named_mut("smart_attenuation").context("a probe has no smart_attenuation")?;
                                // The marked scene's smart programs draw the plain
                                // falloff: a probe that used the smart one loses it.
                                *smart_probes_lost += usize::from(*smart == ConfigValue::Boolean(true));
                                *smart = ConfigValue::Boolean(projected);
                                *parallax_probes += usize::from(projected);
                            }
                        }
                        _ => {}
                    }
                    materials_of(object, importer)?;
                }
            }
            ("server_objects", ConfigValue::Named(lists)) => {
                for list in lists {
                    let ConfigValue::Indexed(objects) = &mut list.value else { continue };
                    for object in objects {
                        if let Some(ConfigValue::String(model)) = object.named_mut("visual_geometry").and_then(|v| v.named_mut("model_name")) {
                            *model = importer.model(&model.replace('\\', "/"), "model")?;
                        }
                    }
                }
            }
            ("project_name", ConfigValue::String(name)) => *name = m.level.clone(),
            _ => {}
        }
    }
    Ok(())
}

/// A probe's cubemap is the level's own file: it moves to the port's level
/// folder with its diffuse companion and options.
fn probe_cubemap(texture: &str, m: &Manifest, importer: &mut files::Importer<'_>) -> Result<String> {
    let from_folder = format!("environment_probes/{}/", m.donor_level);
    let to = match texture.strip_prefix(&from_folder) {
        Some(rest) => format!("environment_probes/{}/{rest}", m.level),
        None => return importer.texture(texture),
    };
    let mut found = false;
    for suffix in [".dds", ".options", "_diffuse.dds", "_diffuse.options"] {
        let from = format!("textures/{texture}{suffix}");
        if importer.donor.is_file(&from) {
            found |= suffix == ".dds";
            let bytes = importer.donor.read(&from)?;
            importer.out.insert(format!("textures/{to}{suffix}"), bytes);
        }
    }
    if !found {
        importer.report.missing.insert(format!("textures/{texture}.dds"));
    }
    Ok(to)
}

/// The names in an attached list, at any depth.
fn names(value: &ConfigValue, out: &mut BTreeSet<String>) {
    match value {
        ConfigValue::Named(fields) => {
            for f in fields {
                match &f.value {
                    ConfigValue::String(s) if f.name == "name" => {
                        out.insert(s.clone());
                    }
                    other => names(other, out),
                }
            }
        }
        ConfigValue::Indexed(list) => list.iter().for_each(|v| names(v, out)),
        _ => {}
    }
}

/// The project without the server lists a derived mode leaves out, and
/// without the client objects and statics those lists' objects own (a
/// battery point's props, lights and boxes, named in its attached lists).
/// Server objects they name stay: the points' battery spawners are the
/// battery mode's spawners.
fn without_lists(project: &mut ConfigValue, lists: &[String]) -> BTreeMap<String, usize> {
    let mut gone = BTreeMap::new();
    if lists.is_empty() {
        return gone;
    }
    let mut owned = BTreeSet::new();
    if let Some(ConfigValue::Named(server)) = project.named_mut("server_objects") {
        for list in server.iter_mut().filter(|l| lists.contains(&l.name)) {
            if let ConfigValue::Indexed(objects) = &mut list.value {
                for o in objects.iter() {
                    for attached in ["attached_objects", "attached_c_objects"] {
                        if let Some(a) = o.named(attached) {
                            names(a, &mut owned);
                        }
                    }
                }
                *gone.entry(list.name.clone()).or_default() += objects.len();
                objects.clear();
            }
        }
    }
    // A group's member is owned through its group's name.
    let is_owned = |name: &str| owned.contains(name) || name.rsplit_once('/').is_some_and(|(group, _)| owned.contains(group));
    let ConfigValue::Named(top) = project else { return gone };
    for field in top {
        let (ConfigValue::Indexed(list), key) = (&mut field.value, field.name.as_str()) else { continue };
        let before = list.len();
        match key {
            "client_objects" => list.retain(|o| !is_owned(text(o, "full_name").unwrap_or_default())),
            "temporary_collision_objects" => list.retain(|o| {
                let name = text(o, "name").unwrap_or_default();
                !is_owned(name.strip_prefix("walker_").unwrap_or(name))
            }),
            "temporary_render_visuals" => list.retain(|o| {
                let name = text(o, "render_name").unwrap_or_default();
                !is_owned(name.strip_prefix("render_").unwrap_or(name))
            }),
            _ => {}
        }
        if before != list.len() {
            *gone.entry(key.to_owned()).or_default() += before - list.len();
        }
    }
    gone
}

fn count(project: &ConfigValue, key: &str) -> usize {
    match project.named(key) {
        Some(ConfigValue::Indexed(list)) => list.len(),
        _ => 0,
    }
}

/// The mode config players have with this map's identity.
fn options(from: &ConfigValue, m: &Manifest, project: &str, map: &NewMap) -> Result<ConfigValue> {
    let mut out = from.clone();
    let level = m.map_id.context("a new map needs the manifest's map_id")?;
    let name = m.map_name_key.clone().context("a new map needs the manifest's map_name_key")?;
    out.set_path("mode.name", ConfigValue::String(map.options.clone()))?;
    out.set_path("mode.ui.map_name", ConfigValue::String(name))?;
    out.set_path("mode.ui.loading_screen_id", ConfigValue::Integer(map.loading_screen_id))?;
    out.set_path("mode.match_making_params.map_id", ConfigValue::Integer(level))?;
    out.set_path("mode.match_making_params.map_weather_id", ConfigValue::Integer(map.weather_id))?;
    out.set_path("mode.match_making_params.map_name", ConfigValue::String(format!("{}/{project}", m.level)))?;
    Ok(out)
}

fn with_overrides(project: &mut ConfigValue, overrides: &BTreeMap<String, f32>) -> Result<()> {
    if overrides.is_empty() {
        return Ok(());
    }
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else { bail!("no client objects") };
    let environment = objects.iter_mut().find(|o| text(o, "game_object_type") == Some("environment")).context("no environment")?;
    for (name, value) in overrides {
        // In the number type the setting has: a whole number where it is one.
        let written = match environment.named("properties").and_then(|p| p.named(name)) {
            Some(ConfigValue::Integer(_)) if value.fract() == 0.0 => ConfigValue::Integer(*value as i32),
            Some(_) => ConfigValue::Float(*value),
            None => bail!("the environment has no {name} to override"),
        };
        environment.set_path(&format!("properties.{name}"), written)?;
    }
    Ok(())
}

/// The old-form project with 0.26g's fields on every object and static.
fn with_026g_fields(old: &ConfigValue, templates: &fields::Templates, scene: &fields::Scene<'_>, report: &mut fields::FieldReport) -> Result<ConfigValue> {
    let ConfigValue::Named(top) = old else { bail!("a project is not a table") };
    let mut built = Vec::new();
    for field in top {
        let value = match (field.name.as_str(), &field.value) {
            ("client_objects", ConfigValue::Indexed(objects)) => {
                let mut out = Vec::new();
                for o in objects {
                    out.extend(templates.object("client_objects", o, scene, report)?);
                }
                ConfigValue::Indexed(out)
            }
            ("server_objects", ConfigValue::Named(lists)) => {
                let mut server = Vec::new();
                for list in lists {
                    let ConfigValue::Indexed(objects) = &list.value else {
                        server.push(list.clone());
                        continue;
                    };
                    let mut out = Vec::new();
                    for o in objects {
                        out.extend(templates.object(&list.name, o, scene, report)?);
                    }
                    // A list 0.26g has no objects of is not a list it reads.
                    if !out.is_empty() || objects.is_empty() {
                        server.push(NamedValue { name: list.name.clone(), value: ConfigValue::Indexed(out) });
                    }
                }
                ConfigValue::Named(server)
            }
            (list @ ("render_visuals" | "collision_objects" | "temporary_render_visuals" | "temporary_collision_objects"), ConfigValue::Indexed(entries)) => {
                ConfigValue::Indexed(entries.iter().map(|e| templates.static_entry(list, e, report)).collect::<Result<_>>()?)
            }
            (_, value) => value.clone(),
        };
        built.push(NamedValue { name: field.name.clone(), value });
    }
    Ok(ConfigValue::Named(built))
}

/// The archives a list file names, one a line, the first at the bottom.
fn archives(list: &Path) -> Result<Vec<PathBuf>> {
    let layers: Vec<PathBuf> = fs::read_to_string(list)
        .with_context(|| format!("reading {}", list.display()))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect();
    ensure!(!layers.is_empty(), "{}: no archives", list.display());
    Ok(layers)
}

/// Builds the pack of `manifest_path` into `output`. `donor` lists the donor
/// build's archives; `shipped` what players have: the stock db, then every
/// offered patch in order.
/// `unknown_switches`: what a material does whose switch set players'
/// materials do not use (`files::UnknownSwitches`). `wanted_programs`: a
/// file naming shader programs the game asked for and did not find (any
/// text with `shaders/sm_4_0/<family>/<key>` paths in it, such as the
/// asserts of a headless load): each is supplied from the donor or listed
/// with why not (`programs`). `donor_programs`: every program of the
/// donor's that players lack and 0.26g can feed is supplied.
pub fn build(
    manifest_path: &Path,
    donor: &Path,
    shipped: &Path,
    output: &Path,
    unknown_switches: files::UnknownSwitches,
    wanted_programs: Option<&Path>,
    donor_programs: bool,
) -> Result<()> {
    let m: Manifest = serde_json::from_slice(&fs::read(manifest_path)?).with_context(|| format!("reading {}", manifest_path.display()))?;
    ensure!(!output.exists(), "output must be new");
    let (donor_layers, shipped_layers) = (archives(donor)?, archives(shipped)?);
    crate::survarium_026::check_hash(&shipped_layers[0], &m.stock_sha256)?;
    let mut donor = ResourceDb::open_layers(&donor_layers)?;
    let mut shipped = ResourceDb::open_layers(&shipped_layers)?;
    let mut scan_donor = ResourceDb::open_layers(&donor_layers)?;
    let mut scan_shipped = ResourceDb::open_layers(&shipped_layers)?;
    let mut stock = ResourceDb::open(&shipped_layers[0])?;

    // 0.26g's own projects and materials are the definition of its forms.
    let mut templates = fields::Templates::default();
    let stock_projects: Vec<String> = stock.paths_with_prefix("projects/").filter(|p| p.ends_with("/client_project")).map(str::to_owned).collect();
    for path in &stock_projects {
        templates.learn(&cfg(&mut stock, path)?);
    }
    let mut effects = files::Effects::default();
    let materials = |db: &ResourceDb| -> Vec<String> { db.paths_with_prefix("material_instances/").filter(|p| p.ends_with(".material") && db.is_file(p)).map(str::to_owned).collect() };
    for path in materials(&stock) {
        if let Ok(config) = BinaryConfig::parse(&stock.read(&path)?) {
            effects.learn_fields(&config.root);
        }
    }
    for path in materials(&shipped) {
        if let Ok(config) = BinaryConfig::parse(&shipped.read(&path)?) {
            effects.learn_programs(&config.root);
        }
    }

    let leave_out: Vec<String> = m.leave_out.keys().cloned().collect();
    let mut importer = files::Importer::new(&mut donor, &mut shipped, &effects, &m.namespace);
    importer.unknown_switches = unknown_switches;
    let mut reports = Map::new();
    let mut dictionary = Vec::new();
    let mut projects: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for p in &m.projects {
        let mut report = ProjectReport::default();
        let from = format!("projects/{}/{}/client_project", m.donor_level, p.donor);
        let to = format!("projects/{}/{}/client_project", m.level, p.project);
        let source = cfg(importer.donor, &from)?;
        let mut old = if schema::is_new_form(&source) {
            let (kept, gone) = schema::without_kinds(&source, &leave_out)?;
            report.left_out_by_decision = gone;
            let (old, schema_report) = schema::to_old_form(&kept)?;
            report.schema = schema_report;
            old
        } else {
            ensure!(leave_out.is_empty(), "{from}: leaving kinds out of an old-form project is not written");
            source
        };
        for (list, n) in without_lists(&mut old, &p.without_lists) {
            *report.left_out_by_decision.entry(format!("without {}: {list}", p.without_lists.join(", "))).or_default() += n;
        }
        place_files(&mut old, &m, p, &mut importer, &mut report.parallax_probes, &mut report.smart_probes_lost).with_context(|| from.clone())?;
        let scene = fields::Scene { project: &p.project, weather_layer: &p.layer };
        let mut built = with_026g_fields(&old, &templates, &scene, &mut report.fields).with_context(|| from.clone())?;
        with_overrides(&mut built, &p.environment_overrides)?;
        report.client_objects = count(&built, "client_objects");
        report.render_visuals = count(&built, "render_visuals");
        report.collision_objects = count(&built, "collision_objects");
        let bytes = BinaryConfig { root: built }.to_bytes()?;
        BinaryConfig::parse(&bytes).context("the built project reads back")?;

        // What else the project needs that players lack.
        let mut overrides = BTreeMap::new();
        overrides.insert(from.clone(), bytes.clone());
        let plan = dependency::scan_with_forced_donor_and_overrides(&mut scan_donor, &mut scan_shipped, [from.clone()], std::iter::empty(), &overrides)?;
        let level_folder = format!("projects/{}/", m.donor_level);
        for path in plan.donor_files.iter().filter(|f| **f != from) {
            if let Some(rest) = path.strip_prefix(&level_folder) {
                // The level's own files (vegetation layers) move with it.
                ensure!(!rest.ends_with("/client_project"), "{path}: another project is referenced");
                let bytes = importer.donor.read(path)?;
                importer.out.insert(format!("projects/{}/{rest}", m.level), bytes);
            } else if let Some(name) = path.strip_prefix("material_instances/").and_then(|p| p.strip_suffix(".material")) {
                importer.material(name)?;
            } else if let Some(name) = path.strip_prefix("textures/").and_then(|p| p.strip_suffix(".dds")) {
                importer.texture(name)?;
            } else if path.starts_with("textures/") && path.ends_with(".options") {
                // With its image.
                continue;
            } else if let Some((name, kind)) = path
                .strip_prefix("models/")
                .and_then(|p| p.split_once(".model/").map(|(n, _)| (n, "model")).or_else(|| p.split_once(".skinned_model/").map(|(n, _)| (n, "skinned_model"))))
            {
                importer.model(name, kind)?;
            } else if !importer.out.contains_key(path) {
                let bytes = importer.donor.read(path)?;
                importer.out.insert(path.clone(), bytes);
            }
            *report.added.entry(kind_of_path(path).to_owned()).or_default() += 1;
        }
        // The level's own files that players have under the same name (the old London's vegetation
        // layers) are the donor's too: under the port's level, and in place where they differ.
        for path in plan.target_files.iter().filter(|f| f.starts_with(&level_folder) && !f.ends_with("/client_project")) {
            let rest = &path[level_folder.len()..];
            let to_path = format!("projects/{}/{rest}", m.level);
            if !scan_donor.is_file(path) || importer.out.contains_key(&to_path) {
                continue;
            }
            let bytes = scan_donor.read(path)?;
            if m.level != m.donor_level || bytes != scan_shipped.read(path)? {
                importer.out.insert(to_path, bytes);
                *report.added.entry("level file".to_owned()).or_default() += 1;
            }
        }
        for path in &plan.target_files {
            let kind = kind_of_path(path);
            if matches!(kind, "particle" | "sound" | "animation" | "other") && scan_donor.is_file(path) && scan_donor.read(path)? != scan_shipped.read(path)? {
                *report.kept_as_players_have.entry(kind.to_owned()).or_default() += 1;
            }
        }
        // A name the port made is in the pack, not in either archive.
        let own_probes = format!("environment_probes/{}/", m.level);
        report.unresolved = plan
            .unresolved_references
            .iter()
            .filter(|r| !r.contains(&m.namespace) && !r.contains(&own_probes) && !importer.out.keys().any(|k| k.contains(r.as_str())))
            .cloned()
            .collect();

        ensure!(projects.insert(to.clone(), bytes).is_none(), "{to}: built twice");
        if let Some(map) = &p.map {
            let from_options = cfg(importer.shipped, &map.options_from)?;
            projects.insert(format!("gameplay/game_modes/{}.options", map.options), BinaryConfig { root: options(&from_options, &m, &p.project, map)? }.to_bytes()?);
            dictionary.push(match json!({"id": map.id, "cfg_mode_name": format!("{}.options", map.options)}) {
                Json::Object(o) => o,
                _ => unreachable!(),
            });
        }
        println!(
            "{}/{}{}: {} client objects, {} render visuals, {} collisions; left out by decision {:?}; kinds 0.26g lacks {:?}; unresolved {}",
            m.level,
            p.project,
            p.map.as_ref().map_or(" (in place)".to_owned(), |map| format!(" (maps_dict {})", map.id)),
            report.client_objects, report.render_visuals, report.collision_objects, report.left_out_by_decision, report.fields.kinds_left_out, report.unresolved.len()
        );
        reports.insert(to, serde_json::to_value(&report)?);
    }

    let files::Importer { out, report: file_report, shipped, donor, .. } = importer;
    let mut out = out;
    let mut supplied = Vec::new();
    let mut families = BTreeMap::new();
    let mut families_skipped = BTreeMap::new();
    if wanted_programs.is_some() || donor_programs {
        let orders = m.shader_keys.as_ref().context("the manifest names no shader key orders")?;
        let beside = manifest_path.parent().unwrap_or(Path::new("."));
        let order = |file: &Path| -> Result<programs::KeyOrder> {
            programs::KeyOrder::parse(&fs::read_to_string(beside.join(file)).with_context(|| format!("reading {}", file.display()))?)
        };
        let target_keys = programs::Keys::read(order(&orders.target)?, shipped)?;
        let donor_keys = programs::Keys::read(order(&orders.donor)?, donor)?;
        let mut supplier = programs::Supplier::new(&target_keys, &donor_keys);
        if donor_programs {
            ensure!(!orders.families.is_empty(), "the manifest's shader_keys names no families to supply");
            let (blobs, by_family, skipped) = supplier.supply_all(shipped, donor, &orders.families)?;
            println!(
                "the donor's programs players lack: {} supplied of {} in {} families; {} families left alone",
                blobs.len(),
                by_family.values().map(|f: &programs::FamilySupply| f.lacking).sum::<usize>(),
                by_family.len(),
                skipped.len()
            );
            out.extend(blobs);
            families = by_family;
            families_skipped = skipped;
        }
        if let Some(wanted) = wanted_programs {
            let text = fs::read_to_string(wanted).with_context(|| format!("reading {}", wanted.display()))?;
            let mut paths = BTreeSet::new();
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || "/._".contains(c))) {
                if let Some(at) = word.find("shaders/sm_4_0/") {
                    paths.insert(word[at..].trim_end_matches('.').to_owned());
                }
            }
            for path in &paths {
                if out.contains_key(path) {
                    supplied.push(programs::Supply { path: path.clone(), donor: Some("supplied with the donor's programs".into()), not_supplied: None });
                    continue;
                }
                let (supply, blob) = supplier.supply(shipped, donor, path)?;
                if let Some(blob) = blob {
                    out.insert(path.clone(), blob);
                }
                supplied.push(supply);
            }
            println!(
                "programs asked for {}: supplied from the donor {}, not supplied {}",
                supplied.len(),
                supplied.iter().filter(|s| s.donor.is_some()).count(),
                supplied.iter().filter(|s| s.not_supplied.is_some()).count()
            );
        }
    }
    ensure!(
        file_report.without_programs.is_empty(),
        "materials whose switches have no shader programs and no smaller set that has them: {:?}",
        file_report.without_programs
    );
    let mut all = out;
    for (path, bytes) in projects {
        ensure!(all.insert(path.clone(), bytes).is_none(), "{path}: written twice");
    }
    let mut operations = Vec::new();
    let (mut added, mut replaced, mut same) = (0, 0, 0);
    for (path, bytes) in &all {
        let operation = if shipped.is_file(path) {
            let theirs = shipped.read(path)?;
            if &theirs == bytes {
                same += 1;
                continue;
            }
            replaced += 1;
            Operation::ReplaceFile { path: path.clone(), source: None, base_sha256: sha256_hex(&theirs) }
        } else {
            added += 1;
            Operation::AddFile { path: path.clone(), source: None }
        };
        let at = crate::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, bytes)?;
        operations.push(operation);
    }
    let new_ids: Vec<Json> = m.projects.iter().filter_map(|p| p.map.as_ref()).map(|map| json!({"id": map.id})).collect();
    if !dictionary.is_empty() {
        operations.push(Operation::DictionaryAdd { table: "maps_dict".into(), id_field: None, entries: dictionary });
    }
    let mut notes = vec![format!(
        "Built by build-survarium-near-map from {}'s own projects of {} as {}: {added} files added, {replaced} replaced, {same} already players'; imports under {}.",
        m.donor_build, m.donor_level, m.level, m.namespace
    )];
    for (kind, why) in &m.leave_out {
        notes.push(format!("Left out with its group: {kind} ({why})."));
    }
    let pack = PackManifest {
        format_version: 1,
        id: m.pack.id.clone(),
        version: m.pack.version.clone(),
        target_build: TARGET_BUILD.into(),
        description: Some(m.pack.description.clone()),
        requires: vec![],
        base_db_sha256: Some(m.stock_sha256.clone()),
        operations,
        server: (!new_ids.is_empty()).then(|| json!({"maps": new_ids})),
        notes,
    };
    fs::create_dir_all(output)?;
    fs::write(output.join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    fs::write(output.join("report.json"), serde_json::to_vec_pretty(&json!({"files": file_report, "projects": reports, "programs": supplied, "donor_programs": families, "donor_program_families_left_alone": families_skipped}))?)?;
    println!(
        "{added} files added, {replaced} replaced, {same} already players'; textures {:?}, materials {:?}, models {:?}; {} materials took a smaller switch set; {} names missing in both builds",
        file_report.textures,
        file_report.materials,
        file_report.models,
        file_report.degraded.len(),
        file_report.missing.len()
    );
    Ok(())
}
