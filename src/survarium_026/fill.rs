// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional "0.26 gameplay fill" packs: native 0.26 gameplay objects added to a
//! ported map. This is not fidelity conversion: packs are only applied when a
//! build names them, so the faithful port stays reproducible without them.
//!
//! A pack copies named native objects from a hash-pinned native project. A
//! group may be moved rigidly (anchor -> position, plus a yaw about the anchor)
//! and re-teamed or renamed. Collector entries give a ported team collector the
//! native charger model, its interaction zone and its collision.
use super::*;
use crate::skeleton_animation::{basis_to_euler, euler_to_basis, multiply};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FillPack {
    pub name: String,
    pub map_id: u8,
    pub template_project: String,
    pub template_sha256: String,
    #[serde(default)]
    pub groups: Vec<FillGroup>,
    #[serde(default)]
    pub collectors: Vec<CollectorFill>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Where the group's anchor goes.
    pub position: [f32; 3],
    /// Rotation about the vertical axis through the anchor, radians.
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FillGroup {
    pub name: String,
    /// Native client objects copied by exact full name.
    #[serde(default)]
    pub client_objects: Vec<String>,
    /// Native client objects copied by full-name prefix (at least one each).
    #[serde(default)]
    pub client_object_prefixes: Vec<String>,
    /// Native models whose render visuals and collision objects within
    /// `radius` of `anchor` are copied.
    #[serde(default)]
    pub models: Vec<String>,
    pub anchor: [f32; 3],
    #[serde(default)]
    pub radius: f32,
    #[serde(default)]
    pub to: Option<Placement>,
    #[serde(default)]
    pub team: Option<i32>,
    /// String replacements on copied objects, e.g. a name prefix per team.
    #[serde(default)]
    pub rename: BTreeMap<String, String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectorFill {
    pub team: i32,
    /// Native `victory_items_container` full name supplying model and zone.
    pub template_container: String,
    /// Floor point under the native charger.
    pub anchor: [f32; 3],
    pub to: Placement,
    /// Old collector model whose collision instances at the old visual are removed.
    pub replaced_model: String,
}

/// Radius around a native charger's visual_geometry that holds its parts.
const CHARGER_RADIUS: f32 = 1.0;

fn float3(o: &ConfigValue, key: &str) -> Option<[f32; 3]> {
    match o.named(key) {
        Some(ConfigValue::Float3(v)) => Some(*v),
        _ => None,
    }
}

impl Placement {
    fn apply(&self, anchor: [f32; 3], object: &mut ConfigValue) -> Result<()> {
        let yaw = euler_to_basis([0.0, self.yaw, 0.0]);
        if let Some(p) = float3(object, "position") {
            let local = [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
            let moved: [f32; 3] = std::array::from_fn(|j| {
                (0..3).map(|i| local[i] * yaw[i][j]).sum::<f32>() + self.position[j]
            });
            object.set_path("position", ConfigValue::Float3(moved))?;
        }
        if let Some(r) = float3(object, "rotation") {
            let rotated = basis_to_euler(&multiply(&euler_to_basis(r), &yaw));
            object.set_path("rotation", ConfigValue::Float3(rotated))?;
        }
        Ok(())
    }
}

fn rename(value: &mut ConfigValue, map: &BTreeMap<String, String>) {
    match value {
        ConfigValue::String(s) => {
            for (from, to) in map {
                if s.contains(from.as_str()) {
                    *s = s.replace(from.as_str(), to);
                }
            }
        }
        ConfigValue::Named(fields) => fields.iter_mut().for_each(|f| rename(&mut f.value, map)),
        ConfigValue::Indexed(items) => items.iter_mut().for_each(|v| rename(v, map)),
        _ => {}
    }
}

fn indexed_mut<'a>(project: &'a mut ConfigValue, key: &str) -> Result<&'a mut Vec<ConfigValue>> {
    if project.named(key).is_none() {
        project.set_path(key, ConfigValue::Indexed(Vec::new()))?;
    }
    match project.named_mut(key) {
        Some(ConfigValue::Indexed(v)) => Ok(v),
        Some(v @ ConfigValue::Named(_)) => {
            *v = ConfigValue::Indexed(Vec::new());
            match v {
                ConfigValue::Indexed(v) => Ok(v),
                _ => unreachable!(),
            }
        }
        _ => bail!("{key} is not an array"),
    }
}

fn model_of(o: &ConfigValue) -> Option<String> {
    let name = string(o, "lib_name").or_else(|| string(o, "model_name"))?;
    let name = name.trim_start_matches("resources/models/");
    Some(name.split(".model/").next().unwrap_or(name).to_owned())
}

fn near(o: &ConfigValue, anchor: [f32; 3], radius: f32) -> bool {
    float3(o, "position")
        .is_some_and(|p| (0..3).map(|i| (p[i] - anchor[i]).powi(2)).sum::<f32>() <= radius * radius)
}

/// Apply one pack to a converted project; returns notes for the build record.
pub struct Applied {
    pub notes: Vec<String>,
    /// Native model references added by the pack (render or collision).
    pub native_models: BTreeSet<String>,
}

pub fn apply(project: &mut ConfigValue, native: &ConfigValue, pack: &FillPack) -> Result<Applied> {
    let mut notes = Vec::new();
    let mut native_models = BTreeSet::new();
    let native_objects = array(native.named("client_objects").context("native objects")?)?;
    let mut names: BTreeSet<String> = array(project.named("client_objects").context("objects")?)?
        .iter()
        .filter_map(|o| string(o, "full_name").map(str::to_owned))
        .collect();
    for group in &pack.groups {
        let mut objects: Vec<ConfigValue> = Vec::new();
        for name in &group.client_objects {
            let found: Vec<_> = native_objects
                .iter()
                .filter(|o| string(o, "full_name") == Some(name.as_str()))
                .collect();
            ensure!(
                found.len() == 1,
                "{}: native object {name} not unique",
                group.name
            );
            objects.push(found[0].clone());
        }
        for prefix in &group.client_object_prefixes {
            let before = objects.len();
            objects.extend(
                native_objects
                    .iter()
                    .filter(|o| {
                        string(o, "full_name").is_some_and(|n| n.starts_with(prefix.as_str()))
                    })
                    .cloned(),
            );
            ensure!(
                objects.len() > before,
                "{}: no native object under {prefix}",
                group.name
            );
        }
        let copied = |key: &str| -> Result<Vec<ConfigValue>> {
            Ok(array(native.named(key).context("native group")?)?
                .iter()
                .filter(|o| {
                    model_of(o).is_some_and(|m| group.models.contains(&m))
                        && near(o, group.anchor, group.radius)
                })
                .cloned()
                .collect())
        };
        let visuals = copied("render_visuals")?;
        let collisions = copied("collision_objects")?;
        ensure!(
            group.models.is_empty() || !visuals.is_empty(),
            "{}: no native visual of {:?} near the anchor",
            group.name,
            group.models
        );
        let counts = (objects.len(), visuals.len(), collisions.len());
        for (key, mut items) in [
            ("client_objects", objects),
            ("render_visuals", visuals),
            ("collision_objects", collisions),
        ] {
            for item in &mut items {
                rename(item, &group.rename);
                if let Some(team) = group.team {
                    if item.named("team").is_some() {
                        item.set_path("team", ConfigValue::Integer(team))?;
                    }
                }
                if let Some(to) = group.to {
                    to.apply(group.anchor, item)?;
                }
                if key != "client_objects" {
                    if let Some(lib) = string(item, "lib_name") {
                        native_models.insert(lib.to_owned());
                    }
                }
                if key == "client_objects" {
                    let name = string(item, "full_name")
                        .context("copied object name")?
                        .to_owned();
                    ensure!(
                        names.insert(name.clone()),
                        "{}: object name {name} already exists",
                        group.name
                    );
                }
            }
            indexed_mut(project, key)?.extend(items);
        }
        notes.push(format!(
            "gameplay fill {} / {}: {} objects, {} visuals, {} collisions{}",
            pack.name,
            group.name,
            counts.0,
            counts.1,
            counts.2,
            group.to.map_or(String::new(), |p| format!(
                " moved to {:?} yaw {}",
                p.position, p.yaw
            ))
        ));
    }
    for collector in &pack.collectors {
        let (note, models) = apply_collector(project, native, native_objects, collector)?;
        notes.push(note);
        native_models.extend(models);
    }
    Ok(Applied {
        notes,
        native_models,
    })
}

fn apply_collector(
    project: &mut ConfigValue,
    native: &ConfigValue,
    native_objects: &[ConfigValue],
    fill: &CollectorFill,
) -> Result<(String, BTreeSet<String>)> {
    let template = array(
        native
            .named("server_objects")
            .and_then(|g| g.named("victory_items_containers"))
            .context("native collectors")?,
    )?
    .iter()
    .find(|c| string(c, "full_name") == Some(fill.template_container.as_str()))
    .context("native template collector")?
    .clone();
    let native_visual = template
        .named("visual_geometry")
        .context("native charger visual")?
        .clone();
    let native_visual_at = float3(&native_visual, "position").context("native visual position")?;
    let zone_name = array(
        template
            .named("collision_geometries")
            .context("native zone ref")?,
    )?
    .first()
    .and_then(|z| string(z, "name"))
    .context("native zone name")?
    .to_owned();
    let native_zone = native_objects
        .iter()
        .find(|o| string(o, "full_name") == Some(zone_name.as_str()))
        .context("native zone")?
        .clone();
    let charger = model_of(&native_visual).context("native charger model")?;
    // The native charger is drawn by static render visuals (level_04: two
    // stacked box_atlant12, 0.77 m apart) at its visual_geometry; the
    // visual_geometry itself draws nothing. Copy every instance of the model
    // within CHARGER_RADIUS of it, render visuals and collisions alike.
    let charger_parts = |key: &str| -> Result<Vec<ConfigValue>> {
        Ok(array(native.named(key).context("native charger group")?)?
            .iter()
            .filter(|o| {
                model_of(o).as_deref() == Some(charger.as_str())
                    && near(o, native_visual_at, CHARGER_RADIUS)
            })
            .cloned()
            .collect())
    };
    let charger_visuals = charger_parts("render_visuals")?;
    let charger_collisions = charger_parts("collision_objects")?;
    ensure!(
        !charger_visuals.is_empty() && !charger_collisions.is_empty(),
        "native charger has no render visual or no collision near its visual_geometry"
    );
    // Our collector of this team, its zone and its old model.
    let ConfigValue::Indexed(ours) = project
        .named_mut("server_objects")
        .and_then(|g| g.named_mut("victory_items_containers"))
        .context("collectors")?
    else {
        bail!("collector array")
    };
    let collector = ours
        .iter_mut()
        .find(|c| c.named("team") == Some(&ConfigValue::Integer(fill.team)))
        .with_context(|| format!("no collector for team {}", fill.team))?;
    let old_visual_at = collector
        .named("visual_geometry")
        .and_then(|v| float3(v, "position"))
        .context("old collector visual")?;
    let our_zone = array(
        collector
            .named("collision_geometries")
            .context("zone ref")?,
    )?
    .first()
    .and_then(|z| string(z, "name"))
    .context("zone name")?
    .to_owned();
    let mut visual = native_visual;
    fill.to.apply(fill.anchor, &mut visual)?;
    if let Some(name) = string(collector, "full_name").map(str::to_owned) {
        visual.set_path("name", ConfigValue::String(name))?;
    }
    collector.set_path("visual_geometry", visual)?;
    let ConfigValue::Indexed(objects) = project.named_mut("client_objects").context("objects")?
    else {
        bail!("objects array")
    };
    let zone = objects
        .iter_mut()
        .find(|o| string(o, "full_name") == Some(our_zone.as_str()))
        .context("our collector zone")?;
    let mut moved = native_zone;
    fill.to.apply(fill.anchor, &mut moved)?;
    for key in ["position", "rotation", "scale", "meshes"] {
        if let Some(value) = moved.named(key) {
            zone.set_path(key, value.clone())?;
        }
    }
    // The old collector's static model and its collision go; the charger's come in.
    let mut removed = [0; 2];
    let mut added = [0; 2];
    let mut models = BTreeSet::new();
    for (i, (key, parts)) in [("render_visuals", charger_visuals), ("collision_objects", charger_collisions)]
        .into_iter()
        .enumerate()
    {
        let items = indexed_mut(project, key)?;
        let before = items.len();
        items.retain(|o| {
            !(model_of(o).as_deref() == Some(fill.replaced_model.as_str())
                && near(o, old_visual_at, 0.1))
        });
        removed[i] = before - items.len();
        for mut part in parts {
            fill.to.apply(fill.anchor, &mut part)?;
            if let Some(lib) = string(&part, "lib_name") {
                models.insert(lib.to_owned());
            }
            items.push(part);
            added[i] += 1;
        }
    }
    ensure!(
        removed[1] > 0,
        "old collector collision of {} not found",
        fill.replaced_model
    );
    let note = format!(
        "gameplay fill collector team {}: {} at {:?} yaw {} ({} render visuals and {} collisions added; {} old visuals and {} old collisions removed)",
        fill.team, charger, fill.to.position, fill.to.yaw, added[0], added[1], removed[0], removed[1]
    );
    Ok((note, models))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn s(v: &str) -> ConfigValue {
        ConfigValue::String(v.into())
    }

    fn f3(v: [f32; 3]) -> ConfigValue {
        ConfigValue::Float3(v)
    }
    fn part(lib: &str, at: [f32; 3]) -> ConfigValue {
        named(vec![("lib_name", s(lib)), ("position", f3(at)), ("rotation", f3([0.0; 3]))])
    }
    fn zone(name: &str, at: [f32; 3]) -> ConfigValue {
        named(vec![
            ("full_name", s(name)),
            ("position", f3(at)),
            ("rotation", f3([0.0; 3])),
            ("scale", f3([1.0; 3])),
            ("meshes", ConfigValue::Indexed(vec![named(vec![("scale", f3([2.0; 3]))])])),
        ])
    }
    fn container(team: i32, name: &str, model: &str, visual_at: [f32; 3], zone: &str) -> ConfigValue {
        named(vec![
            ("full_name", s(name)),
            ("team", ConfigValue::Integer(team)),
            (
                "visual_geometry",
                named(vec![("model_name", s(model)), ("position", f3(visual_at)), ("rotation", f3([0.0; 3]))]),
            ),
            ("collision_geometries", ConfigValue::Indexed(vec![named(vec![("name", s(zone))])])),
        ])
    }

    #[test]
    fn collector_takes_the_whole_native_charger_and_drops_the_old_model() {
        let charger = "item/storage_device/box_atlant12";
        let old = "item/quest/container";
        let native = named(vec![
            ("client_objects", ConfigValue::Indexed(vec![zone("logic/vip/team2/container_collision", [71.0, 4.0, 54.0])])),
            (
                "render_visuals",
                ConfigValue::Indexed(vec![
                    part(charger, [71.334, 4.579, 54.393]),
                    part(charger, [71.343, 3.811, 54.397]),
                    part(charger, [-30.9, 2.2, 40.4]),
                ]),
            ),
            (
                "collision_objects",
                ConfigValue::Indexed(vec![
                    part(&format!("resources/models/{charger}.model/collision"), [71.334, 4.579, 54.393]),
                    part(&format!("resources/models/{charger}.model/collision"), [71.343, 3.811, 54.397]),
                    part(&format!("resources/models/{charger}.model/collision"), [-30.9, 2.2, 40.4]),
                ]),
            ),
            (
                "server_objects",
                named(vec![(
                    "victory_items_containers",
                    ConfigValue::Indexed(vec![container(
                        1,
                        "logic/vip/team2/victory_items_container",
                        charger,
                        [71.334, 4.579, 54.393],
                        "logic/vip/team2/container_collision",
                    )]),
                )]),
            ),
        ]);
        let mut project = named(vec![
            ("client_objects", ConfigValue::Indexed(vec![zone("logic/team1/victory_container_collision", [-73.4, 5.6, 41.0])])),
            ("render_visuals", ConfigValue::Indexed(vec![part(old, [-73.942, 5.611, 44.038])])),
            (
                "collision_objects",
                ConfigValue::Indexed(vec![part(&format!("resources/models/{old}.model/collision"), [-73.942, 5.611, 44.038])]),
            ),
            (
                "server_objects",
                named(vec![(
                    "victory_items_containers",
                    ConfigValue::Indexed(vec![container(
                        0,
                        "logic/team1/victory_items_container",
                        old,
                        [-73.942, 5.611, 44.038],
                        "logic/team1/victory_container_collision",
                    )]),
                )]),
            ),
        ]);
        let pack = FillPack {
            name: "chargers".into(),
            map_id: 194,
            template_project: "projects/level_04/day_2/client_project".into(),
            template_sha256: String::new(),
            groups: vec![],
            collectors: vec![CollectorFill {
                team: 0,
                template_container: "logic/vip/team2/victory_items_container".into(),
                anchor: [71.334, 3.817, 54.393],
                to: Placement { position: [-73.942, 5.611, 44.038], yaw: 0.0 },
                replaced_model: old.into(),
            }],
        };
        apply(&mut project, &native, &pack).unwrap();
        for key in ["render_visuals", "collision_objects"] {
            let items = array(project.named(key).unwrap()).unwrap();
            assert_eq!(items.len(), 2, "{key}: both stacked boxes, the old model gone");
            let mut heights: Vec<f32> = items
                .iter()
                .map(|o| {
                    assert_eq!(model_of(o).as_deref(), Some(charger), "{key}");
                    let p = float3(o, "position").unwrap();
                    assert!((p[0] + 73.942).abs() < 0.02 && (p[2] - 44.038).abs() < 0.02, "{key} {p:?}");
                    p[1]
                })
                .collect();
            heights.sort_by(f32::total_cmp);
            assert!((heights[0] - 5.605).abs() < 1e-3 && (heights[1] - 6.373).abs() < 1e-3, "{key} {heights:?}");
        }
        let collector = &array(project.named("server_objects").unwrap().named("victory_items_containers").unwrap()).unwrap()[0];
        let visual = collector.named("visual_geometry").unwrap();
        assert_eq!(string(visual, "model_name"), Some(charger));
        assert_eq!(string(visual, "name"), Some("logic/team1/victory_items_container"));
    }

    fn pack(text: &str) -> FillPack {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn chemical_packs_keep_the_native_anomaly_logic_and_draw_the_level_03_cloud() {
        let anomaly = pack(include_str!("../../ports/gameplay-fill/chemical-011e-anomaly.json"));
        let prefixes: Vec<&str> = anomaly.groups.iter().flat_map(|g| g.client_object_prefixes.iter().map(String::as_str)).collect();
        assert_eq!(prefixes, ["logic/anomaly/"], "no level_04 fx in the anomaly pack");
        let fog = pack(include_str!("../../ports/gameplay-fill/chemical-011e-rust-fog.json"));
        assert_eq!(fog.map_id, 194);
        assert_eq!(fog.template_project, "projects/level_03/day_2/client_project");
        let mut names = BTreeSet::new();
        let mut dark = 0;
        for group in &fog.groups {
            assert!(group.client_object_prefixes.is_empty() && group.models.is_empty() && group.team.is_none());
            let [source] = group.client_objects.as_slice() else { panic!("{}: one native object", group.name) };
            assert!(source == "fx/smoke" || source == "fx/smoke_dark", "{source}");
            dark += usize::from(source == "fx/smoke_dark");
            let renamed = &group.rename[source];
            assert!(renamed.starts_with("fx/fill_rust_fog/") && names.insert(renamed.clone()), "{renamed}");
            // Inside the 194 moss field, at most 6 m up its slope.
            let p = group.to.expect("placed").position;
            assert!((-5.0..40.0).contains(&p[0]) && (-25.0..10.0).contains(&p[2]) && (2.0..9.0).contains(&p[1]), "{p:?}");
        }
        assert_eq!(dark, 1);
        assert!(fog.groups.len() >= 10, "level_03 uses nine clouds for a smaller field");
        let resupply = pack(include_str!("../../ports/gameplay-fill/chemical-011e-resupply.json"));
        assert!(resupply.groups.iter().all(|g| g.to.is_none()), "crates stay at the native 0.26 spots");
    }

    #[test]
    fn group_is_copied_moved_reteamed_and_renamed_once() {
        let zone = named(vec![
            ("full_name", s("logic/vip/team1/resupply/resupply_zone")),
            ("team", ConfigValue::Integer(0)),
            ("position", ConfigValue::Float3([11.0, 2.0, 0.0])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
            (
                "collision_geometries",
                ConfigValue::Indexed(vec![named(vec![(
                    "name",
                    s("logic/vip/team1/resupply/resupply_collision"),
                )])]),
            ),
        ]);
        let crate_visual = named(vec![
            ("lib_name", s("item/quest/container_closed")),
            ("position", ConfigValue::Float3([10.0, 2.0, 0.0])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
        ]);
        let far_visual = named(vec![
            ("lib_name", s("item/quest/container_closed")),
            ("position", ConfigValue::Float3([50.0, 2.0, 0.0])),
        ]);
        let native = named(vec![
            ("client_objects", ConfigValue::Indexed(vec![zone])),
            (
                "render_visuals",
                ConfigValue::Indexed(vec![crate_visual, far_visual]),
            ),
            ("collision_objects", ConfigValue::Indexed(vec![])),
        ]);
        let mut project = named(vec![("client_objects", ConfigValue::Indexed(vec![]))]);
        let pack = FillPack {
            name: "test".into(),
            map_id: 194,
            template_project: "projects/level_04/day_2/client_project".into(),
            template_sha256: String::new(),
            groups: vec![FillGroup {
                name: "resupply team 1".into(),
                client_objects: vec!["logic/vip/team1/resupply/resupply_zone".into()],
                client_object_prefixes: vec![],
                models: vec!["item/quest/container_closed".into()],
                anchor: [10.0, 2.0, 0.0],
                radius: 2.0,
                to: Some(Placement {
                    position: [100.0, 6.5, 80.0],
                    yaw: std::f32::consts::FRAC_PI_2,
                }),
                team: Some(1),
                rename: BTreeMap::from([("logic/vip/team1/".into(), "logic/fill/team2/".into())]),
            }],
            collectors: vec![],
        };
        let applied = apply(&mut project, &native, &pack).unwrap();
        assert_eq!(applied.notes.len(), 1);
        assert_eq!(
            applied.native_models,
            BTreeSet::from(["item/quest/container_closed".to_owned()])
        );
        let object = &array(project.named("client_objects").unwrap()).unwrap()[0];
        assert_eq!(
            string(object, "full_name"),
            Some("logic/fill/team2/resupply/resupply_zone")
        );
        assert_eq!(object.named("team"), Some(&ConfigValue::Integer(1)));
        let reference = &array(object.named("collision_geometries").unwrap()).unwrap()[0];
        assert_eq!(
            string(reference, "name"),
            Some("logic/fill/team2/resupply/resupply_collision")
        );
        // The zone 1 m along +X of the anchor turns a quarter about +Y.
        let p = float3(object, "position").unwrap();
        let expected = euler_to_basis([0.0, std::f32::consts::FRAC_PI_2, 0.0])[0];
        for i in 0..3 {
            assert!(
                (p[i] - ([100.0, 6.5, 80.0][i] + expected[i])).abs() < 1e-4,
                "{p:?}"
            );
        }
        let visuals = array(project.named("render_visuals").unwrap()).unwrap();
        assert_eq!(
            visuals.len(),
            1,
            "only the visual near the anchor is copied"
        );
        // Applying the same pack again would duplicate a named object.
        assert!(apply(&mut project, &native, &pack).is_err());
    }
}
