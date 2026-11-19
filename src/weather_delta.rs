// SPDX-License-Identifier: GPL-3.0-or-later
//! A ported map's weather project, made from the day project skhid ships: the
//! donor's own difference between its day and its weather project, laid over
//! the shipped day.
//!
//! The shipped day already carries every authored edit (spawns, removed
//! anomalies, lamps, pools); the donor's day and weather projects are the same
//! scene, so what differs between them is the weather. Client objects are
//! matched by `full_name`, render visuals and collisions by their content.
//! A removed object goes, a changed object takes only the fields the weather
//! changed (in the shipped schema), an added object is converted after a
//! shipped object of its kind. Nothing else of the shipped day changes.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use crate::binary_config::{ConfigValue, NamedValue};

/// What a weather is called in the shipped game: its project folder
/// (`rain_2`), its layer name and the environment's settings 0.26g has for it.
pub struct Variant<'a> {
    pub project: &'a str,
    pub layer: &'a str,
    /// A native 0.26g environment object of the same weather: its
    /// `properties` name the settings 0.26g reads for it.
    pub native_environment: &'a ConfigValue,
}

/// What the weather added, removed and changed, by kind, and what had no
/// shipped counterpart.
#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    pub removed: BTreeMap<String, usize>,
    pub changed: BTreeMap<String, usize>,
    pub added: BTreeMap<String, usize>,
    /// Weather objects left out, with why.
    pub left_out: BTreeMap<String, String>,
    /// Changed objects the shipped day does not have (an authored removal).
    pub changed_but_not_shipped: BTreeMap<String, usize>,
    /// Fields the shipped day had edited that the weather changes too: the
    /// shipped (authored) value is kept.
    pub conflicts: Vec<String>,
    pub render_visuals: (usize, usize),
    /// Visuals and collisions the weather removes that the shipped day lacks,
    /// or adds that it has already: no change.
    pub already_absent: usize,
    pub collision_objects: (usize, usize),
    pub environment_properties: usize,
    pub lamps_renumbered: usize,
}

fn kind(object: &ConfigValue) -> String {
    match object.named("lib_name") {
        Some(ConfigValue::String(name)) => name.clone(),
        _ => String::new(),
    }
}

fn full_name(object: &ConfigValue) -> Result<&str> {
    match object.named("full_name") {
        Some(ConfigValue::String(name)) => Ok(name),
        _ => bail!("a client object has no full_name"),
    }
}

fn fields(object: &ConfigValue) -> Result<&[NamedValue]> {
    match object {
        ConfigValue::Named(fields) => Ok(fields),
        _ => bail!("a client object is not a named table"),
    }
}

fn list<'a>(project: &'a ConfigValue, key: &str) -> Result<&'a [ConfigValue]> {
    match project.named(key) {
        Some(ConfigValue::Indexed(items)) => Ok(items),
        None => Ok(&[]),
        _ => bail!("{key} is not a list"),
    }
}

fn is_light(kind: &str) -> bool {
    matches!(kind, "point_light" | "spot_light")
}

/// A field of a donor object in the shipped schema, as the shipped day's own
/// conversion of the same objects has it (patch 14's London): `is_enabled` is
/// a light's `enabled` and nothing elsewhere; an ambient light's colour is a
/// packed ARGB integer.
fn shipped_field(kind: &str, name: &str, value: &ConfigValue) -> Option<(String, ConfigValue)> {
    match (kind, name, value) {
        (k, "is_enabled", v) if is_light(k) => Some(("enabled".into(), v.clone())),
        (_, "is_enabled", _) => None,
        ("ambient_light", "color", ConfigValue::Float3(c)) => {
            let byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
            let packed = 0xff00_0000 | byte(c[0]) << 16 | byte(c[1]) << 8 | byte(c[2]);
            Some(("color".into(), ConfigValue::Integer(packed as i32)))
        }
        _ => Some((name.into(), value.clone())),
    }
}

/// `value` in the number type the shipped schema has for the field (`like`):
/// 0.31a2 writes a whole float as an integer and some integers as floats;
/// 0.26g's configs keep one type per key. A fraction is never cut.
fn as_type_of(value: ConfigValue, like: &ConfigValue) -> ConfigValue {
    match (value, like) {
        (ConfigValue::Integer(i), ConfigValue::Float(_)) => ConfigValue::Float(i as f32),
        (ConfigValue::Float(f), ConfigValue::Integer(_)) if f.fract() == 0.0 => {
            ConfigValue::Integer(f as i32)
        }
        (value, _) => value,
    }
}

fn set_field(object: &mut ConfigValue, name: &str, value: ConfigValue) {
    if let ConfigValue::Named(fields) = object {
        match fields.iter_mut().find(|f| f.name == name) {
            Some(field) => field.value = value,
            None => fields.push(NamedValue {
                name: name.into(),
                value,
            }),
        }
    }
}

/// A probe names its cubemap per project (`textures.<project>`, as native
/// 0.26g maps have every project's) beside `texture`, its own project's.
fn set_probe_texture(object: &mut ConfigValue, project: &str, texture: &ConfigValue) {
    match object.named_mut("textures") {
        Some(table @ ConfigValue::Named(_)) => set_field(table, project, texture.clone()),
        _ => set_field(
            object,
            "textures",
            ConfigValue::Named(vec![NamedValue {
                name: project.into(),
                value: texture.clone(),
            }]),
        ),
    }
}

/// A donor object in the shipped schema, after `template` (a shipped object
/// of its kind): the template's fields in its order, each the donor's where
/// the donor has it, else the template's.
fn convert(donor: &ConfigValue, template: &ConfigValue, project: &str) -> Result<ConfigValue> {
    let kind = kind(donor);
    let mut mapped: BTreeMap<String, ConfigValue> = BTreeMap::new();
    for field in fields(donor)? {
        if let Some((name, value)) = shipped_field(&kind, &field.name, &field.value) {
            mapped.insert(name, value);
        }
    }
    let mut out = Vec::new();
    for field in fields(template)? {
        let value = match field.name.as_str() {
            // The template's own cubemaps are not this probe's.
            "textures" if kind == "environment_probe" => continue,
            _ => mapped
                .get(&field.name)
                .map(|v| as_type_of(v.clone(), &field.value))
                .unwrap_or_else(|| field.value.clone()),
        };
        out.push(NamedValue {
            name: field.name.clone(),
            value,
        });
    }
    let mut object = ConfigValue::Named(out);
    if kind == "environment_probe"
        && let Some(texture) = mapped.get("texture")
    {
        set_probe_texture(&mut object, project, texture);
    }
    Ok(object)
}

/// The environment: the shipped day's object, its `properties` those of the
/// shipped day and of 0.26g's own environment of this weather, each set to the
/// donor weather's first key frame where the donor has it (patch 14 made the
/// day's so, from key frame 0 over native level_01 day's names).
fn environment(
    shipped: &ConfigValue,
    donor: &ConfigValue,
    variant: &Variant<'_>,
) -> Result<(ConfigValue, usize)> {
    let frame = donor
        .named("effect")
        .and_then(|e| e.named("key_frames"))
        .and_then(|k| match k {
            ConfigValue::Indexed(frames) => frames.first(),
            _ => None,
        })
        .and_then(|f| f.named("properties"))
        .context("the donor environment has no first key frame")?;
    let shipped_properties = fields(
        shipped
            .named("properties")
            .context("the shipped environment has no properties")?,
    )?;
    let native_properties = fields(
        variant
            .native_environment
            .named("properties")
            .context("the native environment has no properties")?,
    )?;
    let mut names: Vec<&NamedValue> = shipped_properties.iter().collect();
    for field in native_properties {
        if !names.iter().any(|f| f.name == field.name) {
            names.push(field);
        }
    }
    let mut properties = Vec::new();
    let mut from_donor = 0;
    for field in names {
        let value = match frame.named(&field.name) {
            Some(v) => {
                from_donor += 1;
                as_type_of(v.clone(), &field.value)
            }
            None => field.value.clone(),
        };
        properties.push(NamedValue {
            name: field.name.clone(),
            value,
        });
    }
    let mut object = shipped.clone();
    set_field(&mut object, "properties", ConfigValue::Named(properties));
    set_field(&mut object, "layer", ConfigValue::String(variant.layer.into()));
    Ok((object, from_donor))
}

const SAME: f32 = 1e-3;

fn near(a: &ConfigValue, b: &ConfigValue) -> bool {
    match (a, b) {
        (ConfigValue::Float(x), ConfigValue::Float(y)) => (x - y).abs() <= SAME,
        (ConfigValue::Float2(x), ConfigValue::Float2(y)) => {
            x.iter().zip(y).all(|(p, q)| (p - q).abs() <= SAME)
        }
        (ConfigValue::Float3(x), ConfigValue::Float3(y)) => {
            x.iter().zip(y).all(|(p, q)| (p - q).abs() <= SAME)
        }
        (ConfigValue::Float4(x), ConfigValue::Float4(y)) => {
            x.iter().zip(y).all(|(p, q)| (p - q).abs() <= SAME)
        }
        (ConfigValue::Named(x), ConfigValue::Named(y)) => {
            x.len() == y.len()
                && x.iter().all(|f| y.iter().any(|g| g.name == f.name && near(&f.value, &g.value)))
        }
        (ConfigValue::Indexed(x), ConfigValue::Indexed(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| near(p, q))
        }
        (x, y) => x == y,
    }
}

/// `list`'s entries the donor's day has and its weather does not, and the
/// weather's the day has not, matched one to one within float noise.
fn entry_delta<'a>(day: &[ConfigValue], weather: &'a [ConfigValue]) -> (Vec<ConfigValue>, Vec<&'a ConfigValue>) {
    let mut unmatched: Vec<bool> = vec![true; weather.len()];
    let mut removed = Vec::new();
    for entry in day {
        match weather
            .iter()
            .enumerate()
            .position(|(i, w)| unmatched[i] && near(entry, w))
        {
            Some(i) => unmatched[i] = false,
            None => removed.push(entry.clone()),
        }
    }
    let added = weather
        .iter()
        .zip(&unmatched)
        .filter(|(_, u)| **u)
        .map(|(w, _)| w)
        .collect();
    (removed, added)
}

/// The shipped list without the removed entries, the added ones appended;
/// and each kept entry's new index.
fn apply_entries(
    shipped: &[ConfigValue],
    removed: &[ConfigValue],
    added: &[&ConfigValue],
    what: &str,
) -> Result<(Vec<ConfigValue>, BTreeMap<usize, usize>, usize)> {
    let mut gone = vec![false; shipped.len()];
    let mut absent = 0;
    for entry in removed {
        // An entry skhid removed from the day already is gone in the weather too.
        match shipped
            .iter()
            .enumerate()
            .position(|(i, s)| !gone[i] && near(entry, s))
        {
            Some(at) => gone[at] = true,
            None => absent += 1,
        }
    }
    let _ = what;
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    for (i, entry) in shipped.iter().enumerate() {
        if !gone[i] {
            index.insert(i, out.len());
            out.push(entry.clone());
        }
    }
    // An entry the weather adds that the shipped day has already (patch 14
    // wrote some of the day's as the donor's weather has them, `ford[3]` as
    // `ford`) is not added twice.
    let kept = out.len();
    let mut claimed = vec![false; kept];
    for entry in added {
        match out[..kept]
            .iter()
            .enumerate()
            .position(|(i, s)| !claimed[i] && near(entry, s))
        {
            Some(i) => {
                claimed[i] = true;
                absent += 1;
            }
            None => out.push((*entry).clone()),
        }
    }
    Ok((out, index, absent))
}

/// `srp_lamps`' render-visual indices after the visuals moved.
fn renumber_lamps(project: &mut ConfigValue, index: &BTreeMap<usize, usize>) -> Result<usize> {
    let Some(ConfigValue::Named(lamps)) = project
        .named_mut("srp_lamps")
        .and_then(|l| l.named_mut("lamps"))
    else {
        return Ok(0);
    };
    let mut moved = 0;
    for lamp in lamps.iter_mut() {
        let Some(ConfigValue::Named(visuals)) = lamp.value.named_mut("visuals") else {
            continue;
        };
        for visual in visuals.iter_mut() {
            let Some(ConfigValue::Integer(old)) = visual.value.named_mut("index") else {
                continue;
            };
            let new = *index
                .get(&(*old as usize))
                .with_context(|| format!("lamp {} loses its render visual {old}", lamp.name))?;
            if new != *old as usize {
                moved += 1;
            }
            *old = new as i32;
        }
    }
    Ok(moved)
}

/// Object kinds 0.26g has no object type for: left out of every weather.
const NOT_IN_026G: &[&str] = &["track", "sound_environment"];

/// The weather project of `variant`, from the shipped day and the donor's day
/// and weather projects of the same mode.
pub fn weather_project(
    donor_day: &ConfigValue,
    donor_weather: &ConfigValue,
    shipped_day: &ConfigValue,
    variant: &Variant<'_>,
) -> Result<(ConfigValue, Report)> {
    let mut report = Report::default();
    let index = |objects: &[ConfigValue]| -> Result<BTreeMap<String, ConfigValue>> {
        let mut map = BTreeMap::new();
        for o in objects {
            ensure!(
                map.insert(full_name(o)?.to_owned(), o.clone()).is_none(),
                "two client objects are called {}",
                full_name(o)?
            );
        }
        Ok(map)
    };
    let day = index(list(donor_day, "client_objects")?)?;
    let weather = index(list(donor_weather, "client_objects")?)?;
    let shipped = list(shipped_day, "client_objects")?;
    let shipped_names: BTreeSet<&str> = shipped.iter().filter_map(|o| full_name(o).ok()).collect();

    let mut objects = Vec::new();
    for object in shipped {
        let name = full_name(object)?;
        let kind = kind(object);
        let (Some(d), Some(w)) = (day.get(name), weather.get(name)) else {
            if day.contains_key(name) {
                *report.removed.entry(kind).or_default() += 1;
                continue;
            }
            objects.push(object.clone()); // an object skhid added
            continue;
        };
        if kind == "environment" {
            let (env, n) = environment(object, w, variant)?;
            report.environment_properties = n;
            *report.changed.entry(kind).or_default() += 1;
            objects.push(env);
            continue;
        }
        if d == w {
            objects.push(object.clone());
            continue;
        }
        let mut out = object.clone();
        for field in fields(w)? {
            if d.named(&field.name) == Some(&field.value) {
                continue;
            }
            let Some((to, value)) = shipped_field(&kind, &field.name, &field.value) else {
                continue;
            };
            if object.named(&to).is_none() {
                continue; // a field the shipped schema has not
            }
            // A field skhid edited on the day keeps the edit (a probe's
            // fitted radius): the authored value wins over the weather's.
            if let Some(before) = d.named(&field.name)
                && let Some((_, before)) = shipped_field(&kind, &field.name, before)
                && object.named(&to).is_some_and(|now| !near(now, &before))
            {
                report.conflicts.push(format!("{name}.{to}"));
                continue;
            }
            let value = match object.named(&to) {
                Some(like) => as_type_of(value, like),
                None => value,
            };
            if to == "texture" && kind == "environment_probe" {
                set_probe_texture(&mut out, variant.project, &value);
            }
            set_field(&mut out, &to, value);
        }
        *report.changed.entry(kind).or_default() += 1;
        objects.push(out);
    }
    for (name, d) in &day {
        if weather.contains_key(name) && d != &weather[name] && !shipped_names.contains(name.as_str()) {
            *report.changed_but_not_shipped.entry(kind(d)).or_default() += 1;
        }
    }
    // Added objects, in the donor weather's order.
    for w in list(donor_weather, "client_objects")? {
        let name = full_name(w)?;
        if day.contains_key(name) {
            continue;
        }
        let kind = kind(w);
        if NOT_IN_026G.contains(&kind.as_str()) {
            report
                .left_out
                .insert(name.into(), format!("{kind}: no 0.26g object type"));
            continue;
        }
        let template = shipped
            .iter()
            .find(|s| self::kind(s) == kind)
            .or_else(|| {
                let family = w.named("game_object_type");
                shipped.iter().find(|s| s.named("game_object_type") == family && family.is_some())
            });
        let Some(template) = template else {
            report
                .left_out
                .insert(name.into(), format!("{kind}: no shipped object of its kind to follow"));
            continue;
        };
        objects.push(convert(w, template, variant.project)?);
        *report.added.entry(kind).or_default() += 1;
    }

    let mut project = shipped_day.clone();
    set_field(&mut project, "client_objects", ConfigValue::Indexed(objects));

    let (removed, added) = entry_delta(list(donor_day, "render_visuals")?, list(donor_weather, "render_visuals")?);
    let (visuals, moved, absent) = apply_entries(list(shipped_day, "render_visuals")?, &removed, &added, "render visual")?;
    report.render_visuals = (removed.len(), added.len());
    report.already_absent += absent;
    set_field(&mut project, "render_visuals", ConfigValue::Indexed(visuals));
    report.lamps_renumbered = renumber_lamps(&mut project, &moved)?;

    let (removed, added) = entry_delta(list(donor_day, "collision_objects")?, list(donor_weather, "collision_objects")?);
    let (collisions, _, absent) = apply_entries(list(shipped_day, "collision_objects")?, &removed, &added, "collision")?;
    report.collision_objects = (removed.len(), added.len());
    report.already_absent += absent;
    set_field(&mut project, "collision_objects", ConfigValue::Indexed(collisions));
    Ok((project, report))
}

/// The shipped schema check: each donor day object converted after its
/// shipped counterpart must be that counterpart, but for fields skhid's own
/// edits changed. Returns the fields that differ, by kind.
pub fn conversion_check(donor_day: &ConfigValue, shipped_day: &ConfigValue, project: &str) -> Result<BTreeMap<String, BTreeMap<String, usize>>> {
    let mut out: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let day: BTreeMap<&str, &ConfigValue> = list(donor_day, "client_objects")?
        .iter()
        .filter_map(|o| full_name(o).ok().map(|n| (n, o)))
        .collect();
    for s in list(shipped_day, "client_objects")? {
        let Some(d) = day.get(full_name(s)?) else { continue };
        let kind = kind(s);
        if kind == "environment" {
            continue;
        }
        let converted = convert(d, s, project)?;
        for field in fields(s)? {
            if converted.named(&field.name).is_none_or(|v| !near(v, &field.value)) {
                *out.entry(kind.clone()).or_default().entry(field.name.clone()).or_default() += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(fields: &[(&str, ConfigValue)]) -> ConfigValue {
        ConfigValue::Named(
            fields
                .iter()
                .map(|(n, v)| NamedValue {
                    name: (*n).into(),
                    value: v.clone(),
                })
                .collect(),
        )
    }
    fn s(x: &str) -> ConfigValue {
        ConfigValue::String(x.into())
    }

    #[test]
    fn ambient_colour_packs_as_shipped() {
        let (_, v) = shipped_field("ambient_light", "color", &ConfigValue::Float3([0.817, 0.845, 1.0])).unwrap();
        assert_eq!(v, ConfigValue::Integer(0xffd0d7ffu32 as i32));
    }

    #[test]
    fn a_changed_field_lands_and_an_authored_one_stays() {
        let probe = |tex: &str, mult: f32, pos: f32| {
            object(&[
                ("full_name", s("p")),
                ("lib_name", s("environment_probe")),
                ("texture", s(tex)),
                ("ambient_multiplier", ConfigValue::Float(mult)),
                ("position", ConfigValue::Float3([pos, 0.0, 0.0])),
            ])
        };
        let root = |objs: Vec<ConfigValue>| object(&[("client_objects", ConfigValue::Indexed(objs))]);
        let native = object(&[("properties", object(&[]))]);
        let v = Variant { project: "rain_2", layer: "rain", native_environment: &native };
        // skhid moved the probe; the weather changes its cubemap only.
        let (out, report) = weather_project(
            &root(vec![probe("day", 1.0, 0.0)]),
            &root(vec![probe("rain", 1.0, 0.0)]),
            &root(vec![probe("day", 1.0, 5.0)]),
            &v,
        )
        .unwrap();
        let p = &list(&out, "client_objects").unwrap()[0];
        assert_eq!(p.named("texture"), Some(&s("rain")));
        assert_eq!(p.named("position"), Some(&ConfigValue::Float3([5.0, 0.0, 0.0])));
        assert_eq!(p.named("textures").and_then(|t| t.named("rain_2")), Some(&s("rain")));
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn a_removed_visual_moves_the_lamps_after_it() {
        let vis = |n: &str| object(&[("lib_name", s(n))]);
        let lamps = object(&[(
            "lamps",
            object(&[("lamp1", object(&[("visuals", object(&[("intact0", object(&[("index", ConfigValue::Integer(2))]))]))]))]),
        )]);
        let root = |v: Vec<ConfigValue>, lamps: Option<&ConfigValue>| {
            let mut f = vec![("client_objects", ConfigValue::Indexed(vec![])), ("render_visuals", ConfigValue::Indexed(v))];
            if let Some(l) = lamps {
                f.push(("srp_lamps", l.clone()));
            }
            object(&f)
        };
        let native = object(&[("properties", object(&[]))]);
        let v = Variant { project: "rain_2", layer: "rain", native_environment: &native };
        let (out, report) = weather_project(
            &root(vec![vis("a"), vis("b"), vis("lamp")], None),
            &root(vec![vis("b"), vis("lamp"), vis("c")], None),
            &root(vec![vis("a"), vis("b"), vis("lamp")], Some(&lamps)),
            &v,
        )
        .unwrap();
        assert_eq!(report.render_visuals, (1, 1));
        assert_eq!(report.lamps_renumbered, 1);
        let index = out.named("srp_lamps").and_then(|l| l.named("lamps")).and_then(|l| l.named("lamp1")).and_then(|l| l.named("visuals")).and_then(|v| v.named("intact0")).and_then(|i| i.named("index"));
        assert_eq!(index, Some(&ConfigValue::Integer(1)));
        assert_eq!(list(&out, "render_visuals").unwrap().len(), 3);
    }
}
