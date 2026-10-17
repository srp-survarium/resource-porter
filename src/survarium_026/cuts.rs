// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry cuts: open a box-shaped hole in a converted map's static render
//! surfaces and collision (e.g. widen a ceiling hatch so a plank route fits).
//!
//! Every triangle is clipped against the box: the parts outside it are kept
//! (split along the box planes, attributes interpolated linearly), the part
//! inside is dropped. Triangles that miss the box keep their bytes and their
//! vertex indices; split pieces get appended vertices.
use super::*;

/// An axis-aligned box removed from the map's static geometry.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cut {
    pub name: String,
    /// Donor model directory prefix the cut applies to, e.g.
    /// `models/ported/cs/sport_center/`. The converted map's static models
    /// sit at the origin, so the box is in map coordinates.
    pub models: String,
    pub centre: [f32; 3],
    /// Half extents along x, y, z.
    pub half: [f32; 3],
    pub reason: String,
}

impl Cut {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.models.starts_with("models/") && self.models.ends_with('/'),
            "cut {} must name a donor model directory",
            self.name
        );
        ensure!(
            self.centre.iter().chain(&self.half).all(|v| v.is_finite())
                && self.half.iter().all(|v| *v > 0.01 && *v < 10.0),
            "cut {} must be a finite box of 0.02-20 m",
            self.name
        );
        Ok(())
    }
    fn lo(&self) -> [f32; 3] {
        std::array::from_fn(|i| self.centre[i] - self.half[i])
    }
    fn hi(&self) -> [f32; 3] {
        std::array::from_fn(|i| self.centre[i] + self.half[i])
    }
}

/// A clip vertex: position plus linearly interpolated attributes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vert<const N: usize> {
    p: [f32; 3],
    a: [f32; N],
}

fn lerp<const N: usize>(u: &Vert<N>, v: &Vert<N>, t: f32) -> Vert<N> {
    Vert {
        p: std::array::from_fn(|i| u.p[i] + (v.p[i] - u.p[i]) * t),
        a: std::array::from_fn(|i| u.a[i] + (v.a[i] - u.a[i]) * t),
    }
}

/// Split a convex polygon by the plane `p[axis] = value`; returns (below, above).
fn split<const N: usize>(
    poly: &[Vert<N>],
    axis: usize,
    value: f32,
) -> (Vec<Vert<N>>, Vec<Vert<N>>) {
    let (mut below, mut above) = (Vec::new(), Vec::new());
    for i in 0..poly.len() {
        let (u, v) = (&poly[i], &poly[(i + 1) % poly.len()]);
        let (du, dv) = (u.p[axis] - value, v.p[axis] - value);
        if du <= 0.0 {
            below.push(*u);
        }
        if du >= 0.0 {
            above.push(*u);
        }
        if (du < 0.0 && dv > 0.0) || (du > 0.0 && dv < 0.0) {
            let w = lerp(u, v, du / (du - dv));
            below.push(w);
            above.push(w);
        }
    }
    (below, above)
}

fn fan<const N: usize>(poly: &[Vert<N>], out: &mut Vec<[Vert<N>; 3]>) {
    for i in 1..poly.len().saturating_sub(1) {
        let t = [poly[0], poly[i], poly[i + 1]];
        let e1: [f32; 3] = std::array::from_fn(|k| t[1].p[k] - t[0].p[k]);
        let e2: [f32; 3] = std::array::from_fn(|k| t[2].p[k] - t[0].p[k]);
        let c = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        if (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() > 1e-7 {
            out.push(t);
        }
    }
}

/// The parts of a triangle outside `cut`, or `None` when the triangle misses
/// the box entirely (keep it as is).
fn clip<const N: usize>(tri: [Vert<N>; 3], cut: &Cut) -> Option<Vec<[Vert<N>; 3]>> {
    let (lo, hi) = (cut.lo(), cut.hi());
    let misses =
        (0..3).any(|k| tri.iter().all(|v| v.p[k] <= lo[k]) || tri.iter().all(|v| v.p[k] >= hi[k]));
    if misses {
        return None;
    }
    let mut out = Vec::new();
    let mut rest: Vec<Vert<N>> = tri.to_vec();
    for k in 0..3 {
        let (below, above) = split(&rest, k, lo[k]);
        fan(&below, &mut out);
        rest = above;
        if rest.len() < 3 {
            return Some(out);
        }
        let (below, above) = split(&rest, k, hi[k]);
        fan(&above, &mut out);
        rest = below;
        if rest.len() < 3 {
            return Some(out);
        }
    }
    // `rest` lies inside the box: dropped.
    Some(out)
}

/// Clip a triangle list against every cut. `tris` are vertex-index triples into
/// `verts`; returns the new triangle list (indices into `verts`, which grows)
/// and the parent triangle of each output triangle. `None` when no cut touches.
fn cut_mesh<const N: usize>(
    verts: &mut Vec<Vert<N>>,
    tris: &[[usize; 3]],
    cuts: &[Cut],
) -> Option<(Vec<[usize; 3]>, Vec<usize>)> {
    let mut current: Vec<([usize; 3], usize)> =
        tris.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut touched = false;
    for cut in cuts {
        let mut next = Vec::with_capacity(current.len());
        for (t, parent) in current {
            match clip([verts[t[0]], verts[t[1]], verts[t[2]]], cut) {
                None => next.push((t, parent)),
                Some(pieces) => {
                    touched = true;
                    for piece in pieces {
                        let base = verts.len();
                        verts.extend_from_slice(&piece);
                        next.push(([base, base + 1, base + 2], parent));
                    }
                }
            }
        }
        current = next;
    }
    touched.then(|| current.into_iter().unzip())
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated chunk")?
            .try_into()
            .unwrap(),
    ) as usize)
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 31) as i32;
    let m = (h & 1023) as f32;
    sign * if e == 0 {
        m * 2f32.powi(-24)
    } else {
        (1.0 + m / 1024.0) * 2f32.powi(e - 15)
    }
}

/// Counts of a cut static surface: (vertices, indices).
pub type SurfaceCounts = (usize, usize);

/// Cut a converted (0.26 layout) static surface: chunk 3 (count, 28-byte
/// vertices: float3 position, three packed basis vectors, half2 UV), chunk 4
/// (count, u16 indices), then any further chunks unchanged.
pub(super) fn cut_static(bytes: &[u8], cuts: &[Cut]) -> Result<Option<(Vec<u8>, SurfaceCounts)>> {
    ensure!(u32_at(bytes, 0)? == 3, "expected a static vertex chunk");
    let size = u32_at(bytes, 4)?;
    let count = u32_at(bytes, 8)?;
    ensure!(
        count > 0 && size == 4 + 28 * count,
        "expected 28-byte static vertices"
    );
    let end = 8 + size;
    ensure!(
        u32_at(bytes, end)? == 4,
        "expected the index chunk after the vertices"
    );
    let isize_ = u32_at(bytes, end + 4)?;
    let icount = u32_at(bytes, end + 8)?;
    ensure!(
        isize_ == 4 + 2 * icount && icount % 3 == 0,
        "malformed index chunk"
    );
    let tail = &bytes[end + 8 + isize_..];
    let raw = &bytes[12..end];
    let mut verts: Vec<Vert<14>> = raw
        .chunks_exact(28)
        .map(|v| {
            let f = |o: usize| f32::from_le_bytes(v[o..o + 4].try_into().unwrap());
            let mut a = [0f32; 14];
            for (k, b) in v[12..24].iter().enumerate() {
                a[k] = *b as f32;
            }
            a[12] = half_to_f32(u16::from_le_bytes([v[24], v[25]]));
            a[13] = half_to_f32(u16::from_le_bytes([v[26], v[27]]));
            Vert {
                p: [f(0), f(4), f(8)],
                a,
            }
        })
        .collect();
    let idx: Vec<usize> = bytes[end + 12..end + 12 + 2 * icount]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
        .collect();
    ensure!(idx.iter().all(|&i| i < count), "index outside the vertices");
    let tris: Vec<[usize; 3]> = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let original = verts.len();
    let Some((out, _)) = cut_mesh(&mut verts, &tris, cuts) else {
        return Ok(None);
    };
    ensure!(!out.is_empty(), "a cut removes a whole static surface");
    ensure!(
        verts.len() <= super::early::MAX_STATIC_SURFACE_VERTICES,
        "a cut leaves {} vertices on a surface (limit {})",
        verts.len(),
        super::early::MAX_STATIC_SURFACE_VERTICES
    );
    let mut result = Vec::new();
    result.extend(3u32.to_le_bytes());
    result.extend(u32::try_from(4 + 28 * verts.len())?.to_le_bytes());
    result.extend(u32::try_from(verts.len())?.to_le_bytes());
    result.extend(raw);
    for v in &verts[original..] {
        for c in v.p {
            result.extend(c.to_le_bytes());
        }
        for k in 0..12 {
            result.push(v.a[k].round().clamp(0.0, 255.0) as u8);
        }
        for k in 12..14 {
            result.extend(super::early::half(v.a[k])?.to_le_bytes());
        }
    }
    result.extend(4u32.to_le_bytes());
    result.extend(u32::try_from(4 + 2 * out.len() * 3)?.to_le_bytes());
    result.extend(u32::try_from(out.len() * 3)?.to_le_bytes());
    for t in &out {
        for &i in t {
            result.extend(u16::try_from(i)?.to_le_bytes());
        }
    }
    result.extend(tail);
    Ok(Some((result, (verts.len(), out.len() * 3))))
}

/// Cut a model's collision triple: vertices (chunk 0x19, float3), indices
/// (chunk 0x1a, u32 triples) and face_data (0x1b dictionary, 0x1c u16 per
/// triangle). Split triangles keep their parent's face material.
pub(super) fn cut_collision(
    vertices: &[u8],
    indices: &[u8],
    face_data: &[u8],
    cuts: &[Cut],
) -> Result<Option<(Vec<u8>, Vec<u8>, Vec<u8>)>> {
    ensure!(
        u32_at(vertices, 0)? == 0x19 && u32_at(indices, 0)? == 0x1a,
        "unexpected collision chunks"
    );
    let vsize = u32_at(vertices, 4)?;
    let isize_ = u32_at(indices, 4)?;
    ensure!(
        vertices.len() == 8 + vsize && vsize % 12 == 0,
        "malformed collision vertices"
    );
    ensure!(
        indices.len() == 8 + isize_ && isize_ % 12 == 0,
        "malformed collision indices"
    );
    ensure!(
        u32_at(face_data, 0)? == 0x1b,
        "collision face data has no dictionary"
    );
    let dict = u32_at(face_data, 4)?;
    let fo = 8 + dict;
    ensure!(
        u32_at(face_data, fo)? == 0x1c,
        "collision face data has no face chunk"
    );
    let fsize = u32_at(face_data, fo + 4)?;
    ensure!(
        face_data.len() == fo + 8 + fsize && fsize == isize_ / 12 * 2,
        "face data does not match the triangles"
    );
    let mut verts: Vec<Vert<0>> = vertices[8..]
        .chunks_exact(12)
        .map(|v| Vert {
            p: std::array::from_fn(|k| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap())),
            a: [],
        })
        .collect();
    let n = verts.len();
    let tris: Vec<[usize; 3]> = indices[8..]
        .chunks_exact(12)
        .map(|t| {
            std::array::from_fn(|k| {
                u32::from_le_bytes(t[k * 4..k * 4 + 4].try_into().unwrap()) as usize
            })
        })
        .collect();
    ensure!(
        tris.iter().flatten().all(|&i| i < n),
        "collision index outside the vertices"
    );
    let faces: Vec<[u8; 2]> = face_data[fo + 8..]
        .chunks_exact(2)
        .map(|b| [b[0], b[1]])
        .collect();
    let Some((out, parents)) = cut_mesh(&mut verts, &tris, cuts) else {
        return Ok(None);
    };
    let mut v = Vec::new();
    v.extend(0x19u32.to_le_bytes());
    v.extend(u32::try_from(12 * verts.len())?.to_le_bytes());
    for p in &verts {
        for c in p.p {
            v.extend(c.to_le_bytes());
        }
    }
    let mut i = Vec::new();
    i.extend(0x1au32.to_le_bytes());
    i.extend(u32::try_from(12 * out.len())?.to_le_bytes());
    for t in &out {
        for &k in t {
            i.extend(u32::try_from(k)?.to_le_bytes());
        }
    }
    let mut f = face_data[..fo].to_vec();
    f.extend(0x1cu32.to_le_bytes());
    f.extend(u32::try_from(2 * out.len())?.to_le_bytes());
    for &p in &parents {
        f.extend(faces[p]);
    }
    Ok(Some((v, i, f)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut() -> Cut {
        Cut {
            name: "hole".into(),
            models: String::new(),
            centre: [0.5, 0.0, 0.5],
            half: [0.25, 1.0, 0.25],
            reason: String::new(),
        }
    }

    fn area<const N: usize>(t: &[Vert<N>; 3]) -> f32 {
        let e1: [f32; 3] = std::array::from_fn(|k| t[1].p[k] - t[0].p[k]);
        let e2: [f32; 3] = std::array::from_fn(|k| t[2].p[k] - t[0].p[k]);
        let c = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
    }

    #[test]
    fn clipping_removes_exactly_the_box_area() {
        // A 1 x 1 floor quad as two triangles; the cut is a 0.5 x 0.5 square.
        let v = |x: f32, z: f32| Vert::<1> {
            p: [x, 0.0, z],
            a: [x + z],
        };
        let quad = [
            [v(0., 0.), v(1., 0.), v(1., 1.)],
            [v(0., 0.), v(1., 1.), v(0., 1.)],
        ];
        let mut total = 0.0;
        for t in quad {
            let pieces = clip(t, &cut()).unwrap();
            for p in &pieces {
                total += area(p);
                // Attributes stay the linear function they were.
                for q in p {
                    assert!((q.a[0] - (q.p[0] + q.p[2])).abs() < 1e-5);
                    // No piece lies strictly inside the hole.
                }
                let c = [0, 1, 2].map(|k| (p[0].p[k] + p[1].p[k] + p[2].p[k]) / 3.0);
                assert!(!(c[0] > 0.25 && c[0] < 0.75 && c[2] > 0.25 && c[2] < 0.75));
            }
        }
        assert!((total - 0.75).abs() < 1e-5, "{total}");
        // A triangle away from the box is untouched.
        assert!(clip([v(2., 2.), v(3., 2.), v(3., 3.)], &cut()).is_none());
    }

    #[test]
    fn collision_cut_keeps_face_materials_and_counts() {
        let mut vertices = vec![];
        vertices.extend(0x19u32.to_le_bytes());
        vertices.extend(48u32.to_le_bytes());
        for p in [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]] {
            for c in p {
                vertices.extend((c as f32).to_le_bytes());
            }
        }
        let mut indices = vec![];
        indices.extend(0x1au32.to_le_bytes());
        indices.extend(24u32.to_le_bytes());
        for i in [0u32, 1, 2, 0, 2, 3] {
            indices.extend(i.to_le_bytes());
        }
        let mut face = vec![];
        face.extend(0x1bu32.to_le_bytes());
        face.extend(4u32.to_le_bytes());
        face.extend(1u16.to_le_bytes());
        face.extend(b"a\0");
        face.extend(0x1cu32.to_le_bytes());
        face.extend(4u32.to_le_bytes());
        face.extend([0u8, 0, 0, 0]);
        let (v, i, f) = cut_collision(&vertices, &indices, &face, &[cut()])
            .unwrap()
            .unwrap();
        let tris = (i.len() - 8) / 12;
        assert!(tris > 2);
        assert_eq!(f.len(), 12 + 8 + 2 * tris);
        assert_eq!((v.len() - 8) % 12, 0);
        assert!(
            cut_collision(
                &vertices,
                &indices,
                &face,
                &[Cut {
                    centre: [5.0, 0.0, 5.0],
                    ..cut()
                }]
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn static_cut_rewrites_counts_and_keeps_the_layout() {
        let mut b = vec![];
        let verts = [[0f32, 0., 0.], [1., 0., 0.], [1., 0., 1.]];
        b.extend(3u32.to_le_bytes());
        b.extend((4 + 28 * 3u32).to_le_bytes());
        b.extend(3u32.to_le_bytes());
        for p in verts {
            for c in p {
                b.extend(c.to_le_bytes());
            }
            b.extend([128u8, 255, 128, 255, 255, 128, 128, 255, 128, 128, 255, 255]);
            b.extend([0u8, 0, 0, 0]);
        }
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        for i in [0u16, 1, 2] {
            b.extend(i.to_le_bytes());
        }
        let (out, (nv, ni)) = cut_static(&b, &[cut()]).unwrap().unwrap();
        assert_eq!(u32_at(&out, 8).unwrap(), nv);
        let end = 8 + u32_at(&out, 4).unwrap();
        assert_eq!(u32_at(&out, end + 8).unwrap(), ni);
        assert_eq!(&out[12..12 + 84], &b[12..12 + 84]);
        assert!(
            cut_static(
                &b,
                &[Cut {
                    centre: [9.0, 0.0, 9.0],
                    ..cut()
                }]
            )
            .unwrap()
            .is_none()
        );
    }
}
