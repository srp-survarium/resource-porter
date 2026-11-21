// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's cloud shadows on the sun, for a scene with a cloud shadow profile.
//!
//! 0.69d (`cloud_shadows.ps`, with `use_cloud_shadows`) lets through, of the sun at a
//! point, `1 - min((a * cloud_shadow_opacity)^cloud_shadow_opacity_power, 1)`: `a` the alpha
//! of the sky's cloud panorama where the point's ray toward the sun meets the cloud shell
//! (`cloud_altitude` km above the ground), seen from the map's origin (u its azimuth plus
//! the clouds' phase, `frac(sky_clouds_rotation / 360 + clouds_rounds_per_minute / 60 * t)`,
//! exe 0x200442DE1; v its angle from the zenith). On Rudnya (v0.69) that is 0.27 to 0.93 of
//! the sun as the clouds turn, 0.64 on average.
//!
//! 0.26g takes the least of the sun's shadow and `sky_shadows_texture.x` (`sun_shadow.ps`,
//! the 16 variants drawn with `use_sky_shadows`), the texture laid flat on the ground.
//! For a marked scene each of them computes, after its own sample of that texture, 0.69d's
//! instructions 12 to 72 (exact literal words; its constant buffer as the profile's values,
//! its time `scene_time`), with the scene's cloud panorama bound as `sky_shadows_texture`
//! by the environment. Every other scene keeps the native instructions. The tokens were
//! made from 0.69d's listing and bytecode by an assembler checked against both builds'
//! programs, and the branch was run against 0.69d's program on Rudnya's ground: equal to
//! 6e-5 (resource-porter-side check kept with the map's research: `re/atmosphere`).
use super::{assemble, bytes, chunks, instructions, pinned_code, u32_at, words};
use anyhow::{Context, Result, ensure};

pub const SUN_SHADOW_PREFIX: &str = "shaders/sm_4_0/sun_shadow.ps/";

/// The 16 variants that read `sky_shadows_texture` (stock 0.26g), by name and sha256.
pub const SKY_SHADOW_VARIANTS: [(&str, &str); 16] = [
    ("____01_________________________0___________________________________", "7048ccfc3b2f164b36e935d1dc20fd738c38472433c816a1dc7b61b561fa63a5"),
    ("____01_________________________1___________________________________", "8a687d537f132b4255a37ddae35a1e9474e074c4fe410bbbc66424209a3af6a6"),
    ("____01_________________________2___________________________________", "221e285fda2825f8cc0e9e56e0695e777fab62255342c1271b51b201f6ff3759"),
    ("____01_________________________3___________________________________", "221e285fda2825f8cc0e9e56e0695e777fab62255342c1271b51b201f6ff3759"),
    ("____11_________________________0___________________________________", "8a687d537f132b4255a37ddae35a1e9474e074c4fe410bbbc66424209a3af6a6"),
    ("____11_________________________1___________________________________", "f1ba63730863433aad0aa491d1d0aa7af3cb4d4ab2b457d0f89d2e2cc6bbe2f4"),
    ("____11_________________________2___________________________________", "5b0ed9f99f024ff447b1bb9763f680a4f9cb47f676b5568b8ad80981e99a9560"),
    ("____11_________________________3___________________________________", "5b0ed9f99f024ff447b1bb9763f680a4f9cb47f676b5568b8ad80981e99a9560"),
    ("____21_________________________0___________________________________", "c5257a4486eafd9cc6cf03cfb60178ea874e9699aec94ebf2702ba50d142c4dd"),
    ("____21_________________________1___________________________________", "699d135e39d53a0be633f69b01664ca552deaa1a00f913948a14efc68bf1f612"),
    ("____21_________________________2___________________________________", "82565a28c165f5c6be4c0e06e7f112fa8526ac7400da7de97890aa121523682b"),
    ("____21_________________________3___________________________________", "ffd2e88afa3eca29e614372fc5b41851a5de403bd273e8ce8dfc676abce1e4d7"),
    ("____31_________________________0___________________________________", "22a99d5c82795cf266ff600aada7c8701c9f5a305e80c243654e59de129c7bfb"),
    ("____31_________________________1___________________________________", "608255a9f6b55202f08c5ea5374ee9a937ebb5d1387d77ce1681918d4964ead1"),
    ("____31_________________________2___________________________________", "07d069f6cf0f63b6bac8c9d665d8cd39779f1dae8215896313e038b70ab34f73"),
    ("____31_________________________3___________________________________", "a198394dfdda63b846b1a9e7e69504e461cd0d29590043d11384c010ea10576a"),
];

/// A scene drawn with 0.69d's cloud shadows: its marker and its environment's values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudShadowProfile {
    /// The scene's `mie_fog_base_height`.
    pub marker: f32,
    /// The direction sunlight travels (0.69d's, the same axes as the port's world).
    pub light: [f32; 3],
    pub altitude_km: f32,
    pub opacity: f32,
    pub power: f32,
    /// `clouds_rounds_per_minute / 60`: turns of the panorama a second.
    pub rate: f32,
    /// `sky_clouds_rotation / 360`.
    pub phase: f32,
}

/// 0.69d's cloud shadow for one profile, its registers `t`.. `t + 3`, the view position
/// in `view`, the light it lets through into `sky`.x.
fn branch(c: &CloudShadowProfile, t: u32, view: u32, sky: u32) -> Vec<Vec<u32>> {
    vec![
        vec![0x08000018, 0x00100012, t + 3, 0x0020803a, 0x00000001, 0x00000012, 0x00004001, c.marker.to_bits()], // eq r103.x, cb1[18].w, l({MARK})
        vec![0x0304001f, 0x0010000a, t + 3], // if_nz r103.x
        vec![0x08000011, 0x00100042, t + 1, 0x00208e46, 0x00000001, 0x0000000b, 0x00100e46, view], // dp4 r101.z, cb1[11].xyzw, r90.xyzw
        vec![0x08000011, 0x00100012, t + 1, 0x00208e46, 0x00000001, 0x0000000c, 0x00100e46, view], // dp4 r101.x, cb1[12].xyzw, r90.xyzw
        vec![0x08000011, 0x00100022, t + 1, 0x00208e46, 0x00000001, 0x0000000d, 0x00100e46, view], // dp4 r101.y, cb1[13].xyzw, r90.xyzw
        vec![0x0a000032, 0x00100012, t + 3, 0x0020800a, 0x00000001, 0x00000019, 0x00004001, c.rate.to_bits(), 0x00004001, c.phase.to_bits()], // mad r103.x, cb1[25].x, l({RATE}), l({PHASE})
        vec![0x0500001a, 0x00100012, t + 3, 0x0010000a, t + 3], // frc r103.x, r103.x
        vec![0x0a000038, 0x00100072, t + 0, 0x00100246, t + 1, 0x00004002, 0x3a83126f, 0x3a83126f, 0x3a83126f, 0x00000000], // mul r100.xyz, r101.xyzx, l(0x3a83126f, 0x3a83126f, 0x3a83126f, 0x0)
        vec![0x0f000032, 0x00100072, t + 1, 0x00100926, t + 1, 0x00004002, 0x3a83126f, 0x3a83126f, 0x3a83126f, 0x00000000, 0x00004002, 0x00000000, 0x45c6c000, 0x00000000, 0x00000000], // mad r101.xyz, r101.zxyz, l(0x3a83126f, 0x3a83126f, 0x3a83126f, 0x0), l(0x0, 0x45c6c000, 0x0, 0x0)
        vec![0x07000000, 0x00100082, t + 0, 0x00004001, c.altitude_km.to_bits(), 0x00004001, 0x45c6c000], // add r100.w, l({ALT}), l(0x45c6c000)
        vec![0x07000010, 0x00100082, t + 1, 0x00100246, t + 1, 0x00100246, t + 1], // dp3 r101.w, r101.xyzx, r101.xyzx
        vec![0x07000038, 0x00100012, t + 2, 0x0010003a, t + 0, 0x0010003a, t + 0], // mul r102.x, r100.w, r100.w
        vec![0x0700001d, 0x00100012, t + 2, 0x0010003a, t + 1, 0x0010000a, t + 2], // ge r102.x, r101.w, r102.x
        vec![0x0304001f, 0x0010000a, t + 2], // if_nz r102.x
        vec![0x05000036, 0x00100012, sky, 0x00004001, 0x3f800000], // mov r91.x, l(0x3f800000)
        vec![0x01000012], // else
        vec![0x0a000010, 0x00100012, t + 1, 0x00100246, t + 1, 0x00004002, c.light[0].to_bits(), c.light[1].to_bits(), c.light[2].to_bits(), c.light[0].to_bits()], // dp3 r101.x, r101.xyzx, l({LX}, {LY}, {LZ}, {LX})
        vec![0x0a000032, 0x00100022, t + 1, 0x8010000a, 0x00000041, t + 1, 0x0010000a, t + 1, 0x0010003a, t + 1], // mad r101.y, -r101.x, r101.x, r101.w
        vec![0x0a000032, 0x00100082, t + 0, 0x0010003a, t + 0, 0x0010003a, t + 0, 0x8010001a, 0x00000041, t + 1], // mad r100.w, r100.w, r100.w, -r101.y
        vec![0x0500004b, 0x00100082, t + 0, 0x0010003a, t + 0], // sqrt r100.w, r100.w
        vec![0x07000000, 0x00100082, t + 0, 0x0010003a, t + 0, 0x0010000a, t + 1], // add r100.w, r100.w, r101.x
        vec![0x0d000032, 0x00100072, t + 0, 0x80100ff6, 0x00000041, t + 0, 0x00004002, c.light[1].to_bits(), c.light[2].to_bits(), c.light[0].to_bits(), c.light[1].to_bits(), 0x00100246, t + 0], // mad r100.xyz, -r100.wwww, l({LY}, {LZ}, {LX}, {LY}), r100.xyzx
        vec![0x07000010, 0x00100082, t + 0, 0x00100246, t + 0, 0x00100246, t + 0], // dp3 r100.w, r100.xyzx, r100.xyzx
        vec![0x05000044, 0x00100082, t + 0, 0x0010003a, t + 0], // rsq r100.w, r100.w
        vec![0x07000038, 0x00100072, t + 0, 0x00100ff6, t + 0, 0x00100246, t + 0], // mul r100.xyz, r100.wwww, r100.xyzx
        vec![0x08000000, 0x00100082, t + 0, 0x8010000a, 0x000000c1, t + 0, 0x00004001, 0x3f800000], // add r100.w, -|r100.x|, l(0x3f800000)
        vec![0x0500004b, 0x00100082, t + 0, 0x0010003a, t + 0], // sqrt r100.w, r100.w
        vec![0x0a000032, 0x00100012, t + 1, 0x8010000a, 0x00000081, t + 0, 0x00004001, 0xbc996e30, 0x00004001, 0x3d981627], // mad r101.x, |r100.x|, l(0xbc996e30), l(0x3d981627)
        vec![0x0a000032, 0x00100012, t + 1, 0x0010000a, t + 1, 0x8010000a, 0x00000081, t + 0, 0x00004001, 0xbe593484], // mad r101.x, r101.x, |r100.x|, l(0xbe593484)
        vec![0x0a000032, 0x00100012, t + 1, 0x0010000a, t + 1, 0x8010000a, 0x00000081, t + 0, 0x00004001, 0x3fc90da4], // mad r101.x, r101.x, |r100.x|, l(0x3fc90da4)
        vec![0x07000038, 0x00100022, t + 1, 0x0010003a, t + 0, 0x0010000a, t + 1], // mul r101.y, r100.w, r101.x
        vec![0x09000032, 0x00100022, t + 1, 0x0010001a, t + 1, 0x00004001, 0xc0000000, 0x00004001, 0x40490fdb], // mad r101.y, r101.y, l(0xc0000000), l(0x40490fdb)
        vec![0x08000031, 0x001000c2, t + 1, 0x00100806, t + 0, 0x80100806, 0x00000041, t + 0], // lt r101.zw, r100.xxxz, -r100.xxxz
        vec![0x07000001, 0x00100012, t + 0, 0x0010002a, t + 1, 0x0010001a, t + 1], // and r100.x, r101.z, r101.y
        vec![0x09000032, 0x00100012, t + 0, 0x0010000a, t + 1, 0x0010003a, t + 0, 0x0010000a, t + 0], // mad r100.x, r101.x, r100.w, r100.x
        vec![0x09000033, 0x00100082, t + 0, 0x8010002a, 0x00000081, t + 0, 0x8010001a, 0x00000081, t + 0], // min r100.w, |r100.z|, |r100.y|
        vec![0x09000034, 0x00100012, t + 1, 0x8010002a, 0x00000081, t + 0, 0x8010001a, 0x00000081, t + 0], // max r101.x, |r100.z|, |r100.y|
        vec![0x0a00000e, 0x00100012, t + 1, 0x00004002, 0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000, 0x0010000a, t + 1], // div r101.x, l(0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000), r101.x
        vec![0x07000038, 0x00100082, t + 0, 0x0010003a, t + 0, 0x0010000a, t + 1], // mul r100.w, r100.w, r101.x
        vec![0x07000038, 0x00100012, t + 1, 0x0010003a, t + 0, 0x0010003a, t + 0], // mul r101.x, r100.w, r100.w
        vec![0x09000032, 0x00100022, t + 1, 0x0010000a, t + 1, 0x00004001, 0x3caaae5f, 0x00004001, 0xbdae5a36], // mad r101.y, r101.x, l(0x3caaae5f), l(0xbdae5a36)
        vec![0x09000032, 0x00100022, t + 1, 0x0010000a, t + 1, 0x0010001a, t + 1, 0x00004001, 0x3e3876e2], // mad r101.y, r101.x, r101.y, l(0x3e3876e2)
        vec![0x09000032, 0x00100022, t + 1, 0x0010000a, t + 1, 0x0010001a, t + 1, 0x00004001, 0xbea91d04], // mad r101.y, r101.x, r101.y, l(0xbea91d04)
        vec![0x09000032, 0x00100012, t + 1, 0x0010000a, t + 1, 0x0010001a, t + 1, 0x00004001, 0x3f7ff738], // mad r101.x, r101.x, r101.y, l(0x3f7ff738)
        vec![0x07000038, 0x00100022, t + 1, 0x0010003a, t + 0, 0x0010000a, t + 1], // mul r101.y, r100.w, r101.x
        vec![0x09000031, 0x00100042, t + 1, 0x8010002a, 0x00000081, t + 0, 0x8010001a, 0x00000081, t + 0], // lt r101.z, |r100.z|, |r100.y|
        vec![0x09000032, 0x00100022, t + 1, 0x0010001a, t + 1, 0x00004001, 0xc0000000, 0x00004001, 0x3fc90fdb], // mad r101.y, r101.y, l(0xc0000000), l(0x3fc90fdb)
        vec![0x07000001, 0x00100022, t + 1, 0x0010002a, t + 1, 0x0010001a, t + 1], // and r101.y, r101.z, r101.y
        vec![0x09000032, 0x00100082, t + 0, 0x0010003a, t + 0, 0x0010000a, t + 1, 0x0010001a, t + 1], // mad r100.w, r100.w, r101.x, r101.y
        vec![0x07000001, 0x00100012, t + 1, 0x0010003a, t + 1, 0x00004001, 0xc0490fdb], // and r101.x, r101.w, l(0xc0490fdb)
        vec![0x07000000, 0x00100082, t + 0, 0x0010003a, t + 0, 0x0010000a, t + 1], // add r100.w, r100.w, r101.x
        vec![0x07000033, 0x00100012, t + 1, 0x0010002a, t + 0, 0x0010001a, t + 0], // min r101.x, r100.z, r100.y
        vec![0x07000034, 0x00100022, t + 0, 0x0010002a, t + 0, 0x0010001a, t + 0], // max r100.y, r100.z, r100.y
        vec![0x08000031, 0x00100042, t + 0, 0x0010000a, t + 1, 0x8010000a, 0x00000041, t + 1], // lt r100.z, r101.x, -r101.x
        vec![0x0800001d, 0x00100022, t + 0, 0x0010001a, t + 0, 0x8010001a, 0x00000041, t + 0], // ge r100.y, r100.y, -r100.y
        vec![0x07000001, 0x00100022, t + 0, 0x0010001a, t + 0, 0x0010002a, t + 0], // and r100.y, r100.y, r100.z
        vec![0x0a000037, 0x00100022, t + 0, 0x0010001a, t + 0, 0x8010003a, 0x00000041, t + 0, 0x0010003a, t + 0], // movc r100.y, r100.y, -r100.w, r100.w
        vec![0x09000032, 0x00100012, t + 1, 0x0010001a, t + 0, 0x00004001, 0x3e22f983, 0x0010000a, t + 3], // mad r101.x, r100.y, l(0x3e22f983), r103.x
        vec![0x07000038, 0x00100022, t + 1, 0x0010000a, t + 0, 0x00004001, 0x3f22f983], // mul r101.y, r100.x, l(0x3f22f983)
        vec![0x0b000048, 0x001000f2, t + 0, 0x00100046, t + 1, 0x00107e46, 0x00000003, 0x00106000, 0x00000001, 0x00004001, 0x00000000], // sample_l r100.xyzw, r101.xyxx, t3.xyzw, s1, l(0x0)
        vec![0x07000038, 0x00100012, t + 0, 0x0010003a, t + 0, 0x00004001, c.opacity.to_bits()], // mul r100.x, r100.w, l({OP})
        vec![0x0500002f, 0x00100012, t + 0, 0x0010000a, t + 0], // log r100.x, r100.x
        vec![0x07000038, 0x00100012, t + 0, 0x0010000a, t + 0, 0x00004001, c.power.to_bits()], // mul r100.x, r100.x, l({PW})
        vec![0x05000019, 0x00100012, t + 0, 0x0010000a, t + 0], // exp r100.x, r100.x
        vec![0x07000033, 0x00100012, t + 0, 0x0010000a, t + 0, 0x00004001, 0x3f800000], // min r100.x, r100.x, l(0x3f800000)
        vec![0x08000000, 0x00100012, sky, 0x8010000a, 0x00000041, t + 0, 0x00004001, 0x3f800000], // add r91.x, -r100.x, l(0x3f800000)
        vec![0x01000015], // endif
        vec![0x01000015], // endif
    ]
}

/// A `sun_shadow.ps` variant drawing each profile's scene with 0.69d's cloud shadows.
pub fn build_cloud_shadow(target: &[u8], variant: &str, profiles: &[CloudShadowProfile]) -> Result<Vec<u8>> {
    ensure!(!profiles.is_empty(), "no cloud shadow profiles");
    for p in profiles {
        ensure!(
            p.marker.is_finite() && p.light.iter().all(|v| v.is_finite()) && p.light[1] < 0.0
                && p.altitude_km > 0.0 && p.opacity >= 0.0 && p.power > 0.0 && p.rate.is_finite() && p.phase.is_finite(),
            "invalid cloud shadow profile {p:?}"
        );
    }
    let hash = SKY_SHADOW_VARIANTS.iter().find(|(name, _)| *name == variant).context("not a sky shadow variant")?.1;
    let (offset, native) = pinned_code(target, hash)?;
    let mut entries = chunks(native)?;
    let program = entries.iter_mut().find(|(t, _)| t == b"SHDR").context("missing sun shadow program")?;
    let native_words = words(&program.1)?;
    let ops = instructions(&native_words)?;
    let spliced = splice(&ops, profiles)?;
    let mut updated = native_words[..2].to_vec();
    updated.extend(spliced.iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    program.1 = bytes(&updated);
    let reflection = entries.iter_mut().find(|(t, _)| t == b"RDEF").context("missing sun shadow reflection")?;
    mark_used(&mut reflection.1, "static_globals", "atmosphere_parameters3", 288, 16)?;
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

/// The native instructions with each profile's branch after the sample of
/// `sky_shadows_texture` (t3), four more temporaries.
fn splice(ops: &[Vec<u32>], profiles: &[CloudShadowProfile]) -> Result<Vec<Vec<u32>>> {
    let temps_at = ops.iter().position(|i| i[0] & 2047 == 0x68).context("no temporaries")?;
    let t = ops[temps_at][1];
    // sample rX.xyzw, rY.xyxx, t3.xyzw, s1
    let sample = ops
        .iter()
        .position(|i| i[0] & 2047 == 0x45 && i.windows(2).any(|w| w == [0x00107e46, 3]))
        .context("no sample of sky_shadows_texture")?;
    ensure!(ops.iter().filter(|i| i[0] & 2047 == 0x45 && i.windows(2).any(|w| w == [0x00107e46, 3])).count() == 1, "two samples of t3");
    ensure!(ops[sample][1] == 0x001000f2, "the sky sample's destination changed");
    let sky = ops[sample][2];
    // dp4 rA.x, cb1[11].xyzw, rV.xyzw: the world x of the view position rV
    let dp4 = ops[..sample]
        .iter()
        .rposition(|i| i[0] & 2047 == 0x11 && i.len() == 8 && i[3..6] == [0x00208e46, 1, 11] && i[6] == 0x00100e46)
        .context("no world position of the view")?;
    let view = ops[dp4][7];
    // nothing between them writes rV
    for i in &ops[dp4 + 1..sample] {
        ensure!(!(i.len() > 2 && i[1] & 0x000ff000 == 0 && (i[1] >> 20) & 3 == 1 && i[2] == view), "the view position is overwritten before the sample");
    }
    let mut out = ops[..temps_at].to_vec();
    out.push(vec![0x02000068, t + 4]);
    out.extend_from_slice(&ops[temps_at + 1..=sample]);
    for p in profiles {
        out.extend(branch(p, t, view, sky));
    }
    out.extend_from_slice(&ops[sample + 1..]);
    Ok(out)
}

/// Marks a constant of a buffer used (the game binds what the reflection says is used).
fn mark_used(rdef: &mut [u8], buffer: &str, variable: &str, offset: u32, size: u32) -> Result<()> {
    let string = |data: &[u8], at: usize| -> Result<String> {
        let tail = data.get(at..).context("missing reflection string")?;
        let len = tail.iter().position(|b| *b == 0).context("unterminated reflection string")?;
        Ok(std::str::from_utf8(&tail[..len])?.to_owned())
    };
    let count = u32_at(rdef, 0)? as usize;
    let buffers = u32_at(rdef, 4)? as usize;
    for i in 0..count {
        let at = buffers + i * 24;
        if string(rdef, u32_at(rdef, at)? as usize)? != buffer {
            continue;
        }
        let vars = u32_at(rdef, at + 4)? as usize;
        let start = u32_at(rdef, at + 8)? as usize;
        for j in 0..vars {
            let at = start + j * 24;
            if string(rdef, u32_at(rdef, at)? as usize)? == variable {
                ensure!(u32_at(rdef, at + 4)? == offset && u32_at(rdef, at + 8)? == size, "{variable} moved");
                rdef[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
                return Ok(());
            }
        }
    }
    anyhow::bail!("no {buffer}.{variable}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUDNYA: CloudShadowProfile = CloudShadowProfile {
        marker: 33001.0,
        light: [0.5, -0.866_025_4, 0.0],
        altitude_km: 4.0,
        opacity: 1.5,
        power: 1.0,
        rate: 0.001,
        phase: 0.0,
    };

    /// A native-shaped program: temporaries, the view's world x, the sky sample, its use.
    fn native() -> Vec<Vec<u32>> {
        vec![
            vec![0x02000068, 5],
            vec![0x08000011, 0x00100012, 0, 0x00208e46, 1, 11, 0x00100e46, 1], // dp4 r0.x, cb1[11].xyzw, r1.xyzw
            vec![0x09000045, 0x001000f2, 3, 0x00100046, 0, 0x00107e46, 3, 0x00106000, 1], // sample r3.xyzw, r0.xyxx, t3.xyzw, s1
            vec![0x07000033, 0x00102012, 0, 0x0010002a, 0, 0x0010000a, 3], // min o0.x, r0.z, r3.x
            vec![0x0100003e],
        ]
    }

    #[test]
    fn each_profile_runs_after_the_sky_sample_on_fresh_temporaries() {
        let out = splice(&native(), &[RUDNYA]).unwrap();
        assert_eq!(out[0], vec![0x02000068, 9]);
        assert_eq!(out[2], native()[2]);
        let b = branch(&RUDNYA, 5, 1, 3);
        assert_eq!(&out[3..3 + b.len()], &b[..]);
        assert_eq!(&out[3 + b.len()..], &native()[3..]);
        // the marker test, the view position read from r1, the result into r3.x, temporaries 5..8
        assert_eq!(b[0][2], 8);
        assert_eq!(b[0][7], 33001f32.to_bits());
        assert!(b.iter().any(|i| i.ends_with(&[0x00100e46, 1])));
        assert!(b.iter().any(|i| i[0] & 2047 == 0x00 && i[1] == 0x00100012 && i[2] == 3));
        assert!(b.iter().flat_map(|i| i.iter()).all(|w| *w != 100 && *w != 101 && *w != 102 && *w != 103));
        // two profiles, two branches
        let two = splice(&native(), &[RUDNYA, CloudShadowProfile { marker: 33002.0, ..RUDNYA }]).unwrap();
        assert_eq!(two.len(), out.len() + b.len());
    }

    #[test]
    fn a_view_position_written_before_the_sample_is_refused() {
        let mut ops = native();
        ops.insert(2, vec![0x05000036, 0x00100012, 1, 0x00004001, 0]); // mov r1.x, l(0)
        assert!(splice(&ops, &[RUDNYA]).is_err());
    }
}
