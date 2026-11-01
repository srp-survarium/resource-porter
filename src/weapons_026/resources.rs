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
//! unchecked. A resource key naming nothing (an empty string) is refused
//! too.
//!
//! Every clip a config names must animate the skeleton it is played on
//! ([`check_clip_skeletons`]): the gun's for its item clips, the player's
//! for the rest. The lobby played the PKP's idle on the Fort-401 and the
//! PM's on the CZ 75 (`preview_item`, the template's) and crashed in
//! bone_matrices_computer::convert_to_object_matrices (exe+0x3A3354, a
//! divide by zero at +0x3A338A), at every sign-in after.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};

use crate::{BinaryConfig, ConfigValue as V};

/// The player's skeleton: the bones of every clip a config plays on the
/// character (`user_animations`, `recoil_animations`, the lobby's
/// `preview`, deaths).
pub const PLAYER_SKELETON: &str = "animations/skeletons/scavengers_01.skeleton";

/// The root-motion channel every clip carries besides its skeleton's bones.
const ROOT_MOTION: &str = "object_movement";

/// The archive path that must exist for the string `value` at `key`
/// (dotted, indices dropped), `None` for a string that names no resource;
/// an error for a path-like string under a key of unknown kind.
pub fn resource_file(key: &str, value: &str) -> Result<Option<String>> {
    if let Some(clip) = value.strip_prefix("resources/") {
        return Ok(Some(format!("{clip}.b-spline")));
    }
    let leaf = key.rsplit('.').next().unwrap_or(key);
    let names_a_resource = matches!(
        key,
        "object.model" | "object.skeleton" | "particles.shoot" | "user_animations"
    ) || key.ends_with("animations")
        || key.starts_with("user_animations_in_place");
    if value.is_empty() && names_a_resource {
        bail!("{key} names no resource (an empty name)");
    }
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
    if name.is_empty() {
        bail!("a sound of resource type {kind} with an empty name");
    }
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

/// Which skeleton a clip at `key` (dotted, indices dropped) is played on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClipOwner {
    /// The gun's: a state's `animations`, the lobby's `preview_item`.
    Gun,
    /// The character's: `user_animations`, `recoil_animations`, the
    /// lobby's `preview`, deaths.
    Player,
}

impl ClipOwner {
    pub fn of(key: &str) -> Self {
        if key == "user_animations_in_place.preview_item"
            || (key.starts_with("states.") && key.ends_with(".animations"))
        {
            Self::Gun
        } else {
            Self::Player
        }
    }
}

/// The bones a `.skeleton` names (its tables nest one level a generation;
/// `mask` leaves mark them).
pub fn skeleton_bones(bytes: &[u8]) -> Result<BTreeSet<String>> {
    fn walk(v: &V, out: &mut BTreeSet<String>) {
        if let V::Named(fields) = v {
            for f in fields.iter().filter(|f| f.name != "mask") {
                out.insert(f.name.clone());
                walk(&f.value, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(&BinaryConfig::parse(bytes)?.root, &mut out);
    if out.is_empty() {
        bail!("the skeleton names no bones");
    }
    Ok(out)
}

/// The bones a clip's `.bones_names` lists.
pub fn clip_bones(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let config = BinaryConfig::parse(bytes)?;
    let Some(V::Indexed(names)) = config.root.named("bones_names") else {
        bail!("bones_names lists no bones");
    };
    names
        .iter()
        .map(|n| match n {
            V::String(s) => Ok(s.clone()),
            _ => bail!("bones_names holds a non-string"),
        })
        .collect()
}

/// Refuses `cfg` when a clip it names animates bones the skeleton it is
/// played on has not ([`ClipOwner`]): `gun` and `player` are the two
/// skeletons' bones; `bones_of` gives a clip's bones by its `.bones_names`
/// path, `None` for one no archive has (the missing-resource check's).
pub fn check_clip_skeletons(
    name: &str,
    cfg: &V,
    gun: &BTreeSet<String>,
    player: &BTreeSet<String>,
    bones_of: &mut dyn FnMut(&str) -> Result<Option<BTreeSet<String>>>,
) -> Result<()> {
    let mut wrong = vec![];
    let mut seen = BTreeSet::new();
    for (key, file) in resource_files(cfg)? {
        let Some(clip) = file.strip_suffix(".b-spline") else {
            continue;
        };
        if !seen.insert((ClipOwner::of(&key), clip.to_owned())) {
            continue;
        }
        let Some(bones) =
            bones_of(&format!("{clip}.bones_names")).with_context(|| format!("{key}: {clip}"))?
        else {
            continue;
        };
        let (skeleton, whose) = match ClipOwner::of(&key) {
            ClipOwner::Gun => (gun, "the gun's"),
            ClipOwner::Player => (player, "the player's"),
        };
        let foreign: Vec<&str> = bones
            .iter()
            .filter(|b| *b != ROOT_MOTION && !skeleton.contains(*b))
            .map(String::as_str)
            .collect();
        if !foreign.is_empty() {
            wrong.push(format!(
                "{key}: {clip} animates bones {whose} skeleton has not ({})",
                foreign.join(", ")
            ));
        }
    }
    if !wrong.is_empty() {
        bail!(
            "{name} plays clips of another skeleton (the game crashes posing them):\n  {}",
            wrong.join("\n  ")
        );
    }
    Ok(())
}
