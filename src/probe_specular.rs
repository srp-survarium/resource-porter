// SPDX-License-Identifier: GPL-3.0-or-later
//! Reflection-only environment probes, as 0.31a2 draws them.
//!
//! A probe with `affect_diffuse = false` selects the `TDIFFUSE` 0 permutations of
//! `environment_probe_lighting.ps` (key digit 5). 0.26g's write the surface's own albedo into
//! the diffuse accumulator (RT0) under the probe's falloff (skhid `renderer-026g.md` 5.3), so a
//! wall inside such a probe takes its colour as ambient light. 0.31a2's write only the specular
//! accumulator. No 0.26g map has such a probe; London (from 0.31a2) has ten window probes.
//!
//! The probes blend each target SRC_ALPHA / INV_SRC_ALPHA with its own output's alpha, so the
//! one change is the diffuse output's alpha: `mov o0.w, <falloff>` becomes `mov o0.w, l(0.0)`,
//! which leaves RT0 as it was. The specular output, its alpha and every other instruction are
//! unchanged; the instruction keeps its length, and the DXBC checksum is recomputed.
use anyhow::{Context, Result, bail, ensure};

use crate::weather_fog_021::{assemble, bytes, checksum, chunks, instructions, u32_at, words};

pub const FAMILY: &str = "shaders/sm_4_0/environment_probe_lighting.ps/";

/// A permutation key of the family drawn for `affect_diffuse = false`.
pub fn is_reflection_only(key: &str) -> bool {
    key.as_bytes().get(5) == Some(&b'0')
}

const MOV: u32 = 54;
/// `o0.w`: a four-component output operand, mask w, one immediate index.
const O0_W: u32 = 0x0010_2082;
/// `l(x)`: a one-component 32-bit immediate.
const IMMEDIATE_1: u32 = 0x0000_4001;

/// A game shader file (its include records, then the DXBC) with the diffuse output's alpha zero.
pub fn build(blob: &[u8]) -> Result<Vec<u8>> {
    let offset = 4 + u32_at(blob, 0)? as usize * 264;
    let code = blob.get(offset..).context("truncated shader header")?;
    ensure!(checksum(code)? == code[4..20], "shipped DXBC checksum mismatch");
    let mut entries = chunks(code)?;
    let mut changed = 0;
    for (tag, data) in &mut entries {
        if tag != b"SHDR" {
            continue;
        }
        let body = instructions(&words(data)?)?;
        let mut out = words(data)?[..2].to_vec();
        for mut i in body {
            if i[0] & 2047 == MOV && i.len() == 5 && i[1] == O0_W && i[2] == 0 {
                ensure!(i[3] & 0x000F_F000 == 0, "o0.w takes {:#x}, not a temporary", i[3]);
                i[3] = IMMEDIATE_1;
                i[4] = 0;
                changed += 1;
            }
            out.extend(i);
        }
        *data = bytes(&out);
    }
    match changed {
        1 => {}
        n => bail!("{n} writes of o0.w, expected one"),
    }
    let mut result = blob[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader(instructions: &[&[u32]]) -> Vec<u8> {
        let mut shdr = vec![0x0000_0040, 0];
        for i in instructions {
            shdr.extend_from_slice(i);
        }
        shdr[1] = shdr.len() as u32;
        let mut code = b"DXBC".to_vec();
        code.extend([0u8; 16]);
        code.extend(1u32.to_le_bytes());
        code.extend(0u32.to_le_bytes());
        code.extend(1u32.to_le_bytes());
        code.extend(36u32.to_le_bytes());
        let entries = vec![(*b"SHDR", bytes(&shdr))];
        let code = assemble(&code, &entries).unwrap();
        let mut blob = 0u32.to_le_bytes().to_vec();
        blob.extend(code);
        blob
    }

    #[test]
    fn the_diffuse_alpha_becomes_zero_and_nothing_else_changes() {
        let o1_w = O0_W; // o1.w: the same operand token, index 1
        let input = shader(&[
            &[0x0500_0036, O0_W, 0, 0x0010_000A, 0], // mov o0.w, r0.x
            &[0x0500_0036, o1_w, 1, 0x0010_000A, 0], // mov o1.w, r0.x
            &[0x0100_003E],                          // ret
        ]);
        let out = build(&input).unwrap();
        assert_eq!(out.len(), input.len());
        let code = &out[4..];
        assert_eq!(checksum(code).unwrap(), code[4..20]);
        let shdr = &chunks(code).unwrap()[0].1;
        let body = instructions(&words(shdr).unwrap()).unwrap();
        assert_eq!(body[0], vec![0x0500_0036, O0_W, 0, IMMEDIATE_1, 0]);
        assert_eq!(body[1], vec![0x0500_0036, o1_w, 1, 0x0010_000A, 0]);
    }

    #[test]
    fn a_shader_without_the_write_is_refused() {
        assert!(build(&shader(&[&[0x0100_003E]])).is_err());
    }

    #[test]
    fn keys() {
        assert!(is_reflection_only("___0_0____________________00___0_________0"));
        assert!(!is_reflection_only("___0_1____________________00___0_________0"));
    }
}
