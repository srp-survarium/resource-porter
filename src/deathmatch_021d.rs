//! Team Deathmatch (mode 4) variants of the 0.21 maps of a content pack made
//! by `build-survarium021-maps` and `pack from-bundle`.
//!
//! A native 0.26g level has one project per weather and mode
//! (`projects/level_03/day_2`, `day_4`); the TDM one is the battery one
//! without the battery game: its `server_objects` keep only
//! `respawn_points`, and the collectors' props, their zones and the bases'
//! effect zones are gone. The 0.21 port's battery project
//! (`projects/v021d_<level>/<weather>_2`) gets the same treatment:
//! - `server_objects`: only the groups of the native TDM project (the
//!   respawn points, the donor's, as they are);
//! - the collectors' visuals (`render_visuals` and `collision_objects` of
//!   their model) go, with their zones (`collision_geometries`);
//! - the bases' `effect_zone`s go, with their zones, as natively;
//! - a zone another object still names stays.
//!
//! The options are the native TDM ones (`<level>_<w>_4.options`) with the
//! substitutions that make the port's battery options of the native battery
//! ones (its name key, its map number, its project, its mode key), each
//! checked to be one of those. The maps' environment probes (the native
//! lighting) already carry the `<weather>_4` cubemaps.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::{BinaryConfig, ConfigValue, ResourceDb};

/// One variant: the 0.21 source level and weather the pack has as a battery
/// map, and the maps_dict id of its TDM one.
#[derive(Clone, Debug)]
pub struct Variant {
    pub level: String,
    pub weather: String,
    pub id: u8,
}

impl std::str::FromStr for Variant {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let mut it = s.split(':');
        let (Some(level), Some(weather), Some(id), None) =
            (it.next(), it.next(), it.next(), it.next())
        else {
            bail!("a variant is <level>:<weather>:<maps_dict id>, e.g. level_03:day:152");
        };
        Ok(Self {
            level: level.into(),
            weather: weather.into(),
            id: id.parse()?,
        })
    }
}

impl Variant {
    fn short_weather(&self) -> &str {
        if self.weather == "evening" {
            "evn"
        } else {
            &self.weather
        }
    }
    fn project(&self, mode: u8) -> String {
        format!(
            "projects/v021d_{}/{}_{mode}/client_project",
            self.level, self.weather
        )
    }
    fn options(&self, mode: u8) -> String {
        format!(
            "v021d_{}_{}_{mode}.options",
            self.level,
            self.short_weather()
        )
    }
    fn native_options(&self, mode: u8) -> String {
        format!(
            "gameplay/game_modes/{}_{}_{mode}.options",
            self.level,
            self.short_weather()
        )
    }
    fn native_project(&self, mode: u8) -> String {
        format!(
            "projects/{}/{}_{mode}/client_project",
            self.level, self.weather
        )
    }
}

#[derive(Debug, Default)]
pub struct Summary {
    pub id: u8,
    pub project: String,
    pub options: String,
    pub respawn_points: usize,
    pub removed_server_groups: Vec<String>,
    pub removed_visuals: usize,
    pub removed_collision_objects: usize,
    pub removed_zones: Vec<String>,
    pub removed_effect_zones: usize,
    pub options_changes: Vec<String>,
}

fn string<'a>(c: &'a ConfigValue, k: &str) -> Option<&'a str> {
    match c.named(k) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}
fn indexed(c: &ConfigValue) -> &[ConfigValue] {
    match c {
        ConfigValue::Indexed(v) => v,
        _ => &[],
    }
}
fn zones_of(object: &ConfigValue) -> Vec<String> {
    object
        .named("collision_geometries")
        .map(indexed)
        .unwrap_or_default()
        .iter()
        .filter_map(|z| string(z, "name").map(str::to_owned))
        .collect()
}
fn all_strings(c: &ConfigValue) -> Vec<&str> {
    let mut out = Vec::new();
    c.collect_strings(&mut out);
    out
}

/// The TDM project of a port's battery project.
pub fn deathmatch_project(
    battery: &ConfigValue,
    native_tdm: &ConfigValue,
    summary: &mut Summary,
) -> Result<ConfigValue> {
    let mut project = battery.clone();
    let keep: BTreeSet<String> = match native_tdm.named("server_objects") {
        Some(ConfigValue::Named(groups)) => groups.iter().map(|g| g.name.clone()).collect(),
        _ => bail!("the native TDM project has no server_objects"),
    };
    ensure!(
        keep.contains("respawn_points"),
        "the native TDM project has no respawn_points"
    );
    let Some(ConfigValue::Named(groups)) = project.named("server_objects") else {
        bail!("the battery project has no server_objects");
    };
    let collectors: Vec<ConfigValue> = groups
        .iter()
        .find(|g| g.name == "victory_items_containers")
        .map(|g| indexed(&g.value).to_vec())
        .context("the battery project has no victory_items_containers")?;
    ensure!(
        collectors.len() == 2,
        "{} collectors, not 2",
        collectors.len()
    );
    let models: BTreeSet<&str> = collectors
        .iter()
        .filter_map(|c| {
            c.named("visual_geometry")
                .and_then(|v| string(v, "model_name"))
        })
        .collect();
    ensure!(models.len() == 1, "the collectors' models: {models:?}");
    let model = models.into_iter().next().unwrap().to_owned();
    let mut zones: BTreeSet<String> = collectors.iter().flat_map(zones_of).collect();
    // server_objects: the native TDM project's groups only.
    let Some(ConfigValue::Named(groups)) = project.named_mut("server_objects") else {
        unreachable!()
    };
    summary.removed_server_groups = groups
        .iter()
        .filter(|g| !keep.contains(&g.name))
        .map(|g| g.name.clone())
        .collect();
    groups.retain(|g| keep.contains(&g.name));
    summary.respawn_points = groups
        .iter()
        .find(|g| g.name == "respawn_points")
        .map(|g| indexed(&g.value).len())
        .unwrap_or(0);
    ensure!(summary.respawn_points > 0, "no respawn points");
    // The bases' effect zones (native TDM has none).
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        bail!("the battery project has no client_objects");
    };
    let before = objects.len();
    objects.retain(|o| {
        if string(o, "game_object_type") == Some("effect_zone") {
            zones.extend(zones_of(o));
            false
        } else {
            true
        }
    });
    summary.removed_effect_zones = before - objects.len();
    // The collectors' props.
    let prefix = format!("resources/models/{model}.model/");
    if let Some(ConfigValue::Indexed(visuals)) = project.named_mut("render_visuals") {
        let before = visuals.len();
        visuals.retain(|v| string(v, "lib_name") != Some(model.as_str()));
        summary.removed_visuals = before - visuals.len();
    }
    if let Some(ConfigValue::Indexed(collisions)) = project.named_mut("collision_objects") {
        let before = collisions.len();
        collisions.retain(|c| !all_strings(c).iter().any(|s| s.starts_with(&prefix)));
        summary.removed_collision_objects = before - collisions.len();
    }
    ensure!(
        summary.removed_visuals == 2,
        "{} visuals of {model}, not the two collectors'",
        summary.removed_visuals
    );
    // The zones nothing names any more.
    let named = |project: &ConfigValue, zone: &str| -> usize {
        let Some(objects) = project.named("client_objects") else {
            return 0;
        };
        indexed(objects)
            .iter()
            .filter(|o| string(o, "full_name") != Some(zone))
            .map(|o| all_strings(o).into_iter().filter(|s| *s == zone).count())
            .sum::<usize>()
            + project
                .named("server_objects")
                .map(|s| all_strings(s).into_iter().filter(|s| *s == zone).count())
                .unwrap_or(0)
    };
    let unused: BTreeSet<String> = zones
        .iter()
        .filter(|z| named(&project, z) == 0)
        .cloned()
        .collect();
    let Some(ConfigValue::Indexed(objects)) = project.named_mut("client_objects") else {
        unreachable!()
    };
    let before = objects.len();
    objects.retain(|o| {
        !(string(o, "game_object_type") == Some("collision_geometry")
            && string(o, "full_name").is_some_and(|n| unused.contains(n)))
    });
    ensure!(
        before - objects.len() == unused.len(),
        "{} zones named by no object, {} removed",
        unused.len(),
        before - objects.len()
    );
    summary.removed_zones = unused.into_iter().collect();
    // Nothing of the battery game is left.
    for kind in ["victory_items_container", "victory_item_spawner"] {
        ensure!(
            !all_strings(&project).contains(&kind),
            "the TDM project still names {kind}"
        );
    }
    Ok(project)
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Name(String),
    Index(usize),
}

fn leaves(c: &ConfigValue, path: &mut Vec<Step>, out: &mut Vec<(Vec<Step>, ConfigValue)>) {
    match c {
        ConfigValue::Named(fields) => {
            for f in fields {
                path.push(Step::Name(f.name.clone()));
                leaves(&f.value, path, out);
                path.pop();
            }
        }
        ConfigValue::Indexed(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(Step::Index(i));
                leaves(item, path, out);
                path.pop();
            }
        }
        leaf => out.push((path.clone(), leaf.clone())),
    }
}

fn at_mut<'a>(c: &'a mut ConfigValue, path: &[Step]) -> Option<&'a mut ConfigValue> {
    let Some((first, rest)) = path.split_first() else {
        return Some(c);
    };
    let next = match (c, first) {
        (ConfigValue::Named(fields), Step::Name(n)) => {
            &mut fields.iter_mut().find(|f| &f.name == n)?.value
        }
        (ConfigValue::Indexed(items), Step::Index(i)) => items.get_mut(*i)?,
        _ => return None,
    };
    at_mut(next, rest)
}

fn show(path: &[Step]) -> String {
    path.iter()
        .map(|s| match s {
            Step::Name(n) => format!(".{n}"),
            Step::Index(i) => format!("[{i}]"),
        })
        .collect::<String>()
}

/// The TDM options: the native ones with the port's substitutions.
pub fn deathmatch_options(
    native_battery: &ConfigValue,
    port_battery: &ConfigValue,
    native_tdm: &ConfigValue,
    summary: &mut Summary,
) -> Result<ConfigValue> {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    leaves(native_battery, &mut Vec::new(), &mut a);
    leaves(port_battery, &mut Vec::new(), &mut b);
    ensure!(
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.0 == y.0),
        "the port's battery options are not the native ones' shape"
    );
    let mut options = native_tdm.clone();
    for ((path, old), (_, new)) in a.iter().zip(&b) {
        if old == new {
            continue;
        }
        let (from, to) = match (old, new) {
            (ConfigValue::String(o), ConfigValue::String(n)) if o.ends_with("_2") => {
                ensure!(n.ends_with("_2"), "{}: {o:?} -> {n:?}", show(path));
                (
                    ConfigValue::String(format!("{}_4", &o[..o.len() - 2])),
                    ConfigValue::String(format!("{}_4", &n[..n.len() - 2])),
                )
            }
            (ConfigValue::String(_), ConfigValue::String(_))
            | (ConfigValue::Integer(_), ConfigValue::Integer(_)) => (old.clone(), new.clone()),
            _ => bail!(
                "{}: {old:?} -> {new:?} is no substitution this knows",
                show(path)
            ),
        };
        let leaf = at_mut(&mut options, path)
            .with_context(|| format!("the native TDM options have no {}", show(path)))?;
        ensure!(
            *leaf == from,
            "{}: the native TDM options have {leaf:?}, not {from:?}",
            show(path)
        );
        summary
            .options_changes
            .push(format!("{}: {from:?} -> {to:?}", show(path)));
        *leaf = to;
    }
    ensure!(
        !summary.options_changes.is_empty(),
        "the port's battery options are the native ones"
    );
    Ok(options)
}

/// Adds the variants to the pack at `pack` (its pack.json and files/), in place.
pub fn add(
    pack: &Path,
    target_db: &Path,
    variants: &[Variant],
    version: &str,
) -> Result<Vec<Summary>> {
    let manifest_path = pack.join("pack.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let mut target = ResourceDb::open(target_db)?;
    let read_cfg = |bytes: Vec<u8>, what: &str| -> Result<ConfigValue> {
        Ok(BinaryConfig::parse(&bytes)
            .with_context(|| format!("decoding {what}"))?
            .root)
    };
    let mut summaries = Vec::new();
    let mut ids = BTreeSet::new();
    for v in variants {
        ensure!(ids.insert(v.id), "map id {} twice", v.id);
        let mut summary = Summary {
            id: v.id,
            project: v.project(4),
            options: v.options(4),
            ..Default::default()
        };
        let battery_path = pack.join("files").join(v.project(2));
        let battery = read_cfg(
            fs::read(&battery_path).with_context(|| battery_path.display().to_string())?,
            &v.project(2),
        )?;
        let native_tdm_project =
            read_cfg(target.read(&v.native_project(4))?, &v.native_project(4))?;
        let project = deathmatch_project(&battery, &native_tdm_project, &mut summary)?;
        // The probes have this weather's TDM cubemap (the native lighting's).
        let key = format!("{}_4", v.weather);
        for o in project
            .named("client_objects")
            .map(indexed)
            .unwrap_or_default()
        {
            if string(o, "game_object_type") == Some("environment_probe") {
                ensure!(
                    o.named("textures").and_then(|t| t.named(&key)).is_some(),
                    "probe {} has no {key} cubemap",
                    string(o, "full_name").unwrap_or("?")
                );
            }
        }
        let port_options_path = pack.join("files/gameplay/game_modes").join(v.options(2));
        let options = deathmatch_options(
            &read_cfg(target.read(&v.native_options(2))?, &v.native_options(2))?,
            &read_cfg(fs::read(&port_options_path)?, &v.options(2))?,
            &read_cfg(target.read(&v.native_options(4))?, &v.native_options(4))?,
            &mut summary,
        )?;
        let written: BTreeMap<String, Vec<u8>> = BTreeMap::from([
            (v.project(4), BinaryConfig { root: project }.to_bytes()?),
            (
                format!("gameplay/game_modes/{}", v.options(4)),
                BinaryConfig { root: options }.to_bytes()?,
            ),
        ]);
        for (path, bytes) in &written {
            let file = pack.join("files").join(path);
            ensure!(!file.exists(), "{} exists already", file.display());
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(&file, bytes)?;
        }
        let ops = manifest["operations"]
            .as_array_mut()
            .context("pack.json has no operations")?;
        let at = ops
            .iter()
            .position(|op| op["op"] == "dictionary_add")
            .unwrap_or(ops.len());
        for path in written.keys().rev() {
            ops.insert(at, serde_json::json!({"op": "add_file", "path": path}));
        }
        let dictionary = ops
            .iter_mut()
            .find(|op| op["op"] == "dictionary_add" && op["table"] == "maps_dict")
            .context("pack.json adds no maps")?;
        let entries = dictionary["entries"]
            .as_array_mut()
            .context("dictionary_add has no entries")?;
        ensure!(
            !entries.iter().any(|e| e["id"] == v.id),
            "maps_dict {} is in the pack already",
            v.id
        );
        entries.push(serde_json::json!({"cfg_mode_name": v.options(4), "id": v.id}));
        manifest["server"]["maps"]
            .as_array_mut()
            .context("pack.json has no server maps")?
            .push(serde_json::json!({"id": v.id}));
        summaries.push(summary);
    }
    manifest["version"] = serde_json::Value::String(version.into());
    let tmp = manifest_path.with_extension("json.new");
    fs::write(&tmp, serde_json::to_vec_pretty(&manifest)?)?;
    fs::rename(&tmp, &manifest_path)?;
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            fields
                .into_iter()
                .map(|(n, v)| NamedValue {
                    name: n.into(),
                    value: v,
                })
                .collect(),
        )
    }
    fn s(v: &str) -> ConfigValue {
        ConfigValue::String(v.into())
    }
    fn zone(name: &str) -> ConfigValue {
        named(vec![
            ("game_object_type", s("collision_geometry")),
            ("full_name", s(name)),
        ])
    }
    fn uses(zones: &[&str]) -> ConfigValue {
        ConfigValue::Indexed(zones.iter().map(|z| named(vec![("name", s(z))])).collect())
    }
    fn collector(team: i32, zone: &str) -> ConfigValue {
        named(vec![
            ("game_object_type", s("victory_items_container")),
            ("team", ConfigValue::Integer(team)),
            (
                "visual_geometry",
                named(vec![("model_name", s("ported/v021d/item/quest/container"))]),
            ),
            ("collision_geometries", uses(&[zone])),
        ])
    }
    fn battery() -> ConfigValue {
        named(vec![
            (
                "server_objects",
                named(vec![
                    (
                        "respawn_points",
                        ConfigValue::Indexed(vec![named(vec![]), named(vec![])]),
                    ),
                    (
                        "victory_items_containers",
                        ConfigValue::Indexed(vec![collector(1, "a"), collector(0, "b")]),
                    ),
                    (
                        "victory_item_spawners",
                        ConfigValue::Indexed(vec![named(vec![(
                            "game_object_type",
                            s("victory_item_spawner"),
                        )])]),
                    ),
                ]),
            ),
            (
                "client_objects",
                ConfigValue::Indexed(vec![
                    zone("a"),
                    zone("b"),
                    zone("base"),
                    zone("shared"),
                    zone("other"),
                    named(vec![
                        ("game_object_type", s("effect_zone")),
                        ("collision_geometries", uses(&["base", "shared"])),
                    ]),
                    named(vec![
                        ("game_object_type", s("damage_zone")),
                        ("collision_geometries", uses(&["shared"])),
                    ]),
                ]),
            ),
            (
                "render_visuals",
                ConfigValue::Indexed(vec![
                    named(vec![("lib_name", s("ported/v021d/item/quest/container"))]),
                    named(vec![("lib_name", s("ported/v021d/item/quest/container"))]),
                    named(vec![("lib_name", s("props/box"))]),
                ]),
            ),
            (
                "collision_objects",
                ConfigValue::Indexed(vec![
                    named(vec![(
                        "name",
                        s("resources/models/ported/v021d/item/quest/container.model/collision"),
                    )]),
                    named(vec![(
                        "name",
                        s("resources/models/props/box.model/collision"),
                    )]),
                ]),
            ),
        ])
    }
    fn native_tdm() -> ConfigValue {
        named(vec![(
            "server_objects",
            named(vec![("respawn_points", ConfigValue::Indexed(vec![]))]),
        )])
    }

    #[test]
    fn the_battery_game_goes_and_a_zone_still_named_stays() {
        let mut summary = Summary::default();
        let project = deathmatch_project(&battery(), &native_tdm(), &mut summary).unwrap();
        let Some(ConfigValue::Named(groups)) = project.named("server_objects") else {
            panic!()
        };
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["respawn_points"]
        );
        assert_eq!(summary.respawn_points, 2);
        assert_eq!(
            summary.removed_server_groups,
            ["victory_items_containers", "victory_item_spawners"]
        );
        assert_eq!(
            (
                summary.removed_visuals,
                summary.removed_collision_objects,
                summary.removed_effect_zones
            ),
            (2, 1, 1)
        );
        // "shared" is still the damage zone's; "other" was never the battery game's.
        assert_eq!(summary.removed_zones, ["a", "b", "base"]);
        let left: Vec<&str> = indexed(project.named("client_objects").unwrap())
            .iter()
            .filter_map(|o| string(o, "full_name"))
            .collect();
        assert_eq!(left, ["shared", "other"]);
        assert_eq!(indexed(project.named("render_visuals").unwrap()).len(), 1);
    }

    #[test]
    fn a_project_without_two_collectors_is_refused() {
        let mut project = battery();
        let Some(ConfigValue::Named(groups)) = project.named_mut("server_objects") else {
            panic!()
        };
        groups[1].value = ConfigValue::Indexed(vec![collector(1, "a")]);
        assert!(deathmatch_project(&project, &native_tdm(), &mut Summary::default()).is_err());
    }

    fn options(
        key: &str,
        name: &str,
        map_id: i32,
        project: &str,
        tickets: Option<i32>,
    ) -> ConfigValue {
        let mut mode = vec![
            ("ui", named(vec![("map_name", s(key))])),
            ("name", s(name)),
            (
                "match_making_params",
                named(vec![
                    ("map_id", ConfigValue::Integer(map_id)),
                    ("map_name", s(project)),
                ]),
            ),
        ];
        if let Some(t) = tickets {
            mode.push(("tickets_count", ConfigValue::Integer(t)));
        }
        named(vec![("mode", named(mode))])
    }

    #[test]
    fn the_tdm_options_take_the_ports_substitutions() {
        let mut summary = Summary::default();
        let tdm = deathmatch_options(
            &options("st_level_03_name", "level_03_2", 3, "level_03/day_2", None),
            &options(
                "st_level_10_name",
                "v021d_level_03_2",
                10,
                "v021d_level_03/day_2",
                None,
            ),
            &options(
                "st_level_03_name",
                "level_03_4",
                3,
                "level_03/day_4",
                Some(100),
            ),
            &mut summary,
        )
        .unwrap();
        assert_eq!(
            tdm,
            options(
                "st_level_10_name",
                "v021d_level_03_4",
                10,
                "v021d_level_03/day_4",
                Some(100)
            )
        );
        assert_eq!(summary.options_changes.len(), 4);
        // A native TDM value that is not the battery one's is refused.
        assert!(
            deathmatch_options(
                &options("st_level_03_name", "level_03_2", 3, "level_03/day_2", None),
                &options(
                    "st_level_10_name",
                    "v021d_level_03_2",
                    10,
                    "v021d_level_03/day_2",
                    None
                ),
                &options(
                    "st_level_03_name",
                    "level_03_4",
                    4,
                    "level_03/day_4",
                    Some(100)
                ),
                &mut Summary::default(),
            )
            .is_err()
        );
    }
}
