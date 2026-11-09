// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether a player face fits under everything worn with it, and the fix
//! when it does not.
//!
//! 0.26g draws one head under every item that leaves it (no
//! `remove_head_model`), and every such item was made over that head
//! (`black_market_01_head_s`). A face of its own (`faces_026`) is drawn in
//! three states: bare, under a helmet (its helmet variant) and under a mask
//! (its mask variant: a mask wins over a helmet), always over a torso and
//! a back piece. What it may show through is measured, as `mask_fit`
//! measures cloth over the head, along rays out of the posed skull:
//!
//! - **Rays.** Through each sample of the face (its vertices and triangle
//!   middles): from the middle of the skull at the eyes' height (skinned to
//!   Head), or horizontally from the neck's axis (Neck to Head) at the
//!   sample's height, whichever crosses the surface more squarely; below the
//!   native chin (less [`CHIN_BAND`]) always the neck's, so a collar is met
//!   where it stands round the neck and not far down the chest.
//! - **An item's skin** along a ray is the first surface of it facing out
//!   beyond the inside of the head (`FLOOR_DEPTH` under the native head, or
//!   the face where the native head is not on the ray): a helmet's liner
//!   faces in and is passed, goggles stacked on a shell are beyond it. Worn
//!   together (a helmet over a mask), the outermost of the items' skins
//!   covers.
//! - **The rule.** Where an item covers a ray, the face stays `clearance`
//!   inside its skin, or no farther out than the native head is along the
//!   same ray (`NATIVE_TOLERANCE`): the native head is what every live item
//!   was made over, so where the item already touches it the face may touch
//!   it as closely, and no closer. Where the native head itself shows
//!   through the items in a pose, a face may show through them as deep in
//!   the same region of the head ([`REGIONS`]).
//! - **Poses.** The bind pose and [`crate::bandit_026::SampledPoses`]: the
//!   face, the native head and every item skinned through each, the bones
//!   of an item's own bind pose found by name on the native rig (a bone the
//!   rig lacks, as the render bone `headwear-1` gives helmets, follows Head).
//!
//! The fix ([`pull_in`]) moves the face's vertices in along their rays by
//! what the measure says each needs, spread over the neighbouring rings so
//! the surface stays smooth, round after round until every pose passes.
use crate::bandit_026::SampledPoses;
use crate::mask_fit::{Frame, Skin, dot, parse_mesh, read_u32, skin_positions, skins, sub};
use crate::weapon::parse_config;
use crate::{ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Least gap kept under an item's skin in the bind pose (m).
pub const BIND_CLEARANCE: f64 = 0.001;
/// Least gap kept under an item's skin in a sampled pose (m), as the
/// balaclavas' posed gate (`bandit_026::POSED_CLEARANCE`).
pub const POSED_CLEARANCE: f64 = 0.0002;
/// How far beyond the native head the face may come where an item leaves
/// the native head less than the clearance (m).
pub const NATIVE_TOLERANCE: f64 = 0.0002;
/// An item surface this far inside the native head (or the face, where the
/// native head is not) is the
/// item's own inside (a mask's rolled eye holes, a liner), not its skin.
const FLOOR_DEPTH: f64 = 0.01;
/// How far beyond a sample an item skin is looked for (m): one farther out
/// covers nothing the sample could show through.
const REACH: f64 = 0.06;
/// How far beyond a sample the native head is looked for (m).
const NATIVE_REACH: f64 = 0.08;
/// The part of an item that can meet a face (bind pose, m): items are cut
/// to the triangles with a corner in this box before they are posed.
const ZONE_MIN: [f64; 3] = [-0.24, 1.30, -0.26];
const ZONE_MAX: [f64; 3] = [0.24, 1.95, 0.30];
/// Regions of the head ([`Rig::region`]): where the native head shows
/// through an item in a pose, a face may show through it as far there.
const REGIONS: usize = 12 * 8;
/// How far below the native chin a face's ray may still start in the skull (m):
/// a lower chin than the native one is pulled up into it, not back to the neck.
const CHIN_BAND: f64 = 0.025;
/// The most the fix pulls one vertex in a round (m), and in all (m).
const PULL_STEP: f64 = 0.004;
pub const PULL_MAX: f64 = 0.045;
/// Rings of neighbours a pull is spread over, and how much of it each
/// ring keeps.
const SPREAD_RINGS: usize = 4;
const SPREAD_KEEP: f64 = 0.6;
/// Rounds of the fix before giving up.
const PULL_ROUNDS: usize = 16;

/// The native rig: the native head's bind pose, by name.
pub struct Rig {
    names: BTreeMap<String, usize>,
    head: usize,
    neck: usize,
    head_at: [f64; 3],
    neck_at: [f64; 3],
    /// The middle of the skull at the eyes' height.
    skull: [f64; 3],
    /// The native chin's lowest point's height: below it a ray from the
    /// skull leaves the native head through its open neck.
    chin: f64,
}

impl Rig {
    pub fn new(bind_pose: &[u8], native_head: &[u8]) -> Result<Self> {
        let origins = crate::armor::bind_pose_origins(bind_pose, "native head bind pose")?;
        let names: BTreeMap<String, usize> = origins
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.clone(), i))
            .collect();
        let at = |n: &str| -> Result<[f64; 3]> {
            let i = *names.get(n).with_context(|| format!("no {n} bone"))?;
            Ok(origins[i].1.map(f64::from))
        };
        let (head_at, neck_at) = (at("Head")?, at("Neck")?);
        let eyes_y = (at("LeftEye")?[1] + at("RightEye")?[1]) / 2.0;
        let mesh = parse_mesh(native_head, 40, "native head")?;
        let (lo, hi) = bounds(&mesh.positions);
        let chin = mesh
            .positions
            .iter()
            .filter(|p| p[2] > hi[2] * 0.35)
            .map(|p| p[1])
            .fold(f64::INFINITY, f64::min);
        Ok(Self {
            chin,
            head: names["Head"],
            neck: names["Neck"],
            names,
            head_at,
            neck_at,
            skull: [(lo[0] + hi[0]) / 2.0, eyes_y, (lo[2] + hi[2]) / 2.0],
        })
    }

    /// The Head bone's height in the bind pose.
    pub fn head_y(&self) -> f64 {
        self.head_at[1]
    }

    /// Where the ray through a bind-pose point with outward normal `n`
    /// starts, and that centre's skin: the skull's middle, or below the
    /// eyes the neck's axis at the point's height, whichever ray crosses
    /// the surface more squarely (the chin's underside looks down, to the
    /// skull; the neck looks sideways, to its axis).
    fn centre(&self, p: [f64; 3], n: [f64; 3]) -> ([f64; 3], Skin) {
        let skull = (self.skull, vec![(self.head as u8, 1.0)]);
        if p[1] >= self.skull[1] {
            return skull;
        }
        let neck = self.neck_centre(p);
        if p[1] < self.chin - CHIN_BAND {
            return neck;
        }
        let square = |c: [f64; 3]| {
            let d = sub(p, c);
            dot(d, n) / dot(d, d).sqrt().max(1e-9)
        };
        if square(neck.0) > square(skull.0) {
            neck
        } else {
            skull
        }
    }

    /// Which of [`REGIONS`] a bind-pose point is in: its direction from the
    /// skull's middle, in 30 degree slices round the head and 22.5 degree
    /// bands from below to above.
    fn region(&self, p: [f64; 3]) -> usize {
        let d = unit(sub(p, self.skull));
        let around = (d[0].atan2(d[2]).to_degrees() + 180.0) / 30.0;
        let up = (d[1].clamp(-1.0, 1.0).asin().to_degrees() + 90.0) / 22.5;
        (around as usize).min(11) * 8 + (up as usize).min(7)
    }

    fn neck_centre(&self, p: [f64; 3]) -> ([f64; 3], Skin) {
        let s = (p[1] - self.neck_at[1]) / (self.head_at[1] - self.neck_at[1]);
        let z = self.neck_at[2] + s * (self.head_at[2] - self.neck_at[2]);
        let w = s.clamp(0.0, 1.0);
        let skin = if w <= 0.0 {
            vec![(self.neck as u8, 1.0)]
        } else {
            vec![(self.neck as u8, 1.0 - w), (self.head as u8, w)]
        };
        ([self.neck_at[0], p[1], z], skin)
    }

    /// A model's bones on this rig, by name; a bone the rig lacks follows
    /// Head (the helmets' render bone).
    fn map_bones(&self, bind_pose: &[u8], label: &str) -> Result<Vec<u8>> {
        let own = crate::armor::bind_pose_origins(bind_pose, label)?;
        Ok(own
            .iter()
            .map(|(n, _)| *self.names.get(n).unwrap_or(&self.head) as u8)
            .collect())
    }
}

fn bounds(points: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// A surface ready to pose: bind-pose positions, triangles and skins on
/// the native rig.
#[derive(Clone)]
pub struct Posable {
    pub label: String,
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    skins: Vec<Skin>,
}

impl Posable {
    /// A converted model (any stride) skinned on `bind_pose`'s bones.
    pub fn parse(rig: &Rig, label: &str, converted: &[u8], bind_pose: &[u8]) -> Result<Self> {
        let count = read_u32(converted, 8)?;
        ensure!(count > 0, "{label}: no vertices");
        let stride = (read_u32(converted, 4)? - 4) / count;
        let mesh = parse_mesh(converted, stride, label)?;
        let map = rig.map_bones(bind_pose, label)?;
        let skins = skins(converted, stride)?
            .into_iter()
            .map(|s| {
                let mut merged: BTreeMap<u8, f64> = BTreeMap::new();
                for (b, w) in s {
                    let to = *map
                        .get(usize::from(b))
                        .with_context(|| format!("{label}: bone {b} outside its bind pose"))?;
                    *merged.entry(to).or_default() += w;
                }
                Ok(merged.into_iter().collect())
            })
            .collect::<Result<Vec<Skin>>>()?;
        Ok(Self {
            label: label.to_owned(),
            positions: mesh.positions,
            triangles: mesh.triangles,
            skins,
        })
    }

    /// Only the triangles with a corner in the zone a face can meet.
    fn near_head(mut self) -> Self {
        let inside = |p: &[f64; 3]| (0..3).all(|k| p[k] >= ZONE_MIN[k] && p[k] <= ZONE_MAX[k]);
        self.triangles
            .retain(|t| t.iter().any(|&v| inside(&self.positions[v])));
        self
    }

    fn posed(&self, bind: &[Frame], pose: &[Frame]) -> Result<Vec<[f64; 3]>> {
        skin_positions(&self.positions, &self.skins, bind, pose)
    }
}

/// A triangle soup with a bounding volume hierarchy, each triangle tagged
/// with the item it belongs to.
struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
}

#[derive(Clone, Copy)]
struct Tri {
    a: [f64; 3],
    e1: [f64; 3],
    e2: [f64; 3],
    item: u32,
}

#[derive(Clone, Copy)]
struct Node {
    lo: [f64; 3],
    hi: [f64; 3],
    /// A leaf's first triangle, or an inner node's right child (the left
    /// one follows it).
    start: u32,
    count: u32,
}

impl Bvh {
    fn new(mut tris: Vec<Tri>) -> Self {
        let mut nodes = Vec::with_capacity(tris.len() / 2 + 1);
        if !tris.is_empty() {
            let n = tris.len();
            build(&mut nodes, &mut tris, 0, n);
        }
        Self { nodes, tris }
    }

    /// Every hit of the ray `o + t d` with `lo < t <= hi`, as (t, facing
    /// out along the ray, item).
    fn hits(&self, o: [f64; 3], d: [f64; 3], lo: f64, hi: f64, mut f: impl FnMut(f64, bool, u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let inv = d.map(|v| {
            if v.abs() < 1e-12 {
                1e12_f64.copysign(v)
            } else {
                1.0 / v
            }
        });
        let mut stack = vec![0_usize];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            let (mut t0, mut t1) = (lo.max(0.0), hi);
            for k in 0..3 {
                let a = (n.lo[k] - o[k]) * inv[k];
                let b = (n.hi[k] - o[k]) * inv[k];
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
            if t0 > t1 + 1e-9 {
                continue;
            }
            if n.count > 0 {
                for tri in &self.tris[n.start as usize..(n.start + n.count) as usize] {
                    let p = cross3(d, tri.e2);
                    let det = dot(tri.e1, p);
                    if det.abs() < 1e-14 {
                        continue;
                    }
                    let tv = sub(o, tri.a);
                    let u = dot(tv, p) / det;
                    if !(-1e-7..=1.0 + 1e-7).contains(&u) {
                        continue;
                    }
                    let q = cross3(tv, tri.e1);
                    let v = dot(d, q) / det;
                    if v < -1e-7 || u + v > 1.0 + 1e-7 {
                        continue;
                    }
                    let t = dot(tri.e2, q) / det;
                    if t > lo.max(0.0) && t <= hi {
                        f(t, dot(cross3(tri.e1, tri.e2), d) > 0.0, tri.item);
                    }
                }
            } else {
                stack.push(i + 1);
                stack.push(n.start as usize);
            }
        }
    }
}

fn triangle_normal([a, b, c]: [[f64; 3]; 3]) -> [f64; 3] {
    cross3(sub(b, a), sub(c, a))
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = dot(v, v).sqrt();
    if l > 1e-12 { v.map(|x| x / l) } else { v }
}

/// Outward vertex normals: the area-weighted normals of the triangles
/// round each place (seam copies together).
fn vertex_normals(positions: &[[f64; 3]], triangles: &[[usize; 3]]) -> Vec<[f64; 3]> {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
    let mut at: HashMap<[i64; 3], [f64; 3]> = HashMap::new();
    for t in triangles {
        let n = triangle_normal(t.map(|v| positions[v]));
        for &v in t {
            let e = at.entry(key(positions[v])).or_insert([0.0; 3]);
            for k in 0..3 {
                e[k] += n[k];
            }
        }
    }
    positions
        .iter()
        .map(|p| unit(at.get(&key(*p)).copied().unwrap_or([0.0; 3])))
        .collect()
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn build(nodes: &mut Vec<Node>, tris: &mut [Tri], start: usize, end: usize) -> usize {
    let corners = |t: &Tri| {
        [
            t.a,
            [0, 1, 2].map(|k| t.a[k] + t.e1[k]),
            [0, 1, 2].map(|k| t.a[k] + t.e2[k]),
        ]
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let (mut clo, mut chi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for t in &tris[start..end] {
        let c = corners(t);
        for p in c {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        for k in 0..3 {
            let m = (c[0][k] + c[1][k] + c[2][k]) / 3.0;
            clo[k] = clo[k].min(m);
            chi[k] = chi[k].max(m);
        }
    }
    let at = nodes.len();
    nodes.push(Node {
        lo,
        hi,
        start: start as u32,
        count: (end - start) as u32,
    });
    if end - start <= 4 {
        return at;
    }
    let axis = (0..3)
        .max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b])))
        .unwrap();
    let mid = (start + end) / 2;
    let key = |t: &Tri| t.a[axis] + (t.e1[axis] + t.e2[axis]) / 3.0;
    tris[start..end].select_nth_unstable_by(mid - start, |a, b| key(a).total_cmp(&key(b)));
    build(nodes, tris, start, mid);
    let right = build(nodes, tris, mid, end);
    nodes[at].start = right as u32;
    nodes[at].count = 0;
    at
}

fn soup(positions: &[[f64; 3]], triangles: &[[usize; 3]], item: u32, out: &mut Vec<Tri>) {
    for t in triangles {
        let [a, b, c] = t.map(|v| positions[v]);
        out.push(Tri {
            a,
            e1: sub(b, a),
            e2: sub(c, a),
            item,
        });
    }
}

/// Which slot an item covering the head is worn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// A torso (one of its sleeve variants) or a back piece: worn in every
    /// state.
    Body,
    /// A helmet; `with_masks`: it leaves the mask slot free.
    Helmet { with_masks: bool },
    /// A mask; `with_helmets`: it leaves the helmet slot free.
    Mask { with_helmets: bool },
}

/// One item as the face meets it: every surface it is drawn with.
pub struct Cover {
    pub label: String,
    pub slot: Slot,
    pub surfaces: Vec<Posable>,
}

/// What a face is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// No helmet and no mask: the base face.
    Bare,
    /// A helmet and no mask: the helmet variant.
    Helmet,
    /// A mask (and maybe a helmet over it): the mask variant.
    Mask,
}

impl State {
    pub const ALL: [Self; 3] = [Self::Bare, Self::Helmet, Self::Mask];
}

/// Items worn together, for one state.
#[derive(Clone, Debug, Serialize)]
pub struct Worn {
    pub label: String,
    pub covers: Vec<usize>,
}

/// The items of the released game a face meets, and what is worn together
/// in each state.
pub struct Wardrobe {
    pub covers: Vec<Cover>,
    pub worn: BTreeMap<State, Vec<Worn>>,
    /// The items left out because they draw no head (`remove_head_model`).
    pub headless: Vec<String>,
}

impl Wardrobe {
    /// Every helmet, mask, torso (each distinct sleeve variant) and back
    /// piece of the game's item dictionary, as `view` (the stock database
    /// and the released patches) draws them.
    pub fn read(view: &mut ResourceDb, rig: &Rig) -> Result<Self> {
        let dictionary = parse_config(view, "gameplay/db_static_dictionaries", "dictionary")?;
        let Some(V::Indexed(items)) = dictionary.root.named("items_dict") else {
            bail!("the dictionary has no items_dict");
        };
        let mut configs = BTreeSet::new();
        for row in items {
            if let Some(V::String(cfg)) = row.named("cfg_name") {
                for slot in ["helmet", "mask", "torso", "back"] {
                    if cfg.starts_with(&format!("gameplay/items/armour/{slot}/")) {
                        configs.insert(cfg.clone());
                    }
                }
            }
        }
        let mut covers: Vec<Cover> = vec![];
        let mut headless = vec![];
        // Torso variants that meet the face with the same triangles.
        let mut seen_body: HashMap<Vec<u64>, String> = HashMap::new();
        for path in configs {
            let category = path.split('/').nth(3).context("item path")?.to_owned();
            let config = parse_config(view, &path, "item")?;
            let name = path
                .trim_start_matches("gameplay/items/armour/")
                .trim_end_matches(".options")
                .to_owned();
            let data = config.root.named("data").context("item without data")?;
            if data.named("remove_head_model") == Some(&V::Boolean(true)) {
                headless.push(name);
                continue;
            }
            let takes = |slot: &str| {
                matches!(
                    data.named("additional_model_slots"),
                    Some(V::Indexed(s)) if s.iter().any(|v| matches!(v, V::String(x) if x == slot))
                )
            };
            let (slot, keys): (Slot, Vec<&str>) = match category.as_str() {
                "helmet" => (
                    Slot::Helmet {
                        with_masks: !takes("mask"),
                    },
                    vec!["data"],
                ),
                "mask" => (
                    Slot::Mask {
                        with_helmets: !takes("helmet"),
                    },
                    vec!["data"],
                ),
                "torso" => (
                    Slot::Body,
                    vec!["data", "data_full", "data_short", "data_long"],
                ),
                _ => (Slot::Body, vec!["data"]),
            };
            let mut variants: Vec<(String, Vec<Posable>)> = vec![];
            let mut models_seen = BTreeSet::new();
            for key in keys {
                let Some(part) = config.root.named(key) else {
                    continue;
                };
                let mut parts = vec![part];
                if let Some(V::Indexed(extra)) = part.named("additional_models") {
                    parts.extend(extra);
                }
                let mut surfaces = vec![];
                let mut ids = vec![];
                for part in parts {
                    let (Some(V::String(model)), Some(V::String(surface))) =
                        (part.named("base_model"), part.named("part_name"))
                    else {
                        bail!("{path} {key}: a model part without base_model/part_name");
                    };
                    ids.push(format!("{model}/{surface}"));
                    let root = format!("models/{model}.skinned_model/render");
                    let bytes = view
                        .read(&format!("{root}/{surface}/converted_model"))
                        .with_context(|| format!("{path}: {model} {surface}"))?;
                    let bind = view.read(&format!("{root}/bind_pose"))?;
                    let label = format!("{name} {surface}");
                    surfaces.push(Posable::parse(rig, &label, &bytes, &bind)?.near_head());
                }
                ids.sort();
                if !models_seen.insert(ids) {
                    continue;
                }
                surfaces.retain(|s| !s.triangles.is_empty());
                let label = if key == "data" {
                    name.clone()
                } else {
                    format!("{name} ({key})")
                };
                variants.push((label, surfaces));
            }
            for (label, surfaces) in variants {
                if surfaces.is_empty() {
                    continue;
                }
                if slot == Slot::Body {
                    // Sleeve variants share their collar: keep one of each.
                    let mut key = vec![];
                    for s in &surfaces {
                        for t in &s.triangles {
                            for &v in t {
                                for c in s.positions[v] {
                                    key.push((c * 1e5).round() as i64 as u64);
                                }
                            }
                        }
                    }
                    if seen_body.contains_key(&key) {
                        continue;
                    }
                    seen_body.insert(key, label.clone());
                }
                covers.push(Cover {
                    label,
                    slot,
                    surfaces,
                });
            }
        }
        let mut worn: BTreeMap<State, Vec<Worn>> = BTreeMap::new();
        let one = |i: usize, c: &Cover| Worn {
            label: c.label.clone(),
            covers: vec![i],
        };
        for (i, c) in covers.iter().enumerate() {
            match c.slot {
                Slot::Body => {
                    for s in State::ALL {
                        worn.entry(s).or_default().push(one(i, c));
                    }
                }
                Slot::Helmet { .. } => worn.entry(State::Helmet).or_default().push(one(i, c)),
                Slot::Mask { .. } => worn.entry(State::Mask).or_default().push(one(i, c)),
            }
        }
        for (h, helmet) in covers.iter().enumerate() {
            let Slot::Helmet { with_masks: true } = helmet.slot else {
                continue;
            };
            for (m, mask) in covers.iter().enumerate() {
                if let Slot::Mask { with_helmets: true } = mask.slot {
                    worn.entry(State::Mask).or_default().push(Worn {
                        label: format!("{} + {}", helmet.label, mask.label),
                        covers: vec![h, m],
                    });
                }
            }
        }
        ensure!(
            covers.iter().any(|c| matches!(c.slot, Slot::Helmet { .. }))
                && covers.iter().any(|c| matches!(c.slot, Slot::Mask { .. }))
                && covers.iter().any(|c| c.slot == Slot::Body),
            "the game has no helmet, mask or torso to fit under"
        );
        Ok(Self {
            covers,
            worn,
            headless,
        })
    }

    pub fn count(&self, slot: fn(&Slot) -> bool) -> usize {
        self.covers.iter().filter(|c| slot(&c.slot)).count()
    }
}

/// The face surfaces drawn in one state, as 48-byte converted models
/// (the face, then its hair if any).
#[derive(Clone)]
pub struct Drawn {
    pub label: String,
    pub surfaces: Vec<(String, Vec<u8>)>,
    /// Per surface, per vertex: the ray it is measured and pulled along,
    /// fixed from the donor's model ([`Drawn::freeze`]) so the fix moves
    /// every vertex along the ray that measured it, and a face pulled in
    /// never finds deeper insides of an item. Empty: from the surfaces as
    /// they are.
    pub rays: Vec<Vec<Ray>>,
}

/// A vertex's ray: where it starts (bind pose) and its skin, its direction,
/// and how deep under the vertex as the donor made it an item surface is
/// the item's inside where the native head is not on the ray.
#[derive(Clone, Debug)]
pub struct Ray {
    centre: [f64; 3],
    skin: Skin,
    dir: [f64; 3],
    floor: f64,
    region: usize,
}

impl Drawn {
    pub fn new(label: String, surfaces: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            label,
            surfaces,
            rays: vec![],
        }
    }

    /// Fixes every vertex's ray from the surfaces as they are now.
    pub fn freeze(&mut self, rig: &Rig) -> Result<()> {
        if !self.rays.is_empty() {
            return Ok(());
        }
        self.rays = self
            .surfaces
            .iter()
            .map(|(label, bytes)| {
                let mesh = parse_mesh(bytes, 48, label)?;
                let normals = vertex_normals(&mesh.positions, &mesh.triangles);
                Ok(mesh
                    .positions
                    .iter()
                    .zip(normals)
                    .map(|(&p, n)| {
                        let (centre, skin) = rig.centre(p, n);
                        let d = sub(p, centre);
                        Ray {
                            centre,
                            skin,
                            dir: unit(d),
                            floor: dot(d, d).sqrt() - FLOOR_DEPTH,
                            region: rig.region(p),
                        }
                    })
                    .collect())
            })
            .collect::<Result<_>>()?;
        Ok(())
    }

    /// The first `n` surfaces (the face without its hair), with their rays.
    pub fn first(&self, n: usize, label: String) -> Self {
        Self {
            label,
            surfaces: self.surfaces[..n].to_vec(),
            rays: self.rays.iter().take(n).cloned().collect(),
        }
    }
}


/// The samples of a face the rays go through: its vertices and its
/// triangles' middles, each with its skin and its ray's bind-pose centre.
struct Samples {
    bind: Vec<[f64; 3]>,
    skins: Vec<Skin>,
        centres: Vec<([f64; 3], Skin)>,
    /// Item surfaces nearer the centre than this are the item's inside
    /// where the native head is not on the sample's ray.
    floors: Vec<f64>,
    /// For each sample, the vertices (surface, vertex) it moves and how
    /// much of its pull each takes.
    corners: Vec<Vec<(usize, usize, f64)>>,
    /// Each sample's region ([`Rig::region`]).
    regions: Vec<usize>,
}

impl Samples {
    fn new(rig: &Rig, drawn: &Drawn, native_bind_pose: &[u8]) -> Result<Self> {
        let surfaces = drawn
            .surfaces
            .iter()
                        .map(|(label, bytes)| Posable::parse(rig, label, bytes, native_bind_pose))
            .collect::<Result<Vec<_>>>()?;
        let mut drawn = drawn.clone();
        drawn.freeze(rig)?;
        Ok(Self::of(&surfaces, &drawn.rays))
    }

    /// The samples of `surfaces`, each vertex on its ray of `rays`; a
    /// triangle's middle on its corners' (their centres averaged).
    fn of(surfaces: &[Posable], rays: &[Vec<Ray>]) -> Self {
        let mut out = Self {
            bind: vec![],
            skins: vec![],
            centres: vec![],
            floors: vec![],
            corners: vec![],
            regions: vec![],
        };
        for (s, p) in surfaces.iter().enumerate() {
            let rays = &rays[s];
            for (v, (pos, skin)) in p.positions.iter().zip(&p.skins).enumerate() {
                let r = &rays[v];
                out.centres.push((r.centre, r.skin.clone()));
                out.floors.push(r.floor);
                out.regions.push(r.region);
                out.bind.push(*pos);
                out.skins.push(skin.clone());
                out.corners.push(vec![(s, v, 1.0)]);
            }
            for t in &p.triangles {
                let pos =
                    [0, 1, 2].map(|k| t.iter().map(|&v| p.positions[v][k]).sum::<f64>() / 3.0);
                let mut w: BTreeMap<u8, f64> = BTreeMap::new();
                for &v in t {
                    for &(b, x) in &p.skins[v] {
                        *w.entry(b).or_default() += x / 3.0;
                    }
                }
                let r = &rays[t[0]];
                let centre = [0, 1, 2].map(|k| t.iter().map(|&v| rays[v].centre[k]).sum::<f64>() / 3.0);
                out.centres.push((centre, r.skin.clone()));
                out.floors.push(t.iter().map(|&v| rays[v].floor).fold(f64::INFINITY, f64::min));
                out.regions.push(r.region);
                out.bind.push(pos);
                out.skins.push(w.into_iter().collect());
                out.corners.push(t.iter().map(|&v| (s, v, 0.75)).collect());
            }
        }
        out
    }

    /// The native head's samples, on rays from its own surface.
    fn native(rig: &Rig, native: &Posable) -> Self {
        let normals = vertex_normals(&native.positions, &native.triangles);
        let rays: Vec<Ray> = native
            .positions
            .iter()
            .zip(normals)
            .map(|(&p, n)| {
                let (centre, skin) = rig.centre(p, n);
                let d = sub(p, centre);
                Ray {
                    centre,
                    skin,
                    dir: unit(d),
                    floor: dot(d, d).sqrt() - FLOOR_DEPTH,
                    region: rig.region(p),
                }
            })
            .collect();
        Self::of(std::slice::from_ref(native), &[rays])
    }
}

/// The worst a face does under one set of items, over every sampled pose.
#[derive(Clone, Debug, Serialize)]
pub struct Measure {
    pub worn: String,
    /// The least margin (m): how far inside what the rule allows the face
    /// stays; negative where it shows through.
    pub margin: f64,
    /// The least gap under the items' skin (m), and the native head's own
    /// on the same rays.
    pub gap: f64,
    pub native_gap: f64,
    /// The deepest the native head itself shows through these items in a
    /// region of the head (m): the allowance there.
    pub native_through: f64,
    /// Sample-poses through, and where the worst one is (bind pose; the
    /// face looks to +z, -x is the character's left), in which pose.
    pub through: usize,
    pub at: [f64; 3],
    pub pose: String,
}

impl Measure {
    fn new(worn: &str) -> Self {
        Self {
            worn: worn.to_owned(),
            margin: f64::INFINITY,
            gap: f64::INFINITY,
            native_gap: f64::INFINITY,
            native_through: 0.0,
            through: 0,
            at: [0.0; 3],
            pose: String::new(),
        }
    }
    fn merge(&mut self, o: &Self) {
        self.through += o.through;
        self.gap = self.gap.min(o.gap);
        self.native_gap = self.native_gap.min(o.native_gap);
        self.native_through = self.native_through.max(o.native_through);
        if o.margin < self.margin {
            self.margin = o.margin;
            self.at = o.at;
            self.pose.clone_from(&o.pose);
        }
    }
}

/// One drawn face to measure: in which state, and against which worn sets.
pub struct Job<'a> {
    pub drawn: &'a Drawn,
    pub state: State,
}

/// What a sweep over the poses found for one job.
pub struct Sweep {
    pub measures: Vec<Measure>,
    /// Per surface, per vertex: the pull the worst pose asks for (m).
        pub need: Vec<Vec<f64>>,
}

/// Measures every job in every pose (the bind pose first).
pub fn sweep(
    rig: &Rig,
    poses: &SampledPoses,
    native: &Posable,
    wardrobe: &Wardrobe,
    jobs: &[Job<'_>],
) -> Result<Vec<Sweep>> {
    let samples: Vec<Samples> = jobs
        .iter()
        .map(|j| Samples::new(rig, j.drawn, &poses.bind_pose))
        .collect::<Result<_>>()?;
        let native_samples = Samples::native(rig, native);
    let mut labels = vec!["bind pose".to_owned()];
    for (name, clip) in &poses.clips {
        for k in 0..clip.len() {
            labels.push(format!("{name} #{k}"));
        }
    }
    let started = std::time::Instant::now();
    // FACES_BIND_ONLY: the bind pose alone, while looking into a fit.
    let all: Vec<&[Frame]> = if std::env::var_os("FACES_BIND_ONLY").is_some() {
        poses.all().take(1).collect()
    } else {
        poses.all().collect()
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = all.len().div_ceil(threads);
    let parts: Vec<Vec<Sweep>> = std::thread::scope(|scope| {
        let workers: Vec<_> = all
            .chunks(chunk)
            .enumerate()
            .map(|(c, part)| {
                let (samples, labels, native_samples) = (&samples, &labels, &native_samples);
                scope.spawn(move || -> Result<Vec<Sweep>> {
                    let mut acc: Option<Vec<Sweep>> = None;
                    for (k, pose) in part.iter().enumerate() {
                        let index = c * chunk + k;
                        let clearance = if index == 0 {
                            BIND_CLEARANCE
                        } else {
                            POSED_CLEARANCE
                        };
                        let one = measure_pose(
                            &poses.bind,
                            pose,
                            native,
                            wardrobe,
                            jobs,
                            samples,
                            native_samples,
                            clearance,
                            &labels[index],
                        )?;
                        match &mut acc {
                            None => acc = Some(one),
                            Some(a) => merge_sweeps(a, &one),
                        }
                    }
                    acc.context("no pose")
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
    })?;
    let mut parts = parts.into_iter();
    let mut out = parts.next().context("no pose")?;
    for p in parts {
        merge_sweeps(&mut out, &p);
    }
    println!(
        "    swept {} faces through {} poses in {:.0} s",
        jobs.len(),
        all.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(out)
}

fn merge_sweeps(into: &mut [Sweep], from: &[Sweep]) {
    for (a, b) in into.iter_mut().zip(from) {
        for (m, n) in a.measures.iter_mut().zip(&b.measures) {
            m.merge(n);
        }
        for (x, y) in a.need.iter_mut().zip(&b.need) {
            for (p, q) in x.iter_mut().zip(y) {
                *p = p.max(*q);
            }
        }
        
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_pose(
    bind: &[Frame],
    pose: &[Frame],
    native: &Posable,
    wardrobe: &Wardrobe,
    jobs: &[Job<'_>],
    samples: &[Samples],
    native_samples: &Samples,
    clearance: f64,
    label: &str,
) -> Result<Vec<Sweep>> {
    let mut tris = vec![];
    soup(&native.posed(bind, pose)?, &native.triangles, 0, &mut tris);
    let head = Bvh::new(tris);
    let mut tris = vec![];
    for (i, c) in wardrobe.covers.iter().enumerate() {
        for s in &c.surfaces {
            soup(&s.posed(bind, pose)?, &s.triangles, i as u32, &mut tris);
        }
    }
    let items = Bvh::new(tris);
    let n = wardrobe.covers.len();
    // How deep the native head shows through each worn set in each region
    // in this pose.
    let mut allowance: BTreeMap<State, Vec<[f64; REGIONS]>> = wardrobe
        .worn
        .iter()
        .map(|(s, w)| (*s, vec![[0.0; REGIONS]; w.len()]))
        .collect();
    {
        let posed = skin_positions(&native_samples.bind, &native_samples.skins, bind, pose)?;
        let (centres, centre_skins): (Vec<[f64; 3]>, Vec<Skin>) =
            native_samples.centres.iter().cloned().unzip();
        let posed_centres = skin_positions(&centres, &centre_skins, bind, pose)?;
        let mut cover = vec![f64::INFINITY; n];
        for (s, (&p, &c)) in posed.iter().zip(&posed_centres).enumerate() {
            let d = sub(p, c);
            let r = dot(d, d).sqrt();
            if r < 1e-6 {
                continue;
            }
            let dir = d.map(|v| v / r);
            cover.fill(f64::INFINITY);
            items.hits(c, dir, r - FLOOR_DEPTH, r + REACH, |t, out, item| {
                if out {
                    let x = &mut cover[item as usize];
                    *x = x.min(t);
                }
            });
            let region = native_samples.regions[s];
            for (state, worn) in &wardrobe.worn {
                let allow = allowance.get_mut(state).unwrap();
                for (w, a) in worn.iter().zip(allow.iter_mut()) {
                    let skin = w
                        .covers
                        .iter()
                        .map(|&i| cover[i])
                        .filter(|t| t.is_finite())
                        .fold(f64::NEG_INFINITY, f64::max);
                    if skin.is_finite() {
                        a[region] = a[region].max(r - (skin - clearance));
                    }
                }
            }
        }
    }
    let mut out = vec![];
    for (job, samples) in jobs.iter().zip(samples) {
        let worn = &wardrobe.worn[&job.state];
        let allow = &allowance[&job.state];
        let mut measures: Vec<Measure> = worn.iter().map(|w| Measure::new(&w.label)).collect();
        let mut need: Vec<Vec<f64>> = job
            .drawn
            .surfaces
            .iter()
            .map(|(_, b)| read_u32(b, 8).map(|c| vec![0.0; c]))
            .collect::<Result<_>>()?;
        
        let posed = skin_positions(&samples.bind, &samples.skins, bind, pose)?;
        let (centres, centre_skins): (Vec<[f64; 3]>, Vec<Skin>) =
            samples.centres.iter().cloned().unzip();
        let posed_centres = skin_positions(&centres, &centre_skins, bind, pose)?;
        let mut cover = vec![f64::INFINITY; n];
        for (s, (&p, &c)) in posed.iter().zip(&posed_centres).enumerate() {
            let d = sub(p, c);
            let r = dot(d, d).sqrt();
            if r < 1e-6 {
                continue;
            }
            let dir = d.map(|v| v / r);
            let mut r_nat: Option<f64> = None;
            head.hits(c, dir, 0.0, r + NATIVE_REACH, |t, _, _| {
                r_nat = Some(r_nat.map_or(t, |x: f64| x.max(t)));
            });
            // Anchored on the native head where it is there: a face pulled in
            // under it must not find ever deeper insides of an item.
                        let floor = r_nat.map_or(samples.floors[s], |x| x - FLOOR_DEPTH);
            cover.fill(f64::INFINITY);
            items.hits(c, dir, floor, r + REACH, |t, out, item| {
                if out {
                    let x = &mut cover[item as usize];
                    *x = x.min(t);
                }
            });
            let region = samples.regions[s];
            for ((w, m), a) in worn.iter().zip(measures.iter_mut()).zip(allow) {
                let skin = w
                    .covers
                    .iter()
                    .map(|&i| cover[i])
                    .filter(|t| t.is_finite())
                    .fold(f64::NEG_INFINITY, f64::max);
                if !skin.is_finite() {
                    continue;
                }
                let allowed = (skin - clearance + a[region])
                    .max(r_nat.map_or(f64::NEG_INFINITY, |x| x + NATIVE_TOLERANCE));
                m.native_through = m.native_through.max(a[region]);
                let margin = allowed - r;
                m.gap = m.gap.min(skin - r);
                if let Some(x) = r_nat {
                    m.native_gap = m.native_gap.min(skin - x);
                }
                if margin < 0.0 {
                    m.through += 1;
                                        for &(surface, v, share) in &samples.corners[s] {
                        let x = &mut need[surface][v];
                        *x = x.max(-margin * share);
                    }
                }
                if margin < m.margin {
                    m.margin = margin;
                    m.at = samples.bind[s];
                    label.clone_into(&mut m.pose);
                }
            }
        }
                out.push(Sweep { measures, need });
    }
    Ok(out)
}

/// What the fix did to one drawn face.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Pull {
    pub rounds: usize,
    pub moved: usize,
    pub farthest: f64,
}

/// Moves the vertices of `drawn` in by `need` (per surface, per vertex),
/// back along their rays ([`Drawn::rays`]) in the bind pose: seam copies
/// together, each pull spread over [`SPREAD_RINGS`] rings of neighbours,
/// at most [`PULL_STEP`] a round. Adds to `total` (per surface, per vertex).
fn apply_pull(drawn: &mut Drawn, need: &[Vec<f64>], total: &mut [Vec<f64>]) -> Result<()> {
    for (s, (label, bytes)) in drawn.surfaces.iter_mut().enumerate() {
        let mesh = parse_mesh(bytes, 48, label)?;
        let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
        let mut w = vec![0.0_f64; neighbours.len()];
        for (i, &k) in weld.iter().enumerate() {
            w[k] = w[k].max(need[s][i]);
        }
        for _ in 0..SPREAD_RINGS {
            let next: Vec<f64> = (0..w.len())
                .map(|k| {
                    neighbours[k]
                        .iter()
                        .map(|&u| w[u] * SPREAD_KEEP)
                        .fold(w[k], f64::max)
                })
                .collect();
            w = next;
        }
        for (i, &k) in weld.iter().enumerate() {
            if w[k] <= 0.0 {
                continue;
            }
            let step = (w[k] * 1.1 + 0.0002).min(PULL_STEP);
            let p = mesh.positions[i];
                        let along = drawn.rays[s][i].dir;
            let q = [0, 1, 2].map(|j| p[j] - along[j] * step);
            for (j, v) in q.iter().enumerate() {
                let at = 12 + i * 48 + 4 * j;
                bytes[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
            }
            total[s][i] += step;
        }
    }
    Ok(())
}

/// The jobs' faces pulled in until every pose of every job passes
/// ([`sweep`]); the jobs' last sweeps and what each pull did. Fails when a
/// vertex would need more than [`PULL_MAX`].
pub fn pull_in(
    rig: &Rig,
    poses: &SampledPoses,
    native: &Posable,
    wardrobe: &Wardrobe,
    faces: &mut [(Drawn, State)],
) -> Result<(Vec<Sweep>, Vec<Pull>)> {
    let mut totals: Vec<Vec<Vec<f64>>> = faces
        .iter()
        .map(|(d, _)| {
            d.surfaces
                .iter()
                .map(|(_, b)| read_u32(b, 8).map(|c| vec![0.0; c]))
                .collect::<Result<_>>()
        })
        .collect::<Result<_>>()?;
    let mut pulls = vec![Pull::default(); faces.len()];
        let mut last: Vec<Option<Sweep>> = (0..faces.len()).map(|_| None).collect();
    for (drawn, _) in faces.iter_mut() {
        drawn.freeze(rig)?;
    }
    for round in 0..=PULL_ROUNDS {
        // Only the faces still through are measured again: a pull changes
        // no other face.
        let open: Vec<usize> = (0..faces.len())
            .filter(|&i| {
                last[i]
                    .as_ref()
                    .is_none_or(|s: &Sweep| s.measures.iter().any(|m| m.margin < 0.0))
            })
            .collect();
        let jobs: Vec<Job<'_>> = open
            .iter()
            .map(|&i| Job {
                drawn: &faces[i].0,
                state: faces[i].1,
            })
            .collect();
        for (i, s) in open.iter().zip(sweep(rig, poses, native, wardrobe, &jobs)?) {
            last[*i] = Some(s);
        }
        let failing: Vec<usize> = open
            .iter()
            .copied()
            .filter(|&i| {
                last[i]
                    .as_ref()
                    .unwrap()
                    .measures
                    .iter()
                    .any(|m| m.margin < 0.0)
            })
            .collect();
        println!(
            "  pull round {round}: {} of {} faces still show through",
            failing.len(),
            faces.len()
        );
        for &i in &failing {
            let m = last[i]
                .as_ref()
                .unwrap()
                .measures
                .iter()
                .min_by(|a, b| a.margin.total_cmp(&b.margin))
                .unwrap();
            println!(
                "    {}: {:.2} mm under {} at {:?} ({}), pulled up to {:.1} mm",
                faces[i].0.label,
                m.margin * 1000.0,
                m.worn,
                m.at.map(|v| (v * 1000.0).round() / 1000.0),
                m.pose,
                pulls[i].farthest * 1000.0
            );
        }
        if failing.is_empty() || round == PULL_ROUNDS {
            return Ok((last.into_iter().map(|s| s.unwrap()).collect(), pulls));
        }
        for i in failing {
                        let sweep = last[i].as_ref().unwrap();
            apply_pull(&mut faces[i].0, &sweep.need, &mut totals[i])?;
            let farthest = totals[i].iter().flatten().copied().fold(0.0, f64::max);
            pulls[i] = Pull {
                rounds: round + 1,
                moved: totals[i].iter().flatten().filter(|&&t| t > 0.0).count(),
                farthest,
            };
            ensure!(
                farthest <= PULL_MAX,
                "{}: the fix would pull it in {:.1} mm: more than a fit, it needs its own item models",
                faces[i].0.label,
                farthest * 1000.0
            );
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: [f64; 3], b: [f64; 3], c: [f64; 3], item: u32) -> Tri {
        Tri {
            a,
            e1: sub(b, a),
            e2: sub(c, a),
            item,
        }
    }

    #[test]
    fn the_hierarchy_finds_every_hit_and_its_facing() {
        // A row of squares across the x axis at x = 1..=20, two triangles
        // each, facing +x on even items.
        let mut tris = vec![];
        for i in 1..=20 {
            let x = f64::from(i);
            let (a, b, c, d) = (
                [x, -1.0, -1.0],
                [x, 1.0, -1.0],
                [x, 1.0, 1.0],
                [x, -1.0, 1.0],
            );
            if i % 2 == 0 {
                tris.push(tri(a, b, c, i as u32));
                tris.push(tri(a, c, d, i as u32));
            } else {
                tris.push(tri(a, c, b, i as u32));
                tris.push(tri(a, d, c, i as u32));
            }
        }
        let bvh = Bvh::new(tris);
        let mut hits = BTreeMap::new();
        bvh.hits(
            [0.0, 0.1, 0.2],
            [1.0, 0.0, 0.0],
            0.0,
            f64::INFINITY,
            |t, out, item| {
                hits.insert(item, (t, out));
            },
        );
        assert_eq!(hits.len(), 20);
        for (item, (t, out)) in hits {
            assert!((t - f64::from(item)).abs() < 1e-9);
            assert_eq!(out, item % 2 == 0, "{item}");
        }
        // Behind the origin nothing is hit.
        let mut none = 0;
        bvh.hits(
            [30.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            0.0,
            f64::INFINITY,
            |_, _, _| none += 1,
        );
        assert_eq!(none, 0);
    }
}
