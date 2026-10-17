// SPDX-License-Identifier: GPL-3.0-or-later
//! Native 0.26 bonfires in place of converted X-Ray campfires.
//!
//! An X-Ray campfire is a pile of logs (`def_shaders\def_vertex` with the
//! charred `fx\fx_wood_fire` bark) that burns through a fire particle, a light
//! and its vertex lighting. v0.10b keeps none of those, so in 0.26 the logs
//! are near-black blocks under the skylight. The converter hides the log
//! surfaces (an empty material, as on native invisible surfaces) and places
//! the native composite a 0.26 map uses for a campfire at each one: the
//! `props/light/bonefire_01_on` model (logs, stones and emissive embers) with
//! its collision, the `fx/lights/bonfire` flame particle and the fire sound
//! (all as on level_05 day), plus a warm unshadowed point light cloned from
//! level_06's.
use super::*;

/// Native project the bonfire composite is cloned from.
const BONFIRE_PROJECT: &str = "projects/level_05/day_4/client_project";
/// Native project the point light is cloned from.
const LIGHT_PROJECT: &str = "projects/level_06/day_4/client_project";
const BONFIRE_MODEL: &str = "props/light/bonefire_01_on";
const BONFIRE_PARTICLE: &str = "fx/lights/bonfire";
const BONFIRE_SOUND: &str = "scapes/lobby/fire";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Campfires {
    /// The donor material of the X-Ray log surfaces, e.g.
    /// `material_instances/ported/cs/sport_center/shader_0004.material`.
    /// Every surface using it is hidden.
    pub material: String,
    /// Ground point (centre of the log pile, its lowest height) of each fire.
    pub fires: Vec<[f32; 3]>,
    /// Point light range (m), intensity and colour. Defaults: 6 m, 3.0,
    /// (1.0, 0.55, 0.25).
    #[serde(default = "default_range")]
    pub light_range: f32,
    #[serde(default = "default_intensity")]
    pub light_intensity: f32,
    #[serde(default = "default_color")]
    pub light_color: [f32; 3],
}

fn default_range() -> f32 {
    6.0
}
fn default_intensity() -> f32 {
    3.0
}
fn default_color() -> [f32; 3] {
    [1.0, 0.55, 0.25]
}

impl Campfires {
    /// The material name model settings use for the hidden log surfaces.
    pub fn settings_material(&self) -> Result<String> {
        self.material
            .strip_prefix("material_instances/")
            .and_then(|p| p.strip_suffix(".material"))
            .map(str::to_owned)
            .with_context(|| format!("campfire material {} is not a material_instances path", self.material))
    }
}

/// Hide every surface of `settings` (a model settings config) whose material
/// is `material`. Returns the surfaces hidden.
pub fn hide_surfaces(settings: &mut ConfigValue, material: &str) -> Result<Vec<String>> {
    let Some(ConfigValue::Named(surfaces)) = settings.named_mut("material_settings") else {
        return Ok(Vec::new());
    };
    let mut hidden = Vec::new();
    for surface in surfaces {
        if string(&surface.value, "material_name").map(|m| m.replace('\\', "/")) == Some(material.to_owned()) {
            surface.value.set_path("material_name", ConfigValue::String(String::new()))?;
            hidden.push(surface.name.clone());
        }
    }
    Ok(hidden)
}

fn find<'a>(objects: &'a [ConfigValue], pred: impl Fn(&ConfigValue) -> bool, what: &str) -> Result<&'a ConfigValue> {
    objects.iter().find(|o| pred(o)).with_context(|| format!("native template has no {what}"))
}

fn push(project: &mut ConfigValue, group: &str, value: ConfigValue) -> Result<()> {
    match project.named_mut(group) {
        Some(ConfigValue::Indexed(items)) => {
            items.push(value);
            Ok(())
        }
        _ => bail!("project has no {group} array"),
    }
}

/// Add the native bonfire composite at every fire. Returns a note.
pub(super) fn emit(map: &Map, fires: &Campfires, target: &mut ResourceDb, project: &mut ConfigValue) -> Result<String> {
    ensure!(!fires.fires.is_empty(), "campfires lists no fire");
    ensure!(
        fires.light_range.is_finite() && (0.5..=30.0).contains(&fires.light_range)
            && fires.light_intensity.is_finite() && (0.0..=20.0).contains(&fires.light_intensity)
            && fires.light_color.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
        "invalid campfire light"
    );
    for model in [format!("models/{BONFIRE_MODEL}.model/settings"), format!("particles/{BONFIRE_PARTICLE}.particle")] {
        ensure!(target.is_file(&model), "native {model} is missing");
    }
    let native = cfg(target, BONFIRE_PROJECT)?;
    let lights = cfg(target, LIGHT_PROJECT)?;
    let visuals = array(native.named("render_visuals").context("native render visuals")?)?;
    let collisions = array(native.named("collision_objects").context("native collision objects")?)?;
    let objects = array(native.named("client_objects").context("native client objects")?)?;
    let visual = find(visuals, |o| string(o, "lib_name") == Some(BONFIRE_MODEL), "bonfire visual")?.clone();
    let model_path = |o: &ConfigValue| string(o, "lib_name").map(|s| s.replace('\\', "/"));
    let collision = find(
        collisions,
        |o| model_path(o).is_some_and(|p| p.ends_with(&format!("{BONFIRE_MODEL}.model/collision"))),
        "bonfire collision",
    )?
    .clone();
    let walker = find(
        collisions,
        |o| model_path(o).is_some_and(|p| p.ends_with(&format!("{BONFIRE_MODEL}.model/walker_collision"))),
        "bonfire walker collision",
    )?
    .clone();
    let particle = find(
        objects,
        |o| string(o, "game_object_type") == Some("particle") && string(o, "lib_name") == Some(BONFIRE_PARTICLE),
        "bonfire particle",
    )?
    .clone();
    let sound = find(
        objects,
        |o| string(o, "game_object_type") == Some("object_sound") && string(o, "sound_name") == Some(BONFIRE_SOUND),
        "fire sound",
    )?
    .clone();
    let light = find(
        array(lights.named("client_objects").context("native lights")?)?,
        |o| string(o, "game_object_type") == Some("light") && string(o, "lib_name") == Some("point_light"),
        "point light",
    )?
    .clone();
    let base = map
        .donor_project
        .clone()
        .unwrap_or_else(|| map.level.clone());
    for (i, at) in fires.fires.iter().enumerate() {
        ensure!(at.iter().all(|v| v.is_finite()), "campfire {i} is not finite");
        let place = |mut o: ConfigValue, lift: f32, name: &str| -> Result<ConfigValue> {
            o.set_path("position", ConfigValue::Float3([at[0], at[1] + lift, at[2]]))?;
            o.set_path("rotation", ConfigValue::Float3([0.0, 0.0, 0.0]))?;
            if o.named("full_name").is_some() {
                o.set_path("full_name", ConfigValue::String(format!("srp/campfires/{base}_{i:02}/{name}")))?;
                o.set_path("name", ConfigValue::String(name.into()))?;
            }
            if o.named("is_temporary").is_some() {
                o.set_path("is_temporary", ConfigValue::Boolean(false))?;
            }
            Ok(o)
        };
        push(project, "render_visuals", place(visual.clone(), 0.0, "bonfire")?)?;
        push(project, "collision_objects", place(collision.clone(), 0.0, "bonfire")?)?;
        push(project, "collision_objects", place(walker.clone(), 0.0, "bonfire")?)?;
        push(project, "client_objects", place(particle.clone(), 0.0, "bonfire")?)?;
        push(project, "client_objects", place(sound.clone(), 0.7, "fire_sound")?)?;
        let mut l = place(light.clone(), 0.6, "fire_light")?;
        l.set_path("range", ConfigValue::Float(fires.light_range))?;
        l.set_path("intensity", ConfigValue::Float(fires.light_intensity))?;
        l.set_path("color", ConfigValue::Float3(fires.light_color))?;
        l.set_path("is_cast_shadow", ConfigValue::Boolean(false))?;
        l.set_path("enabled", ConfigValue::Boolean(true))?;
        push(project, "client_objects", l)?;
    }
    Ok(format!(
        "{}: {} campfires: X-Ray log surfaces ({}) hidden, native {BONFIRE_MODEL} with collision, {BONFIRE_PARTICLE} particle, {BONFIRE_SOUND} and a {:.0} m unshadowed point light (intensity {}) at each",
        map.donor_project(SourceBuild::V010b),
        fires.fires.len(),
        fires.material,
        fires.light_range,
        fires.light_intensity
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_only_the_campfire_material() {
        let surface = |m: &str| {
            ConfigValue::Named(vec![NamedValue { name: "material_name".into(), value: ConfigValue::String(m.into()) }])
        };
        let mut settings = ConfigValue::Named(vec![NamedValue {
            name: "material_settings".into(),
            value: ConfigValue::Named(vec![
                NamedValue { name: "shader_0004_part_000".into(), value: surface("ported/cs/sport_center/shader_0004") },
                NamedValue { name: "shader_0005_part_000".into(), value: surface("ported/cs/sport_center/shader_0005") },
            ]),
        }]);
        let fires = Campfires {
            material: "material_instances/ported/cs/sport_center/shader_0004.material".into(),
            fires: vec![[0.0; 3]],
            light_range: 6.0,
            light_intensity: 3.0,
            light_color: [1.0, 0.55, 0.25],
        };
        let hidden = hide_surfaces(&mut settings, &fires.settings_material().unwrap()).unwrap();
        assert_eq!(hidden, vec!["shader_0004_part_000".to_string()]);
        let m = |s: &str| {
            settings
                .named("material_settings")
                .and_then(|m| m.named(s))
                .and_then(|m| m.named("material_name"))
                .cloned()
        };
        assert_eq!(m("shader_0004_part_000"), Some(ConfigValue::String(String::new())));
        assert_eq!(m("shader_0005_part_000"), Some(ConfigValue::String("ported/cs/sport_center/shader_0005".into())));
        let bad = Campfires { material: "textures/x.dds".into(), ..fires };
        assert!(bad.settings_material().is_err());
    }
}
