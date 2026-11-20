// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a remodel lies against the live model it replaces: measured, never
//! typed.
//!
//! 0.69d0 placed most guns on their rig as 0.26g did, and a few otherwise
//! (the MP-153 2.2 cm lower). The live hands, sights and points are made for
//! the live placement, so a remodel is laid on the live mesh by a shift. The
//! rule, the same for every gun:
//!
//! * the shift is a trimmed nearest-point fit of the two bodies' vertices
//!   (up to [`SAMPLE`] of each, taken evenly; the nearest [`KEPT`] share of
//!   the pairs moves it, so a part one body has and the other lacks is left
//!   out), run until it settles;
//! * a shift under [`THRESHOLD`] is none: the remodel lies on the live model
//!   as it is;
//! * else it is applied, to 0.1 mm.
//!
//! The gap before and after (the median distance from a remodel vertex to
//! the live mesh) is reported with it: a gap left after the shift is a shape
//! of the gun's own (the Uzi's taller sights), never tuned away.

use std::collections::HashMap;

/// Vertices of each body the fit takes.
pub const SAMPLE: usize = 6000;
/// The nearest share of the pairs the shift follows.
pub const KEPT: f32 = 0.6;
/// A shift shorter than this (metres) is none.
pub const THRESHOLD: f32 = 0.001;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Fit {
    /// The shift that lays the remodel on the live mesh (metres, mesh
    /// space), as measured.
    pub measured: [f32; 3],
    /// The shift applied: `measured` to 0.1 mm, or none under the threshold.
    pub shift: [f32; 3],
    /// The median gap before and after `shift`, mm.
    pub gap_before_mm: f32,
    pub gap_after_mm: f32,
}

/// `points`, evenly thinned to [`SAMPLE`].
fn sample(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if points.len() <= SAMPLE {
        return points.to_vec();
    }
    (0..SAMPLE).map(|k| points[k * points.len() / SAMPLE]).collect()
}

/// Points in cells of 1 cm, for the nearest one.
struct Grid<'a> {
    points: &'a [[f32; 3]],
    cells: HashMap<[i32; 3], Vec<u32>>,
}

const CELL: f32 = 0.01;

fn cell(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c / CELL).floor() as i32)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
}

impl<'a> Grid<'a> {
    fn new(points: &'a [[f32; 3]]) -> Self {
        let mut cells: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            cells.entry(cell(*p)).or_default().push(i as u32);
        }
        Self { points, cells }
    }

    /// The point nearest `q` (the first of equals by index): among the
    /// cells around it, else, for a point with none within 3 cm, among all.
    fn nearest(&self, q: [f32; 3]) -> [f32; 3] {
        let home = cell(q);
        let mut best: Option<(f32, u32)> = None;
        let take = |i: u32, best: &mut Option<(f32, u32)>| {
            let d = distance(q, self.points[i as usize]);
            if best.is_none_or(|(b, at)| d < b || (d == b && i < at)) {
                *best = Some((d, i));
            }
        };
        const RINGS: i32 = 3;
        for dx in -RINGS..=RINGS {
            for dy in -RINGS..=RINGS {
                for dz in -RINGS..=RINGS {
                    let key = [home[0] + dx, home[1] + dy, home[2] + dz];
                    for &i in self.cells.get(&key).into_iter().flatten() {
                        take(i, &mut best);
                    }
                }
            }
        }
        // A point farther than the cells searched may have a nearer one outside them.
        if best.is_none_or(|(d, _)| d > RINGS as f32 * CELL) {
            for i in 0..self.points.len() as u32 {
                take(i, &mut best);
            }
        }
        self.points[best.map_or(0, |(_, i)| i as usize)]
    }
}

fn median(mut values: Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

fn gap(live: &Grid, ours: &[[f32; 3]], shift: [f32; 3]) -> f32 {
    median(
        ours.iter()
            .map(|q| {
                let q = [q[0] + shift[0], q[1] + shift[1], q[2] + shift[2]];
                distance(q, live.nearest(q))
            })
            .collect(),
    )
}

/// The fit of a remodel's vertices `ours` on the live model's `live` (both
/// in mesh space, bind pose).
pub fn fit(live: &[[f32; 3]], ours: &[[f32; 3]]) -> Fit {
    let (live, ours) = (sample(live), sample(ours));
    if live.is_empty() || ours.is_empty() {
        return Fit {
            measured: [0.0; 3],
            shift: [0.0; 3],
            gap_before_mm: 0.0,
            gap_after_mm: 0.0,
        };
    }
    let grid = Grid::new(&live);
    let mut t = [0.0f32; 3];
    for _ in 0..80 {
        let pairs: Vec<([f32; 3], f32)> = ours
            .iter()
            .map(|q| {
                let q = [q[0] + t[0], q[1] + t[1], q[2] + t[2]];
                let p = grid.nearest(q);
                ([p[0] - q[0], p[1] - q[1], p[2] - q[2]], distance(p, q))
            })
            .collect();
        let mut by_distance: Vec<f32> = pairs.iter().map(|(_, r)| *r).collect();
        by_distance.sort_by(f32::total_cmp);
        let limit = by_distance[((by_distance.len() - 1) as f32 * KEPT) as usize];
        let step: [f32; 3] = std::array::from_fn(|k| {
            median(pairs.iter().filter(|(_, r)| *r <= limit).map(|(d, _)| d[k]).collect())
        });
        for k in 0..3 {
            t[k] += step[k];
        }
        if step.iter().map(|c| c * c).sum::<f32>().sqrt() < 2e-5 {
            break;
        }
    }
    let length = t.iter().map(|c| c * c).sum::<f32>().sqrt();
    let shift = if length < THRESHOLD {
        [0.0; 3]
    } else {
        t.map(|c| (c * 1e4).round() / 1e4)
    };
    Fit {
        measured: t,
        shift,
        gap_before_mm: gap(&grid, &ours, [0.0; 3]) * 1000.0,
        gap_after_mm: gap(&grid, &ours, shift) * 1000.0,
    }
}

/// The median distance (mm) from `of`'s vertices to the nearest of `to`:
/// how well a part lies on a body.
pub fn median_gap_mm(of: &[[f32; 3]], to: &[[f32; 3]]) -> f32 {
    let (of, to) = (sample(of), sample(to));
    if of.is_empty() || to.is_empty() {
        return f32::INFINITY;
    }
    gap(&Grid::new(&to), &of, [0.0; 3]) * 1000.0
}

/// What one body has and the other lacks: the share of `of`'s vertices
/// farther than `limit` (metres) from every vertex of `from`, and the box
/// they lie in.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Apart {
    pub share: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

pub fn apart(of: &[[f32; 3]], from: &[[f32; 3]], limit: f32) -> Apart {
    let (of, from) = (sample(of), sample(from));
    if of.is_empty() || from.is_empty() {
        return Apart::default();
    }
    let grid = Grid::new(&from);
    let far: Vec<[f32; 3]> = of.iter().copied().filter(|p| distance(*p, grid.nearest(*p)) > limit).collect();
    let Some(first) = far.first() else {
        return Apart::default();
    };
    let (mut min, mut max) = (*first, *first);
    for p in &far {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    Apart {
        share: far.len() as f32 / of.len() as f32,
        min,
        max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bumpy sheet of points: no two alike under a small shift.
    fn sheet() -> Vec<[f32; 3]> {
        (0..40)
            .flat_map(|i| {
                (0..40).map(move |j| {
                    let (x, z) = (i as f32 * 0.004, j as f32 * 0.011);
                    [x, 0.02 * (37.0 * x).sin() * (23.0 * z).cos(), z]
                })
            })
            .collect()
    }

    #[test]
    fn a_body_on_its_live_model_is_not_shifted() {
        let live = sheet();
        let f = fit(&live, &live);
        assert_eq!(f.shift, [0.0; 3]);
        assert!(f.gap_after_mm < 1e-3);
    }

    #[test]
    fn a_body_placed_otherwise_is_shifted_back() {
        let live = sheet();
        let moved: Vec<[f32; 3]> = live.iter().map(|p| [p[0], p[1] + 0.0222, p[2] + 0.0012]).collect();
        let f = fit(&live, &moved);
        assert!((f.shift[1] + 0.0222).abs() < 2e-4, "{f:?}");
        assert!((f.shift[2] + 0.0012).abs() < 2e-4, "{f:?}");
        assert!(f.gap_before_mm > 1.0 && f.gap_after_mm < 0.2, "{f:?}");
    }

    #[test]
    fn a_part_only_one_body_has_does_not_move_it() {
        let live = sheet();
        let mut ours = live.clone();
        // A third more points far off (a suppressor the live gun lacks).
        ours.extend((0..500).map(|i| [0.3, 0.2 + i as f32 * 1e-4, -0.4]));
        assert_eq!(fit(&live, &ours).shift, [0.0; 3]);
    }
}
