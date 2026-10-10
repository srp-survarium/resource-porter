// SPDX-License-Identifier: GPL-3.0-or-later
//! `gamemtl.xr`: the X-Ray physical-material library.
//!
//! Only the identifier and name of each material are decoded. Friction,
//! damping, flotation and the material-pair table describe X-Ray's own physics
//! solver and have no v0.100b counterpart, so reading them would imply a
//! fidelity the converter does not have.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};

pub const GAMEMTLS_CHUNK_VERSION: u32 = 0x1000;
pub const GAMEMTLS_CHUNK_MTLS: u32 = 0x1002;
pub const GAMEMTL_CHUNK_MAIN: u32 = 0x1000;
pub const GAMEMTL_CHUNK_FLAGS: u32 = 0x1001;

/// Behaviour flags a physical material carries.
///
/// These describe what the *world* does, independently of how it sounds or
/// looks: whether an actor can climb it, walk through it, or be hurt by it.
/// The port translates material names and drops all of this, so the flags are
/// decoded to make that loss measurable rather than invisible.
pub mod flags {
    pub const BREAKABLE: u32 = 1 << 0;
    pub const BOUNCEABLE: u32 = 1 << 2;
    pub const SKIDMARK: u32 = 1 << 3;
    pub const BLOODMARK: u32 = 1 << 4;
    /// The actor can climb this surface: ladders, and climbable fences.
    pub const CLIMABLE: u32 = 1 << 5;
    /// The actor walks through it: foliage, thin bushes.
    pub const PASSABLE: u32 = 1 << 7;
    pub const DYNAMIC: u32 = 1 << 8;
    pub const LIQUID: u32 = 1 << 9;
    pub const SUPPRESS_SHADOWS: u32 = 1 << 10;
    pub const SUPPRESS_WALLMARKS: u32 = 1 << 11;
    pub const ACTOR_OBSTACLE: u32 = 1 << 12;
    pub const NO_RICOCHET: u32 = 1 << 13;
    pub const INJURIOUS: u32 = 1 << 28;
    pub const SHOOTABLE: u32 = 1 << 29;
}

/// One physical material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameMaterial {
    pub name: String,
    pub flags: u32,
}

impl GameMaterial {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Whether a surface of this material stops the player.
    ///
    /// `level.cform` is a collision mesh for *everything*, including surfaces
    /// X-Ray never blocks movement with: water, bushes, and the invisible
    /// volumes the level designers used as triggers. Converting every triangle
    /// makes all of them solid — the player walks on the surface of a pool, and
    /// Cordon's 114,080 bush triangles become walls.
    ///
    /// [`flags::PASSABLE`] is X-Ray's own answer, with one exception:
    /// [`flags::ACTOR_OBSTACLE`] marks the surfaces that are passable to the AI
    /// but not to the player, which is how `materials\fake` implements an
    /// invisible wall. Those must stay solid.
    pub fn blocks_movement(&self) -> bool {
        !self.has(flags::PASSABLE) || self.has(flags::ACTOR_OBSTACLE)
    }

    pub fn is_climbable(&self) -> bool {
        self.has(flags::CLIMABLE)
    }

    /// Names of every flag set, for reporting.
    pub fn flag_names(&self) -> Vec<&'static str> {
        [
            (flags::BREAKABLE, "breakable"),
            (flags::BOUNCEABLE, "bounceable"),
            (flags::SKIDMARK, "skidmark"),
            (flags::BLOODMARK, "bloodmark"),
            (flags::CLIMABLE, "climable"),
            (flags::PASSABLE, "passable"),
            (flags::DYNAMIC, "dynamic"),
            (flags::LIQUID, "liquid"),
            (flags::SUPPRESS_SHADOWS, "suppress_shadows"),
            (flags::SUPPRESS_WALLMARKS, "suppress_wallmarks"),
            (flags::ACTOR_OBSTACLE, "actor_obstacle"),
            (flags::NO_RICOCHET, "no_ricochet"),
            (flags::INJURIOUS, "injurious"),
            (flags::SHOOTABLE, "shootable"),
        ]
        .into_iter()
        .filter(|(flag, _)| self.flags & flag != 0)
        .map(|(_, name)| name)
        .collect()
    }
}

/// Decode the library into an id-to-material map.
pub fn parse_library(bytes: &[u8]) -> Result<BTreeMap<u16, GameMaterial>> {
    let chunks = Chunks::new(bytes, "gamemtl.xr");
    let version = chunks.require(GAMEMTLS_CHUNK_VERSION)?;
    let mut cursor = Cursor::new(version, "gamemtl.xr version");
    let _version = cursor.u16()?;

    let materials = chunks.require(GAMEMTLS_CHUNK_MTLS)?;
    let mut library = BTreeMap::new();
    for entry in Chunks::new(materials, "gamemtl.xr materials").iter() {
        let entry = entry?;
        let chunks = Chunks::new(entry.payload, "gamemtl.xr material");
        let main = chunks.require(GAMEMTL_CHUNK_MAIN)?;
        let mut cursor = Cursor::new(main, "gamemtl.xr material header");
        let id = cursor.u32()?;
        let name = cursor.stringz()?;
        let id = u16::try_from(id)
            .with_context(|| format!("game material id {id} does not fit a cform face"))?;
        if name.is_empty() {
            bail!("game material {id} has an empty name");
        }
        let flags = match chunks.find(GAMEMTL_CHUNK_FLAGS)? {
            Some(payload) => Cursor::new(payload, "gamemtl.xr material flags").u32()?,
            None => bail!("game material {name:?} has no flags chunk"),
        };
        let material = GameMaterial {
            name: name.clone(),
            flags,
        };
        if let Some(existing) = library.insert(id, material) {
            bail!(
                "game material id {id} is defined twice: {:?} and {name:?}",
                existing.name
            );
        }
    }
    if library.is_empty() {
        bail!("gamemtl.xr contains no materials");
    }
    Ok(library)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    pub fn material(index: u32, id: u32, name: &str) -> Vec<u8> {
        material_with_flags(index, id, name, 0)
    }

    pub fn material_with_flags(index: u32, id: u32, name: &str, flags: u32) -> Vec<u8> {
        let mut main = id.to_le_bytes().to_vec();
        main.extend_from_slice(name.as_bytes());
        main.push(0);
        let mut payload = chunk(GAMEMTL_CHUNK_MAIN, &main);
        payload.extend(chunk(GAMEMTL_CHUNK_FLAGS, &flags.to_le_bytes()));
        chunk(index, &payload)
    }

    /// A three-entry library matching the identifiers the Cordon port used.
    pub fn library() -> Vec<u8> {
        let mut materials = material(0, 0, "default");
        materials.extend(material(1, 3, "materials\\concrete"));
        materials.extend(material(2, 39, "materials\\tree_trunk"));
        let mut bytes = chunk(GAMEMTLS_CHUNK_VERSION, &1_u16.to_le_bytes());
        bytes.extend(chunk(GAMEMTLS_CHUNK_MTLS, &materials));
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn reads_identifiers_and_names() {
        let library = parse_library(&library()).unwrap();
        assert_eq!(library[&0].name, "default");
        assert_eq!(library[&3].name, "materials\\concrete");
        assert_eq!(library[&39].name, "materials\\tree_trunk");
    }

    #[test]
    fn rejects_a_duplicate_identifier() {
        let mut materials = material(0, 3, "materials\\concrete");
        materials.extend(material(1, 3, "materials\\asphalt"));
        let mut bytes = chunk(GAMEMTLS_CHUNK_VERSION, &1_u16.to_le_bytes());
        bytes.extend(chunk(GAMEMTLS_CHUNK_MTLS, &materials));
        let error = parse_library(&bytes).unwrap_err().to_string();
        assert!(error.contains("defined twice"), "{error}");
    }

    #[test]
    fn rejects_an_empty_library() {
        let mut bytes = chunk(GAMEMTLS_CHUNK_VERSION, &1_u16.to_le_bytes());
        bytes.extend(chunk(GAMEMTLS_CHUNK_MTLS, &[]));
        assert!(parse_library(&bytes).is_err());
    }
}
