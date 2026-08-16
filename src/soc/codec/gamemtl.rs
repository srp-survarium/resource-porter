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

/// Decode the library into an id-to-name map.
pub fn parse_library(bytes: &[u8]) -> Result<BTreeMap<u16, String>> {
    let chunks = Chunks::new(bytes, "gamemtl.xr");
    let version = chunks.require(GAMEMTLS_CHUNK_VERSION)?;
    let mut cursor = Cursor::new(version, "gamemtl.xr version");
    let _version = cursor.u16()?;

    let materials = chunks.require(GAMEMTLS_CHUNK_MTLS)?;
    let mut library = BTreeMap::new();
    for entry in Chunks::new(materials, "gamemtl.xr materials").iter() {
        let entry = entry?;
        let main = Chunks::new(entry.payload, "gamemtl.xr material").require(GAMEMTL_CHUNK_MAIN)?;
        let mut cursor = Cursor::new(main, "gamemtl.xr material header");
        let id = cursor.u32()?;
        let name = cursor.stringz()?;
        let id = u16::try_from(id)
            .with_context(|| format!("game material id {id} does not fit a cform face"))?;
        if name.is_empty() {
            bail!("game material {id} has an empty name");
        }
        if let Some(existing) = library.insert(id, name.clone()) {
            bail!("game material id {id} is defined twice: {existing:?} and {name:?}");
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
        let mut main = id.to_le_bytes().to_vec();
        main.extend_from_slice(name.as_bytes());
        main.push(0);
        chunk(index, &chunk(GAMEMTL_CHUNK_MAIN, &main))
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
        assert_eq!(library[&0], "default");
        assert_eq!(library[&3], "materials\\concrete");
        assert_eq!(library[&39], "materials\\tree_trunk");
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
