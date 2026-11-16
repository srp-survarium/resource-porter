// SPDX-License-Identifier: GPL-3.0-or-later
//! Another weather of a shipped 0.21 map, as a content pack of its own.
//!
//! In retail 0.21d a level's weathers are one map: `projects/level_03/day`
//! and `rain` have the same `render_visuals`, `collision_objects`,
//! `server_objects` and every client object but the weather's. They differ
//! only in the environment, the objects of the weather's layer
//! (`layer "rain"`: its particles, decals and sounds) and the lighting
//! (probes, ambient lights, lights). [`build`] checks that of the donor, then
//! makes `<weather>_<mode>` of the shipped `day_<mode>` projects (the bytes
//! the packs give players, so every later fix of the day map is kept): the
//! day's weather and lighting objects out, the importer's adapted donor ones
//! in, every other group and object kept byte for byte.
//!
//! What the weather adds is the donor's own: its environment as authored
//! (only the 0.21 conversions the importer manifest opts into and the keys
//! [`Manifest::environment_kept`] names on top), its lighting with its own
//! cubemaps for every key the project loads (the lights 0.26 kept and
//! retuned with 0.26's values), its rain particles, material
//! and textures under `ported/v021d` where the target's differ. The weather
//! shaders are the importer's guarded conversions, built over the bytes
//! players have; they replace the shared native files, so the pack records
//! those bytes and each marked map's marker is checked to be in every
//! converted shader and in no other project.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value as Json};

use crate::content_pack::{self, Operation, PackManifest, sha256_hex};
use crate::survarium_026::{self, LIGHTING_OBJECT_TYPES};
use crate::{BinaryConfig, ConfigValue, NamedValue, ResourceDb};

const NAMESPACE: &str = "ported/v021d";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub pack: PackInfo,
    /// The importer manifest (`build-survarium021-maps`) of the donor
    /// weather: one map, its 0.21 conversions.
    pub import: Json,
    /// The shipped weather the new one is made of (`day`).
    pub from_weather: String,
    /// The new projects: maps_dict id and mode of each.
    pub variants: Vec<Variant>,
    /// A key set to `true` at each new project's root, for the client.
    pub renderer_key: String,
    /// Environment keys the donor does not author kept from the target's
    /// schema fill, each with the reason. Every other filled key is left out
    /// (the 0.26g loader reads each key only if present).
    pub environment_kept: BTreeMap<String, String>,
    /// The probes' `textures` keys, each the probe's donor cubemap.
    pub probe_keys: Vec<String>,
    /// Donor particles (`fx/environment/rain`) the weather's objects place,
    /// imported under `ported/v021d` instead of the target's of that name.
    pub particles: Vec<String>,
    /// Materials the weather's objects and particles use, each with the
    /// textures of it imported under `ported/v021d` (the donor's bytes).
    pub materials: BTreeMap<String, Vec<String>>,
    /// Donor files copied byte for byte to a new path.
    pub donor_copies: Vec<DonorCopy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackInfo {
    pub id: String,
    pub version: String,
    pub description: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: u8,
    pub mode: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DonorCopy {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// The databases: the donor, the stock target, the patches the packs are
/// composed over (`base`, release/content.tsv's base rows) and every patch
/// players have (`live`).
pub struct Inputs<'a> {
    pub donor_db: &'a Path,
    pub target_db: &'a Path,
    pub base_patches: &'a [PathBuf],
    pub live_patches: &'a [PathBuf],
    pub work: &'a Path,
    pub output: &'a Path,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub lines: Vec<String>,
}
impl Summary {
    fn say(&mut self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("{line}");
        self.lines.push(line);
    }
}

fn cfg_bytes(bytes: &[u8], what: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(bytes)
        .with_context(|| format!("decoding {what}"))?
        .root)
}
fn cfg(db: &mut ResourceDb, path: &str) -> Result<ConfigValue> {
    cfg_bytes(&db.read(path)?, path)
}
fn string<'a>(c: &'a ConfigValue, k: &str) -> Option<&'a str> {
    match c.named(k) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}
fn objects(project: &ConfigValue) -> Result<&[ConfigValue]> {
    match project.named("client_objects") {
        Some(ConfigValue::Indexed(v)) => Ok(v),
        _ => bail!("project has no indexed client_objects"),
    }
}
fn objects_mut(project: &mut ConfigValue) -> Result<&mut Vec<ConfigValue>> {
    match project.named_mut("client_objects") {
        Some(ConfigValue::Indexed(v)) => Ok(v),
        _ => bail!("project has no indexed client_objects"),
    }
}
fn kind(o: &ConfigValue) -> &str {
    string(o, "game_object_type").unwrap_or("")
}
fn full_name(o: &ConfigValue) -> &str {
    string(o, "full_name").unwrap_or("?")
}

/// A donor light that the target's native project still has (the same
/// `full_name` within 10 cm of where the layout puts the native one) becomes
/// the native object: 0.26 retuned some 0.21 lights for its renderer. On
/// Vostok the radar building's upper lamp reaches 7 m instead of 10.7, which
/// lit the room below through its floor, and that room's own lamp is 2
/// instead of 0.2. The layout offset is the most common difference of the
/// lights named once on both sides. Returns the names of the lights that
/// changed.
fn native_light_tuning(lights: &mut [ConfigValue], native: &ConfigValue) -> Result<Vec<String>> {
    let position = |o: &ConfigValue| match o.named("position") {
        Some(ConfigValue::Float3(p)) => Some(*p),
        _ => None,
    };
    let native_lights: Vec<&ConfigValue> = objects(native)?
        .iter()
        .filter(|o| kind(o) == "light")
        .collect();
    let once = |list: &[&ConfigValue]| -> BTreeMap<String, [f32; 3]> {
        let mut seen: BTreeMap<String, Option<[f32; 3]>> = BTreeMap::new();
        for o in list {
            seen.entry(full_name(o).to_owned())
                .and_modify(|p| *p = None)
                .or_insert(position(o));
        }
        seen.into_iter()
            .filter_map(|(n, p)| Some((n, p?)))
            .collect()
    };
    let donor_once = once(
        &lights
            .iter()
            .filter(|o| kind(o) == "light")
            .collect::<Vec<_>>(),
    );
    let native_once = once(&native_lights);
    let mut votes: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut pairs = 0;
    for (name, at) in &donor_once {
        if let Some(other) = native_once.get(name) {
            pairs += 1;
            *votes
                .entry([0, 1, 2].map(|i| ((at[i] - other[i]) * 100.).round() as i64))
                .or_default() += 1;
        }
    }
    let (key, count) = votes
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .context("no light named once in both projects")?;
    ensure!(
        count >= 3 && count * 2 >= pairs,
        "the native lights do not match the donor's: the best common offset has {count} of {pairs}"
    );
    let offset = key.map(|v| v as f32 / 100.);
    let native_lights: Vec<ConfigValue> = native_lights
        .into_iter()
        .cloned()
        .map(|mut o| {
            if let Some(ConfigValue::Float3(p)) = o.named_mut("position") {
                for i in 0..3 {
                    p[i] += offset[i];
                }
            }
            o
        })
        .collect();
    let mut retuned = Vec::new();
    for o in lights.iter_mut().filter(|o| kind(o) == "light") {
        let at = position(o).with_context(|| format!("light {} has no position", full_name(o)))?;
        let same: Vec<&ConfigValue> = native_lights
            .iter()
            .filter(|n| {
                full_name(n) == full_name(o)
                    && position(n)
                        .is_some_and(|p| (0..3).map(|i| (p[i] - at[i]).powi(2)).sum::<f32>() < 0.01)
            })
            .collect();
        match same.as_slice() {
            [] => {}
            [n] => {
                if retuned_from(o, n) {
                    retuned.push(full_name(o).to_owned());
                    *o = (*n).clone();
                }
            }
            _ => bail!(
                "light {} is placed twice in the native project",
                full_name(o)
            ),
        }
    }
    Ok(retuned)
}

/// Whether the native object sets a field to another value than the donor's
/// (numbers by value: 0.26 stores some whole floats as integers; fields only
/// the donor has are 0.21's and the 0.26 loader skips them).
fn retuned_from(donor: &ConfigValue, native: &ConfigValue) -> bool {
    fn same(a: &ConfigValue, b: &ConfigValue) -> bool {
        let number = |v: &ConfigValue| match v {
            ConfigValue::Integer(i) => Some(*i as f32),
            ConfigValue::Float(f) => Some(*f),
            _ => None,
        };
        match (a, b, number(a), number(b)) {
            (_, _, Some(x), Some(y)) => (x - y).abs() < 1e-4,
            (ConfigValue::Float3(x), ConfigValue::Float3(y), _, _) => {
                x.iter().zip(y).all(|(x, y)| (x - y).abs() < 1e-4)
            }
            _ => a == b,
        }
    }
    let ConfigValue::Named(fields) = native else {
        return donor != native;
    };
    fields.iter().any(|f| {
        donor
            .named(&f.name)
            .is_none_or(|value| !same(value, &f.value))
    })
}

/// The objects a weather swaps: its environment, its layer's objects, the
/// lighting.
fn is_weather_object(o: &ConfigValue, weather: &str) -> bool {
    kind(o) == "environment"
        || LIGHTING_OBJECT_TYPES.contains(&kind(o))
        || string(o, "layer") == Some(weather)
}

/// A multiset of objects by value (configs have floats: compared by their
/// encoding).
fn multiset(objects: &[&ConfigValue]) -> Result<BTreeMap<Vec<u8>, usize>> {
    let mut out = BTreeMap::new();
    for o in objects {
        *out.entry(BinaryConfig { root: (*o).clone() }.to_bytes()?)
            .or_default() += 1;
    }
    Ok(out)
}

/// 0.21d's `day` and `<weather>` are one map: proves it of the donor, and
/// returns what the weather has of its own (its layer, the environment) and
/// of the lighting (counts, for the summary).
fn check_donor_weathers(
    day: &ConfigValue,
    weather: &ConfigValue,
    from: &str,
    to: &str,
    summary: &mut Summary,
) -> Result<()> {
    let (ConfigValue::Named(a), ConfigValue::Named(b)) = (day, weather) else {
        bail!("donor projects are not tables");
    };
    let names = |v: &[NamedValue]| v.iter().map(|f| f.name.clone()).collect::<BTreeSet<_>>();
    ensure!(
        names(a) == names(b),
        "donor {from} and {to} have other groups"
    );
    for field in a {
        if field.name == "client_objects" {
            continue;
        }
        ensure!(
            weather.named(&field.name) == Some(&field.value),
            "donor {from} and {to} differ in {}",
            field.name
        );
    }
    let split = |p: &ConfigValue, w: &str| -> Result<(Vec<ConfigValue>, Vec<ConfigValue>)> {
        let mut shared = Vec::new();
        let mut own = Vec::new();
        for o in objects(p)? {
            if is_weather_object(o, w) {
                own.push(o.clone());
            } else {
                shared.push(o.clone());
            }
        }
        Ok((shared, own))
    };
    let (shared_day, own_day) = split(day, from)?;
    let (shared_weather, own_weather) = split(weather, to)?;
    ensure!(
        multiset(&shared_day.iter().collect::<Vec<_>>())?
            == multiset(&shared_weather.iter().collect::<Vec<_>>())?,
        "donor {from} and {to} differ in objects other than the weather's and the lighting"
    );
    // The lighting of the default layer is the same placement in both; the
    // probes differ in their cubemaps only.
    let lighting = |own: &[ConfigValue], w: &str| -> Vec<ConfigValue> {
        own.iter()
            .filter(|o| LIGHTING_OBJECT_TYPES.contains(&kind(o)) && string(o, "layer") != Some(w))
            .map(|o| {
                let mut o = o.clone();
                if kind(&o) == "environment_probe"
                    && let ConfigValue::Named(fields) = &mut o
                {
                    fields.retain(|f| f.name != "texture" && f.name != "textures");
                }
                o
            })
            .collect()
    };
    let (light_day, light_weather) = (lighting(&own_day, from), lighting(&own_weather, to));
    ensure!(
        multiset(&light_day.iter().collect::<Vec<_>>())?
            == multiset(&light_weather.iter().collect::<Vec<_>>())?,
        "donor {from} and {to} differ in their default-layer lighting beyond the probes' cubemaps"
    );
    let count = |own: &[ConfigValue], w: &str| -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for o in own.iter().filter(|o| string(o, "layer") == Some(w)) {
            *out.entry(kind(o).to_owned()).or_default() += 1;
        }
        out
    };
    summary.say(format!(
        "donor: {from} and {to} share every group and {} objects; {} lighting objects of the default layer are the same but for the probes' cubemaps; {from}'s layer {:?}, {to}'s layer {:?}",
        shared_day.len(),
        light_day.len(),
        count(&own_day, from),
        count(&own_weather, to)
    ));
    Ok(())
}

/// The environment as the donor authored it: the importer's adapted
/// properties (references relocated, the opted-in 0.21 conversions) without
/// the keys the importer filled from the target's schema, but `kept`.
fn donor_environment(
    adapted: &ConfigValue,
    donor: &ConfigValue,
    converted: &BTreeSet<&str>,
    kept: &BTreeMap<String, String>,
    summary: &mut Summary,
) -> Result<ConfigValue> {
    let (ConfigValue::Named(adapted), ConfigValue::Named(donor)) = (adapted, donor) else {
        bail!("environment properties are not tables");
    };
    let authored: BTreeSet<&str> = donor.iter().map(|f| f.name.as_str()).collect();
    let mut out = Vec::new();
    let mut dropped = Vec::new();
    for field in adapted {
        let name = field.name.as_str();
        if authored.contains(name) {
            let original = &donor.iter().find(|f| f.name == name).unwrap().value;
            if &field.value != original && !converted.contains(name) {
                match (&field.value, original) {
                    (ConfigValue::String(new), ConfigValue::String(old))
                        if new == &format!("{NAMESPACE}/{old}") => {}
                    _ => bail!(
                        "environment {name}: the importer has {:?}, the donor {:?}, and no conversion names it",
                        field.value,
                        original
                    ),
                }
            }
            out.push(field.clone());
        } else if converted.contains(name) || kept.contains_key(name) {
            out.push(field.clone());
        } else {
            dropped.push(name.to_owned());
        }
    }
    for name in kept.keys() {
        ensure!(
            !authored.contains(name.as_str()),
            "environment_kept names {name}, which the donor authors"
        );
        ensure!(
            adapted.iter().any(|f| &f.name == name),
            "environment_kept names {name}, which the importer does not fill"
        );
    }
    for name in converted {
        ensure!(
            out.iter().any(|f| f.name == *name),
            "the 0.21 conversion of {name} is not in the importer's environment"
        );
    }
    summary.say(format!(
        "environment: {} keys: {} the donor's, {} of the 0.21 conversions ({:?}), {} kept fill ({:?}); left out of the target's fill: {dropped:?}",
        out.len(),
        authored.len(),
        converted.iter().filter(|k| !authored.contains(*k)).count(),
        converted,
        kept.len(),
        kept.keys().collect::<Vec<_>>()
    ));
    Ok(ConfigValue::Named(out))
}

/// The environment keys the importer's 0.21 conversions write (see
/// `survarium_026::{prepare_legacy_clouds, mark_legacy_fog}`).
fn converted_keys(map: &survarium_026::Map, donor: &ConfigValue) -> BTreeSet<&'static str> {
    let mut keys = BTreeSet::new();
    if map.legacy_fog_021 {
        keys.insert("mie_fog_base_height");
    }
    if map.legacy_clouds_021 {
        keys.extend([
            "sky_clouds_rotation",
            "stratosphere_rotation",
            "sky_clouds_sunlight",
        ]);
        for key in ["sky_clouds_fog_color", "use_sky_clouds_lighting"] {
            if donor.named(key).is_none() {
                keys.insert(key);
            }
        }
    }
    keys
}

/// The new mode options: the native `<weather>` ones with the substitutions
/// that make the shipped day options of the native day ones. A string the
/// port prefixed (`v021d_`) is the native weather's prefixed; any other
/// change must be at a leaf both native weathers share.
fn weather_options(
    native_day: &ConfigValue,
    shipped_day: &ConfigValue,
    native_weather: &ConfigValue,
    changes: &mut Vec<String>,
) -> Result<ConfigValue> {
    fn walk(
        native_day: &ConfigValue,
        shipped_day: &ConfigValue,
        native: &ConfigValue,
        path: &str,
        changes: &mut Vec<String>,
    ) -> Result<ConfigValue> {
        match (native_day, shipped_day, native) {
            (ConfigValue::Named(a), ConfigValue::Named(b), ConfigValue::Named(c)) => {
                ensure!(
                    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.name == y.name),
                    "{path}: the shipped day options are not the native ones' shape"
                );
                let mut out = c.clone();
                for (x, y) in a.iter().zip(b) {
                    let at = format!("{path}.{}", x.name);
                    let field = out
                        .iter_mut()
                        .find(|f| f.name == x.name)
                        .with_context(|| format!("{at}: the native weather options lack it"))?;
                    field.value = walk(&x.value, &y.value, &field.value, &at, changes)?;
                }
                Ok(ConfigValue::Named(out))
            }
            (ConfigValue::Indexed(a), ConfigValue::Indexed(b), ConfigValue::Indexed(c)) => {
                ensure!(
                    a.len() == b.len() && a.len() == c.len(),
                    "{path}: lists of other lengths"
                );
                let mut out = Vec::new();
                for (i, ((x, y), z)) in a.iter().zip(b).zip(c).enumerate() {
                    out.push(walk(x, y, z, &format!("{path}[{i}]"), changes)?);
                }
                Ok(ConfigValue::Indexed(out))
            }
            (a, b, c) if a == b => Ok(c.clone()),
            (ConfigValue::String(a), ConfigValue::String(b), ConfigValue::String(c))
                if *b == format!("v021d_{a}") =>
            {
                let new = format!("v021d_{c}");
                changes.push(format!("{path}: {c:?} -> {new:?}"));
                Ok(ConfigValue::String(new))
            }
            (a, b, c) => {
                ensure!(
                    a == c,
                    "{path}: the port changed {a:?} to {b:?}, and the native weather has {c:?}"
                );
                changes.push(format!("{path}: {c:?} -> {b:?}"));
                Ok(b.clone())
            }
        }
    }
    let out = walk(native_day, shipped_day, native_weather, "", changes)?;
    ensure!(
        !changes.is_empty(),
        "the shipped day options are the native ones"
    );
    Ok(out)
}

/// Rewrites string leaves equal to a key of `names` at fields named `field`
/// (any field when `None`). Returns how many it rewrote.
fn rename(c: &mut ConfigValue, field: Option<&str>, names: &BTreeMap<String, String>) -> usize {
    fn inner(
        c: &mut ConfigValue,
        here: Option<&str>,
        field: Option<&str>,
        names: &BTreeMap<String, String>,
    ) -> usize {
        match c {
            ConfigValue::String(s) => {
                if (field.is_none() || field == here)
                    && let Some(new) = names.get(s.as_str())
                {
                    *s = new.clone();
                    return 1;
                }
                0
            }
            ConfigValue::Named(v) => v
                .iter_mut()
                .map(|f| inner(&mut f.value, Some(&f.name), field, names))
                .sum(),
            ConfigValue::Indexed(v) => v.iter_mut().map(|e| inner(e, here, field, names)).sum(),
            _ => 0,
        }
    }
    inner(c, None, field, names)
}

fn relocated(name: &str) -> String {
    format!("{NAMESPACE}/{name}")
}

/// The f32 marker's little-endian bytes appear in the shader blob.
fn contains_marker(blob: &[u8], marker: f32) -> bool {
    let needle = marker.to_le_bytes();
    blob.windows(4).any(|w| w == needle)
}

/// `mie_fog_base_height` of a project's environments.
fn markers_of(project: &ConfigValue) -> Vec<f32> {
    let Ok(objects) = objects(project) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter(|o| kind(o) == "environment")
        .filter_map(
            |o| match o.named("properties")?.named("mie_fog_base_height")? {
                ConfigValue::Float(v) => Some(*v),
                ConfigValue::Integer(v) => Some(*v as f32),
                _ => None,
            },
        )
        .collect()
}

struct Bundle {
    root: PathBuf,
    record: Json,
}
impl Bundle {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let file = crate::archive::safe_join(&self.root.join("overlay"), path)?;
        fs::read(&file).with_context(|| format!("the importer emitted no {path}"))
    }
    /// The importer's converted shaders (`0.21d-...` sources).
    fn shaders(&self) -> Result<Vec<String>> {
        Ok(self.record["files"]
            .as_array()
            .context("the importer record has no files")?
            .iter()
            .filter_map(|f| {
                let path = f["path"].as_str()?;
                path.starts_with("shaders/").then(|| path.to_owned())
            })
            .collect())
    }
    fn native_material(&self, path: &str) -> bool {
        self.record["native_materials"]
            .as_array()
            .is_some_and(|v| v.iter().any(|p| p == path))
    }
}

pub fn build(manifest_path: &Path, inputs: &Inputs<'_>) -> Result<Summary> {
    let m: Manifest = serde_json::from_reader(
        fs::File::open(manifest_path)
            .with_context(|| format!("opening {}", manifest_path.display()))?,
    )
    .with_context(|| format!("reading {}", manifest_path.display()))?;
    let import: survarium_026::Manifest = serde_json::from_value(m.import.clone())
        .context("the import manifest is not an importer manifest")?;
    let [map] = import.maps.as_slice() else {
        bail!("the import manifest must have one map, the donor weather")
    };
    ensure!(
        !map.native_lighting,
        "the weather keeps the donor's lighting"
    );
    ensure!(
        map.weather != m.from_weather,
        "the import is the shipped weather itself"
    );
    let weather = map.weather.clone();
    let level = map.level.clone();
    ensure!(!m.variants.is_empty(), "no variants");
    for v in &m.variants {
        ensure!(matches!(v.mode, 2..=5), "mode {} of {}", v.mode, v.id);
    }
    let live_names: BTreeSet<_> = inputs
        .live_patches
        .iter()
        .map(|p| p.file_name().map(|n| n.to_owned()))
        .collect();
    for p in inputs.base_patches {
        ensure!(
            live_names.contains(&p.file_name().map(|n| n.to_owned())),
            "base patch {} is not one of the live patches",
            p.display()
        );
    }
    let mut summary = Summary::default();
    let mut donor = ResourceDb::open(inputs.donor_db)?;
    let mut target = ResourceDb::open(inputs.target_db)?;
    let mut base = ResourceDb::open_layers(&content_pack::patch::base_layers(
        inputs.target_db,
        inputs.base_patches,
    )?)?;
    let mut live = ResourceDb::open_layers(&content_pack::patch::base_layers(
        inputs.target_db,
        inputs.live_patches,
    )?)?;

    // 1. The importer's donor weather, its shaders over the live bytes.
    fs::create_dir_all(inputs.work)?;
    let import_path = inputs.work.join("import.json");
    fs::write(&import_path, serde_json::to_vec_pretty(&m.import)?)?;
    let bundle_root = inputs.work.join("bundle");
    survarium_026::build(
        &import_path,
        inputs.donor_db,
        inputs.target_db,
        inputs.live_patches,
        &bundle_root,
    )
    .context("importing the donor weather")?;
    let bundle = Bundle {
        record: serde_json::from_slice(&fs::read(bundle_root.join("port-manifest.json"))?)?,
        root: bundle_root,
    };
    let adapted_path = format!("projects/v021d_{level}/{weather}_2/client_project");
    let adapted = cfg_bytes(&bundle.read(&adapted_path)?, &adapted_path)?;

    // 2. The donor's weathers are one map.
    let donor_day = cfg(
        &mut donor,
        &format!("projects/{level}/{}/client_project", m.from_weather),
    )?;
    let donor_weather = cfg(
        &mut donor,
        &format!("projects/{level}/{weather}/client_project"),
    )?;
    check_donor_weathers(
        &donor_day,
        &donor_weather,
        &m.from_weather,
        &weather,
        &mut summary,
    )?;

    // 3. The weather's objects, as the new projects take them.
    let donor_env = objects(&donor_weather)?
        .iter()
        .find(|o| kind(o) == "environment")
        .and_then(|o| o.named("properties"))
        .context("the donor weather has no environment")?
        .clone();
    let mut converted: BTreeSet<&str> = converted_keys(map, &donor_env);
    converted.extend(map.environment_overrides.keys().map(String::as_str));
    let particle_names: BTreeMap<String, String> = m
        .particles
        .iter()
        .map(|p| (p.clone(), relocated(p)))
        .collect();
    let material_names: BTreeMap<String, String> = m
        .materials
        .keys()
        .map(|p| (p.clone(), relocated(p)))
        .collect();
    let mut swapped = Vec::new();
    let mut cubemaps = BTreeSet::new();
    let (mut particles_placed, mut materials_named) = (0, 0);
    for o in objects(&adapted)? {
        if !is_weather_object(o, &weather) {
            continue;
        }
        let mut o = o.clone();
        let object_kind = kind(&o).to_owned();
        match object_kind.as_str() {
            "environment" => {
                let properties = donor_environment(
                    o.named("properties")
                        .context("environment without properties")?,
                    &donor_env,
                    &converted,
                    &m.environment_kept,
                    &mut summary,
                )?;
                o.set_path("properties", properties)?;
            }
            "environment_probe" => {
                let key = format!("{weather}_2");
                let cubemap = o
                    .named("textures")
                    .and_then(|t| string(t, &key))
                    .with_context(|| format!("probe {} has no {key}", full_name(&o)))?
                    .to_owned();
                ensure!(
                    cubemap.starts_with(&format!("{NAMESPACE}/environment_probes/")),
                    "probe {}: {key} is {cubemap}, not the donor's",
                    full_name(&o)
                );
                o.set_path(
                    "textures",
                    ConfigValue::Named(
                        m.probe_keys
                            .iter()
                            .map(|k| NamedValue {
                                name: k.clone(),
                                value: ConfigValue::String(cubemap.clone()),
                            })
                            .collect(),
                    ),
                )?;
                o.set_path("texture", ConfigValue::String(cubemap.clone()))?;
                cubemaps.insert(cubemap);
            }
            "particle" => particles_placed += rename(&mut o, Some("lib_name"), &particle_names),
            _ => materials_named += rename(&mut o, None, &material_names),
        }
        swapped.push(o);
    }
    let native_path = format!("projects/{level}/{weather}_2/client_project");
    let native = cfg(&mut target, &native_path)?;
    let retuned = native_light_tuning(&mut swapped, &native)?;
    summary.say(format!(
        "{weather}: the donor lights 0.26 retuned take {native_path}'s: {retuned:?}"
    ));
    summary.say(format!(
        "{weather}: {} objects of the weather and lighting ({}), {} cubemaps; {particles_placed} particles and {materials_named} material references moved to {NAMESPACE}",
        swapped.len(),
        {
            let mut by = BTreeMap::<&str, usize>::new();
            for o in &swapped {
                *by.entry(kind(o)).or_default() += 1;
            }
            format!("{by:?}")
        },
        cubemaps.len()
    ));
    ensure!(
        particles_placed > 0,
        "no placed particle is one of {:?}",
        m.particles
    );
    let markers: BTreeSet<u32> = swapped
        .iter()
        .filter(|o| kind(o) == "environment")
        .flat_map(|o| {
            markers_of(&ConfigValue::Named(vec![NamedValue {
                name: "client_objects".into(),
                value: ConfigValue::Indexed(vec![o.clone()]),
            }]))
        })
        .map(f32::to_bits)
        .collect();
    let marker = if map.legacy_fog_021 {
        let expected = -32021.0 - f32::from(map.id);
        ensure!(
            markers == BTreeSet::from([expected.to_bits()]),
            "the environment's marker is not {expected}"
        );
        Some(expected)
    } else {
        None
    };

    // 4. The new files.
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    fn add(files: &mut BTreeMap<String, Vec<u8>>, path: String, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            files.insert(path.clone(), bytes).is_none(),
            "{path} made twice"
        );
        Ok(())
    }
    let mut dictionary = Vec::new();
    for v in &m.variants {
        let day_path = format!(
            "projects/v021d_{level}/{}_{}/client_project",
            m.from_weather, v.mode
        );
        let day_bytes = base.read(&day_path)?;
        let day = cfg_bytes(&day_bytes, &day_path)?;
        ensure!(
            BinaryConfig { root: day.clone() }.to_bytes()? == day_bytes,
            "{day_path} does not encode back to its bytes"
        );
        let mut project = day.clone();
        let list = objects_mut(&mut project)?;
        let before = list.len();
        let removed: Vec<ConfigValue> = list
            .iter()
            .filter(|o| is_weather_object(o, &m.from_weather))
            .cloned()
            .collect();
        list.retain(|o| !is_weather_object(o, &m.from_weather));
        let kept = list.len();
        let weather_only: Vec<&str> = removed
            .iter()
            .filter(|o| !LIGHTING_OBJECT_TYPES.contains(&kind(o)))
            .map(kind)
            .collect();
        ensure!(
            weather_only == ["environment"],
            "{day_path}: the day's weather objects besides the lighting are {weather_only:?}, not its environment"
        );
        list.extend(swapped.iter().cloned());
        ensure!(
            project.named(&m.renderer_key).is_none(),
            "{day_path} has {} already",
            m.renderer_key
        );
        project.set_path(&m.renderer_key, ConfigValue::Boolean(true))?;
        // Everything else is the shipped day's, byte for byte.
        let (ConfigValue::Named(a), ConfigValue::Named(b)) = (&day, &project) else {
            unreachable!()
        };
        for field in a {
            if field.name != "client_objects" {
                ensure!(
                    project.named(&field.name) == Some(&field.value),
                    "{}: changed",
                    field.name
                );
            }
        }
        ensure!(b.len() == a.len() + 1, "the new project has other groups");
        let kept_day: Vec<&ConfigValue> = objects(&day)?
            .iter()
            .filter(|o| !is_weather_object(o, &m.from_weather))
            .collect();
        let kept_new: Vec<&ConfigValue> = objects(&project)?[..kept].iter().collect();
        ensure!(kept_day == kept_new, "kept objects changed");
        let path = format!("projects/v021d_{level}/{weather}_{}/client_project", v.mode);
        summary.say(format!(
            "{path} ({}): {kept} of {before} objects of {day_path} kept as they are, {} of its lighting and environment out, {} in; every other group as shipped; {} = true",
            v.id,
            removed.len(),
            swapped.len(),
            m.renderer_key
        ));
        add(&mut files, path, BinaryConfig { root: project }.to_bytes()?)?;

        // The options.
        let options = |w: &str, mode: u8| {
            format!(
                "{level}_{}_{mode}.options",
                if w == "evening" { "evn" } else { w }
            )
        };
        let native_day = cfg(
            &mut target,
            &format!("gameplay/game_modes/{}", options(&m.from_weather, v.mode)),
        )?;
        let shipped_day = cfg(
            &mut base,
            &format!(
                "gameplay/game_modes/v021d_{}",
                options(&m.from_weather, v.mode)
            ),
        )?;
        let native_weather = cfg(
            &mut target,
            &format!("gameplay/game_modes/{}", options(&weather, v.mode)),
        )?;
        let mut changes = Vec::new();
        let new = weather_options(&native_day, &shipped_day, &native_weather, &mut changes)?;
        let map_name = format!("v021d_{level}/{weather}_{}", v.mode);
        ensure!(
            new.named("mode")
                .and_then(|m| m.named("match_making_params"))
                .and_then(|p| string(p, "map_name"))
                == Some(map_name.as_str()),
            "the new options do not name {map_name}"
        );
        let name = format!("v021d_{}", options(&weather, v.mode));
        summary.say(format!(
            "gameplay/game_modes/{name}: native {} with {changes:?}",
            options(&weather, v.mode)
        ));
        add(
            &mut files,
            format!("gameplay/game_modes/{name}"),
            BinaryConfig { root: new }.to_bytes()?,
        )?;
        let mut entry = Map::new();
        entry.insert("cfg_mode_name".into(), Json::String(name));
        entry.insert("id".into(), Json::from(v.id));
        dictionary.push(entry);
    }

    // The probes' cubemaps (each `.dds`, `.options` and its `_diffuse`).
    for cubemap in &cubemaps {
        for suffix in ["", "_diffuse"] {
            for extension in ["dds", "options"] {
                let path = format!("textures/{cubemap}{suffix}.{extension}");
                add(&mut files, path.clone(), bundle.read(&path)?)?;
            }
        }
    }
    // The particles, the donor's, as the importer adapts a donor particle.
    let defaults = survarium_026::particle_defaults(&mut target)?;
    for name in &m.particles {
        let path = format!("particles/{name}.particle");
        let mut c = cfg(&mut donor, &path)?;
        survarium_026::adapt_particle(&mut c, &defaults)?;
        let n = rename(&mut c, Some("Material"), &material_names);
        ensure!(n > 0, "{path} uses none of the moved materials");
        add(
            &mut files,
            format!("particles/{}.particle", relocated(name)),
            BinaryConfig { root: c }.to_bytes()?,
        )?;
    }
    // The materials, with their textures the donor's.
    for (name, textures) in &m.materials {
        let ported = format!("material_instances/{}.material", relocated(name));
        let native = format!("material_instances/{name}.material");
        let bytes = match bundle.read(&ported) {
            Ok(bytes) => bytes,
            Err(_) => {
                ensure!(
                    bundle.native_material(&native),
                    "the importer has neither {ported} nor {native} as its conversion"
                );
                base.read(&native)?
            }
        };
        let mut c = cfg_bytes(&bytes, &ported)?;
        let names: BTreeMap<String, String> =
            textures.iter().map(|t| (t.clone(), relocated(t))).collect();
        let n = rename(&mut c, None, &names);
        ensure!(n > 0, "{name} binds none of {textures:?}");
        add(&mut files, ported, BinaryConfig { root: c }.to_bytes()?)?;
        for texture in textures {
            for extension in ["dds", "options"] {
                let from = format!("textures/{texture}.{extension}");
                add(
                    &mut files,
                    format!("textures/{}.{extension}", relocated(texture)),
                    donor.read(&from)?,
                )?;
            }
        }
    }
    for copy in &m.donor_copies {
        let bytes = donor.read(&copy.from)?;
        if let Some(sha) = &copy.sha256 {
            ensure!(
                &sha256_hex(&bytes) == sha,
                "{}: sha256 {} not {sha}",
                copy.from,
                sha256_hex(&bytes)
            );
        }
        add(&mut files, copy.to.clone(), bytes)?;
    }
    for path in files.keys() {
        ensure!(
            !live.contains(path),
            "{path} is a live file; the pack only adds files"
        );
        ensure!(!base.contains(path), "{path} is a base file");
    }

    // The converted shaders replace the shared files players have.
    let mut replaced = BTreeMap::new();
    for path in bundle.shaders()? {
        let bytes = bundle.read(&path)?;
        let now = live.read(&path)?;
        ensure!(
            base.read(&path)? == now,
            "{path}: the base and players' bytes differ"
        );
        if let Some(marker) = marker {
            ensure!(
                contains_marker(&bytes, marker),
                "{path} has no marker {marker}"
            );
        }
        replaced.insert(path, (sha256_hex(&now), bytes));
    }
    summary.say(format!(
        "{} weather shaders converted over players' bytes, each with marker {:?}",
        replaced.len(),
        marker
    ));
    // No other project carries the marker: unmarked maps keep the native path.
    if let Some(marker) = marker {
        let projects: Vec<String> = live
            .paths_with_prefix("projects/")
            .filter(|p| p.ends_with("/client_project") && live.is_file(p))
            .map(str::to_owned)
            .collect();
        for path in &projects {
            let project = cfg(&mut live, path)?;
            ensure!(
                !markers_of(&project).contains(&marker),
                "{path} carries the marker {marker} already"
            );
        }
        summary.say(format!(
            "marker {marker}: in none of the {} projects players have",
            projects.len()
        ));
    }

    // maps_dict.
    let registered = cfg(&mut live, content_pack::DICTIONARY)?;
    if let Some(ConfigValue::Indexed(entries)) = registered.named("maps_dict") {
        for entry in entries {
            if let Some(ConfigValue::Integer(id)) = entry.named("id") {
                ensure!(
                    !m.variants.iter().any(|v| i32::from(v.id) == *id),
                    "maps_dict {id} is taken"
                );
            }
        }
    }

    // 5. Every reference resolves over what players have, and what the
    //    weather adds is the donor's art.
    let check = inputs.work.join("check.patch");
    if check.exists() {
        fs::remove_file(&check)?;
    }
    let mut overlay: BTreeMap<String, Vec<u8>> = files.clone();
    for (path, (_, bytes)) in &replaced {
        overlay.insert(path.clone(), bytes.clone());
    }
    let only = format!("projects/v021d_{level}/__{weather}_objects/client_project");
    overlay.insert(
        only.clone(),
        BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "client_objects".into(),
                value: ConfigValue::Indexed(swapped.clone()),
            }]),
        }
        .to_bytes()?,
    );
    crate::archive::write_patch(&overlay, &check)?;
    let mut layers = content_pack::patch::base_layers(inputs.target_db, inputs.live_patches)?;
    layers.push(check.clone());
    let mut composed = ResourceDb::open_layers(&layers)?;
    let mut composed2 = ResourceDb::open_layers(&layers)?;
    let roots: Vec<String> = files
        .keys()
        .filter(|p| p.ends_with("/client_project"))
        .cloned()
        .collect();
    let plan = crate::dependency::scan(&mut composed, &mut composed2, roots.clone())?;
    ensure!(
        plan.unresolved_references.is_empty(),
        "unresolved references: {:?}",
        plan.unresolved_references
    );
    let missing: Vec<_> = content_pack::probes::missing_in_db(&mut composed)?
        .into_iter()
        .filter(|p| roots.contains(&p.project))
        .collect();
    ensure!(missing.is_empty(), "probe cubemaps missing: {missing:?}");
    summary.say(format!(
        "references: the new projects and options resolve over players' files ({} files), every probe cubemap is there",
        plan.donor_files.len() + plan.target_files.len()
    ));
    let weather_plan = crate::dependency::scan(&mut composed, &mut composed2, [only.clone()])?;
    ensure!(
        weather_plan.unresolved_references.is_empty(),
        "the weather's objects: unresolved {:?}",
        weather_plan.unresolved_references
    );
    let mut art = 0;
    let mut not_donor = Vec::new();
    for path in weather_plan
        .donor_files
        .iter()
        .chain(&weather_plan.target_files)
    {
        if !(path.ends_with(".dds") || path.ends_with(".ogg")) {
            continue;
        }
        let original = path.replacen(&format!("/{NAMESPACE}/"), "/", 1);
        if !donor.is_file(&original) {
            not_donor.push(path.clone());
            continue;
        }
        art += 1;
        ensure!(
            composed.read(path)? == donor.read(&original)?,
            "{path}: the weather's objects use bytes other than the donor's {original}"
        );
    }
    summary.say(format!(
        "the weather's objects: {art} textures and sounds, each the donor's bytes; {} the donor lacks: {not_donor:?}",
        not_donor.len()
    ));
    fs::remove_file(&check)?;

    // 6. The pack.
    ensure!(
        !inputs.output.exists(),
        "{} exists; never overwritten",
        inputs.output.display()
    );
    let mut operations = Vec::new();
    for (path, bytes) in &files {
        let file = crate::archive::safe_join(&inputs.output.join("files"), path)?;
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, bytes)?;
        operations.push(Operation::AddFile {
            path: path.clone(),
            source: None,
        });
    }
    for (path, (base_sha256, bytes)) in &replaced {
        let file = crate::archive::safe_join(&inputs.output.join("files"), path)?;
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, bytes)?;
        operations.push(Operation::ReplaceFile {
            path: path.clone(),
            source: None,
            base_sha256: base_sha256.clone(),
        });
    }
    operations.push(Operation::DictionaryAdd {
        table: "maps_dict".into(),
        id_field: None,
        entries: dictionary,
    });
    let pack = PackManifest {
        format_version: 1,
        id: m.pack.id.clone(),
        version: m.pack.version.clone(),
        target_build: content_pack::TARGET_BUILD.into(),
        description: Some(m.pack.description.clone()),
        requires: Vec::new(),
        base_db_sha256: Some(import.target_sha256.clone()),
        operations,
        server: Some(serde_json::json!({
            "maps": m.variants.iter().map(|v| serde_json::json!({"id": v.id})).collect::<Vec<_>>()
        })),
        notes: vec![format!(
            "made by resource-porter vostok-port build-021d-weather --manifest {} from the shipped {} projects (the base patches {}) and the donor's {weather}",
            manifest_path.display(),
            m.from_weather,
            inputs
                .base_patches
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )],
    };
    fs::write(
        inputs.output.join("pack.json"),
        serde_json::to_vec_pretty(&pack)?,
    )?;
    summary.say(format!(
        "pack {}@{}: {} new files, {} replaced shaders, maps_dict {:?}",
        m.pack.id,
        m.pack.version,
        files.len(),
        replaced.len(),
        m.variants.iter().map(|v| v.id).collect::<Vec<_>>()
    ));
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> ConfigValue {
        ConfigValue::String(v.into())
    }
    fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            fields
                .into_iter()
                .map(|(n, value)| NamedValue {
                    name: n.into(),
                    value,
                })
                .collect(),
        )
    }

    #[test]
    fn options_keep_the_weather_and_the_port_substitutions() {
        let opts = |name: &str, map: &str, key: &str, id: i32, screen: i32| {
            named(vec![(
                "mode",
                named(vec![
                    ("name", s(name)),
                    ("map_name", s(map)),
                    ("map_key", s(key)),
                    ("map_id", ConfigValue::Integer(id)),
                    ("loading_screen_id", ConfigValue::Integer(screen)),
                ]),
            )])
        };
        let native_day = opts("level_03_2", "level_03/day_2", "st_level_03_name", 3, 2);
        let shipped = opts(
            "v021d_level_03_2",
            "v021d_level_03/day_2",
            "st_level_10_name",
            10,
            2,
        );
        let native_rain = opts(
            "level_03_rain_2",
            "level_03/rain_2",
            "st_level_03_name",
            3,
            4,
        );
        let mut changes = Vec::new();
        let out = weather_options(&native_day, &shipped, &native_rain, &mut changes).unwrap();
        assert_eq!(
            out,
            opts(
                "v021d_level_03_rain_2",
                "v021d_level_03/rain_2",
                "st_level_10_name",
                10,
                4
            )
        );
        assert_eq!(changes.len(), 4);
        // A port change at a leaf the weathers do not share is refused.
        let shipped_screen = opts(
            "v021d_level_03_2",
            "v021d_level_03/day_2",
            "st_level_10_name",
            10,
            9,
        );
        assert!(
            weather_options(&native_day, &shipped_screen, &native_rain, &mut Vec::new()).is_err()
        );
    }

    #[test]
    fn environment_drops_the_fill_and_keeps_conversions() {
        let donor = named(vec![
            ("use_rain", ConfigValue::Boolean(true)),
            ("sky_clouds_texture", s("sky/sky2")),
        ]);
        let adapted = named(vec![
            ("use_rain", ConfigValue::Boolean(true)),
            ("sky_clouds_texture", s("ported/v021d/sky/sky2")),
            ("sun_size", ConfigValue::Float(0.55)),
            ("mie_fog_base_height", ConfigValue::Float(-32189.0)),
            ("use_bokeh_template_image", ConfigValue::Boolean(false)),
        ]);
        let kept = BTreeMap::from([("use_bokeh_template_image".to_owned(), "why".to_owned())]);
        let out = donor_environment(
            &adapted,
            &donor,
            &BTreeSet::from(["mie_fog_base_height"]),
            &kept,
            &mut Summary::default(),
        )
        .unwrap();
        let ConfigValue::Named(fields) = out else {
            panic!()
        };
        let names: Vec<_> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "use_rain",
                "sky_clouds_texture",
                "mie_fog_base_height",
                "use_bokeh_template_image"
            ]
        );
        // A donor value the importer changed without a conversion is refused.
        let changed = named(vec![
            ("use_rain", ConfigValue::Boolean(false)),
            ("sky_clouds_texture", s("ported/v021d/sky/sky2")),
        ]);
        assert!(
            donor_environment(
                &changed,
                &donor,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &mut Summary::default()
            )
            .is_err()
        );
    }

    #[test]
    fn rename_only_the_named_field() {
        let mut c = named(vec![
            ("lib_name", s("fx/environment/rain")),
            ("other", s("fx/environment/rain")),
        ]);
        let names = BTreeMap::from([(
            "fx/environment/rain".to_owned(),
            "ported/v021d/fx/environment/rain".to_owned(),
        )]);
        assert_eq!(rename(&mut c, Some("lib_name"), &names), 1);
        assert_eq!(c.named("other"), Some(&s("fx/environment/rain")));
    }

    #[test]
    fn lights_0_26_kept_take_its_tuning() {
        let light = |name: &str, at: [f32; 3], range: f32| {
            named(vec![
                ("game_object_type", s("light")),
                ("full_name", s(name)),
                ("position", ConfigValue::Float3(at)),
                ("range", ConfigValue::Float(range)),
            ])
        };
        // The native layout is 2 m higher, as 0.26 level_06 is.
        let native = named(vec![(
            "client_objects",
            ConfigValue::Indexed(vec![
                light("upper", [0.0, 2.0625, 0.0], 7.0),
                light("a", [5.0, 2.0, 0.0], 10.7),
                light("b", [6.0, 2.0, 0.0], 10.7),
                // The same 10 m, stored as an integer.
                named(vec![
                    ("game_object_type", s("light")),
                    ("full_name", s("c")),
                    ("position", ConfigValue::Float3([7.0, 2.0, 0.0])),
                    ("range", ConfigValue::Integer(10)),
                ]),
                light("moved", [9.0, 2.0, 0.0], 3.0),
            ]),
        )]);
        let mut c = light("c", [7.0, 0.0, 0.0], 10.0);
        // A 0.21 field the 0.26 loader skips.
        c.set_path("offset", ConfigValue::Float3([0.0, -0.265, 0.0]))
            .unwrap();
        let mut lights = vec![
            light("upper", [0.0, 0.0, 0.0], 10.7),
            light("a", [5.0, 0.0, 0.0], 10.7),
            light("b", [6.0, 0.0, 0.0], 10.7),
            c,
            light("moved", [0.0, 0.0, 9.0], 10.7),
            light("sun", [1.0, 1.0, 1.0], 300.0),
        ];
        let before = lights.clone();
        let retuned = native_light_tuning(&mut lights, &native).unwrap();
        assert_eq!(retuned, ["upper"]);
        assert_eq!(lights[0], light("upper", [0.0, 0.0625, 0.0], 7.0));
        assert_eq!(lights[1..], before[1..]);
    }

    #[test]
    fn marker_bytes_are_found() {
        let marker = -32189.0_f32;
        let mut blob = vec![0_u8; 9];
        blob.extend(marker.to_le_bytes());
        assert!(contains_marker(&blob, marker));
        assert!(!contains_marker(&blob[..10], marker));
    }
}
