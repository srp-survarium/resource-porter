// SPDX-License-Identifier: GPL-3.0-or-later
//! Player faces for v0.26g: the eight faces of 0.69d0 as 0.69d0 draws them,
//! and the STK2 Novice's face with a scalp grafted where his cap was.
//!
//! What the game reads is `gameplay/bodyparts/default`'s `body_parts.heads`,
//! in 0.31a2's form: `heads[i].data` (the bare face), `data_helmet` and
//! `data_mask`, each `{base_model, part_name, additional_models}` (the hair
//! or the scalp an additional model of the bare face); a missing variant is
//! the bare face. `heads[0]` is the native head (`body_parts.head`, which
//! stays as it is). Each head also names its picker icon, `heads[i].icon` =
//! `[row, col]` of the lobby movie's equipment atlas.
//!
//! - **0.69d0.** `models/character/human/head/head_*`: the surfaces bind
//!   the native rig in its bind pose (`face_items_069d::same_rig_for`) and
//!   convert in place ([`crate::v069d_skinned`]): 0.69d0's vertices, bone
//!   weights, basis and UVs, nothing cut, moved or re-weighted. The helmet
//!   and mask variants are 0.69d0's own; `head_05` and `head_07` have hair
//!   (`*_hair_vc`), which only the bare face draws. Their materials are
//!   0.69d0's, input for input ([`material_069`]).
//! - **STK2.** `neutral_01_head` (the Novice): the export's model on the
//!   native rig turned half a turn about Y (`duty_026::rotate_mesh`), moved
//!   onto the native head ([`align_onto`]), its short neck stretched down to
//!   the native neck's bottom and cut and skinned as it; his knit cap off and
//!   the native head's scalp grafted on ([`graft_scalp`]).
//! - **The fit report.** Every face in every state under every live item
//!   that leaves the head, in the bind pose and the sampled poses of
//!   `bandit_026::POSE_CLIPS` ([`face_fit::sweep`]): measured and recorded,
//!   never changed.
use crate::bandit_026::{HEAD_MODEL, SampledPoses};
use crate::face_fit::{self, Drawn, Job, Measure, Posable, Rig, State, Wardrobe};
use crate::face_items_069d::same_rig_for;
use crate::face_shape::{Carried, NeckJoin, Seam, join_native_neck};
use crate::manifest::BuiltFile;
use crate::mask_fit::{dot, parse_mesh, read_u32, sub};
use crate::outfit_026::{Writer, s};
use crate::weapon::parse_config;
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The port's folder under `models/ported/`, `material_instances/ported/`
/// and `textures/ported/`.
pub const PORT: &str = "faces";
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";
const NATIVE_HEAD: &str = "character/human/base/base_character_head";
const NATIVE_PART: &str = "black_market_01_head_s";
const BODYPARTS: &str = "gameplay/bodyparts/default";
/// Where the STK2 export keeps its converted models and its bind poses.
const STK2_PLATFORM: &str = "resources/platforms/pc_dx11/models/character/human";
const STK2_EXPORTED: &str = "resources/exported/models/character/human";
/// STK2's character textures under `--stk2-textures`.
const STK2_TEXTURES: &str = "resources/sources/textures/character/human";

/// Where a face comes from.
#[derive(Clone, Copy, Debug)]
pub enum Donor {
    /// The native head, `heads[0]`.
    Native,
    /// A 0.69d0 head under `models/character/human/head/`: its model and
    /// surface, its hair surface, and its helmet and mask models (same
    /// surface name) where 0.69d has them.
    V069 {
        model: &'static str,
        surface: &'static str,
        hair: Option<&'static str>,
        helmet: Option<&'static str>,
        mask: Option<&'static str>,
    },
    /// A face of the STK2 export: its model under the export's
    /// `models/character/human/`, its surface and its texture stem under
    /// [`STK2_TEXTURES`].
    Stk2 {
        model: &'static str,
        surface: &'static str,
        texture: &'static str,
        /// Its cap (the mesh at its top) taken off and the native head's
        /// scalp grafted on ([`graft_scalp`]).
        scalp: bool,
    },
}

/// One face of the picker.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub index: usize,
    /// Its name under [`PORT`].
    pub key: &'static str,
    pub donor: Donor,
}

const fn v069(
    index: usize,
    key: &'static str,
    model: &'static str,
    surface: &'static str,
    hair: Option<&'static str>,
    helmet: Option<&'static str>,
    mask: Option<&'static str>,
) -> Face {
    Face {
        index,
        key,
        donor: Donor::V069 {
            model,
            surface,
            hair,
            helmet,
            mask,
        },
    }
}

/// The faces, in picker order. 0.69d0's `head_04` is its own mask
/// variant, and `head_01` .. `head_03` have no variants.
pub const FACES: [Face; 10] = [
    Face {
        index: 0,
        key: "native",
        donor: Donor::Native,
    },
    v069(
        1,
        "069d_head_default",
        "head_default",
        "head_default_sg",
        None,
        Some("head_default_helmet"),
        Some("head_default_mask"),
    ),
    v069(2, "069d_head_01", "head_01", "head_01_s", None, None, None),
    v069(3, "069d_head_02", "head_02", "head_02_s", None, None, None),
    v069(4, "069d_head_03", "head_03", "head_03_s", None, None, None),
    v069(
        5,
        "069d_head_04",
        "head_04",
        "head_04_s",
        None,
        Some("head_04_helmet"),
        None,
    ),
    v069(
        6,
        "069d_head_05",
        "head_05",
        "head_05_s",
        Some("head_05_hair_vc"),
        Some("head_05_helmet"),
        Some("head_05_mask"),
    ),
    v069(
        7,
        "069d_head_06",
        "head_06",
        "head_06_s",
        None,
        Some("head_06_helmet"),
        Some("head_06_mask"),
    ),
    v069(
        8,
        "069d_head_07",
        "head_07",
        "head_07_s",
        Some("head_07_hair_vc"),
        Some("head_07_helmet"),
        Some("head_07_mask"),
    ),
    Face {
        index: 9,
        key: "stk2_neutral_01_head",
        donor: Donor::Stk2 {
            model: "neutral/neutral_01/neutral_01_head",
            surface: "neutral_01_head_s",
            texture: "neutral/neutral_01_novice/neutral_01_novice_head",
            scalp: true,
        },
    },
];

/// Where the build reads its donors.
pub struct Sources<'a> {
    /// 0.69d0's files, extracted at their archive paths.
    pub v069: &'a Path,
    /// The STK2 characters export (its `resources/exported` and
    /// `resources/platforms/pc_dx11`).
    pub stk2: &'a Path,
    /// STK2's `resources/sources/textures/character/human` TGAs.
    pub stk2_textures: &'a Path,
}

fn variant_name(state: State) -> &'static str {
    match state {
        State::Bare => "",
        State::Helmet => "_helmet",
        State::Mask => "_mask",
    }
}

/// What a donor gives of a face: the drawn surfaces of each state, and how
/// each one's neck was joined.
type Donated = (BTreeMap<State, Drawn>, Vec<(String, NeckJoin)>);

/// A face as its donor gives it: the drawn surfaces of each state it has
/// a model for (always the bare one), their necks joined to the body.
fn donor_faces(
    sources: &Sources<'_>,
    bind_pose: &[u8],
    native_head: &[u8],
    rig: &Rig,
    face: &Face,
) -> Result<Donated> {
    let mut out = BTreeMap::new();
    let native = parse_mesh(native_head, 40, "native head")?;
    let seam = Seam::new(&native)?;
    let mut joins = vec![];
    // FACES_NO_NECK: the donors' own necks, for the numbers before.
    let no_neck = std::env::var_os("FACES_NO_NECK").is_some();
    let mut join = |label: &str, mesh: &[u8]| -> Result<Vec<u8>> {
        if no_neck {
            return Ok(mesh.to_vec());
        }
        let (joined, how) = join_native_neck(mesh, native_head, rig, label)?;
        joins.push((label.to_owned(), how));
        Ok(joined)
    };
    match face.donor {
        Donor::Native => bail!("the native head is not built"),
        Donor::V069 {
            model,
            surface,
            hair,
            helmet,
            mask,
        } => {
            let read = |model: &str, surface: &str| -> Result<Vec<u8>> {
                let root = format!("models/character/human/head/{model}.skinned_model");
                let path = sources
                    .v069
                    .join(format!("{root}/render/{surface}/converted_model"));
                let donor = fs::read(&path).with_context(|| path.display().to_string())?;
                let label = format!("0.69d {model} {surface}");
                let parsed = crate::v069d_skinned::parse_069d(&donor, &label)?;
                let used = parsed
                    .vertices
                    .iter()
                    .flat_map(|v| v.bones.iter().zip(v.weights))
                    .filter(|(_, w)| *w > 0.0)
                    .map(|(&b, _)| usize::from(b))
                    .collect();
                let pose = fs::read(sources.v069.join(format!("{root}/render/bind_pose")))?;
                same_rig_for(&pose, bind_pose, &label, Some(&used))?;
                // As 0.69d0 draws it: its vertices, skin, basis and UVs.
                crate::v069d_skinned::to_026_four_bone(&parsed)
            };
            let mut bare = vec![(
                surface.to_owned(),
                join(&format!("{} bare", face.key), &read(model, surface)?)?,
            )];
            if let Some(h) = hair {
                bare.push((h.to_owned(), read(model, h)?));
            }
            out.insert(State::Bare, Drawn::new(format!("{} bare", face.key), bare));
            for (state, m) in [(State::Helmet, helmet), (State::Mask, mask)] {
                if let Some(m) = m {
                    out.insert(
                        state,
                        Drawn::new(
                            format!("{} {state:?}", face.key).to_lowercase(),
                            vec![(
                                surface.to_owned(),
                                join(
                                    &format!("{} {state:?}", face.key).to_lowercase(),
                                    &read(m, surface)?,
                                )?,
                            )],
                        ),
                    );
                }
            }
        }
        Donor::Stk2 {
            model,
            surface,
            scalp,
            ..
        } => {
            let exported = sources
                .stk2
                .join(STK2_EXPORTED)
                .join(format!("{model}.skinned_model"));
            let mut db =
                ResourceDb::open(&exported).with_context(|| exported.display().to_string())?;
            let bones = crate::duty_026::check_bind(&db.read("render/bind_pose")?, bind_pose)?;
            let path = sources.stk2.join(STK2_PLATFORM).join(format!(
                "{model}.skinned_model/render/{surface}/converted_model"
            ));
            let bytes = fs::read(&path).with_context(|| path.display().to_string())?;
            let (mut mesh, ..) = crate::duty_026::rotate_mesh(&bytes, bones)?;
            let (t, rms) = align_onto(&mesh, &native.positions, &native.triangles, rig.head_y())?;
            translate(&mut mesh, t)?;
            let stretched = stretch_neck(&mut mesh, seam.bottom() - STRETCH_BELOW_SEAM)?;
            let mesh = join(&format!("{} bare", face.key), &mesh)?;
            println!(
                "{}: moved {:.1} {:.1} {:.1} mm onto the native skull ({:.1} mm apart after); neck stretched down {:.1} mm",
                face.key,
                t[0] * 1000.0,
                t[1] * 1000.0,
                t[2] * 1000.0,
                rms * 1000.0,
                stretched * 1000.0
            );
            let surfaces = if scalp {
                let (bare, grafted) = graft_scalp(&mesh, native_head)?;
                vec![(surface.to_owned(), bare), (SCALP.to_owned(), grafted)]
            } else {
                vec![(surface.to_owned(), mesh)]
            };
            out.insert(
                State::Bare,
                Drawn::new(format!("{} bare", face.key), surfaces),
            );
        }
    }
    Ok((out, joins))
}

/// How far below the native neck's edge a short neck is stretched (m),
/// so the cut along the edge takes it all round.
const STRETCH_BELOW_SEAM: f64 = 0.005;

/// Positions of a 48-byte surface moved by `t`.
fn translate(mesh: &mut [u8], t: [f64; 3]) -> Result<()> {
    let n = read_u32(mesh, 8)?;
    for i in 0..n {
        for (k, d) in t.iter().enumerate() {
            let at = 12 + i * 48 + 4 * k;
            let v = f64::from(f32::from_le_bytes(mesh[at..at + 4].try_into()?)) + d;
            mesh[at..at + 4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
    Ok(())
}

/// Points spread over a mesh's triangles (`n` steps an edge).
fn dense(positions: &[[f64; 3]], triangles: &[[usize; 3]], n: usize) -> Vec<[f64; 3]> {
    let mut out = vec![];
    for t in triangles {
        let [a, b, c] = t.map(|v| positions[v]);
        for i in 0..=n {
            for j in 0..=n - i {
                let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                let w = 1.0 - u - v;
                out.push([0, 1, 2].map(|k| a[k] * w + b[k] * u + c[k] * v));
            }
        }
    }
    out
}

/// Nearest points on a 5 mm grid.
struct Grid {
    cells: std::collections::HashMap<[i64; 3], Vec<[f64; 3]>>,
}

impl Grid {
    const CELL: f64 = 0.005;
    fn new(points: &[[f64; 3]]) -> Self {
        let mut cells: std::collections::HashMap<[i64; 3], Vec<[f64; 3]>> = Default::default();
        for p in points {
            cells.entry(Self::key(*p)).or_default().push(*p);
        }
        Self { cells }
    }
    fn key(p: [f64; 3]) -> [i64; 3] {
        p.map(|v| (v / Self::CELL).floor() as i64)
    }
    fn nearest(&self, p: [f64; 3]) -> Option<[f64; 3]> {
        let k = Self::key(p);
        for reach in 1..6_i64 {
            let mut best: Option<([f64; 3], f64)> = None;
            for x in -reach..=reach {
                for y in -reach..=reach {
                    for z in -reach..=reach {
                        if let Some(c) = self.cells.get(&[k[0] + x, k[1] + y, k[2] + z]) {
                            for q in c {
                                let d = sub(*q, p);
                                let d = dot(d, d);
                                if best.is_none_or(|(_, b)| d < b) {
                                    best = Some((*q, d));
                                }
                            }
                        }
                    }
                }
            }
            if let Some((q, d)) = best
                && d.sqrt() <= reach as f64 * Self::CELL
            {
                return Some(q);
            }
        }
        None
    }
}

/// The move (x kept 0: both are symmetric) that lays a face's skull above
/// `above` onto the native one's, both ways point to surface, and the mean
/// distance left.
fn align_onto(
    mesh: &[u8],
    native: &[[f64; 3]],
    native_triangles: &[[usize; 3]],
    above: f64,
) -> Result<([f64; 3], f64)> {
    let face = parse_mesh(mesh, 48, "face")?;
    let ours = dense(&face.positions, &face.triangles, 4);
    let theirs: Vec<[f64; 3]> = dense(native, native_triangles, 10)
        .into_iter()
        .filter(|p| p[1] > above)
        .collect();
    let native_grid = Grid::new(&theirs);
    let mut t = [0.0_f64; 3];
    let mut rms = 0.0;
    for _ in 0..80 {
        let moved: Vec<[f64; 3]> = ours
            .iter()
            .map(|p| [p[0] + t[0], p[1] + t[1], p[2] + t[2]])
            .filter(|p| p[1] > above)
            .collect();
        let face_grid = Grid::new(&moved);
        let (mut g, mut sq, mut n) = ([0.0_f64; 3], 0.0, 0.0);
        for p in &moved {
            if let Some(q) = native_grid.nearest(*p) {
                let d = sub(q, *p);
                for k in 0..3 {
                    g[k] += d[k];
                }
                sq += dot(d, d);
                n += 1.0;
            }
        }
        for q in &theirs {
            if let Some(p) = face_grid.nearest(*q) {
                let d = sub(*q, p);
                for k in 0..3 {
                    g[k] += d[k];
                }
                sq += dot(d, d);
                n += 1.0;
            }
        }
        ensure!(n > 0.0, "the face and the native skull do not meet");
        rms = (sq / n).sqrt();
        t[1] += 0.8 * g[1] / n;
        t[2] += 0.8 * g[2] / n;
    }
    Ok((t, rms))
}

/// Stretches a neck that ends above the native one down to `target` (below
/// the native neck's edge, which it is then cut on): heights below the
/// native chin's (`NECK_TOP`) are mapped linearly from the face's bottom to
/// it. Returns how far the bottom came down (m).
fn stretch_neck(mesh: &mut [u8], target: f64) -> Result<f64> {
    /// Above this height nothing moves (m).
    const NECK_TOP: f64 = 1.56;
    let n = read_u32(mesh, 8)?;
    let y = |m: &[u8], i: usize| {
        f64::from(f32::from_le_bytes(
            m[12 + i * 48 + 4..12 + i * 48 + 8].try_into().unwrap(),
        ))
    };
    let bottom = (0..n).map(|i| y(mesh, i)).fold(f64::INFINITY, f64::min);
    if bottom <= target {
        return Ok(0.0);
    }
    ensure!(bottom < NECK_TOP, "the face has no neck");
    for i in 0..n {
        let v = y(mesh, i);
        if v < NECK_TOP {
            let s = (NECK_TOP - v) / (NECK_TOP - bottom);
            let w = NECK_TOP - s * (NECK_TOP - target);
            let at = 12 + i * 48 + 4;
            mesh[at..at + 4].copy_from_slice(&(w as f32).to_le_bytes());
        }
    }
    Ok(bottom - target)
}

/// The grafted scalp's surface (an additional model of the bare face).
const SCALP: &str = "scalp_s";
/// Rounds in which the scalp settles round its edge.
const SCALP_SETTLE: usize = 400;
/// A face's own skin top and a native-head scalp joined to it: the face
/// without the mesh at its top (the Novice's knit cap, modelled on his
/// head), and the native head cut along the face's open top, its cut edge
/// laid on that open edge and the rest of it moved with the edge (each
/// vertex by the mean of its neighbours' moves, the edge's held: a scalp
/// the size of his skull, with no ledge over his skin). The cut follows
/// the open edge's height round the head: where that edge runs lower than
/// its height there (behind the Novice's ears) a gap stays, which is not
/// right yet. The scalp keeps the native head's skin, basis and UVs and
/// draws with the native head's material.
fn graft_scalp(face: &[u8], native_head: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
    let mesh = parse_mesh(face, 48, "face")?;
    let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
    let mut part = vec![usize::MAX; neighbours.len()];
    let mut sizes = vec![];
    for start in 0..neighbours.len() {
        if part[start] != usize::MAX {
            continue;
        }
        let id = sizes.len();
        part[start] = id;
        let (mut stack, mut size) = (vec![start], 0);
        while let Some(k) = stack.pop() {
            size += 1;
            for &u in &neighbours[k] {
                if part[u] == usize::MAX {
                    part[u] = id;
                    stack.push(u);
                }
            }
        }
        sizes.push(size);
    }
    let top = (0..mesh.positions.len())
        .max_by(|&a, &b| mesh.positions[a][1].total_cmp(&mesh.positions[b][1]))
        .context("an empty face")?;
    let cap = part[weld[top]];
    let skin = (0..sizes.len())
        .max_by_key(|&c| sizes[c])
        .context("no skin")?;
    ensure!(
        cap != skin,
        "the face's top is its skin: no cap to take off"
    );
    let capped: std::collections::HashSet<[i64; 3]> = mesh
        .positions
        .iter()
        .zip(&weld)
        .filter(|(_, k)| part[**k] == cap)
        .map(|(p, _)| key(*p))
        .collect();
    let bare = crate::outfit_026::keep_triangles(face, |c| {
        !c.iter().all(|(p, _)| capped.contains(&key(*p)))
    })?;

    // The face's open top: (angle round the up axis, height).
    let kept = parse_mesh(&bare, 48, "face")?;
    // Each welded edge: how many triangles use it, and its ends.
    type Edge = (usize, [f64; 3], [f64; 3]);
    let mut uses: BTreeMap<([i64; 3], [i64; 3]), Edge> = BTreeMap::new();
    for t in &kept.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (pa, pb) = (kept.positions[a], kept.positions[b]);
            let k = if key(pa) < key(pb) {
                (key(pa), key(pb))
            } else {
                (key(pb), key(pa))
            };
            uses.entry(k).or_insert((0, pa, pb)).0 += 1;
        }
    }
    let azimuth = |p: [f64; 3]| p[0].atan2(p[2]);
    // The open top is the loop of open edges with the highest point (the
    // eyes' openings and the neck's end are loops of their own).
    let open: Vec<[[f64; 3]; 2]> = uses
        .values()
        .filter(|(n, _, _)| *n == 1)
        .map(|(_, a, b)| [*a, *b])
        .collect();
    let mut loop_of: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut loops = 0;
    for start in open.iter().flatten() {
        if loop_of.contains_key(&key(*start)) {
            continue;
        }
        loop_of.insert(key(*start), loops);
        let mut grew = true;
        while grew {
            grew = false;
            for [a, b] in &open {
                match (
                    loop_of.get(&key(*a)).copied(),
                    loop_of.get(&key(*b)).copied(),
                ) {
                    (Some(l), None) if l == loops => {
                        loop_of.insert(key(*b), l);
                        grew = true;
                    }
                    (None, Some(l)) if l == loops => {
                        loop_of.insert(key(*a), l);
                        grew = true;
                    }
                    _ => {}
                }
            }
        }
        loops += 1;
    }
    let highest = open
        .iter()
        .flatten()
        .max_by(|a, b| a[1].total_cmp(&b[1]))
        .context("the face has no open edge")?;
    let top_loop = loop_of[&key(*highest)];
    let rim_segments: Vec<[[f64; 3]; 2]> = open
        .iter()
        .filter(|[a, _]| loop_of[&key(*a)] == top_loop)
        .copied()
        .collect();
    // Its height round the head, a point in each 5 degrees.
    const RIM_BINS: usize = 72;
    let tau = std::f64::consts::TAU;
    let mut top: Vec<Option<(f64, f64)>> = vec![None; RIM_BINS];
    for p in rim_segments.iter().flatten() {
        let a = azimuth(*p);
        let bin = (((a + tau / 2.0) / tau * RIM_BINS as f64) as usize).min(RIM_BINS - 1);
        if top[bin].is_none_or(|(_, y)| p[1] > y) {
            top[bin] = Some((a, p[1]));
        }
    }
    let rim: Vec<(f64, f64)> = top.into_iter().flatten().collect();
    ensure!(rim.len() >= 8, "the face has no open top");
    let rim_at = |a: f64| -> f64 {
        let n = rim.len();
        let after = rim.iter().position(|q| q.0 >= a).unwrap_or(n);
        let (lo, hi) = (rim[(after + n - 1) % n], rim[after % n]);
        let span = (hi.0 - lo.0).rem_euclid(tau).max(1e-9);
        let t = ((a - lo.0).rem_euclid(tau) / span).clamp(0.0, 1.0);
        lo.1 + t * (hi.1 - lo.1)
    };

    // The native head above that line, its triangles across it clipped.
    let native = widen_to_four_bone(native_head)?;
    let head = parse_mesh(&native, 48, "native head")?;
    let above: Vec<f64> = head
        .positions
        .iter()
        .map(|&p| p[1] - rim_at(azimuth(p)))
        .collect();
    let vertex = |i: usize| native[12 + i * 48..12 + (i + 1) * 48].to_vec();
    let mut vertices: Vec<Vec<u8>> = vec![];
    let mut index: BTreeMap<(usize, usize), u16> = BTreeMap::new();
    let mut add = |vertices: &mut Vec<Vec<u8>>, a: usize, b: usize| -> Result<u16> {
        if let Some(&i) = index.get(&(a, b)) {
            return Ok(i);
        }
        let mut v = vertex(a);
        if a != b {
            // On the cut: a's vertex moved along a-b to it, its UV with it.
            let t = above[a] / (above[a] - above[b]);
            let f = |i: usize, at: usize| {
                f32::from_le_bytes(
                    native[12 + i * 48 + at..12 + i * 48 + at + 4]
                        .try_into()
                        .unwrap(),
                )
            };
            for at in [0, 4, 8, 40, 44] {
                let x = f(a, at) + t as f32 * (f(b, at) - f(a, at));
                v[at..at + 4].copy_from_slice(&x.to_le_bytes());
            }
        }
        let i = u16::try_from(vertices.len())?;
        vertices.push(v);
        index.insert((a, b), i);
        Ok(i)
    };
    let mut triangles: Vec<[u16; 3]> = vec![];
    for t in &head.triangles {
        let mut polygon = vec![];
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if above[a] > 0.0 {
                polygon.push(add(&mut vertices, a, a)?);
            }
            if (above[a] > 0.0) != (above[b] > 0.0) {
                let (inside, outside) = if above[a] > 0.0 { (a, b) } else { (b, a) };
                polygon.push(add(&mut vertices, inside, outside)?);
            }
        }
        for k in 1..polygon.len().saturating_sub(1) {
            triangles.push([polygon[0], polygon[k], polygon[k + 1]]);
        }
    }
    ensure!(!triangles.is_empty(), "no native scalp above the face");

    // The cut edge onto the face's open edge, the rest of the scalp with it.
    let on_rim = |p: [f64; 3]| -> [f64; 3] {
        let mut best = (f64::INFINITY, p);
        for [a, b] in &rim_segments {
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
    };
    let at = |v: &[u8]| {
        [0, 1, 2].map(|k| f64::from(f32::from_le_bytes(v[4 * k..4 * k + 4].try_into().unwrap())))
    };
    let positions: Vec<[f64; 3]> = vertices.iter().map(|v| at(v)).collect();
    let cut_edge: std::collections::HashSet<[i64; 3]> = index
        .iter()
        .filter(|((a, b), _)| a != b)
        .map(|(_, i)| key(positions[usize::from(*i)]))
        .collect();
    let scalp = crate::mask_fit::Mesh {
        positions: positions.clone(),
        triangles: triangles.iter().map(|t| t.map(usize::from)).collect(),
    };
    let (weld, neighbours) = crate::mask_fit::weld_vertices(&scalp);
    let mut held = vec![false; neighbours.len()];
    let mut moves = vec![[0.0_f64; 3]; neighbours.len()];
    for (i, p) in positions.iter().enumerate() {
        if cut_edge.contains(&key(*p)) {
            held[weld[i]] = true;
            moves[weld[i]] = sub(on_rim(*p), *p);
        }
    }
    for _ in 0..SCALP_SETTLE {
        moves = (0..moves.len())
            .map(|k| {
                if held[k] || neighbours[k].is_empty() {
                    return moves[k];
                }
                let n = neighbours[k].len() as f64;
                [0, 1, 2].map(|j| neighbours[k].iter().map(|&u| moves[u][j]).sum::<f64>() / n)
            })
            .collect();
    }
    let mut farthest = 0.0_f64;
    for (i, v) in vertices.iter_mut().enumerate() {
        let m = moves[weld[i]];
        farthest = farthest.max(dot(m, m).sqrt());
        for k in 0..3 {
            let x = (positions[i][k] + m[k]) as f32;
            v[4 * k..4 * k + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    println!(
        "  scalp: cap of {} vertices off; {} native triangles over the face's top ({:.3} to {:.3} m), its cut edge ({} vertices) on the face's open edge, the scalp moved with it by up to {:.1} mm",
        capped.len(),
        triangles.len(),
        rim.iter().map(|r| r.1).fold(f64::INFINITY, f64::min),
        rim.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max),
        held.iter().filter(|h| **h).count(),
        farthest * 1000.0
    );
    let mut out = vec![];
    let push = |out: &mut Vec<u8>, v: usize| -> Result<()> {
        out.extend_from_slice(&u32::try_from(v)?.to_le_bytes());
        Ok(())
    };
    push(&mut out, 3)?;
    push(&mut out, 4 + vertices.len() * 48)?;
    push(&mut out, vertices.len())?;
    for v in &vertices {
        out.extend_from_slice(v);
    }
    push(&mut out, 4)?;
    push(&mut out, 4 + triangles.len() * 6)?;
    push(&mut out, triangles.len() * 3)?;
    for i in triangles.iter().flatten() {
        out.extend_from_slice(&i.to_le_bytes());
    }
    Ok((bare, out))
}

/// A two-bone (40-byte) native surface in the four-bone (48-byte) layout:
/// its weight and the rest, a third of 0.
fn widen_to_four_bone(model: &[u8]) -> Result<Vec<u8>> {
    let n = read_u32(model, 8)?;
    ensure!(read_u32(model, 4)? == 4 + n * 40, "not a two-bone surface");
    let mut out = Vec::with_capacity(model.len() + n * 8);
    out.extend_from_slice(&model[..4]);
    out.extend_from_slice(&u32::try_from(4 + n * 48)?.to_le_bytes());
    out.extend_from_slice(&model[8..12]);
    for i in 0..n {
        let v = &model[12 + i * 40..12 + (i + 1) * 40];
        let w = f32::from_le_bytes(v[16..20].try_into()?);
        out.extend_from_slice(&v[..16]);
        for x in [w, 1.0 - w, 0.0] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(&v[20..40]);
    }
    out.extend_from_slice(&model[12 + n * 40..]);
    Ok(out)
}

/// Every face's drawn surfaces per state, the items made for a face, and
/// the fit record.
pub struct Fitted {
    pub faces: Vec<(Face, BTreeMap<State, Drawn>)>,
    pub items: Vec<FaceItem>,
    pub record: serde_json::Value,
    /// What still shows through or stands off: the gate fails on any that
    /// the open list does not name ([`build`]).
    pub failures: Vec<Misfit>,
}

/// A face that does not fit something.
#[derive(Clone, Debug, Serialize)]
pub struct Misfit {
    pub face: String,
    pub state: State,
    /// The worn set it shows through, or `<item> stands off`.
    pub what: String,
    /// How far through, or how much farther off (mm).
    pub mm: f64,
    pub pose: String,
}

/// An item's model made for one face ([`crate::face_shape::Carry`]).
pub struct FaceItem {
    pub face: Face,
    /// The item's label (`helmet/<name>`), its config and the entry the
    /// model is drawn from.
    pub label: String,
    pub config: String,
    pub key: String,
    /// The entry's parts (model, surface), and every surface of their
    /// models (the lower levels of detail too) carried, by (model, surface).
    pub parts: Vec<(String, String)>,
    pub surfaces: BTreeMap<(String, String), Vec<u8>>,
    pub smooth: usize,
    pub carried: Carried,
    /// The rounds it took to settle on the face ([`face_fit::settle`]) and
    /// the farthest a vertex moved in them (m).
    pub settled: (usize, f64),
}

#[derive(Serialize)]
struct StateRecord {
    face: String,
    state: State,
    model: String,
    worst: Vec<Measure>,
    worn_sets: usize,
    through: usize,
    /// Triangles of covered skin left out of this state's model.
    cut: usize,
    /// Chin and jaw against collars in a pose (not covered by them in the
    /// bind pose): the sets it happens in and the deepest (mm).
    contact_sets: usize,
    contact_mm: f64,
    /// Items whose seen edge stands off the skin, with the excess (mm).
    stand_off: Vec<face_fit::StandOff>,
}

/// One face while it is fitted.
struct Shaped {
    face: Face,
    /// What each state is drawn with before covered skin is cut.
    whole: BTreeMap<State, Drawn>,
    /// What each state is drawn with.
    drawn: BTreeMap<State, Drawn>,
    cut: BTreeMap<State, usize>,
    /// The wardrobe's covers drawn with a model made for this face.
    swaps: BTreeMap<usize, usize>,
    /// How many models were made for a cover, and the last one.
    tries: BTreeMap<usize, usize>,
    items: BTreeMap<usize, FaceItem>,
    stand_off: BTreeMap<usize, face_fit::StandOff>,
}

/// The smoothing of an item's carry, tried in turn until it fits: one
/// level, since [`face_fit::settle`] moves out what the smooth carry left
/// under the skin.
const CARRY_LEVELS: [usize; 1] = [crate::face_shape::CARRY_SMOOTH];
/// A variant's vertex its maker moved by more than this (m) is moved.
const MOVED: f64 = 0.0005;

fn merged_mesh(drawn: &Drawn) -> Result<crate::mask_fit::Mesh> {
    let mut out = crate::mask_fit::Mesh {
        positions: vec![],
        triangles: vec![],
    };
    for (label, bytes) in &drawn.surfaces {
        let m = parse_mesh(bytes, 48, label)?;
        let base = out.positions.len();
        out.positions.extend(m.positions);
        out.triangles
            .extend(m.triangles.into_iter().map(|t| t.map(|v| v + base)));
    }
    Ok(out)
}

/// Loads every face as its donor has it, joins its neck to the body
/// where the native head's is, leaves the skin no one can see under any
/// helmet (or any mask) out of that state's model, makes a model of each
/// helmet and mask that does not fit a face for that face, and measures
/// every face in every state under every live item that draws the head
/// ([`face_fit::sweep`], [`face_fit::stand_off`]).
pub fn fit_all(view: &mut ResourceDb, sources: &Sources<'_>, faces: &[Face]) -> Result<Fitted> {
    let bind_pose = view.read(HEAD_BIND_POSE)?;
    let native_head = view.read(HEAD_MODEL)?;
    let rig = Rig::new(&bind_pose, &native_head)?;
    let mut built: Vec<(Face, BTreeMap<State, Drawn>)> = vec![];
    let mut necks: Vec<(String, NeckJoin)> = vec![];
    for face in faces {
        if matches!(face.donor, Donor::Native) {
            continue;
        }
        let (variants, joins) = donor_faces(sources, &bind_pose, &native_head, &rig, face)?;
        for (label, j) in &joins {
            println!(
                "  {label}: neck cut {:.1} mm below the native edge ({} triangles), {} vertices laid onto the native neck (up to {:.1} mm, none above {:.3} m), {} that follow the Head as the donor made them",
                j.below * 1000.0,
                j.cut,
                j.moved,
                j.farthest * 1000.0,
                j.highest_moved,
                j.kept
            );
        }
        necks.extend(joins);
        built.push((*face, variants));
    }
    // FACES_NO_REPORT: the faces without the fit report, while looking at
    // their pictures.
    if std::env::var_os("FACES_NO_REPORT").is_some() {
        return Ok(Fitted {
            faces: built,
            items: vec![],
            record: json!({ "report": "skipped" }),
            failures: vec![],
        });
    }
    let mut wardrobe = Wardrobe::read(view, &rig)?;
    println!(
        "Measuring under {} helmets, {} masks, {} torso and back models ({} worn sets; {} items draw no head: {})",
        wardrobe.count(|s| matches!(s, face_fit::Slot::Helmet { .. })),
        wardrobe.count(|s| matches!(s, face_fit::Slot::Mask { .. })),
        wardrobe.count(|s| *s == face_fit::Slot::Body),
        wardrobe.worn.values().map(Vec::len).sum::<usize>(),
        wardrobe.headless.len(),
        wardrobe.headless.join(", ")
    );
    let poses = SampledPoses::read(view)?;
    let native = Posable::parse(&rig, "native head", &native_head, &bind_pose)?;
    let native_mesh = parse_mesh(&native_head, 40, "native head")?;
    let is_helmet: fn(&face_fit::Slot) -> bool = |s| matches!(s, face_fit::Slot::Helmet { .. });
    let is_mask: fn(&face_fit::Slot) -> bool = |s| matches!(s, face_fit::Slot::Mask { .. });
    let slot_of = |state: State| {
        if state == State::Helmet {
            is_helmet
        } else {
            is_mask
        }
    };
    // FACES_NO_CUT, FACES_NO_ITEMS: the numbers without one step.
    let (no_cut, no_items) = (
        std::env::var_os("FACES_NO_CUT").is_some(),
        std::env::var_os("FACES_NO_ITEMS").is_some(),
    );

    let mut donor_moves = vec![];
    let mut shaped: Vec<Shaped> = vec![];
    for (face, donor) in built {
        let bare = donor[&State::Bare].clone();
        // 0.69d0's hair is a surface of the bare face only, as in 0.69d0.
        let skins = match face.donor {
            Donor::V069 { .. } => 1,
            _ => bare.surfaces.len(),
        };
        let without_hair = |label: String| Drawn::new(label, bare.surfaces[..skins].to_vec());
        // What the donor's own variants moved, and whether it is in sight.
        for state in [State::Helmet, State::Mask] {
            let Some(variant) = donor.get(&state) else {
                continue;
            };
            let moved = crate::face_shape::moved_from(&bare.surfaces[0].1, &variant.surfaces[0].1)?;
            let sight = face_fit::covered(&wardrobe, variant, slot_of(state))?;
            let (mut count, mut seen, mut farthest, mut seen_farthest) = (0, 0, 0.0_f64, 0.0_f64);
            let mut under: BTreeMap<usize, usize> = BTreeMap::new();
            for (m, s) in moved.iter().zip(&sight.seen_under[0]) {
                let Some(m) = *m else { continue };
                if m <= MOVED {
                    continue;
                }
                count += 1;
                farthest = farthest.max(m);
                if !s.is_empty() {
                    seen += 1;
                    seen_farthest = seen_farthest.max(m);
                    for &i in s {
                        *under.entry(i).or_default() += 1;
                    }
                }
            }
            println!(
                "  {}: 0.69d0 moved {count} vertices of it (up to {:.1} mm); {seen} of them in sight under one of {} live items (up to {:.1} mm)",
                variant.label,
                farthest * 1000.0,
                under.len(),
                seen_farthest * 1000.0
            );
            donor_moves.push(json!({
                "variant": variant.label,
                "moved": count,
                "farthest_mm": farthest * 1000.0,
                "in_sight": seen,
                "in_sight_farthest_mm": seen_farthest * 1000.0,
                "in_sight_under": under.iter().map(|(i, n)| json!([wardrobe.covers[*i].label, n])).collect::<Vec<_>>(),
            }));
        }
        let helmet = donor
            .get(&State::Helmet)
            .cloned()
            .unwrap_or_else(|| without_hair(format!("{} helmet", face.key)));
        // Under a mask the face is the one drawn under a helmet: 0.69d0's
        // own mask variants push the nose, lips and chin in under its masks
        // by up to 34 mm, and under the live masks that leave them in sight
        // the face would be another one. A mask that does not fit a face
        // gets a model for it instead (0.69d0's `models.head_<n>`).
        // FACES_DONOR_MASK: 0.69d0's mask variants, for the numbers.
        let mask = if std::env::var_os("FACES_DONOR_MASK").is_some() {
            donor
                .get(&State::Mask)
                .cloned()
                .unwrap_or_else(|| helmet.clone())
        } else {
            Drawn::new(format!("{} mask", face.key), helmet.surfaces.clone())
        };
        let whole = BTreeMap::from([
            (State::Bare, bare),
            (State::Helmet, helmet),
            (State::Mask, mask),
        ]);
        let mut drawn = whole.clone();
        let mut cut = BTreeMap::new();
        for state in [State::Helmet, State::Mask] {
            if no_cut {
                continue;
            }
            let sight = face_fit::covered(&wardrobe, &whole[&state], slot_of(state))?;
            let d = drawn.get_mut(&state).unwrap();
            let mut gone = 0;
            for ((label, bytes), covered) in d.surfaces.iter_mut().zip(&sight.covered) {
                if let Some((kept, n)) = crate::face_shape::cut_covered(bytes, covered, label)? {
                    *bytes = kept;
                    gone += n;
                }
            }
            // The item that leaves the most of this face in sight: what
            // limits the cut.
            let mut seen_by: BTreeMap<usize, usize> = BTreeMap::new();
            for s in sight.seen_under.iter().flatten() {
                for &i in s {
                    *seen_by.entry(i).or_default() += 1;
                }
            }
            let least = seen_by.iter().max_by_key(|(_, n)| **n);
            println!(
                "  {}: {gone} triangles no one can see under any item of the slot left out{}",
                d.label,
                least.map_or(String::new(), |(i, n)| format!(
                    " ({} leaves {n} vertices in sight)",
                    wardrobe.covers[*i].label
                ))
            );
            cut.insert(state, gone);
        }
        shaped.push(Shaped {
            face,
            whole,
            drawn,
            cut,
            swaps: BTreeMap::new(),
            tries: BTreeMap::new(),
            items: BTreeMap::new(),
            stand_off: BTreeMap::new(),
        });
    }

    let slot_covers = |w: &Wardrobe, slot: fn(&face_fit::Slot) -> bool| -> Vec<usize> {
        (0..w.items).filter(|&i| slot(&w.covers[i].slot)).collect()
    };
    let helmets = slot_covers(&wardrobe, is_helmet);
    let masks = slot_covers(&wardrobe, is_mask);
    let edges: BTreeMap<usize, face_fit::SkinEdges> = helmets
        .iter()
        .chain(&masks)
        .map(|&i| (i, face_fit::skin_edges(&wardrobe.covers[i], &native)))
        .collect();
    let whole_posable = |d: &Drawn| -> Result<Vec<Posable>> {
        d.surfaces
            .iter()
            .map(|(label, bytes)| Posable::parse(&rig, label, bytes, &bind_pose))
            .collect()
    };

    // An item made for a face is settled on it in the bind pose: how it
    // sits on the face. Settled in the sampled poses too, cloth that
    // follows the neck (a hood's drape, a flap) was pushed centimetres out
    // of the way of a jaw that only meets it when the head bows.
    let settle_poses: Vec<&[crate::skeleton_animation::Frame]> = poses.all().take(1).collect();
    let mut sweeps: Vec<Option<face_fit::Sweep>> = (0..shaped.len() * 3).map(|_| None).collect();
    let mut open: Vec<usize> = (0..shaped.len()).collect();
    for round in 0.. {
        // The seen edges of each helmet and mask on each open face.
        for &f in &open {
            let face = &mut shaped[f];
            for (state, covers) in [(State::Helmet, &helmets), (State::Mask, &masks)] {
                let skin = whole_posable(&face.whole[&state])?;
                for &i in covers {
                    if round > 0 && !face.swaps.contains_key(&i) {
                        continue;
                    }
                    let now = &wardrobe.covers[face.swaps.get(&i).copied().unwrap_or(i)];
                    let mut off = face_fit::stand_off(&edges[&i], now, &skin);
                    off.item.clone_from(&wardrobe.covers[i].label);
                    face.stand_off.insert(i, off);
                }
            }
        }
        let jobs: Vec<Job<'_>> = open
            .iter()
            .flat_map(|&f| {
                let face = &shaped[f];
                State::ALL.map(|state| Job {
                    drawn: &face.drawn[&state],
                    state,
                    swaps: &face.swaps,
                    only: None,
                })
            })
            .collect();
        let swept = face_fit::sweep(&rig, &poses, &native, &wardrobe, &jobs)?;
        drop(jobs);
        for (k, sweep) in swept.into_iter().enumerate() {
            sweeps[open[k / 3] * 3 + k % 3] = Some(sweep);
        }
        // The helmets and masks that do not fit a face yet.
        let mut next = vec![];
        for &f in &open {
            let mut need: std::collections::BTreeSet<usize> = shaped[f]
                .stand_off
                .iter()
                .filter(|(_, o)| o.off > 0)
                .map(|(i, _)| *i)
                .collect();
            let stood_off = need.len();
            // A helmet or a mask the face shows through when it is worn
            // alone; then, of a helmet over a mask that each fit alone, both.
            let mut together = vec![];
            for (state, at) in [(State::Helmet, 1), (State::Mask, 2)] {
                let sweep = sweeps[f * 3 + at].as_ref().unwrap();
                for (w, m) in wardrobe.worn[&state].iter().zip(&sweep.measures) {
                    if m.margin >= 0.0 {
                        continue;
                    }
                    match w.covers[..] {
                        [i] if wardrobe.covers[i].slot != face_fit::Slot::Body => {
                            need.insert(i);
                        }
                        [_] => {}
                        _ => together.push(w.covers.clone()),
                    }
                }
            }
            for covers in together {
                if !covers.iter().any(|i| need.contains(i)) {
                    need.extend(covers);
                }
            }
            // What each needs read (the surfaces of its models, their
            // bind poses), then each made on a thread of its own.
            struct Todo {
                cover: usize,
                tried: usize,
                state: State,
                surfaces: BTreeMap<(String, String), Vec<u8>>,
                binds: Vec<Vec<u8>>,
            }
            let mut todo = vec![];
            for i in need {
                let tried = shaped[f].tries.get(&i).copied().unwrap_or(0);
                if no_items || tried >= CARRY_LEVELS.len() {
                    continue;
                }
                let cover = &wardrobe.covers[i];
                let mut surfaces = BTreeMap::new();
                for (model, _) in &cover.parts {
                    let root = format!("models/{model}.skinned_model/render/");
                    let all: Vec<String> = view
                        .paths_with_prefix(&root)
                        .filter_map(|p| p[root.len()..].strip_suffix("/converted_model"))
                        .map(str::to_owned)
                        .collect();
                    for surface in all {
                        if let std::collections::btree_map::Entry::Vacant(e) =
                            surfaces.entry((model.clone(), surface))
                        {
                            let bytes =
                                view.read(&format!("{root}{}/converted_model", e.key().1))?;
                            e.insert(bytes);
                        }
                    }
                }
                let binds = cover
                    .parts
                    .iter()
                    .map(|(model, _)| {
                        view.read(&format!("models/{model}.skinned_model/render/bind_pose"))
                    })
                    .collect::<Result<_>>()?;
                todo.push(Todo {
                    cover: i,
                    tried,
                    state: if is_helmet(&cover.slot) {
                        State::Helmet
                    } else {
                        State::Mask
                    },
                    surfaces,
                    binds,
                });
            }
            let face = &shaped[f];
            let make = |t: Todo| -> Result<(usize, usize, FaceItem, face_fit::Cover)> {
                let smooth = CARRY_LEVELS[t.tried];
                let carry = crate::face_shape::Carry::new(
                    &rig,
                    &native_mesh,
                    &merged_mesh(&face.whole[&t.state])?,
                );
                let cover = &wardrobe.covers[t.cover];
                let label = format!("{} for {}", cover.label, face.face.key);
                let mut carried = Carried::default();
                let mut surfaces = BTreeMap::new();
                for (key, bytes) in t.surfaces {
                    let (moved, how) = carry.apply(&bytes, smooth, &label)?;
                    if how.farthest > carried.farthest {
                        carried = how;
                    }
                    surfaces.insert(key, moved);
                }
                let mut posables = vec![];
                for (part, bind) in cover.parts.iter().zip(&t.binds) {
                    posables.push(Posable::parse(
                        &rig,
                        &label,
                        surfaces
                            .get(part)
                            .with_context(|| format!("{label}: no {}", part.1))?,
                        bind,
                    )?);
                }
                let mut made = wardrobe.for_face(t.cover, label, &posables);
                let settled = face_fit::settle(
                    &rig,
                    &poses.bind,
                    &settle_poses,
                    &bind_pose,
                    &native,
                    cover,
                    &mut made,
                    &face.whole[&t.state],
                )?;
                for (part, surface) in cover.parts.iter().zip(&made.surfaces) {
                    surface.write_positions(
                        surfaces
                            .get_mut(part)
                            .context("a part without its surface")?,
                    )?;
                }
                let item = FaceItem {
                    face: face.face,
                    label: cover.label.clone(),
                    config: cover.config.clone(),
                    key: cover.key.clone(),
                    parts: cover.parts.clone(),
                    surfaces,
                    smooth,
                    carried,
                    settled,
                };
                Ok((t.cover, t.tried, item, made))
            };
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let mut lots: Vec<Vec<Todo>> = (0..threads).map(|_| vec![]).collect();
            for (k, t) in todo.into_iter().enumerate() {
                lots[k % threads].push(t);
            }
            let done: Vec<(usize, usize, FaceItem, face_fit::Cover)> =
                std::thread::scope(|scope| {
                    let workers: Vec<_> = lots
                        .into_iter()
                        .filter(|lot| !lot.is_empty())
                        .map(|lot| {
                            let make = &make;
                            scope.spawn(move || {
                                lot.into_iter().map(make).collect::<Result<Vec<_>>>()
                            })
                        })
                        .collect();
                    workers
                        .into_iter()
                        .map(|w| {
                            w.join()
                                .map_err(|_| anyhow::anyhow!("an item's thread panicked"))?
                        })
                        .collect::<Result<Vec<_>>>()
                })?
                .into_iter()
                .flatten()
                .collect();
            let mut made = 0;
            let mut done = done;
            done.sort_by_key(|d| d.0);
            for (i, tried, item, cover) in done {
                let at = wardrobe.add(cover);
                let face = &mut shaped[f];
                face.swaps.insert(i, at);
                face.tries.insert(i, tried + 1);
                face.items.insert(i, item);
                made += 1;
            }
            if made > 0 {
                println!(
                    "  round {round}: {made} helmet and mask models made for {} ({stood_off} stood off its skin)",
                    shaped[f].face.key
                );
                next.push(f);
            }
        }
        if next.is_empty() {
            break;
        }
        open = next;
    }

    let mut states = vec![];
    let mut failures = vec![];
    for (f, face) in shaped.iter().enumerate() {
        for (at, state) in State::ALL.into_iter().enumerate() {
            let sweep = sweeps[f * 3 + at].as_ref().unwrap();
            let mut worst: Vec<Measure> = sweep
                .measures
                .iter()
                .filter(|m| m.margin.is_finite())
                .cloned()
                .collect();
            worst.sort_by(|a, b| a.margin.total_cmp(&b.margin));
            let through = worst.iter().filter(|m| m.margin < 0.0).count();
            let contact_sets = sweep.measures.iter().filter(|m| m.contacts > 0).count();
            let contact = sweep.measures.iter().map(|m| m.contact).fold(0.0, f64::max);
            let covers = match state {
                State::Bare => &[][..],
                State::Helmet => &helmets[..],
                State::Mask => &masks[..],
            };
            let stand_off: Vec<face_fit::StandOff> = covers
                .iter()
                .filter_map(|i| face.stand_off.get(i))
                .filter(|o| o.off > 0)
                .cloned()
                .collect();
            let differs = state != State::Bare
                && face.drawn[&state].surfaces != face.drawn[&State::Bare].surfaces;
            let model = if differs {
                format!("{}{}", face.face.key, variant_name(state))
            } else {
                face.face.key.to_owned()
            };
            println!(
                "  {} {:?} ({model}): through {through} of {} worn sets{}; chin and jaw meet {contact_sets} collars in a pose (up to {:.1} mm); {} items stand off",
                face.face.key,
                state,
                sweep.measures.len(),
                worst
                    .first()
                    .filter(|w| w.margin < 0.0)
                    .map_or(String::new(), |w| format!(
                        ", worst {:.1} mm under {} ({})",
                        w.margin * 1000.0,
                        w.worn,
                        w.pose
                    )),
                contact * 1000.0,
                stand_off.len()
            );
            for m in worst.iter().filter(|m| m.margin < 0.0) {
                failures.push(Misfit {
                    face: face.face.key.to_owned(),
                    state,
                    what: m.worn.clone(),
                    mm: -m.margin * 1000.0,
                    pose: m.pose.clone(),
                });
            }
            for o in &stand_off {
                failures.push(Misfit {
                    face: face.face.key.to_owned(),
                    state,
                    what: format!("{} stands off", o.item),
                    mm: o.excess * 1000.0,
                    pose: "bind pose".to_owned(),
                });
            }
            states.push(StateRecord {
                face: face.face.key.to_owned(),
                state,
                model,
                worn_sets: sweep.measures.len(),
                through,
                worst: worst.into_iter().take(5).collect(),
                cut: face.cut.get(&state).copied().unwrap_or(0),
                contact_sets,
                contact_mm: contact * 1000.0,
                stand_off,
            });
        }
    }
    let items: Vec<FaceItem> = shaped
        .iter_mut()
        .flat_map(|s| std::mem::take(&mut s.items).into_values())
        .collect();
    let record = json!({
        "misfits": failures,
        "clearance_bind_mm": face_fit::BIND_CLEARANCE * 1000.0,
        "clearance_posed_mm": face_fit::POSED_CLEARANCE * 1000.0,
        "native_tolerance_mm": face_fit::NATIVE_TOLERANCE * 1000.0,
        "stand_off_mm": face_fit::STAND_OFF * 1000.0,
        "poses": 1 + poses.clips.iter().map(|(_, c)| c.len()).sum::<usize>(),
        "clips": poses.clips.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "covers": wardrobe.covers[..wardrobe.items].iter().map(|c| json!({"item": c.label, "slot": c.slot})).collect::<Vec<_>>(),
        "headless": wardrobe.headless,
        "worn_sets": wardrobe.worn.iter().map(|(s, w)| (format!("{s:?}").to_lowercase(), w.len())).collect::<BTreeMap<_, _>>(),
        "necks": necks.iter().map(|(label, j)| json!({"model": label, "join": j})).collect::<Vec<_>>(),
        "donor_variants": donor_moves,
        "items_for_faces": items.iter().map(|i| json!({
            "face": i.face.key,
            "index": i.face.index,
            "item": i.label,
            "smooth": i.smooth,
            "carried": i.carried,
            "settle_rounds": i.settled.0,
            "settled_mm": i.settled.1 * 1000.0,
        })).collect::<Vec<_>>(),
        "states": states,
    });
    let faces = shaped
        .into_iter()
        .map(|s| {
            let bare = s.drawn[&State::Bare].surfaces.clone();
            let drawn = s
                .drawn
                .into_iter()
                .filter(|(state, d)| *state == State::Bare || d.surfaces != bare)
                .collect();
            (s.face, drawn)
        })
        .collect();
    Ok(Fitted {
        faces,
        items,
        record,
        failures,
    })
}

/// Builds the faces' bundle: `overlay/` with the models, materials,
/// textures and `gameplay/bodyparts/default`, `port-manifest.json`, the
/// fit record `faces-fit.json`, and `render/` (every face and item surface
/// with its colour map, for the pictures).
pub fn build(
    sources: &Sources<'_>,
    db: &Path,
    patches: &[PathBuf],
    output: &Path,
    measure_only: bool,
    open: Option<&Path>,
) -> Result<()> {
    let mut view = crate::novice_026::game_view(db, patches)?;
    if measure_only {
        fs::create_dir_all(output)?;
    } else {
        crate::weapon::prepare_empty_output(output)?;
    }
    // FACES_ONLY=<key>,..: only those faces, while looking into a fit
    // (with --measure-only).
    let only = std::env::var("FACES_ONLY").ok();
    let faces: Vec<Face> = FACES
        .into_iter()
        .filter(|f| {
            only.as_ref()
                .is_none_or(|o| o.split(',').any(|k| k == f.key))
        })
        .collect();
    ensure!(
        measure_only || faces.len() == FACES.len(),
        "FACES_ONLY is for --measure-only"
    );
    let mut fitted = fit_all(&mut view, sources, &faces)?;
    // The open list: what is known not to fit and why, a line each,
    // `face<TAB>state<TAB>what<TAB>mm<TAB>why`. A misfit it names, no deeper
    // than it says, does not stop the build; any other does.
    let mut listed: BTreeMap<(String, String, String), f64> = BTreeMap::new();
    if let Some(path) = open {
        for line in fs::read_to_string(path)
            .with_context(|| path.display().to_string())?
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let f: Vec<&str> = line.split('\t').collect();
            ensure!(
                f.len() == 5 && !f[4].trim().is_empty(),
                "open list: {line:?}"
            );
            listed.insert(
                (f[0].to_owned(), f[1].to_owned(), f[2].to_owned()),
                f[3].parse()
                    .with_context(|| format!("open list: {line:?}"))?,
            );
        }
    }
    let state_name = |s: State| format!("{s:?}").to_lowercase();
    // A misfit is open when the list names it for its state, or (a torso
    // is worn in every state) for another; a helmet over a mask when it
    // names either of the two.
    let bound = |m: &Misfit| -> Option<f64> {
        let named = |state: &str, what: &str| {
            listed
                .get(&(m.face.clone(), state.to_owned(), what.to_owned()))
                .copied()
        };
        let any = |what: &str| {
            ["bare", "helmet", "mask"]
                .iter()
                .filter_map(|s| named(s, what))
                .reduce(f64::max)
        };
        named(&state_name(m.state), &m.what)
            .or_else(|| any(&m.what))
            .or_else(|| {
                m.what
                    .split(" + ")
                    .filter(|p| *p != m.what)
                    .filter_map(any)
                    .reduce(f64::max)
            })
    };
    let (known, new): (Vec<&Misfit>, Vec<&Misfit>) = fitted
        .failures
        .iter()
        .partition(|m| bound(m).is_some_and(|mm| m.mm <= mm + 0.05));
    fitted.record["gate"] = json!(if !new.is_empty() {
        "failed"
    } else if known.is_empty() {
        "passed"
    } else {
        "passed but for the open list"
    });
    fitted.record["open"] = json!(known.len());
    fitted.record["new"] = json!(new.len());
    fs::write(
        output.join("faces-fit.json"),
        serde_json::to_vec_pretty(&fitted.record)?,
    )?;
    // FACES_WRITE_OPEN=<file>: this build's misfits as open list lines
    // (a torso once, under the state it is deepest in; no helmet over a
    // mask that one of the two explains), each with what kind it is: the
    // why to be checked and written out.
    if let Some(path) = std::env::var_os("FACES_WRITE_OPEN") {
        let mut worst: BTreeMap<(String, String), (f64, String, &str)> = BTreeMap::new();
        for m in &fitted.failures {
            let body = m.what.starts_with("torso/") || m.what.starts_with("back/");
            let why = if m.what.ends_with("stands off") {
                "its seen edge stands farther off this face than off the native head"
            } else if m.pose != "bind pose" {
                if body {
                    "in a pose only: the chin, jaw or throat, as its donor made them, against a collar that follows the chest"
                } else {
                    "in a pose only: the jaw or throat, as its donor made them, against the part of it that follows the neck"
                }
            } else if body {
                "under it in the bind pose"
            } else {
                "under it in the bind pose: its model for this face does not settle"
            };
            let mm = (m.mm * 10.0).ceil() / 10.0;
            let e = worst.entry((m.face.clone(), m.what.clone())).or_insert((
                0.0,
                state_name(m.state),
                why,
            ));
            if mm > e.0 {
                *e = (mm, state_name(m.state), why);
            }
        }
        let single =
            |face: &str, what: &str| worst.get(&(face.to_owned(), what.to_owned())).map(|w| w.0);
        let mut lines = vec!["# face\tstate\twhat\tmm\twhy".to_owned()];
        for ((face, what), (mm, state, why)) in &worst {
            // A helmet over a mask is named only when it is deeper than
            // either of the two alone.
            let alone = what
                .split(" + ")
                .filter(|p| p != what)
                .filter_map(|p| single(face, p))
                .reduce(f64::max);
            if alone.is_some_and(|a| *mm <= a + 0.05) {
                continue;
            }
            lines.push(format!("{face}\t{state}\t{what}\t{mm:.1}\t{why}"));
        }
        fs::write(path, lines.join("\n") + "\n")?;
    }
    println!(
        "The gate: {} misfits, {} of them on the open list, {} new",
        fitted.failures.len(),
        known.len(),
        new.len()
    );
    for m in new.iter().take(60) {
        println!(
            "  does not fit: {} {:?}: {} by {:.1} mm ({})",
            m.face, m.state, m.what, m.mm, m.pose
        );
    }
    // FACES_REPORT_ONLY: the bundle of faces that do not all fit, to look
    // at them (never for a pack).
    let report_only = std::env::var_os("FACES_REPORT_ONLY").is_some();
    let stop = !new.is_empty();
    let news = new.len();
    if !measure_only && (!stop || report_only) {
        write_bundle(&mut view, sources, &fitted, output)?;
    }
    ensure!(
        !stop || report_only || measure_only,
        "{news} misfits the open list does not name (faces-fit.json): no bundle"
    );
    Ok(())
}

/// The native materials the faces' materials copy: the native head's
/// (subsurface scattering, diffuse, normal, gloss and Fresnel maps) and,
/// for the hair, a released character material with an alpha test.
const HEAD_MATERIAL: &str =
    "material_instances/characters/human/black_market_01/black_market_01_head.material";
const HAIR_MATERIAL: &str =
    "material_instances/characters/human/edge_hunter_01/edge_hunter_01_torso_alpha.material";
const STAGE: &str = "material.g_stage.effect";
/// The atlas column of the faces' picker icons (`heads[i].icon`), row
/// `index + 1`: the first column no released or open pack uses (0.29's
/// equipment takes 12..=15).
pub const ICON_COLUMN: i32 = 16;
/// STK2's 2048 maps drawn at the native head's 1024.
const STK2_MAP: u32 = 1024;

/// A map's role in a native character material.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Diffuse,
    Normal,
    Gloss,
    Fresnel,
}

impl Role {
    fn key(self) -> &'static str {
        match self {
            Self::Diffuse => "texture_diffuse",
            Self::Normal => "texture_normal",
            Self::Gloss => "texture_roughness",
            Self::Fresnel => "texture_fresnel",
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Diffuse => "",
            Self::Normal => "_nmap",
            Self::Gloss => "_smap",
            Self::Fresnel => "_fmap",
        }
    }
}

fn field<'a>(c: &'a BinaryConfig, key: &str) -> Option<&'a V> {
    c.root
        .named("material")?
        .named("g_stage")?
        .named("effect")?
        .named(key)?
        .named("value")
}

fn texture_of(c: &BinaryConfig, key: &str, label: &str) -> Result<String> {
    match field(c, key) {
        Some(V::String(t)) if !t.is_empty() => Ok(t.clone()),
        _ => bail!("{label}: no {key}"),
    }
}

/// The colours of mip 0 of a DXT1 or DXT5 DDS, as luminance of its RGB
/// (a grey map's value), every texel.
fn luminances(dds: &[u8], label: &str) -> Result<Vec<f64>> {
    let info = crate::soc::source::parse_dds(dds, label)?;
    let (block, colour_at) = match &info.four_cc {
        b"DXT1" => (8, 0),
        b"DXT5" => (16, 8),
        other => bail!("{label}: {}", String::from_utf8_lossy(other)),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    let data = dds.get(128..128 + blocks * block).context("short DDS")?;
    let mut out = Vec::with_capacity(blocks * 16);
    for b in data.chunks_exact(block) {
        let c = &b[colour_at..colour_at + 8];
        let rgb = |v: u16| {
            [
                f64::from((v >> 11) & 31) / 31.0,
                f64::from((v >> 5) & 63) / 63.0,
                f64::from(v & 31) / 31.0,
            ]
        };
        let (v0, v1) = (
            u16::from_le_bytes([c[0], c[1]]),
            u16::from_le_bytes([c[2], c[3]]),
        );
        let (a, z) = (rgb(v0), rgb(v1));
        let four = block == 16 || v0 > v1;
        let palette = [
            a,
            z,
            [0, 1, 2].map(|k| {
                if four {
                    (2.0 * a[k] + z[k]) / 3.0
                } else {
                    (a[k] + z[k]) / 2.0
                }
            }),
            [0, 1, 2].map(|k| if four { (a[k] + 2.0 * z[k]) / 3.0 } else { 0.0 }),
        ];
        let bits = u32::from_le_bytes(c[4..8].try_into()?);
        for t in 0..16 {
            let p = palette[((bits >> (2 * t)) & 3) as usize];
            out.push(0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]);
        }
    }
    Ok(out)
}

fn percentile(mut v: Vec<f64>, q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// How skin answers light natively: the native head's gloss map at its 90th
/// percentile, and its mean Fresnel.
struct Skin {
    gloss_p90: f64,
    fresnel: f64,
}

impl Skin {
    fn read(view: &mut ResourceDb, head: &BinaryConfig) -> Result<Self> {
        let mut map = |key: &str| -> Result<Vec<f64>> {
            let t = texture_of(head, key, HEAD_MATERIAL)?;
            luminances(&view.read(&format!("textures/{t}.dds"))?, &t)
        };
        let gloss = map("texture_roughness")?;
        let fresnel = map("texture_fresnel")?;
        Ok(Self {
            gloss_p90: percentile(gloss, 0.9),
            fresnel: fresnel.iter().sum::<f64>() / fresnel.len() as f64,
        })
    }
}

fn set(c: &mut BinaryConfig, path: &str, value: V) -> Result<()> {
    c.root.set_path(path, value)
}

fn constant_range(c: &mut BinaryConfig, what: &str, min: f64, max: f64) -> Result<()> {
    set(
        c,
        &format!("{STAGE}.constant_{what}_min.value"),
        V::Float(min as f32),
    )?;
    set(
        c,
        &format!("{STAGE}.constant_{what}_max.value"),
        V::Float(max as f32),
    )
}

/// Writes a donor map at `textures/<dest>.dds` with the template's options
/// of the same role.
#[allow(clippy::too_many_arguments)]
fn write_map(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    template: &BinaryConfig,
    template_label: &str,
    role: Role,
    dds: &[u8],
    dest: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = crate::duty_026::texture_dds(dds, dest, role == Role::Diffuse)?;
    out.raw(&format!("textures/{dest}.dds"), &bytes, files)?;
    let options = parse_config(
        view,
        &format!(
            "textures/{}.options",
            texture_of(template, role.key(), template_label)?
        ),
        "texture options",
    )?;
    out.config(&format!("textures/{dest}.options"), &options, files)
}

/// The material of a face (or its hair): the template with the donor's maps
/// and the native skin's gloss range and Fresnel. Returns its name under
/// `material_instances/`.
#[allow(clippy::too_many_arguments)]
fn face_material(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    skin: &Skin,
    template_path: &str,
    name: &str,
    maps: &[(Role, Vec<u8>)],
    files: &mut Vec<BuiltFile>,
) -> Result<(String, serde_json::Value)> {
    let mut material = parse_config(view, template_path, "template material")?;
    let template = material.clone();
    let mut record = serde_json::Map::new();
    let mut gloss_max = None;
    for (role, dds) in maps {
        let dest = format!("ported/{PORT}/{name}{}", role.suffix());
        write_map(
            out,
            view,
            &template,
            template_path,
            *role,
            dds,
            &dest,
            files,
        )?;
        set(
            &mut material,
            &format!("{STAGE}.{}.value", role.key()),
            V::String(dest.clone()),
        )?;
        if *role == Role::Gloss {
            // 0.69d's smoothness on the native skin's gloss: its 90th
            // percentile where the native head's is.
            let p90 = percentile(luminances(dds, &dest)?, 0.9).max(1e-3);
            gloss_max = Some((skin.gloss_p90 / p90).min(1.0));
        }
        record.insert(role.key().to_owned(), json!(dest));
    }
    match gloss_max {
        Some(max) => constant_range(&mut material, "roughness", 0.0, max)?,
        // No map of its own: the native skin's gloss, constant (the
        // template's map stays bound, min = max).
        None => constant_range(
            &mut material,
            "roughness",
            skin.gloss_p90 / 2.0,
            skin.gloss_p90 / 2.0,
        )?,
    }
    // The skin's Fresnel, constant: 0.69d's metalness maps are no Fresnel.
    constant_range(&mut material, "fresnel", skin.fresnel, skin.fresnel)?;
    record.insert("gloss_max".into(), json!(gloss_max));
    record.insert("fresnel".into(), json!(skin.fresnel));
    let path = format!("ported/{PORT}/{name}");
    out.config(
        &format!("material_instances/{path}.material"),
        &material,
        files,
    )?;
    Ok((path, serde_json::Value::Object(record)))
}

/// A value of a 0.69d0 material's effect (`material.g_stage.effect.<key>.value`).
fn effect_069<'a>(c: &'a BinaryConfig, key: &str) -> Option<&'a V> {
    field(c, key)
}

fn number_069(c: &BinaryConfig, key: &str, label: &str) -> Result<f64> {
    match effect_069(c, key) {
        Some(V::Float(x)) => Ok(f64::from(*x)),
        Some(V::Integer(x)) => Ok(f64::from(*x)),
        other => bail!("{label}: {key} is {other:?}"),
    }
}

fn flag_069(c: &BinaryConfig, key: &str) -> bool {
    matches!(effect_069(c, key), Some(V::Boolean(true)))
}

/// The map a 0.69d0 material binds at `key` when its `use` flag is on and
/// its file is in the donor's files: (its DDS).
fn map_069(
    sources: &Sources<'_>,
    c: &BinaryConfig,
    key: &str,
    flag: &str,
) -> Result<Option<Vec<u8>>> {
    if !flag_069(c, flag) {
        return Ok(None);
    }
    let Some(V::String(path)) = effect_069(c, key) else {
        return Ok(None);
    };
    if path.is_empty() {
        return Ok(None);
    }
    let file = sources.v069.join(format!("textures/{path}.dds"));
    if !file.exists() {
        return Ok(None);
    }
    Ok(Some(
        fs::read(&file).with_context(|| file.display().to_string())?,
    ))
}

/// The 0.26g material of a 0.69d0 head or hair material, input for input
/// over a native template (the native head's for skin, a released alpha-
/// tested character material for hair): albedo to `texture_diffuse`,
/// normal map to `texture_normal`, smoothness map and its range to
/// `texture_roughness` (0.26g's map is a gloss: the light's roughness is
/// one less it) or the constant smoothness, the metalness map (its
/// `_fmap`) or constant to the Fresnel, `use_subsurface_scattering`, and
/// the hair's alpha test, its reference and its two sides. Returns its name
/// under `material_instances/` and a record.
#[allow(clippy::too_many_arguments)]
fn material_069(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    template_path: &str,
    donor_path: &str,
    name: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<(String, serde_json::Value)> {
    let file = sources
        .v069
        .join(format!("material_instances/{donor_path}.material"));
    let donor = BinaryConfig::parse(&fs::read(&file).with_context(|| file.display().to_string())?)
        .with_context(|| donor_path.to_owned())?;
    let mut material = parse_config(view, template_path, "template material")?;
    let template = material.clone();
    let mut record = serde_json::Map::new();
    let mut bind = |material: &mut BinaryConfig,
                    role: Role,
                    dds: &[u8],
                    files: &mut Vec<BuiltFile>|
     -> Result<()> {
        let dest = format!("ported/{PORT}/{name}{}", role.suffix());
        write_map(out, view, &template, template_path, role, dds, &dest, files)?;
        set(
            material,
            &format!("{STAGE}.{}.value", role.key()),
            V::String(dest.clone()),
        )?;
        record.insert(role.key().to_owned(), json!(dest));
        Ok(())
    };
    let albedo = map_069(sources, &donor, "t_albedo", "use_t_albedo")?
        .with_context(|| format!("{donor_path}: no albedo"))?;
    bind(&mut material, Role::Diffuse, &albedo, files)?;
    if let Some(normal) = map_069(sources, &donor, "t_normal", "use_t_normal")? {
        bind(&mut material, Role::Normal, &normal, files)?;
    }
    // Smoothness: 0.69d0's GGX alpha is (1 - s)^2; 0.26g's light takes r =
    // 1 - gloss and makes alpha 0.5 r^6 + 0.1 r. The map is baked texel by
    // texel to the gloss giving 0.69d0's alpha, its range then 0..1; a
    // constant smoothness is translated alike.
    let (smin, smax) = (
        number_069(&donor, "min_smoothness", donor_path)?,
        number_069(&donor, "max_smoothness", donor_path)?,
    );
    let smoothness = match map_069(sources, &donor, "t_smoothness", "use_t_smoothness")? {
        Some(map) => {
            let work = out.overlay.with_file_name("textures-work");
            let baked = bake_gloss(&map, smin, smax, &work, name)?;
            bind(&mut material, Role::Gloss, &baked, files)?;
            constant_range(&mut material, "roughness", 0.0, 1.0)?;
            json!({"map": [smin, smax]})
        }
        None => {
            let s = number_069(&donor, "smoothness", donor_path)?;
            let g = gloss_of_smoothness(s);
            constant_range(&mut material, "roughness", g, g)?;
            json!({"constant": s, "gloss": g})
        }
    };
    // Metalness: 0.69d0's F0 is 0.04 + m (albedo - 0.04) and its diffuse
    // albedo (1 - m). 0.26g's Fresnel is x + y t_fresnel.G: the metalness map
    // as it is, x 0.04 and y the albedo's mean luminance less 0.04; the
    // diffuse scaled by one less the mean metalness.
    // Over the texels the surface uses (the atlas' black gaps left out).
    let albedo_mean = {
        let l: Vec<f64> = luminances(&albedo, donor_path)?
            .into_iter()
            .filter(|&x| x > 0.02)
            .collect();
        l.iter().sum::<f64>() / l.len().max(1) as f64
    };
    let metalness = match map_069(sources, &donor, "t_metalness", "use_t_metalness")? {
        Some(map) => {
            bind(&mut material, Role::Fresnel, &map, files)?;
            constant_range(
                &mut material,
                "fresnel",
                DIELECTRIC_F0,
                albedo_mean.max(DIELECTRIC_F0),
            )?;
            let l = luminances(&map, donor_path)?;
            l.iter().sum::<f64>() / l.len() as f64
        }
        None => {
            let m = number_069(&donor, "metalness", donor_path)?;
            let f = DIELECTRIC_F0 + m * (albedo_mean - DIELECTRIC_F0);
            constant_range(&mut material, "fresnel", f, f)?;
            m
        }
    };
    let d = (1.0 - metalness) as f32;
    set(
        &mut material,
        &format!("{STAGE}.constant_diffuse.value"),
        V::Float4([d, d, d, 1.0]),
    )?;
    // 0.69d0 names use_subsurface_scattering but never reads it: it has no
    // subsurface scattering (nor the masks and thickness maps it names).
    set(
        &mut material,
        &format!("{STAGE}.use_subsurface_scattering.value"),
        V::Boolean(false),
    )?;
    if flag_069(&donor, "use_alpha_test") {
        set(
            &mut material,
            &format!("{STAGE}.use_alpha_test.value"),
            V::Boolean(true),
        )?;
        set(
            &mut material,
            &format!("{STAGE}.alpha_ref.value"),
            V::Float(number_069(&donor, "alpha_ref", donor_path)? as f32),
        )?;
    }
    let two_sided = matches!(
        donor
            .root
            .named("material")
            .and_then(|m| m.named("g_stage"))
            .and_then(|g| g.named("two_sided"))
            .and_then(|t| t.named("value")),
        Some(V::Boolean(true))
    );
    set(
        &mut material,
        &format!("{STAGE}.two_sided.value"),
        V::Boolean(two_sided),
    )?;
    record.insert("smoothness".into(), smoothness);
    record.insert("metalness_mean".into(), json!(metalness));
    record.insert("albedo_mean".into(), json!(albedo_mean));
    record.insert("two_sided".into(), json!(two_sided));
    let path = format!("ported/{PORT}/{name}");
    out.config(
        &format!("material_instances/{path}.material"),
        &material,
        files,
    )?;
    Ok((path, serde_json::Value::Object(record)))
}

/// A dielectric's reflectance at normal incidence, 0.69d0's F0 at metalness 0.
const DIELECTRIC_F0: f64 = 0.04;

/// 0.26g's gloss giving 0.69d0's GGX alpha for smoothness `s`: 0.69d0's
/// alpha is (1 - s)^2, 0.26g's 0.5 r^6 + 0.1 r with r one less the gloss
/// (it reaches 0.6 at r = 1: a rougher 0.69d0 surface is drawn at r = 1).
fn gloss_of_smoothness(s: f64) -> f64 {
    let alpha = (1.0 - s.clamp(0.0, 1.0)).powi(2);
    let f = |r: f64| 0.5 * r.powi(6) + 0.1 * r;
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    if alpha >= f(1.0) {
        return 0.0;
    }
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if f(mid) < alpha {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    1.0 - (lo + hi) / 2.0
}

/// A 0.69d0 smoothness map (`min + (max - min) * texel`) baked to 0.26g's
/// gloss ([`gloss_of_smoothness`]) through a 256-entry lookup, as a DXT1 DDS.
fn bake_gloss(map: &[u8], min: f64, max: f64, work: &Path, name: &str) -> Result<Vec<u8>> {
    fs::create_dir_all(work)?;
    let lut = work.join(format!("{name}_gloss_lut.pgm"));
    let mut text = String::from("P2\n256 1\n255\n");
    for t in 0..256 {
        let s = min + (max - min) * f64::from(t) / 255.0;
        text.push_str(&format!(
            "{}\n",
            (gloss_of_smoothness(s) * 255.0).round() as u8
        ));
    }
    fs::write(&lut, text)?;
    let source = work.join(format!("{name}_smap.dds"));
    fs::write(&source, map)?;
    let tga = work.join(format!("{name}_gloss.tga"));
    magick(&[
        &source.display().to_string(),
        "-alpha",
        "off",
        &lut.display().to_string(),
        "-clut",
        "-type",
        "TrueColor",
        "-compress",
        "None",
        &tga.display().to_string(),
    ])?;
    let dds = tga.with_extension("dds");
    crate::armor::convert_texture(&tga, &dds, crate::manifest::TextureCompression::Dxt1)?;
    Ok(fs::read(&dds)?)
}

/// Runs ImageMagick with `args`; its standard output.
fn magick(args: &[&str]) -> Result<String> {
    let result = std::process::Command::new("magick")
        .args(args)
        .output()
        .context("starting ImageMagick; enter the repository Nix shell")?;
    ensure!(
        result.status.success(),
        "ImageMagick: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

/// The mean colour of a region (`WxH+X+Y`) of an image.
fn mean_rgb(image: &Path, region: &str) -> Result<[f64; 3]> {
    let text = magick(&[
        &image.display().to_string(),
        "-crop",
        region,
        "+repage",
        "-format",
        "%[fx:mean.r] %[fx:mean.g] %[fx:mean.b]",
        "info:",
    ])?;
    let v: Vec<f64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    ensure!(v.len() == 3, "no mean colour of {region}: {text}");
    Ok([v[0], v[1], v[2]])
}

/// The native head's forehead in its 1024 colour map, and a patch of its
/// short side hair (mirrored into a band, over the forehead's upper part,
/// at [`SCALP_HAIR_AT`], feathered): the grafted scalp's hairline sits
/// there, a forehead's width above the face's brows.
const NATIVE_FOREHEAD: &str = "180x80+420+160";
const NATIVE_SIDE_HAIR: &str = "200x120+90+150";
const SCALP_HAIR_SIZE: (u32, u32) = (420, 170);
const SCALP_HAIR_AT: &str = "+302+70";
/// The STK2 face's own forehead in its 2048 colour map (below the cap's
/// shadow).
const STK2_FOREHEAD: &str = "150x20+950+860";
/// The native forehead's lowest skin, stretched over its brows and eyes.
const NATIVE_BROW_SKIN: &str = "300x16+362+246";
const NATIVE_BROWS_SIZE: &str = "300x90!";
const NATIVE_BROWS_AT: &str = "+362+258";

/// The grafted scalp's colour map: the native head's, all hair above the
/// hairline, toned to the face's own skin (the mean of its forehead over
/// the native one's, a gain a channel).
fn scalp_diffuse(
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    face: &Face,
    work: &Path,
) -> Result<Vec<u8>> {
    let Donor::Stk2 { texture, .. } = face.donor else {
        bail!("{}: only a STK2 face has a grafted scalp", face.key);
    };
    fs::create_dir_all(work)?;
    let head = parse_config(view, HEAD_MATERIAL, "head material")?;
    let native = work.join("native_head.dds");
    fs::write(
        &native,
        view.read(&format!(
            "textures/{}.dds",
            texture_of(&head, "texture_diffuse", HEAD_MATERIAL)?
        ))?,
    )?;
    let own = sources
        .stk2_textures
        .join(STK2_TEXTURES)
        .join(format!("{texture}.tga"));
    let (theirs, ours) = (
        mean_rgb(&native, NATIVE_FOREHEAD)?,
        mean_rgb(&own, STK2_FOREHEAD)?,
    );
    let gain: Vec<String> = (0..3)
        .map(|k| format!("{:.4}", ours[k] / theirs[k].max(1e-3)))
        .collect();
    let (w, h) = SCALP_HAIR_SIZE;
    let tga = work.join(format!("{}_scalp.tga", face.key));
    let n = native.display().to_string();
    let size = format!("{w}x{h}!");
    let mask = format!(
        "roundrectangle {},{} {},{} {r},{r}",
        w / 16,
        h / 8,
        w - w / 16,
        h - h * 3 / 16,
        r = h / 4
    );
    magick(&[
        &n,
        "-alpha",
        "off",
        // The brows and eyes under the forehead skin above them: the scalp
        // comes down to the face's brows at the temples.
        "(",
        "+clone",
        "-crop",
        NATIVE_BROW_SKIN,
        "+repage",
        "-resize",
        NATIVE_BROWS_SIZE,
        ")",
        "-geometry",
        NATIVE_BROWS_AT,
        "-composite",
        "(",
        &n,
        "-alpha",
        "off",
        "-crop",
        NATIVE_SIDE_HAIR,
        "+repage",
        "(",
        "+clone",
        "-flop",
        ")",
        "+append",
        "-resize",
        &size,
        "-modulate",
        "110",
        "(",
        "-size",
        &format!("{w}x{h}"),
        "xc:black",
        "-fill",
        "white",
        "-draw",
        &mask,
        "-blur",
        "0x16",
        ")",
        "-alpha",
        "off",
        "-compose",
        "CopyOpacity",
        "-composite",
        ")",
        "-geometry",
        SCALP_HAIR_AT,
        "-compose",
        "over",
        "-composite",
        "-channel",
        "R",
        "-evaluate",
        "multiply",
        &gain[0],
        "-channel",
        "G",
        "-evaluate",
        "multiply",
        &gain[1],
        "-channel",
        "B",
        "-evaluate",
        "multiply",
        &gain[2],
        "+channel",
        "-type",
        "TrueColor",
        "-compress",
        "None",
        &tga.display().to_string(),
    ])?;
    let dds = tga.with_extension("dds");
    crate::armor::convert_texture(&tga, &dds, crate::manifest::TextureCompression::Dxt1)?;
    Ok(fs::read(&dds)?)
}

/// The grafted scalp's material: the native head's, with
/// [`scalp_diffuse`] for its colour (its normal, gloss and Fresnel maps
/// the native head's own: the scalp keeps its UVs).
fn scalp_material(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    face: &Face,
    work: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<(String, serde_json::Value)> {
    let dds = scalp_diffuse(view, sources, face, work)?;
    let mut material = parse_config(view, HEAD_MATERIAL, "head material")?;
    let template = material.clone();
    let name = format!("{}_scalp", face.key);
    let dest = format!("ported/{PORT}/{name}");
    write_map(
        out,
        view,
        &template,
        HEAD_MATERIAL,
        Role::Diffuse,
        &dds,
        &dest,
        files,
    )?;
    set(
        &mut material,
        &format!("{STAGE}.texture_diffuse.value"),
        V::String(dest.clone()),
    )?;
    let path = format!("ported/{PORT}/{name}");
    out.config(
        &format!("material_instances/{path}.material"),
        &material,
        files,
    )?;
    Ok((
        path,
        json!({ "texture_diffuse": dest, "from": "the native head's material" }),
    ))
}

/// A face's maps: 0.69d's DDS as they are, STK2's TGAs encoded at the
/// native head's size.
fn donor_maps(
    sources: &Sources<'_>,
    face: &Face,
    work: &Path,
    hair: bool,
) -> Result<Vec<(Role, Vec<u8>)>> {
    match face.donor {
        Donor::V069 { model, hair: h, .. } => {
            let stem = match (hair, h) {
                (false, _) => model.to_owned(),
                (true, Some(_)) => format!("{model}_hair"),
                (true, None) => bail!("{model} has no hair"),
            };
            let dir = sources
                .v069
                .join(format!("textures/character/human/head/{model}"));
            let mut out = vec![];
            for role in [Role::Diffuse, Role::Normal, Role::Gloss] {
                let path = dir.join(format!("{stem}{}.dds", role.suffix()));
                if role == Role::Gloss && !path.exists() {
                    continue;
                }
                out.push((
                    role,
                    fs::read(&path).with_context(|| path.display().to_string())?,
                ));
            }
            Ok(out)
        }
        Donor::Stk2 { texture, scalp, .. } => {
            ensure!(!hair, "STK2 faces have no hair");
            fs::create_dir_all(work)?;
            let mut out = vec![];
            for (role, compression) in [
                (Role::Diffuse, crate::manifest::TextureCompression::Dxt1),
                (
                    Role::Normal,
                    crate::manifest::TextureCompression::Dxt5Normal,
                ),
            ] {
                let mut source = sources
                    .stk2_textures
                    .join(STK2_TEXTURES)
                    .join(format!("{texture}{}.tga", role.suffix()));
                if scalp && role == Role::Diffuse {
                    let uncapped = work.join(format!("{}_uncapped.ppm", face.key));
                    uncapped_map(&source, &uncapped)?;
                    source = uncapped;
                }
                let small = work.join(format!("{}{}.tga", face.key, role.suffix()));
                let result = std::process::Command::new("magick")
                    .arg(&source)
                    .args(["-alpha", "off", "-filter", "Lanczos"])
                    .args(["-resize", &format!("{STK2_MAP}x{STK2_MAP}!")])
                    .args(["-type", "TrueColor", "-compress", "None"])
                    .arg(&small)
                    .output()
                    .context("starting ImageMagick; enter the repository Nix shell")?;
                ensure!(
                    result.status.success(),
                    "ImageMagick: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let dds = small.with_extension("dds");
                crate::armor::convert_texture(&small, &dds, compression)?;
                out.push((role, fs::read(&dds)?));
            }
            Ok(out)
        }
        Donor::Native => bail!("the native head has its maps"),
    }
}

/// The forehead in a STK2 face's 2048 colour map whose top its cap
/// shadowed (columns, and the rows its open top and first skin are in),
/// and the forehead skin that sets the skin's brightness there.
const STK2_SHADOW_COLUMNS: (usize, usize) = (480, 1580);
const STK2_SHADOW_ROWS: (usize, usize) = (760, 1000);
const STK2_FOREHEAD_SKIN: ((usize, usize), (usize, usize)) = ((950, 1100), (860, 880));

/// A capped face's forehead without its cap's shadow: in each column of
/// [`STK2_SHADOW_COLUMNS`], the band from the chart's top (above 15 % of
/// the forehead skin's brightness) to its skin (above 70 %) takes the first
/// skin rows' colours, repeated. `rgb` is the map's rows of RGB bytes.
fn lift_cap_shadow(rgb: &mut [u8], width: usize) -> usize {
    let lum = |rgb: &[u8], x: usize, y: usize| {
        let p = &rgb[(y * width + x) * 3..];
        (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])) / 255.0
    };
    let ((x0, x1), (y0, y1)) = STK2_FOREHEAD_SKIN;
    let mut skin: Vec<f64> = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| lum(rgb, x, y))
        .collect();
    skin.sort_by(f64::total_cmp);
    let reference = skin[skin.len() / 2];
    let (top, bottom) = STK2_SHADOW_ROWS;
    let mut lifted = 0;
    for x in STK2_SHADOW_COLUMNS.0..STK2_SHADOW_COLUMNS.1 {
        let Some(start) = (top..bottom).find(|&y| lum(rgb, x, y) > 0.15 * reference) else {
            continue;
        };
        let Some(skin_at) = (start..bottom).find(|&y| lum(rgb, x, y) > 0.7 * reference) else {
            continue;
        };
        if skin_at - start > 120 || skin_at - start < 2 {
            continue;
        }
        for y in start.saturating_sub(6)..skin_at {
            let from = skin_at + 2 + (skin_at - y) % 10;
            for c in 0..3 {
                rgb[(y * width + x) * 3 + c] = rgb[(from * width + x) * 3 + c];
            }
        }
        lifted += 1;
    }
    lifted
}

/// A STK2 face's colour map (`source`) as an RGB PPM at `out`, its cap's
/// shadow lifted ([`lift_cap_shadow`]).
fn uncapped_map(source: &Path, out: &Path) -> Result<()> {
    let result = std::process::Command::new("magick")
        .arg(source)
        .args(["-alpha", "off", "-depth", "8", "ppm:-"])
        .output()
        .context("starting ImageMagick; enter the repository Nix shell")?;
    ensure!(
        result.status.success(),
        "ImageMagick: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut ppm = result.stdout;
    // "P6 <w> <h> 255" and one whitespace byte, then the rows.
    let mut fields = vec![];
    let mut at = 0;
    while fields.len() < 4 {
        while ppm[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while !ppm[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&ppm[start..at]).into_owned());
    }
    at += 1;
    ensure!(
        fields[0] == "P6" && fields[3] == "255",
        "not an 8-bit PPM: {fields:?}"
    );
    let width: usize = fields[1].parse()?;
    let height: usize = fields[2].parse()?;
    ensure!(
        width == 2048 && height == 2048,
        "{}: a {width}x{height} map, not STK2's 2048",
        source.display()
    );
    let lifted = lift_cap_shadow(&mut ppm[at..at + width * height * 3], width);
    println!("  the cap's shadow lifted off {lifted} columns of the forehead");
    fs::write(out, ppm)?;
    Ok(())
}

/// Bounds as native model properties store them: half a turn about the up
/// axis (`-x, y, -z`).
fn stored_bounds(min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    ([-max[0], min[1], -max[2]], [-min[0], max[1], -min[2]])
}

/// The model folder of one drawn face: the native head's bind pose,
/// skeleton, settings and properties around its surfaces.
fn model_files(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    root: &str,
    drawn: &Drawn,
    materials: &[String],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let native = "models/character/human/base/base_character_head.skinned_model";
    ensure!(
        !view.contains(&format!("{root}/settings")),
        "{root} is in the released game"
    );
    out.raw(
        &format!("{root}/render/bind_pose"),
        &view.read(&format!("{native}/render/bind_pose"))?,
        files,
    )?;
    out.raw(
        &format!("{root}/skeleton"),
        &view.read(&format!("{native}/skeleton"))?,
        files,
    )?;
    let mut settings = parse_config(view, &format!("{native}/settings"), "head settings")?;
    let Some(V::Named(entries)) = settings.root.named_mut("material_settings") else {
        bail!("head settings without material_settings");
    };
    ensure!(entries.len() == 1, "the native head has other surfaces");
    let template = entries[0].value.clone();
    entries.clear();
    for ((surface, _), material) in drawn.surfaces.iter().zip(materials) {
        let mut v = template.clone();
        v.set_path("material_name", s(material))?;
        entries.push(NamedValue {
            name: surface.clone(),
            value: v,
        });
    }
    set(
        &mut settings,
        "lod_hierrarchy.LOD0.surfaces",
        V::Indexed(drawn.surfaces.iter().map(|(n, _)| s(n)).collect()),
    )?;
    out.config(&format!("{root}/settings"), &settings, files)?;
    let (mut all_min, mut all_max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for (surface, bytes) in &drawn.surfaces {
        let (min, max) = crate::outfit_026::converted_bounds(bytes)?;
        for k in 0..3 {
            all_min[k] = all_min[k].min(min[k]);
            all_max[k] = all_max[k].max(max[k]);
        }
        out.raw(
            &format!("{root}/render/{surface}/converted_model"),
            bytes,
            files,
        )?;
        let mut props = parse_config(
            view,
            &format!("{native}/render/{NATIVE_PART}/export_properties"),
            "surface properties",
        )?;
        let vertices = read_u32(bytes, 8)?;
        let indices = read_u32(bytes, 12 + vertices * 48 + 8)?;
        for (key, value) in [
            ("source.vertices", vertices),
            ("source.indices", indices),
            ("source.primitives", indices / 3),
        ] {
            set(&mut props, key, V::Integer(i32::try_from(value)?))?;
        }
        // 43 + bones a vertex.
        set(&mut props, "type", V::Integer(47))?;
        set(
            &mut props,
            "source.mesh_name",
            s(format!("|{}|{surface}", drawn.label)),
        )?;
        let (lo, hi) = stored_bounds(min, max);
        set(&mut props, "bounding_box.min", V::Float3(lo))?;
        set(&mut props, "bounding_box.max", V::Float3(hi))?;
        out.config(
            &format!("{root}/render/{surface}/export_properties"),
            &props,
            files,
        )?;
    }
    let mut render = parse_config(
        view,
        &format!("{native}/render/export_properties"),
        "render properties",
    )?;
    let (lo, hi) = stored_bounds(all_min, all_max);
    set(&mut render, "bounding_box.min", V::Float3(lo))?;
    set(&mut render, "bounding_box.max", V::Float3(hi))?;
    set(
        &mut render,
        "src_filename",
        s(format!("skhid/{PORT}/{}", drawn.label)),
    )?;
    out.config(&format!("{root}/render/export_properties"), &render, files)
}

fn named(entries: Vec<(&str, V)>) -> V {
    V::Named(
        entries
            .into_iter()
            .map(|(name, value)| NamedValue {
                name: name.to_owned(),
                value,
            })
            .collect(),
    )
}

/// One `{base_model, part_name, additional_models}` record.
fn record(model: &str, surfaces: &[String]) -> V {
    let mut fields = vec![("base_model", s(model)), ("part_name", s(&surfaces[0]))];
    if surfaces.len() > 1 {
        fields.push((
            "additional_models",
            V::Indexed(
                surfaces[1..]
                    .iter()
                    .map(|p| named(vec![("base_model", s(model)), ("part_name", s(p))]))
                    .collect(),
            ),
        ));
    }
    named(fields)
}

/// `gameplay/bodyparts/default` with `body_parts.heads`: the native head,
/// then every face.
fn bodyparts(view: &mut ResourceDb, heads: Vec<V>) -> Result<BinaryConfig> {
    let mut c = parse_config(view, BODYPARTS, "body parts")?;
    let parts = c.root.named("body_parts").context("no body_parts")?;
    ensure!(
        parts.named("heads").is_none(),
        "{BODYPARTS} has heads already"
    );
    let head = parts
        .named("head")
        .and_then(|h| h.named("data"))
        .context("no body_parts.head.data")?;
    ensure!(
        head.named("base_model") == Some(&s(NATIVE_HEAD))
            && head.named("part_name") == Some(&s(NATIVE_PART)),
        "body_parts.head is not the native head"
    );
    let Some(V::Named(fields)) = c.root.named_mut("body_parts") else {
        bail!("body_parts is not a table");
    };
    fields.push(NamedValue {
        name: "heads".into(),
        value: V::Indexed(heads),
    });
    Ok(c)
}

fn icon(index: usize) -> V {
    V::Indexed(vec![V::Integer(index as i32 + 1), V::Integer(ICON_COLUMN)])
}

fn write_bundle(
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    fitted: &Fitted,
    output: &Path,
) -> Result<()> {
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "faces-026",
    };
    let mut files = vec![];
    let head_material = parse_config(view, HEAD_MATERIAL, "head material")?;
    let skin = Skin::read(view, &head_material)?;
    let mut heads = vec![named(vec![
        ("data", record(NATIVE_HEAD, &[NATIVE_PART.to_owned()])),
        ("icon", icon(0)),
    ])];
    let mut materials_record = serde_json::Map::new();
    let render = output.join("render");
    fs::create_dir_all(&render)?;
    let mut scene = vec![];
    let mut item_scene = vec![];
    for (face, variants) in &fitted.faces {
        // The face's material, and its other surface's (0.69d0's hair, or
        // the native scalp grafted on a STK2 face).
        let (face_mat, extra_mat) = match face.donor {
            Donor::V069 { model, hair, .. } => {
                let (m, rec) = material_069(
                    &out,
                    view,
                    sources,
                    HEAD_MATERIAL,
                    &format!("characters/human/head/{model}"),
                    face.key,
                    &mut files,
                )?;
                materials_record.insert(face.key.to_owned(), rec);
                let hair = match hair {
                    Some(_) => {
                        let name = format!("{}_hair", face.key);
                        let (m, rec) = material_069(
                            &out,
                            view,
                            sources,
                            HAIR_MATERIAL,
                            &format!("characters/human/head/{model}_hair"),
                            &name,
                            &mut files,
                        )?;
                        materials_record.insert(name, rec);
                        Some(m)
                    }
                    None => None,
                };
                (m, hair)
            }
            Donor::Stk2 { scalp, .. } => {
                let maps = donor_maps(sources, face, &output.join("textures-work"), false)?;
                let (m, rec) = face_material(
                    &out,
                    view,
                    &skin,
                    HEAD_MATERIAL,
                    face.key,
                    &maps,
                    &mut files,
                )?;
                materials_record.insert(face.key.to_owned(), rec);
                let scalp = if scalp {
                    let (m, rec) = scalp_material(
                        &out,
                        view,
                        sources,
                        face,
                        &output.join("textures-work"),
                        &mut files,
                    )?;
                    materials_record.insert(format!("{}_scalp", face.key), rec);
                    Some(m)
                } else {
                    None
                };
                (m, scalp)
            }
            Donor::Native => bail!("the native head is not built"),
        };
        let mut entry = vec![];
        for state in State::ALL {
            let Some(drawn) = variants.get(&state) else {
                continue;
            };
            let model = format!("ported/{PORT}/{}{}", face.key, variant_name(state));
            let mut materials = vec![face_mat.clone()];
            if drawn.surfaces.len() > 1 {
                materials.push(
                    extra_mat
                        .clone()
                        .context("a second surface without its material")?,
                );
            }
            model_files(
                &out,
                view,
                &format!("models/{model}.skinned_model"),
                drawn,
                &materials,
                &mut files,
            )?;
            let surfaces: Vec<String> = drawn.surfaces.iter().map(|(n, _)| n.clone()).collect();
            let key = match state {
                State::Bare => "data",
                State::Helmet => "data_helmet",
                State::Mask => "data_mask",
            };
            entry.push((key, record(&model, &surfaces)));
            for ((surface, bytes), material) in drawn.surfaces.iter().zip(&materials) {
                let file = format!("{}{}-{surface}", face.key, variant_name(state));
                fs::write(render.join(&file), bytes)?;
                scene.push(json!({
                    "face": face.key,
                    "index": face.index,
                    "state": state,
                    "surface": file,
                    "diffuse": format!("overlay/textures/{}.dds", texture_of(&parse_config_file(&overlay, &format!("material_instances/{material}.material"))?, "texture_diffuse", material)?),
                }));
            }
        }
        entry.push(("icon", icon(face.index)));
        heads.push(named(entry));
    }
    ensure!(heads.len() == FACES.len(), "a face is missing");
    let config = bodyparts(view, heads)?;
    out.config(BODYPARTS, &config, &mut files)?;
    // The helmets and masks made for a face: each one's models whole, their
    // surfaces carried, and the item's `data_head_<face>` naming them.
    let mut item_configs: BTreeMap<String, BinaryConfig> = BTreeMap::new();
    for item in &fitted.items {
        let mut models: BTreeMap<String, String> = BTreeMap::new();
        for (model, _) in &item.parts {
            models
                .entry(model.clone())
                .or_insert_with(|| format!("ported/{PORT}/items/{}/{model}", item.face.key));
        }
        for (model, new) in &models {
            let from = format!("models/{model}.skinned_model/");
            let paths: Vec<String> = view.paths_with_prefix(&from).map(str::to_owned).collect();
            for path in paths {
                let rest = &path[from.len()..];
                let carried = rest
                    .strip_prefix("render/")
                    .and_then(|r| r.strip_suffix("/converted_model"))
                    .and_then(|surface| item.surfaces.get(&(model.clone(), surface.to_owned())));
                let bytes = match carried {
                    Some(b) => b.clone(),
                    None => view.read(&path)?,
                };
                out.raw(
                    &format!("models/{new}.skinned_model/{rest}"),
                    &bytes,
                    &mut files,
                )?;
            }
        }
        if !item_configs.contains_key(&item.config) {
            item_configs.insert(
                item.config.clone(),
                parse_config(view, &item.config, "item")?,
            );
        }
        let config = item_configs.get_mut(&item.config).unwrap();
        let mut entry = config
            .root
            .named(&item.key)
            .cloned()
            .with_context(|| format!("{}: no {}", item.config, item.key))?;
        let rename = |v: &mut V| -> Result<()> {
            let new = match v.named("base_model") {
                Some(V::String(m)) => models.get(m).cloned(),
                _ => None,
            };
            if let Some(new) = new {
                v.set_path("base_model", s(new))?;
            }
            Ok(())
        };
        rename(&mut entry)?;
        if let Some(V::Indexed(extra)) = entry.named_mut("additional_models") {
            for e in extra {
                rename(e)?;
            }
        }
        let V::Named(root) = &mut config.root else {
            bail!("{}: not a table", item.config);
        };
        root.push(NamedValue {
            name: format!("{}_head_{}", item.key, item.face.index),
            value: entry,
        });
        for (k, (model, surface)) in item.parts.iter().enumerate() {
            let file = format!(
                "{}-{}-{k}-{surface}",
                item.face.key,
                item.label.replace(['/', ' ', '(', ')'], "_")
            );
            fs::write(
                render.join(&file),
                &item.surfaces[&(model.clone(), surface.clone())],
            )?;
            item_scene.push(
                json!({"face": item.face.key, "item": item.label, "part": k, "surface": file}),
            );
        }
    }
    for (path, config) in &item_configs {
        out.config(path, config, &mut files)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": "faces-026",
            "donor_build": "0.69d0 + STK2",
            "target_build": "0.26g",
            "armor_dict_ids": [],
            "files": files,
        }))?,
    )?;
    fs::write(
        output.join("faces-materials.json"),
        serde_json::to_vec_pretty(&materials_record)?,
    )?;
    write_render_scene(view, fitted, &render, scene, item_scene)?;
    crate::model_validation::validate_overlay_on(view, &overlay)?;
    println!(
        "Built {} faces: {} resources",
        fitted.faces.len(),
        files.len()
    );
    Ok(())
}

fn parse_config_file(overlay: &Path, path: &str) -> Result<BinaryConfig> {
    BinaryConfig::parse(&fs::read(overlay.join(path))?).with_context(|| path.to_owned())
}

/// What the pictures need from the game: every cover's surfaces (whole,
/// bind pose) with its colour map, and the native head's.
fn write_render_scene(
    view: &mut ResourceDb,
    fitted: &Fitted,
    render: &Path,
    faces: Vec<serde_json::Value>,
    for_faces: Vec<serde_json::Value>,
) -> Result<()> {
    let items = fitted.record["covers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut covers = vec![];
    let dir = render.join("items");
    fs::create_dir_all(&dir)?;
    let dictionary = parse_config(view, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(rows)) = dictionary.root.named("items_dict") else {
        bail!("no items_dict");
    };
    let mut configs = BTreeMap::new();
    for row in rows {
        if let Some(V::String(cfg)) = row.named("cfg_name") {
            configs.insert(
                cfg.trim_start_matches("gameplay/items/armour/")
                    .trim_end_matches(".options")
                    .to_owned(),
                cfg.clone(),
            );
        }
    }
    for item in items {
        let label = item["item"].as_str().context("cover label")?.to_owned();
        let (name, key) = match label.split_once(" (") {
            Some((n, k)) => (n.to_owned(), k.trim_end_matches(')').to_owned()),
            None => (label.clone(), "data".to_owned()),
        };
        let cfg = configs
            .get(&name)
            .with_context(|| format!("no config for {name}"))?;
        let config = parse_config(view, cfg, "item")?;
        let part = config
            .root
            .named(&key)
            .with_context(|| format!("{cfg}: no {key}"))?;
        let mut parts = vec![part];
        if let Some(V::Indexed(extra)) = part.named("additional_models") {
            parts.extend(extra);
        }
        let mut surfaces = vec![];
        for p in parts {
            let (Some(V::String(model)), Some(V::String(surface))) =
                (p.named("base_model"), p.named("part_name"))
            else {
                continue;
            };
            let root = format!("models/{model}.skinned_model");
            let bytes = view.read(&format!("{root}/render/{surface}/converted_model"))?;
            let file = format!("{}-{surface}", label.replace(['/', ' ', '(', ')'], "_"));
            fs::write(dir.join(&file), &bytes)?;
            let settings = parse_config(view, &format!("{root}/settings"), "settings")?;
            let diffuse = settings
                .root
                .named("material_settings")
                .and_then(|m| m.named(surface))
                .and_then(|m| m.named("material_name"))
                .and_then(|m| match m {
                    V::String(m) => Some(m.clone()),
                    _ => None,
                })
                .and_then(|m| {
                    let c = parse_config(
                        view,
                        &format!("material_instances/{m}.material"),
                        "material",
                    )
                    .ok()?;
                    texture_of(&c, "texture_diffuse", &m).ok()
                });
            let tex = match diffuse {
                Some(t) if view.contains(&format!("textures/{t}.dds")) => {
                    let name = format!("{}.dds", t.replace('/', "_"));
                    if !dir.join(&name).exists() {
                        fs::write(dir.join(&name), view.read(&format!("textures/{t}.dds"))?)?;
                    }
                    Some(format!("items/{name}"))
                }
                _ => None,
            };
            surfaces.push(json!({"surface": format!("items/{file}"), "diffuse": tex}));
        }
        covers.push(json!({"item": label, "slot": item["slot"], "surfaces": surfaces}));
    }
    // The native head with its own map.
    let native_tex = texture_of(
        &parse_config(view, HEAD_MATERIAL, "head material")?,
        "texture_diffuse",
        HEAD_MATERIAL,
    )?;
    fs::write(render.join("native-head"), view.read(HEAD_MODEL)?)?;
    fs::write(
        render.join("native-head.dds"),
        view.read(&format!("textures/{native_tex}.dds"))?,
    )?;
    fs::write(
        render.join("scene.json"),
        serde_json::to_vec_pretty(&json!({
            "native": {"surface": "native-head", "diffuse": "native-head.dds"},
            "faces": faces,
            "covers": covers,
            "for_faces": for_faces,
        }))?,
    )?;
    Ok(())
}
