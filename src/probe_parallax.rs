// SPDX-License-Identifier: GPL-3.0-or-later
//! Box-projected (parallax-corrected) probe reflections, as 0.31a2 draws them, behind a scene marker.
//!
//! 0.31a2's box probes with `use_parallax_correction` look their specular cubemap up along the
//! reflection ray's exit point from the probe's box instead of along the ray itself: in the probe's
//! unit box (`world_to_outer_probe`), `t = min over axes of max((1 - p) / r, (-1 - p) / r)`, then the
//! direction `P + R t - probe_location` (its key digit 9; skhid `docs/notes/london.md`). 0.26g's box
//! programs have the same constants and draw the plain ray. Here each box program of
//! `environment_probe_lighting.ps` gets, before its specular cubemap sample, a branch taken only when
//! the scene's marker (`static_globals` row 18 `.w`, the environment's `mie_fog_base_height`) is one
//! of `markers`, which replaces the lookup direction with the box-projected one. Every native
//! instruction stays; the world position is copied to a new temporary where the program computes it;
//! `probe_location` is marked used in the reflection (the engine uploads it to every probe effect).
use anyhow::{Context, Result, bail, ensure};

use crate::weather_fog_021::{assemble, bytes, checksum, chunks, instructions, u32_at, words};

pub const FAMILY: &str = "shaders/sm_4_0/environment_probe_lighting.ps/";

/// A box permutation of the family (key digit 41, `ENV_PROBE_GEOMETRY_TYPE`).
pub fn is_box(key: &str) -> bool {
    key.as_bytes().get(41) == Some(&b'1')
}

/// The per-probe channel: a box permutation with smart attenuation (key digit 31, `LOD_INDEX`).
/// No London probe has smart attenuation in 0.31a2; London's box-projected probes get it set, and on
/// London's marked scenes these permutations draw the plain box falloff with the projected
/// reflection, so the projection reaches only the probes 0.31a2 projects. Elsewhere they are native.
pub fn is_channel(key: &str) -> bool {
    is_box(key) && key.as_bytes().get(31) == Some(&b'1')
}

/// Whether the permutation writes the probe's diffuse light (key digit 5, `TDIFFUSE`).
pub fn affects_diffuse(key: &str) -> bool {
    key.as_bytes().get(5) == Some(&b'1')
}

const DCL_TEMPS: u32 = 104;
const DP4: u32 = 17;
const SAMPLE: u32 = 69;
const SAMPLE_L: u32 = 72;

fn temp_dst(reg: u32, mask: u32) -> [u32; 2] {
    [0x0010_0002 | (mask << 4), reg]
}
fn temp_src(reg: u32, swizzle: u32) -> [u32; 2] {
    [0x0010_0006 | (swizzle << 4), reg]
}
fn temp_neg(reg: u32, swizzle: u32) -> [u32; 3] {
    [0x8010_0006 | (swizzle << 4), 0x41, reg]
}
fn select(reg: u32, comp: u32) -> [u32; 2] {
    [0x0010_000a | (comp << 4), reg]
}
fn cb(buffer: u32, row: u32, swizzle: u32) -> [u32; 3] {
    [0x0020_8006 | (swizzle << 4), buffer, row]
}
fn cb_neg(buffer: u32, row: u32, swizzle: u32) -> [u32; 4] {
    [0x8020_8006 | (swizzle << 4), 0x41, buffer, row]
}
fn imm4(v: [f32; 4]) -> [u32; 5] {
    [0x0000_4002, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()]
}
fn op(code: u32, operands: &[&[u32]]) -> Vec<u32> {
    let mut out = vec![0];
    for o in operands {
        out.extend_from_slice(o);
    }
    out[0] = code | ((out.len() as u32) << 24);
    out
}
const XYZX: u32 = 0x24;
const XYZW: u32 = 0xe4;
const XXXX: u32 = 0x00;

/// The swizzle that puts `xyz` of a source into the destination components `comps` (in order).
fn spread(comps: &[u32; 3]) -> u32 {
    let mut s = [0u32; 4];
    for (k, &c) in comps.iter().enumerate() {
        s[c as usize] = k as u32;
    }
    s[0] | (s[1] << 2) | (s[2] << 4) | (s[3] << 6)
}

/// The resource slot of `name` in a program's reflection (RDEF binding table).
fn binding(rdef: &[u8], name: &str) -> Result<u32> {
    let count = u32_at(rdef, 8)? as usize;
    let table = u32_at(rdef, 12)? as usize;
    for i in 0..count {
        let at = table + 32 * i;
        let label = u32_at(rdef, at)? as usize;
        let end = rdef[label..].iter().position(|b| *b == 0).context("unterminated name")?;
        if &rdef[label..label + end] == name.as_bytes() {
            return u32_at(rdef, at + 20);
        }
    }
    bail!("no binding {name}")
}

/// Marks the `$Globals` variable `name` used (D3D10_SVF_USED) in a reflection.
fn mark_used(rdef: &mut [u8], name: &str) -> Result<()> {
    let buffers = u32_at(rdef, 0)? as usize;
    let at = u32_at(rdef, 4)? as usize;
    for b in 0..buffers {
        let cb = at + 24 * b;
        let vars = u32_at(rdef, cb + 4)? as usize;
        let first = u32_at(rdef, cb + 8)? as usize;
        for v in 0..vars {
            let var = first + 24 * v;
            let label = u32_at(rdef, var)? as usize;
            let end = rdef[label..].iter().position(|b| *b == 0).context("unterminated name")?;
            if &rdef[label..label + end] == name.as_bytes() {
                let flags = u32_at(rdef, var + 12)? | 2;
                rdef[var + 12..var + 16].copy_from_slice(&flags.to_le_bytes());
                return Ok(());
            }
        }
    }
    bail!("no variable {name}")
}

const RET: u32 = 62;

/// A channel program (its include records, then the DXBC) with the marked branches: the
/// box-projected lookup before its specular sample, and before `ret` the plain box falloff
/// (0.26g's non-smart one) in the alpha it writes (`o1.w`, and `o0.w` when it writes diffuse).
pub fn build(blob: &[u8], markers: &[f32], diffuse: bool) -> Result<Vec<u8>> {
    ensure!(!markers.is_empty() && markers.len() <= 4, "one to four markers");
    let offset = 4 + u32_at(blob, 0)? as usize * 264;
    let code = blob.get(offset..).context("truncated shader header")?;
    ensure!(checksum(code)? == code[4..20], "shipped DXBC checksum mismatch");
    let mut entries = chunks(code)?;
    let ri = entries.iter().position(|(t, _)| t == b"RDEF").context("no reflection")?;
    let specular = binding(&entries[ri].1, "t_probe_cubemap")?;
    mark_used(&mut entries[ri].1, "probe_location")?;
    let si = entries.iter().position(|(t, _)| t == b"SHDR").context("no program")?;
    let w = words(&entries[si].1)?;
    let ops = instructions(&w)?;
    let temps = ops
        .iter()
        .find(|o| o[0] & 2047 == DCL_TEMPS)
        .map(|o| o[1])
        .context("no dcl_temps")?;
    let (s, a, b, c) = (temps, temps + 1, temps + 2, temps + 3);
    // The world position: the program's `dp4 rP.z, cb1[13], ...` (m_V2W's third row).
    let p_at = ops
        .iter()
        .position(|o| o[0] & 2047 == DP4 && o[1] == 0x0010_0042 && o[3] == 0x0020_8e46 && o[4] == 1 && o[5] == 13)
        .context("no world position")?;
    let p = ops[p_at][2];
    // The specular cubemap sample: its direction register and swizzle.
    // (A two-lobe program samples it twice along the same direction: one correction serves both.)
    let samples: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o[0] & 2047, SAMPLE | SAMPLE_L) && o[5] == 0x0010_7e46 && o[6] == specular)
        .map(|(i, _)| i)
        .collect();
    let (&sample_at, &last) = (samples.first().context("no specular cubemap sample")?, samples.last().unwrap());
    let sample = &ops[sample_at];
    for &i in &samples {
        ensure!(ops[i][3..5] == sample[3..5], "specular samples along different directions");
    }
    let dir_swz = (sample[3] >> 4) & 0xff;
    let dir_mask = (0..3).fold(0, |m, k| m | (1 << ((dir_swz >> (2 * k)) & 3)));
    for o in &ops[sample_at..last] {
        let writes_d = o.len() > 2
            && o[1] & 0x000f_f00f == 0x0000_0002
            && o[1] & 0x0030_0000 == 0x0010_0000
            && o[2] == sample[4]
            && (o[1] >> 4) & 0xf & dir_mask != 0;
        ensure!(!writes_d, "the direction changes between the specular samples");
    }
    ensure!(sample_at > p_at, "the specular sample comes before the world position");
    ensure!(sample[3] & 0xfff0_000f == 0x0010_0006, "the direction is not a swizzled temporary");
    let d = sample[4];
    let swz = (sample[3] >> 4) & 0xff;
    let comps = [swz & 3, (swz >> 2) & 3, (swz >> 4) & 3];
    ensure!(comps[0] != comps[1] && comps[1] != comps[2] && comps[0] != comps[2], "a repeated direction component");
    let mask = comps.iter().fold(0, |m, c| m | (1 << c));
    let spread_swz = spread(&comps);

    let guard = || {
        let mut g = Vec::new();
        for (i, m) in markers.iter().enumerate() {
            let r = if i == 0 { a } else { b };
            g.push(op(0x18, &[&temp_dst(r, 1), &[0x0020_803a, 1, 18], &[0x0000_4001, m.to_bits()]]));
            if i > 0 {
                g.push(op(0x3c, &[&temp_dst(a, 1), &select(a, 0), &select(b, 0)]));
            }
        }
        g.push(vec![0x0304_001f, 0x0010_000a, a]);
        g
    };
    let mut branch = guard();
    for (row, comp) in [(8, 1), (9, 2), (10, 4)] {
        branch.push(op(0x10, &[&temp_dst(a, comp), &cb(0, row, XYZX), &temp_src(d, swz)]));
    }
    branch.push(op(0x36, &[&temp_dst(s, 8), &[0x0000_4001, 1.0f32.to_bits()]]));
    for (row, comp) in [(8, 1), (9, 2), (10, 4)] {
        branch.push(op(0x11, &[&temp_dst(b, comp), &cb(0, row, XYZW), &temp_src(s, XYZW)]));
    }
    branch.push(op(0x00, &[&temp_dst(c, 7), &temp_neg(b, XYZX), &imm4([1.0, 1.0, 1.0, 0.0])]));
    branch.push(op(0x0e, &[&temp_dst(c, 7), &temp_src(c, XYZX), &temp_src(a, XYZX)]));
    branch.push(op(0x00, &[&temp_dst(b, 7), &temp_neg(b, XYZX), &imm4([-1.0, -1.0, -1.0, 0.0])]));
    branch.push(op(0x0e, &[&temp_dst(b, 7), &temp_src(b, XYZX), &temp_src(a, XYZX)]));
    branch.push(op(0x34, &[&temp_dst(b, 7), &temp_src(b, XYZX), &temp_src(c, XYZX)]));
    branch.push(op(0x33, &[&temp_dst(c, 1), &select(b, 2), &select(b, 1)]));
    branch.push(op(0x33, &[&temp_dst(c, 1), &select(c, 0), &select(b, 0)]));
    branch.push(op(0x32, &[&temp_dst(d, mask), &temp_src(d, XYZW), &temp_src(c, XXXX), &temp_src(s, spread_swz)]));
    branch.push(op(0x00, &[&temp_dst(d, mask), &temp_src(d, XYZW), &cb_neg(0, 19, spread_swz)]));
    branch.push(vec![0x0100_0015]);

    // The plain box falloff (0.26g's non-smart box program): p = world_to_outer_probe * P,
    // t = sat((outer * |p| - inner) / (outer - inner)), alpha = prod(1 - t*t*(3 - 2t)).
    let mut falloff = guard();
    falloff.push(op(0x36, &[&temp_dst(s, 8), &[0x0000_4001, 1.0f32.to_bits()]]));
    for (row, comp) in [(8, 1), (9, 2), (10, 4)] {
        falloff.push(op(0x11, &[&temp_dst(b, comp), &cb(0, row, XYZW), &temp_src(s, XYZW)]));
    }
    falloff.push(op(0x32, &[&temp_dst(b, 7), &cb(0, 17, XYZX), &[0x8010_0006 | (XYZX << 4), 0x81, b], &cb_neg(0, 18, XYZX)]));
    falloff.push(op(0x00, &[&temp_dst(c, 7), &cb(0, 17, XYZX), &cb_neg(0, 18, XYZX)]));
    falloff.push(op(0x0e | 0x2000, &[&temp_dst(b, 7), &temp_src(b, XYZX), &temp_src(c, XYZX)]));
    falloff.push(op(0x32, &[&temp_dst(c, 7), &temp_src(b, XYZX), &imm4([-2.0, -2.0, -2.0, 0.0]), &imm4([3.0, 3.0, 3.0, 0.0])]));
    falloff.push(op(0x38, &[&temp_dst(b, 7), &temp_src(b, XYZX), &temp_src(b, XYZX)]));
    falloff.push(op(0x32, &[&temp_dst(b, 7), &temp_neg(c, XYZX), &temp_src(b, XYZX), &imm4([1.0, 1.0, 1.0, 0.0])]));
    falloff.push(op(0x38, &[&temp_dst(b, 1), &select(b, 1), &select(b, 0)]));
    falloff.push(op(0x38, &[&temp_dst(b, 1), &select(b, 2), &select(b, 0)]));
    falloff.push(op(0x36, &[&[0x0010_2082, 1], &select(b, 0)]));
    if diffuse {
        falloff.push(op(0x36, &[&[0x0010_2082, 0], &select(b, 0)]));
    }
    falloff.push(vec![0x0100_0015]);
    let ret_at = ops.len() - 1;
    ensure!(ops[ret_at][0] & 2047 == RET, "the program does not end in ret");
    ensure!(
        ops.iter().filter(|o| o[0] & 2047 == RET).count() == 1,
        "more than one ret"
    );

    let mut out = w[..2].to_vec();
    for (i, o) in ops.iter().enumerate() {
        if o[0] & 2047 == DCL_TEMPS {
            out.extend([o[0], temps + 4]);
            continue;
        }
        if i == sample_at {
            out.extend(branch.iter().flatten());
        }
        if i == ret_at {
            out.extend(falloff.iter().flatten());
        }
        out.extend(o);
        if i == p_at {
            out.extend(op(0x36, &[&temp_dst(s, 7), &temp_src(p, XYZX)]));
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
    fn encodings_match_the_compiler() {
        // mad r2.xyz, cb0[17].xyzx, |r2.xyzx|, -cb0[18].xyzx from a shipped box program
        assert_eq!(temp_dst(2, 7), [0x0010_0072, 2]);
        assert_eq!(cb(0, 17, XYZX), [0x0020_8246, 0, 17]);
        assert_eq!(cb_neg(0, 18, XYZX), [0x8020_8246, 0x41, 0, 18]);
        // dp4 r3.z, cb1[13].xyzw, r2.xyzw
        assert_eq!(
            op(0x11, &[&temp_dst(3, 4), &cb(1, 13, XYZW), &temp_src(2, XYZW)]),
            vec![0x0800_0011, 0x0010_0042, 3, 0x0020_8e46, 1, 13, 0x0010_0e46, 2]
        );
        // the direction r0.yzwy
        assert_eq!(temp_src(0, 0x79), [0x0010_0796, 0]);
        assert_eq!(select(1, 0), [0x0010_000a, 1]);
    }

    #[test]
    fn spread_puts_xyz_into_the_direction_components() {
        // direction .yzw: y <- x, z <- y, w <- z
        assert_eq!(spread(&[1, 2, 3]), 0x90);
        assert_eq!(spread(&[0, 1, 2]), 0x24);
    }

    #[test]
    fn keys() {
        assert!(is_box("___0_1____________________00___0_________1_________________________"));
        assert!(!is_box("___0_1____________________00___0_________0_________________________"));
    }
}
