//! Retarget a Shadow of Chernobyl motion onto the v0.100b character rig.
//!
//! Both engines store a pose the same way — every bone carries a complete
//! local frame, replaced outright by the animation rather than added to a rest
//! pose — and both sample at 30 frames per second (`SAMPLE_FPS` there,
//! `default_fps` here), so no resampling is needed and frame *n* of the source
//! is frame *n* of the result.
//!
//! What differs is the rig. X-Ray's `bip01` skeleton has 45 bones, v0.100b's
//! has 69 plus a root-motion channel, and the two were bound in different rest
//! poses with different bone axes. Mapping local rotations across by name
//! would twist every limb by whatever the rest poses disagree by, so the
//! transfer is made in model space instead:
//!
//! 1. accumulate the source bone's model-space orientation for the frame;
//! 2. take it relative to the *source* rest pose, which is the motion's own
//!    zero — this is the part that is genuinely rig-independent;
//! 3. apply that same model-space change to the *target* rest pose;
//! 4. read the result back as a local frame for the target's parent.
//!
//! Bones the source has no counterpart for — the twist bones, the ring and
//! pinky fingers, the weapon attachment points — hold their rest frame, which
//! is what a v0.100b clip does for a bone it does not drive anyway.
//!
//! Bone lengths stay the target's: only rotations cross over, plus the pelvis
//! displacement, which is where X-Ray keeps a motion's travel.

use anyhow::{Context, Result, bail};

use crate::skeleton_animation::{
    Basis, BoneTracks, CHANNELS, Channel, Clip, IDENTITY_BASIS, Rig, basis_to_euler, multiply,
    transform, transpose,
};
use crate::soc::codec::{omf, skeleton};

/// The root-motion channel every v0.100b clip carries first.
pub const ROOT_MOTION_BONE: &str = "object_movement";

/// Which X-Ray bone drives which v0.100b bone.
///
/// X-Ray numbers fingers `finger0`/`finger1`/`finger2` for thumb, index and
/// middle and has no ring or little finger; it also has no twist bones, no
/// separate knee helper, and one toe. Every target bone missing from this
/// table keeps its rest frame.
pub const BONE_MAP: &[(&str, &str)] = &[
    ("bip01_pelvis", "Hip"),
    ("bip01_spine", "Spin"),
    ("bip01_spine1", "Spin_1"),
    ("bip01_spine2", "Chest"),
    ("bip01_neck", "Neck"),
    ("bip01_head", "Head"),
    ("jaw_1", "Jaw"),
    ("eye_left", "LeftEye"),
    ("eye_right", "RightEye"),
    ("bip01_l_clavicle", "LeftClavicle"),
    ("bip01_l_upperarm", "LeftArm"),
    ("bip01_l_forearm", "LeftForeArm"),
    ("bip01_l_hand", "LeftHand"),
    ("bip01_l_finger0", "LeftThumb1"),
    ("bip01_l_finger01", "LeftThumb2"),
    ("bip01_l_finger02", "LeftThumb3"),
    ("bip01_l_finger1", "LeftIndex1"),
    ("bip01_l_finger11", "LeftIndex2"),
    ("bip01_l_finger12", "LeftIndex3"),
    ("bip01_l_finger2", "LeftMiddle1"),
    ("bip01_l_finger21", "LeftMiddle2"),
    ("bip01_l_finger22", "LeftMiddle3"),
    ("bip01_l_thigh", "LeftUpLeg"),
    ("bip01_l_calf", "LeftLeg"),
    ("bip01_l_foot", "LeftFoot"),
    ("bip01_l_toe0", "LeftToe"),
    ("bip01_r_clavicle", "RightClavicle"),
    ("bip01_r_upperarm", "RightArm"),
    ("bip01_r_forearm", "RightForeArm"),
    ("bip01_r_hand", "RightHand"),
    ("bip01_r_finger0", "RightThumb1"),
    ("bip01_r_finger01", "RightThumb2"),
    ("bip01_r_finger02", "RightThumb3"),
    ("bip01_r_finger1", "RightIndex1"),
    ("bip01_r_finger11", "RightIndex2"),
    ("bip01_r_finger12", "RightIndex3"),
    ("bip01_r_finger2", "RightMiddle1"),
    ("bip01_r_finger21", "RightMiddle2"),
    ("bip01_r_finger22", "RightMiddle3"),
    ("bip01_r_thigh", "RightUpLeg"),
    ("bip01_r_calf", "RightLeg"),
    ("bip01_r_foot", "RightFoot"),
    ("bip01_r_toe0", "RightToe"),
];

/// How the two rigs are oriented relative to each other.
///
/// They are not oriented the same way: X-Ray's left limbs sit at positive X
/// and its toes point along -Z, while v0.100b's left limbs sit at negative X
/// and its toes point along +Z. The rigs are the same handedness, turned
/// around — so one yaw takes the source rest pose onto the target's, and every
/// transferred rotation has to be conjugated by it or the port comes out with
/// its arms swapped.
///
/// The yaw is measured from the rest poses rather than assumed, because a
/// hard-coded half turn would silently do the wrong thing for a bank built on
/// a different rig. `residual` is how far the aligned rest poses still
/// disagree, in metres per bone, and is what makes this checkable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    pub yaw: f32,
    pub residual: f32,
}

impl Alignment {
    pub fn basis(&self) -> Basis {
        let (sin, cos) = (self.yaw.sin(), self.yaw.cos());
        [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]]
    }
}

/// Solve for the yaw that best takes the source rest pose onto the target's.
///
/// Positions are taken relative to each rig's own hip so the two heights do
/// not enter, and only bones the map pairs are used.
pub fn align(source: &skeleton::Skeleton, rig: &Rig) -> Result<Alignment> {
    let source_rest = source.model_rest();
    let source_hip = source
        .index("bip01_pelvis")
        .map(|index| source_rest[index].translation())
        .unwrap_or([0.0; 3]);
    let target_hip = rig
        .index("Hip")
        .map(|index| rig.bones[index].rest_translation)
        .unwrap_or([0.0; 3]);

    let mut pairs = Vec::new();
    for (source_name, target_name) in BONE_MAP {
        let (Some(source_index), Some(target_index)) =
            (source.index(source_name), rig.index(target_name))
        else {
            continue;
        };
        let from = source_rest[source_index].translation();
        let to = rig.bones[target_index].rest_translation;
        pairs.push((
            [
                from[0] - source_hip[0],
                from[1] - source_hip[1],
                from[2] - source_hip[2],
            ],
            [to[0] - target_hip[0], to[1] - target_hip[1], to[2] - target_hip[2]],
        ));
    }
    if pairs.is_empty() {
        bail!("the bone map pairs no bones these two rigs share");
    }

    // Least squares over a rotation about Y alone.
    let mut cross = 0.0_f32;
    let mut dot = 0.0_f32;
    for (from, to) in &pairs {
        cross += from[0] * to[2] - from[2] * to[0];
        dot += from[0] * to[0] + from[2] * to[2];
    }
    let yaw = cross.atan2(dot);
    let alignment = Alignment { yaw, residual: 0.0 };
    let basis = alignment.basis();
    let mut residual = 0.0_f32;
    for (from, to) in &pairs {
        let turned = transform(*from, &basis);
        let error = ((turned[0] - to[0]).powi(2)
            + (turned[1] - to[1]).powi(2)
            + (turned[2] - to[2]).powi(2))
        .sqrt();
        residual = residual.max(error);
    }
    Ok(Alignment { yaw, residual })
}

/// Which child each bone points at when its rest orientation is matched.
///
/// A bone with several children — the chest carries a neck and two clavicles,
/// the hip carries a spine and two thighs — has no single direction, and
/// picking one arbitrarily swings the whole body off a sideways reference. The
/// chain is therefore declared: each entry is the child whose direction
/// defines its parent's aim.
///
/// The hip is deliberately absent. It is the root of every mapped chain and
/// its orientation is already fixed by the whole-rig alignment; turning it to
/// point at a thigh would rotate the entire character.
const REST_CHAIN: &[(&str, &str)] = &[
    ("Spin", "Spin_1"),
    ("Spin_1", "Chest"),
    ("Chest", "Neck"),
    ("Neck", "Head"),
    ("LeftClavicle", "LeftArm"),
    ("LeftArm", "LeftForeArm"),
    ("LeftForeArm", "LeftHand"),
    ("LeftHand", "LeftMiddle1"),
    ("LeftUpLeg", "LeftLeg"),
    ("LeftLeg", "LeftFoot"),
    ("LeftFoot", "LeftToe"),
    ("RightClavicle", "RightArm"),
    ("RightArm", "RightForeArm"),
    ("RightForeArm", "RightHand"),
    ("RightHand", "RightMiddle1"),
    ("RightUpLeg", "RightLeg"),
    ("RightLeg", "RightFoot"),
    ("RightFoot", "RightToe"),
];

/// Pose the target rig the way the source rig rests.
///
/// A rest-relative transfer assumes the two rest poses *are* the same pose.
/// These two are not: X-Ray rests a stalker with the arms out near shoulder
/// height, v0.100b rests its character with the arms hanging at about forty
/// degrees. Transferring deltas straight across therefore lands every ported
/// arm that much too low, and the error is invisible in a residual because
/// the curves still reproduce exactly the (wrong) pose they were given.
///
/// So the target rest is first swung onto the source's: walking the hierarchy
/// parents-first, each mapped bone is turned by the minimal rotation that
/// points it at its child the way the source points at the matching one, and
/// the subtree follows. Only orientation moves — bone lengths stay the
/// target's, which is why the ported hands land in the target's proportions
/// rather than a stalker's.
fn matched_rest(
    rig: &Rig,
    source: &skeleton::Skeleton,
    mapped: &[Option<usize>],
    alignment: &Alignment,
) -> Vec<Basis> {
    let to_target = alignment.basis();
    let source_rest = source.model_rest();
    let source_position: Vec<[f32; 3]> = source_rest
        .iter()
        .map(|frame| transform(frame.translation(), &to_target))
        .collect();

    let mut basis: Vec<Basis> = (0..rig.bones.len()).map(|index| rig.rest_basis(index)).collect();
    let mut position: Vec<[f32; 3]> = rig
        .bones
        .iter()
        .map(|bone| bone.rest_translation)
        .collect();

    for index in rig_order(rig) {
        let Some(source_index) = mapped[index] else {
            continue;
        };
        let Some((_, child_name)) = REST_CHAIN
            .iter()
            .find(|(parent, _)| *parent == rig.bones[index].name)
        else {
            continue;
        };
        let Some(child) = rig.index(child_name) else {
            continue;
        };
        let Some(source_child) = mapped[child] else {
            continue;
        };

        let target_direction = direction(position[index], position[child]);
        let source_direction = direction(source_position[source_index], source_position[source_child]);
        let (Some(from), Some(to)) = (target_direction, source_direction) else {
            continue;
        };
        let turn = minimal_rotation(from, to);

        let pivot = position[index];
        for descendant in subtree(rig, index) {
            basis[descendant] = multiply(&basis[descendant], &turn);
            let offset = [
                position[descendant][0] - pivot[0],
                position[descendant][1] - pivot[1],
                position[descendant][2] - pivot[2],
            ];
            let turned = transform(offset, &turn);
            position[descendant] = [
                pivot[0] + turned[0],
                pivot[1] + turned[1],
                pivot[2] + turned[2],
            ];
        }
    }
    basis
}

/// A bone and everything below it, parents first.
fn subtree(rig: &Rig, root: usize) -> Vec<usize> {
    let mut inside = vec![false; rig.bones.len()];
    inside[root] = true;
    let mut members = vec![root];
    for index in rig_order(rig) {
        if inside[index] {
            continue;
        }
        if let Some(parent) = rig.bones[index].parent
            && inside[parent]
        {
            inside[index] = true;
            members.push(index);
        }
    }
    members
}

fn direction(from: [f32; 3], to: [f32; 3]) -> Option<[f32; 3]> {
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if length < 1e-5 {
        return None;
    }
    Some([delta[0] / length, delta[1] / length, delta[2] / length])
}

/// The shortest rotation taking `from` onto `to`, as a row-vector basis.
fn minimal_rotation(from: [f32; 3], to: [f32; 3]) -> Basis {
    let axis = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    let sin = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let cos = from[0] * to[0] + from[1] * to[1] + from[2] * to[2];
    if sin < 1e-6 {
        if cos > 0.0 {
            return IDENTITY_BASIS;
        }
        // Opposed: turn a half circle about any perpendicular axis.
        let perpendicular = if from[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let axis = [
            from[1] * perpendicular[2] - from[2] * perpendicular[1],
            from[2] * perpendicular[0] - from[0] * perpendicular[2],
            from[0] * perpendicular[1] - from[1] * perpendicular[0],
        ];
        return rodrigues(normalise(axis), std::f32::consts::PI);
    }
    rodrigues(
        [axis[0] / sin, axis[1] / sin, axis[2] / sin],
        sin.atan2(cos),
    )
}

fn normalise(vector: [f32; 3]) -> [f32; 3] {
    let length =
        (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

/// Rodrigues' rotation, transposed into the engine's row-vector convention.
fn rodrigues(axis: [f32; 3], angle: f32) -> Basis {
    let (sin, cos) = (angle.sin(), angle.cos());
    let t = 1.0 - cos;
    let (x, y, z) = (axis[0], axis[1], axis[2]);
    [
        [t * x * x + cos, t * x * y + sin * z, t * x * z - sin * y],
        [t * x * y - sin * z, t * y * y + cos, t * y * z + sin * x],
        [t * x * z + sin * y, t * y * z - sin * x, t * z * z + cos],
    ]
}

/// What a retarget did, for the build log and for fail-closed checks.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub motion: String,
    /// Sample count, including the wrap sample that closes the loop.
    pub samples: usize,
    /// Target bones a source bone drives.
    pub driven: usize,
    /// Target bones holding their rest frame.
    pub held: usize,
    /// The worst error between a fitted curve and the pose it was fitted to,
    /// in radians for rotation channels and metres for translation ones.
    pub max_residual: f32,
    /// How the rigs were found to be oriented relative to each other.
    pub alignment: Alignment,
}

/// Retarget one motion.
pub fn retarget(
    motion: &omf::Motion,
    bank: &omf::Bank,
    source: &skeleton::Skeleton,
    rig: &Rig,
) -> Result<(Clip, Report)> {
    if motion.frames == 0 {
        bail!("motion {} has no frames", motion.name);
    }

    // Source bone index -> track in the bank's key stream.
    let tracks: Vec<usize> = source
        .bones
        .iter()
        .map(|bone| {
            bank.bone_index(&bone.name)
                .with_context(|| format!("the bank has no track for model bone {}", bone.name))
        })
        .collect::<Result<_>>()?;

    let source_rest = model_space_rest(source);
    let target_order = clip_bone_order(rig);
    let mapped = mapping(source, rig)?;
    let alignment = align(source, rig)?;
    let to_target = alignment.basis();
    let to_source = transpose(&to_target);
    let rest = matched_rest(rig, source, &mapped, &alignment);

    // One extra sample closes the loop: X-Ray wraps frame N onto frame 0, so
    // the curve has to arrive back where it started.
    let samples = motion.frames as usize + 1;
    let mut translations = vec![vec![[0.0_f32; 3]; samples]; target_order.len()];
    let mut rotations = vec![vec![[0.0_f32; 3]; samples]; target_order.len()];

    for sample in 0..samples {
        let frame = (sample as u32) % motion.frames;
        let source_pose = model_space_pose(motion, source, &tracks, frame);

        // Target model-space orientation, parents before children.
        let mut target_pose = vec![IDENTITY_BASIS; rig.bones.len()];
        for index in rig_order(rig) {
            let parent = rig.bones[index].parent.map(|parent| target_pose[parent]);
            target_pose[index] = match mapped[index] {
                Some(source_index) => {
                    // The bone's change from its own rest, in source model
                    // axes, turned into target model axes.
                    let change = multiply(
                        &transpose(&source_rest[source_index]),
                        &source_pose[source_index],
                    );
                    let change = multiply(&multiply(&to_source, &change), &to_target);
                    multiply(&rest[index], &change)
                }
                None => {
                    let (_, local) = rig.local_rest(index);
                    let local = crate::skeleton_animation::euler_to_basis(local);
                    match parent {
                        Some(parent) => multiply(&local, &parent),
                        None => local,
                    }
                }
            };
        }

        for (slot, bone) in target_order.iter().enumerate() {
            let Some(index) = *bone else {
                // The root-motion channel is not part of the rig and stays put:
                // a ladder animation is played in place, and the ladder itself
                // moves the player.
                translations[slot][sample] = [0.0; 3];
                rotations[slot][sample] = [0.0; 3];
                continue;
            };
            let local = match rig.bones[index].parent {
                Some(parent) => multiply(&target_pose[index], &transpose(&target_pose[parent])),
                None => target_pose[index],
            };
            rotations[slot][sample] = basis_to_euler(&local);
            translations[slot][sample] = rig.local_rest(index).0;
        }

        // X-Ray keeps a motion's travel on the pelvis; carry the displacement
        // from its rest position over to the bone the pelvis maps to.
        if let (Some(pelvis), Some(hip_slot)) = (source.index("bip01_pelvis"), hip_slot(&target_order, rig)) {
            let track = &motion.tracks[tracks[pelvis]];
            let moved = track.translation(frame);
            let rest = source.bones[pelvis].rest_translation;
            let index = target_order[hip_slot].expect("the hip is a rig bone");
            let (rest_local, _) = rig.local_rest(index);
            let displacement = transform(
                [
                    moved[0] - rest[0],
                    moved[1] - rest[1],
                    moved[2] - rest[2],
                ],
                &to_target,
            );
            let displacement = match rig.bones[index].parent {
                Some(parent) => transform(displacement, &transpose(&rig.rest_basis(parent))),
                None => displacement,
            };
            translations[hip_slot][sample] = [
                rest_local[0] + displacement[0],
                rest_local[1] + displacement[1],
                rest_local[2] + displacement[2],
            ];
        }
    }

    let times: Vec<f32> = (0..samples).map(|sample| sample as f32).collect();
    let end = *times.last().expect("at least two samples");
    let mut clip_tracks = Vec::with_capacity(target_order.len());
    let mut max_residual = 0.0_f32;
    for slot in 0..target_order.len() {
        let mut channels: Vec<Channel> = Vec::with_capacity(CHANNELS);
        for axis in 0..3 {
            let values: Vec<f32> = translations[slot].iter().map(|value| value[axis]).collect();
            let channel = Channel::fit(&times, &values)?;
            max_residual = max_residual.max(channel.residual(&times, &values));
            channels.push(channel);
        }
        for axis in 0..3 {
            let values = unwrapped(&rotations[slot], axis);
            let channel = Channel::fit(&times, &values)?;
            max_residual = max_residual.max(channel.residual(&times, &values));
            channels.push(channel);
        }
        for _ in 0..3 {
            channels.push(Channel::constant(1.0, end));
        }
        clip_tracks.push(BoneTracks {
            channels: channels.try_into().expect("nine channels"),
        });
    }

    let driven = mapped.iter().filter(|bone| bone.is_some()).count();
    let report = Report {
        motion: motion.name.clone(),
        samples,
        driven,
        held: rig.bones.len() - driven,
        max_residual,
        alignment,
    };
    let clip = Clip {
        bones: target_order
            .iter()
            .map(|bone| match bone {
                Some(index) => rig.bones[*index].name.clone(),
                None => ROOT_MOTION_BONE.to_owned(),
            })
            .collect(),
        tracks: clip_tracks,
        animation_type: 0,
    };
    Ok((clip, report))
}

/// One bone's model-space position in both rigs at one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseSample {
    pub source_bone: String,
    pub target_bone: String,
    pub source: [f32; 3],
    pub target: [f32; 3],
}

/// Where the ported pose actually puts each mapped bone, next to where the
/// source pose puts it.
///
/// A residual only says the curves reproduce the poses that were asked for; it
/// says nothing about whether the *right* pose was asked for. Two rigs of
/// different proportions never agree exactly, but a limb pointing the wrong
/// way, an inverted body or a rotation applied in the wrong frame all show up
/// here as metres of disagreement.
pub fn compare_pose(
    clip: &Clip,
    rig: &Rig,
    motion: &omf::Motion,
    bank: &omf::Bank,
    source: &skeleton::Skeleton,
    frame: u32,
) -> Result<Vec<PoseSample>> {
    let tracks: Vec<usize> = source
        .bones
        .iter()
        .map(|bone| {
            bank.bone_index(&bone.name)
                .with_context(|| format!("the bank has no track for model bone {}", bone.name))
        })
        .collect::<Result<_>>()?;

    // Source model-space frames, rotation and position together.
    let mut source_frames = vec![(IDENTITY_BASIS, [0.0_f32; 3]); source.bones.len()];
    for index in source_order(source) {
        let track = &motion.tracks[tracks[index]];
        let local = quaternion_basis(track.rotation(frame));
        let offset = track.translation(frame);
        source_frames[index] = match source.bones[index].parent {
            Some(parent) => {
                let (parent_basis, parent_position) = source_frames[parent];
                let moved = transform(offset, &parent_basis);
                (
                    multiply(&local, &parent_basis),
                    [
                        parent_position[0] + moved[0],
                        parent_position[1] + moved[1],
                        parent_position[2] + moved[2],
                    ],
                )
            }
            None => (local, offset),
        };
    }

    // Target model-space frames, read back out of the emitted curves.
    let time = frame as f32;
    let mut target_frames = vec![(IDENTITY_BASIS, [0.0_f32; 3]); rig.bones.len()];
    for index in rig_order(rig) {
        let slot = clip
            .bones
            .iter()
            .position(|bone| bone == &rig.bones[index].name)
            .with_context(|| format!("the clip does not drive {}", rig.bones[index].name))?;
        let channels = &clip.tracks[slot].channels;
        let offset = [
            channels[0].evaluate(time),
            channels[1].evaluate(time),
            channels[2].evaluate(time),
        ];
        let local = crate::skeleton_animation::euler_to_basis([
            channels[3].evaluate(time),
            channels[4].evaluate(time),
            channels[5].evaluate(time),
        ]);
        target_frames[index] = match rig.bones[index].parent {
            Some(parent) => {
                let (parent_basis, parent_position) = target_frames[parent];
                let moved = transform(offset, &parent_basis);
                (
                    multiply(&local, &parent_basis),
                    [
                        parent_position[0] + moved[0],
                        parent_position[1] + moved[1],
                        parent_position[2] + moved[2],
                    ],
                )
            }
            None => (local, offset),
        };
    }

    let mut samples = Vec::new();
    for (source_name, target_name) in BONE_MAP {
        let (Some(source_index), Some(target_index)) =
            (source.index(source_name), rig.index(target_name))
        else {
            continue;
        };
        samples.push(PoseSample {
            source_bone: (*source_name).to_owned(),
            target_bone: (*target_name).to_owned(),
            source: source_frames[source_index].1,
            target: target_frames[target_index].1,
        });
    }
    Ok(samples)
}

/// Source bone indices, parents before children.
fn source_order(source: &skeleton::Skeleton) -> Vec<usize> {
    let mut order = Vec::with_capacity(source.bones.len());
    let mut placed = vec![false; source.bones.len()];
    while order.len() < source.bones.len() {
        let mut progressed = false;
        for index in 0..source.bones.len() {
            if placed[index] {
                continue;
            }
            let ready = match source.bones[index].parent {
                Some(parent) => placed[parent],
                None => true,
            };
            if ready {
                placed[index] = true;
                order.push(index);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    order
}

/// Euler angles are fitted as three independent curves, so a track that steps
/// across the +/-pi seam would be fitted as a full turn in one frame. Unwrap
/// each axis first: the curve is smooth, only its representation is not.
fn unwrapped(frames: &[[f32; 3]], axis: usize) -> Vec<f32> {
    let mut values = Vec::with_capacity(frames.len());
    let mut previous = 0.0_f32;
    for (index, frame) in frames.iter().enumerate() {
        let mut value = frame[axis];
        if index > 0 {
            let turns = ((value - previous) / std::f32::consts::TAU).round();
            value -= turns * std::f32::consts::TAU;
        }
        previous = value;
        values.push(value);
    }
    values
}

/// Bones in the order the clip declares them: the root-motion channel, then
/// the rig's own bones.
fn clip_bone_order(rig: &Rig) -> Vec<Option<usize>> {
    let mut order = vec![None];
    order.extend((0..rig.bones.len()).map(Some));
    order
}

fn hip_slot(order: &[Option<usize>], rig: &Rig) -> Option<usize> {
    let hip = rig.index("Hip")?;
    order.iter().position(|bone| *bone == Some(hip))
}

/// Rig bone indices, parents before children.
fn rig_order(rig: &Rig) -> Vec<usize> {
    let mut order = Vec::with_capacity(rig.bones.len());
    let mut placed = vec![false; rig.bones.len()];
    while order.len() < rig.bones.len() {
        let mut progressed = false;
        for index in 0..rig.bones.len() {
            if placed[index] {
                continue;
            }
            let ready = match rig.bones[index].parent {
                Some(parent) => placed[parent],
                None => true,
            };
            if ready {
                placed[index] = true;
                order.push(index);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    order
}

/// Target bone index -> source bone index, from the reviewed table.
fn mapping(source: &skeleton::Skeleton, rig: &Rig) -> Result<Vec<Option<usize>>> {
    let mut mapped = vec![None; rig.bones.len()];
    for (source_name, target_name) in BONE_MAP {
        let Some(target) = rig.index(target_name) else {
            bail!("the target rig has no bone {target_name} named by the bone map");
        };
        let Some(index) = source.index(source_name) else {
            // A bank may be built for a model that lacks a facial bone; that
            // is a smaller rig, not a broken map.
            continue;
        };
        mapped[target] = Some(index);
    }
    Ok(mapped)
}

/// Model-space rest orientation per source bone.
fn model_space_rest(source: &skeleton::Skeleton) -> Vec<Basis> {
    source
        .model_rest()
        .iter()
        .map(|frame| frame.basis())
        .collect()
}

/// Model-space orientation per source bone at one frame.
fn model_space_pose(
    motion: &omf::Motion,
    source: &skeleton::Skeleton,
    tracks: &[usize],
    frame: u32,
) -> Vec<Basis> {
    let mut pose = vec![IDENTITY_BASIS; source.bones.len()];
    let mut resolved = vec![false; source.bones.len()];
    let mut pending: Vec<usize> = (0..source.bones.len()).collect();
    while !pending.is_empty() {
        let mut progressed = false;
        pending.retain(|index| {
            let ready = match source.bones[*index].parent {
                Some(parent) => resolved[parent],
                None => true,
            };
            if !ready {
                return true;
            }
            let local = quaternion_basis(motion.tracks[tracks[*index]].rotation(frame));
            pose[*index] = match source.bones[*index].parent {
                Some(parent) => multiply(&local, &pose[parent]),
                None => local,
            };
            resolved[*index] = true;
            progressed = true;
            false
        });
        if !progressed {
            break;
        }
    }
    pose
}

/// `Fmatrix::mk_xform`: X-Ray's quaternion to a row-vector basis.
fn quaternion_basis(q: [f32; 4]) -> Basis {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton_animation::euler_to_basis;

    /// A quaternion that turns a quarter about Y must produce the same basis
    /// X-Ray's `mk_xform` does. Row-vector convention: the X row swings onto
    /// +Z and the Z row onto -X, which is the transpose of what a
    /// column-vector formula would give — get this backwards and every ported
    /// limb rotates the wrong way.
    #[test]
    fn decodes_a_quaternion_the_way_xray_does() {
        let half = std::f32::consts::FRAC_PI_4;
        let basis = quaternion_basis([0.0, half.sin(), 0.0, half.cos()]);
        assert!((basis[0][0]).abs() < 1e-6, "{basis:?}");
        assert!((basis[0][2] - 1.0).abs() < 1e-6, "{basis:?}");
        assert!((basis[2][0] + 1.0).abs() < 1e-6, "{basis:?}");
    }

    /// The Euler round trip has to be exact, or every bone drifts by whatever
    /// the conversion loses.
    #[test]
    fn euler_angles_survive_a_round_trip() {
        for angles in [
            [0.0_f32, 0.0, 0.0],
            [0.3, -0.7, 1.1],
            [-1.5, 0.2, -2.9],
            [std::f32::consts::FRAC_PI_2 - 0.01, 0.4, 0.9],
        ] {
            let basis = euler_to_basis(angles);
            let read_back = basis_to_euler(&basis);
            let rebuilt = euler_to_basis(read_back);
            for row in 0..3 {
                for column in 0..3 {
                    assert!(
                        (basis[row][column] - rebuilt[row][column]).abs() < 1e-5,
                        "{angles:?} -> {read_back:?}"
                    );
                }
            }
        }
    }

    /// A track that crosses the +/-pi seam must come back as a continuous
    /// curve, not as a full turn taken in one frame.
    #[test]
    fn unwraps_angles_across_the_seam() {
        let frames = vec![
            [3.10_f32, 0.0, 0.0],
            [3.14, 0.0, 0.0],
            [-3.13, 0.0, 0.0],
            [-3.09, 0.0, 0.0],
        ];
        let values = unwrapped(&frames, 0);
        for pair in values.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() < 0.2,
                "seam not unwrapped: {values:?}"
            );
        }
    }

    #[test]
    fn orders_parents_before_children() {
        let rig = Rig {
            bones: vec![
                crate::skeleton_animation::RigBone {
                    name: "Hand".to_owned(),
                    parent: Some(1),
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0; 3],
                },
                crate::skeleton_animation::RigBone {
                    name: "Arm".to_owned(),
                    parent: None,
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0; 3],
                },
            ],
        };
        assert_eq!(rig_order(&rig), vec![1, 0]);
    }
}
