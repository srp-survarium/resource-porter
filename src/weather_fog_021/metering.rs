// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's metering for a scene with an exposure profile: the mean luminance the tone
//! block follows is the weighted geometric mean of the frame, as 0.69d's is, where 0.26g's
//! is the arithmetic mean (a dark room with a sunlit doorway is exposed for the doorway).
//!
//! 0.26g meters in `stage_postprocess::measure_per_pixel_luminance`: `gather_log_luminance`
//! writes `dot(clamp(c, 0, 1e6), luma) + 0.001` of 256 x 256 points of the frame into x, y
//! and z (no logarithm, whatever its name) and 0 into w, eight 2 x 2 reductions
//! (`gather_luminance`, the last `gather_exp_luminance`) sum x, take the least y and the
//! greatest z and write 0 into w, and the last multiplies the sum by 1/65536. 0.69d
//! (`sample_luminance.ps`, `hdr_downsample_stage`, 0x20051CCBE of the retail exe) writes
//! `w ln(max(luminance, 0.01))` and `w` of 64 x 64 points of its pre-exposed frame, `w` 1
//! within a disc round the frame's centre and 0 outside, averages both and takes
//! `exp(x / w)` on the CPU (0x200504640), then divides by the exposure the frame was drawn
//! with (0x200504517).
//!
//! For a marked scene the first program writes `w log2(max(luminance, floor))` into x and
//! `w` into w, the reductions sum w as they sum x, and the last writes `2^(x / w)`: the same
//! weighted geometric mean, in 0.26g's light scale. y and z and every other scene keep the
//! native instructions.
use super::tone_map::{ExposureProfile, MIN_KEY_MARKER, add_marker_buffer_and_textures};
use super::{assemble, bytes, chunks, instructions, pinned_code, words};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

pub const LOG_METER_PREFIX: &str = "shaders/sm_4_0/gather_log_luminance.ps/";
pub const SUM_METER_PREFIX: &str = "shaders/sm_4_0/gather_luminance.ps/";
pub const MEAN_METER_PREFIX: &str = "shaders/sm_4_0/gather_exp_luminance.ps/";
const LOG_METER_HASH: &str = "05d1924036665f9d3c5ef08552850468be482614e88225752fedcec7082eaf0b";
const SUM_METER_HASH: &str = "b84e0a846012aa647f1da6d054666cbe34be52a80221e4963cbcc29b54ee6848";
const MEAN_METER_HASH: &str = "4592c20b41dfca8427dc4ce7cf3bbb465858964007010173ae650fbb39d08e71";

/// The least luminance 0.69d meters a point of its frame at, in its pre-exposed frame's
/// units (`max r0.z, r0.z, l(0.01)` in `sample_luminance.ps`).
const PIXEL_FLOOR_069: f32 = 0.01;

/// The radius of 0.69d's metering disc, in half-heights of the frame round its centre
/// (`sample_luminance.ps` compares `|(2u - 1) aspect, 2v - 1|` with `cb4[0].y`): the whole
/// of a 16:10 frame, all but 0.1 % of a 16:9 one (its corners), 82 % of a 21:9 one.
const DISC_RADIUS_069: f32 = 2.0;

/// The adapted mean luminance the first program reads (the tone pass's name for it; patch
/// 170 binds `$user$frame_luminance` there, which holds the previous frame's when the chain
/// begins).
const ADAPTED: &str = "t_frame_luminance";

/// The least luminance a point of a marked scene's frame is metered at, in 0.26g's light
/// scale. 0.69d's floor is 0.01 of its pre-exposed frame, which in 0.26g's scale is
/// `0.01 * Lc / ceiling`, `Lc` the adapted mean clamped to the profile's range (0.01 at the
/// ceiling, where the two scales meet): the first program reads the adapted mean. Where it
/// is not bound (an exe without patch 170) it reads 0, and the floor is this one, the
/// longest exposure's: no point is metered brighter than 0.69d meters it.
#[cfg(test)]
fn pixel_floor(e: &ExposureProfile) -> f32 {
    PIXEL_FLOOR_069 * e.floor / e.ceiling
}

/// What a program's reflection binds besides its own: the marker's buffer and the textures
/// asked for, in order.
struct Bound {
    marker: u32,
    textures: Vec<u32>,
}

fn check(exposures: &[ExposureProfile]) -> Result<()> {
    ensure!(!exposures.is_empty(), "no exposure profiles to meter for");
    let mut seen = BTreeSet::new();
    for e in exposures {
        ensure!(
            e.marker.is_finite()
                && e.marker >= MIN_KEY_MARKER
                && e.marker.fract() == 0.0
                && seen.insert(e.marker.to_bits())
                && [e.floor, e.ceiling].iter().all(|v| v.is_finite() && *v > 0.0)
                && e.floor < e.ceiling,
            "invalid or duplicate exposure profile"
        );
    }
    Ok(())
}

/// `eq rT.x, cbS[18].w, l(marker)` and `if_nz rT.x`: the scene carries the marker.
fn if_marked(slot: u32, temp: u32, marker: f32) -> [Vec<u32>; 2] {
    [
        vec![0x08000018, 0x00100012, temp, 0x0020803a, slot, 18, 0x00004001, marker.to_bits()],
        vec![0x0304001f, 0x0010000a, temp],
    ]
}

/// The program with the marker's buffer declared, `extra` more temporaries (the first of
/// them returned) and its instructions up to `body`.
fn open(ops: &[Vec<u32>], slot: u32, body: usize, extra: u32) -> Result<(Vec<Vec<u32>>, u32)> {
    let temps = ops
        .iter()
        .position(|op| op[0] & 2047 == 104)
        .context("missing temporaries")?;
    ensure!(
        temps < body && ops[temps].len() == 2 && ops[..temps].iter().all(|op| op[0] & 2047 != 89),
        "unexpected declarations"
    );
    let temp = ops[temps][1];
    let mut out = ops[..temps].to_vec();
    out.push(vec![0x04000059, 0x00208e46, slot, 19]); // static_globals
    out.push(vec![0x02000068, temp + extra]);
    out.extend_from_slice(&ops[temps + 1..body]);
    Ok((out, temp))
}

/// `gather_log_luminance` writing the weighted logarithm for a marked scene. The program's
/// last four instructions are pinned token for token:
///
/// ```text
/// dp3 r0.x, r0.xyzx, l(0.2125, 0.7154, 0.0721, 0)
/// add o0.xyz, r0.xxxx, l(0.001, 0.001, 0.001, 0)      else branch, as it is
/// mov o0.w, l(0)                                     else branch, as it is
/// ret
/// ```
///
/// and a marked scene has, for the `add` and the `mov`,
///
/// ```text
/// add o0.yz, r0.xxxx, l(0, 0.001, 0.001, 0)           y and z as the native add
/// ld r2.x, l(0, 0, 0, 0), t1.xyzw                     the adapted mean (0 unbound)
/// max r2.x, r2.x, l(floor)
/// min r2.x, r2.x, l(ceiling)
/// mul r2.x, r2.x, l(0.01 / ceiling)                   0.69d's floor of the moment
/// max r1.x, r0.x, r2.x
/// log r1.x, r1.x
/// resinfo r2.xy, l(0), t0.xyzw                        the frame's width and height
/// div r2.x, r2.x, r2.y
/// mad r2.yz, v1.xxyx, l(2, 2, 2, 2), l(-1, -1, -1, -1)
/// mul r2.y, r2.y, r2.x
/// dp2 r2.y, r2.yzyy, r2.yzyy
/// ge r2.y, l(radius^2), r2.y
/// and r2.y, r2.y, l(1.0)                              the weight: 1 in the disc, else 0
/// mul o0.x, r1.x, r2.y
/// mov o0.w, r2.y
/// ```
fn log_meter(ops: &[Vec<u32>], bound: &Bound, exposures: &[ExposureProfile]) -> Result<Vec<Vec<u32>>> {
    const DP3: [u32; 10] = [0x0a000010, 0x00100012, 0, 0x00100246, 0, 0x00004002, 0x3e59999a, 0x3f372474, 0x3d93a92a, 0];
    const ADD: [u32; 10] = [0x0a000000, 0x00102072, 0, 0x00100006, 0, 0x00004002, 0x3a83126f, 0x3a83126f, 0x3a83126f, 0];
    const ALPHA: [u32; 5] = [0x05000036, 0x00102082, 0, 0x00004001, 0];
    let n = ops.len();
    ensure!(
        n > 4 && ops[n - 4] == DP3 && ops[n - 3] == ADD && ops[n - 2] == ALPHA && ops[n - 1] == [0x0100003e],
        "the native luminance program changed"
    );
    let slot = bound.marker;
    let adapted = *bound.textures.first().context("no texture for the adapted mean")?;
    let (mut out, t) = open(ops, slot, n - 3, 2)?;
    let u = t + 1;
    // dcl_resource_texture2d (float,float,float,float) t<adapted>, after the frame's.
    let frame = out.iter().position(|op| op[0] & 2047 == 88).context("missing the frame's declaration")?;
    out.insert(frame + 1, vec![0x04001858, 0x00107000, adapted, 0x00005555]);
    let two = 2f32.to_bits();
    let minus_one = (-1f32).to_bits();
    for e in exposures {
        out.extend(if_marked(slot, t, e.marker));
        out.push(vec![0x0a000000, 0x00102062, 0, 0x00100006, 0, 0x00004002, 0, ADD[7], ADD[8], 0]);
        out.push(vec![0x0a00002d, 0x00100012, u, 0x00004002, 0, 0, 0, 0, 0x00107e46, adapted]);
        out.push(vec![0x07000034, 0x00100012, u, 0x0010000a, u, 0x00004001, e.floor.to_bits()]);
        out.push(vec![0x07000033, 0x00100012, u, 0x0010000a, u, 0x00004001, e.ceiling.to_bits()]);
        out.push(vec![0x07000038, 0x00100012, u, 0x0010000a, u, 0x00004001, (PIXEL_FLOOR_069 / e.ceiling).to_bits()]);
        out.push(vec![0x07000034, 0x00100012, t, 0x0010000a, 0, 0x0010000a, u]);
        out.push(vec![0x0500002f, 0x00100012, t, 0x0010000a, t]);
        out.push(vec![0x0700003d, 0x00100032, u, 0x00004001, 0, 0x00107e46, 0]);
        out.push(vec![0x0700000e, 0x00100012, u, 0x0010000a, u, 0x0010001a, u]);
        out.push(vec![0x0f000032, 0x00100062, u, 0x00101106, 1, 0x00004002, two, two, two, two, 0x00004002, minus_one, minus_one, minus_one, minus_one]);
        out.push(vec![0x07000038, 0x00100022, u, 0x0010001a, u, 0x0010000a, u]);
        out.push(vec![0x0700000f, 0x00100022, u, 0x00100596, u, 0x00100596, u]);
        out.push(vec![0x0700001d, 0x00100022, u, 0x00004001, (DISC_RADIUS_069 * DISC_RADIUS_069).to_bits(), 0x0010001a, u]);
        out.push(vec![0x07000001, 0x00100022, u, 0x0010001a, u, 0x00004001, 1f32.to_bits()]);
        out.push(vec![0x07000038, 0x00102012, 0, 0x0010000a, t, 0x0010001a, u]);
        out.push(vec![0x05000036, 0x00102082, 0, 0x0010001a, u]);
        out.push(vec![0x01000012]);
    }
    out.push(ops[n - 3].clone());
    out.push(ops[n - 2].clone());
    out.extend(std::iter::repeat_n(vec![0x01000015], exposures.len()));
    out.push(ops[n - 1].clone());
    Ok(out)
}

/// `r0.w + r1.w + r2.w + r3.w` into `dest` (the target's operand token and register), by
/// way of `rT.x`: the weights of the four samples of a 2 x 2 reduction, in `r0` to `r3`
/// with their w untouched after them (as both reductions have them, pinned by their hashes).
fn sum_of_weights(dest: u32, register: u32, t: u32) -> [Vec<u32>; 3] {
    [
        vec![0x07000000, 0x00100012, t, 0x0010003a, 0, 0x0010003a, 1],
        vec![0x07000000, 0x00100012, t, 0x0010000a, t, 0x0010003a, 2],
        vec![0x07000000, dest, register, 0x0010000a, t, 0x0010003a, 3],
    ]
}

/// `gather_luminance` summing the weight of a marked scene's points into w as it sums x.
/// Its first instruction (`mov o0.w, l(0)`) and last (`ret`) are pinned; a marked scene
/// then has `w = r0.w + r1.w + r2.w + r3.w`.
fn sum_meter(ops: &[Vec<u32>], bound: &Bound, exposures: &[ExposureProfile]) -> Result<Vec<Vec<u32>>> {
    let slot = bound.marker;
    let n = ops.len();
    let first = ops.iter().position(|op| op[0] & 2047 == 104).context("missing temporaries")? + 1;
    ensure!(
        n > first + 1 && ops[first] == [0x05000036, 0x00102082, 0, 0x00004001, 0] && ops[n - 1] == [0x0100003e],
        "the native reduction changed"
    );
    let (mut out, t) = open(ops, slot, n - 1, 1)?;
    for e in exposures {
        out.extend(if_marked(slot, t, e.marker));
        out.extend(sum_of_weights(0x00102082, 0, t));
        out.push(vec![0x01000012]);
    }
    out.extend(std::iter::repeat_n(vec![0x01000015], exposures.len()));
    out.push(ops[n - 1].clone());
    Ok(out)
}

/// `gather_exp_luminance` turning the weighted mean of the logarithms back for a marked
/// scene. Its last two instructions are pinned,
///
/// ```text
/// mul o0.x, r0.x, l(1/65536)                          else branch, as it is
/// ret
/// ```
///
/// and a marked scene has `w = r0.w + r1.w + r2.w + r3.w` (65536 where it is 0: no weight
/// reached it), then `div r4.x, r0.x, r4.y` and `exp o0.x, r4.x`.
fn mean_meter(ops: &[Vec<u32>], bound: &Bound, exposures: &[ExposureProfile]) -> Result<Vec<Vec<u32>>> {
    let slot = bound.marker;
    const MEAN: [u32; 7] = [0x07000038, 0x00102012, 0, 0x0010000a, 0, 0x00004001, 0x37800000];
    let n = ops.len();
    ensure!(
        n > 2 && ops[n - 2] == MEAN && ops[n - 1] == [0x0100003e],
        "the native mean luminance program changed"
    );
    let (mut out, t) = open(ops, slot, n - 2, 1)?;
    for e in exposures {
        out.extend(if_marked(slot, t, e.marker));
        out.extend(sum_of_weights(0x00100022, t, t));
        out.push(vec![0x07000018, 0x00100042, t, 0x0010001a, t, 0x00004001, 0]);
        out.push(vec![0x09000037, 0x00100022, t, 0x0010002a, t, 0x00004001, 65536f32.to_bits(), 0x0010001a, t]);
        out.push(vec![0x0700000e, 0x00100012, t, 0x0010000a, 0, 0x0010001a, t]);
        out.push(vec![0x05000019, 0x00102012, 0, 0x0010000a, t]);
        out.push(vec![0x01000012]);
    }
    out.push(ops[n - 2].clone());
    out.extend(std::iter::repeat_n(vec![0x01000015], exposures.len()));
    out.push(ops[n - 1].clone());
    Ok(out)
}

type Meter = fn(&[Vec<u32>], &Bound, &[ExposureProfile]) -> Result<Vec<Vec<u32>>>;

fn build(target: &[u8], hash: &str, exposures: &[ExposureProfile], textures: &[&str], meter: Meter) -> Result<Vec<u8>> {
    check(exposures)?;
    let (offset, native) = pinned_code(target, hash)?;
    let mut entries = chunks(native)?;
    let reflection = entries
        .iter_mut()
        .find(|(t, _)| t == b"RDEF")
        .context("missing metering reflection")?;
    let (with_marker, marker, textures) = add_marker_buffer_and_textures(&reflection.1, textures)?;
    reflection.1 = with_marker;
    let bound = Bound { marker, textures };
    let program = entries
        .iter_mut()
        .find(|(t, _)| t == b"SHDR")
        .context("missing metering program")?;
    let native_words = words(&program.1)?;
    let mut updated = native_words[..2].to_vec();
    updated.extend(meter(&instructions(&native_words)?, &bound, exposures)?.iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    program.1 = bytes(&updated);
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

/// The native `gather_log_luminance` program metering each exposure profile's scene by the
/// weighted logarithm of its luminance.
pub fn build_log_meter(target: &[u8], exposures: &[ExposureProfile]) -> Result<Vec<u8>> {
    build(target, LOG_METER_HASH, exposures, &[ADAPTED], log_meter)
}

/// The native `gather_luminance` program summing each exposure profile's scene's weights.
pub fn build_sum_meter(target: &[u8], exposures: &[ExposureProfile]) -> Result<Vec<u8>> {
    build(target, SUM_METER_HASH, exposures, &[], sum_meter)
}

/// The native `gather_exp_luminance` program giving each exposure profile's scene the
/// weighted geometric mean.
pub fn build_mean_meter(target: &[u8], exposures: &[ExposureProfile]) -> Result<Vec<u8>> {
    build(target, MEAN_METER_HASH, exposures, &[], mean_meter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather_fog_021::tone_map::add_marker_buffer;
    use crate::weather_fog_021::u32_at;

    const RUDNYA: ExposureProfile = ExposureProfile { marker: 33001.0, floor: 0.001151, ceiling: 0.104167, key: 0.2334 };

    fn hex(text: &str) -> Vec<Vec<u32>> {
        text.lines().map(|l| l.split_whitespace().map(|w| u32::from_str_radix(w, 16).unwrap()).collect()).collect()
    }

    /// The native `gather_log_luminance` program.
    fn native_log() -> Vec<Vec<u32>> {
        hex("0300005a 00106000 00000000
            04001858 00107000 00000000 00005555
            03001062 00101032 00000001
            03000065 001020f2 00000000
            02000068 00000001
            09000045 001000f2 00000000 00101046 00000001 00107e46 00000000 00106000 00000000
            0a000034 00100072 00000000 00100246 00000000 00004002 00000000 00000000 00000000 00000000
            0a000033 00100072 00000000 00100246 00000000 00004002 49742400 49742400 49742400 00000000
            0a000010 00100012 00000000 00100246 00000000 00004002 3e59999a 3f372474 3d93a92a 00000000
            0a000000 00102072 00000000 00100006 00000000 00004002 3a83126f 3a83126f 3a83126f 00000000
            05000036 00102082 00000000 00004001 00000000
            0100003e")
    }

    /// The native `gather_exp_luminance` program.
    fn native_mean() -> Vec<Vec<u32>> {
        hex("0300005a 00106000 00000000
            04001858 00107000 00000000 00005555
            03001062 00101032 00000001
            03000065 001020f2 00000000
            02000068 00000004
            05000036 00102082 00000000 00004001 00000000
            8a000045 0001fe01 001000f2 00000000 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 00004001 4f000000
            8a000045 0001e001 001000f2 00000001 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 0010001a 00000001
            8a000045 00001e01 001000f2 00000002 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 0010001a 00000002
            8a000045 00000001 001000f2 00000003 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00102022 00000000 0010001a 00000000 0010001a 00000003
            07000034 00100022 00000000 0010002a 00000000 00004001 cf000000
            07000000 00100012 00000000 0010000a 00000000 0010000a 00000001
            07000034 00100022 00000000 0010002a 00000001 0010001a 00000000
            07000034 00100022 00000000 0010002a 00000002 0010001a 00000000
            07000000 00100012 00000000 0010000a 00000002 0010000a 00000000
            07000000 00100012 00000000 0010000a 00000003 0010000a 00000000
            07000034 00102042 00000000 0010002a 00000003 0010001a 00000000
            07000038 00102012 00000000 0010000a 00000000 00004001 37800000
            0100003e")
    }

    /// The native `gather_luminance` program.
    fn native_sum() -> Vec<Vec<u32>> {
        hex("0300005a 00106000 00000000
            04001858 00107000 00000000 00005555
            03001062 00101032 00000001
            03000065 001020f2 00000000
            02000068 00000004
            05000036 00102082 00000000 00004001 00000000
            8a000045 0001fe01 001000f2 00000000 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 00004001 4f000000
            8a000045 0001e001 001000f2 00000001 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 0010001a 00000001
            8a000045 00001e01 001000f2 00000002 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00100022 00000000 0010001a 00000000 0010001a 00000002
            8a000045 00000001 001000f2 00000003 00101046 00000001 00107e46 00000000 00106000 00000000
            07000033 00102022 00000000 0010001a 00000000 0010001a 00000003
            07000034 00100022 00000000 0010002a 00000000 00004001 cf000000
            07000000 00100012 00000000 0010000a 00000000 0010000a 00000001
            07000034 00100022 00000000 0010002a 00000001 0010001a 00000000
            07000034 00100022 00000000 0010002a 00000002 0010001a 00000000
            07000000 00100012 00000000 0010000a 00000002 0010000a 00000000
            07000000 00102012 00000000 0010000a 00000003 0010000a 00000000
            07000034 00102042 00000000 0010002a 00000003 0010001a 00000000
            0100003e")
    }

    /// The native `gather_log_luminance` reflection: a sampler and a texture, no buffer.
    fn native_reflection() -> Vec<u8> {
        let text = "0000000000000000020000001c0000000004ffff08810000790000005c00000003000000000000000000000000000000\
            0000000001000000000000006a000000020000000500000004000000ffffffff00000000010000000c000000735f616363756d\
            756c61746f7200745f6672616d655f636f6c6f7230004d6963726f736f66742028522920484c534c2053686164657220436f6d\
            70696c657220392e32392e3935322e3331313100abab";
        (0..text.len() / 2).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// The marker's buffer at `marker`, the adapted mean (the first program's) at t1.
    fn at(marker: u32) -> Bound {
        Bound { marker, textures: vec![1] }
    }

    /// A 16:9 frame's centre: where every point weighs 1; no adapted mean bound.
    const CENTRE: Point = Point { uv: [0.5, 0.5], size: [1920.0, 1080.0], adapted: 0.0 };

    /// A point of the frame (`v1`), the frame's size (`resinfo` of `t0`) and the adapted
    /// mean (`ld` of t1: 0 where it is not bound).
    #[derive(Clone, Copy)]
    struct Point {
        uv: [f32; 2],
        size: [f32; 2],
        adapted: f32,
    }

    /// What a program writes at the frame's centre.
    fn run(ops: &[Vec<u32>], texels: &[[f32; 4]], marker: f32) -> [f32; 4] {
        run_at(ops, texels, marker, CENTRE)
    }

    /// What a program writes: the instructions these programs have, on four-component
    /// registers. `texels` are what its samples return, in order; `marker` is cb[18].w.
    fn run_at(ops: &[Vec<u32>], texels: &[[f32; 4]], marker: f32, point: Point) -> [f32; 4] {
        let mut temps = [[0f32; 4]; 8];
        let mut output = [f32::NAN; 4];
        let mut texels = texels.iter();
        // Per open `if`: whether the branch being read runs.
        let mut blocks: Vec<bool> = Vec::new();
        for op in ops {
            let code = op[0] & 2047;
            let live = blocks.iter().all(|taken| *taken);
            match code {
                // Declarations.
                88 | 89 | 90 | 98 | 101 | 104 => continue,
                18 => {
                    let taken = blocks.pop().unwrap();
                    blocks.push(!taken);
                    continue;
                }
                21 => {
                    blocks.pop().unwrap();
                    continue;
                }
                _ if !live => {
                    if code == 31 {
                        blocks.push(false);
                    }
                    continue;
                }
                _ => {}
            }
            // Operands: (kind, register, components or mask, immediate values).
            let mut at = 1 + usize::from(op[0] >> 31 != 0);
            let mut operands = Vec::new();
            while at < op.len() {
                let token = op[at];
                assert_eq!(token >> 31, 0, "operand modifiers are not used here");
                let kind = (token >> 12) & 255;
                let dimension = ((token >> 20) & 3) as usize;
                let select: [usize; 4] = match (token & 3, (token >> 2) & 3) {
                    (2, 1) => std::array::from_fn(|i| ((token >> (4 + 2 * i)) & 3) as usize),
                    (2, 2) => [((token >> 4) & 3) as usize; 4],
                    _ => [0, 1, 2, 3],
                };
                let mask = (token >> 4) & 15;
                at += 1;
                let value: [f32; 4] = match kind {
                    0 => select.map(|c| temps[op[at] as usize][c]),
                    1 => {
                        assert_eq!(op[at], 1);
                        select.map(|c| [point.uv[0], point.uv[1], 0.0, 0.0][c])
                    }
                    4 if token & 3 == 1 => [f32::from_bits(op[at]); 4],
                    4 => std::array::from_fn(|i| f32::from_bits(op[at + i])),
                    8 => {
                        assert_eq!((op[at + 1], select), (18, [3; 4]));
                        [marker; 4]
                    }
                    _ => [0.0; 4],
                };
                let register = op[at];
                at += if kind == 4 { if token & 3 == 1 { 1 } else { 4 } } else { dimension };
                operands.push((kind, register, mask, value));
            }
            let source = |i: usize| operands[i].3;
            let bits = |b: bool| f32::from_bits(if b { u32::MAX } else { 0 });
            let result: [f32; 4] = match code {
                0 => std::array::from_fn(|i| source(1)[i] + source(2)[i]),
                1 => std::array::from_fn(|i| f32::from_bits(source(1)[i].to_bits() & source(2)[i].to_bits())),
                14 => std::array::from_fn(|i| source(1)[i] / source(2)[i]),
                15 => [(0..2).map(|i| source(1)[i] * source(2)[i]).sum(); 4],
                29 => std::array::from_fn(|i| bits(source(1)[i] >= source(2)[i])),
                50 => std::array::from_fn(|i| source(1)[i] * source(2)[i] + source(3)[i]),
                55 => std::array::from_fn(|i| if source(1)[i].to_bits() != 0 { source(2)[i] } else { source(3)[i] }),
                45 => [point.adapted, 0.0, 0.0, 0.0],
                61 => [point.size[0], point.size[1], 1.0, 1.0],
                16 => [(0..3).map(|i| source(1)[i] * source(2)[i]).sum(); 4],
                24 => std::array::from_fn(|i| bits(source(1)[i] == source(2)[i])),
                25 => source(1).map(f32::exp2),
                31 => {
                    blocks.push(source(0)[0].to_bits() != 0);
                    continue;
                }
                47 => source(1).map(f32::log2),
                51 => std::array::from_fn(|i| source(1)[i].min(source(2)[i])),
                52 => std::array::from_fn(|i| source(1)[i].max(source(2)[i])),
                54 => source(1),
                56 => std::array::from_fn(|i| source(1)[i] * source(2)[i]),
                62 => break,
                69 => *texels.next().unwrap(),
                other => panic!("instruction {other} is not in these programs"),
            };
            let (kind, register, mask, _) = operands[0];
            let target = if kind == 2 { &mut output } else { &mut temps[register as usize] };
            for i in 0..4 {
                if mask & (1 << i) != 0 {
                    target[i] = result[i];
                }
            }
        }
        assert!(blocks.is_empty());
        output
    }

    #[test]
    fn profiles_and_programs_are_checked() {
        for bad in [
            vec![],
            vec![ExposureProfile { marker: 60.0, ..RUDNYA }],
            vec![ExposureProfile { marker: 33001.5, ..RUDNYA }],
            vec![ExposureProfile { floor: 0.2, ..RUDNYA }],
            vec![RUDNYA; 2],
        ] {
            assert!(build_log_meter(&[], &bad).is_err());
            assert!(build_sum_meter(&[], &bad).is_err());
            assert!(build_mean_meter(&[], &bad).is_err());
        }
        // Not the pinned program.
        assert!(build_log_meter(&[0; 400], &[RUDNYA]).is_err());
        // Another tail is refused, and a program that already has a constant buffer
        // declared is not one of these two.
        let mut other = native_log();
        other[9][6] = 0.002f32.to_bits();
        assert!(log_meter(&other, &at(0), &[RUDNYA]).is_err());
        let mut other = native_mean();
        other[21][6] = 0.5f32.to_bits();
        assert!(mean_meter(&other, &at(0), &[RUDNYA]).is_err());
        let mut other = native_sum();
        other[5][4] = 1f32.to_bits();
        assert!(sum_meter(&other, &at(0), &[RUDNYA]).is_err());
        let mut other = native_log();
        other.insert(0, vec![0x04000059, 0x00208e46, 0, 1]);
        assert!(log_meter(&other, &at(1), &[RUDNYA]).is_err());
    }

    #[test]
    fn the_marker_buffer_takes_the_first_free_slot() {
        let old = native_reflection();
        let (new, slot) = add_marker_buffer(&old).unwrap();
        // No buffer before it: the game binds it at b0.
        assert_eq!(slot, 0);
        assert_eq!((u32_at(&new, 0).unwrap(), u32_at(&new, 8).unwrap(), u32_at(&new, 12).unwrap()), (1, 3, 28));
        let text = |at: usize| {
            let at = u32_at(&new, at).unwrap() as usize;
            std::str::from_utf8(&new[at..at + new[at..].iter().position(|b| *b == 0).unwrap()]).unwrap().to_owned()
        };
        let table = u32_at(&new, 4).unwrap() as usize;
        // The native bindings as they were, then the buffer's; every binding name before
        // the buffer table.
        for (i, name) in ["s_accumulator", "t_frame_color0", "static_globals"].iter().enumerate() {
            assert_eq!(&text(28 + 32 * i), name);
            assert!((28 + 96..table).contains(&(u32_at(&new, 28 + 32 * i).unwrap() as usize)));
        }
        assert_eq!(new[32..60], old[32..60]);
        assert_eq!(new[64..92], old[64..92]);
        assert_eq!(words(&new[96..124]).unwrap(), [0, 0, 0, 0, 0, 1, 0]);
        // One buffer of 304 bytes with the one used float4 at offset 288.
        assert_eq!(text(table), "static_globals");
        assert_eq!(words(&new[table + 4..table + 24]).unwrap()[..1], [1]);
        assert_eq!(words(&new[table + 12..table + 24]).unwrap(), [304, 0, 0]);
        let variable = u32_at(&new, table + 8).unwrap() as usize;
        assert_eq!(text(variable), "atmosphere_parameters3");
        assert_eq!(words(&new[variable + 4..variable + 16]).unwrap(), [288, 16, 2]);
        let ty = u32_at(&new, variable + 16).unwrap() as usize;
        assert_eq!(new[ty..ty + 16], [1, 0, 3, 0, 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!((ty % 4, new.len() % 4 < 4), (0, true));
        assert!(text(24).starts_with("Microsoft (R) HLSL Shader Compiler"));
        // A second buffer goes to the next slot; the same buffer twice is refused.
        assert!(add_marker_buffer(&new).is_err());
        let mut renamed = new.clone();
        let at = u32_at(&renamed, 28 + 64).unwrap() as usize;
        renamed[at] = b'S';
        assert_eq!(add_marker_buffer(&renamed).unwrap().1, 1);
        // A buffer bound away from its place in the table is refused (the game would bind
        // it at its place).
        let mut moved = renamed;
        moved[28 + 64 + 20] = 1;
        assert!(add_marker_buffer(&moved).is_err());
    }

    #[test]
    fn the_adapted_mean_is_bound_after_the_frame() {
        let (new, slot, textures) = add_marker_buffer_and_textures(&native_reflection(), &[ADAPTED]).unwrap();
        assert_eq!((slot, textures), (0, vec![1]));
        assert_eq!(u32_at(&new, 8).unwrap(), 4);
        let text = |at: usize| {
            let at = u32_at(&new, at).unwrap() as usize;
            std::str::from_utf8(&new[at..at + new[at..].iter().position(|b| *b == 0).unwrap()]).unwrap().to_owned()
        };
        // The native sampler and frame, the adapted mean (a float4 2D texture at t1), the buffer.
        for (i, name) in ["s_accumulator", "t_frame_color0", ADAPTED, "static_globals"].iter().enumerate() {
            assert_eq!(&text(28 + 32 * i), name);
        }
        assert_eq!(words(&new[28 + 64 + 4..28 + 96]).unwrap(), [2, 5, 4, u32::MAX, 1, 1, 0x0c]);
        assert_eq!(words(&new[28 + 32 + 4..28 + 64]).unwrap()[..5], [2, 5, 4, u32::MAX, 0]);
        // The same name twice is refused.
        assert!(add_marker_buffer_and_textures(&native_reflection(), &["t_frame_color0"]).is_err());
    }

    #[test]
    fn an_unmarked_scene_keeps_the_native_arithmetic() {
        let two = [RUDNYA, ExposureProfile { marker: 33002.0, ..RUDNYA }];
        for profiles in [&two[..1], &two[..]] {
            let (native, adapted) = (native_log(), log_meter(&native_log(), &at(0), profiles).unwrap());
            // The declarations but the buffer's and one more temporary; the native
            // instructions in their order, the `add` in the innermost else.
            // The adapted mean's texture declared after the frame's.
            assert_eq!(adapted[..2], native[..2]);
            assert_eq!(adapted[2], [0x04001858, 0x00107000, 1, 0x00005555]);
            assert_eq!(adapted[3..5], native[2..4]);
            assert_eq!(adapted[5], [0x04000059, 0x00208e46, 0, 19]);
            assert_eq!(adapted[6], [0x02000068, 3]);
            assert_eq!(adapted[7..11], native[5..9]);
            let n = adapted.len();
            let ends = profiles.len();
            // The `add` and the `mov` in the innermost else, then the `ret`.
            assert_eq!(adapted[n - 3 - ends..n - 1 - ends], native[9..11]);
            assert_eq!(adapted[n - 4 - ends], [0x01000012]);
            assert!(adapted[n - 1 - ends..n - 1].iter().all(|op| op == &[0x01000015]));
            assert_eq!(adapted[n - 1], native[11]);
            for colour in [[0.0, 0.0, 0.0, 1.0], [0.3, 0.2, 0.1, 1.0], [5e6, 1.0, -1.0, 0.0], [1e-5, 2e-5, 0.0, 0.5]] {
                let expected = run(&native, &[colour], 0.0).map(f32::to_bits);
                for marker in [0.0, 60.0, -32198.0, 33003.0] {
                    assert_eq!(run(&adapted, &[colour], marker).map(f32::to_bits), expected);
                }
            }
            let (native, adapted) = (native_mean(), mean_meter(&native_mean(), &at(0), profiles).unwrap());
            assert_eq!(adapted[..4], native[..4]);
            assert_eq!(adapted[4], [0x04000059, 0x00208e46, 0, 19]);
            assert_eq!(adapted[5], [0x02000068, 5]);
            assert_eq!(adapted[6..22], native[5..21]);
            let n = adapted.len();
            assert_eq!(adapted[n - 2 - ends], native[21]);
            assert_eq!(adapted[n - 1], native[22]);
            let texels = [[3.0, 0.2, 0.9, 0.0], [70000.0, 0.004, 0.5, 0.0], [0.5, 0.3, 4.0, 0.0], [12.0, 0.1, 0.2, 0.0]];
            let expected = run(&native, &texels, 0.0).map(f32::to_bits);
            assert_eq!(f32::from_bits(expected[0]), 70015.5 / 65536.0);
            for marker in [0.0, -32198.0, 33003.0] {
                assert_eq!(run(&adapted, &texels, marker).map(f32::to_bits), expected);
            }
            let (native, adapted) = (native_sum(), sum_meter(&native_sum(), &at(0), profiles).unwrap());
            assert_eq!(adapted[..4], native[..4]);
            assert_eq!(adapted[4], [0x04000059, 0x00208e46, 0, 19]);
            assert_eq!(adapted[5], [0x02000068, 5]);
            let n = adapted.len();
            // The native instructions, then per profile its branch (two, three and an else),
            // the ends and the `ret`.
            assert_eq!(adapted[6..n - 1 - 7 * ends], native[5..native.len() - 1]);
            assert_eq!(adapted[n - 1], native[native.len() - 1]);
            let texels = texels.map(|[x, y, z, _]| [x, y, z, 7.0]);
            let expected = run(&native, &texels, 0.0).map(f32::to_bits);
            assert_eq!(f32::from_bits(expected[0]), 70015.5);
            assert_eq!(f32::from_bits(expected[3]), 0.0);
            for marker in [0.0, -32198.0, 33003.0] {
                assert_eq!(run(&adapted, &texels, marker).map(f32::to_bits), expected);
            }
        }
    }

    #[test]
    fn a_marked_scene_is_metered_by_its_geometric_mean() {
        let floor = pixel_floor(&RUDNYA);
        // 0.69d's 0.01 of the pre-exposed frame at its longest exposure (EV 6 of 6..12.5).
        assert!((floor / (0.01 * 2f32.powf(6.0 - 12.5)) - 1.0).abs() < 1e-3);
        let log = log_meter(&native_log(), &at(0), &[RUDNYA]).unwrap();
        let mean = mean_meter(&native_mean(), &at(0), &[RUDNYA]).unwrap();
        let native = [native_log(), native_mean()];
        let grey = |l: f32| [l, l, l, 1.0];
        let luminance = |l: f32| l * (0.2125 + 0.7154 + 0.0721);
        // One point: the logarithm in x; y, z and w as the native program writes them.
        for l in [0.0, 1e-5, 0.003, 0.2, 40.0] {
            let (marked, plain) = (run(&log, &[grey(l)], 33001.0), run(&native[0], &[grey(l)], 33001.0));
            assert!((marked[0] - luminance(l).max(floor).log2()).abs() < 1e-5, "{l}");
            assert_eq!(marked.map(f32::to_bits)[1..3], plain.map(f32::to_bits)[1..3]);
            assert_eq!(marked[3], 1.0);
        }
        // The reductions sum the weight; the last gives 2^(x / w), the least y and the
        // greatest z as before.
        let sum = sum_meter(&native_sum(), &at(0), &[RUDNYA]).unwrap();
        let texels = [[-200000.0, 0.2, 0.9, 16384.0], [-150000.0, 0.004, 0.5, 16384.0], [-80000.0, 0.3, 4.0, 16384.0], [-94288.0, 0.1, 0.2, 16384.0]];
        let (marked, plain) = (run(&sum, &texels, 33001.0), run(&native_sum(), &texels, 33001.0));
        assert_eq!(marked.map(f32::to_bits)[..3], plain.map(f32::to_bits)[..3]);
        assert_eq!(marked[3], 65536.0);
        let (marked, plain) = (run(&mean, &texels, 33001.0), run(&native[1], &texels, 33001.0));
        assert!((marked[0] - 2f32.powf(-8.0)).abs() < 1e-7);
        assert_eq!(marked.map(f32::to_bits)[1..], plain.map(f32::to_bits)[1..]);
        // Half the weight: the same sum is twice the mean logarithm.
        let halved = texels.map(|[x, y, z, w]| [x, y, z, w / 2.0]);
        assert!((run(&mean, &halved, 33001.0)[0] - 2f32.powf(-16.0)).abs() < 1e-9);
        // No weight at all (w not kept by the targets): the unweighted mean, never a
        // division by 0.
        let unweighted = texels.map(|[x, y, z, _]| [x, y, z, 0.0]);
        assert!((run(&mean, &unweighted, 33001.0)[0] - 2f32.powf(-8.0)).abs() < 1e-7);
        // A church with a sunlit doorway: nine parts in ten at 0.001, one at 0.2. The chain
        // (the reductions between the two programs sum x) gives the geometric mean, which
        // stays near the room; the native chain gives the arithmetic one, near the door.
        let chain = |first: &[Vec<u32>], last: &[Vec<u32>], marker: f32| {
            let (dark, bright) = (run(first, &[grey(0.001)], marker)[0], run(first, &[grey(0.2)], marker)[0]);
            let quarter = 65536.0 / 4.0;
            let sums = [dark, dark, 0.6 * dark + 0.4 * bright, dark].map(|x| [x * quarter, 0.0, 0.0, quarter]);
            run(last, &sums, marker)[0]
        };
        let geometric = chain(&log, &mean, 33001.0);
        assert!((geometric / (0.001f32.powf(0.9) * 0.2f32.powf(0.1)) - 1.0).abs() < 1e-3, "{geometric}");
        let arithmetic = chain(&native[0], &native[1], 33001.0);
        assert!((arithmetic / (0.9 * 0.001 + 0.1 * 0.2 + 0.001) - 1.0).abs() < 1e-3, "{arithmetic}");
        assert_eq!(chain(&log, &mean, 0.0).to_bits(), arithmetic.to_bits());
        // A flat frame meters as itself, so the profile's range and key stay as they are.
        let flat = |l: f32| {
            let x = run(&log, &[grey(l)], 33001.0)[0] * 65536.0 / 4.0;
            run(&mean, &[[x, 0.0, 0.0, 65536.0 / 4.0]; 4], 33001.0)[0]
        };
        for l in [RUDNYA.floor, 0.01, RUDNYA.ceiling, 1.5] {
            assert!((flat(l) / luminance(l) - 1.0).abs() < 1e-4, "{l}");
        }
        // A black frame meters as the floor, far below the profile's own.
        assert!((flat(0.0) / floor - 1.0).abs() < 1e-4 && floor < 0.1 * RUDNYA.floor);
    }

    #[test]
    fn the_floor_follows_the_adapted_mean_as_0_69d_s_follows_its_exposure() {
        let log = log_meter(&native_log(), &at(0), &[RUDNYA]).unwrap();
        let black = |adapted: f32| run_at(&log, &[[0.0; 4]], 33001.0, Point { adapted, ..CENTRE })[0];
        // 0.01 of the frame as exposed: at the ceiling (a sunlit view) 0.01 itself; at the
        // floor, and with nothing bound (0), the longest exposure's, `pixel_floor`.
        assert!((black(RUDNYA.ceiling) - 0.01f32.log2()).abs() < 1e-5);
        assert!((black(RUDNYA.ceiling * 10.0) - 0.01f32.log2()).abs() < 1e-5);
        assert!((black(0.01) - (0.01 * 0.01 / RUDNYA.ceiling).log2()).abs() < 1e-5);
        for adapted in [0.0, RUDNYA.floor, RUDNYA.floor / 4.0, f32::NAN] {
            assert_eq!(black(adapted), pixel_floor(&RUDNYA).log2(), "{adapted}");
        }
        // A point above the floor is metered as itself whatever the mean.
        let grey = run_at(&log, &[[0.2; 4]], 33001.0, Point { adapted: RUDNYA.ceiling, ..CENTRE })[0];
        assert!((grey - (0.2f32 * (0.2125 + 0.7154 + 0.0721)).log2()).abs() < 1e-5);
    }

    #[test]
    fn points_outside_0_69d_s_disc_weigh_nothing() {
        let log = log_meter(&native_log(), &at(0), &[RUDNYA]).unwrap();
        let shot = |uv: [f32; 2], size: [f32; 2]| run_at(&log, &[[0.2, 0.2, 0.2, 1.0]], 33001.0, Point { uv, size, adapted: 0.0 });
        let inside = 0.2f32.log2();
        for (uv, size, weight) in [
            // A 16:10 frame is metered whole, a 16:9 one but its corners.
            ([0.0, 0.0], [1680.0, 1050.0], 1.0),
            ([0.0, 0.0], [1920.0, 1080.0], 0.0),
            ([0.05, 0.05], [1920.0, 1080.0], 1.0),
            ([1.0, 0.5], [1920.0, 1080.0], 1.0),
            // A 21:9 one has its sides left out.
            ([0.02, 0.5], [2560.0, 1080.0], 0.0),
            ([0.9, 0.5], [2560.0, 1080.0], 1.0),
            ([0.5, 0.0], [2560.0, 1080.0], 1.0),
        ] {
            let [x, _, _, w] = shot(uv, size);
            assert_eq!(w, weight, "{uv:?} {size:?}");
            assert!((x - weight * inside).abs() < 1e-5, "{uv:?} {size:?}");
        }
        // Unmarked, the native 0 in w wherever the point is.
        assert_eq!(run_at(&log, &[[0.2; 4]], 0.0, Point { uv: [0.02, 0.5], size: [2560.0, 1080.0], adapted: 0.0 })[3], 0.0);
        // A 21:9 frame bright at its sides and dark within: metered as the dark part
        // (0.69d's), not as the whole frame.
        let mean = mean_meter(&native_mean(), &at(0), &[RUDNYA]).unwrap();
        let point = |u: f32, l: f32| run_at(&log, &[[l, l, l, 1.0]], 33001.0, Point { uv: [u, 0.5], size: [2560.0, 1080.0], adapted: 0.0 });
        let (side, middle) = (point(0.02, 2.0), point(0.5, 0.01));
        let quarter = 65536.0 / 4.0;
        // Four quarters of the frame: one of sides, three of the middle.
        let sums = [side, middle, middle, middle].map(|[x, _, _, w]| [x * quarter, 0.0, 0.0, w * quarter]);
        let metered = run(&mean, &sums, 33001.0)[0];
        assert!((metered / (0.01 * (0.2125 + 0.7154 + 0.0721)) - 1.0).abs() < 1e-3, "{metered}");
    }

    #[test]
    fn each_profile_has_its_own_branch_and_floor() {
        let other = ExposureProfile { marker: 33002.0, floor: 0.01, ceiling: 0.1, key: 0.2 };
        let log = log_meter(&native_log(), &at(0), &[RUDNYA, other]).unwrap();
        let mean = mean_meter(&native_mean(), &at(0), &[RUDNYA, other]).unwrap();
        assert_eq!(run(&log, &[[0.0; 4]], 33001.0)[0], pixel_floor(&RUDNYA).log2());
        assert_eq!(run(&log, &[[0.0; 4]], 33002.0)[0], 0.001f32.log2());
        for marker in [33001.0, 33002.0] {
            assert_eq!(run(&mean, &[[-65536.0, 0.0, 0.0, 0.0]; 4], marker)[0], 2f32.powf(-4.0));
        }
        // The buffer's slot is the one the reflection gave it.
        let at_b1 = log_meter(&native_log(), &at(1), &[RUDNYA]).unwrap();
        assert_eq!(at_b1[5], [0x04000059, 0x00208e46, 1, 19]);
        assert_eq!(at_b1[11][3..6], [0x0020803a, 1, 18]);
    }
}
