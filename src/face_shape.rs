// SPDX-License-Identifier: GPL-3.0-or-later
//! What a player face is drawn with under the game's items, made without
//! moving any skin that is seen.
//!
//! - **The neck's seam** ([`join_native_neck`]). Every body of the game was
//!   made for the native head's neck: its collars stand round that neck and
//!   its neck hole ends on that neck's open edge. A face's neck is cut along
//!   that edge and, below the jaw, laid onto the native neck: only skin that
//!   bends with the neck moves (skinned to Head by less than
//!   [`NECK_HEAD_WEIGHT`]), the more the less it follows the Head; everything
//!   that follows the Head (the face, the ears, the teeth, eyes and mouth,
//!   the skull) stays as its donor made it, bit for bit, which the function
//!   checks.
//! - **Covered skin** ([`cut_covered`]). Skin no one can see under any item
//!   of a state (every helmet, or every mask) is not drawn in that state's
//!   variant.
//! - **An item for a face** ([`carry_item`]). An item made over the native
//!   head, carried onto a face: each vertex keeps its distance from the skin
//!   under it.
use crate::face_fit::Rig;
use crate::mask_fit::{
    Mesh, Skin, closest_on_triangle, dot, farthest_hit, parse_mesh, put_skin, read_u32, skins, sub,
};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, HashSet};

/// Skin skinned to Head by less than this bends with the neck (the neck,
/// the nape and the throat under the jaw): it is laid onto the native neck,
/// wholly where it follows the Head by [`NECK_WHOLLY`] or less.
pub const NECK_HEAD_WEIGHT: f64 = 0.9;
pub const NECK_WHOLLY: f64 = 0.5;
/// Within this of the native neck's open edge (m) a face's neck is on it.
const SEAM_SNAP: f64 = 0.002;
/// Below this height (m) an open edge of the native head is its neck's.
const NECK_BELOW: f64 = 1.56;
/// A ray's hit on the native neck farther than this from the vertex (m) is
/// not the skin under it (the other side of the neck): the closest skin is.
const RAY_TRUST: f64 = 0.03;

/// A 48-byte converted surface as vertices and triangles.
#[derive(Clone)]
pub(crate) struct Soup {
    pub vertices: Vec<[u8; 48]>,
    pub triangles: Vec<[u16; 3]>,
    /// Any chunk after the indices.
    tail: Vec<u8>,
}

fn f32_at(v: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(v[at..at + 4].try_into().unwrap()))
}

pub(crate) fn position(v: &[u8; 48]) -> [f64; 3] {
    [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]
}

pub(crate) fn set_position(v: &mut [u8; 48], p: [f64; 3]) {
    for k in 0..3 {
        v[4 * k..4 * k + 4].copy_from_slice(&(p[k] as f32).to_le_bytes());
    }
}

/// A 48-byte vertex's skin.
pub(crate) fn skin_of(v: &[u8; 48]) -> Skin {
    let w = [f32_at(v, 16), f32_at(v, 20), f32_at(v, 24)];
    let weights = [w[0], w[1], w[2], 1.0 - w[0] - w[1] - w[2]];
    let mut skin: BTreeMap<u8, f64> = BTreeMap::new();
    for (&bone, weight) in v[12..16].iter().zip(weights) {
        if weight > 1e-6 {
            *skin.entry(bone).or_default() += weight;
        }
    }
    skin.into_iter().collect()
}

fn mix_skins(a: &Skin, b: &Skin, t: f64) -> Skin {
    let mut mixed: BTreeMap<u8, f64> = BTreeMap::new();
    for &(bone, w) in a {
        *mixed.entry(bone).or_default() += w * (1.0 - t);
    }
    for &(bone, w) in b {
        *mixed.entry(bone).or_default() += w * t;
    }
    mixed.into_iter().filter(|(_, w)| *w > 1e-6).collect()
}

impl Soup {
    pub fn parse(model: &[u8], label: &str) -> Result<Self> {
        let mesh = parse_mesh(model, 48, label)?;
        let n = mesh.positions.len();
        let vertices = (0..n)
            .map(|i| model[12 + i * 48..12 + (i + 1) * 48].try_into().unwrap())
            .collect();
        let triangles = mesh.triangles.iter().map(|t| t.map(|v| v as u16)).collect();
        let index_chunk = 12 + n * 48;
        let rest = index_chunk + 8 + read_u32(model, index_chunk + 4)?;
        Ok(Self {
            vertices,
            triangles,
            tail: model.get(rest..).context("truncated surface")?.to_vec(),
        })
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        ensure!(
            self.vertices.len() <= usize::from(u16::MAX),
            "a surface of {} vertices",
            self.vertices.len()
        );
        let mut out = vec![];
        out.extend_from_slice(&3_u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(4 + self.vertices.len() * 48)?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.vertices.len())?.to_le_bytes());
        for v in &self.vertices {
            out.extend_from_slice(v);
        }
        out.extend_from_slice(&4_u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(4 + self.triangles.len() * 6)?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.triangles.len() * 3)?.to_le_bytes());
        for i in self.triangles.iter().flatten() {
            out.extend_from_slice(&i.to_le_bytes());
        }
        out.extend_from_slice(&self.tail);
        Ok(out)
    }

    /// Only the triangles `keep` accepts (by index) and the vertices they
    /// use, in their order.
    pub fn keep(&self, keep: impl Fn(usize) -> bool) -> Result<Self> {
        let mut remap = vec![u16::MAX; self.vertices.len()];
        let mut out = Self {
            vertices: vec![],
            triangles: vec![],
            tail: self.tail.clone(),
        };
        for (k, t) in self.triangles.iter().enumerate() {
            if !keep(k) {
                continue;
            }
            let mut corners = [0_u16; 3];
            for (c, &v) in corners.iter_mut().zip(t) {
                let at = &mut remap[usize::from(v)];
                if *at == u16::MAX {
                    *at = u16::try_from(out.vertices.len())?;
                    out.vertices.push(self.vertices[usize::from(v)]);
                }
                *c = *at;
            }
            out.triangles.push(corners);
        }
        Ok(out)
    }

    /// The part where `above` (a value per vertex) is positive: triangles
    /// across zero are clipped on it, a new vertex there taking its inside
    /// corner's basis and both corners' position, UV and skin in proportion.
    /// Returns the part and how many triangles were dropped or clipped.
    pub fn clip(&self, above: &[f64]) -> Result<(Self, usize)> {
        let mut out = Self {
            vertices: vec![],
            triangles: vec![],
            tail: self.tail.clone(),
        };
        let mut index: BTreeMap<(u16, u16), u16> = BTreeMap::new();
        let mut touched = 0;
        let mut add = |out: &mut Self, a: u16, b: u16| -> Result<u16> {
            if let Some(&i) = index.get(&(a, b)) {
                return Ok(i);
            }
            let (va, vb) = (
                &self.vertices[usize::from(a)],
                &self.vertices[usize::from(b)],
            );
            let mut v = *va;
            if a != b {
                let t = above[usize::from(a)] / (above[usize::from(a)] - above[usize::from(b)]);
                for at in [0, 4, 8, 40, 44] {
                    let x = f32_at(va, at) + t * (f32_at(vb, at) - f32_at(va, at));
                    v[at..at + 4].copy_from_slice(&(x as f32).to_le_bytes());
                }
                // The tangent's and binormal's top bytes carry the UV's
                // fraction of a 1024th.
                v[35] = crate::v069d_skinned::uv_fraction(f32_at(&v, 40) as f32);
                v[39] = crate::v069d_skinned::uv_fraction(f32_at(&v, 44) as f32);
                put_skin(&mut v, &mix_skins(&skin_of(va), &skin_of(vb), t));
            }
            let i = u16::try_from(out.vertices.len())?;
            out.vertices.push(v);
            index.insert((a, b), i);
            Ok(i)
        };
        for t in &self.triangles {
            let inside = t.map(|v| above[usize::from(v)] > 0.0);
            if inside != [true; 3] {
                touched += 1;
            }
            let mut polygon = vec![];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if inside[k] {
                    polygon.push(add(&mut out, a, a)?);
                }
                if inside[k] != inside[(k + 1) % 3] {
                    let (i, o) = if inside[k] { (a, b) } else { (b, a) };
                    polygon.push(add(&mut out, i, o)?);
                }
            }
            for k in 1..polygon.len().saturating_sub(1) {
                out.triangles.push([polygon[0], polygon[k], polygon[k + 1]]);
            }
        }
        Ok((out, touched))
    }
}

/// The native neck's open edge: the loop the bodies' neck holes end on.
pub(crate) struct Seam {
    segments: Vec<[[f64; 3]; 2]>,
    /// (angle round the neck's axis in radians, height), by angle.
    heights: Vec<(f64, f64)>,
}

impl Seam {
    /// The axis the angle is taken round (x, z), behind the throat.
    const AXIS: [f64; 2] = [0.0, -0.04];

    pub fn new(native: &Mesh) -> Result<Self> {
        let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
        type Edge = (usize, [f64; 3], [f64; 3]);
        let mut edges: BTreeMap<([i64; 3], [i64; 3]), Edge> = BTreeMap::new();
        for t in &native.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (pa, pb) = (native.positions[a], native.positions[b]);
                let k = if key(pa) < key(pb) {
                    (key(pa), key(pb))
                } else {
                    (key(pb), key(pa))
                };
                edges.entry(k).or_insert((0, pa, pb)).0 += 1;
            }
        }
        let segments: Vec<[[f64; 3]; 2]> = edges
            .values()
            .filter(|(n, a, b)| *n == 1 && a[1] < NECK_BELOW && b[1] < NECK_BELOW)
            .map(|(_, a, b)| [*a, *b])
            .collect();
        ensure!(segments.len() >= 8, "the native neck has no open edge");
        let mut heights: Vec<(f64, f64)> = segments
            .iter()
            .flatten()
            .map(|p| (Self::angle(*p), p[1]))
            .collect();
        heights.sort_by(|a, b| a.0.total_cmp(&b.0));
        heights.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        Ok(Self { segments, heights })
    }

    fn angle(p: [f64; 3]) -> f64 {
        (p[0] - Self::AXIS[0]).atan2(p[2] - Self::AXIS[1])
    }

    /// The edge's height at `p`'s angle round the neck.
    pub fn height_at(&self, p: [f64; 3]) -> f64 {
        let a = Self::angle(p);
        let n = self.heights.len();
        let tau = std::f64::consts::TAU;
        let after = self.heights.iter().position(|q| q.0 >= a).unwrap_or(n);
        let (lo, hi) = (self.heights[(after + n - 1) % n], self.heights[after % n]);
        let span = (hi.0 - lo.0).rem_euclid(tau).max(1e-9);
        let t = ((a - lo.0).rem_euclid(tau) / span).clamp(0.0, 1.0);
        lo.1 + t * (hi.1 - lo.1)
    }

    /// The point of the edge closest to `p`.
    pub fn closest(&self, p: [f64; 3]) -> [f64; 3] {
        let mut best = (f64::INFINITY, p);
        for [a, b] in &self.segments {
            let ab = sub(*b, *a);
            let t = (dot(sub(p, *a), ab) / dot(ab, ab).max(1e-18)).clamp(0.0, 1.0);
            let q = [0, 1, 2].map(|k| a[k] + t * ab[k]);
            let d = sub(p, q);
            let d = dot(d, d);
            if d < best.0 {
                best = (d, q);
            }
        }
        best.1
    }

    /// The lowest point of the edge (m).
    pub fn bottom(&self) -> f64 {
        self.heights
            .iter()
            .map(|p| p.1)
            .fold(f64::INFINITY, f64::min)
    }
}

/// What [`join_native_neck`] did to one surface.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct NeckJoin {
    /// Triangles below the native neck's edge dropped or clipped on it.
    pub cut: usize,
    /// How far below the edge the donor's neck reached (m).
    pub below: f64,
    /// Vertices laid onto the native neck, wholly or in part, and the
    /// farthest one moved (m).
    pub moved: usize,
    pub farthest: f64,
    /// The highest vertex that moved (m), and the most it follows the Head.
    pub highest_moved: f64,
    /// Vertices that follow the Head by [`NECK_HEAD_WEIGHT`] or more: each
    /// one checked to be the donor's, bit for bit.
    pub kept: usize,
}

fn head_weight(skin: &Skin, head: u8) -> f64 {
    skin.iter()
        .filter(|(b, _)| *b == head)
        .map(|(_, w)| w)
        .sum()
}

/// How much of the way onto the native neck a vertex goes, by how much it
/// follows the Head: none from [`NECK_HEAD_WEIGHT`] up, all from
/// [`NECK_WHOLLY`] down.
fn neck_share(head_weight: f64) -> f64 {
    let x = ((NECK_HEAD_WEIGHT - head_weight) / (NECK_HEAD_WEIGHT - NECK_WHOLLY)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The native head's skin at the point of its surface under (or closest
/// to) `p`: along the horizontal ray from the neck's axis where that meets
/// the neck near `p`, else the closest point. Returns the point and its skin.
fn native_under(p: [f64; 3], rig: &Rig, native: &Mesh, native_skins: &[Skin]) -> ([f64; 3], Skin) {
    let centre = rig.neck_axis(p);
    let d = sub(p, centre);
    let r = dot(d, d).sqrt();
    let on_ray = (r > 1e-9)
        .then(|| {
            farthest_hit(
                centre,
                d.map(|v| v / r),
                &native.positions,
                &native.triangles,
            )
        })
        .flatten()
        .filter(|(t, _, _)| (t - r).abs() <= RAY_TRUST)
        .map(|(_, tri, bary)| (tri, bary));
    let (tri, bary) = on_ray.unwrap_or_else(|| {
        let mut best = (f64::INFINITY, 0, [1.0, 0.0, 0.0]);
        for (t, tri) in native.triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|v| native.positions[v]);
            let bary = closest_on_triangle(p, a, b, c);
            let q = [0, 1, 2].map(|k| bary[0] * a[k] + bary[1] * b[k] + bary[2] * c[k]);
            let d = sub(p, q);
            let d = dot(d, d);
            if d < best.0 {
                best = (d, t, bary);
            }
        }
        (best.1, best.2)
    });
    let corners = native.triangles[tri];
    let at = [0, 1, 2].map(|k| {
        (0..3)
            .map(|c| bary[c] * native.positions[corners[c]][k])
            .sum::<f64>()
    });
    let mut skin: BTreeMap<u8, f64> = BTreeMap::new();
    for (corner, b) in corners.iter().zip(bary) {
        for &(bone, w) in &native_skins[*corner] {
            *skin.entry(bone).or_default() += w * b;
        }
    }
    (at, skin.into_iter().filter(|(_, w)| *w > 1e-6).collect())
}

/// A face's skin surface joined to the body where the native head is: cut
/// along the native neck's open edge, its last ring on that edge, and its
/// neck below the jaw laid onto the native neck with the native neck's
/// skin, each vertex by [`neck_share`] of how little it follows the Head.
/// A vertex that follows the Head by [`NECK_HEAD_WEIGHT`] or more is not
/// touched: the function fails if one differs from the donor's in any bit.
pub(crate) fn join_native_neck(
    face: &[u8],
    native_head: &[u8],
    rig: &Rig,
    label: &str,
) -> Result<(Vec<u8>, NeckJoin)> {
    let donor = Soup::parse(face, label)?;
    let native = parse_mesh(native_head, 40, "native head")?;
    let native_skins = skins(native_head, 40)?;
    let seam = Seam::new(&native)?;
    let head = rig.head_bone();
    let above: Vec<f64> = donor
        .vertices
        .iter()
        .map(|v| {
            let p = position(v);
            p[1] - seam.height_at(p)
        })
        .collect();
    let below = above.iter().copied().fold(0.0, f64::min).abs();
    let (mut soup, cut) = donor.clip(&above)?;
    ensure!(
        !soup.triangles.is_empty(),
        "{label}: nothing above the native neck's edge"
    );
    let mut join = NeckJoin {
        cut,
        below,
        ..NeckJoin::default()
    };
    let donors: HashSet<[u8; 48]> = donor.vertices.iter().copied().collect();
    for v in &mut soup.vertices {
        let own = skin_of(v);
        let follows = head_weight(&own, head);
        if follows >= NECK_HEAD_WEIGHT {
            ensure!(
                donors.contains(v),
                "{label}: a vertex that follows the Head is not the donor's"
            );
            join.kept += 1;
            continue;
        }
        let p = position(v);
        let on_seam = p[1] - seam.height_at(p) < SEAM_SNAP;
        let share = if on_seam { 1.0 } else { neck_share(follows) };
        let (mut target, skin) = native_under(p, rig, &native, &native_skins);
        if on_seam {
            target = seam.closest(p);
        }
        let q = [0, 1, 2].map(|k| p[k] + share * (target[k] - p[k]));
        let moved = sub(q, p);
        let moved = dot(moved, moved).sqrt();
        set_position(v, q);
        put_skin(v, &mix_skins(&own, &skin, share));
        if moved > 1e-6 {
            join.moved += 1;
            join.farthest = join.farthest.max(moved);
            join.highest_moved = join.highest_moved.max(p[1]);
        }
    }
    Ok((soup.bytes()?, join))
}

/// The triangles of a surface with a corner in sight: a triangle whose
/// three corners are `covered` is left out. Returns the surface and how many
/// triangles went; `None` when nothing is covered.
pub(crate) fn cut_covered(
    model: &[u8],
    covered: &[bool],
    label: &str,
) -> Result<Option<(Vec<u8>, usize)>> {
    let soup = Soup::parse(model, label)?;
    ensure!(
        covered.len() == soup.vertices.len(),
        "{label}: sight of another surface"
    );
    let gone = |k: usize| soup.triangles[k].iter().all(|&v| covered[usize::from(v)]);
    let cut = (0..soup.triangles.len()).filter(|&k| gone(k)).count();
    if cut == 0 {
        return Ok(None);
    }
    ensure!(cut < soup.triangles.len(), "{label}: wholly covered");
    Ok(Some((soup.keep(|k| !gone(k))?.bytes()?, cut)))
}

/// How far along the native head's normal the face's skin is looked for,
/// each way (m): more is not the same place of the head.
const CARRY_REACH: f64 = 0.025;
/// An item's vertex this far inside the native head (m) is under the item's
/// outer layer, and moves with it.
const INSIDE_DEPTH: f64 = 0.001;
/// Rounds of smoothing of an item's carry: its shape keeps its small
/// features, the carry is the face's broad difference from the native head.
pub const CARRY_SMOOTH: usize = 6;

/// How an item was carried onto a face.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Carried {
    /// Vertices moved by more than 0.1 mm, the farthest and the mean (m).
    pub moved: usize,
    pub farthest: f64,
    pub mean: f64,
    /// Vertices inside the native head, or whose line met no skin of the
    /// face within the reach: they take their neighbours' carry.
    pub unmet: usize,
}

/// An item made over the native head carried onto a face. Each vertex is
/// moved along the native head's normal at the point of it nearest the
/// vertex, by how far out (or in) along that normal the face's skin nearest
/// that point is: it keeps the native head's place under it (so an
/// eye hole stays on the eyes, an edge on the same part of the head) and the
/// distance from the skin its maker gave it.
pub(crate) struct Carry {
    native: Mesh,
    /// The native head's vertex normals, welded copies together.
    normals: Vec<[f64; 3]>,
    face: crate::face_fit::Solid,
}

fn unit3(v: [f64; 3]) -> [f64; 3] {
    let l = dot(v, v).sqrt();
    if l < 1e-12 {
        [0.0; 3]
    } else {
        v.map(|x| x / l)
    }
}

impl Carry {
    pub fn new(native: &Mesh, face: &Mesh) -> Self {
        let (weld, _) = crate::mask_fit::weld_vertices(native);
        let mut welded = vec![[0.0; 3]; weld.iter().copied().max().map_or(0, |m| m + 1)];
        for t in &native.triangles {
            let [a, b, c] = t.map(|v| native.positions[v]);
            let n = crate::mask_fit::cross(sub(b, a), sub(c, a));
            for &v in t {
                let w = &mut welded[weld[v]];
                for k in 0..3 {
                    w[k] += n[k];
                }
            }
        }
        let normals = weld.iter().map(|&k| unit3(welded[k])).collect();
        Self {
            native: native.clone(),
            normals,
            face: crate::face_fit::Solid::new(&face.positions, &face.triangles),
        }
    }

    /// The native head's point nearest `p` and its normal there.
    fn nearest(&self, p: [f64; 3]) -> ([f64; 3], [f64; 3]) {
        let mut best = (f64::INFINITY, 0, [1.0, 0.0, 0.0]);
        for (t, tri) in self.native.triangles.iter().enumerate() {
            let [a, b, c] = tri.map(|v| self.native.positions[v]);
            let bary = closest_on_triangle(p, a, b, c);
            let q = [0, 1, 2].map(|k| bary[0] * a[k] + bary[1] * b[k] + bary[2] * c[k]);
            let d = sub(p, q);
            let d = dot(d, d);
            if d < best.0 {
                best = (d, t, bary);
            }
        }
        let tri = self.native.triangles[best.1];
        let at = |k: usize| {
            (0..3)
                .map(|c| best.2[c] * self.native.positions[tri[c]][k])
                .sum::<f64>()
        };
        let n = unit3([0, 1, 2].map(|k| {
            (0..3)
                .map(|c| best.2[c] * self.normals[tri[c]][k])
                .sum::<f64>()
        }));
        ([at(0), at(1), at(2)], n)
    }

    /// How a vertex at `p` moves: along the native normal under it, to the
    /// face's skin nearest the native head's on that line, within
    /// [`CARRY_REACH`].
    fn at(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        let (q, n) = self.nearest(p);
        if dot(n, n) < 0.5 {
            return None;
        }
        let from = [0, 1, 2].map(|k| q[k] - n[k] * CARRY_REACH);
        let t = self
            .face
            .nearest_facing(from, n, 2.0 * CARRY_REACH, CARRY_REACH)?
            - CARRY_REACH;
        Some(n.map(|v| v * t))
    }

    /// A converted surface of any vertex layout with its positions carried.
    /// A vertex whose line meets no skin takes its neighbours' carry. The
    /// carry is smoothed `smooth` rounds over the surface (seam copies
    /// together), but on its open edges, which keep their own: an edge stays
    /// on the skin as its maker laid it.
    pub fn apply(&self, model: &[u8], smooth: usize, label: &str) -> Result<(Vec<u8>, Carried)> {
        let count = read_u32(model, 8)?;
        ensure!(count > 0, "{label}: no vertices");
        let stride = (read_u32(model, 4)? - 4) / count;
        let mesh = parse_mesh(model, stride, label)?;
        let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
        let n = neighbours.len();
        // What lies inside the native head (a mask's rolled insides, under
        // its outer layer) moves as the rest of the item round it does: it
        // stays under the layer that hid it.
        let inside = crate::mask_fit::interior_vertices(
            &mesh.positions,
            &self.native,
            INSIDE_DEPTH,
            f64::NEG_INFINITY,
        );
        let mut by: Vec<Option<[f64; 3]>> = vec![None; n];
        let mut seen = vec![false; n];
        for (i, &k) in weld.iter().enumerate() {
            if !seen[k] {
                seen[k] = true;
                by[k] = if inside[i] {
                    None
                } else {
                    self.at(mesh.positions[i])
                };
            }
        }
        let mut how = Carried {
            unmet: by.iter().filter(|b| b.is_none()).count(),
            ..Carried::default()
        };
        loop {
            let mut filled = vec![];
            for k in 0..n {
                if by[k].is_some() {
                    continue;
                }
                let known: Vec<[f64; 3]> = neighbours[k].iter().filter_map(|&u| by[u]).collect();
                if !known.is_empty() {
                    let m = known.len() as f64;
                    filled.push((
                        k,
                        [0, 1, 2].map(|j| known.iter().map(|d| d[j]).sum::<f64>() / m),
                    ));
                }
            }
            if filled.is_empty() {
                break;
            }
            for (k, d) in filled {
                by[k] = Some(d);
            }
        }
        let mut by: Vec<[f64; 3]> = by.into_iter().map(|d| d.unwrap_or([0.0; 3])).collect();
        // Open edges: an edge of the welded mesh that one triangle uses.
        let mut uses: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for t in &mesh.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (weld[a], weld[b]);
                *uses.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut edge = vec![false; n];
        for ((a, b), u) in uses {
            if u == 1 {
                edge[a] = true;
                edge[b] = true;
            }
        }
        for _ in 0..smooth {
            by = (0..n)
                .map(|k| {
                    if edge[k] || neighbours[k].is_empty() {
                        return by[k];
                    }
                    let m = neighbours[k].len() as f64;
                    [0, 1, 2].map(|j| {
                        let mean = neighbours[k].iter().map(|&u| by[u][j]).sum::<f64>() / m;
                        (by[k][j] + mean) / 2.0
                    })
                })
                .collect();
        }
        let mut out = model.to_vec();
        let mut total = 0.0;
        for (i, &k) in weld.iter().enumerate() {
            let l = dot(by[k], by[k]).sqrt();
            total += l;
            if l > 1e-4 {
                how.moved += 1;
            }
            how.farthest = how.farthest.max(l);
            let at = 12 + i * stride;
            for j in 0..3 {
                let v = (mesh.positions[i][j] + by[k][j]) as f32;
                out[at + 4 * j..at + 4 * j + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        how.mean = total / weld.len().max(1) as f64;
        Ok((out, how))
    }
}

/// How far each vertex of a variant is from the vertex of the bare face
/// with its UV (m): what its maker moved. A vertex whose UV the bare face
/// has not is left out (`None`).
pub(crate) fn moved_from(bare: &[u8], variant: &[u8]) -> Result<Vec<Option<f64>>> {
    let (bare, variant) = (
        Soup::parse(bare, "bare face")?,
        Soup::parse(variant, "variant")?,
    );
    let uv = |v: &[u8; 48]| {
        [
            (f32_at(v, 40) * 1e5).round() as i64,
            (f32_at(v, 44) * 1e5).round() as i64,
        ]
    };
    let mut by_uv: BTreeMap<[i64; 2], Vec<[f64; 3]>> = BTreeMap::new();
    for v in &bare.vertices {
        by_uv.entry(uv(v)).or_default().push(position(v));
    }
    Ok(variant
        .vertices
        .iter()
        .map(|v| {
            let p = position(v);
            by_uv.get(&uv(v)).map(|at| {
                at.iter()
                    .map(|q| {
                        let d = sub(p, *q);
                        dot(d, d).sqrt()
                    })
                    .fold(f64::INFINITY, f64::min)
            })
        })
        .collect())
}

/// What [`onto_live_neck`] did.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct NeckCarry {
    /// Vertices that bend with the neck, moved wholly or in part onto the
    /// game's model, the farthest (m), and those of them with no vertex of
    /// that model at their UV (moved with their neighbours).
    pub moved: usize,
    pub farthest: f64,
    pub unmatched: usize,
}

/// A 0.69d0 item made for one face (`donor`, converted) with the part of it
/// that bends with the neck laid where the game's own model of it (`live`,
/// made over the native head and tucked into the game's collars) has it: as
/// a face's neck is joined to the body ([`join_native_neck`]), each vertex by
/// [`neck_share`] of how little it follows the Head, its skin mixed in the
/// same share. A vertex that follows the Head by [`NECK_HEAD_WEIGHT`] or more
/// is the donor's, bit for bit. The two models' vertices are matched by UV
/// (the game's model is the donor's of another face, refined and fitted).
pub(crate) fn onto_live_neck(
    donor: &[u8],
    live: &[u8],
    rig: &Rig,
    label: &str,
) -> Result<(Vec<u8>, NeckCarry)> {
    let mut soup = Soup::parse(donor, label)?;
    let game = Soup::parse(live, &format!("{label} (the game's)"))?;
    let head = rig.head_bone();
    let uv = |v: &[u8; 48]| [f32_at(v, 40), f32_at(v, 44)];
    let key = |u: [f64; 2]| u.map(|x| (x * 1000.0).floor() as i64);
    let mut by_uv: BTreeMap<[i64; 2], Vec<usize>> = BTreeMap::new();
    for (i, v) in game.vertices.iter().enumerate() {
        by_uv.entry(key(uv(v))).or_default().push(i);
    }
    let n = soup.vertices.len();
    let mut target: Vec<Option<([f64; 3], Skin)>> = vec![None; n];
    let mut share = vec![0.0; n];
    let mut how = NeckCarry::default();
    for (i, v) in soup.vertices.iter().enumerate() {
        share[i] = neck_share(head_weight(&skin_of(v), head));
        if share[i] <= 0.0 {
            continue;
        }
        let (u, p) = (uv(v), position(v));
        let k = key(u);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in by_uv.get(&[k[0] + dx, k[1] + dy]).into_iter().flatten() {
                    let w = uv(&game.vertices[j]);
                    if (w[0] - u[0]).abs() > 2e-4 || (w[1] - u[1]).abs() > 2e-4 {
                        continue;
                    }
                    let d = sub(position(&game.vertices[j]), p);
                    let d = dot(d, d);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, j));
                    }
                }
            }
        }
        if let Some((_, j)) = best {
            let g = &game.vertices[j];
            target[i] = Some((position(g), skin_of(g)));
        }
    }
    // Unmatched vertices move as their matched neighbours do.
    let mesh = parse_mesh(donor, 48, label)?;
    let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
    let mut moves: Vec<Option<[f64; 3]>> = vec![None; neighbours.len()];
    for i in 0..n {
        if let Some((t, _)) = &target[i] {
            moves[weld[i]] = Some(sub(*t, position(&soup.vertices[i])));
        }
    }
    loop {
        let mut filled = vec![];
        for k in 0..moves.len() {
            if moves[k].is_some() {
                continue;
            }
            let known: Vec<[f64; 3]> = neighbours[k].iter().filter_map(|&u| moves[u]).collect();
            if !known.is_empty() {
                let m = known.len() as f64;
                filled.push((
                    k,
                    [0, 1, 2].map(|j| known.iter().map(|d| d[j]).sum::<f64>() / m),
                ));
            }
        }
        if filled.is_empty() {
            break;
        }
        for (k, d) in filled {
            moves[k] = Some(d);
        }
    }
    for (i, v) in soup.vertices.iter_mut().enumerate() {
        if share[i] <= 0.0 {
            continue;
        }
        let Some(m) = moves[weld[i]] else {
            continue;
        };
        if target[i].is_none() {
            how.unmatched += 1;
        }
        let p = position(v);
        let q = [0, 1, 2].map(|k| p[k] + share[i] * m[k]);
        set_position(v, q);
        if let Some((_, skin)) = &target[i] {
            let mixed = mix_skins(&skin_of(v), skin, share[i]);
            put_skin(v, &mixed);
        }
        let d = share[i] * dot(m, m).sqrt();
        if d > 1e-6 {
            how.moved += 1;
            how.farthest = how.farthest.max(d);
        }
    }
    Ok((soup.bytes()?, how))
}
