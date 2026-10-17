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
    /// More maps the pack applies to (a release id of the same map, its
    /// Team Deathmatch variant).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_map_ids: Vec<u8>,
    pub template_project: String,
    pub template_sha256: String,
    #[serde(default)]
    pub groups: Vec<FillGroup>,
    #[serde(default)]
    pub collectors: Vec<CollectorFill>,
    /// Native client objects copied under a new name with field overrides.
    #[serde(default)]
    pub clones: Vec<CloneFill>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneFill {
    /// Native client object copied (exact full name).
    pub source: String,
    /// The copy's full name; its `name` becomes the last path component.
    pub full_name: String,
    /// Fields merged onto the copy. A JSON object merges into the matching
    /// named table, a JSON array replaces an array (element i built on the
    /// template's element i, or its last one), and a scalar takes the type of
    /// the field it replaces (an integer field stays an integer).
    #[serde(default)]
    pub set: serde_json::Map<String, serde_json::Value>,
}

/// A JSON override built on `like`, the template value it replaces.
fn merged(value: &serde_json::Value, like: Option<&ConfigValue>) -> Result<ConfigValue> {
    use serde_json::Value as J;
    Ok(match (value, like) {
        (J::Object(fields), like) => {
            let mut out = match like {
                Some(v @ ConfigValue::Named(_)) => v.clone(),
                None => ConfigValue::Named(Vec::new()),
                Some(_) => bail!("an object cannot replace a scalar or an array"),
            };
            for (key, v) in fields {
                let next = merged(v, out.named(key))?;
                out.set_path(key, next)?;
            }
            out
        }
        (J::Array(items), Some(ConfigValue::Float2(_) | ConfigValue::Float3(_) | ConfigValue::Float4(_))) | (J::Array(items), None)
            if (2..=4).contains(&items.len()) && items.iter().all(J::is_number) && !matches!(like, Some(ConfigValue::Indexed(_))) =>
        {
            let f: Vec<f32> = items.iter().map(|v| v.as_f64().unwrap() as f32).collect();
            match f.len() {
                2 => ConfigValue::Float2([f[0], f[1]]),
                3 => ConfigValue::Float3([f[0], f[1], f[2]]),
                _ => ConfigValue::Float4([f[0], f[1], f[2], f[3]]),
            }
        }
        (J::Array(items), like) => {
            let template = match like {
                Some(ConfigValue::Indexed(v)) => v.as_slice(),
                None => &[],
                Some(_) => bail!("an array cannot replace a scalar or a table"),
            };
            ConfigValue::Indexed(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| merged(v, template.get(i).or(template.last())))
                    .collect::<Result<_>>()?,
            )
        }
        (J::Bool(b), None | Some(ConfigValue::Boolean(_))) => ConfigValue::Boolean(*b),
        (J::String(s), None | Some(ConfigValue::String(_))) => ConfigValue::String(s.clone()),
        (J::Number(n), Some(ConfigValue::Integer(_))) => ConfigValue::Integer(
            n.as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .with_context(|| format!("{n} is not an i32 for an integer field"))?,
        ),
        (J::Number(n), Some(ConfigValue::Float(_))) => ConfigValue::Float(n.as_f64().unwrap() as f32),
        (J::Number(n), None) => match n.as_i64().and_then(|v| i32::try_from(v).ok()) {
            Some(v) => ConfigValue::Integer(v),
            None => ConfigValue::Float(n.as_f64().unwrap() as f32),
        },
        (v, Some(like)) => bail!("{v} cannot replace {like:?}"),
        (v, None) => bail!("unsupported override {v}"),
    })
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Where the group's anchor goes.
    pub position: [f32; 3],
    /// Rotation about the vertical axis through the anchor, radians.
    #[serde(default)]
    pub yaw: f32,
    /// Tilt to a sloped floor (engine XYZ Euler with `yaw`), radians.
    #[serde(default)]
    pub pitch: f32,
    #[serde(default)]
    pub roll: f32,
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

impl FillPack {
    /// Whether the pack applies to map `id`.
    pub fn applies_to(&self, id: u8) -> bool {
        self.map_id == id || self.also_map_ids.contains(&id)
    }
}

impl Placement {
    fn apply(&self, anchor: [f32; 3], object: &mut ConfigValue) -> Result<()> {
        let yaw = euler_to_basis([self.pitch, self.yaw, self.roll]);
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
    for clone in &pack.clones {
        let found: Vec<_> = native_objects
            .iter()
            .filter(|o| string(o, "full_name") == Some(clone.source.as_str()))
            .collect();
        ensure!(found.len() == 1, "clone source {} not unique", clone.source);
        let mut object = found[0].clone();
        for (key, value) in &clone.set {
            ensure!(key != "full_name" && key != "name", "{}: name fields come from full_name", clone.full_name);
            let next = merged(value, object.named(key))
                .with_context(|| format!("{}: field {key}", clone.full_name))?;
            object.set_path(key, next)?;
        }
        object.set_path("full_name", ConfigValue::String(clone.full_name.clone()))?;
        let short = clone.full_name.rsplit('/').next().unwrap_or(&clone.full_name).to_owned();
        object.set_path("name", ConfigValue::String(short))?;
        ensure!(names.insert(clone.full_name.clone()), "object name {} already exists", clone.full_name);
        indexed_mut(project, "client_objects")?.push(object);
    }
    if !pack.clones.is_empty() {
        notes.push(format!("gameplay fill {}: {} cloned native objects", pack.name, pack.clones.len()));
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
            also_map_ids: vec![],
            template_project: "projects/level_04/day_2/client_project".into(),
            template_sha256: String::new(),
            groups: vec![],
            collectors: vec![CollectorFill {
                team: 0,
                template_container: "logic/vip/team2/victory_items_container".into(),
                anchor: [71.334, 3.817, 54.393],
                to: Placement { position: [-73.942, 5.611, 44.038], yaw: 0.0, pitch: 0.0, roll: 0.0 },
                replaced_model: old.into(),
            }],
            clones: vec![],
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
        // Each crate stands beside its own team's battery container.
        let resupply = pack(include_str!("../../ports/gameplay-fill/chemical-011e-resupply.json"));
        let containers = pack(include_str!("../../ports/gameplay-fill/chemical-011e-containers.json"));
        for g in &resupply.groups {
            let crate_at = g.to.expect("placed").position;
            let container = containers.collectors.iter().find(|c| Some(c.team) == g.team).expect("same-team container").to.position;
            let d = ((crate_at[0] - container[0]).powi(2) + (crate_at[2] - container[2]).powi(2)).sqrt();
            // 4.2 m for team 1: the owner-approved placement of revision 18
            // (66.8, 3.879, 62.9), along the tunnel wall.
            assert!(d < 4.5, "team {:?} crate {d} m from its container", g.team);
        }
    }

    #[test]
    fn clone_merges_overrides_with_the_template_types() {
        let state = |threshold: i32, zones: &[&str]| {
            named(vec![
                ("energy_threshold", ConfigValue::Integer(threshold)),
                ("active_time_sec", ConfigValue::Integer(0)),
                (
                    "groups",
                    ConfigValue::Indexed(vec![named(vec![
                        ("keep_all_zones_active", ConfigValue::Boolean(true)),
                        ("zones", ConfigValue::Indexed(zones.iter().map(|z| s(z)).collect())),
                    ])]),
                ),
            ])
        };
        let core = named(vec![
            ("full_name", s("anomaly/fruit/fruit_anomaly")),
            ("name", s("fruit_anomaly")),
            ("energy_enabled", ConfigValue::Boolean(true)),
            ("energy_decrease_speed", ConfigValue::Integer(25)),
            ("max_hit", ConfigValue::Float(0.07)),
            ("position", ConfigValue::Float3([22.9, 9.8, 113.8])),
            ("states", ConfigValue::Indexed(vec![state(0, &["dummy"]), state(20, &["lvl1", "lvl1_fx"])])),
        ]);
        let native = named(vec![("client_objects", ConfigValue::Indexed(vec![core]))]);
        let mut project = named(vec![("client_objects", ConfigValue::Indexed(vec![]))]);
        let set: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"energy_enabled": false, "energy_decrease_speed": 0, "max_hit": 0, "position": [15.75, 2.3, 40.0],
                "states": [{"active_time_sec": 195, "groups": [{"zones": ["a/dummy"]}]},
                           {"energy_threshold": 1, "groups": [{"zones": ["a/fx"]}]}]}"#,
        )
        .unwrap();
        let pack = FillPack {
            name: "clone".into(),
            map_id: 194,
            also_map_ids: vec![],
            template_project: "projects/level_02/day_2/client_project".into(),
            template_sha256: String::new(),
            groups: vec![],
            collectors: vec![],
            clones: vec![CloneFill { source: "anomaly/fruit/fruit_anomaly".into(), full_name: "logic/fill/track/stage_01".into(), set }],
        };
        apply(&mut project, &native, &pack).unwrap();
        let o = &array(project.named("client_objects").unwrap()).unwrap()[0];
        assert_eq!(string(o, "full_name"), Some("logic/fill/track/stage_01"));
        assert_eq!(string(o, "name"), Some("stage_01"));
        assert_eq!(o.named("energy_enabled"), Some(&ConfigValue::Boolean(false)));
        assert_eq!(o.named("energy_decrease_speed"), Some(&ConfigValue::Integer(0)), "an integer stays an integer");
        assert_eq!(o.named("max_hit"), Some(&ConfigValue::Float(0.0)), "a float stays a float");
        assert_eq!(o.named("position"), Some(&ConfigValue::Float3([15.75, 2.3, 40.0])));
        let states = array(o.named("states").unwrap()).unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].named("active_time_sec"), Some(&ConfigValue::Integer(195)));
        assert_eq!(states[0].named("energy_threshold"), Some(&ConfigValue::Integer(0)), "untouched fields stay");
        assert_eq!(states[1].named("energy_threshold"), Some(&ConfigValue::Integer(1)));
        let group = &array(states[1].named("groups").unwrap()).unwrap()[0];
        assert_eq!(group.named("keep_all_zones_active"), Some(&ConfigValue::Boolean(true)));
        assert_eq!(array(group.named("zones").unwrap()).unwrap(), &[s("a/fx")], "arrays are replaced");
        // A type change and a duplicate name are refused.
        let mut bad = pack.clone();
        bad.clones[0].set = serde_json::from_str(r#"{"energy_enabled": 3}"#).unwrap();
        assert!(apply(&mut named(vec![("client_objects", ConfigValue::Indexed(vec![]))]), &native, &bad).is_err());
        assert!(apply(&mut project, &native, &pack).is_err());
    }

    #[test]
    fn track_anomaly_pack_is_a_closed_set_of_timed_native_clones() {
        let track = pack(include_str!("../../ports/gameplay-fill/chemical-011e-track-anomaly.json"));
        assert_eq!(track.map_id, 194);
        assert_eq!(track.template_project, "projects/level_02/day_2/client_project");
        assert!(track.groups.is_empty() && track.collectors.is_empty());
        let names: BTreeSet<&str> = track.clones.iter().map(|c| c.full_name.as_str()).collect();
        assert_eq!(names.len(), track.clones.len(), "unique names");
        assert!(names.iter().all(|n| n.starts_with("logic/fill/track_anomaly/")));
        let refs = |v: &serde_json::Value, out: &mut Vec<String>| {
            fn walk(v: &serde_json::Value, key: &str, out: &mut Vec<String>) {
                match v {
                    serde_json::Value::Object(m) => m.iter().for_each(|(k, v)| walk(v, k, out)),
                    serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, key, out)),
                    serde_json::Value::String(s) if key == "zones" || key == "name" => out.push(s.clone()),
                    _ => {}
                }
            }
            walk(v, "", out)
        };
        let mut starts = Vec::new();
        let mut controllers = 0;
        for c in &track.clones {
            let mut linked = Vec::new();
            refs(&serde_json::Value::Object(c.set.clone()), &mut linked);
            for l in &linked {
                assert!(names.contains(l.as_str()), "{}: {l} is not in the pack", c.full_name);
            }
            if c.source == "anomaly/fruit/fruit_anomaly" {
                controllers += 1;
                assert_eq!(c.set["energy_enabled"], false, "players cannot move the timers");
                assert_eq!(c.set["max_artefacts_count"], 0, "no artefact spawn without containers");
                let states = c.set["states"].as_array().unwrap();
                assert_eq!(states[0]["energy_threshold"], 0);
                assert_eq!(states[0]["energy_on_exit"], 1);
                if c.full_name.contains("/stage_") {
                    assert_eq!(states.len(), 2);
                    assert_eq!(states[1]["active_time_sec"], 0, "a fog stage stays on");
                    starts.push(states[0]["active_time_sec"].as_i64().unwrap());
                } else {
                    // The damage chain: each timed state exits into the next threshold.
                    for (j, st) in states.iter().enumerate() {
                        assert_eq!(st["energy_threshold"], j as i64);
                        let timed = st["active_time_sec"].as_i64().unwrap() > 0;
                        assert_eq!(st["energy_on_exit"].as_i64().unwrap(), if timed { j as i64 + 1 } else { 0 });
                    }
                }
            }
        }
        assert_eq!(controllers, starts.len() + 1);
        assert!(starts.windows(2).all(|w| w[1] - w[0] == starts[1] - starts[0]), "evenly spaced stages");
    }

    #[test]
    fn track_damage_is_capped_contiguous_segments_shared_by_the_steps() {
        let track = pack(include_str!("../../ports/gameplay-fill/chemical-011e-track-anomaly.json"));
        let by_name: BTreeMap<&str, &CloneFill> = track.clones.iter().map(|c| (c.full_name.as_str(), c)).collect();
        let f = |v: &serde_json::Value| v.as_f64().unwrap() as f32;
        // Segment geometries: boxes along the track, each lane box low over its own bed.
        let mut segments: Vec<(&str, f32, f32)> = Vec::new();
        for c in track.clones.iter().filter(|c| c.full_name.contains("/damage/segment_")) {
            let at = c.set["position"].as_array().unwrap();
            let meshes = c.set["meshes"].as_array().unwrap();
            assert!(meshes.len() >= 2, "{}: lanes", c.full_name);
            let mut lanes = Vec::new();
            for m in meshes {
                assert_eq!(m["type"], 1, "boxes");
                assert!(m["rotation"].as_array().unwrap().iter().all(|r| f(r) == 0.0));
                let (p, sc) = (m["position"].as_array().unwrap(), m["scale"].as_array().unwrap());
                let height = 2.0 * f(&sc[1]);
                assert!((2.5..=4.0).contains(&height), "{}: a box {height} m tall", c.full_name);
                let top = f(&at[1]) + f(&p[1]) + f(&sc[1]);
                assert!(top < 6.0, "{}: box top at {top}, wagon roofs stand at 7.2", c.full_name);
                lanes.push((f(&at[0]) + f(&p[0]) - f(&sc[0]), f(&at[0]) + f(&p[0]) + f(&sc[0])));
                assert_eq!(f(&p[2]), 0.0);
            }
            assert!(lanes.windows(2).all(|w| w[0].1 > w[1].0), "{}: lanes overlap", c.full_name);
            let half = f(&meshes[0]["scale"][2]);
            segments.push((c.full_name.as_str(), f(&at[2]) - half, f(&at[2]) + half));
        }
        assert!(segments.len() >= 20);
        assert!(segments.windows(2).all(|w| w[0].2 > w[1].1 && w[0].1 < w[1].1), "segments run along the track and overlap");
        // Each step's zone holds the segments from the first one on; the last step holds them all.
        let controller = by_name["logic/fill/track_anomaly/damage/controller"];
        let states = controller.set["states"].as_array().unwrap();
        let mut held = 0;
        for st in &states[1..] {
            let zone = by_name[st["groups"][0]["zones"][0].as_str().unwrap()];
            let names: Vec<&str> = zone.set["collision_geometries"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
            assert_eq!(names, segments[..names.len()].iter().map(|s| s.0).collect::<Vec<_>>(), "{}", zone.full_name);
            assert!(names.len() > held, "{}: the damage grows", zone.full_name);
            held = names.len();
        }
        assert_eq!(held, segments.len(), "the last step reaches the end of the track");
        // The fog reaches past the last box: no damage without a cloud.
        let last_cloud = track
            .clones
            .iter()
            .filter(|c| c.full_name.ends_with("/fog_zone"))
            .map(|c| f(&c.set["position"][2]))
            .fold(f32::MIN, f32::max);
        assert!(segments.last().unwrap().2 <= last_cloud + 5.0);
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
            also_map_ids: vec![],
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
                    yaw: std::f32::consts::FRAC_PI_2, pitch: 0.0, roll: 0.0 }),
                team: Some(1),
                rename: BTreeMap::from([("logic/vip/team1/".into(), "logic/fill/team2/".into())]),
            }],
            collectors: vec![],
            clones: vec![],
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
