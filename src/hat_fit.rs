// SPDX-License-Identifier: GPL-3.0-or-later
//! Fits a donor hat (headwear without eye holes, such as a knitted cap) onto
//! the native v0.26g head and over every stock mask worn under it.
//!
//! The donor modelled the hat on its own head, which stands higher and
//! further forward on the skeleton than the native one. Its face is in the
//! same surface, so the face is aligned onto the native head (translation
//! only, so the hat keeps its size), the hat follows it rigidly, and then the
//! hat is pushed outward wherever the native head or a mask would show
//! through it. A hat in the helmet slot is worn together with any mask, and
//! stock masks have no per-helmet variant, so it must cover all of them.
use crate::mask_fit::{
    Mesh, Shell, bone_origins, closest_on_triangle, dot, farthest_hit, parse_mesh, sub,
    weld_vertices,
};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

/// Gap kept between the native head and the hat.
pub const HEAD_CLEARANCE: f64 = 0.004;
/// Gap kept between a mask's outer skin and the hat over it.
pub const SHELL_CLEARANCE: f64 = 0.0015;
/// A hat vertex this far or less under a mask's surface is over the mask
/// and pushed out; deeper, the mask stands in front of the hat there.
pub const MASK_UNDER_DEPTH: f64 = 0.004;
/// A mask part farther than this outside the aligned hat is not cloth under
/// it: the fit would have to inflate the hat, so it fails instead.
pub const MAX_PUSH: f64 = 0.015;
/// The share of the face kept by each alignment step: the farthest fifth
/// (the jaw and nose, which differ most between the two heads) is ignored.
const ALIGN_KEEP: f64 = 0.8;
const ALIGN_STEPS: usize = 60;
const CONVERGED: f64 = 0.0002;
const PUSH_PASSES: usize = 40;

#[derive(Debug, Clone)]
pub struct HatShellFit {
    pub label: String,
    /// Mask vertices under the hat that were outside it (closer than the
    /// clearance to its surface, or through it), and the farthest (m),
    /// before and after the fit.
    pub outside_before: usize,
    pub depth_before: f64,
    pub outside_after: usize,
    pub depth_after: f64,
}

#[derive(Debug)]
pub struct HatFit {
    /// The rigid move of the hat with the donor face (m).
    pub offset: [f64; 3],
    /// Median distance of the aligned donor face from the native head (m).
    pub face_median: f64,
    pub pushed_vertices: usize,
    pub max_push: f64,
    pub shells: Vec<HatShellFit>,
}

/// Distance from `p` to the closest point of `mesh`, and that point.
fn closest_point(p: [f64; 3], mesh: &Mesh) -> (f64, [f64; 3]) {
    let mut best = (f64::INFINITY, p);
    for tri in &mesh.triangles {
        let [a, b, c] = tri.map(|v| mesh.positions[v]);
        let w = closest_on_triangle(p, a, b, c);
        let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * c[k]);
        let d = sub(p, q);
        let d = dot(d, d);
        if d < best.0 {
            best = (d, q);
        }
    }
    (best.0.sqrt(), best.1)
}

/// The translation that lays the donor `face` onto the native head: trimmed
/// point-to-surface ICP, starting from the eyes at the native eye height.
/// Returns the translation and the median remaining distance.
pub(crate) fn align_face(face: &[[f64; 3]], start: [f64; 3], head: &Mesh) -> ([f64; 3], f64) {
    let mut t = start;
    for _ in 0..ALIGN_STEPS {
        let mut pairs: Vec<(f64, [f64; 3])> = face
            .iter()
            .map(|p| {
                let moved = [0, 1, 2].map(|k| p[k] + t[k]);
                let (d, q) = closest_point(moved, head);
                (d, sub(q, moved))
            })
            .collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        pairs.truncate(((pairs.len() as f64) * ALIGN_KEEP).ceil() as usize);
        let n = pairs.len() as f64;
        let step = [0, 1, 2].map(|k| pairs.iter().map(|(_, d)| d[k]).sum::<f64>() / n);
        for k in 0..3 {
            t[k] += step[k];
        }
        if dot(step, step).sqrt() < 1e-6 {
            break;
        }
    }
    let mut d: Vec<f64> = face
        .iter()
        .map(|p| closest_point([0, 1, 2].map(|k| p[k] + t[k]), head).0)
        .collect();
    d.sort_by(f64::total_cmp);
    (t, d[d.len() / 2])
}

/// The point the fit's rays start from: the middle of the native head's
/// bounds at the native eye height, as `mask_fit` takes it.
pub(crate) fn head_centre(head: &Mesh, bind_pose: &[u8]) -> Result<[f64; 3]> {
    let bones = bone_origins(bind_pose)?;
    let eye = |n: &str| bones.get(n).copied().with_context(|| format!("no {n} bone"));
    let eyes_y = (eye("LeftEye")?[1] + eye("RightEye")?[1]) / 2.0;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &head.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    Ok([(lo[0] + hi[0]) / 2.0, eyes_y, (lo[2] + hi[2]) / 2.0])
}

fn ray(centre: [f64; 3], p: [f64; 3]) -> ([f64; 3], f64) {
    let d = sub(p, centre);
    let r = dot(d, d).sqrt();
    (d.map(|v| v / r), r)
}

/// Vertices of `inner` that the hat covers (a ray from `centre` through them
/// meets the hat) but that lie closer than `clearance` under its surface or
/// outside it; the count and the farthest one outside the clearance (m).
fn uncovered(
    hat: &[[f64; 3]],
    triangles: &[[usize; 3]],
    inner: &Mesh,
    centre: [f64; 3],
    clearance: f64,
) -> (usize, f64) {
    let (mut count, mut worst) = (0, 0.0_f64);
    for &p in &inner.positions {
        let (dir, r) = ray(centre, p);
        if let Some((t, _, _)) = farthest_hit(centre, dir, hat, triangles) {
            let pen = r + clearance - t;
            if pen > 1e-5 {
                count += 1;
                worst = worst.max(pen);
            }
        }
    }
    (count, worst)
}

/// Fits a converted hat surface (48-byte vertices) in place: moved with the
/// donor `face` (points in the hat's space) onto the native `head` (40-byte
/// vertices), then pushed out over the head and every shell in `masks`.
/// `start` seeds the alignment (the eyes at the native eye height).
pub fn fit_hat(
    hat: &mut [u8],
    face: &[[f64; 3]],
    start: [f64; 3],
    head: &[u8],
    bind_pose: &[u8],
    masks: &[Shell],
) -> Result<HatFit> {
    let mesh = parse_mesh(hat, 48, "hat")?;
    let head = parse_mesh(head, 40, "native head")?;
    let (offset, face_median) = align_face(face, start, &head);
    let centre = head_centre(&head, bind_pose)?;
    let (weld, neighbours) = weld_vertices(&mesh);
    let welded_count = neighbours.len();
    let start_positions: Vec<[f64; 3]> = mesh
        .positions
        .iter()
        .map(|p| [0, 1, 2].map(|k| p[k] + offset[k]))
        .collect();
    let mut positions = start_positions.clone();
    let before: Vec<(usize, f64)> = masks
        .iter()
        .map(|m| uncovered(&positions, &mesh.triangles, &m.mesh, centre, SHELL_CLEARANCE))
        .collect();
    // What the hat must stay outside of, each with its clearance.
    let mut inner: Vec<(&Mesh, f64)> = vec![(&head, HEAD_CLEARANCE)];
    inner.extend(masks.iter().map(|m| (&m.mesh, SHELL_CLEARANCE)));
    // Where each mask is under the hat, judged once on the aligned hat so
    // that pushing the hat out cannot draw it over a part in front of it.
    let under: Vec<Vec<bool>> = masks
        .iter()
        .map(|m| {
            positions
                .iter()
                .map(|&p| {
                    let (dir, r) = ray(centre, p);
                    farthest_hit(centre, dir, &m.mesh.positions, &m.mesh.triangles)
                        .is_some_and(|(t, _, _)| t + SHELL_CLEARANCE - r <= MASK_UNDER_DEPTH)
                })
                .collect()
        })
        .collect();
    let mut converged = false;
    for _ in 0..PUSH_PASSES {
        let mut need = vec![0.0_f64; welded_count];
        for (n, &(surface, clearance)) in inner.iter().enumerate() {
            let is_head = n == 0;
            // Hat vertices inside the surface (behind its farthest skin).
            // A mask's surface far in front of the hat is not under it (a
            // brow plate in front of the hem): the hem stays behind it.
            for (i, &p) in positions.iter().enumerate() {
                let (dir, r) = ray(centre, p);
                if let Some((t, _, _)) =
                    farthest_hit(centre, dir, &surface.positions, &surface.triangles)
                {
                    let depth = t + clearance - r;
                    if is_head || under[n - 1][i] {
                        let w = &mut need[weld[i]];
                        *w = w.max(depth);
                    }
                }
            }
            // Surface vertices poking between hat vertices.
            for &q in &surface.positions {
                let (dir, r) = ray(centre, q);
                if let Some((t, tri, bary)) =
                    farthest_hit(centre, dir, &positions, &mesh.triangles)
                {
                    let pen = r + clearance - t;
                    if pen > 0.0 {
                        for (corner, b) in mesh.triangles[tri].iter().zip(bary) {
                            let w = &mut need[weld[*corner]];
                            *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
                        }
                    }
                }
            }
        }
        if need.iter().all(|&n| n < CONVERGED) {
            converged = true;
            break;
        }
        // Spread each push to its neighbours: knit stretches smoothly over
        // what is under it instead of tenting at one vertex.
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
            let (dir, _) = ray(centre, *p);
            let push = need[weld[i]];
            for k in 0..3 {
                p[k] += dir[k] * push;
            }
        }
    }
    ensure!(
        converged,
        "the hat did not clear the native head and the masks after {PUSH_PASSES} passes"
    );
    let (mut max_push, mut pushed_vertices) = (0.0_f64, 0);
    let mut seen = BTreeSet::new();
    for (i, p) in positions.iter().enumerate() {
        let d = sub(*p, start_positions[i]);
        let push = dot(d, d).sqrt();
        max_push = max_push.max(push);
        if push > 1e-4 && seen.insert(weld[i]) {
            pushed_vertices += 1;
        }
    }
    ensure!(
        max_push <= MAX_PUSH,
        "the hat had to be pushed {:.1} mm out (more than {:.0} mm): something under it is not cloth; before the fit: {}",
        max_push * 1000.0,
        MAX_PUSH * 1000.0,
        masks
            .iter()
            .zip(&before)
            .map(|(m, (n, d))| format!("{} {n} up to {:.1} mm", m.label, d * 1000.0))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let shells = masks
        .iter()
        .zip(before)
        .map(|(m, (outside_before, depth_before))| {
            let (outside_after, depth_after) =
                uncovered(&positions, &mesh.triangles, &m.mesh, centre, SHELL_CLEARANCE * 0.5);
            HatShellFit {
                label: m.label.clone(),
                outside_before,
                depth_before,
                outside_after,
                depth_after,
            }
        })
        .collect();
    for (i, p) in positions.iter().enumerate() {
        for (k, v) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            hat[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    Ok(HatFit {
        offset,
        face_median,
        pushed_vertices,
        max_push,
        shells,
    })
}

/// How far `inner` stays under `outer` in one pose: both skinned from their
/// own bind frames to the pose (`bones` maps each model's bone indices to the
/// full rig's), then a ray from the posed head centre through every vertex
/// of `inner` that `outer` covers. Returns the least gap (m) and how many
/// vertices are outside.
#[allow(clippy::too_many_arguments)]
pub(crate) fn posed_gap(
    outer: &Posed<'_>,
    inner: &Posed<'_>,
    centre: [f64; 3],
    head_bone: usize,
    bind: &[crate::mask_fit::Frame],
    pose: &[crate::mask_fit::Frame],
) -> Result<(f64, usize)> {
    let skin = |m: &Posed<'_>| -> Result<Vec<[f64; 3]>> {
        let skins: Vec<crate::mask_fit::Skin> = m
            .skins
            .iter()
            .map(|s| s.iter().map(|&(b, w)| (m.bones[usize::from(b)], w)).collect())
            .collect();
        crate::mask_fit::skin_positions(&m.mesh.positions, &skins, bind, pose)
    };
    let posed_outer = skin(outer)?;
    let posed_inner = skin(inner)?;
    let posed_centre =
        crate::mask_fit::skin_positions(&[centre], &[vec![(head_bone as u8, 1.0)]], bind, pose)?[0];
    let (mut min, mut through) = (f64::INFINITY, 0);
    for &p in &posed_inner {
        let (dir, r) = ray(posed_centre, p);
        if let Some((t, _, _)) = farthest_hit(posed_centre, dir, &posed_outer, &outer.mesh.triangles)
        {
            min = min.min(t - r);
            through += usize::from(t < r);
        }
    }
    Ok((min, through))
}

/// A skinned surface for [`posed_gap`]: its mesh, each vertex's skin by its
/// own bone indices, and those indices in the full rig.
pub(crate) struct Posed<'a> {
    pub mesh: &'a Mesh,
    pub skins: Vec<crate::mask_fit::Skin>,
    pub bones: Vec<u8>,
}

impl<'a> Posed<'a> {
    /// `model_bind_pose` is the surface's own `render/bind_pose`; `rig_names`
    /// the full rig's bone names in order.
    pub(crate) fn new(
        mesh: &'a Mesh,
        model: &[u8],
        model_bind_pose: &[u8],
        rig_names: &[String],
    ) -> Result<Self> {
        let count = crate::mask_fit::read_u32(model, 8)?;
        let stride = (crate::mask_fit::read_u32(model, 4)? - 4) / count;
        let skins = crate::mask_fit::skins(model, stride)?;
        let names = crate::armor::bind_pose_origins(model_bind_pose, "surface bind pose")?;
        let bones = names
            .iter()
            .map(|(name, _)| {
                let at = rig_names
                    .iter()
                    .position(|n| n == name)
                    .with_context(|| format!("the rig has no bone {name}"))?;
                Ok(u8::try_from(at)?)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            mesh,
            skins,
            bones,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(c: [f64; 3], r: f64, segments: usize, rings: usize) -> Mesh {
        let mut positions = vec![];
        for i in 0..=rings {
            let theta = std::f64::consts::PI * i as f64 / rings as f64;
            for j in 0..segments {
                let phi = 2.0 * std::f64::consts::PI * j as f64 / segments as f64;
                positions.push([
                    c[0] + r * theta.sin() * phi.cos(),
                    c[1] + r * theta.cos(),
                    c[2] + r * theta.sin() * phi.sin(),
                ]);
            }
        }
        let mut triangles = vec![];
        for i in 0..rings {
            for j in 0..segments {
                let a = i * segments + j;
                let b = i * segments + (j + 1) % segments;
                let (c2, d) = (a + segments, b + segments);
                triangles.push([a, c2, b]);
                triangles.push([b, c2, d]);
            }
        }
        Mesh {
            positions,
            triangles,
        }
    }

    #[test]
    fn the_face_alignment_finds_a_pure_shift() {
        let head = sphere([0.0, 1.6, 0.0], 0.09, 24, 12);
        let shift = [0.004, 0.038, 0.04];
        let face: Vec<[f64; 3]> = head
            .positions
            .iter()
            .filter(|p| p[2] > 0.02)
            .map(|p| [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]])
            .collect();
        let (t, median) = align_face(&face, [0.0, -0.03, 0.0], &head);
        for k in 0..3 {
            assert!((t[k] + shift[k]).abs() < 2e-3, "{t:?}");
        }
        assert!(median < 2e-3, "{median}");
    }

    #[test]
    fn uncovered_counts_what_pokes_through() {
        let hat = sphere([0.0, 0.0, 0.0], 0.1, 16, 8);
        let inner = sphere([0.0, 0.0, 0.0], 0.095, 8, 4);
        let (n, _) = uncovered(&hat.positions, &hat.triangles, &inner, [0.0; 3], 0.002);
        assert_eq!(n, 0);
        let (n, worst) = uncovered(&hat.positions, &hat.triangles, &inner, [0.0; 3], 0.01);
        assert!(n > 0 && worst > 0.004 && worst < 0.006, "{n} {worst}");
    }
}
