// SPDX-License-Identifier: GPL-3.0-or-later
//! Blackout's team armbands: the blue and the green band of S.T.A.L.K.E.R.
//! Clear Sky / Call of Pripyat multiplayer, one on each upper arm of the
//! native 0.26g player.
//!
//! A Blackout match has no interface: players tell the teams apart by the
//! band. Each team gets a model, `models/ported/blackout/band_<team>`, with
//! one surface, [`SURFACE`]; the game adds it to every player's visual as a
//! part (`base_model` [`Team::base_model`], `part_name` [`SURFACE`]). It is
//! no item: no config, dictionary row, icon or text.
//!
//! The donors are `meshes/actors/stalker_mp/mp_team_{blue,green}_1.ogf`
//! (the same bytes in Clear Sky and Call of Pripyat). Each draws its bands
//! as a child mesh of its own, on the `models\selflightl` shader, a strip of
//! 10-11 quads around each upper arm, weighted to `bip01_{l,r}_upperarm` with
//! a little clavicle and forearm. That strip's UVs run over the band in the
//! suit's texture, and over the ties and the jacket around it too: on a
//! native torso that would put a patch of S.T.A.L.K.E.R. jacket on the band.
//! So the port keeps what the donor says about the band (its place along
//! the upper arm, its width, where the emblem faces) and its cloth (only the
//! coloured band of the texture, centred on the emblem), and makes the ring
//! anew on the native arm:
//!
//! * as wide as the donor's band (as a fraction of the upper arm, `Arm` to
//!   `ForeArm`), lower on the arm than the donor's ([`Placement`]: the
//!   native armpit reaches further down), the emblem at the donor's angle;
//! * wound around the outer envelope of the sleeves of every torso the
//!   player can wear (stock and released ports, every glove class), as a
//!   cloth band is pulled tight: the convex hull of that envelope plus
//!   [`CLEARANCE`], then pushed out wherever a facet comes nearer than that.
//!   The native sleeves differ a lot (the sniper's ghillie sleeve, the
//!   Renaissance combat suit's shoulder plate, jackets cut fuller in front or
//!   behind), so a band over all of them stands off most of them;
//!   [`Placement::cover`] trades that for sinking into the bulkiest;
//! * rigid on the native upper-arm bone (the donor's is 95-100 % on it;
//!   taking the sleeves' own skin measured worse in the game's clips).
//!
//! The maps: the band's cloth cut from the team texture and resampled to
//! [`TEXTURE_SIZE`]; a flat normal map (the donor's bump,
//! `act_stalker_bandit_2_bump`, is the bandit suit's, with no band in it).
//! The material is the cloth one the other attachment ports use.
use crate::mask_fit::{
    Skin, bone_origins, cross, dot, farthest_hit, parse_mesh, read_u32, skins, sub,
};
use crate::outfit_026::{Finish, Writer};
use crate::soc::codec::{skeleton, skinned};
use crate::soc::source::SocSource;
use crate::weapon::parse_config;
use crate::{ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};
use std::fs;
use std::path::{Path, PathBuf};

pub const PORT: &str = "blackout";
/// The band's surface in both models: a part's `part_name`.
pub const SURFACE: &str = "band";
/// The donor's band shader.
const DONOR_SHADER: &str = "models\\selflightl";

/// One team's band.
pub struct Team {
    pub name: &'static str,
    /// The donor model, under `gamedata`.
    pub model: &'static str,
    /// The donor band's texture, as its mesh names it.
    pub texture: &'static str,
    /// The band's cloth in that 1024x1024 texture (x, y, width, height in
    /// texels): the coloured band inside its stitched edge, rows centred on
    /// the emblem (blue: band rows 52-251, emblem 124-167; green: band
    /// 40-195, emblem 100-137).
    pub cloth: [u32; 4],
}

impl Team {
    /// The model as a part names it: `models/<base_model>.skinned_model`.
    pub fn base_model(&self) -> String {
        format!("ported/{PORT}/band_{}", self.name)
    }

    /// The material instance (under `material_instances/`, no extension).
    pub fn material(&self) -> String {
        format!("ported/{PORT}/band_{}", self.name)
    }

    fn model_root(&self) -> String {
        format!("models/{}.skinned_model", self.base_model())
    }

    /// The emblem's centre as a donor UV `v`.
    fn emblem_v(&self) -> f64 {
        (f64::from(self.cloth[1]) + f64::from(self.cloth[3]) / 2.0) / 1024.0
    }
}

pub const TEAMS: [Team; 2] = [
    Team {
        name: "blue",
        model: "meshes/actors/stalker_mp/mp_team_blue_1.ogf",
        texture: "act\\act_mp_team_blue_1",
        cloth: [444, 56, 49, 180],
    },
    Team {
        name: "green",
        model: "meshes/actors/stalker_mp/mp_team_green_1.ogf",
        texture: "act\\act_mp_team_green_1",
        cloth: [502, 45, 32, 148],
    },
];

/// Each map: the band's width across, its length around the arm.
pub const TEXTURE_SIZE: [u32; 2] = [64, 256];
/// Where the band sits and how much it covers.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Placement {
    /// The band's lower edge, as a fraction of the native upper arm (`Arm`
    /// 0, `ForeArm` 1); it keeps the donors' width above that.
    pub distal: f64,
    /// At every angle and height, the quantile of the torsos' sleeve radii
    /// the band goes over: 1 is outside every torso; lower is snugger on
    /// most and sinks into the bulkiest.
    pub cover: f64,
}

impl Default for Placement {
    /// The donors' band sits at 0.37-0.59 of their upper arm, right under the
    /// deltoid. On the native rig that height is still the armpit in the bind
    /// pose: most jackets' sleeves join the body's side there, and the
    /// Renaissance combat suit's shoulder plate hangs to about 0.66, and in
    /// the game's clips the sleeve there moves with the chest. From about
    /// 0.65 down every sleeve is a tube of its own: the band sits at
    /// 0.65-0.88, clear of the plate and still 33 mm above the elbow joint.
    /// It goes over every torso.
    fn default() -> Self {
        Self {
            distal: 0.88,
            cover: 1.0,
        }
    }
}
/// Quads around each arm.
const SEGMENTS: usize = 32;
/// The least gap the band keeps over every sleeve in the bind pose (m).
pub const CLEARANCE: f64 = 0.005;
/// Sleeve surface farther than this from the arm's axis is not sleeve.
const MAX_RADIUS: f64 = 0.2;
/// How the band is measured against the sleeves: axial samples across its
/// width and angles around the arm.
const AXIAL_SAMPLES: usize = 14;
const ANGLE_SAMPLES: usize = 192;
/// A vertex is sleeve when at least this much of its skin is on the arm.
const SLEEVE_WEIGHT: f64 = 0.5;
/// Rounds of pushing facets out to the clearance.
const PUSH_ROUNDS: usize = 40;

/// The player's bind pose (`gameplay/bodyparts/default`'s
/// `body_parts.bind_pose`): the band's vertices are in it.
const BIND_POSE: &str =
    "models/character/human/scavengers/scavengers_01_torso_s1.skinned_model/render/bind_pose";
/// The native model whose settings, skeleton and properties the band's
/// start from.
const MODEL_TEMPLATE: &str =
    "models/character/human/scavengers/scavengers_01_torso_s1.skinned_model";
const SURFACE_TEMPLATE: &str = "scavengers_01_torso_s";
/// The two arms: native bone prefix, donor bone infix.
const SIDES: [(&str, &str); 2] = [("Left", "_l_"), ("Right", "_r_")];
const TORSO_KEYS: [&str; 5] = [
    "data",
    "data_full",
    "data_short",
    "data_medium",
    "data_long",
];

/// A bone axis with a frame around it: `u` the body's forward (+z) made
/// square to the axis, `v` = axis x `u`; angles run from `u` toward `v`.
#[derive(Clone, Copy, Debug)]
struct Arm {
    origin: [f64; 3],
    dir: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    length: f64,
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    a.map(|x| x / l)
}

impl Arm {
    fn new(shoulder: [f64; 3], elbow: [f64; 3]) -> Result<Self> {
        let d = sub(elbow, shoulder);
        let length = dot(d, d).sqrt();
        ensure!(length > 0.05, "upper arm of {length} m");
        let dir = d.map(|x| x / length);
        let forward = [0.0, 0.0, 1.0];
        let u = normalize(sub(forward, dir.map(|x| x * dot(dir, forward))));
        Ok(Self {
            origin: shoulder,
            dir,
            u,
            v: cross(dir, u),
            length,
        })
    }

    /// (along the axis, from it, angle).
    fn cylindrical(&self, p: [f64; 3]) -> (f64, f64, f64) {
        let w = sub(p, self.origin);
        let (x, y) = (dot(w, self.u), dot(w, self.v));
        (dot(w, self.dir), x.hypot(y), y.atan2(x))
    }

    fn radial(&self, angle: f64) -> [f64; 3] {
        let (s, c) = angle.sin_cos();
        std::array::from_fn(|k| self.u[k] * c + self.v[k] * s)
    }

    fn point(&self, s: f64, r: f64, angle: f64) -> [f64; 3] {
        let radial = self.radial(angle);
        std::array::from_fn(|k| self.origin[k] + self.dir[k] * s + radial[k] * r)
    }
}

/// An angle in (-pi, pi].
fn wrap(a: f64) -> f64 {
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

/// What the donor says about one arm's band.
#[derive(Clone, Debug, Serialize)]
pub struct DonorArm {
    pub side: String,
    pub vertices: usize,
    pub triangles: usize,
    /// Donor bones and the summed weight of the band's vertices on each.
    pub bones: BTreeMap<String, f64>,
    /// The band's edges as fractions of the upper arm (shoulder 0, elbow 1).
    pub proximal: f64,
    pub distal: f64,
    /// Across the band along the arm (m).
    pub width: f64,
    /// Around the arm, the mean of its two edges (m).
    pub circumference: f64,
    /// The angle the emblem faces ([`Arm`]'s frame, radians).
    pub emblem: f64,
    /// +1 when the texture's v grows as the angle falls.
    pub turn: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DonorBand {
    pub team: String,
    pub texture: String,
    pub shader: String,
    pub vertices: usize,
    pub triangles: usize,
    pub upper_arm: f64,
    pub arms: Vec<DonorArm>,
}

/// The band child of a team's donor model, measured on the donor's rest
/// pose.
fn donor_band(source: &mut SocSource, team: &Team) -> Result<DonorBand> {
    let bytes = source.read(team.model)?;
    let meshes = skinned::parse_meshes(&bytes, team.model)?;
    let rig = skeleton::parse(&bytes, team.model)?;
    let rest = rig.model_rest();
    let band = meshes
        .iter()
        .find(|m| m.shader == DONOR_SHADER && m.texture == team.texture)
        .with_context(|| format!("{}: no {DONOR_SHADER} band child", team.model))?;
    let mut arms = vec![];
    for (native, infix) in SIDES {
        let bone = |name: &str| -> Result<[f64; 3]> {
            let i = rig
                .index(name)
                .with_context(|| format!("{}: no {name}", team.model))?;
            Ok(rest[i].translation().map(f64::from))
        };
        let arm = Arm::new(
            bone(&format!("bip01{infix}upperarm"))?,
            bone(&format!("bip01{infix}forearm"))?,
        )?;
        // The arm a vertex is on: its heaviest bone's.
        let mine: Vec<usize> = (0..band.vertices.len())
            .filter(|&i| {
                let heaviest = band.vertices[i]
                    .bones
                    .iter()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|b| b.0);
                heaviest.is_some_and(|b| rig.bones[usize::from(b)].name.contains(infix))
            })
            .collect();
        ensure!(
            mine.len() >= 6,
            "{}: {} band vertices on the {native} arm",
            team.model,
            mine.len()
        );
        let mut bones = BTreeMap::new();
        for &i in &mine {
            for &(b, w) in &band.vertices[i].bones {
                if w > 1e-4 {
                    *bones
                        .entry(rig.bones[usize::from(b)].name.clone())
                        .or_insert(0.0) += f64::from(w);
                }
            }
        }
        let triangles = band
            .triangles
            .iter()
            .filter(|t| t.iter().all(|&i| mine.contains(&usize::from(i))))
            .count();
        let at = |i: usize| -> (f64, f64, f64) {
            arm.cylindrical(band.vertices[i].position.map(f64::from))
        };
        let mid = mine.iter().map(|&i| at(i).0).sum::<f64>() / mine.len() as f64;
        let (proximal, distal): (Vec<usize>, Vec<usize>) =
            mine.iter().partition(|&&i| at(i).0 < mid);
        let mean_s = |e: &[usize]| e.iter().map(|&i| at(i).0).sum::<f64>() / e.len() as f64;
        let perimeter = |e: &[usize]| {
            let mut ring: Vec<(f64, [f64; 3])> = vec![];
            for &i in e {
                let p = band.vertices[i].position.map(f64::from);
                if !ring
                    .iter()
                    .any(|(_, q)| dot(sub(p, *q), sub(p, *q)) < 1e-10)
                {
                    ring.push((at(i).2, p));
                }
            }
            ring.sort_by(|a, b| a.0.total_cmp(&b.0));
            (0..ring.len())
                .map(|k| {
                    let d = sub(ring[(k + 1) % ring.len()].1, ring[k].1);
                    dot(d, d).sqrt()
                })
                .sum::<f64>()
        };
        // The emblem: where along the ring the texture's v passes its
        // centre (the proximal edge, sorted by v).
        let mut by_v: Vec<(f64, f64)> = proximal
            .iter()
            .map(|&i| (f64::from(band.vertices[i].uv[1]), at(i).2))
            .collect();
        by_v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let target = team.emblem_v();
        let k = by_v
            .windows(2)
            .position(|w| w[0].0 <= target && target <= w[1].0)
            .with_context(|| format!("{}: the emblem's v {target} is off the band", team.model))?;
        let (lo, hi) = (by_v[k], by_v[k + 1]);
        let step = wrap(hi.1 - lo.1);
        let emblem = wrap(lo.1 + step * (target - lo.0) / (hi.0 - lo.0).max(1e-9));
        arms.push(DonorArm {
            side: native.to_owned(),
            vertices: mine.len(),
            triangles,
            bones,
            proximal: mean_s(&proximal) / arm.length,
            distal: mean_s(&distal) / arm.length,
            width: mean_s(&distal) - mean_s(&proximal),
            circumference: (perimeter(&proximal) + perimeter(&distal)) / 2.0,
            emblem,
            turn: -step.signum(),
        });
    }
    Ok(DonorBand {
        team: team.name.to_owned(),
        texture: band.texture.clone(),
        shader: band.shader.clone(),
        vertices: band.vertices.len(),
        triangles: band.triangles.len(),
        upper_arm: {
            let i = rig.index("bip01_l_upperarm").context("no upper arm")?;
            let j = rig.index("bip01_l_forearm").context("no forearm")?;
            let d = sub(
                rest[j].translation().map(f64::from),
                rest[i].translation().map(f64::from),
            );
            dot(d, d).sqrt()
        },
        arms,
    })
}

/// One surface of a torso model: its bind-pose vertices, skins, and per
/// side the triangles of its sleeve.
struct Surface {
    positions: Vec<[f64; 3]>,
    skins: Vec<Skin>,
    sleeve: [Vec<[usize; 3]>; 2],
    /// Every triangle.
    triangles: Vec<[usize; 3]>,
}

struct Torso {
    model: String,
    surfaces: Vec<Surface>,
    /// Arm bones (roll, shoulder) its own bind pose binds otherwise than
    /// the player's: the game skins every part with the player's.
    rebound: Vec<String>,
}

/// The bones of `used` whose frames differ between two bind poses of the
/// same bones.
fn differing_bones(a: &[u8], b: &[u8], used: &BTreeSet<usize>) -> Result<Vec<String>> {
    let (a, b) = (
        crate::armor::bind_pose_frames(a, "bind pose")?,
        crate::armor::bind_pose_frames(b, "player bind pose")?,
    );
    Ok(used
        .iter()
        .filter(|&&i| {
            a[i].1
                .iter()
                .zip(&b[i].1)
                .any(|(x, y)| (x - y).abs() >= 1e-4)
        })
        .map(|&i| a[i].0.clone())
        .collect())
}

/// Every model a torso can be drawn with: the torso items' (stock and the
/// released ports, every glove class) and the bare default.
fn torso_models(view: &mut ResourceDb) -> Result<BTreeSet<String>> {
    let configs: Vec<String> = view
        .paths_with_prefix("gameplay/items/armour/torso/")
        .filter(|p| p.ends_with(".options"))
        .map(str::to_owned)
        .collect();
    let mut models = BTreeSet::new();
    let mut take = |v: &V| {
        if let Some(V::String(m)) = v.named("base_model") {
            models.insert(m.clone());
        }
    };
    for path in &configs {
        let c = parse_config(view, path, "torso")?;
        for key in TORSO_KEYS {
            if let Some(data) = c.root.named(key) {
                take(data);
                if let Some(V::Indexed(extra)) = data.named("additional_models") {
                    extra.iter().for_each(&mut take);
                }
            }
        }
    }
    let defaults = parse_config(view, "gameplay/bodyparts/default", "body parts")?;
    if let Some(V::Named(classes)) = defaults
        .root
        .named("body_parts")
        .and_then(|b| b.named("torso"))
    {
        classes.iter().for_each(|c| take(&c.value));
    }
    ensure!(models.len() > 10, "only {} torso models", models.len());
    Ok(models)
}

/// The sleeves of `model`: triangles whose every corner is mostly on the
/// side's arm bones.
fn torso(
    view: &mut ResourceDb,
    model: &str,
    arm_bones: &[BTreeSet<u8>; 2],
    bind_pose: &[u8],
    axes: &BTreeSet<usize>,
    used: &BTreeSet<usize>,
) -> Result<Torso> {
    let root = format!("models/{model}.skinned_model/render/");
    let own = view.read(&format!("{root}bind_pose"))?;
    // The same bones, and the arms' axes where the player's are.
    crate::face_items_069d::same_rig_for(&own, bind_pose, model, Some(axes))?;
    let rebound = differing_bones(&own, bind_pose, used)?;
    let paths: Vec<String> = view
        .paths_with_prefix(&root)
        .filter(|p| p.ends_with("/converted_model"))
        // The sniper's ghillie fringes (`edge_sniper_06_alpha_s`): cut-out
        // strands hanging loose around the arm, no sleeve to go over.
        .filter(|p| !p.contains("_alpha_s/"))
        .map(str::to_owned)
        .collect();
    ensure!(!paths.is_empty(), "{model} has no surfaces");
    let mut surfaces = vec![];
    for path in paths {
        let b = view.read(&path)?;
        let count = read_u32(&b, 8)?;
        ensure!(count > 0, "{path}: empty");
        let stride = (read_u32(&b, 4)? - 4) / count;
        let mesh = parse_mesh(&b, stride, &path)?;
        let skin = skins(&b, stride)?;
        let on = |side: usize, v: usize| -> bool {
            skin[v]
                .iter()
                .filter(|(bone, _)| arm_bones[side].contains(bone))
                .map(|(_, w)| w)
                .sum::<f64>()
                >= SLEEVE_WEIGHT
        };
        let sleeve = [0, 1].map(|side| {
            mesh.triangles
                .iter()
                .copied()
                .filter(|t| t.iter().all(|&v| on(side, v)))
                .collect()
        });
        surfaces.push(Surface {
            positions: mesh.positions,
            skins: skin,
            sleeve,
            triangles: mesh.triangles,
        });
    }
    Ok(Torso {
        model: model.to_owned(),
        surfaces,
        rebound,
    })
}

/// The farthest sleeve surface on a ray from the axis (m), if any.
///
/// Where no sleeve triangle is on the ray (the armpit side, which some
/// torsos weight mostly to the chest), the first surface the ray meets from
/// inside the arm: the sleeve, whatever it is weighted to, before the body.
fn sleeve_radius(torso: &Torso, side: usize, arm: &Arm, s: f64, angle: f64) -> Option<f64> {
    let origin = arm.point(s, 0.0, 0.0);
    let dir = arm.radial(angle);
    let best = |a: Option<f64>, t: f64, far: bool| {
        Some(a.map_or(t, |b| if far { b.max(t) } else { b.min(t) }))
    };
    let sleeve = torso
        .surfaces
        .iter()
        .filter_map(|surface| farthest_hit(origin, dir, &surface.positions, &surface.sleeve[side]))
        .map(|hit| hit.0)
        .filter(|&t| t <= MAX_RADIUS)
        .fold(None, |a, t| best(a, t, true));
    sleeve.or_else(|| {
        torso
            .surfaces
            .iter()
            .flat_map(|surface| {
                crate::mask_fit::ray_hits(origin, dir, &surface.positions, &surface.triangles)
            })
            .map(|hit| hit.0)
            .filter(|&t| t > 0.0 && t <= MAX_RADIUS)
            .fold(None, |a, t| best(a, t, false))
    })
}

/// The axial samples across the band and the angle samples around it.
fn samples(proximal: f64, distal: f64) -> (Vec<f64>, Vec<f64>) {
    let s = (0..AXIAL_SAMPLES)
        .map(|i| proximal + (distal - proximal) * (i as f64 + 0.5) / AXIAL_SAMPLES as f64)
        .collect();
    let a = (0..ANGLE_SAMPLES)
        .map(|j| -PI + TAU * j as f64 / ANGLE_SAMPLES as f64)
        .collect();
    (s, a)
}

/// The radius at `angle` of the convex hull of the polar points
/// `(angles[j], radii[j])` (the origin inside it).
fn hull_radius(angles: &[f64], radii: &[f64], angle: f64) -> f64 {
    let mut pts: Vec<[f64; 2]> = angles
        .iter()
        .zip(radii)
        .map(|(&a, &r)| [r * a.cos(), r * a.sin()])
        .collect();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f64; 2]> = vec![];
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    // The ray from the origin at `angle` leaves the hull through one edge.
    let d = [angle.cos(), angle.sin()];
    let mut best = 0.0_f64;
    for k in 0..hull.len() {
        let (a, b) = (hull[k], hull[(k + 1) % hull.len()]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let den = d[0] * e[1] - d[1] * e[0];
        if den.abs() < 1e-12 {
            continue;
        }
        let t = (a[0] * e[1] - a[1] * e[0]) / den;
        let w = (a[0] * d[1] - a[1] * d[0]) / den;
        if t > 0.0 && (-1e-9..=1.0 + 1e-9).contains(&w) {
            best = best.max(t);
        }
    }
    best
}

/// One arm's band as built: its frame, vertices (bind pose) and UVs.
struct BandArm {
    arm: Arm,
    positions: Vec<[f64; 3]>,
    /// Each vertex's bones and weights.
    skins: Vec<Skin>,
    uvs: Vec<[f64; 2]>,
    triangles: Vec<[usize; 3]>,
}

/// The radius the band goes over at one sample: the `q` quantile of the
/// torsos' sleeve radii there (0 with none).
fn cover(mut radii: Vec<f64>, q: f64) -> f64 {
    if radii.is_empty() {
        return 0.0;
    }
    radii.sort_by(f64::total_cmp);
    radii[((radii.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize]
}

/// The ring's vertices: column `k` of [`SEGMENTS`] + 1 (the last is the
/// seam's copy of the first), proximal then distal. `v` runs around the arm
/// so that 0.5 (the emblem, centred in the cut cloth) faces `emblem`, the
/// seam opposite; `u` runs across, 0 at the shoulder's edge.
fn ring_layout(emblem: f64, turn: f64) -> Vec<(f64, [f64; 2], usize)> {
    let mut out = vec![];
    for k in 0..=SEGMENTS {
        let v = k as f64 / SEGMENTS as f64;
        let angle = wrap(emblem - turn * TAU * (v - 0.5));
        for edge in 0..2 {
            out.push((angle, [edge as f64, v], edge));
        }
    }
    out
}

/// Triangles of the ring, wound so the face normal, by `front`'s sign
/// (+1: `(b - a) x (c - a)` points out of a front face), points away from
/// the arm.
fn ring_triangles(positions: &[[f64; 3]], arm: &Arm, front: f64) -> Vec<[usize; 3]> {
    let mut out = vec![];
    for k in 0..SEGMENTS {
        let (p0, d0, p1, d1) = (2 * k, 2 * k + 1, 2 * k + 2, 2 * k + 3);
        for mut t in [[p0, d0, d1], [p0, d1, p1]] {
            let [a, b, c] = t.map(|i| positions[i]);
            let n = cross(sub(b, a), sub(c, a));
            let centre: [f64; 3] = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0);
            let (s, _, _) = arm.cylindrical(centre);
            let outward = sub(centre, arm.point(s, 0.0, 0.0));
            if dot(n, outward) * front < 0.0 {
                t.swap(1, 2);
            }
            out.push(t);
        }
    }
    out
}

/// Which way stock skinned surfaces wind a front face: the sign of
/// `(b - a) x (c - a)` against the stored vertex normals.
fn front_face_sign(model: &[u8]) -> Result<f64> {
    let mesh = parse_mesh(model, 48, "template surface")?;
    let normal = |i: usize| -> [f64; 3] {
        let at = 12 + i * 48 + 28;
        let b = &model[at..at + 4];
        [b[2], b[1], b[0]].map(|c| f64::from(c) / 127.5 - 1.0)
    };
    let mut score = 0.0;
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i]);
        let n = cross(sub(b, a), sub(c, a));
        let stored: [f64; 3] =
            std::array::from_fn(|k| t.iter().map(|&i| normal(i)[k]).sum::<f64>());
        score += dot(n, stored).signum();
    }
    ensure!(
        score.abs() > mesh.triangles.len() as f64 * 0.8,
        "the template's winding is not consistent ({score} of {})",
        mesh.triangles.len()
    );
    Ok(score.signum())
}

/// A 48-byte four-weight converted surface of the bands: rigid on each
/// arm's bone, normals from the faces, tangent along `u`, binormal along `v`.
fn encode(arms: &[BandArm]) -> Result<Vec<u8>> {
    let count: usize = arms.iter().map(|a| a.positions.len()).sum();
    let mut out = vec![];
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + count * 48)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(count)?.to_le_bytes());
    let mut indices = vec![];
    let mut base = 0;
    for band in arms {
        let mut normals = vec![[0.0; 3]; band.positions.len()];
        let mut tangents = vec![[0.0; 3]; band.positions.len()];
        let mut binormals = vec![[0.0; 3]; band.positions.len()];
        for t in &band.triangles {
            let [a, b, c] = t.map(|i| band.positions[i]);
            let [ua, ub, uc] = t.map(|i| band.uvs[i]);
            let (e1, e2) = (sub(b, a), sub(c, a));
            let n = cross(e1, e2);
            let (du1, dv1, du2, dv2) = (ub[0] - ua[0], ub[1] - ua[1], uc[0] - ua[0], uc[1] - ua[1]);
            let det = du1 * dv2 - du2 * dv1;
            ensure!(det.abs() > 1e-12, "a band triangle has no UV area");
            let tangent: [f64; 3] = std::array::from_fn(|k| (e1[k] * dv2 - e2[k] * dv1) / det);
            let binormal: [f64; 3] = std::array::from_fn(|k| (e2[k] * du1 - e1[k] * du2) / det);
            for &i in t {
                for k in 0..3 {
                    normals[i][k] += n[k];
                    tangents[i][k] += tangent[k];
                    binormals[i][k] += binormal[k];
                }
            }
        }
        for (i, &p) in band.positions.iter().enumerate() {
            // Out from the arm, whichever way the faces wind.
            let (s, _, _) = band.arm.cylindrical(p);
            let mut n = normalize(normals[i]);
            if dot(n, sub(p, band.arm.point(s, 0.0, 0.0))) < 0.0 {
                n = n.map(|x| -x);
            }
            let ortho = |x: [f64; 3]| normalize(sub(x, n.map(|c| c * dot(x, n))));
            let (t, b) = (ortho(tangents[i]), ortho(binormals[i]));
            for x in p {
                out.extend_from_slice(&(x as f32).to_le_bytes());
            }
            // The four heaviest bones, renormalised; empty slots repeat the
            // first bone at no weight.
            let mut skin = band.skins[i].clone();
            skin.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            skin.truncate(4);
            let total: f64 = skin.iter().map(|s| s.1).sum();
            ensure!(total > 0.0, "a band vertex has no skin");
            let slot = |k: usize| skin.get(k).copied().unwrap_or((skin[0].0, 0.0));
            out.extend_from_slice(&[0, 1, 2, 3].map(|k| slot(k).0));
            for k in 0..3 {
                out.extend_from_slice(&((slot(k).1 / total) as f32).to_le_bytes());
            }
            let uv = band.uvs[i].map(|x| x as f32);
            let f = |x: [f64; 3]| x.map(|c| c as f32);
            out.extend_from_slice(&crate::armor::packed_basis(f(n), 127).to_le_bytes());
            out.extend_from_slice(
                &crate::armor::packed_basis(f(t), crate::armor::quantized_uv_fraction(uv[0]))
                    .to_le_bytes(),
            );
            out.extend_from_slice(
                &crate::armor::packed_basis(f(b), crate::armor::quantized_uv_fraction(uv[1]))
                    .to_le_bytes(),
            );
            for x in uv {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        for t in &band.triangles {
            for &i in t {
                indices.push(u16::try_from(base + i)?);
            }
        }
        base += band.positions.len();
    }
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + indices.len() * 2)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(indices.len())?.to_le_bytes());
    for i in indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    Ok(out)
}

/// The band's gaps over one torso's sleeve, measured on the sample grid.
#[derive(Clone, Debug, Serialize)]
pub struct TorsoGap {
    pub model: String,
    /// Least and largest gap between the band and the sleeve under it (m).
    pub least: f64,
    pub largest: f64,
    pub mean: f64,
    /// Samples of the grid with sleeve under them, of all.
    pub covered: usize,
    pub samples: usize,
}

/// The worst sampled pose of a clip.
#[derive(Clone, Debug, Serialize)]
pub struct PoseGap {
    pub clip: String,
    /// The least gap between a sleeve vertex under the band and the band (m);
    /// negative where the sleeve comes through.
    pub least: f64,
    pub torso: String,
    pub side: String,
    /// Where that sleeve vertex is in the bind pose: along the upper arm
    /// (0 shoulder, 1 elbow) and around it (degrees, the arm frame).
    pub at: f64,
    pub angle: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub placement: Placement,
    pub donors: Vec<DonorBand>,
    /// The native upper arm (Arm to ForeArm), m.
    pub upper_arm: f64,
    pub proximal: f64,
    pub distal: f64,
    /// Across the band, m.
    pub width: f64,
    /// Around each arm (left, right), m, at the middle of the band.
    pub circumference: [f64; 2],
    /// The band's middle below the shoulder joint (Arm bone), m.
    pub below_shoulder: f64,
    pub emblem_degrees: [f64; 2],
    pub vertices: usize,
    pub triangles: usize,
    pub push_rounds: usize,
    pub torsos: Vec<TorsoGap>,
    pub poses: Vec<PoseGap>,
}

/// Builds the two band models, their materials and maps into
/// `output/overlay` with `output/port-manifest.json` (a bundle for
/// `pack from-bundle`) and `output/report.json`.
///
/// `source`: a Clear Sky or Call of Pripyat installation (or unpacked
/// gamedata); `patches`: the released `resources.db.patch_<n>`, whose
/// torsos the band is fitted over too.
pub fn build(
    source: &Path,
    db: &Path,
    patches: &[PathBuf],
    placement: Placement,
    output: &Path,
) -> Result<Report> {
    ensure!(
        (0.3..=0.97).contains(&placement.distal) && (0.0..=1.0).contains(&placement.cover),
        "placement {placement:?} is off the upper arm"
    );
    crate::weapon::prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut source = SocSource::open(source)?;
    let donors = TEAMS
        .iter()
        .map(|t| donor_band(&mut source, t))
        .collect::<Result<Vec<_>>>()?;
    for d in &donors {
        println!(
            "Donor {} band: {} vertices, {} triangles ({}, {}), upper arm {:.3} m",
            d.team, d.vertices, d.triangles, d.texture, d.shader, d.upper_arm
        );
        for a in &d.arms {
            println!(
                "  {}: {} vertices, {} triangles, {:.2}-{:.2} of the upper arm, {:.1} mm wide, {:.0} mm around, emblem at {:.0} deg; weights {:?}",
                a.side,
                a.vertices,
                a.triangles,
                a.proximal,
                a.distal,
                a.width * 1000.0,
                a.circumference * 1000.0,
                a.emblem.to_degrees(),
                a.bones
            );
        }
    }

    let mut view = crate::novice_026::game_view(db, patches)?;
    let bind_pose = view.read(BIND_POSE)?;
    let names: Vec<String> = crate::armor::bind_pose_origins(&bind_pose, "player bind pose")?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let index = |n: &str| -> Result<usize> {
        names
            .iter()
            .position(|b| b == n)
            .with_context(|| format!("no bone {n}"))
    };
    let origins = bone_origins(&bind_pose)?;
    let mut arm_bones: [BTreeSet<u8>; 2] = Default::default();
    let (mut used, mut axes) = (BTreeSet::new(), BTreeSet::new());
    let mut arms = vec![];
    for (k, (side, _)) in SIDES.iter().enumerate() {
        for b in ["Arm", "Arm_Roll", "ForeArm", "ForeArm_Roll", "Shoulder"] {
            let i = index(&format!("{side}{b}"))?;
            arm_bones[k].insert(u8::try_from(i)?);
            used.insert(i);
            if b == "Arm" || b == "ForeArm" {
                axes.insert(i);
            }
        }
        arms.push(Arm::new(
            origins[&format!("{side}Arm")],
            origins[&format!("{side}ForeArm")],
        )?);
    }
    // The clips are sampled on the head's rig: it must bind the arms as the
    // player's pose does.
    crate::face_items_069d::same_rig_for(
        &view.read(
            "models/character/human/base/base_character_head.skinned_model/render/bind_pose",
        )?,
        &bind_pose,
        "head bind pose",
        Some(&used),
    )?;

    let models = torso_models(&mut view)?;
    let torsos = models
        .iter()
        .map(|m| torso(&mut view, m, &arm_bones, &bind_pose, &axes, &used))
        .collect::<Result<Vec<_>>>()?;
    println!("Fitting over {} torso models", torsos.len());
    for t in torsos.iter().filter(|t| !t.rebound.is_empty()) {
        println!(
            "  {} binds {:?} otherwise than the player's bind pose",
            t.model, t.rebound
        );
    }

    // Where the band goes: as wide as the donors' (as a fraction of the
    // upper arm), its distal edge at the placement's; the emblem where
    // theirs faces.
    let mean = |f: &dyn Fn(&DonorArm) -> f64| {
        let all: Vec<f64> = donors.iter().flat_map(|d| d.arms.iter().map(f)).collect();
        all.iter().sum::<f64>() / all.len() as f64
    };
    let (donor_proximal, donor_distal) = (mean(&|a| a.proximal), mean(&|a| a.distal));
    let distal = placement.distal;
    let proximal = distal - (donor_distal - donor_proximal);
    println!(
        "Band at {proximal:.2}-{distal:.2} of the native upper arm (the donors' at {donor_proximal:.2}-{donor_distal:.2})"
    );
    let mut flips = vec![];
    for d in &donors {
        let flip = d.arms[0].turn != donors[0].arms[0].turn;
        ensure!(
            d.arms
                .iter()
                .zip(&donors[0].arms)
                .all(|(a, b)| (a.turn != b.turn) == flip),
            "the {} donor turns its texture one way on one arm and the other on the other",
            d.team
        );
        flips.push(flip);
    }
    let mut band_arms = vec![];
    let mut grids = vec![];
    let mut emblem_degrees = [0.0; 2];
    for (k, arm) in arms.iter().enumerate() {
        let per_team: Vec<&DonorArm> = donors.iter().map(|d| &d.arms[k]).collect();
        // The first team's way around; a team whose donor runs its texture
        // the other way gets its cloth flipped (`flips`), so both show as
        // their donors do on one geometry.
        let turn = per_team[0].turn;
        let (sx, sy) = per_team.iter().fold((0.0, 0.0), |(x, y), a| {
            (x + a.emblem.cos(), y + a.emblem.sin())
        });
        let emblem = sy.atan2(sx);
        emblem_degrees[k] = emblem.to_degrees();
        let (s0, s1) = (proximal * arm.length, distal * arm.length);
        let (axial, angles) = samples(s0, s1);
        // Every torso's sleeve radius on the grid.
        let grid: Vec<Vec<Vec<Option<f64>>>> = torsos
            .iter()
            .map(|t| {
                axial
                    .iter()
                    .map(|&s| {
                        angles
                            .iter()
                            .map(|&a| sleeve_radius(t, k, arm, s, a))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let envelope: Vec<Vec<f64>> = (0..axial.len())
            .map(|i| {
                (0..angles.len())
                    .map(|j| {
                        cover(
                            grid.iter().filter_map(|g| g[i][j]).collect(),
                            placement.cover,
                        )
                    })
                    .collect()
            })
            .collect();
        let uncovered = envelope.iter().flatten().filter(|&&r| r == 0.0).count();
        ensure!(
            uncovered == 0,
            "{uncovered} samples of the {} arm have no sleeve of any torso",
            SIDES[k].0
        );
        // Each edge pulled tight over its half of the band.
        let half = axial.len() / 2;
        let edge_hull = |rows: std::ops::Range<usize>, angle: f64| -> f64 {
            let radii: Vec<f64> = (0..angles.len())
                .map(|j| rows.clone().map(|i| envelope[i][j]).fold(0.0, f64::max) + CLEARANCE)
                .collect();
            hull_radius(&angles, &radii, angle)
        };
        let mut positions = vec![];
        let mut uvs = vec![];
        for (angle, uv, edge) in ring_layout(emblem, turn) {
            let (s, rows) = if edge == 0 {
                (s0, 0..half)
            } else {
                (s1, half..axial.len())
            };
            positions.push(arm.point(s, edge_hull(rows, angle), angle));
            uvs.push(uv);
        }
        let bone = u8::try_from(index(&format!("{}Arm", SIDES[k].0))?)?;
        band_arms.push(BandArm {
            arm: *arm,
            skins: vec![vec![(bone, 1.0)]; positions.len()],
            positions,
            uvs,
            triangles: vec![],
        });
        grids.push((axial, angles, grid, envelope));
    }
    let template_surface = view.read(&format!(
        "{MODEL_TEMPLATE}/render/{SURFACE_TEMPLATE}/converted_model"
    ))?;
    let front = front_face_sign(&template_surface)?;
    for band in &mut band_arms {
        band.triangles = ring_triangles(&band.positions, &band.arm, front);
    }

    // Push facets out wherever one comes nearer than the clearance.
    let mut push_rounds = 0;
    for round in 0..PUSH_ROUNDS {
        let mut short_anywhere = false;
        for (band, (axial, angles, _, envelope)) in band_arms.iter_mut().zip(&grids) {
            let mut need = vec![0.0_f64; band.positions.len()];
            for (i, &s) in axial.iter().enumerate() {
                for (j, &a) in angles.iter().enumerate() {
                    let origin = band.arm.point(s, 0.0, 0.0);
                    let hit =
                        farthest_hit(origin, band.arm.radial(a), &band.positions, &band.triangles)
                            .context("a ray from the arm misses the band")?;
                    let short = envelope[i][j] + CLEARANCE - hit.0;
                    if short > 1e-5 {
                        for &c in &band.triangles[hit.1] {
                            need[c] = need[c].max(short + 1e-4);
                        }
                    }
                }
            }
            for (p, n) in band.positions.iter_mut().zip(need) {
                if n > 0.0 {
                    short_anywhere = true;
                    let (s, r, a) = band.arm.cylindrical(*p);
                    *p = band.arm.point(s, r + n, a);
                }
            }
        }
        if !short_anywhere {
            break;
        }
        push_rounds = round + 1;
    }
    ensure!(
        push_rounds < PUSH_ROUNDS,
        "the band does not clear the sleeves after {PUSH_ROUNDS} rounds"
    );

    // The gaps over each torso.
    let mut gaps = vec![];
    for (t, torso) in torsos.iter().enumerate() {
        let mut g = TorsoGap {
            model: torso.model.clone(),
            least: f64::INFINITY,
            largest: 0.0,
            mean: 0.0,
            covered: 0,
            samples: 0,
        };
        for (band, (axial, angles, grid, _)) in band_arms.iter().zip(&grids) {
            for (i, &s) in axial.iter().enumerate() {
                for (j, &a) in angles.iter().enumerate() {
                    g.samples += 1;
                    let Some(r) = grid[t][i][j] else { continue };
                    let hit = farthest_hit(
                        band.arm.point(s, 0.0, 0.0),
                        band.arm.radial(a),
                        &band.positions,
                        &band.triangles,
                    )
                    .context("a ray from the arm misses the band")?;
                    let gap = hit.0 - r;
                    g.covered += 1;
                    g.least = g.least.min(gap);
                    g.largest = g.largest.max(gap);
                    g.mean += gap;
                }
            }
        }
        g.mean /= g.covered.max(1) as f64;
        gaps.push(g);
    }
    // Models with no sleeve under the band at all are no torsos (the Duty
    // and Freedom suits draw their trousers as a torso item's extra model).
    for g in gaps.iter().filter(|g| g.covered == 0) {
        println!("  {}: no sleeve under the band (not a torso)", g.model);
    }
    gaps.retain(|g| g.covered > 0);
    gaps.sort_by(|a, b| a.mean.total_cmp(&b.mean));
    let least = gaps.iter().map(|g| g.least).fold(f64::INFINITY, f64::min);
    if placement.cover >= 1.0 {
        ensure!(
            least >= CLEARANCE - 2e-4,
            "the band comes {:.2} mm near a sleeve",
            least * 1000.0
        );
    }

    // The game's clips.
    let poses = pose_gaps(&mut view, &band_arms, &torsos, proximal, distal)?;

    let surface = encode(&band_arms)?;
    let report = Report {
        donors,
        placement,
        upper_arm: arms[0].length,
        proximal,
        distal,
        width: (distal - proximal) * arms[0].length,
        circumference: [0, 1].map(|k| {
            let b = &band_arms[k];
            // The ring halfway across: the mean of each column's two edges.
            let mid: Vec<[f64; 3]> = (0..SEGMENTS)
                .map(|c| {
                    std::array::from_fn(|x| {
                        (b.positions[2 * c][x] + b.positions[2 * c + 1][x]) / 2.0
                    })
                })
                .collect();
            (0..SEGMENTS)
                .map(|c| {
                    let d = sub(mid[(c + 1) % SEGMENTS], mid[c]);
                    dot(d, d).sqrt()
                })
                .sum()
        }),
        below_shoulder: (proximal + distal) / 2.0 * arms[0].length,
        emblem_degrees,
        vertices: band_arms.iter().map(|b| b.positions.len()).sum(),
        triangles: band_arms.iter().map(|b| b.triangles.len()).sum(),
        push_rounds,
        torsos: gaps,
        poses,
    };

    // --- The models, materials and maps ---
    let out = Writer {
        overlay: &overlay,
        tag: "Blackout-bands",
    };
    let mut files = vec![];
    let maps = output.join("maps");
    fs::create_dir_all(&maps)?;
    for (team, flip) in TEAMS.iter().zip(flips) {
        let stem = maps.join(format!("band_{}", team.name));
        cut_cloth(&mut source, team, flip, &stem)?;
        flat_normal_tga(
            &PathBuf::from(format!("{}_nmap.tga", stem.display())),
            TEXTURE_SIZE,
        )?;
        out.cloth_material_cropped(
            &mut view,
            PORT,
            &format!("band_{}", team.name),
            &stem,
            None,
            Finish::CLOTH,
            &mut files,
        )?;
        write_model(&out, &mut view, team, &surface, &bind_pose, &mut files)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "blackout-bands",
            "description": "Blackout team armbands from S.T.A.L.K.E.R. Clear Sky / Call of Pripyat multiplayer",
            "files": files,
        }))?,
    )?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    Ok(report)
}

/// The band's cloth from the team's texture, resampled to [`TEXTURE_SIZE`]
/// (with `flip`, upside down: its way around the arm reversed), as
/// `<stem>.tga`.
fn cut_cloth(source: &mut SocSource, team: &Team, flip: bool, stem: &Path) -> Result<()> {
    let name = format!("textures/{}.dds", team.texture.replace('\\', "/"));
    let bytes = source.read(&name)?;
    let info = crate::soc::source::parse_dds(&bytes, &name)?;
    ensure!(
        info.width == 1024 && info.height == 1024,
        "{name} is {}x{}, not 1024x1024",
        info.width,
        info.height
    );
    let dds = stem.with_extension("source.dds");
    fs::write(&dds, &bytes)?;
    let [x, y, w, h] = team.cloth;
    let tga = stem.with_extension("tga");
    let result = std::process::Command::new("magick")
        .arg(&dds)
        .args([
            "-crop",
            &format!("{w}x{h}+{x}+{y}"),
            "+repage",
            "-alpha",
            "off",
        ])
        .args(flip.then_some("-flip"))
        .args([
            "-filter",
            "Lanczos",
            "-resize",
            &format!("{}x{}!", TEXTURE_SIZE[0], TEXTURE_SIZE[1]),
        ])
        .args(["-type", "TrueColor", "-compress", "None"])
        .arg(&tga)
        .output()
        .context("starting ImageMagick; enter the repository Nix shell")?;
    fs::remove_file(&dds)?;
    ensure!(
        result.status.success(),
        "ImageMagick could not cut {name}: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    );
    ensure!(
        crate::outfit_026::tga_size(&tga)? == TEXTURE_SIZE,
        "{} is not {TEXTURE_SIZE:?}",
        tga.display()
    );
    // The cut must be the team's colour.
    let mean = tga_mean(&tga)?;
    let (main, others) = match team.name {
        "blue" => (mean[2], [mean[0], mean[1]]),
        "green" => (mean[1], [mean[0], mean[2]]),
        other => bail!("no colour check for team {other}"),
    };
    ensure!(
        others.iter().all(|&o| main > o + 25.0),
        "the {} cut is not {} (mean RGB {mean:?})",
        team.name,
        team.name
    );
    Ok(())
}

/// Mean RGB of an uncompressed 24-bit TGA.
fn tga_mean(path: &Path) -> Result<[f64; 3]> {
    let b = fs::read(path)?;
    ensure!(
        b.len() > 18 && b[2] == 2 && b[16] == 24,
        "{} is not a 24-bit TGA",
        path.display()
    );
    let pixels = &b[18 + usize::from(b[0])..];
    let n = (pixels.len() / 3) as f64;
    let mut sum = [0.0; 3];
    for p in pixels.chunks_exact(3) {
        // BGR.
        sum[0] += f64::from(p[2]);
        sum[1] += f64::from(p[1]);
        sum[2] += f64::from(p[0]);
    }
    Ok(sum.map(|s| s / n))
}

/// A flat tangent-space normal map (0, 0, 1) as a 24-bit TGA.
fn flat_normal_tga(path: &Path, size: [u32; 2]) -> Result<()> {
    let mut b = vec![0_u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&u16::try_from(size[0])?.to_le_bytes());
    b.extend_from_slice(&u16::try_from(size[1])?.to_le_bytes());
    b.extend_from_slice(&[24, 0x20]);
    for _ in 0..size[0] * size[1] {
        b.extend_from_slice(&[255, 128, 128]);
    }
    fs::write(path, b)?;
    Ok(())
}

/// A team's model folder: the player's bind pose, the template's skeleton,
/// settings naming [`SURFACE`] and the team's material, and properties.
fn write_model(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    team: &Team,
    surface: &[u8],
    bind_pose: &[u8],
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let root = team.model_root();
    out.raw(&format!("{root}/render/bind_pose"), bind_pose, files)?;
    out.raw(
        &format!("{root}/skeleton"),
        &view.read(&format!("{MODEL_TEMPLATE}/skeleton"))?,
        files,
    )?;
    let mut settings = parse_config(view, &format!("{MODEL_TEMPLATE}/settings"), "settings")?;
    let Some(V::Named(materials)) = settings.root.named_mut("material_settings") else {
        bail!("{MODEL_TEMPLATE} settings without material_settings");
    };
    ensure!(
        materials.len() == 1 && materials[0].name == SURFACE_TEMPLATE,
        "{MODEL_TEMPLATE} has other surfaces"
    );
    materials[0].name = SURFACE.to_owned();
    settings.root.set_path(
        &format!("material_settings.{SURFACE}.material_name"),
        V::String(team.material()),
    )?;
    settings.root.set_path(
        "lod_hierrarchy.LOD0.surfaces",
        V::Indexed(vec![V::String(SURFACE.to_owned())]),
    )?;
    out.config(&format!("{root}/settings"), &settings, files)?;

    let (min, max) = crate::outfit_026::converted_bounds(surface)?;
    let vertices = read_u32(surface, 8)?;
    let indices = read_u32(surface, 12 + vertices * 48 + 8)?;
    out.raw(
        &format!("{root}/render/{SURFACE}/converted_model"),
        surface,
        files,
    )?;
    let mut props = parse_config(
        view,
        &format!("{MODEL_TEMPLATE}/render/{SURFACE_TEMPLATE}/export_properties"),
        "surface properties",
    )?;
    for (key, value) in [
        ("source.vertices", vertices),
        ("source.indices", indices),
        ("source.primitives", indices / 3),
    ] {
        props
            .root
            .set_path(key, V::Integer(i32::try_from(value)?))?;
    }
    // 43 + bones a vertex.
    props.root.set_path("type", V::Integer(47))?;
    props.root.set_path(
        "source.mesh_name",
        V::String(format!("|mp_team_{}_1|{SURFACE}", team.name)),
    )?;
    props
        .root
        .set_path("source.texture_name", V::String(team.material()))?;
    props.root.set_path("bounding_box.min", V::Float3(min))?;
    props.root.set_path("bounding_box.max", V::Float3(max))?;
    out.config(
        &format!("{root}/render/{SURFACE}/export_properties"),
        &props,
        files,
    )?;
    let mut render = parse_config(
        view,
        &format!("{MODEL_TEMPLATE}/render/export_properties"),
        "render properties",
    )?;
    render.root.set_path(
        "src_filename",
        V::String(format!("S.T.A.L.K.E.R. CS-CoP/{}", team.model)),
    )?;
    render.root.set_path("bounding_box.min", V::Float3(min))?;
    render.root.set_path("bounding_box.max", V::Float3(max))?;
    out.config(&format!("{root}/render/export_properties"), &render, files)
}

/// The least gap between the band and the sleeve under it in the worst
/// sampled pose of each clip of [`crate::bandit_026::POSE_CLIPS`] (lobby
/// idles, match stand, aim, run, sprint, crouch): band and sleeve skinned
/// through it, measured on rays from the posed upper-arm axis through every
/// sleeve vertex under the band.
fn pose_gaps(
    view: &mut ResourceDb,
    bands: &[BandArm],
    torsos: &[Torso],
    proximal: f64,
    distal: f64,
) -> Result<Vec<PoseGap>> {
    let poses = crate::bandit_026::SampledPoses::read(view)?;
    let names = crate::armor::bind_pose_origins(&poses.bind_pose, "head bind pose")?;
    let index = |n: &str| {
        names
            .iter()
            .position(|(b, _)| b == n)
            .with_context(|| format!("no bone {n}"))
    };
    let mut axis_bones = vec![];
    for (side, _) in SIDES {
        axis_bones.push([
            u8::try_from(index(&format!("{side}Arm"))?)?,
            u8::try_from(index(&format!("{side}ForeArm"))?)?,
        ]);
    }
    // Sleeve vertices under the band, by torso and side, with their skins.
    type SleevePoints = Vec<([f64; 3], Skin)>;
    let margin = 0.005;
    let under: Vec<[SleevePoints; 2]> = torsos
        .iter()
        .map(|t| {
            [0, 1].map(|k| {
                let arm = &bands[k].arm;
                let (s0, s1) = (proximal * arm.length - margin, distal * arm.length + margin);
                let mut out = vec![];
                for surface in &t.surfaces {
                    let mut seen = BTreeSet::new();
                    for &v in surface.sleeve[k].iter().flatten() {
                        let (s, _, _) = arm.cylindrical(surface.positions[v]);
                        if (s0..=s1).contains(&s) && seen.insert(v) {
                            out.push((surface.positions[v], surface.skins[v].clone()));
                        }
                    }
                }
                out
            })
        })
        .collect();
    let check =
        |pose: &[crate::skeleton_animation::Frame]| -> Result<(f64, usize, usize, f64, f64)> {
            let mut worst = (f64::INFINITY, 0, 0, 0.0, 0.0);
            for (k, band) in bands.iter().enumerate() {
                let posed_band = crate::mask_fit::skin_positions(
                    &band.positions,
                    &band.skins,
                    &poses.bind,
                    pose,
                )?;
                let ends = crate::mask_fit::skin_positions(
                    &[band.arm.origin, band.arm.point(band.arm.length, 0.0, 0.0)],
                    &[vec![(axis_bones[k][0], 1.0)], vec![(axis_bones[k][1], 1.0)]],
                    &poses.bind,
                    pose,
                )?;
                let dir = normalize(sub(ends[1], ends[0]));
                for (t, sides) in under.iter().enumerate() {
                    let (points, skins): (Vec<[f64; 3]>, Vec<Skin>) =
                        sides[k].iter().cloned().unzip();
                    let posed =
                        crate::mask_fit::skin_positions(&points, &skins, &poses.bind, pose)?;
                    for (p, bind_at) in posed.into_iter().zip(&points) {
                        let w = sub(p, ends[0]);
                        let foot: [f64; 3] =
                            std::array::from_fn(|x| ends[0][x] + dir[x] * dot(w, dir));
                        let out = sub(p, foot);
                        let r = dot(out, out).sqrt();
                        if r < 1e-6 {
                            continue;
                        }
                        let Some(hit) =
                            farthest_hit(foot, out.map(|x| x / r), &posed_band, &band.triangles)
                        else {
                            continue;
                        };
                        if hit.0 - r < worst.0 {
                            let (s, _, a) = band.arm.cylindrical(*bind_at);
                            worst = (hit.0 - r, t, k, s / band.arm.length, a.to_degrees());
                        }
                    }
                }
            }
            Ok(worst)
        };
    let all = poses.map(check)?;
    let label = |(gap, t, k, at, angle): (f64, usize, usize, f64, f64), clip: &str| PoseGap {
        clip: clip.to_owned(),
        least: gap,
        torso: torsos[t].model.clone(),
        side: SIDES[k].0.to_owned(),
        at,
        angle,
    };
    let samples = (all.len() - 1) / poses.clips.len();
    let mut out = vec![label(all[0], "bind pose")];
    for (c, (name, _)) in poses.clips.iter().enumerate() {
        let worst = all[1 + c * samples..1 + (c + 1) * samples]
            .iter()
            .copied()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .context("no poses sampled")?;
        out.push(label(worst, name));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm() -> Arm {
        // The native left upper arm, hanging out and down.
        Arm::new([-0.2018, 1.4444, -0.0263], [-0.3737, 1.2345, -0.0103]).unwrap()
    }

    #[test]
    fn the_frame_is_square_and_starts_forward() {
        let a = arm();
        for (x, y) in [(a.dir, a.u), (a.dir, a.v), (a.u, a.v)] {
            assert!(dot(x, y).abs() < 1e-12);
        }
        assert!(a.u[2] > 0.99);
        let p = a.point(0.1, 0.05, 1.0);
        let (s, r, angle) = a.cylindrical(p);
        assert!((s - 0.1).abs() < 1e-12 && (r - 0.05).abs() < 1e-12 && (angle - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_emblem_faces_its_angle_and_the_seam_is_opposite() {
        for turn in [1.0, -1.0] {
            let layout = ring_layout(1.2, turn);
            assert_eq!(layout.len(), 2 * (SEGMENTS + 1));
            let at = |v: f64| {
                layout
                    .iter()
                    .find(|(_, uv, e)| *e == 0 && (uv[1] - v).abs() < 1e-12)
                    .unwrap()
                    .0
            };
            assert!((at(0.5) - 1.2).abs() < 1e-12);
            assert!(wrap(at(0.0) - (1.2 + PI)).abs() < 1e-9);
            assert!(wrap(at(1.0) - at(0.0)).abs() < 1e-9);
            // v grows as the angle falls when turn is +1.
            assert!(wrap(at(0.5 + 1.0 / SEGMENTS as f64) - at(0.5)) * turn < 0.0);
        }
    }

    #[test]
    fn the_hull_of_a_dented_circle_is_the_circle() {
        let angles: Vec<f64> = (0..64).map(|j| -PI + TAU * j as f64 / 64.0).collect();
        let radii: Vec<f64> = (0..64)
            .map(|j| if j % 8 == 3 { 0.03 } else { 0.05 })
            .collect();
        for k in 0..64 {
            let r = hull_radius(&angles, &radii, angles[k] + 0.01);
            assert!((r - 0.05).abs() < 0.0005, "{r}");
        }
    }

    #[test]
    fn a_ring_encodes_rigid_outward_and_wound_to_the_front() {
        let a = arm();
        let layout = ring_layout(0.0, 1.0);
        let positions: Vec<[f64; 3]> = layout
            .iter()
            .map(|(angle, _, edge)| a.point(0.1 + 0.06 * *edge as f64, 0.05, *angle))
            .collect();
        let uvs: Vec<[f64; 2]> = layout.iter().map(|l| l.1).collect();
        for front in [1.0, -1.0] {
            let band = BandArm {
                arm: a,
                skins: vec![vec![(52, 1.0)]; positions.len()],
                positions: positions.clone(),
                uvs: uvs.clone(),
                triangles: ring_triangles(&positions, &a, front),
            };
            assert_eq!(band.triangles.len(), 2 * SEGMENTS);
            let model = encode(std::slice::from_ref(&band)).unwrap();
            let mesh = parse_mesh(&model, 48, "band").unwrap();
            assert_eq!(mesh.positions.len(), 2 * (SEGMENTS + 1));
            assert!(
                skins(&model, 48)
                    .unwrap()
                    .iter()
                    .all(|s| s == &vec![(52, 1.0)])
            );
            // Stored normals point out of the arm, and the winding agrees
            // with them by `front`.
            assert_eq!(front_face_sign(&model).unwrap(), front);
        }
    }
}
