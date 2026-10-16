// SPDX-License-Identifier: GPL-3.0-or-later
//! Hold chosen bones of retail skeleton clips at a pose another clip gives,
//! as a content pack of `replace_file` operations.
//!
//! The case it was written for: the Mosin's item clips `idle` and `shoot`
//! park the stripper clip (`bn_oboima`) and its cartridges 1.4 m under the
//! barrel, where the lobby's rifle-raising fidgets and the third-person view
//! show it floating. Every other Mosin clip stows it inside the buttstock,
//! and so do 0.69d's re-exported `idle` and `shoot`. Retail writes those
//! tracks as constants (four equal control points and the end knot), so the
//! fix is an in-place rewrite of their values: the clip keeps its size, its
//! bones and every other curve byte for byte. A track that is not such a
//! constant is refused rather than refitted.
//!
//! With a model to check against, every rewritten clip is posed across its
//! length and the moved bones' vertices must lie inside the box of one
//! named bone's vertices (the rifle's body), so a pose that hides nothing
//! fails the build.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue};
use crate::content_pack::{
    MANIFEST_FILE, Operation, PackManifest, TARGET_BUILD, patch::base_layers, sha256_hex,
    validate_resource_path,
};
use crate::skeleton_animation::{CHANNELS, Clip, Frame, Rig, transform, transpose};

/// `bi_spline_bone_animation_baked`: nine pointer slots per bone.
const BONE_RECORD_BYTES: usize = 0x48;
/// One event channel header in a v0.26g clip.
const EVENT_RECORD_BYTES: usize = 0x10;
/// Pairs of a retail constant curve: four control points and the end knot.
const CONSTANT_PAIRS: usize = 5;
/// Retail's exporter leaves its constants' control points an ulp or two
/// apart (-0.16485989 then -0.16485988 ×3): those still hold one value.
const CONSTANT_EPS: f32 = 1e-5;
/// A rigid weapon vertex: position, bone index, packed frame, uv.
const RIGID_VERTEX_BYTES: usize = 36;
/// Poses sampled across each clip for the bounds check.
const BOUNDS_SAMPLES: usize = 17;
/// How far (m) a moved vertex may stand outside the bounding bone's box.
pub const BOUNDS_TOLERANCE: f32 = 0.001;

/// A rigid vertex: bind position and bone index.
pub type RigidVertex = ([f32; 3], usize);

/// One bone's nine channel values: translation, Euler angles, scale.
pub type BonePose = [f32; CHANNELS];

/// `--inside-model` / `--inside-bone`.
#[derive(Clone, Debug)]
pub struct BoundsCheck {
    /// A `.skinned_model` folder, e.g. `models/weapons/mosin_nagant/mosin_nagant.skinned_model`.
    pub model: String,
    /// The bone whose vertices bound the moved ones (the weapon's body).
    pub inside_bone: String,
}

pub struct ClipPoseOptions<'a> {
    pub base_db: &'a Path,
    pub base_patches: &'a [PathBuf],
    /// Clip paths without extension (`…/item/stand/3rd_view/idle`).
    pub clips: &'a [String],
    pub bones: &'a [String],
    /// The clip whose pose at `time` the bones take.
    pub reference: &'a str,
    pub time: f32,
    pub bounds: Option<&'a BoundsCheck>,
    pub pack_dir: &'a Path,
    pub id: &'a str,
    pub version: &'a str,
    pub force: bool,
}

#[derive(Debug)]
pub struct ClipReport {
    pub path: String,
    pub bytes: usize,
    pub changed_bytes: usize,
    pub base_sha256: String,
    pub sha256: String,
    /// The farthest a moved vertex stands outside the bounding box, before
    /// and after, over the sampled poses (with a bounds check).
    pub outside_before: Option<f32>,
    pub outside_after: Option<f32>,
}

/// The pose `bones` take in `clip` at `time`.
pub fn reference_pose(
    clip: &Clip,
    bones: &[String],
    time: f32,
) -> Result<BTreeMap<String, BonePose>> {
    let mut pose = BTreeMap::new();
    for bone in bones {
        let track = clip
            .bones
            .iter()
            .position(|name| name == bone)
            .with_context(|| format!("the reference clip does not drive {bone}"))?;
        let channels = &clip.tracks[track].channels;
        pose.insert(
            bone.clone(),
            std::array::from_fn(|c| channels[c].evaluate(time)),
        );
    }
    Ok(pose)
}

/// Byte offset of each bone's nine `pairs` counts in a `.b-spline` image.
fn channel_offsets(bspline: &[u8], bones: usize) -> Result<Vec<[(usize, usize); CHANNELS]>> {
    let header = bspline
        .get(..4)
        .context("b-spline shorter than its header")?;
    let count = usize::from(u16::from_le_bytes([header[0], header[1]]));
    ensure!(
        count == bones,
        "b-spline drives {count} bones; bones_names lists {bones}"
    );
    let mut at = 4 + count * BONE_RECORD_BYTES + usize::from(header[2]) * EVENT_RECORD_BYTES;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = [(0, 0); CHANNELS];
        for slot in &mut row {
            let pairs = bspline.get(at..at + 4).context("truncated b-spline")?;
            let pairs = u32::from_le_bytes(pairs.try_into()?) as usize;
            ensure!(
                at + 4 + pairs * 8 <= bspline.len(),
                "truncated b-spline curve"
            );
            *slot = (at, pairs);
            at += 4 + pairs * 8;
        }
        out.push(row);
    }
    Ok(out)
}

/// `bspline` with each of `pose`'s bones held at its values for the whole
/// clip. Every channel of those bones must be a retail constant curve; its
/// four control points are overwritten, its knots and the unused end point
/// stay. Nothing else changes, so the size does not either.
pub fn hold_pose(
    bspline: &[u8],
    bones_names: &[u8],
    pose: &BTreeMap<String, BonePose>,
) -> Result<Vec<u8>> {
    let clip = Clip::parse(bspline, bones_names)?;
    let offsets = channel_offsets(bspline, clip.bones.len())?;
    let mut out = bspline.to_vec();
    for (bone, values) in pose {
        let index = clip
            .bones
            .iter()
            .position(|name| name == bone)
            .with_context(|| format!("the clip does not drive {bone}"))?;
        for (channel, &(at, pairs)) in offsets[index].iter().enumerate() {
            let curve = &clip.tracks[index].channels[channel];
            let first = curve.points[0];
            let held = curve.knots[..pairs.min(4)]
                .iter()
                .all(|k| *k == curve.knots[0])
                && curve.points[..pairs.min(4)]
                    .iter()
                    .all(|p| (p - first).abs() <= CONSTANT_EPS * first.abs().max(1.0));
            if pairs != CONSTANT_PAIRS || !held {
                bail!(
                    "{bone} channel {channel} is not a constant curve ({pairs} pairs); refusing to refit it"
                );
            }
            for point in 0..4 {
                let slot = at + 4 + point * 8 + 4;
                out[slot..slot + 4].copy_from_slice(&values[channel].to_le_bytes());
            }
        }
    }
    debug_assert_eq!(out.len(), bspline.len());
    Ok(out)
}

/// A rigid skinned surface: each vertex's bind position and bone index.
pub fn rigid_vertices(converted_model: &[u8]) -> Result<Vec<RigidVertex>> {
    let read = |at: usize| -> Result<u32> {
        let bytes = converted_model
            .get(at..at + 4)
            .context("truncated converted model")?;
        Ok(u32::from_le_bytes(bytes.try_into()?))
    };
    ensure!(read(0)? == 3, "converted model has no vertex chunk first");
    let count = read(8)? as usize;
    ensure!(count > 0, "converted model has no vertices");
    let stride = (read(4)? as usize - 4) / count;
    ensure!(
        stride == RIGID_VERTEX_BYTES && read(4)? as usize == 4 + count * stride,
        "converted model vertices are {stride} bytes; only rigid {RIGID_VERTEX_BYTES}-byte weapon vertices are checked"
    );
    (0..count)
        .map(|i| {
            let at = 12 + i * stride;
            let f = |k: usize| -> Result<f32> { Ok(f32::from_bits(read(at + k * 4)?)) };
            Ok(([f(0)?, f(1)?, f(2)?], read(at + 12)? as usize))
        })
        .collect()
}

fn skin(vertex: [f32; 3], rest: &Frame, posed: &Frame) -> [f32; 3] {
    let local = transform(
        [0, 1, 2].map(|k| vertex[k] - rest.1[k]),
        &transpose(&rest.0),
    );
    let world = transform(local, &posed.0);
    [0, 1, 2].map(|k| world[k] + posed.1[k])
}

/// The farthest any vertex of `moved` stands outside the box of `inside`'s
/// vertices, over poses sampled across `clip`.
pub fn farthest_outside(
    rig: &Rig,
    vertices: &[RigidVertex],
    clip: &Clip,
    moved: &[String],
    inside: &str,
) -> Result<f32> {
    let inside = rig
        .index(inside)
        .with_context(|| format!("the model has no bone {inside}"))?;
    let moved: Vec<usize> = moved
        .iter()
        .map(|bone| {
            rig.index(bone)
                .with_context(|| format!("the model has no bone {bone}"))
        })
        .collect::<Result<_>>()?;
    ensure!(
        vertices.iter().all(|(_, bone)| *bone < rig.bones.len()),
        "a vertex names a bone the bind pose lacks"
    );
    ensure!(
        vertices.iter().any(|(_, bone)| *bone == inside),
        "no vertex follows the bounding bone"
    );
    let rest = rig.rest();
    let end = clip.duration();
    let mut worst = 0.0_f32;
    for sample in 0..BOUNDS_SAMPLES {
        let pose = rig.pose(clip, end * sample as f32 / (BOUNDS_SAMPLES - 1) as f32);
        let posed = |(v, bone): &RigidVertex| skin(*v, &rest[*bone], &pose[*bone]);
        let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in vertices.iter().filter(|(_, b)| *b == inside).map(posed) {
            for k in 0..3 {
                low[k] = low[k].min(p[k]);
                high[k] = high[k].max(p[k]);
            }
        }
        for p in vertices
            .iter()
            .filter(|(_, b)| moved.contains(b))
            .map(posed)
        {
            let gap = (0..3)
                .map(|k| (low[k] - p[k]).max(p[k] - high[k]).max(0.0))
                .map(|d| d * d)
                .sum::<f32>()
                .sqrt();
            worst = worst.max(gap);
        }
    }
    Ok(worst)
}

/// The rig and every LOD0 surface's vertices of a rigid skinned model.
fn load_model(db: &mut ResourceDb, model: &str) -> Result<(Rig, Vec<RigidVertex>)> {
    let skeleton_ref = BinaryConfig::parse(&db.read(&format!("{model}/skeleton"))?)?;
    let Some(ConfigValue::String(skeleton)) = skeleton_ref.root.named("skeleton") else {
        bail!("{model}/skeleton names no skeleton");
    };
    let skeleton = format!(
        "{}.skeleton",
        skeleton.strip_prefix("resources/").unwrap_or(skeleton)
    );
    let rig = Rig::parse(
        &db.read(&format!("{model}/render/bind_pose"))?,
        &db.read(&skeleton)?,
    )
    .with_context(|| format!("{model} with {skeleton}"))?;
    let settings = BinaryConfig::parse(&db.read(&format!("{model}/settings"))?)?;
    let surfaces = match settings
        .root
        .named("lod_hierrarchy")
        .and_then(|lods| lods.named("LOD0"))
        .and_then(|lod| lod.named("surfaces"))
    {
        Some(ConfigValue::Indexed(surfaces)) => surfaces.clone(),
        _ => bail!("{model}/settings lists no LOD0 surfaces"),
    };
    let mut vertices = Vec::new();
    for surface in surfaces {
        let ConfigValue::String(surface) = surface else {
            bail!("{model}: a surface name is not a string");
        };
        let path = format!("{model}/render/{surface}/converted_model");
        vertices.extend(rigid_vertices(&db.read(&path)?).with_context(|| path.clone())?);
    }
    Ok((rig, vertices))
}

pub fn build(options: &ClipPoseOptions<'_>) -> Result<Vec<ClipReport>> {
    ensure!(!options.clips.is_empty(), "no --clip given");
    ensure!(!options.bones.is_empty(), "no --bone given");
    for clip in options
        .clips
        .iter()
        .map(String::as_str)
        .chain([options.reference])
    {
        validate_resource_path(&format!("{clip}.b-spline"))?;
    }
    if options.pack_dir.exists() {
        ensure!(
            options.force,
            "{} exists; pass --force to replace it",
            options.pack_dir.display()
        );
        fs::remove_dir_all(options.pack_dir)?;
    }
    let mut db = ResourceDb::open_layers(&base_layers(options.base_db, options.base_patches)?)?;
    let read_clip = |db: &mut ResourceDb, path: &str| -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            db.read(&format!("{path}.b-spline"))?,
            db.read(&format!("{path}.bones_names"))?,
        ))
    };
    let (bspline, names) = read_clip(&mut db, options.reference)?;
    let reference = Clip::parse(&bspline, &names).context(options.reference.to_owned())?;
    let pose = reference_pose(&reference, options.bones, options.time)?;
    let model = match options.bounds {
        Some(bounds) => Some(load_model(&mut db, &bounds.model)?),
        None => None,
    };

    let mut reports = Vec::new();
    let mut operations = Vec::new();
    for path in options.clips {
        let (before, names) = read_clip(&mut db, path)?;
        let after = hold_pose(&before, &names, &pose).with_context(|| path.clone())?;
        ensure!(after.len() == before.len(), "{path}: the size changed");
        let (mut outside_before, mut outside_after) = (None, None);
        if let (Some((rig, vertices)), Some(bounds)) = (&model, options.bounds) {
            let inside = &bounds.inside_bone;
            outside_before = Some(farthest_outside(
                rig,
                vertices,
                &Clip::parse(&before, &names)?,
                options.bones,
                inside,
            )?);
            let gap = farthest_outside(
                rig,
                vertices,
                &Clip::parse(&after, &names)?,
                options.bones,
                inside,
            )?;
            ensure!(
                gap <= BOUNDS_TOLERANCE,
                "{path}: the held bones stand {gap:.3} m outside {inside}'s box"
            );
            outside_after = Some(gap);
        }
        let resource = format!("{path}.b-spline");
        let file = safe_join(&options.pack_dir.join("files"), &resource)?;
        fs::create_dir_all(file.parent().context("pack file has no folder")?)?;
        fs::write(&file, &after)?;
        operations.push(Operation::ReplaceFile {
            path: resource.clone(),
            source: None,
            base_sha256: sha256_hex(&before),
        });
        reports.push(ClipReport {
            path: resource,
            bytes: after.len(),
            changed_bytes: before.iter().zip(&after).filter(|(a, b)| a != b).count(),
            base_sha256: sha256_hex(&before),
            sha256: sha256_hex(&after),
            outside_before,
            outside_after,
        });
    }

    let mut notes = vec![format!(
        "Bones {} held at {} t={} in {}.",
        options.bones.join(", "),
        options.reference,
        options.time,
        options.clips.join(", ")
    )];
    if let Some(bounds) = options.bounds {
        notes.push(format!(
            "Checked over {BOUNDS_SAMPLES} poses per clip: the held bones stay inside the box of {}'s vertices of {}.",
            bounds.inside_bone, bounds.model
        ));
    }
    let manifest = PackManifest {
        format_version: 1,
        id: options.id.to_owned(),
        version: options.version.to_owned(),
        target_build: TARGET_BUILD.to_owned(),
        description: Some("Held bone tracks of retail clips (build-clip-pose-patch)".to_owned()),
        requires: Vec::new(),
        base_db_sha256: None,
        operations,
        server: None,
        notes,
    };
    fs::write(
        options.pack_dir.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(reports)
}

#[cfg(test)]
mod tests;
