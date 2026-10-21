// SPDX-License-Identifier: GPL-3.0-or-later
//! Skinned surfaces (`converted_model`) of Survarium 0.69d in the v0.26g
//! layout.
//!
//! Both builds lay a surface out as chunks: 3 the vertices (count, then the
//! vertices), 4 the 16-bit indices (count, then the indices). 0.69d adds
//! 5, the bones the surface uses (a count byte, then one byte a bone), 6,
//! one bounding box a bone (min and max, six floats), and two chunks of mesh
//! statistics, 110 and 114. 0.26g reads none of them.
//!
//! A vertex of either build: float3 position, four bone bytes (indices into
//! the bind pose; unused slots repeat a bone at weight 0), then one float a
//! weight but the last (which is the rest). Its size says how many bones.
//! Then:
//!
//! - 0.26g: normal, tangent and binormal packed as bytes `z, y, x, w` (each
//!   `(v + 1) * 127.5`); `w` is 127 on the normal and the fraction of
//!   `u * 1024` and `v * 1024` on the tangent and binormal. Then float2 UV.
//!   36 bytes for one bone up to 48 for four.
//! - 0.69d: the same three vectors, each one float equal to
//!   `(x << 16 | y << 8 | z) / 2^24` (the three bytes exactly in the
//!   mantissa, as Duga found for statics), then half2 UV. 32 bytes for one
//!   bone up to 44.
//!
//! The rig is the same: 0.69d's head-worn models bind the 69 bones of
//! 0.26g's player in the same order at the same place (only Root and Item
//! differ, which nothing is skinned to), so bone indices carry over.
use anyhow::{Context, Result, bail, ensure};

/// A vertex of either build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub bones: [u8; 4],
    /// All four weights (the last is one less the others).
    pub weights: [f32; 4],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub uv: [f32; 2],
}

/// A skinned surface.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    /// Bones in use, from chunk 5.
    pub bones: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let bytes = b.get(at..at + 4).context("truncated surface")?;
    Ok(u32::from_le_bytes(bytes.try_into()?))
}

fn f32_at(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}

/// The chunks of a surface, by id, in their order.
fn chunks(b: &[u8]) -> Result<Vec<(u32, &[u8])>> {
    let mut out = vec![];
    let mut at = 0;
    while at < b.len() {
        let id = u32_at(b, at)?;
        let size = usize::try_from(u32_at(b, at + 4)?)?;
        let body = b
            .get(at + 8..at + 8 + size)
            .with_context(|| format!("chunk {id} runs past the surface"))?;
        out.push((id, body));
        at += 8 + size;
    }
    Ok(out)
}

/// One vector of the 0.69d basis: three bytes carried by a float's mantissa.
fn float_basis(value: f32) -> Result<[f32; 3]> {
    ensure!(
        (0.0..1.0).contains(&value),
        "basis float {value} is outside [0, 1)"
    );
    let packed = (f64::from(value) * 16_777_216.0).round() as u32;
    ensure!(
        packed < 1 << 24,
        "basis float {value} needs more than 24 bits"
    );
    Ok(unpack(packed))
}

fn unpack(packed: u32) -> [f32; 3] {
    let c = |shift: u32| f32::from(((packed >> shift) & 0xff) as u8) / 127.5 - 1.0;
    normalized([c(16), c(8), c(0)])
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > f32::EPSILON {
        v.map(|x| x / l)
    } else {
        v
    }
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa / 1024.0 * 2f32.powi(-14),
        31 => f32::INFINITY,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Reads a 0.69d skinned surface.
pub fn parse_069d(b: &[u8], label: &str) -> Result<Surface> {
    let chunks = chunks(b).with_context(|| label.to_owned())?;
    let ids: Vec<u32> = chunks.iter().map(|(id, _)| *id).collect();
    ensure!(
        ids.first() == Some(&3) && ids.contains(&4) && ids.contains(&5),
        "{label}: chunks {ids:?} are not a 0.69d skinned surface"
    );
    let chunk = |id: u32| {
        chunks
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, body)| *body)
            .with_context(|| format!("{label}: no chunk {id}"))
    };
    let bone_list = chunk(5)?;
    let used = usize::from(*bone_list.first().context("empty bone chunk")?);
    ensure!(
        bone_list.len() == used + 1,
        "{label}: bone chunk lists {used} bones in {} bytes",
        bone_list.len()
    );
    let bones = bone_list[1..].to_vec();
    let vb = chunk(3)?;
    let count = usize::try_from(u32_at(vb, 0)?)?;
    ensure!(count > 0, "{label}: no vertices");
    let stride = (vb.len() - 4) / count;
    ensure!(
        vb.len() == 4 + count * stride && (32..=44).contains(&stride) && stride % 4 == 0,
        "{label}: {} vertex bytes for {count} vertices",
        vb.len() - 4
    );
    let influences = (stride - 32) / 4 + 1;
    let mut vertices = Vec::with_capacity(count);
    for i in 0..count {
        let v = &vb[4 + i * stride..4 + (i + 1) * stride];
        let position = [f32_at(v, 0)?, f32_at(v, 4)?, f32_at(v, 8)?];
        ensure!(
            position.iter().all(|p| p.is_finite()),
            "{label}: vertex {i} is not finite"
        );
        let mut vbones = [v[12], v[13], v[14], v[15]];
        for k in influences..4 {
            vbones[k] = vbones[0];
        }
        let mut weights = [0.0; 4];
        let mut rest = 1.0;
        for k in 0..influences - 1 {
            weights[k] = f32_at(v, 16 + 4 * k)?;
            rest -= weights[k];
        }
        weights[influences - 1] = rest;
        for k in 0..influences {
            ensure!(
                bones.contains(&vbones[k]) || weights[k].abs() < 1e-6,
                "{label}: vertex {i} uses bone {} outside the surface's list {bones:?}",
                vbones[k]
            );
        }
        let at = 16 + 4 * (influences - 1);
        let [normal, tangent, binormal] = [0, 1, 2].map(|k| f32_at(v, at + 4 * k));
        let uv_bits = u32_at(v, at + 12)?;
        vertices.push(Vertex {
            position,
            bones: vbones,
            weights,
            normal: float_basis(normal?)?,
            tangent: float_basis(tangent?)?,
            binormal: float_basis(binormal?)?,
            uv: [half(uv_bits as u16), half((uv_bits >> 16) as u16)],
        });
    }
    let ib = chunk(4)?;
    let n = usize::try_from(u32_at(ib, 0)?)?;
    ensure!(
        ib.len() == 4 + 2 * n && n % 3 == 0,
        "{label}: index chunk of {} bytes for {n} indices",
        ib.len()
    );
    let indices: Vec<u16> = ib[4..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    if let Some(bad) = indices.iter().find(|&&i| usize::from(i) >= count) {
        bail!("{label}: index {bad} past {count} vertices");
    }
    Ok(Surface {
        vertices,
        indices,
        bones,
    })
}

/// A unit vector packed as the 0.26g basis bytes, with `w` in the top byte.
pub fn packed_basis(v: [f32; 3], w: u8) -> u32 {
    let v = normalized(v);
    let c = |x: f32| (((x + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
    (u32::from(w) << 24) | (c(v[0]) << 16) | (c(v[1]) << 8) | c(v[2])
}

/// The fraction of `uv * 1024` the 0.26g tangent and binormal carry.
fn uv_fraction(value: f32) -> u8 {
    let rebased = value * 1024.0;
    let primary = rebased.floor().clamp(-32768.0, 32767.0);
    (255.5 * (rebased - primary)).floor().clamp(0.0, 255.0) as u8
}

/// The surface in the v0.26g four-bone layout (48-byte vertices), the layout
/// `mask_fit` works on.
pub fn to_026_four_bone(surface: &Surface) -> Result<Vec<u8>> {
    let count = surface.vertices.len();
    ensure!(count <= usize::from(u16::MAX), "too many vertices");
    let mut out = Vec::with_capacity(24 + count * 48 + surface.indices.len() * 2);
    let push = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    push(&mut out, 3);
    push(&mut out, u32::try_from(4 + count * 48)?);
    push(&mut out, u32::try_from(count)?);
    for v in &surface.vertices {
        for p in v.position {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&v.bones);
        for w in &v.weights[..3] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        push(&mut out, packed_basis(v.normal, 127));
        push(&mut out, packed_basis(v.tangent, uv_fraction(v.uv[0])));
        push(&mut out, packed_basis(v.binormal, uv_fraction(v.uv[1])));
        for t in v.uv {
            out.extend_from_slice(&t.to_le_bytes());
        }
    }
    push(&mut out, 4);
    push(&mut out, u32::try_from(4 + surface.indices.len() * 2)?);
    push(&mut out, u32::try_from(surface.indices.len())?);
    for i in &surface.indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float_of(v: [f32; 3]) -> f32 {
        (packed_basis(v, 0) as f64 / 16_777_216.0) as f32
    }

    /// One 0.69d surface of `influences`-bone vertices: a triangle.
    fn surface_069d(influences: usize) -> Vec<u8> {
        let mut vb = vec![];
        vb.extend_from_slice(&3u32.to_le_bytes());
        for (i, p) in [[0.0f32, 1.6, 0.1], [0.05, 1.7, 0.1], [-0.05, 1.7, 0.1]]
            .iter()
            .enumerate()
        {
            for c in p {
                vb.extend_from_slice(&c.to_le_bytes());
            }
            let bones = [44u8, 21, 19, 0];
            vb.extend_from_slice(&bones);
            for k in 0..influences - 1 {
                vb.extend_from_slice(&(0.5f32 / (k + 1) as f32).to_le_bytes());
            }
            for v in [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
                vb.extend_from_slice(&float_of(v).to_le_bytes());
            }
            // half2: u = 0.5 (0x3800), v = 0.25 + i/4.
            let v_half: u16 = [0x3400, 0x3800, 0x3a00][i];
            vb.extend_from_slice(&0x3800u16.to_le_bytes());
            vb.extend_from_slice(&v_half.to_le_bytes());
        }
        let mut b = vec![];
        for (id, body) in [
            (3u32, vb),
            (5, {
                let mut l = vec![influences as u8];
                l.extend_from_slice(&[44u8, 21, 19, 0][..influences]);
                l
            }),
            (6, vec![0; 24 * influences]),
            (4, {
                let mut ib = 3u32.to_le_bytes().to_vec();
                for i in [0u16, 1, 2] {
                    ib.extend_from_slice(&i.to_le_bytes());
                }
                ib
            }),
            (110, vec![0; 4]),
            (114, vec![0; 72]),
        ] {
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&(body.len() as u32).to_le_bytes());
            b.extend_from_slice(&body);
        }
        b
    }

    #[test]
    fn reads_every_0_69d_vertex_size() {
        for influences in 1..=4 {
            let s = parse_069d(&surface_069d(influences), "test").unwrap();
            assert_eq!(s.vertices.len(), 3);
            assert_eq!(s.indices, [0, 1, 2]);
            let v = s.vertices[1];
            assert_eq!(v.position, [0.05, 1.7, 0.1]);
            assert_eq!(v.uv, [0.5, 0.5]);
            let total: f32 = v.weights.iter().sum();
            assert!((total - 1.0).abs() < 1e-6, "{:?}", v.weights);
            // Unused slots repeat the first bone.
            for k in influences..4 {
                assert_eq!(v.bones[k], 44);
            }
            for (got, want) in [
                (v.normal, [0.0, 0.0, 1.0]),
                (v.tangent, [1.0, 0.0, 0.0]),
                (v.binormal, [0.0, 1.0, 0.0]),
            ] {
                for k in 0..3 {
                    assert!((got[k] - want[k]).abs() < 0.01, "{got:?} != {want:?}");
                }
            }
        }
    }

    #[test]
    fn writes_the_026g_four_bone_layout() {
        let s = parse_069d(&surface_069d(2), "test").unwrap();
        let b = to_026_four_bone(&s).unwrap();
        // Vertex chunk, three 48-byte vertices, index chunk.
        assert_eq!(u32_at(&b, 0).unwrap(), 3);
        assert_eq!(u32_at(&b, 4).unwrap(), 4 + 3 * 48);
        assert_eq!(u32_at(&b, 12 + 3 * 48).unwrap(), 4);
        let v = &b[12 + 48..12 + 96];
        assert_eq!(f32_at(v, 4).unwrap(), 1.7);
        assert_eq!(&v[12..16], &[44, 21, 44, 44]);
        assert_eq!(f32_at(v, 16).unwrap(), 0.5);
        // The normal faces +z: bytes z, y, x = 254 or 255 (a byte of the
        // 0.69d basis, renormalised), 127, 127, and w = 127.
        let normal = u32_at(v, 28).unwrap();
        assert_eq!(normal >> 8, 0x7f_7f7f);
        assert!(normal & 0xff >= 254, "{normal:08x}");
        assert_eq!(f32_at(v, 40).unwrap(), 0.5);
        assert_eq!(f32_at(v, 44).unwrap(), 0.5);
        // What the porter's readers expect of a converted mask.
        let mesh = crate::mask_fit::parse_mesh(&b, 48, "test").unwrap();
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
        let skins = crate::mask_fit::skins(&b, 48).unwrap();
        let total: f64 = skins[1].iter().map(|(_, w)| w).sum();
        assert!((total - 1.0).abs() < 1e-6);
    }

    #[test]
    fn refuses_what_is_not_a_0_69d_surface() {
        let s = parse_069d(&surface_069d(1), "test").unwrap();
        let native = to_026_four_bone(&s).unwrap();
        assert!(parse_069d(&native, "test").is_err());
        let mut short = surface_069d(1);
        short.truncate(short.len() - 10);
        assert!(parse_069d(&short, "test").is_err());
    }

    #[test]
    fn halves_decode() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0), 0.0);
    }
}
