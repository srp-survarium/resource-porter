// SPDX-License-Identifier: GPL-3.0-or-later
//! Weapons of later Survarium builds for v0.26g, as data: a port manifest
//! (`ports/weapons-*.json`, [`manifest`]) names the guns, their ids, cells,
//! templates and balance; [`build`] makes a bundle of them (`vostok-port
//! port-weapons`), which `pack from-bundle` turns into a content pack;
//! [`compare`] checks a bundle against files players already have
//! (`vostok-port compare-weapons`: patch 19's 0.29a3 weapons). How to add a
//! gun: `docs/weapons-026.md`.
//!
//! The parts:
//!
//! * [`donor`]: a build's files in 0.26g's layouts (the [`donor::Donor`]
//!   trait; [`donor::ArchiveDonor`] for 0.29a3 and 0.31a2) and what its
//!   configs mean ([`donor::Rules`]);
//! * [`clips`]: the clips' event headers, 12 bytes in the later builds and
//!   16 in 0.26g;
//! * [`config`]: an item config on a native template's keys;
//! * [`build`]: carrying every file a gun names, materials and sound
//!   options on native templates, the firing sounds' reverb layers, the
//!   dictionary rows, icons;
//! * [`modules`]: attachable modules (scopes, flash hiders, muzzle brakes,
//!   grips), placed at the gun's locators.

use crate::{ConfigValue as V, NamedValue};

mod build;
pub mod clips;
pub mod compare;
pub mod config;
pub mod donor;
pub mod icons;
pub mod manifest;
pub mod modules;

pub use build::{BuildReport, build, conform_material, environment_layer};
pub use config::conform;
pub use donor::strip_mesh_statistics;

pub(crate) fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}

pub(crate) fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}

pub(crate) fn string_of(v: Option<&V>) -> Option<&str> {
    match v {
        Some(V::String(s)) => Some(s),
        _ => None,
    }
}

/// The value at a dotted path (`a.b.0.c`).
pub(crate) fn get<'v>(root: &'v V, path: &str) -> Option<&'v V> {
    path.split('.').try_fold(root, |v, part| match v {
        V::Named(_) => v.named(part),
        V::Indexed(items) => items.get(part.parse::<usize>().ok()?),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
