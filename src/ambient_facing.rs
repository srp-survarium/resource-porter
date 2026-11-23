// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's facing term on a 0.26g ambient light, carried in its `attenuation_power`.
//!
//! 0.69d (`ambient_light_*.ps`, with `dot_normal`) draws a light times
//! `saturate(saturate(x * 0.5 + 0.5) * (1 + cutoff) - cutoff)^power`, `x` the cosine
//! between the surface's normal and the direction to the light's centre; 0.26g's
//! `dot_normal` draws `max(x, 0)`. Besides its colour and intensity, a 0.26g ambient light
//! hands its program one float, `attenuation_power` (`ambient_light_color_and_power.w`, the
//! property's bits copied unchanged from the project: exe 0x290856, 0x290A4B, 0x1C4745,
//! 0x181AEF). A light converted from 0.69d with `dot_normal` carries in it, besides 0.26g's
//! distance power, the facing term's cutoff and power; the adapted `ambient_light.ps`
//! (`weather_fog_021::ambient_wrap`) decodes it for a 0.69d scene and draws 0.69d's term.
//!
//! The float's bits are `BASE + payload`, `BASE` the bits of 2^20 and the payload below
//! 2^29 (the value 2^20 to 2^85, finite):
//!
//! | bits    | field                 | value                               |
//! |---------|-----------------------|-------------------------------------|
//! | 0..13   | distance power `p`    | `n / 128` (0 to 63.99)              |
//! | 13..21  | facing power          | `n * 0.1f` (0.1 to 25.5)            |
//! | 21..29  | cutoff                | `(n - 128) * 0.01f` (-1.28 to 1.27) |
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

/// The largest distance power the encoding holds.
pub const MAX_POWER: f32 = ((1 << POWER_BITS) - 1) as f32 * POWER_STEP;

/// A facing term as 0.69d's object gives it (`dot_normal_cutoff`, `dot_normal_power`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facing {
    pub cutoff: f32,
    pub power: f32,
}

impl Facing {
    /// 0.69d's term for the cosine `x`, in the order and roundings of its program.
    pub fn eval(&self, x: f32) -> f32 {
        let y = (x * 0.5 + 0.5).clamp(0.0, 1.0);
        let y = (y * (1.0 + self.cutoff) - self.cutoff).clamp(0.0, 1.0);
        (y.log2() * self.power).exp2()
    }
}

/// What an encoded `attenuation_power` carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Encoded {
    /// 0.26g's distance power (sphere lights; boxes have none).
    pub power: f32,
    pub facing: Facing,
}

/// The distance power the encoding draws for `p`: the nearest step, at least one.
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

/// `attenuation_power` carrying the distance power `p` (taken to its step) and 0.69d's
/// facing term, which must be exactly representable.
pub fn encode(p: f32, facing: Facing) -> Result<f32, String> {
    if !p.is_finite() || p < 0.0 {
        return Err(format!("invalid distance power {p}"));
    }
    if facing.power.is_nan() || facing.power <= 0.0 {
        return Err(format!("facing power {} (0.69d draws NaN at 0)", facing.power));
    }
    let n_p = (quantize_power(p) / POWER_STEP) as u32;
    let n_f = field(facing.power, FACING_POWER_STEP, 0, "facing power")?;
    let n_c = field(facing.cutoff, CUTOFF_STEP, CUTOFF_OFFSET, "cutoff")?;
    let payload = n_p | n_f << FACING_POWER_SHIFT | n_c << CUTOFF_SHIFT;
    Ok(f32::from_bits(BASE + payload))
}

/// What `value` carries, or `None` for a plain power.
pub fn decode(value: f32) -> Option<Encoded> {
    let payload = value.to_bits().wrapping_sub(BASE);
    if payload >= PAYLOAD_LIMIT {
        return None;
    }
    let power = (payload & ((1 << POWER_BITS) - 1)) as f32 * POWER_STEP;
    let facing_power = ((payload >> FACING_POWER_SHIFT) & 0xff) as f32 * FACING_POWER_STEP;
    let cutoff = ((payload >> CUTOFF_SHIFT) as i32 - CUTOFF_OFFSET) as f32 * CUTOFF_STEP;
    Some(Encoded { power, facing: Facing { cutoff, power: facing_power } })
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
                let e = encode(p, facing).unwrap();
                let d = decode(e).unwrap();
                assert_eq!(d.facing.cutoff.to_bits(), cutoff.to_bits(), "{cutoff} {power}");
                assert_eq!(d.facing.power.to_bits(), power.to_bits(), "{cutoff} {power}");
                assert_eq!(d.power, quantize_power(p));
                assert!((d.power - p.min(MAX_POWER)).abs() <= POWER_STEP);
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
                let d = decode(encode(3.0, Facing { cutoff, power }).unwrap()).unwrap();
                assert_eq!((d.facing.cutoff, d.facing.power, d.power), (cutoff, power, 3.0));
            }
        }
        // the extremes of the payload stay finite
        assert!(f32::from_bits(BASE + PAYLOAD_LIMIT - 1).is_finite());
    }

    #[test]
    fn plain_powers_decode_as_plain() {
        for v in [0.0, 0.5, 1.0, 13.2, 1000.0, 1048575.9, -1.0, -1048576.0, f32::INFINITY, f32::NAN, 1e30] {
            assert_eq!(decode(v), None, "{v}");
        }
    }

    #[test]
    fn a_decimal_decodes_to_its_f32_or_the_one_beside_it() {
        let d = decode(encode(1.0, Facing { cutoff: 0.3, power: 0.7 }).unwrap()).unwrap();
        assert!(d.facing.cutoff.to_bits().abs_diff(0.3f32.to_bits()) <= 1);
        assert!(d.facing.power.to_bits().abs_diff(0.7f32.to_bits()) <= 1);
    }

    #[test]
    fn unrepresentable_terms_are_refused() {
        assert!(encode(1.0, Facing { cutoff: 0.0, power: 0.25 }).is_err());
        assert!(encode(1.0, Facing { cutoff: 0.005, power: 1.0 }).is_err());
        assert!(encode(1.0, Facing { cutoff: 0.0, power: 0.0 }).is_err());
        assert!(encode(1.0, Facing { cutoff: 2.0, power: 1.0 }).is_err());
        assert!(encode(-1.0, Facing { cutoff: 0.0, power: 1.0 }).is_err());
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
