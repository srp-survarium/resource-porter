// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's exposure, tone curve and colour grading for a 0.26g scene.
//!
//! 0.69d (exe and `tone_mapping.ps`, read from both):
//! * Exposure. Lights are drawn pre-exposed by `E = 1 / (1.2 * 2^EV)`; each
//!   frame `EV = clamp(log2(L) + 3 + bias, min, max)` of the view's metered
//!   luminance `L` (cd/m^2, a weighted log-average), so a view inside the
//!   range is exposed to a mean of `1 / (1.2 * 8)` and one outside it keeps
//!   the exposure of the range's end.
//! * Colour grading (`use_color_grading`), on the exposed colour in AP1:
//!   shadows, midtones and highlights each take saturation about the luma,
//!   contrast and gamma about 0.18, gain and offset, and are mixed by the
//!   luma (`shadows_max`, `highlights_min`).
//! * The curve `y = x^a / (x^d * b + c)` (`Curve`), then the sRGB encode.
//!
//! 0.26g (`complex_post_process_blend.ps`): the curve `1 - 2^(-x * A)` of
//! the colour times `(k / L)^0.2` to the power 1.2, `A = 4.5 / (10 clamp(L,
//! 0.05, 2) + 1)`, then a 16^3 colour grading texture looked up by the sRGB
//! encoded result. The curve's `exp` is the shader model's (sm_4_0 `exp` is
//! `2^x`, `log` is `log2`): no `log2(e)` scales its argument.
//!
//! So a 0.69d scene takes two things. An exposure profile (`Exposure`): the
//! native tone block with the mean clamped to 0.69d's range, which
//! resource-porter's lighting pack puts behind the scene's marker (its
//! `mie_fog_base_height`). And its own grading texture (`Tone::lut`): for
//! each colour 0.26g's curve can show, what 0.69d's grading and curve show
//! for the light that gives it.
use crate::ConfigValue;
use anyhow::{Context, Result, ensure};

/// `y = x^a / (x^d * b + c)`, 0.69d's tone curve (`tone_mapping.ps`
/// constants `m_material_parameters_0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub a: f32,
    pub d: f32,
    pub b: f32,
    pub c: f32,
}

impl Curve {
    /// From the environment's `tone_mapping_*` properties, as the exe packs
    /// them (0x200556ac0): with `white_clip` 0 the curve takes `mid_in` to
    /// `mid_out` and `white_point` to 1; `white_clip` moves `b`.
    pub fn new(contrast: f32, shoulder: f32, mid_in: f32, mid_out: f32, white_point: f32, white_clip: f32) -> Self {
        let a = contrast.clamp(0.5, 2.0);
        let d = a * (0.9 + 0.1 * shoulder.clamp(0.0, 1.0));
        let (mi, mo, hm) = (mid_in.clamp(0.001, 10.0), mid_out.clamp(0.001, 10.0), white_point.clamp(1.0, 50.0));
        let den = (hm.powf(d) - mi.powf(d)) * mo;
        Self {
            a,
            d,
            b: (hm.powf(a) * mo - mi.powf(a)) / den - white_clip.clamp(-5.0, 0.5),
            c: (mi.powf(a) * hm.powf(d) - hm.powf(a) * mo * mi.powf(d)) / den,
        }
    }

    pub fn shown(&self, x: f32) -> f32 {
        let x = x.max(0.0);
        x.powf(self.a) / (x.powf(self.d) * self.b + self.c)
    }

    /// The light the curve shows as white (1), at most `limit`.
    fn white(&self, limit: f32) -> f32 {
        if self.shown(limit) < 1.0 {
            return limit;
        }
        let (mut lo, mut hi) = (0.0, limit);
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            if self.shown(mid) < 1.0 { lo = mid } else { hi = mid }
        }
        hi
    }
}

/// One range's grading constants per channel (`tone_mapping.ps`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub saturation: [f32; 3],
    pub contrast: [f32; 3],
    /// The exponent the shader applies (the reciprocal of the gamma).
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub offset: f32,
}

impl Range {
    fn graded(&self, luma: f32, chroma: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| {
            let v = (luma + chroma[i] * self.saturation[i]).max(0.0) * 5.555_555_5;
            (v.powf(self.contrast[i]) * 0.18).powf(self.gamma[i]) * self.gain[i] + self.offset
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grading {
    pub shadows: Range,
    pub midtones: Range,
    pub highlights: Range,
    pub shadows_max: f32,
    pub highlights_min: f32,
}

const SRGB_TO_AP1: [[f32; 3]; 3] = [[0.613_191, 0.339_512, 0.047_366], [0.070_207, 0.916_336, 0.013_450], [0.020_619, 0.109_567, 0.869_607]];
const AP1_TO_SRGB: [[f32; 3]; 3] = [[1.699_52, -0.621_791, -0.083_258], [-0.129_817, 1.140_8, -0.010_549], [-0.011_258, -0.128_969, 1.152_97]];
const AP1_LUMA: [f32; 3] = [0.272_229, 0.674_082, 0.053_690];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Grading {
    pub fn graded(&self, rgb: [f32; 3]) -> [f32; 3] {
        let ap1 = SRGB_TO_AP1.map(|row| dot(row, rgb));
        let luma = dot(ap1, AP1_LUMA);
        let chroma = ap1.map(|v| v - luma);
        let in_shadows = 1.0 - smoothstep(luma / self.shadows_max);
        let in_highlights = smoothstep((luma - self.highlights_min) / (1.0 - self.highlights_min));
        let in_midtones = 1.0 - in_shadows - in_highlights;
        let (s, m, h) = (self.shadows.graded(luma, chroma), self.midtones.graded(luma, chroma), self.highlights.graded(luma, chroma));
        let mixed: [f32; 3] = std::array::from_fn(|i| s[i] * in_shadows + h[i] * in_highlights + m[i] * in_midtones);
        AP1_TO_SRGB.map(|row| dot(row, mixed))
    }
}

/// A 0.69d environment's exposure range, grading and curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub min_ev: f32,
    pub max_ev: f32,
    /// `auto_exposure_bias` (EV added: positive is darker).
    pub bias: f32,
    pub grading: Option<Grading>,
    pub curve: Curve,
}

/// The native tone block's constants for a 0.69d scene (resource-porter's
/// lighting pack, `ExposureProfile`), in 0.26 values at the light scale of
/// the scene's maximum exposure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exposure {
    /// The scene's `mie_fog_base_height`, which selects the profile.
    pub marker: f32,
    /// The adapted mean below which the exposure stays (0.69d's minimum EV).
    pub floor: f32,
    /// The mean above which it stays (its maximum EV).
    pub ceiling: f32,
    /// The exposure key: the light 0.69d shows as white lands at 0.98 of
    /// 0.26g's curve, so every colour below it keeps a place in the grading
    /// texture.
    pub key: f32,
}

/// 0.26g's `k(L) = 1.03 - 0.1 / (2 + log10(L + 1))`.
fn native_k(mean: f32) -> f32 {
    1.03 - 0.1 / (2.0 + (mean + 1.0).log10())
}

impl Exposure {
    /// The profile's gain on a colour at an adapted mean at or above the
    /// ceiling, and its curve's `A` there.
    fn at_ceiling(&self) -> (f32, f32) {
        ((native_k(self.ceiling) / self.ceiling).powf(0.2), self.key / (10.0 * self.ceiling))
    }

    /// What 0.26g's curve shows (linear, before the grading texture) for a
    /// light `x` at the ceiling: `1 - 2^(-(x * gain)^1.2 * a)`.
    pub fn shown(&self, x: f32) -> f32 {
        let (gain, a) = self.at_ceiling();
        1.0 - (-(x.max(0.0) * gain).powf(1.2) * a).exp2()
    }

    /// The light that 0.26g's curve shows as `shown`.
    fn light(&self, shown: f32) -> f32 {
        let (gain, a) = self.at_ceiling();
        (-(1.0 - shown.clamp(0.0, 0.9999)).log2() / a).powf(1.0 / 1.2) / gain
    }
}

/// A float4 property and its master slider (`.w`), or the engine's default.
fn slider(environment: &ConfigValue, name: &str, default_w: f32) -> [f32; 4] {
    match environment.named(name).map(|v| v.named("value").unwrap_or(v)) {
        Some(ConfigValue::Float4(v)) => *v,
        _ => [1.0, 1.0, 1.0, default_w],
    }
}

fn number(environment: &ConfigValue, name: &str) -> Option<f32> {
    match environment.named(name).map(|v| v.named("value").unwrap_or(v)) {
        Some(ConfigValue::Float(v)) => Some(*v),
        Some(ConfigValue::Integer(v)) => Some(*v as f32),
        _ => None,
    }
}

impl Tone {
    /// From a 0.69d `environment` object. Absent properties take the engine's
    /// defaults (its constructor, 0x20017bd50).
    pub fn read(environment: &ConfigValue) -> Result<Self> {
        let n = |name: &str, default: f32| number(environment, name).unwrap_or(default);
        let grading = matches!(environment.named("use_color_grading"), Some(ConfigValue::Boolean(true))).then(|| {
            // The exe multiplies each range's property by the global one
            // (0x20051fd00): saturation and gain as 2 w rgb, contrast as
            // (w + 0.5) rgb, gamma as (w + 1) rgb and then its reciprocal.
            let range = |name: &str| {
                let both = |what: &str, default_w: f32, f: fn(f32) -> f32| -> [f32; 3] {
                    let (g, r) = (slider(environment, &format!("global_{what}"), default_w), slider(environment, &format!("{name}_{what}"), default_w));
                    std::array::from_fn(|i| f(g[3]) * g[i] * f(r[3]) * r[i])
                };
                Range {
                    saturation: both("saturation", 0.5, |w| 2.0 * w),
                    contrast: both("contrast", 0.5, |w| w + 0.5),
                    gamma: both("gamma", 0.0, |w| w + 1.0).map(|v| 1.0 / v),
                    gain: both("gain", 0.5, |w| 2.0 * w),
                    offset: n("global_offset", 0.0) + n(&format!("{name}_offset"), 0.0),
                }
            };
            Grading { shadows: range("shadows"), midtones: range("midtones"), highlights: range("highlights"), shadows_max: n("shadows_max", 0.05), highlights_min: n("highlights_min", 0.75) }
        });
        let tone = Self {
            min_ev: number(environment, "min_auto_exposure_value").context("min_auto_exposure_value")?,
            max_ev: number(environment, "max_auto_exposure_value").context("max_auto_exposure_value")?,
            bias: n("auto_exposure_bias", 0.0),
            grading,
            curve: Curve::new(
                n("tone_mapping_contrast", 1.2),
                n("tone_mapping_shoulder", 1.0),
                n("tone_mapping_mid_in", 0.18),
                n("tone_mapping_mid_out", 0.18),
                n("tone_mapping_white_point", 20.0),
                n("tone_mapping_white_clip", 0.0),
            ),
        };
        // Equal ends are a fixed exposure (0.69d's lobby: EV 9.5).
        ensure!(tone.min_ev <= tone.max_ev, "the exposure range is empty");
        Ok(tone)
    }

    /// What 0.69d shows (linear, before its sRGB encode) for an exposed
    /// colour.
    pub fn shown(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.grading.map_or(rgb, |g| g.graded(rgb)).map(|v| self.curve.shown(v))
    }

    /// The exposure profile of the scene. 0.26 values are 0.69d's exposed
    /// values at its maximum EV (`LightScale`), where a view of mean
    /// luminance `2^(max - 3 - bias)` sits: its mean there is
    /// `2^(-3 - bias) / 1.2`, the ceiling; the floor is the same mean at the
    /// minimum EV.
    pub fn exposure(&self, marker: f32) -> Exposure {
        let ceiling = 2f32.powf(-3.0 - self.bias) / 1.2;
        let floor = ceiling * 2f32.powf(self.min_ev - self.max_ev);
        let mut exposure = Exposure { marker, floor, ceiling, key: 1.0 };
        let (gain, _) = exposure.at_ceiling();
        let white = self.curve.white(16.0);
        exposure.key = -(0.02f32).log2() * 10.0 * ceiling / (white * gain).powf(1.2);
        exposure
    }

    /// The scene's colour grading texture, on a native one's header
    /// (`like`: 16^3 BGRA with two more levels, as every 0.26g one). Texel
    /// (r, g, b) is what 0.69d shows for the light 0.26g's curve shows as
    /// that colour under `exposure`, both sRGB encoded as
    /// `complex_post_process_blend.ps` reads and writes them.
    pub fn lut(&self, exposure: &Exposure, like: &[u8]) -> Result<Vec<u8>> {
        const SIDE: usize = 16;
        const LEVELS: [usize; 3] = [16, 8, 4];
        let texels: usize = LEVELS.iter().map(|s| s * s * SIDE * 4).sum();
        let le = |at: usize| u32::from_le_bytes(like[at..at + 4].try_into().unwrap());
        ensure!(
            like.len() == 128 + texels + 1 && &like[..4] == b"DDS " && le(12) == 16 && le(16) == 16 && le(24) == 16 && le(28) == 3 && le(88) == 32 && le(92) == 0x00ff_0000,
            "the native colour grading texture is not 16^3 BGRA with three levels"
        );
        let decode = |v: f32| ((v + 0.055) / 1.055).max(0.0).powf(2.4);
        let encode = |v: f32| (v.max(0.0).powf(1.0 / 2.4) * 1.055 - 0.055).clamp(0.0, 1.0);
        // Black stays black: texel 0 also holds everything darker than it.
        let light = |i: usize| if i == 0 { 0.0 } else { exposure.light(decode(i as f32 / (SIDE - 1) as f32)) };
        let mut top = vec![[0f32; 3]; SIDE * SIDE * SIDE];
        for b in 0..SIDE {
            for g in 0..SIDE {
                for r in 0..SIDE {
                    top[(b * SIDE + g) * SIDE + r] = self.shown([light(r), light(g), light(b)]).map(encode);
                }
            }
        }
        let mut out = like[..128].to_vec();
        for side in LEVELS {
            // A smaller level averages the texels of the top one over red
            // and green; blue keeps its 16 slices, as the native textures.
            let step = SIDE / side;
            for b in 0..SIDE {
                for g in 0..side {
                    for r in 0..side {
                        let mut sum = [0f32; 3];
                        for dg in 0..step {
                            for dr in 0..step {
                                let t = top[(b * SIDE + g * step + dg) * SIDE + r * step + dr];
                                (0..3).for_each(|c| sum[c] += t[c]);
                            }
                        }
                        let byte = |c: usize| (sum[c] / (step * step) as f32 * 255.0).round() as u8;
                        out.extend([byte(2), byte(1), byte(0), 255]);
                    }
                }
            }
        }
        out.push(like[like.len() - 1]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rudnya() -> Tone {
        // 0.69d level_01's environment.
        let range = |sat: f32, con: f32, gam: [f32; 3], gain: f32| Range { saturation: [sat; 3], contrast: [con; 3], gamma: gam, gain: [gain; 3], offset: 0.0 };
        Tone {
            min_ev: 6.0,
            max_ev: 12.5,
            bias: 0.0,
            grading: Some(Grading {
                shadows: range(0.969, 0.946_725, [1.142_857, 1.084_599, 1.0], 0.9918),
                midtones: range(1.02, 0.971, [1.044_932, 1.027_749, 1.0], 0.95),
                highlights: range(1.02, 0.971, [1.0; 3], 0.95),
                shadows_max: 0.02,
                highlights_min: 0.75,
            }),
            curve: Curve::new(1.3, 0.0, 0.28, 0.18, 2.0, -0.5),
        }
    }

    #[test]
    fn the_curve_is_packed_as_the_exe_packs_it() {
        let c = rudnya().curve;
        for (got, want) in [c.a, c.d, c.b, c.c].iter().zip([1.3, 1.17, 1.191_747, 0.905_777]) {
            assert!((got - want).abs() < 2e-4, "{c:?}");
        }
        assert!((c.shown(0.28) - 0.1627).abs() < 2e-4 && (c.shown(1.0) - 0.4768).abs() < 2e-4 && (c.shown(2.0) - 0.6864).abs() < 2e-4);
        // Without a white clip the curve takes mid_in to mid_out and the white point to 1.
        let plain = Curve::new(1.2, 1.0, 0.18, 0.18, 20.0, 0.0);
        assert!((plain.shown(0.18) - 0.18).abs() < 1e-5 && (plain.shown(20.0) - 1.0).abs() < 1e-4);
        assert!((c.shown(c.white(16.0)) - 1.0).abs() < 1e-4 && (6.0..9.0).contains(&c.white(16.0)));
    }

    #[test]
    fn grading_constants_come_from_the_environment_as_the_exe_combines_them() {
        use crate::binary_config::NamedValue;
        let f4 = |name: &str, v: [f32; 4]| NamedValue { name: name.into(), value: ConfigValue::Float4(v) };
        let f = |name: &str, v: f32| NamedValue { name: name.into(), value: ConfigValue::Float(v) };
        let environment = ConfigValue::Named(vec![
            NamedValue { name: "use_color_grading".into(), value: ConfigValue::Boolean(true) },
            f4("global_saturation", [1.0, 1.0, 1.0, 0.51]),
            f4("global_contrast", [1.0, 1.0, 1.0, 0.471]),
            f4("global_gain", [1.0, 1.0, 1.0, 0.475]),
            f4("shadows_saturation", [1.0, 1.0, 1.0, 0.475]),
            f4("shadows_contrast", [1.0, 1.0, 1.0, 0.475]),
            f4("shadows_gamma", [0.875, 0.922, 1.0, 0.0]),
            f4("shadows_gain", [1.0, 1.0, 1.0, 0.522]),
            f4("midtones_gamma", [0.957, 0.973, 1.0, 0.0]),
            f("shadows_max", 0.02),
            f("min_auto_exposure_value", 6.0),
            f("max_auto_exposure_value", 12.5),
            f("tone_mapping_contrast", 1.3),
            f("tone_mapping_shoulder", 0.0),
            f("tone_mapping_mid_in", 0.28),
            f("tone_mapping_mid_out", 0.18),
            f("tone_mapping_white_point", 2.0),
            f("tone_mapping_white_clip", -0.5),
        ]);
        let (read, want) = (Tone::read(&environment).unwrap(), rudnya());
        let (g, w) = (read.grading.unwrap(), want.grading.unwrap());
        for (a, b) in [(g.shadows, w.shadows), (g.midtones, w.midtones), (g.highlights, w.highlights)] {
            for i in 0..3 {
                assert!((a.saturation[i] - b.saturation[i]).abs() < 1e-3 && (a.contrast[i] - b.contrast[i]).abs() < 1e-3, "{a:?} {b:?}");
                assert!((a.gamma[i] - b.gamma[i]).abs() < 1e-3 && (a.gain[i] - b.gain[i]).abs() < 1e-3, "{a:?} {b:?}");
            }
        }
        assert_eq!((g.shadows_max, g.highlights_min, read.curve), (0.02, 0.75, want.curve));
        // No grading without the switch.
        assert!(Tone::read(&ConfigValue::Named(vec![f("min_auto_exposure_value", 6.0), f("max_auto_exposure_value", 12.5)])).unwrap().grading.is_none());
    }

    #[test]
    fn the_exposure_profile_spans_the_environments_range() {
        let e = rudnya().exposure(33001.0);
        assert!((e.ceiling - 0.104_167).abs() < 1e-5 && (e.floor - e.ceiling / 90.51).abs() < 1e-5, "{e:?}");
        // 0.69d's white lands at 0.98 of 0.26g's curve; light and shown invert each other.
        let white = rudnya().curve.white(16.0);
        assert!((e.shown(white) - 0.98).abs() < 1e-3, "{e:?}");
        for x in [0.01, 0.28, 2.0] {
            assert!((e.light(e.shown(x)) / x - 1.0).abs() < 1e-3);
        }
        // A positive bias darkens: the same mean sits one stop lower.
        let biased = Tone { bias: 1.0, ..rudnya() }.exposure(33001.0);
        assert!((biased.ceiling / e.ceiling - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_curve_is_the_programs_with_its_exp_base_two() {
        // The profile's last three instructions of complex_post_process_blend.ps:
        //   mul r0.xyz, r0.xyzx, -r1.xxxx   (r1.x = key / (10 Lc))
        //   exp r0.xyz, r0.xyzx             (sm_4_0 `exp`: 2^x)
        //   add r0.xyz, -r0.xyzx, l(1.0)
        // on (x * gain)^1.2, evaluated as the shader model defines them.
        let sm4_exp = |v: f32| 2f32.powf(v);
        let e = rudnya().exposure(33001.0);
        let (gain, a) = e.at_ceiling();
        for x in [0.05, 0.28, 1.0, 4.0] {
            let program = 1.0 - sm4_exp(-(x * gain).powf(1.2) * a);
            assert!((e.shown(x) - program).abs() < 1e-6, "{x}");
        }
        // Rudnya v0.69's key (the lighting pack's exposure_profiles row).
        assert!((e.key - 0.3367).abs() < 5e-4, "{e:?}");
    }

    #[test]
    fn the_grading_texture_shows_what_the_donor_shows() {
        let tone = rudnya();
        let e = tone.exposure(33001.0);
        let mut like = vec![0u8; 128];
        like[..4].copy_from_slice(b"DDS ");
        for (at, v) in [(12, 16u32), (16, 16), (24, 16), (28, 3), (88, 32), (92, 0x00ff_0000)] {
            like[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        like.resize(128 + 16 * 16 * 16 * 4 + 8 * 8 * 16 * 4 + 4 * 4 * 16 * 4, 0);
        like.push(7);
        let lut = tone.lut(&e, &like).unwrap();
        assert_eq!((lut.len(), lut[lut.len() - 1], &lut[..128]), (like.len(), 7, &like[..128]));
        let texel = |r: usize, g: usize, b: usize| -> [f32; 3] {
            let at = 128 + ((b * 16 + g) * 16 + r) * 4;
            [lut[at + 2], lut[at + 1], lut[at]].map(|v| v as f32 / 255.0)
        };
        assert_eq!(texel(0, 0, 0), [0.0; 3]);
        // A grey texel: decode as the shader does, and compare with 0.69d's own pipeline for that light.
        let decode = |v: f32| ((v + 0.055) / 1.055).powf(2.4);
        for i in [3usize, 8, 12, 15] {
            let x = e.light(decode(i as f32 / 15.0));
            let want = tone.shown([x; 3]);
            let got = texel(i, i, i).map(decode);
            for c in 0..3 {
                assert!((got[c] - want[c].min(1.0)).abs() < 0.01, "{i}: {got:?} {want:?}");
            }
        }
        // A header of another shape is refused.
        like[12] = 32;
        assert!(tone.lut(&e, &like).is_err());
    }
}
