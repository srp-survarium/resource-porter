// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d light in 0.26g's units: environment probes, the sun, ambient lights.
//!
//! 0.69d lights in physical units under an exposure; 0.26 in unitless linear
//! values its eye adaptation maps to the screen. One scale (`LightScale`)
//! takes 0.69d scene luminance to 0.26 linear values, fixed by the sun, so
//! the sun, the probes and the fill lights keep 0.69d's proportions.
//!
//! Probe encodings (shader disassembly, both builds):
//! * 0.69d (`copy_probe_lighting.ps` writes, `environment_probe.ps` reads):
//!   `m = a / (1 - a)`, `L = (rgb * m)^2` in capture units; scene luminance is
//!   `L * 1.2 * 2^capture_exposure_value`. Diffuse light comes from nine SH
//!   coefficients per probe (`sh_eval`), in the same capture units.
//! * 0.26 (`environment_probe_lighting.ps` reads, `fill_environment_probe_*`
//!   writes): `L = 0.25 * (rgb / a)^2`; written as `e = 4 L`,
//!   `m = max(1, max(e))`, `rgb = sqrt(e / m)`, `a = sqrt(1 / m)`. The
//!   `_diffuse` cube holds irradiance `E = integral of L cos` encoded the same
//!   way without the 4, so it decodes to `E / 4`, which the lighting
//!   multiplies by the albedo (and the sun's colour likewise, with no 1/pi).
use crate::dds_cube::{CubeRgba8, decode_cube, encode_cube};
use anyhow::{Result, ensure};

/// 0.69d scene luminance (its capture units times `1.2 * 2^EV`) to 0.26
/// linear radiance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightScale(pub f32);

/// A 0.69d environment probe's lighting data.
#[derive(Clone, Debug, PartialEq)]
pub struct DonorProbe {
    /// `textures/environment_probes/<texture>.dds`, the donor's cube map.
    pub texture: String,
    pub capture_exposure_value: f32,
    /// Weight of the SH bands 1-2 against band 0 (0.69d default 0.5).
    pub directional_factor: f32,
    pub sh: [[f32; 3]; 9],
}

impl DonorProbe {
    /// Capture units to 0.69d scene luminance.
    pub fn exposure(&self) -> f32 {
        1.2 * 2f32.powf(self.capture_exposure_value)
    }

    /// Outgoing radiance of a white Lambertian surface facing `n` (E / pi),
    /// in capture units, as `environment_probe.ps` evaluates it.
    pub fn sh_eval(&self, n: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = n;
        let c = &self.sh;
        std::array::from_fn(|k| {
            let l0 = 0.282_094_78 * c[0][k];
            let l12 = -0.325_735 * y * c[1][k] + 0.325_735 * z * c[2][k] - 0.325_735 * x * c[3][k]
                + 0.273_137 * x * y * c[4][k]
                - 0.273_137 * y * z * c[5][k]
                + 0.25 * (0.946_174_7 * z * z - 0.315_391_57) * c[6][k]
                - 0.273_137 * x * z * c[7][k]
                + 0.136_568_56 * (x * x - y * y) * c[8][k];
            (l0 + self.directional_factor * l12).max(0.0)
        })
    }
}

/// 0.69d probe texel to capture-unit radiance.
pub fn decode_069d(texel: [u8; 4]) -> [f32; 3] {
    let a = (texel[3] as f32 / 255.0).min(254.5 / 255.0);
    let m = a / (1.0 - a);
    std::array::from_fn(|k| (texel[k] as f32 / 255.0 * m).powi(2))
}

/// 0.26 probe texel from linear radiance (`scale` 4 for the specular cube,
/// 1 for the irradiance cube).
pub fn encode_026(value: [f32; 3], scale: f32) -> [u8; 4] {
    let e = value.map(|v| (v * scale).max(0.0));
    let m = e.iter().cloned().fold(1.0f32, f32::max);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte((e[0] / m).sqrt()), byte((e[1] / m).sqrt()), byte((e[2] / m).sqrt()), byte((1.0 / m).sqrt())]
}

/// 0.26 probe texel to linear radiance (`scale` as in `encode_026`).
pub fn decode_026(texel: [u8; 4], scale: f32) -> [f32; 3] {
    let a = (texel[3] as f32 / 255.0).max(1.0 / 255.0);
    std::array::from_fn(|k| (texel[k] as f32 / 255.0 / a).powi(2) / scale)
}

/// The direction through the centre of texel (`x`, `y`) of a `size` face
/// (D3D cube order +X -X +Y -Y +Z -Z), normalised.
pub fn cube_direction(face: usize, x: usize, y: usize, size: usize) -> [f32; 3] {
    let s = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    let t = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    let d = match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    };
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    d.map(|v| v / n)
}

/// Edge of 0.26's irradiance cube and its mip count (16, 8, 4).
const DIFFUSE_SIZE: usize = 16;
const DIFFUSE_MIPS: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_round_trip() {
        for v in [[0.0, 0.0, 0.0], [0.1, 0.2, 0.3], [1.0, 0.5, 0.25], [12.0, 9.0, 3.0]] {
            let back = decode_026(encode_026(v, 4.0), 4.0);
            for k in 0..3 {
                assert!((back[k] - v[k]).abs() <= 0.02 * v[k].max(0.05), "{v:?} {back:?}");
            }
        }
        // 0.69d: a = 0.5 is m = 1, so L = rgb^2.
        let l = decode_069d([255, 128, 0, 128]);
        assert!((l[0] - 1.0).abs() < 0.02 && (l[1] - 0.25).abs() < 0.02 && l[2] == 0.0, "{l:?}");
    }

    #[test]
    fn cube_directions_follow_d3d() {
        assert_eq!(cube_direction(2, 0, 0, 1), [0.0, 1.0, 0.0]);
        assert_eq!(cube_direction(4, 0, 0, 1), [0.0, 0.0, 1.0]);
        // Face +X: texel row grows downwards (-y), column towards -z.
        let d = cube_direction(0, 1, 1, 2);
        assert!(d[0] > 0.0 && d[1] < 0.0 && d[2] < 0.0);
    }

    #[test]
    fn sh_band_zero_is_uniform() {
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0, 2.0, 3.0];
        let p = DonorProbe { texture: "t".into(), capture_exposure_value: 0.0, directional_factor: 0.5, sh };
        let up = p.sh_eval([0.0, 1.0, 0.0]);
        let side = p.sh_eval([1.0, 0.0, 0.0]);
        assert_eq!(up, side);
        assert!((up[0] - 0.282_094_78).abs() < 1e-6);
    }
}

// ---------------------------------------------------------------------------
// Volumes and 0.69d ambient lights
// ---------------------------------------------------------------------------

/// A probe's or light's shape, as both builds place it (scale = half extents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere { radius: f32 },
    Box { half: [f32; 3] },
    /// 0.69d geometry 2: radius over x/z, half height on y.
    Cylinder { radius: f32, half_height: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub centre: [f32; 3],
    /// Rotation about +y (radians); the scenes rotate volumes about y only.
    pub yaw: f32,
    pub shape: Shape,
    /// Width of the falloff band inside the outer surface.
    pub side_width: f32,
}

impl Volume {
    /// The radius 0.26 sorts probes by (larger drawn first) and culls with.
    pub fn sort_radius(&self) -> f32 {
        match self.shape {
            Shape::Sphere { radius } => radius,
            Shape::Box { half } => (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt(),
            Shape::Cylinder { radius, half_height } => (radius * radius + half_height * half_height).sqrt(),
        }
    }

    fn local(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.centre[0], p[1] - self.centre[1], p[2] - self.centre[2]];
        let (s, c) = self.yaw.sin_cos();
        [c * d[0] - s * d[2], d[1], s * d[0] + c * d[2]]
    }

    /// 0.69d's falloff (`ambient_light_*.ps`, `environment_probe.ps`): per
    /// axis `t = saturate((d - inner) / (outer - inner))`, `(1 - t)^p` for
    /// p > 1, else `1 - t^(1/p)`; 0 outside.
    pub fn weight(&self, p: [f32; 3], power: f32) -> f32 {
        let fall = |d: f32, outer: f32| {
            let inner = (outer - self.side_width).max(0.01).min(outer - 1e-3);
            let t = ((d - inner) / (outer - inner)).clamp(0.0, 1.0);
            if power > 1.0 { (1.0 - t).powf(power) } else { 1.0 - t.powf(1.0 / power.max(1e-3)) }
        };
        let l = self.local(p);
        match self.shape {
            Shape::Sphere { radius } => fall((l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt(), radius),
            Shape::Box { half } => (0..3).map(|i| fall(l[i].abs(), half[i])).product(),
            Shape::Cylinder { radius, half_height } => fall((l[0] * l[0] + l[2] * l[2]).sqrt(), radius) * fall(l[1].abs(), half_height),
        }
    }

    pub fn contains(&self, p: [f32; 3]) -> bool {
        self.weight(p, 1.0) > 0.0
    }
}

/// What a 0.69d ambient light does to the light under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightEffect {
    /// Type 0: adds `E / pi` (scene units, linear colour) to diffuse light.
    Add { radiance: [f32; 3] },
    /// Type 1: multiplies the light by `factor` (multiplier x linear colour).
    Multiply { factor: [f32; 3] },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DonorLight {
    pub volume: Volume,
    pub effect: LightEffect,
    pub attenuation_power: f32,
    /// `dot_normal`: (cutoff, power) of its facing term.
    pub facing: Option<(f32, f32)>,
}

impl DonorLight {
    /// The facing term for a surface with normal `n` at `p`.
    fn facing(&self, n: [f32; 3], p: [f32; 3]) -> f32 {
        let Some((cutoff, power)) = self.facing else { return 1.0 };
        let d = [self.volume.centre[0] - p[0], self.volume.centre[1] - p[1], self.volume.centre[2] - p[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        // At the light itself every surface around faces it.
        if len < 0.5 {
            return 1.0;
        }
        let ndl = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]) / len;
        ((ndl * 0.5 + 0.5) * (1.0 + cutoff) - cutoff).clamp(0.0, 1.0).powf(power)
    }
}

/// One light over a 0.26 probe volume: its effect as at `at` (the volume's
/// centre), scaled by `weight`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Term {
    pub light: DonorLight,
    pub at: [f32; 3],
    pub weight: f32,
}

/// The diffuse light (E / pi, 0.69d scene units) a 0.26 probe holds: a
/// donor probe's SH, then lights in the order 0.26 draws them over it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffuseRecipe {
    pub base: DonorProbe,
    pub terms: Vec<Term>,
}

impl DiffuseRecipe {
    pub fn eval(&self, n: [f32; 3]) -> [f32; 3] {
        let k = self.base.exposure();
        let mut e = self.base.sh_eval(n).map(|v| v * k);
        for Term { light, at, weight } in &self.terms {
            let w = weight * light.facing(n, *at);
            match light.effect {
                LightEffect::Add { radiance } => (0..3).for_each(|c| e[c] += radiance[c] * w),
                LightEffect::Multiply { factor } => (0..3).for_each(|c| e[c] *= 1.0 + (factor[c] - 1.0) * w),
            }
        }
        e
    }
}

/// The 0.26 irradiance cube for a recipe: holds `E / pi` (times the scale)
/// so that it decodes to the light a white surface sends back, as 0.69d's
/// SH evaluation gives it.
pub fn diffuse_cube(recipe: &DiffuseRecipe, scale: LightScale) -> Vec<u8> {
    let faces = (0..6)
        .map(|face| {
            (0..DIFFUSE_MIPS)
                .map(|mip| {
                    let s = DIFFUSE_SIZE >> mip;
                    (0..s * s).map(|i| encode_026(recipe.eval(cube_direction(face, i % s, i / s, s)).map(|v| v * scale.0), 4.0)).collect()
                })
                .collect()
        })
        .collect();
    encode_cube(&CubeRgba8 { size: DIFFUSE_SIZE, faces }, 0)
}

/// A 0.69d probe's cube map in 0.26's encoding and scale, from mip
/// `first_mip` down (a fill light's volume reuses its host's at 32 texels).
pub fn specular_cube(donor_dds: &[u8], probe: &DonorProbe, scale: LightScale, first_mip: usize) -> Result<(Vec<u8>, usize)> {
    let cube = decode_cube(donor_dds)?;
    ensure!(cube.mips() > first_mip && (cube.size >> first_mip) >= 4, "{}: no mip {first_mip}", probe.texture);
    let k = probe.exposure() * scale.0;
    // 0.26's encoding tops out at a = 1/255 (e = 65025); keep a margin.
    const MAX_E: f32 = 60000.0;
    let mut clamped = 0;
    let faces = cube
        .faces
        .iter()
        .map(|chain| {
            chain[first_mip..]
                .iter()
                .map(|image| {
                    image
                        .iter()
                        .map(|&t| {
                            let l = decode_069d(t).map(|v| v * k);
                            if l.iter().any(|v| v * 4.0 > MAX_E) {
                                clamped += 1;
                            }
                            encode_026(l.map(|v| v.min(MAX_E / 4.0)), 4.0)
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    Ok((encode_cube(&CubeRgba8 { size: cube.size >> first_mip, faces }, 0), clamped))
}

/// 0.69d sun: `sun_lighting.ps` lights a white surface facing it with
/// `sun_intensity_r2 / pi * SOLAR * T` (cd/m^2), `SOLAR` the Bruneton
/// solar irradiance in lux and `T` the transmittance of its atmosphere
/// (Rayleigh `air_density` x (5.8045, 13.5629, 33.1125)e-3/km over 8 km,
/// Mie `air_turbidity` x 2e-3/km over 1.2 km, ozone over 15 km) along the
/// sun's path at `elevation` degrees.
pub fn sun_radiance(sun_intensity_r2: f32, air_density: f32, air_turbidity: f32, elevation: f32) -> [f32; 3] {
    const SOLAR: [f32; 3] = [144_809.86, 129_443.62, 127_098.89];
    const RAYLEIGH: [f32; 3] = [5.8045e-3, 1.35629e-2, 3.31125e-2];
    const OZONE: [f32; 3] = [0.650e-3, 1.881e-3, 0.085e-3];
    let mu = elevation.to_radians().sin().max(0.05);
    std::array::from_fn(|c| {
        let tau = air_density * RAYLEIGH[c] * 8.0 + air_turbidity * 2e-3 * 1.2 + OZONE[c] * 15.0;
        sun_intensity_r2 / std::f32::consts::PI * SOLAR[c] * (-tau / mu).exp()
    })
}

#[cfg(test)]
mod light_tests {
    use super::*;

    #[test]
    fn sun_matches_the_atmosphere_model() {
        // Rudnya: elevation 60, density 2, turbidity 9, intensity 0.85 ->
        // (106653, 80874, 57098) lux normal to the beam.
        let r = sun_radiance(0.85, 2.0, 9.0, 60.0).map(|v| v * std::f32::consts::PI);
        for (got, want) in r.iter().zip([106_653.0, 80_874.0, 57_098.0]) {
            assert!((got / want - 1.0).abs() < 0.02, "{r:?}");
        }
    }

    #[test]
    fn falloff_and_order() {
        let v = Volume { centre: [0.0; 3], yaw: 0.0, shape: Shape::Sphere { radius: 4.0 }, side_width: 2.0 };
        assert_eq!(v.weight([1.0, 0.0, 0.0], 2.0), 1.0);
        assert!((v.weight([3.0, 0.0, 0.0], 2.0) - 0.25).abs() < 1e-5);
        assert_eq!(v.weight([5.0, 0.0, 0.0], 2.0), 0.0);
        let b = Volume { shape: Shape::Box { half: [3.0, 4.0, 12.0] }, ..v };
        assert_eq!(b.sort_radius(), 13.0);
        // Rotated a quarter turn, the long axis lies along x.
        let r = Volume { yaw: std::f32::consts::FRAC_PI_2, ..b };
        assert!(r.contains([11.0, 0.0, 0.0]) && !r.contains([0.0, 0.0, 11.0]));
    }

    #[test]
    fn recipe_adds_and_multiplies_in_order() {
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0 / 0.282_094_78 / 1.2; 3];
        let base = DonorProbe { texture: "t".into(), capture_exposure_value: 0.0, directional_factor: 0.5, sh };
        let volume = Volume { centre: [0.0; 3], yaw: 0.0, shape: Shape::Sphere { radius: 4.0 }, side_width: 1.0 };
        let add = DonorLight { volume, effect: LightEffect::Add { radiance: [2.0; 3] }, attenuation_power: 2.0, facing: None };
        let mul = DonorLight { effect: LightEffect::Multiply { factor: [3.0; 3] }, ..add };
        let recipe = DiffuseRecipe {
            base,
            terms: vec![Term { light: add, at: [0.0; 3], weight: 1.0 }, Term { light: mul, at: [0.0; 3], weight: 0.5 }],
        };
        let e = recipe.eval([0.0, 1.0, 0.0]);
        assert!((e[0] - 6.0).abs() < 1e-4, "{e:?}");
    }
}
