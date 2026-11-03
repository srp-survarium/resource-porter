// SPDX-License-Identifier: GPL-3.0-or-later
//! Fits converted clothing around stock accessories along a body axis.
//!
//! Stock items choose a garment variant per accessory class: boots select the
//! `data_medium`/`data_long` trousers, gloves the `data_short`/`data_long`
//! jacket. Each variant is the same cloth with its radius around the limb
//! axis clamped against every stock accessory of that class: trousers tucked
//! inside boot shafts, sleeves laid over glove cuffs.
use crate::mask_fit::{Mesh, bone_origins, cross, dot, parse_mesh, read_u32, sub};
use anyhow::{Context, Result, ensure};

/// Axial sampling step of the accessory profiles.
const STEP: f64 = 0.005;
const ANGLES: usize = 48;
/// Fraction of a slice's median radius below which a hit is inner geometry.
const INNER: f64 = 0.6;
/// A cuff lip or strap is not a cross-section of the enclosing garment.
/// Interpolate small cracks only, never the missing back of a partial arc.
const MIN_COVERAGE: usize = ANGLES * 3 / 4;
const MAX_GAP: usize = ANGLES / 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    /// The garment goes inside the accessory (trousers into boots).
    Inside,
    /// The garment goes over the accessory (sleeves over gloves).
    Outside,
}

pub struct LimbFit<'a> {
    /// (proximal, distal) bone names of each axis. An axis whose proximal
    /// bone is off-centre only takes vertices on its own side.
    pub axes: &'a [(&'a str, &'a str)],
    pub layer: Layer,
    pub clearance: f64,
    /// Radius change per metre beyond the accessory opening, so the cloth
    /// eases back to its own shape instead of stepping.
    pub ease: f64,
    /// Accessory surface farther than this from the axis is ignored (arms
    /// seen from the spine, toes seen from the shin).
    pub max_radius: f64,
    /// Outside layers stop this far past the distal bone (the hand itself is
    /// not sleeve).
    pub distal_reach: f64,
}

#[derive(Default, Debug)]
pub struct LimbFitReport {
    pub moved: usize,
    pub max_move: f64,
}

struct Axis {
    origin: [f64; 3],
    dir: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    side: f64,
    distal: f64,
}

impl Axis {
    fn new(a: [f64; 3], b: [f64; 3]) -> Result<Self> {
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        ensure!(len > 1e-4, "degenerate fit axis");
        let dir = d.map(|x| x / len);
        let helper = if dir[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let u = normalize(cross(dir, helper));
        let v = cross(dir, u);
        let side = if a[0].abs() < 0.02 {
            0.0
        } else {
            a[0].signum()
        };
        Ok(Axis {
            origin: a,
            dir,
            u,
            v,
            side,
            distal: len,
        })
    }

    fn owns(&self, p: [f64; 3]) -> bool {
        self.side == 0.0 || p[0] * self.side > 0.0
    }

    /// (axial position, radius, angle) of a point.
    fn cylindrical(&self, p: [f64; 3]) -> (f64, f64, f64) {
        let w = sub(p, self.origin);
        let s = dot(w, self.dir);
        let (x, y) = (dot(w, self.u), dot(w, self.v));
        (s, x.hypot(y), y.atan2(x))
    }

    fn point(&self, s: f64, r: f64, angle: f64) -> [f64; 3] {
        let (c, n) = (angle.cos() * r, angle.sin() * r);
        std::array::from_fn(|k| self.origin[k] + self.dir[k] * s + self.u[k] * c + self.v[k] * n)
    }
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    a.map(|x| x / l)
}

/// Accessory radius around one axis, sampled from its opening (`start`)
/// toward the distal end.
struct Profile {
    start: f64,
    radii: Vec<[f64; ANGLES]>,
}

impl Profile {
    fn sample(mesh: &Mesh, axis: &Axis, limit: f64, max_radius: f64) -> Option<Profile> {
        let tris: Vec<([[f64; 3]; 3], f64, f64)> = mesh
            .triangles
            .iter()
            .map(|t| t.map(|i| mesh.positions[i]))
            .filter(|t| t.iter().all(|&p| axis.owns(p)))
            .map(|t| {
                let s = t.map(|p| axis.cylindrical(p).0);
                (t, s[0].min(s[1]).min(s[2]), s[0].max(s[1]).max(s[2]))
            })
            .filter(|(t, _, _)| t.iter().any(|&p| axis.cylindrical(p).1 <= max_radius))
            .collect();
        let start = tris.iter().map(|t| t.1).fold(f64::INFINITY, f64::min);
        let end = tris
            .iter()
            .map(|t| t.2)
            .fold(f64::NEG_INFINITY, f64::max)
            .min(limit);
        if start.partial_cmp(&end) != Some(std::cmp::Ordering::Less) {
            return None;
        }
        let mut slices: Vec<Option<[f64; ANGLES]>> = vec![];
        let mut s = start + STEP / 2.0;
        while s <= end {
            let near: Vec<&[[f64; 3]; 3]> = tris
                .iter()
                .filter(|t| t.1 <= s && s <= t.2)
                .map(|t| &t.0)
                .collect();
            let mut hits = [None; ANGLES];
            for (k, hit) in hits.iter_mut().enumerate() {
                let angle = k as f64 * std::f64::consts::TAU / ANGLES as f64;
                let origin = axis.point(s, 0.0, 0.0);
                let dir = sub(axis.point(s, 1.0, angle), origin);
                *hit = farthest_within(origin, dir, &near, max_radius);
            }
            slices.push(fill_angles(hits));
            s += STEP;
        }
        // Slices without any surface take the nearest sampled neighbour.
        let first = slices.iter().position(Option::is_some)?;
        let last = slices.iter().rposition(Option::is_some)?;
        let radii = (first..=last)
            .map(|i| {
                (0..slices.len())
                    .filter_map(|j| slices[j].map(|r| (i.abs_diff(j), r)))
                    .min_by_key(|(d, _)| *d)
                    .map(|(_, r)| r)
                    .unwrap()
            })
            .collect();
        Some(Profile {
            start: start + first as f64 * STEP,
            radii,
        })
    }

    fn radius(&self, s: f64, angle: f64) -> f64 {
        let f = ((s - self.start) / STEP - 0.5).clamp(0.0, (self.radii.len() - 1) as f64);
        let (i, fs) = (f.floor() as usize, f.fract());
        let a = angle.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * ANGLES as f64;
        let (k, fa) = (a.floor() as usize % ANGLES, a.fract());
        let row = |r: &[f64; ANGLES]| r[k] * (1.0 - fa) + r[(k + 1) % ANGLES] * fa;
        let j = (i + 1).min(self.radii.len() - 1);
        row(&self.radii[i]) * (1.0 - fs) + row(&self.radii[j]) * fs
    }

    fn end(&self) -> f64 {
        self.start + self.radii.len() as f64 * STEP
    }

    fn bound(&self, s: f64, angle: f64, spec: &LimbFit<'_>) -> Option<f64> {
        // A sleeve or trouser leg does not continue beyond its actual cuff.
        if s > self.end() {
            return None;
        }
        let opening = self.start - s;
        let base = self.radius(s, angle);
        match spec.layer {
            Layer::Inside if opening > 0.0 => Some(base + spec.ease * opening),
            Layer::Inside => Some(base),
            Layer::Outside if opening > 0.0 => {
                Some(base - spec.ease * opening).filter(|b| *b > 0.0)
            }
            Layer::Outside => Some(base),
        }
    }
}

/// Angles the slice did not hit take the circular interpolation of their
/// nearest hit neighbours. Hits far inside the rest of the slice are inner
/// parts (belts, linings) seen through an opening, not the outer surface.
fn fill_angles(mut hits: [Option<f64>; ANGLES]) -> Option<[f64; ANGLES]> {
    let mut sorted: Vec<f64> = hits.iter().flatten().copied().collect();
    sorted.sort_by(f64::total_cmp);
    if let Some(&median) = sorted.get(sorted.len() / 2) {
        for hit in &mut hits {
            *hit = hit.filter(|r| *r >= median * INNER);
        }
    }
    let known: Vec<usize> = (0..ANGLES).filter(|&k| hits[k].is_some()).collect();
    if known.len() < MIN_COVERAGE
        || known
            .iter()
            .enumerate()
            .any(|(i, &k)| (known[(i + 1) % known.len()] + ANGLES - k - 1) % ANGLES > MAX_GAP)
    {
        return None;
    }
    Some(std::array::from_fn(|k| {
        if let Some(r) = hits[k] {
            return r;
        }
        let next = known
            .iter()
            .copied()
            .min_by_key(|&j| (j + ANGLES - k) % ANGLES)
            .unwrap();
        let prev = known
            .iter()
            .copied()
            .min_by_key(|&j| (k + ANGLES - j) % ANGLES)
            .unwrap();
        let (dn, dp) = ((next + ANGLES - k) % ANGLES, (k + ANGLES - prev) % ANGLES);
        let (rn, rp) = (hits[next].unwrap(), hits[prev].unwrap());
        (rn * dp as f64 + rp * dn as f64) / (dn + dp) as f64
    }))
}

fn farthest_within(
    origin: [f64; 3],
    dir: [f64; 3],
    tris: &[&[[f64; 3]; 3]],
    max: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for [a, b, c] in tris.iter().map(|t| **t) {
        let (e1, e2) = (sub(b, a), sub(c, a));
        let p = cross(dir, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-12 {
            continue;
        }
        let tv = sub(origin, a);
        let u = dot(tv, p) / det;
        let q = cross(tv, e1);
        let v = dot(dir, q) / det;
        let t = dot(e2, q) / det;
        if u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 && t > 0.0 && t <= max {
            best = Some(best.map_or(t, |b: f64| b.max(t)));
        }
    }
    best
}

fn parse_any(b: &[u8], label: &str) -> Result<Mesh> {
    let count = read_u32(b, 8)?;
    ensure!(count > 0, "{label}: empty model");
    parse_mesh(b, (read_u32(b, 4)? - 4) / count, label)
}

/// Fits `garment` (48-byte converted vertices) in place against every
/// accessory. All models must share `bind_pose`.
pub fn fit(
    garment: &mut [u8],
    accessories: &[(String, Vec<u8>)],
    bind_pose: &[u8],
    spec: &LimbFit<'_>,
) -> Result<LimbFitReport> {
    let mesh = parse_mesh(garment, 48, "garment")?;
    let bones = bone_origins(bind_pose)?;
    let accessories = accessories
        .iter()
        .map(|(name, b)| parse_any(b, name))
        .collect::<Result<Vec<_>>>()?;
    let mut positions = mesh.positions.clone();
    let mut report = LimbFitReport::default();
    for &(from, to) in spec.axes {
        let bone = |n: &str| {
            bones
                .get(n)
                .copied()
                .with_context(|| format!("no bone {n}"))
        };
        let axis = Axis::new(bone(from)?, bone(to)?)?;
        let limit = match spec.layer {
            Layer::Inside => f64::INFINITY,
            Layer::Outside => axis.distal + spec.distal_reach,
        };
        let profiles: Vec<Profile> = accessories
            .iter()
            .filter_map(|m| Profile::sample(m, &axis, limit, spec.max_radius))
            .collect();
        if profiles.is_empty() {
            continue;
        }
        for (i, p) in mesh.positions.iter().enumerate() {
            if !axis.owns(*p) {
                continue;
            }
            let (s, r, angle) = axis.cylindrical(*p);
            if r > spec.max_radius || r < 1e-6 {
                continue;
            }
            let bounds = profiles.iter().filter_map(|pr| pr.bound(s, angle, spec));
            let target = match spec.layer {
                Layer::Inside => bounds.fold(f64::INFINITY, f64::min) - spec.clearance,
                Layer::Outside => bounds.fold(f64::NEG_INFINITY, f64::max) + spec.clearance,
            };
            let moves = match spec.layer {
                Layer::Inside => r > target && target > 0.0,
                Layer::Outside => r < target,
            };
            if moves {
                positions[i] = axis.point(s, target, angle);
                report.moved += 1;
                report.max_move = report.max_move.max((r - target).abs());
            }
        }
    }
    for (i, p) in positions.iter().enumerate() {
        for (k, value) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            garment[at..at + 4].copy_from_slice(&(*value as f32).to_le_bytes());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tube(mesh: &mut Mesh, radius: f64, start: f64, end: f64, sectors: usize) {
        let offset = mesh.positions.len();
        for z in [start, end] {
            for k in 0..=sectors {
                let angle = k as f64 * std::f64::consts::TAU / ANGLES as f64;
                mesh.positions
                    .push([radius * angle.cos(), radius * angle.sin(), z]);
            }
        }
        for k in 0..sectors {
            let a = offset + k;
            let b = a + sectors + 1;
            mesh.triangles.extend([[a, a + 1, b], [a + 1, b + 1, b]]);
        }
    }

    #[test]
    fn a_partial_cuff_lip_does_not_become_a_narrow_shaft() {
        let mut mesh = Mesh {
            positions: vec![],
            triangles: vec![],
        };
        // The narrow inner lip starts before the enclosing shaft. The old
        // quarter-circle threshold fabricated its missing circumference and
        // narrowed every garment to that lip, even above the actual boot.
        tube(&mut mesh, 0.02, 0.0, 0.03, ANGLES / 3);
        tube(&mut mesh, 0.08, 0.03, 0.20, ANGLES);
        let axis = Axis::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap();
        let profile = Profile::sample(&mesh, &axis, 1.0, 0.15).unwrap();
        assert!((profile.start - 0.03).abs() < STEP);
        for k in 0..ANGLES {
            assert!(
                (profile.radius(
                    profile.start,
                    k as f64 * std::f64::consts::TAU / ANGLES as f64
                ) - 0.08)
                    .abs()
                    < 0.001
            );
        }
    }

    #[test]
    fn an_open_arc_is_not_an_enclosing_accessory() {
        let mut mesh = Mesh {
            positions: vec![],
            triangles: vec![],
        };
        tube(&mut mesh, 0.04, 0.0, 0.2, ANGLES / 2);
        let axis = Axis::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap();
        assert!(Profile::sample(&mesh, &axis, 1.0, 0.15).is_none());
    }

    #[test]
    fn small_mesh_cracks_are_interpolated_but_a_missing_side_is_not() {
        let mut hits = [Some(0.08); ANGLES];
        hits[0] = None;
        hits[ANGLES - 1] = None;
        assert_eq!(fill_angles(hits).unwrap(), [0.08; ANGLES]);
        for hit in hits.iter_mut().take(ANGLES / 4) {
            *hit = None;
        }
        assert!(fill_angles(hits).is_none());
    }

    #[test]
    fn trailing_partial_faces_do_not_extend_the_accessory() {
        let mut mesh = Mesh {
            positions: vec![],
            triangles: vec![],
        };
        tube(&mut mesh, 0.08, 0.0, 0.1, ANGLES);
        tube(&mut mesh, 0.02, 0.1, 0.2, ANGLES / 3);
        let axis = Axis::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap();
        let profile = Profile::sample(&mesh, &axis, 1.0, 0.15).unwrap();
        assert!((profile.end() - 0.1).abs() < STEP);
    }
    #[test]
    fn a_cuff_does_not_squeeze_the_exposed_limb_beyond_it() {
        let profile = Profile {
            start: 0.1,
            radii: vec![[0.04; ANGLES]; 10],
        };
        for layer in [Layer::Inside, Layer::Outside] {
            let spec = LimbFit {
                axes: &[],
                layer,
                clearance: 0.003,
                ease: 0.5,
                max_radius: 0.15,
                distal_reach: 0.0,
            };
            assert_eq!(profile.bound(0.12, 0.0, &spec), Some(0.04));
            assert_eq!(profile.bound(0.16, 0.0, &spec), None);
        }
    }
}
