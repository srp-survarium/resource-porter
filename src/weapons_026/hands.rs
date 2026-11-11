// SPDX-License-Identifier: GPL-3.0-or-later
//! A gun's hands as 0.26g draws them, and a hands clip moved onto 0.26g's
//! first-person stance.
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
//! 0.1 mm) and play them in the first view on another, where the IK covers
//! the 1 to 26 cm between hand and contact point. Their rifles' first-person
//! stance holds the gun where 0.26g's does (to 1 mm), so their rifles play
//! on 0.26g as on the donor; their pistols' holds it 22 cm further out, so
//! a pistol's clips are re-based ([`rebase`]) onto 0.26g's stance from the
//! donor's first-person one: the arms and the gun where the donor's first
//! view has them, relative to the chest (0.26g's head stands on its chest
//! as the donors' first-person head does, so the view is the donor's too).

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

/// A gun's skeleton with its two contact bones.
pub struct Gun {
    rig: Rig,
    contacts: [usize; 2],
}

impl Gun {
    pub fn new(rig: Rig) -> Result<Self> {
        let contact = |name: &str| {
            rig.index(name)
                .with_context(|| format!("the gun has no {name}"))
        };
        let contacts = [contact(HANDS[0].1)?, contact(HANDS[1].1)?];
        Ok(Self { rig, contacts })
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

/// Each state's (and substate's) hands clips, each with the gun's clip of
/// the same name in it, else its first.
pub fn hands_pairs(v: &ConfigValue, out: &mut Vec<(String, String)>) {
    let ConfigValue::Named(fields) = v else {
        return;
    };
    let strings = |key: &str| -> Vec<String> {
        match v.named(key) {
            Some(ConfigValue::Indexed(list)) => list
                .iter()
                .filter_map(|c| {
                    match c {
                        ConfigValue::String(s) => Some(s.as_str()),
                        _ => None,
                    }
                    .map(str::to_owned)
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
            out.push((user.clone(), item.clone()));
        }
    }
    for f in fields {
        hands_pairs(&f.value, out);
    }
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
    let gun = Gun::new(Rig::parse(
        &read(&format!("models/{model}.skinned_model/render/bind_pose"))?,
        &read(&format!("animations/skeletons/{gun_skeleton}.skeleton"))?,
    )?)?;
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
    let mut pairs = vec![];
    if let Some(states) = config.named("states") {
        hands_pairs(states, &mut pairs);
    }
    let mut out: Vec<(String, Contact)> = vec![];
    for (user, item) in pairs {
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
                (ROOT, z, [0.0, std::f32::consts::PI, 0.0], z, [0.0, std::f32::consts::PI, 0.0]),
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
    fn a_full_clip_is_not_a_hands_clip() {
        let s = skeleton();
        let st = stance(&s, [0.0; 3], [0.0; 3]);
        let full = clip(&[("LeftHand", [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3])], 0);
        assert!(pose(&s, &st, &full, 0.0).is_err());
    }
}
