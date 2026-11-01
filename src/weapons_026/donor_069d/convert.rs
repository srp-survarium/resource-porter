// SPDX-License-Identifier: GPL-3.0-or-later
//! Byte-level conversions of 0.69d0's meshes and textures to 0.26g's layouts
//! (its clips' event headers are the core's [`super::super::clips`]), each
//! established against the AK-12, which both builds ship
//! (`docs/weapons-069d-026.md`):
//!
//! * **Skinned meshes** (type 44, one bone a vertex): 0.69d stores 32-byte
//!   vertices (position, bone, three basis floats, half2 UV) and 0.26g 36
//!   bytes (position, bone, three packed basis words, float2 UV). The basis
//!   float is `(x << 16 | y << 8 | z) / 2^24`, the bytes 0.26g packs.
//!   0.69d's chunks 5/6 (used bones and their boxes) and 110/114 (mesh
//!   statistics) have no 0.26g counterpart.
//! * **Static meshes** (the attachment modules) become skinned surfaces of
//!   the weapon, bound to one bone and moved to their socket.
//! * **Textures**: the same DDS layout. 0.69d's maps are PBR (albedo,
//!   smoothness, metalness); [`diffuse_from_albedo`], [`gloss_from_smoothness`]
//!   and [`fresnel_from_metalness`] map each pixel onto 0.26g's diffuse, gloss
//!   and Fresnel, and the map is encoded again (DXT1, the source's mips).

use anyhow::{Context, Result, bail, ensure};

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated u32")?.try_into()?,
    ))
}
fn f32_at(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}

// ---------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------

/// A 0.26g type-44 vertex: position, bone, normal/tangent/binormal words,
/// float2 UV.
pub const SKINNED_026: usize = 36;
const SKINNED_069: usize = 32;
const SKINNED_069_TWO_BONES: usize = 36;

/// The 0.26g word of a 0.69d basis float. The normal's top byte is 127 on
/// every stock 0.26g weapon; tangent and binormal top bytes vary there
/// without a pattern and 0.69d stores none, so they take the static 255.
fn basis_word(value: f32, alpha: u32) -> Result<u32> {
    ensure!(
        value.is_finite() && (0.0..1.0).contains(&value),
        "basis value {value} is not a 0.69d packed float"
    );
    let packed = (f64::from(value) * 16_777_216.0).round() as u32;
    ensure!(packed <= 0xff_ffff, "packed basis overflow");
    Ok(alpha << 24 | packed)
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let frac = f32::from(bits & 0x3ff);
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// A mesh: vertices in 0.26g's skinned layout and triangle indices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skinned {
    pub vertices: Vec<[u8; SKINNED_026]>,
    pub indices: Vec<u16>,
}

impl Skinned {
    pub fn position(&self, i: usize) -> [f32; 3] {
        let v = &self.vertices[i];
        [0, 4, 8].map(|o| f32::from_le_bytes(v[o..o + 4].try_into().unwrap()))
    }
    pub fn bone(&self, i: usize) -> u32 {
        u32::from_le_bytes(self.vertices[i][12..16].try_into().unwrap())
    }
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for i in 0..self.vertices.len() {
            let p = self.position(i);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }
    /// The 0.26g `converted_model`: vertex chunk 3, index chunk 4.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        ensure!(!self.vertices.is_empty(), "empty mesh");
        ensure!(
            self.vertices.len() <= 65_536,
            "too many vertices for u16 indices"
        );
        ensure!(
            self.indices.len().is_multiple_of(3),
            "indices are not triangles"
        );
        let mut out = vec![];
        out.extend(3u32.to_le_bytes());
        out.extend(u32::try_from(4 + self.vertices.len() * SKINNED_026)?.to_le_bytes());
        out.extend(u32::try_from(self.vertices.len())?.to_le_bytes());
        for v in &self.vertices {
            out.extend_from_slice(v);
        }
        out.extend(4u32.to_le_bytes());
        out.extend(u32::try_from(4 + self.indices.len() * 2)?.to_le_bytes());
        out.extend(u32::try_from(self.indices.len())?.to_le_bytes());
        for i in &self.indices {
            out.extend(i.to_le_bytes());
        }
        Ok(out)
    }
    /// Reads a 0.26g skinned `converted_model`.
    pub fn parse_026(b: &[u8]) -> Result<Self> {
        let chunks = chunks(b)?;
        let (vertices, stride) = vertex_chunk(&chunks)?;
        ensure!(
            stride == SKINNED_026,
            "not a 0.26g skinned mesh (stride {stride})"
        );
        Ok(Self {
            vertices: vertices
                .chunks_exact(SKINNED_026)
                .map(|v| v.try_into().unwrap())
                .collect(),
            indices: index_chunk(&chunks, vertices.len() / SKINNED_026)?,
        })
    }
    /// Appends `other`, renumbering its indices.
    pub fn append(&mut self, other: &Skinned) -> Result<()> {
        let base = u16::try_from(self.vertices.len()).context("mesh too large to merge")?;
        ensure!(
            self.vertices.len() + other.vertices.len() <= 65_536,
            "merged mesh exceeds u16 indices"
        );
        self.vertices.extend_from_slice(&other.vertices);
        self.indices.extend(other.indices.iter().map(|i| i + base));
        Ok(())
    }
}

fn chunks(b: &[u8]) -> Result<Vec<(u32, &[u8])>> {
    let mut out = vec![];
    let mut at = 0;
    while at < b.len() {
        let id = u32_at(b, at)?;
        let size = u32_at(b, at + 4)? as usize;
        let body = b
            .get(at + 8..at + 8 + size)
            .with_context(|| format!("truncated chunk {id}"))?;
        out.push((id, body));
        at += 8 + size;
    }
    Ok(out)
}

fn vertex_chunk<'a>(chunks: &[(u32, &'a [u8])]) -> Result<(&'a [u8], usize)> {
    let [(3, body), ..] = chunks else {
        bail!("mesh does not start with a vertex chunk");
    };
    let n = u32_at(body, 0)? as usize;
    ensure!(n > 0, "mesh has no vertices");
    let stride = (body.len() - 4) / n;
    ensure!(
        4 + stride * n == body.len(),
        "vertex chunk size is not a stride"
    );
    Ok((&body[4..], stride))
}

fn index_chunk(chunks: &[(u32, &[u8])], vertices: usize) -> Result<Vec<u16>> {
    let body = chunks
        .iter()
        .find(|(id, _)| *id == 4)
        .context("mesh has no index chunk")?
        .1;
    let n = u32_at(body, 0)? as usize;
    ensure!(
        body.len() == 4 + n * 2 && n.is_multiple_of(3),
        "invalid index chunk"
    );
    let indices: Vec<u16> = body[4..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    ensure!(
        indices.iter().all(|i| usize::from(*i) < vertices),
        "index outside the vertex array"
    );
    Ok(indices)
}

/// Converts a 0.69d skinned `converted_model` to 0.26g's (type 44, one bone
/// a vertex). 0.69d has two layouts: 32 bytes (position, `u32` bone, three
/// basis floats, half2 UV) and 36 bytes (position, two `u8` bones and two
/// unused, the first bone's `f32` weight, the basis and UV as before). A
/// two-bone vertex goes wholly to its heavier bone: 0.26g's weapon meshes
/// carry one (the AKMSU's recoil spring is the one blended part met).
pub fn skinned_069_to_026(b: &[u8], bones: usize) -> Result<Skinned> {
    let chunks = chunks(b)?;
    for (id, _) in &chunks {
        ensure!(
            matches!(id, 3 | 4 | 5 | 6 | 110 | 114),
            "unknown 0.69d skinned chunk {id}"
        );
    }
    let (vertices, stride) = vertex_chunk(&chunks)?;
    let (basis_at, uv_at) = match stride {
        SKINNED_069 => (16, 28),
        SKINNED_069_TWO_BONES => (20, 32),
        other => bail!("not a 0.69d skinned mesh (stride {other})"),
    };
    let mut out = Skinned::default();
    for v in vertices.chunks_exact(stride) {
        let mut w = [0u8; SKINNED_026];
        for k in 0..3 {
            ensure!(f32_at(v, k * 4)?.is_finite(), "non-finite position");
        }
        w[..12].copy_from_slice(&v[..12]);
        let bone = if stride == SKINNED_069 {
            u32_at(v, 12)?
        } else {
            let weight = f32_at(v, 16)?;
            ensure!((0.0..=1.0).contains(&weight), "bone weight {weight}");
            u32::from(if weight >= 0.5 { v[12] } else { v[13] })
        };
        ensure!((bone as usize) < bones, "vertex bone outside the skeleton");
        w[12..16].copy_from_slice(&bone.to_le_bytes());
        for (k, alpha) in [0x7f, 0xff, 0xff].into_iter().enumerate() {
            let word = basis_word(f32_at(v, basis_at + 4 * k)?, alpha)?;
            w[16 + 4 * k..20 + 4 * k].copy_from_slice(&word.to_le_bytes());
        }
        for k in 0..2 {
            let uv = half(u16::from_le_bytes([v[uv_at + 2 * k], v[uv_at + 1 + 2 * k]]));
            ensure!(uv.is_finite(), "non-finite UV");
            w[28 + 4 * k..32 + 4 * k].copy_from_slice(&uv.to_le_bytes());
        }
        out.vertices.push(w);
    }
    out.indices = index_chunk(&chunks, out.vertices.len())?;
    Ok(out)
}

/// A 0.69d static `converted_model` (an attachment module, authored in the
/// weapon mesh's orientation around its socket) as a skinned surface: turned
/// by `rotation` (a 3x3 matrix, rows), moved by `offset` and bound wholly
/// to `bone`.
pub fn static_069_to_skinned(
    b: &[u8],
    rotation: [[f32; 3]; 3],
    offset: [f32; 3],
    bone: u32,
) -> Result<Skinned> {
    let chunks = chunks(b)?;
    for (id, _) in &chunks {
        ensure!(
            matches!(id, 3 | 4 | 110 | 114),
            "unknown 0.69d static chunk {id}"
        );
    }
    let (vertices, stride) = vertex_chunk(&chunks)?;
    ensure!(
        matches!(stride, 28 | 32),
        "not a 0.69d static mesh (stride {stride})"
    );
    let turn = |v: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|r| {
            rotation[r][0] * v[0] + rotation[r][1] * v[1] + rotation[r][2] * v[2]
        })
    };
    let mut out = Skinned::default();
    for v in vertices.chunks_exact(stride) {
        let mut w = [0u8; SKINNED_026];
        let p = turn([f32_at(v, 0)?, f32_at(v, 4)?, f32_at(v, 8)?]);
        for k in 0..3 {
            let p = p[k] + offset[k];
            ensure!(p.is_finite(), "non-finite position");
            w[4 * k..4 * k + 4].copy_from_slice(&p.to_le_bytes());
        }
        w[12..16].copy_from_slice(&bone.to_le_bytes());
        for (k, alpha) in [0x7f, 0xff, 0xff].into_iter().enumerate() {
            let value = turn_basis(f32_at(v, 12 + 4 * k)?, &turn)?;
            let word = basis_word(value, alpha)?;
            w[16 + 4 * k..20 + 4 * k].copy_from_slice(&word.to_le_bytes());
        }
        for k in 0..2 {
            let uv = half(u16::from_le_bytes([v[24 + 2 * k], v[25 + 2 * k]]));
            ensure!(uv.is_finite(), "non-finite UV");
            w[28 + 4 * k..32 + 4 * k].copy_from_slice(&uv.to_le_bytes());
        }
        out.vertices.push(w);
    }
    out.indices = index_chunk(&chunks, out.vertices.len())?;
    Ok(out)
}

/// A packed basis float (`(x << 16 | y << 8 | z) / 2^24`, each byte a
/// component in `[-1, 1]` as `127.5 * (c + 1)`) turned by `turn`.
fn turn_basis(value: f32, turn: &impl Fn([f32; 3]) -> [f32; 3]) -> Result<f32> {
    ensure!(
        value.is_finite() && (0.0..1.0).contains(&value),
        "basis value {value} is not a 0.69d packed float"
    );
    let packed = (f64::from(value) * 16_777_216.0).round() as u32;
    let unpack = |shift: u32| f32::from(((packed >> shift) & 0xff) as u8) / 127.5 - 1.0;
    let v = turn([unpack(16), unpack(8), unpack(0)]);
    let pack = |c: f32| (((c.clamp(-1.0, 1.0) + 1.0) * 127.5).round() as u32).min(255);
    let packed = pack(v[0]) << 16 | pack(v[1]) << 8 | pack(v[2]);
    Ok((f64::from(packed) / 16_777_216.0) as f32)
}

// ---------------------------------------------------------------------------
// Textures
// ---------------------------------------------------------------------------

/// The top mip of a DXT1 DDS as RGB pixels, row by row: `(width, height,
/// pixels)`.
pub fn decode_dxt1(bytes: &[u8]) -> Result<(usize, usize, Vec<[u8; 3]>)> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS");
    ensure!(&bytes[84..88] == b"DXT1", "not DXT1");
    let h = u32_at(bytes, 12)? as usize;
    let w = u32_at(bytes, 16)? as usize;
    ensure!(
        w.is_multiple_of(4) && h.is_multiple_of(4) && w > 0 && h > 0,
        "odd DDS size {w}x{h}"
    );
    let mut px = vec![[0u8; 3]; w * h];
    let rgb = |c: u16| {
        [
            (u32::from(c >> 11) * 255 / 31) as u8,
            (u32::from((c >> 5) & 63) * 255 / 63) as u8,
            (u32::from(c & 31) * 255 / 31) as u8,
        ]
    };
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            let at = 128 + (by * (w / 4) + bx) * 8;
            let block = bytes
                .get(at..at + 8)
                .context("DDS shorter than its top mip")?;
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            let (p0, p1) = (rgb(c0), rgb(c1));
            let mix = |a: u8, b: u8, wa: u32, wb: u32| {
                ((u32::from(a) * wa + u32::from(b) * wb) / (wa + wb)) as u8
            };
            let palette = if c0 > c1 {
                [
                    p0,
                    p1,
                    [0, 1, 2].map(|k| mix(p0[k], p1[k], 2, 1)),
                    [0, 1, 2].map(|k| mix(p0[k], p1[k], 1, 2)),
                ]
            } else {
                [
                    p0,
                    p1,
                    [0, 1, 2].map(|k| mix(p0[k], p1[k], 1, 1)),
                    [0, 0, 0],
                ]
            };
            let bits = u32::from_le_bytes(block[4..8].try_into().unwrap());
            for k in 0..16 {
                let (x, y) = (bx * 4 + k % 4, by * 4 + k / 4);
                px[y * w + x] = palette[((bits >> (2 * k)) & 3) as usize];
            }
        }
    }
    Ok((w, h, px))
}

/// The calibration, fitted on the ten texture sets both builds ship with the
/// same layout (their normal maps match: the A-545, AK-12, AK-74M, AK-74N,
/// AKMN, ASh-12, PPSh, ACOG, Cobra and CompM4). The gloss and Fresnel are
/// fitted per pixel; the diffuse to each set's mean, contrast and saturation
/// (the PPSh: stock 34.5, 20.9, 7.0 of 255; made 32.2, 16.4, 5.4).
///
/// Before, the gloss and Fresnel curves were applied to the DXT colour
/// endpoints and a block whose endpoints the curve made equal was left as it
/// was: raw metalness as Fresnel in up to 63 % of a map's blocks (the AK-74M's;
/// the A-545's 38 %), so the guns came out 2 to 3 times as metallic as the
/// stock ones (Fresnel 0.26 to 0.38 against 0.10 to 0.18). The Fresnel was 0
/// where the metalness is 0, where 0.26g gives every surface some (median
/// 21/255). And the diffuse, `0.514 * (albedo * (1 - 0.8 m)) ^ 0.75`, was right
/// on average but had half the stock contrast and saturation.
///
/// 0.26g diffuse from a 0.69d albedo and metalness, per pixel (sRGB values):
/// `0.76 * albedo * (1 - 0.5 * metalness)`. A metalness workflow paints bare
/// metal bright in the albedo (its reflectance there); 0.26g draws the
/// diffuse map as diffuse, so metal goes darker.
pub fn diffuse_from_albedo(albedo: [u8; 3], metalness: u8) -> [u8; 3] {
    let m = f32::from(metalness) / 255.0;
    albedo.map(|a| {
        (0.76 * f32::from(a) * (1.0 - 0.5 * m))
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// 0.26g gloss (the `texture_roughness` slot: 0.26g's g-buffer pass reads it
/// as smoothness, `1 - (min + (max - min) g)` the GGX roughness) from a 0.69d
/// smoothness: `0.36 * smoothness`.
pub fn gloss_from_smoothness(smoothness: u8) -> u8 {
    (0.36 * f32::from(smoothness)).round() as u8
}

/// 0.26g Fresnel F0 from a 0.69d metalness: `0.08 + 0.09 * sqrt(metalness)`.
pub fn fresnel_from_metalness(metalness: u8) -> u8 {
    let m = f32::from(metalness) / 255.0;
    (255.0 * (0.08 + 0.09 * m.sqrt())).round().clamp(0.0, 255.0) as u8
}

/// An uncompressed 24-bit TGA, top row first, for the DDS encoder.
pub fn tga(w: usize, h: usize, px: &[[u8; 3]]) -> Result<Vec<u8>> {
    ensure!(px.len() == w * h, "pixel count");
    let mut out = vec![0u8; 18];
    out[2] = 2;
    out[12..14].copy_from_slice(&u16::try_from(w)?.to_le_bytes());
    out[14..16].copy_from_slice(&u16::try_from(h)?.to_le_bytes());
    out[16] = 24;
    out[17] = 0x20; // origin top-left
    for p in px {
        out.extend([p[2], p[1], p[0]]);
    }
    Ok(out)
}

/// Sets the values of the event channel `name` of a clip in 0.26g's layout
/// (the knots' times, then a byte value each: a `sound_events` channel's
/// indices in its state's sound list), one for each knot.
pub fn set_event_values(clip: &mut [u8], name: &str, values: &[u8]) -> Result<()> {
    use crate::weapons_026::clips::{BONE_RECORD, EVENT_HEADER_026, clip_header, event_names};
    let h = clip_header(clip)?;
    let index = event_names(clip)?
        .iter()
        .position(|n| n == name)
        .with_context(|| format!("no event channel {name}"))?;
    let header = 4 + h.bones * BONE_RECORD + index * EVENT_HEADER_026;
    let knots = usize::from(clip[header + 8]);
    ensure!(
        knots == values.len(),
        "{name} has {knots} knots, {} values given",
        values.len()
    );
    let at = header + u32_at(clip, header)? as usize + 4 * knots;
    clip.get_mut(at..at + knots)
        .context("event values outside the clip")?
        .copy_from_slice(values);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip of `bones` constant bones and the given event channels, in the
    /// 0.69d layout (12-byte event headers), with the curves laid out in
    /// slot order and the events after them.
    fn half_bits(v: f32) -> u16 {
        // Exact for the values the tests use.
        if v == 0.0 {
            return 0;
        }
        let e = v.abs().log2().floor() as i32;
        let frac = ((v.abs() / 2f32.powi(e) - 1.0) * 1024.0).round() as u16;
        ((e + 15) as u16) << 10 | frac
    }

    #[test]
    fn skinned_vertices_widen_to_36_bytes_and_drop_bone_boxes() {
        let mut v = vec![];
        v.extend(3u32.to_le_bytes());
        v.extend((4u32 + 32).to_le_bytes());
        v.extend(1u32.to_le_bytes());
        for p in [0.5f32, -0.25, 2.0] {
            v.extend(p.to_le_bytes());
        }
        v.extend(2u32.to_le_bytes());
        for packed in [0x7f80ffu32, 0x0ab07f, 0x252580] {
            v.extend((packed as f32 / 16_777_216.0).to_le_bytes());
        }
        v.extend(half_bits(0.75).to_le_bytes());
        v.extend(half_bits(0.125).to_le_bytes());
        let mut b = v;
        for (id, body) in [(5u32, vec![1u8, 2]), (6, vec![0; 24])] {
            b.extend(id.to_le_bytes());
            b.extend((body.len() as u32).to_le_bytes());
            b.extend(body);
        }
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend([0u8; 6]);
        let mesh = skinned_069_to_026(&b, 3).unwrap();
        assert_eq!(mesh.position(0), [0.5, -0.25, 2.0]);
        assert_eq!(mesh.bone(0), 2);
        let words: Vec<u32> = (0..3)
            .map(|k| {
                u32::from_le_bytes(mesh.vertices[0][16 + 4 * k..20 + 4 * k].try_into().unwrap())
            })
            .collect();
        assert_eq!(words, [0x7f7f80ff, 0xff0ab07f, 0xff252580]);
        let uv: Vec<f32> = (0..2)
            .map(|k| {
                f32::from_le_bytes(mesh.vertices[0][28 + 4 * k..32 + 4 * k].try_into().unwrap())
            })
            .collect();
        assert_eq!(uv, [0.75, 0.125]);
        let bytes = mesh.to_bytes().unwrap();
        assert_eq!(Skinned::parse_026(&bytes).unwrap(), mesh);
        assert!(
            skinned_069_to_026(&b, 2).is_err(),
            "bone 2 of a two-bone skeleton"
        );
    }

    #[test]
    fn a_static_module_moves_to_its_socket_on_one_bone() {
        let mut b = vec![];
        b.extend(3u32.to_le_bytes());
        b.extend((4u32 + 28).to_le_bytes());
        b.extend(1u32.to_le_bytes());
        for p in [0.0f32, 0.01, 0.02] {
            b.extend(p.to_le_bytes());
        }
        for packed in [0x7f80ffu32, 0x0ab07f, 0x252580] {
            b.extend((packed as f32 / 16_777_216.0).to_le_bytes());
        }
        b.extend(half_bits(0.5).to_le_bytes());
        b.extend(half_bits(0.25).to_le_bytes());
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend([0u8; 6]);
        const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mesh = static_069_to_skinned(&b, IDENTITY, [0.0, 0.0645, 0.5522], 1).unwrap();
        let p = mesh.position(0);
        assert!((p[1] - 0.0745).abs() < 1e-6 && (p[2] - 0.5722).abs() < 1e-6);
        assert_eq!(mesh.bone(0), 1);
        // A quarter turn about y takes +z to +x, the normal with it.
        let quarter = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
        let turned = static_069_to_skinned(&b, quarter, [0.0; 3], 1).unwrap();
        let p = turned.position(0);
        assert!(
            (p[0] - 0.02).abs() < 1e-6 && (p[1] - 0.01).abs() < 1e-6 && p[2].abs() < 1e-6,
            "{p:?}"
        );
        let normal = u32::from_le_bytes(turned.vertices[0][16..20].try_into().unwrap());
        // 0x7f80ff: (0, 0.004, 1) turns to (1, 0.004, 0): x byte 255, z byte 127 or 128.
        assert_eq!(normal >> 16 & 0xff, 0xff);
        assert!(matches!(normal & 0xff, 0x7f | 0x80), "{normal:#x}");
    }

    #[test]
    fn a_two_bone_vertex_goes_to_its_heavier_bone() {
        let vertex = |b0: u8, b1: u8, weight: f32| {
            let mut v = vec![];
            for p in [0.0f32, 0.01, 0.02] {
                v.extend(p.to_le_bytes());
            }
            v.extend([b0, b1, 0, 0]);
            v.extend(weight.to_le_bytes());
            for packed in [0x7f80ffu32, 0x0ab07f, 0x252580] {
                v.extend((packed as f32 / 16_777_216.0).to_le_bytes());
            }
            v.extend(half_bits(0.5).to_le_bytes());
            v.extend(half_bits(0.25).to_le_bytes());
            v
        };
        let mut b = vec![];
        b.extend(3u32.to_le_bytes());
        b.extend((4u32 + 3 * 36).to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend(vertex(1, 1, 1.0));
        b.extend(vertex(1, 2, 0.75));
        b.extend(vertex(1, 2, 0.25));
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend([0u8, 0, 1, 0, 2, 0]);
        let mesh = skinned_069_to_026(&b, 3).unwrap();
        assert_eq!([mesh.bone(0), mesh.bone(1), mesh.bone(2)], [1, 1, 2]);
        let uv: Vec<f32> = (0..2)
            .map(|k| {
                f32::from_le_bytes(mesh.vertices[0][28 + 4 * k..32 + 4 * k].try_into().unwrap())
            })
            .collect();
        assert_eq!(uv, [0.5, 0.25]);
        assert!(
            skinned_069_to_026(&b, 2).is_err(),
            "bone 2 of a two-bone skeleton"
        );
    }

    #[test]
    fn metal_goes_dark_in_the_diffuse_and_paint_keeps_its_shade() {
        let paint = diffuse_from_albedo([140, 120, 90], 0);
        let metal = diffuse_from_albedo([230, 230, 230], 255);
        assert!(metal[0] < paint[0], "{metal:?} {paint:?}");
        assert_eq!(diffuse_from_albedo([0, 0, 0], 0), [0, 0, 0]);
        assert_eq!(paint, [106, 91, 68], "the albedo's contrast and hue kept");
    }

    #[test]
    fn every_surface_has_some_fresnel_and_gloss_follows_smoothness() {
        assert_eq!(
            fresnel_from_metalness(0),
            20,
            "a dielectric's F0 as 0.26g's maps have it"
        );
        assert_eq!(fresnel_from_metalness(255), 43);
        assert!((0..255).all(|m| fresnel_from_metalness(m) <= fresnel_from_metalness(m + 1)));
        assert_eq!(gloss_from_smoothness(0), 0);
        assert_eq!(gloss_from_smoothness(255), 92);
    }

    #[test]
    fn a_dxt1_block_decodes_to_its_palette() {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend(0xf800u16.to_le_bytes()); // red
        dds.extend(0x001fu16.to_le_bytes()); // blue
        dds.extend(0b01u32.to_le_bytes()); // texel 0 = c1, the rest c0
        let (w, h, px) = decode_dxt1(&dds).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(px[0], [0, 0, 255]);
        assert_eq!(px[1], [255, 0, 0]);
        let t = tga(4, 4, &px).unwrap();
        assert_eq!(t.len(), 18 + 48);
        assert_eq!(&t[18..21], &[255, 0, 0], "BGR");
    }
}
