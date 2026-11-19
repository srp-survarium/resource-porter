// SPDX-License-Identifier: GPL-3.0-or-later
//! A scene's own rain-shelter darkening in the composition, behind its marker.
//!
//! In rain, 0.26g's composition darkens what the rain does not reach by a fixed 0.85
//! (`mad r, r, l(5.0), l(-0.85)` then `mad r, s, r, l(0.85)`); 0.31a2 reads the factor from the
//! environment (`rain_shadow_diffuse_multiplier`), and London's rain authors 0.7. A shelter
//! profile runs the same two instructions with its own factor where the scene's marker
//! (`static_globals` row 18 `.w`) is its marker; elsewhere the native pair runs unchanged.
use super::{assemble, bytes, chunks, instructions, u32_at, words};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShelterProfile {
    pub marker: f32,
    pub factor: f32,
}

const NATIVE: f32 = 0.85;
const MAD: u32 = 50;
const DCL_TEMPS: u32 = 104;

fn replaced(op: &[u32], from: f32, to: f32) -> Result<Vec<u32>> {
    let mut out = op.to_vec();
    let mut n = 0;
    for i in 1..out.len() - 1 {
        if out[i] == 0x0000_4001 && out[i + 1] == from.to_bits() {
            out[i + 1] = to.to_bits();
            n += 1;
        }
    }
    ensure!(n == 1, "{n} immediates {from} in the shelter instruction");
    Ok(out)
}

/// A composition program (include records, then DXBC) with the profiles' branches.
pub fn build_shelter(blob: &[u8], profiles: &[ShelterProfile]) -> Result<Vec<u8>> {
    ensure!(!profiles.is_empty(), "no shelter profiles");
    for p in profiles {
        ensure!(p.marker.is_finite() && p.marker >= 1000.0 && p.marker.fract() == 0.0, "invalid shelter marker");
        ensure!(p.factor.is_finite() && p.factor > 0.0 && p.factor <= 1.0, "invalid shelter factor");
    }
    let offset = 4 + u32_at(blob, 0)? as usize * 264;
    let code = blob.get(offset..).context("truncated shader header")?;
    let mut entries = chunks(code)?;
    let si = entries.iter().position(|(t, _)| t == b"SHDR").context("no program")?;
    let w = words(&entries[si].1)?;
    let ops = instructions(&w)?;
    let has = |o: &Vec<u32>, v: f32| o.windows(2).any(|x| x[0] == 0x0000_4001 && x[1] == v.to_bits());
    let at = ops
        .windows(2)
        .position(|p| p[0][0] & 2047 == MAD && p[1][0] & 2047 == MAD && has(&p[0], -NATIVE) && has(&p[0], 5.0) && has(&p[1], NATIVE))
        .context("no rain-shelter pair")?;
    ensure!(
        ops.iter().filter(|o| o[0] & 2047 == MAD && has(o, NATIVE)).count() == 1,
        "more than one rain-shelter pair"
    );
    let temps = ops.iter().find(|o| o[0] & 2047 == DCL_TEMPS).map(|o| o[1]).context("no dcl_temps")?;
    let t = temps;
    let mut out = w[..2].to_vec();
    for (i, o) in ops.iter().enumerate() {
        if o[0] & 2047 == DCL_TEMPS {
            out.extend([o[0], temps + 1]);
            continue;
        }
        if i == at {
            for p in profiles {
                out.extend([0x0800_0018, 0x0010_0012, t, 0x0020_803a, 1, 18, 0x0000_4001, p.marker.to_bits()]);
                out.extend([0x0304_001f, 0x0010_000a, t]);
                out.extend(replaced(&ops[at], -NATIVE, -p.factor)?);
                out.extend(replaced(&ops[at + 1], NATIVE, p.factor)?);
                out.push(0x0100_0012);
            }
        }
        out.extend(o);
        if i == at + 1 {
            out.extend(std::iter::repeat_n(0x0100_0015, profiles.len()));
        }
    }
    out[1] = out.len() as u32;
    instructions(&out)?;
    entries[si].1 = bytes(&out);
    let mut result = blob[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_checked() {
        for p in [
            ShelterProfile { marker: 60.0, factor: 0.7 },
            ShelterProfile { marker: 32919.5, factor: 0.7 },
            ShelterProfile { marker: 32919.0, factor: 0.0 },
            ShelterProfile { marker: 32919.0, factor: 1.5 },
        ] {
            assert!(build_shelter(&[0; 8], &[p]).is_err());
        }
    }

    #[test]
    fn the_factor_is_replaced_once() {
        let a = vec![0x0b00_0032, 0x0010_0012, 0, 0x0010_000a, 0, 0x0000_4001, 5.0f32.to_bits(), 0x0000_4001, (-0.85f32).to_bits()];
        let b = replaced(&a, -0.85, -0.7).unwrap();
        assert_eq!(b[8], (-0.7f32).to_bits());
        assert_eq!(b[..8], a[..8]);
        assert!(replaced(&a, 0.85, 0.7).is_err());
    }
}
