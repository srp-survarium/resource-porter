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
    let charger_collisions: Vec<ConfigValue> = array(
        native
            .named("collision_objects")
            .context("native collisions")?,
    )?
    .iter()
    .filter(|o| model_of(o).as_deref() == Some(charger.as_str()) && near(o, native_visual_at, 0.05))
    .cloned()
    .collect();
    ensure!(
        !charger_collisions.is_empty(),
        "native charger has no collision instances"
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
    let collisions = indexed_mut(project, "collision_objects")?;
    let before = collisions.len();
    collisions.retain(|o| {
        !(model_of(o).as_deref() == Some(fill.replaced_model.as_str())
            && near(o, old_visual_at, 0.1))
    });
    let removed = before - collisions.len();
    ensure!(
        removed > 0,
        "old collector collision of {} not found",
        fill.replaced_model
    );
    let mut models = BTreeSet::new();
    for mut c in charger_collisions {
        fill.to.apply(fill.anchor, &mut c)?;
        if let Some(lib) = string(&c, "lib_name") {
            models.insert(lib.to_owned());
        }
        collisions.push(c);
    }
    let note = format!(
        "gameplay fill collector team {}: {} at {:?} yaw {} ({removed} old collision instances removed)",
        fill.team, charger, fill.to.position, fill.to.yaw
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
