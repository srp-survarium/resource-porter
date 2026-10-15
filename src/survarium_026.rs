// SPDX-License-Identifier: GPL-3.0-or-later
//! Add older retail maps to 0.26g without replacing its maps or shaders.
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

mod early;
pub mod fill;
pub use early::{collector_visual, repair_chemical_collector_zone};

const DICTIONARY: &str = "gameplay/db_static_dictionaries";
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum SourceBuild {
    #[serde(rename = "0.10b")]
    V010b,
    #[serde(rename = "0.11a")]
    V011a,
    #[serde(rename = "0.11e")]
    V011e,
    #[serde(rename = "0.20e")]
    V020e,
    #[default]
    #[serde(rename = "0.21d")]
    V021d,
}

impl SourceBuild {
    fn prefix(self) -> &'static str {
        match self {
            Self::V010b => "v010b",
            Self::V011a => "v011a",
            Self::V011e => "v011e",
            Self::V020e => "v020e",
            Self::V021d => "v021d",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::V010b => "0.10b-802",
            Self::V011a => "0.11a-816",
            Self::V011e => "0.11e-884",
            Self::V020e => "0.20e-1916",
            Self::V021d => "0.21d-2010",
        }
    }
    fn early(self) -> bool {
        matches!(self, Self::V010b | Self::V011a | Self::V011e)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub source_build: SourceBuild,
    pub donor_sha256: String,
    pub target_sha256: String,
    #[serde(default)]
    pub server_project_fallback: Option<ServerProjectFallback>,
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
    /// Surfaces audited to use repeating UVs; integer island offsets preserve sampling.
    #[serde(default)]
    pub wrapped_uv_surfaces: BTreeSet<String>,
    /// Donor collision directories replaced by the developers' re-authored
    /// collision of the same model in the target build, pinned by both
    /// primitive payload hashes. Used only for audited authored defects.
    #[serde(default)]
    pub collision_successors: BTreeMap<String, CollisionSuccessor>,
    /// Reviewed game-material bindings: model settings path -> surface -> material.
    #[serde(default)]
    pub game_material_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// Audited replacements of individual collision primitives, keyed by the
    /// donor collision directory and pinned by its primitive payload hash.
    #[serde(default)]
    pub collision_primitive_overrides: BTreeMap<String, PrimitiveOverrides>,
    /// Donor material switch restorations observed loading at runtime, for
    /// cases where the static variant proof is too conservative. Each entry
    /// is pinned to the shipped shader catalogue the loads were observed with.
    #[serde(default)]
    pub runtime_proven_switches: BTreeMap<String, RuntimeProvenSwitches>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProvenSwitches {
    pub shader_catalog_sha256: String,
    pub switches: BTreeMap<String, i64>,
    /// Sessions or captures that loaded the restored material without a
    /// missing-shader error.
    pub evidence: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveOverrides {
    pub donor_primitives_sha256: String,
    pub reason: String,
    pub primitives: Vec<PrimitiveOverride>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveOverride {
    pub index: usize,
    pub from: PrimitiveTransform,
    pub to: PrimitiveTransform,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveTransform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSuccessor {
    pub donor_primitives_sha256: String,
    pub target_primitives_sha256: String,
    pub reason: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerProjectFallback {
    pub source_build: SourceBuild,
    pub donor_sha256: String,
    /// Only these missing server projects may be read, each pinned by payload hash.
    pub projects: BTreeMap<String, String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Map {
    pub id: u8,
    pub level: String,
    pub weather: String,
    /// Look-defining environment choices for early donors. Bug fixes apply
    /// to every look; only the choices listed on `EnvironmentLook` differ.
    #[serde(default, skip_serializing_if = "EnvironmentLook::is_source")]
    pub environment_look: EnvironmentLook,
}

/// Named environment looks. `source` follows the donor; the legacy look
/// reproduces the earlier 0.10b-atmosphere candidate (snapshot
/// survarium-look-variants/2026-10-15-legacy-010b-atmosphere): Rayleigh haze
/// starts at the native template's near distance. Its LUT, geometry fog,
/// tone curve and god-ray choices live in the matching preset variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentLook {
    #[default]
    Source,
    Legacy010bAtmosphere,
}
impl EnvironmentLook {
    fn is_source(&self) -> bool {
        *self == Self::Source
    }
}
impl Map {
    fn donor_project(&self, source: SourceBuild) -> String {
        if source.early() {
            let suffix = match self.weather.as_str() {
                "day" => "",
                "evening" => "_evn",
                "rain" => "_rain",
                _ => unreachable!(),
            };
            format!("projects/{}{suffix}/client_project", self.level)
        } else {
            format!("projects/{}/{}/client_project", self.level, self.weather)
        }
    }
    fn target_project(&self) -> String {
        format!("projects/{}/{}_2/client_project", self.level, self.weather)
    }
    fn project_name(&self, source: SourceBuild) -> String {
        let root = if source.early() {
            self.donor_project(source)
                .trim_start_matches("projects/")
                .trim_end_matches("/client_project")
                .to_owned()
        } else {
            self.level.clone()
        };
        format!("{}_{root}/{}_2", source.prefix(), self.weather)
    }
    fn output_project(&self, source: SourceBuild) -> String {
        format!("projects/{}/client_project", self.project_name(source))
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
    pub server_project_fallback: Option<ServerProjectFallback>,
    pub maps: Vec<Map>,
    pub files: Vec<BuiltFile>,
    pub converted_materials: BTreeMap<String, String>,
    pub shared_dependencies: BTreeSet<String>,
    pub repaired_model_settings: BTreeSet<String>,
    pub adapted_particles: BTreeSet<String>,
    pub collision_repairs: BTreeMap<String, String>,
    pub converted_geometry: BTreeSet<String>,
    pub compatibility_notes: Vec<String>,
    pub vegetation_instances: BTreeMap<String, usize>,
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
/// Reviewed manifest bindings (any surface) win. Early builds also have a
/// single `foliage` game material; 0.20e and 0.26 split it into
/// deciduous/coniferous with different hit pairs, so unreviewed foliage takes
/// the target build's own class for the same model surface, else the
/// coniferous fallback is recorded for review. Reviewed bindings also carry
/// footstep fixes: 0.26 adds sprint/crouch actor pairs that early builds lack
/// (0.11e pairs only `actor`/`actor_local`), and some native classes such as
/// `mud` leave `actor_local_sprint` empty.
fn classify_early_game_materials(
    path: &str,
    settings: &ConfigValue,
    target: &mut ResourceDb,
    reviewed: Option<&BTreeMap<String, String>>,
    notes: &mut Vec<String>,
) -> Result<BTreeMap<String, String>> {
    let Some(ConfigValue::Named(entries)) = settings.named("game_material_settings") else {
        return Ok(BTreeMap::new());
    };
    let native = if target.is_file(path) {
        Some(cfg(target, path)?)
    } else {
        None
    };
    let mut bindings = BTreeMap::new();
    for entry in entries {
        if let Some(name) = reviewed.and_then(|r| r.get(&entry.name)) {
            bindings.insert(entry.name.clone(), name.clone());
            continue;
        }
        let successor_of = |surface: &str| {
            native
                .as_ref()
                .and_then(|n| n.named("game_material_settings"))
                .and_then(|g| g.named(surface))
                .filter(|e| !unassigned_game_material(e))
                .and_then(|e| string(e, "game_material_name"))
                .map(str::to_owned)
        };
        if unassigned_game_material(&entry.value) {
            // 0.11e's get_pair substituted its default material for an
            // unassigned surface; 0.26 indexes its 128-entry material table
            // with 65535 instead. Take the target's own binding of the same
            // model surface when the developers assigned one.
            if let Some(name) = successor_of(&entry.name) {
                notes.push(format!(
                    "{path}: unassigned {} takes the target binding {name}",
                    entry.name
                ));
                bindings.insert(entry.name.clone(), name);
            }
            continue;
        }
        if string(&entry.value, "game_material_name") != Some("foliage") {
            continue;
        }
        let successor = native
            .as_ref()
            .and_then(|n| n.named("game_material_settings"))
            .and_then(|g| g.named(&entry.name))
            .and_then(|e| string(e, "game_material_name"))
            .filter(|name| name.starts_with("foliage_"));
        match successor {
            Some(name) => {
                bindings.insert(entry.name.clone(), name.to_owned());
            }
            None => notes.push(format!(
                "{path}: {} foliage has no reviewed or target class; coniferous fallback",
                entry.name
            )),
        }
    }
    if let Some(reviewed) = reviewed {
        for surface in reviewed.keys() {
            ensure!(
                bindings.contains_key(surface),
                "{path}: reviewed game material for missing surface {surface}"
            );
        }
    }
    Ok(bindings)
}

/// An early settings entry with no game material (id 65535, "not assigned or
/// deleted" or an empty name).
fn unassigned_game_material(entry: &ConfigValue) -> bool {
    matches!(entry.named("game_material_id"), Some(ConfigValue::Integer(65535)))
        || matches!(
            string(entry, "game_material_name"),
            Some("" | "not assigned or deleted")
        )
}

/// Replace audited primitives of one donor collision payload. The payload
/// hash and every replaced primitive's exact type and transform are checked,
/// so a changed donor stops the build instead of moving other geometry.
fn apply_primitive_overrides(
    root: &mut ConfigValue,
    original: &[u8],
    overrides: &PrimitiveOverrides,
) -> Result<()> {
    ensure!(
        format!("{:x}", Sha256::digest(original)) == overrides.donor_primitives_sha256,
        "donor collision primitives changed"
    );
    let Some(ConfigValue::Indexed(primitives)) = root.named_mut("primitives") else {
        bail!("collision has no primitive array");
    };
    let mut seen = BTreeSet::new();
    for o in &overrides.primitives {
        ensure!(seen.insert(o.index), "primitive {} overridden twice", o.index);
        let primitive = primitives
            .get_mut(o.index)
            .with_context(|| format!("no primitive {}", o.index))?;
        let field = |p: &ConfigValue, name: &str| match p.named(name) {
            Some(ConfigValue::Float3(v)) => Ok(*v),
            _ => bail!("primitive {} lacks {name}", o.index),
        };
        let current = PrimitiveTransform {
            position: field(primitive, "position")?,
            rotation: field(primitive, "rotation")?,
            scale: field(primitive, "scale")?,
        };
        ensure!(
            current == o.from,
            "primitive {} does not match its audited source",
            o.index
        );
        for (name, value) in [
            ("position", o.to.position),
            ("rotation", o.to.rotation),
            ("scale", o.to.scale),
        ] {
            ensure!(value.iter().all(|v| v.is_finite()), "non-finite override");
            primitive.set_path(name, ConfigValue::Float3(value))?;
        }
    }
    Ok(())
}

/// Respawn-point gameplay rules introduced after the early/0.20e builds.
const RESPAWN_RULES: [&str; 3] = [
    "invulnerability_time",
    "ally_check_radius",
    "enemy_check_radius",
];

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

pub fn battery_collision_offset(options: &ConfigValue) -> Result<[f32; 3]> {
    let collision = array(
        options
            .named("collision_geometries")
            .context("battery collisions")?,
    )?
    .first()
    .context("battery collision missing")?;
    match collision.named("position") {
        Some(ConfigValue::Float3(position)) if position.iter().all(|v| v.is_finite()) => {
            Ok(*position)
        }
        _ => bail!("invalid battery collision offset"),
    }
}

pub fn battery_body_transform(object: &ConfigValue, offset: [f32; 3]) -> Result<ConfigValue> {
    let Some(ConfigValue::Float3(position)) = object.named("position") else {
        bail!("battery spawn has no position");
    };
    let Some(ConfigValue::Float3(rotation)) = object.named("rotation") else {
        bail!("battery spawn has no rotation");
    };
    ensure!(
        position
            .iter()
            .chain(rotation)
            .chain(&offset)
            .all(|v| v.is_finite()),
        "invalid battery spawn transform"
    );
    let basis = crate::skeleton_animation::euler_to_basis(*rotation);
    let body_position = std::array::from_fn(|axis| {
        position[axis] + (0..3).map(|i| offset[i] * basis[i][axis]).sum::<f32>()
    });
    Ok(ConfigValue::Named(vec![
        NamedValue {
            name: "position".into(),
            value: ConfigValue::Float3(body_position),
        },
        NamedValue {
            name: "rotation".into(),
            value: ConfigValue::Float3(*rotation),
        },
    ]))
}

fn adapt_objects(
    objects: &mut ConfigValue,
    templates: &[ConfigValue],
    battery_offset: [f32; 3],
) -> Result<()> {
    let ConfigValue::Indexed(objects) = objects else {
        return Ok(());
    };
    for object in objects {
        let kind = string(object, "game_object_type")
            .context("object has no type")?
            .to_owned();
        if kind == "lpv_occluder" {
            // Native 2777 load (0x290f80) only calls load_transform (0x2a61f0).
            // No retail map supplies a template; retain the complete donor box.
            for field in ["position", "rotation", "scale"] {
                ensure!(
                    matches!(object.named(field), Some(ConfigValue::Float3(v)) if v.iter().all(|x| x.is_finite())),
                    "invalid LPV occluder {field}"
                );
            }
            object.set_path("is_temporary", ConfigValue::Boolean(false))?;
            continue;
        }
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
        // Native spawned_item is a body-centre transform; the authored parent
        // is the mesh origin. Existing native transforms are already converted.
        if kind == "victory_item_spawner" && object.named("spawned_item").is_none() {
            object.set_path(
                "spawned_item",
                battery_body_transform(object, battery_offset)?,
            )?;
        }
        // A new local transform belongs to this light, never to a different
        // lamp chosen as the native schema template.
        if kind == "light" && object.named("offset").is_none() {
            object.set_path(
                "offset",
                ConfigValue::Named(
                    [
                        ("position", [0.0; 3]),
                        ("rotation", [0.0; 3]),
                        ("scale", [1.0; 3]),
                    ]
                    .into_iter()
                    .map(|(name, value)| NamedValue {
                        name: name.into(),
                        value: ConfigValue::Float3(value),
                    })
                    .collect(),
                ),
            )?;
        }
        // Native anomaly loaders retain compatibility defaults for absent
        // fields. A different map's template supplies gameplay values and
        // object links, not schema: keep the donor object intact instead.
        if !matches!(
            kind.as_str(),
            "generic_anomaly_core" | "damage_zone" | "artefact_container"
        ) {
            // Respawn rules the source never had stay at the native loader's
            // absent-key defaults (0x2F4400: protection 0, ally/enemy check
            // radius 0 = no check) instead of another map's 6 s / 0.8 / 1.2.
            let absent: Vec<&str> = if kind == "respawn_point" {
                RESPAWN_RULES
                    .iter()
                    .copied()
                    .filter(|field| object.named(field).is_none())
                    .collect()
            } else {
                Vec::new()
            };
            fill_missing(object, template);
            if let ConfigValue::Named(fields) = object {
                fields.retain(|f| !absent.contains(&f.name.as_str()));
            }
        }
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

fn preserve_020_cloud_lighting(project: &mut ConfigValue) -> Result<usize> {
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("source project lacks client objects");
    };
    let mut changed = 0;
    for object in objects {
        if string(object, "game_object_type") != Some("environment") {
            continue;
        }
        let properties = object
            .named_mut("properties")
            .context("environment lacks properties")?;
        // 020's constructor defaults this absent switch to false. The native
        // template enables its expanded cloud-lighting implementation.
        if properties.named("use_sky_clouds_lighting").is_none() {
            properties.set_path("use_sky_clouds_lighting", ConfigValue::Boolean(false))?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn preserve_020_mie_falloff(project: &mut ConfigValue) -> Result<usize> {
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("source project lacks client objects");
    };
    let mut changed = 0;
    for object in objects {
        if string(object, "game_object_type") != Some("environment") {
            continue;
        }
        let properties = object
            .named_mut("properties")
            .context("environment lacks properties")?;
        if properties.named("mie_fog_sun_color_falloff").is_none() {
            // 020's angular sun/fog exponent was renamed in 026. Its source
            // constructor defaults to 1; do not inherit another map's value.
            let value = properties
                .named("mie_fog_falloff")
                .cloned()
                .unwrap_or(ConfigValue::Float(1.0));
            properties.set_path("mie_fog_sun_color_falloff", value)?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn adapt_project(
    mut donor: ConfigValue,
    target: &ConfigValue,
    map: &Map,
    all_templates: &[ConfigValue],
    battery_offset: [f32; 3],
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
        .context("source project lacks client_objects")?;
    adapt_objects(donor_objects, &templates, battery_offset)?;
    let ConfigValue::Named(groups) = donor
        .named_mut("server_objects")
        .context("source project lacks server_objects")?
    else {
        bail!("invalid server_objects")
    };
    for group in groups {
        adapt_objects(&mut group.value, &templates, battery_offset)?;
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
    validate_anomalies(&donor)?;
    validate_gameplay(&donor)?;
    Ok(donor)
}

/// Anomaly links are scene object names, not resource dependencies.
fn validate_anomalies(project: &ConfigValue) -> Result<()> {
    let objects = array(
        project
            .named("client_objects")
            .context("missing client objects")?,
    )?;
    let by_name: BTreeMap<_, _> = objects
        .iter()
        .filter_map(|o| Some((string(o, "full_name")?, string(o, "game_object_type")?)))
        .collect();
    let require = |name: &str, kind: &str| -> Result<()> {
        ensure!(
            by_name.get(name) == Some(&kind),
            "missing {kind} anomaly link {name}"
        );
        Ok(())
    };
    for object in objects {
        let kind = string(object, "game_object_type");
        if kind == Some("generic_anomaly_core") {
            for state in array(object.named("states").context("anomaly has no states")?)? {
                for group in array(
                    state
                        .named("groups")
                        .context("anomaly state has no groups")?,
                )? {
                    for zone in array(group.named("zones").context("anomaly group has no zones")?)?
                    {
                        let ConfigValue::String(name) = zone else {
                            bail!("invalid anomaly zone link")
                        };
                        require(name, "damage_zone")?;
                    }
                }
            }
            if object.named("artefacts_enabled") == Some(&ConfigValue::Boolean(true)) {
                for container in array(
                    object
                        .named("artefact_containers")
                        .context("enabled anomaly has no containers")?,
                )? {
                    let ConfigValue::String(name) = container else {
                        bail!("invalid artefact container link")
                    };
                    require(name, "artefact_container")?;
                }
            }
        }
        if matches!(kind, Some("damage_zone" | "artefact_container")) {
            for geometry in array(
                object
                    .named("collision_geometries")
                    .context("anomaly object has no collision zones")?,
            )? {
                require(
                    string(geometry, "name").context("unnamed anomaly collision zone")?,
                    "collision_geometry",
                )?;
            }
        }
    }
    Ok(())
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

/// Replace audited donor collision with the target build's re-authored
/// collision of the same model. Both primitive payloads are hash-pinned, the
/// donor directory must actually be imported, and every target file in the
/// directory (including meshes the donor lacks) is relocated beside the
/// donor model, so instance transforms and material bindings stay unchanged.
fn emit_collision_successors(
    successors: &BTreeMap<String, CollisionSuccessor>,
    donor_files: &BTreeSet<String>,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    source: SourceBuild,
    output: &Path,
    files: &mut Vec<BuiltFile>,
    repairs: &mut BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let mut dirs = Vec::new();
    for (dir, pin) in successors {
        ensure!(
            dir.starts_with("models/")
                && (dir.ends_with(".model/collision") || dir.ends_with(".model/walker_collision")),
            "collision successor must name a model collision directory: {dir}"
        );
        let primitives = format!("{dir}/exported_primitives");
        ensure!(
            donor_files.contains(&primitives),
            "collision successor is not imported from the donor: {dir}"
        );
        let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        ensure!(
            digest(&donor.read(&primitives)?) == pin.donor_primitives_sha256,
            "donor collision changed: {dir}"
        );
        ensure!(
            digest(&target.read(&primitives)?) == pin.target_primitives_sha256,
            "target successor collision changed: {dir}"
        );
        let prefix = format!("{dir}/");
        let paths: Vec<String> = target
            .paths_with_prefix(&prefix)
            .filter(|p| target.is_file(p))
            .map(str::to_owned)
            .collect();
        for path in paths {
            let dest = relocate_path(&path, source);
            ensure!(
                !target.contains(&dest),
                "would overwrite native resource {dest}"
            );
            let bytes = target.read(&path)?;
            emit(output, &dest, &bytes, "0.26g-successor-collision", files)?;
        }
        repairs.insert(
            dir.clone(),
            format!("target-build successor collision: {}", pin.reason),
        );
        dirs.push(dir.clone());
    }
    Ok(dirs)
}

fn relocate_path(path: &str, source: SourceBuild) -> String {
    let namespace = format!("ported/{}", source.prefix());
    for family in ["sounds/single/", "sounds/collections/", "sounds/composite/"] {
        if let Some(tail) = path.strip_prefix(family) {
            if source.early() && family == "sounds/single/" {
                if let Some(stem) = tail.strip_suffix(".single_sound_options") {
                    return format!("{family}{namespace}/{stem}.raw_options");
                }
            }
            return format!("{family}{namespace}/{tail}");
        }
    }
    let (family, tail) = path.split_once('/').expect("resource has a family");
    if family == "projects" {
        format!("projects/{}_{tail}", source.prefix())
    } else {
        format!("{family}/{namespace}/{tail}")
    }
}

fn replacements(paths: &BTreeSet<String>, source: SourceBuild) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for path in paths {
        let dest = relocate_path(path, source);
        result.insert(path.clone(), dest.clone());
        result.insert(format!("resources/{path}"), format!("resources/{dest}"));
        for marker in [".model/", ".skinned_model/"] {
            if let Some((model, _)) = path.split_once(marker) {
                let model_ext = &marker[..marker.len() - 1];
                let full = format!("{model}{model_ext}");
                let relocated = relocate_path(&full, source);
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
            ("sounds/single/", ".single_sound_options"),
            ("sounds/collections/", ".sound_collection"),
            ("sounds/composite/", ".composite_sound"),
        ] {
            if path.starts_with(family) && path.ends_with(ext) {
                result.insert(
                    path.trim_start_matches(family).trim_end_matches(ext).into(),
                    dest.trim_start_matches(family)
                        .trim_end_matches(ext)
                        .trim_end_matches(".raw_options")
                        .into(),
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
                        ("sounds/single/", ".single_sound_options"),
                        ("sounds/collections/", ".sound_collection"),
                        ("sounds/composite/", ".composite_sound"),
                    ],
                    _ => unreachable!(),
                };
                for (prefix, extension) in candidates {
                    if let Some(dest) =
                        substitutions.get(&format!("{prefix}{normalized}{extension}"))
                    {
                        let short = dest.strip_prefix(prefix).unwrap();
                        *s = short
                            .strip_suffix(extension)
                            .or_else(|| {
                                (*extension == ".single_sound_options")
                                    .then(|| short.strip_suffix(".raw_options"))
                                    .flatten()
                            })
                            .expect("relocated resource extension")
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

/// Early sound files whose donor stem has a native-format `.raw_options`.
/// The 0.11e loader reads only spl_preset/spl_value; 0.26 re-mixed many of the
/// same-named sounds (different spl_value plus volume_k), so resolving the
/// shared native options changes their level. Import the donor stem whole.
/// Stems with only a legacy `.single_sound_options` (0.10b/0.11a) are left to
/// the dependency scanner.
fn early_sound_with_raw_options(donor: &ResourceDb, path: &str) -> bool {
    let Some(tail) = path.strip_prefix("sounds/single/") else {
        return false;
    };
    let stem = [".raw_options", ".high.ogg", ".high.rms"]
        .iter()
        .find_map(|ext| tail.strip_suffix(ext));
    stem.is_some_and(|stem| donor.is_file(&format!("sounds/single/{stem}.raw_options")))
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

/// A 4x4 single-mip DXT5 texture that is zero in every channel, followed by
/// the archive's trailing sRGB byte (0: linear).
fn null_texture_dds() -> Vec<u8> {
    let mut dds = Vec::with_capacity(128 + 16 + 1);
    dds.extend_from_slice(b"DDS ");
    let mut header = [0u32; 31];
    header[0] = 124; // dwSize
    header[1] = 0x1 | 0x2 | 0x4 | 0x1000 | 0x80000; // CAPS|HEIGHT|WIDTH|PIXELFORMAT|LINEARSIZE
    header[2] = 4; // height
    header[3] = 4; // width
    header[4] = 16; // linear size
    header[6] = 1; // mip count
    header[18] = 32; // pixel format size
    header[19] = 0x4; // DDPF_FOURCC
    header[20] = u32::from_le_bytes(*b"DXT5");
    header[26] = 0x1000; // DDSCAPS_TEXTURE
    for word in header {
        dds.extend_from_slice(&word.to_le_bytes());
    }
    dds.extend_from_slice(&[0u8; 16]);
    dds.push(0);
    dds
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
    server_donor_path: Option<&Path>,
) -> Result<Record> {
    build_with_fill(manifest_path, donor_path, target_path, output, server_donor_path, &[])
}

/// The faithful port plus optional, explicitly named 0.26 gameplay-fill packs.
pub fn build_with_fill(
    manifest_path: &Path,
    donor_path: &Path,
    target_path: &Path,
    output: &Path,
    server_donor_path: Option<&Path>,
    fill_paths: &[std::path::PathBuf],
) -> Result<Record> {
    let m: Manifest = serde_json::from_reader(fs::File::open(manifest_path)?)?;
    let fills: Vec<fill::FillPack> = fill_paths
        .iter()
        .map(|p| -> Result<_> {
            serde_json::from_reader(fs::File::open(p)?).with_context(|| format!("reading {}", p.display()))
        })
        .collect::<Result<_>>()?;
    for pack in &fills {
        ensure!(
            m.maps.iter().any(|map| map.id == pack.map_id),
            "gameplay fill {} names map {} which this manifest does not build",
            pack.name,
            pack.map_id
        );
    }
    eprintln!("Checking donor and target fingerprints...");
    check_hash(donor_path, &m.donor_sha256)?;
    check_hash(target_path, &m.target_sha256)?;
    let mut server_donor = match (&m.server_project_fallback, server_donor_path) {
        (None, None) => None,
        (Some(fallback), Some(path)) => {
            check_hash(path, &fallback.donor_sha256)?;
            Some(ResourceDb::open(path)?)
        }
        _ => bail!("server fallback manifest and --server-donor-db must be supplied together"),
    };
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
            ids.insert(map.id) && projects.insert(map.donor_project(m.source_build)),
            "duplicate map ID or project"
        );
    }
    let mut donor = ResourceDb::open(donor_path)?;
    let mut target = ResourceDb::open(target_path)?;
    let battery_offset =
        battery_collision_offset(&cfg(&mut target, "gameplay/victory_items/default.options")?)?;
    let mut dictionary = cfg(&mut target, DICTIONARY)?;
    let ConfigValue::Indexed(registered) = dictionary
        .named("maps_dict")
        .context("0.26 has no maps_dict")?
    else {
        bail!("unexpected map dictionary schema")
    };
    for entry in registered {
        if let Some(ConfigValue::Integer(id)) = entry.named("id") {
            ensure!(
                !ids.contains(&u8::try_from(*id)?),
                "map ID {id} already used"
            );
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
    if m.source_build.early() || m.source_build == SourceBuild::V020e {
        // Factory no longer authors some old scene types (fog volumes and
        // wire sets). Rudnya supplies schemas, never its placed objects.
        let fallback = cfg(&mut target, "projects/level_01/day_2/client_project")?;
        templates.extend(
            array(
                fallback
                    .named("client_objects")
                    .context("fallback objects")?,
            )?
            .iter()
            .cloned(),
        );
    }
    let mut overrides = BTreeMap::new();
    // Native model references added by gameplay-fill packs (never by the port).
    let mut fill_models = BTreeSet::new();
    let mut collision_repairs = BTreeMap::new();
    let mut compatibility_notes = Vec::new();
    for map in &m.maps {
        let raw = cfg(&mut donor, &map.donor_project(m.source_build))?;
        let mut raw = if m.source_build.early() {
            let server_path = map
                .donor_project(m.source_build)
                .replace("client_project", "server_project");
            let server = if donor.is_file(&server_path) {
                cfg(&mut donor, &server_path)?
            } else {
                let fallback = m
                    .server_project_fallback
                    .as_ref()
                    .context("missing source server project; no fallback declared")?;
                let expected = fallback
                    .projects
                    .get(&server_path)
                    .context("missing server project is not allowlisted")?;
                let bytes = server_donor.as_mut().unwrap().read(&server_path)?;
                ensure!(
                    format!("{:x}", Sha256::digest(&bytes)) == *expected,
                    "fallback server payload hash mismatch"
                );
                let server = BinaryConfig::parse(&bytes)?.root;
                early::validate_server_fallback(&raw, &server)?;
                compatibility_notes.push(format!("{server_path}: absent from {}; gameplay uses pinned {} server project {expected}; respawns, collectors and zones exactly match the primary client", m.source_build.label(), fallback.source_build.label()));
                server
            };
            early::project(
                raw,
                &mut donor,
                server,
                &target_projects[&map.target_project()],
                map,
                m.source_build,
                &mut compatibility_notes,
            )?
        } else {
            raw
        };
        if m.source_build == SourceBuild::V020e {
            let count = preserve_020_cloud_lighting(&mut raw)?;
            compatibility_notes.push(format!(
                "{}: restored source-default cloud lighting off in {count} environments with no authored switch",
                map.donor_project(m.source_build)
            ));
            let count = preserve_020_mie_falloff(&mut raw)?;
            compatibility_notes.push(format!(
                "{}: mapped source Mie angular falloff to its native field in {count} environments",
                map.donor_project(m.source_build)
            ));
        }
        let mut project = adapt_project(
            raw,
            &target_projects[&map.target_project()],
            map,
            &templates,
            battery_offset,
        )
        .with_context(|| format!("adapting {}", map.donor_project(m.source_build)))?;
        adapt_collisions(
            &mut project,
            &donor,
            &m.missing_collision_resources,
            &map.donor_project(m.source_build),
            &mut collision_repairs,
        )?;
        for pack in fills.iter().filter(|p| p.map_id == map.id) {
            let bytes = target.read(&pack.template_project)?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == pack.template_sha256,
                "gameplay fill {}: native template project changed",
                pack.name
            );
            let native = BinaryConfig::parse(&bytes)?.root;
            let applied = fill::apply(&mut project, &native, pack)
                .with_context(|| format!("applying gameplay fill {}", pack.name))?;
            compatibility_notes.extend(applied.notes);
            fill_models.extend(applied.native_models);
        }
        rewrite(&mut project, &m.reference_replacements);
        overrides.insert(
            map.donor_project(m.source_build),
            BinaryConfig { root: project }.to_bytes()?,
        );
    }
    let forced: BTreeSet<String> = donor
        .entries()
        .keys()
        .filter(|p| {
            donor.is_file(p)
                && (p.starts_with("textures/")
                    || p.starts_with("material_instances/")
                    || (m.source_build == SourceBuild::V020e
                        && (p.starts_with("particles/") || p.starts_with("sounds/")))
                    || (m.source_build.early() && early_sound_with_raw_options(&donor, p))
                    || p.ends_with(".veg"))
        })
        .cloned()
        .chain(
            donor
                .entries()
                .keys()
                .filter_map(|p| p.split_once(".model/").map(|(p, _)| format!("{p}.model"))),
        )
        .collect();
    eprintln!("Resolving map dependencies...");
    let plan = crate::dependency::scan_with_forced_donor_and_overrides(
        &mut donor,
        &mut target,
        projects.clone(),
        forced,
        &overrides,
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
    let mut substitutions = replacements(&plan.donor_files, m.source_build);
    for map in &m.maps {
        substitutions.insert(
            map.donor_project(m.source_build),
            map.output_project(m.source_build),
        );
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
        let project = BinaryConfig::parse(&overrides[&map.donor_project(m.source_build)])?;
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
                // Gameplay-fill instances use native models shared from the target.
                let native_fill = fill_models.contains(reference)
                    && !donor.contains(&model)
                    && target.paths_with_prefix(&format!("{model}/")).next().is_some();
                ensure!(imported || native_fill, "source model not imported: {reference}");
            }
        }
    }
    let mut catalog = crate::material_conversion::MaterialCatalog::load(&mut target)?;
    catalog.set_runtime_proven_switches(m.runtime_proven_switches.iter().map(|(material, entry)| {
        (material.as_str(), entry.shader_catalog_sha256.as_str(), &entry.switches)
    }))?;
    let null_texture = format!("ported/{}/srp/null", m.source_build.prefix());
    catalog.set_null_texture(Some(null_texture.clone()));
    let mut null_texture_used = false;
    let game_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let material_names = BTreeMap::from([
        ("foliage".into(), "foliage_coniferous".into()),
        ("wood_tin".into(), "wood_thin".into()),
    ]);
    let particle_defaults = particle_defaults(&mut target)?;
    let mut adapted_particles = BTreeSet::new();
    crate::weapon::prepare_empty_output(output)?;
    let mut files = Vec::new();
    let mut converted_geometry = BTreeSet::new();
    let mut vegetation_instances = BTreeMap::new();
    let mut converted_materials = BTreeMap::new();
    let mut repaired_model_settings = BTreeSet::new();
    for path in &m.wrapped_uv_surfaces {
        ensure!(
            m.source_build.early()
                && path.ends_with("/converted_model")
                && plan.donor_files.contains(path),
            "wrapped UV surface must name a selected early geometry resource: {path}"
        );
    }
    for dir in m.collision_primitive_overrides.keys() {
        ensure!(
            plan.donor_files.contains(&format!("{dir}/exported_primitives")),
            "primitive override names a collision the maps do not import: {dir}"
        );
    }
    let successor_dirs = emit_collision_successors(
        &m.collision_successors,
        &plan.donor_files,
        &mut donor,
        &mut target,
        m.source_build,
        output,
        &mut files,
        &mut collision_repairs,
    )?;
    for path in &plan.donor_files {
        if successor_dirs
            .iter()
            .any(|dir| path.starts_with(&format!("{dir}/")))
        {
            continue;
        }
        let mut bytes = if let Some(b) = overrides.get(path) {
            b.clone()
        } else {
            donor.read(path)?
        };
        if m.source_build.early() && path.ends_with("/converted_model") {
            bytes = early::geometry(&bytes, m.wrapped_uv_surfaces.contains(path))
                .with_context(|| format!("converting early geometry {path}"))?;
            if m.wrapped_uv_surfaces.contains(path) {
                compatibility_notes.push(format!("{path}: integer UV island offsets removed before half packing; repeating material sampling preserved"));
            }
            converted_geometry.insert(path.clone());
        }
        if path.starts_with("textures/") && path.ends_with(".dds") {
            if let Some((bc3, report)) = crate::dds_bc3::bc2_to_bc3(&bytes)
                .with_context(|| format!("re-encoding BC2 texture {path}"))?
            {
                bytes = bc3;
                compatibility_notes.push(format!(
                    "{path}: BC2 re-encoded as BC3 for the 0.26 texture streamer (colour blocks, mips and sRGB byte kept; {} blocks, max alpha error {:.1}/255, {} alpha-test threshold crossings)",
                    report.blocks, report.max_alpha_error, report.threshold_crossings
                ));
            }
        }
        if path.ends_with(".veg") {
            vegetation_instances.insert(
                path.clone(),
                early::vegetation_instances(&bytes)
                    .with_context(|| format!("checking original vegetation {path}"))?,
            );
        }
        if m.source_build.early() && path.ends_with(".single_sound_options") {
            bytes = target.read("sounds/single/weather/wind/wind_day.raw_options")?;
            compatibility_notes.push(format!("{path}: original audio retained with native wind ambience attenuation; legacy SPL curve not reproduced"));
        }
        if path.starts_with("material_instances/") && path.ends_with(".material") {
            let explicit = m
                .material_templates
                .get(path)
                .map(String::as_str)
                .or_else(|| {
                    (!m.source_build.early() && target.is_file(path)).then_some(path.as_str())
                });
            let converted = catalog
                .convert(path, &bytes, explicit, &donor, &target)
                .with_context(|| format!("converting {path}"))?;
            bytes = converted.bytes;
            for note in &converted.switch_notes {
                null_texture_used |= note.contains("bound to the all-zero texture");
                compatibility_notes.push(format!("{path}: {note}"));
            }
            converted_materials.insert(path.clone(), converted.template_path);
        }
        if is_config_resource(path) {
            let mut c =
                BinaryConfig::parse(&bytes).with_context(|| format!("decoding config {path}"))?;
            if let Some(overrides) = path
                .strip_suffix("/exported_primitives")
                .and_then(|dir| m.collision_primitive_overrides.get(dir))
            {
                apply_primitive_overrides(&mut c.root, &bytes, overrides)
                    .with_context(|| format!("overriding primitives of {path}"))?;
                collision_repairs.insert(path.clone(), overrides.reason.clone());
            }
            if path.ends_with(".particle") && adapt_particle(&mut c.root, &particle_defaults)? {
                adapted_particles.insert(path.clone());
            }
            if let Some(model) = path.strip_suffix(".model/settings") {
                if m.source_build.early() && c.root.named("game_material_settings").is_some() {
                    let bindings = classify_early_game_materials(
                        path,
                        &c.root,
                        &mut target,
                        m.game_material_overrides.get(path),
                        &mut compatibility_notes,
                    )?;
                    crate::target_native_map::remap_game_material_settings(
                        &mut c.root,
                        path,
                        &game_materials,
                        &material_names,
                        Some(&bindings),
                    )?;
                }
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
        let diffuse_probe = if m.source_build.early()
            && path.contains("/environment_probes/")
            && path.ends_with(".dds")
            && !path.ends_with("_diffuse.dds")
        {
            Some(
                early::diffuse_probe(&bytes)
                    .with_context(|| format!("deriving diffuse probe {path}"))?,
            )
        } else {
            None
        };
        emit(
            output,
            dest,
            &bytes,
            &format!("{}+0.26g-adapter", m.source_build.label()),
            &mut files,
        )?;
        // A donor that ships its own RMS cache (0.11e) keeps it; only builds
        // without one (0.10b/0.11a) get a cache generated from the audio.
        if m.source_build.early()
            && path.ends_with(".high.ogg")
            && !plan
                .donor_files
                .contains(&(path.trim_end_matches(".ogg").to_owned() + ".rms"))
        {
            let rms =
                early::sound_rms(&bytes).with_context(|| format!("generating RMS for {path}"))?;
            emit(
                output,
                &(dest.trim_end_matches(".ogg").to_owned() + ".rms"),
                &rms,
                "original-audio-rms",
                &mut files,
            )?;
        }
        if let Some(diffuse) = diffuse_probe {
            let diffuse_path = dest.trim_end_matches(".dds").to_owned() + "_diffuse.dds";
            ensure!(
                !target.contains(&diffuse_path),
                "derived probe overwrites native resource"
            );
            emit(
                output,
                &diffuse_path,
                &diffuse,
                "early-probe-coarse-mips",
                &mut files,
            )?;
            let options_path = path.trim_end_matches(".dds").to_owned() + ".options";
            let options = donor.read(&options_path)?;
            emit(
                output,
                &(dest.trim_end_matches(".dds").to_owned() + "_diffuse.options"),
                &options,
                "early-probe-options",
                &mut files,
            )?;
            compatibility_notes.push(format!(
                "{diffuse_path}: source HDR bytes retained; source 16x16 mip tail approximates diffuse irradiance"
            ));
        }
    }
    for map in &m.maps {
        let mode_name = format!("{}_{}", m.source_build.prefix(), map.mode());
        let mut mode = cfg(&mut target, &format!("gameplay/game_modes/{}", map.mode()))?;
        mode.set_path(
            "mode.name",
            ConfigValue::String(format!("{}_{}_2", m.source_build.prefix(), map.level)),
        )?;
        mode.set_path(
            "mode.match_making_params.map_name",
            ConfigValue::String(map.project_name(m.source_build)),
        )?;
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
    if null_texture_used {
        let dds = format!("textures/{null_texture}.dds");
        ensure!(!target.contains(&dds), "null texture overwrites native resource {dds}");
        emit(output, &dds, &null_texture_dds(), "srp-null-texture", &mut files)?;
        emit(
            output,
            &format!("textures/{null_texture}.options"),
            &target.read("textures/editor/default_nmap.options")?,
            "srp-null-texture (linear options of textures/editor/default_nmap)",
            &mut files,
        )?;
    }
    let record = Record {
        format_version: 1,
        asset_id: format!("retail-{}-maps-to-026g", m.source_build.prefix()),
        donor_build: m.source_build.label().into(),
        target_build: "0.26g0-2777".into(),
        target_db_sha256: m.target_sha256,
        server_project_fallback: m.server_project_fallback,
        maps: m.maps,
        files,
        converted_materials,
        shared_dependencies: plan.target_files,
        repaired_model_settings,
        adapted_particles,
        collision_repairs,
        converted_geometry,
        compatibility_notes,
        vegetation_instances,
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
        rewrite_inner(
            &mut scene,
            &replacements(&paths, SourceBuild::V021d),
            true,
            None,
        );
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
        rewrite_inner(
            &mut scene,
            &replacements(&paths, SourceBuild::V021d),
            true,
            None,
        );
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
        let occluder = named(vec![
            (
                "game_object_type",
                ConfigValue::String("lpv_occluder".into()),
            ),
            ("position", ConfigValue::Float3([88.238, 6.05, 118.91])),
            ("rotation", ConfigValue::Float3([0.; 3])),
            ("scale", ConfigValue::Float3([39.459, 11.679, 21.743])),
        ]);
        let mut objects = ConfigValue::Indexed(vec![occluder.clone()]);
        adapt_objects(&mut objects, &[], [0., 0.12, 0.]).unwrap();
        for field in ["position", "rotation", "scale"] {
            assert_eq!(
                array(&objects).unwrap()[0].named(field),
                occluder.named(field)
            );
        }
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
            environment_look: Default::default(),
        };
        let output = adapt_project(donor.clone(), &donor, &map, &[], [0., 0.12, 0.]).unwrap();
        assert_eq!(string(&output, "project_name"), Some("level_02"));
        assert_eq!(
            map.output_project(SourceBuild::V021d),
            "projects/v021d_level_02/rain_2/client_project"
        );
        assert_eq!(
            map.project_name(SourceBuild::V021d),
            "v021d_level_02/rain_2"
        );
    }
    #[test]
    fn objects_do_not_inherit_another_maps_gameplay_or_light_transform() {
        for (old, explicit, expected) in [
            (Some(2.0), None, 2.0),
            (None, None, 1.0),
            (Some(2.0), Some(3.0), 3.0),
        ] {
            let mut properties = named(vec![]);
            for (key, value) in [
                ("mie_fog_falloff", old),
                ("mie_fog_sun_color_falloff", explicit),
            ] {
                if let Some(value) = value {
                    properties.set_path(key, ConfigValue::Float(value)).unwrap();
                }
            }
            let mut project = named(vec![(
                "client_objects",
                ConfigValue::Indexed(vec![named(vec![
                    ("game_object_type", ConfigValue::String("environment".into())),
                    ("properties", properties),
                ])]),
            )]);
            assert_eq!(
                preserve_020_mie_falloff(&mut project).unwrap(),
                usize::from(explicit.is_none())
            );
            let env = &array(project.named("client_objects").unwrap()).unwrap()[0];
            assert_eq!(
                env.named("properties")
                    .unwrap()
                    .named("mie_fog_sun_color_falloff"),
                Some(&ConfigValue::Float(expected))
            );
        }
        for authored in [None, Some(false), Some(true)] {
            let mut properties = named(vec![]);
            if let Some(value) = authored {
                properties
                    .set_path("use_sky_clouds_lighting", ConfigValue::Boolean(value))
                    .unwrap();
            }
            let mut project = named(vec![(
                "client_objects",
                ConfigValue::Indexed(vec![named(vec![
                    ("game_object_type", ConfigValue::String("environment".into())),
                    ("properties", properties),
                ])]),
            )]);
            assert_eq!(
                preserve_020_cloud_lighting(&mut project).unwrap(),
                usize::from(authored.is_none())
            );
            let env = &array(project.named("client_objects").unwrap()).unwrap()[0];
            assert_eq!(
                env.named("properties")
                    .unwrap()
                    .named("use_sky_clouds_lighting"),
                Some(&ConfigValue::Boolean(authored.unwrap_or(false)))
            );
        }
        for kind in ["generic_anomaly_core", "damage_zone", "artefact_container"] {
            let donor = named(vec![
                ("game_object_type", ConfigValue::String(kind.into())),
                ("lib_name", ConfigValue::String(kind.into())),
                ("position", ConfigValue::Float3([1.0, 2.0, 3.0])),
            ]);
            let template = named(vec![
                ("game_object_type", ConfigValue::String(kind.into())),
                ("lib_name", ConfigValue::String(kind.into())),
                ("position", ConfigValue::Float3([4.0, 5.0, 6.0])),
                ("max_artefacts_count", ConfigValue::Integer(1)),
                ("artefacts_first_spawn_time_sec", ConfigValue::Integer(60)),
                ("artefacts_cooldown_time_sec", ConfigValue::Integer(30)),
                (
                    "artefact_containers",
                    ConfigValue::Indexed(vec![ConfigValue::String("native/other-zone".into())]),
                ),
                (
                    "idle_particle",
                    ConfigValue::String("native/other-particle".into()),
                ),
            ]);
            let mut expected = donor.clone();
            expected
                .set_path("is_temporary", ConfigValue::Boolean(false))
                .unwrap();
            let mut objects = ConfigValue::Indexed(vec![donor]);
            adapt_objects(&mut objects, &[template], [0., 0.12, 0.]).unwrap();
            assert_eq!(objects, ConfigValue::Indexed(vec![expected]));
        }
        // Respawn rules absent from the source keep the loader defaults; an
        // authored (already converted) protection time is retained.
        let spawn_template = named(vec![
            ("game_object_type", ConfigValue::String("respawn_point".into())),
            ("lib_name", ConfigValue::String("respawn_point".into())),
            ("invulnerability_time", ConfigValue::Integer(6)),
            ("ally_check_radius", ConfigValue::Float(0.8)),
            ("enemy_check_radius", ConfigValue::Float(1.2)),
            ("layer", ConfigValue::String("default".into())),
        ]);
        for authored in [None, Some(2)] {
            let mut donor = named(vec![
                ("game_object_type", ConfigValue::String("respawn_point".into())),
                ("lib_name", ConfigValue::String("respawn_point".into())),
                ("team", ConfigValue::Integer(1)),
            ]);
            if let Some(seconds) = authored {
                donor
                    .set_path("invulnerability_time", ConfigValue::Integer(seconds))
                    .unwrap();
            }
            let mut objects = ConfigValue::Indexed(vec![donor]);
            adapt_objects(&mut objects, &[spawn_template.clone()], [0., 0.12, 0.])
                .unwrap();
            let spawn = &array(&objects).unwrap()[0];
            assert_eq!(spawn.named("ally_check_radius"), None);
            assert_eq!(spawn.named("enemy_check_radius"), None);
            assert_eq!(
                spawn.named("invulnerability_time"),
                authored.map(ConfigValue::Integer).as_ref()
            );
            assert_eq!(
                spawn.named("layer"),
                Some(&ConfigValue::String("default".into()))
            );
        }
        let light = named(vec![
            ("game_object_type", ConfigValue::String("light".into())),
            ("lib_name", ConfigValue::String("spot_light".into())),
            ("position", ConfigValue::Float3([3., 4., 5.])),
        ]);
        let mut template = light.clone();
        template.set_path("offset", named(vec![])).unwrap();
        template
            .set_path(
                "offset.position",
                ConfigValue::Float3([0.01, -0.265, 0.002]),
            )
            .unwrap();
        template
            .set_path("offset.rotation", ConfigValue::Float3([0.; 3]))
            .unwrap();
        template
            .set_path("offset.scale", ConfigValue::Float3([1.; 3]))
            .unwrap();
        for authored_offset in [None, Some([0., 0.5, 0.])] {
            let mut donor = light.clone();
            if let Some(value) = authored_offset {
                donor.set_path("offset", named(vec![])).unwrap();
                donor
                    .set_path("offset.position", ConfigValue::Float3(value))
                    .unwrap();
            }
            let mut objects = ConfigValue::Indexed(vec![donor]);
            adapt_objects(&mut objects, &[template.clone()], [0., 0.12, 0.]).unwrap();
            let result = &array(&objects).unwrap()[0];
            assert_eq!(result.named("position"), light.named("position"));
            assert_eq!(
                result.named("offset").unwrap().named("position"),
                Some(&ConfigValue::Float3(authored_offset.unwrap_or([0.; 3])))
            );
        }
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
    fn early_foliage_takes_reviewed_or_successor_class() {
        let settings = |surfaces: &[(&str, &str)]| {
            named(vec![(
                "game_material_settings",
                ConfigValue::Named(
                    surfaces
                        .iter()
                        .map(|(surface, material)| NamedValue {
                            name: (*surface).into(),
                            value: named(vec![
                                (
                                    "game_material_name",
                                    ConfigValue::String((*material).into()),
                                ),
                                ("game_material_id", ConfigValue::Integer(1)),
                            ]),
                        })
                        .collect(),
                ),
            )])
        };
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("target");
        let native = |path: &str, surfaces: &[(&str, &str)]| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let bytes = BinaryConfig {
                root: settings(surfaces),
            }
            .to_bytes()
            .unwrap();
            fs::write(file, bytes).unwrap();
        };
        native("models/bush.model/settings", &[("leaves", "foliage_deciduous")]);
        native("models/fir.model/settings", &[("leaves", "wood_tree")]);
        let db = temp.path().join("target.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &db,
        )
        .unwrap();
        let mut target = ResourceDb::open(db).unwrap();
        let early = settings(&[("leaves", "foliage"), ("trunk", "wood_tree")]);
        let mut notes = Vec::new();
        // The target's own foliage class for the same model surface.
        let mut classify = |path: &str, reviewed: Option<&BTreeMap<String, String>>| {
            classify_early_game_materials(path, &early, &mut target, reviewed, &mut notes)
        };
        let bush = classify("models/bush.model/settings", None).unwrap();
        assert_eq!(
            bush,
            BTreeMap::from([("leaves".into(), "foliage_deciduous".into())])
        );
        // A non-foliage successor binding is not a class: fallback, recorded.
        assert!(classify("models/fir.model/settings", None).unwrap().is_empty());
        // A reviewed binding wins; one for an absent surface is rejected.
        let reviewed = BTreeMap::from([("leaves".into(), "foliage_coniferous".into())]);
        let poplar = classify("models/poplar.model/settings", Some(&reviewed)).unwrap();
        assert_eq!(poplar, reviewed);
        let wrong = BTreeMap::from([("bark".into(), "foliage_deciduous".into())]);
        assert!(classify("models/poplar.model/settings", Some(&wrong)).is_err());
        // Reviewed bindings apply to any surface (e.g. a footstep class).
        let walkable = BTreeMap::from([("trunk".into(), "grass".into())]);
        assert_eq!(
            classify("models/bush.model/settings", Some(&walkable)).unwrap(),
            BTreeMap::from([
                ("leaves".into(), "foliage_deciduous".into()),
                ("trunk".into(), "grass".into()),
            ])
        );
        drop(classify);
        // The fir fallback, then the unreviewed surface of the rejected call.
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn unassigned_early_surface_takes_assigned_target_binding_only() {
        let settings = |surfaces: &[(&str, &str, i32)]| {
            named(vec![(
                "game_material_settings",
                ConfigValue::Named(
                    surfaces
                        .iter()
                        .map(|(surface, material, id)| NamedValue {
                            name: (*surface).into(),
                            value: named(vec![
                                (
                                    "game_material_name",
                                    ConfigValue::String((*material).into()),
                                ),
                                ("game_material_id", ConfigValue::Integer(*id)),
                            ]),
                        })
                        .collect(),
                ),
            )])
        };
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("target");
        for (path, surfaces) in [
            ("models/turbine.model/settings", vec![("default", "metall_thick", 18)]),
            ("models/bg.model/settings", vec![("default", "not assigned or deleted", 65535)]),
        ] {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, BinaryConfig { root: settings(&surfaces) }.to_bytes().unwrap()).unwrap();
        }
        let db = temp.path().join("target.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &db,
        )
        .unwrap();
        let mut target = ResourceDb::open(db).unwrap();
        let early = settings(&[
            ("default", "not assigned or deleted", 65535),
            ("coll_concrete", "concrete", 19),
        ]);
        let mut notes = Vec::new();
        let mut classify = |path: &str| {
            classify_early_game_materials(path, &early, &mut target, None, &mut notes).unwrap()
        };
        // The developers' own binding of the same surface replaces 65535.
        assert_eq!(
            classify("models/turbine.model/settings"),
            BTreeMap::from([("default".into(), "metall_thick".into())])
        );
        // A target that is itself unassigned, or no target model: unchanged.
        assert!(classify("models/bg.model/settings").is_empty());
        assert!(classify("models/test_cube.model/settings").is_empty());
        drop(classify);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn primitive_override_moves_only_audited_boxes() {
        let boxed = |z: f32| {
            named(vec![
                ("type", ConfigValue::Integer(1)),
                ("position", ConfigValue::Float3([22.5, 37.7, z])),
                ("rotation", ConfigValue::Float3([1.5707964, 0., 0.])),
                ("scale", ConfigValue::Float3([26.3, 1.76, 37.6])),
            ])
        };
        let root = named(vec![(
            "primitives",
            ConfigValue::Indexed(vec![boxed(10.), boxed(133.64)]),
        )]);
        let bytes = BinaryConfig { root: root.clone() }.to_bytes().unwrap();
        let transform = |z: f32, thickness: f32| PrimitiveTransform {
            position: [22.5, 37.7, z],
            rotation: [1.5707964, 0., 0.],
            scale: [26.3, thickness, 37.6],
        };
        let overrides = |hash: String, from_z: f32| PrimitiveOverrides {
            donor_primitives_sha256: hash,
            reason: "test".into(),
            primitives: vec![PrimitiveOverride {
                index: 1,
                from: transform(from_z, 1.76),
                to: transform(139.89, 1.57),
            }],
        };
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let mut moved = root.clone();
        apply_primitive_overrides(&mut moved, &bytes, &overrides(hash.clone(), 133.64))
            .unwrap();
        let primitives = array(moved.named("primitives").unwrap()).unwrap();
        assert_eq!(primitives[0], boxed(10.));
        assert_eq!(
            primitives[1].named("position"),
            Some(&ConfigValue::Float3([22.5, 37.7, 139.89]))
        );
        assert_eq!(
            primitives[1].named("scale"),
            Some(&ConfigValue::Float3([26.3, 1.57, 37.6]))
        );
        // A changed payload or a primitive that is not the audited one is rejected.
        let other = format!("{:x}", Sha256::digest(b"other"));
        let rejected = |o: PrimitiveOverrides| {
            apply_primitive_overrides(&mut root.clone(), &bytes, &o).is_err()
        };
        assert!(rejected(overrides(other, 133.64)));
        assert!(rejected(overrides(hash, 133.0)));
    }

    #[test]
    fn collision_successor_replaces_only_a_pinned_imported_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let pack = |name: &str, files: &[(&str, &[u8])]| {
            let root = temp.path().join(name);
            for (path, bytes) in files {
                let file = root.join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, bytes).unwrap();
            }
            let db = temp.path().join(format!("{name}.db"));
            resources_db::pack::assemble_to_file(
                &resources_db::pack::build_src_tree(&root).unwrap(),
                &db,
            )
            .unwrap();
            ResourceDb::open(db).unwrap()
        };
        let dir = "models/boiler.model/walker_collision";
        let primitives = format!("{dir}/exported_primitives");
        let mut donor = pack("donor", &[(&primitives, b"lying drums")]);
        let mut target = pack(
            "target",
            &[(&primitives, b"vertical tanks"), (&format!("{dir}/vertices"), b"mesh")],
        );
        let digest = |b: &[u8]| format!("{:x}", Sha256::digest(b));
        let pin = |donor_hash: String| {
            BTreeMap::from([(
                dir.to_owned(),
                CollisionSuccessor {
                    donor_primitives_sha256: donor_hash,
                    target_primitives_sha256: digest(b"vertical tanks"),
                    reason: "test".into(),
                },
            )])
        };
        let imported = BTreeSet::from([primitives.clone()]);
        let mut run = |successors: &BTreeMap<String, CollisionSuccessor>,
                       imported: &BTreeSet<String>,
                       out: &str| {
            let output = temp.path().join(out);
            let mut files = Vec::new();
            let result = emit_collision_successors(
                successors,
                imported,
                &mut donor,
                &mut target,
                SourceBuild::V011e,
                &output,
                &mut files,
                &mut BTreeMap::new(),
            );
            (result, files, output)
        };
        let (dirs, files, output) = run(&pin(digest(b"lying drums")), &imported, "ok");
        assert_eq!(dirs.unwrap(), vec![dir.to_owned()]);
        let mut written: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        written.sort();
        assert_eq!(
            written,
            [
                "models/ported/v011e/boiler.model/walker_collision/exported_primitives",
                "models/ported/v011e/boiler.model/walker_collision/vertices"
            ]
        );
        assert_eq!(
            fs::read(output.join("overlay").join(&written[0])).unwrap(),
            b"vertical tanks"
        );
        // A changed donor payload or a directory the map does not import is rejected.
        assert!(run(&pin(digest(b"other")), &imported, "changed").0.is_err());
        assert!(run(&pin(digest(b"lying drums")), &BTreeSet::new(), "unused").0.is_err());
    }

    #[test]
    fn collision_fallback_retains_scale_and_missing_geometry_needs_review() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        let file = root.join("models/one.model/collision/vertices");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"geometry").unwrap();
        let dbpath = temp.path().join("source.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &dbpath,
        )
        .unwrap();
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
        // Retail 0.20e/0.26 anomal_tree_0X_ and soviet_comp_01: one kind is an
        // empty folder node, the other has a mesh. Each instance must keep
        // exactly the source's shape set: nothing for the empty kind.
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        for (model, full, empty) in [
            ("tree", "collision", "walker_collision"),
            ("comp", "walker_collision", "collision"),
        ] {
            let file = root.join(format!("models/{model}.model/{full}/vertices"));
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"geometry").unwrap();
            fs::create_dir_all(root.join(format!("models/{model}.model/{empty}"))).unwrap();
        }
        let dbpath = temp.path().join("source.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &dbpath,
        )
        .unwrap();
        let db = ResourceDb::open(dbpath).unwrap();
        assert!(db.contains("models/tree.model/walker_collision"));
        let make = |r: &str| named(vec![("lib_name", ConfigValue::String(r.into()))]);
        let mut project = named(vec![(
            "collision_objects",
            ConfigValue::Indexed(vec![
                make("resources/models/tree.model/collision"),
                make("resources/models/tree.model/walker_collision#[1.67][1.67][1.67]"),
                make("resources/models/comp.model/walker_collision"),
                make("resources/models/comp.model/collision"),
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
                "resources/models/comp.model/walker_collision"
            ]
        );
        assert_eq!(repairs.len(), 2);
        assert!(repairs.values().all(|v| v.starts_with("omitted: empty")));
    }

    #[test]
    fn early_sound_stems_with_raw_options_come_from_the_donor() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("input");
        for file in [
            "sounds/single/scapes/industrial_space.raw_options",
            "sounds/single/scapes/industrial_space.high.ogg",
            "sounds/single/scapes/industrial_space.high.rms",
            "sounds/single/weather/rain/rain.single_sound_options",
            "sounds/single/weather/rain/rain.high.ogg",
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let dbpath = temp.path().join("donor.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&root).unwrap(),
            &dbpath,
        )
        .unwrap();
        let db = ResourceDb::open(dbpath).unwrap();
        for ext in ["raw_options", "high.ogg", "high.rms"] {
            assert!(early_sound_with_raw_options(
                &db,
                &format!("sounds/single/scapes/industrial_space.{ext}")
            ));
        }
        // Legacy options keep the scanner's native-companion policy.
        assert!(!early_sound_with_raw_options(
            &db,
            "sounds/single/weather/rain/rain.high.ogg"
        ));
        assert!(!early_sound_with_raw_options(
            &db,
            "sounds/single/weather/rain/rain.single_sound_options"
        ));
        assert!(!early_sound_with_raw_options(&db, "particles/x.particle"));
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
            environment_look: Default::default(),
        };
        let converted = adapt_project(donor.clone(), &donor, &map, &[], [0., 0.12, 0.]).unwrap();
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
        adapt_objects(&mut objects, &[template.clone()], [0., 0.12, 0.]).unwrap();
        let o = &array(&objects).unwrap()[0];
        assert_eq!(
            o.named("spawned_item").unwrap().named("position"),
            Some(&ConfigValue::Float3([10., 2.12, 3.]))
        );
        assert_eq!(
            o.named("position"),
            Some(&ConfigValue::Float3([10., 2., 3.]))
        );
        let once = objects.clone();
        adapt_objects(&mut objects, &[template], [0., 0.12, 0.]).unwrap();
        assert_eq!(
            objects, once,
            "existing body origins must not receive a second offset"
        );
        // Native authored examples cover tilted and nearly inverted Euler bases.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tools/day-map-objectives/native-battery-origin-contract-20261014.json"
        ))
        .unwrap();
        let options = named(vec![(
            "collision_geometries",
            ConfigValue::Indexed(vec![named(vec![(
                "position",
                ConfigValue::Float3([0., 0.12, 0.]),
            )])]),
        )]);
        let offset = battery_collision_offset(&options).unwrap();
        for row in fixture["native_spawners"].as_array().unwrap() {
            let xyz =
                |v: &serde_json::Value| std::array::from_fn(|i| v[i].as_f64().unwrap() as f32);
            let object = named(vec![
                ("position", ConfigValue::Float3(xyz(&row["position"]))),
                ("rotation", ConfigValue::Float3(xyz(&row["rotation"]))),
            ]);
            let result = battery_body_transform(&object, offset).unwrap();
            let ConfigValue::Float3(actual) = result.named("position").unwrap() else {
                panic!()
            };
            let expected: [f32; 3] = xyz(&row["spawned_item"]["position"]);
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 0.001)
            );
        }
    }
    #[test]
    fn relocation_covers_render_collision_material_and_texture_references() {
        let paths = BTreeSet::from([
            "models/item/quest/container.model/settings".into(),
            "textures/a.dds".into(),
            "material_instances/a.material".into(),
        ]);
        let swaps = replacements(&paths, SourceBuild::V021d);
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
