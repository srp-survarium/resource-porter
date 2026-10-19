// SPDX-License-Identifier: GPL-3.0-or-later
//! Add retail 0.21d maps to 0.26g without replacing its maps or shaders.
use crate::manifest::BuiltFile;
use crate::{BinaryConfig, ConfigValue, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

const DICTIONARY: &str = "gameplay/db_static_dictionaries";
const NAMESPACE: &str = "ported/v021d";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub donor_sha256: String,
    pub target_sha256: String,
    pub maps: Vec<Map>,
    #[serde(default)]
    pub reference_replacements: BTreeMap<String, String>,
    #[serde(default)]
    pub material_templates: BTreeMap<String, String>,
    #[serde(default)]
    pub surface_renames: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub lod0_promotions: BTreeSet<String>,
    #[serde(default)]
    pub missing_collision_resources: BTreeSet<String>,
    /// A reviewed constant finish for a donor material 0.26g has no material
    /// of (a material of the same path takes the native finish).
    #[serde(default)]
    pub material_finishes: BTreeMap<String, crate::material_finish::ConstantFinish>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Map {
    pub id: u8,
    pub level: String,
    pub weather: String,
    /// Take the lighting objects (environment probes, ambient lights, lights)
    /// from the native 0.26 project of the same level and weather instead of
    /// the donor's. For levels that 0.26 itself ships, whose probes were
    /// re-baked and re-tuned for 0.26's lighting.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_lighting: bool,
    /// `mode.match_making_params.map_id` of the mode config: the number a
    /// server counts a map by, whatever its weather and mode. Unset, the
    /// native map's (3 for level_03), which makes the import a weather
    /// variant of the native map to the server. The client never reads it
    /// (the exe has no `map_id` key); the server reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_id: Option<u8>,
    /// The map's own name (`mode.ui.map_name`) in the client's six
    /// languages. Unset, the native map's (`st_level_03_name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<MapName>,
}

/// A map name string: its localization key and its text in each language
/// the 0.26g client ships (`original` is the Russian).
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapName {
    pub key: String,
    pub english: String,
    pub russian: String,
    pub ukrainian: String,
    pub german: String,
    pub polish: String,
}
impl MapName {
    fn text(&self, language: &str) -> Result<&str> {
        Ok(match language {
            "english" => &self.english,
            "original" | "russian" => &self.russian,
            "ukrainian" => &self.ukrainian,
            "german" => &self.german,
            "polish" => &self.polish,
            _ => bail!("no map name for the {language} localization"),
        })
    }
}

impl Map {
    fn donor_project(&self) -> String {
        format!("projects/{}/{}/client_project", self.level, self.weather)
    }
    fn target_project(&self) -> String {
        format!("projects/{}/{}_2/client_project", self.level, self.weather)
    }
    fn project_name(&self) -> String {
        format!("v021d_{}/{}_2", self.level, self.weather)
    }
    fn output_project(&self) -> String {
        format!("projects/{}/client_project", self.project_name())
    }
    fn mode(&self) -> String {
        format!(
            "{}_{}_2.options",
            self.level,
            if self.weather == "evening" {
                "evn"
            } else {
                &self.weather
            }
        )
    }
}

#[derive(Serialize)]
pub struct Record {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub target_db_sha256: String,
    pub maps: Vec<Map>,
    pub files: Vec<BuiltFile>,
    pub converted_materials: BTreeMap<String, String>,
    pub shared_dependencies: BTreeSet<String>,
    pub repaired_model_settings: BTreeSet<String>,
    pub adapted_particles: BTreeSet<String>,
    pub collision_repairs: BTreeMap<String, String>,
    /// Converted materials that took the finish of the 0.26g material of the
    /// same path ([`crate::material_finish::adopt_native_finish`]).
    pub native_finish_materials: BTreeSet<String>,
    /// Donor textures referenced as the target's own, not imported.
    pub native_textures: BTreeMap<String, crate::native_equivalence::TextureMatch>,
    /// Donor materials whose conversion is the target's material of the same
    /// path, referenced as the target's own.
    pub native_materials: BTreeSet<String>,
}

pub fn check_hash(path: &Path, expected: &str) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "database fingerprint mismatch: {}",
        path.display()
    );
    Ok(())
}

fn cfg(db: &mut ResourceDb, path: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(&db.read(path)?)
        .with_context(|| format!("decoding {path}"))?
        .root)
}
fn string<'a>(c: &'a ConfigValue, k: &str) -> Option<&'a str> {
    match c.named(k) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}
fn array(c: &ConfigValue) -> Result<&[ConfigValue]> {
    match c {
        ConfigValue::Indexed(v) => Ok(v),
        ConfigValue::Named(v) if v.is_empty() => Ok(&[]),
        _ => bail!("expected array"),
    }
}

/// Fill new fields, preserving the donor's authored values and array members.
fn fill_missing(value: &mut ConfigValue, template: &ConfigValue) {
    if let (ConfigValue::Named(dst), ConfigValue::Named(src)) = (value, template) {
        for field in src {
            if let Some(old) = dst.iter_mut().find(|v| v.name == field.name) {
                fill_missing(&mut old.value, &field.value);
            } else {
                dst.push(field.clone());
            }
        }
    }
}

fn fill_missing_empty_groups(value: &mut ConfigValue, template: &ConfigValue) {
    if let (ConfigValue::Named(dst), ConfigValue::Named(src)) = (value, template) {
        for field in src {
            if !dst.iter().any(|v| v.name == field.name) {
                let value = match &field.value {
                    ConfigValue::Indexed(_) => ConfigValue::Indexed(Vec::new()),
                    ConfigValue::Named(_) => ConfigValue::Named(Vec::new()),
                    other => other.clone(),
                };
                dst.push(NamedValue {
                    name: field.name.clone(),
                    value,
                });
            }
        }
    }
}

fn adapt_objects(objects: &mut ConfigValue, templates: &[ConfigValue]) -> Result<()> {
    let ConfigValue::Indexed(objects) = objects else {
        return Ok(());
    };
    for object in objects {
        let kind = string(object, "game_object_type")
            .context("object has no type")?
            .to_owned();
        let template = templates
            .iter()
            .find(|p| {
                string(p, "game_object_type") == Some(&kind)
                    && string(p, "lib_name") == string(object, "lib_name")
            })
            .or_else(|| {
                templates
                    .iter()
                    .find(|p| string(p, "game_object_type") == Some(&kind))
            })
            .with_context(|| format!("no 0.26 object template for {kind}"))?;
        // A new nested world-space transform must come from this spawn, never
        // from an unrelated point in the template map.
        if kind == "victory_item_spawner" && object.named("spawned_item").is_none() {
            let fields = ["position", "rotation"]
                .into_iter()
                .map(|name| {
                    Ok(NamedValue {
                        name: name.into(),
                        value: object
                            .named(name)
                            .with_context(|| format!("spawn has no {name}"))?
                            .clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            object.set_path("spawned_item", ConfigValue::Named(fields))?;
        }
        fill_missing(object, template);
        object.set_path("is_temporary", ConfigValue::Boolean(false))?;
        if kind == "environment_probe" {
            if let Some(ConfigValue::Named(textures)) = object.named_mut("textures") {
                // The lookup uses the project variant, now suffixed by mode.
                let old: Vec<_> = textures
                    .iter()
                    .filter(|v| matches!(v.name.as_str(), "day" | "evening" | "rain"))
                    .cloned()
                    .collect();
                for mut entry in old {
                    entry.name.push_str("_2");
                    textures.retain(|v| v.name != entry.name);
                    textures.push(entry);
                }
            }
        }
    }
    Ok(())
}

/// Lighting object types replaced by `Map::native_lighting`.
const LIGHTING_OBJECT_TYPES: [&str; 3] = ["environment_probe", "ambient_light", "light"];

/// Translation from the native project's layout to the donor's: the most
/// common position difference of models placed exactly once in both. 0.26
/// level_03 matches 0.21d exactly; 0.26 level_06 is the same layout raised by
/// 7.28 m.
fn layout_offset(donor: &ConfigValue, native: &ConfigValue) -> Result<[f32; 3]> {
    let placements = |project: &ConfigValue| -> Result<BTreeMap<String, Vec<[f32; 3]>>> {
        let mut result: BTreeMap<String, Vec<[f32; 3]>> = BTreeMap::new();
        if let Some(visuals) = project.named("render_visuals") {
            for visual in array(visuals)? {
                if let (Some(name), Some(ConfigValue::Float3(position))) =
                    (string(visual, "lib_name"), visual.named("position"))
                {
                    result.entry(name.to_owned()).or_default().push(*position);
                }
            }
        }
        Ok(result)
    };
    let (donor, native) = (placements(donor)?, placements(native)?);
    let mut votes: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut unique = 0;
    for (name, at) in &donor {
        let Some(other) = native.get(name) else { continue };
        if at.len() != 1 || other.len() != 1 {
            continue;
        }
        unique += 1;
        let key = [0, 1, 2].map(|i| ((at[0][i] - other[0][i]) * 100.).round() as i64);
        *votes.entry(key).or_default() += 1;
    }
    let (key, count) = votes
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .context("no model placed once in both projects")?;
    ensure!(
        count >= 10 && count * 2 >= unique,
        "native layout does not match the donor: best common offset has {count} of {unique} unique models"
    );
    Ok(key.map(|v| v as f32 / 100.))
}

/// Replace the donor's lighting objects with the native project's, moved by
/// `offset` (native to donor layout). Returns (removed, added).
fn adopt_native_lighting(
    project: &mut ConfigValue,
    native: &ConfigValue,
    offset: [f32; 3],
) -> Result<(usize, usize)> {
    let is_lighting = |o: &ConfigValue| {
        string(o, "game_object_type").is_some_and(|kind| LIGHTING_OBJECT_TYPES.contains(&kind))
    };
    let replacement: Vec<ConfigValue> = array(
        native
            .named("client_objects")
            .context("native project lacks client_objects")?,
    )?
    .iter()
    .filter(|o| is_lighting(o))
    .cloned()
    .map(|mut o| {
        if let Some(ConfigValue::Float3(position)) = o.named_mut("position") {
            for i in 0..3 {
                position[i] += offset[i];
            }
        }
        o
    })
    .collect();
    ensure!(
        replacement
            .iter()
            .any(|o| string(o, "game_object_type") == Some("environment_probe")),
        "native project has no environment probes"
    );
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("project lacks indexed client_objects");
    };
    let before = objects.len();
    objects.retain(|o| !is_lighting(o));
    let removed = before - objects.len();
    let added = replacement.len();
    objects.extend(replacement);
    Ok((removed, added))
}

fn adapt_project(
    mut donor: ConfigValue,
    target: &ConfigValue,
    map: &Map,
    all_templates: &[ConfigValue],
) -> Result<ConfigValue> {
    let mut templates = array(
        target
            .named("client_objects")
            .context("target objects missing")?,
    )?
    .to_vec();
    if let Some(ConfigValue::Named(groups)) = target.named("server_objects") {
        for group in groups {
            templates.extend(array(&group.value)?.iter().cloned());
        }
    }
    templates.extend_from_slice(all_templates);
    let donor_objects = donor
        .named_mut("client_objects")
        .context("0.21 project lacks client_objects")?;
    adapt_objects(donor_objects, &templates)?;
    let ConfigValue::Named(groups) = donor
        .named_mut("server_objects")
        .context("0.21 project lacks server_objects")?
    else {
        bail!("invalid server_objects")
    };
    for group in groups {
        adapt_objects(&mut group.value, &templates)?;
    }
    // Add root-level fields only. Recursing across heterogeneous object arrays
    // would incorrectly copy another map's objects and transforms.
    if let (Some(dst), Some(src)) = (
        donor.named_mut("server_objects"),
        target.named("server_objects"),
    ) {
        fill_missing_empty_groups(dst, src);
    }
    fill_missing_empty_groups(&mut donor, target);
    // This is the HUD minimap key, not the resource directory. The retail HUD
    // falls back to level_09 for unknown names. Keep the authored key while
    // output_project()/mode.map_name relocate the actual project resources.
    ensure!(
        string(&donor, "project_name") == Some(map.level.as_str()),
        "source project has no matching retail HUD minimap key"
    );
    if let Some(ConfigValue::Indexed(visuals)) = donor.named_mut("render_visuals") {
        for visual in visuals {
            if visual.named("visual_name").is_none() {
                visual.set_path(
                    "visual_name",
                    ConfigValue::String(
                        string(visual, "lib_name")
                            .context("render visual lacks model lib_name")?
                            .to_owned(),
                    ),
                )?;
            }
        }
    }
    validate_gameplay(&donor)?;
    Ok(donor)
}

/// All authored interaction-zone references must still resolve after upgrading.
fn validate_gameplay(project: &ConfigValue) -> Result<()> {
    let objects = array(
        project
            .named("client_objects")
            .context("missing client objects")?,
    )?;
    let names: BTreeSet<_> = objects
        .iter()
        .filter(|o| string(o, "game_object_type") == Some("collision_geometry"))
        .filter_map(|o| string(o, "full_name"))
        .collect();
    let groups = project
        .named("server_objects")
        .context("missing server objects")?;
    let collectors = array(
        groups
            .named("victory_items_containers")
            .context("missing collectors")?,
    )?;
    ensure!(
        collectors.len() == 2,
        "battery mode requires two collectors"
    );
    let teams: BTreeSet<_> = collectors
        .iter()
        .filter_map(|o| match o.named("team") {
            Some(ConfigValue::Integer(v)) => Some(*v),
            _ => None,
        })
        .collect();
    ensure!(
        teams == BTreeSet::from([0, 1]),
        "collectors must belong to opposite teams"
    );
    fn check(c: &ConfigValue, names: &BTreeSet<&str>) -> Result<()> {
        match c {
            ConfigValue::Named(fields) => {
                for field in fields {
                    if field.name == "collision_geometries" {
                        for zone in array(&field.value)? {
                            let name = string(zone, "name").context("unnamed interaction zone")?;
                            ensure!(names.contains(name), "missing interaction zone {name}");
                        }
                    } else {
                        check(&field.value, names)?;
                    }
                }
            }
            ConfigValue::Indexed(v) => {
                for child in v {
                    check(child, names)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    check(groups, &names)
}

fn relocate_path(path: &str) -> String {
    for family in ["sounds/single/", "sounds/collections/", "sounds/composite/"] {
        if let Some(tail) = path.strip_prefix(family) {
            return format!("{family}{NAMESPACE}/{tail}");
        }
    }
    let (family, tail) = path.split_once('/').expect("resource has a family");
    if family == "projects" {
        format!("projects/v021d_{tail}")
    } else {
        format!("{family}/{NAMESPACE}/{tail}")
    }
}

fn replacements(paths: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for path in paths {
        let dest = relocate_path(path);
        result.insert(path.clone(), dest.clone());
        result.insert(format!("resources/{path}"), format!("resources/{dest}"));
        for marker in [".model/", ".skinned_model/"] {
            if let Some((model, _)) = path.split_once(marker) {
                let model_ext = &marker[..marker.len() - 1];
                let full = format!("{model}{model_ext}");
                let relocated = relocate_path(&full);
                result.insert(full.clone(), relocated.clone());
                result.insert(
                    format!("resources/{full}"),
                    format!("resources/{relocated}"),
                );
                result.insert(
                    model.trim_start_matches("models/").into(),
                    relocated
                        .trim_start_matches("models/")
                        .trim_end_matches(model_ext)
                        .into(),
                );
            }
        }
        for (family, ext) in [
            ("textures/", ".dds"),
            ("material_instances/", ".material"),
            ("particles/", ".particle"),
            ("particles/", ".binary_particle"),
            ("sounds/single/", ".raw_options"),
            ("sounds/collections/", ".sound_collection"),
            ("sounds/composite/", ".composite_sound"),
        ] {
            if path.starts_with(family) && path.ends_with(ext) {
                result.insert(
                    path.trim_start_matches(family).trim_end_matches(ext).into(),
                    dest.trim_start_matches(family).trim_end_matches(ext).into(),
                );
            }
        }
    }
    result
}

fn rewrite(c: &mut ConfigValue, substitutions: &BTreeMap<String, String>) {
    rewrite_inner(c, substitutions, false, None);
}

fn rewrite_inner(
    c: &mut ConfigValue,
    substitutions: &BTreeMap<String, String>,
    resource_names: bool,
    family: Option<&str>,
) {
    match c {
        ConfigValue::String(s) => {
            let normalized = s.replace('\\', "/");
            if let Some(family) = family {
                // Short names are local to their resource family. A material
                // and a shared native particle can both be fx/lights/oil_lamp.
                let candidates: &[(&str, &str)] = match family {
                    "particle" => &[
                        ("particles/", ".particle"),
                        ("particles/", ".binary_particle"),
                    ],
                    "texture" => &[("textures/", ".dds")],
                    "material" => &[("material_instances/", ".material")],
                    "sound" => &[
                        ("sounds/single/", ".raw_options"),
                        ("sounds/collections/", ".sound_collection"),
                        ("sounds/composite/", ".composite_sound"),
                    ],
                    _ => unreachable!(),
                };
                for (prefix, extension) in candidates {
                    if let Some(dest) =
                        substitutions.get(&format!("{prefix}{normalized}{extension}"))
                    {
                        *s = dest
                            .strip_prefix(prefix)
                            .unwrap()
                            .strip_suffix(extension)
                            .unwrap()
                            .into();
                        return;
                    }
                }
                // Explicitly qualified references still have an unambiguous
                // mapping. Otherwise retain the shared native short name.
                if let Some(dest) = substitutions.get(&normalized).filter(|_| {
                    normalized.starts_with("resources/")
                        || candidates.iter().any(|(p, _)| normalized.starts_with(p))
                }) {
                    *s = dest.clone();
                }
                return;
            }
            let (base, suffix) = normalized
                .split_once('#')
                .map_or((normalized.as_str(), ""), |(a, b)| (a, b));
            let mapped = substitutions.get(base).cloned().or_else(|| {
                // Model collision subresources carry suffixes such as /collision.
                [".model/", ".skinned_model/"].iter().find_map(|marker| {
                    let at = base.find(marker)? + marker.len() - 1;
                    substitutions
                        .get(&base[..at])
                        .map(|p| format!("{p}{}", &base[at..]))
                })
            });
            if let Some(mut mapped) = mapped {
                if !suffix.is_empty() {
                    mapped.push('#');
                    mapped.push_str(suffix);
                }
                *s = mapped;
            }
        }
        ConfigValue::Named(v) => {
            let particle = v.iter().any(|f| {
                f.name == "game_object_type"
                    && matches!(&f.value, ConfigValue::String(s) if s == "particle")
            });
            for f in v {
                let child_family = if !resource_names {
                    None
                } else if f.name == "lib_name" && particle {
                    Some("particle")
                } else if f.name.starts_with("texture_") || f.name.ends_with("_texture") {
                    Some("texture")
                } else if f.name == "material_name" {
                    Some("material")
                } else if f.name == "sound_name" {
                    Some("sound")
                } else if f.name == "value" {
                    family
                } else {
                    None
                };
                rewrite_inner(&mut f.value, substitutions, resource_names, child_family)
            }
        }
        ConfigValue::Indexed(v) => {
            for f in v {
                rewrite_inner(f, substitutions, resource_names, family)
            }
        }
        _ => (),
    }
}

/// Remove stale references to geometry absent from the donor itself, retaining
/// its LOD assignments. New material associations require an explicit review.
fn repair_settings(
    c: &mut ConfigValue,
    actual: &BTreeSet<String>,
    renames: &BTreeMap<String, String>,
) -> Result<bool> {
    let before = c.clone();
    if let Some(ConfigValue::Named(materials)) = c.named_mut("material_settings") {
        for entry in materials.iter_mut() {
            if let Some(name) = renames.get(&entry.name) {
                entry.name = name.clone();
            }
        }
        materials.retain(|entry| actual.contains(&entry.name));
        let named: BTreeSet<_> = materials.iter().map(|entry| entry.name.clone()).collect();
        ensure!(
            &named == actual,
            "render geometry has no reviewed material association: {:?}",
            actual.difference(&named).collect::<Vec<_>>()
        );
    } else {
        bail!("model has no material_settings")
    }
    fn prune(c: &mut ConfigValue, actual: &BTreeSet<String>, renames: &BTreeMap<String, String>) {
        match c {
            ConfigValue::Indexed(v) => {
                for value in v.iter_mut() {
                    if let ConfigValue::String(s) = value {
                        if let Some(new) = renames.get(s) {
                            *s = new.clone();
                        }
                    }
                }
                v.retain(|value| !matches!(value,ConfigValue::String(s) if !actual.contains(s)));
            }
            ConfigValue::Named(v) => {
                for entry in v {
                    prune(&mut entry.value, actual, renames)
                }
            }
            _ => (),
        }
    }
    if let Some(lod) = c.named_mut("lod_hierrarchy") {
        prune(lod, actual, renames);
    }
    Ok(*c != before)
}

fn particle_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "particle_entity_emitter_type" => &["AffectTransformToVelocity"],
        "particle_action_random_velocity_type" => {
            &["UseAnimatedMultiplier", "AnimatedVelocityMultiplier"]
        }
        "particle_action_turbulence_type" => &["Noise"],
        "particle_action_light_type" => &["MinScreenFactorForShadowCasting", "AffectClouds"],
        _ => &[],
    }
}
fn particle_defaults(target: &mut ResourceDb) -> Result<BTreeMap<String, ConfigValue>> {
    fn collect(c: &ConfigValue, defaults: &mut BTreeMap<String, ConfigValue>) {
        if let Some(kind) = string(c, "particle_entity_type") {
            if let Some(properties) = c.named("properties") {
                for name in particle_fields(kind) {
                    if let Some(value) = properties.named(name) {
                        defaults
                            .entry(format!("{kind}/{name}"))
                            .or_insert_with(|| value.clone());
                    }
                }
            }
        }
        match c {
            ConfigValue::Named(v) => {
                for e in v {
                    collect(&e.value, defaults)
                }
            }
            ConfigValue::Indexed(v) => {
                for e in v {
                    collect(e, defaults)
                }
            }
            _ => (),
        }
    }
    let mut defaults = BTreeMap::new();
    let paths: Vec<_> = target
        .paths_with_prefix("particles/")
        .filter(|p| p.ends_with(".particle"))
        .map(str::to_owned)
        .collect();
    for path in paths {
        collect(&cfg(target, &path)?, &mut defaults);
    }
    // These switches did not exist in 0.21; leave the new behaviours off.
    for (kind, name) in [
        ("particle_entity_emitter_type", "AffectTransformToVelocity"),
        (
            "particle_action_random_velocity_type",
            "UseAnimatedMultiplier",
        ),
        ("particle_action_light_type", "AffectClouds"),
    ] {
        let key = format!("{kind}/{name}");
        ensure!(
            defaults.contains_key(&key),
            "missing native particle property {key}"
        );
        defaults.insert(key, ConfigValue::Boolean(false));
    }
    Ok(defaults)
}
fn adapt_particle(c: &mut ConfigValue, defaults: &BTreeMap<String, ConfigValue>) -> Result<bool> {
    let mut changed = false;
    if let Some(kind) = string(c, "particle_entity_type").map(str::to_owned) {
        if let Some(properties) = c.named_mut("properties") {
            for name in particle_fields(&kind) {
                if properties.named(name).is_none() {
                    let value = defaults
                        .get(&format!("{kind}/{name}"))
                        .with_context(|| format!("no native particle default for {kind}/{name}"))?;
                    properties.set_path(name, value.clone())?;
                    changed = true;
                }
            }
        }
    }
    match c {
        ConfigValue::Named(v) => {
            for e in v {
                changed |= adapt_particle(&mut e.value, defaults)?
            }
        }
        ConfigValue::Indexed(v) => {
            for e in v {
                changed |= adapt_particle(e, defaults)?
            }
        }
        _ => (),
    }
    Ok(changed)
}
fn adapt_collisions(
    project: &mut ConfigValue,
    donor: &ResourceDb,
    allowed_missing: &BTreeSet<String>,
    project_path: &str,
    repairs: &mut BTreeMap<String, String>,
) -> Result<()> {
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("collision_objects") else {
        return Ok(());
    };
    let mut retained = Vec::new();
    let available = |path: &str| {
        donor.is_file(path)
            || donor
                .paths_with_prefix(&format!("{path}/"))
                .any(|p| donor.is_file(p))
    };
    for mut object in std::mem::take(objects) {
        let reference = string(&object, "lib_name")
            .context("collision lacks lib_name")?
            .to_owned();
        let normalized = reference.replace('\\', "/");
        let (base, suffix) = normalized
            .split_once('#')
            .map_or((normalized.as_str(), String::new()), |(a, b)| {
                (a, format!("#{b}"))
            });
        let path = base.trim_start_matches("resources/");
        if !available(path) && donor.contains(path) {
            // An empty folder node is an authored "no shape": the source cook
            // probes it by path and creates nothing (retail 0.26 ships the
            // same empty walker nodes for these models). Substituting the
            // other collision kind would add bullet geometry to the movement
            // group, or walker geometry to the bullet group.
            repairs.insert(
                format!("{project_path} -> {reference}"),
                "omitted: empty collision payload in donor; source creates no shape".into(),
            );
            continue;
        }
        if !available(path) {
            let (model, kind) = path.rsplit_once('/').context("invalid collision path")?;
            let alternate = format!(
                "{model}/{}",
                if kind == "walker_collision" {
                    "collision"
                } else {
                    "walker_collision"
                }
            );
            if available(&alternate) {
                let replacement = format!(
                    "{}{alternate}{suffix}",
                    if base.starts_with("resources/") {
                        "resources/"
                    } else {
                        ""
                    }
                );
                object.set_path("lib_name", ConfigValue::String(replacement.clone()))?;
                repairs.insert(format!("{project_path} -> {reference}"), replacement);
            } else {
                ensure!(
                    allowed_missing.contains(path),
                    "no donor collision for {reference}"
                );
                repairs.insert(
                    format!("{project_path} -> {reference}"),
                    "omitted: no collision payload in donor".into(),
                );
                continue;
            }
        }
        retained.push(object);
    }
    *objects = retained;
    Ok(())
}

fn is_material(path: &str) -> bool {
    path.starts_with("material_instances/") && path.ends_with(".material")
}

fn texture_options(dds: &str) -> String {
    format!("{}.options", dds.strip_suffix(".dds").unwrap_or(dds))
}

struct ConvertedMaterial {
    root: ConfigValue,
    template_path: String,
    /// The native maps it binds ([`crate::material_finish::adopt_native_finish`]).
    native_maps: BTreeSet<String>,
    native_finish: bool,
}

/// A donor material on its 0.26 template, with the finish of the target's
/// material of the same path, or the manifest's reviewed constant finish.
fn convert_material(
    m: &Manifest,
    catalog: &crate::material_conversion::MaterialCatalog,
    donor: &ResourceDb,
    target: &mut ResourceDb,
    path: &str,
    bytes: &[u8],
) -> Result<ConvertedMaterial> {
    let native = target.is_file(path);
    let explicit = m
        .material_templates
        .get(path)
        .map(String::as_str)
        .or_else(|| native.then_some(path));
    let converted = catalog
        .convert(path, bytes, explicit, donor, target)
        .with_context(|| format!("converting {path}"))?;
    let mut root = BinaryConfig::parse(&converted.bytes)?.root;
    let mut native_maps = BTreeSet::new();
    if native {
        ensure!(
            !m.material_finishes.contains_key(path),
            "{path} has a native material; it takes the native finish"
        );
        native_maps = crate::material_finish::adopt_native_finish(&mut root, &cfg(target, path)?)
            .with_context(|| format!("native finish of {path}"))?;
    } else if let Some(finish) = m.material_finishes.get(path) {
        crate::material_finish::set_constant_finish(&mut root, *finish)
            .with_context(|| format!("finish of {path}"))?;
    }
    Ok(ConvertedMaterial {
        root,
        template_path: converted.template_path,
        native_maps,
        native_finish: native,
    })
}

/// The donor textures and converted materials that are the target's own
/// (see [`crate::native_equivalence`]). A converted material is the
/// target's when it is the same config and every texture it binds is the
/// target's.
fn native_equivalents(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    donor_files: &BTreeSet<String>,
    materials: &BTreeMap<String, ConvertedMaterial>,
    native_bound: &BTreeSet<String>,
) -> Result<(
    BTreeMap<String, crate::native_equivalence::TextureMatch>,
    BTreeSet<String>,
)> {
    let mut textures = BTreeMap::new();
    for path in donor_files
        .iter()
        .filter(|p| p.starts_with("textures/") && p.ends_with(".dds"))
    {
        if !target.is_file(path) {
            continue;
        }
        let options = texture_options(path);
        let donor_options = if donor.is_file(&options) {
            Some(donor.read(&options)?)
        } else {
            None
        };
        let target_options = if target.is_file(&options) {
            Some(target.read(&options)?)
        } else {
            None
        };
        if let Some(kind) = crate::native_equivalence::texture_match(
            &donor.read(path)?,
            donor_options.as_deref(),
            &target.read(path)?,
            target_options.as_deref(),
        ) {
            textures.insert(path.clone(), kind);
        }
    }
    let mut native_materials = BTreeSet::new();
    for (path, converted) in materials {
        if !target.is_file(path) || cfg(target, path)? != converted.root {
            continue;
        }
        let all_native = crate::material_finish::material_textures(&converted.root)
            .iter()
            .all(|texture| {
                let Ok(name) = crate::material_conversion::canonical_texture_reference(texture)
                else {
                    return false;
                };
                let dds = format!("textures/{name}.dds");
                native_bound.contains(&name)
                    || textures.contains_key(&dds)
                    || (!donor_files.contains(&dds) && target.is_file(&dds))
            });
        if all_native {
            native_materials.insert(path.clone());
        }
    }
    Ok((textures, native_materials))
}

fn is_config_resource(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "client_project"
            | "server_project"
            | "settings"
            | "export_properties"
            | "exported_primitives"
    ) || [
        ".material",
        ".options",
        ".particle",
        ".binary_particle",
        ".sound_collection",
        ".composite_sound",
        ".raw_options",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
}

fn emit(
    output: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let dest = crate::archive::safe_join(&output.join("overlay"), path)?;
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(dest, bytes)?;
    files.push(crate::weapon::record_file(path, source, bytes));
    Ok(())
}

/// Native School geometry with School's overcast lighting and Rudnya's fog class.
pub fn build_native_fog(target_path: &Path, output: &Path) -> Result<()> {
    const HASH: &str = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    const PROJECT: &str = "projects/level_02/day_2/client_project";
    check_hash(target_path, HASH)?;
    ensure!(!output.exists(), "output already exists");
    let mut db = ResourceDb::open(target_path)?;
    let mut school = cfg(&mut db, PROJECT)?;
    let original = school.clone();
    let rain = cfg(&mut db, "projects/level_02/rain_2/client_project")?;
    let rudnya = cfg(&mut db, "projects/level_01/day_2/client_project")?;
    let scene_object = |project: &ConfigValue, kind: &str| -> Result<ConfigValue> {
        let objects = array(
            project
                .named("client_objects")
                .context("missing client objects")?,
        )?;
        let matches: Vec<_> = objects
            .iter()
            .filter(|o| string(o, "game_object_type") == Some(kind))
            .collect();
        ensure!(matches.len() == 1, "expected one {kind}");
        Ok(matches[0].clone())
    };
    let rain_env = scene_object(&rain, "environment")?;
    let mut properties = rain_env
        .named("properties")
        .context("environment properties")?
        .clone();
    for (key, value) in [
        ("rayleigh_fog_near_distance", 0.0),
        ("rayleigh_fog_far_distance", 90.0),
        ("mie_fog_near_distance", 1.0),
        ("mie_fog_far_distance", 70.0),
        ("mie_fog_base_height", 100.0),
        ("sun_intensity", 0.05),
        ("god_rays_intensity", 0.0),
        ("sky_clouds_fog_up_limit", 1.0),
    ] {
        properties.set_path(key, ConfigValue::Float(value))?;
    }
    for key in [
        "use_rain",
        "use_sun",
        "use_sun_shadows",
        "use_dynamic_lens_flare",
    ] {
        properties.set_path(key, ConfigValue::Boolean(false))?;
    }
    properties.set_path(
        "mie_fog_color",
        ConfigValue::Float4([0.55, 0.58, 0.60, 1.0]),
    )?;
    properties.set_path(
        "sky_clouds_fog_color",
        ConfigValue::Float4([0.55, 0.58, 0.60, 1.0]),
    )?;
    let mut fog = scene_object(&rudnya, "volume_fog")?;
    // AABB extends beyond the 220 x 220 School minimap and all playable floors.
    fog.set_path("position", ConfigValue::Float3([21.079, 30.0, 105.098]))?;
    fog.set_path("scale", ConfigValue::Float3([160.0, 90.0, 160.0]))?;
    fog.set_path("color", ConfigValue::Float3([0.55, 0.58, 0.60]))?;
    for (key, value) in [
        ("density", 0.06),
        ("transparency_multiplier", 0.85),
        ("near_density", 0.4),
        // Native fog noise whitens the scene during iron-sight aiming in 0.26g.
        // Zero noise retains volumetric density and height falloff; playtested.
        ("noise_scale", 0.0),
        ("wave_length", 0.25),
        ("speed", 0.15),
        ("density_offset", 0.2),
        ("height_falloff_offset", 0.01),
    ] {
        fog.set_path(key, ConfigValue::Float(value))?;
    }
    let ConfigValue::Indexed(objects) = school
        .named_mut("client_objects")
        .context("client objects")?
    else {
        bail!("expected indexed client objects");
    };
    let env = objects
        .iter_mut()
        .find(|o| string(o, "game_object_type") == Some("environment"))
        .context("School environment")?;
    env.set_path("properties", properties)?;
    objects.push(fog);
    // Restore the two intentional scene edits and compare the entire project.
    let mut unchanged = school.clone();
    let ConfigValue::Indexed(objects) = unchanged.named_mut("client_objects").unwrap() else {
        unreachable!()
    };
    objects.pop();
    let original_environment = scene_object(&original, "environment")?;
    *objects
        .iter_mut()
        .find(|o| string(o, "game_object_type") == Some("environment"))
        .unwrap() = original_environment;
    ensure!(
        unchanged == original,
        "unexpected non-environment project change"
    );
    fn sort_fields(value: &mut ConfigValue) {
        match value {
            ConfigValue::Named(fields) => {
                fields.sort_by_key(|field| crc32fast::hash(field.name.as_bytes()));
                for field in fields {
                    sort_fields(&mut field.value);
                }
            }
            ConfigValue::Indexed(values) => {
                for value in values {
                    sort_fields(value);
                }
            }
            _ => {}
        }
    }
    sort_fields(&mut school);
    let bytes = BinaryConfig {
        root: school.clone(),
    }
    .to_bytes()?;
    ensure!(
        BinaryConfig::parse(&bytes)?.root == school,
        "project roundtrip changed values"
    );
    let mut files = Vec::new();
    emit(
        output,
        PROJECT,
        &bytes,
        "native-026-overcast-volume-fog",
        &mut files,
    )?;
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format_version": 1, "asset_id": "native-026-school-volume-fog",
            "target_db_sha256": HASH, "files": files,
        }))?,
    )?;
    Ok(())
}

pub fn build(
    manifest_path: &Path,
    donor_path: &Path,
    target_path: &Path,
    output: &Path,
) -> Result<Record> {
    let m: Manifest = serde_json::from_reader(fs::File::open(manifest_path)?)?;
    eprintln!("Checking donor and target fingerprints...");
    check_hash(donor_path, &m.donor_sha256)?;
    check_hash(target_path, &m.target_sha256)?;
    ensure!(!m.maps.is_empty(), "no maps requested");
    let mut ids = BTreeSet::new();
    let mut projects = BTreeSet::new();
    for map in &m.maps {
        ensure!(
            matches!(
                map.level.as_str(),
                "level_01" | "level_02" | "level_03" | "level_04" | "level_05" | "level_06"
            ),
            "unsupported source level"
        );
        ensure!(
            matches!(map.weather.as_str(), "day" | "evening" | "rain"),
            "unsupported weather"
        );
        ensure!(
            ids.insert(map.id) && projects.insert(map.donor_project()),
            "duplicate map ID or project"
        );
    }
    let mut donor = ResourceDb::open(donor_path)?;
    let mut target = ResourceDb::open(target_path)?;
    let mut dictionary = cfg(&mut target, DICTIONARY)?;
    let ConfigValue::Indexed(registered) = dictionary
        .named("maps_dict")
        .context("0.26 has no maps_dict")?
    else {
        bail!("unexpected map dictionary schema")
    };
    let mut native_map_ids = BTreeSet::new();
    for entry in registered {
        if let Some(ConfigValue::Integer(id)) = entry.named("id") {
            ensure!(
                !ids.contains(&u8::try_from(*id)?),
                "map ID {id} already used"
            );
        }
        // The server's map numbers the target's modes use (1 to 8, 88).
        if let Some(name) = string(entry, "cfg_mode_name") {
            let path = format!("gameplay/game_modes/{name}");
            if target.is_file(&path)
                && let Some(ConfigValue::Integer(map_id)) = cfg(&mut target, &path)?
                    .named("mode")
                    .and_then(|mode| mode.named("match_making_params"))
                    .and_then(|making| making.named("map_id"))
            {
                native_map_ids.insert(*map_id);
            }
        }
    }
    // A map number is one map: the variants of one source level share theirs,
    // and it is no native map's.
    let mut map_ids: BTreeMap<u8, &str> = BTreeMap::new();
    let mut name_keys: BTreeMap<&str, &str> = BTreeMap::new();
    for map in &m.maps {
        if let Some(map_id) = map.map_id {
            ensure!(
                !native_map_ids.contains(&i32::from(map_id)),
                "map number {map_id} of {} is a native map's",
                map.donor_project()
            );
            let level = *map_ids.entry(map_id).or_insert(&map.level);
            ensure!(
                level == map.level,
                "map number {map_id} is given to {level} and {}",
                map.level
            );
        }
        ensure!(
            m.maps
                .iter()
                .filter(|other| other.level == map.level)
                .all(|other| other.map_id == map.map_id),
            "the variants of {} have different map numbers",
            map.level
        );
        if let Some(name) = &map.name {
            let level = *name_keys.entry(&name.key).or_insert(&map.level);
            ensure!(
                level == map.level,
                "name key {} is given to {level} and {}",
                name.key,
                map.level
            );
        }
    }
    let mut localizations = BTreeMap::new();
    if m.maps.iter().any(|map| map.name.is_some()) {
        for language in crate::localization_026::LANGUAGES {
            let root = cfg(
                &mut target,
                &crate::content_pack::localization_path(language),
            )?;
            for map in &m.maps {
                if let Some(name) = &map.name {
                    ensure!(
                        crate::localization_026::string_of(&root, &name.key)?.is_none(),
                        "the {language} localization already has {}",
                        name.key
                    );
                }
            }
            localizations.insert(language.to_owned(), root);
        }
    }
    let mut templates = Vec::new();
    let mut target_projects = BTreeMap::new();
    for map in &m.maps {
        let project = cfg(&mut target, &map.target_project())?;
        templates.extend(
            array(
                project
                    .named("client_objects")
                    .context("target objects missing")?,
            )?
            .iter()
            .cloned(),
        );
        if let Some(ConfigValue::Named(groups)) = project.named("server_objects") {
            for group in groups {
                templates.extend(array(&group.value)?.iter().cloned());
            }
        }
        target_projects.insert(map.target_project(), project);
    }
    let mut overrides = BTreeMap::new();
    let mut collision_repairs = BTreeMap::new();
    for map in &m.maps {
        let mut project = adapt_project(
            cfg(&mut donor, &map.donor_project())?,
            &target_projects[&map.target_project()],
            map,
            &templates,
        )
        .with_context(|| format!("adapting {}", map.donor_project()))?;
        if map.native_lighting {
            let native = &target_projects[&map.target_project()];
            let offset = layout_offset(&project, native)
                .with_context(|| format!("native lighting layout for {}", map.donor_project()))?;
            let (removed, added) = adopt_native_lighting(&mut project, native, offset)
                .with_context(|| format!("native lighting for {}", map.donor_project()))?;
            eprintln!(
                "{}: {removed} donor lighting objects (probes, ambient lights, lights) replaced by the {added} of native {} moved by {offset:?}; the donor probes were baked and tuned for 0.21 (diffuse multipliers up to 7.5; 0.26 has no sun light object)",
                map.donor_project(),
                map.target_project()
            );
        }
        adapt_collisions(
            &mut project,
            &donor,
            &m.missing_collision_resources,
            &map.donor_project(),
            &mut collision_repairs,
        )?;
        rewrite(&mut project, &m.reference_replacements);
        overrides.insert(
            map.donor_project(),
            BinaryConfig { root: project }.to_bytes()?,
        );
    }
    let forced: BTreeSet<String> = donor
        .paths()
        .filter(|p| {
            donor.is_file(p)
                && (p.starts_with("textures/")
                    || p.starts_with("material_instances/")
                    || p.ends_with(".veg"))
        })
        .map(str::to_owned)
        .chain(
            donor
                .paths()
                .filter_map(|p| p.split_once(".model/").map(|(p, _)| format!("{p}.model"))),
        )
        .collect();
    let mut forced = forced;
    eprintln!("Resolving map dependencies...");
    let first = crate::dependency::scan_with_forced_donor_and_overrides(
        &mut donor,
        &mut target,
        projects.clone(),
        forced.clone(),
        &overrides,
    )?;
    ensure!(
        first.unresolved_references.is_empty(),
        "unresolved dependencies: {:?}",
        first.unresolved_references
    );
    // Convert the materials first: a material that takes the native finish
    // binds native maps, which must resolve to the target's textures.
    let catalog = crate::material_conversion::MaterialCatalog::load(&mut target)?;
    let mut scan_overrides = overrides.clone();
    let mut converted_roots = BTreeMap::new();
    let mut native_bound = BTreeSet::new();
    let mut donor_bound = BTreeSet::new();
    let mut native_finish_materials = BTreeSet::new();
    for path in first.donor_files.iter().filter(|p| is_material(p)) {
        let bytes = donor.read(path)?;
        let converted = convert_material(&m, &catalog, &donor, &mut target, path, &bytes)?;
        // The native finish maps and the template's own textures that the
        // donor material does not bind (a template's unused
        // `normal_waves_texture`) are the target's, as the template has them,
        // unless a donor slot binds the same name: one name is one texture,
        // and the donor's art stays (0.26g's `flora/bark_rot_aref_vc` reads
        // the bark colour map `flora/bark_rot` as gloss).
        let donor_textures: BTreeSet<String> =
            crate::material_finish::material_textures(&BinaryConfig::parse(&bytes)?.root)
                .iter()
                .filter_map(|name| crate::material_conversion::canonical_texture_reference(name).ok())
                .collect();
        for (slot, name) in crate::material_finish::material_texture_slots(&converted.root) {
            let Ok(name) = crate::material_conversion::canonical_texture_reference(&name) else {
                continue;
            };
            let native_slot = converted.native_finish
                && crate::material_finish::is_finish_slot(&slot)
                && converted.native_maps.contains(&name);
            if !native_slot && donor_textures.contains(&name) {
                donor_bound.insert(name);
            } else {
                native_bound.insert(name);
            }
        }
        if converted.native_finish {
            native_finish_materials.insert(path.clone());
        }
        scan_overrides.insert(
            path.clone(),
            BinaryConfig {
                root: converted.root.clone(),
            }
            .to_bytes()?,
        );
        converted_roots.insert(path.clone(), converted);
    }
    // Retail can name a map it lacks (0.26g's `vehicle/truck/uaz/uaz` names
    // 0.21d's `editor/default_spec`): that one stays the donor's.
    let native_bound: BTreeSet<String> = native_bound
        .into_iter()
        .filter(|name| {
            !donor_bound.contains(name) && target.is_file(&format!("textures/{name}.dds"))
        })
        .collect();
    for path in m.material_finishes.keys() {
        ensure!(
            converted_roots.contains_key(path),
            "material_finishes names {path}, which the maps do not use"
        );
    }
    let (native_textures, native_materials) = native_equivalents(
        &mut donor,
        &mut target,
        &first.donor_files,
        &converted_roots,
        &native_bound,
    )?;
    eprintln!(
        "{} materials take the native finish; {} donor textures and {} converted materials are the target's own",
        native_finish_materials.len(),
        native_textures.len(),
        native_materials.len()
    );
    for name in &native_bound {
        for extension in [".dds", ".options"] {
            forced.remove(&format!("textures/{name}{extension}"));
        }
    }
    for path in native_textures.keys() {
        forced.remove(path);
        forced.remove(&texture_options(path));
    }
    for path in &native_materials {
        forced.remove(path);
    }
    let plan = crate::dependency::scan_with_forced_donor_and_overrides(
        &mut donor,
        &mut target,
        projects.clone(),
        forced,
        &scan_overrides,
    )?;
    ensure!(
        plan.unresolved_references.is_empty(),
        "unresolved dependencies: {:?}",
        plan.unresolved_references
    );
    // Only map assets may be imported. Runtime gameplay, shaders and character
    // animation remain native; an unsupported donor-only dependency must stop.
    for path in &plan.donor_files {
        ensure!(
            [
                "models/",
                "textures/",
                "material_instances/",
                "projects/",
                "sounds/",
                "particles/"
            ]
            .iter()
            .any(|p| path.starts_with(p)),
            "unsupported donor dependency {path}"
        );
    }
    let imported_models: BTreeSet<_> = plan
        .donor_files
        .iter()
        .filter_map(|p| {
            p.split_once(".model/")
                .map(|(model, _)| format!("{model}.model/"))
        })
        .collect();
    for prefix in imported_models {
        for path in donor
            .paths_with_prefix(&prefix)
            .filter(|p| donor.is_file(p))
        {
            ensure!(
                plan.donor_files.contains(path),
                "incomplete donor model: missing {path}"
            );
        }
    }
    let mut substitutions = replacements(&plan.donor_files);
    for map in &m.maps {
        substitutions.insert(map.donor_project(), map.output_project());
    }
    // Grass loading prepends the project resource directory to filenames.
    // The namespace preserves each project's directory layout, so relative
    // vegetation paths must remain relative after relocation.
    eprintln!(
        "Importing {} donor files; sharing {} target files...",
        plan.donor_files.len(),
        plan.target_files.len()
    );
    // Missing render/collision assets must fail even when generic string
    // scanning mistakes a scene label for a non-resource identifier.
    for map in &m.maps {
        let project = BinaryConfig::parse(&overrides[&map.donor_project()])?;
        for group in ["render_visuals", "collision_objects"] {
            for object in array(
                project
                    .root
                    .named(group)
                    .context("missing model instances")?,
            )? {
                let reference =
                    string(object, "lib_name").context("model instance lacks lib_name")?;
                let normalized = reference.replace('\\', "/");
                let normalized = normalized.trim_start_matches("resources/");
                let model = if let Some((model, _)) = normalized.split_once(".model/") {
                    format!("{model}.model")
                } else {
                    format!("models/{normalized}.model")
                };
                let imported = if group == "render_visuals" {
                    plan.donor_files.contains(&format!("{model}/settings"))
                } else {
                    plan.donor_files
                        .range(format!("{model}/")..)
                        .next()
                        .is_some_and(|p| p.starts_with(&format!("{model}/")))
                };
                ensure!(imported, "source model not imported: {reference}");
            }
        }
    }
    let particle_defaults = particle_defaults(&mut target)?;
    let mut adapted_particles = BTreeSet::new();
    crate::weapon::prepare_empty_output(output)?;
    let mut files = Vec::new();
    let mut converted_materials = BTreeMap::new();
    let mut repaired_model_settings = BTreeSet::new();
    for path in &plan.donor_files {
        let mut bytes = if let Some(b) = overrides.get(path) {
            b.clone()
        } else {
            donor.read(path)?
        };
        if path.ends_with(".veg") {
            // The client never reads the editor's paint mask; it only
            // takes managed resource memory (crate::vegetation).
            bytes = crate::vegetation::drop_paint_mask(&bytes)
                .with_context(|| format!("dropping the paint mask of {path}"))?;
        }
        if is_material(path) {
            let converted = match converted_roots.remove(path) {
                Some(converted) => converted,
                None => convert_material(&m, &catalog, &donor, &mut target, path, &bytes)?,
            };
            bytes = BinaryConfig {
                root: converted.root,
            }
            .to_bytes()?;
            converted_materials.insert(path.clone(), converted.template_path);
        }
        if is_config_resource(path) {
            let mut c =
                BinaryConfig::parse(&bytes).with_context(|| format!("decoding config {path}"))?;
            if path.ends_with(".particle") && adapt_particle(&mut c.root, &particle_defaults)? {
                adapted_particles.insert(path.clone());
            }
            if let Some(model) = path.strip_suffix(".model/settings") {
                let prefix = format!("{model}.model/render/");
                let actual: BTreeSet<_> = plan
                    .donor_files
                    .range(prefix.clone()..)
                    .take_while(|p| p.starts_with(&prefix))
                    .filter_map(|p| {
                        p.strip_prefix(&prefix)?
                            .strip_suffix("/converted_model")
                            .map(str::to_owned)
                    })
                    .collect();
                if !actual.is_empty()
                    && repair_settings(
                        &mut c.root,
                        &actual,
                        m.surface_renames.get(path).unwrap_or(&BTreeMap::new()),
                    )
                    .with_context(|| format!("repairing {path}"))?
                {
                    repaired_model_settings.insert(path.clone());
                }
            }
            if m.lod0_promotions.contains(path) {
                let old = c
                    .root
                    .named("lod_hierrarchy")
                    .context("promotion has no LODs")?;
                let level = old
                    .named("LOD1")
                    .and_then(|l| l.named("surfaces"))
                    .context("promotion has no LOD1 surfaces")?
                    .clone();
                ensure!(!array(&level)?.is_empty(), "cannot promote an empty LOD");
                c.root.set_path(
                    "lod_hierrarchy",
                    ConfigValue::Named(vec![
                        NamedValue {
                            name: "LOD0".into(),
                            value: ConfigValue::Named(vec![NamedValue {
                                name: "surfaces".into(),
                                value: level,
                            }]),
                        },
                        NamedValue {
                            name: "unassigned".into(),
                            value: ConfigValue::Named(Vec::new()),
                        },
                    ]),
                )?;
                c.root
                    .set_path("lod_switching.default_params", ConfigValue::Boolean(true))?;
                repaired_model_settings.insert(path.clone());
            }
            rewrite(&mut c.root, &m.reference_replacements);
            rewrite_inner(&mut c.root, &substitutions, true, None);
            bytes = c.to_bytes()?;
        }
        let dest = substitutions.get(path).context("missing output mapping")?;
        ensure!(
            !target.contains(dest),
            "would overwrite native resource {dest}"
        );
        emit(output, dest, &bytes, "0.21d+0.26g-adapter", &mut files)?;
    }
    for map in &m.maps {
        let mode_name = format!("v021d_{}", map.mode());
        let mut mode = cfg(&mut target, &format!("gameplay/game_modes/{}", map.mode()))?;
        mode.set_path(
            "mode.name",
            ConfigValue::String(format!("v021d_{}_2", map.level)),
        )?;
        mode.set_path(
            "mode.match_making_params.map_name",
            ConfigValue::String(map.project_name()),
        )?;
        if let Some(map_id) = map.map_id {
            mode.set_path(
                "mode.match_making_params.map_id",
                ConfigValue::Integer(map_id.into()),
            )?;
        }
        if let Some(name) = &map.name {
            mode.set_path("mode.ui.map_name", ConfigValue::String(name.key.clone()))?;
            for language in crate::localization_026::LANGUAGES {
                let localization = localizations
                    .get_mut(language)
                    .with_context(|| format!("0.26 has no {language} localization"))?;
                crate::localization_026::set_string(localization, &name.key, name.text(language)?)?;
            }
        }
        let path = format!("gameplay/game_modes/{mode_name}");
        ensure!(!target.contains(&path), "mode already exists");
        emit(
            output,
            &path,
            &BinaryConfig { root: mode }.to_bytes()?,
            "0.26g-mode-template",
            &mut files,
        )?;
        let ConfigValue::Indexed(entries) = dictionary.named_mut("maps_dict").unwrap() else {
            unreachable!()
        };
        entries.push(ConfigValue::Named(vec![
            NamedValue {
                name: "id".into(),
                value: ConfigValue::Integer(map.id.into()),
            },
            NamedValue {
                name: "cfg_mode_name".into(),
                value: ConfigValue::String(mode_name),
            },
        ]));
    }
    emit(
        output,
        DICTIONARY,
        &BinaryConfig { root: dictionary }.to_bytes()?,
        "0.26g+additive-map-registration",
        &mut files,
    )?;
    if m.maps.iter().any(|map| map.name.is_some()) {
        for (language, root) in localizations {
            emit(
                output,
                &crate::content_pack::localization_path(&language),
                &BinaryConfig { root }.to_bytes()?,
                "0.26g+additive-map-names",
                &mut files,
            )?;
        }
    }
    let record = Record {
        format_version: 1,
        asset_id: "retail-021d-maps-to-026g".into(),
        donor_build: "0.21d-2010".into(),
        target_build: "0.26g0-2777".into(),
        target_db_sha256: m.target_sha256,
        maps: m.maps,
        files,
        converted_materials,
        shared_dependencies: plan.target_files,
        repaired_model_settings,
        adapted_particles,
        collision_repairs,
        native_finish_materials,
        native_textures,
        native_materials,
    };
    serde_json::to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_lighting_replaces_only_lighting_objects() {
        let object = |kind: &str, name: &str| {
            ConfigValue::Named(vec![
                NamedValue { name: "game_object_type".into(), value: ConfigValue::String(kind.into()) },
                NamedValue { name: "name".into(), value: ConfigValue::String(name.into()) },
            ])
        };
        let objects = |list: Vec<ConfigValue>| {
            ConfigValue::Named(vec![NamedValue {
                name: "client_objects".into(),
                value: ConfigValue::Indexed(list),
            }])
        };
        let mut project = objects(vec![
            object("environment_probe", "old probe"),
            object("light", "sun"),
            object("ambient_light", "old fill"),
            object("respawn_point", "spawn"),
            object("collision_geometry", "zone"),
        ]);
        let native = objects(vec![
            object("environment_probe", "indoor"),
            object("environment_probe", "global"),
            object("light", "spot_light"),
            object("ambient_light", "fill"),
            object("particle", "smoke"),
        ]);
        assert_eq!(adopt_native_lighting(&mut project, &native, [0., -1.5, 0.]).unwrap(), (3, 4));
        let names: Vec<_> = array(project.named("client_objects").unwrap())
            .unwrap()
            .iter()
            .map(|o| string(o, "name").unwrap().to_owned())
            .collect();
        assert_eq!(names, ["spawn", "zone", "indoor", "global", "spot_light", "fill"]);
        assert!(adopt_native_lighting(&mut project, &objects(vec![object("light", "x")]), [0.; 3]).is_err());
        // Positions follow the layout offset.
        let mut probe = object("environment_probe", "p");
        probe.set_path("position", ConfigValue::Float3([1., 10., 2.])).unwrap_or_else(|_| {
            if let ConfigValue::Named(v) = &mut probe {
                v.push(NamedValue { name: "position".into(), value: ConfigValue::Float3([1., 10., 2.]) });
            }
        });
        let mut target = objects(vec![]);
        adopt_native_lighting(&mut target, &objects(vec![probe]), [0., -7.28, 0.]).unwrap();
        let moved = &array(target.named("client_objects").unwrap()).unwrap()[0];
        assert_eq!(moved.named("position"), Some(&ConfigValue::Float3([1., 10. - 7.28, 2.])));
    }

    #[test]
    fn layout_offset_finds_the_common_translation() {
        let visual = |name: &str, at: [f32; 3]| {
            ConfigValue::Named(vec![
                NamedValue { name: "lib_name".into(), value: ConfigValue::String(name.into()) },
                NamedValue { name: "position".into(), value: ConfigValue::Float3(at) },
            ])
        };
        let project = |list: Vec<ConfigValue>| {
            ConfigValue::Named(vec![NamedValue { name: "render_visuals".into(), value: ConfigValue::Indexed(list) }])
        };
        let donor = project((0..12).map(|i| visual(&format!("m{i}"), [i as f32, 0., 0.])).collect());
        let mut moved: Vec<_> = (0..11).map(|i| visual(&format!("m{i}"), [i as f32, 7.28, 0.])).collect();
        moved.push(visual("m11", [50., 0., 0.]));
        assert_eq!(layout_offset(&donor, &project(moved)).unwrap(), [0., -7.28, 0.]);
        let unrelated = project((0..12).map(|i| visual(&format!("m{i}"), [i as f32 * 3., 1., 0.])).collect());
        assert!(layout_offset(&donor, &unrelated).is_err());
    }
    #[test]
    fn shared_particle_and_texture_names_do_not_follow_a_material_alias() {
        let paths = BTreeSet::from(["material_instances/fx/lights/oil_lamp.material".into()]);
        let mut scene = named(vec![
            ("game_object_type", ConfigValue::String("particle".into())),
            ("lib_name", ConfigValue::String("fx/lights/oil_lamp".into())),
            (
                "texture_diffuse",
                named(vec![(
                    "value",
                    ConfigValue::String("fx/lights/oil_lamp".into()),
                )]),
            ),
            (
                "material_name",
                ConfigValue::String("fx/lights/oil_lamp".into()),
            ),
        ]);
        rewrite_inner(&mut scene, &replacements(&paths), true, None);
        assert_eq!(string(&scene, "lib_name"), Some("fx/lights/oil_lamp"));
        assert_eq!(
            string(scene.named("texture_diffuse").unwrap(), "value"),
            Some("fx/lights/oil_lamp")
        );
        assert_eq!(
            string(&scene, "material_name"),
            Some("ported/v021d/fx/lights/oil_lamp")
        );
        let paths = BTreeSet::from([
            "particles/fx/lights/oil_lamp.particle".into(),
            "textures/fx/lights/oil_lamp.dds".into(),
        ]);
        rewrite_inner(&mut scene, &replacements(&paths), true, None);
        assert_eq!(
            string(&scene, "lib_name"),
            Some("ported/v021d/fx/lights/oil_lamp")
        );
        assert_eq!(
            string(scene.named("texture_diffuse").unwrap(), "value"),
            Some("ported/v021d/fx/lights/oil_lamp")
        );
    }

    fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            fields
                .into_iter()
                .map(|(k, v)| NamedValue {
                    name: k.into(),
                    value: v,
                })
                .collect(),
        )
    }
    #[test]
    fn imported_project_retains_hud_key_without_losing_resource_namespace() {
        let collectors = (0..2)
            .map(|team| {
                named(vec![
                    (
                        "game_object_type",
                        ConfigValue::String("victory_items_container".into()),
                    ),
                    ("team", ConfigValue::Integer(team)),
                ])
            })
            .collect();
        let donor = named(vec![
            ("project_name", ConfigValue::String("level_02".into())),
            ("client_objects", ConfigValue::Indexed(Vec::new())),
            (
                "server_objects",
                named(vec![(
                    "victory_items_containers",
                    ConfigValue::Indexed(collectors),
                )]),
            ),
        ]);
        let map = Map {
            id: 165,
            level: "level_02".into(),
            weather: "rain".into(),
            native_lighting: false,
            map_id: None,
            name: None,
        };
        let output = adapt_project(donor.clone(), &donor, &map, &[]).unwrap();
        assert_eq!(string(&output, "project_name"), Some("level_02"));
        assert_eq!(
            map.output_project(),
            "projects/v021d_level_02/rain_2/client_project"
        );
        assert_eq!(map.project_name(), "v021d_level_02/rain_2");
    }
    #[test]
    fn new_particle_fields_do_not_replace_authored_properties() {
        let kind = "particle_action_random_velocity_type";
        let mut node = named(vec![
            ("particle_entity_type", ConfigValue::String(kind.into())),
            (
                "properties",
                named(vec![("Velocity", ConfigValue::Float(7.0))]),
            ),
        ]);
        let defaults = BTreeMap::from([
            (
                format!("{kind}/UseAnimatedMultiplier"),
                ConfigValue::Boolean(false),
            ),
            (format!("{kind}/AnimatedVelocityMultiplier"), named(vec![])),
        ]);
        assert!(adapt_particle(&mut node, &defaults).unwrap());
        let p = node.named("properties").unwrap();
        assert_eq!(p.named("Velocity"), Some(&ConfigValue::Float(7.0)));
        assert_eq!(
            p.named("UseAnimatedMultiplier"),
            Some(&ConfigValue::Boolean(false))
        );
        assert!(!adapt_particle(&mut node, &defaults).unwrap());
    }

    #[test]
    fn collision_fallback_retains_scale_and_missing_geometry_needs_review() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        let file = root.join("models/one.model/collision/vertices");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"geometry").unwrap();
        let dbpath = temp.path().join("source.db");
        crate::archive::write_folder(&root, &dbpath).unwrap();
        let db = ResourceDb::open(dbpath).unwrap();
        let make = |r: &str| named(vec![("lib_name", ConfigValue::String(r.into()))]);
        let project = || {
            named(vec![(
                "collision_objects",
                ConfigValue::Indexed(vec![
                    make("resources/models/one.model/walker_collision#[2][2][2]"),
                    make("resources/models/two.model/collision"),
                ]),
            )])
        };
        assert!(
            adapt_collisions(
                &mut project(),
                &db,
                &BTreeSet::new(),
                "map",
                &mut BTreeMap::new()
            )
            .is_err()
        );
        let mut p = project();
        let mut repairs = BTreeMap::new();
        adapt_collisions(
            &mut p,
            &db,
            &BTreeSet::from(["models/two.model/collision".into()]),
            "map",
            &mut repairs,
        )
        .unwrap();
        let objects = array(p.named("collision_objects").unwrap()).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(
            string(&objects[0], "lib_name"),
            Some("resources/models/one.model/collision#[2][2][2]")
        );
        assert_eq!(repairs.len(), 2);
    }

    #[test]
    fn empty_collision_folder_is_omitted_not_replaced_by_the_other_kind() {
        // Retail 0.21d level/03/anomal_tree_0X_ (RLS Vostok and Mamayev
        // Kurgan), soviet_comp_01 and barrel_stove_01_fire: one kind is an
        // empty folder node, the other has a mesh. Each instance must keep
        // exactly the source's shape set: nothing for the empty kind. A
        // reference to a folder the donor lacks altogether still takes the
        // other kind, as before.
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        for (model, full, empty) in [
            ("tree", "collision", Some("walker_collision")),
            ("comp", "walker_collision", Some("collision")),
            ("absent", "collision", None),
        ] {
            let file = root.join(format!("models/{model}.model/{full}/vertices"));
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"geometry").unwrap();
            if let Some(empty) = empty {
                fs::create_dir_all(root.join(format!("models/{model}.model/{empty}"))).unwrap();
            }
        }
        let dbpath = temp.path().join("source.db");
        crate::archive::write_folder(&root, &dbpath).unwrap();
        let db = ResourceDb::open(dbpath).unwrap();
        assert!(db.is_empty_folder("models/tree.model/walker_collision"));
        assert!(!db.contains("models/absent.model/walker_collision"));
        let make = |r: &str| named(vec![("lib_name", ConfigValue::String(r.into()))]);
        let mut project = named(vec![(
            "collision_objects",
            ConfigValue::Indexed(vec![
                make("resources/models/tree.model/collision"),
                make("resources/models/tree.model/walker_collision#[1.67][1.67][1.67]"),
                make("resources/models/comp.model/walker_collision"),
                make("resources/models/comp.model/collision"),
                make("resources/models/absent.model/walker_collision#[2][2][2]"),
            ]),
        )]);
        let mut repairs = BTreeMap::new();
        adapt_collisions(&mut project, &db, &BTreeSet::new(), "map", &mut repairs).unwrap();
        let kept: Vec<_> = array(project.named("collision_objects").unwrap())
            .unwrap()
            .iter()
            .map(|o| string(o, "lib_name").unwrap().to_owned())
            .collect();
        assert_eq!(
            kept,
            [
                "resources/models/tree.model/collision",
                "resources/models/comp.model/walker_collision",
                "resources/models/absent.model/collision#[2][2][2]",
            ]
        );
        assert_eq!(repairs.len(), 3);
        let omitted: Vec<_> = repairs
            .iter()
            .filter(|(_, v)| v.starts_with("omitted: empty"))
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(
            omitted,
            [
                "map -> resources/models/comp.model/collision",
                "map -> resources/models/tree.model/walker_collision#[1.67][1.67][1.67]",
            ]
        );
    }

    #[test]
    fn a_collision_folder_whose_files_share_another_kinds_bytes_is_kept() {
        // Retail 0.21d stores level/03/anomal_tree_01_'s walker_collision as
        // the same bytes as its collision (one copy, two nodes), and
        // barrel_stove_01_fire's collision as its hq_collision's. Both kinds
        // are there: every instance keeps its own kind (an archive reader
        // that missed shared nodes made the walker instances bullet ones).
        use resources_db::write::{Stored, write_to};
        let files: BTreeMap<String, Vec<u8>> = [
            ("models/tree.model/collision/vertices", b"tree vertices".as_slice()),
            ("models/tree.model/collision/indices", b"tree indices"),
            ("models/tree.model/walker_collision/vertices", b"tree vertices"),
            ("models/tree.model/walker_collision/indices", b"tree indices"),
        ]
        .into_iter()
        .map(|(path, bytes)| (path.to_owned(), bytes.to_vec()))
        .collect();
        let table: BTreeMap<String, Stored> = files
            .iter()
            .map(|(path, bytes)| {
                let kind = if path.ends_with("vertices") { 1 } else { 2 };
                let stored = Stored {
                    size: u32::try_from(bytes.len()).unwrap(),
                    hash: resources_db::crc32(bytes),
                    share: Some(kind),
                };
                (path.clone(), stored)
            })
            .collect();
        let temp = tempfile::TempDir::new().unwrap();
        let dbpath = temp.path().join("source.db");
        let mut out = fs::File::create(&dbpath).unwrap();
        write_to(&mut out, &table, &BTreeSet::new(), |path| Ok(files[path].clone())).unwrap();
        drop(out);
        let db = ResourceDb::open(dbpath).unwrap();
        assert!(db.is_file("models/tree.model/walker_collision/vertices"));
        let references = [
            "resources/models/tree.model/collision",
            "resources/models/tree.model/walker_collision#[1.67][1.67][1.67]",
        ];
        let make = |r: &str| named(vec![("lib_name", ConfigValue::String(r.into()))]);
        let mut project = named(vec![(
            "collision_objects",
            ConfigValue::Indexed(references.iter().map(|r| make(r)).collect()),
        )]);
        let mut repairs = BTreeMap::new();
        adapt_collisions(&mut project, &db, &BTreeSet::new(), "map", &mut repairs).unwrap();
        let kept: Vec<_> = array(project.named("collision_objects").unwrap())
            .unwrap()
            .iter()
            .map(|o| string(o, "lib_name").unwrap().to_owned())
            .collect();
        assert_eq!(kept, references);
        assert!(repairs.is_empty(), "{repairs:?}");
    }

    #[test]
    fn visual_name_is_the_model_resource_not_an_instance_label() {
        let collector = |team| {
            named(vec![
                (
                    "game_object_type",
                    ConfigValue::String("victory_items_container".into()),
                ),
                ("team", ConfigValue::Integer(team)),
            ])
        };
        let donor = named(vec![
            ("project_name", ConfigValue::String("level_03".into())),
            ("client_objects", ConfigValue::Indexed(Vec::new())),
            (
                "server_objects",
                named(vec![(
                    "victory_items_containers",
                    ConfigValue::Indexed(vec![collector(0), collector(1)]),
                )]),
            ),
            (
                "render_visuals",
                ConfigValue::Indexed(vec![named(vec![(
                    "lib_name",
                    ConfigValue::String("level/03/ground".into()),
                )])]),
            ),
        ]);
        let map = Map {
            id: 166,
            level: "level_03".into(),
            weather: "day".into(),
            native_lighting: false,
            map_id: None,
            name: None,
        };
        let converted = adapt_project(donor.clone(), &donor, &map, &[]).unwrap();
        let visual = &array(converted.named("render_visuals").unwrap()).unwrap()[0];
        assert_eq!(string(visual, "visual_name"), Some("level/03/ground"));
    }

    #[test]
    fn collision_and_vegetation_payloads_are_never_decoded_as_configs() {
        for p in [
            "models/a.model/collision/indices",
            "models/a.model/render/a/converted_model",
            "projects/a/vegetation/grass.veg",
        ] {
            assert!(!is_config_resource(p));
        }
        assert!(is_config_resource("models/a.model/settings"));
    }

    #[test]
    fn stale_geometry_references_are_pruned_without_reassigning_lods() {
        let list = |names: &[&str]| {
            ConfigValue::Indexed(
                names
                    .iter()
                    .map(|s| ConfigValue::String((*s).into()))
                    .collect(),
            )
        };
        let mut c = named(vec![
            (
                "material_settings",
                named(vec![
                    ("body", named(vec![])),
                    ("missing", named(vec![])),
                    ("distant", named(vec![])),
                ]),
            ),
            (
                "lod_hierrarchy",
                named(vec![
                    (
                        "LOD0",
                        named(vec![("surfaces", list(&["body", "missing"]))]),
                    ),
                    ("LOD2", named(vec![("surfaces", list(&["distant"]))])),
                ]),
            ),
        ]);
        let actual = BTreeSet::from(["body".into(), "distant".into()]);
        assert!(repair_settings(&mut c, &actual, &BTreeMap::new()).unwrap());
        let lod = c.named("lod_hierrarchy").unwrap();
        assert_eq!(
            lod.named("LOD0").unwrap().named("surfaces"),
            Some(&list(&["body"]))
        );
        assert_eq!(
            lod.named("LOD2").unwrap().named("surfaces"),
            Some(&list(&["distant"]))
        );
        let unknown = BTreeSet::from(["unreviewed".into()]);
        assert!(repair_settings(&mut c, &unknown, &BTreeMap::new()).is_err());
    }

    #[test]
    fn new_object_groups_are_empty_not_copied_from_template() {
        let mut donor = named(vec![(
            "old",
            ConfigValue::Indexed(vec![ConfigValue::Integer(7)]),
        )]);
        let target = named(vec![
            ("old", ConfigValue::Indexed(vec![ConfigValue::Integer(9)])),
            ("new", ConfigValue::Indexed(vec![ConfigValue::Integer(99)])),
        ]);
        fill_missing_empty_groups(&mut donor, &target);
        assert_eq!(donor.named("new"), Some(&ConfigValue::Indexed(Vec::new())));
        assert_eq!(
            donor.named("old"),
            Some(&ConfigValue::Indexed(vec![ConfigValue::Integer(7)]))
        );
    }
    #[test]
    fn battery_spawn_uses_its_own_transform() {
        let make = |x| {
            named(vec![
                (
                    "game_object_type",
                    ConfigValue::String("victory_item_spawner".into()),
                ),
                ("position", ConfigValue::Float3([x, 2., 3.])),
                ("rotation", ConfigValue::Float3([0., 1., 0.])),
            ])
        };
        let donor = make(10.);
        let mut template = make(999.);
        template
            .set_path(
                "spawned_item",
                named(vec![
                    ("position", ConfigValue::Float3([999.; 3])),
                    ("rotation", ConfigValue::Float3([0.; 3])),
                ]),
            )
            .unwrap();
        let mut objects = ConfigValue::Indexed(vec![donor]);
        adapt_objects(&mut objects, &[template]).unwrap();
        let o = &array(&objects).unwrap()[0];
        assert_eq!(
            o.named("spawned_item").unwrap().named("position"),
            o.named("position")
        );
    }

    #[test]
    fn the_two_map_release_names_its_maps_and_gives_each_its_own_number() {
        // Skhid revision 17: RLS Vostok and Mamayev Kurgan of 0.21d, by day,
        // numbered 10 and 11 (not the native 3 and 6), each named in all six
        // languages as "<the language's name> (v0.21)".
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ports/survarium-021d-two-maps-to-026g.json");
        let m: Manifest = serde_json::from_reader(fs::File::open(path).unwrap()).unwrap();
        let summary: Vec<_> = m
            .maps
            .iter()
            .map(|map| {
                (
                    map.id,
                    map.level.as_str(),
                    map.weather.as_str(),
                    map.native_lighting,
                    map.map_id,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (166, "level_03", "day", true, Some(10)),
                (175, "level_06", "day", true, Some(11))
            ]
        );
        for map in &m.maps {
            let name = map.name.as_ref().unwrap();
            assert_eq!(name.key, format!("st_level_{}_name", map.map_id.unwrap()));
            for language in crate::localization_026::LANGUAGES {
                let text = name.text(language).unwrap();
                let place = text.strip_suffix(" (v0.21)").unwrap();
                assert!(!place.is_empty() && !place.contains("0.21"), "{text}");
            }
        }
        assert_eq!(m.maps[0].name.as_ref().unwrap().english, "RLS Vostok (v0.21)");
        assert_eq!(m.maps[1].name.as_ref().unwrap().english, "Mamayev Kurgan (v0.21)");
    }

    #[test]
    fn relocation_covers_render_collision_material_and_texture_references() {
        let paths = BTreeSet::from([
            "models/item/quest/container.model/settings".into(),
            "textures/a.dds".into(),
            "material_instances/a.material".into(),
        ]);
        let swaps = replacements(&paths);
        for (before, after) in [
            ("item/quest/container", "ported/v021d/item/quest/container"),
            (
                "resources/models/item/quest/container.model/collision#[1,1,1]",
                "resources/models/ported/v021d/item/quest/container.model/collision#[1,1,1]",
            ),
            ("a", "ported/v021d/a"),
        ] {
            let mut v = ConfigValue::String(before.into());
            rewrite(&mut v, &swaps);
            assert_eq!(v, ConfigValue::String(after.into()));
        }
    }
}
