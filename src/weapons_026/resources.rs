// SPDX-License-Identifier: GPL-3.0-or-later
//! Every resource an item config names must exist: in the target (the stock
//! database and the players' patches) or in the bundle. A config naming a
//! missing one is refused before it ships (a gun whose `particles.shoot`
//! named `fx/weapons/m4a1`, which no 0.26g archive has, crashed the client
//! the moment the lobby drew it: exe+0x352308 after "cook of
//! 'fx/weapons/m4a1' [class 58] failed").
//!
//! A config's strings name resources by their key: [`resource_files`] says
//! which file each one is; a path-like string under a key it does not know
//! is refused too, so a new kind of reference is never let through
//! unchecked.

use anyhow::{Result, bail};

use crate::ConfigValue as V;

/// The archive path that must exist for the string `value` at `key`
/// (dotted, indices dropped), `None` for a string that names no resource;
/// an error for a path-like string under a key of unknown kind.
pub fn resource_file(key: &str, value: &str) -> Result<Option<String>> {
    if let Some(clip) = value.strip_prefix("resources/") {
        return Ok(Some(format!("{clip}.b-spline")));
    }
    let leaf = key.rsplit('.').next().unwrap_or(key);
    Ok(Some(match leaf {
        "model" if key == "object.model" => format!("models/{value}.skinned_model/settings"),
        "skeleton" if key == "object.skeleton" => format!("animations/skeletons/{value}.skeleton"),
        "shoot" if key == "particles.shoot" => format!("particles/{value}.particle"),
        // A shell model the engine spawns only with bullet_shells_count > 0,
        // which no 0.26g weapon has (every native config names one no
        // archive has: `shells/5_45x040`).
        "bullet_shells" if key == "particles.bullet_shells" => return Ok(None),
        "user_animations" if key == "user_animations" => value.to_owned(),
        _ if !value.contains('/') => return Ok(None),
        _ => bail!("{key} names {value}, a resource of a kind the port does not check"),
    }))
}

/// A sound a state names, by its resource type.
pub fn sound_file(kind: i32, name: &str) -> Result<String> {
    Ok(match kind {
        256 => format!("sounds/single/{name}.high.ogg"),
        257 => format!("sounds/composite/{name}.composite_sound"),
        258 => format!("sounds/collections/{name}.sound_collection"),
        other => bail!("sound {name} of unknown resource type {other}"),
    })
}

/// The resources `cfg` names (archive paths, with the key naming each).
pub fn resource_files(cfg: &V) -> Result<Vec<(String, String)>> {
    fn walk(v: &V, key: &str, out: &mut Vec<(String, String)>) -> Result<()> {
        match v {
            V::Named(fields) => {
                for f in fields {
                    let k = if key.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{key}.{}", f.name)
                    };
                    walk(&f.value, &k, out)?;
                }
            }
            V::Indexed(items) => {
                if let [V::Integer(kind), V::String(name)] = items.as_slice() {
                    out.push((key.to_owned(), sound_file(*kind, name)?));
                    return Ok(());
                }
                for item in items {
                    walk(item, key, out)?;
                }
            }
            V::String(s) => {
                if let Some(file) = resource_file(key, s)? {
                    out.push((key.to_owned(), file));
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut out = vec![];
    walk(cfg, "", &mut out)?;
    Ok(out)
}

/// Refuses `cfg` when a resource it names is in neither the target nor the
/// bundle (`exists`), listing every missing one; a name the live config it
/// is made from has at the same key (`live_too`) is the game's own and kept.
pub fn check_resources(
    name: &str,
    cfg: &V,
    exists: &dyn Fn(&str) -> bool,
    live_too: &dyn Fn(&str, &str) -> bool,
) -> Result<()> {
    let missing: Vec<String> = resource_files(cfg)?
        .into_iter()
        .filter(|(key, file)| !exists(file) && !live_too(key, file))
        .map(|(key, file)| format!("{key}: {file}"))
        .collect();
    if !missing.is_empty() {
        bail!(
            "{name} names resources no archive has (the game would fail to cook them):\n  {}",
            missing.join("\n  ")
        );
    }
    Ok(())
}
