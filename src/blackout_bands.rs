// SPDX-License-Identifier: GPL-3.0-or-later
//! Blackout's team armbands: a plain blue (team 0) or red (team 1) cloth
//! band on both upper arms of the native 0.26g player, made for every torso
//! the player can wear.
//!
//! A Blackout match has no interface: players tell the teams apart by the
//! band. The game adds the band to every player's visual as one more part of
//! the combined skin, next to the torso: for a torso part whose `base_model`
//! is `<B>`, the band is [`band_model`]`(<B>, colour)`, surface [`SURFACE`].
//! It is no item: no config, dictionary row, icon or text.
//!
//! One ring for every torso cannot sit on any of them: the native sleeves
//! differ by centimetres (pack 1 was a hoop 70 cm around, standing 2-5 cm
//! off most sleeves, low on the arm and rigid on the upper-arm bone while
//! the sleeve near the elbow follows the forearm). So each torso model gets
//! its own band, an offset shell of its own sleeve:
//!
//! * the sleeve's outermost, outward-facing triangles of the upper arm (skin
//!   mostly on the side's arm bones) between [`WINDOW`] of the way from the
//!   shoulder joint (`Arm`) to the elbow (`ForeArm`), cut at those two planes,
//!   square to that bone line;
//! * moved [`OFFSET`] out along the shell's own smooth normals;
//! * skinned square to the arm (`square_skins`: on the upper arm's own
//!   bones, `Arm` and its twist bone `Arm_Roll`, the same all round each
//!   height), so both edges stay square to it in every pose. The sleeve's
//!   own skin leans on the clavicle and chest at the top and the forearm at
//!   the bottom, so in a pose the sleeve shears: pack 5 copied that skin and
//!   its rings leaned up to 32 degrees (a slanted stripe on a dead player).
//!   Square, the band stands a little farther off the sleeve where the
//!   sleeve sags in a pose ([`OFFSET`] keeps it out of it);
//! * the torso's own `skeleton` and `render/bind_pose`, so the combined skin
//!   maps its bones exactly as it maps the torso's.
//!
//! A torso whose sleeve does not go around the arm there (no arm geometry)
//! gets no band; [`build`] lists it.
use crate::mask_fit::{Skin, bone_origins, cross, dot, farthest_hit, read_u32, sub};
use crate::outfit_026::{Finish, Writer};
use crate::weapon::parse_config;
use crate::{ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};
use std::fs;
use std::path::{Path, PathBuf};

pub const PORT: &str = "blackout";
/// The band's surface in every band model: a part's `part_name`.
pub const SURFACE: &str = "band";

/// One team's colour.
pub struct Colour {
    pub name: &'static str,
    pub team: u8,
    /// sRGB, clean (pack 2).
    pub rgb: [u8; 3],
    /// sRGB, worn: a dull navy and a brick red.
    pub worn: [u8; 3],
}

pub const COLOURS: [Colour; 2] = [
    Colour {
        name: "blue",
        team: 0,
        rgb: [28, 64, 178],
        worn: [38, 64, 122],
    },
    Colour {
        name: "red",
        team: 1,
        rgb: [176, 26, 26],
        worn: [146, 50, 36],
    },
];

/// How the band's cloth looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Look {
    /// A solid team colour on the plain cloth material (pack 2).
    Plain,
    /// The game's grey woven fabric (the "Grey fabric" camouflage:
    /// diffuse, weave normal map, roughness map) on the Scavenger jacket's
    /// material, tinted to the clean team colour.
    Cloth,
    /// The worn team colour with dirt and fading, on the plain cloth
    /// material with a flat normal map.
    Worn,
    /// The game's fabric tinted to the worn colour, with the dirt and
    /// fading.
    WornCloth,
}

impl Look {
    fn colour(self, c: &Colour) -> [u8; 3] {
        match self {
            Look::Plain | Look::Cloth => c.rgb,
            Look::Worn | Look::WornCloth => c.worn,
        }
    }

    fn fabric(self) -> bool {
        matches!(self, Look::Cloth | Look::WornCloth)
    }

    fn worn(self) -> bool {
        matches!(self, Look::Worn | Look::WornCloth)
    }
}

impl Colour {
    /// The material instance every band of this colour uses (under
    /// `material_instances/`, no extension).
    pub fn material(&self) -> String {
        format!("ported/{PORT}/band_{}", self.name)
    }
}

/// The band model for a torso part whose `base_model` is `torso` (as the
/// item configs and `gameplay/bodyparts/default` write it, e.g.
/// `character/human/edge/edge_hunter_01_torso_s1`): `ported/blackout/` and
/// the torso's path with every `/` as `_`, then `_<colour>`. The model is
/// `models/<that>.skinned_model`.
pub fn band_model(torso: &str, colour: &str) -> String {
    format!("ported/{PORT}/{}_{colour}", torso.replace('/', "_"))
}

/// The band between these fractions of the upper arm (`Arm` 0, `ForeArm`
/// 1): 60 mm of the 272 mm arm, its middle 180 mm below the shoulder joint.
/// Packs 2-5 had it at 0.45-0.67; there the sleeves lean most on the
/// clavicle and chest (a mean 12 % of their skin), which shears them in a
/// pose. At 0.55-0.77 that is least (6-8 %) before the forearm takes over.
pub const WINDOW: [f64; 2] = [0.55, 0.77];
/// How far the band stands out of the sleeve (m). 6 mm, not pack 5's 4:
/// a band made square no longer follows every shear of its sleeve.
pub const OFFSET: f64 = 0.006;
/// A triangle is sleeve when its corners' skins are on average at least
/// this much on the side's arm bones.
const SLEEVE_WEIGHT: f64 = 0.5;
/// A triangle is the outermost when nothing of the sleeve is farther out on
/// the ray from the arm's axis through its middle than this (m).
const OUTERMOST: f64 = 0.003;
/// Sleeve farther than this from the axis is not sleeve (m).
const MAX_RADIUS: f64 = 0.2;
/// A torso gets bands when, on each arm, rays from the axis at the band's
/// middle meet the band at least at this fraction of angles.
const MIN_COVERAGE: f64 = 0.6;
const COVERAGE_RAYS: usize = 72;
/// Texture edge of the solid colour maps.
const MAP_SIZE: u32 = 32;

/// The player's bind pose (`gameplay/bodyparts/default`'s
/// `body_parts.bind_pose`) and rig, which the clips are sampled on.
const BIND_POSE: &str =
    "models/character/human/scavengers/scavengers_01_torso_s1.skinned_model/render/bind_pose";
const PLAYER_SKELETON: &str = "animations/skeletons/scavengers_01.skeleton";
/// The native model whose settings and properties the bands' start from.
const MODEL_TEMPLATE: &str =
    "models/character/human/scavengers/scavengers_01_torso_s1.skinned_model";
const SURFACE_TEMPLATE: &str = "scavengers_01_torso_s";
const SIDES: [&str; 2] = ["Left", "Right"];
const TORSO_KEYS: [&str; 5] = [
    "data",
    "data_full",
    "data_short",
    "data_medium",
    "data_long",
];
/// The clips the band is checked through: a match idle and run with the
/// AK, the AK lobby idle (pack 1's worst) and a pistol lobby idle.
pub const POSE_CLIPS: [&str; 6] = [
    "ak/player/stand/3rd_view/on_site/on_site_idle",
    "ak/player/stand/3rd_view/run/forward",
    "ak/player/stand/3rd_view/on_site/idle_lobby",
    "tt33/player/stand/3rd_view/on_site/idle_wpn_loby",
    "ak/player/stand/3rd_view/death/on_site_back",
    "ak/player/stand/3rd_view/death/on_site_fwd",
];
const POSE_SAMPLES: usize = 24;
/// Sleeve found out along a band vertex's line no farther than this past the
/// band comes over it (m).
const OVER: f64 = 0.015;

/// A bone axis with a frame around it: `u` the body's forward (+z) made
/// square to the axis, `v` = axis x `u`.
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

    /// Out from the axis through `p`, square to it.
    fn outward(&self, p: [f64; 3]) -> [f64; 3] {
        let (s, _, _) = self.cylindrical(p);
        normalize(sub(p, self.point(s, 0.0, 0.0)))
    }
}

/// A torso model's surfaces as one mesh, in the player's bind pose.
struct Torso {
    base_model: String,
    positions: Vec<[f64; 3]>,
    skins: Vec<Skin>,
    /// The stored vertex normals.
    normals: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
    skeleton: Vec<u8>,
    bind_pose: Vec<u8>,
}

/// The `base_model` of every torso part the game can draw: the torso items'
/// (stock and released ports, every glove class; not their additional
/// models) and the bare default's.
fn torso_records(view: &mut ResourceDb) -> Result<BTreeSet<String>> {
    let configs: Vec<String> = view
        .paths_with_prefix("gameplay/items/armour/torso/")
        .filter(|p| p.ends_with(".options"))
        .map(str::to_owned)
        .collect();
    let mut records = BTreeSet::new();
    let mut take = |v: &V| {
        if let Some(V::String(m)) = v.named("base_model") {
            records.insert(m.clone());
        }
    };
    for path in &configs {
        let c = parse_config(view, path, "torso")?;
        for key in TORSO_KEYS {
            if let Some(data) = c.root.named(key) {
                take(data);
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
    ensure!(records.len() > 10, "only {} torso records", records.len());
    Ok(records)
}

type SurfaceData = (Vec<[f64; 3]>, Vec<Skin>, Vec<[f64; 3]>, Vec<[usize; 3]>);

/// A converted surface's vertices: position, skin and stored normal, for
/// any of the 36-48 byte skinned layouts (the basis and UVs are the last 20
/// bytes), and its triangles.
fn surface(b: &[u8], label: &str) -> Result<SurfaceData> {
    let count = read_u32(b, 8)?;
    ensure!(count > 0, "{label}: empty");
    let stride = (read_u32(b, 4)? - 4) / count;
    let mesh = crate::mask_fit::parse_mesh(b, stride, label)?;
    let skins = crate::mask_fit::skins(b, stride)?;
    let normals = (0..count)
        .map(|i| {
            let at = 12 + i * stride + stride - 20;
            let n = b.get(at..at + 3).context("truncated vertex")?;
            Ok([n[2], n[1], n[0]].map(|c| f64::from(c) / 127.5 - 1.0))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((mesh.positions, skins, normals, mesh.triangles))
}

fn load_torso(view: &mut ResourceDb, base_model: &str, bind_pose: &[u8]) -> Result<Torso> {
    let root = format!("models/{base_model}.skinned_model");
    let own = view.read(&format!("{root}/render/bind_pose"))?;
    // The same bones in the same order: the skins index both alike.
    let (a, b) = (
        crate::armor::bind_pose_frames(&own, base_model)?,
        crate::armor::bind_pose_frames(bind_pose, "player bind pose")?,
    );
    ensure!(
        a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.0 == y.0),
        "{base_model} has other bones than the player"
    );
    let paths: Vec<String> = view
        .paths_with_prefix(&format!("{root}/render/"))
        .filter(|p| p.ends_with("/converted_model"))
        // The sniper's ghillie fringes (`edge_sniper_06_alpha_s`): cut-out
        // strands hanging loose, no sleeve to lie on.
        .filter(|p| !p.contains("_alpha_s/"))
        .map(str::to_owned)
        .collect();
    ensure!(!paths.is_empty(), "{base_model} has no surfaces");
    let mut t = Torso {
        base_model: base_model.to_owned(),
        positions: vec![],
        skins: vec![],
        normals: vec![],
        triangles: vec![],
        skeleton: view.read(&format!("{root}/skeleton"))?,
        bind_pose: own,
    };
    for path in paths {
        let (p, s, n, tris) = surface(&view.read(&path)?, &path)?;
        let base = t.positions.len();
        t.triangles
            .extend(tris.into_iter().map(|tri| tri.map(|i| i + base)));
        t.positions.extend(p);
        t.skins.extend(s);
        t.normals.extend(n);
    }
    Ok(t)
}

/// A band vertex before the offset: where it is on the sleeve and its skin.
#[derive(Clone, Debug)]
struct Corner {
    key: Key,
    position: [f64; 3],
    skin: Skin,
}

/// Corners at one place are one vertex: a sleeve vertex by its place, a cut
/// point by its edge's ends and plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Vertex([i64; 3]),
    Cut([i64; 3], [i64; 3], u8),
}

fn quantize(p: [f64; 3]) -> [i64; 3] {
    p.map(|x| (x * 1e5).round() as i64)
}

fn blend(a: &Skin, b: &Skin, t: f64) -> Skin {
    let mut w: BTreeMap<u8, f64> = BTreeMap::new();
    for &(bone, x) in a {
        *w.entry(bone).or_default() += x * (1.0 - t);
    }
    for &(bone, x) in b {
        *w.entry(bone).or_default() += x * t;
    }
    w.into_iter().filter(|(_, x)| *x > 1e-6).collect()
}

/// A corner's place, for keying the cuts through its edges.
fn corner_place(c: &Corner) -> [i64; 3] {
    match c.key {
        Key::Vertex(q) => q,
        Key::Cut(..) => quantize(c.position),
    }
}

/// The part of a polygon at or past `at` along the axis (`keep_above`) or
/// at or before it.
fn clip(polygon: Vec<Corner>, arm: &Arm, at: f64, keep_above: bool, plane: u8) -> Vec<Corner> {
    let d = |c: &Corner| {
        let s = arm.cylindrical(c.position).0 - at;
        if keep_above { s } else { -s }
    };
    let mut out = vec![];
    for k in 0..polygon.len() {
        let (a, b) = (&polygon[k], &polygon[(k + 1) % polygon.len()]);
        let (da, db) = (d(a), d(b));
        if da >= 0.0 {
            out.push(a.clone());
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let (ka, kb) = (corner_place(a), corner_place(b));
            out.push(Corner {
                key: Key::Cut(ka.min(kb), ka.max(kb), plane),
                position: std::array::from_fn(|i| {
                    a.position[i] + (b.position[i] - a.position[i]) * t
                }),
                skin: blend(&a.skin, &b.skin, t),
            });
        }
    }
    out
}

/// One arm's band of one torso.
#[derive(Default)]
struct Shell {
    /// Offset positions.
    positions: Vec<[f64; 3]>,
    /// The sleeve points they were made from.
    sources: Vec<[f64; 3]>,
    normals: Vec<[f64; 3]>,
    skins: Vec<Skin>,
    /// The skins of the sleeve points they were made from.
    source_skins: Vec<Skin>,
    uvs: Vec<[f64; 2]>,
    triangles: Vec<[usize; 3]>,
    /// Which cut plane a vertex lies on: 0 the shoulder's, 1 the elbow's.
    edges: Vec<Option<u8>>,
    /// Rays from the axis at the band's middle that meet it, of all.
    coverage: f64,
}

/// The side's arm weight of each vertex.
fn arm_weights(torso: &Torso, arm_bones: &BTreeSet<u8>) -> Vec<f64> {
    torso
        .skins
        .iter()
        .map(|s| {
            s.iter()
                .filter(|(b, _)| arm_bones.contains(b))
                .map(|(_, w)| w)
                .sum()
        })
        .collect()
}

/// The band on one arm of `torso`, wound so faces point out by `front`'s
/// sign of `(b - a) x (c - a)`.
fn shell(torso: &Torso, arm: &Arm, arm_bones: &BTreeSet<u8>, front: f64) -> Shell {
    let (s0, s1) = (WINDOW[0] * arm.length, WINDOW[1] * arm.length);
    let weight = arm_weights(torso, arm_bones);
    let centroid = |t: &[usize; 3]| -> [f64; 3] {
        std::array::from_fn(|k| t.iter().map(|&i| torso.positions[i][k]).sum::<f64>() / 3.0)
    };
    // The sleeve around the window.
    let candidates: Vec<[usize; 3]> = torso
        .triangles
        .iter()
        .copied()
        .filter(|t| t.iter().map(|&i| weight[i]).sum::<f64>() / 3.0 >= SLEEVE_WEIGHT)
        .filter(|t| {
            let s = t.map(|i| arm.cylindrical(torso.positions[i]).0);
            let lo = s[0].min(s[1]).min(s[2]);
            let hi = s[0].max(s[1]).max(s[2]);
            hi >= s0 - 0.01 && lo <= s1 + 0.01 && arm.cylindrical(centroid(t)).1 <= MAX_RADIUS
        })
        .collect();
    // Its outer skin: facing out, nothing of the sleeve farther out.
    let outer: Vec<[usize; 3]> = candidates
        .iter()
        .copied()
        .filter(|t| {
            let c = centroid(t);
            let n: [f64; 3] =
                std::array::from_fn(|k| t.iter().map(|&i| torso.normals[i][k]).sum::<f64>());
            if dot(n, arm.outward(c)) <= 0.0 {
                return false;
            }
            let (s, r, a) = arm.cylindrical(c);
            farthest_hit(
                arm.point(s, 0.0, 0.0),
                arm.radial(a),
                &torso.positions,
                &candidates,
            )
            .is_none_or(|hit| hit.0 <= r + OUTERMOST)
        })
        .collect();
    // Cut to the window; corners at one place are one vertex.
    let mut index: BTreeMap<Key, usize> = BTreeMap::new();
    let mut corners: Vec<Corner> = vec![];
    let mut triangles = vec![];
    for t in &outer {
        let polygon: Vec<Corner> = t
            .iter()
            .map(|&i| Corner {
                key: Key::Vertex(quantize(torso.positions[i])),
                position: torso.positions[i],
                skin: torso.skins[i].clone(),
            })
            .collect();
        let polygon = clip(clip(polygon, arm, s0, true, 0), arm, s1, false, 1);
        if polygon.len() < 3 {
            continue;
        }
        let ids: Vec<usize> = polygon
            .into_iter()
            .map(|c| {
                *index.entry(c.key).or_insert_with(|| {
                    corners.push(c);
                    corners.len() - 1
                })
            })
            .collect();
        for k in 1..ids.len() - 1 {
            let tri = [ids[0], ids[k], ids[k + 1]];
            if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                triangles.push(tri);
            }
        }
    }
    let mut out = Shell::default();
    if triangles.is_empty() {
        return out;
    }
    // Smooth normals of the cut sleeve, out of the arm; faces wound out.
    let mut normals = vec![[0.0; 3]; corners.len()];
    for t in &mut triangles {
        let [a, b, c] = t.map(|i| corners[i].position);
        let face = cross(sub(b, a), sub(c, a));
        let mid: [f64; 3] = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0);
        let n = if dot(face, arm.outward(mid)) < 0.0 {
            face.map(|x| -x)
        } else {
            face
        };
        if dot(face, n) * front < 0.0 {
            t.swap(1, 2);
        }
        for &i in t.iter() {
            for k in 0..3 {
                normals[i][k] += n[k];
            }
        }
    }
    for (c, n) in corners.into_iter().zip(normals) {
        let len = dot(n, n).sqrt();
        let n = if len > 1e-12 {
            n.map(|x| x / len)
        } else {
            arm.outward(c.position)
        };
        let (s, _, a) = arm.cylindrical(c.position);
        out.positions
            .push(std::array::from_fn(|k| c.position[k] + n[k] * OFFSET));
        out.sources.push(c.position);
        out.normals.push(n);
        out.edges.push(match c.key {
            Key::Cut(_, _, plane) => Some(plane),
            Key::Vertex(_) => None,
        });
        out.source_skins.push(c.skin.clone());
        out.skins.push(c.skin);
        out.uvs
            .push([((s - s0) / (s1 - s0)).clamp(0.0, 1.0), (a + PI) / TAU]);
    }
    out.triangles = triangles;
    let mid = (s0 + s1) / 2.0;
    let hits = (0..COVERAGE_RAYS)
        .filter(|&j| {
            let a = -PI + TAU * j as f64 / COVERAGE_RAYS as f64;
            farthest_hit(
                arm.point(mid, 0.0, 0.0),
                arm.radial(a),
                &out.positions,
                &out.triangles,
            )
            .is_some()
        })
        .count();
    out.coverage = hits as f64 / COVERAGE_RAYS as f64;
    out
}

/// The band's skin made square to the upper arm: on the upper arm's own
/// bones only (`Arm` and its twist bone `Arm_Roll`, which turns about the
/// arm's axis), the twist share the same all round the arm at each height,
/// the mean of the sleeve's there (within [`RING_SPAN`]).
///
/// The sleeve's own skin differs round the arm: its top leans on the
/// shoulder, its inside on the chest, its lower edge on the forearm. A ring
/// of points cut square in the bind pose then spreads along the arm in a
/// pose (up to 25-30 degrees in the match and death clips): a slanted
/// stripe. Bones that only turn the arm about its axis keep every plane
/// square to it in every pose.
fn square_skins(shell: &Shell, arm: &Arm, [arm_bone, roll]: [u8; 2]) -> Vec<Skin> {
    let s: Vec<f64> = shell
        .sources
        .iter()
        .map(|p| arm.cylindrical(*p).0)
        .collect();
    let share: Vec<Option<f64>> = shell
        .source_skins
        .iter()
        .map(|skin| {
            let w = |b: u8| {
                skin.iter()
                    .filter(|(x, _)| *x == b)
                    .map(|(_, w)| w)
                    .sum::<f64>()
            };
            let (a, r) = (w(arm_bone), w(roll));
            (a + r > 1e-6).then(|| r / (a + r))
        })
        .collect();
    (0..shell.source_skins.len())
        .map(|i| {
            let near: Vec<f64> = (0..s.len())
                .filter(|&j| (s[j] - s[i]).abs() <= RING_SPAN)
                .filter_map(|j| share[j])
                .collect();
            let r = if near.is_empty() {
                0.0
            } else {
                near.iter().sum::<f64>() / near.len() as f64
            };
            if r > 1e-6 {
                vec![(arm_bone, 1.0 - r), (roll, r)]
            } else {
                vec![(arm_bone, 1.0)]
            }
        })
        .collect()
}

/// Band vertices this near along the arm share their twist share (m).
const RING_SPAN: f64 = 0.008;

/// Which way stock skinned surfaces wind a front face: the sign of
/// `(b - a) x (c - a)` against the stored vertex normals.
fn front_face_sign(model: &[u8]) -> Result<f64> {
    let (positions, _, normals, triangles) = surface(model, "template surface")?;
    let mut score = 0.0;
    for t in &triangles {
        let [a, b, c] = t.map(|i| positions[i]);
        let n = cross(sub(b, a), sub(c, a));
        let stored: [f64; 3] =
            std::array::from_fn(|k| t.iter().map(|&i| normals[i][k]).sum::<f64>());
        score += dot(n, stored).signum();
    }
    ensure!(
        score.abs() > triangles.len() as f64 * 0.8,
        "the template's winding is not consistent ({score} of {})",
        triangles.len()
    );
    Ok(score.signum())
}

/// A 48-byte four-weight converted surface of both arms' bands: the
/// tangent along the arm, the binormal around it (the maps are flat).
fn encode(arms: &[(&Arm, &Shell)]) -> Result<Vec<u8>> {
    let count: usize = arms.iter().map(|(_, s)| s.positions.len()).sum();
    ensure!(
        count > 0 && count <= usize::from(u16::MAX),
        "{count} band vertices"
    );
    let mut out = vec![];
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + count * 48)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(count)?.to_le_bytes());
    let mut indices = vec![];
    let mut base = 0;
    for (arm, band) in arms {
        for i in 0..band.positions.len() {
            let n = band.normals[i];
            let t = normalize(sub(arm.dir, n.map(|c| c * dot(arm.dir, n))));
            let b = cross(n, t);
            for x in band.positions[i] {
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

/// One torso's bands.
#[derive(Clone, Debug, Serialize)]
pub struct TorsoBand {
    pub base_model: String,
    /// Left, right.
    pub vertices: [usize; 2],
    pub triangles: [usize; 2],
    pub coverage: [f64; 2],
}

/// The band's distance to its sleeve over every band vertex and torso in
/// the sampled poses of one clip (m); negative where the sleeve comes over
/// the band.
#[derive(Clone, Debug, Serialize)]
pub struct PoseGap {
    pub clip: String,
    pub least: f64,
    pub mean: f64,
    pub largest: f64,
    /// The torso with the least gap, and the side.
    pub worst: String,
    pub side: String,
    /// Band vertices the sleeve comes over by more than 1 mm, of all, over
    /// all samples.
    pub covered: usize,
    /// Band vertices off their sleeve (`is_off`: under it, or floating),
    /// of all, over all samples.
    pub off: usize,
    pub measured: usize,
    /// The torsos whose band is most often off its sleeve: the fraction of
    /// their band vertices over all samples.
    pub most_off: Vec<(String, f64)>,
    /// How far the band's edges lean from square to the upper arm (the
    /// posed `Arm` to `ForeArm` line), degrees: the worst, where, and the
    /// mean over every edge, torso and sample.
    pub tilt_worst: f64,
    pub tilt_worst_torso: String,
    pub tilt_worst_side: String,
    pub tilt_mean: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub look: Look,
    pub upper_arm: f64,
    pub window: [f64; 2],
    pub offset: f64,
    pub bands: Vec<TorsoBand>,
    /// Torso records without a usable sleeve, and why.
    pub skipped: Vec<(String, String)>,
    pub poses: Vec<PoseGap>,
}

/// Builds the band models of every torso, two materials and their maps
/// into `output/overlay`, with `output/port-manifest.json` (a bundle for
/// `pack from-bundle`), `output/torsos.tsv` (the torso records that have
/// bands) and `output/report.json`.
///
/// `patches`: the released `resources.db.patch_<n>`, whose torsos get
/// bands too.
pub fn build(db: &Path, patches: &[PathBuf], look: Look, output: &Path) -> Result<Report> {
    crate::weapon::prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut view = crate::novice_026::game_view(db, patches)?;
    let bind_pose = view.read(BIND_POSE)?;
    let names: Vec<String> = crate::armor::bind_pose_origins(&bind_pose, "player bind pose")?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let index = |n: &str| -> Result<u8> {
        u8::try_from(
            names
                .iter()
                .position(|b| b == n)
                .with_context(|| format!("no bone {n}"))?,
        )
        .context("bone index")
    };
    let origins = bone_origins(&bind_pose)?;
    let mut arm_bones: [BTreeSet<u8>; 2] = Default::default();
    let mut upper = [[0_u8; 2]; 2];
    let mut arms = vec![];
    for (k, side) in SIDES.iter().enumerate() {
        for b in ["Arm", "Arm_Roll", "ForeArm", "ForeArm_Roll", "Shoulder"] {
            arm_bones[k].insert(index(&format!("{side}{b}"))?);
        }
        upper[k] = [
            index(&format!("{side}Arm"))?,
            index(&format!("{side}Arm_Roll"))?,
        ];
        {}
        arms.push(Arm::new(
            origins[&format!("{side}Arm")],
            origins[&format!("{side}ForeArm")],
        )?);
    }
    let front = front_face_sign(&view.read(&format!(
        "{MODEL_TEMPLATE}/render/{SURFACE_TEMPLATE}/converted_model"
    ))?)?;

    let records = torso_records(&mut view)?;
    let mut bands = vec![];
    let mut skipped = vec![];
    let mut built: Vec<(Torso, [Shell; 2])> = vec![];
    for record in &records {
        let torso = match load_torso(&mut view, record, &bind_pose) {
            Ok(t) => t,
            Err(e) => {
                skipped.push((record.clone(), format!("{e:#}")));
                continue;
            }
        };
        let shells = [0, 1].map(|k| shell(&torso, &arms[k], &arm_bones[k], front));
        let coverage = [shells[0].coverage, shells[1].coverage];
        if coverage.iter().any(|&c| c < MIN_COVERAGE) {
            skipped.push((
                record.clone(),
                format!(
                    "no sleeve around the upper arm (rays meeting a band: left {:.0}%, right {:.0}%)",
                    coverage[0] * 100.0,
                    coverage[1] * 100.0
                ),
            ));
            continue;
        }
        bands.push(TorsoBand {
            base_model: record.clone(),
            vertices: [shells[0].positions.len(), shells[1].positions.len()],
            triangles: [shells[0].triangles.len(), shells[1].triangles.len()],
            coverage,
        });
        built.push((torso, shells));
    }
    ensure!(!built.is_empty(), "no torso has a sleeve");
    // Square to the upper arm in every pose.
    for (_, shells) in &mut built {
        for (k, shell) in shells.iter_mut().enumerate() {
            shell.skins = square_skins(shell, &arms[k], upper[k]);
        }
    }
    let poses = sample_poses(&mut view, &bind_pose)?;
    let poses = pose_gaps(&built, &poses)?;

    // --- Models, materials, maps ---
    let out = Writer {
        overlay: &overlay,
        tag: "Blackout-bands",
    };
    let mut files = vec![];
    let maps = output.join("maps");
    fs::create_dir_all(&maps)?;
    materials(&out, &mut view, look, &maps, &mut files)?;
    for (torso, shells) in &built {
        let surface = encode(&[(&arms[0], &shells[0]), (&arms[1], &shells[1])])?;
        for colour in &COLOURS {
            write_model(&out, &mut view, torso, colour, &surface, &mut files)?;
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "blackout-bands",
            "description": "Blackout team armbands: a blue and a red cloth band on each torso's sleeves",
            "files": files,
        }))?,
    )?;
    let tsv: String = bands
        .iter()
        .map(|b| format!("{}\n", b.base_model))
        .collect();
    fs::write(output.join("torsos.tsv"), tsv)?;
    let report = Report {
        look,
        upper_arm: arms[0].length,
        window: WINDOW,
        offset: OFFSET,
        bands,
        skipped,
        poses,
    };
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    Ok(report)
}

/// A solid `rgb` 24-bit TGA of [`MAP_SIZE`].
fn solid_tga(path: &Path, rgb: [u8; 3]) -> Result<()> {
    write_tga(path, MAP_SIZE, MAP_SIZE, |_, _| rgb)
}

/// A 24-bit top-left TGA of `w` x `h`, each texel from `texel(x, y)`.
fn write_tga(path: &Path, w: u32, h: u32, texel: impl Fn(u32, u32) -> [u8; 3]) -> Result<()> {
    let mut b = vec![0_u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&u16::try_from(w)?.to_le_bytes());
    b.extend_from_slice(&u16::try_from(h)?.to_le_bytes());
    b.extend_from_slice(&[24, 0x20]);
    for y in 0..h {
        for x in 0..w {
            let [r, g, bl] = texel(x, y);
            b.extend_from_slice(&[bl, g, r]);
        }
    }
    fs::write(path, b)?;
    Ok(())
}

/// The game's grey woven fabric: the "Grey fabric" camouflage
/// (`camo_fabric_grey`) a player can put on a jacket, a tileable 512x512
/// set of diffuse, weave normal map and roughness map.
pub const FABRIC: &str = "character/human/camo/scavenger/fabric_grey";
/// The fabric's material: the Scavenger jacket's, a skinned character
/// torso's cloth (`complex`, skeletal meshes, the fuzzy sheen of cloth),
/// whose diffuse, normal and roughness maps the camouflage stands in for.
pub const FABRIC_MATERIAL: &str =
    "material_instances/characters/human/scavengers_01/scavengers_01_torso.material";
/// The jacket's Fresnel map over its upper-arm sleeve, mean (its texels
/// at the middles of the sleeve's triangles there): the band's constant.
const SLEEVE_FRESNEL: f32 = 0.012;
/// The cloth a 512-texel fabric tile spans (m): threads about 1 mm apart.
const FABRIC_SPAN: f64 = 0.25;
/// The band's maps: across the band and around the arm, and the cloth
/// they span (60 mm across; around, two whole fabric tiles, so the map
/// meets itself where the band's UVs wrap).
const BAND_MAP: [u32; 2] = [128, 1024];
const BAND_SPAN: [f64; 2] = [0.06, 0.5];
/// Dust ground into worn cloth (sRGB).
const DIRT: [u8; 3] = [88, 78, 62];

fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// A decoded texture, RGBA bytes.
struct Pixels {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    /// `textures/<name>.dds` of the game, decoded by ImageMagick.
    fn decode(view: &mut ResourceDb, name: &str, work: &Path) -> Result<Self> {
        let path = format!("textures/{name}.dds");
        let bytes = view.read(&path)?;
        let info = crate::soc::source::parse_dds(&bytes, &path)?;
        let stem = work.join(name.replace('/', "_"));
        let (dds, raw) = (stem.with_extension("dds"), stem.with_extension("rgba"));
        fs::write(&dds, &bytes)?;
        let result = std::process::Command::new("magick")
            .arg(&dds)
            .args(["-depth", "8"])
            .arg(format!("rgba:{}", raw.display()))
            .output()
            .context("starting ImageMagick; enter the repository Nix shell")?;
        ensure!(
            result.status.success(),
            "ImageMagick could not decode {path}: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
        let rgba = fs::read(&raw)?;
        fs::remove_file(&dds)?;
        fs::remove_file(&raw)?;
        ensure!(
            rgba.len() == (info.width * info.height * 4) as usize,
            "{path}: decoded {} bytes for {}x{}",
            rgba.len(),
            info.width,
            info.height
        );
        Ok(Self {
            w: info.width,
            h: info.height,
            rgba,
        })
    }

    /// Bilinear, wrapping, 0..1 per channel.
    fn sample(&self, x: f64, y: f64) -> [f64; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f64, j: f64, k: usize| -> f64 {
            let i = (i as i64).rem_euclid(i64::from(self.w)) as usize;
            let j = (j as i64).rem_euclid(i64::from(self.h)) as usize;
            f64::from(self.rgba[(j * self.w as usize + i) * 4 + k]) / 255.0
        };
        std::array::from_fn(|k| {
            let top = at(x0, y0, k) * (1.0 - fx) + at(x0 + 1.0, y0, k) * fx;
            let bottom = at(x0, y0 + 1.0, k) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0, k) * fx;
            top * (1.0 - fy) + bottom * fy
        })
    }

    /// This tile at the band's texel `(x, y)` of [`BAND_MAP`].
    fn on_band(&self, x: u32, y: u32) -> [f64; 4] {
        let scale = |k: usize, t: u32, size: u32| {
            (f64::from(t) + 0.5) / f64::from(BAND_MAP[k]) * BAND_SPAN[k] / FABRIC_SPAN
                * f64::from(size)
        };
        self.sample(scale(0, x, self.w), scale(1, y, self.h))
    }
}

/// Smooth value noise in 0..1 with `cell` texel cells, wrapping around the
/// band (`BAND_MAP[1]` must be a multiple of `cell`).
fn noise(x: u32, y: u32, cell: u32, seed: u64) -> f64 {
    let hash = |i: i64, j: i64| -> f64 {
        let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 11) as f64 / (1_u64 << 53) as f64
    };
    let period = i64::from(BAND_MAP[1] / cell);
    let (fx, fy) = (
        (f64::from(x) + 0.5) / f64::from(cell),
        (f64::from(y) + 0.5) / f64::from(cell),
    );
    let (i, j) = (fx.floor() as i64, fy.floor() as i64);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (u, v) = (s(fx - fx.floor()), s(fy - fy.floor()));
    let at = |di: i64, dj: i64| hash(i + di, (j + dj).rem_euclid(period));
    let top = at(0, 0) * (1.0 - u) + at(1, 0) * u;
    let bottom = at(0, 1) * (1.0 - u) + at(1, 1) * u;
    top * (1.0 - v) + bottom * v
}

/// Worn cloth: grain, faded patches and ground-in dust over linear `c`.
fn wear(c: [f64; 3], x: u32, y: u32) -> [f64; 3] {
    let grain = (noise(x, y, 8, 3) - 0.5) * 0.16 + (noise(x, y, 2, 4) - 0.5) * 0.08;
    let fade = noise(x, y, 256, 5) * 0.6 + noise(x, y, 64, 6) * 0.4;
    let dirt = noise(x, y, 128, 7) * 0.6 + noise(x, y, 32, 8) * 0.4;
    let fade = ((fade - 0.45) / 0.4).clamp(0.0, 1.0) * 0.3;
    let dirt = ((dirt - 0.5) / 0.35).clamp(0.0, 1.0) * 0.25;
    let grey = luminance(c);
    let dust = DIRT.map(to_linear);
    std::array::from_fn(|k| {
        let g = c[k] * (1.0 + grain);
        let faded = g * (1.0 - fade) + (grey * 1.25) * fade;
        faded * (1.0 - dirt) + dust[k] * dirt
    })
}

/// The bands' two materials and their maps, as `look` has them.
fn materials(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    look: Look,
    maps: &Path,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    if look.fabric() {
        return fabric_materials(out, view, look, maps, files);
    }
    for colour in &COLOURS {
        let stem = maps.join(format!("band_{}", colour.name));
        let rgb = look.colour(colour);
        if look.worn() {
            let base = rgb.map(to_linear);
            write_tga(
                &stem.with_extension("tga"),
                BAND_MAP[0],
                BAND_MAP[1],
                |x, y| wear(base, x, y).map(to_srgb),
            )?;
        } else {
            solid_tga(&stem.with_extension("tga"), rgb)?;
        }
        solid_tga(
            &PathBuf::from(format!("{}_nmap.tga", stem.display())),
            [128, 128, 255],
        )?;
        out.cloth_material_cropped(
            view,
            PORT,
            &format!("band_{}", colour.name),
            &stem,
            None,
            Finish::CLOTH,
            files,
        )?;
    }
    Ok(())
}

/// One converted map of the bands, `textures/ported/blackout/<name>`, with
/// the options of the fabric's map it comes from.
#[allow(clippy::too_many_arguments)]
fn band_texture(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    tga: &Path,
    name: &str,
    compression: crate::manifest::TextureCompression,
    srgb: bool,
    options_from: &str,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<String> {
    let path = format!("textures/ported/{PORT}/{name}");
    let dds = tga.with_extension("dds");
    crate::armor::convert_texture(tga, &dds, compression)?;
    let bytes = crate::duty_026::texture_dds(&fs::read(&dds)?, &path, srgb)?;
    out.raw(&format!("{path}.dds"), &bytes, files)?;
    let options = parse_config(
        view,
        &format!("textures/{options_from}.options"),
        "texture options",
    )?;
    out.config(&format!("{path}.options"), &options, files)?;
    Ok(format!("ported/{PORT}/{name}"))
}

/// The game's grey fabric, tiled at its own weave over the band and tinted
/// to each team's colour (its light and dark kept: each texel's luminance
/// against the tile's mean scales the colour), on the Scavenger jacket's
/// material with the fabric's own weave normal map and roughness map.
fn fabric_materials(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    look: Look,
    maps: &Path,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    use crate::manifest::TextureCompression;
    let diffuse = Pixels::decode(view, FABRIC, maps)?;
    let normal = Pixels::decode(view, &format!("{FABRIC}_nmap"), maps)?;
    let rough = Pixels::decode(view, &format!("{FABRIC}_smap"), maps)?;
    let [w, h] = BAND_MAP;
    // The weave: DXT5nm, X in alpha and Y in green, as an RGB normal map.
    let tga = maps.join("band_fabric_normal.tga");
    write_tga(&tga, w, h, |x, y| {
        let t = normal.on_band(x, y);
        let (nx, ny) = (t[3] * 2.0 - 1.0, t[1] * 2.0 - 1.0);
        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        [nx, ny, nz].map(|c| ((c * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8)
    })?;
    let normal_name = band_texture(
        out,
        view,
        &tga,
        "band_fabric_normal",
        TextureCompression::Dxt5Normal,
        false,
        &format!("{FABRIC}_nmap"),
        files,
    )?;
    let tga = maps.join("band_fabric_roughness.tga");
    write_tga(&tga, w, h, |x, y| {
        let t = rough.on_band(x, y);
        [t[0], t[1], t[2]].map(|c| (c * 255.0).round() as u8)
    })?;
    let rough_name = band_texture(
        out,
        view,
        &tga,
        "band_fabric_roughness",
        TextureCompression::Dxt1,
        false,
        &format!("{FABRIC}_smap"),
        files,
    )?;
    let mean = {
        let n = (diffuse.w * diffuse.h) as usize;
        (0..n)
            .map(|i| luminance(std::array::from_fn(|k| to_linear(diffuse.rgba[i * 4 + k]))))
            .sum::<f64>()
            / n as f64
    };
    for colour in &COLOURS {
        let tint = look.colour(colour).map(to_linear);
        let tga = maps.join(format!("band_{}_diffuse.tga", colour.name));
        write_tga(&tga, w, h, |x, y| {
            let t = diffuse.on_band(x, y);
            let lum = luminance(std::array::from_fn(|k| {
                to_linear((t[k] * 255.0).round() as u8)
            }));
            let shade = (lum / mean).clamp(0.0, 2.5);
            let c = tint.map(|v| v * shade);
            let c = if look.worn() { wear(c, x, y) } else { c };
            c.map(to_srgb)
        })?;
        let diffuse_name = band_texture(
            out,
            view,
            &tga,
            &format!("band_{}_diffuse", colour.name),
            TextureCompression::Dxt1,
            true,
            FABRIC,
            files,
        )?;
        let mut mat = parse_config(view, FABRIC_MATERIAL, "material")?;
        for (slot, value) in [
            ("texture_diffuse", &diffuse_name),
            ("texture_normal", &normal_name),
            ("texture_alphablended_normal", &normal_name),
            ("texture_detail_normal", &normal_name),
            ("texture_roughness", &rough_name),
        ] {
            mat.root.set_path(
                &format!("material.g_stage.effect.{slot}.value"),
                V::String(value.clone()),
            )?;
        }
        for key in ["constant_fresnel_min", "constant_fresnel_max"] {
            mat.root.set_path(
                &format!("material.g_stage.effect.{key}.value"),
                V::Float(SLEEVE_FRESNEL),
            )?;
        }
        out.config(
            &format!("material_instances/{}.material", colour.material()),
            &mat,
            files,
        )?;
    }
    Ok(())
}

/// A band model folder: the torso's own bind pose and skeleton, settings
/// naming [`SURFACE`] and the colour's material, and properties.
fn write_model(
    out: &Writer<'_>,
    view: &mut ResourceDb,
    torso: &Torso,
    colour: &Colour,
    surface: &[u8],
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let root = format!(
        "models/{}.skinned_model",
        band_model(&torso.base_model, colour.name)
    );
    out.raw(&format!("{root}/render/bind_pose"), &torso.bind_pose, files)?;
    out.raw(&format!("{root}/skeleton"), &torso.skeleton, files)?;
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
        V::String(colour.material()),
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
        V::String(format!("|{}|{SURFACE}", torso.base_model)),
    )?;
    props
        .root
        .set_path("source.texture_name", V::String(colour.material()))?;
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
        V::String(format!("blackout-bands/{}", torso.base_model)),
    )?;
    render.root.set_path("bounding_box.min", V::Float3(min))?;
    render.root.set_path("bounding_box.max", V::Float3(max))?;
    out.config(&format!("{root}/render/export_properties"), &render, files)
}

/// The game's clips sampled on the player's rig, and what the measures
/// need of it.
struct Poses {
    bind: Vec<crate::skeleton_animation::Frame>,
    /// The bind pose, then [`POSE_SAMPLES`] poses of each of [`POSE_CLIPS`].
    clips: Vec<(String, Vec<Vec<crate::skeleton_animation::Frame>>)>,
    /// Each side's `Arm` and `ForeArm`.
    joints: [[u8; 2]; 2],
    /// Every arm and shoulder bone, both sides.
    on_arm: BTreeSet<u8>,
}

fn sample_poses(view: &mut ResourceDb, bind_pose: &[u8]) -> Result<Poses> {
    use crate::skeleton_animation::{Clip, Rig};
    let rig = Rig::parse(bind_pose, &view.read(PLAYER_SKELETON)?)?;
    let bind = rig.rest();
    let mut clips = vec![("bind pose".to_owned(), vec![bind.clone()])];
    for name in POSE_CLIPS {
        let path = format!("animations/single/weapons/{name}");
        let clip = Clip::parse(
            &view.read(&format!("{path}.b-spline"))?,
            &view.read(&format!("{path}.bones_names"))?,
        )
        .with_context(|| path.clone())?;
        let end = clip.duration();
        clips.push((
            name.to_owned(),
            (0..POSE_SAMPLES)
                .map(|k| rig.pose(&clip, end * k as f32 / (POSE_SAMPLES - 1) as f32))
                .collect(),
        ));
    }
    let index = |n: String| {
        rig.index(&n)
            .and_then(|i| u8::try_from(i).ok())
            .with_context(|| format!("no {n}"))
    };
    let mut joints = [[0; 2]; 2];
    for (k, side) in SIDES.iter().enumerate() {
        joints[k] = [
            index(format!("{side}Arm"))?,
            index(format!("{side}ForeArm"))?,
        ];
    }
    let on_arm = rig
        .bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.name.contains("Arm") || b.name.contains("Shoulder"))
        .filter_map(|(i, _)| u8::try_from(i).ok())
        .collect();
    Ok(Poses {
        bind,
        clips,
        joints,
        on_arm,
    })
}

/// A torso's triangles the band could meet in a pose: mostly on the arm.
fn near_sleeve(torso: &Torso, on_arm: &BTreeSet<u8>) -> Vec<[usize; 3]> {
    let w = arm_weights(torso, on_arm);
    torso
        .triangles
        .iter()
        .copied()
        .filter(|t| t.iter().map(|&i| w[i]).sum::<f64>() / 3.0 >= 0.2)
        .collect()
}

/// One torso's bands in one pose: each band vertex's gap to its sleeve
/// (m) and each edge's lean from square to the upper arm (degrees), with
/// their side.
#[derive(Default)]
struct PoseFit {
    gaps: Vec<(f64, usize)>,
    tilts: Vec<(f64, usize)>,
}

/// Band and sleeve skinned into `pose` (each with its own skin): each band
/// vertex measured from the sleeve point it was made from; out from the
/// arm, the gap is that distance, less where the sleeve comes out over the
/// band within [`OVER`] of it (farther out is other geometry: the body by
/// the armpit, a pouch); into the arm, it is negative. Each band edge's
/// plane against the posed `Arm` to `ForeArm` line.
fn fit_in_pose(
    torso: &Torso,
    shells: &[Shell; 2],
    near: &[[usize; 3]],
    poses: &Poses,
    pose: &[crate::skeleton_animation::Frame],
) -> Result<PoseFit> {
    let bind = &poses.bind;
    let mut fit = PoseFit::default();
    let sleeve = crate::mask_fit::skin_positions(&torso.positions, &torso.skins, bind, pose)?;
    for (k, shell) in shells.iter().enumerate() {
        let j = poses.joints[k];
        let origin = |b: u8| bind[usize::from(b)].1.map(f64::from);
        let ends = crate::mask_fit::skin_positions(
            &[origin(j[0]), origin(j[1])],
            &[vec![(j[0], 1.0)], vec![(j[1], 1.0)]],
            bind,
            pose,
        )?;
        let axis = normalize(sub(ends[1], ends[0]));
        let band = crate::mask_fit::skin_positions(&shell.positions, &shell.skins, bind, pose)?;
        for plane in [0, 1] {
            let ring: Vec<[f64; 3]> = band
                .iter()
                .zip(&shell.edges)
                .filter(|(_, e)| **e == Some(plane))
                .map(|(p, _)| *p)
                .collect();
            if let Some(normal) = plane_normal(&ring) {
                let a = dot(normal, axis).abs().clamp(0.0, 1.0).acos().to_degrees();
                fit.tilts.push((a, k));
            }
        }
        for b in &band {
            // On the ray from the posed arm's axis out through the band
            // vertex: the sleeve's outermost surface up to OVER beyond it.
            let w = sub(*b, ends[0]);
            let foot: [f64; 3] = std::array::from_fn(|x| ends[0][x] + axis[x] * dot(w, axis));
            let out = sub(*b, foot);
            let r = dot(out, out).sqrt();
            if r < 1e-6 {
                continue;
            }
            let sleeve_r = crate::mask_fit::ray_hits(foot, out.map(|x| x / r), &sleeve, near)
                .into_iter()
                .map(|h| h.0)
                .filter(|&h| h > 0.0 && h < r + OVER)
                .fold(None, |m: Option<f64>, h| {
                    Some(m.map_or(h, |m: f64| m.max(h)))
                });
            // Nothing of the sleeve under it: it stands off by more than
            // OVER.
            fit.gaps
                .push((sleeve_r.map_or(OFFSET + OVER, |s| r - s), k));
        }
    }
    Ok(fit)
}

/// A band vertex is off its sleeve when the sleeve comes over it by more
/// than 1 mm or it stands more than [`OFF_FLOAT`] farther out than
/// [`OFFSET`].
fn is_off(gap: f64) -> bool {
    !(-0.001..=OFFSET + OFF_FLOAT).contains(&gap)
}

/// Standing this much farther out than [`OFFSET`] is floating (m).
const OFF_FLOAT: f64 = 0.004;

/// The normal of the plane through `points` (least squares): the
/// eigenvector of their scatter with the least eigenvalue (Jacobi), if the
/// points span a plane.
#[allow(clippy::needless_range_loop)]
fn plane_normal(points: &[[f64; 3]]) -> Option<[f64; 3]> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mean: [f64; 3] = std::array::from_fn(|k| points.iter().map(|p| p[k]).sum::<f64>() / n);
    let mut a = [[0.0; 3]; 3];
    for p in points {
        let d = sub(*p, mean);
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += d[i] * d[j];
            }
        }
    }
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let (mut p, mut q, mut big) = (0, 1, 0.0);
        for i in 0..3 {
            for j in i + 1..3 {
                if a[i][j].abs() > big {
                    (p, q, big) = (i, j, a[i][j].abs());
                }
            }
        }
        if big < 1e-15 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = if theta == 0.0 {
            1.0
        } else {
            theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
        };
        let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
        let mut b = a;
        for k in 0..3 {
            b[k][p] = c * a[k][p] - s * a[k][q];
            b[k][q] = s * a[k][p] + c * a[k][q];
        }
        let mut d = b;
        for k in 0..3 {
            d[p][k] = c * b[p][k] - s * b[q][k];
            d[q][k] = s * b[p][k] + c * b[q][k];
        }
        a = d;
        for row in &mut v {
            let (x, y) = (row[p], row[q]);
            row[p] = c * x - s * y;
            row[q] = s * x + c * y;
        }
    }
    let k = (0..3).min_by(|&i, &j| a[i][i].total_cmp(&a[j][j]))?;
    let mut spreads = [a[0][0], a[1][1], a[2][2]];
    spreads.sort_by(f64::total_cmp);
    (spreads[1] > 1e-8).then(|| normalize([v[0][k], v[1][k], v[2][k]]))
}

/// Over every torso, per clip: the gaps, the vertices off their sleeve and
/// the edges' lean.
fn pose_gaps(built: &[(Torso, [Shell; 2])], poses: &Poses) -> Result<Vec<PoseGap>> {
    let near: Vec<Vec<[usize; 3]>> = built
        .iter()
        .map(|(t, _)| near_sleeve(t, &poses.on_arm))
        .collect();
    let mut out = vec![];
    for (clip, frames) in &poses.clips {
        // Each torso's fits over the clip's samples, on a few threads.
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let chunk = built.len().div_ceil(threads);
        let per_torso: Vec<Vec<PoseFit>> = std::thread::scope(|scope| {
            let workers: Vec<_> = built
                .chunks(chunk)
                .zip(near.chunks(chunk))
                .map(|(part, nears)| {
                    scope.spawn(move || {
                        part.iter()
                            .zip(nears)
                            .map(|((torso, shells), n)| {
                                frames
                                    .iter()
                                    .map(|f| fit_in_pose(torso, shells, n, poses, f))
                                    .collect::<Result<Vec<_>>>()
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| {
                    w.join()
                        .map_err(|_| anyhow::anyhow!("a pose thread panicked"))?
                })
                .collect::<Result<Vec<_>>>()
                .map(|v| v.into_iter().flatten().collect())
        })?;
        let mut least = (f64::INFINITY, 0, 0);
        let mut largest = f64::NEG_INFINITY;
        let (mut sum, mut measured, mut off, mut covered) = (0.0, 0, 0, 0);
        let mut tilt_worst = (0.0, 0, 0);
        let (mut tilt_sum, mut tilts) = (0.0, 0);
        let mut most_off = vec![];
        for (t, fits) in per_torso.iter().enumerate() {
            let mut torso_off = 0;
            let mut torso_n = 0;
            for f in fits {
                for &(g, k) in &f.gaps {
                    sum += g;
                    measured += 1;
                    torso_n += 1;
                    off += usize::from(is_off(g));
                    torso_off += usize::from(is_off(g));
                    covered += usize::from(g < -0.001);
                    largest = largest.max(g);
                    if g < least.0 {
                        least = (g, t, k);
                    }
                }
                for &(a, k) in &f.tilts {
                    tilt_sum += a;
                    tilts += 1;
                    if a > tilt_worst.0 {
                        tilt_worst = (a, t, k);
                    }
                }
            }
            most_off.push((
                built[t].0.base_model.clone(),
                torso_off as f64 / torso_n.max(1) as f64,
            ));
        }
        most_off.sort_by(|a, b| b.1.total_cmp(&a.1));
        most_off.truncate(5);
        out.push(PoseGap {
            clip: clip.clone(),
            least: least.0,
            mean: sum / measured.max(1) as f64,
            largest,
            worst: built[least.1].0.base_model.clone(),
            side: SIDES[least.2].to_owned(),
            covered,
            off,
            measured,
            most_off,
            tilt_worst: tilt_worst.0,
            tilt_worst_torso: built[tilt_worst.1].0.base_model.clone(),
            tilt_worst_side: SIDES[tilt_worst.2].to_owned(),
            tilt_mean: tilt_sum / tilts.max(1) as f64,
        });
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
    fn the_naming_flattens_the_torso_path() {
        assert_eq!(
            band_model("character/human/edge/edge_hunter_01_torso_s1", "red"),
            "ported/blackout/character_human_edge_edge_hunter_01_torso_s1_red"
        );
        assert_eq!(
            band_model("ported/duty_026/duty_03_torso_s1", "blue"),
            "ported/blackout/ported_duty_026_duty_03_torso_s1_blue"
        );
    }

    /// A tube of `n` quads around the arm from the shoulder to the elbow,
    /// skinned to bone 7, wound out by `front`.
    fn tube(a: &Arm, radius: f64, n: usize, front: f64) -> Torso {
        let mut positions = vec![];
        let mut normals = vec![];
        for k in 0..n {
            let angle = -PI + TAU * k as f64 / n as f64;
            for s in [0.0, a.length] {
                positions.push(a.point(s, radius, angle));
                normals.push(a.radial(angle));
            }
        }
        let mut triangles = vec![];
        for k in 0..n {
            let (p0, d0) = (2 * k, 2 * k + 1);
            let (p1, d1) = ((2 * k + 2) % (2 * n), (2 * k + 3) % (2 * n));
            for mut t in [[p0, d0, d1], [p0, d1, p1]] {
                let [x, y, z] = t.map(|i| positions[i]);
                let mid: [f64; 3] = std::array::from_fn(|c| (x[c] + y[c] + z[c]) / 3.0);
                if dot(cross(sub(y, x), sub(z, x)), a.outward(mid)) * front < 0.0 {
                    t.swap(1, 2);
                }
                triangles.push(t);
            }
        }
        Torso {
            base_model: "tube".into(),
            skins: vec![vec![(7, 1.0)]; positions.len()],
            positions,
            normals,
            triangles,
            skeleton: vec![],
            bind_pose: vec![],
        }
    }

    #[test]
    fn the_shell_is_the_cut_sleeve_moved_out_and_skinned_as_it() {
        let a = arm();
        let bones: BTreeSet<u8> = [7].into();
        for front in [1.0, -1.0] {
            let t = tube(&a, 0.05, 24, front);
            let s = shell(&t, &a, &bones, front);
            assert!(s.coverage > 0.99, "{}", s.coverage);
            for (p, src) in s.positions.iter().zip(&s.sources) {
                let (ps, pr, _) = a.cylindrical(*p);
                let (ss, _, _) = a.cylindrical(*src);
                assert!((ps - ss).abs() < 1e-9);
                assert!(ps >= WINDOW[0] * a.length - 1e-9 && ps <= WINDOW[1] * a.length + 1e-9);
                // The corners are on the radius; the offset is along the
                // smooth normal.
                assert!((pr - (0.05 + OFFSET)).abs() < 6e-4, "{pr}");
            }
            assert!(s.skins.iter().all(|k| k == &vec![(7, 1.0)]));
            let model = encode(&[(&a, &s)]).unwrap();
            assert_eq!(front_face_sign(&model).unwrap(), front);
        }
    }

    #[test]
    fn an_inner_layer_under_the_sleeve_gets_no_band() {
        let a = arm();
        let bones: BTreeSet<u8> = [7].into();
        let mut t = tube(&a, 0.06, 24, 1.0);
        let inner = tube(&a, 0.045, 24, 1.0);
        let base = t.positions.len();
        t.triangles
            .extend(inner.triangles.iter().map(|tri| tri.map(|i| i + base)));
        t.positions.extend(inner.positions);
        t.normals.extend(inner.normals);
        t.skins.extend(inner.skins);
        let s = shell(&t, &a, &bones, 1.0);
        assert!(!s.sources.is_empty());
        assert!(s.sources.iter().all(|p| a.cylindrical(*p).1 > 0.055));
    }

    #[test]
    fn no_arm_weight_no_band() {
        let a = arm();
        let t = tube(&a, 0.05, 24, 1.0);
        let s = shell(&t, &a, &[3].into(), 1.0);
        assert!(s.triangles.is_empty() && s.coverage == 0.0);
    }

    #[test]
    fn worn_cloth_meets_itself_around_the_arm() {
        for cell in [2, 8, 32, 64, 128, 256] {
            for x in [0, 17, 127] {
                assert_eq!(noise(x, 0, cell, 9), noise(x, BAND_MAP[1], cell, 9));
            }
        }
        let c = [0.2, 0.05, 0.03];
        assert_eq!(wear(c, 5, 0), wear(c, 5, BAND_MAP[1]));
    }

    #[test]
    fn square_skins_are_the_upper_arm_alone_and_the_same_round_a_ring() {
        let a = arm();
        let mut t = tube(&a, 0.05, 24, 1.0);
        // Round the arm the sleeve leans on other bones: the top on the
        // shoulder (9), the inside on the chest (4).
        for (i, p) in t.positions.clone().iter().enumerate() {
            let (_, _, angle) = a.cylindrical(*p);
            t.skins[i] = if angle > 1.0 {
                vec![(7, 0.5), (8, 0.3), (9, 0.2)]
            } else if angle < -1.0 {
                vec![(4, 0.2), (7, 0.6), (8, 0.2)]
            } else {
                vec![(7, 0.7), (8, 0.3)]
            };
        }
        let mut s = shell(&t, &a, &[7, 8, 9].into(), 1.0);
        assert_eq!(s.skins, s.source_skins);
        s.skins = square_skins(&s, &a, [7, 8]);
        for (skin, e) in s.skins.iter().zip(&s.edges) {
            assert!(skin.iter().all(|(b, _)| *b == 7 || *b == 8), "{skin:?}");
            assert!((skin.iter().map(|x| x.1).sum::<f64>() - 1.0).abs() < 1e-9);
            if e.is_some() {
                let first = &s.skins[s.edges.iter().position(|x| x == e).unwrap()];
                assert_eq!(skin, first);
            }
        }
    }

    #[test]
    fn a_ring_square_to_the_arm_has_the_arm_for_its_normal() {
        let a = arm();
        let ring: Vec<[f64; 3]> = (0..12)
            .map(|k| a.point(0.1, 0.05, k as f64 * TAU / 12.0))
            .collect();
        let n = plane_normal(&ring).unwrap();
        assert!(dot(n, a.dir).abs() > 1.0 - 1e-9);
    }

    #[test]
    fn a_cut_blends_the_skins_of_its_edge() {
        let s = blend(&vec![(1, 1.0)], &vec![(2, 1.0)], 0.25);
        assert_eq!(s, vec![(1, 0.75), (2, 0.25)]);
    }
}
