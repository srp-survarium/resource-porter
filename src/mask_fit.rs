// SPDX-License-Identifier: GPL-3.0-or-later
//! Fits donor headwear onto the native v0.26g head: the eye holes are moved
//! over the native eyes, the part below the jaw is tucked onto the native
//! neck, then the cloth is pushed outward wherever the native head would show
//! through it.
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The eye holes sit on the cornea, this far ahead of the native eye bones
/// (measured from the iris texels of base_character_head).
const CORNEA_AHEAD_OF_EYE_BONE: f64 = 0.0155;
/// Gap kept between the native head surface and the fitted mask.
const CLEARANCE: f64 = 0.004;
/// Tighter gap on the neck so the hem stays under stock collars.
const NECK_CLEARANCE: f64 = 0.003;
/// Below this height above the Neck bone the mask lies fully on the neck.
const TUCK_FULL_ABOVE_NECK: f64 = 0.05;
const CONVERGED: f64 = 0.0003;

pub struct MaskFit {
    pub offset: [f32; 3],
    pub tucked_vertices: usize,
    pub max_push: f32,
    pub pushed_vertices: usize,
}

pub(crate) struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
}

pub(crate) fn read_u32(b: &[u8], at: usize) -> Result<usize> {
    let bytes = b.get(at..at + 4).context("truncated converted model")?;
    Ok(u32::from_le_bytes(bytes.try_into()?) as usize)
}

fn read_f32(b: &[u8], at: usize) -> Result<f64> {
    let bytes = b.get(at..at + 4).context("truncated converted model")?;
    Ok(f64::from(f32::from_le_bytes(bytes.try_into()?)))
}

pub(crate) fn parse_mesh(b: &[u8], stride: usize, label: &str) -> Result<Mesh> {
    ensure!(read_u32(b, 0)? == 3, "{label}: no vertex chunk");
    let count = read_u32(b, 8)?;
    ensure!(
        read_u32(b, 4)? == 4 + count * stride,
        "{label}: vertex stride is not {stride}"
    );
    let index_chunk = 12 + count * stride;
    ensure!(read_u32(b, index_chunk)? == 4, "{label}: no index chunk");
    let indices = read_u32(b, index_chunk + 8)?;
    ensure!(
        indices % 3 == 0,
        "{label}: index count {indices} is not triangles"
    );
    let positions = (0..count)
        .map(|i| {
            let at = 12 + i * stride;
            Ok([read_f32(b, at)?, read_f32(b, at + 4)?, read_f32(b, at + 8)?])
        })
        .collect::<Result<Vec<_>>>()?;
    let index = |k: usize| -> Result<usize> {
        let at = index_chunk + 12 + k * 2;
        let bytes = b.get(at..at + 2).context("truncated index chunk")?;
        let v = usize::from(u16::from_le_bytes(bytes.try_into()?));
        ensure!(v < count, "{label}: index {v} out of range");
        Ok(v)
    };
    let triangles = (0..indices / 3)
        .map(|t| Ok([index(t * 3)?, index(t * 3 + 1)?, index(t * 3 + 2)?]))
        .collect::<Result<Vec<_>>>()?;
    Ok(Mesh {
        positions,
        triangles,
    })
}

pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Farthest intersection of the ray `origin + t * dir` with the mesh, as
/// (t, triangle, barycentric weights).
fn farthest_hit(
    origin: [f64; 3],
    dir: [f64; 3],
    positions: &[[f64; 3]],
    triangles: &[[usize; 3]],
) -> Option<(f64, usize, [f64; 3])> {
    let mut best: Option<(f64, usize, [f64; 3])> = None;
    for (index, tri) in triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|v| positions[v]);
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
        let inside = u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 && t > 0.0;
        if inside && best.is_none_or(|(bt, _, _)| t > bt) {
            best = Some((t, index, [1.0 - u - v, u, v]));
        }
    }
    best
}

pub(crate) fn bone_origins(bind_pose: &[u8]) -> Result<BTreeMap<String, [f64; 3]>> {
    Ok(
        crate::armor::bind_pose_origins(bind_pose, "native head bind pose")?
            .into_iter()
            .map(|(name, t)| (name, t.map(f64::from)))
            .collect(),
    )
}

/// Fits a converted mask surface (48-byte vertices) in place onto the native
/// head surface (40-byte vertices). Both must share `bind_pose`.
pub fn fit_to_native_head(mask: &mut [u8], head: &[u8], bind_pose: &[u8]) -> Result<MaskFit> {
    let mesh = parse_mesh(mask, 48, "mask")?;
    let head = parse_mesh(head, 40, "native head")?;
    let bones = bone_origins(bind_pose)?;
    let bone = |name: &str| {
        bones
            .get(name)
            .copied()
            .with_context(|| format!("no {name} bone"))
    };
    let (left_eye, right_eye) = (bone("LeftEye")?, bone("RightEye")?);
    let (neck, head_bone) = (bone("Neck")?, bone("Head")?);

    // Seam-split copies share a position; weld them so they move together.
    let mut welded = HashMap::new();
    let weld: Vec<usize> = mesh
        .positions
        .iter()
        .map(|p| {
            let key = p.map(|v| (v * 1e5).round() as i64);
            let next = welded.len();
            *welded.entry(key).or_insert(next)
        })
        .collect();
    let welded_count = welded.len();
    let mut edge_uses: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut neighbours = vec![BTreeSet::new(); welded_count];
    for tri in &mesh.triangles {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let (a, b) = (weld[a], weld[b]);
            *edge_uses.entry((a.min(b), a.max(b))).or_default() += 1;
            neighbours[a].insert(b);
            neighbours[b].insert(a);
        }
    }
    let mut welded_positions = vec![[0.0; 3]; welded_count];
    for (i, &w) in weld.iter().enumerate() {
        welded_positions[w] = mesh.positions[i];
    }

    // Openings are loops of edges used by one triangle. The eye holes are the
    // two highest; the neck opening is the lowest.
    let mut rim: Vec<Vec<usize>> = vec![vec![]; welded_count];
    for (&(a, b), &uses) in &edge_uses {
        if uses == 1 {
            rim[a].push(b);
            rim[b].push(a);
        }
    }
    let mut seen = vec![false; welded_count];
    let mut openings = vec![];
    for start in 0..welded_count {
        if rim[start].is_empty() || seen[start] {
            continue;
        }
        let (mut stack, mut sum, mut n) = (vec![start], [0.0; 3], 0.0);
        seen[start] = true;
        while let Some(v) = stack.pop() {
            for k in 0..3 {
                sum[k] += welded_positions[v][k];
            }
            n += 1.0;
            for &u in &rim[v] {
                if !seen[u] {
                    seen[u] = true;
                    stack.push(u);
                }
            }
        }
        openings.push(sum.map(|s| s / n));
    }
    openings.sort_by(|a, b| b[1].total_cmp(&a[1]));
    let [first, second] = match openings.as_slice() {
        [a, b, _, ..] => [*a, *b],
        _ => bail!(
            "mask has {} openings; expected two eye holes and a neck",
            openings.len()
        ),
    };
    ensure!(
        first[0] * second[0] < 0.0,
        "the two highest mask openings are not a left/right eye pair"
    );
    let holes = [0, 1, 2].map(|k| (first[k] + second[k]) / 2.0);
    let mut eyes = [0, 1, 2].map(|k| (left_eye[k] + right_eye[k]) / 2.0);
    eyes[2] += CORNEA_AHEAD_OF_EYE_BONE;

    // Rigid move of the whole mask: shifting only the head-weighted part folds
    // the jaw where its weights blend into the neck.
    let offset = sub(eyes, holes);
    let mut start: Vec<[f64; 3]> = mesh
        .positions
        .iter()
        .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]])
        .collect();
    let tucked_vertices = tuck_into_neck(&mut start, &head, neck, head_bone);
    let mut positions = start.clone();

    // Push outward along rays from inside the skull.
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &head.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let centre = [(lo[0] + hi[0]) / 2.0, eyes[1], (lo[2] + hi[2]) / 2.0];
    let skull_floor = (neck[1] + head_bone[1]) / 2.0;
    let skull: Vec<[f64; 3]> = head
        .positions
        .iter()
        .copied()
        .filter(|p| p[1] > skull_floor)
        .collect();
    let ray = |p: [f64; 3]| {
        let d = sub(p, centre);
        let r = dot(d, d).sqrt();
        (d.map(|v| v / r), r)
    };
    let mut converged = false;
    for _ in 0..30 {
        let mut need = vec![0.0_f64; welded_count];
        // Mask vertices inside the head. Rays from the skull run steeply down
        // through the neck and would drag the tucked part back down.
        for (i, &p) in positions.iter().enumerate() {
            if p[1] < neck[1] + TUCK_FULL_ABOVE_NECK {
                continue;
            }
            let (dir, r) = ray(p);
            if let Some((t, _, _)) = farthest_hit(centre, dir, &head.positions, &head.triangles) {
                let w = &mut need[weld[i]];
                *w = w.max(t + CLEARANCE - r);
            }
        }
        // Head vertices poking between mask vertices.
        for &p in &skull {
            let (dir, r) = ray(p);
            if let Some((t, tri, bary)) = farthest_hit(centre, dir, &positions, &mesh.triangles) {
                let pen = r + CLEARANCE - t;
                if pen > 0.0 {
                    for (corner, b) in mesh.triangles[tri].iter().zip(bary) {
                        let w = &mut need[weld[*corner]];
                        *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
                    }
                }
            }
        }
        if need.iter().all(|&n| n < CONVERGED) {
            converged = true;
            break;
        }
        // Spread each push to its neighbours so the cloth stays smooth.
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
            let (dir, _) = ray(*p);
            let push = need[weld[i]];
            for k in 0..3 {
                p[k] += dir[k] * push;
            }
        }
    }
    ensure!(
        converged,
        "mask did not clear the native head after 30 passes"
    );

    let mut max_push = 0.0_f64;
    let mut pushed_vertices = 0;
    for (i, p) in positions.iter().enumerate() {
        let d = sub(*p, start[i]);
        let push = dot(d, d).sqrt();
        max_push = max_push.max(push);
        pushed_vertices += usize::from(push > 1e-4);
        for (k, v) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            mask[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    Ok(MaskFit {
        offset: offset.map(|v| v as f32),
        tucked_vertices,
        max_push: max_push as f32,
        pushed_vertices,
    })
}

/// Pulls the part below the jaw onto the native neck and shortens it, so the
/// hem ends inside every stock collar instead of draping over the chest like
/// it did on the donor body. Heights are relative to the native Neck bone.
fn tuck_into_neck(
    positions: &mut [[f64; 3]],
    head: &Mesh,
    neck: [f64; 3],
    head_bone: [f64; 3],
) -> usize {
    // Untouched above the jaw, fully on the neck below the chin.
    let start = neck[1] + 0.09;
    let full = neck[1] + TUCK_FULL_ABOVE_NECK;
    // The hem is squeezed from here down to just above where the native
    // throat ends, which every torso covers.
    let squeeze = neck[1] + 0.03;
    let hem = head
        .positions
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min)
        + 0.004;
    // Below this the native head is only a throat strip, so the neck is
    // sampled no lower.
    let sample_floor = neck[1] + 0.016;
    let lowest = positions.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let axis = |y: f64| {
        let t = (y - neck[1]) / (head_bone[1] - neck[1]);
        [0, 1, 2].map(|k| neck[k] + t * (head_bone[k] - neck[k]))
    };
    let mut tucked = 0;
    for p in positions.iter_mut() {
        if p[1] >= start {
            continue;
        }
        let mut y = p[1];
        if y < squeeze && lowest < hem {
            y = squeeze - (squeeze - y) * (squeeze - hem) / (squeeze - lowest);
        }
        let s = ((start - y) / (start - full)).clamp(0.0, 1.0);
        let s = s * s * (3.0 - 2.0 * s);
        let centre = axis(y);
        let out = [p[0] - centre[0], 0.0, p[2] - centre[2]];
        let r = dot(out, out).sqrt();
        if r < 1e-6 {
            continue;
        }
        let dir = out.map(|v| v / r);
        // Clear the head a little above each vertex too, so the cloth hangs
        // from the chin to the throat instead of cutting under the jaw.
        let target = (0..=4)
            .filter_map(|k| {
                let sample_y = y.max(sample_floor) + f64::from(k) * 0.005;
                let sample = axis(sample_y);
                let origin = [sample[0], sample_y, sample[2]];
                let (t, _, _) = farthest_hit(origin, dir, &head.positions, &head.triangles)?;
                let hit = [0, 1, 2].map(|k| origin[k] + dir[k] * t);
                Some(dot(sub(hit, centre), dir) + NECK_CLEARANCE)
            })
            .fold(None, |best: Option<f64>, t| {
                Some(best.map_or(t, |b| b.max(t)))
            })
            .unwrap_or(r);
        let radius = r + s * (target - r);
        *p = [centre[0] + dir[0] * radius, y, centre[2] + dir[2] * radius];
        tucked += 1;
    }
    tucked
}
