// SPDX-License-Identifier: GPL-3.0-or-later
//! Player faces for v0.26g: the eight faces of 0.69d0 as 0.69d0 draws them,
//! and the STK2 Novice's face with a scalp grafted where his cap was.
//!
//! What the game reads is `gameplay/bodyparts/default`'s `body_parts.heads`,
//! in 0.31a2's form: `heads[i].data` (the bare face), `data_helmet` and
//! `data_mask`, each `{base_model, part_name, additional_models}` (the hair
//! or the scalp an additional model of the bare face); a missing variant is
//! the bare face. `heads[0]` is the native head (`body_parts.head`, which
//! stays as it is). The lobby's faces' window shows its own pictures of the
//! faces (skhid's movie), so a head names no icon.
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
//!   a 0.69d0 face's short scalp joined on at his hairline ([`graft_scalp_069`]).
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
        /// Its cap (the mesh at its top) taken off and the short
        /// scalp of a 0.69d0 face joined on ([`graft_scalp_069`]).
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
type Donated = (
    BTreeMap<State, Drawn>,
    Vec<(String, NeckJoin)>,
    Option<ScalpBake>,
);

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
    let mut scalp_bake = None;
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
            let read = |model: &str, surface: &str| read_069(sources, bind_pose, model, surface);
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
                // The 0.69d0 scalp chosen by its look on him in the game
                // ([`NOVICE_SCALP`]; FACES_NOVICE_SCALP=<head>: another).
                let only = Some(
                    std::env::var("FACES_NOVICE_SCALP").unwrap_or_else(|_| NOVICE_SCALP.to_owned()),
                );
                let mut best: Option<(Vec<u8>, ScalpJoin, ScalpBake)> = None;
                for (model, s) in SCALP_DONORS {
                    if only.as_ref().is_some_and(|o| o != model) {
                        continue;
                    }
                    let donor = read_069(sources, bind_pose, model, s)?;
                    let (head, join, bake) = graft_scalp_069(&mesh, &donor, model)?;
                    println!(
                        "  scalp of {model}: his cap ({} vertices) and {} triangles above his hairline off, {} of its triangles, its edge onto his cut by {:.1} mm on average (at most {:.1}), {} triangles close the seam",
                        join.cap,
                        join.cut,
                        join.triangles,
                        join.edge_mean * 1000.0,
                        join.edge_farthest * 1000.0,
                        join.bridge
                    );
                    if best.as_ref().is_none_or(|b| join.edge_mean < b.1.edge_mean) {
                        best = Some((head, join, bake));
                    }
                }
                let (head, join, bake) = best.context("no scalp donor")?;
                println!("  the scalp: {}'s", join.donor);
                scalp_bake = Some(bake);
                vec![(surface.to_owned(), head)]
            } else {
                vec![(surface.to_owned(), mesh)]
            };
            out.insert(
                State::Bare,
                Drawn::new(format!("{} bare", face.key), surfaces),
            );
        }
    }
    Ok((out, joins, scalp_bake))
}

/// The live item (`mask/face_069d_<name>`) that is 0.69d0's face-wear with a
/// model for each 0.69d0 face (its `_head_00` the game's).
fn face_wear_spec(item: &str) -> Option<&'static crate::face_items_069d::Spec> {
    crate::face_items_069d::shipped().find(|s| {
        item == format!("mask/{}_{}", crate::face_items_069d::PORT, s.name)
            && s.model.ends_with("_head_00")
    })
}

/// 0.69d0's own model of a live face-wear item (`mask/face_069d_<name>`)
/// for a 0.69d0 face, converted as 0.69d0 draws it: `equipment.<item>.models
/// .head_<n>` of the four balaclavas, each the item's `_head_00` model of
/// `face_items_069d` for 0.69d0's face n: its cloth and the face it shows
/// ([`FACE_WEAR_FACE`]; 0.69d0 draws it with `remove_head_model`). `None` for
/// any other item or face.
fn face_wear_069d(
    sources: &Sources<'_>,
    bind_pose: &[u8],
    face: &Face,
    item: &str,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let Donor::V069 { model: head, .. } = face.donor else {
        return Ok(None);
    };
    let n = match head {
        "head_default" => 0,
        h => h
            .strip_prefix("head_")
            .and_then(|n| n.parse::<usize>().ok())
            .with_context(|| format!("0.69d0 head {h}"))?,
    };
    let Some(spec) = face_wear_spec(item) else {
        return Ok(None);
    };
    let model = format!("{}_head_{n:02}", spec.model.trim_end_matches("_head_00"));
    let root = sources
        .v069
        .join(format!("models/{model}.skinned_model/render"));
    let pose = fs::read(root.join("bind_pose"))?;
    let read = |surface: &str| -> Result<Vec<u8>> {
        let path = root.join(format!("{surface}/converted_model"));
        let bytes = fs::read(&path).with_context(|| path.display().to_string())?;
        let label = format!("0.69d {model} {surface}");
        let parsed = crate::v069d_skinned::parse_069d(&bytes, &label)?;
        let used = parsed
            .vertices
            .iter()
            .flat_map(|v| v.bones.iter().zip(v.weights))
            .filter(|(_, w)| *w > 0.0)
            .map(|(&b, _)| usize::from(b))
            .collect();
        same_rig_for(&pose, bind_pose, &label, Some(&used))?;
        crate::v069d_skinned::to_026_four_bone(&parsed)
    };
    Ok(Some((read(spec.surface)?, read(FACE_WEAR_FACE)?)))
}

/// The surface of 0.69d0's face-wear models with the face they show in their
/// eye holes (the face's mask head there, drawn with the face's material).
const FACE_WEAR_FACE: &str = "head_s";

/// A 0.69d0 head's surface as 0.69d0 draws it: its vertices, skin, basis
/// and UVs, on the native rig.
fn read_069(
    sources: &Sources<'_>,
    bind_pose: &[u8],
    model: &str,
    surface: &str,
) -> Result<Vec<u8>> {
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
    crate::v069d_skinned::to_026_four_bone(&parsed)
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

/// The 0.69d0 faces whose scalp the Novice may take: the short cuts (not
/// `head_05`'s mohawk nor `head_07`'s long hair), each its model and surface.
pub const SCALP_DONORS: [(&str, &str); 6] = [
    ("head_default", "head_default_sg"),
    ("head_01", "head_01_s"),
    ("head_02", "head_02_s"),
    ("head_03", "head_03_s"),
    ("head_04", "head_04_s"),
    ("head_06", "head_06_s"),
];

/// The scalp the Novice wears: `head_01`'s (dark, short, its forehead
/// smooth), chosen among [`SCALP_DONORS`] by its look on him in the game.
const NOVICE_SCALP: &str = "head_01";

/// The Novice's hairline, round his head: (degrees from his face, height in
/// m). Everything of his above it is his cap's line, its shadow and his own
/// hair, which the 0.69d0 scalp takes the place of: across the forehead
/// just under the band his cap shadowed, over the ears, behind them down to
/// the nape.
const HAIRLINE: [(f64, f64); 9] = [
    (0.0, 1.697),
    (35.0, 1.696),
    (55.0, 1.692),
    (75.0, 1.690),
    (95.0, 1.689),
    (112.0, 1.668),
    (128.0, 1.636),
    (148.0, 1.608),
    (180.0, 1.598),
];
/// An ear's vertex is kept this far (m) under the hairline at least: the
/// cut goes round the ears, not through them.
const EAR_KEEP: f64 = 0.003;
/// A vertex of the donor this near one of the Novice's ears (m) is of its
/// ear, which the scalp does not take.
const EAR_NEAR: f64 = 0.012;
/// How far from the seam the scalp's normals give way to the joined
/// surface's (m).
const SEAM_NORMALS: f64 = 0.012;
/// Rounds in which folds of the scalp are undone.
const UNFOLD_ROUNDS: usize = 200;
/// Rounds in which the scalp settles onto the cut.
const SCALP_SETTLE: usize = 400;

fn hairline_at(degrees: f64) -> f64 {
    let a = degrees.abs().min(180.0);
    let k = HAIRLINE
        .iter()
        .position(|h| h.0 >= a)
        .unwrap_or(HAIRLINE.len() - 1)
        .max(1);
    let ((a0, y0), (a1, y1)) = (HAIRLINE[k - 1], HAIRLINE[k]);
    y0 + (y1 - y0) * ((a - a0) / (a1 - a0)).clamp(0.0, 1.0)
}

/// The Novice's ears: his colour map's ear patches, below and right of
/// his face (UV), above the neck.
fn is_ear(v: &[u8; 48]) -> bool {
    let (u, w) = (
        f32::from_le_bytes(v[40..44].try_into().unwrap()),
        f32::from_le_bytes(v[44..48].try_into().unwrap()),
    );
    u > 0.78 && w > 0.72
}

/// His face without the mesh at its top (his knit cap, modelled on his head).
fn take_cap_off(face: &[u8]) -> Result<(Vec<u8>, usize, [f64; 4])> {
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
    // Where the cap is in his colour map: free for the scalp now.
    let mut cap_uv = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (i, k) in weld.iter().enumerate() {
        if part[*k] == cap {
            let uv = vertex_uv(&face[12 + i * 48..12 + (i + 1) * 48]);
            cap_uv = [
                cap_uv[0].min(uv[0]),
                cap_uv[1].min(uv[1]),
                cap_uv[2].max(uv[0]),
                cap_uv[3].max(uv[1]),
            ];
        }
    }
    Ok((bare, capped.len(), cap_uv))
}

/// A 48-byte vertex's UV.
fn vertex_uv(v: &[u8]) -> [f64; 2] {
    [40, 44].map(|at| f64::from(f32::from_le_bytes(v[at..at + 4].try_into().unwrap())))
}

/// Sets a 48-byte vertex's UV (and the fractions its basis carries).
fn set_vertex_uv(v: &mut [u8; 48], uv: [f64; 2]) {
    for (k, at) in [40, 44].into_iter().enumerate() {
        v[at..at + 4].copy_from_slice(&(uv[k] as f32).to_le_bytes());
    }
    v[35] = crate::v069d_skinned::uv_fraction(uv[0] as f32);
    v[39] = crate::v069d_skinned::uv_fraction(uv[1] as f32);
}

/// The ordered vertices (welded) of a surface's longest open loop.
fn longest_open_loop(mesh: &crate::mask_fit::Mesh, weld: &[usize]) -> Vec<usize> {
    let mut uses: BTreeMap<(usize, usize), (usize, usize, usize)> = BTreeMap::new();
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (wa, wb) = (weld[a], weld[b]);
            uses.entry((wa.min(wb), wa.max(wb)))
                .or_insert((0, wa, wb))
                .0 += 1;
        }
    }
    // Directed open edges, a -> b as the triangle runs.
    let mut next: BTreeMap<usize, usize> = BTreeMap::new();
    for (n, a, b) in uses.values() {
        if *n == 1 {
            next.insert(*a, *b);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut best: Vec<usize> = vec![];
    for &start in next.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut lp = vec![start];
        seen.insert(start);
        let mut at = start;
        while let Some(&b) = next.get(&at) {
            if b == start || seen.contains(&b) {
                break;
            }
            seen.insert(b);
            lp.push(b);
            at = b;
        }
        if lp.len() > best.len() {
            best = lp;
        }
    }
    best
}

/// How a 0.69d0 scalp joined the Novice's head.
#[derive(Clone, Debug, Serialize)]
pub struct ScalpJoin {
    pub donor: String,
    /// His vertices above the hairline left out (and his cap's), the
    /// scalp's triangles, the farthest and the mean move of its edge onto
    /// his cut (m), and the bridge's triangles closing the seam.
    pub cap: usize,
    pub cut: usize,
    pub triangles: usize,
    pub edge_farthest: f64,
    pub edge_mean: f64,
    pub bridge: usize,
}

/// A point of the seam: where it is, and the UV of his skin and of the
/// scalp there (both in his colour map).
#[derive(Clone, Copy, Debug)]
pub struct SeamPoint {
    pub at: [f64; 3],
    pub his: [f64; 2],
    pub scalp: [f64; 2],
}

/// What his maps need of the graft ([`bake_novice_maps`]): the scalp's
/// place in his UV map (`rect`, the cap's old place) and how the donor's
/// UVs map into it (`his = donor * scale + offset`), and the seam.
#[derive(Clone, Debug)]
pub struct ScalpBake {
    pub donor: String,
    pub rect: [f64; 4],
    pub scale: [f64; 2],
    pub offset: [f64; 2],
    pub seam: Vec<SeamPoint>,
}

/// The Novice's head, one surface: his cap, the band it shadowed and his
/// own hair off along [`HAIRLINE`] (his ears kept), and a 0.69d0 face's
/// scalp above the same line joined on. The scalp's cut edge is laid on his
/// at the same share of its length round the head (so it follows his edge
/// all round, round the ears too), the rest of it moved with the edge (each
/// vertex by the mean of its neighbours' moves), and the two edges closed by
/// a strip of the scalp's triangles; on the seam the scalp takes his
/// normals. The scalp keeps the donor's skin and basis; its UVs go into the
/// cap's old place in his colour map, which [`bake_novice_maps`] paints
/// with the donor's scalp. Returns the head, the join and what the maps need.
fn graft_scalp_069(
    face: &[u8],
    donor: &[u8],
    donor_name: &str,
) -> Result<(Vec<u8>, ScalpJoin, ScalpBake)> {
    use crate::face_shape::{Soup, position, set_position};
    let (capless, cap, cap_uv) = take_cap_off(face)?;
    let his = Soup::parse(&capless, "Novice")?;
    // The azimuth's axis: his head's middle (x, z) above the jaw.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for v in &his.vertices {
        let p = position(v);
        if p[1] > 1.62 {
            for (k, c) in [0, 2].into_iter().enumerate() {
                lo[k] = lo[k].min(p[c]);
                hi[k] = hi[k].max(p[c]);
            }
        }
    }
    let axis = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let line = |p: [f64; 3]| hairline_at((p[0] - axis[0]).atan2(p[2] - axis[1]).to_degrees());
    // His head under the line.
    let under: Vec<f64> = his
        .vertices
        .iter()
        .map(|v| {
            let p = position(v);
            let d = line(p) - p[1];
            if is_ear(v) { d.max(EAR_KEEP) } else { d }
        })
        .collect();
    let (mut head, cut) = his.clip(&under)?;
    let head_mesh = parse_mesh(&head.bytes()?, 48, "Novice")?;
    let (head_weld, _) = crate::mask_fit::weld_vertices(&head_mesh);
    let his_loop = longest_open_loop(&head_mesh, &head_weld);
    ensure!(
        his_loop.len() >= 16,
        "the Novice's head has no open top after the cut"
    );
    let first_of = |weld: &[usize], k: usize| weld.iter().position(|&w| w == k).unwrap();
    let his_at: Vec<usize> = his_loop.iter().map(|&k| first_of(&head_weld, k)).collect();
    let his_points: Vec<[f64; 3]> = his_at.iter().map(|&i| head_mesh.positions[i]).collect();
    let his_ears: Vec<[f64; 3]> = his
        .vertices
        .iter()
        .filter(|v| is_ear(v))
        .map(position)
        .collect();
    // The cap's place in his map, a little in from its edges: nothing of
    // his kept skin may be in it.
    let inset = 0.01;
    let rect = [
        cap_uv[0] + inset,
        cap_uv[1] + inset,
        cap_uv[2] - inset,
        cap_uv[3] - inset,
    ];
    ensure!(
        rect[0] < rect[2] && rect[1] < rect[3],
        "the Novice's cap has no room in his map"
    );
    let inside =
        |uv: [f64; 2]| uv[0] > rect[0] && uv[0] < rect[2] && uv[1] > rect[1] && uv[1] < rect[3];
    ensure!(
        !head.vertices.iter().any(|v| inside(vertex_uv(v))),
        "his kept skin is in the cap's place of his map"
    );

    // The donor above the line, but for its ears.
    let theirs = Soup::parse(donor, donor_name)?;
    let over: Vec<f64> = theirs
        .vertices
        .iter()
        .map(|v| {
            let p = position(v);
            let d = p[1] - line(p);
            let near_ear = his_ears.iter().any(|e| {
                let q = sub(p, *e);
                dot(q, q) < EAR_NEAR * EAR_NEAR
            });
            if near_ear { d.min(-EAR_KEEP) } else { d }
        })
        .collect();
    let (mut scalp, _) = theirs.clip(&over)?;
    // Only its largest piece.
    {
        let m = parse_mesh(&scalp.bytes()?, 48, "scalp")?;
        let (weld, neighbours) = crate::mask_fit::weld_vertices(&m);
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
        let main = (0..sizes.len())
            .max_by_key(|&c| sizes[c])
            .context("no scalp")?;
        let tris = m.triangles.clone();
        scalp = scalp.keep(|t| part[weld[tris[t][0]]] == main)?;
    }
    let scalp_mesh = parse_mesh(&scalp.bytes()?, 48, "scalp")?;
    let (weld, neighbours) = crate::mask_fit::weld_vertices(&scalp_mesh);
    let their_loop = longest_open_loop(&scalp_mesh, &weld);
    ensure!(
        their_loop.len() >= 16,
        "{donor_name}: the scalp has no open edge"
    );
    let ours: Vec<usize> = their_loop.iter().map(|&k| first_of(&weld, k)).collect();
    let (n, m) = (ours.len(), his_points.len());
    let at = |s: &Soup, i: usize| position(&s.vertices[i]);
    // Both loops the same way round and from the nearest pair.
    let start_his = (0..m)
        .min_by(|&a, &b| {
            let (da, db) = (
                sub(his_points[a], at(&scalp, ours[0])),
                sub(his_points[b], at(&scalp, ours[0])),
            );
            dot(da, da).total_cmp(&dot(db, db))
        })
        .unwrap();
    let ahead = at(&scalp, ours[n / 8]);
    let forward = {
        let f = sub(his_points[(start_his + m / 8) % m], ahead);
        let b = sub(his_points[(start_his + m - m / 8) % m], ahead);
        dot(f, f) < dot(b, b)
    };
    let his_index = |j: usize| -> usize {
        if forward {
            (start_his + j) % m
        } else {
            (start_his + m - j % m) % m
        }
    };
    let his_ordered: Vec<[f64; 3]> = (0..m).map(|j| his_points[his_index(j)]).collect();
    // The two edges matched in order, each vertex to the nearest it may be
    // (a path through the pairs from the start round to the start again,
    // never going back: dynamic time warping), so the scalp's edge follows
    // his all round, round the ears too.
    let dist = |i: usize, j: usize| {
        let d = sub(at(&scalp, ours[i]), his_ordered[j]);
        dot(d, d).sqrt()
    };
    let mut cost = vec![vec![f64::INFINITY; m]; n];
    for i in 0..n {
        for j in 0..m {
            let before = match (i, j) {
                (0, 0) => 0.0,
                (0, _) => cost[0][j - 1],
                (_, 0) => cost[i - 1][0],
                _ => cost[i - 1][j].min(cost[i][j - 1]).min(cost[i - 1][j - 1]),
            };
            cost[i][j] = before + dist(i, j);
        }
    }
    let mut path = vec![(n - 1, m - 1)];
    while let Some(&(i, j)) = path.last() {
        if (i, j) == (0, 0) {
            break;
        }
        let step = if i == 0 {
            (0, j - 1)
        } else if j == 0 {
            (i - 1, 0)
        } else {
            [(i - 1, j - 1), (i - 1, j), (i, j - 1)]
                .into_iter()
                .min_by(|a, b| cost[a.0][a.1].total_cmp(&cost[b.0][b.1]))
                .unwrap()
        };
        path.push(step);
    }
    path.reverse();
    // Each vertex's match: the nearest of the path's pairs it is in.
    let mut his_of = vec![(f64::INFINITY, 0); n];
    let mut scalp_of = vec![(f64::INFINITY, 0); m];
    for &(i, j) in &path {
        let d = dist(i, j);
        if d < his_of[i].0 {
            his_of[i] = (d, j);
        }
        if d < scalp_of[j].0 {
            scalp_of[j] = (d, i);
        }
    }
    // Its edge onto his, vertex for vertex (no vertex of one edge on a side
    // of the other: such a T-joint leaves cracks), the rest with it.
    let mut held = vec![false; neighbours.len()];
    let mut moves = vec![[0.0_f64; 3]; neighbours.len()];
    let (mut farthest, mut total) = (0.0_f64, 0.0);
    for (k, &i) in ours.iter().enumerate() {
        let target = his_ordered[his_of[k].1];
        let p = at(&scalp, i);
        let w = weld[i];
        held[w] = true;
        moves[w] = sub(target, p);
        let l = dot(moves[w], moves[w]).sqrt();
        farthest = farthest.max(l);
        total += l;
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
    // Where the edge moved far (round the ears) the moves can fold the
    // scalp's triangles over: a folded triangle's free corners go to their
    // neighbours' middle, round after round, until none is folded.
    let original = scalp_mesh.positions.clone();
    let mut placed: Vec<[f64; 3]> = (0..neighbours.len())
        .map(|k| {
            let i = first_of(&weld, k);
            let (p, mv) = (original[i], moves[k]);
            [p[0] + mv[0], p[1] + mv[1], p[2] + mv[2]]
        })
        .collect();
    // Which way a triangle should face: its corners' normals (a few of the
    // donor's own triangles round the ears face against theirs: as they
    // were, not folds).
    let facing: Vec<[f64; 3]> = scalp_mesh
        .triangles
        .iter()
        .map(|t| {
            t.iter().fold([0.0; 3], |s, &v| {
                let n = [2, 1, 0].map(|k| f64::from(scalp.vertices[v][28 + k]) / 127.5 - 1.0);
                [s[0] + n[0], s[1] + n[1], s[2] + n[2]]
            })
        })
        .collect();
    let folded = |placed: &[[f64; 3]]| -> Vec<usize> {
        scalp_mesh
            .triangles
            .iter()
            .enumerate()
            .filter(|&(k, t)| {
                let [a, b, c] = t.map(|v| original[v]);
                let [pa, pb, pc] = t.map(|v| placed[weld[v]]);
                let was = crate::mask_fit::cross(sub(b, a), sub(c, a));
                let now = crate::mask_fit::cross(sub(pb, pa), sub(pc, pa));
                // (A triangle the edge collapsed, two corners on one of his
                // edge vertices, covers nothing: not a fold.)
                dot(now, now) > 1e-14 && dot(was, facing[k]) >= 0.0 && dot(now, facing[k]) < 0.0
            })
            .map(|(k, _)| k)
            .collect()
    };
    let mut unfolded = 0;
    for _ in 0..UNFOLD_ROUNDS {
        let bad = folded(&placed);
        if bad.is_empty() {
            break;
        }
        unfolded += 1;
        let mut free: Vec<usize> = bad
            .iter()
            .flat_map(|&t| scalp_mesh.triangles[t].map(|v| weld[v]))
            .filter(|&k| !held[k])
            .collect();
        free.sort_unstable();
        free.dedup();
        for k in free {
            let n = neighbours[k].len() as f64;
            placed[k] =
                [0, 1, 2].map(|j| neighbours[k].iter().map(|&u| placed[u][j]).sum::<f64>() / n);
        }
    }
    let still_folded = folded(&placed).len();
    for (i, v) in scalp.vertices.iter_mut().enumerate() {
        set_position(v, placed[weld[i]]);
    }
    println!(
        "  {donor_name}'s scalp: folds undone in {unfolded} rounds, {still_folded} triangles still folded"
    );
    // The scalp's UVs into the cap's place of his map.
    let (mut ulo, mut uhi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for v in &scalp.vertices {
        let uv = vertex_uv(v);
        for k in 0..2 {
            ulo[k] = ulo[k].min(uv[k]);
            uhi[k] = uhi[k].max(uv[k]);
        }
    }
    let scale = [0, 1].map(|k| (rect[k + 2] - rect[k]) / (uhi[k] - ulo[k]).max(1e-9));
    let offset = [0, 1].map(|k| rect[k] - ulo[k] * scale[k]);
    for v in &mut scalp.vertices {
        let uv = vertex_uv(v);
        set_vertex_uv(v, [0, 1].map(|k| uv[k] * scale[k] + offset[k]));
    }
    // On the seam the scalp takes his skin and normal, its match's: posed,
    // a vertex skinned otherwise than the one it sits on parts from it (a
    // crack along the seam in the game).
    for (k, &i) in ours.iter().enumerate() {
        let his = head.vertices[his_at[his_index(his_of[k].1)]];
        scalp.vertices[i][12..32].copy_from_slice(&his[12..32]);
    }
    // The seam's points (his edge vertices) with the scalp's UV there.
    let seam: Vec<SeamPoint> = (0..m)
        .map(|j| SeamPoint {
            at: his_ordered[j],
            his: vertex_uv(&head.vertices[his_at[his_index(j)]]),
            scalp: vertex_uv(&scalp.vertices[ours[scalp_of[j].1]]),
        })
        .collect();
    // The strip between the two edges: his edge's points as the scalp's
    // vertices (its UVs there, his normals), then a zipper along both loops.
    let mut theirs_on_his = vec![];
    for (j, p) in his_ordered.iter().enumerate() {
        let mut v = scalp.vertices[ours[scalp_of[j].1]];
        set_position(&mut v, *p);
        set_vertex_uv(&mut v, seam[j].scalp);
        v[12..32].copy_from_slice(&head.vertices[his_at[his_index(j)]][12..32]);
        theirs_on_his.push(u16::try_from(scalp.vertices.len())?);
        scalp.vertices.push(v);
    }
    // The strip's winding: each of its triangles runs the scalp's edge the
    // other way from the scalp's own triangles (the edge loop's order), and
    // his edge as his do (his loop is the other way round from the scalp's:
    // `forward` false), so the strip is wound as both. (A winding guessed
    // from each sliver's own normal culled some: cracks along the seam.)
    ensure!(
        !forward,
        "{donor_name}: the scalp's edge and his run the same way round: one of them faces in"
    );
    // The zipper along the path, round to the start.
    let mut bridge = 0;
    let mut steps = path.clone();
    steps.push((n, m));
    for w in steps.windows(2) {
        let ((i, j), (i2, j2)) = (w[0], w[1]);
        let o = |i: usize| ours[i % n] as u16;
        let h = |j: usize| theirs_on_his[j % m];
        let mut tris = vec![];
        if j2 > j {
            tris.push([o(i), h(j), h(j2)]);
        }
        if i2 > i {
            tris.push([o(i), h(j2), o(i2)]);
        }
        for [a, b, c] in tris {
            // A triangle with two corners on one point covers nothing.
            let [pa, pb, pc] = [a, b, c].map(|v| at(&scalp, usize::from(v)));
            let same = |p: [f64; 3], q: [f64; 3]| {
                let d = sub(p, q);
                dot(d, d) < 1e-14
            };
            if !(same(pa, pb) || same(pb, pc) || same(pa, pc)) {
                scalp.triangles.push([a, b, c]);
                bridge += 1;
            }
        }
    }
    let join = ScalpJoin {
        donor: donor_name.to_owned(),
        cap,
        cut,
        triangles: scalp.triangles.len(),
        edge_farthest: farthest,
        edge_mean: total / n as f64,
        bridge,
    };
    // One surface: his head, then the scalp.
    let base = u16::try_from(head.vertices.len())?;
    let soup_triangles = head.triangles.len();
    head.vertices.extend(scalp.vertices.iter().copied());
    head.triangles
        .extend(scalp.triangles.iter().map(|t| t.map(|v| v + base)));
    // The normals across the seam from the joined surface: his edge's (cut
    // vertices, which took an inside corner's normal), the strip's, and the
    // scalp's near it, more the nearer (his own vertices keep theirs).
    {
        let mesh = parse_mesh(&head.bytes()?, 48, "Novice")?;
        let (weld, _) = crate::mask_fit::weld_vertices(&mesh);
        let mut sums = vec![[0.0_f64; 3]; weld.iter().copied().max().map_or(0, |k| k + 1)];
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|v| mesh.positions[v]);
            let n = crate::mask_fit::cross(sub(b, a), sub(c, a));
            for &v in t {
                for k in 0..3 {
                    sums[weld[v]][k] += n[k];
                }
            }
        }
        let stored =
            |v: &[u8; 48]| -> [f64; 3] { [2, 1, 0].map(|k| f64::from(v[28 + k]) / 127.5 - 1.0) };
        let on_edge: std::collections::HashSet<usize> = his_at.iter().copied().collect();
        let seam_distance = |p: [f64; 3]| -> f64 {
            his_ordered
                .iter()
                .map(|q| {
                    let d = sub(p, *q);
                    dot(d, d).sqrt()
                })
                .fold(f64::INFINITY, f64::min)
        };
        for (i, v) in head.vertices.iter_mut().enumerate() {
            let scalp_part = i >= usize::from(base);
            if !scalp_part && !on_edge.contains(&i) {
                continue;
            }
            let share = if scalp_part {
                let x = (seam_distance(position(v)) / SEAM_NORMALS).clamp(0.0, 1.0);
                1.0 - x * x * (3.0 - 2.0 * x)
            } else {
                1.0
            };
            if share <= 0.0 {
                continue;
            }
            let own = stored(v);
            let mut n = sums[weld[i]];
            let l = dot(n, n).sqrt();
            if l < 1e-12 {
                continue;
            }
            n = n.map(|x| x / l);
            if dot(n, own) < 0.0 {
                n = n.map(|x| -x);
            }
            let mixed = [0, 1, 2].map(|k| (own[k] + share * (n[k] - own[k])) as f32);
            let packed = crate::v069d_skinned::packed_basis(mixed, v[31]);
            v[28..32].copy_from_slice(&packed.to_le_bytes());
        }
        // The winding of the scalp's triangles near the seam (the strip's,
        // and the donor's round its ears, which faced in there behind its
        // own ears and showed as cracks): the way their corners' normals
        // face.
        let mut turned = 0;
        for t in head.triangles.iter_mut().skip(soup_triangles) {
            let [a, b, c] = t.map(|i| position(&head.vertices[usize::from(i)]));
            if seam_distance(a).min(seam_distance(b)).min(seam_distance(c)) > SEAM_NORMALS {
                continue;
            }
            let g = crate::mask_fit::cross(sub(b, a), sub(c, a));
            let n = t
                .iter()
                .map(|&i| stored(&head.vertices[usize::from(i)]))
                .fold([0.0; 3], |s, n| [s[0] + n[0], s[1] + n[1], s[2] + n[2]]);
            if dot(g, g) > 1e-14 && dot(g, n) < 0.0 {
                t.swap(1, 2);
                turned += 1;
            }
        }
        println!("  {donor_name}'s scalp: {turned} triangles near the seam turned to face out");
    }
    let bake = ScalpBake {
        donor: donor_name.to_owned(),
        rect,
        scale,
        offset,
        seam,
    };
    Ok((head.bytes()?, join, bake))
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
    /// What a grafted scalp needs of its face's maps, by face.
    pub bakes: BTreeMap<&'static str, ScalpBake>,
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
    /// Where it is worst (bind pose).
    pub at: [f64; 3],
}

/// Where an item's model for a face comes from.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemMade {
    /// The game's model carried from the native head onto the face.
    Carried,
    /// 0.69d0's own model of the item for the face, its neck laid where the
    /// game's model has it.
    Donor(crate::face_shape::NeckCarry),
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
    pub how: ItemMade,
    /// 0.69d0's face-wear: the face it shows in its eye holes (its
    /// `head_s`, drawn with the face's material), for an entry that draws no
    /// head (`remove_head_model`).
    pub face_part: Option<Vec<u8>>,
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
    /// The masks drawn with 0.69d0's own model of them for this face, which
    /// draws no head (the face it shows is the model's own): nothing of the
    /// face is measured under them.
    headless: std::collections::BTreeSet<usize>,
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
    let mut bakes: BTreeMap<&'static str, ScalpBake> = BTreeMap::new();
    let mut necks: Vec<(String, NeckJoin)> = vec![];
    for face in faces {
        if matches!(face.donor, Donor::Native) {
            continue;
        }
        let (variants, joins, bake) = donor_faces(sources, &bind_pose, &native_head, &rig, face)?;
        if let Some(bake) = bake {
            bakes.insert(face.key, bake);
        }
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
            bakes,
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
    // FACES_NO_CUT: the numbers without the cut.
    let no_cut = std::env::var_os("FACES_NO_CUT").is_some();

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
        // Under a mask the face is 0.69d0's own mask head where it has one,
        // as 0.69d0 draws a masked face (its face-wear for each face is made
        // over it), else the helmet head.
        let mask = donor
            .get(&State::Mask)
            .cloned()
            .unwrap_or_else(|| helmet.clone());
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
            headless: std::collections::BTreeSet::new(),
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
    // 0.69d0's own face-wear for each of its faces: the item's model for
    // that face, which draws no head but the face it shows in its eye holes.
    for face in &mut shaped {
        for &i in &masks {
            let cover = &wardrobe.covers[i];
            let Some((cloth, face_part)) =
                face_wear_069d(sources, &bind_pose, &face.face, &cover.label)?
            else {
                continue;
            };
            let [part] = &cover.parts[..] else {
                bail!("{}: face-wear of more than one part", cover.label);
            };
            let label = format!("{} for {}", cover.label, face.face.key);
            let live = view.read(&format!(
                "models/{}.skinned_model/render/{}/converted_model",
                part.0, part.1
            ))?;
            let (joined, neck) = crate::face_shape::onto_live_neck(&cloth, &live, &rig, &label)?;
            face.items.insert(
                i,
                FaceItem {
                    face: face.face,
                    label: cover.label.clone(),
                    config: cover.config.clone(),
                    key: cover.key.clone(),
                    parts: cover.parts.clone(),
                    surfaces: BTreeMap::from([(part.clone(), joined)]),
                    smooth: 0,
                    carried: Carried::default(),
                    how: ItemMade::Donor(neck),
                    face_part: Some(face_part),
                    settled: (0, 0.0),
                },
            );
            face.headless.insert(i);
        }
    }
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
                    if face.headless.contains(&i) {
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
            // Only masks are made for a face: the helmets are the game's.
            let mut need: std::collections::BTreeSet<usize> = shaped[f]
                .stand_off
                .iter()
                .filter(|(i, o)| o.off > 0 && is_mask(&wardrobe.covers[**i].slot))
                .map(|(i, _)| *i)
                .collect();
            let stood_off = need.len();
            // A mask the face shows through when it is worn alone; then, of
            // a helmet over a mask that each fit alone, the mask.
            let mut together = vec![];
            for (state, at) in [(State::Mask, 2)] {
                let sweep = sweeps[f * 3 + at].as_ref().unwrap();
                for (w, m) in wardrobe.worn[&state].iter().zip(&sweep.measures) {
                    if m.margin >= 0.0 || w.covers.iter().any(|i| shaped[f].headless.contains(i)) {
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
                    need.extend(
                        covers
                            .into_iter()
                            .filter(|&i| is_mask(&wardrobe.covers[i].slot)),
                    );
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
                // 0.69d0's face-wear is not carried: its rolled insides,
                // under the native head, do not follow a face as its cloth
                // does (they came through on the Novice); a face 0.69d0 has
                // no model for wears the game's.
                if tried >= CARRY_LEVELS.len()
                    || shaped[f].headless.contains(&i)
                    || face_wear_spec(&wardrobe.covers[i].label).is_some()
                {
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
            type Made = (usize, usize, FaceItem, face_fit::Cover);
            let make = |t: Todo| -> Result<Made> {
                let smooth = CARRY_LEVELS[t.tried];
                let cover = &wardrobe.covers[t.cover];
                let label = format!("{} for {}", cover.label, face.face.key);
                let mut carried = Carried::default();
                let mut surfaces = BTreeMap::new();
                let carry = crate::face_shape::Carry::new(
                    &native_mesh,
                    &merged_mesh(&face.whole[&t.state])?,
                );
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
                    how: ItemMade::Carried,
                    face_part: None,
                    settled,
                };
                Ok((t.cover, t.tried + 1, item, made))
            };
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let mut lots: Vec<Vec<Todo>> = (0..threads).map(|_| vec![]).collect();
            for (k, t) in todo.into_iter().enumerate() {
                lots[k % threads].push(t);
            }
            let done: Vec<Made> = std::thread::scope(|scope| {
                let workers: Vec<_> = lots
                    .into_iter()
                    .filter(|lot| !lot.is_empty())
                    .map(|lot| {
                        let make = &make;
                        scope.spawn(move || lot.into_iter().map(make).collect::<Result<Vec<_>>>())
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
                face.tries.insert(i, tried);
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
            // Under a mask drawn with 0.69d0's own model for the face, no
            // head is drawn: nothing of the face to measure.
            let mut worst: Vec<Measure> = sweep
                .measures
                .iter()
                .zip(&wardrobe.worn[&state])
                .filter(|(m, w)| {
                    m.margin.is_finite() && !w.covers.iter().any(|i| face.headless.contains(i))
                })
                .map(|(m, _)| m.clone())
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
                    at: m.at,
                });
            }
            for o in &stand_off {
                failures.push(Misfit {
                    face: face.face.key.to_owned(),
                    state,
                    what: format!("{} stands off", o.item),
                    mm: o.excess * 1000.0,
                    pose: "bind pose".to_owned(),
                    at: o.at,
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
            "made": i.how,
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
        bakes,
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
    // FACES_DUMP=1: every drawn face and item made for a face as its
    // converted surfaces under `dump/`, while looking into a fit.
    if std::env::var_os("FACES_DUMP").is_some() {
        let dump = output.join("dump");
        fs::create_dir_all(&dump)?;
        for (face, drawn) in &fitted.faces {
            for (state, d) in drawn {
                for (surface, bytes) in &d.surfaces {
                    fs::write(
                        dump.join(format!("{}-{state:?}-{surface}", face.key)),
                        bytes,
                    )?;
                }
            }
        }
        for item in &fitted.items {
            for ((_, surface), bytes) in &item.surfaces {
                let name = format!(
                    "{}-{}-{surface}",
                    item.face.key,
                    item.label.replace('/', "_")
                );
                fs::write(dump.join(name), bytes)?;
            }
        }
    }
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
/// STK2's 2048 maps drawn at the native head's 1024.
const STK2_MAP: u32 = 1024;
/// The Novice's maps with the scalp baked in: his head and the scalp in one
/// map, at the size of 0.69d0's (2048 wide).
const NOVICE_MAP: u32 = 2048;

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

/// The middle (median) UV of a surface's vertices in front of the face
/// (|x| under 25 mm, z over 50 mm) between two heights (m).
fn forehead_uv(surface: &[u8], label: &str, from: f64, to: f64) -> Result<[f64; 2]> {
    let mesh = crate::face_shape::Soup::parse(surface, label)?;
    let uvs: Vec<[f64; 2]> = mesh
        .vertices
        .iter()
        .filter(|v| {
            let p = crate::face_shape::position(v);
            p[0].abs() < 0.025 && p[1] > from && p[1] < to && p[2] > 0.05
        })
        .map(|v| vertex_uv(v))
        .collect();
    ensure!(
        !uvs.is_empty(),
        "{label}: no forehead between {from} and {to} m"
    );
    let median = |k: usize| {
        let mut x: Vec<f64> = uvs.iter().map(|u| u[k]).collect();
        x.sort_by(f64::total_cmp);
        x[x.len() / 2]
    };
    Ok([median(0), median(1)])
}

/// An 8-bit image's texels, `channels` a texel.
struct Raster {
    width: usize,
    height: usize,
    channels: usize,
    data: Vec<u8>,
}

impl Raster {
    /// The first image of a file (a DDS's top mip), as RGB or RGBA.
    fn read(path: &Path, rgba: bool) -> Result<Self> {
        let first = format!("{}[0]", path.display());
        let size = magick(&["identify", "-format", "%w %h", &first])?;
        let wh: Vec<usize> = size
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        ensure!(wh.len() == 2, "{}: no size ({size})", path.display());
        let channels = if rgba { 4 } else { 3 };
        let result = std::process::Command::new("magick")
            .arg(&first)
            .args(["-depth", "8", if rgba { "rgba:-" } else { "rgb:-" }])
            .output()
            .context("starting ImageMagick; enter the repository Nix shell")?;
        ensure!(
            result.status.success(),
            "ImageMagick: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        ensure!(
            result.stdout.len() == wh[0] * wh[1] * channels,
            "{}: {} bytes for {}x{}",
            path.display(),
            result.stdout.len(),
            wh[0],
            wh[1]
        );
        Ok(Self {
            width: wh[0],
            height: wh[1],
            channels,
            data: result.stdout,
        })
    }

    /// Bilinear at `uv` (clamped), each channel 0..255.
    fn sample(&self, uv: [f64; 2]) -> [f64; 4] {
        let x = (uv[0] * self.width as f64 - 0.5).clamp(0.0, (self.width - 1) as f64);
        let y = (uv[1] * self.height as f64 - 0.5).clamp(0.0, (self.height - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let mut out = [0.0; 4];
        for (c, o) in out.iter_mut().enumerate().take(self.channels) {
            let t =
                |x: usize, y: usize| f64::from(self.data[(y * self.width + x) * self.channels + c]);
            *o = (t(x0, y0) * (1.0 - fx) + t(x1, y0) * fx) * (1.0 - fy)
                + (t(x0, y1) * (1.0 - fx) + t(x1, y1) * fx) * fy;
        }
        out
    }

    fn set(&mut self, x: usize, y: usize, v: [f64; 4]) {
        let at = (y * self.width + x) * self.channels;
        for (d, c) in self.data[at..at + self.channels].iter_mut().zip(v) {
            *d = c.round().clamp(0.0, 255.0) as u8;
        }
    }

    fn get(&self, x: usize, y: usize) -> [f64; 4] {
        let at = (y * self.width + x) * self.channels;
        let mut out = [0.0; 4];
        for (o, &d) in out.iter_mut().zip(&self.data[at..at + self.channels]) {
            *o = f64::from(d);
        }
        out
    }

    /// As a binary PPM (RGB).
    fn write_ppm(&self, path: &Path) -> Result<()> {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        for t in self.data.chunks(self.channels) {
            out.extend_from_slice(&t[..3]);
        }
        fs::write(path, out)?;
        Ok(())
    }
}

/// His forehead in his map: the columns whose cut is at most this many
/// texels under its top, and the columns its skin fades in over at their ends.
const BAND_SPAN: f64 = 130.0;
const BAND_FADE: f64 = 24.0;
/// How far under the cut his real skin is looked for (texels), and how far
/// above it his skin is painted (for the blend's means and the mips).
const BAND_DEPTH: usize = 110;
const BAND_ABOVE: usize = 24;

/// His map under his cap: from the cut down, the cap's painted shadow and
/// brim, then his skin. Under the forehead's part of the cut (`cut`, his UVs
/// along it), each column takes from where his skin starts upwards his skin
/// under it, mirrored there (no edge where it starts: the same texels meet);
/// the normal map with it (its y turned with the mirror). The start is
/// smoothed along the columns, and the paint fades in at the forehead's ends.
/// Returns the columns painted and the most rows under the cut.
fn skin_over_cap_band(
    colour: &mut Raster,
    normals: &mut Raster,
    cut: &[[f64; 2]],
) -> Result<(usize, usize)> {
    let (w, h) = (colour.width, colour.height);
    let mut cut_row = vec![f64::INFINITY; w];
    for k in 0..cut.len() {
        let (a, b) = (cut[k], cut[(k + 1) % cut.len()]);
        let (ax, ay, bx, by) = (
            a[0] * w as f64,
            a[1] * h as f64,
            b[0] * w as f64,
            b[1] * h as f64,
        );
        // (Two points a map apart: the loop crossing between islands.)
        if (bx - ax).abs() > w as f64 / 8.0 {
            continue;
        }
        let (x0, x1) = (
            ax.min(bx).ceil().max(0.0) as usize,
            (ax.max(bx).floor() as usize).min(w - 1),
        );
        for (x, row) in cut_row.iter_mut().enumerate().take(x1 + 1).skip(x0) {
            let t = if (bx - ax).abs() < 1e-9 {
                0.0
            } else {
                (x as f64 - ax) / (bx - ax)
            };
            *row = row.min(ay + t * (by - ay));
        }
    }
    let middle = (0..w)
        .min_by(|&a, &b| cut_row[a].total_cmp(&cut_row[b]))
        .context("no cut")?;
    let top = cut_row[middle];
    ensure!(top.is_finite(), "his cut crosses no column of his map");
    let within = |x: usize| cut_row[x] <= top + BAND_SPAN;
    let (mut lo, mut hi) = (middle, middle);
    while lo > 0 && within(lo - 1) {
        lo -= 1;
    }
    while hi + 1 < w && within(hi + 1) {
        hi += 1;
    }
    let lum = |x: usize, y: usize| {
        let p = colour.get(x, y);
        0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]
    };
    // His skin's brightness: under the cut's middle, past the band.
    let mut skin: Vec<f64> = (middle.saturating_sub(60)..=(middle + 60).min(w - 1))
        .flat_map(|x| {
            let y = cut_row[x] as usize;
            (y + 30..y + 50).map(move |y| (x, y))
        })
        .map(|(x, y)| lum(x, y))
        .collect();
    skin.sort_by(f64::total_cmp);
    let reference = skin[skin.len() / 2];
    // Where his skin starts in each column: six rows on at its brightness.
    let raw: Vec<f64> = (lo..=hi)
        .map(|x| {
            let from = cut_row[x] as usize;
            (from..from + BAND_DEPTH)
                .find(|&y| (y..y + 6).all(|y| lum(x, y) > 0.85 * reference))
                .unwrap_or(from + BAND_DEPTH) as f64
        })
        .collect();
    let n = raw.len();
    let median: Vec<f64> = (0..n)
        .map(|i| {
            let mut v: Vec<f64> = raw[i.saturating_sub(20)..(i + 21).min(n)].to_vec();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        })
        .collect();
    let starts: Vec<f64> = (0..n)
        .map(|i| {
            let v = &median[i.saturating_sub(30)..(i + 31).min(n)];
            v.iter().sum::<f64>() / v.len() as f64
        })
        .collect();
    let (c0, n0) = (
        Raster {
            data: colour.data.clone(),
            ..*colour
        },
        Raster {
            data: normals.data.clone(),
            ..*normals
        },
    );
    let mut deepest = 0;
    for (i, x) in (lo..=hi).enumerate() {
        let ends = ((x - lo) as f64).min((hi - x) as f64) / BAND_FADE;
        let fade = ends.clamp(0.0, 1.0);
        let fade = fade * fade * (3.0 - 2.0 * fade);
        let start = (starts[i].round() as usize).max(cut_row[x] as usize + 4);
        deepest = deepest.max(start - cut_row[x] as usize);
        for y in (cut_row[x] as usize).saturating_sub(BAND_ABOVE)..start {
            let from = (2 * start - y).min(h - 1);
            let mix = |a: [f64; 4], b: [f64; 4]| [0, 1, 2, 3].map(|c| a[c] + fade * (b[c] - a[c]));
            colour.set(x, y, mix(c0.get(x, y), c0.get(x, from)));
            let mut nv = n0.get(x, from);
            nv[1] = 255.0 - nv[1];
            normals.set(x, y, mix(n0.get(x, y), nv));
        }
    }
    Ok((hi - lo + 1, deepest))
}

/// How far either side of the seam the tone gives way to the seam's and the
/// detail thins (m).
const SEAM_BAND: f64 = 0.015;
/// How much of either side's detail is gone at the seam itself.
const HAIR_THINS: f64 = 0.6;
/// Texels painted past the edge of a part of the map near the seam: the
/// mips take them, and what was there (his cap's lifted shadow, pale) came
/// through at a glancing view as a light line along the seam.
const SEAM_DILATE: usize = 48;

/// The Novice's colour and normal maps as one (2048), for his head drawn
/// as one surface with one material: his own, his cap's old place painted
/// with the donor's scalp (its colour toned to his skin: the mean of his
/// forehead just under the seam over the donor's just above it, a gain a
/// channel; its normal map's), and over [`SEAM_BAND`] either side of the
/// seam the tone giving way to both sides' mean there and the detail
/// thinning (colours and normals): at the seam both sides are alike.
/// Returns the colour and normal maps' PPMs and the record.
fn bake_novice_maps(
    view: &mut ResourceDb,
    sources: &Sources<'_>,
    face: &Face,
    bake: &ScalpBake,
    head: &[u8],
    work: &Path,
) -> Result<(PathBuf, PathBuf, serde_json::Value)> {
    let Donor::Stk2 { texture, .. } = face.donor else {
        bail!("{}: only a STK2 face has a grafted scalp", face.key);
    };
    let (model, surface) = SCALP_DONORS
        .iter()
        .find(|d| d.0 == bake.donor)
        .with_context(|| format!("{} is not a scalp donor", bake.donor))?;
    fs::create_dir_all(work)?;
    let dir = sources
        .v069
        .join(format!("textures/character/human/head/{model}"));
    let theirs = Raster::read(&dir.join(format!("{model}.dds")), false)?;
    let their_normals = Raster::read(&dir.join(format!("{model}_nmap.dds")), true)?;
    let own = sources
        .stk2_textures
        .join(STK2_TEXTURES)
        .join(format!("{texture}.tga"));
    let mut colour = Raster::read(&own, false)?;
    let mut normals = Raster::read(
        &sources
            .stk2_textures
            .join(STK2_TEXTURES)
            .join(format!("{texture}_nmap.tga")),
        false,
    )?;
    ensure!(
        colour.width == normals.width && colour.height == normals.height,
        "his colour and normal maps differ in size"
    );
    let cut: Vec<[f64; 2]> = bake.seam.iter().map(|p| p.his).collect();
    let (columns, rows) = skin_over_cap_band(&mut colour, &mut normals, &cut)?;
    println!(
        "  {}'s cap band: his skin over {columns} columns (up to {rows} rows under the cut)",
        face.key
    );
    // The tone: his forehead just under the seam over the donor's above it.
    let bind_pose = view.read(HEAD_BIND_POSE)?;
    let line = HAIRLINE[0].1;
    let mean = |r: &Raster, uv: [f64; 2]| -> [f64; 3] {
        let mut sum = [0.0; 3];
        let mut n = 0.0;
        for dy in -8..=8 {
            for dx in -24..=24 {
                let s = r.sample([
                    uv[0] + f64::from(dx) / r.width as f64,
                    uv[1] + f64::from(dy) / r.height as f64,
                ]);
                for k in 0..3 {
                    sum[k] += s[k];
                }
                n += 1.0;
            }
        }
        sum.map(|v| v / n)
    };
    let theirs_at = forehead_uv(
        &read_069(sources, &bind_pose, model, surface)?,
        model,
        line,
        line + 0.012,
    )?;
    let ours_at = forehead_uv(head, face.key, line - 0.014, line - 0.002)?;
    let (t, o) = (mean(&theirs, theirs_at), mean(&colour, ours_at));
    let gain = [0, 1, 2].map(|k| o[k] / t[k].max(1.0));
    // The cap's old place: the donor's scalp.
    let (w, h) = (colour.width, colour.height);
    let px = |r: [f64; 4]| -> [usize; 4] {
        [
            (r[0] * w as f64).floor() as usize,
            (r[1] * h as f64).floor() as usize,
            ((r[2] * w as f64).ceil() as usize).min(w - 1),
            ((r[3] * h as f64).ceil() as usize).min(h - 1),
        ]
    };
    let [x0, y0, x1, y1] = px([
        bake.rect[0] - 0.005,
        bake.rect[1] - 0.005,
        bake.rect[2] + 0.005,
        bake.rect[3] + 0.005,
    ]);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let uv = [(x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64];
            let donor_uv = [0, 1].map(|k| (uv[k] - bake.offset[k]) / bake.scale[k]);
            let c = theirs.sample(donor_uv);
            colour.set(x, y, [c[0] * gain[0], c[1] * gain[1], c[2] * gain[2], 0.0]);
            // 0.69d0's normal map holds x in alpha and y in green; his, x in red.
            let n = their_normals.sample(donor_uv);
            let (nx, ny) = (n[3] / 127.5 - 1.0, n[1] / 127.5 - 1.0);
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            normals.set(x, y, [n[3], n[1], (nz + 1.0) * 127.5, 0.0]);
        }
    }
    // Across the seam: each texel of a triangle near it, by its point on
    // the head, takes the tone of both sides' mean at the nearest point of
    // the seam (the means over a few texels) in place of its own and keeps
    // less of its detail the nearer it is: at the seam both sides are alike.
    let (colour0, normals0) = (
        Raster {
            data: colour.data.clone(),
            ..colour
        },
        Raster {
            data: normals.data.clone(),
            ..normals
        },
    );
    let soup = crate::face_shape::Soup::parse(head, face.key)?;
    let seam = &bake.seam;
    let m = seam.len();
    let nearest = |p: [f64; 3]| -> (f64, usize, f64) {
        let mut best = (f64::INFINITY, 0, 0.0);
        for k in 0..m {
            let (a, b) = (seam[k].at, seam[(k + 1) % m].at);
            let ab = sub(b, a);
            let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-18)).clamp(0.0, 1.0);
            let d = sub(p, [0, 1, 2].map(|j| a[j] + t * ab[j]));
            let d = dot(d, d);
            if d < best.0 {
                best = (d, k, t);
            }
        }
        (best.0.sqrt(), best.1, best.2)
    };
    // The texels any triangle of the head uses: never painted over below.
    let mut used = vec![false; w * h];
    for tri in &soup.triangles {
        let uv = tri.map(|i| vertex_uv(&soup.vertices[usize::from(i)]));
        let (fx, fy) = (uv.map(|u| u[0] * w as f64), uv.map(|u| u[1] * h as f64));
        let area = (fx[1] - fx[0]) * (fy[2] - fy[0]) - (fx[2] - fx[0]) * (fy[1] - fy[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let lo = |f: [f64; 3], n: usize| {
            (f.iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as usize)
                .min(n - 1)
        };
        let hi = |f: [f64; 3], n: usize| {
            (f.iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                .max(0.0) as usize)
                .min(n - 1)
        };
        for y in lo(fy, h)..=hi(fy, h) {
            for x in lo(fx, w)..=hi(fx, w) {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let b1 = ((cx - fx[0]) * (fy[2] - fy[0]) - (fx[2] - fx[0]) * (cy - fy[0])) / area;
                let b2 = ((fx[1] - fx[0]) * (cy - fy[0]) - (cx - fx[0]) * (fy[1] - fy[0])) / area;
                if b1 >= -0.02 && b2 >= -0.02 && 1.0 - b1 - b2 >= -0.02 {
                    used[y * w + x] = true;
                }
            }
        }
    }
    // The mean round a texel of the texels triangles use (not the empty map
    // past an island's edge, which would light its border).
    let box_mean = |r: &Raster, uv: [f64; 2]| -> [f64; 4] {
        let (cx, cy) = ((uv[0] * w as f64) as i64, (uv[1] * h as f64) as i64);
        let mut sum = [0.0; 4];
        let mut n = 0.0;
        for y in (cy - 5).max(0)..=(cy + 5).min(h as i64 - 1) {
            for x in (cx - 5).max(0)..=(cx + 5).min(w as i64 - 1) {
                let (x, y) = (x as usize, y as usize);
                if !used[y * w + x] {
                    continue;
                }
                let v = r.get(x, y);
                for c in 0..4 {
                    sum[c] += v[c];
                }
                n += 1.0;
            }
        }
        if n == 0.0 {
            return r.sample(uv);
        }
        sum.map(|v| v / n)
    };
    // Per seam point, per map: the mean of both sides there.
    let means: Vec<[[f64; 4]; 2]> = seam
        .iter()
        .map(|p| {
            [&colour0, &normals0].map(|r| {
                let (a, b) = (box_mean(r, p.his), box_mean(r, p.scalp));
                [0, 1, 2, 3].map(|c| (a[c] + b[c]) / 2.0)
            })
        })
        .collect();
    let mut covered = vec![false; w * h];
    let mut blended = 0;
    for tri in &soup.triangles {
        let v = tri.map(|i| &soup.vertices[usize::from(i)]);
        let pos = v.map(crate::face_shape::position);
        if pos.iter().all(|p| nearest(*p).0 > SEAM_BAND + 0.01) {
            continue;
        }
        let uv = v.map(|v| vertex_uv(v));
        let fx = uv.map(|u| u[0] * w as f64);
        let fy = uv.map(|u| u[1] * h as f64);
        let (bx0, bx1) = (
            fx.iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as usize,
            (fx.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() as usize).min(w - 1),
        );
        let (by0, by1) = (
            fy.iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as usize,
            (fy.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() as usize).min(h - 1),
        );
        let area = (fx[1] - fx[0]) * (fy[2] - fy[0]) - (fx[2] - fx[0]) * (fy[1] - fy[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in by0..=by1 {
            for x in bx0..=bx1 {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let b1 = ((cx - fx[0]) * (fy[2] - fy[0]) - (fx[2] - fx[0]) * (cy - fy[0])) / area;
                let b2 = ((fx[1] - fx[0]) * (cy - fy[0]) - (cx - fx[0]) * (fy[1] - fy[0])) / area;
                let b0 = 1.0 - b1 - b2;
                if b0 < -0.02 || b1 < -0.02 || b2 < -0.02 {
                    continue;
                }
                let p = [0, 1, 2].map(|k| b0 * pos[0][k] + b1 * pos[1][k] + b2 * pos[2][k]);
                let (d, k, t) = nearest(p);
                if d >= SEAM_BAND {
                    continue;
                }
                let x01 = (d / SEAM_BAND).clamp(0.0, 1.0);
                let wgt = 1.0 - x01 * x01 * (3.0 - 2.0 * x01);
                let (ma, mb) = (&means[k], &means[(k + 1) % m]);
                let uv_here = [(x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64];
                // Its own tone (the mean round it) gives way to the seam's,
                // and its detail thins towards it: both sides alike there.
                // The normal map goes flat at the seam: the two sides' tangent
                // frames differ, and only the (joined) vertex normals light it.
                let shift = |r0: &Raster, map: usize| -> [f64; 4] {
                    let own = r0.get(x, y);
                    let tone = box_mean(r0, uv_here);
                    let thins = if map == 0 { HAIR_THINS } else { 1.0 };
                    [0, 1, 2, 3].map(|c| {
                        let target = if map == 0 {
                            ma[map][c] + t * (mb[map][c] - ma[map][c])
                        } else {
                            [127.5, 127.5, 255.0, 255.0][c]
                        };
                        let detail = (own[c] - tone[c]) * (1.0 - thins * wgt);
                        tone[c] + wgt * (target - tone[c]) + detail
                    })
                };
                let (cv, nv) = (shift(&colour0, 0), shift(&normals0, 1));
                colour.set(x, y, cv);
                normals.set(x, y, nv);
                covered[y * w + x] = true;
                blended += 1;
            }
        }
    }
    // Past the edges of what was blended, the edge's texels (the mips).
    let (mut bx0, mut by0, mut bx1, mut by1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if covered[y * w + x] {
                (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
            }
        }
    }
    let (bx0, by0) = (
        bx0.saturating_sub(SEAM_DILATE + 1).max(1),
        by0.saturating_sub(SEAM_DILATE + 1).max(1),
    );
    let (bx1, by1) = (
        (bx1 + SEAM_DILATE + 1).min(w - 2),
        (by1 + SEAM_DILATE + 1).min(h - 2),
    );
    for _ in 0..SEAM_DILATE {
        let mut next = covered.clone();
        let mut add = vec![];
        for y in by0..=by1.max(by0) {
            for x in bx0..=bx1.max(bx0) {
                if covered[y * w + x] {
                    continue;
                }
                let around: Vec<(usize, usize)> = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                    .into_iter()
                    .filter(|&(a, b)| covered[b * w + a])
                    .collect();
                if around.is_empty() {
                    continue;
                }
                let avg = |r: &Raster| {
                    let mut sum = [0.0; 4];
                    for &(a, b) in &around {
                        let v = r.get(a, b);
                        for c in 0..4 {
                            sum[c] += v[c];
                        }
                    }
                    sum.map(|v| v / around.len() as f64)
                };
                add.push((x, y, avg(&colour), avg(&normals)));
                next[y * w + x] = true;
            }
        }
        for (x, y, c, n) in add {
            // Never over a texel a triangle uses. (In the scalp's place too:
            // what lies past its part there is the rest of the donor's map,
            // its face, and the filtering took it in as a line along the seam.)
            if used[y * w + x] {
                continue;
            }
            colour.set(x, y, c);
            normals.set(x, y, n);
        }
        covered = next;
    }
    let colour_path = work.join(format!("{}_baked.ppm", face.key));
    let normal_path = work.join(format!("{}_baked_nmap.ppm", face.key));
    colour.write_ppm(&colour_path)?;
    normals.write_ppm(&normal_path)?;
    println!(
        "  {}'s maps: {}'s scalp in his cap's place (gain {:.3} {:.3} {:.3}), {blended} texels blended across the seam",
        face.key, bake.donor, gain[0], gain[1], gain[2]
    );
    Ok((
        colour_path,
        normal_path,
        json!({"scalp_of": bake.donor, "gain": gain, "rect": bake.rect, "seam_band_m": SEAM_BAND, "blended_texels": blended}),
    ))
}

/// A face's maps: 0.69d's DDS as they are, STK2's TGAs encoded at the
/// native head's size.
fn donor_maps(
    sources: &Sources<'_>,
    face: &Face,
    work: &Path,
    hair: bool,
    baked: Option<&(PathBuf, PathBuf)>,
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
                let mut source = sources
                    .stk2_textures
                    .join(STK2_TEXTURES)
                    .join(format!("{texture}{}.tga", role.suffix()));
                // A grafted scalp: the maps baked with it, at their size.
                let mut size = STK2_MAP;
                if let Some((colour, normal)) = baked {
                    source = if role == Role::Diffuse {
                        colour.clone()
                    } else {
                        normal.clone()
                    };
                    size = NOVICE_MAP;
                }
                let small = work.join(format!("{}{}.tga", face.key, role.suffix()));
                let result = std::process::Command::new("magick")
                    .arg(&source)
                    .args(["-alpha", "off", "-filter", "Lanczos"])
                    .args(["-resize", &format!("{size}x{size}!")])
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
            Donor::Stk2 { .. } => {
                // A grafted scalp is a part of his head: one surface, one
                // material, his maps with the scalp baked in.
                let work = output.join("textures-work");
                let baked = match fitted.bakes.get(face.key) {
                    Some(bake) => {
                        let (colour, normal, rec) = bake_novice_maps(
                            view,
                            sources,
                            face,
                            bake,
                            &variants[&State::Bare].surfaces[0].1,
                            &work,
                        )?;
                        materials_record.insert(format!("{}_scalp", face.key), rec);
                        Some((colour, normal))
                    }
                    None => None,
                };
                let maps = donor_maps(sources, face, &work, false, baked.as_ref())?;
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
                (m, None)
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
                if item.face_part.is_some() && rest == "settings" {
                    continue;
                }
                let carried = rest
                    .strip_prefix("render/")
                    .and_then(|r| r.strip_suffix("/converted_model"))
                    .and_then(|surface| item.surfaces.get(&(model.clone(), surface.to_owned())));
                // A surface's properties name its counts: those of the model
                // made for the face (its bounds are the game model's, within
                // millimetres).
                let made_props = rest
                    .strip_prefix("render/")
                    .and_then(|r| r.strip_suffix("/export_properties"))
                    .and_then(|surface| item.surfaces.get(&(model.clone(), surface.to_owned())));
                if let Some(made) = made_props {
                    let mut props = parse_config(view, &path, "surface properties")?;
                    let vertices = read_u32(made, 8)?;
                    let indices = read_u32(made, 12 + read_u32(made, 4)? - 4 + 8)?;
                    for (key, value) in [
                        ("source.vertices", vertices),
                        ("source.indices", indices),
                        ("source.primitives", indices / 3),
                    ] {
                        if props
                            .root
                            .named("source")
                            .and_then(|v| v.named(key.trim_start_matches("source.")))
                            .is_some()
                        {
                            set(&mut props, key, V::Integer(i32::try_from(value)?))?;
                        }
                    }
                    out.config(
                        &format!("models/{new}.skinned_model/{rest}"),
                        &props,
                        &mut files,
                    )?;
                    continue;
                }
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
        // 0.69d0's face-wear: the face it shows, a surface of its model drawn
        // with the face's material, and an entry that draws no head.
        if let Some(face_part) = &item.face_part {
            let [(model, surface)] = &item.parts[..] else {
                bail!("{}: face-wear of more than one part", item.label);
            };
            let from = format!("models/{model}.skinned_model");
            let to = format!("models/{}.skinned_model", models[model]);
            out.raw(
                &format!("{to}/render/{FACE_WEAR_FACE}/converted_model"),
                face_part,
                &mut files,
            )?;
            let mut props = parse_config(
                view,
                &format!("{from}/render/{surface}/export_properties"),
                "surface properties",
            )?;
            let vertices = read_u32(face_part, 8)?;
            let indices = read_u32(face_part, 12 + read_u32(face_part, 4)? - 4 + 8)?;
            for (key, value) in [
                ("source.vertices", vertices),
                ("source.indices", indices),
                ("source.primitives", indices / 3),
            ] {
                set(&mut props, key, V::Integer(i32::try_from(value)?))?;
            }
            set(
                &mut props,
                "source.mesh_name",
                s(format!("|{}|{FACE_WEAR_FACE}", item.label)),
            )?;
            out.config(
                &format!("{to}/render/{FACE_WEAR_FACE}/export_properties"),
                &props,
                &mut files,
            )?;
            let mut settings = parse_config(view, &format!("{from}/settings"), "item settings")?;
            let Some(V::Named(materials)) = settings.root.named_mut("material_settings") else {
                bail!("{}: settings without material_settings", item.label);
            };
            let mut skin = materials.first().context("no material")?.clone();
            skin.name = FACE_WEAR_FACE.to_owned();
            skin.value.set_path(
                "material_name",
                s(format!("ported/{PORT}/{}", item.face.key)),
            )?;
            materials.push(skin);
            let Some(V::Indexed(lod0)) = settings
                .root
                .named_mut("lod_hierrarchy")
                .and_then(|l| l.named_mut("LOD0"))
                .and_then(|l| l.named_mut("surfaces"))
            else {
                bail!("{}: settings without LOD0 surfaces", item.label);
            };
            lod0.push(s(FACE_WEAR_FACE));
            out.config(&format!("{to}/settings"), &settings, &mut files)?;
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
        if item.face_part.is_some() {
            let model = models.values().next().context("no model")?.clone();
            entry.set_path("remove_head_model", V::Boolean(true))?;
            let face = named(vec![
                ("base_model", s(&model)),
                ("part_name", s(FACE_WEAR_FACE)),
            ]);
            match entry.named_mut("additional_models") {
                Some(V::Indexed(extra)) => extra.push(face),
                _ => entry.set_path("additional_models", V::Indexed(vec![face]))?,
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
