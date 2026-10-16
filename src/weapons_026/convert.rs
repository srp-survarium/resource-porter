// SPDX-License-Identifier: GPL-3.0-or-later
//! Byte-level conversions of 0.69d weapon resources to the 0.26g layouts.
//!
//! Each was established against a weapon both builds ship (the AK-12), see
//! `docs/weapons-069d-026.md`:
//!
//! * **Animations** (`.b-spline`): the same memory image, except that 0.69d
//!   writes each event channel's header in 12 bytes and 0.26g in 16 (a
//!   trailing pointer slot the engine fills on load). Every offset in the
//!   image is relative to the slot that holds it, so the bone slots (which do
//!   not move) shift by 4 per event channel and event header `i` (which
//!   moves by `4 * i`) by `4 * (events - i)`. Converted, the AK-12's shared
//!   clips are byte-identical to 0.26g's apart from uninitialised padding.
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
// Animations
// ---------------------------------------------------------------------------

/// `bi_spline_bone_animation_baked`: nine pointer slots of 8 bytes a bone.
const BONE_RECORD: usize = 0x48;
const EVENT_069: usize = 12;
const EVENT_026: usize = 16;

/// One event channel of a 0.26g clip: its name and, per knot, its time and
/// domain byte (for `sound_events`: the index of the state's sound to play,
/// 0xff for "the next one").
#[derive(Clone, Debug, PartialEq)]
pub struct EventChannel {
    pub name: String,
    pub knots: Vec<f32>,
    pub domains: Vec<u8>,
}

/// A clip's header fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipHeader {
    pub bones: usize,
    pub events: usize,
    pub animation_type: u8,
}

pub fn clip_header(b: &[u8]) -> Result<ClipHeader> {
    ensure!(b.len() >= 4, "b-spline shorter than its header");
    Ok(ClipHeader {
        bones: usize::from(u16::from_le_bytes([b[0], b[1]])),
        events: usize::from(b[2]),
        animation_type: b[3],
    })
}

/// Rewrites a 0.69d clip in the 0.26g layout (see the module docs).
pub fn bspline_069_to_026(b: &[u8]) -> Result<Vec<u8>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    let data = base + h.events * EVENT_069;
    ensure!(b.len() >= data, "b-spline shorter than its records");
    let shift = (4 * h.events) as u32;
    let mut out = Vec::with_capacity(b.len() + shift as usize);
    out.extend_from_slice(&b[..4]);
    for slot in 0..h.bones * 9 {
        let at = 4 + slot * 8;
        let offset = u32_at(b, at)?;
        ensure!(
            at + offset as usize + 4 <= b.len(),
            "bone slot {slot} points outside the clip"
        );
        out.extend_from_slice(&(offset + shift).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    for event in 0..h.events {
        let at = base + event * EVENT_069;
        let offset = u32_at(b, at)?;
        ensure!(u32_at(b, at + 4)? == 0, "event {event} header is not 0.69d's");
        ensure!(
            at + offset as usize <= b.len(),
            "event {event} points outside the clip"
        );
        // Header `event` moves by 4 * event; its data by 4 * events.
        out.extend_from_slice(&(offset + shift - 4 * event as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&b[at + 8..at + 12]);
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&b[data..]);
    // Every slot must still resolve to the same bytes.
    for slot in 0..h.bones * 9 {
        let (old, new) = (4 + slot * 8, 4 + slot * 8);
        let a = old + u32_at(b, old)? as usize;
        let c = new + u32_at(&out, new)? as usize;
        let pairs = u32_at(b, a)? as usize;
        ensure!(
            b.get(a..a + 4 + pairs * 8) == out.get(c..c + 4 + pairs * 8),
            "bone slot {slot} lost its curve"
        );
    }
    Ok(out)
}

/// The event channels of a 0.26g clip.
pub fn events_026(b: &[u8]) -> Result<Vec<EventChannel>> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    (0..h.events)
        .map(|event| {
            let header = base + event * EVENT_026;
            let at = header + u32_at(b, header)? as usize;
            let knots = usize::from(*b.get(header + 8).context("truncated event header")?);
            let mut channel = EventChannel {
                name: String::new(),
                knots: (0..knots)
                    .map(|k| f32_at(b, at + 4 * k))
                    .collect::<Result<_>>()?,
                domains: b
                    .get(at + 4 * knots..at + 5 * knots)
                    .context("truncated event domains")?
                    .to_vec(),
            };
            let name = b.get(at + 5 * knots..).context("truncated event name")?;
            let end = name
                .iter()
                .position(|c| *c == 0)
                .context("unterminated event name")?;
            channel.name = String::from_utf8(name[..end].to_vec())?;
            Ok(channel)
        })
        .collect()
}

/// Sets the domain bytes of a 0.26g clip's `sound_events` channel: which of
/// the state's sounds each knot plays.
pub fn set_sound_events(b: &mut [u8], indices: &[u8]) -> Result<()> {
    let h = clip_header(b)?;
    let base = 4 + h.bones * BONE_RECORD;
    for event in 0..h.events {
        let header = base + event * EVENT_026;
        let at = header + u32_at(b, header)? as usize;
        let knots = usize::from(b[header + 8]);
        let name = &b[at + 5 * knots..];
        if name.starts_with(b"sound_events\0") {
            ensure!(
                knots == indices.len(),
                "sound_events has {knots} knots, not {}",
                indices.len()
            );
            b[at + 4 * knots..at + 5 * knots].copy_from_slice(indices);
            return Ok(());
        }
    }
    bail!("the clip has no sound_events channel")
}

/// The clip's length: its longest curve's end knot.
pub fn clip_length_026(b: &[u8]) -> Result<f32> {
    let h = clip_header(b)?;
    let mut end = 0f32;
    for slot in 0..h.bones * 9 {
        let at = 4 + slot * 8;
        let curve = at + u32_at(b, at)? as usize;
        let pairs = u32_at(b, curve)? as usize;
        ensure!(pairs > 0, "empty curve");
        end = end.max(f32_at(b, curve + 4 + (pairs - 1) * 8)?);
    }
    Ok(end)
}

// ---------------------------------------------------------------------------
// Meshes
// ---------------------------------------------------------------------------

/// A 0.26g type-44 vertex: position, bone, normal/tangent/binormal words,
/// float2 UV.
pub const SKINNED_026: usize = 36;
const SKINNED_069: usize = 32;

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
        ensure!(self.vertices.len() <= 65_536, "too many vertices for u16 indices");
        ensure!(self.indices.len() % 3 == 0, "indices are not triangles");
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
        ensure!(stride == SKINNED_026, "not a 0.26g skinned mesh (stride {stride})");
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
    ensure!(4 + stride * n == body.len(), "vertex chunk size is not a stride");
    Ok((&body[4..], stride))
}

fn index_chunk(chunks: &[(u32, &[u8])], vertices: usize) -> Result<Vec<u16>> {
    let body = chunks
        .iter()
        .find(|(id, _)| *id == 4)
        .context("mesh has no index chunk")?
        .1;
    let n = u32_at(body, 0)? as usize;
    ensure!(body.len() == 4 + n * 2 && n % 3 == 0, "invalid index chunk");
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

/// Converts a 0.69d skinned (type 44) `converted_model` to 0.26g's.
pub fn skinned_069_to_026(b: &[u8], bones: usize) -> Result<Skinned> {
    let chunks = chunks(b)?;
    for (id, _) in &chunks {
        ensure!(
            matches!(id, 3 | 4 | 5 | 6 | 110 | 114),
            "unknown 0.69d skinned chunk {id}"
        );
    }
    let (vertices, stride) = vertex_chunk(&chunks)?;
    ensure!(stride == SKINNED_069, "not a 0.69d skinned mesh (stride {stride})");
    let mut out = Skinned::default();
    for v in vertices.chunks_exact(SKINNED_069) {
        let mut w = [0u8; SKINNED_026];
        for k in 0..3 {
            ensure!(f32_at(v, k * 4)?.is_finite(), "non-finite position");
        }
        w[..16].copy_from_slice(&v[..16]);
        ensure!((u32_at(v, 12)? as usize) < bones, "vertex bone outside the skeleton");
        for (k, alpha) in [0x7f, 0xff, 0xff].into_iter().enumerate() {
            let word = basis_word(f32_at(v, 16 + 4 * k)?, alpha)?;
            w[16 + 4 * k..20 + 4 * k].copy_from_slice(&word.to_le_bytes());
        }
        for k in 0..2 {
            let uv = half(u16::from_le_bytes([v[28 + 2 * k], v[29 + 2 * k]]));
            ensure!(uv.is_finite(), "non-finite UV");
            w[28 + 4 * k..32 + 4 * k].copy_from_slice(&uv.to_le_bytes());
        }
        out.vertices.push(w);
    }
    out.indices = index_chunk(&chunks, out.vertices.len())?;
    Ok(out)
}

/// A 0.69d static `converted_model` (an attachment module, authored in the
/// weapon mesh's orientation around its socket) as a skinned surface: moved
/// by `offset` and bound wholly to `bone`.
pub fn static_069_to_skinned(b: &[u8], offset: [f32; 3], bone: u32) -> Result<Skinned> {
    let chunks = chunks(b)?;
    for (id, _) in &chunks {
        ensure!(matches!(id, 3 | 4 | 110 | 114), "unknown 0.69d static chunk {id}");
    }
    let (vertices, stride) = vertex_chunk(&chunks)?;
    ensure!(
        matches!(stride, 28 | 32),
        "not a 0.69d static mesh (stride {stride})"
    );
    let mut out = Skinned::default();
    for v in vertices.chunks_exact(stride) {
        let mut w = [0u8; SKINNED_026];
        for k in 0..3 {
            let p = f32_at(v, 4 * k)? + offset[k];
            ensure!(p.is_finite(), "non-finite position");
            w[4 * k..4 * k + 4].copy_from_slice(&p.to_le_bytes());
        }
        w[12..16].copy_from_slice(&bone.to_le_bytes());
        for (k, alpha) in [0x7f, 0xff, 0xff].into_iter().enumerate() {
            let word = basis_word(f32_at(v, 12 + 4 * k)?, alpha)?;
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
        other => bail!("unsupported DDS format {:?}", String::from_utf8_lossy(other)),
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
    ensure!(w % 4 == 0 && h % 4 == 0 && w > 0 && h > 0, "odd DDS size {w}x{h}");
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
            let block = bytes.get(at..at + 8).context("DDS shorter than its top mip")?;
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            let (p0, p1) = (rgb(c0), rgb(c1));
            let mix = |a: u8, b: u8, wa: u32, wb: u32| ((u32::from(a) * wa + u32::from(b) * wb) / (wa + wb)) as u8;
            let palette = if c0 > c1 {
                [p0, p1, [0, 1, 2].map(|k| mix(p0[k], p1[k], 2, 1)), [0, 1, 2].map(|k| mix(p0[k], p1[k], 1, 2))]
            } else {
                [p0, p1, [0, 1, 2].map(|k| mix(p0[k], p1[k], 1, 1)), [0, 0, 0]]
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
    fn clip_069(bones: usize, events: &[(&str, &[f32], &[u8])]) -> Vec<u8> {
        let base = 4 + bones * BONE_RECORD;
        let data = base + events.len() * EVENT_069;
        let mut tail = vec![];
        let mut slots = vec![];
        for slot in 0..bones * 9 {
            let at = data + tail.len();
            slots.push(at - (4 + slot * 8));
            tail.extend(5u32.to_le_bytes());
            for k in 0..5 {
                tail.extend((k as f32).to_le_bytes());
                tail.extend((slot as f32).to_le_bytes());
            }
        }
        let mut headers = vec![];
        for (i, (name, knots, domains)) in events.iter().enumerate() {
            let at = data + tail.len();
            headers.push((at - (base + i * EVENT_069), knots.len()));
            for k in *knots {
                tail.extend(k.to_le_bytes());
            }
            tail.extend_from_slice(domains);
            tail.extend(name.as_bytes());
            tail.push(0);
            while tail.len() % 4 != 0 {
                tail.push(0);
            }
        }
        let mut b = vec![bones as u8, 0, events.len() as u8, 1];
        for s in slots {
            b.extend((s as u32).to_le_bytes());
            b.extend(0u32.to_le_bytes());
        }
        for (offset, knots) in headers {
            b.extend((offset as u32).to_le_bytes());
            b.extend(0u32.to_le_bytes());
            b.extend([knots as u8, 0, knots as u8, 2]);
        }
        b.extend(tail);
        b
    }

    #[test]
    fn a_069_clip_keeps_its_curves_and_events_in_the_026_layout() {
        let clip = clip_069(
            3,
            &[
                ("left_hand_ik", &[1.0, 20.0], &[2, 4]),
                ("sound_events", &[17.0, 43.0, 90.0], &[0xff, 0xff, 0xff]),
            ],
        );
        let out = bspline_069_to_026(&clip).unwrap();
        assert_eq!(out.len(), clip.len() + 8);
        let events = events_026(&out).unwrap();
        assert_eq!(events[0].name, "left_hand_ik");
        assert_eq!(events[0].knots, [1.0, 20.0]);
        assert_eq!(events[0].domains, [2, 4]);
        assert_eq!(events[1].name, "sound_events");
        assert_eq!(events[1].knots, [17.0, 43.0, 90.0]);
        assert_eq!(clip_length_026(&out).unwrap(), 4.0);
        let mut out = out;
        set_sound_events(&mut out, &[1, 2, 3]).unwrap();
        assert_eq!(events_026(&out).unwrap()[1].domains, [1, 2, 3]);
        assert!(set_sound_events(&mut out, &[1]).is_err());
    }

    #[test]
    fn a_clip_without_events_is_unchanged_but_its_pointer_slots() {
        let clip = clip_069(2, &[]);
        assert_eq!(bspline_069_to_026(&clip).unwrap(), clip);
    }

    #[test]
    fn an_already_converted_clip_is_refused() {
        let clip = clip_069(1, &[("sound_events", &[1.0], &[0xff])]);
        let out = bspline_069_to_026(&clip).unwrap();
        // The 0.26g header's second word is the 0.69d header's next offset.
        let mut tampered = out.clone();
        tampered[4 + BONE_RECORD + 4] = 1;
        assert!(bspline_069_to_026(&tampered).is_err());
    }

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
            .map(|k| u32::from_le_bytes(mesh.vertices[0][16 + 4 * k..20 + 4 * k].try_into().unwrap()))
            .collect();
        assert_eq!(words, [0x7f7f80ff, 0xff0ab07f, 0xff252580]);
        let uv: Vec<f32> = (0..2)
            .map(|k| f32::from_le_bytes(mesh.vertices[0][28 + 4 * k..32 + 4 * k].try_into().unwrap()))
            .collect();
        assert_eq!(uv, [0.75, 0.125]);
        let bytes = mesh.to_bytes().unwrap();
        assert_eq!(Skinned::parse_026(&bytes).unwrap(), mesh);
        assert!(skinned_069_to_026(&b, 2).is_err(), "bone 2 of a two-bone skeleton");
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
        let mesh = static_069_to_skinned(&b, [0.0, 0.0645, 0.5522], 1).unwrap();
        let p = mesh.position(0);
        assert!((p[1] - 0.0745).abs() < 1e-6 && (p[2] - 0.5722).abs() < 1e-6);
        assert_eq!(mesh.bone(0), 1);
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
