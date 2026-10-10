// SPDX-License-Identifier: GPL-3.0-or-later
//! `shaders.xr`: the shader library a compiled level's shader table indexes.
//!
//! A level surface names a shader as a *string* (`def_shaders\def_aref`). That
//! string is only a key: what it resolves to is a **blender** — one of X-Ray's
//! ~86 C++ classes, selected by an eight-character class id, which emits render
//! passes when the level loads.
//!
//! The class id is the authoritative statement of what a surface is. `LM_AREF`
//! is alpha-tested because the level compiler recorded that class, not because
//! a name looked like it. Classification built on names is pattern-matching on
//! text an artist typed; classification built on class ids reads what the data
//! says.
//!
//! Only the fixed `CBlender_DESC` header and the common `IBlender` properties
//! are decoded. Everything after those is a per-class layout, and guessing at
//! 86 of them would invent exactly the kind of unverified detail this converter
//! is built to avoid.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};

/// The chunk of `shaders.xr` holding blenders. Chunk 1 holds constant/matrix
/// tables, which the port does not use.
pub const SHADERS_CHUNK_BLENDERS: u32 = 2;

/// `sizeof(CBlender_DESC)` under `#pragma pack(push, 4)`:
/// `u64` + `char[128]` + `char[32]` + `u32` + `u16`, rounded up to 176.
const DESC_SIZE: usize = 176;

/// `xrProperties` ids.
const PID_MARKER: u32 = 0;
const PID_MATRIX: u32 = 1;
const PID_TEXTURE: u32 = 3;
const PID_INTEGER: u32 = 4;
const PID_BOOL: u32 = 6;

/// An eight-character class id, as `make_clsid` packs it: the first character
/// occupies the most significant byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClassId(pub u64);

impl ClassId {
    /// The tag as written in `Blender_CLSID.h`, trailing spaces trimmed.
    pub fn tag(self) -> String {
        let bytes: [u8; 8] =
            std::array::from_fn(|index| ((self.0 >> (56 - index * 8)) & 0xff) as u8);
        String::from_utf8_lossy(&bytes).trim_end().to_owned()
    }
}

impl std::fmt::Display for ClassId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.tag())
    }
}

/// The properties every blender carries, whatever its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommonProperties {
    /// Render order within a pass.
    pub priority: i32,
    /// Whether the surface needs strictly sorted blending.
    pub strict_sorting: bool,
    /// Base texture name, when the blender fixes one.
    pub texture: String,
    /// Texture-coordinate transform name, when one is bound.
    pub transform: String,
}

/// One entry of the shader library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blender {
    /// The name a level shader table refers to.
    pub name: String,
    pub class_id: ClassId,
    pub version: u16,
    /// `None` when the property block could not be decoded; the class id, which
    /// is what classification needs, is still valid.
    pub common: Option<CommonProperties>,
    /// The four tiled detail textures a bump-mapped ground blender (`BmmDold`,
    /// `LmBmmD`) blends by its base texture's `_mask`, in mask channel order
    /// R, G, B, A. `None` for every other class, and for blenders older than
    /// version 3, which carry no per-channel names.
    pub detail_layers: Option<[String; 4]>,
}

/// Decode `shaders.xr` into a name-keyed library.
pub fn parse_library(bytes: &[u8]) -> Result<BTreeMap<String, Blender>> {
    let payload = Chunks::new(bytes, "shaders.xr").require(SHADERS_CHUNK_BLENDERS)?;
    let mut library = BTreeMap::new();
    for (expected, chunk) in Chunks::new(payload, "shaders.xr blenders")
        .iter()
        .enumerate()
    {
        let chunk = chunk?;
        if chunk.id != u32::try_from(expected)? {
            bail!(
                "shaders.xr blender {expected} is stored with identifier {}; the container is not \
                 a dense array",
                chunk.id
            );
        }
        let blender = parse_blender(chunk.payload, expected)?;
        if let Some(existing) = library.insert(blender.name.clone(), blender) {
            bail!("shaders.xr defines {:?} twice", existing.name);
        }
    }
    if library.is_empty() {
        bail!("shaders.xr contains no blenders");
    }
    Ok(library)
}

fn parse_blender(payload: &[u8], index: usize) -> Result<Blender> {
    let desc = payload
        .get(..DESC_SIZE)
        .with_context(|| format!("shaders.xr blender {index} is shorter than its description"))?;

    let class_id = ClassId(u64::from_le_bytes(desc[0..8].try_into().unwrap()));
    let name = fixed_string(&desc[8..136])
        .with_context(|| format!("reading the name of shaders.xr blender {index}"))?;
    let version = u16::from_le_bytes(desc[172..174].try_into().unwrap());
    if name.is_empty() {
        bail!("shaders.xr blender {index} has an empty name");
    }

    // The property block is best-effort: a class the port has never seen still
    // yields a usable class id.
    let common = parse_common_properties(&payload[DESC_SIZE..]).ok();
    let detail_layers = if matches!(class_id.tag().as_str(), "BmmDold" | "LmBmmD") && version >= 3 {
        parse_detail_layers(&payload[DESC_SIZE..]).ok()
    } else {
        None
    };

    Ok(Blender {
        name,
        class_id,
        version,
        common,
        detail_layers,
    })
}

/// Read the six properties `IBlender::Load` reads, in its order.
fn parse_common_properties(bytes: &[u8]) -> Result<CommonProperties> {
    let mut cursor = Cursor::new(bytes, "shaders.xr blender properties");
    read_property(&mut cursor, PID_MARKER, 0)?;
    let priority = read_property(&mut cursor, PID_INTEGER, 12)?;
    let strict_sorting = read_property(&mut cursor, PID_BOOL, 4)?;
    read_property(&mut cursor, PID_MARKER, 0)?;
    let texture = read_property(&mut cursor, PID_TEXTURE, 64)?;
    let transform = read_property(&mut cursor, PID_MATRIX, 64)?;

    Ok(CommonProperties {
        // `xrP_Integer` is `{ value, min, max }`; only the value is meaningful.
        priority: i32::from_le_bytes(priority[0..4].try_into().unwrap()),
        strict_sorting: u32::from_le_bytes(strict_sorting[0..4].try_into().unwrap()) != 0,
        texture: fixed_string(texture)?,
        transform: fixed_string(transform)?,
    })
}

/// `CBlender_BmmD::Load` at version 3: the common block, a marker, the
/// legacy detail map and its transform, then the R2 detail texture per mask
/// channel.
fn parse_detail_layers(bytes: &[u8]) -> Result<[String; 4]> {
    let mut cursor = Cursor::new(bytes, "shaders.xr BmmD properties");
    read_property(&mut cursor, PID_MARKER, 0)?;
    read_property(&mut cursor, PID_INTEGER, 12)?;
    read_property(&mut cursor, PID_BOOL, 4)?;
    read_property(&mut cursor, PID_MARKER, 0)?;
    read_property(&mut cursor, PID_TEXTURE, 64)?;
    read_property(&mut cursor, PID_MATRIX, 64)?;
    read_property(&mut cursor, PID_MARKER, 0)?;
    read_property(&mut cursor, PID_TEXTURE, 64)?;
    read_property(&mut cursor, PID_MATRIX, 64)?;
    let mut layers: [String; 4] = Default::default();
    for layer in &mut layers {
        *layer = fixed_string(read_property(&mut cursor, PID_TEXTURE, 64)?)?;
    }
    Ok(layers)
}

/// One property: `u32 type`, a NUL-terminated property name, then fixed data.
fn read_property<'a>(cursor: &mut Cursor<'a>, expected: u32, size: usize) -> Result<&'a [u8]> {
    let kind = cursor.u32()?;
    if kind != expected {
        bail!("expected property type {expected}, found {kind}");
    }
    cursor.stringz()?;
    cursor.take(size)
}

/// Decode a fixed-width, NUL-padded field.
fn fixed_string(bytes: &[u8]) -> Result<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    Ok(std::str::from_utf8(&bytes[..end])
        .context("blender string is not ASCII")?
        .to_owned())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    /// Pack an eight-character tag the way `make_clsid` does.
    pub fn clsid(tag: &str) -> u64 {
        let mut padded = [b' '; 8];
        for (slot, byte) in padded.iter_mut().zip(tag.bytes()) {
            *slot = byte;
        }
        padded
            .iter()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte))
    }

    fn property(kind: u32, name: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_le_bytes().to_vec();
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(data);
        bytes
    }

    pub fn blender(index: u32, tag: &str, name: &str, priority: i32, sorting: bool) -> Vec<u8> {
        let mut payload = vec![0_u8; DESC_SIZE];
        payload[0..8].copy_from_slice(&clsid(tag).to_le_bytes());
        payload[8..8 + name.len()].copy_from_slice(name.as_bytes());
        payload[172..174].copy_from_slice(&3_u16.to_le_bytes());

        let mut integer = priority.to_le_bytes().to_vec();
        integer.extend_from_slice(&0_i32.to_le_bytes());
        integer.extend_from_slice(&255_i32.to_le_bytes());

        payload.extend(property(PID_MARKER, "marker", &[]));
        payload.extend(property(PID_INTEGER, "Priority", &integer));
        payload.extend(property(
            PID_BOOL,
            "Strict sorting",
            &u32::from(sorting).to_le_bytes(),
        ));
        payload.extend(property(PID_MARKER, "marker", &[]));
        let mut texture = vec![0_u8; 64];
        texture[..8].copy_from_slice(b"$base_te");
        payload.extend(property(PID_TEXTURE, "Base Texture", &texture));
        payload.extend(property(PID_MATRIX, "Base Matrix", &vec![0_u8; 64]));

        chunk(index, &payload)
    }

    pub fn bmmd(index: u32, name: &str, layers: [&str; 4]) -> Vec<u8> {
        let mut bytes = blender(index, "BmmDold", name, 1, false);
        // Drop the chunk header to extend the payload, then rebuild it.
        let mut payload = bytes.split_off(8);
        payload.extend(property(PID_MARKER, "Detail map", &[]));
        payload.extend(property(PID_TEXTURE, "Name", &vec![0_u8; 64]));
        payload.extend(property(PID_MATRIX, "Transform", &vec![0_u8; 64]));
        for (channel, layer) in ["R2-R", "R2-G", "R2-B", "R2-A"].iter().zip(layers) {
            let mut texture = vec![0_u8; 64];
            texture[..layer.len()].copy_from_slice(layer.as_bytes());
            payload.extend(property(PID_TEXTURE, channel, &texture));
        }
        chunk(index, &payload)
    }

    pub fn library(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut blenders = Vec::new();
        for (index, (tag, name)) in entries.iter().enumerate() {
            blenders.extend(blender(index as u32, tag, name, 0, false));
        }
        chunk(SHADERS_CHUNK_BLENDERS, &blenders)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn unpacks_the_class_id_tag() {
        assert_eq!(ClassId(clsid("LM_AREF")).tag(), "LM_AREF");
        assert_eq!(ClassId(clsid("LM")).tag(), "LM");
        assert_eq!(ClassId(clsid("MODELEbB")).tag(), "MODELEbB");
    }

    #[test]
    fn reads_names_and_class_ids() {
        let bytes = library(&[
            ("LM", "default"),
            ("LM_AREF", "def_shaders\\def_aref"),
            ("D_TREE", "flora\\leaf_wave"),
        ]);
        let library = parse_library(&bytes).unwrap();
        assert_eq!(library.len(), 3);
        assert_eq!(library["default"].class_id.tag(), "LM");
        assert_eq!(library["def_shaders\\def_aref"].class_id.tag(), "LM_AREF");
        assert_eq!(library["flora\\leaf_wave"].version, 3);
    }

    #[test]
    fn reads_the_bump_mapped_ground_detail_layers() {
        let bytes = chunk(
            SHADERS_CHUNK_BLENDERS,
            &bmmd(
                0,
                "levels\\escape_grass",
                [
                    "detail\\detail_grnd_grass",
                    "detail\\detail_grnd_asphalt",
                    "detail\\detail_grnd_earth",
                    "detail\\detail_grnd_yantar",
                ],
            ),
        );
        let library = parse_library(&bytes).unwrap();
        let layers = library["levels\\escape_grass"]
            .detail_layers
            .as_ref()
            .unwrap();
        assert_eq!(layers[0], "detail\\detail_grnd_grass");
        assert_eq!(layers[3], "detail\\detail_grnd_yantar");
        assert!(library["levels\\escape_grass"].common.is_some());
    }

    #[test]
    fn reads_the_common_properties() {
        let bytes = chunk(
            SHADERS_CHUNK_BLENDERS,
            &blender(0, "LM", "default", 7, true),
        );
        let library = parse_library(&bytes).unwrap();
        let common = library["default"].common.as_ref().unwrap();
        assert_eq!(common.priority, 7);
        assert!(common.strict_sorting);
        assert_eq!(common.texture, "$base_te");
    }

    #[test]
    fn a_class_with_an_unreadable_property_block_still_yields_its_class_id() {
        // Truncate everything after the description.
        let mut payload = vec![0_u8; DESC_SIZE];
        payload[0..8].copy_from_slice(&clsid("LmBmmD").to_le_bytes());
        payload[8..15].copy_from_slice(b"unknown");
        let bytes = chunk(SHADERS_CHUNK_BLENDERS, &chunk(0, &payload));

        let library = parse_library(&bytes).unwrap();
        assert_eq!(library["unknown"].class_id.tag(), "LmBmmD");
        assert!(library["unknown"].common.is_none());
    }

    #[test]
    fn rejects_a_duplicate_name() {
        let bytes = library(&[("LM", "default"), ("V", "default")]);
        let error = parse_library(&bytes).unwrap_err().to_string();
        assert!(error.contains("twice"), "{error}");
    }

    #[test]
    fn rejects_an_empty_name() {
        let bytes = library(&[("LM", "")]);
        assert!(parse_library(&bytes).is_err());
    }

    #[test]
    fn rejects_a_sparse_container() {
        let mut blenders = blender(0, "LM", "a", 0, false);
        blenders.extend(blender(2, "LM", "b", 0, false));
        let bytes = chunk(SHADERS_CHUNK_BLENDERS, &blenders);
        let error = parse_library(&bytes).unwrap_err().to_string();
        assert!(error.contains("dense array"), "{error}");
    }
}
