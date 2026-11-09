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

/// A face as its donor gives it: the drawn surfaces of each state it has
/// a model for (always the bare one).
fn donor_faces(
    sources: &Sources<'_>,
    bind_pose: &[u8],
    native_head: &[u8],
    above: f64,
    face: &Face,
) -> Result<BTreeMap<State, Drawn>> {
    let mut out = BTreeMap::new();
    let native = parse_mesh(native_head, 40, "native head")?;
    let edge = NeckEdge::new(&native)?;
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
            let mut bare = vec![(surface.to_owned(), read(model, surface)?)];
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
                            vec![(surface.to_owned(), read(m, surface)?)],
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
            let (t, rms) = align_onto(&mesh, &native.positions, &native.triangles, above)?;
            translate(&mut mesh, t)?;
            let stretched = stretch_neck(&mut mesh, &edge)?;
            cut_neck(&mut mesh, &edge)?;
            native_neck_skin(&mut mesh, native_head)?;
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
                let (bare, grafted) = graft_scalp(&mesh, native_head, bind_pose)?;
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
    Ok(out)
}

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

/// Stretches a neck that ends above the native one down to the native
/// neck's bottom: heights below the native chin's (`NECK_TOP`) are mapped
/// linearly from the face's bottom to the native bottom. Returns how far
/// the bottom came down (m).
fn stretch_neck(mesh: &mut [u8], edge: &NeckEdge) -> Result<f64> {
    /// Above this height nothing moves (m).
    const NECK_TOP: f64 = 1.56;
    let n = read_u32(mesh, 8)?;
    let y = |m: &[u8], i: usize| {
        f64::from(f32::from_le_bytes(
            m[12 + i * 48 + 4..12 + i * 48 + 8].try_into().unwrap(),
        ))
    };
    let bottom = (0..n).map(|i| y(mesh, i)).fold(f64::INFINITY, f64::min);
    let target = edge.bottom();
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

/// Below this height (m) a face's neck takes the native neck's skin, fully
/// below [`NECK_SKIN_FULL`]: the native head's neck follows the chest as
/// the collars made over it do, and a donor's neck that kept its own
/// skin swung out through them when the head turned.
const NECK_SKIN_TOP: f64 = 1.56;
const NECK_SKIN_FULL: f64 = 1.52;

/// Gives the face's neck the native neck's skin ([`NECK_SKIN_TOP`]).
/// Returns how many vertices changed.
fn native_neck_skin(mesh: &mut [u8], native_head: &[u8]) -> Result<usize> {
    let mut native = mesh.to_vec();
    crate::mask_fit::follow_head_weights(&mut native, native_head)?;
    let own = crate::mask_fit::skins(mesh, 48)?;
    let theirs = crate::mask_fit::skins(&native, 48)?;
    let face = parse_mesh(mesh, 48, "face")?;
    let mut changed = 0;
    for (i, p) in face.positions.iter().enumerate() {
        if p[1] >= NECK_SKIN_TOP {
            continue;
        }
        let w = ((NECK_SKIN_TOP - p[1]) / (NECK_SKIN_TOP - NECK_SKIN_FULL)).clamp(0.0, 1.0);
        let mut mixed: BTreeMap<u8, f64> = BTreeMap::new();
        for &(b, x) in &own[i] {
            *mixed.entry(b).or_default() += x * (1.0 - w);
        }
        for &(b, x) in &theirs[i] {
            *mixed.entry(b).or_default() += x * w;
        }
        let skin: Vec<(u8, f64)> = mixed.into_iter().filter(|(_, x)| *x > 1e-6).collect();
        crate::mask_fit::put_skin(&mut mesh[12 + i * 48..12 + (i + 1) * 48], &skin);
        changed += 1;
    }
    Ok(changed)
}

/// The native neck's open edge: its height (m) round the neck, by the
/// angle round the neck's axis. The native neck ends aslant, at 1.480 m
/// under the chin and 1.524 m at the nape; a face's neck ends 3 mm above it
/// all round ([`cut_neck`]), so every ray through the face's last ring
/// still meets the native neck the rule compares it with.
pub(crate) struct NeckEdge {
    /// (angle in radians, height), by angle.
    points: Vec<(f64, f64)>,
}

impl NeckEdge {
    /// The axis the angle is taken round (x, z), behind the throat.
    const AXIS: [f64; 2] = [0.0, -0.04];
    const ABOVE: f64 = 0.003;

    fn new(native: &crate::mask_fit::Mesh) -> Result<Self> {
        let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
        // Each welded edge: how many triangles use it, and its ends.
        type Edge = (usize, [f64; 3], [f64; 3]);
        let mut edges: BTreeMap<([i64; 3], [i64; 3]), Edge> = BTreeMap::new();
        for t in &native.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (pa, pb) = (native.positions[a], native.positions[b]);
                let k = if key(pa) < key(pb) {
                    (key(pa), key(pb))
                } else {
                    (key(pb), key(pa))
                };
                edges.entry(k).or_insert((0, pa, pb)).0 += 1;
            }
        }
        let mut points: Vec<(f64, f64)> = edges
            .values()
            .filter(|(n, _, _)| *n == 1)
            .flat_map(|(_, a, b)| [*a, *b])
            .filter(|p| p[1] < 1.56)
            .map(|p| (Self::angle(p), p[1]))
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        ensure!(points.len() >= 8, "the native neck has no open edge");
        Ok(Self { points })
    }

    fn angle(p: [f64; 3]) -> f64 {
        (p[0] - Self::AXIS[0]).atan2(p[2] - Self::AXIS[1])
    }

    /// Where a face's neck ends below `p`.
    fn cut_at(&self, p: [f64; 3]) -> f64 {
        let a = Self::angle(p);
        let n = self.points.len();
        let tau = std::f64::consts::TAU;
        let after = self.points.iter().position(|q| q.0 >= a).unwrap_or(n);
        let (lo, hi) = (self.points[(after + n - 1) % n], self.points[after % n]);
        let span = (hi.0 - lo.0).rem_euclid(tau).max(1e-9);
        let t = ((a - lo.0).rem_euclid(tau) / span).clamp(0.0, 1.0);
        lo.1 + t * (hi.1 - lo.1) + Self::ABOVE
    }

    /// The lowest the face's neck ends (m).
    fn bottom(&self) -> f64 {
        self.points
            .iter()
            .map(|p| p.1)
            .fold(f64::INFINITY, f64::min)
            + Self::ABOVE
    }
}

/// Cuts a neck that reaches below the native one along the native neck's
/// edge ([`NeckEdge`]): a STK2 neck stretched down to it ends on it.
/// Triangles wholly below go; the corners of those across the cut come up
/// onto it. Returns the most cut off (m).
fn cut_neck(mesh: &mut Vec<u8>, edge: &NeckEdge) -> Result<f64> {
    let face = parse_mesh(mesh, 48, "face")?;
    let below = |p: &[f64; 3]| p[1] < edge.cut_at(*p);
    let cut = face
        .positions
        .iter()
        .filter(|p| below(p))
        .map(|p| edge.cut_at(*p) - p[1])
        .fold(0.0, f64::max);
    if cut == 0.0 {
        return Ok(0.0);
    }
    *mesh = crate::outfit_026::keep_triangles(mesh, |c| c.iter().any(|(p, _)| !below(p)))?;
    let n = read_u32(mesh, 8)?;
    for i in 0..n {
        let at = 12 + i * 48;
        let p = [0, 1, 2].map(|k| {
            f64::from(f32::from_le_bytes(
                mesh[at + 4 * k..at + 4 * k + 4].try_into().unwrap(),
            ))
        });
        if below(&p) {
            mesh[at + 4..at + 8].copy_from_slice(&(edge.cut_at(p) as f32).to_le_bytes());
        }
    }
    Ok(cut)
}

/// The grafted scalp's surface (an additional model of the bare face).
const SCALP: &str = "scalp_s";
/// How far below a face's open top the scalp reaches, tucked under its
/// edge (m), over the forehead and at the nape: at the nape the band under
/// the cap is painted in its shadow (over the forehead it is lifted off,
/// [`lift_cap_shadow`]), and over the forehead the native brows begin
/// just below it.
const SCALP_OVERLAP_FRONT: f64 = 0.003;
const SCALP_OVERLAP_BACK: f64 = 0.016;

/// [`SCALP_OVERLAP_FRONT`] blended round the head to [`SCALP_OVERLAP_BACK`].
fn scalp_overlap(azimuth: f64) -> f64 {
    SCALP_OVERLAP_FRONT + (SCALP_OVERLAP_BACK - SCALP_OVERLAP_FRONT) * (1.0 - azimuth.cos()) / 2.0
}
/// The least the scalp stays outside the face's skin where both are (m).
const SCALP_GAP: f64 = 0.002;

/// A face's own skin top and a native-head scalp over it: the face without
/// the mesh at its top (the Novice's knit cap, modelled on his head), and
/// the native head cut along the face's open top (less
/// [`scalp_overlap`]), each scalp vertex lifted [`SCALP_GAP`] outside the
/// face's skin where the skin is outside it. The scalp keeps the native
/// head's skin, basis and UVs: it draws with the native head's material,
/// and fits every item as the native head does.
fn graft_scalp(face: &[u8], native_head: &[u8], bind_pose: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
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
    // The open top is the highest open edge round the head, a point in each
    // 5 degrees: the eyes' openings are open edges too, lower down.
    const RIM_BINS: usize = 72;
    let tau = std::f64::consts::TAU;
    let mut top: Vec<Option<(f64, f64)>> = vec![None; RIM_BINS];
    for p in uses
        .values()
        .filter(|(n, _, _)| *n == 1)
        .flat_map(|(_, a, b)| [*a, *b])
        .filter(|p| p[1] > 1.6)
    {
        let a = azimuth(p);
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
        .map(|&p| p[1] - (rim_at(azimuth(p)) - scalp_overlap(azimuth(p))))
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

    // Lifted outside the face's skin, along rays from the skull's middle.
    let origins = crate::armor::bind_pose_origins(bind_pose, "native head bind pose")?;
    let eye = |n: &str| {
        origins
            .iter()
            .find(|(name, _)| name == n)
            .map(|(_, p)| f64::from(p[1]))
    };
    let eyes =
        (eye("LeftEye").context("no LeftEye")? + eye("RightEye").context("no RightEye")?) / 2.0;
    let centre = [0.0, eyes, 0.0];
    let skin_tris: Vec<[[f64; 3]; 3]> = kept
        .triangles
        .iter()
        .map(|t| t.map(|v| kept.positions[v]))
        .collect();
    let mut lifted = 0;
    for v in &mut vertices {
        let p = [0, 1, 2]
            .map(|k| f64::from(f32::from_le_bytes(v[4 * k..4 * k + 4].try_into().unwrap())));
        let d = sub(p, centre);
        let r = dot(d, d).sqrt();
        let dir = d.map(|x| x / r);
        let Some(skin_r) = skin_tris
            .iter()
            .filter_map(|t| ray_hit(centre, dir, t))
            .reduce(f64::max)
        else {
            continue;
        };
        if skin_r + SCALP_GAP > r {
            for k in 0..3 {
                let x = (centre[k] + dir[k] * (skin_r + SCALP_GAP)) as f32;
                v[4 * k..4 * k + 4].copy_from_slice(&x.to_le_bytes());
            }
            lifted += 1;
        }
    }
    println!(
        "  scalp: cap of {} vertices off; {} native triangles over the face's top ({:.3} to {:.3} m), {lifted} vertices lifted over its skin",
        capped.len(),
        triangles.len(),
        rim.iter().map(|r| r.1).fold(f64::INFINITY, f64::min),
        rim.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max)
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

/// Where a ray from `o` along unit `d` meets triangle `t`, if ahead.
fn ray_hit(o: [f64; 3], d: [f64; 3], t: &[[f64; 3]; 3]) -> Option<f64> {
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = sub(o, t[0]);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    let along = dot(e2, q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && along > 0.0).then_some(along)
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

/// Every face's drawn surfaces per state, and the fit report.
pub struct Fitted {
    pub faces: Vec<(Face, BTreeMap<State, Drawn>)>,
    pub record: serde_json::Value,
}

#[derive(Serialize)]
struct StateRecord {
    face: String,
    state: State,
    model: String,
    worst: Vec<Measure>,
    worn_sets: usize,
    through: usize,
}

/// The surfaces a face draws in `state`: its variant, or its bare face.
fn effective(variants: &BTreeMap<State, Drawn>, state: State) -> &Drawn {
    variants.get(&state).unwrap_or(&variants[&State::Bare])
}

/// Loads every face as its donor has it and measures it under every live
/// item that draws the head ([`face_fit::sweep`]): a report, which never
/// changes a face.
pub fn fit_all(view: &mut ResourceDb, sources: &Sources<'_>, faces: &[Face]) -> Result<Fitted> {
    let bind_pose = view.read(HEAD_BIND_POSE)?;
    let native_head = view.read(HEAD_MODEL)?;
    let rig = Rig::new(&bind_pose, &native_head)?;
    let mut built: Vec<(Face, BTreeMap<State, Drawn>)> = vec![];
    for face in faces {
        if matches!(face.donor, Donor::Native) {
            continue;
        }
        built.push((
            *face,
            donor_faces(sources, &bind_pose, &native_head, rig.head_y(), face)?,
        ));
    }
    // FACES_NO_REPORT: the faces without the fit report, while looking at
    // their pictures.
    if std::env::var_os("FACES_NO_REPORT").is_some() {
        return Ok(Fitted {
            faces: built,
            record: json!({ "report": "skipped" }),
        });
    }
    let wardrobe = Wardrobe::read(view, &rig)?;
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
    let jobs: Vec<Job<'_>> = built
        .iter()
        .flat_map(|(_, v)| {
            State::ALL.map(|state| Job {
                drawn: effective(v, state),
                state,
            })
        })
        .collect();
    let sweeps = face_fit::sweep(&rig, &poses, &native, &wardrobe, &jobs)?;
    let mut states = vec![];
    for (k, (job, sweep)) in jobs.iter().zip(&sweeps).enumerate() {
        let (face, variants) = &built[k / 3];
        let mut worst: Vec<Measure> = sweep.measures.clone();
        worst.sort_by(|a, b| a.margin.total_cmp(&b.margin));
        let through = worst.iter().filter(|m| m.margin < 0.0).count();
        let model = if variants.contains_key(&job.state) {
            format!("{}{}", face.key, variant_name(job.state))
        } else {
            face.key.to_owned()
        };
        if let Some(w) = worst.first() {
            println!(
                "  {} {:?} ({model}): through {through} of {} worn sets; worst {:.1} mm under {} ({})",
                face.key,
                job.state,
                worst.len(),
                w.margin * 1000.0,
                w.worn,
                w.pose
            );
        }
        states.push(StateRecord {
            face: face.key.to_owned(),
            state: job.state,
            model,
            worn_sets: sweep.measures.len(),
            through,
            worst: worst.into_iter().take(5).collect(),
        });
    }
    let record = json!({
        "report_only": true,
        "clearance_bind_mm": face_fit::BIND_CLEARANCE * 1000.0,
        "clearance_posed_mm": face_fit::POSED_CLEARANCE * 1000.0,
        "native_tolerance_mm": face_fit::NATIVE_TOLERANCE * 1000.0,
        "poses": 1 + poses.clips.iter().map(|(_, c)| c.len()).sum::<usize>(),
        "clips": poses.clips.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "covers": wardrobe.covers.iter().map(|c| json!({"item": c.label, "slot": c.slot})).collect::<Vec<_>>(),
        "headless": wardrobe.headless,
        "worn_sets": wardrobe.worn.iter().map(|(s, w)| (format!("{s:?}").to_lowercase(), w.len())).collect::<BTreeMap<_, _>>(),
        "states": states,
    });
    Ok(Fitted {
        faces: built,
        record,
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
) -> Result<()> {
    let mut view = crate::novice_026::game_view(db, patches)?;
    if measure_only {
        fs::create_dir_all(output)?;
    } else {
        crate::weapon::prepare_empty_output(output)?;
    }
    let fitted = fit_all(&mut view, sources, &FACES)?;
    fs::write(
        output.join("faces-fit.json"),
        serde_json::to_vec_pretty(&fitted.record)?,
    )?;
    if measure_only {
        return Ok(());
    }
    write_bundle(&mut view, sources, &fitted, output)
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
    write_render_scene(view, fitted, &render, scene)?;
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
        }))?,
    )?;
    Ok(())
}
