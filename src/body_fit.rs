// SPDX-License-Identifier: GPL-3.0-or-later
//! Fits a converted jacket over what is worn under it along the body.
//!
//! A torso item is drawn together with every stock trousers, the native
//! head's neck and every mask's neck hem. Where those come through the
//! jacket's waist or collar by a little, the jacket is pushed out over them
//! horizontally from the body's axis, smoothly. Parts that stand far out
//! (thigh holsters, pouches on a belt, a gas mask's jaw) are not cloth under
//! the jacket: stock long jackets cross them the same way, and covering them
//! would inflate the jacket for everyone, so a connected part that comes out
//! further than `cap` is left as it is (and reported).
use crate::mask_fit::{Mesh, dot, farthest_hit, sub, weld_vertices};
use anyhow::{Result, ensure};
use std::collections::HashMap;

/// How far beyond the scope the garment may move (m).
const MOVE_MARGIN: f64 = 0.05;

pub(crate) struct Cover<'a> {
    pub label: String,
    pub mesh: &'a Mesh,
}

#[derive(Clone, Copy)]
pub struct BodyFit<'a> {
    /// The body axis, bottom to top: (x, y, z) points by rising y.
    pub axis: &'a [[f64; 3]],
    /// Only garment and accessory points within this height range, and
    /// within `max_x` of the middle, take part (the trunk, not the arms).
    pub y_range: (f64, f64),
    pub max_x: f64,
    pub clearance: f64,
    pub cap: f64,
    /// Judge a whole connected part by its farthest point (holsters, belt
    /// pouches), or each point alone (one-piece masks, whose hem lies under
    /// the collar while their jaw stands in front of it).
    pub by_part: bool,
    /// Accessory points straight in front of the axis (horizontal direction
    /// with a forward component above this) are not under the garment: a
    /// respirator hangs in front of a collar. 1 takes every direction.
    pub max_forward: f64,
}

#[derive(Debug, Default, Clone)]
pub struct CoverReport {
    pub label: String,
    /// Accessory points outside the garment (or inside the clearance)
    /// before and after, and the farthest after (m).
    pub outside_before: usize,
    pub outside_after: usize,
    pub depth_after: f64,
    /// Points of parts left uncovered because they stand out beyond `cap`,
    /// and the farthest of them (m).
    pub left_points: usize,
    pub left_depth: f64,
}

#[derive(Debug, Default)]
pub struct BodyFitReport {
    pub moved: usize,
    pub max_move: f64,
    pub covers: Vec<CoverReport>,
}

impl BodyFit<'_> {
    fn axis_at(&self, y: f64) -> [f64; 3] {
        let a = self.axis;
        if y <= a[0][1] {
            return [a[0][0], y, a[0][2]];
        }
        for w in a.windows(2) {
            if y <= w[1][1] {
                let t = (y - w[0][1]) / (w[1][1] - w[0][1]);
                return [w[0][0] + t * (w[1][0] - w[0][0]), y, w[0][2] + t * (w[1][2] - w[0][2])];
            }
        }
        let last = a[a.len() - 1];
        [last[0], y, last[2]]
    }

    fn in_scope(&self, p: [f64; 3]) -> bool {
        p[1] >= self.y_range.0 && p[1] <= self.y_range.1 && p[0].abs() <= self.max_x
    }

    /// (origin on the axis, horizontal direction, distance) of a point.
    fn ray(&self, p: [f64; 3]) -> Option<([f64; 3], [f64; 3], f64)> {
        let o = self.axis_at(p[1]);
        let d = [p[0] - o[0], 0.0, p[2] - o[2]];
        let r = dot(d, d).sqrt();
        (r > 1e-6).then(|| (o, d.map(|v| v / r), r))
    }
}

/// Points of an accessory: (position, connected part).
fn parts(mesh: &Mesh) -> Vec<usize> {
    let (weld, neighbours) = weld_vertices(mesh);
    let mut part = vec![usize::MAX; neighbours.len()];
    let mut next = 0;
    for start in 0..neighbours.len() {
        if part[start] != usize::MAX {
            continue;
        }
        let mut stack = vec![start];
        part[start] = next;
        while let Some(v) = stack.pop() {
            for &u in &neighbours[v] {
                if part[u] == usize::MAX {
                    part[u] = next;
                    stack.push(u);
                }
            }
        }
        next += 1;
    }
    weld.iter().map(|&w| part[w]).collect()
}

/// How far each in-scope accessory point is outside the garment (plus the
/// clearance), where the garment covers it: (point index, penetration).
fn pokes(
    fit: &BodyFit<'_>,
    garment: &[[f64; 3]],
    triangles: &[[usize; 3]],
    acc: &Mesh,
) -> Vec<(usize, f64, usize, [f64; 3])> {
    let mut out = vec![];
    for (i, &q) in acc.positions.iter().enumerate() {
        if !fit.in_scope(q) {
            continue;
        }
        let Some((o, dir, r)) = fit.ray(q) else {
            continue;
        };
        if dir[2] > fit.max_forward {
            continue;
        }
        if let Some((t, tri, bary)) = farthest_hit(o, dir, garment, triangles) {
            let pen = r + fit.clearance - t;
            if pen > 0.0 {
                out.push((i, pen, tri, bary));
            }
        }
    }
    out
}

/// Pushes `garment` (48-byte vertices) out over every accessory in place.
pub(crate) fn fit<'m>(garment: &mut [u8], covers: &[Cover<'m>], spec: &BodyFit<'_>) -> Result<BodyFitReport> {
    let mesh = crate::mask_fit::parse_mesh(garment, 48, "garment")?;
    let (weld, neighbours) = weld_vertices(&mesh);
    let welded_count = neighbours.len();
    let start = mesh.positions.clone();
    let mut positions = start.clone();
    let mut report = BodyFitReport::default();
    // Parts standing out beyond the cap, judged on the garment as it came.
    let mut skipped: Vec<std::collections::BTreeSet<usize>> = vec![];
    for c in covers {
        let part = parts(c.mesh);
        let mut worst: HashMap<usize, (f64, usize)> = HashMap::new();
        let found = pokes(spec, &positions, &mesh.triangles, c.mesh);
        for &(i, pen, _, _) in &found {
            let e = worst.entry(part[i]).or_insert((0.0, 0));
            e.0 = e.0.max(pen - spec.clearance);
            e.1 += 1;
        }
        let skip: std::collections::BTreeSet<usize> = if spec.by_part {
            worst
                .iter()
                .filter(|(_, (depth, _))| *depth > spec.cap)
                .map(|(p, _)| *p)
                .collect()
        } else {
            Default::default()
        };
        let mut r = CoverReport {
            label: c.label.clone(),
            outside_before: found.len(),
            ..Default::default()
        };
        let far = |i: usize, pen: f64| {
            if spec.by_part {
                skip.contains(&part[i])
            } else {
                pen - spec.clearance > spec.cap
            }
        };
        for &(i, pen, _, _) in &found {
            if far(i, pen) {
                r.left_points += 1;
                r.left_depth = r.left_depth.max(pen - spec.clearance);
            }
        }
        report.covers.push(r);
        // Store the skipped points by index, not by part.
        skipped.push(
            found
                .iter()
                .filter(|&&(i, pen, _, _)| far(i, pen))
                .map(|&(i, _, _, _)| i)
                .chain((0..c.mesh.positions.len()).filter(|&i| spec.by_part && skip.contains(&part[i])))
                .collect(),
        );
    }
    // The garment moves a little beyond the scope too, so a point near its
    // edge can push the triangles it meets there.
    let movable: Vec<bool> = positions
        .iter()
        .map(|&p| {
            p[1] >= spec.y_range.0 - MOVE_MARGIN
                && p[1] <= spec.y_range.1 + MOVE_MARGIN
                && p[0].abs() <= spec.max_x + MOVE_MARGIN
        })
        .collect();
    let mut converged = false;
    for _ in 0..40 {
        let mut need = vec![0.0_f64; welded_count];
        for (c, skip) in covers.iter().zip(&skipped) {
            for (i, pen, tri, bary) in pokes(spec, &positions, &mesh.triangles, c.mesh) {
                // A part judged whole stays as judged; a point alone is
                // judged again as the garment moves out toward it.
                let far = if spec.by_part {
                    skip.contains(&i)
                } else {
                    pen - spec.clearance > spec.cap
                };
                if far {
                    continue;
                }
                for (corner, b) in mesh.triangles[tri].iter().zip(bary) {
                    if movable[*corner] {
                        let w = &mut need[weld[*corner]];
                        *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
                    }
                }
            }
        }
        // The garment never moves farther than the cap: what needs more is
        // left (and reported), it does not inflate the garment.
        for (i, p) in positions.iter().enumerate() {
            let d = sub(*p, start[i]);
            if dot(d, d).sqrt() >= spec.cap + spec.clearance - 1e-6 {
                need[weld[i]] = 0.0;
            }
        }
        if need.iter().all(|&n| n < 2e-4) {
            converged = true;
            break;
        }
        for _ in 0..4 {
            need = (0..welded_count)
                .map(|v| {
                    let n = &neighbours[v];
                    let mean = n.iter().map(|&u| need[u]).sum::<f64>() / n.len().max(1) as f64;
                    need[v].max(0.6 * mean)
                })
                .collect();
        }
        for (i, p) in positions.iter_mut().enumerate() {
            let push = need[weld[i]];
            if push <= 0.0 || !movable[i] {
                continue;
            }
            if let Some((_, dir, _)) = spec.ray(*p) {
                let room = spec.cap + spec.clearance - {
                    let d = sub(*p, start[i]);
                    dot(d, d).sqrt()
                };
                let push = push.min(room.max(0.0));
                for k in 0..3 {
                    p[k] += dir[k] * push;
                }
            }
        }
    }
    ensure!(converged, "the garment did not clear what is under it in 40 passes");
    for (c, (skip, r)) in covers.iter().zip(skipped.iter().zip(&mut report.covers)) {
        let after: Vec<_> = pokes(spec, &positions, &mesh.triangles, c.mesh)
            .into_iter()
            .filter(|(i, pen, _, _)| {
                let far = if spec.by_part {
                    skip.contains(i)
                } else {
                    pen - spec.clearance > spec.cap
                };
                !far && *pen > spec.clearance * 0.5
            })
            .collect();
        r.outside_after = after.len();
        r.depth_after = after
            .iter()
            .map(|(_, pen, _, _)| pen - spec.clearance)
            .fold(0.0, f64::max);
    }
    for (i, p) in positions.iter().enumerate() {
        let d = sub(*p, start[i]);
        let m = dot(d, d).sqrt();
        if m > 1e-5 {
            report.moved += 1;
            report.max_move = report.max_move.max(m);
        }
        for (k, v) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            garment[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed vertical tube of `radius` around the y axis from y0 to y1.
    fn tube(radius: f64, y0: f64, y1: f64, segments: usize, rings: usize) -> Mesh {
        let mut positions = vec![];
        for i in 0..=rings {
            let y = y0 + (y1 - y0) * i as f64 / rings as f64;
            for j in 0..segments {
                let a = std::f64::consts::TAU * j as f64 / segments as f64;
                positions.push([radius * a.cos(), y, radius * a.sin()]);
            }
        }
        let mut triangles = vec![];
        for i in 0..rings {
            for j in 0..segments {
                let a = i * segments + j;
                let b = i * segments + (j + 1) % segments;
                triangles.push([a, a + segments, b]);
                triangles.push([b, a + segments, b + segments]);
            }
        }
        Mesh {
            positions,
            triangles,
        }
    }

    /// A converted surface (48-byte vertices, positions only) of a mesh.
    fn converted(mesh: &Mesh) -> Vec<u8> {
        let mut out = vec![];
        out.extend_from_slice(&3_u32.to_le_bytes());
        out.extend_from_slice(&((4 + mesh.positions.len() * 48) as u32).to_le_bytes());
        out.extend_from_slice(&(mesh.positions.len() as u32).to_le_bytes());
        for p in &mesh.positions {
            let mut v = [0_u8; 48];
            for k in 0..3 {
                v[k * 4..k * 4 + 4].copy_from_slice(&(p[k] as f32).to_le_bytes());
            }
            out.extend_from_slice(&v);
        }
        out.extend_from_slice(&4_u32.to_le_bytes());
        out.extend_from_slice(&((4 + mesh.triangles.len() * 6) as u32).to_le_bytes());
        out.extend_from_slice(&((mesh.triangles.len() * 3) as u32).to_le_bytes());
        for t in &mesh.triangles {
            for &i in t {
                out.extend_from_slice(&(i as u16).to_le_bytes());
            }
        }
        out
    }

    fn spec(by_part: bool) -> BodyFit<'static> {
        const AXIS: [[f64; 3]; 2] = [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        BodyFit {
            axis: &AXIS,
            y_range: (0.9, 1.1),
            max_x: 1.0,
            clearance: 0.004,
            cap: 0.025,
            by_part,
            max_forward: 1.0,
        }
    }

    #[test]
    fn the_garment_goes_out_over_what_comes_through_a_little() {
        let jacket = tube(0.15, 0.8, 1.2, 32, 8);
        let mut bytes = converted(&jacket);
        // Trousers 1 cm wider than the jacket inside its scope.
        let trousers = tube(0.16, 0.95, 1.05, 24, 2);
        let covers = [Cover {
            label: "trousers".into(),
            mesh: &trousers,
        }];
        let report = fit(&mut bytes, &covers, &spec(true)).unwrap();
        assert!(report.moved > 0);
        assert_eq!(report.covers[0].outside_after, 0, "{:?}", report.covers[0]);
        assert!(report.max_move < 0.025, "{}", report.max_move);
    }

    #[test]
    fn a_part_standing_far_out_is_left_and_does_not_inflate_the_garment() {
        let jacket = tube(0.15, 0.8, 1.2, 32, 8);
        let mut bytes = converted(&jacket);
        // A holster 5 cm outside the jacket: a part of its own.
        let holster = tube(0.2, 0.97, 1.03, 12, 1);
        let covers = [Cover {
            label: "holster".into(),
            mesh: &holster,
        }];
        let report = fit(&mut bytes, &covers, &spec(true)).unwrap();
        assert_eq!(report.moved, 0);
        assert!(report.covers[0].left_points > 0);
        // Judged point by point, the garment still never moves past the cap.
        let mut bytes = converted(&jacket);
        let report = fit(&mut bytes, &covers, &spec(false)).unwrap();
        assert!(report.max_move <= 0.025 + 0.004 + 1e-6, "{}", report.max_move);
    }
}

/// Points of `acc` (positions over `acc_triangles`' mesh, any pose) within
/// the scope and outside `garment` by more than `margin`: their count and
/// the farthest (m). For checks of posed meshes.
pub(crate) fn outside(
    spec: &BodyFit<'_>,
    garment: &[[f64; 3]],
    triangles: &[[usize; 3]],
    acc: &[[f64; 3]],
    margin: f64,
) -> (usize, f64) {
    let mesh = Mesh {
        positions: acc.to_vec(),
        triangles: vec![],
    };
    let quiet = BodyFit {
        clearance: 0.0,
        ..*spec
    };
    let found = pokes(&quiet, garment, triangles, &mesh);
    let bad: Vec<f64> = found
        .into_iter()
        .map(|(_, pen, _, _)| pen)
        .filter(|&pen| pen > margin)
        .collect();
    (bad.len(), bad.iter().copied().fold(0.0, f64::max))
}
