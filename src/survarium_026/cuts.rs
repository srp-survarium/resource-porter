// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry cuts: open a box-shaped hole in a converted map's static render
//! surfaces and collision (e.g. widen a ceiling hatch so a plank route fits).
//!
//! Every triangle is clipped against the box: the parts outside it are kept
//! (split along the box planes, attributes interpolated linearly), the part
//! inside is dropped. Triangles that miss the box keep their bytes and their
//! vertex indices; split pieces get appended vertices.
//!
//! Where the box goes through something solid (a floor slab's thickness, a
//! wall), the box's own faces close it: each surface the box opens leaves an
//! edge on a box face, and the part of every face that lies inside the solid
//! gets a cap, in the render surfaces and in the collision alike. A cap takes
//! the material, texture scale and face material of the surface next to it,
//! turned round that surface's edge, so an opening shows no void and a walker
//! meets what he sees.
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
            let mut w = lerp(u, v, du / (du - dv));
            w.p[axis] = value;
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

fn misses<const N: usize>(tri: &[Vert<N>; 3], cut: &Cut) -> bool {
    let (lo, hi) = (cut.lo(), cut.hi());
    (0..3).any(|k| tri.iter().all(|v| v.p[k] <= lo[k]) || tri.iter().all(|v| v.p[k] >= hi[k]))
}

/// The parts of a triangle outside `cut`, or `None` when the triangle misses
/// the box entirely (keep it as is).
fn clip<const N: usize>(tri: [Vert<N>; 3], cut: &Cut) -> Option<Vec<[Vert<N>; 3]>> {
    if misses(&tri, cut) {
        return None;
    }
    let (lo, hi) = (cut.lo(), cut.hi());
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

/// A triangle list being cut: its vertices, its triangles and the source
/// triangle of each (whose face material a collision triangle keeps).
struct Mesh<const N: usize> {
    verts: Vec<Vert<N>>,
    tris: Vec<[usize; 3]>,
    parents: Vec<usize>,
}

impl<const N: usize> Mesh<N> {
    fn new(verts: Vec<Vert<N>>, tris: Vec<[usize; 3]>) -> Self {
        let parents = (0..tris.len()).collect();
        Self { verts, tris, parents }
    }
    /// Drop what lies inside `cut`; whether anything changed.
    fn clip(&mut self, cut: &Cut) -> bool {
        let mut touched = false;
        let mut tris = Vec::with_capacity(self.tris.len());
        let mut parents = Vec::with_capacity(self.tris.len());
        for (t, parent) in self.tris.iter().zip(&self.parents) {
            match clip(t.map(|i| self.verts[i]), cut) {
                None => {
                    tris.push(*t);
                    parents.push(*parent);
                }
                Some(pieces) => {
                    touched = true;
                    for piece in pieces {
                        let base = self.verts.len();
                        self.verts.extend_from_slice(&piece);
                        tris.push([base, base + 1, base + 2]);
                        parents.push(*parent);
                    }
                }
            }
        }
        (self.tris, self.parents) = (tris, parents);
        touched
    }
}

type V3 = [f64; 3];

fn v64(p: [f32; 3]) -> V3 {
    p.map(f64::from)
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    a.map(|c| c * s)
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> Option<V3> {
    let l = length(a);
    (l > 1e-9).then(|| scale(a, 1.0 / l))
}
fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// Coordinates closer than this (0.1 mm) are one place.
const EPS: f64 = 1e-4;
/// How far a box face looks out of the box for the surface that tells
/// whether it lies in something solid.
const LOOK_OUT: f64 = 2.0;

/// One face of the cut box, in its plane's 2D coordinates (u, v); v is
/// height on the four upright faces.
#[derive(Clone, Copy, Debug)]
struct Face {
    k: usize,
    w: f64,
    /// Into the box: the side a cap on this face faces.
    inward: V3,
    iu: usize,
    iv: usize,
    lo: [f64; 2],
    hi: [f64; 2],
}

impl Face {
    fn all(cut: &Cut) -> Vec<Face> {
        let (lo, hi) = (v64(cut.lo()), v64(cut.hi()));
        let mut faces = Vec::new();
        for k in 0..3 {
            let (iu, iv) = match k {
                0 => (2, 1),
                1 => (0, 2),
                _ => (0, 1),
            };
            for (w, s) in [(lo[k], 1.0), (hi[k], -1.0)] {
                let mut inward = [0.0; 3];
                inward[k] = s;
                faces.push(Face { k, w, inward, iu, iv, lo: [lo[iu], lo[iv]], hi: [hi[iu], hi[iv]] });
            }
        }
        faces
    }
    fn at(&self, uv: [f64; 2]) -> V3 {
        let mut p = [0.0; 3];
        p[self.k] = self.w;
        p[self.iu] = uv[0];
        p[self.iv] = uv[1];
        p
    }
    fn uv(&self, p: V3) -> [f64; 2] {
        [p[self.iu], p[self.iv]]
    }
    fn contains(&self, uv: [f64; 2]) -> bool {
        (0..2).all(|i| uv[i] >= self.lo[i] && uv[i] <= self.hi[i])
    }
    /// Where a triangle that reaches into the box past this face meets the
    /// face's plane, clipped to the face: the edge the cut leaves in its
    /// surface there.
    fn edge(&self, p: &[V3; 3]) -> Option<([f64; 2], [f64; 2])> {
        let s = self.inward[self.k];
        let d = p.map(|q| (q[self.k] - self.w) * s);
        if !d.iter().any(|&x| x > 0.0) {
            return None;
        }
        let mut points: Vec<V3> = Vec::new();
        for i in 0..3 {
            let j = (i + 1) % 3;
            if d[i] == 0.0 {
                points.push(p[i]);
            }
            if d[i] * d[j] < 0.0 {
                // The same point for both triangles of a shared edge.
                let (a, b, da, db) = if p[i] < p[j] { (p[i], p[j], d[i], d[j]) } else { (p[j], p[i], d[j], d[i]) };
                let mut x = add(a, scale(sub(b, a), da / (da - db)));
                x[self.k] = self.w;
                points.push(x);
            }
        }
        let [a, b] = points[..] else { return None };
        let (a, b) = (self.uv(a), self.uv(b));
        // Liang-Barsky against the face rectangle.
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for i in 0..2 {
            let d = b[i] - a[i];
            for (q, r) in [(-d, a[i] - self.lo[i]), (d, self.hi[i] - a[i])] {
                if q == 0.0 {
                    if r < 0.0 {
                        return None;
                    }
                } else if q < 0.0 {
                    t0 = t0.max(r / q);
                } else {
                    t1 = t1.min(r / q);
                }
            }
        }
        let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let (a, b) = (at(t0), at(t1.max(t0)));
        ((b[0] - a[0]).abs() + (b[1] - a[1]).abs() > EPS).then_some((a, b))
    }
}

/// A triangle of the meshes before the cut, by its mesh.
struct Source<const N: usize> {
    mesh: usize,
    corners: [Vert<N>; 3],
    parent: usize,
    p: [V3; 3],
    normal: V3,
}

/// An edge the cut leaves on a box face, and the side of it that is air (the
/// way its surface faces).
struct Edge {
    a: [f64; 2],
    b: [f64; 2],
    air: [f64; 2],
    source: usize,
}

/// A piece of a box face no edge crosses: between `u[0]` and `u[1]`, from
/// `lower` to `upper` (v at both ends).
struct Cell {
    u: [f64; 2],
    lower: [f64; 2],
    upper: [f64; 2],
}

impl Cell {
    fn centre(&self) -> [f64; 2] {
        [
            (self.u[0] + self.u[1]) / 2.0,
            (self.lower[0] + self.lower[1] + self.upper[0] + self.upper[1]) / 4.0,
        ]
    }
    fn contains(&self, uv: [f64; 2]) -> bool {
        if uv[0] < self.u[0] || uv[0] > self.u[1] {
            return false;
        }
        let t = (uv[0] - self.u[0]) / (self.u[1] - self.u[0]);
        let at = |v: [f64; 2]| v[0] + (v[1] - v[0]) * t;
        uv[1] >= at(self.lower) && uv[1] <= at(self.upper)
    }
}

/// The face cut along every edge: vertical strips between the edges' ends and
/// crossings, each split at the edges that run through it.
fn cells(face: &Face, edges: &[Edge]) -> Vec<Cell> {
    let mut us = vec![face.lo[0], face.hi[0]];
    for (i, e) in edges.iter().enumerate() {
        us.extend([e.a[0], e.b[0]]);
        for f in &edges[i + 1..] {
            let (r, s) = ([e.b[0] - e.a[0], e.b[1] - e.a[1]], [f.b[0] - f.a[0], f.b[1] - f.a[1]]);
            let d = cross2(r, s);
            if d.abs() < 1e-12 {
                continue;
            }
            let q = [f.a[0] - e.a[0], f.a[1] - e.a[1]];
            let (t, w) = (cross2(q, s) / d, cross2(q, r) / d);
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&w) {
                us.push(e.a[0] + r[0] * t);
            }
        }
    }
    us.retain(|u| *u >= face.lo[0] && *u <= face.hi[0]);
    us.sort_by(f64::total_cmp);
    us.dedup_by(|later, earlier| *later - *earlier < EPS);
    let clamp = |v: f64| v.clamp(face.lo[1], face.hi[1]);
    let mut cells = Vec::new();
    for w in us.windows(2) {
        let (u0, u1) = (w[0], w[1]);
        let mid = (u0 + u1) / 2.0;
        let mut bounds: Vec<[f64; 3]> = edges
            .iter()
            .filter(|e| {
                (e.b[0] - e.a[0]).abs() > EPS && e.a[0].min(e.b[0]) <= u0 + EPS && e.a[0].max(e.b[0]) >= u1 - EPS
            })
            .map(|e| {
                let v = |u: f64| clamp(e.a[1] + (e.b[1] - e.a[1]) * (u - e.a[0]) / (e.b[0] - e.a[0]));
                [v(u0), v(u1), v(mid)]
            })
            .collect();
        bounds.push([face.lo[1]; 3]);
        bounds.push([face.hi[1]; 3]);
        bounds.sort_by(|a, b| a[2].total_cmp(&b[2]));
        for b in bounds.windows(2) {
            if b[1][2] - b[0][2] > EPS {
                cells.push(Cell { u: [u0, u1], lower: [b[0][0], b[0][1]], upper: [b[1][0], b[1][1]] });
            }
        }
    }
    cells
}

/// The surface nearest to a point of a box face, looking along the face four
/// ways (at the edges the cut leaves there) and out of the box (at the
/// surfaces beyond it): whether the point is inside something solid (that
/// surface faces away from it), and the surface.
fn nearest<const N: usize>(face: &Face, edges: &[Edge], sources: &[Source<N>], c: [f64; 2]) -> Option<(bool, usize)> {
    let mut best: Option<(f64, bool, usize)> = None;
    let mut consider = |t: f64, solid: bool, source: usize| {
        if best.is_none_or(|(d, _, _)| t < d) {
            best = Some((t, solid, source));
        }
    };
    for dir in [[0.0, 1.0], [0.0, -1.0], [1.0, 0.0], [-1.0, 0.0]] {
        for e in edges {
            let s = [e.b[0] - e.a[0], e.b[1] - e.a[1]];
            let d = cross2(dir, s);
            if d.abs() < 1e-12 {
                continue;
            }
            let q = [e.a[0] - c[0], e.a[1] - c[1]];
            let (t, w) = (cross2(q, s) / d, cross2(q, dir) / d);
            if t > 1e-7 && (-1e-9..=1.0 + 1e-9).contains(&w) {
                consider(t, dir[0] * e.air[0] + dir[1] * e.air[1] > 0.0, e.source);
            }
        }
    }
    let origin = face.at(c);
    let dir = scale(face.inward, -1.0);
    for (i, s) in sources.iter().enumerate() {
        // Moller-Trumbore.
        let (e1, e2) = (sub(s.p[1], s.p[0]), sub(s.p[2], s.p[0]));
        let h = cross(dir, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-12 {
            continue;
        }
        let q = sub(origin, s.p[0]);
        let a = dot(q, h) / det;
        let r = cross(q, e1);
        let b = dot(dir, r) / det;
        let t = dot(e2, r) / det;
        if a >= 0.0 && b >= 0.0 && a + b <= 1.0 && t > EPS && t < LOOK_OUT {
            consider(t, dot(dir, s.normal) > 0.0, i);
        }
    }
    best.map(|(_, solid, source)| (solid, source))
}

/// How a cap takes the look of a surface: the cap turned about the line where
/// the surface's plane meets the cap's, into the surface's plane (as if the
/// surface ran on past the cut's edge), so its texture runs on round the
/// corner at its own scale. A surface parallel to the cap is projected.
struct Fold {
    origin: V3,
    n_t: V3,
    n_c: V3,
    e: V3,
    d_t: V3,
    d_c: V3,
    hinged: bool,
}

impl Fold {
    fn new(p: &[V3; 3], n_t: V3, n_c: V3) -> Fold {
        let hinge = cross(n_t, n_c);
        if length(hinge) > 0.05 {
            let e = unit(hinge).unwrap();
            let d_c = unit(sub(scale(n_c, dot(n_t, n_c)), n_t)).unwrap();
            let d_t = unit(sub(n_c, scale(n_t, dot(n_c, n_t)))).unwrap();
            Fold { origin: p[0], n_t, n_c, e, d_t, d_c, hinged: true }
        } else {
            let axis = if n_t[0].abs() < 0.6 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let e = unit(cross(axis, n_t)).unwrap();
            let d_t = cross(n_t, e);
            Fold { origin: p[0], n_t, n_c, e, d_t, d_c: d_t, hinged: false }
        }
    }
    /// The point of the surface's plane a point of the cap shows.
    fn point(&self, q: V3) -> V3 {
        let h = dot(sub(q, self.origin), self.n_t);
        if self.hinged {
            let t = h / dot(self.d_c, self.n_t);
            add(sub(q, scale(self.d_c, t)), scale(self.d_t, t))
        } else {
            sub(q, scale(self.n_t, h))
        }
    }
    /// A direction of the surface as the cap has it.
    fn vector(&self, v: V3) -> V3 {
        add(
            add(scale(self.e, dot(v, self.e)), scale(self.d_c, dot(v, self.d_t))),
            scale(self.n_c, dot(v, self.n_t)),
        )
    }
}

fn barycentric(p: &[V3; 3], q: V3) -> [f64; 3] {
    let (v0, v1, v2) = (sub(p[1], p[0]), sub(p[2], p[0]), sub(q, p[0]));
    let (d00, d01, d11, d20, d21) = (dot(v0, v0), dot(v0, v1), dot(v1, v1), dot(v2, v0), dot(v2, v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-18 {
        return [1.0 / 3.0; 3];
    }
    let b = (d11 * d20 - d01 * d21) / den;
    let c = (d00 * d21 - d01 * d20) / den;
    [1.0 - b - c, b, c]
}

/// A cap triangle on its way into a mesh, by the surface it takes its look from.
struct Cap {
    source: usize,
    normal: V3,
    p: [V3; 3],
}

/// Cut `cut` out of `meshes` (one kind of mesh: every render surface, or
/// every collision mesh, of the models it opens) and cap the box's faces
/// where they lie in something solid. `vertex` makes a cap's vertex from the
/// surface it takes its look from.
fn cut_and_cap<const N: usize>(
    meshes: &mut [&mut Mesh<N>],
    cut: &Cut,
    vertex: impl Fn(&[Vert<N>; 3], &Fold, V3) -> Vert<N>,
) -> (Vec<bool>, Opened) {
    let (lo, hi) = (v64(cut.lo()), v64(cut.hi()));
    let near = |p: &[V3; 3]| {
        (0..3).all(|k| p.iter().any(|q| q[k] >= lo[k] - LOOK_OUT) && p.iter().any(|q| q[k] <= hi[k] + LOOK_OUT))
    };
    let mut sources: Vec<Source<N>> = Vec::new();
    let mut opened: Vec<usize> = Vec::new();
    for (m, mesh) in meshes.iter().enumerate() {
        for (t, parent) in mesh.tris.iter().zip(&mesh.parents) {
            let corners = t.map(|i| mesh.verts[i]);
            let p = corners.map(|v| v64(v.p));
            if !near(&p) {
                continue;
            }
            let Some(normal) = unit(cross(sub(p[1], p[0]), sub(p[2], p[0]))) else { continue };
            if !misses(&corners, cut) {
                opened.push(sources.len());
            }
            sources.push(Source { mesh: m, corners, parent: *parent, p, normal });
        }
    }
    let mut caps: Vec<Cap> = Vec::new();
    let mut uncapped = 0;
    for face in Face::all(cut) {
        let edges: Vec<Edge> = opened
            .iter()
            .filter_map(|&i| {
                let s = &sources[i];
                let (a, b) = face.edge(&s.p)?;
                let air = [s.normal[face.iu], s.normal[face.iv]];
                let l = (air[0] * air[0] + air[1] * air[1]).sqrt();
                // A surface almost in the face's plane has no side there.
                (l > 0.05).then(|| Edge { a, b, air: [air[0] / l, air[1] / l], source: i })
            })
            .collect();
        let cells = cells(&face, &edges);
        let mut solid = vec![false; cells.len()];
        for (i, cell) in cells.iter().enumerate() {
            let Some((true, source)) = nearest(&face, &edges, &sources, cell.centre()) else { continue };
            solid[i] = true;
            let mut corners: Vec<V3> = Vec::new();
            for q in [
                [cell.u[0], cell.lower[0]],
                [cell.u[1], cell.lower[1]],
                [cell.u[1], cell.upper[1]],
                [cell.u[0], cell.upper[0]],
            ] {
                let q = face.at(q);
                if corners.last().is_none_or(|l| length(sub(*l, q)) > EPS) {
                    corners.push(q);
                }
            }
            for j in 1..corners.len().saturating_sub(1) {
                let mut p = [corners[0], corners[j], corners[j + 1]];
                let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
                if length(n) < 1e-9 {
                    continue;
                }
                if dot(n, face.inward) < 0.0 {
                    p.swap(1, 2);
                }
                caps.push(Cap { source, normal: face.inward, p });
            }
        }
        // An edge with no cap behind it: a nearer surface faces the other
        // way there (a ladder rail open at its back against a wall).
        for e in &edges {
            let m = [(e.a[0] + e.b[0]) / 2.0, (e.a[1] + e.b[1]) / 2.0];
            // Just past the edge: a rung or a frame is a few millimetres thick.
            let probe = [m[0] - e.air[0] * 2.0 * EPS, m[1] - e.air[1] * 2.0 * EPS];
            if face.contains(probe) && cells.iter().position(|c| c.contains(probe)).is_some_and(|i| !solid[i]) {
                uncapped += 1;
            }
        }
    }
    let mut changed: Vec<bool> = meshes.iter_mut().map(|m| m.clip(cut)).collect();
    for cap in &caps {
        let s = &sources[cap.source];
        let fold = Fold::new(&s.p, s.normal, cap.normal);
        let mesh = &mut meshes[s.mesh];
        let base = mesh.verts.len();
        mesh.verts.extend(cap.p.map(|q| vertex(&s.corners, &fold, q)));
        mesh.tris.push([base, base + 1, base + 2]);
        mesh.parents.push(s.parent);
        changed[s.mesh] = true;
    }
    let opened = Opened { meshes: changed.iter().filter(|c| **c).count(), caps: caps.len(), uncapped };
    (changed, opened)
}

/// What a cut did to one kind of mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opened {
    /// Meshes it changed.
    pub meshes: usize,
    /// Cap triangles it added.
    pub caps: usize,
    /// Edges it left with no cap behind them.
    pub uncapped: usize,
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

/// A packed basis vector (bytes z, y, x, alpha) of a vertex's attributes.
fn basis(a: &[f32]) -> V3 {
    [a[2], a[1], a[0]].map(|b| f64::from(b) / 127.5 - 1.0)
}

fn pack_basis(v: V3, alpha: f32, out: &mut [f32]) {
    let byte = |c: f64| ((c + 1.0) * 127.5).floor().clamp(0.0, 255.0) as f32;
    out.copy_from_slice(&[byte(v[2]), byte(v[1]), byte(v[0]), alpha]);
}

/// A render cap's vertex: the UV the surface has where the fold puts it, the
/// surface's tangent frame turned with the fold, the cap's normal.
fn static_cap_vertex(corners: &[Vert<14>; 3], fold: &Fold, q: V3) -> Vert<14> {
    let p = corners.map(|v| v64(v.p));
    let l = barycentric(&p, fold.point(q));
    let nearest = (0..3).max_by(|&i, &j| l[i].total_cmp(&l[j])).unwrap();
    let mut a = corners[nearest].a;
    let uv = |k: usize| (0..3).map(|i| l[i] * f64::from(corners[i].a[k])).sum::<f64>() as f32;
    (a[12], a[13]) = (uv(12), uv(13));
    let mean = |o: usize| unit((0..3).fold([0.0; 3], |s, i| add(s, basis(&corners[i].a[o..o + 3]))));
    let tangent = mean(4).map_or(fold.e, |t| fold.vector(t));
    let binormal = mean(8).map_or_else(|| cross(fold.n_c, fold.e), |b| fold.vector(b));
    let alphas = [a[3], a[7], a[11]];
    pack_basis(fold.n_c, alphas[0], &mut a[0..4]);
    pack_basis(tangent, alphas[1], &mut a[4..8]);
    pack_basis(binormal, alphas[2], &mut a[8..12]);
    Vert { p: q.map(|c| c as f32), a }
}

/// Counts of a cut static surface: (vertices, indices).
pub type SurfaceCounts = (usize, usize);

/// A converted (0.26 layout) static surface: chunk 3 (count, 28-byte
/// vertices: float3 position, three packed basis vectors, half2 UV), chunk 4
/// (count, u16 indices), then any further chunks unchanged.
pub(super) struct StaticSurface {
    raw: Vec<u8>,
    original: usize,
    mesh: Mesh<14>,
    tail: Vec<u8>,
    changed: bool,
}

impl StaticSurface {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self> {
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
        let raw = bytes[12..end].to_vec();
        let verts: Vec<Vert<14>> = raw
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
        let idx: Vec<usize> = bytes
            .get(end + 12..end + 12 + 2 * icount)
            .context("truncated indices")?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
            .collect();
        ensure!(idx.iter().all(|&i| i < count), "index outside the vertices");
        let tris = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        Ok(Self {
            raw,
            original: count,
            mesh: Mesh::new(verts, tris),
            tail: bytes[end + 8 + isize_..].to_vec(),
            changed: false,
        })
    }

    /// The surface's bytes and counts, or `None` when no cut changed it.
    pub(super) fn into_bytes(self) -> Result<Option<(Vec<u8>, SurfaceCounts)>> {
        if !self.changed {
            return Ok(None);
        }
        let Mesh { verts, tris, .. } = self.mesh;
        ensure!(!tris.is_empty(), "a cut removes a whole static surface");
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
        result.extend(&self.raw);
        for v in &verts[self.original..] {
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
        result.extend(u32::try_from(4 + 2 * tris.len() * 3)?.to_le_bytes());
        result.extend(u32::try_from(tris.len() * 3)?.to_le_bytes());
        for t in &tris {
            for &i in t {
                result.extend(u16::try_from(i)?.to_le_bytes());
            }
        }
        result.extend(&self.tail);
        Ok(Some((result, (verts.len(), tris.len() * 3))))
    }
}

/// Cut and cap render surfaces together (every surface under the cut's
/// models).
pub(super) fn cut_surfaces(surfaces: &mut [&mut StaticSurface], cut: &Cut) -> Opened {
    let mut meshes: Vec<&mut Mesh<14>> = surfaces.iter_mut().map(|s| &mut s.mesh).collect();
    let (changed, opened) = cut_and_cap(&mut meshes, cut, static_cap_vertex);
    for (s, c) in surfaces.iter_mut().zip(&changed) {
        s.changed |= c;
    }
    opened
}

/// A collision mesh's vertices, indices and face_data.
pub(super) type CollisionFiles = (Vec<u8>, Vec<u8>, Vec<u8>);

/// A model's collision triple: vertices (chunk 0x19, float3), indices
/// (chunk 0x1a, u32 triples) and face_data (0x1b dictionary, 0x1c u16 per
/// triangle). Split triangles and caps keep their source's face material.
pub(super) struct CollisionMesh {
    mesh: Mesh<0>,
    dictionary: Vec<u8>,
    faces: Vec<[u8; 2]>,
    changed: bool,
}

impl CollisionMesh {
    pub(super) fn parse(vertices: &[u8], indices: &[u8], face_data: &[u8]) -> Result<Self> {
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
        let verts: Vec<Vert<0>> = vertices[8..]
            .chunks_exact(12)
            .map(|v| Vert {
                p: std::array::from_fn(|k| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap())),
                a: [],
            })
            .collect();
        let tris: Vec<[usize; 3]> = indices[8..]
            .chunks_exact(12)
            .map(|t| {
                std::array::from_fn(|k| {
                    u32::from_le_bytes(t[k * 4..k * 4 + 4].try_into().unwrap()) as usize
                })
            })
            .collect();
        ensure!(
            tris.iter().flatten().all(|&i| i < verts.len()),
            "collision index outside the vertices"
        );
        Ok(Self {
            mesh: Mesh::new(verts, tris),
            dictionary: face_data[..fo].to_vec(),
            faces: face_data[fo + 8..].chunks_exact(2).map(|b| [b[0], b[1]]).collect(),
            changed: false,
        })
    }

    /// The mesh's files, or `None` when no cut changed it.
    pub(super) fn into_bytes(self) -> Result<Option<CollisionFiles>> {
        if !self.changed {
            return Ok(None);
        }
        let Mesh { verts, tris, parents } = self.mesh;
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
        i.extend(u32::try_from(12 * tris.len())?.to_le_bytes());
        for t in &tris {
            for &k in t {
                i.extend(u32::try_from(k)?.to_le_bytes());
            }
        }
        let mut f = self.dictionary;
        f.extend(0x1cu32.to_le_bytes());
        f.extend(u32::try_from(2 * tris.len())?.to_le_bytes());
        for &p in &parents {
            f.extend(self.faces[p]);
        }
        Ok(Some((v, i, f)))
    }
}

/// Cut and cap collision meshes together (every model under the cut's
/// models).
pub(super) fn cut_collisions(meshes: &mut [&mut CollisionMesh], cut: &Cut) -> Opened {
    let mut inner: Vec<&mut Mesh<0>> = meshes.iter_mut().map(|m| &mut m.mesh).collect();
    let (changed, opened) = cut_and_cap(&mut inner, cut, |_, _, q| Vert { p: q.map(|c| c as f32), a: [] });
    for (m, c) in meshes.iter_mut().zip(&changed) {
        m.changed |= c;
    }
    opened
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
                }
                // No piece lies strictly inside the hole.
                let c = [0, 1, 2].map(|k| (p[0].p[k] + p[1].p[k] + p[2].p[k]) / 3.0);
                assert!(!(c[0] > 0.25 && c[0] < 0.75 && c[2] > 0.25 && c[2] < 0.75));
            }
        }
        assert!((total - 0.75).abs() < 1e-5, "{total}");
        // A triangle away from the box is untouched.
        assert!(clip([v(2., 2.), v(3., 2.), v(3., 3.)], &cut()).is_none());
    }

    /// A closed slab, x and z 0..4, y 0..1, faces outward, as two meshes
    /// (x 0..2 and 2..4: two surfaces with their own looks).
    fn slab() -> (Mesh<0>, Mesh<0>) {
        let quad = |c: [[f32; 3]; 4]| [[c[0], c[1], c[2]], [c[0], c[2], c[3]]];
        let mut halves = Vec::new();
        for (x0, x1) in [(0.0f32, 2.0f32), (2.0, 4.0)] {
            let mut tris = Vec::new();
            tris.extend(quad([[x0, 1., 0.], [x0, 1., 4.], [x1, 1., 4.], [x1, 1., 0.]]));
            tris.extend(quad([[x0, 0., 0.], [x1, 0., 0.], [x1, 0., 4.], [x0, 0., 4.]]));
            tris.extend(quad([[x0, 0., 0.], [x0, 1., 0.], [x1, 1., 0.], [x1, 0., 0.]]));
            tris.extend(quad([[x0, 0., 4.], [x1, 0., 4.], [x1, 1., 4.], [x0, 1., 4.]]));
            if x0 == 0.0 {
                tris.extend(quad([[0., 0., 0.], [0., 0., 4.], [0., 1., 4.], [0., 1., 0.]]));
            } else {
                tris.extend(quad([[4., 0., 0.], [4., 1., 0.], [4., 1., 4.], [4., 0., 4.]]));
            }
            let verts: Vec<Vert<0>> = tris.iter().flatten().map(|p| Vert { p: *p, a: [] }).collect();
            let idx = (0..tris.len()).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect();
            halves.push(Mesh::new(verts, idx));
        }
        let b = halves.pop().unwrap();
        (halves.pop().unwrap(), b)
    }

    /// The sum of the area vectors: zero for a closed surface.
    fn area_vector(mesh: &Mesh<0>) -> V3 {
        mesh.tris.iter().fold([0.0; 3], |s, t| {
            let p = t.map(|i| v64(mesh.verts[i].p));
            add(s, scale(cross(sub(p[1], p[0]), sub(p[2], p[0])), 0.5))
        })
    }

    fn total_area(mesh: &Mesh<0>) -> f64 {
        mesh.tris
            .iter()
            .map(|t| {
                let p = t.map(|i| v64(mesh.verts[i].p));
                length(cross(sub(p[1], p[0]), sub(p[2], p[0]))) / 2.0
            })
            .sum()
    }

    fn plain(_: &[Vert<0>; 3], _: &Fold, q: V3) -> Vert<0> {
        Vert { p: q.map(|c| c as f32), a: [] }
    }

    #[test]
    fn a_hole_through_a_slab_gets_its_four_walls() {
        let (mut a, mut b) = slab();
        let before = total_area(&a) + total_area(&b);
        // A 1 x 1 m hole through the slab, across the two meshes' seam.
        let hole = Cut { centre: [2.0, 0.5, 2.0], half: [0.5, 1.0, 0.5], ..cut() };
        let (changed, opened) = cut_and_cap(&mut [&mut a, &mut b], &hole, plain);
        assert_eq!(opened.uncapped, 0);
        assert_eq!(changed, [true, true]);
        // Top and bottom lose 1 m2 each; the hole's four walls are 1 x 1 m.
        let after = total_area(&a) + total_area(&b);
        assert!((after - (before - 2.0 + 4.0)).abs() < 1e-4, "{before} -> {after}");
        let s = add(area_vector(&a), area_vector(&b));
        assert!(length(s) < 1e-4, "{s:?}");
    }

    #[test]
    fn a_pit_in_a_slab_gets_walls_and_a_floor() {
        let (mut a, mut b) = slab();
        let before = total_area(&a) + total_area(&b);
        // Down to y = 0.5 from above: the box's bottom face lies inside the
        // slab, with no edge on it.
        let pit = Cut { centre: [1.0, 1.0, 2.0], half: [0.5, 0.5, 0.5], ..cut() };
        let (changed, opened) = cut_and_cap(&mut [&mut a, &mut b], &pit, plain);
        assert_eq!(opened.uncapped, 0);
        assert_eq!(changed, [true, false]);
        let after = total_area(&a) + total_area(&b);
        assert!((after - (before - 1.0 + 4.0 * 0.5 + 1.0)).abs() < 1e-4, "{before} -> {after}");
        assert!(length(add(area_vector(&a), area_vector(&b))) < 1e-4);
    }

    #[test]
    fn a_cap_continues_the_texture_round_the_edge() {
        // A floor at y = 1 facing up, u = x, v = z, kept east of a box that
        // ends at x = 2; the cap there faces -x into the box and runs the
        // floor on over its edge: 0.25 m down shows u = 1.75.
        let corner = |x: f32, z: f32| {
            let mut a = [0.0; 14];
            a[12] = x;
            a[13] = z;
            Vert { p: [x, 1.0, z], a }
        };
        let floor = [corner(0., 0.), corner(0., 4.), corner(4., 4.)];
        let p = floor.map(|v| v64(v.p));
        let fold = Fold::new(&p, [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]);
        let v = static_cap_vertex(&floor, &fold, [2.0, 0.75, 3.0]);
        assert!((v.a[12] - 1.75).abs() < 1e-5 && (v.a[13] - 3.0).abs() < 1e-5, "{:?}", &v.a[12..]);
        assert_eq!(basis(&v.a[0..3]).map(|c| c.round()), [-1.0, 0.0, 0.0]);
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
        let across = Cut { centre: [0.5, 0.25, 0.5], half: [0.25, 0.5, 0.25], ..cut() };
        let mut mesh = CollisionMesh::parse(&vertices, &indices, &face).unwrap();
        assert_eq!(cut_collisions(&mut [&mut mesh], &across).meshes, 1);
        let (v, i, f) = mesh.into_bytes().unwrap().unwrap();
        let tris = (i.len() - 8) / 12;
        assert!(tris > 2);
        assert_eq!(f.len(), 12 + 8 + 2 * tris);
        assert_eq!((v.len() - 8) % 12, 0);
        let mut far = CollisionMesh::parse(&vertices, &indices, &face).unwrap();
        let away = Cut { centre: [5.0, 0.0, 5.0], ..cut() };
        assert_eq!(cut_collisions(&mut [&mut far], &away), Opened::default());
        assert!(far.into_bytes().unwrap().is_none());
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
            b.extend([128u8, 0, 128, 255, 255, 128, 128, 255, 128, 128, 255, 255]);
            b.extend([0u8, 0, 0, 0]);
        }
        b.extend(4u32.to_le_bytes());
        b.extend(10u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        for i in [0u16, 1, 2] {
            b.extend(i.to_le_bytes());
        }
        let across = Cut { centre: [0.5, 0.25, 0.5], half: [0.25, 0.5, 0.25], ..cut() };
        let mut s = StaticSurface::parse(&b).unwrap();
        assert_eq!(cut_surfaces(&mut [&mut s], &across).meshes, 1);
        let (out, (nv, ni)) = s.into_bytes().unwrap().unwrap();
        assert_eq!(u32_at(&out, 8).unwrap(), nv);
        let end = 8 + u32_at(&out, 4).unwrap();
        assert_eq!(u32_at(&out, end + 8).unwrap(), ni);
        assert_eq!(&out[12..12 + 84], &b[12..12 + 84]);
        let mut far = StaticSurface::parse(&b).unwrap();
        let away = Cut { centre: [9.0, 0.0, 9.0], ..cut() };
        assert_eq!(cut_surfaces(&mut [&mut far], &away), Opened::default());
        assert!(far.into_bytes().unwrap().is_none());
    }
}
