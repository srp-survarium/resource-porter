// SPDX-License-Identifier: GPL-3.0-or-later
//! Find places on a level's collision mesh where a player can stand.
//!
//! A converted map inherits no spawn points: Shadow of Chernobyl keeps them in
//! `level.spawn` as engine-specific server objects, and nothing in the target
//! build can read those. Picking the centre of the map's bounding box instead
//! is what the ported maps did, and for `mp_pool` the centre of the box is the
//! swimming pool -- the player spawns in the water, in the dark, at the bottom.
//!
//! So the ground is measured rather than guessed. The collision mesh already
//! says where a player can stand: an upward-facing surface with room above it.

use std::collections::BTreeMap;

use crate::soc::codec::cform::Cform;

/// A place a player could be put down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Where to spawn: on the surface, plus [`STANDING_CLEARANCE`].
    pub position: [f32; 3],
    /// Total upward-facing area in the cell, in square units.
    ///
    /// Larger is safer: a wide flat cell is a floor or a courtyard, a small one
    /// might be a windowsill or the top of a crate.
    pub area: f32,
    /// Vertical room above the surface before the next collision above it.
    pub headroom: f32,
}

/// How far above the surface a spawn is placed, so a player is not born
/// intersecting the floor.
pub const STANDING_CLEARANCE: f32 = 0.5;

/// A player is about this tall; less room than this is a crawlspace.
pub const REQUIRED_HEADROOM: f32 = 2.0;

/// Only near-horizontal surfaces can be stood on. `0.85` is roughly a
/// 32-degree slope: walkable, but not a wall or a steep roof.
const MIN_UP: f32 = 0.85;

#[derive(Default)]
struct Cell {
    /// Upward-facing area, and the running sum of surface centres.
    area: f32,
    sum_x: f32,
    sum_z: f32,
    samples: f32,
    /// Every surface height in the cell, whichever way it faces: the ones we
    /// cannot stand on are still things to hit your head on.
    heights: Vec<f32>,
}

/// Rank standable places on the collision mesh, widest ground first.
///
/// `cell` is the grid resolution in world units; `2.0` gives cells a player
/// fits in comfortably.
pub fn standable(cform: &Cform, cell: f32, limit: usize) -> Vec<Candidate> {
    assert!(cell > 0.0, "cell size must be positive");

    let mut cells: BTreeMap<(i64, i64), Cell> = BTreeMap::new();
    for face in &cform.faces {
        let [a, b, c] = face.vertices.map(|index| cform.vertices[index as usize]);
        let edge0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let edge1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let normal = [
            edge0[1] * edge1[2] - edge0[2] * edge1[1],
            edge0[2] * edge1[0] - edge0[0] * edge1[2],
            edge0[0] * edge1[1] - edge0[1] * edge1[0],
        ];
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if !length.is_normal() {
            continue;
        }

        let centre = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let entry = cells
            .entry((
                (centre[0] / cell).floor() as i64,
                (centre[2] / cell).floor() as i64,
            ))
            .or_default();
        entry.heights.push(centre[1]);

        // A cform carries no winding guarantee, so the normal is taken
        // unsigned: a floor is a floor from either side.
        if (normal[1] / length).abs() >= MIN_UP {
            entry.area += length / 2.0;
            entry.sum_x += centre[0];
            entry.sum_z += centre[2];
            entry.samples += 1.0;
        }
    }

    let mut ranked: Vec<Candidate> = cells
        .into_values()
        .filter(|cell| cell.samples > 0.0)
        .filter_map(|mut cell| {
            cell.heights.sort_by(|left, right| {
                left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
            });
            // Stand on the lowest surface in the cell with room above it: the
            // floor of a room, not the roof over it.
            let (floor, headroom) = cell.heights.iter().find_map(|floor| {
                let headroom = cell
                    .heights
                    .iter()
                    .find(|above| **above - floor > 0.1)
                    .map_or(f32::INFINITY, |above| above - floor);
                (headroom >= REQUIRED_HEADROOM).then_some((*floor, headroom))
            })?;
            Some(Candidate {
                position: [
                    cell.sum_x / cell.samples,
                    floor + STANDING_CLEARANCE,
                    cell.sum_z / cell.samples,
                ],
                area: cell.area,
                headroom,
            })
        })
        .collect();

    ranked.sort_by(|left, right| {
        right
            .area
            .partial_cmp(&left.area)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(limit);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::codec::cform::{CformFace, CformHeader};

    fn quad(
        vertices: &mut Vec<[f32; 3]>,
        faces: &mut Vec<CformFace>,
        (x0, z0): (f32, f32),
        (x1, z1): (f32, f32),
        y: f32,
    ) {
        let base = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        for triangle in [[base, base + 1, base + 2], [base, base + 2, base + 3]] {
            faces.push(CformFace {
                vertices: triangle,
                material: 0,
                suppress_shadows: false,
                suppress_wallmarks: false,
                sector: 0,
            });
        }
    }

    /// A wide deck at y=0 away from the origin, and a small deep pit at the
    /// centre -- the shape that made the old centre-of-the-box spawn drop the
    /// player into `mp_pool`'s water.
    fn pool_like() -> Cform {
        let mut vertices = Vec::new();
        let mut faces = Vec::new();
        quad(&mut vertices, &mut faces, (20.0, 20.0), (40.0, 40.0), 0.0);
        quad(&mut vertices, &mut faces, (-2.0, -2.0), (2.0, 2.0), -5.0);
        Cform {
            header: CformHeader {
                version: 4,
                vertex_count: vertices.len() as u32,
                face_count: faces.len() as u32,
                bounding_box: ([-2.0, -5.0, -2.0], [40.0, 0.0, 40.0]),
            },
            vertices,
            faces,
        }
    }

    #[test]
    fn prefers_the_widest_floor_over_the_pit() {
        let best = *standable(&pool_like(), 2.0, 4)
            .first()
            .expect("a candidate");
        assert!(best.position[1] > -1.0, "spawned in the pit: {best:?}");
        assert!(best.position[0] > 10.0, "not on the deck: {best:?}");
        assert!(best.position[2] > 10.0, "not on the deck: {best:?}");
    }

    #[test]
    fn stands_clear_of_the_surface() {
        assert_eq!(
            standable(&pool_like(), 2.0, 1)[0].position[1],
            STANDING_CLEARANCE
        );
    }

    /// A floor with a ceiling too low to stand under is not a spawn.
    #[test]
    fn rejects_a_crawlspace() {
        let mut vertices = Vec::new();
        let mut faces = Vec::new();
        quad(&mut vertices, &mut faces, (0.0, 0.0), (10.0, 10.0), 0.0);
        quad(&mut vertices, &mut faces, (0.0, 0.0), (10.0, 10.0), 1.0);
        let cform = Cform {
            header: CformHeader {
                version: 4,
                vertex_count: vertices.len() as u32,
                face_count: faces.len() as u32,
                bounding_box: ([0.0; 3], [10.0, 1.0, 10.0]),
            },
            vertices,
            faces,
        };
        // The lower floor is unusable, so the only spawn is on top of the slab.
        let ranked = standable(&cform, 64.0, 4);
        assert!(
            ranked.iter().all(|c| c.headroom >= REQUIRED_HEADROOM),
            "{ranked:?}"
        );
        assert_eq!(ranked[0].position[1], 1.0 + STANDING_CLEARANCE);
    }
}
