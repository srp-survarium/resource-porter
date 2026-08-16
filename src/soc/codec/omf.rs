//! X-Ray 1.x motion banks (`.omf`).
//!
//! A bank is a chunked container holding two chunks that matter:
//!
//! * `OGF_S_SMPARAMS` — the *definitions*: which bones the bank animates, how
//!   they are split into blend partitions (`torso`, `legs`, …), and one record
//!   per motion carrying its name and playback parameters;
//! * `OGF_S_MOTIONS` — the *keys*, one sub-chunk per motion, sampled at a fixed
//!   30 frames per second.
//!
//! The bank names its own bones, so nothing here needs the actor `.ogf` that
//! plays it. The engine reads the partition entries as
//! `motion bone index -> object bone id`; this decoder keeps the same index and
//! resolves it to the bone *name*, which is what a port maps by.
//!
//! Keys are quantised. Rotations are four `s16` scaled by `1/32767` and
//! translations are `s8` or `s16` scaled by a per-track `size`/`init` pair, so
//! a track that never moves stores one key rather than one per frame. Both are
//! expanded here: a caller asking for frame *n* gets a float, and does not have
//! to know which of the four storage shapes the exporter chose.
//!
//! Layout is `xrCore/Animation/SkeletonMotions.cpp motions_value::load` and
//! `Layers/xrRender/AnimationKeyCalculate.h`.

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};

/// `OGF_S_MOTIONS` from `xrCore/FMesh.hpp`.
pub const OGF_S_MOTIONS: u32 = 14;
/// `OGF_S_SMPARAMS` from `xrCore/FMesh.hpp`.
pub const OGF_S_SMPARAMS: u32 = 15;

/// `xrOGF_SMParamsVersion`. Later versions append per-motion marks.
pub const MAX_SMPARAMS_VERSION: u16 = 4;

/// `SAMPLE_FPS`. Every bank is sampled at this rate; nothing stores its own.
pub const SAMPLE_FPS: f32 = 30.0;

/// `KEY_QuantI`, the rotation key scale.
const KEY_QUANT_I: f32 = 1.0 / 32767.0;

/// `flTKeyPresent`: the track stores one translation key per frame.
const FL_T_KEY_PRESENT: u8 = 1 << 0;
/// `flRKeyAbsent`: the track stores a single, constant rotation.
const FL_R_KEY_ABSENT: u8 = 1 << 1;
/// `flTKey16IsBit`: translation keys are `s16` triples rather than `s8`.
const FL_T_KEY_16: u8 = 1 << 2;

/// One decoded motion bank.
#[derive(Clone, Debug)]
pub struct Bank {
    pub version: u16,
    /// Bone names indexed by motion bone index — the order the key stream uses.
    pub bones: Vec<String>,
    /// Blend partitions, in declaration order.
    pub partitions: Vec<Partition>,
    /// Motions in bank order; `motions[i]` owns the keys of sub-chunk `i + 1`.
    pub motions: Vec<Motion>,
}

/// One blend partition: the bones an animation layer is allowed to drive.
#[derive(Clone, Debug)]
pub struct Partition {
    pub name: String,
    pub bones: Vec<String>,
}

/// One motion: a named, fixed-rate sampling of every bone in the bank.
#[derive(Clone, Debug)]
pub struct Motion {
    pub name: String,
    pub flags: u32,
    /// Sample count. Duration is `frames / SAMPLE_FPS` seconds.
    pub frames: u32,
    pub speed: f32,
    pub power: f32,
    pub accrue: f32,
    pub falloff: f32,
    /// One track per bone, indexed like [`Bank::bones`].
    pub tracks: Vec<Track>,
}

/// One bone's motion within one motion.
///
/// Each channel holds either a single constant value or one value per frame;
/// [`Track::rotation`] and [`Track::translation`] hide the difference.
#[derive(Clone, Debug)]
pub struct Track {
    /// Quaternions as `(x, y, z, w)`, exactly as the bank quantised them.
    pub rotations: Vec<[f32; 4]>,
    pub translations: Vec<[f32; 3]>,
}

impl Track {
    /// The rotation at `frame`, wrapping like the engine's `frame % count`.
    pub fn rotation(&self, frame: u32) -> [f32; 4] {
        self.rotations[frame as usize % self.rotations.len()]
    }

    /// The translation at `frame`, wrapping like the engine's `frame % count`.
    pub fn translation(&self, frame: u32) -> [f32; 3] {
        self.translations[frame as usize % self.translations.len()]
    }

    /// Whether the bone holds one pose for the whole motion.
    pub fn is_static(&self) -> bool {
        self.rotations.len() == 1 && self.translations.len() == 1
    }
}

impl Bank {
    /// The motion with this name, matched case-insensitively as the engine does.
    pub fn motion(&self, name: &str) -> Option<&Motion> {
        self.motions
            .iter()
            .find(|motion| motion.name.eq_ignore_ascii_case(name))
    }

    /// The index of a bone in the key stream.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .position(|bone| bone.eq_ignore_ascii_case(name))
    }
}

impl Motion {
    /// Duration in seconds at the bank's fixed sample rate.
    pub fn duration(&self) -> f32 {
        self.frames as f32 / SAMPLE_FPS
    }
}

/// Decode a motion bank.
pub fn parse(bytes: &[u8], label: &str) -> Result<Bank> {
    let chunks = Chunks::new(bytes, label);
    let params = chunks
        .require(OGF_S_SMPARAMS)
        .with_context(|| format!("{label}: motion bank has no OGF_S_SMPARAMS"))?;
    let (version, bones, partitions, mut motions) = parse_params(params, label)?;

    let motion_chunks = Chunks::new(
        chunks
            .require(OGF_S_MOTIONS)
            .with_context(|| format!("{label}: motion bank has no OGF_S_MOTIONS"))?,
        label,
    );

    let count_payload = motion_chunks
        .require(0)
        .with_context(|| format!("{label}: motion chunk has no count"))?;
    let declared = Cursor::new(count_payload, label).u32()?;
    if declared as usize != motions.len() {
        bail!(
            "{label}: {} motion definitions but {declared} motion key chunks",
            motions.len()
        );
    }

    for (index, motion) in motions.iter_mut().enumerate() {
        let id = u32::try_from(index + 1)?;
        let payload = motion_chunks
            .require(id)
            .with_context(|| format!("{label}: motion {} has no key chunk", motion.name))?;
        let mut cursor = Cursor::new(payload, label);
        let name = cursor.stringz()?;
        if !name.eq_ignore_ascii_case(&motion.name) {
            bail!(
                "{label}: motion {index} is named {:?} in the definitions and {name:?} in the keys",
                motion.name
            );
        }
        motion.frames = cursor.u32()?;
        motion.tracks = parse_tracks(&mut cursor, bones.len(), motion.frames, label, &name)?;
        if !cursor.is_empty() {
            bail!(
                "{label}: motion {name} leaves {} bytes after its last bone",
                cursor.remaining()
            );
        }
    }

    Ok(Bank {
        version,
        bones,
        partitions,
        motions,
    })
}

/// Decode `OGF_S_SMPARAMS`: partitions first, then one record per motion.
fn parse_params(
    payload: &[u8],
    label: &str,
) -> Result<(u16, Vec<String>, Vec<Partition>, Vec<Motion>)> {
    let mut cursor = Cursor::new(payload, label);
    let version = cursor.u16()?;
    if version > MAX_SMPARAMS_VERSION {
        bail!("{label}: motion bank declares unsupported SMPARAMS version {version}");
    }

    let partition_count = cursor.u16()?;
    let mut partitions = Vec::with_capacity(partition_count as usize);
    // motion bone index -> name. The engine sizes this from the object it is
    // played on; here it grows to whatever the partitions actually reference.
    let mut bones: Vec<Option<String>> = Vec::new();
    for _ in 0..partition_count {
        let name = cursor.stringz()?;
        let bone_count = cursor.u16()?;
        let mut partition_bones = Vec::with_capacity(bone_count as usize);
        for _ in 0..bone_count {
            let bone = cursor.stringz()?;
            let index = usize::try_from(cursor.u32()?)?;
            if bones.len() <= index {
                bones.resize(index + 1, None);
            }
            if let Some(existing) = &bones[index] {
                bail!("{label}: motion bone {index} is both {existing:?} and {bone:?}");
            }
            bones[index] = Some(bone.clone());
            partition_bones.push(bone);
        }
        partitions.push(Partition {
            name,
            bones: partition_bones,
        });
    }

    let bones = bones
        .into_iter()
        .enumerate()
        .map(|(index, bone)| {
            bone.with_context(|| format!("{label}: no partition claims motion bone {index}"))
        })
        .collect::<Result<Vec<String>>>()?;

    let motion_count = cursor.u16()?;
    let mut motions = Vec::with_capacity(motion_count as usize);
    for _ in 0..motion_count {
        let name = cursor.stringz()?;
        let flags = cursor.u32()?;
        // CMotionDef::Load
        let _bone_or_part = cursor.u16()?;
        let _motion = cursor.u16()?;
        let speed = cursor.f32()?;
        let power = cursor.f32()?;
        let accrue = cursor.f32()?;
        let falloff = cursor.f32()?;
        if version >= 4 {
            let marks = cursor.u32()?;
            for _ in 0..marks {
                // motion_marks::Load writes the name with a line terminator
                // rather than a NUL, then one interval pair per entry.
                terminated_string(&mut cursor)?;
                let intervals = cursor.u32()?;
                for _ in 0..intervals {
                    let _start = cursor.f32()?;
                    let _end = cursor.f32()?;
                }
            }
        }
        motions.push(Motion {
            name,
            flags,
            frames: 0,
            speed,
            power,
            accrue,
            falloff,
            tracks: Vec::new(),
        });
    }

    Ok((version, bones, partitions, motions))
}

/// Decode every bone's keys for one motion, in motion bone order.
fn parse_tracks(
    cursor: &mut Cursor<'_>,
    bones: usize,
    frames: u32,
    label: &str,
    motion: &str,
) -> Result<Vec<Track>> {
    let frames = usize::try_from(frames)?;
    let mut tracks = Vec::with_capacity(bones);
    for bone in 0..bones {
        let flags = cursor.u8()?;

        let rotations = if flags & FL_R_KEY_ABSENT != 0 {
            vec![rotation_key(cursor)?]
        } else {
            let _crc = cursor.u32()?;
            (0..frames)
                .map(|_| rotation_key(cursor))
                .collect::<Result<Vec<_>>>()?
        };

        let translations = if flags & FL_T_KEY_PRESENT != 0 {
            let _crc = cursor.u32()?;
            let wide = flags & FL_T_KEY_16 != 0;
            let quantised = (0..frames)
                .map(|_| {
                    Ok(if wide {
                        [
                            cursor.i16()? as f32,
                            cursor.i16()? as f32,
                            cursor.i16()? as f32,
                        ]
                    } else {
                        [
                            cursor.i8()? as f32,
                            cursor.i8()? as f32,
                            cursor.i8()? as f32,
                        ]
                    })
                })
                .collect::<Result<Vec<[f32; 3]>>>()?;
            let size = cursor.float3()?;
            let init = cursor.float3()?;
            quantised
                .into_iter()
                .map(|key| {
                    [
                        key[0] * size[0] + init[0],
                        key[1] * size[1] + init[1],
                        key[2] * size[2] + init[2],
                    ]
                })
                .collect()
        } else {
            vec![cursor.float3()?]
        };

        if rotations.is_empty() || translations.is_empty() {
            bail!("{label}: motion {motion} bone {bone} declares no keys");
        }
        tracks.push(Track {
            rotations,
            translations,
        });
    }
    Ok(tracks)
}

/// One quantised rotation key, expanded to `(x, y, z, w)`.
fn rotation_key(cursor: &mut Cursor<'_>) -> Result<[f32; 4]> {
    Ok([
        cursor.i16()? as f32 * KEY_QUANT_I,
        cursor.i16()? as f32 * KEY_QUANT_I,
        cursor.i16()? as f32 * KEY_QUANT_I,
        cursor.i16()? as f32 * KEY_QUANT_I,
    ])
}

/// `IReader::r_string`: bytes up to a line terminator, which is consumed.
fn terminated_string(cursor: &mut Cursor<'_>) -> Result<String> {
    let mut text = Vec::new();
    loop {
        let byte = cursor.u8()?;
        match byte {
            b'\r' => {
                // A bare \r is the terminator too; only swallow a \n that follows.
                if cursor.remaining() > 0 {
                    let mut peek = *cursor;
                    if peek.u8()? == b'\n' {
                        *cursor = peek;
                    }
                }
                break;
            }
            b'\n' => break,
            other => text.push(other),
        }
    }
    Ok(String::from_utf8(text)?)
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

    fn stringz(text: &str) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    /// One partition naming two bones, and one motion definition.
    fn params(version: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // partitions
        bytes.extend(stringz("default"));
        bytes.extend_from_slice(&2u16.to_le_bytes()); // bones in partition
        bytes.extend(stringz("bip01_pelvis"));
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend(stringz("bip01_spine"));
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // motions
        bytes.extend(stringz("cl_run_fwd_0"));
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&0u16.to_le_bytes()); // bone_or_part
        bytes.extend_from_slice(&0u16.to_le_bytes()); // motion
        for value in [1.0f32, 1.0, 2.0, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        if version >= 4 {
            bytes.extend_from_slice(&0u32.to_le_bytes()); // marks
        }
        bytes
    }

    /// Two bones over two frames: the first animated, the second static.
    fn motion_keys() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(stringz("cl_run_fwd_0"));
        bytes.extend_from_slice(&2u32.to_le_bytes()); // frames

        // bone 0: per-frame rotation and 8-bit translation keys
        bytes.push(FL_T_KEY_PRESENT);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // rotation crc
        for key in [[0i16, 0, 0, 32767], [0, 32767, 0, 0]] {
            for component in key {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // translation crc
        bytes.extend_from_slice(&[0u8, 0, 0]);
        bytes.extend_from_slice(&[100u8, 0, 0]);
        for value in [0.01f32, 0.01, 0.01] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0.5f32, 1.5, 2.5] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        // bone 1: one rotation, one translation
        bytes.push(FL_R_KEY_ABSENT);
        for component in [0i16, 0, 0, 32767] {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        for value in [1.0f32, 2.0, 3.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn bank(version: u16) -> Vec<u8> {
        let mut motions = chunk(0, &1u32.to_le_bytes());
        motions.extend(chunk(1, &motion_keys()));
        let mut bytes = chunk(OGF_S_SMPARAMS, &params(version));
        bytes.extend(chunk(OGF_S_MOTIONS, &motions));
        bytes
    }

    #[test]
    fn decodes_bones_partitions_and_motions() {
        let bank = parse(&bank(4), "mp_stalker.omf").unwrap();
        assert_eq!(bank.bones, vec!["bip01_pelvis", "bip01_spine"]);
        assert_eq!(bank.partitions.len(), 1);
        assert_eq!(bank.partitions[0].name, "default");
        assert_eq!(bank.motions.len(), 1);
        let motion = bank.motion("CL_RUN_FWD_0").expect("case-insensitive lookup");
        assert_eq!(motion.frames, 2);
        assert!((motion.duration() - 2.0 / 30.0).abs() < 1e-6);
    }

    /// A track with one key per frame reports both; a static one repeats.
    #[test]
    fn expands_quantised_keys() {
        let bank = parse(&bank(4), "mp_stalker.omf").unwrap();
        let motion = &bank.motions[0];

        let animated = &motion.tracks[0];
        assert!(!animated.is_static());
        assert_eq!(animated.rotation(0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(animated.rotation(1), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(animated.translation(0), [0.5, 1.5, 2.5]);
        let moved = animated.translation(1);
        assert!((moved[0] - 1.5).abs() < 1e-5, "{moved:?}");

        let still = &motion.tracks[1];
        assert!(still.is_static());
        assert_eq!(still.translation(0), [1.0, 2.0, 3.0]);
        assert_eq!(still.translation(7), [1.0, 2.0, 3.0]);
    }

    /// Version 3 banks stop before the per-motion marks.
    #[test]
    fn reads_a_bank_without_marks() {
        let bank = parse(&bank(3), "stalker_animation.omf").unwrap();
        assert_eq!(bank.version, 3);
        assert_eq!(bank.motions[0].name, "cl_run_fwd_0");
    }

    #[test]
    fn rejects_a_definition_and_key_count_mismatch() {
        let mut motions = chunk(0, &2u32.to_le_bytes());
        motions.extend(chunk(1, &motion_keys()));
        let mut bytes = chunk(OGF_S_SMPARAMS, &params(4));
        bytes.extend(chunk(OGF_S_MOTIONS, &motions));
        let error = parse(&bytes, "bank.omf").unwrap_err().to_string();
        assert!(error.contains("motion key chunks"), "{error}");
    }

    /// A bone left out of every partition would silently shift the key stream.
    #[test]
    fn rejects_an_unclaimed_bone_index() {
        let mut params = Vec::new();
        params.extend_from_slice(&4u16.to_le_bytes());
        params.extend_from_slice(&1u16.to_le_bytes());
        params.extend(stringz("default"));
        params.extend_from_slice(&1u16.to_le_bytes());
        params.extend(stringz("bip01_spine"));
        params.extend_from_slice(&1u32.to_le_bytes()); // index 0 never claimed
        params.extend_from_slice(&0u16.to_le_bytes());
        let bytes = chunk(OGF_S_SMPARAMS, &params);
        let error = parse(&bytes, "bank.omf").unwrap_err().to_string();
        assert!(error.contains("no partition claims motion bone 0"), "{error}");
    }
}
