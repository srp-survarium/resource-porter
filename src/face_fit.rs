// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether a player face fits under everything worn with it: the gate of
//! the faces' build.
//!
//! 0.26g draws one head under every item that leaves it (no
//! `remove_head_model`), and every such item was made over that head
//! (`black_market_01_head_s`). A face of its own (`faces_026`) is drawn in
//! three states: bare, under a helmet (its helmet variant) and under a mask
//! (its mask variant: a mask wins over a helmet), always over a torso and
//! a back piece. Three things are measured.
//!
//! **What shows through** ([`sweep`]), along rays out of the posed skull:
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
//!   it as closely, and no closer. Under a model of the item made for the
//!   face ([`Wardrobe::for_face`]) that is: as far under that model's
//!   skin as the native head is under the game's item's. On a ray where the native head itself is
//!   through the item, skin is what is seen there today, and a face shows
//!   skin there too; where the native head shows through the items in a
//!   pose, a face may show through them as deep in the same region of the
//!   head ([`REGIONS`]). A sample on the native head's own surface (the
//!   neck's last ring, laid on the native neck's open edge) is the native
//!   head there.
//! - **Contact.** Skin a torso or back piece does not cover in the bind
//!   pose (within [`COVER_NEAR`] under it: a collar covers the neck, not the
//!   chin) can come down into it in a pose, as the native chin meets a
//!   collar when the head bows. Nothing covers that skin, so nothing shows
//!   through: how deep a face's chin and jaw go where the native ones stay
//!   out is recorded for each worn set ([`Measure::contact`]) and is not part
//!   of the margin.
//! - **Poses.** The bind pose and [`crate::bandit_026::SampledPoses`]: the
//!   face, the native head and every item skinned through each, the bones
//!   of an item's own bind pose found by name on the native rig (a bone the
//!   rig lacks, as the render bone `headwear-1` gives helmets, follows Head).
//!
//! **What is in sight** ([`covered`]): the skin no one can see under any
//! helmet (or any mask), which that state's model leaves out.
//!
//! **What stands off** ([`stand_off`]): an item's seen edges on the skin
//! ([`skin_edges`]) no more than [`STAND_OFF`] farther off a face than off
//! the native head.
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

    /// The Head bone's index.
    pub fn head_bone(&self) -> u8 {
        self.head as u8
    }

    /// The point of the neck's axis (Neck to Head) at `p`'s height.
    pub fn neck_axis(&self, p: [f64; 3]) -> [f64; 3] {
        self.neck_centre(p).0
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
    /// The model's vertex each of these is, where only a part of the model
    /// is kept ([`Posable::near_head`]); empty: all of it, in its order.
    origin: Vec<usize>,
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
            origin: vec![],
        })
    }

    /// Only the triangles with a corner in the zone a face can meet, and
    /// the vertices they use.
    fn near_head(mut self) -> Self {
        let inside = |p: &[f64; 3]| (0..3).all(|k| p[k] >= ZONE_MIN[k] && p[k] <= ZONE_MAX[k]);
        self.triangles
            .retain(|t| t.iter().any(|&v| inside(&self.positions[v])));
        let mut used = vec![usize::MAX; self.positions.len()];
        let mut origin = vec![];
        for t in &mut self.triangles {
            for v in t {
                if used[*v] == usize::MAX {
                    used[*v] = origin.len();
                    origin.push(*v);
                }
                *v = used[*v];
            }
        }
        self.like(&origin, self.triangles.clone())
    }

    /// The vertices `origin` of this model, with `triangles` over them.
    fn like(&self, origin: &[usize], triangles: Vec<[usize; 3]>) -> Self {
        Self {
            label: self.label.clone(),
            positions: origin.iter().map(|&v| self.positions[v]).collect(),
            skins: origin.iter().map(|&v| self.skins[v].clone()).collect(),
            triangles,
            origin: origin.to_vec(),
        }
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
    /// The item's config, the key of the entry it is drawn from (`data`,
    /// or a sleeve variant's) and that entry's parts (the base model, then
    /// its additional models) as (model, surface).
    pub config: String,
    pub key: String,
    pub parts: Vec<(String, String)>,
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
    /// How many of `covers` are the game's items: the rest are models made
    /// for one face ([`Wardrobe::for_face`]).
    pub items: usize,
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
            // (label, key, parts, surfaces) of each entry drawn.
            type Entry = (String, String, Vec<(String, String)>, Vec<Posable>);
            let mut variants: Vec<Entry> = vec![];
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
                let mut sources = vec![];
                for part in parts {
                    let (Some(V::String(model)), Some(V::String(surface))) =
                        (part.named("base_model"), part.named("part_name"))
                    else {
                        bail!("{path} {key}: a model part without base_model/part_name");
                    };
                    ids.push(format!("{model}/{surface}"));
                    sources.push((model.clone(), surface.clone()));
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
                let label = if key == "data" {
                    name.clone()
                } else {
                    format!("{name} ({key})")
                };
                variants.push((label, key.to_owned(), sources, surfaces));
            }
            for (label, key, parts, surfaces) in variants {
                if surfaces.iter().all(|s| s.triangles.is_empty()) {
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
                    config: path.clone(),
                    key,
                    parts,
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
            items: covers.len(),
            covers,
            worn,
            headless,
        })
    }

    pub fn count(&self, slot: fn(&Slot) -> bool) -> usize {
        self.covers[..self.items]
            .iter()
            .filter(|c| slot(&c.slot))
            .count()
    }

    /// A model of cover `base` made for one face (`surfaces`, in the
    /// order of the cover's parts), as a cover: the same part of each model
    /// as the game's cover keeps, vertex for vertex.
    pub fn for_face(&self, base: usize, label: String, surfaces: &[Posable]) -> Cover {
        let b = &self.covers[base];
        Cover {
            label,
            slot: b.slot,
            surfaces: surfaces
                .iter()
                .zip(&b.surfaces)
                .map(|(s, base)| s.like(&base.origin, base.triangles.clone()))
                .collect(),
            config: b.config.clone(),
            key: b.key.clone(),
            parts: b.parts.clone(),
        }
    }

    /// Adds a cover made for one face ([`Wardrobe::for_face`]): no worn
    /// set names it, the jobs that swap it in measure it in its item's place.
    pub fn add(&mut self, cover: Cover) -> usize {
        self.covers.push(cover);
        self.covers.len() - 1
    }
}

/// The face surfaces drawn in one state, as 48-byte converted models
/// (the face, then its hair if any).
#[derive(Clone)]
pub struct Drawn {
    pub label: String,
    pub surfaces: Vec<(String, Vec<u8>)>,
    /// Per surface, per vertex: the ray it is measured along
    /// ([`Drawn::freeze`]). Empty: from the surfaces as they are.
    pub rays: Vec<Vec<Ray>>,
}

/// A vertex's ray: where it starts (bind pose) and its skin, its direction,
/// and how deep under the vertex as the donor made it an item surface is
/// the item's inside where the native head is not on the ray.
#[derive(Clone, Debug)]
pub struct Ray {
    centre: [f64; 3],
    skin: Skin,
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
    /// For each sample, the vertices (surface, vertex) it stands for and
    /// each one's share of it.
    corners: Vec<Vec<(usize, usize, f64)>>,
    /// Each sample's region ([`Rig::region`]).
    regions: Vec<usize>,
    /// Whether the sample lies on the native head's surface (within
    /// [`NATIVE_TOLERANCE`], bind pose): the neck's last ring on the native
    /// neck's open edge, where a ray may pass the native head by.
    on_native: Vec<bool>,
}

impl Samples {
    fn new(rig: &Rig, drawn: &Drawn, native_bind_pose: &[u8], native: &Posable) -> Result<Self> {
        let surfaces = drawn
            .surfaces
            .iter()
            .map(|(label, bytes)| Posable::parse(rig, label, bytes, native_bind_pose))
            .collect::<Result<Vec<_>>>()?;
        let mut drawn = drawn.clone();
        drawn.freeze(rig)?;
        let mut out = Self::of(&surfaces, &drawn.rays);
        out.on_native = out
            .bind
            .iter()
            .map(|&p| {
                native.triangles.iter().any(|t| {
                    let [a, b, c] = t.map(|v| native.positions[v]);
                    let w = crate::mask_fit::closest_on_triangle(p, a, b, c);
                    let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * c[k]);
                    let d = sub(p, q);
                    dot(d, d) <= NATIVE_TOLERANCE * NATIVE_TOLERANCE
                })
            })
            .collect();
        Ok(out)
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
            on_native: vec![],
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
                let centre =
                    [0, 1, 2].map(|k| t.iter().map(|&v| rays[v].centre[k]).sum::<f64>() / 3.0);
                out.centres.push((centre, r.skin.clone()));
                out.floors.push(
                    t.iter()
                        .map(|&v| rays[v].floor)
                        .fold(f64::INFINITY, f64::min),
                );
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
    /// Skin a torso or back piece does not cover in the bind pose (the
    /// chin, the jaw) that comes into it in a pose, where the native head
    /// stays out: the deepest (m) and the sample-poses. Reported, not part
    /// of the margin: nothing covers that skin, it meets the collar.
    pub contact: f64,
    pub contacts: usize,
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
            contact: 0.0,
            contacts: 0,
            through: 0,
            at: [0.0; 3],
            pose: String::new(),
        }
    }
    fn merge(&mut self, o: &Self) {
        self.through += o.through;
        self.contacts += o.contacts;
        self.contact = self.contact.max(o.contact);
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
    /// Covers this face is drawn with another model of ([`Wardrobe::for_face`]):
    /// the wardrobe's cover to the one drawn in its place.
    pub swaps: &'a BTreeMap<usize, usize>,
    /// Only the worn sets with this cover (a head drawn for one item).
    pub only: Option<usize>,
}

/// What a sweep over the poses found for one job.
pub struct Sweep {
    pub measures: Vec<Measure>,
    /// Per surface, per vertex: how far through it is in its worst pose (m).
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
        .map(|j| Samples::new(rig, j.drawn, &poses.bind_pose, native))
        .collect::<Result<_>>()?;
    let native_samples = Samples::native(rig, native);
    let bodies: Vec<Option<usize>> = {
        let mut next = 0;
        wardrobe
            .covers
            .iter()
            .map(|c| {
                (c.slot == Slot::Body).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect()
    };
    let body_cover: Vec<Vec<Vec<bool>>> = samples
        .iter()
        .map(|s| bind_body_cover(&poses.bind, native, wardrobe, &bodies, s))
        .collect::<Result<_>>()?;
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
                let (bodies, body_cover) = (&bodies, &body_cover);
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
                            (bodies, body_cover),
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
    (bodies, body_cover): (&[Option<usize>], &[Vec<Vec<bool>>]),
    clearance: f64,
    label: &str,
) -> Result<Vec<Sweep>> {
    let mut tris = vec![];
    soup(&native.posed(bind, pose)?, &native.triangles, 0, &mut tris);
    let head = Bvh::new(tris);
    let mut tris = vec![];
    for (i, c) in wardrobe.covers[..wardrobe.items].iter().enumerate() {
        for s in &c.surfaces {
            soup(&s.posed(bind, pose)?, &s.triangles, i as u32, &mut tris);
        }
    }
    let items = Bvh::new(tris);
    // The models made for a face, by the face (the jobs of one face share
    // its swaps): a ray meets the game's items and its own face's.
    let mut own: Vec<(*const BTreeMap<usize, usize>, Bvh)> = vec![];
    for job in jobs {
        let key = std::ptr::from_ref(job.swaps);
        if own.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let mut tris = vec![];
        for &i in job.swaps.values() {
            for s in &wardrobe.covers[i].surfaces {
                soup(&s.posed(bind, pose)?, &s.triangles, i as u32, &mut tris);
            }
        }
        own.push((key, Bvh::new(tris)));
    }
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
    for ((job, samples), body_cover) in jobs.iter().zip(samples).zip(body_cover) {
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
        // The worn sets each cover is in (the game's item, or the model
        // this face is drawn with in its place): a ray is measured only
        // against the sets of the covers it meets.
        let mut sets_of: Vec<Vec<u32>> = vec![vec![]; n];
        for (k, w) in worn.iter().enumerate() {
            if job.only.is_some_and(|c| !w.covers.contains(&c)) {
                continue;
            }
            for &i in &w.covers {
                sets_of[i].push(k as u32);
                if let Some(&swap) = job.swaps.get(&i) {
                    sets_of[swap].push(k as u32);
                }
            }
        }
        let (mut met, mut visit): (Vec<usize>, Vec<u32>) = (vec![], vec![]);
        let made_for_face = &own
            .iter()
            .find(|(k, _)| *k == std::ptr::from_ref(job.swaps))
            .context("a job without its face's models")?
            .1;
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
            if samples.on_native[s] {
                r_nat = Some(r_nat.map_or(r, |x| x.max(r)));
            }
            // Anchored on the native head where it is there.
            let floor = r_nat.map_or(samples.floors[s], |x| x - FLOOR_DEPTH);
            for &i in &met {
                cover[i] = f64::INFINITY;
            }
            met.clear();
            for bvh in [&items, made_for_face] {
                bvh.hits(c, dir, floor, r + REACH, |t, out, item| {
                    if out {
                        let x = &mut cover[item as usize];
                        if x.is_infinite() {
                            met.push(item as usize);
                        }
                        *x = x.min(t);
                    }
                });
            }
            visit.clear();
            for &i in &met {
                visit.extend(&sets_of[i]);
            }
            visit.sort_unstable();
            visit.dedup();
            let region = samples.regions[s];
            for &k in &visit {
                let (w, m, a) = (
                    &worn[k as usize],
                    &mut measures[k as usize],
                    &allow[k as usize],
                );
                let outermost = |of: &dyn Fn(usize) -> usize| {
                    w.covers
                        .iter()
                        .map(|&i| cover[of(i)])
                        .filter(|t| t.is_finite())
                        .fold(f64::NEG_INFINITY, f64::max)
                };
                // The skin of what this face is drawn with, and of the game's
                // own items, which the native head is under.
                let skin = outermost(&|i| job.swaps.get(&i).copied().unwrap_or(i));
                let made = outermost(&|i| i);
                if !skin.is_finite() {
                    continue;
                }
                // Where the native head itself is through the item on this
                // ray, skin is seen there today: a face shows skin there too.
                if r_nat.is_some_and(|x| made.is_finite() && x >= made) {
                    continue;
                }
                // As far under its item as the native head is under the
                // game's (the same item: no farther out than the native head).
                let as_native = match r_nat {
                    Some(x) if made.is_finite() => skin - (made - x) + NATIVE_TOLERANCE,
                    _ => f64::NEG_INFINITY,
                };
                let allowed = (skin - clearance + a[region]).max(as_native);
                m.native_through = m.native_through.max(a[region]);
                let margin = allowed - r;
                // Skin a torso does not cover in the bind pose meets it in
                // this pose: a contact, not a show-through.
                if let [only] = w.covers[..]
                    && let Some(body) = bodies[only]
                    && !body_cover[s][body]
                {
                    if margin < 0.0 {
                        m.contacts += 1;
                        m.contact = m.contact.max(-margin);
                    }
                    continue;
                }
                m.gap = m.gap.min(skin - r);
                if let Some(x) = r_nat
                    && made.is_finite()
                {
                    m.native_gap = m.native_gap.min(made - x);
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

/// An item's skin within this beyond a sample (m), bind pose, covers it:
/// a collar round the neck does, the chest under the chin does not.
const COVER_NEAR: f64 = 0.02;

/// Per sample, per torso or back piece (`bodies`: its number by cover):
/// whether it covers the sample in the bind pose ([`COVER_NEAR`]).
fn bind_body_cover(
    bind: &[Frame],
    native: &Posable,
    wardrobe: &Wardrobe,
    bodies: &[Option<usize>],
    samples: &Samples,
) -> Result<Vec<Vec<bool>>> {
    let mut tris = vec![];
    soup(&native.posed(bind, bind)?, &native.triangles, 0, &mut tris);
    let head = Bvh::new(tris);
    let mut tris = vec![];
    for (c, body) in wardrobe.covers.iter().zip(bodies) {
        if let Some(b) = body {
            for s in &c.surfaces {
                soup(&s.positions, &s.triangles, *b as u32, &mut tris);
            }
        }
    }
    let items = Bvh::new(tris);
    let count = bodies.iter().flatten().count();
    Ok(samples
        .bind
        .iter()
        .zip(&samples.centres)
        .zip(&samples.floors)
        .map(|((&p, (c, _)), &own_floor)| {
            let mut covered = vec![false; count];
            let d = sub(p, *c);
            let r = dot(d, d).sqrt();
            if r < 1e-6 {
                return covered;
            }
            let dir = d.map(|v| v / r);
            let mut r_nat: Option<f64> = None;
            head.hits(*c, dir, 0.0, r + NATIVE_REACH, |t, _, _| {
                r_nat = Some(r_nat.map_or(t, |x: f64| x.max(t)));
            });
            let floor = r_nat.map_or(own_floor, |x| x - FLOOR_DEPTH);
            items.hits(*c, dir, floor, r + COVER_NEAR, |_, out, item| {
                if out {
                    covered[item as usize] = true;
                }
            });
            covered
        })
        .collect())
}

/// The most a vertex of an item is moved out in one round of [`settle`]
/// (m), the rounds, and what a move adds to what the measure asks (m).
const SETTLE_STEP: f64 = 0.004;
const SETTLE_ROUNDS: usize = 12;
const SETTLE_ROOM: f64 = 0.0003;
/// The most a vertex is moved in all (m): where settling asks for more
/// (round after round at an item's edge), the item does not fit by a
/// carry, and the gate says so.
const SETTLE_MOST: f64 = 0.006;

impl Posable {
    /// Writes these positions into the model they are a part of.
    pub fn write_positions(&self, model: &mut [u8]) -> Result<()> {
        let count = read_u32(model, 8)?;
        ensure!(count > 0, "{}: no vertices", self.label);
        let stride = (read_u32(model, 4)? - 4) / count;
        for (k, p) in self.positions.iter().enumerate() {
            let v = if self.origin.is_empty() {
                k
            } else {
                self.origin[k]
            };
            let at = 12 + v * stride;
            for (j, x) in p.iter().enumerate() {
                model
                    .get_mut(at + 4 * j..at + 4 * j + 4)
                    .context("a vertex outside its model")?
                    .copy_from_slice(&(*x as f32).to_le_bytes());
            }
        }
        Ok(())
    }
}

/// Settles a cover made for a face (`made`, of the game's `base`) on the
/// face's skin (`drawn`): in each of `poses` (the bind pose first), where a
/// sample of the face is nearer the cover's skin than the rule lets it be
/// ([`sweep`]'s, for a cover made for a face), the triangle of the cover
/// over it moves out along the sample's ray by what is missing, its
/// neighbours by half; round after round until nothing is. Returns the
/// rounds and the farthest a vertex moved in all (m).
#[allow(clippy::too_many_arguments)]
pub fn settle(
    rig: &Rig,
    bind: &[Frame],
    poses: &[&[Frame]],
    bind_pose: &[u8],
    native: &Posable,
    base: &Cover,
    made: &mut Cover,
    drawn: &Drawn,
) -> Result<(usize, f64)> {
    let samples = Samples::new(rig, drawn, bind_pose, native)?;
    let (centres, centre_skins): (Vec<[f64; 3]>, Vec<Skin>) =
        samples.centres.iter().cloned().unzip();
    // Each pose: the samples, their centres, the native head and the game's cover.
    struct Posed {
        at: Vec<[f64; 3]>,
        centres: Vec<[f64; 3]>,
        head: Bvh,
        base: Bvh,
    }
    let mut posed = vec![];
    for pose in poses {
        let mut tris = vec![];
        soup(&native.posed(bind, pose)?, &native.triangles, 0, &mut tris);
        let head = Bvh::new(tris);
        let mut tris = vec![];
        for s in &base.surfaces {
            soup(&s.posed(bind, pose)?, &s.triangles, 0, &mut tris);
        }
        posed.push(Posed {
            at: skin_positions(&samples.bind, &samples.skins, bind, pose)?,
            centres: skin_positions(&centres, &centre_skins, bind, pose)?,
            head,
            base: Bvh::new(tris),
        });
    }
    // A cover's triangles, numbered through its surfaces.
    let mut first = vec![];
    let mut count = 0;
    for s in &made.surfaces {
        first.push(count);
        count += s.triangles.len();
    }
    let welds: Vec<(Vec<usize>, Vec<std::collections::BTreeSet<usize>>)> = made
        .surfaces
        .iter()
        .map(|s| {
            crate::mask_fit::weld_vertices(&crate::mask_fit::Mesh {
                positions: s.positions.clone(),
                triangles: s.triangles.clone(),
            })
        })
        .collect();
    let mut total: Vec<Vec<f64>> = made
        .surfaces
        .iter()
        .map(|s| vec![0.0; s.positions.len()])
        .collect();
    let mut rounds = 0;
    for _ in 0..SETTLE_ROUNDS {
        // Per surface, per welded vertex: how far out, and along what.
        let mut push: Vec<Vec<(f64, [f64; 3])>> = welds
            .iter()
            .map(|(_, n)| vec![(0.0, [0.0; 3]); n.len()])
            .collect();
        let mut any = false;
        for (pose, p) in poses.iter().zip(&posed) {
            let mut tris = vec![];
            for (k, s) in made.surfaces.iter().enumerate() {
                let at = s.posed(bind, pose)?;
                for (t, tri) in s.triangles.iter().enumerate() {
                    let [a, b, c] = tri.map(|v| at[v]);
                    tris.push(Tri {
                        a,
                        e1: sub(b, a),
                        e2: sub(c, a),
                        item: (first[k] + t) as u32,
                    });
                }
            }
            let cover = Bvh::new(tris);
            let clearance = if std::ptr::eq(*pose, poses[0]) {
                BIND_CLEARANCE
            } else {
                POSED_CLEARANCE
            };
            for (s, (&at, &c)) in p.at.iter().zip(&p.centres).enumerate() {
                let d = sub(at, c);
                let r = dot(d, d).sqrt();
                if r < 1e-6 {
                    continue;
                }
                let dir = d.map(|v| v / r);
                let mut r_nat: Option<f64> = None;
                p.head.hits(c, dir, 0.0, r + NATIVE_REACH, |t, _, _| {
                    r_nat = Some(r_nat.map_or(t, |x: f64| x.max(t)));
                });
                if samples.on_native[s] {
                    r_nat = Some(r_nat.map_or(r, |x| x.max(r)));
                }
                let floor = r_nat.map_or(samples.floors[s], |x| x - FLOOR_DEPTH);
                let mut game = f64::INFINITY;
                p.base.hits(c, dir, floor, r + REACH, |t, out, _| {
                    if out {
                        game = game.min(t);
                    }
                });
                let mut skin = (f64::INFINITY, 0_u32);
                cover.hits(c, dir, floor, r + REACH, |t, out, tri| {
                    if out && t < skin.0 {
                        skin = (t, tri);
                    }
                });
                if !skin.0.is_finite() || r_nat.is_some_and(|x| game.is_finite() && x >= game) {
                    continue;
                }
                let as_native = match r_nat {
                    Some(x) if game.is_finite() => skin.0 - (game - x) + NATIVE_TOLERANCE,
                    _ => f64::NEG_INFINITY,
                };
                let missing = r - (skin.0 - clearance).max(as_native);
                if missing <= 0.0 {
                    continue;
                }
                any = true;
                let k = first.partition_point(|&f| f <= skin.1 as usize) - 1;
                let tri = made.surfaces[k].triangles[skin.1 as usize - first[k]];
                // The ray as the bind pose has it: the cover's vertices
                // are moved there.
                let bind_dir = unit(sub(samples.bind[s], samples.centres[s].0));
                for v in tri {
                    let e = &mut push[k][welds[k].0[v]];
                    if missing + SETTLE_ROOM > e.0 {
                        *e = (missing + SETTLE_ROOM, bind_dir);
                    }
                }
            }
        }
        if !any {
            break;
        }
        rounds += 1;
        for (k, s) in made.surfaces.iter_mut().enumerate() {
            let (weld, neighbours) = &welds[k];
            let asked = push[k].clone();
            for (v, n) in neighbours.iter().enumerate() {
                for &u in n {
                    if asked[u].0 / 2.0 > push[k][v].0 {
                        push[k][v] = (asked[u].0 / 2.0, asked[u].1);
                    }
                }
            }
            for (v, p) in s.positions.iter_mut().enumerate() {
                let (by, dir) = push[k][weld[v]];
                let by = by.min(SETTLE_STEP).min(SETTLE_MOST - total[k][v]);
                if by > 0.0 {
                    for j in 0..3 {
                        p[j] += dir[j] * by;
                    }
                    total[k][v] += by;
                }
            }
        }
    }
    Ok((rounds, total.iter().flatten().copied().fold(0.0, f64::max)))
}

/// A surface to cast rays at (bind pose).
pub(crate) struct Solid {
    bvh: Bvh,
}

impl Solid {
    pub fn new(positions: &[[f64; 3]], triangles: &[[usize; 3]]) -> Self {
        let mut tris = vec![];
        soup(positions, triangles, 0, &mut tris);
        Self {
            bvh: Bvh::new(tris),
        }
    }

    /// The farthest crossing of the ray `o + t d` (t > 0) with the surface.
    pub fn farthest(&self, o: [f64; 3], d: [f64; 3]) -> Option<f64> {
        let mut best: Option<f64> = None;
        self.bvh.hits(o, d, 0.0, f64::INFINITY, |t, _, _| {
            best = Some(best.map_or(t, |x: f64| x.max(t)));
        });
        best
    }
}

impl Rig {
    /// Where the ray through a point near the head starts, for carrying an
    /// item from one head onto another: the skull's middle, below the chin
    /// the neck's axis, and between them a point passing from one to the
    /// other, so the rays change smoothly down an item.
    pub fn carry_centre(&self, p: [f64; 3]) -> [f64; 3] {
        let x = ((p[1] - (self.chin - CHIN_BAND)) / (2.0 * CHIN_BAND)).clamp(0.0, 1.0);
        let w = x * x * (3.0 - 2.0 * x);
        let neck = self.neck_centre(p).0;
        [0, 1, 2].map(|k| neck[k] + w * (self.skull[k] - neck[k]))
    }
}

/// Directions spread evenly over the half space round `n` (a Fibonacci
/// sphere's points at least `GRAZE` off the surface).
fn outward(n: [f64; 3], count: usize) -> Vec<[f64; 3]> {
    const GRAZE: f64 = 0.08;
    let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    (0..2 * count)
        .map(|i| {
            let y = 1.0 - (i as f64 + 0.5) / count as f64;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = golden * i as f64;
            [a.cos() * r, y, a.sin() * r]
        })
        .filter(|d| dot(*d, n) > GRAZE)
        .collect()
}

/// How far a sight line is followed before it has left the items (m).
const SIGHT_REACH: f64 = 0.6;
/// Sight lines per vertex ([`covered`]).
const SIGHT_LINES: usize = 96;
/// A sight line starts this far off the skin (m).
const SIGHT_OFF: f64 = 0.0005;
/// Rings of covered neighbours a vertex needs to be out of sight itself
/// ([`covered`]): the cut's edge stays that far under every item's edge.
const COVER_RINGS: usize = 2;

/// What each vertex of a drawn face can be seen under, in the bind pose.
pub struct Sight {
    /// Per surface, per vertex: out of sight under every item of the slot,
    /// each worn alone, and so are its neighbours [`COVER_RINGS`] rings round.
    pub covered: Vec<Vec<bool>>,
    /// Per surface, per vertex: the items (wardrobe covers) that leave it
    /// in sight.
    pub seen_under: Vec<Vec<Vec<usize>>>,
}

/// Which vertices of `drawn` no one can see under any item of a slot
/// (`in_slot`), each item worn alone: from each vertex, [`SIGHT_LINES`]
/// sight lines over the half space outside the skin; a vertex is out of
/// sight under an item when every one of them meets the item. The face's
/// own skin hides nothing here (an ear does not make the skin behind it
/// safe to leave out).
pub fn covered(wardrobe: &Wardrobe, drawn: &Drawn, in_slot: fn(&Slot) -> bool) -> Result<Sight> {
    let items: Vec<usize> = (0..wardrobe.items)
        .filter(|&i| in_slot(&wardrobe.covers[i].slot))
        .collect();
    let mut tris = vec![];
    for (k, &i) in items.iter().enumerate() {
        for s in &wardrobe.covers[i].surfaces {
            soup(&s.positions, &s.triangles, k as u32, &mut tris);
        }
    }
    let bvh = Bvh::new(tris);
    let mut sight = Sight {
        covered: vec![],
        seen_under: vec![],
    };
    for (label, bytes) in &drawn.surfaces {
        let mesh = parse_mesh(bytes, 48, label)?;
        let normals = vertex_normals(&mesh.positions, &mesh.triangles);
        let seen: Vec<Vec<usize>> = mesh
            .positions
            .iter()
            .zip(&normals)
            .map(|(&p, &n)| {
                let o = [0, 1, 2].map(|k| p[k] + n[k] * SIGHT_OFF);
                let mut open = vec![false; items.len()];
                let mut met = vec![false; items.len()];
                for d in outward(n, SIGHT_LINES) {
                    met.fill(false);
                    bvh.hits(o, d, 0.0, SIGHT_REACH, |_, _, item| {
                        met[item as usize] = true
                    });
                    for (o, m) in open.iter_mut().zip(&met) {
                        *o |= !m;
                    }
                }
                items
                    .iter()
                    .zip(&open)
                    .filter(|(_, o)| **o)
                    .map(|(i, _)| *i)
                    .collect()
            })
            .collect();
        let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
        let mut hidden = vec![true; neighbours.len()];
        for (v, s) in seen.iter().enumerate() {
            hidden[weld[v]] &= s.is_empty();
        }
        for _ in 0..COVER_RINGS {
            hidden = (0..hidden.len())
                .map(|k| hidden[k] && neighbours[k].iter().all(|&u| hidden[u]))
                .collect();
        }
        sight
            .covered
            .push(weld.iter().map(|&k| hidden[k]).collect());
        sight.seen_under.push(seen);
    }
    Ok(sight)
}

/// How far off the skin an item's open edges may stand beyond what they
/// stand off the native head (m).
pub const STAND_OFF: f64 = 0.004;
/// An item's open edge within this of the native head (m) is an edge on
/// the skin, which a gap under it shows.
const EDGE_NEAR: f64 = 0.012;

/// How an item's seen edges sit on a face.
#[derive(Clone, Debug, Serialize)]
pub struct StandOff {
    pub item: String,
    /// The most an open edge stands farther off the face than off the
    /// native head (m), and where (bind pose).
    pub excess: f64,
    pub at: [f64; 3],
    /// Edge points measured, and those beyond [`STAND_OFF`].
    pub edge_points: usize,
    pub off: usize,
}

fn closest_distance(p: [f64; 3], positions: &[[f64; 3]], triangles: &[[usize; 3]]) -> f64 {
    let mut best = f64::INFINITY;
    for t in triangles {
        let [a, b, c] = t.map(|v| positions[v]);
        let w = crate::mask_fit::closest_on_triangle(p, a, b, c);
        let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * c[k]);
        let d = sub(p, q);
        best = best.min(dot(d, d));
    }
    best.sqrt()
}

/// The points of a cover's open edges that lie on the native head's skin
/// (within [`EDGE_NEAR`] of it) and can be seen (at least [`EDGE_SEEN`] of
/// the sight lines from the point, all round it, pass the item and the
/// native head: a liner's edge inside a helmet is not one): each as
/// (surface, vertex, vertex), the point halfway between the two (the same
/// vertex twice: the vertex), with its distance from the native head.
pub struct SkinEdges {
    points: Vec<(usize, usize, usize)>,
    native: Vec<f64>,
}

fn edge_point(cover: &Cover, (s, a, b): (usize, usize, usize)) -> [f64; 3] {
    let p = &cover.surfaces[s].positions;
    [0, 1, 2].map(|k| (p[a][k] + p[b][k]) / 2.0)
}

/// The share of sight lines from an edge point that must be free for the
/// edge to count as seen.
const EDGE_SEEN: f64 = 0.15;

pub fn skin_edges(cover: &Cover, native: &Posable) -> SkinEdges {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
    let mut tris = vec![];
    soup(&native.positions, &native.triangles, 0, &mut tris);
    for s in &cover.surfaces {
        soup(&s.positions, &s.triangles, 1, &mut tris);
    }
    let blockers = Bvh::new(tris);
    let lines = outward([0.0, 1.0, 0.0], SIGHT_LINES)
        .into_iter()
        .flat_map(|d| [d, d.map(|v| -v)])
        .collect::<Vec<_>>();
    let seen = |p: [f64; 3]| {
        let free = lines
            .iter()
            .filter(|d| {
                let mut met = false;
                blockers.hits(p, **d, 1e-4, SIGHT_REACH, |_, _, _| met = true);
                !met
            })
            .count();
        free as f64 >= EDGE_SEEN * lines.len() as f64
    };
    let mut out = SkinEdges {
        points: vec![],
        native: vec![],
    };
    for (index, s) in cover.surfaces.iter().enumerate() {
        // A welded edge: how many triangles use it, and its two vertices.
        type Ends = ([i64; 3], [i64; 3]);
        let mut uses: BTreeMap<Ends, (usize, usize, usize)> = BTreeMap::new();
        for t in &s.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (ka, kb) = (key(s.positions[a]), key(s.positions[b]));
                let k = if ka < kb { (ka, kb) } else { (kb, ka) };
                uses.entry(k).or_insert((0, a, b)).0 += 1;
            }
        }
        for &(n, a, b) in uses.values() {
            if n != 1 {
                continue;
            }
            for at in [(index, a, a), (index, a, b)] {
                let d =
                    closest_distance(edge_point(cover, at), &native.positions, &native.triangles);
                if d <= EDGE_NEAR && seen(edge_point(cover, at)) {
                    out.points.push(at);
                    out.native.push(d);
                }
            }
        }
    }
    out
}

/// How much farther off a face's skin than off the native head's the open
/// edges of an item stand: `edges` of the game's cover, `cover` the model
/// the face is drawn with (the same one, or the one made for the face).
pub fn stand_off(edges: &SkinEdges, cover: &Cover, face: &[Posable]) -> StandOff {
    let mut out = StandOff {
        item: cover.label.clone(),
        excess: 0.0,
        at: [0.0; 3],
        edge_points: edges.points.len(),
        off: 0,
    };
    for (&at, &native) in edges.points.iter().zip(&edges.native) {
        let p = edge_point(cover, at);
        let d = face
            .iter()
            .map(|f| closest_distance(p, &f.positions, &f.triangles))
            .fold(f64::INFINITY, f64::min);
        let excess = d - native;
        if excess > STAND_OFF {
            out.off += 1;
        }
        if excess > out.excess {
            out.excess = excess;
            out.at = p;
        }
    }
    out
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
