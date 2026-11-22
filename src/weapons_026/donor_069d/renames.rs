// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d0's clips and sounds under names of their own where 0.26g has an
//! older file of the same name. The core ships a donor file only when the
//! target lacks its path, so 0.69d0's PPSh clips (the drum grip) and its
//! newer sound mixes (`weapons/akm_n`, `weapons/ppsh`, ...) would otherwise
//! play as 0.26g's. A rename moves a donor file to its new path and every
//! name of it in the donor's configs (item configs, composites,
//! collections); the donor still serves the old path too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::binary_config::ConfigValue as V;

/// The renames of a 0.69d0 port (the donor sidecar's `renames`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Renames {
    /// Clip folders, as paths: `animations/single/weapons/ppsh/` to
    /// `animations/single/weapons/ppsh_drum/`.
    #[serde(default)]
    pub clips: BTreeMap<String, String>,
    /// Sound names (of a composite, a collection or a single sound) and
    /// every name under them: `weapons/akm_n` to `weapons/akm_n_069` also
    /// moves `weapons/akm_n_m` and `weapons/akm_n/akm_n_reload_1`.
    #[serde(default)]
    pub sounds: BTreeMap<String, String>,
}

const SOUND_ROOTS: [&str; 3] = ["sounds/single/", "sounds/composite/", "sounds/collections/"];

/// `rest` continues a name past a rule's prefix: nothing, a folder or a
/// suffix (`_m`, `_silencer`), not another word (`ak_74` is not `ak_74u`).
fn continues(rest: &str) -> bool {
    rest.is_empty() || rest.starts_with('/') || rest.starts_with('_')
}

/// The file a sound of resource type `kind` named `name` is.
pub fn sound_path(kind: i32, name: &str) -> String {
    match kind {
        257 => format!("sounds/composite/{name}.composite_sound"),
        258 => format!("sounds/collections/{name}.sound_collection"),
        _ => format!("sounds/single/{name}.high.ogg"),
    }
}

impl Renames {
    /// The donor path that the renamed `path` stands for.
    pub fn donor_path(&self, path: &str) -> Option<String> {
        for (from, to) in &self.clips {
            if let Some(rest) = path.strip_prefix(to.as_str()) {
                return Some(format!("{from}{rest}"));
            }
        }
        for root in SOUND_ROOTS {
            let Some(name) = path.strip_prefix(root) else {
                continue;
            };
            for (from, to) in &self.sounds {
                // A path goes on to its extension too.
                if let Some(rest) = name.strip_prefix(to.as_str())
                    && (continues(rest) || rest.starts_with('.'))
                {
                    return Some(format!("{root}{from}{rest}"));
                }
            }
        }
        None
    }

    /// A clip's new name (`resources/...`), if a rule moves it and the donor
    /// (`has`) has the clip.
    fn clip(&self, name: &str, has: &dyn Fn(&str) -> bool) -> Option<String> {
        let path = name.strip_prefix("resources/")?;
        self.clips.iter().find_map(|(from, to)| {
            let rest = path.strip_prefix(from.as_str())?;
            has(&format!("{path}.b-spline")).then(|| format!("resources/{to}{rest}"))
        })
    }

    /// A sound's new name, if a rule moves it and the donor has the sound.
    fn sound(&self, kind: i32, name: &str, has: &dyn Fn(&str) -> bool) -> Option<String> {
        self.sounds.iter().find_map(|(from, to)| {
            let rest = name.strip_prefix(from.as_str())?;
            (continues(rest) && has(&sound_path(kind, name))).then(|| format!("{to}{rest}"))
        })
    }

    /// Renames, in place, the clips and sounds `v` names: clip strings
    /// (`resources/animations/...`), sound lists' `[type, name]` pairs and
    /// sound items' `filename` (type `resource_type`, 256 when absent).
    pub fn apply(&self, v: &mut V, has: &dyn Fn(&str) -> bool) {
        match v {
            V::String(s) => {
                if let Some(new) = self.clip(s, has) {
                    *s = new;
                }
            }
            V::Indexed(items) => {
                if let [V::Integer(kind), V::String(name)] = items.as_mut_slice()
                    && (256..=258).contains(kind)
                {
                    if let Some(new) = self.sound(*kind, name, has) {
                        *name = new;
                    }
                    return;
                }
                for item in items {
                    self.apply(item, has);
                }
            }
            V::Named(fields) => {
                let kind = fields
                    .iter()
                    .find(|f| f.name == "resource_type")
                    .and_then(|f| match f.value {
                        V::Integer(k) => Some(k),
                        _ => None,
                    })
                    .unwrap_or(256);
                for f in fields.iter_mut() {
                    if f.name == "filename"
                        && let V::String(name) = &mut f.value
                    {
                        if let Some(new) = self.sound(kind, name, has) {
                            *name = new;
                        }
                        continue;
                    }
                    self.apply(&mut f.value, has);
                }
            }
            _ => {}
        }
    }
}
