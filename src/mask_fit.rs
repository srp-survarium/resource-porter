// SPDX-License-Identifier: GPL-3.0-or-later
//! Fits donor headwear onto the native v0.26g head: the eye holes are moved
//! over the native eyes, the part below the jaw is tucked onto the native
//! neck, then the cloth is pushed outward wherever the native head would show
//! through it.
//!
//! Stock masks have no per-helmet variant or hide flag: a helmet that cannot
//! be worn over a mask takes the mask slot (`additional_model_slots`), and
//! every other helmet simply covers it. So the cloth over the skull is then
//! refined and pulled in under the outer skin of every such helmet, between
//! it and the head (`fit_to_native_head_under`).
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
/// Gap kept under the outer skin of every helmet worn over the mask.
const SHELL_CLEARANCE: f64 = 0.0005;
/// Under a helmet the cloth comes down to this gap over the head. Closer
/// than this the head's facets start to show through the mask's coarser
/// ones when no helmet is worn.
const HEAD_CLEARANCE_UNDER_SHELLS: f64 = 0.002;
/// Where a helmet nearly touches the head (the Hunter's hood beside its
/// goggle strap leaves 1.2 mm), the cloth takes the middle of the gap, but
/// comes no closer to the head than this.
const MIN_HEAD_CLEARANCE: f64 = 0.0005;
/// The pull moves the cloth this far at most per pass, so neighbours follow
/// together instead of a lone vertex dragging a triangle into the head.
const PULL_STEP: f64 = 0.001;
const PULL_PASSES: usize = 80;
/// In the last passes a vertex the head pushed back keeps that height.
const SETTLE_PASSES: usize = 20;

pub struct MaskFit {
    pub offset: [f32; 3],
    pub tucked_vertices: usize,
    pub max_push: f32,
    pub pushed_vertices: usize,
    /// Vertices pulled in under a helmet, and the deepest pull.
    pub pulled_vertices: usize,
    pub max_pull: f32,
    /// Per shell: how much of the fitted mask is outside it.
    pub shells: Vec<ShellFit>,
}

/// A helmet surface the mask is worn under (any vertex stride).
pub struct Shell {
    pub label: String,
    pub(crate) mesh: Mesh,
}

impl Shell {
    pub fn parse(label: impl Into<String>, converted_model: &[u8]) -> Result<Self> {
        let label = label.into();
        let count = read_u32(converted_model, 8)?;
        ensure!(count > 0, "{label}: empty vertex chunk");
        let stride = (read_u32(converted_model, 4)? - 4) / count;
        let mesh = parse_mesh(converted_model, stride, &label)?;
        Ok(Self { label, mesh })
    }
}

#[derive(Debug, Clone)]
pub struct ShellFit {
    pub label: String,
    /// Before and after the helmet pass: cranium vertices outside the outer
    /// skin, and the farthest one (m).
    pub outside_before: usize,
    pub depth_before: f32,
    pub outside_after: usize,
    pub depth_after: f32,
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

/// All intersections of the ray `origin + t * dir` (t > 0) with the mesh, as
/// (t, whether the triangle faces along the ray).
fn ray_hits(
    origin: [f64; 3],
    dir: [f64; 3],
    positions: &[[f64; 3]],
    triangles: &[[usize; 3]],
) -> Vec<(f64, bool)> {
    let mut hits = vec![];
    for tri in triangles {
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
        if u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 && t > 0.0 {
            hits.push((t, dot(cross(e1, e2), dir) > 0.0));
        }
    }
    hits
}

/// Distance from `centre` along `dir` to a shell's outer skin: its first
/// outward-facing surface beyond `floor`. Inward-facing liners are skipped,
/// and so is anything stacked outside the skin (goggles pushed up on a hood).
fn shell_skin(centre: [f64; 3], dir: [f64; 3], shell: &Mesh, floor: f64) -> Option<f64> {
    ray_hits(centre, dir, &shell.positions, &shell.triangles)
        .into_iter()
        .filter(|&(t, outward)| outward && t > floor)
        .map(|(t, _)| t)
        .min_by(f64::total_cmp)
}

/// Mask vertices outside a shell's outer skin: the ray from the vertex back
/// to `centre` first meets the front of a shell triangle. Returns the count
/// and the deepest (m).
fn outside_shell(
    positions: &[[f64; 3]],
    consider: &[bool],
    centre: [f64; 3],
    shell: &Mesh,
) -> (usize, f64) {
    let (mut count, mut deepest) = (0, 0.0_f64);
    for (p, _) in positions.iter().zip(consider).filter(|(_, c)| **c) {
        let d = sub(centre, *p);
        let r = dot(d, d).sqrt();
        let dir = d.map(|v| v / r);
        let first = ray_hits(*p, dir, &shell.positions, &shell.triangles)
            .into_iter()
            .filter(|&(t, _)| t < r)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((t, false)) = first
            && t > 0.0005
        {
            count += 1;
            deepest = deepest.max(t);
        }
    }
    (count, deepest)
}

/// The thinnest room any shell leaves over the head along `dir` (m), or
/// infinity where no shell covers it.
fn room_under_shells(centre: [f64; 3], dir: [f64; 3], head_at: f64, shells: &[Shell]) -> f64 {
    shells
        .iter()
        .filter_map(|s| shell_skin(centre, dir, &s.mesh, head_at))
        .map(|skin| skin - head_at)
        .fold(f64::INFINITY, f64::min)
}

/// How close to the head the cloth may come, given the room a helmet
/// leaves there: HEAD_CLEARANCE_UNDER_SHELLS, or the middle of a thinner
/// gap, but never closer than MIN_HEAD_CLEARANCE.
fn head_clearance(room: f64) -> f64 {
    (room / 2.0).clamp(MIN_HEAD_CLEARANCE, HEAD_CLEARANCE_UNDER_SHELLS)
}

/// How far under a skin the cloth stays, given the room there.
fn shell_clearance(room: f64) -> f64 {
    (room / 2.0).clamp(0.0, SHELL_CLEARANCE)
}

/// Pulls the cloth in under every shell's outer skin, keeping it between the
/// head and the skin (`head_clearance`, `shell_clearance`). Vertices move
/// along their rays from `centre`; only `movable` ones move. Returns how far
/// each welded vertex was pulled.
#[allow(clippy::too_many_arguments)]
fn pull_under_shells(
    positions: &mut [[f64; 3]],
    weld: &[usize],
    neighbours: &[BTreeSet<usize>],
    triangles: &[[usize; 3]],
    movable: &[bool],
    centre: [f64; 3],
    head: &Mesh,
    shells: &[Shell],
) -> Vec<f64> {
    let welded_count = neighbours.len();
    let ray = |p: [f64; 3]| {
        let d = sub(p, centre);
        let r = dot(d, d).sqrt();
        (d.map(|v| v / r), r)
    };
    // The lowest each welded vertex may go, and the skin it must go under.
    // Radial moves keep the ray, so both are fixed.
    let mut floor = vec![f64::INFINITY; welded_count];
    let mut ceiling = vec![f64::INFINITY; welded_count];
    let mut free = vec![false; welded_count];
    for (i, &p) in positions.iter().enumerate() {
        let w = weld[i];
        if !movable[i] || free[w] {
            continue;
        }
        free[w] = true;
        let (dir, r) = ray(p);
        let Some((head_at, _, _)) = farthest_hit(centre, dir, &head.positions, &head.triangles)
        else {
            floor[w] = r;
            continue;
        };
        let room = room_under_shells(centre, dir, head_at, shells);
        floor[w] = (head_at + head_clearance(room)).min(r);
        ceiling[w] = head_at + room - shell_clearance(room);
    }
    // Each head vertex keeps its own clearance under the cloth.
    let head_clear: Vec<f64> = head
        .positions
        .iter()
        .map(|&p| {
            let (dir, r) = ray(p);
            head_clearance(room_under_shells(centre, dir, r, shells))
        })
        .collect();
    // Skin vertices: those that are a shell's outer skin on their own ray.
    let skins: Vec<([f64; 3], f64)> = shells
        .iter()
        .flat_map(|shell| {
            shell.mesh.positions.iter().filter_map(|&q| {
                let (dir, r) = ray(q);
                let head_at = farthest_hit(centre, dir, &head.positions, &head.triangles)?.0;
                let skin = shell_skin(centre, dir, &shell.mesh, head_at)?;
                ((skin - r).abs() < 1e-4).then(|| (q, shell_clearance(r - head_at)))
            })
        })
        .collect();
    let mut pulled = vec![0.0_f64; welded_count];
    for pass in 0..PULL_PASSES {
        let mut need = vec![0.0_f64; welded_count];
        // Mask vertices outside a skin.
        let mut seen = vec![false; welded_count];
        for (i, &p) in positions.iter().enumerate() {
            let w = weld[i];
            if free[w] && !seen[w] {
                seen[w] = true;
                need[w] = (ray(p).1 - ceiling[w].max(floor[w])).max(0.0);
            }
        }
        // Skin vertices under the mask, between mask vertices.
        for &(q, clearance) in &skins {
            let (dir, r) = ray(q);
            let Some((t, tri, bary)) = farthest_hit(centre, dir, positions, triangles) else {
                continue;
            };
            let pen = t - (r - clearance);
            if pen <= 0.0 {
                continue;
            }
            for (corner, b) in triangles[tri].iter().zip(bary) {
                if movable[*corner] {
                    let w = &mut need[weld[*corner]];
                    *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
                }
            }
        }
        // Spread so the cloth dents smoothly instead of creasing.
        for _ in 0..3 {
            need = (0..welded_count)
                .map(|v| {
                    if !free[v] {
                        return 0.0;
                    }
                    let n = &neighbours[v];
                    let mean = n.iter().map(|&u| need[u]).sum::<f64>() / n.len().max(1) as f64;
                    need[v].max(0.5 * mean)
                })
                .collect();
        }
        let mut step = vec![0.0_f64; welded_count];
        let mut seen = vec![false; welded_count];
        for (i, p) in positions.iter().enumerate() {
            let w = weld[i];
            if free[w] && !seen[w] {
                seen[w] = true;
                step[w] = need[w].min(PULL_STEP).min(ray(*p).1 - floor[w]).max(0.0);
            }
        }
        for (i, p) in positions.iter_mut().enumerate() {
            let s = step[weld[i]];
            if s > 0.0 {
                let (dir, _) = ray(*p);
                for k in 0..3 {
                    p[k] -= dir[k] * s;
                }
            }
        }
        let back = cover_head(
            positions,
            weld,
            triangles,
            movable,
            centre,
            head,
            &head_clear,
        );
        for w in 0..welded_count {
            pulled[w] += step[w] - back[w];
        }
        // Late on, where the head needed the cloth back out, that is its
        // floor, so pulls and pushes cannot chase each other.
        for (i, p) in positions.iter().enumerate() {
            let w = weld[i];
            if back[w] > 0.0 && pass >= PULL_PASSES - SETTLE_PASSES {
                floor[w] = floor[w].max(ray(*p).1);
            }
        }
        if step
            .iter()
            .zip(&back)
            .all(|(s, b)| (s - b).abs() < CONVERGED)
        {
            break;
        }
    }
    // Whatever the last pull did, the head stays covered.
    for _ in 0..30 {
        let back = cover_head(
            positions,
            weld,
            triangles,
            movable,
            centre,
            head,
            &head_clear,
        );
        for (total, b) in pulled.iter_mut().zip(&back) {
            *total -= b;
        }
        if back.iter().all(|&b| b < 1e-5) {
            break;
        }
    }
    pulled
}

/// Pushes movable corners back out wherever a head vertex between mask
/// vertices comes closer than its `clearance` to the cloth. Returns the push
/// per welded vertex.
fn cover_head(
    positions: &mut [[f64; 3]],
    weld: &[usize],
    triangles: &[[usize; 3]],
    movable: &[bool],
    centre: [f64; 3],
    head: &Mesh,
    clearance: &[f64],
) -> Vec<f64> {
    let welded_count = weld.iter().max().map_or(0, |m| m + 1);
    let ray = |p: [f64; 3]| {
        let d = sub(p, centre);
        let r = dot(d, d).sqrt();
        (d.map(|v| v / r), r)
    };
    let mut back = vec![0.0_f64; welded_count];
    for (&p, &clear) in head.positions.iter().zip(clearance) {
        let (dir, r) = ray(p);
        if let Some((t, tri, bary)) = farthest_hit(centre, dir, positions, triangles) {
            let pen = r + clear - t;
            if pen > 0.0 {
                for (corner, b) in triangles[tri].iter().zip(bary) {
                    if movable[*corner] {
                        let w = &mut back[weld[*corner]];
                        *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
                    }
                }
            }
        }
    }
    for (i, p) in positions.iter_mut().enumerate() {
        let s = back[weld[i]];
        if s > 0.0 {
            let (dir, _) = ray(*p);
            for k in 0..3 {
                p[k] += dir[k] * s;
            }
        }
    }
    back
}

/// Fits a converted mask surface (48-byte vertices) in place onto the native
/// head surface (40-byte vertices). Both must share `bind_pose`.
pub fn fit_to_native_head(mask: &mut Vec<u8>, head: &[u8], bind_pose: &[u8]) -> Result<MaskFit> {
    fit_to_native_head_under(mask, head, bind_pose, &[])
}

/// As `fit_to_native_head`, then the part over the skull is pulled in under
/// the outer skin of every helmet in `shells` (those worn with a mask).
pub fn fit_to_native_head_under(
    mask: &mut Vec<u8>,
    head: &[u8],
    bind_pose: &[u8],
    shells: &[Shell],
) -> Result<MaskFit> {
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
    }

    for (i, p) in positions.iter().enumerate() {
        for (k, v) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            mask[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    // Under helmets the cloth over the skull must follow the head closely,
    // closer than its own facets allow: halve its edges there first.
    let (mesh, weld, neighbours, mut positions) = if shells.is_empty() {
        (mesh, weld, neighbours, positions)
    } else {
        *mask = refine(mask, |p| p[1] >= neck[1] + TUCK_FULL_ABOVE_NECK)?;
        let mesh = parse_mesh(mask, 48, "refined mask")?;
        let (weld, neighbours) = weld_vertices(&mesh);
        let positions = mesh.positions.clone();
        (mesh, weld, neighbours, positions)
    };
    let welded_count = neighbours.len();

    // Helmets are worn over it: the cloth over the skull goes under each
    // helmet's outer skin. The tucked part is left to the collars.
    let movable: Vec<bool> = positions
        .iter()
        .map(|p| p[1] >= neck[1] + TUCK_FULL_ABOVE_NECK)
        .collect();
    let before: Vec<(usize, f64)> = shells
        .iter()
        .map(|s| outside_shell(&positions, &movable, centre, &s.mesh))
        .collect();
    let pulled = if shells.is_empty() {
        vec![0.0; welded_count]
    } else {
        pull_under_shells(
            &mut positions,
            &weld,
            &neighbours,
            &mesh.triangles,
            &movable,
            centre,
            &head,
            shells,
        )
    };
    let shells = shells
        .iter()
        .zip(before)
        .map(|(s, (outside_before, depth_before))| {
            let (outside_after, depth_after) = outside_shell(&positions, &movable, centre, &s.mesh);
            ShellFit {
                label: s.label.clone(),
                outside_before,
                depth_before: depth_before as f32,
                outside_after,
                depth_after: depth_after as f32,
            }
        })
        .collect();
    // The head must still not show through the cloth anywhere.
    for &p in &skull {
        if p[1] < neck[1] + TUCK_FULL_ABOVE_NECK {
            continue;
        }
        let (dir, r) = ray(p);
        if let Some((t, _, _)) = farthest_hit(centre, dir, &positions, &mesh.triangles) {
            ensure!(
                t >= r + MIN_HEAD_CLEARANCE * 0.5,
                "the head shows through the mask under a helmet at {p:?}"
            );
        }
    }

    for (i, p) in positions.iter().enumerate() {
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
        pulled_vertices: pulled.iter().filter(|&&p| p > 1e-4).count(),
        max_pull: pulled.iter().copied().fold(0.0, f64::max) as f32,
        shells,
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

/// Welds seam-split copies (same position) and lists each welded vertex's
/// neighbours.
fn weld_vertices(mesh: &Mesh) -> (Vec<usize>, Vec<BTreeSet<usize>>) {
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
    let mut neighbours = vec![BTreeSet::new(); welded.len()];
    for tri in &mesh.triangles {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            neighbours[weld[a]].insert(weld[b]);
            neighbours[weld[b]].insert(weld[a]);
        }
    }
    (weld, neighbours)
}

/// The vertex halfway between two 48-byte converted-model vertices:
/// position, UV and basis averaged, skin weights averaged where both use the
/// same bones (else the first's).
fn midpoint_vertex(a: &[u8], b: &[u8]) -> [u8; 48] {
    let f = |v: &[u8], at: usize| f32::from_le_bytes(v[at..at + 4].try_into().unwrap());
    let mut out = [0_u8; 48];
    out.copy_from_slice(a);
    let mut put = |at: usize, value: f32| out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    for at in [0, 4, 8, 40, 44] {
        put(at, (f(a, at) + f(b, at)) / 2.0);
    }
    if a[12..16] == b[12..16] {
        for at in [16, 20, 24] {
            put(at, (f(a, at) + f(b, at)) / 2.0);
        }
    }
    let uv = [(f(a, 40) + f(b, 40)) / 2.0, (f(a, 44) + f(b, 44)) / 2.0];
    // Normal, tangent, binormal: 0..255 components, the top byte keeps 127
    // for the normal and the sub-texel UV fraction for the other two.
    for (k, at) in [28_usize, 32, 36].into_iter().enumerate() {
        let unpack = |v: &[u8]| {
            let w = u32::from_le_bytes(v[at..at + 4].try_into().unwrap());
            [16, 8, 0].map(|s| f64::from((w >> s) & 255) / 127.5 - 1.0)
        };
        let (pa, pb) = (unpack(a), unpack(b));
        let mut v = [0, 1, 2].map(|i| pa[i] + pb[i]);
        let len = dot(v, v).sqrt().max(1e-9);
        v = v.map(|c| c / len);
        let component = |c: f64| (((c + 1.0) * 127.5).floor() as i64).clamp(0, 255) as u32;
        let alpha = match k {
            0 => u32::from(a[at + 3]),
            _ => {
                let rebased = f64::from(uv[k - 1]) * 1024.0;
                ((255.5 * (rebased - rebased.floor()))
                    .floor()
                    .clamp(0.0, 255.0)) as u32
            }
        };
        let w = (alpha << 24) | (component(v[0]) << 16) | (component(v[1]) << 8) | component(v[2]);
        out[at..at + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// Splits at its middle every edge whose two (welded) ends `split` accepts,
/// re-triangulating without T-junctions, so the cloth there can follow the
/// head and a tight helmet more closely than its own coarse facets allow.
fn refine(mask: &[u8], split: impl Fn([f64; 3]) -> bool) -> Result<Vec<u8>> {
    let mesh = parse_mesh(mask, 48, "mask")?;
    let (weld, _) = weld_vertices(&mesh);
    let count = mesh.positions.len();
    let vertex = |i: usize| &mask[12 + i * 48..12 + (i + 1) * 48];
    let mut vertices: Vec<[u8; 48]> = (0..count).map(|i| vertex(i).try_into().unwrap()).collect();
    let mut middle: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut mid = |a: usize, b: usize, vertices: &mut Vec<[u8; 48]>| -> Option<usize> {
        if !(split(mesh.positions[a]) && split(mesh.positions[b])) || weld[a] == weld[b] {
            return None;
        }
        let key = (a.min(b), a.max(b));
        Some(*middle.entry(key).or_insert_with(|| {
            vertices.push(midpoint_vertex(vertex(key.0), vertex(key.1)));
            vertices.len() - 1
        }))
    };
    let mut triangles = vec![];
    for &[v0, v1, v2] in &mesh.triangles {
        let mut t = [v0, v1, v2];
        let mut m = [
            mid(t[0], t[1], &mut vertices),
            mid(t[1], t[2], &mut vertices),
            mid(t[2], t[0], &mut vertices),
        ];
        // Rotate so the split edges come first.
        for _ in 0..3 {
            let splits = m.iter().filter(|x| x.is_some()).count();
            let ok = match splits {
                1 => m[0].is_some(),
                2 => m[2].is_none(),
                _ => true,
            };
            if ok {
                break;
            }
            t.rotate_left(1);
            m.rotate_left(1);
        }
        let [a, b, c] = t;
        match m {
            [None, None, None] => triangles.push([a, b, c]),
            [Some(ab), None, None] => triangles.extend([[a, ab, c], [ab, b, c]]),
            [Some(ab), Some(bc), None] => triangles.extend([[ab, b, bc], [a, ab, bc], [a, bc, c]]),
            [Some(ab), Some(bc), Some(ca)] => {
                triangles.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]])
            }
            _ => bail!("unexpected split pattern"),
        }
    }
    ensure!(
        vertices.len() <= usize::from(u16::MAX),
        "refined mask needs 32-bit indices"
    );
    let mut out = vec![];
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + vertices.len() * 48)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(vertices.len())?.to_le_bytes());
    for v in &vertices {
        out.extend_from_slice(v);
    }
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + triangles.len() * 6)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(triangles.len() * 3)?.to_le_bytes());
    for t in &triangles {
        for &i in t {
            out.extend_from_slice(&u16::try_from(i)?.to_le_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A UV sphere around `c` with outward-facing triangles.
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
        let at = |i: usize, j: usize| i * segments + j % segments;
        let mut triangles = vec![];
        for i in 0..rings {
            for j in 0..segments {
                for tri in [
                    [at(i, j), at(i, j + 1), at(i + 1, j)],
                    [at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)],
                ] {
                    let [a, b, d] = tri.map(|v| positions[v]);
                    let n = cross(sub(b, a), sub(d, a));
                    if dot(n, n) < 1e-18 {
                        continue;
                    }
                    let mid = [0, 1, 2].map(|k| (a[k] + b[k] + d[k]) / 3.0 - c[k]);
                    triangles.push(if dot(n, mid) > 0.0 {
                        tri
                    } else {
                        [tri[0], tri[2], tri[1]]
                    });
                }
            }
        }
        Mesh {
            positions,
            triangles,
        }
    }

    fn radius(p: [f64; 3]) -> f64 {
        dot(p, p).sqrt()
    }

    fn pull(mask: &Mesh, head: &Mesh, shell_radius: f64) -> Vec<[f64; 3]> {
        let shells = [Shell {
            label: "shell".into(),
            mesh: sphere([0.0; 3], shell_radius, 48, 24),
        }];
        let (weld, neighbours) = weld_vertices(mask);
        let mut positions = mask.positions.clone();
        let movable = vec![true; positions.len()];
        pull_under_shells(
            &mut positions,
            &weld,
            &neighbours,
            &mask.triangles,
            &movable,
            [0.0; 3],
            head,
            &shells,
        );
        positions
    }

    #[test]
    fn cloth_goes_under_a_helmet_but_stays_over_the_head() {
        let head = sphere([0.0; 3], 0.090, 32, 16);
        let mask = sphere([0.0; 3], 0.100, 32, 16);
        let fitted = pull(&mask, &head, 0.097);
        for p in fitted {
            let r = radius(p);
            assert!(
                r <= 0.097 - SHELL_CLEARANCE + 1e-4,
                "outside the helmet at {r}"
            );
            assert!(r >= 0.090 + MIN_HEAD_CLEARANCE, "inside the head at {r}");
        }
    }

    #[test]
    fn a_helmet_that_nearly_touches_the_head_leaves_the_cloth_in_the_gap() {
        // The Hunter's hood leaves 1.2 mm beside its goggle strap.
        let head = sphere([0.0; 3], 0.090, 32, 16);
        let mask = sphere([0.0; 3], 0.095, 32, 16);
        let fitted = pull(&mask, &head, 0.0912);
        for p in fitted {
            let r = radius(p);
            assert!(r < 0.0912, "outside the hood at {r}");
            assert!(r > 0.0903, "on the head at {r}");
        }
    }

    #[test]
    fn without_a_helmet_over_it_the_cloth_stays_put() {
        let head = sphere([0.0; 3], 0.090, 32, 16);
        let mask = sphere([0.0; 3], 0.094, 32, 16);
        let fitted = pull(&mask, &head, 0.120);
        for (p, q) in fitted.iter().zip(&mask.positions) {
            assert!((radius(*p) - radius(*q)).abs() < 1e-9);
        }
    }

    fn vertex(p: [f32; 3], uv: [f32; 2]) -> [u8; 48] {
        let mut v = [0_u8; 48];
        for (k, c) in p.iter().enumerate() {
            v[k * 4..k * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
        v[12..16].copy_from_slice(&[7, 7, 7, 7]);
        v[16..20].copy_from_slice(&1.0_f32.to_le_bytes());
        for at in [28, 32, 36] {
            v[at..at + 4].copy_from_slice(&0x7f_80_80_ff_u32.to_le_bytes());
        }
        v[40..44].copy_from_slice(&uv[0].to_le_bytes());
        v[44..48].copy_from_slice(&uv[1].to_le_bytes());
        v
    }

    fn model(vertices: &[[u8; 48]], indices: &[u16]) -> Vec<u8> {
        let mut b = vec![];
        b.extend_from_slice(&3_u32.to_le_bytes());
        b.extend_from_slice(&((4 + vertices.len() * 48) as u32).to_le_bytes());
        b.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        for v in vertices {
            b.extend_from_slice(v);
        }
        b.extend_from_slice(&4_u32.to_le_bytes());
        b.extend_from_slice(&((4 + indices.len() * 2) as u32).to_le_bytes());
        b.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        for i in indices {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b
    }

    #[test]
    fn refining_splits_marked_edges_without_cracks() {
        // A quad of two triangles with three corners marked: the edges
        // between marked corners (1-2 and 2-3) are halved, the diagonal is not.
        let v = [
            vertex([0.0, 0.0, 0.0], [0.0, 0.0]),
            vertex([1.0, 0.0, 0.0], [1.0, 0.0]),
            vertex([1.0, 1.0, 0.0], [1.0, 1.0]),
            vertex([0.0, 1.0, 0.0], [0.0, 1.0]),
        ];
        let bytes = model(&v, &[0, 1, 2, 0, 2, 3]);
        let out = refine(&bytes, |p| p[1] > 0.5 || p[0] > 0.5).unwrap();
        let mesh = parse_mesh(&out, 48, "refined").unwrap();
        // Vertices 1, 2, 3 are marked: edges 1-2 and 2-3 split.
        assert_eq!(mesh.positions.len(), 6);
        // Each triangle has one split edge, so each becomes two.
        assert_eq!(mesh.triangles.len(), 4);
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i]);
                cross(sub(b, a), sub(c, a))[2] / 2.0
            })
            .sum();
        assert!(
            (area - 1.0).abs() < 1e-6,
            "area {area}: winding or cover changed"
        );
        // Every edge between two triangles is shared exactly: no T-junction.
        let mut uses: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for t in &mesh.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *uses.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let boundary = uses.values().filter(|&&n| n == 1).count();
        assert_eq!(
            boundary, 6,
            "the outline is the 4 sides, two of them halved"
        );
        // The midpoint of 1-2 carries the averaged position and UV.
        let m = &out[12 + 4 * 48..12 + 5 * 48];
        let f = |at: usize| f32::from_le_bytes(m[at..at + 4].try_into().unwrap());
        assert_eq!([f(0), f(4), f(40), f(44)], [1.0, 0.5, 1.0, 0.5]);
    }
}
