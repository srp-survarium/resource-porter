// SPDX-License-Identifier: GPL-3.0-or-later
//! X-Ray detail objects as the target's grass: where each tuft stands, and
//! the tuft itself as a grass model.
//!
//! X-Ray stores no positions. `CDetailManager::cache_Decompress`
//! (`xrRender/DetailManager_Decompress.cpp`) regenerates every 2 m slot when
//! the camera comes near: a jittered grid of candidates, each kept where the
//! slot's corner densities, bilinearly interpolated and ordered-dithered,
//! beat a 16x16 threshold matrix; one of the slot's four objects is picked
//! among those that pass; the candidate drops straight down onto the highest
//! static collision face inside the slot's height range. The random streams
//! are seeded by the slot's coordinates, so the result is the same every
//! time. [`place`] runs that procedure once, at the engine's default
//! `r__detail_density` unless the recipe says otherwise, and the target's
//! grass system then draws the baked positions (`.veg` instances).
//!
//! What does not carry over: X-Ray's per-slot `c_dir`/`c_hemi` (baked sun
//! and sky visibility) and `c_r/g/b` tint. A target grass instance has one
//! hard-coded colour (`grass_world::grass_layer_resources_ready` passes
//! 200,200,200) and the `.veg` file has no colour field; the target lights
//! grass with its own sun shadows and skylight instead.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};

use super::surface::{STATIC_VERTEX_DECLARATION, packed_basis};
use crate::soc::codec::cform::Cform;
use crate::soc::codec::details::{DetailObject, Details, EMPTY, SLOT_SIZE};

/// `ps_r__Detail_density`'s default: metres between candidate rows.
pub const DEFAULT_DENSITY: f32 = 0.3;
/// `EPS_L`, the margin `cache_Task` grows a slot's box by.
const EPS_L: f32 = 0.001;

/// `CRandom` (`xrCore/_random.h`): the MSVC `rand()` generator.
#[derive(Clone, Copy, Debug)]
pub struct XRayRandom(i32);

impl XRayRandom {
    pub fn new(seed: i32) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.0 >> 16) & 0x7fff
    }

    pub fn below(&mut self, max: i32) -> i32 {
        self.next() % max
    }

    pub fn unit(&mut self) -> f32 {
        self.next() as f32 / 32767.0
    }

    /// `randFs(range)`: uniform in `[-range, range]`.
    pub fn signed(&mut self, range: f32) -> f32 {
        -range + self.unit() * (range + range)
    }
}

/// `bwdithermap(2, dither)`.
pub fn dither_matrix() -> [[i32; 16]; 16] {
    const MAGIC: [[i32; 4]; 4] = [[0, 14, 3, 13], [11, 5, 8, 6], [12, 2, 15, 1], [7, 9, 4, 10]];
    let n = 255.0_f32;
    let factor = (n - 1.0) / 16.0;
    let mut out = [[0; 16]; 16];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                for l in 0..4 {
                    out[4 * k + i][4 * l + j] =
                        (0.5 + MAGIC[i][j] as f32 * factor + (MAGIC[k][l] as f32 / 16.0) * factor)
                            as i32;
                }
            }
        }
    }
    out
}

/// `Interpolate` in `DetailManager_Decompress.cpp`.
fn interpolate(base: &[f32; 4], x: u32, y: u32, size: u32) -> f32 {
    let f = size as f32;
    let fx = x as f32 / f;
    let ifx = 1.0 - fx;
    let fy = y as f32 / f;
    let ify = 1.0 - fy;
    let c01 = base[0] * ifx + base[1] * fx;
    let c23 = base[2] * ifx + base[3] * fx;
    let c02 = base[0] * ify + base[2] * fy;
    let c13 = base[1] * ify + base[3] * fy;
    let cx = ify * c01 + fy * c23;
    let cy = ifx * c02 + fx * c13;
    (cx + cy) / 2.0
}

/// The static collision faces a detail lands on: the faces
/// `CDB::TestRayTri(..., bCull = TRUE)` hits from above, i.e. whose
/// `(v1 - v0) x (v2 - v0)` points up. Passable faces are expected to be
/// gone already (`SGameMtl::flPassable`), as the map build drops them.
pub struct Ground {
    triangles: Vec<[[f32; 3]; 3]>,
    grid: BTreeMap<(i32, i32), Vec<u32>>,
}

const GROUND_CELL: f32 = SLOT_SIZE;

impl Ground {
    pub fn from_cform(cform: &Cform) -> Self {
        Self::new(
            cform
                .faces
                .iter()
                .map(|face| face.vertices.map(|index| cform.vertices[index as usize]))
                .collect(),
        )
    }

    pub fn new(triangles: Vec<[[f32; 3]; 3]>) -> Self {
        let triangles: Vec<_> = triangles.into_iter().filter(|t| facing_up(t)).collect();
        let mut grid: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
        for (index, t) in triangles.iter().enumerate() {
            let cell = |axis: usize, pick: fn(f32, f32) -> f32| {
                (t.iter().map(|v| v[axis]).fold(t[0][axis], pick) / GROUND_CELL).floor() as i32
            };
            for x in cell(0, f32::min)..=cell(0, f32::max) {
                for z in cell(2, f32::min)..=cell(2, f32::max) {
                    grid.entry((x, z)).or_default().push(index as u32);
                }
            }
        }
        Self { triangles, grid }
    }

    /// The highest face straight below `(x, top, z)`, if it is not below
    /// `bottom`.
    pub fn drop_onto(&self, x: f32, z: f32, top: f32, bottom: f32) -> Option<f32> {
        let key = (
            (x / GROUND_CELL).floor() as i32,
            (z / GROUND_CELL).floor() as i32,
        );
        let best = self
            .grid
            .get(&key)?
            .iter()
            .filter_map(|&i| height_at(&self.triangles[i as usize], x, z))
            .filter(|&y| y <= top)
            .reduce(f32::max)?;
        (best >= bottom).then_some(best)
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn facing_up(t: &[[f32; 3]; 3]) -> bool {
    cross(sub(t[1], t[0]), sub(t[2], t[0]))[1] > 1e-12
}

fn height_at(t: &[[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = *t;
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-12 {
        return None;
    }
    let w0 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let w1 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let w2 = 1.0 - w0 - w1;
    const EDGE: f32 = -1e-5;
    (w0 >= EDGE && w1 >= EDGE && w2 >= EDGE).then(|| w0 * a[1] + w1 * b[1] + w2 * c[1])
}

/// One detail standing on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub object: u8,
    pub position: [f32; 3],
}

/// Every detail X-Ray would draw, at `density` metres between candidates.
pub fn place(details: &Details, ground: &Ground, density: f32) -> Result<Vec<Placement>> {
    ensure!(
        density > 0.0 && density < SLOT_SIZE,
        "detail density {density} m is not in (0, 2)"
    );
    let dither = dither_matrix();
    let jitter = density / 1.7;
    let d_size = (SLOT_SIZE / density).ceil() as u32;
    let mut out = Vec::new();
    for z in 0..details.size[1] {
        for x in 0..details.size[0] {
            let (slot, min) = details.slot(x, z);
            if slot.is_empty() {
                continue;
            }
            let [sx, sz] = details.world_slot(x, z);
            let bottom = slot.y_base() - EPS_L;
            let top = slot.y_base() + slot.y_height() + EPS_L;
            let min = [min[0] - EPS_L, min[1] - EPS_L];
            let alpha: [[f32; 4]; 4] = std::array::from_fn(|part| {
                slot.densities(part).map(|a| 255.0 * f32::from(a) / 15.0)
            });
            let seed = (0x1207_1980_u32 ^ (sx.wrapping_mul(sz) as u32)) as i32;
            let mut selection = XRayRandom::new(seed);
            let mut jitter_random = XRayRandom::new(seed);
            for cz in 0..=d_size {
                for cx in 0..=d_size {
                    let shift_x = jitter_random.below(16) as u32;
                    let shift_z = jitter_random.below(16) as u32;
                    let mut selected = [0_usize; 4];
                    let mut count = 0;
                    for part in 0..4 {
                        if slot.id(part) == EMPTY {
                            continue;
                        }
                        let (dx, dz) = (cx.min(d_size - 1), cz.min(d_size - 1));
                        let c = ((interpolate(&alpha[part], dx, dz, d_size) + 0.5).floor() as i32)
                            .clamp(0, 255);
                        if c > dither[((dx + shift_x) % 16) as usize]
                            [((dz + shift_z) % 16) as usize]
                        {
                            selected[count] = part;
                            count += 1;
                        }
                    }
                    if count == 0 {
                        continue;
                    }
                    let part = if count == 1 {
                        selected[0]
                    } else {
                        selected[selection.below(count as i32) as usize]
                    };
                    // `Fvector::set(x, y, z)`'s two jitter draws are in
                    // unspecified order in C++; x first here.
                    let px = cx as f32 / d_size as f32 * SLOT_SIZE
                        + min[0]
                        + jitter_random.signed(jitter);
                    let pz = cz as f32 / d_size as f32 * SLOT_SIZE
                        + min[1]
                        + jitter_random.signed(jitter);
                    let Some(py) = ground.drop_onto(px, pz, top, bottom) else {
                        continue;
                    };
                    out.push(Placement {
                        object: slot.id(part),
                        position: [px, py, pz],
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The scale X-Ray draws a detail at, `randF(min * 0.5, max * 0.9)`, as the
/// target layer's base scale and absolute random delta
/// (`select_model_scale`: `base - delta + rand(2 * delta)`).
pub fn layer_scale(object: &DetailObject) -> (f32, f32) {
    let (low, high) = (object.min_scale * 0.5, object.max_scale * 0.9);
    ((low + high) / 2.0, (high - low).max(0.0) / 2.0)
}

/// One LOD of a converted detail model: 32-byte static vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct GrassMesh {
    pub vertices: Vec<[u8; 32]>,
    pub indices: Vec<u16>,
    pub bounds: ([f32; 3], [f32; 3]),
}

impl GrassMesh {
    /// The `converted_model` payload (vertex chunk 3, index chunk 4).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let vertex_chunk = STATIC_VERTEX_DECLARATION.len() + 4 + self.vertices.len() * 32;
        let mut out = Vec::new();
        out.extend(3_u32.to_le_bytes());
        out.extend(u32::try_from(vertex_chunk)?.to_le_bytes());
        out.extend(STATIC_VERTEX_DECLARATION);
        out.extend(u32::try_from(self.vertices.len())?.to_le_bytes());
        for v in &self.vertices {
            out.extend(v);
        }
        out.extend(4_u32.to_le_bytes());
        out.extend(u32::try_from(4 + self.indices.len() * 2)?.to_le_bytes());
        out.extend(u32::try_from(self.indices.len())?.to_le_bytes());
        for i in &self.indices {
            out.extend(i.to_le_bytes());
        }
        Ok(out)
    }
}

/// A detail model as the target's two grass LODs.
///
/// Normals are X-Ray's own: its detail shaders (`deffer_detail_w_flat.vs`)
/// light a vertex along `normalize(P - base + (0, 0.75, 0))`, outward from
/// the tuft's foot and mostly up, at the scale the tuft is drawn
/// ([`layer_scale`]). A card's face normal instead lies nearly flat, and
/// under a high sun the tufts go black. Tangent and binormal follow the
/// texture axes. LOD 1 keeps the larger half of the model's cards (its
/// connected pieces), as the native grass drops cards with distance; a
/// one-card model keeps its card.
pub fn grass_lods(object: &DetailObject) -> Result<[GrassMesh; 2]> {
    let triangles: Vec<[usize; 3]> = object
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]].map(usize::from))
        .collect();
    if triangles.is_empty() {
        bail!("detail model has no triangles");
    }
    let scale = layer_scale(object).0;
    let lod0 = mesh(object, &triangles, scale)?;

    // Cards: triangles joined by shared vertices.
    let mut parent: Vec<usize> = (0..object.vertices.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for t in &triangles {
        for corner in 1..3 {
            let (a, b) = (root(&mut parent, t[0]), root(&mut parent, t[corner]));
            parent[a] = b;
        }
    }
    let mut cards: BTreeMap<usize, (f32, Vec<[usize; 3]>)> = BTreeMap::new();
    for t in &triangles {
        let p = t.map(|i| object.vertices[i].position);
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let card = cards.entry(root(&mut parent, t[0])).or_default();
        card.0 += (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        card.1.push(*t);
    }
    let mut cards: Vec<(f32, Vec<[usize; 3]>)> = cards.into_values().collect();
    let keep = cards.len().div_ceil(2);
    // Largest first; ties keep file order (stable sort).
    cards.sort_by(|a, b| b.0.total_cmp(&a.0));
    let lod1_triangles: Vec<[usize; 3]> = cards.into_iter().take(keep).flat_map(|c| c.1).collect();
    let lod1 = mesh(object, &lod1_triangles, scale)?;
    Ok([lod0, lod1])
}

fn normalized(v: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-12 {
        v.map(|c| c / length)
    } else {
        fallback
    }
}

/// The vertices `triangles` use, compacted, with normals and texture axes.
fn mesh(object: &DetailObject, triangles: &[[usize; 3]], scale: f32) -> Result<GrassMesh> {
    let mut remap: BTreeMap<usize, u16> = BTreeMap::new();
    for t in triangles {
        for &i in t {
            let next =
                u16::try_from(remap.len()).context("detail model exceeds 65,536 vertices")?;
            remap.entry(i).or_insert(next);
        }
    }
    let mut order = vec![0_usize; remap.len()];
    for (&source, &target) in &remap {
        order[usize::from(target)] = source;
    }
    let mut tangent = vec![[0.0_f32; 3]; order.len()];
    let mut binormal = vec![[0.0_f32; 3]; order.len()];
    for t in triangles {
        let v = t.map(|i| &object.vertices[i]);
        let (e1, e2) = (
            sub(v[1].position, v[0].position),
            sub(v[2].position, v[0].position),
        );
        let (du1, dv1) = (v[1].uv[0] - v[0].uv[0], v[1].uv[1] - v[0].uv[1]);
        let (du2, dv2) = (v[2].uv[0] - v[0].uv[0], v[2].uv[1] - v[0].uv[1]);
        let det = du1 * dv2 - du2 * dv1;
        let (t_dir, b_dir) = if det.abs() > 1e-12 {
            let r = 1.0 / det;
            (
                std::array::from_fn(|k| (e1[k] * dv2 - e2[k] * dv1) * r),
                std::array::from_fn(|k| (e2[k] * du1 - e1[k] * du2) * r),
            )
        } else {
            ([0.0; 3], [0.0; 3])
        };
        for &i in t {
            let slot = usize::from(remap[&i]);
            for k in 0..3 {
                tangent[slot][k] += t_dir[k];
                binormal[slot][k] += b_dir[k];
            }
        }
    }
    let mut bounds = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let vertices = order
        .iter()
        .enumerate()
        .map(|(slot, &source)| {
            let v = &object.vertices[source];
            for k in 0..3 {
                bounds.0[k] = bounds.0[k].min(v.position[k]);
                bounds.1[k] = bounds.1[k].max(v.position[k]);
            }
            let p = v.position;
            let n = normalized(
                [p[0] * scale, p[1] * scale + 0.75, p[2] * scale],
                [0.0, 1.0, 0.0],
            );
            // Gram-Schmidt against the normal; a degenerate mapping falls
            // back to any perpendicular pair.
            let project = |a: [f32; 3]| {
                let d = a[0] * n[0] + a[1] * n[1] + a[2] * n[2];
                [a[0] - n[0] * d, a[1] - n[1] * d, a[2] - n[2] * d]
            };
            let t = normalized(
                project(tangent[slot]),
                normalized(cross([0.0, 0.0, 1.0], n), [1.0, 0.0, 0.0]),
            );
            let b = normalized(project(binormal[slot]), cross(n, t));
            let mut out = [0_u8; 32];
            for k in 0..3 {
                out[4 * k..4 * k + 4].copy_from_slice(&v.position[k].to_le_bytes());
            }
            out[12..16].copy_from_slice(&packed_basis(n, 127).to_le_bytes());
            out[16..20].copy_from_slice(&packed_basis(t, 255).to_le_bytes());
            out[20..24].copy_from_slice(&packed_basis(b, 255).to_le_bytes());
            out[24..28].copy_from_slice(&v.uv[0].to_le_bytes());
            out[28..32].copy_from_slice(&v.uv[1].to_le_bytes());
            out
        })
        .collect();
    let indices = triangles
        .iter()
        .flat_map(|t| t.map(|i| remap[&i]))
        .collect();
    Ok(GrassMesh {
        vertices,
        indices,
        bounds,
    })
}

/// A `.veg` instance file the target grass cook reads: positions (`0x58`)
/// and normals (`0x59`) as float3 arrays. The editor paint mask the retail
/// files also carry is not read at run time and is left out.
pub fn encode_vegetation(positions: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + positions.len() * 24);
    for (chunk, normal) in [(0x58_u32, false), (0x59_u32, true)] {
        out.extend(chunk.to_le_bytes());
        out.extend(
            u32::try_from(positions.len() * 12)
                .expect("veg size")
                .to_le_bytes(),
        );
        for p in positions {
            let value = if normal { [0.0, 1.0, 0.0] } else { *p };
            for c in value {
                out.extend(c.to_le_bytes());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::codec::details::{self, DetailSlot, DetailVertex};

    #[test]
    fn random_is_msvc_rand() {
        // `srand(1); rand()...` on MSVC.
        let mut r = XRayRandom::new(1);
        assert_eq!(
            [r.next(), r.next(), r.next(), r.next(), r.next()],
            [41, 18467, 6334, 26500, 19169]
        );
    }

    #[test]
    fn dither_spans_the_byte_range() {
        let d = dither_matrix();
        assert_eq!(d[0][0], 0);
        let max = d.iter().flatten().max().unwrap();
        // Below 255, so a full density keeps every candidate.
        assert_eq!(*max, 253);
    }

    fn plane(y: f32) -> Ground {
        // Two upward triangles covering -10..10 (counter-clockwise seen from
        // below, which is X-Ray's front face for a downward ray).
        let (a, b, c, d) = (
            [-10.0, y, -10.0],
            [-10.0, y, 10.0],
            [10.0, y, 10.0],
            [10.0, y, -10.0],
        );
        Ground::new(vec![[a, b, c], [a, c, d]])
    }

    fn details_with(slots: Vec<DetailSlot>, size: [u32; 2]) -> Details {
        details::parse(&details::tests::encode([1, 1], size, 0, &slots)).unwrap()
    }

    #[test]
    fn a_full_slot_fills_every_candidate_on_the_ground() {
        let details = details_with(vec![details::tests::full_slot(0, -1.0, 3.0)], [1, 1]);
        let placed = place(&details, &plane(0.5), DEFAULT_DENSITY).unwrap();
        // ceil(2 / 0.3) = 7: an 8 x 8 candidate grid, all above the threshold.
        assert_eq!(placed.len(), 64);
        for p in &placed {
            assert_eq!(p.object, 0);
            assert!((p.position[1] - 0.5).abs() < 1e-5);
            // Slot (0,0) in the file is world slot (-1,-1): x, z in -2..0,
            // plus up to 0.3 / 1.7 m of jitter.
            for axis in [0, 2] {
                assert!(p.position[axis] > -2.2 && p.position[axis] < 0.2, "{p:?}");
            }
        }
        // Deterministic: the same slot gives the same tufts.
        assert_eq!(
            placed,
            place(&details, &plane(0.5), DEFAULT_DENSITY).unwrap()
        );
    }

    #[test]
    fn density_thins_and_the_height_range_bounds_the_drop() {
        let mut half = details::tests::full_slot(0, -1.0, 3.0);
        half.palette[0] = 0x7777;
        let details = details_with(vec![half], [1, 1]);
        let n = place(&details, &plane(0.0), DEFAULT_DENSITY).unwrap().len();
        assert!(n > 20 && n < 44, "{n}");
        // Ground below the slot's range, or a face turned down, holds nothing.
        assert!(
            place(&details, &plane(-5.0), DEFAULT_DENSITY)
                .unwrap()
                .is_empty()
        );
        let (a, b, c) = ([-10.0, 0.0, -10.0], [10.0, 0.0, 10.0], [-10.0, 0.0, 10.0]);
        assert!(
            place(&details, &Ground::new(vec![[a, b, c]]), DEFAULT_DENSITY)
                .unwrap()
                .is_empty()
        );
        // The highest face under the slot's top wins: a floor 1 m up.
        let two = Ground::new(vec![
            [[-10.0, 0.0, -10.0], [-10.0, 0.0, 10.0], [10.0, 0.0, 10.0]],
            [[-10.0, 1.0, -10.0], [-10.0, 1.0, 10.0], [10.0, 1.0, 10.0]],
            [[-10.0, 0.0, -10.0], [10.0, 0.0, 10.0], [10.0, 0.0, -10.0]],
            [[-10.0, 1.0, -10.0], [10.0, 1.0, 10.0], [10.0, 1.0, -10.0]],
        ]);
        let placed = place(&details, &two, DEFAULT_DENSITY).unwrap();
        assert!(!placed.is_empty() && placed.iter().all(|p| (p.position[1] - 1.0).abs() < 1e-5));
    }

    #[test]
    fn the_scale_range_is_xrays() {
        let details = details_with(vec![details::tests::full_slot(0, 0.0, 1.0)], [1, 1]);
        // min 1, max 2: X-Ray draws 0.5 .. 1.8.
        let (base, delta) = layer_scale(&details.objects[0]);
        assert!((base - 1.15).abs() < 1e-6 && (delta - 0.65).abs() < 1e-6);
    }

    #[test]
    fn models_keep_geometry_and_lod1_drops_the_smaller_cards() {
        let quad = |x: f32, size: f32| -> Vec<DetailVertex> {
            [[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]]
                .into_iter()
                .map(|[u, y]| DetailVertex {
                    position: [x + u, y, 0.0],
                    uv: [u / size, 1.0 - y / size],
                })
                .collect()
        };
        let mut vertices = quad(0.0, 1.0);
        vertices.extend(quad(2.0, 0.5));
        vertices.extend(quad(4.0, 0.8));
        let indices = (0..3_u16)
            .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| i + 4 * q))
            .collect();
        let object = DetailObject {
            shader: "details\\blend".into(),
            texture: "build_details".into(),
            flags: 0,
            min_scale: 1.0,
            max_scale: 1.0,
            vertices,
            indices,
        };
        let [lod0, lod1] = grass_lods(&object).unwrap();
        assert_eq!((lod0.vertices.len(), lod0.indices.len()), (12, 18));
        // Two of three cards: the 1 m and the 0.8 m one.
        assert_eq!((lod1.vertices.len(), lod1.indices.len()), (8, 12));
        assert_eq!(lod1.bounds.1[0], 4.8);
        // X-Ray's normal at the foot is straight up (scale 0.5 .. 0.9: the
        // vertex at the origin), packed as (x, y, z) -> bits 16, 8, 0.
        let n = u32::from_le_bytes(lod0.vertices[0][12..16].try_into().unwrap());
        assert_eq!(
            (n >> 24, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff),
            (127, 127, 255, 127)
        );
        // A tip leans out: (1, 1, 0) * 0.7 + (0, 0.75, 0).
        let n = u32::from_le_bytes(lod0.vertices[2][12..16].try_into().unwrap());
        assert!((n >> 16) & 0xff > 180 && (n >> 8) & 0xff > 200, "{n:x}");
        let bytes = lod0.encode().unwrap();
        assert_eq!(&bytes[8..56], &STATIC_VERTEX_DECLARATION);
        assert_eq!(bytes.len(), 8 + 48 + 4 + 12 * 32 + 12 + 18 * 2);
    }

    #[test]
    fn vegetation_files_carry_positions_and_up_normals() {
        let bytes = encode_vegetation(&[[1.0, 2.0, 3.0]]);
        assert_eq!(&bytes[..8], [0x58_u32, 12].map(u32::to_le_bytes).concat());
        assert_eq!(
            &bytes[20..28],
            [0x59_u32, 12].map(u32::to_le_bytes).concat()
        );
        assert_eq!(
            &bytes[8..20],
            [1.0_f32, 2.0, 3.0].map(f32::to_le_bytes).concat()
        );
        assert_eq!(
            &bytes[28..40],
            [0.0_f32, 1.0, 0.0].map(f32::to_le_bytes).concat()
        );
    }
}
