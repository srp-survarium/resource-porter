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
//!   smoothness, metalness); [`ToneCurve`]s map them onto 0.26g's diffuse,
//!   gloss and Fresnel ranges by editing the DXT block endpoints.

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

/// A monotone map of 8-bit channel values, piecewise linear through knots.
#[derive(Clone, Copy, Debug)]
pub struct ToneCurve(pub &'static [(u8, u8)]);

impl ToneCurve {
    pub fn apply(&self, v: u8) -> u8 {
        let knots = self.0;
        let x = f32::from(v);
        for pair in knots.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            if v <= x1 {
                let t = (x - f32::from(x0)) / f32::from(x1 - x0).max(1.0);
                return (f32::from(y0) + t * (f32::from(y1) - f32::from(y0)))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        knots.last().map_or(v, |k| k.1)
    }
}

/// 0.69d -> 0.26g calibration, fitted on the seven assault-rifle texture sets
/// both builds ship (AK-74N, AK-12, A-545, AKMN, Vepr main/second, SR-3M):
/// quantiles of each 0.69d map against its 0.26g counterpart (mip 3), pooled.
/// Smoothness is 0.36 x (power 0.98, kept linear); metalness to Fresnel
/// follows 0.162 x m^0.388 (normalised). Per-pixel fits do no better for
/// these two (RMS 0.100 against 0.103, 0.082 against 0.093), so they are
/// remapped losslessly in the DXT endpoints. [`ALBEDO`] alone (about half as
/// bright from 32 up) leaves bare metal bright: the diffuse uses
/// [`diffuse_from_albedo`] instead; the curve stays for reference.
pub const ALBEDO: ToneCurve = ToneCurve(&[
    (0, 4),
    (16, 15),
    (32, 20),
    (48, 25),
    (64, 32),
    (80, 37),
    (96, 43),
    (112, 51),
    (128, 63),
    (144, 75),
    (160, 83),
    (255, 134),
]);
pub const SMOOTHNESS_TO_GLOSS: ToneCurve = ToneCurve(&[(0, 0), (255, 91)]);
pub const METALNESS_TO_FRESNEL: ToneCurve = ToneCurve(&[
    (0, 0),
    (4, 8),
    (16, 14),
    (32, 18),
    (64, 24),
    (96, 28),
    (128, 32),
    (160, 34),
    (192, 37),
    (224, 39),
    (255, 41),
]);

fn remap_565(c: u16, curve: ToneCurve) -> u16 {
    let r = u32::from(c >> 11) * 255 / 31;
    let g = u32::from((c >> 5) & 63) * 255 / 63;
    let b = u32::from(c & 31) * 255 / 31;
    let [r, g, b] = [r, g, b].map(|v| u32::from(curve.apply(v as u8)));
    let pack = |v: u32, max: u32| ((v * max + 127) / 255) as u16;
    (pack(r, 31) << 11) | (pack(g, 63) << 5) | pack(b, 31)
}

/// Applies `curve` to every colour endpoint of a DXT1 or DXT5 DDS, every
/// mip. Blocks keep their indices; DXT1 blocks keep their mode (an endpoint
/// pair that the curve would make equal or reorder is left as it is, so a
/// transparent-black block cannot appear).
pub fn remap_dds(bytes: &[u8], curve: ToneCurve) -> Result<Vec<u8>> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS");
    let h = u32_at(bytes, 12)? as usize;
    let w = u32_at(bytes, 16)? as usize;
    let mips = (u32_at(bytes, 28)? as usize).max(1);
    let block = match &bytes[84..88] {
        b"DXT1" => 8,
        b"DXT5" => 16,
        other => bail!(
            "unsupported DDS format {:?}",
            String::from_utf8_lossy(other)
        ),
    };
    let mut out = bytes.to_vec();
    let mut at = 128;
    for mip in 0..mips {
        let blocks = (w >> mip).max(1).div_ceil(4) * (h >> mip).max(1).div_ceil(4);
        for _ in 0..blocks {
            let c = at + block - 8;
            ensure!(c + 8 <= out.len(), "DDS shorter than its mips");
            let c0 = u16::from_le_bytes([out[c], out[c + 1]]);
            let c1 = u16::from_le_bytes([out[c + 2], out[c + 3]]);
            let (n0, n1) = (remap_565(c0, curve), remap_565(c1, curve));
            // DXT1 picks its mode by c0 > c1; DXT5 colour is always 4-colour.
            let keeps_mode = block == 16 || (c0 > c1) == (n0 > n1) && (c0 == c1) == (n0 == n1);
            if keeps_mode {
                out[c..c + 2].copy_from_slice(&n0.to_le_bytes());
                out[c + 2..c + 4].copy_from_slice(&n1.to_le_bytes());
            }
            at += block;
        }
    }
    Ok(out)
}

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

/// 0.26g diffuse from a 0.69d albedo and metalness, per pixel (sRGB, 0..1):
/// `0.514 * (albedo * (1 - 0.8 * metalness)) ^ 0.75`.
///
/// A metalness workflow paints bare metal bright in the albedo (it is the
/// metal's reflectance there); 0.26g draws the diffuse map as diffuse, so
/// metal must go dark. Fitted per pixel on the seven assault-rifle texture
/// sets both builds ship, whose UV layouts match (RMS error 0.054 against
/// 0.071 for albedo alone; metalness correlates 0.90 with the AK-12's 0.26g
/// Fresnel map).
pub fn diffuse_from_albedo(albedo: [u8; 3], metalness: u8) -> [u8; 3] {
    let m = f32::from(metalness) / 255.0;
    albedo.map(|a| {
        let v = 0.514 * ((f32::from(a) / 255.0) * (1.0 - 0.8 * m)).powf(0.75);
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    })
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
        assert!(paint[0] > paint[2]);
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

    #[test]
    fn tone_curves_interpolate_and_keep_dxt1_modes() {
        assert_eq!(ALBEDO.apply(0), 4);
        assert_eq!(ALBEDO.apply(24), 18);
        assert_eq!(SMOOTHNESS_TO_GLOSS.apply(255), 91);
        assert_eq!(SMOOTHNESS_TO_GLOSS.apply(144), 51);
        // One 4x4 DXT1 block, one mip: c0 > c1 stays four-colour.
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend(0xffffu16.to_le_bytes());
        dds.extend(0x8410u16.to_le_bytes());
        dds.extend([0x1b; 4]);
        let out = remap_dds(&dds, SMOOTHNESS_TO_GLOSS).unwrap();
        let c0 = u16::from_le_bytes([out[128], out[129]]);
        let c1 = u16::from_le_bytes([out[130], out[131]]);
        assert!(c0 > c1 && c0 < 0xffff);
        assert_eq!(&out[132..136], &[0x1b; 4]);
    }
}
