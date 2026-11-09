// SPDX-License-Identifier: GPL-3.0-or-later
//! Player faces for v0.26g: the eight faces of 0.69d0 and two bare faces of
//! the STK2 characters export, each with the variants it is drawn in under
//! a helmet and under a mask, fitted under every live item that leaves the
//! head (`face_fit`).
//!
//! What the game reads is `gameplay/bodyparts/default`'s `body_parts.heads`,
//! in 0.31a2's form: `heads[i].data` (the bare face), `data_helmet` and
//! `data_mask`, each `{base_model, part_name, additional_models}` (the hair
//! an additional model of the bare face); a missing variant is the bare
//! face. `heads[0]` is the native head (`body_parts.head`, which stays as
//! it is). Each head also names its picker icon, `heads[i].icon` = `[row,
//! col]` of the lobby movie's equipment atlas.
//!
//! - **0.69d0.** `models/character/human/head/head_*`: the surfaces bind
//!   the native rig in its bind pose (`face_items_069d::same_rig_for`) and
//!   convert in place ([`crate::v069d_skinned`]); `head_05` and `head_07`
//!   have hair (`*_hair_vc`), which only the bare face draws.
//! - **STK2.** `bandit_01_head_03` and `neutral_01_head` (the Novice's face
//!   under his knit cap): the export's models on the native rig turned half
//!   a turn about Y (`duty_026::rotate_mesh`), then moved onto the native
//!   head ([`align_onto`]: the skull above the Head bone, point to surface);
//!   a neck shorter than the native one is stretched down to its bottom.
//! - **Necks.** Every face's neck ends 3 mm above the native neck's slanted
//!   open edge ([`NeckEdge`]) and takes the native neck's skin below the
//!   jaw ([`native_neck_skin`]), so it meets the collars as the native neck
//!   does.
//!   Their helmet and mask variants are made here as 0.69d's were: the
//!   bare face pulled in under every helmet, and under every mask.
//! - **The fit.** Every face's bare face against every torso and back piece,
//!   its helmet variant under every helmet, its mask variant under every
//!   mask and every helmet worn over one, in the bind pose and the sampled
//!   poses of `bandit_026::POSE_CLIPS` ([`face_fit::sweep`]); where it shows
//!   through, the variant is pulled in ([`face_fit::pull_in`]). The build
//!   stops if any face still shows through anything.
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
pub const FACES: [Face; 11] = [
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
        key: "stk2_bandit_01_head_03",
        donor: Donor::Stk2 {
            model: "bandit/bandit_01/bandit_01_head_03",
            surface: "bandit_01_head_s",
            texture: "bandit/bandit_01/bandit_01_head",
        },
    },
    Face {
        index: 10,
        key: "stk2_neutral_01_head",
        donor: Donor::Stk2 {
            model: "neutral/neutral_01/neutral_01_head",
            surface: "neutral_01_head_s",
            texture: "neutral/neutral_01_novice/neutral_01_novice_head",
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
                let mut mesh = crate::v069d_skinned::to_026_four_bone(&parsed)?;
                let cut = cut_neck(&mut mesh, &edge)?;
                let skinned = native_neck_skin(&mut mesh, native_head)?;
                println!(
                    "{label}: neck cut {:.1} mm up, at the native neck's bottom; {skinned} neck vertices on the native neck's skin",
                    cut * 1000.0
                );
                Ok(mesh)
            };
            let mut bare = vec![(surface.to_owned(), read(model, surface)?)];
            if let Some(h) = hair {
                bare.push((h.to_owned(), read(model, h)?));
            }
            out.insert(
                State::Bare,
                Drawn::new(format!("{} bare", face.key), bare),
            );
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
        Donor::Stk2 { model, surface, .. } => {
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
            out.insert(
                State::Bare,
                Drawn::new(format!("{} bare", face.key), vec![(surface.to_owned(), mesh)]),
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
/// edge ([`NeckEdge`]): 0.69d's necks hang 4 to 5 cm lower, inside every
/// collar, and there they follow the head's turn through the torso.
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

/// Every face's drawn surfaces per state after the fit, and the fit's
/// record.
pub struct Fitted {
    pub faces: Vec<(Face, BTreeMap<State, Drawn>)>,
    pub record: serde_json::Value,
}

#[derive(Serialize)]
struct StateRecord {
    face: String,
    state: State,
    model: String,
    pulled_vertices: usize,
    pulled_mm: f64,
    worst: Vec<Measure>,
    worn_sets: usize,
    through: usize,
}

/// The surfaces a face draws in `state`: its variant, or its bare face.
fn effective(variants: &BTreeMap<State, Drawn>, state: State) -> &Drawn {
    variants.get(&state).unwrap_or(&variants[&State::Bare])
}

/// The bare face without its hair: where a helmet or mask variant starts
/// from when the donor has none.
fn hairless(bare: &Drawn, state: State, key: &str) -> Drawn {
    bare.first(1, format!("{key} {state:?}").to_lowercase())
}

/// Loads, measures and fits every face.
pub fn fit_all(
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    faces: &[Face],
    measure_only: bool,
) -> Result<Fitted> {
    let bind_pose = view.read(HEAD_BIND_POSE)?;
    let native_head = view.read(HEAD_MODEL)?;
    let rig = Rig::new(&bind_pose, &native_head)?;
    let wardrobe = Wardrobe::read(view, &rig)?;
    println!(
        "Fitting under {} helmets, {} masks, {} torso and back models ({} worn sets; {} items draw no head: {})",
        wardrobe.count(|s| matches!(s, face_fit::Slot::Helmet { .. })),
        wardrobe.count(|s| matches!(s, face_fit::Slot::Mask { .. })),
        wardrobe.count(|s| *s == face_fit::Slot::Body),
        wardrobe.worn.values().map(Vec::len).sum::<usize>(),
        wardrobe.headless.len(),
        wardrobe.headless.join(", ")
    );
    let poses = SampledPoses::read(view)?;
    let native = Posable::parse(&rig, "native head", &native_head, &bind_pose)?;
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

    // FACES_SKIP_FIT: the donors' variants as they are, unmeasured, to look
    // into the bundle and its pictures while working on them (never a pack).
    if std::env::var_os("FACES_SKIP_FIT").is_some() {
        for (face, variants) in &mut built {
            for state in [State::Helmet, State::Mask] {
                if !variants.contains_key(&state) && variants[&State::Bare].surfaces.len() > 1 {
                    let h = hairless(&variants[&State::Bare], state, face.key);
                    variants.insert(state, h);
                }
            }
        }
        let covers: Vec<_> = wardrobe
            .covers
            .iter()
            .map(|c| json!({"item": c.label, "slot": c.slot}))
            .collect();
        return Ok(Fitted {
            faces: built,
            record: json!({"unfitted": true, "covers": covers}),
        });
    }
    // What the donors' own models do, before any fix.
    let before = {
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
        let mut out = vec![];
        for (job, sweep) in jobs.iter().zip(&sweeps) {
            let through: Vec<&Measure> = sweep.measures.iter().filter(|m| m.margin < 0.0).collect();
            let worst = sweep
                .measures
                .iter()
                .min_by(|a, b| a.margin.total_cmp(&b.margin));
            println!(
                "  donor {} as {:?}: through {} of {} worn sets{}",
                job.drawn.label,
                job.state,
                through.len(),
                sweep.measures.len(),
                worst.map_or(String::new(), |w| format!(
                    "; worst {:.1} mm under {} at {:?} ({})",
                    w.margin * 1000.0,
                    w.worn,
                    w.at.map(|v| (v * 1000.0).round() / 1000.0),
                    w.pose
                ))
            );
            out.push(json!({
                "model": job.drawn.label,
                "state": job.state,
                "worn_sets_through": through.len(),
                "worst_mm": worst.map(|w| w.margin * 1000.0),
                "worst_under": worst.map(|w| w.worn.clone()),
            }));
        }
        out
    };
    if measure_only {
        return Ok(Fitted {
            faces: built,
            record: json!({ "donors": before }),
        });
    }

    // The bare faces under the torsos and backs first: every variant made
    // from a bare face starts from the fitted one.
    let mut pulls: BTreeMap<(usize, State), face_fit::Pull> = BTreeMap::new();
    let mut jobs: Vec<(Drawn, State)> = built
        .iter()
        .map(|(_, v)| (v[&State::Bare].clone(), State::Bare))
        .collect();
    println!("Fitting the bare faces under the torsos and backs");
    let (_, p) = face_fit::pull_in(&rig, &poses, &native, &wardrobe, &mut jobs)?;
    for (((face, variants), (drawn, _)), pull) in built.iter_mut().zip(jobs).zip(p) {
        variants.insert(State::Bare, drawn);
        pulls.insert((face.index, State::Bare), pull);
    }
    let mut jobs: Vec<(Drawn, State)> = vec![];
    let mut owners = vec![];
    for (face, variants) in &built {
        for state in [State::Helmet, State::Mask] {
            let drawn = match variants.get(&state) {
                Some(d) => d.clone(),
                None => hairless(&variants[&State::Bare], state, face.key),
            };
            jobs.push((drawn, state));
            owners.push(face.index);
        }
    }
    println!("Fitting the helmet and mask variants");
    let (_, p) = face_fit::pull_in(&rig, &poses, &native, &wardrobe, &mut jobs)?;
    for (((drawn, state), owner), pull) in jobs.into_iter().zip(owners).zip(p) {
        let (_, variants) = built.iter_mut().find(|(f, _)| f.index == owner).unwrap();
        let had_hair = variants[&State::Bare].surfaces.len() > 1;
        // A variant is kept where the donor has one, where the fit changed
        // the bare face, or where the bare face has hair to leave out.
        if variants.contains_key(&state) || pull.moved > 0 || had_hair {
            variants.insert(state, drawn);
        }
        pulls.insert((owner, state), pull);
    }

    // The gate: every face in every state, every pose.
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
    let mut failures = vec![];
    for (k, (job, sweep)) in jobs.iter().zip(&sweeps).enumerate() {
        let (face, variants) = &built[k / 3];
        let mut worst: Vec<Measure> = sweep.measures.clone();
        worst.sort_by(|a, b| a.margin.total_cmp(&b.margin));
        let through = worst.iter().filter(|m| m.margin < 0.0).count();
        if through > 0 {
            failures.push(format!(
                "{} {:?}: through {through} worn sets, worst {:.2} mm under {}",
                face.key,
                job.state,
                worst[0].margin * 1000.0,
                worst[0].worn
            ));
        }
        let pull = pulls
            .get(&(face.index, job.state))
            .cloned()
            .unwrap_or_default();
        let model = if variants.contains_key(&job.state) {
            format!("{}{}", face.key, variant_name(job.state))
        } else {
            face.key.to_owned()
        };
        println!(
            "  {} {:?} ({model}): least margin {:.2} mm under {} ({}), least gap {:.2} mm; pulled {} vertices up to {:.1} mm",
            face.key,
            job.state,
            worst[0].margin * 1000.0,
            worst[0].worn,
            worst[0].pose,
            worst.iter().map(|m| m.gap).fold(f64::INFINITY, f64::min) * 1000.0,
            pull.moved,
            pull.farthest * 1000.0
        );
        states.push(StateRecord {
            face: face.key.to_owned(),
            state: job.state,
            model,
            pulled_vertices: pull.moved,
            pulled_mm: pull.farthest * 1000.0,
            worn_sets: sweep.measures.len(),
            through,
            worst: worst.into_iter().take(5).collect(),
        });
    }
    ensure!(
        failures.is_empty(),
        "faces show through the game's items:\n{}",
        failures.join("\n")
    );
    let record = json!({
        "clearance_bind_mm": face_fit::BIND_CLEARANCE * 1000.0,
        "clearance_posed_mm": face_fit::POSED_CLEARANCE * 1000.0,
        "native_tolerance_mm": face_fit::NATIVE_TOLERANCE * 1000.0,
        "poses": 1 + poses.clips.iter().map(|(_, c)| c.len()).sum::<usize>(),
        "clips": poses.clips.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "covers": wardrobe.covers.iter().map(|c| json!({"item": c.label, "slot": c.slot})).collect::<Vec<_>>(),
        "headless": wardrobe.headless,
        "worn_sets": wardrobe.worn.iter().map(|(s, w)| (format!("{s:?}").to_lowercase(), w.len())).collect::<BTreeMap<_, _>>(),
        "donors": before,
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
    let fitted = fit_all(&mut view, sources, &FACES, measure_only)?;
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
}

impl Role {
    fn key(self) -> &'static str {
        match self {
            Self::Diffuse => "texture_diffuse",
            Self::Normal => "texture_normal",
            Self::Gloss => "texture_roughness",
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Diffuse => "",
            Self::Normal => "_nmap",
            Self::Gloss => "_smap",
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
        Donor::Stk2 { texture, .. } => {
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
                let source = sources
                    .stk2_textures
                    .join(STK2_TEXTURES)
                    .join(format!("{texture}{}.tga", role.suffix()));
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
        let maps = donor_maps(sources, face, &output.join("textures-work"), false)?;
        let (face_mat, rec) = face_material(
            &out,
            view,
            &skin,
            HEAD_MATERIAL,
            face.key,
            &maps,
            &mut files,
        )?;
        materials_record.insert(face.key.to_owned(), rec);
        let hair = variants[&State::Bare].surfaces.len() > 1;
        let hair_mat = if hair {
            let maps = donor_maps(sources, face, &output.join("textures-work"), true)?;
            let (m, rec) = face_material(
                &out,
                view,
                &skin,
                HAIR_MATERIAL,
                &format!("{}_hair", face.key),
                &maps,
                &mut files,
            )?;
            materials_record.insert(format!("{}_hair", face.key), rec);
            Some(m)
        } else {
            None
        };
        let mut entry = vec![];
        for state in State::ALL {
            let Some(drawn) = variants.get(&state) else {
                continue;
            };
            let model = format!("ported/{PORT}/{}{}", face.key, variant_name(state));
            let mut materials = vec![face_mat.clone()];
            if drawn.surfaces.len() > 1 {
                materials.push(hair_mat.clone().context("hair without its material")?);
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
