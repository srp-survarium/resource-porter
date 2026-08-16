//! `level` and `level.geom` readers for X-Ray 1.x compiled levels.
//!
//! A compiled level is split across two containers:
//!
//! * `level` holds the build header, the shader/texture table, and one OGF
//!   visual per rendered piece of the world;
//! * `level.geom` holds the shared vertex and index buffers those visuals
//!   index into.
//!
//! Nothing here interprets a visual — see [`super::ogf`] — and nothing merges
//! surfaces. This layer only proves that the containers are internally
//! consistent.

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};
use super::vertex::{SocVertexLayout, parse_declaration};

/// `fsL_Chunks` from `xray-16/src/Common/LevelStructure.hpp`.
pub const FSL_HEADER: u32 = 1;
pub const FSL_SHADERS: u32 = 2;
pub const FSL_VISUALS: u32 = 3;
pub const FSL_VB: u32 = 9;
pub const FSL_IB: u32 = 10;
pub const FSL_SWIS: u32 = 11;

/// `XRCL_PRODUCTION_VERSION`: the level format shipped by Shadow of Chernobyl.
pub const XRLC_VERSION_SOC: u16 = 14;

/// `hdrLEVEL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelHeader {
    pub xrlc_version: u16,
    pub quality: u16,
}

/// One entry of the level shader table.
///
/// X-Ray stores render state and texture binding as a single
/// `"<shader>/<texture list>"` string. The texture list is comma separated:
/// the first entry is the base texture, and the remaining entries are the
/// lightmaps baked by `xrLC`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderEntry {
    pub shader: String,
    pub textures: Vec<String>,
}

impl ShaderEntry {
    /// The base (diffuse) texture, if this entry binds one.
    pub fn base_texture(&self) -> Option<&str> {
        self.textures.first().map(String::as_str)
    }

    /// The lightmaps `xrLC` baked for this surface. They have no v0.100b
    /// analogue and are reported rather than converted.
    pub fn lightmaps(&self) -> &[String] {
        self.textures.get(1..).unwrap_or_default()
    }
}

/// The decoded shader table. Index 0 is X-Ray's reserved empty entry, so the
/// table is addressed by the `shader_id` stored in each OGF header.
#[derive(Clone, Debug, Default)]
pub struct ShaderTable {
    entries: Vec<Option<ShaderEntry>>,
}

impl ShaderTable {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: u16) -> Result<&ShaderEntry> {
        self.entries
            .get(usize::from(id))
            .with_context(|| format!("shader id {id} is outside the level shader table"))?
            .as_ref()
            .with_context(|| format!("shader id {id} refers to a reserved empty table entry"))
    }
}

/// A shared vertex buffer plus the declaration that gives it meaning.
#[derive(Clone, Debug)]
pub struct VertexBuffer {
    pub layout: SocVertexLayout,
    pub count: usize,
    pub data: Vec<u8>,
}

impl VertexBuffer {
    /// Borrow the record for one vertex.
    pub fn record(&self, index: usize) -> Result<&[u8]> {
        let start = index
            .checked_mul(self.layout.stride)
            .context("vertex offset overflow")?;
        self.data
            .get(start..start + self.layout.stride)
            .with_context(|| format!("vertex {index} is outside a buffer of {} ", self.count))
    }
}

/// The shared geometry of one level.
#[derive(Clone, Debug, Default)]
pub struct GeometryBuffers {
    pub vertex_buffers: Vec<VertexBuffer>,
    pub index_buffers: Vec<Vec<u16>>,
    /// `fsL_SWIS`, one full-detail sliding window per record.
    ///
    /// X-Ray's own comment on the chunk is "collapse info, usually for trees":
    /// a progressive mesh keeps every detail level in one index range, and a
    /// tree visual names the record describing its own by `OGF_SWICONTAINER`.
    /// Only window 0 is kept, because the port has no LOD switching and window
    /// 0 is what `lod_id = iFloor((1 - LOD) * (count - 1) + 0.5)` selects at
    /// full detail.
    pub sliding_windows: Vec<SlideWindow>,
}

/// One `FSlideWindow`: `u32 offset; u16 num_tris; u16 num_verts;`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SlideWindow {
    pub offset: u32,
    pub triangles: u16,
    pub vertices: u16,
}

/// Parse the `fsL_HEADER` chunk.
pub fn parse_header(level: &[u8]) -> Result<LevelHeader> {
    let payload = Chunks::new(level, "level").require(FSL_HEADER)?;
    let mut cursor = Cursor::new(payload, "level header");
    let header = LevelHeader {
        xrlc_version: cursor.u16()?,
        quality: cursor.u16()?,
    };
    if header.xrlc_version != XRLC_VERSION_SOC {
        bail!(
            "level was built by xrLC version {}, but only version {XRLC_VERSION_SOC} \
             (Shadow of Chernobyl) is supported",
            header.xrlc_version
        );
    }
    Ok(header)
}

/// Parse the `fsL_SHADERS` chunk.
pub fn parse_shaders(level: &[u8]) -> Result<ShaderTable> {
    let payload = Chunks::new(level, "level").require(FSL_SHADERS)?;
    let mut cursor = Cursor::new(payload, "level shaders");
    let count = usize::try_from(cursor.u32()?)?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let raw = cursor
            .stringz()
            .with_context(|| format!("reading shader table entry {index}"))?;
        if raw.is_empty() {
            entries.push(None);
            continue;
        }
        let (shader, textures) = raw.split_once('/').with_context(|| {
            format!("shader table entry {index} ({raw:?}) has no '<shader>/<textures>' separator")
        })?;
        if shader.is_empty() {
            bail!("shader table entry {index} ({raw:?}) has an empty shader name");
        }
        entries.push(Some(ShaderEntry {
            shader: shader.to_owned(),
            textures: textures
                .split(',')
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        }));
    }
    if !cursor.is_empty() {
        bail!(
            "level shader table has {} trailing bytes after {count} entries",
            cursor.remaining()
        );
    }
    Ok(ShaderTable { entries })
}

/// Borrow the payload of every indexed child of `fsL_VISUALS`, in file order.
pub fn visual_payloads(level: &[u8]) -> Result<Vec<&[u8]>> {
    let payload = Chunks::new(level, "level").require(FSL_VISUALS)?;
    let mut visuals = Vec::new();
    for (expected, chunk) in Chunks::new(payload, "level visuals").iter().enumerate() {
        let chunk = chunk?;
        // The visual container addresses its children by array index, so a gap
        // means the file is not the one the shader table describes.
        if chunk.id != u32::try_from(expected)? {
            bail!(
                "level visual {expected} is stored with identifier {}; the visual container is not \
                 a dense array",
                chunk.id
            );
        }
        visuals.push(chunk.payload);
    }
    Ok(visuals)
}

/// Parse `level.geom` into its shared vertex and index buffers.
pub fn parse_geometry(geom: &[u8]) -> Result<GeometryBuffers> {
    let chunks = Chunks::new(geom, "level.geom");
    let mut buffers = GeometryBuffers::default();

    // `fsL_SWIS`: one record per progressive mesh, each a list of detail
    // levels. Only the full-detail window of each is kept.
    if let Some(payload) = chunks.find(FSL_SWIS)? {
        let mut cursor = Cursor::new(payload, "level.geom sliding windows");
        let items = usize::try_from(cursor.u32()?)?;
        buffers.sliding_windows.reserve(items);
        for index in 0..items {
            for _ in 0..4 {
                cursor.u32()?; // reserved[4]
            }
            let count = usize::try_from(cursor.u32()?)?;
            if count == 0 {
                bail!("sliding window record {index} declares no detail levels");
            }
            let window = SlideWindow {
                offset: cursor.u32()?,
                triangles: cursor.u16()?,
                vertices: cursor.u16()?,
            };
            for _ in 1..count {
                cursor.u32()?;
                cursor.u16()?;
                cursor.u16()?;
            }
            buffers.sliding_windows.push(window);
        }
    }

    let vertex_payload = chunks.require(FSL_VB)?;
    let mut cursor = Cursor::new(vertex_payload, "level.geom vertex buffers");
    let count = usize::try_from(cursor.u32()?)?;
    for index in 0..count {
        let elements = parse_declaration(&mut cursor)
            .with_context(|| format!("reading vertex declaration {index}"))?;
        let layout = SocVertexLayout::resolve(&elements)
            .with_context(|| format!("resolving vertex declaration {index}"))?;
        let vertex_count = usize::try_from(cursor.u32()?)?;
        let size = vertex_count
            .checked_mul(layout.stride)
            .context("vertex buffer size overflow")?;
        let data = cursor
            .take(size)
            .with_context(|| format!("reading vertex buffer {index}"))?
            .to_vec();
        buffers.vertex_buffers.push(VertexBuffer {
            layout,
            count: vertex_count,
            data,
        });
    }
    if !cursor.is_empty() {
        bail!(
            "level.geom has {} trailing bytes after {count} vertex buffers",
            cursor.remaining()
        );
    }

    let index_payload = chunks.require(FSL_IB)?;
    let mut cursor = Cursor::new(index_payload, "level.geom index buffers");
    let count = usize::try_from(cursor.u32()?)?;
    for index in 0..count {
        let index_count = usize::try_from(cursor.u32()?)?;
        let size = index_count
            .checked_mul(2)
            .context("index buffer size overflow")?;
        let raw = cursor
            .take(size)
            .with_context(|| format!("reading index buffer {index}"))?;
        buffers.index_buffers.push(
            raw.chunks_exact(2)
                .map(|pair| u16::from_le_bytes(pair.try_into().unwrap()))
                .collect(),
        );
    }
    if !cursor.is_empty() {
        bail!(
            "level.geom has {} trailing bytes after {count} index buffers",
            cursor.remaining()
        );
    }
    Ok(buffers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn shader_payload(entries: &[&str]) -> Vec<u8> {
        let mut payload = (entries.len() as u32).to_le_bytes().to_vec();
        for entry in entries {
            payload.extend_from_slice(entry.as_bytes());
            payload.push(0);
        }
        payload
    }

    #[test]
    fn accepts_the_shadow_of_chernobyl_build_version() {
        let level = chunk(FSL_HEADER, &[14, 0, 1, 0]);
        assert_eq!(
            parse_header(&level).unwrap(),
            LevelHeader {
                xrlc_version: 14,
                quality: 1,
            }
        );
    }

    #[test]
    fn rejects_a_foreign_build_version() {
        let level = chunk(FSL_HEADER, &[13, 0, 1, 0]);
        let error = parse_header(&level).unwrap_err().to_string();
        assert!(error.contains("xrLC version 13"), "{error}");
    }

    #[test]
    fn splits_shader_and_texture_list() {
        let level = chunk(
            FSL_SHADERS,
            &shader_payload(&["", "default/levels\\escape_grass,lm_l01#0,lm_l01#1"]),
        );
        let table = parse_shaders(&level).unwrap();
        assert_eq!(table.len(), 2);
        assert!(table.get(0).is_err());
        let entry = table.get(1).unwrap();
        assert_eq!(entry.shader, "default");
        assert_eq!(entry.base_texture(), Some("levels\\escape_grass"));
        assert_eq!(entry.lightmaps(), ["lm_l01#0", "lm_l01#1"]);
    }

    #[test]
    fn rejects_a_shader_entry_without_a_texture_separator() {
        let level = chunk(FSL_SHADERS, &shader_payload(&["", "default"]));
        let error = parse_shaders(&level).unwrap_err().to_string();
        assert!(error.contains("separator"), "{error}");
    }

    #[test]
    fn requires_a_dense_visual_array() {
        let mut visuals = chunk(0, b"a");
        visuals.extend(chunk(2, b"b"));
        let level = chunk(FSL_VISUALS, &visuals);
        let error = visual_payloads(&level).unwrap_err().to_string();
        assert!(error.contains("dense array"), "{error}");
    }

    #[test]
    fn reads_shared_vertex_and_index_buffers() {
        let mut declaration = Vec::new();
        for (offset, kind, usage) in [(0_u16, 2_u8, 0_u8), (12, 4, 3), (16, 4, 6), (20, 4, 7)] {
            let mut element = [0_u8; 8];
            element[2..4].copy_from_slice(&offset.to_le_bytes());
            element[4] = kind;
            element[6] = usage;
            declaration.extend(element);
        }
        let mut texcoord = [0_u8; 8];
        texcoord[2..4].copy_from_slice(&24_u16.to_le_bytes());
        texcoord[4] = 6; // SHORT2
        texcoord[6] = 5; // TEXCOORD
        declaration.extend(texcoord);
        let mut end = [0_u8; 8];
        end[0..2].copy_from_slice(&0xff_u16.to_le_bytes());
        end[4] = 17;
        declaration.extend(end);

        let mut vertex_payload = 1_u32.to_le_bytes().to_vec();
        vertex_payload.extend(&declaration);
        vertex_payload.extend(2_u32.to_le_bytes());
        vertex_payload.extend(vec![0_u8; 2 * 28]);

        let mut index_payload = 1_u32.to_le_bytes().to_vec();
        index_payload.extend(3_u32.to_le_bytes());
        index_payload.extend([0_u8, 0, 1, 0, 2, 0]);

        let mut geom = chunk(FSL_VB, &vertex_payload);
        geom.extend(chunk(FSL_IB, &index_payload));

        let buffers = parse_geometry(&geom).unwrap();
        assert_eq!(buffers.vertex_buffers.len(), 1);
        assert_eq!(buffers.vertex_buffers[0].count, 2);
        assert_eq!(buffers.vertex_buffers[0].layout.stride, 28);
        assert_eq!(buffers.index_buffers, vec![vec![0_u16, 1, 2]]);
    }
}
