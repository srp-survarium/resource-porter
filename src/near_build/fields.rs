// SPDX-License-Identifier: GPL-3.0-or-later
//! An old-form object with the fields 0.26g's own projects give its kind.
//!
//! 0.26g's projects are the definition: for each list and `game_object_type`,
//! the fields its objects carry, with their number types. A donor object keeps
//! the fields 0.26g's objects of its kind have and loses the others (each
//! reported by name); a field every 0.26g object of the kind has and the donor
//! lacks takes 0.26g's value when that is one value for the whole kind. A kind
//! 0.26g has no object of is left out.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use crate::binary_config::{ConfigValue, NamedValue};

/// How 0.26g's objects of a kind use one field.
#[derive(Debug)]
struct FieldUse {
    count: usize,
    first: ConfigValue,
    constant: bool,
    integer: bool,
    float: bool,
}

#[derive(Debug, Default)]
struct Kind {
    objects: usize,
    fields: BTreeMap<String, FieldUse>,
}

/// What 0.26g's projects hold, by list and kind.
#[derive(Debug, Default)]
pub struct Templates {
    kinds: BTreeMap<(String, String), Kind>,
    /// `lib_guid` by kind and `lib_name` (the editor's library entry).
    lib_guids: BTreeMap<(String, String), String>,
    /// The field names of each static list's entries.
    statics: BTreeMap<String, BTreeSet<String>>,
    /// The environment's settings: every name a 0.26g project sets.
    environment: BTreeMap<String, FieldUse>,
}

const STATIC_LISTS: &[&str] = &["render_visuals", "collision_objects", "temporary_render_visuals", "temporary_collision_objects"];

fn text<'a>(object: &'a ConfigValue, key: &str) -> Option<&'a str> {
    match object.named(key) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}

fn note_field(fields: &mut BTreeMap<String, FieldUse>, field: &NamedValue) {
    let integer = matches!(field.value, ConfigValue::Integer(_));
    let float = matches!(field.value, ConfigValue::Float(_));
    match fields.get_mut(&field.name) {
        Some(known) => {
            known.count += 1;
            known.constant &= known.first == field.value;
            known.integer |= integer;
            known.float |= float;
        }
        None => {
            fields.insert(field.name.clone(), FieldUse { count: 1, first: field.value.clone(), constant: true, integer, float });
        }
    }
}

impl Templates {
    /// Adds a 0.26g project's objects.
    pub fn learn(&mut self, project: &ConfigValue) {
        let mut object = |list: &str, o: &ConfigValue| {
            let (Some(kind), ConfigValue::Named(fields)) = (text(o, "game_object_type"), o) else { return };
            let known = self.kinds.entry((list.to_owned(), kind.to_owned())).or_default();
            known.objects += 1;
            for field in fields {
                note_field(&mut known.fields, field);
            }
            if let (Some(lib), Some(guid)) = (text(o, "lib_name"), text(o, "lib_guid")) {
                self.lib_guids.insert((kind.to_owned(), lib.to_owned()), guid.to_owned());
            }
            if kind == "environment"
                && let Some(ConfigValue::Named(properties)) = o.named("properties")
            {
                for field in properties {
                    note_field(&mut self.environment, field);
                }
            }
        };
        if let Some(ConfigValue::Indexed(objects)) = project.named("client_objects") {
            for o in objects {
                object("client_objects", o);
            }
        }
        if let Some(ConfigValue::Named(lists)) = project.named("server_objects") {
            for list in lists {
                if let ConfigValue::Indexed(objects) = &list.value {
                    for o in objects {
                        object(&list.name, o);
                    }
                }
            }
        }
        for list in STATIC_LISTS {
            if let Some(ConfigValue::Indexed(entries)) = project.named(list) {
                let names = self.statics.entry((*list).to_owned()).or_default();
                for entry in entries {
                    if let ConfigValue::Named(fields) = entry {
                        names.extend(fields.iter().map(|f| f.name.clone()));
                    }
                }
            }
        }
    }

    pub fn has_kind(&self, list: &str, kind: &str) -> bool {
        self.kinds.contains_key(&(list.to_owned(), kind.to_owned()))
    }
}

/// What the field sets changed, by kind.
#[derive(Debug, Default, serde::Serialize)]
pub struct FieldReport {
    /// Objects of kinds 0.26g has none of.
    pub kinds_left_out: BTreeMap<String, usize>,
    /// Donor fields 0.26g's objects of the kind do not have: `kind.field`.
    pub fields_dropped: BTreeMap<String, usize>,
    /// Fields added with 0.26g's one value for the kind.
    pub fields_added: BTreeMap<String, usize>,
    /// Fields every 0.26g object of the kind has, with varying values, that
    /// the donor lacks and no rule fills: the object goes without.
    pub fields_missing: BTreeMap<String, usize>,
    /// Environment settings of the donor that no 0.26g project sets.
    pub environment_not_in_026g: Vec<String>,
    pub environment_settings: usize,
    /// Settings whose key frames differ: the first frame's value is used.
    pub environment_animated: Vec<String>,
}

/// The number as 0.26g's objects write the field: both builds store a whole
/// float as an integer, so only a field 0.26g always writes one way changes,
/// and a fraction is never cut.
fn as_written(value: ConfigValue, like: &FieldUse) -> ConfigValue {
    match value {
        ConfigValue::Integer(i) if like.float && !like.integer => ConfigValue::Float(i as f32),
        ConfigValue::Float(f) if like.integer && !like.float && f.fract() == 0.0 => ConfigValue::Integer(f as i32),
        other => other,
    }
}

/// 0.26g packs an ambient light's colour as ARGB bytes.
fn packed_colour(c: [f32; 3]) -> ConfigValue {
    let byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    ConfigValue::Integer((0xff00_0000 | byte(c[0]) << 16 | byte(c[1]) << 8 | byte(c[2])) as i32)
}

/// What the project's own rules fill in.
pub struct Scene<'a> {
    /// The project folder's name (`day_3`): a probe lists its cubemap under it.
    pub project: &'a str,
    /// The environment's layer (`day`, `rain`, `evening`), as 0.26g names it.
    pub weather_layer: &'a str,
}

impl Templates {
    /// The object with 0.26g's fields, or none for a kind 0.26g has not.
    pub fn object(&self, list: &str, object: &ConfigValue, scene: &Scene<'_>, report: &mut FieldReport) -> Result<Option<ConfigValue>> {
        let kind = text(object, "game_object_type").context("an object has no game_object_type")?;
        let Some(known) = self.kinds.get(&(list.to_owned(), kind.to_owned())) else {
            *report.kinds_left_out.entry(format!("{list}: {kind}")).or_default() += 1;
            return Ok(None);
        };
        if kind == "environment" {
            return Ok(Some(self.environment(object, known, scene, report)?));
        }
        let ConfigValue::Named(fields) = object else { bail!("an object is not a table") };
        let mut out = Vec::new();
        for field in fields {
            let Some(like) = known.fields.get(&field.name) else {
                *report.fields_dropped.entry(format!("{kind}.{}", field.name)).or_default() += 1;
                continue;
            };
            let value = match (kind, field.name.as_str(), &field.value) {
                ("ambient_light", "color", ConfigValue::Float3(c)) if matches!(like.first, ConfigValue::Integer(_)) => packed_colour(*c),
                (_, _, value) => as_written(value.clone(), like),
            };
            out.push(NamedValue { name: field.name.clone(), value });
        }
        let mut out = ConfigValue::Named(out);
        for (name, like) in &known.fields {
            if out.named(name).is_some() || like.count < known.objects {
                continue;
            }
            let value = match name.as_str() {
                // The editor's record of a member's place in its group: the
                // game's object loaders do not read it.
                "offset" => continue,
                "layer" => ConfigValue::String("default".into()),
                "lib_guid" => {
                    let lib = text(&out, "lib_name").unwrap_or_default();
                    match self.lib_guids.get(&(kind.to_owned(), lib.to_owned())) {
                        Some(guid) => ConfigValue::String(guid.clone()),
                        None if like.constant => like.first.clone(),
                        None => {
                            *report.fields_missing.entry(format!("{kind}.lib_guid ({lib})")).or_default() += 1;
                            continue;
                        }
                    }
                }
                "textures" if kind == "environment_probe" => match out.named("texture") {
                    Some(texture) => ConfigValue::Named(vec![NamedValue { name: scene.project.into(), value: texture.clone() }]),
                    None => continue,
                },
                _ if like.constant => like.first.clone(),
                _ => {
                    *report.fields_missing.entry(format!("{kind}.{name}")).or_default() += 1;
                    continue;
                }
            };
            *report.fields_added.entry(format!("{kind}.{name}")).or_default() += 1;
            if let ConfigValue::Named(fields) = &mut out {
                fields.push(NamedValue { name: name.clone(), value });
            }
        }
        Ok(Some(out))
    }

    /// The environment as 0.26g writes it: the object's own fields and
    /// `properties`, each setting a 0.26g project sets that the donor has, at
    /// the donor's first key frame. A setting the donor lacks is left to the
    /// game's default; one no 0.26g project sets is reported.
    fn environment(&self, object: &ConfigValue, known: &Kind, scene: &Scene<'_>, report: &mut FieldReport) -> Result<ConfigValue> {
        let frames = match object.named("effect").and_then(|e| e.named("key_frames")) {
            Some(ConfigValue::Indexed(frames)) => frames.as_slice(),
            _ => &[],
        };
        let ConfigValue::Named(own) = object else { bail!("the environment is not a table") };
        // 0.26g's form already: the settings are in `properties`.
        let settings: &[NamedValue] = match (object.named("properties"), frames.first().and_then(|f| f.named("properties"))) {
            (Some(ConfigValue::Named(p)), _) => p,
            (None, Some(ConfigValue::Named(p))) => p,
            _ => own,
        };
        let mut properties = Vec::new();
        for field in settings {
            let Some(like) = self.environment.get(&field.name) else {
                if !known.fields.contains_key(&field.name) && !matches!(field.value, ConfigValue::Named(_) | ConfigValue::Indexed(_)) {
                    report.environment_not_in_026g.push(field.name.clone());
                }
                continue;
            };
            if frames.iter().skip(1).any(|f| f.named("properties").and_then(|p| p.named(&field.name)).is_some_and(|v| v != &field.value)) {
                report.environment_animated.push(field.name.clone());
            }
            properties.push(NamedValue { name: field.name.clone(), value: as_written(field.value.clone(), like) });
        }
        report.environment_settings = properties.len();
        let mut out = Vec::new();
        for field in own {
            if field.name == "properties" || field.name == "layer" {
                continue;
            }
            if let Some(like) = known.fields.get(&field.name) {
                out.push(NamedValue { name: field.name.clone(), value: as_written(field.value.clone(), like) });
            }
        }
        for (name, like) in &known.fields {
            if out.iter().any(|f| &f.name == name) || like.count < known.objects || name == "properties" || name == "layer" {
                continue;
            }
            let value = match name.as_str() {
                "lib_guid" => match self.lib_guids.get(&("environment".to_owned(), "environment".to_owned())) {
                    Some(guid) => ConfigValue::String(guid.clone()),
                    None => continue,
                },
                _ if like.constant => like.first.clone(),
                _ => {
                    *report.fields_missing.entry(format!("environment.{name}")).or_default() += 1;
                    continue;
                }
            };
            out.push(NamedValue { name: name.clone(), value });
        }
        out.push(NamedValue { name: "properties".into(), value: ConfigValue::Named(properties) });
        out.push(NamedValue { name: "layer".into(), value: ConfigValue::String(scene.weather_layer.into()) });
        Ok(ConfigValue::Named(out))
    }

    /// A static entry with the fields 0.26g's entries of its list have.
    pub fn static_entry(&self, list: &str, entry: &ConfigValue, report: &mut FieldReport) -> Result<ConfigValue> {
        let ConfigValue::Named(fields) = entry else { bail!("a static is not a table") };
        let known = self.statics.get(list).with_context(|| format!("no 0.26g project has a {list} list"))?;
        let mut out = Vec::new();
        for field in fields {
            if known.contains(&field.name) {
                out.push(field.clone());
            } else {
                *report.fields_dropped.entry(format!("{list}.{}", field.name)).or_default() += 1;
            }
        }
        Ok(ConfigValue::Named(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> ConfigValue {
        ConfigValue::String(x.into())
    }
    fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(fields.into_iter().map(|(name, value)| NamedValue { name: name.into(), value }).collect())
    }
    fn stock() -> Templates {
        let light = |intensity: ConfigValue, layer: &str| {
            named(vec![
                ("game_object_type", s("ambient_light")),
                ("lib_name", s("ambient_light")),
                ("lib_guid", s("05ab")),
                ("type", s("object")),
                ("color", ConfigValue::Integer(-1)),
                ("intensity", intensity),
                ("radius", ConfigValue::Float(2.5)),
                ("layer", s(layer)),
            ])
        };
        let environment = named(vec![
            ("game_object_type", s("environment")),
            ("lib_name", s("environment")),
            ("layer", s("day")),
            ("properties", named(vec![("sun_intensity", ConfigValue::Float(1.5)), ("use_sun", ConfigValue::Boolean(true))])),
        ]);
        let mut t = Templates::default();
        t.learn(&named(vec![
            ("client_objects", ConfigValue::Indexed(vec![light(ConfigValue::Float(0.6), "default"), light(ConfigValue::Integer(1), "day"), environment])),
            ("render_visuals", ConfigValue::Indexed(vec![named(vec![("lib_name", s("a")), ("visual_name", s("a"))])])),
        ]));
        t
    }
    const SCENE: Scene<'static> = Scene { project: "day_3", weather_layer: "day" };

    #[test]
    fn an_object_takes_the_fields_of_its_kind() {
        let donor = named(vec![
            ("game_object_type", s("ambient_light")),
            ("lib_name", s("ambient_light")),
            ("type", s("object")),
            ("is_enabled", ConfigValue::Boolean(true)),
            ("color", ConfigValue::Float3([1.0, 0.5, 0.0])),
            ("intensity", ConfigValue::Float(1.2)),
            ("radius", ConfigValue::Integer(8)),
        ]);
        let mut report = FieldReport::default();
        let out = stock().object("client_objects", &donor, &SCENE, &mut report).unwrap().unwrap();
        assert!(out.named("is_enabled").is_none());
        assert_eq!(out.named("color"), Some(&ConfigValue::Integer(0xffff8000u32 as i32)));
        assert_eq!(out.named("intensity"), Some(&ConfigValue::Float(1.2)));
        // 0.26g writes radius as a float in every object: the whole number follows.
        assert_eq!(out.named("radius"), Some(&ConfigValue::Float(8.0)));
        assert_eq!(out.named("lib_guid"), Some(&s("05ab")));
        assert_eq!(out.named("layer"), Some(&s("default")));
        assert_eq!(report.fields_dropped.get("ambient_light.is_enabled"), Some(&1));
    }

    #[test]
    fn a_kind_026g_lacks_is_left_out_and_the_environment_keeps_its_first_frame() {
        let mut report = FieldReport::default();
        let portal = named(vec![("game_object_type", s("portal_system"))]);
        assert!(stock().object("client_objects", &portal, &SCENE, &mut report).unwrap().is_none());
        assert_eq!(report.kinds_left_out.get("client_objects: portal_system"), Some(&1));
        let frame = |sun: f32| named(vec![("properties", named(vec![("sun_intensity", ConfigValue::Float(sun)), ("wind_strength_r2", ConfigValue::Float(0.05))]))]);
        let environment = named(vec![
            ("game_object_type", s("environment")),
            ("lib_name", s("environment")),
            ("sun_intensity", ConfigValue::Float(9.0)),
            ("effect", named(vec![("key_frames", ConfigValue::Indexed(vec![frame(2.0), frame(3.0)]))])),
        ]);
        let out = stock().object("client_objects", &environment, &SCENE, &mut report).unwrap().unwrap();
        assert_eq!(out.named("properties").and_then(|p| p.named("sun_intensity")), Some(&ConfigValue::Float(2.0)));
        assert_eq!(out.named("layer"), Some(&s("day")));
        assert!(out.named("effect").is_none() && out.named("sun_intensity").is_none());
        assert_eq!(report.environment_not_in_026g, ["wind_strength_r2"]);
        assert_eq!(report.environment_animated, ["sun_intensity"]);
    }

    #[test]
    fn a_static_keeps_the_fields_its_list_has() {
        let mut report = FieldReport::default();
        let entry = named(vec![("lib_name", s("a")), ("visual_name", s("a")), ("use_as_occluder", ConfigValue::Boolean(false))]);
        let out = stock().static_entry("render_visuals", &entry, &mut report).unwrap();
        assert!(out.named("use_as_occluder").is_none() && out.named("visual_name").is_some());
    }
}
