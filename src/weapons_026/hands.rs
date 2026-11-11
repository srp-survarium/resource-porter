// SPDX-License-Identifier: GPL-3.0-or-later
//! A gun's hands as 0.26g draws them, and a later build's hands clip made
//! to play on 0.26g as its own first view played it.
//!
//! What the engine does (0.26g build 2777, read from the exe; names and
//! addresses in skhid-srp's `tools/re-kit/v026g`):
//!
//! * **The player's pose.** A state's hands clip (`.../hands_only/<state>`,
//!   `animation_type` 1) is additive: every bone's frame is the body clip's
//!   with the hands clip's on it, the body's rotation first
//!   (`rotation = base * additive`, translations added). The body is the
//!   gun's `user_animations` set's: in the first view `stand_hud[0]`, in the
//!   third `stand[0]`; 0.26g's sets name
//!   `weapons/ak/player/stand/3rd_view/on_site/on_site_idle` for both, for
//!   rifles and pistols alike. 0.26g's hands clips move the arms and the
//!   `Weapon` bone only.
//! * **The gun** hangs on the player's `Weapon` bone, a child of `Root` (not
//!   of a hand), turned half a turn about its own Y
//!   (`portable_interactive_object_core::update_bones_matrices`, 0x2FF600,
//!   0x293AE0); the gun's own clip moves its bones in that frame. The
//!   first-person camera follows the `Head` bone (0.10b's
//!   `calculated_head_matrix`).
//! * **Hand IK** (`hand_to_weapon_ik_solver`, 0x392910..0x3931A0; the
//!   `ik_on_hands` console switch, on): a clip's `left_hand_ik` and
//!   `right_hand_ik` event channels set each hand's target (0x392D90; value
//!   `v`: `v / 2` is 0 the hand's own place, 1 the gun's `*_hand_cont` bone,
//!   2..5 the model's `<Hand>_idle`, `_reload`, `_reload_pivot`, `_lobby`
//!   locator; odd `v` the long transition). The arm is bent to put the hand
//!   there. 0.69d0's exe maps the values the same way (0x200564C50).
//!
//! On 0.26g's stance 0.26g's own clips put each hand on the gun's contact
//! bone within 0.1 mm (the gun's clips carry the contact points the hands
//! were drawn with; [`contact`] measures it). The later builds (0.31a2,
//! 0.46c2, 0.69d0) draw their guns' hands on a third-person stance
//! (`base/player/3rd_view/stand/[pistol/]on_site_idle_long`, there within
//! 0.1 mm) and play them in the first view on another, the hand IK covering
//! the 1 to 26 cm between hand and contact point. 0.26g's first view did
//! not show that IK for 0.46c2's MP-443 (resource-porter faa703a: no hands,
//! the gun alone, with its idle points in the arms' reach), and where it
//! does, it bends the arms as the baked clip has them; so a carried clip
//! whose hands are off the gun has the IK baked in ([`bake_ik`]): each
//! frame's targets as the clip's events set them and the engine blends them,
//! the arms bent onto them as the engine bends them.
//!
//! The later builds' rifle stance for the first view holds the gun where
//! 0.26g's does (to 1 mm); their pistols' holds it 22 cm further out, so a
//! pistol's clips are first re-based ([`rebase`]) from the donor's
//! first-person stance onto 0.26g's: the arms and the gun where the donor's
//! first view has them, relative to the chest (0.26g's head stands on its
//! chest as the donors' first-person head does, so the view is the donor's
//! too).

use anyhow::{Context, Result, ensure};

use crate::binary_config::{BinaryConfig, ConfigValue};
use crate::skeleton_animation::{
    Basis, Channel, Clip, Frame, ROOT_MOTION_CHANNEL, Rig, basis_to_euler, euler_to_basis,
    multiply, transform, transpose,
};

/// `animation_type` of an additive clip.
pub const ADDITIVE: u8 = 1;
/// The player's bones the hands' chains hang from.
pub const CHEST: &str = "Chest";
pub const HEAD: &str = "Head";
const ROOT: &str = "Root";
const HIP: &str = "Hip";
const WEAPON: &str = "Weapon";
const ARMS: [&str; 2] = ["LeftClavicle", "RightClavicle"];
const HANDS: [(&str, &str); 2] = [
    ("LeftHand", "left_hand_cont"),
    ("RightHand", "right_hand_cont"),
];
/// The gun's frame on the `Weapon` bone: half a turn about its Y.
const HALF_TURN: Basis = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];

/// A `.skeleton`'s bones, parents first (its nesting), the root-motion
/// channel left out (the engine computes its children from the identity).
#[derive(Clone, Debug)]
pub struct Skeleton {
    names: Vec<String>,
    parents: Vec<Option<usize>>,
}

impl Skeleton {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        fn walk(value: &ConfigValue, parent: Option<usize>, out: &mut Skeleton) {
            let ConfigValue::Named(entries) = value else {
                return;
            };
            for entry in entries.iter().filter(|e| e.name != "mask") {
                let this = if entry.name == ROOT_MOTION_CHANNEL {
                    None
                } else {
                    out.names.push(entry.name.clone());
                    out.parents.push(parent);
                    Some(out.names.len() - 1)
                };
                walk(&entry.value, this, out);
            }
        }
        let mut skeleton = Self {
            names: vec![],
            parents: vec![],
        };
        walk(&BinaryConfig::parse(bytes)?.root, None, &mut skeleton);
        ensure!(!skeleton.names.is_empty(), "the skeleton names no bones");
        Ok(skeleton)
    }

    pub fn index(&self, name: &str) -> Result<usize> {
        self.names
            .iter()
            .position(|n| n == name)
            .with_context(|| format!("the skeleton has no bone {name}"))
    }

    fn is_under(&self, bone: usize, ancestor: usize) -> bool {
        let mut at = Some(bone);
        while let Some(b) = at {
            if b == ancestor {
                return true;
            }
            at = self.parents[b];
        }
        false
    }
}

fn compose(local: &Frame, parent: &Frame) -> Frame {
    let offset = transform(local.1, &parent.0);
    (
        multiply(&local.0, &parent.0),
        [0, 1, 2].map(|k| offset[k] + parent.1[k]),
    )
}

/// `child` in `parent`'s frame (both in one frame).
fn relative(child: &Frame, parent: &Frame) -> Frame {
    let inverse = transpose(&parent.0);
    (
        multiply(&child.0, &inverse),
        transform([0, 1, 2].map(|k| child.1[k] - parent.1[k]), &inverse),
    )
}

/// A bone's translation and Euler angles in `clip` at `time`.
fn channels(clip: &Clip, bone: &str, time: f32) -> Option<([f32; 3], [f32; 3])> {
    let track = clip.bones.iter().position(|b| b == bone)?;
    let c = &clip.tracks[track].channels;
    let at = |k: usize| c[k].evaluate(time);
    Some(([at(0), at(1), at(2)], [at(3), at(4), at(5)]))
}

/// A body clip's first frame: the frame the hands clips are drawn on.
#[derive(Clone, Debug)]
pub struct Stance {
    /// Per skeleton bone: its Euler angles and translation.
    locals: Vec<([f32; 3], [f32; 3])>,
}

impl Stance {
    /// `clip` must drive every bone of `skeleton` (a full-body clip).
    pub fn of(skeleton: &Skeleton, clip: &Clip) -> Result<Self> {
        let locals = skeleton
            .names
            .iter()
            .map(|n| {
                channels(clip, n, 0.0).with_context(|| format!("the stance does not drive {n}"))
            })
            .collect::<Result<_>>()?;
        Ok(Self { locals })
    }
}

/// The player's pose at `time` of the additive `hands` on `stance`: every
/// bone's frame in the player's (model) frame.
pub fn pose(skeleton: &Skeleton, stance: &Stance, hands: &Clip, time: f32) -> Result<Vec<Frame>> {
    ensure!(
        hands.animation_type == ADDITIVE,
        "a hands clip is additive; this one is of type {}",
        hands.animation_type
    );
    let mut frames: Vec<Frame> = Vec::with_capacity(skeleton.names.len());
    for (bone, name) in skeleton.names.iter().enumerate() {
        let (base_t, base_r) = stance.locals[bone];
        let (add_t, add_r) = channels(hands, name, time).unwrap_or(([0.0; 3], [0.0; 3]));
        let local = (
            multiply(&euler_to_basis(base_r), &euler_to_basis(add_r)),
            [0, 1, 2].map(|k| base_t[k] + add_t[k]),
        );
        frames.push(match skeleton.parents[bone] {
            None => local,
            Some(parent) => compose(&local, &frames[parent]),
        });
    }
    Ok(frames)
}

impl Stance {
    /// How far apart two stances hold the gun in the view: the `Weapon`
    /// bone in the `Head` bone's frame (the first-person camera's), with no
    /// hands clip on them: (metres, radians).
    pub fn gun_apart(&self, other: &Stance, skeleton: &Skeleton) -> Result<(f32, f32)> {
        let none = Clip {
            bones: vec![],
            tracks: vec![],
            animation_type: ADDITIVE,
        };
        let (weapon, head) = (skeleton.index(WEAPON)?, skeleton.index(HEAD)?);
        let in_view = |stance: &Stance| -> Result<Frame> {
            let p = pose(skeleton, stance, &none, 0.0)?;
            Ok(relative(&p[weapon], &p[head]))
        };
        let (a, b) = (in_view(self)?, in_view(other)?);
        let d = (0..3)
            .map(|k| (a.1[k] - b.1[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        let r = multiply(&a.0, &transpose(&b.0));
        let angle = ((r[0][0] + r[1][1] + r[2][2] - 1.0) / 2.0)
            .clamp(-1.0, 1.0)
            .acos();
        Ok((d, angle))
    }
}

/// A gun's skeleton with its two contact bones and the hand locators of its
/// model (`<Hand>_idle`, `_reload`, ... : the bone and the frame in it).
pub struct Gun {
    rig: Rig,
    contacts: [usize; 2],
    locators: Vec<(String, usize, Frame)>,
}

impl Gun {
    /// `properties`: the model's `render/export_properties` (its locators).
    pub fn new(rig: Rig, properties: &ConfigValue) -> Result<Self> {
        let contact = |name: &str| {
            rig.index(name)
                .with_context(|| format!("the gun has no {name}"))
        };
        let contacts = [contact(HANDS[0].1)?, contact(HANDS[1].1)?];
        let mut locators = vec![];
        if let Some(ConfigValue::Indexed(list)) = properties.named("locators") {
            for l in list {
                let (
                    Some(ConfigValue::String(name)),
                    Some(ConfigValue::String(bone)),
                    Some(ConfigValue::Float3(position)),
                    Some(ConfigValue::Float3(rotation)),
                ) = (
                    l.named("name"),
                    l.named("bone_name"),
                    l.named("position"),
                    l.named("rotation"),
                )
                else {
                    continue;
                };
                if !HANDS
                    .iter()
                    .any(|(h, _)| name.starts_with(&format!("{h}_")))
                {
                    continue;
                }
                // A locator on no bone (0.26g's AK-12 `*_reload`) is on the root.
                let bone = rig.index(bone).unwrap_or(0);
                locators.push((name.clone(), bone, (euler_to_basis(*rotation), *position)));
            }
        }
        Ok(Self {
            rig,
            contacts,
            locators,
        })
    }

    /// The hand's locator of `kind`, else its `_idle` one (as
    /// `initialize_locators`, 0x392AE0, fills them).
    fn locator(&self, hand: usize, kind: Locator) -> Option<(usize, &Frame)> {
        let find = |suffix: &str| {
            let name = format!("{}_{suffix}", HANDS[hand].0);
            self.locators
                .iter()
                .find(|l| l.0 == name)
                .map(|l| (l.1, &l.2))
        };
        find(kind.suffix()).or_else(|| find(Locator::Idle.suffix()))
    }
}

/// Each hand's distance from its contact bone (metres), a sample a frame.
#[derive(Clone, Debug)]
pub struct Contact {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Contact {
    /// The median of both hands' samples.
    pub fn median(&self) -> f32 {
        let mut all: Vec<f32> = self.left.iter().chain(&self.right).copied().collect();
        all.sort_by(f32::total_cmp);
        all[all.len() / 2]
    }
    pub fn worst(&self) -> f32 {
        self.left
            .iter()
            .chain(&self.right)
            .copied()
            .fold(0.0, f32::max)
    }
}

/// The frames a clip is sampled at: every whole frame and its end.
fn frames_of(clip: &Clip) -> Vec<f32> {
    let end = clip.duration();
    let mut times: Vec<f32> = (0..=end.floor() as u32).map(|f| f as f32).collect();
    if end > times[times.len() - 1] + 1e-3 {
        times.push(end);
    }
    times
}

/// Where `hands` on `stance` puts each hand against `gun` playing `item`.
pub fn contact(
    skeleton: &Skeleton,
    stance: &Stance,
    hands: &Clip,
    gun: &Gun,
    item: &Clip,
) -> Result<Contact> {
    let weapon = skeleton.index(WEAPON)?;
    let hand_bones = [skeleton.index(HANDS[0].0)?, skeleton.index(HANDS[1].0)?];
    let mut out = Contact {
        left: vec![],
        right: vec![],
    };
    for time in frames_of(hands) {
        let player = pose(skeleton, stance, hands, time)?;
        let carried = (multiply(&HALF_TURN, &player[weapon].0), player[weapon].1);
        let gun_pose = gun.rig.pose(item, time.min(item.duration()));
        for (side, (bone, contact)) in hand_bones.iter().zip(gun.contacts).enumerate() {
            let at = compose(&gun_pose[contact], &carried).1;
            let d = (0..3)
                .map(|k| (player[*bone].1[k] - at[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            if side == 0 {
                out.left.push(d)
            } else {
                out.right.push(d)
            }
        }
    }
    Ok(out)
}

/// The most the hands and the gun (`Weapon`) of `a` (a clip on a stance)
/// are from `b`'s, frame by frame, in `reference`'s frame (metres).
pub fn drift(
    skeleton: &Skeleton,
    a: (&Stance, &Clip),
    b: (&Stance, &Clip),
    reference: &str,
) -> Result<f32> {
    let reference = skeleton.index(reference)?;
    let bones = [
        skeleton.index(HANDS[0].0)?,
        skeleton.index(HANDS[1].0)?,
        skeleton.index(WEAPON)?,
    ];
    let mut worst = 0.0f32;
    for time in frames_of(a.1) {
        let (pa, pb) = (
            pose(skeleton, a.0, a.1, time)?,
            pose(skeleton, b.0, b.1, time)?,
        );
        for bone in bones {
            let (ra, rb) = (
                relative(&pa[bone], &pa[reference]).1,
                relative(&pb[bone], &pb[reference]).1,
            );
            worst = worst.max((0..3).map(|k| (ra[k] - rb[k]).powi(2)).sum::<f32>().sqrt());
        }
    }
    Ok(worst)
}

/// A hands clip of a weapon state: the state (its key under `states`),
/// the clip and the gun's clip of the same name in the state, else its
/// first.
#[derive(Clone, Debug)]
pub struct HandsClip {
    pub state: String,
    pub user: String,
    pub item: String,
}

/// Every state's (and substate's) hands clips, in order.
pub fn hands_pairs(states: &ConfigValue) -> Vec<HandsClip> {
    fn walk(v: &ConfigValue, state: &str, out: &mut Vec<HandsClip>) {
        let ConfigValue::Named(fields) = v else {
            return;
        };
        let strings = |key: &str| -> Vec<String> {
            match v.named(key) {
                Some(ConfigValue::Indexed(list)) => list
                    .iter()
                    .filter_map(|c| match c {
                        ConfigValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            }
        };
        let (users, items) = (strings("user_animations"), strings("animations"));
        let stem = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
        for user in &users {
            let item = items
                .iter()
                .find(|i| stem(i) == stem(user))
                .or(items.first());
            if let Some(item) = item {
                out.push(HandsClip {
                    state: state.to_owned(),
                    user: user.clone(),
                    item: item.clone(),
                });
            }
        }
        for f in fields {
            walk(&f.value, state, out);
        }
    }
    let mut out = vec![];
    if let ConfigValue::Named(fields) = states {
        for f in fields {
            walk(&f.value, &f.name, &mut out);
        }
    }
    out
}

/// A weapon config's hands clips against its gun, on the first-person
/// stance of its `user_animations` set or on `stance` (a body clip): per
/// hands clip, the contact. `read` serves a file as the game would (the
/// stock db under the patches and packs).
pub fn gun_contact(
    read: &mut dyn FnMut(&str) -> Result<Vec<u8>>,
    config: &ConfigValue,
    stance: Option<&str>,
) -> Result<Vec<(String, Contact)>> {
    let string = |path: &str| -> Result<String> {
        let mut at = config;
        for part in path.split('.') {
            at = at
                .named(part)
                .with_context(|| format!("the config has no {path}"))?;
        }
        match at {
            ConfigValue::String(s) => Ok(s.clone()),
            _ => anyhow::bail!("{path} is not a string"),
        }
    };
    let clip = |read: &mut dyn FnMut(&str) -> Result<Vec<u8>>, name: &str| -> Result<Clip> {
        let name = name.trim_start_matches("resources/");
        Clip::parse(
            &read(&format!("{name}.b-spline"))?,
            &read(&format!("{name}.bones_names"))?,
        )
        .with_context(|| name.to_owned())
    };
    let skeleton = Skeleton::parse(&read("animations/skeletons/scavengers_01.skeleton")?)?;
    let (model, gun_skeleton) = (string("object.model")?, string("object.skeleton")?);
    let gun = Gun::new(
        Rig::parse(
            &read(&format!("models/{model}.skinned_model/render/bind_pose"))?,
            &read(&format!("animations/skeletons/{gun_skeleton}.skeleton"))?,
        )?,
        &BinaryConfig::parse(&read(&format!(
            "models/{model}.skinned_model/render/export_properties"
        ))?)?
        .root,
    )?;
    let body = match stance {
        Some(s) => s.to_owned(),
        None => {
            let set = BinaryConfig::parse(&read(&string("user_animations")?)?)?;
            match set.root.named("stand_hud") {
                Some(ConfigValue::Indexed(l)) => match l.first() {
                    Some(ConfigValue::String(s)) => s.clone(),
                    _ => anyhow::bail!("stand_hud[0] is not a clip"),
                },
                _ => anyhow::bail!("the set has no stand_hud"),
            }
        }
    };
    let stance = Stance::of(&skeleton, &clip(read, &body)?)?;
    let pairs = config.named("states").map(hands_pairs).unwrap_or_default();
    let mut out: Vec<(String, Contact)> = vec![];
    for HandsClip { user, item, .. } in pairs {
        if out.iter().any(|(u, _)| *u == user) {
            continue;
        }
        let hands = clip(read, &user)?;
        if hands.animation_type != ADDITIVE {
            continue;
        }
        let c = contact(&skeleton, &stance, &hands, &gun, &clip(read, &item)?)?;
        out.push((user, c));
    }
    Ok(out)
}

/// The Euler angles of `basis` nearest `near` (either solution, each angle
/// within half a turn of `near`'s).
fn nearest_angles(basis: &Basis, near: [f32; 3]) -> Result<[f32; 3]> {
    use std::f32::consts::{PI, TAU};
    let e = basis_to_euler(basis);
    let candidates = [e, [e[0] + PI, PI - e[1], e[2] + PI]];
    let mut best: Option<([f32; 3], f32)> = None;
    for c in candidates {
        let c: [f32; 3] = std::array::from_fn(|k| c[k] - TAU * ((c[k] - near[k]) / TAU).round());
        let back = euler_to_basis(c);
        let error = (0..3)
            .flat_map(|r| (0..3).map(move |k| (r, k)))
            .map(|(r, k)| (back[r][k] - basis[r][k]).abs())
            .fold(0.0, f32::max);
        if error > 1e-4 {
            continue;
        }
        let distance: f32 = (0..3).map(|k| (c[k] - near[k]).abs()).sum();
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((c, distance));
        }
    }
    best.map(|(c, _)| c)
        .context("no Euler angles reproduce a re-based rotation")
}

/// `hands`, played on `from`, re-based onto `to`: on `to` it puts the arms
/// (the clavicles and all under them) where it put them on `from` relative
/// to the chest, and the bones the player carries things on (`Root`'s
/// children but the hip: `Weapon`, `Pistol`, `Item`) where it put them
/// relative to the chest too. Every other bone's curves stay as they are.
/// The changed bones' translation and rotation curves are sampled a frame
/// and fitted again ([`Channel::fit`]); their scale curves stay.
pub fn rebase(skeleton: &Skeleton, hands: &Clip, from: &Stance, to: &Stance) -> Result<Clip> {
    let chest = skeleton.index(CHEST)?;
    let root = skeleton.index(ROOT)?;
    let hip = skeleton.index(HIP)?;
    let arms = ARMS
        .iter()
        .map(|a| skeleton.index(a))
        .collect::<Result<Vec<_>>>()?;
    let moved: Vec<usize> = (0..skeleton.names.len())
        .filter(|b| {
            arms.iter().any(|a| skeleton.is_under(*b, *a))
                || (skeleton.parents[*b] == Some(root) && *b != hip)
        })
        .collect();
    let times = frames_of(hands);
    // Per moved bone, per frame: the additive translation and angles.
    let mut samples: Vec<Vec<([f32; 3], [f32; 3])>> =
        vec![Vec::with_capacity(times.len()); moved.len()];
    for time in &times {
        let drawn = pose(skeleton, from, hands, *time)?;
        let ours = pose(skeleton, to, hands, *time)?;
        for (slot, bone) in moved.iter().enumerate() {
            let name = &skeleton.names[*bone];
            // The local frame the bone must have on `to`.
            let local: Frame = if arms.iter().any(|a| skeleton.is_under(*bone, *a)) {
                let (base_t, base_r) = from.locals[*bone];
                let (add_t, add_r) = channels(hands, name, *time).unwrap_or(([0.0; 3], [0.0; 3]));
                (
                    multiply(&euler_to_basis(base_r), &euler_to_basis(add_r)),
                    [0, 1, 2].map(|k| base_t[k] + add_t[k]),
                )
            } else {
                let in_chest = relative(&drawn[*bone], &drawn[chest]);
                relative(&compose(&in_chest, &ours[chest]), &ours[root])
            };
            let (base_t, base_r) = to.locals[*bone];
            let rotation = multiply(&transpose(&euler_to_basis(base_r)), &local.0);
            let translation = [0, 1, 2].map(|k| local.1[k] - base_t[k]);
            let near = samples[slot]
                .last()
                .map(|s| s.1)
                .or_else(|| channels(hands, name, *time).map(|c| c.1))
                .unwrap_or([0.0; 3]);
            let angles = nearest_angles(&rotation, near)
                .with_context(|| format!("{name} at frame {time}"))?;
            samples[slot].push((translation, angles));
        }
    }
    let mut out = hands.clone();
    for (slot, bone) in moved.iter().enumerate() {
        let name = &skeleton.names[*bone];
        let track = out
            .bones
            .iter()
            .position(|b| b == name)
            .with_context(|| format!("the hands clip does not drive {name}"))?;
        for k in 0..6 {
            let values: Vec<f32> = samples[slot]
                .iter()
                .map(|(t, r)| if k < 3 { t[k] } else { r[k - 3] })
                .collect();
            let channel = Channel::fit(&times, &values)?;
            let residual = channel.residual(&times, &values);
            ensure!(
                residual < 1e-4,
                "{name}'s re-based channel {k} fits within {residual}"
            );
            out.tracks[track].channels[k] = channel;
        }
    }
    Ok(out)
}

/// Where the engine's hand IK puts a hand (0.26g `hand_to_weapon_ik_solver`,
/// the modes of 0x392D90).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The hand where the clip puts it (IK off).
    Own,
    /// The gun's `*_hand_cont` bone.
    Contact,
    /// The model's `<Hand>_<name>` locator (`_idle` where it has no other).
    Locator(Locator),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locator {
    Idle,
    Reload,
    ReloadPivot,
    Lobby,
}

impl Locator {
    fn suffix(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Reload => "reload",
            Self::ReloadPivot => "reload_pivot",
            Self::Lobby => "lobby",
        }
    }
}

/// How fast a target is taken: an odd event value 100 ms, an even one
/// 300 ms (0x392CE0: `hands_ik_short` / `_long_transition_time_ms`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Change {
    target: Target,
    milliseconds: f32,
}

/// A `*_hand_ik` event value, as 0x392D90 reads it.
fn change_of(value: u8) -> Result<Change> {
    let target = match value / 2 {
        0 => Target::Own,
        1 => Target::Contact,
        2 => Target::Locator(Locator::Idle),
        3 => Target::Locator(Locator::Reload),
        4 => Target::Locator(Locator::ReloadPivot),
        5 => Target::Locator(Locator::Lobby),
        _ => anyhow::bail!("hand IK event value {value} is none of 0..11"),
    };
    let milliseconds = if value % 2 == 1 { 100.0 } else { 300.0 };
    Ok(Change {
        target,
        milliseconds,
    })
}

/// One hand's IK: settled on a target, or blending from one to another
/// until `end` (milliseconds of the clip) over `length`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HandIk {
    Settled(Target),
    Blending {
        from: Target,
        to: Target,
        end: f32,
        length: f32,
    },
}

/// `fermi_interpolator(length, 0.005).interpolated_value(t)`.
fn fermi(length: f32, t: f32) -> f32 {
    let epsilon = 0.005f32;
    let t0 = length / 2.0;
    let d = t0 / (1.0 / epsilon + 1.0).ln();
    ((1.0 + 2.0 * epsilon) / (1.0 + (-(t - t0) / d).exp()) - epsilon).clamp(0.0, 1.0)
}

impl HandIk {
    /// How far a blend has gone from `from` to `to` at `now` (0x393A80).
    fn weight(end: f32, length: f32, now: f32) -> f32 {
        1.0 - fermi(length, end - now)
    }

    /// The state at `now`: a blend that has ended is settled.
    fn at(self, now: f32) -> Self {
        match self {
            Self::Blending { to, end, .. } if now >= end => Self::Settled(to),
            s => s,
        }
    }

    /// `activate_hand` (0x392CE0): a new target blends in from the current
    /// one; going back to the one it is blending from takes only the way
    /// already gone.
    fn change(self, change: Change, now: f32) -> Self {
        let length = change.milliseconds;
        match self.at(now) {
            Self::Settled(current) if current == change.target => self,
            Self::Settled(current) => Self::Blending {
                from: current,
                to: change.target,
                end: now + length,
                length,
            },
            Self::Blending { to, .. } if to == change.target => self,
            Self::Blending {
                from,
                to,
                end,
                length: old,
            } => {
                let back = if from == change.target {
                    (length * Self::weight(end, old, now) + 0.5).floor()
                } else {
                    length
                };
                Self::Blending {
                    from: to,
                    to: change.target,
                    end: now + back,
                    length,
                }
            }
        }
    }
}

/// `get_rotation_matrix(from, to)`: the turn taking direction `from` to
/// `to` (row vectors: `transform(from, &m)` is `to`).
fn turn_between(from: [f32; 3], to: [f32; 3]) -> Basis {
    let cross = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    let cos = (0..3)
        .map(|k| from[k] * to[k])
        .sum::<f32>()
        .clamp(-1.0, 1.0);
    let sin = (0..3).map(|k| cross[k] * cross[k]).sum::<f32>().sqrt();
    if sin < 1e-7 {
        return crate::skeleton_animation::IDENTITY_BASIS;
    }
    let axis = cross.map(|c| c / sin);
    turn_about(axis, sin.atan2(cos))
}

/// A turn by `angle` about `axis` (unit), row vectors, right-handed: it
/// takes a direction perpendicular to `axis` toward `axis` x it.
fn turn_about(axis: [f32; 3], angle: f32) -> Basis {
    let (s, c) = angle.sin_cos();
    let [x, y, z] = axis;
    let t = 1.0 - c;
    // Column-vector Rodrigues matrix, transposed for row vectors.
    let m = [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ];
    transpose(&m)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (0..3).map(|k| v[k] * v[k]).sum::<f32>().sqrt();
    v.map(|c| c / l)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    (0..3).map(|k| v[k] * v[k]).sum::<f32>().sqrt()
}

/// A basis as a unit quaternion (x, y, z, w) and back, for the blend's
/// `slerp`.
fn quaternion(b: &Basis) -> [f32; 4] {
    // Row-vector basis: its transpose is the column-vector rotation.
    let m = transpose(b);
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            s / 4.0,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            s / 4.0,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][1] + m[1][0]) / s,
            s / 4.0,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            s / 4.0,
            (m[1][0] - m[0][1]) / s,
        ]
    };
    let l = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    q.map(|c| c / l)
}

fn basis_of(q: [f32; 4]) -> Basis {
    let [x, y, z, w] = q;
    let m = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    transpose(&m)
}

/// `mix_transformations(first, second, k)`: the position along the line,
/// the turn by `slerp`.
fn mix(first: &Frame, second: &Frame, k: f32) -> Frame {
    let (a, mut b) = (quaternion(&first.0), quaternion(&second.0));
    let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    if dot < 0.0 {
        b = b.map(|c| -c);
        dot = -dot;
    }
    let q = if dot > 0.9995 {
        normalize4(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * k))
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        let (sa, sb) = (((1.0 - k) * theta).sin(), (k * theta).sin());
        normalize4(std::array::from_fn(|i| {
            (a[i] * sa + b[i] * sb) / theta.sin()
        }))
    };
    (
        basis_of(q),
        [0, 1, 2].map(|i| first.1[i] * (1.0 - k) + second.1[i] * k),
    )
}

fn normalize4(q: [f32; 4]) -> [f32; 4] {
    let l = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    q.map(|c| c / l)
}

/// `process_hand`: the arm (the hand's grandparent) and the forearm (its
/// parent) turned so the hand reaches `target`, and the hand turned as
/// `target`; `frames` are model-space and stay consistent below the hand.
/// Returns the new locals of the arm, the forearm and the hand.
fn reach(skeleton: &Skeleton, frames: &[Frame], hand: usize, target: &Frame) -> Option<[Frame; 3]> {
    let forearm = skeleton.parents[hand]?;
    let arm = skeleton.parents[forearm]?;
    let shoulder = skeleton.parents[arm]?;
    let forearm_len = length(sub(frames[hand].1, frames[forearm].1));
    let arm_len = length(sub(frames[forearm].1, frames[arm].1));
    let arm_pos = frames[arm].1;
    let to_hand = sub(target.1, arm_pos);
    let to_hand_len = length(to_hand);
    if to_hand_len < 1e-5 {
        return None;
    }
    let to_hand_dir = normalize(to_hand);
    let forearm_dir = normalize(sub(frames[forearm].1, arm_pos));
    let cross = [
        forearm_dir[1] * to_hand_dir[2] - forearm_dir[2] * to_hand_dir[1],
        forearm_dir[2] * to_hand_dir[0] - forearm_dir[0] * to_hand_dir[2],
        forearm_dir[0] * to_hand_dir[1] - forearm_dir[1] * to_hand_dir[0],
    ];
    // get_angle: the shoulder's angle between the arm and the line to the hand.
    let cos = ((arm_len * arm_len + to_hand_len * to_hand_len - forearm_len * forearm_len)
        / (2.0 * arm_len * to_hand_len))
        .clamp(-1.0, 1.0);
    let alpha = cos.acos();
    // The elbow keeps its side: the new arm direction is the line to the hand
    // turned back toward the old arm direction by alpha.
    let new_forearm_dir = if length(cross) < 1e-7 {
        forearm_dir
    } else {
        transform(to_hand_dir, &turn_about(normalize(cross), -alpha))
    };
    let arm_turn = turn_between(forearm_dir, new_forearm_dir);
    let arm_obj = (multiply(&frames[arm].0, &arm_turn), arm_pos);
    let forearm_local = relative(&frames[forearm], &frames[arm]);
    let forearm_obj = compose(&forearm_local, &arm_obj);
    let hand_local = relative(&frames[hand], &frames[forearm]);
    let hand_pos = compose(&hand_local, &forearm_obj).1;
    let forearm_turn = turn_between(
        normalize(sub(hand_pos, forearm_obj.1)),
        normalize(sub(target.1, forearm_obj.1)),
    );
    let forearm_obj = (multiply(&forearm_obj.0, &forearm_turn), forearm_obj.1);
    let mut hand_new = relative(target, &forearm_obj);
    let l = length(hand_new.1);
    if (l - forearm_len).abs() > 1e-5 {
        hand_new.1 = hand_new.1.map(|c| c / l * forearm_len);
    }
    Some([
        relative(&arm_obj, &frames[shoulder]),
        relative(&forearm_obj, &arm_obj),
        hand_new,
    ])
}

/// A bone's additive translation and Euler angles at a frame.
type Sample = ([f32; 3], [f32; 3]);

/// What the hand IK does to `hands` on `stance` with `gun` playing `item`,
/// baked into the clip: per frame, each hand's target as the clip's
/// `*_hand_ik` events (`events[hand]`: (frame, value)) set it, blended as
/// the engine blends, and the arm bent onto it as the engine bends it; the
/// arms', forearms' and hands' curves of the result. `milliseconds` is a
/// frame's length as the state plays the clip, `start` the target the
/// hands hold as the clip starts. With it, how far a hand fell short of its
/// target at worst (metres; a target out of the arm's reach).
#[allow(clippy::too_many_arguments)]
pub fn bake_ik(
    skeleton: &Skeleton,
    stance: &Stance,
    hands: &Clip,
    events: [&[(f32, u8)]; 2],
    gun: &Gun,
    item: &Clip,
    milliseconds: f32,
    start: Target,
) -> Result<(Clip, f32)> {
    let weapon = skeleton.index(WEAPON)?;
    let hand_bones = [skeleton.index(HANDS[0].0)?, skeleton.index(HANDS[1].0)?];
    let chains: Vec<[usize; 3]> = hand_bones
        .iter()
        .map(|h| {
            let forearm = skeleton.parents[*h].context("a hand without a forearm")?;
            let arm = skeleton.parents[forearm].context("a forearm without an arm")?;
            Ok([arm, forearm, *h])
        })
        .collect::<Result<_>>()?;
    let changes: Vec<Vec<(f32, Change)>> = events
        .iter()
        .map(|e| {
            e.iter()
                .map(|(k, v)| Ok((*k, change_of(*v)?)))
                .collect::<Result<_>>()
        })
        .collect::<Result<_>>()?;
    let times = frames_of(hands);
    let mut states = [HandIk::Settled(start), HandIk::Settled(start)];
    let mut next = [0usize, 0];
    let mut short = 0.0f32;
    // Per hand, per chain bone, per frame: (translation, angles) additive.
    let mut samples: [[Vec<Sample>; 3]; 2] = Default::default();
    for time in &times {
        let now = time * milliseconds;
        let mut player = pose(skeleton, stance, hands, *time)?;
        let carried = (multiply(&HALF_TURN, &player[weapon].0), player[weapon].1);
        let gun_pose = gun.rig.pose(item, time.min(item.duration()));
        for side in 0..2 {
            while let Some((knot, change)) = changes[side].get(next[side]) {
                if *knot > *time + 1e-4 {
                    break;
                }
                states[side] = states[side].change(*change, knot * milliseconds);
                next[side] += 1;
            }
            let target_of = |t: Target, player: &[Frame]| -> Result<Frame> {
                Ok(match t {
                    Target::Own => player[hand_bones[side]],
                    Target::Contact => compose(&gun_pose[gun.contacts[side]], &carried),
                    Target::Locator(l) => {
                        let (bone, frame) = gun
                            .locator(side, l)
                            .with_context(|| format!("the gun has no {}_idle", HANDS[side].0))?;
                        compose(&compose(frame, &gun_pose[bone]), &carried)
                    }
                })
            };
            let target = match states[side].at(now) {
                HandIk::Settled(t) => target_of(t, &player)?,
                HandIk::Blending {
                    from,
                    to,
                    end,
                    length,
                } => mix(
                    &target_of(from, &player)?,
                    &target_of(to, &player)?,
                    HandIk::weight(end, length, now),
                ),
            };
            let chain = chains[side];
            let locals = match reach(skeleton, &player, hand_bones[side], &target) {
                Some(l) => l,
                None => [
                    relative(
                        &player[chain[0]],
                        &player[skeleton.parents[chain[0]].unwrap()],
                    ),
                    relative(&player[chain[1]], &player[chain[0]]),
                    relative(&player[chain[2]], &player[chain[1]]),
                ],
            };
            // The chain's new model frames, for the next hand's frames and the samples.
            let shoulder = skeleton.parents[chain[0]].unwrap();
            let arm = compose(&locals[0], &player[shoulder]);
            let forearm = compose(&locals[1], &arm);
            let hand = compose(&locals[2], &forearm);
            short = short.max(length(sub(hand.1, target.1)));
            for (bone, frame) in chain.iter().zip([arm, forearm, hand]) {
                player[*bone] = frame;
            }
            for (k, bone) in chain.iter().enumerate() {
                let (base_t, base_r) = stance.locals[*bone];
                let rotation = multiply(&transpose(&euler_to_basis(base_r)), &locals[k].0);
                let translation = [0, 1, 2].map(|i| locals[k].1[i] - base_t[i]);
                let name = &skeleton.names[*bone];
                let near = samples[side][k]
                    .last()
                    .map(|s| s.1)
                    .or_else(|| channels(hands, name, *time).map(|c| c.1))
                    .unwrap_or([0.0; 3]);
                let angles = nearest_angles(&rotation, near)
                    .with_context(|| format!("{name} at frame {time}"))?;
                samples[side][k].push((translation, angles));
            }
        }
    }
    let mut out = hands.clone();
    for side in 0..2 {
        for (k, bone) in chains[side].iter().enumerate() {
            let name = &skeleton.names[*bone];
            let track = out
                .bones
                .iter()
                .position(|b| b == name)
                .with_context(|| format!("the hands clip does not drive {name}"))?;
            for c in 0..6 {
                let values: Vec<f32> = samples[side][k]
                    .iter()
                    .map(|(t, r)| if c < 3 { t[c] } else { r[c - 3] })
                    .collect();
                let channel = Channel::fit(&times, &values)?;
                let residual = channel.residual(&times, &values);
                ensure!(
                    residual < 1e-4,
                    "{name}'s IK channel {c} fits within {residual}"
                );
                out.tracks[track].channels[c] = channel;
            }
        }
    }
    Ok((out, short))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_config::NamedValue;
    use crate::skeleton_animation::BoneTracks;

    fn named(name: &str, children: Vec<NamedValue>) -> NamedValue {
        let mut value = vec![NamedValue {
            name: "mask".into(),
            value: ConfigValue::Integer(1),
        }];
        value.extend(children);
        NamedValue {
            name: name.into(),
            value: ConfigValue::Named(value),
        }
    }

    /// object_movement - Root - (Hip - Chest - (LeftClavicle - LeftHand,
    /// RightClavicle - RightHand), Weapon)
    fn skeleton() -> Skeleton {
        let tree = ConfigValue::Named(vec![named(
            ROOT_MOTION_CHANNEL,
            vec![named(
                ROOT,
                vec![
                    named(
                        HIP,
                        vec![named(
                            CHEST,
                            vec![
                                named("LeftClavicle", vec![named("LeftHand", vec![])]),
                                named("RightClavicle", vec![named("RightHand", vec![])]),
                            ],
                        )],
                    ),
                    named(WEAPON, vec![]),
                ],
            )],
        )]);
        Skeleton::parse(&BinaryConfig { root: tree }.to_bytes().unwrap()).unwrap()
    }

    /// A bone moving from (translation, angles) to (translation, angles).
    type Move<'a> = (&'a str, [f32; 3], [f32; 3], [f32; 3], [f32; 3]);

    fn clip(bones: &[Move], kind: u8) -> Clip {
        // Each bone moves linearly from its first frame to its second over 10 frames.
        let times: Vec<f32> = (0..=10).map(|f| f as f32).collect();
        let curve = |a: f32, b: f32| {
            let values: Vec<f32> = times.iter().map(|t| a + (b - a) * t / 10.0).collect();
            Channel::fit(&times, &values).unwrap()
        };
        Clip {
            bones: bones.iter().map(|b| b.0.to_string()).collect(),
            tracks: bones
                .iter()
                .map(|(_, t0, r0, t1, r1)| BoneTracks {
                    channels: [
                        curve(t0[0], t1[0]),
                        curve(t0[1], t1[1]),
                        curve(t0[2], t1[2]),
                        curve(r0[0], r1[0]),
                        curve(r0[1], r1[1]),
                        curve(r0[2], r1[2]),
                        Channel::constant(1.0, 10.0),
                        Channel::constant(1.0, 10.0),
                        Channel::constant(1.0, 10.0),
                    ],
                })
                .collect(),
            animation_type: kind,
        }
    }

    fn stance(s: &Skeleton, hip: [f32; 3], chest: [f32; 3]) -> Stance {
        let z = [0.0; 3];
        let c = clip(
            &[
                (
                    ROOT,
                    z,
                    [0.0, std::f32::consts::PI, 0.0],
                    z,
                    [0.0, std::f32::consts::PI, 0.0],
                ),
                (HIP, [0.0, 0.9, 0.0], hip, [0.0, 0.9, 0.0], hip),
                (CHEST, [0.0, 0.3, 0.0], chest, [0.0, 0.3, 0.0], chest),
                (
                    "LeftClavicle",
                    [0.1, 0.2, 0.0],
                    [0.1, 0.0, 0.2],
                    [0.1, 0.2, 0.0],
                    [0.1, 0.0, 0.2],
                ),
                ("LeftHand", [0.5, 0.0, 0.0], z, [0.5, 0.0, 0.0], z),
                (
                    "RightClavicle",
                    [-0.1, 0.2, 0.0],
                    [0.0, 0.3, 0.1],
                    [-0.1, 0.2, 0.0],
                    [0.0, 0.3, 0.1],
                ),
                ("RightHand", [-0.5, 0.0, 0.0], z, [-0.5, 0.0, 0.0], z),
                (WEAPON, [0.0, 1.3, 0.3], z, [0.0, 1.3, 0.3], z),
            ],
            0,
        );
        Stance::of(s, &c).unwrap()
    }

    /// A hand in the `Weapon` bone's frame.
    fn hand_in_weapon(s: &Skeleton, st: &Stance, hands: &Clip, hand: &str, t: f32) -> [f32; 3] {
        let p = pose(s, st, hands, t).unwrap();
        relative(&p[s.index(hand).unwrap()], &p[s.index(WEAPON).unwrap()]).1
    }

    #[test]
    fn a_rebased_clip_keeps_the_hands_where_they_were_on_the_gun() {
        let s = skeleton();
        let drawn = stance(&s, [0.2, 0.0, 0.0], [0.1, 0.05, 0.0]);
        let ours = stance(&s, [0.0, 0.1, 0.0], [-0.2, 0.0, 0.1]);
        let z = [0.0; 3];
        let hands = clip(
            &[
                ("LeftClavicle", z, [0.3, 0.2, 0.0], z, [0.5, -0.1, 0.2]),
                (
                    "LeftHand",
                    [0.0, 0.05, 0.0],
                    [0.2, 0.0, 0.0],
                    z,
                    [0.0, 0.4, 0.0],
                ),
                ("RightClavicle", z, [0.0, 0.1, 0.0], z, [0.2, 0.0, 0.3]),
                ("RightHand", z, z, [0.01, 0.0, 0.0], [0.1, 0.0, 0.0]),
                (
                    WEAPON,
                    [0.0, 0.02, 0.01],
                    [0.0, 0.1, 0.0],
                    [0.01, 0.0, 0.0],
                    z,
                ),
            ],
            ADDITIVE,
        );
        let rebased = rebase(&s, &hands, &drawn, &ours).unwrap();
        // At the frames it was sampled at, exactly; between them, as the curves run.
        for (t, within) in [
            (0.0, 1e-3),
            (2.0, 1e-3),
            (2.5, 5e-3),
            (5.0, 1e-3),
            (10.0, 1e-3),
        ] {
            for hand in ["LeftHand", "RightHand"] {
                let want = hand_in_weapon(&s, &drawn, &hands, hand, t);
                let before = hand_in_weapon(&s, &ours, &hands, hand, t);
                let got = hand_in_weapon(&s, &ours, &rebased, hand, t);
                let d = |a: [f32; 3], b: [f32; 3]| {
                    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
                };
                assert!(d(want, before) > 0.01, "the stances differ for {hand}");
                assert!(
                    d(want, got) < within,
                    "{hand} at {t}: {:?} against {:?}",
                    got,
                    want
                );
            }
        }
        // The rest of the body keeps its curves.
        let hip = rebased.bones.iter().position(|b| b == HIP);
        assert!(hip.is_none(), "the hip was not in the clip and is not now");
    }

    #[test]
    fn a_clip_on_its_own_stance_rebases_to_itself() {
        let s = skeleton();
        let st = stance(&s, [0.2, 0.0, 0.0], [0.1, 0.05, 0.0]);
        let z = [0.0; 3];
        let hands = clip(
            &[
                ("LeftClavicle", z, [0.3, 0.2, 0.0], z, [0.5, -0.1, 0.2]),
                ("LeftHand", z, z, z, z),
                ("RightClavicle", z, z, z, z),
                ("RightHand", z, z, z, z),
                (WEAPON, z, z, z, z),
            ],
            ADDITIVE,
        );
        let rebased = rebase(&s, &hands, &st, &st).unwrap();
        let track = rebased
            .bones
            .iter()
            .position(|b| b == "LeftClavicle")
            .unwrap();
        for t in [0.0, 3.0, 7.5, 10.0] {
            for k in 0..6 {
                let a = hands.tracks[0].channels[k].evaluate(t);
                let b = rebased.tracks[track].channels[k].evaluate(t);
                assert!((a - b).abs() < 1e-4, "channel {k} at {t}: {b} for {a}");
            }
        }
    }

    #[test]
    fn the_arm_reaches_its_target_and_keeps_its_lengths() {
        let s = skeleton();
        let st = stance(&s, [0.2, 0.0, 0.0], [0.1, 0.05, 0.0]);
        let none = Clip {
            bones: vec![],
            tracks: vec![],
            animation_type: ADDITIVE,
        };
        let frames = pose(&s, &st, &none, 0.0).unwrap();
        // The test rig's arm: LeftClavicle (as the "arm") - LeftHand has no
        // forearm, so give the chain Chest - LeftClavicle - LeftHand: hand,
        // forearm = clavicle, arm = chest.
        let hand = s.index("LeftHand").unwrap();
        let (forearm, arm) = (
            s.parents[hand].unwrap(),
            s.parents[s.parents[hand].unwrap()].unwrap(),
        );
        let upper = length(sub(frames[forearm].1, frames[arm].1));
        let lower = length(sub(frames[hand].1, frames[forearm].1));
        let goal = compose(
            &(euler_to_basis([0.3, -0.2, 0.1]), [0.25, -0.15, 0.2]),
            &frames[arm],
        );
        assert!(length(sub(goal.1, frames[arm].1)) < upper + lower);
        let [a, f, h] = reach(&s, &frames, hand, &goal).unwrap();
        let arm_obj = compose(&a, &frames[s.parents[arm].unwrap()]);
        let forearm_obj = compose(&f, &arm_obj);
        let hand_obj = compose(&h, &forearm_obj);
        assert!(
            length(sub(hand_obj.1, goal.1)) < 1e-4,
            "{:?} for {:?}",
            hand_obj.1,
            goal.1
        );
        assert!((length(f.1) - upper).abs() < 1e-5 && (length(h.1) - lower).abs() < 1e-5);
        let turn = multiply(&hand_obj.0, &transpose(&goal.0));
        assert!(
            (turn[0][0] + turn[1][1] + turn[2][2] - 3.0).abs() < 1e-4,
            "the hand is turned as the target"
        );
    }

    #[test]
    fn a_hand_blends_to_a_new_target_and_settles() {
        let ik = HandIk::Settled(Target::Locator(Locator::Idle));
        let ik = ik.change(change_of(2).unwrap(), 1000.0);
        assert_eq!(
            ik,
            HandIk::Blending {
                from: Target::Locator(Locator::Idle),
                to: Target::Contact,
                end: 1300.0,
                length: 300.0
            }
        );
        let HandIk::Blending { end, length, .. } = ik else {
            unreachable!()
        };
        assert!(HandIk::weight(end, length, 1000.0) < 0.01);
        assert!((HandIk::weight(end, length, 1150.0) - 0.5).abs() < 0.01);
        assert!(HandIk::weight(end, length, 1299.0) > 0.99);
        assert_eq!(ik.at(1300.0), HandIk::Settled(Target::Contact));
        // An odd value: the short transition.
        let quick = HandIk::Settled(Target::Contact).change(change_of(1).unwrap(), 0.0);
        assert!(matches!(quick, HandIk::Blending { to: Target::Own, end, .. } if end == 100.0));
        assert!(change_of(12).is_err());
    }

    #[test]
    fn a_full_clip_is_not_a_hands_clip() {
        let s = skeleton();
        let st = stance(&s, [0.0; 3], [0.0; 3]);
        let full = clip(&[("LeftHand", [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3])], 0);
        assert!(pose(&s, &st, &full, 0.0).is_err());
    }
}
