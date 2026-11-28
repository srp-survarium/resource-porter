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
use crate::ambient_facing::{self, Facing, Falloff};
use crate::binary_config::ConfigValue;
use crate::dds_cube::{CubeRgba8, decode_cube, encode_cube};
use anyhow::{Result, bail, ensure};

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
    /// The probe's `multiplier` (default 1, at least 0): 0.69d scales its
    /// diffuse light by it, not its reflections (`environment_probe.ps`
    /// multiplies the SH by `m_material_parameters_5.z`, which
    /// `update_probe_constants` fills with `max(multiplier, 0)`). Interiors
    /// carry 1.5 to 10.
    pub multiplier: f32,
    pub sh: [[f32; 3]; 9],
}

impl DonorProbe {
    /// Capture units to 0.69d scene luminance.
    pub fn exposure(&self) -> f32 {
        1.2 * 2f32.powf(self.capture_exposure_value)
    }

    /// Capture units to the 0.69d scene luminance of its diffuse light: the
    /// exposure and the probe's multiplier.
    pub fn diffuse_scale(&self) -> f32 {
        self.exposure() * self.multiplier
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
        let p = DonorProbe { texture: "t".into(), capture_exposure_value: 0.0, directional_factor: 0.5, multiplier: 1.0, sh };
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
    /// The object's `rotation` (radians), Euler angles as both builds'
    /// `create_rotation` takes them. The scenes tip volumes over too: the
    /// church's floor lights lie on their side (x ~1.5), the bus stop's probe
    /// turns about every axis.
    pub rotation: [f32; 3],
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

    /// `p` in the volume's frame: the engine's row-vector transform
    /// (`world = local * basis + centre`) inverted, the basis orthonormal.
    fn local(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.centre[0], p[1] - self.centre[1], p[2] - self.centre[2]];
        let b = crate::skeleton_animation::euler_to_basis(self.rotation);
        std::array::from_fn(|i| b[i][0] * d[0] + b[i][1] * d[1] + b[i][2] * d[2])
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
    /// `dot_normal`: its facing term (`dot_normal_cutoff`, `dot_normal_power`).
    pub facing: Option<Facing>,
    /// `gradient_attenuation`: a box's falloff times `saturate(0.5 - 0.5 y)^power` along its
    /// local y (unit coordinates), full at one face, nothing at the other.
    pub gradient: bool,
}

/// Simpson's rule over [a, b].
fn integral(a: f32, b: f32, f: impl Fn(f32) -> f32) -> f32 {
    const N: usize = 256;
    let h = (b - a) / N as f32;
    let sum: f32 = (0..=N).map(|i| f(a + h * i as f32) * if i == 0 || i == N { 1.0 } else if i % 2 == 1 { 4.0 } else { 2.0 }).sum();
    sum * h / 3.0
}

/// 0.26 draws a sphere light with `(1 - (d / 0.98 R)^2)^p` (`ambient_light.ps`);
/// 0.69d fades from `outer - side_width` to `outer` (`Volume::weight`). The
/// 0.26 radius that ends where 0.69d's light ends, and the power closest to
/// its falloff over the sphere's volume.
pub fn sphere_fit(outer: f32, side_width: f32, power: f32) -> (f32, f32) {
    let donor = Volume { centre: [0.0; 3], rotation: [0.0; 3], shape: Shape::Sphere { radius: outer }, side_width };
    let error = |p: f32| integral(0.0, 1.0, |u| ((1.0 - u * u).max(0.0).powf(p) - donor.weight([u * outer, 0.0, 0.0], power)).powi(2) * u * u);
    // The error has one minimum in p: a golden-section search on log p.
    let (mut lo, mut hi) = (0.02f32.ln(), 64f32.ln());
    const G: f32 = 0.618_034;
    for _ in 0..48 {
        let (a, b) = (hi - G * (hi - lo), lo + G * (hi - lo));
        if error(a.exp()) < error(b.exp()) { hi = b } else { lo = a }
    }
    (outer / 0.98, (0.5 * (lo + hi)).exp())
}

/// A 0.26 ambient light (`object_ambient_light`, drawn by `ambient_light.ps`):
/// with an intensity above 1 it adds `colour^2.2 * (intensity - 1)` times its
/// falloff to the diffuse light, otherwise it multiplies the light under it
/// towards `colour^2.2 * intensity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeLight {
    /// A sphere (its radius 0.26's) or a box (its `side_width` the donor's).
    pub volume: Volume,
    /// sRGB, as the object stores it.
    pub colour: [f32; 3],
    pub intensity: f32,
    /// The property: a sphere's 0.26 distance power, or encoded (`ambient_facing`) a
    /// sphere's with 0.69d's facing term, or a box's 0.69d falloff (its power and gradient)
    /// with its facing term or none; the lighting pack's `ambient_light.ps` draws an encoded
    /// light in a 0.69d scene.
    pub attenuation_power: f32,
    /// `affect_specular`: a darkening light multiplies the probes' reflection
    /// under it too (0.26 draws it only for an intensity at most 1).
    pub darkens_reflections: bool,
}

impl NativeLight {
    /// `dot_normal`: on for a light whose power is encoded (only the `dot_normal` programs
    /// decode it).
    pub fn dot_normal(&self) -> bool {
        ambient_facing::is_encoded(self.attenuation_power)
    }
}

impl DonorLight {
    /// The light's shape and falloff as 0.26 draws them: a sphere with 0.26's
    /// falloff fitted to 0.69d's; a box with 0.69d's own (its side width, power and
    /// gradient), which the lighting pack draws; a cylinder (0.26 has none) on the
    /// box around it, the lighting pack drawing 0.69d's cylinder in it.
    fn native_volume(&self) -> (Volume, Falloff) {
        let v = self.volume;
        let (power, gradient) = (self.attenuation_power, self.gradient);
        match v.shape {
            Shape::Sphere { radius } => {
                let (radius, power) = sphere_fit(radius, v.side_width, power);
                (Volume { shape: Shape::Sphere { radius }, ..v }, Falloff::Sphere { power })
            }
            Shape::Box { .. } => (v, Falloff::Box { power, gradient }),
            Shape::Cylinder { radius, half_height } => {
                (Volume { shape: Shape::Box { half: [radius, half_height, radius] }, ..v }, Falloff::Cylinder { power, gradient })
            }
        }
    }

    /// The property `attenuation_power` for `falloff`: a sphere without a facing
    /// term keeps its plain power; a sphere with one and every box are encoded
    /// (a box without one with `Facing::NONE`).
    fn attenuation_power(&self, falloff: Falloff) -> Result<f32> {
        let encode = |facing| ambient_facing::encode(falloff, facing).map_err(anyhow::Error::msg);
        match (falloff, self.facing) {
            (Falloff::Sphere { power }, None) => Ok(power),
            (Falloff::Sphere { .. }, Some(facing)) => encode(facing),
            (Falloff::Box { .. } | Falloff::Cylinder { .. }, facing) => encode(facing.unwrap_or(Facing::NONE)),
        }
    }

    /// The 0.26 light that adds `radiance` (0.26's scale, the light a white
    /// surface facing it sends back) with this light's shape, falloff and
    /// facing.
    pub fn native_additive(&self, radiance: [f32; 3]) -> Result<NativeLight> {
        let (volume, falloff) = self.native_volume();
        let v = radiance.map(|c| c.max(0.0));
        let peak = v.iter().cloned().fold(1e-6, f32::max);
        Ok(NativeLight {
            volume,
            colour: v.map(|c| (c / peak).powf(1.0 / 2.2)),
            intensity: 1.0 + peak,
            attenuation_power: self.attenuation_power(falloff)?,
            darkens_reflections: false,
        })
    }

    /// The 0.26 light that multiplies the light under it by `factor` (each
    /// channel at most 1), the probes' reflection too with `darkens_reflections`.
    pub fn native_multiply(&self, factor: [f32; 3], darkens_reflections: bool) -> Result<NativeLight> {
        let (volume, falloff) = self.native_volume();
        Ok(NativeLight {
            volume,
            colour: factor.map(|f| f.clamp(0.0, 1.0).powf(1.0 / 2.2)),
            intensity: 1.0,
            attenuation_power: self.attenuation_power(falloff)?,
            darkens_reflections,
        })
    }
}

/// The 0.26 irradiance cube of a 0.69d probe: `E / pi` from its SH times its
/// multiplier, in 0.26's scale, so that it decodes to the light a white
/// surface sends back as `environment_probe.ps` gives it.
pub fn diffuse_cube(probe: &DonorProbe, scale: LightScale) -> Vec<u8> {
    let k = probe.diffuse_scale() * scale.0;
    let faces = (0..6)
        .map(|face| {
            (0..DIFFUSE_MIPS)
                .map(|mip| {
                    let s = DIFFUSE_SIZE >> mip;
                    (0..s * s).map(|i| encode_026(probe.sh_eval(cube_direction(face, i % s, i / s, s)).map(|v| v * k), 4.0)).collect()
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

/// What 0.69d's environment does to its sun and sky beyond `sun_radiance`.
/// Its sun light (0x200443310) and the sun its sky scatters (0x200442D33)
/// are `sun_intensity_r2 * sun_tint.rgb`; the sky is also times
/// `sky_intensity`. Absent, each is 1 (the loader 0x200222F18).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DonorSky {
    pub sun_tint: [f32; 3],
    pub sky_intensity: f32,
}

impl DonorSky {
    pub fn read(environment: &ConfigValue) -> Result<Self> {
        let sun_tint = match environment.named("sun_tint") {
            None => [1.0; 3],
            Some(ConfigValue::Float4([r, g, b, _]) | ConfigValue::Float3([r, g, b])) => [*r, *g, *b],
            Some(ConfigValue::Float(v)) => [*v; 3],
            Some(ConfigValue::Integer(v)) => [*v as f32; 3],
            Some(other) => bail!("sun_tint is not a colour: {other:?}"),
        };
        let sky_intensity = match environment.named("sky_intensity") {
            None => 1.0,
            Some(ConfigValue::Float(v)) => *v,
            Some(ConfigValue::Integer(v)) => *v as f32,
            Some(other) => bail!("sky_intensity is not a number: {other:?}"),
        };
        ensure!(
            sun_tint.iter().chain([&sky_intensity]).all(|v| v.is_finite() && *v >= 0.0),
            "sun_tint {sun_tint:?} or sky_intensity {sky_intensity} out of range"
        );
        Ok(Self { sun_tint, sky_intensity })
    }

    pub fn tinted(&self, sun: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|c| sun[c] * self.sun_tint[c])
    }

    /// 0.69d's sky over the one 0.26's atmosphere draws under the tinted sun
    /// `sun`, both relative to the untinted `white` (where they agree: Rudnya).
    /// 0.69d's follows the tinted sun's luminance times `sky_intensity`;
    /// 0.26's Rayleigh sky follows `sun_intensity^1.4`.
    pub fn scale_over_026(&self, white: [f32; 3], sun: [f32; 3]) -> f32 {
        let luma = |v: [f32; 3]| 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        let peak = |v: [f32; 3]| v.iter().cloned().fold(0.0, f32::max);
        let donor = luma(sun) / luma(white) * self.sky_intensity;
        let native = (peak(sun) / peak(white)).powf(1.4);
        donor / native
    }
}

/// A sun of `radiance` (per channel, the scene's light scale) as 0.26g's
/// environment stores it: `sun_color` and `sun_intensity`. The game raises
/// `sun_color` to 2.2 when it loads it (the environment loader at 0x34e447,
/// through 0x1bfcb0, every channel) and lights with that times
/// `sun_intensity` (`light_color`, `stage_composition::execute`), so the
/// colour is written as the linear colour to 1/2.2.
pub fn sun_environment(radiance: [f32; 3]) -> ([f32; 3], f32) {
    let intensity = radiance.iter().cloned().fold(0.0, f32::max);
    (radiance.map(|v| (v / intensity).max(0.0).powf(1.0 / 2.2)), intensity)
}

/// 0.69d point and spot lights (`deferred_lighting.ps`, constants from
/// 0x4427a0): a white Lambertian surface at `d` facing the light sends back
/// `c * luminous_power * 1000 / D * (1 - (d / range)^16)^2 / d^2` cd/m^2,
/// `c` the linear colour, `D` 4 pi^2 for a point light and pi^2 for a spot
/// (its power is not spread over the cone), times `diffuse_influence_factor`;
/// the object's `intensity` and `attenuation_power` are not read. 0.26 draws
/// `c * intensity * (1 - (d / R)^2)^p` with no 1/d^2 (`point_light_accumulator.ps`),
/// `R` the range, a spot's `range / sin(penumbra / 2)`. The 0.26 light keeps
/// the donor's range (where its light ends) and takes the intensity and
/// power closest to the donor's falloff in ratio, by volume over a tenth to
/// nine tenths of the range (the 1/d^2 near the bulb has no 0.26 shape).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub luminous_power: f32,
    pub range: f32,
    pub diffuse_influence: f32,
    /// A spot's full penumbra angle (radians); none for a point light.
    pub spot_penumbra: Option<f32>,
}

impl PointLight {
    /// 0.69d's radiance off a white surface at `d` facing it (cd/m^2, white light).
    pub fn radiance(&self, d: f32) -> f32 {
        let denominator = if self.spot_penumbra.is_some() { std::f32::consts::PI.powi(2) } else { 4.0 * std::f32::consts::PI.powi(2) };
        let window = (1.0 - (d / self.range).powi(16)).max(0.0).powi(2);
        self.luminous_power * 1000.0 / denominator * self.diffuse_influence * window / (d * d).max(1e-4)
    }

    /// The 0.26 light's (intensity, attenuation power) in `scale`.
    pub fn native(&self, scale: LightScale) -> (f32, f32) {
        // 0.26's falloff radius in ranges.
        let rho = self.spot_penumbra.map_or(1.0, |p| 1.0 / (0.5 * p).sin().max(0.05));
        // Weighted least squares of ln(donor) = ln(k) + p ln(1 - (u / rho)^2).
        const N: usize = 256;
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for i in 0..=N {
            let u = 0.1 + 0.8 * i as f32 / N as f32;
            let w = (u * u) as f64;
            let x = (1.0 - (u / rho).powi(2)).ln() as f64;
            let y = (self.radiance(u * self.range) as f64).ln();
            sw += w;
            sx += w * x;
            sy += w * y;
            sxx += w * x * x;
            sxy += w * x * y;
        }
        let p = (sw * sxy - sx * sy) / (sw * sxx - sx * sx);
        let ln_k = (sy - p * sx) / sw;
        (ln_k.exp() as f32 * scale.0, p as f32)
    }
}

#[cfg(test)]
mod light_tests {
    use super::*;

    #[test]
    fn the_sun_takes_its_tint_and_the_sky_its_intensity() {
        let env = |fields: Vec<(&str, ConfigValue)>| {
            ConfigValue::Named(fields.into_iter().map(|(n, value)| crate::binary_config::NamedValue { name: n.into(), value }).collect())
        };
        let white = sun_radiance(0.85, 2.0, 20.0, 38.17);
        // Rudnya has neither key: nothing changes.
        let plain = DonorSky::read(&env(vec![])).unwrap();
        assert_eq!(plain.tinted(white), white);
        assert_eq!(plain.scale_over_026(white, white), 1.0);
        // School: a grey 0.518 tint; 0.26's sky falls as 0.518^1.4.
        let tint = ConfigValue::Float4([0.518, 0.518, 0.518, 1.0]);
        let school = DonorSky::read(&env(vec![("sun_tint", tint), ("sky_intensity", ConfigValue::Integer(1))])).unwrap();
        let sun = school.tinted(white);
        assert!((sun[1] / white[1] - 0.518).abs() < 1e-6);
        assert!((school.scale_over_026(white, sun) - 0.518f32.powf(-0.4)).abs() < 1e-4);
        // Chemical Plant: half the sky.
        let tint = ConfigValue::Float4([0.604, 0.604, 0.604, 1.0]);
        let chem = DonorSky::read(&env(vec![("sun_tint", tint), ("sky_intensity", ConfigValue::Float(0.5))])).unwrap();
        assert!((chem.scale_over_026(white, chem.tinted(white)) - 0.5 * 0.604f32.powf(-0.4)).abs() < 1e-4);
    }

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
    fn the_game_lights_with_the_suns_radiance() {
        // What the game draws with: sun_color^2.2 (its loader) times sun_intensity.
        let radiance = [4.883, 3.702, 2.613];
        let (colour, intensity) = sun_environment(radiance);
        let lit = colour.map(|c| c.powf(2.2) * intensity);
        for (got, want) in lit.iter().zip(radiance) {
            assert!((got / want - 1.0).abs() < 1e-4, "{lit:?}");
        }
        assert_eq!(colour[0], 1.0);
    }

    #[test]
    fn a_point_light_follows_the_donors_falloff() {
        // The lobby's workshop lamp: 11 lm, 1.8 m; 1114 cd/m^2 at 0.5 m, 279 at 1 m.
        let lamp = PointLight { luminous_power: 11.0, range: 1.8, diffuse_influence: 1.0, spot_penumbra: None };
        assert!((lamp.radiance(0.5) / 1114.6 - 1.0).abs() < 0.01 && (lamp.radiance(1.0) / 278.7 - 1.0).abs() < 0.01);
        let (intensity, power) = lamp.native(LightScale(1.0));
        let native = |d: f32| intensity * (1.0 - (d / 1.8f32).powi(2)).max(0.0).powf(power);
        for d in [0.5, 1.0, 1.4] {
            assert!((native(d) / lamp.radiance(d)).ln().abs() < 0.5, "{d}: {} {}", native(d), lamp.radiance(d));
        }
        // A spot's power is not spread over its cone: four times a point's.
        let spot = PointLight { spot_penumbra: Some(2.269), ..lamp };
        assert!((spot.radiance(0.5) / lamp.radiance(0.5) - 4.0).abs() < 1e-3);
    }

    #[test]
    fn falloff_and_order() {
        let v = Volume { centre: [0.0; 3], rotation: [0.0; 3], shape: Shape::Sphere { radius: 4.0 }, side_width: 2.0 };
        assert_eq!(v.weight([1.0, 0.0, 0.0], 2.0), 1.0);
        assert!((v.weight([3.0, 0.0, 0.0], 2.0) - 0.25).abs() < 1e-5);
        assert_eq!(v.weight([5.0, 0.0, 0.0], 2.0), 0.0);
        let b = Volume { shape: Shape::Box { half: [3.0, 4.0, 12.0] }, ..v };
        assert_eq!(b.sort_radius(), 13.0);
        // Rotated a quarter turn, the long axis lies along x.
        let r = Volume { rotation: [0.0, std::f32::consts::FRAC_PI_2, 0.0], ..b };
        assert!(r.contains([11.0, 0.0, 0.0]) && !r.contains([0.0, 0.0, 11.0]));
        // Tipped a quarter turn about x, the tall axis (y) lies along z.
        let tall = Volume { shape: Shape::Box { half: [1.0, 4.0, 1.0] }, ..v };
        let tipped = Volume { rotation: [std::f32::consts::FRAC_PI_2, 0.0, 0.0], ..tall };
        assert!(tall.contains([0.0, 3.5, 0.0]) && !tipped.contains([0.0, 3.5, 0.0]) && tipped.contains([0.0, 0.0, 3.5]));
        // Any rotation: a point placed in the box's frame by the engine's
        // row-vector transform is inside; the transposed placement is not.
        let rotation = [0.3, 1.1, -0.7];
        let basis = crate::skeleton_animation::euler_to_basis(rotation);
        let local = [3.5, 0.2, -0.3];
        let row: [f32; 3] = std::array::from_fn(|j| (0..3).map(|i| local[i] * basis[i][j]).sum());
        let column: [f32; 3] = std::array::from_fn(|i| (0..3).map(|j| basis[i][j] * local[j]).sum());
        let slab = Volume { rotation, shape: Shape::Box { half: [4.0, 0.5, 0.5] }, ..v };
        assert!(slab.contains(row) && !slab.contains(column), "{row:?} {column:?}");
    }

    #[test]
    fn fits_follow_the_donor_falloff() {
        // A sphere that fades over its whole radius with power 2 is (1 - d/R)^2:
        // 0.26's (1 - (d/R)^2)^p is closest by volume at p 4.9, within 0.18
        // everywhere (0.26 has no flat middle: a light with one is followed
        // only within 0.33; its exact shape needs 0.69d's own shader).
        let (radius, power) = sphere_fit(4.0, 4.0, 2.0);
        assert!((radius - 4.0 / 0.98).abs() < 1e-5 && (power - 4.91).abs() < 0.05, "{power}");
        for i in 0..=20 {
            let u = i as f32 / 20.0;
            assert!(((1.0 - u * u).powf(power) - (1.0 - u).powi(2)).abs() < 0.18, "{u}");
        }
        // A light flat to half its radius keeps a low power.
        assert!((sphere_fit(4.0, 2.0, 2.0).1 - 1.37).abs() < 0.05);
    }

    #[test]
    fn donor_lights_become_native_lights() {
        let volume = Volume { centre: [0.0; 3], rotation: [0.0; 3], shape: Shape::Sphere { radius: 4.0 }, side_width: 2.0 };
        let facing = Facing { cutoff: 0.0, power: 4.0 };
        let light = DonorLight { volume, effect: LightEffect::Add { radiance: [0.0; 3] }, attenuation_power: 2.0, facing: Some(facing), gradient: false };
        let n = light.native_additive([0.4, 0.2, 0.1]).unwrap();
        // Adds colour^2.2 * (intensity - 1): the radiance; the shader draws
        // 0.69d's facing term from the power, the fitted distance power with it.
        for c in 0..3 {
            let added = n.colour[c].powf(2.2) * (n.intensity - 1.0);
            assert!((added - [0.4, 0.2, 0.1][c]).abs() < 1e-4, "{n:?}");
        }
        let fit = sphere_fit(4.0, 2.0, 2.0).1;
        let carried = ambient_facing::decode(n.attenuation_power, ambient_facing::Shape::Sphere).unwrap();
        assert_eq!((carried.facing, carried.falloff), (facing, Falloff::Sphere { power: ambient_facing::quantize_power(fit) }));
        assert!(n.intensity > 1.0 && n.dot_normal() && matches!(n.volume.shape, Shape::Sphere { radius } if (radius - 4.0 / 0.98).abs() < 1e-5));
        // A box keeps the donor's side width, power and gradient (0.26 draws no power)
        // beside the term.
        let cube = DonorLight { volume: Volume { shape: Shape::Box { half: [2.0, 1.0, 3.0] }, ..volume }, gradient: true, ..light };
        let b = cube.native_multiply([0.5, 0.5, 0.5], false).unwrap();
        let carried = ambient_facing::decode(b.attenuation_power, ambient_facing::Shape::Box).unwrap();
        assert_eq!((carried.facing, carried.falloff), (facing, Falloff::Box { power: 2.0, gradient: true }));
        assert!(b.dot_normal() && b.volume.side_width == 2.0);
        // A cylinder is drawn on the box around it, as a cylinder; a darkening light keeps
        // intensity 1; without a facing term it carries none, drawn by the adapted program.
        let cylinder = DonorLight { volume: Volume { shape: Shape::Cylinder { radius: 2.0, half_height: 3.0 }, ..volume }, facing: None, ..light };
        let d = cylinder.native_multiply([0.5, 0.25, 0.0], true).unwrap();
        assert_eq!(d.volume.shape, Shape::Box { half: [2.0, 3.0, 2.0] });
        let carried = ambient_facing::decode(d.attenuation_power, ambient_facing::Shape::Box).unwrap();
        assert_eq!((carried.facing, carried.falloff), (Facing::NONE, Falloff::Cylinder { power: 2.0, gradient: false }));
        assert!(d.intensity == 1.0 && d.dot_normal() && d.darkens_reflections && !b.darkens_reflections);
        assert!((d.colour[0].powf(2.2) - 0.5).abs() < 1e-4 && d.colour[2] == 0.0);
        // A sphere without a facing term keeps a plain power.
        let plain = DonorLight { facing: None, ..light }.native_additive([0.1; 3]).unwrap();
        assert!(!plain.dot_normal() && (plain.attenuation_power - fit).abs() < 1e-6);
        // A term the encoding cannot carry exactly is refused.
        let odd = DonorLight { facing: Some(Facing { cutoff: 0.0, power: 0.25 }), ..light };
        assert!(odd.native_additive([0.1; 3]).is_err());
    }
}
