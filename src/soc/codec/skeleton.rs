// SPDX-License-Identifier: GPL-3.0-or-later
//! The skeleton an X-Ray 1.x actor model carries.
//!
//! A motion bank names its bones but says nothing about how they are joined or
//! where they rest, and a retarget needs both: the pose a motion is authored
//! against is the pose the model was bound in. Two chunks of the actor's
//! `.ogf` supply it —
//!
//! * `OGF_S_BONE_NAMES` — each bone's name and its parent's name, in the order
//!   the model indexes them;
//! * `OGF_S_IKDATA` — per bone, a pile of physics properties this port ignores
//!   and, at the end of each record, the `bind_transform`: a local rest
//!   rotation as X-Ray Euler angles plus a local rest offset.
//!
//! "Local" is relative to the parent bone, which is what an animation key
//! replaces at runtime. The rest pose is therefore directly comparable with a
//! motion's frame, and the model-space rest pose is the accumulated product
//! down the hierarchy.
//!
//! Layout is `Layers/xrRender/SkeletonCustom.cpp CKinematics::Load`, the Euler
//! convention `xrCore/_matrix.h setXYZi` composed by `setHPB`.

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};

/// `OGF_S_BONE_NAMES` from `xrCore/FMesh.hpp`.
pub const OGF_S_BONE_NAMES: u32 = 13;
/// `OGF_S_IKDATA` from `xrCore/FMesh.hpp`.
pub const OGF_S_IKDATA: u32 = 16;

/// `Fobb`: a 3x3 basis, a centre, and half-extents.
const OBB_BYTES: usize = 15 * 4;
/// `SBoneShape`: type, flags, an obb, a sphere, and a cylinder.
const BONE_SHAPE_BYTES: usize = 2 + 2 + OBB_BYTES + 4 * 4 + 8 * 4;
/// `SJointIKData` before `friction`: type, three limits, and five floats.
/// Versions above zero append `friction`, so the record is four bytes longer.
const JOINT_IK_BYTES: usize = 4 + 3 * 16 + 5 * 4;

/// One bone of an actor skeleton.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index into [`Skeleton::bones`], or `None` for the root.
    pub parent: Option<usize>,
    /// Rest rotation relative to the parent, as X-Ray XYZ Euler angles.
    pub rest_rotation: [f32; 3],
    /// Rest offset relative to the parent.
    pub rest_translation: [f32; 3],
}

/// An actor skeleton: bones in model index order, each naming its parent.
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .position(|bone| bone.name.eq_ignore_ascii_case(name))
    }

    /// Each bone's rest transform relative to the parent.
    pub fn local_rest(&self, index: usize) -> Matrix {
        let bone = &self.bones[index];
        Matrix::from_xray_frame(bone.rest_rotation, bone.rest_translation)
    }

    /// Each bone's rest transform in model space, parents applied first.
    ///
    /// Bones are stored parents-first by the exporter, but this does not rely
    /// on that: it resolves each chain on demand.
    pub fn model_rest(&self) -> Vec<Matrix> {
        let mut resolved: Vec<Option<Matrix>> = vec![None; self.bones.len()];
        for index in 0..self.bones.len() {
            self.resolve_model_rest(index, &mut resolved);
        }
        resolved
            .into_iter()
            .map(|frame| frame.expect("every bone resolved"))
            .collect()
    }

    fn resolve_model_rest(&self, index: usize, resolved: &mut Vec<Option<Matrix>>) -> Matrix {
        if let Some(frame) = resolved[index] {
            return frame;
        }
        let local = self.local_rest(index);
        let frame = match self.bones[index].parent {
            Some(parent) => self.resolve_model_rest(parent, resolved).multiply(local),
            None => local,
        };
        resolved[index] = Some(frame);
        frame
    }
}

/// A 4x4 affine transform in X-Ray's row-vector convention: a row vector is
/// multiplied on the left, so `child.multiply(parent)` is *not* the same as
/// `parent.multiply(child)` and composition runs parent-first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    /// Row-major: `rows[3]` is the translation.
    pub rows: [[f32; 3]; 4],
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        rows: [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
        ],
    };

    /// `Fmatrix::setXYZi(angles)` followed by `translate_over(offset)`.
    pub fn from_xray_frame(angles: [f32; 3], offset: [f32; 3]) -> Self {
        let mut frame = Self::set_hpb(-angles[1], -angles[0], -angles[2]);
        frame.rows[3] = offset;
        frame
    }

    /// `Fmatrix::setHPB`.
    pub fn set_hpb(h: f32, p: f32, b: f32) -> Self {
        let (sh, ch) = (h.sin(), h.cos());
        let (sp, cp) = (p.sin(), p.cos());
        let (sb, cb) = (b.sin(), b.cos());
        let cc = ch * cb;
        let cs = ch * sb;
        let sc = sh * cb;
        let ss = sh * sb;
        Self {
            rows: [
                [cc - sp * ss, -cp * sb, sp * cs + sc],
                [sp * sc + cs, cp * cb, ss - sp * cc],
                [-cp * sh, sp, cp * ch],
                [0.0, 0.0, 0.0],
            ],
        }
    }

    /// `self` applied after `rhs`: the product a child bone forms with its
    /// parent when `self` is the parent.
    pub fn multiply(&self, rhs: Matrix) -> Matrix {
        let mut rows = [[0.0_f32; 3]; 4];
        for (row, out) in rhs.rows.iter().zip(rows.iter_mut()) {
            for axis in 0..3 {
                out[axis] = row[0] * self.rows[0][axis]
                    + row[1] * self.rows[1][axis]
                    + row[2] * self.rows[2][axis];
            }
        }
        for axis in 0..3 {
            rows[3][axis] += self.rows[3][axis];
        }
        Matrix { rows }
    }

    /// The rotation part only.
    pub fn basis(&self) -> [[f32; 3]; 3] {
        [self.rows[0], self.rows[1], self.rows[2]]
    }

    pub fn translation(&self) -> [f32; 3] {
        self.rows[3]
    }
}

/// Decode the skeleton of an actor model.
pub fn parse(bytes: &[u8], label: &str) -> Result<Skeleton> {
    let chunks = Chunks::new(bytes, label);
    let names = chunks
        .require(OGF_S_BONE_NAMES)
        .with_context(|| format!("{label}: model carries no OGF_S_BONE_NAMES"))?;
    let mut cursor = Cursor::new(names, label);
    let count = usize::try_from(cursor.u32()?)?;
    let mut named = Vec::with_capacity(count);
    for index in 0..count {
        let name = cursor.stringz()?.to_ascii_lowercase();
        let parent = cursor.stringz()?.to_ascii_lowercase();
        cursor
            .take(OBB_BYTES)
            .with_context(|| format!("{label}: bone {index} has a truncated obb"))?;
        named.push((name, parent));
    }
    if !cursor.is_empty() {
        bail!(
            "{label}: {} trailing bytes after {count} bone names",
            cursor.remaining()
        );
    }

    let ik = chunks
        .require(OGF_S_IKDATA)
        .with_context(|| format!("{label}: model carries no OGF_S_IKDATA"))?;
    let mut cursor = Cursor::new(ik, label);
    let mut bones = Vec::with_capacity(count);
    for (index, (name, parent)) in named.iter().enumerate() {
        let version = cursor.u32()?;
        let _game_material = cursor.stringz()?;
        cursor
            .take(BONE_SHAPE_BYTES)
            .with_context(|| format!("{label}: bone {index} has a truncated shape"))?;
        let joint = if version > 0 {
            JOINT_IK_BYTES + 4
        } else {
            JOINT_IK_BYTES
        };
        cursor
            .take(joint)
            .with_context(|| format!("{label}: bone {index} has truncated joint data"))?;
        let rest_rotation = cursor.float3()?;
        let rest_translation = cursor.float3()?;
        let _mass = cursor.f32()?;
        let _center_of_mass = cursor.float3()?;

        let parent = if parent.is_empty() {
            None
        } else {
            Some(
                named
                    .iter()
                    .position(|(candidate, _)| candidate == parent)
                    .with_context(|| {
                        format!("{label}: bone {name} names a parent {parent} the model lacks")
                    })?,
            )
        };
        if parent == Some(index) {
            bail!("{label}: bone {name} is its own parent");
        }
        bones.push(Bone {
            name: name.clone(),
            parent,
            rest_rotation,
            rest_translation,
        });
    }
    if !cursor.is_empty() {
        bail!(
            "{label}: {} trailing bytes after {count} bone frames",
            cursor.remaining()
        );
    }
    if bones.iter().filter(|bone| bone.parent.is_none()).count() != 1 {
        bail!("{label}: skeleton does not have exactly one root bone");
    }

    Ok(Skeleton { bones })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn model(bones: &[(&str, &str, [f32; 3], [f32; 3])]) -> Vec<u8> {
        let mut names = (bones.len() as u32).to_le_bytes().to_vec();
        for (name, parent, _, _) in bones {
            names.extend_from_slice(name.as_bytes());
            names.push(0);
            names.extend_from_slice(parent.as_bytes());
            names.push(0);
            names.extend(std::iter::repeat_n(0u8, OBB_BYTES));
        }

        let mut ik = Vec::new();
        for (_, _, rotation, translation) in bones {
            ik.extend_from_slice(&1u32.to_le_bytes()); // version
            ik.push(0); // game material name
            ik.extend(std::iter::repeat_n(0u8, BONE_SHAPE_BYTES));
            ik.extend(std::iter::repeat_n(0u8, JOINT_IK_BYTES + 4));
            for value in rotation.iter().chain(translation.iter()) {
                ik.extend_from_slice(&value.to_le_bytes());
            }
            ik.extend_from_slice(&1.0f32.to_le_bytes()); // mass
            ik.extend(std::iter::repeat_n(0u8, 12)); // centre of mass
        }

        let mut bytes = chunk(OGF_S_BONE_NAMES, &names);
        bytes.extend(chunk(OGF_S_IKDATA, &ik));
        bytes
    }

    #[test]
    fn resolves_parents_and_rest_frames() {
        let bytes = model(&[
            ("bip01_pelvis", "", [0.0, 0.0, 0.0], [0.0, 0.9, 0.0]),
            (
                "bip01_spine",
                "bip01_pelvis",
                [0.0, 0.0, 0.0],
                [0.0, 0.1, 0.0],
            ),
        ]);
        let skeleton = parse(&bytes, "stalker.ogf").unwrap();
        assert_eq!(skeleton.bones[0].parent, None);
        assert_eq!(skeleton.bones[1].parent, Some(0));

        // The spine rests one tenth above the pelvis, which rests at 0.9.
        let model_rest = skeleton.model_rest();
        let spine = model_rest[1].translation();
        assert!((spine[1] - 1.0).abs() < 1e-6, "{spine:?}");
    }

    /// Names are matched case-insensitively; the exporter lowercases them.
    #[test]
    fn lowercases_bone_names() {
        let bytes = model(&[("BIP01_Pelvis", "", [0.0; 3], [0.0; 3])]);
        let skeleton = parse(&bytes, "stalker.ogf").unwrap();
        assert_eq!(skeleton.bones[0].name, "bip01_pelvis");
        assert_eq!(skeleton.index("bip01_PELVIS"), Some(0));
    }

    #[test]
    fn rejects_a_missing_parent() {
        let bytes = model(&[("bip01_spine", "bip01_pelvis", [0.0; 3], [0.0; 3])]);
        let error = parse(&bytes, "stalker.ogf").unwrap_err().to_string();
        assert!(error.contains("names a parent"), "{error}");
    }

    /// A rotation of 90 degrees about X must reproduce X-Ray's own basis.
    ///
    /// `setXYZi(x, 0, 0)` is `setHPB(0, -x, 0)`, and the sign of that negation
    /// is the whole reason this is pinned: it decides which way every ported
    /// limb bends. At `x = +90` degrees the Y row swings onto `+Z` and the Z
    /// row onto `-Y`.
    #[test]
    fn matches_the_xray_euler_convention() {
        let frame =
            Matrix::from_xray_frame([std::f32::consts::FRAC_PI_2, 0.0, 0.0], [0.0, 0.0, 0.0]);
        let basis = frame.basis();
        assert!((basis[0][0] - 1.0).abs() < 1e-6, "{basis:?}");
        assert!((basis[1][2] - 1.0).abs() < 1e-6, "{basis:?}");
        assert!((basis[2][1] + 1.0).abs() < 1e-6, "{basis:?}");
    }
}
