// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's facing term and box falloff on a 0.26g ambient light, carried in its
//! `attenuation_power`.
//!
//! 0.69d (`ambient_light_*.ps`, with `dot_normal`) draws a light times
//! `saturate(saturate(x * 0.5 + 0.5) * (1 + cutoff) - cutoff)^power`, `x` the cosine
//! between the surface's normal and the direction to the light's centre; 0.26g's
//! `dot_normal` draws `max(x, 0)`. Besides its colour and intensity, a 0.26g ambient light
//! hands its program one float, `attenuation_power` (`ambient_light_color_and_power.w`, the
//! property's bits copied unchanged from the project: exe 0x290856, 0x290A4B, 0x1C4745,
//! 0x181AEF). A light converted from 0.69d with `dot_normal` carries in it, besides 0.26g's
//! distance power (a box: 0.69d's own falloff), the facing term's cutoff and power; the
//! adapted `ambient_light.ps` (`weather_fog_021::ambient_wrap`) decodes it for a 0.69d scene
//! and draws 0.69d's term and box falloff.
//!
//! The float's bits are `BASE + payload`, `BASE` the bits of 2^20 and the payload below
//! 2^29 (the value 2^20 to 2^85, finite):
//!
//! | bits    | field                 | value                               |
//! |---------|-----------------------|-------------------------------------|
//! | 0..13   | the falloff           | a sphere's or a box's, below        |
//! | 13..21  | facing power          | `n * 0.1f` (0.1 to 25.5)            |
//! | 21..29  | cutoff                | `(n - 128) * 0.01f` (-1.28 to 1.27) |
//!
//! The falloff field is the shape's ([`Falloff`]): a sphere's 0.26g distance power `n / 128`
//! (0 to 63.99); a box's 0.69d falloff, which 0.26g's box program does not draw (it has no
//! power): its power `n / 128` in bits 0..11 (to 15.99), bit 11 kept clear, bit 12 its
//! `gradient_attenuation`.
//!
//! A light without 0.69d's facing (`dot_normal` off) whose falloff only the adapted program
//! draws (every box) carries [`Facing::NONE`], cutoff -1: the term is exactly 1.
//!
//! Decoding is integer arithmetic and one correctly rounded f32 multiply each, the same in
//! the program and here: every value 0.69d uses decodes to its own bits, another decimal to
//! its f32 or the one beside it (0.3: 0.29999998). 0.26g's own scenes never write a power near 2^20 (its maps: 0.5 to
//! 13.2); a program without the adaptation draws a sphere light so encoded as nothing
//! (`(1 - d^2 / R^2)^p`).

/// The bits of 2^20: the smallest encoded value.
pub const BASE: u32 = 0x4980_0000;
/// Payloads are below this.
pub const PAYLOAD_LIMIT: u32 = 1 << 29;
pub const POWER_BITS: u32 = 13;
pub const POWER_STEP: f32 = 1.0 / 128.0;
pub const FACING_POWER_SHIFT: u32 = 13;
pub const FACING_POWER_STEP: f32 = 0.1;
pub const CUTOFF_SHIFT: u32 = 21;
pub const CUTOFF_OFFSET: i32 = 128;
pub const CUTOFF_STEP: f32 = 0.01;

/// The largest distance power a sphere's falloff holds.
pub const MAX_POWER: f32 = ((1 << POWER_BITS) - 1) as f32 * POWER_STEP;
/// A box's falloff: its power in the low bits, its gradient in bit 12.
pub const BOX_POWER_BITS: u32 = 11;
pub const BOX_GRADIENT: u32 = 1 << 12;
/// The largest falloff power a box holds.
pub const MAX_BOX_POWER: f32 = ((1 << BOX_POWER_BITS) - 1) as f32 * POWER_STEP;

/// A facing term as 0.69d's object gives it (`dot_normal_cutoff`, `dot_normal_power`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facing {
    pub cutoff: f32,
    pub power: f32,
}

impl Facing {
    /// No facing: `saturate(y * 0 + 1)^1`, 1 for every cosine (0.69d clamps its own cutoff
    /// at 0, so -1 is never a donor's).
    pub const NONE: Facing = Facing { cutoff: -1.0, power: 1.0 };

    /// 0.69d's term for the cosine `x`, in the order and roundings of its program.
    pub fn eval(&self, x: f32) -> f32 {
        let y = (x * 0.5 + 0.5).clamp(0.0, 1.0);
        let y = (y * (1.0 + self.cutoff) - self.cutoff).clamp(0.0, 1.0);
        (y.log2() * self.power).exp2()
    }
}

/// A light's shape as the program that draws it knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sphere,
    Box,
}

/// What the falloff field carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Falloff {
    /// 0.26g's `(1 - d^2 / (0.98 R)^2)^power`, its power fitted to 0.69d's sphere.
    Sphere { power: f32 },
    /// 0.69d's box (`ambient_light_*.ps`): per axis `(1 - t)^power`, `t` over the side width
    /// inside each face (0.69d keeps the inner box at least 0.01 from the centre), and with
    /// `gradient` (`gradient_attenuation`) times `saturate(0.5 - 0.5 y)^power` along its local
    /// y, `y` -1 to 1 face to face.
    Box { power: f32, gradient: bool },
}

impl Falloff {
    pub fn shape(&self) -> Shape {
        match self {
            Falloff::Sphere { .. } => Shape::Sphere,
            Falloff::Box { .. } => Shape::Box,
        }
    }
}

/// What an encoded `attenuation_power` carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Encoded {
    pub falloff: Falloff,
    pub facing: Facing,
}

/// The distance power the encoding draws for a sphere's `p`: the nearest step, at least one.
pub fn quantize_power(p: f32) -> f32 {
    (p / POWER_STEP).round().clamp(1.0, ((1 << POWER_BITS) - 1) as f32) * POWER_STEP
}

fn field(value: f32, step: f32, offset: i32, what: &str) -> Result<u32, String> {
    let n = (value / step).round() as i64 + offset as i64;
    // the step's multiple is the f32 nearest n * step or the one beside it
    let decoded = ((n - offset as i64) as f32) * step;
    if !(0..256).contains(&n) || (decoded - value).abs() > value.abs().max(step) * f32::EPSILON {
        return Err(format!("{what} {value} is not a step of {step} in the facing encoding"));
    }
    Ok(n as u32)
}

/// The falloff field of `falloff`.
fn falloff_field(falloff: Falloff) -> Result<u32, String> {
    match falloff {
        Falloff::Sphere { power } => {
            if !power.is_finite() || power < 0.0 {
                return Err(format!("invalid distance power {power}"));
            }
            Ok((quantize_power(power) / POWER_STEP) as u32)
        }
        Falloff::Box { power, gradient } => {
            if !power.is_finite() || power <= 0.0 || power > MAX_BOX_POWER {
                return Err(format!("box falloff power {power} is not in (0, {MAX_BOX_POWER}]"));
            }
            let n = ((power / POWER_STEP).round() as u32).max(1);
            Ok(n | if gradient { BOX_GRADIENT } else { 0 })
        }
    }
}

/// `attenuation_power` carrying `falloff` (a power taken to its step) and 0.69d's facing
/// term, which must be exactly representable.
pub fn encode(falloff: Falloff, facing: Facing) -> Result<f32, String> {
    if facing.power.is_nan() || facing.power <= 0.0 {
        return Err(format!("facing power {} (0.69d draws NaN at 0)", facing.power));
    }
    let n_p = falloff_field(falloff)?;
    let n_f = field(facing.power, FACING_POWER_STEP, 0, "facing power")?;
    let n_c = field(facing.cutoff, CUTOFF_STEP, CUTOFF_OFFSET, "cutoff")?;
    let payload = n_p | n_f << FACING_POWER_SHIFT | n_c << CUTOFF_SHIFT;
    Ok(f32::from_bits(BASE + payload))
}

/// The payload of an encoded value, `None` for a plain power.
fn payload(value: f32) -> Option<u32> {
    let payload = value.to_bits().wrapping_sub(BASE);
    (payload < PAYLOAD_LIMIT).then_some(payload)
}

/// Does `value` carry an encoding (and so need the adapted program, through `dot_normal`)?
pub fn is_encoded(value: f32) -> bool {
    payload(value).is_some()
}

/// What `value` carries for a light of `shape`, or `None` for a plain power.
pub fn decode(value: f32, shape: Shape) -> Option<Encoded> {
    let payload = payload(value)?;
    let bits = payload & ((1 << POWER_BITS) - 1);
    let falloff = match shape {
        Shape::Sphere => Falloff::Sphere { power: bits as f32 * POWER_STEP },
        Shape::Box => Falloff::Box { power: (bits & ((1 << BOX_POWER_BITS) - 1)) as f32 * POWER_STEP, gradient: bits & BOX_GRADIENT != 0 },
    };
    let facing_power = ((payload >> FACING_POWER_SHIFT) & 0xff) as f32 * FACING_POWER_STEP;
    let cutoff = ((payload >> CUTOFF_SHIFT) as i32 - CUTOFF_OFFSET) as f32 * CUTOFF_STEP;
    Some(Encoded { falloff, facing: Facing { cutoff, power: facing_power } })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every (cutoff, power) pair of 0.69d's ambient lights with `dot_normal` (Rudnya,
    /// School, Vostok Radar Station, Cologne, the lobby; `resources.56.db`, `.72.db`):
    /// cutoff 0 throughout, powers 0.1, 0.5 and 1 to 10.
    const DONOR_PAIRS: [(f32, f32); 12] = [
        (0.0, 0.1), (0.0, 0.5), (0.0, 1.0), (0.0, 2.0), (0.0, 3.0), (0.0, 4.0),
        (0.0, 5.0), (0.0, 6.0), (0.0, 7.0), (0.0, 8.0), (0.0, 9.0), (0.0, 10.0),
    ];

    #[test]
    fn every_donor_pair_round_trips_exactly() {
        for (cutoff, power) in DONOR_PAIRS {
            let facing = Facing { cutoff, power };
            for p in [0.02, 0.5, 1.0, 1.734, 2.0, 7.77, 40.0, 64.0] {
                let e = encode(Falloff::Sphere { power: p }, facing).unwrap();
                let d = decode(e, Shape::Sphere).unwrap();
                assert_eq!(d.facing.cutoff.to_bits(), cutoff.to_bits(), "{cutoff} {power}");
                assert_eq!(d.facing.power.to_bits(), power.to_bits(), "{cutoff} {power}");
                assert_eq!(d.falloff, Falloff::Sphere { power: quantize_power(p) });
                let Falloff::Sphere { power: q } = d.falloff else { unreachable!() };
                assert!((q - p.min(MAX_POWER)).abs() <= POWER_STEP);
                assert!(e.is_finite() && e >= 1048576.0);
            }
        }
    }

    #[test]
    fn the_fields_cover_their_ranges() {
        for n in 1..256 {
            let power = n as f32 * FACING_POWER_STEP;
            for c in 0..256 {
                let cutoff = (c - CUTOFF_OFFSET) as f32 * CUTOFF_STEP;
                let d = decode(encode(Falloff::Sphere { power: 3.0 }, Facing { cutoff, power }).unwrap(), Shape::Sphere).unwrap();
                assert_eq!((d.facing.cutoff, d.facing.power, d.falloff), (cutoff, power, Falloff::Sphere { power: 3.0 }));
            }
        }
        // the extremes of the payload stay finite
        assert!(f32::from_bits(BASE + PAYLOAD_LIMIT - 1).is_finite());
    }

    #[test]
    fn plain_powers_decode_as_plain() {
        for v in [0.0, 0.5, 1.0, 13.2, 1000.0, 1048575.9, -1.0, -1048576.0, f32::INFINITY, f32::NAN, 1e30] {
            assert_eq!(decode(v, Shape::Sphere), None, "{v}");
            assert_eq!(decode(v, Shape::Box), None, "{v}");
            assert!(!is_encoded(v));
        }
    }

    #[test]
    fn a_decimal_decodes_to_its_f32_or_the_one_beside_it() {
        let d = decode(encode(Falloff::Sphere { power: 1.0 }, Facing { cutoff: 0.3, power: 0.7 }).unwrap(), Shape::Sphere).unwrap();
        assert!(d.facing.cutoff.to_bits().abs_diff(0.3f32.to_bits()) <= 1);
        assert!(d.facing.power.to_bits().abs_diff(0.7f32.to_bits()) <= 1);
    }

    #[test]
    fn unrepresentable_terms_are_refused() {
        let sphere = Falloff::Sphere { power: 1.0 };
        assert!(encode(sphere, Facing { cutoff: 0.0, power: 0.25 }).is_err());
        assert!(encode(sphere, Facing { cutoff: 0.005, power: 1.0 }).is_err());
        assert!(encode(sphere, Facing { cutoff: 0.0, power: 0.0 }).is_err());
        assert!(encode(sphere, Facing { cutoff: 2.0, power: 1.0 }).is_err());
        assert!(encode(Falloff::Sphere { power: -1.0 }, Facing { cutoff: 0.0, power: 1.0 }).is_err());
        for power in [0.0, -1.0, 16.0, f32::NAN] {
            assert!(encode(Falloff::Box { power, gradient: false }, Facing::NONE).is_err(), "{power}");
        }
    }

    /// Every box falloff of 0.69d's lights (powers 0.1 to 4; 0.3 and 0.5 to their step),
    /// with and without its gradient, facing or none.
    #[test]
    fn box_falloffs_round_trip_and_keep_the_gradient() {
        for power in [0.1f32, 0.3, 0.5, 1.0, 2.0, 3.0, 4.0, 15.99] {
            for gradient in [false, true] {
                for facing in [Facing::NONE, Facing { cutoff: 0.0, power: 4.0 }] {
                    let e = encode(Falloff::Box { power, gradient }, facing).unwrap();
                    assert!(is_encoded(e));
                    let d = decode(e, Shape::Box).unwrap();
                    assert_eq!(d.facing, facing);
                    let Falloff::Box { power: q, gradient: g } = d.falloff else { unreachable!() };
                    assert_eq!(g, gradient);
                    assert!((q - power).abs() <= POWER_STEP / 2.0, "{power} {q}");
                }
            }
        }
        assert_eq!(Falloff::Box { power: 1.0, gradient: true }.shape(), Shape::Box);
    }

    #[test]
    fn no_facing_is_exactly_one() {
        let d = decode(encode(Falloff::Box { power: 1.0, gradient: false }, Facing::NONE).unwrap(), Shape::Box).unwrap();
        assert_eq!(d.facing.cutoff.to_bits(), (-1.0f32).to_bits());
        for x in [-1.0f32, -0.3, 0.0, 0.7, 1.0] {
            assert_eq!(d.facing.eval(x), 1.0);
        }
    }

    #[test]
    fn the_term_is_0_69ds() {
        // half-Lambert squared at power 2, nothing behind at cutoff 1
        let f = Facing { cutoff: 0.0, power: 2.0 };
        assert!((f.eval(0.0) - 0.25).abs() < 1e-6 && (f.eval(1.0) - 1.0).abs() < 1e-6);
        assert!(f.eval(-1.0) == 0.0);
        let f = Facing { cutoff: 1.0, power: 1.0 };
        assert!(f.eval(-0.5) == 0.0 && (f.eval(0.5) - 0.5).abs() < 1e-6);
    }
}
