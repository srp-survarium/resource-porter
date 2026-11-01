// SPDX-License-Identifier: GPL-3.0-or-later
//! A gun's item config: the donor's, on a native template's keys.
//!
//! The rules, each as patch 19's 0.29a3 weapons have it (`compare.rs`
//! checks them against those files):
//!
//! * Of every named table the template's keys, with the donor's values
//!   where it has them ([`conform`]); an indexed table (clip lists, sound
//!   lists, recoil) is the donor's whole. So the keys 0.26g lacks go
//!   (`emotion_animations`, the `victory` poses, `details_to_craft`,
//!   `faction_data`, `compatible_ammo`, which becomes `items_compatibility`
//!   rows), and the keys only 0.26g has come from the template.
//! * The build's [`Rules`]: renamed keys, scaled values, state types.
//! * A state or part that plays a clip of the later builds' shared
//!   `animations/single/weapons/base/` (0.26g has none: its rifles play
//!   `weapons/ak/*`) is the template's whole: melee, grenade, recoil.
//! * The native systems' values are the template's ([`NATIVE_PATHS`]):
//!   durability, repair, modification mask, damage fall-off floor, melee,
//!   the muzzle particle count. They are the economy and the melee of 0.26g.
//! * Then the port's own: the balance (`set`), the icons, the card.

use anyhow::{Context, Result, bail, ensure};

use super::donor::Rules;
use super::manifest::Weapon;
use super::{field, get, s};
use crate::{ConfigValue as V, NamedValue};

/// Config paths taken from the template whatever the donor has.
pub const NATIVE_PATHS: &[&str] = &[
    "melee",
    "parameters.degrade_per_match",
    "parameters.degrade_per_use",
    "parameters.repair_cost_ratio",
    "parameters.available_modifications_mask",
    "parameters.min_damage_amount",
    "particles.shoot_pfx_count",
];

/// `donor` on `template`'s keys: of each named table the template's keys,
/// with the donor's values where it has them (recursively) and the
/// template's otherwise. An indexed table is the donor's whole; an integer
/// where the template has a float becomes a float (a float stays one: the
/// engine reads either).
pub fn conform(donor: &V, template: &V) -> V {
    match (donor, template) {
        (V::Named(d), V::Named(t)) => V::Named(
            t.iter()
                .map(|tf| match d.iter().find(|df| df.name == tf.name) {
                    Some(df) => field(&tf.name, conform(&df.value, &tf.value)),
                    None => tf.clone(),
                })
                .collect(),
        ),
        (V::Integer(i), V::Float(_)) => V::Float(*i as f32),
        (d, _) => d.clone(),
    }
}

/// Every key any of `configs` has (named tables merged; an indexed table's
/// items merged into one): what 0.26g's weapons read.
pub fn schema<'a>(configs: impl IntoIterator<Item = &'a V>) -> V {
    fn merge(into: &mut V, v: &V) {
        match (into, v) {
            (V::Named(a), V::Named(b)) => {
                for f in b {
                    match a.iter_mut().find(|x| x.name == f.name) {
                        Some(x) => merge(&mut x.value, &f.value),
                        None => a.push(f.clone()),
                    }
                }
            }
            (V::Indexed(a), V::Indexed(b)) => {
                for item in b {
                    match a.first_mut() {
                        Some(first) => merge(first, item),
                        None => a.push(item.clone()),
                    }
                }
                a.truncate(1);
            }
            _ => {}
        }
    }
    let mut out = V::Named(vec![]);
    for c in configs {
        merge(&mut out, c);
    }
    out
}

/// [`conform`], keeping besides the donor's keys that `schema` (0.26g's
/// weapons together) has: a key some native weapon has is one 0.26g reads
/// (`aim_offset`, `reload_start_time`, ...), though the template lacks it.
pub fn conform_schema(donor: &V, template: &V, schema: &V) -> V {
    conform_schema_at(donor, template, schema, true)
}

/// Keys 0.26g's exe reads (their names are in it) that no native weapon
/// has: a premium weapon's built-in bonuses.
pub const READ_BUT_UNUSED: &[&str] = &["default_modifications"];

fn conform_schema_at(donor: &V, template: &V, schema: &V, top: bool) -> V {
    match (donor, template) {
        (V::Named(d), V::Named(t)) => {
            let mut out: Vec<NamedValue> = t
                .iter()
                .map(|tf| match d.iter().find(|df| df.name == tf.name) {
                    Some(df) => field(
                        &tf.name,
                        conform_schema_at(
                            &df.value,
                            &tf.value,
                            schema.named(&tf.name).unwrap_or(&V::Named(vec![])),
                            false,
                        ),
                    ),
                    None => tf.clone(),
                })
                .collect();
            for df in d {
                if !t.iter().any(|tf| tf.name == df.name)
                    && (schema.named(&df.name).is_some()
                        || (top && READ_BUT_UNUSED.contains(&df.name.as_str())))
                {
                    out.push(df.clone());
                }
            }
            V::Named(out)
        }
        (d, t) => conform(d, t),
    }
}

/// Whether `v` names a clip of the later builds' shared
/// `weapons/base/*` set, which 0.26g lacks.
pub fn plays_base(v: &V) -> bool {
    let mut strings = vec![];
    v.collect_strings(&mut strings);
    strings
        .iter()
        .any(|s| s.contains("animations/single/weapons/base/"))
}

/// The clips `v` names in a set 0.26g has (`weapons/ak`, `weapons/uzi`)
/// that 0.26g's set lacks: a later build's addition to a shared set (patch
/// 19's AEK-919K adds its reload to `weapons/uzi`). Noted; a copy of a
/// native weapon keeps the native states instead (`copy_of_template`).
pub fn foreign_clips(v: &V, foreign: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut strings = vec![];
    v.collect_strings(&mut strings);
    let mut out: Vec<String> = strings
        .into_iter()
        .filter(|s| s.starts_with("resources/animations/") && foreign(s))
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn rename_keys(v: &mut V, rules: &Rules) {
    match v {
        V::Named(fields) => {
            for f in fields.iter_mut() {
                if let Some((_, new)) = rules.renames.iter().find(|(old, _)| *old == f.name) {
                    f.name = (*new).into();
                }
                rename_keys(&mut f.value, rules);
            }
        }
        V::Indexed(items) => items.iter_mut().for_each(|i| rename_keys(i, rules)),
        _ => {}
    }
}

/// A JSON value as `like`'s config type.
pub fn typed(value: &serde_json::Value, like: Option<&V>) -> Result<V> {
    use serde_json::Value as J;
    Ok(match (value, like) {
        (J::Number(n), Some(V::Float(_))) => V::Float(n.as_f64().context("number")? as f32),
        (J::Number(n), Some(V::Integer(_))) => {
            let f = n.as_f64().context("number")?;
            if f.fract() == 0.0 {
                V::Integer(f as i32)
            } else {
                V::Float(f as f32)
            }
        }
        (J::Number(n), None) if n.is_i64() => V::Integer(n.as_i64().unwrap().try_into()?),
        (J::Number(n), None) => V::Float(n.as_f64().context("number")? as f32),
        (J::String(t), Some(V::String(_)) | None) => V::String(t.clone()),
        (J::Bool(b), Some(V::Boolean(_)) | None) => V::Boolean(*b),
        (J::Array(items), _) => V::Indexed(
            items
                .iter()
                .map(|i| typed(i, None))
                .collect::<Result<_>>()?,
        ),
        (value, like) => bail!("{value} where the template has {like:?}"),
    })
}

/// Sets `path` (dotted, `[n]`-free: `a.b.c` or `a.0.b`) to `value`, typed
/// as what is there.
pub fn set_typed(cfg: &mut V, path: &str, value: &serde_json::Value) -> Result<()> {
    let v = typed(value, get(cfg, path)).with_context(|| path.to_owned())?;
    ensure!(get(cfg, path).is_some(), "{path}: no such key");
    cfg.set_path(path, v)
}

/// The weapon card's properties, as the native rifles' (prop id, value):
/// damage, pierce and stopping power x 100, aim time, dispersion, distance,
/// rate of fire, weight. A bolt action's card shows another rate than its
/// `rounds_per_minute` (the Remington 700's 32 for 120): such a gun sets
/// `ui_desc.props_list.6.prop_value` itself.
pub fn card_props(cfg: &V) -> Result<Vec<(i32, String)>> {
    let num = |key: &str| -> Result<f32> {
        match get(cfg, &format!("parameters.{key}")) {
            Some(V::Float(f)) => Ok(*f),
            Some(V::Integer(i)) => Ok(*i as f32),
            other => bail!("parameters.{key} is {other:?}"),
        }
    };
    let fmt = |v: f32| {
        let t = format!("{v:.3}");
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let pct = |v: f32| format!("{}", (v * 100.0).round() as i32);
    Ok(vec![
        (1, pct(num("bullet_damage")?)),
        (3, pct(num("bullet_pierce")?)),
        (133, fmt(num("aim_time")?)),
        (132, pct(num("stopping_power")?)),
        (2, fmt(num("base_dispersion")?)),
        (24, format!("{}", num("min_damage_distance")? as i32)),
        (4, format!("{}", num("rounds_per_minute")? as i32)),
        // The native cards round the weight to 0.1 kg (the SKS's 3.75: 3.8).
        (15, fmt((num("weight")? * 10.0).round() / 10.0)),
    ])
}

/// The gun's config, and what the build did that 0.26g cannot take
/// (states left out, parts kept native), for the record.
pub fn weapon_config(
    rules: &Rules,
    weapon: &Weapon,
    donor: &V,
    template: &V,
    schema: &V,
    foreign: &dyn Fn(&str) -> bool,
) -> Result<(V, Vec<String>)> {
    let mut notes = vec![];
    let mut donor = donor.clone();
    rename_keys(&mut donor, rules);
    for (path, factor) in rules.scaled {
        let scaled = match get(&donor, path) {
            Some(V::Float(f)) => f64::from(*f) * f64::from(*factor),
            Some(V::Integer(i)) => f64::from(*i) * f64::from(*factor),
            Some(other) => bail!("{path} is {other:?}"),
            None => continue,
        };
        // 0.3 x 100 is 30, as the native configs write it.
        let rounded = (scaled * 1e4).round() / 1e4;
        let value = if rounded.fract() == 0.0 {
            V::Integer(rounded as i32)
        } else {
            V::Float(rounded as f32)
        };
        donor.set_path(path, value)?;
    }
    let V::Named(top) = template else {
        bail!("the template is not a table");
    };
    let mut out = Vec::with_capacity(top.len());
    for tf in top {
        let theirs = donor.named(&tf.name);
        let value = match (tf.name.as_str(), theirs) {
            ("states", Some(_)) if weapon.copy_of_template => {
                notes.push("states: the template's (a copy of its weapon)".into());
                tf.value.clone()
            }
            ("states", Some(states)) => {
                states_config(rules, states, &tf.value, schema, foreign, &mut notes)?
            }
            (_, Some(v)) if plays_base(v) => {
                notes.push(format!(
                    "{}: the template's (the donor's plays weapons/base)",
                    tf.name
                ));
                tf.value.clone()
            }
            (_, Some(v)) => conform_schema(
                v,
                &tf.value,
                schema.named(&tf.name).unwrap_or(&V::Named(vec![])),
            ),
            (_, None) => tf.value.clone(),
        };
        out.push(NamedValue {
            name: tf.name.clone(),
            value,
        });
    }
    for f in donor_top_level(&donor) {
        if top.iter().any(|t| t.name == f) {
            continue;
        }
        if schema.named(&f).is_some() || READ_BUT_UNUSED.contains(&f.as_str()) {
            let v = donor.named(&f).unwrap().clone();
            out.push(NamedValue { name: f, value: v });
        } else {
            notes.push(format!("{f}: the donor's, which 0.26g has not, left out"));
        }
    }
    let mut cfg = V::Named(out);
    // A gun's fitted scope (`addons`) is its own, whatever the template has.
    if let Some(addons) = donor.named("addons") {
        let mut merged = match template.named("addons") {
            Some(t) => conform(addons, t),
            None => V::Named(vec![]),
        };
        if let (V::Named(m), V::Named(d)) = (&mut merged, addons) {
            for f in d {
                if !m.iter().any(|x| x.name == f.name) {
                    m.push(f.clone());
                }
            }
        }
        cfg.set_path("addons", merged)?;
    }
    for path in NATIVE_PATHS
        .iter()
        .copied()
        .chain(weapon.from_template.iter().map(String::as_str))
    {
        let v = get(template, path)
            .with_context(|| format!("the template has no {path}"))?
            .clone();
        cfg.set_path(path, v)?;
    }
    if let Some(icon) = &weapon.lobby_icon {
        cfg.set_path(
            "ui_desc.icon",
            V::Indexed(vec![
                V::Integer(icon.row as i32),
                V::Integer(icon.col as i32),
            ]),
        )?;
    }
    cfg.set_path("ui_desc.combat_log_icon", V::Integer(weapon.kill_feed.icon))?;
    for (path, value) in &weapon.set {
        set_typed(&mut cfg, path, value)?;
    }
    if let Some(V::Float(d)) = get(&cfg, "parameters.base_dispersion").cloned()
        && get(&cfg, "dispersion.base_dispersion").is_some()
    {
        cfg.set_path("dispersion.base_dispersion", V::Float(d))?;
    }
    let props = card_props(&cfg)?;
    cfg.set_path(
        "ui_desc.props_list",
        V::Indexed(
            props
                .into_iter()
                .map(|(id, value)| {
                    V::Named(vec![
                        field("prop_value", s(value)),
                        field("prop_id", V::Integer(id)),
                    ])
                })
                .collect(),
        ),
    )?;
    // The card's values the port sets (a bolt action's rate, `ui_desc.card`).
    for (path, value) in weapon
        .set
        .iter()
        .filter(|(p, _)| p.starts_with("ui_desc.props_list"))
    {
        set_typed(&mut cfg, path, value)?;
    }
    Ok((cfg, notes))
}

fn donor_top_level(donor: &V) -> Vec<String> {
    match donor {
        V::Named(f) => f.iter().map(|f| f.name.clone()).collect(),
        _ => vec![],
    }
}

/// The template's states, each the donor's (its type mapped) unless it
/// plays a clip 0.26g lacks; donor states 0.26g has no counterpart of go.
fn states_config(
    rules: &Rules,
    donor: &V,
    template: &V,
    schema: &V,
    foreign: &dyn Fn(&str) -> bool,
    notes: &mut Vec<String>,
) -> Result<V> {
    let (V::Named(theirs), V::Named(ours)) = (donor, template) else {
        bail!("states is not a table");
    };
    let mut out = vec![];
    for t in ours {
        let value = match theirs.iter().find(|d| d.name == t.name) {
            Some(d) => {
                let states = schema.named("states").and_then(|s| s.named(&t.name));
                // The donor's state as it is, less what no native state of
                // the kind has: a state's parts depend on its type (a
                // magazine reload has no shell substates), so none is added.
                let mut merged = match states {
                    Some(sc) => super::build::known_keys_only(&d.value, sc),
                    None => conform(&d.value, &t.value),
                };
                if plays_base(&merged) {
                    notes.push(format!(
                        "states.{}: the template's (the donor's plays weapons/base)",
                        t.name
                    ));
                    t.value.clone()
                } else {
                    for clip in foreign_clips(&merged, foreign) {
                        notes.push(format!("states.{}: adds {clip} to a set 0.26g has", t.name));
                    }
                    let kind = match d.value.named("state_type") {
                        Some(V::Integer(k)) => rules.state_type(*k)?,
                        other => bail!("states.{}.state_type is {other:?}", t.name),
                    };
                    merged.set_path("state_type", V::Integer(kind))?;
                    merged
                }
            }
            None => t.value.clone(),
        };
        out.push(field(&t.name, value));
    }
    for d in theirs {
        if !ours.iter().any(|t| t.name == d.name) {
            notes.push(format!(
                "states.{}: the donor's, which the template has not, left out",
                d.name
            ));
        }
    }
    Ok(V::Named(out))
}
