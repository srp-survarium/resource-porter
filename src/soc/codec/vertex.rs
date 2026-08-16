//! Direct3D 9 vertex declarations as stored in `level.geom`.
//!
//! X-Ray 1.x writes the literal `D3DVERTEXELEMENT9` array in front of every
//! vertex buffer, so the source layout is self-describing. This module decodes
//! that declaration and turns one source vertex into the engine-neutral
//! [`SocVertex`] used by the rest of the converter.
//!
//! Two source conventions matter, both taken from
//! `xray-16/src/Common/OGF_GContainer_Vertices.hpp`:
//!
//! * `SHORT2` texture coordinates are quantised to 1/1024 of a unit and carry
//!   their fractional remainder in the *alpha* byte of the tangent (u) and
//!   binormal (v). Ignoring those bytes shifts every texel by up to one
//!   quantisation step.
//! * `SHORT4` texture coordinates (multi-use models) carry no remainder; the
//!   two leading components are the whole coordinate.

use anyhow::{Context, Result, bail};

use super::chunks::Cursor;

/// `D3DDECLTYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclType {
    Float1,
    Float2,
    Float3,
    Float4,
    Color,
    UByte4,
    Short2,
    Short4,
    Other(u8),
}

impl DeclType {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Float1,
            1 => Self::Float2,
            2 => Self::Float3,
            3 => Self::Float4,
            4 => Self::Color,
            5 => Self::UByte4,
            6 => Self::Short2,
            7 => Self::Short4,
            other => Self::Other(other),
        }
    }

    /// Byte width of one attribute.
    pub fn size(self) -> Result<usize> {
        Ok(match self {
            Self::Float1 => 4,
            Self::Float2 => 8,
            Self::Float3 => 12,
            Self::Float4 => 16,
            Self::Color | Self::UByte4 | Self::Short2 => 4,
            Self::Short4 => 8,
            Self::Other(value) => bail!("unsupported D3DDECLTYPE {value}"),
        })
    }
}

/// `D3DDECLUSAGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclUsage {
    Position,
    Normal,
    TexCoord,
    Tangent,
    Binormal,
    Color,
    Other(u8),
}

impl DeclUsage {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Position,
            3 => Self::Normal,
            5 => Self::TexCoord,
            6 => Self::Tangent,
            7 => Self::Binormal,
            10 => Self::Color,
            other => Self::Other(other),
        }
    }
}

/// One `D3DVERTEXELEMENT9` record.
#[derive(Clone, Copy, Debug)]
pub struct VertexElement {
    pub stream: u16,
    pub offset: u16,
    pub kind: DeclType,
    pub usage: DeclUsage,
    pub usage_index: u8,
}

/// `D3DDECL_END()` is `{ 0xFF, 0, D3DDECLTYPE_UNUSED, 0, 0, 0 }`.
const DECL_END_STREAM: u16 = 0xff;
const DECL_TYPE_UNUSED: u8 = 17;

/// Read a declaration up to and including its `D3DDECL_END()` terminator.
pub fn parse_declaration(cursor: &mut Cursor<'_>) -> Result<Vec<VertexElement>> {
    let mut elements = Vec::new();
    loop {
        if elements.len() > 64 {
            bail!("vertex declaration is not terminated within 64 elements");
        }
        let stream = cursor.u16()?;
        let offset = cursor.u16()?;
        let kind = cursor.u8()?;
        let _method = cursor.u8()?;
        let usage = cursor.u8()?;
        let usage_index = cursor.u8()?;
        if stream == DECL_END_STREAM && kind == DECL_TYPE_UNUSED {
            return Ok(elements);
        }
        elements.push(VertexElement {
            stream,
            offset,
            kind: DeclType::from_raw(kind),
            usage: DeclUsage::from_raw(usage),
            usage_index,
        });
    }
}

/// Byte stride implied by a declaration, mirroring `GetDeclVertexSize`.
pub fn declaration_stride(elements: &[VertexElement]) -> Result<usize> {
    let mut stride = 0;
    for element in elements {
        if element.stream != 0 {
            bail!(
                "level geometry uses vertex stream {}, but only stream 0 is supported",
                element.stream
            );
        }
        stride = stride.max(usize::from(element.offset) + element.kind.size()?);
    }
    if stride == 0 {
        bail!("vertex declaration describes no attributes");
    }
    Ok(stride)
}

/// One source vertex resolved into engine-neutral values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SocVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub uv: [f32; 2],
}

/// A declaration resolved into the attribute offsets the converter needs.
#[derive(Clone, Debug)]
pub struct SocVertexLayout {
    pub stride: usize,
    position: usize,
    normal: usize,
    tangent: usize,
    binormal: usize,
    texcoord: usize,
    texcoord_kind: DeclType,
}

/// `s24_tc_base` uses `quant = 32768 / 32`, so one whole texture unit is 1024
/// quantisation steps and the stored remainder is a 1/255 subdivision of one
/// step.
const TC_SCALE: f32 = 32.0 / 32768.0;

/// Expand a `D3DCOLOR` byte the way the GPU feeds it to a vertex shader.
///
/// This is what makes the texture-coordinate remainder a *fraction* of a
/// quantisation step. X-Ray unpacks the coordinate in the vertex shader:
///
/// ```hlsl
/// // res/gamedata/shaders/r2/shared/common.h
/// float2 unpack_tc_base( float2 tc, float du, float dv ) {
///     return (tc.xy + float2(du,dv)) * (32.f/32768.f);
/// }
/// ```
///
/// called as `unpack_tc_base(v.tc, v.T.w, v.B.w)`. `T` and `B` are declared
/// `D3DCOLOR`, which the input assembler normalises to `[0,1]` -- so `du` is
/// `alpha/255`, never the raw byte. Adding the byte instead displaces the
/// coordinate by up to `255 * 32/32768` = 0.249 texture units, differently at
/// every vertex, which smears the texture across each triangle.
fn unorm8(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

impl SocVertexLayout {
    /// Resolve a declaration, rejecting layouts whose meaning is not proven.
    pub fn resolve(elements: &[VertexElement]) -> Result<Self> {
        let stride = declaration_stride(elements)?;
        let find = |usage: DeclUsage, index: u8| {
            elements
                .iter()
                .find(|element| element.usage == usage && element.usage_index == index)
        };
        let position = find(DeclUsage::Position, 0)
            .context("level geometry declaration has no POSITION attribute")?;
        if position.kind != DeclType::Float3 {
            bail!("POSITION is {:?}, expected FLOAT3", position.kind);
        }
        let basis = |usage: DeclUsage, name: &str| -> Result<usize> {
            let element = find(usage, 0)
                .with_context(|| format!("level geometry declaration has no {name} attribute"))?;
            if element.kind != DeclType::Color {
                bail!("{name} is {:?}, expected D3DCOLOR", element.kind);
            }
            Ok(usize::from(element.offset))
        };
        let texcoord =
            find(DeclUsage::TexCoord, 0).context("level geometry declaration has no TEXCOORD0")?;
        match texcoord.kind {
            DeclType::Short2 | DeclType::Short4 | DeclType::Float2 => {}
            other => bail!("TEXCOORD0 is {other:?}, expected SHORT2, SHORT4 or FLOAT2"),
        }
        Ok(Self {
            stride,
            position: usize::from(position.offset),
            normal: basis(DeclUsage::Normal, "NORMAL")?,
            tangent: basis(DeclUsage::Tangent, "TANGENT")?,
            binormal: basis(DeclUsage::Binormal, "BINORMAL")?,
            texcoord: usize::from(texcoord.offset),
            texcoord_kind: texcoord.kind,
        })
    }

    /// Decode one vertex from a `stride`-sized record.
    pub fn decode(&self, record: &[u8]) -> Result<SocVertex> {
        if record.len() != self.stride {
            bail!(
                "vertex record is {} bytes, expected {}",
                record.len(),
                self.stride
            );
        }
        let position = [
            read_f32(record, self.position)?,
            read_f32(record, self.position + 4)?,
            read_f32(record, self.position + 8)?,
        ];
        let (normal, _) = unpack_basis(read_u32(record, self.normal)?);
        let (tangent, tangent_alpha) = unpack_basis(read_u32(record, self.tangent)?);
        let (binormal, binormal_alpha) = unpack_basis(read_u32(record, self.binormal)?);
        let uv = match self.texcoord_kind {
            // The fractional remainder lives in the tangent/binormal alpha.
            DeclType::Short2 => [
                (f32::from(read_i16(record, self.texcoord)?) + unorm8(tangent_alpha)) * TC_SCALE,
                (f32::from(read_i16(record, self.texcoord + 2)?) + unorm8(binormal_alpha))
                    * TC_SCALE,
            ],
            // Multi-use models store the whole coordinate.
            DeclType::Short4 => [
                f32::from(read_i16(record, self.texcoord)?) * TC_SCALE,
                f32::from(read_i16(record, self.texcoord + 2)?) * TC_SCALE,
            ],
            DeclType::Float2 => [
                read_f32(record, self.texcoord)?,
                read_f32(record, self.texcoord + 4)?,
            ],
            other => bail!("unsupported TEXCOORD0 type {other:?}"),
        };
        let vertex = SocVertex {
            position,
            normal,
            tangent,
            binormal,
            uv,
        };
        if !vertex
            .position
            .into_iter()
            .chain(vertex.normal)
            .chain(vertex.tangent)
            .chain(vertex.binormal)
            .chain(vertex.uv)
            .all(f32::is_finite)
        {
            bail!("decoded vertex contains a non-finite component");
        }
        Ok(vertex)
    }
}

/// `u8_vec4` stores x/y/z in the red/green/blue bytes of a `D3DCOLOR` and keeps
/// alpha for unrelated payload (hemisphere light, or a texture-coordinate
/// remainder).
fn unpack_basis(color: u32) -> ([f32; 3], u8) {
    let channel = |shift: u32| f32::from(((color >> shift) & 0xff) as u8) / 255.0 * 2.0 - 1.0;
    (
        [channel(16), channel(8), channel(0)],
        ((color >> 24) & 0xff) as u8,
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .with_context(|| format!("missing u32 at vertex byte {offset}"))?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_i16(bytes: &[u8], offset: usize) -> Result<i16> {
    let slice = bytes
        .get(offset..offset + 2)
        .with_context(|| format!("missing i16 at vertex byte {offset}"))?;
    Ok(i16::from_le_bytes(slice.try_into().unwrap()))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(offset: u16, kind: u8, usage: u8) -> [u8; 8] {
        let mut bytes = [0_u8; 8];
        bytes[2..4].copy_from_slice(&offset.to_le_bytes());
        bytes[4] = kind;
        bytes[6] = usage;
        bytes
    }

    fn decl_end() -> [u8; 8] {
        let mut bytes = [0_u8; 8];
        bytes[0..2].copy_from_slice(&DECL_END_STREAM.to_le_bytes());
        bytes[4] = DECL_TYPE_UNUSED;
        bytes
    }

    /// `r1_decl_lmap`: position, basis triple, packed base UV, lightmap UV.
    fn r1_decl_lmap() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(element(0, 2, 0));
        bytes.extend(element(12, 4, 3));
        bytes.extend(element(16, 4, 6));
        bytes.extend(element(20, 4, 7));
        bytes.extend(element(24, 6, 5));
        let mut lightmap = element(28, 6, 5);
        lightmap[7] = 1;
        bytes.extend(lightmap);
        bytes.extend(decl_end());
        bytes
    }

    #[test]
    fn parses_the_lightmapped_level_declaration() {
        let bytes = r1_decl_lmap();
        let mut cursor = Cursor::new(&bytes, "level.geom");
        let elements = parse_declaration(&mut cursor).unwrap();
        assert_eq!(elements.len(), 6);
        assert!(cursor.is_empty());
        assert_eq!(declaration_stride(&elements).unwrap(), 32);
    }

    #[test]
    fn reconstructs_the_quantised_texture_coordinate() {
        let bytes = r1_decl_lmap();
        let mut cursor = Cursor::new(&bytes, "level.geom");
        let layout = SocVertexLayout::resolve(&parse_declaration(&mut cursor).unwrap()).unwrap();

        let mut record = vec![0_u8; 32];
        record[0..4].copy_from_slice(&1.0_f32.to_le_bytes());
        record[4..8].copy_from_slice(&2.0_f32.to_le_bytes());
        record[8..12].copy_from_slice(&3.0_f32.to_le_bytes());
        // Basis vectors encoded by `u8_vec4`: 255 -> +1, 128 -> ~0.
        record[12..16].copy_from_slice(&0x00_ff_80_80_u32.to_le_bytes()); // normal +x
        record[16..20].copy_from_slice(&0x80_80_ff_80_u32.to_le_bytes()); // tangent +y, alpha 128
        record[20..24].copy_from_slice(&0x40_80_80_ff_u32.to_le_bytes()); // binormal +z, alpha 64
        record[24..26].copy_from_slice(&1024_i16.to_le_bytes());
        record[26..28].copy_from_slice(&(-2048_i16).to_le_bytes());

        let vertex = layout.decode(&record).unwrap();
        assert_eq!(vertex.position, [1.0, 2.0, 3.0]);
        assert_eq!(vertex.normal[0], 1.0);
        assert_eq!(vertex.tangent[1], 1.0);
        assert_eq!(vertex.binormal[2], 1.0);
        // One whole texture unit is 1024 steps, plus 128/255 of one step.
        // The remainder is a fraction of one quantisation step, not whole
        // steps: D3DCOLOR reaches the shader normalised to [0,1].
        assert!((vertex.uv[0] - (1024.0 + 128.0 / 255.0) * TC_SCALE).abs() < 1e-6);
        assert!((vertex.uv[1] - (-2048.0 + 64.0 / 255.0) * TC_SCALE).abs() < 1e-6);
    }

    #[test]
    fn short4_texture_coordinates_carry_no_remainder() {
        let mut bytes = Vec::new();
        bytes.extend(element(0, 2, 0));
        bytes.extend(element(12, 4, 3));
        bytes.extend(element(16, 4, 6));
        bytes.extend(element(20, 4, 7));
        bytes.extend(element(24, 7, 5));
        bytes.extend(decl_end());
        let mut cursor = Cursor::new(&bytes, "level.geom");
        let layout = SocVertexLayout::resolve(&parse_declaration(&mut cursor).unwrap()).unwrap();
        assert_eq!(layout.stride, 32);

        let mut record = vec![0_u8; 32];
        record[16..20].copy_from_slice(&0xff_80_80_80_u32.to_le_bytes()); // tangent alpha 255
        record[24..26].copy_from_slice(&1024_i16.to_le_bytes());
        let vertex = layout.decode(&record).unwrap();
        assert!((vertex.uv[0] - 1024.0 * TC_SCALE).abs() < 1e-6);
    }

    #[test]
    fn rejects_a_declaration_without_a_tangent_basis() {
        let mut bytes = Vec::new();
        bytes.extend(element(0, 2, 0));
        bytes.extend(element(12, 4, 3));
        bytes.extend(element(16, 1, 5));
        bytes.extend(decl_end());
        let mut cursor = Cursor::new(&bytes, "level.geom");
        let elements = parse_declaration(&mut cursor).unwrap();
        let error = SocVertexLayout::resolve(&elements).unwrap_err().to_string();
        assert!(error.contains("TANGENT"), "{error}");
    }
}
