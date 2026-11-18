//! Native permutations required by the original 0.21 dirt alpha-test contract.
//! These paths are absent from stock; no existing native program is replaced.
use super::{assemble, bytes, chunks, instructions, pinned_code, u32_at, words};
use crate::ResourceDb;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeSet;

const PREFIX: &str = "shaders/sm_4_0/gbuffer_pass.ps/";
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mask {
    Vertex,
    Zero,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    vertex: u8,
    mask: Mask,
    high: String,
    normal: String,
    low: String,
    alpha: String,
}

fn key(quality: u8, vertex: u8, alias: &str) -> String {
    format!("_{quality}___1000_10001_00____{vertex}_____0__{alias}00____00000____000__________0__")
}
fn selector(key: &str, at: usize, value: u8) -> Result<String> {
    let mut bytes = key.as_bytes().to_vec();
    ensure!(
        bytes[at] == b'0' || bytes[at] == b'1',
        "shader selector changed"
    );
    bytes[at] = value;
    Ok(String::from_utf8(bytes)?)
}

fn add_alpha(high: &[u8], alpha_normal: &[u8], variant: &Variant) -> Result<Vec<u8>> {
    let (offset, code) = pinned_code(high, &variant.high)?;
    let (_, reference) = pinned_code(alpha_normal, &variant.normal)?;
    let mut entries = chunks(code)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("SHDR")?;
    let old_words = words(&entries[index].1)?;
    let mut ops = instructions(&old_words)?;
    // Native mask-only cutoff: r0.w = mask - .4. The base sample already
    // retains alpha in r2.w; material alpha_ref lives at cb1[15].z.
    // Verify the formula against the shipped alpha+parallax+normal program.
    let reference_chunks = chunks(reference)?;
    let reference_ops = instructions(&words(
        &reference_chunks
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("reference SHDR")?
            .1,
    )?)?;
    match variant.mask {
        Mask::Vertex => {
            let cutoff = vec![
                0x07000000, 0x00100082, 0, 0x0010003a, 0, 0x00004001, 0xbecccccd,
            ];
            ensure!(
                ops.iter().filter(|op| **op == cutoff).count() == 1,
                "ambiguous dirt cutoff"
            );
            let at = ops
                .iter()
                .position(|op| *op == cutoff)
                .context("dirt cutoff")?;
            ensure!(
                ops[at + 2] == [0x0304000d, 0x0010003a, 0],
                "dirt discard changed"
            );
            ensure!(
                reference_ops.contains(&vec![
                    0x0b000032, 0x00100042, 0, 0x0010002a, 0, 0x0010000a, 2, 0x8020802a, 0x41, 1,
                    15
                ]),
                "native alpha formula changed"
            );
            ops[at] = vec![
                0x0b000032, 0x00100082, 0, 0x0010003a, 0, 0x0010003a, 2, 0x8020802a, 0x41, 1, 15,
            ];
        }
        Mask::Zero => {
            // These shipped vertex classes supply no vertex-alpha mask.
            // Their alpha-enabled neighbors compile 0*alpha-alpha_ref < 0.
            let matching: Vec<_> = reference_ops
                .windows(2)
                .filter(|pair| {
                    let op = &pair[0];
                    op.len() == 9
                        && op[0] == 0x09000031
                        && op[3..] == [0x8020802a, 0x41, 1, 15, 0x00004001, 0]
                        && pair[1][0] == 0x0304000d
                })
                .collect();
            ensure!(
                matching.len() == 1,
                "native zero-mask alpha formula changed"
            );
            let cutoff = vec![0x0304000d, 0x00004001, u32::MAX];
            ensure!(
                ops.iter().filter(|op| **op == cutoff).count() == 1,
                "ambiguous zero-mask discard"
            );
            let at = ops
                .iter()
                .position(|op| *op == cutoff)
                .context("zero-mask discard")?;
            let ti = ops
                .iter()
                .position(|op| op[0] & 2047 == 104)
                .context("temporaries")?;
            let scratch = ops[ti][1];
            ensure!(scratch < 64, "unexpected temporary count");
            ops[ti][1] += 1;
            ops.splice(
                at..at + 1,
                [
                    vec![
                        0x09000031, 0x00100012, scratch, 0x8020802a, 0x41, 1, 15, 0x00004001, 0,
                    ],
                    vec![0x0304000d, 0x0010000a, scratch],
                ],
            );
        }
    }
    let mut program = old_words[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let reflection = &mut entries
        .iter_mut()
        .find(|(tag, _)| tag == b"RDEF")
        .context("RDEF")?
        .1;
    let mut variables = Vec::new();
    for buffer in 0..u32_at(reflection, 0)? as usize {
        let at = u32_at(reflection, 4)? as usize + buffer * 24;
        for field in 0..u32_at(reflection, at + 4)? as usize {
            let variable = u32_at(reflection, at + 8)? as usize + field * 24;
            let name = u32_at(reflection, variable)? as usize;
            if reflection.get(name..name + 20) == Some(b"alpha_ref_parameter\0") {
                variables.push(variable);
            }
        }
    }
    ensure!(variables.len() == 1, "ambiguous alpha reflection");
    let variable = variables[0];
    let name = u32_at(reflection, variable)? as usize;
    ensure!(
        reflection.get(name..name + 20) == Some(b"alpha_ref_parameter\0")
            && u32_at(reflection, variable + 4)? == 248
            && u32_at(reflection, variable + 8)? == 4
            && u32_at(reflection, variable + 12)? == 0,
        "alpha reflection changed"
    );
    // The engine discovers uploaded constants from reflection. Match the
    // existing alpha variant's D3D_SVF_USED flag, not just the shader opcode.
    reflection[variable + 12..variable + 16].copy_from_slice(&2u32.to_le_bytes());
    let mut result = high[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(result)
}

pub fn build(native: &mut ResourceDb) -> Result<Vec<(String, Vec<u8>)>> {
    let mut output = Vec::new();
    let variants: Vec<Variant> = serde_json::from_str(include_str!("visibility-pins.json"))?;
    ensure!(
        variants.iter().map(|v| v.vertex).collect::<BTreeSet<_>>()
            == BTreeSet::from([0, 1, 2, 10, 12, 13, 14])
            && variants.len() == 7,
        "native vertex family changed"
    );
    for variant in variants {
        let low_key = key(0, variant.vertex, "0_021");
        let high_key = key(3, variant.vertex, "0_021");
        let low = native.read(&format!("{PREFIX}{low_key}"))?;
        pinned_code(&low, &variant.low)?;
        let plain = selector(&low_key, 10, b'0')?;
        ensure!(
            low == native.read(&format!("{PREFIX}{plain}"))?,
            "0-prefixed parallax is no longer inactive"
        );
        let alpha = native.read(&format!("{PREFIX}{}", selector(&plain, 8, b'1')?))?;
        pinned_code(&alpha, &variant.alpha)?;
        let high_source = native.read(&format!("{PREFIX}{high_key}"))?;
        let normal = selector(&selector(&high_key, 6, b'1')?, 8, b'1')?;
        let high = add_alpha(
            &high_source,
            &native.read(&format!("{PREFIX}{normal}"))?,
            &variant,
        )?;
        for alias in ["0_021", "2_121"] {
            for (quality, source, blob) in [(0, &low, &alpha), (3, &high_source, &high)] {
                let source_key = key(quality, variant.vertex, alias);
                ensure!(
                    *source == native.read(&format!("{PREFIX}{source_key}"))?,
                    "native alias differs"
                );
                let path = format!("{PREFIX}{}", selector(&source_key, 8, b'1')?);
                if native.contains(&path) {
                    ensure!(
                        native.read(&path)? == *blob,
                        "conflicting visibility shader: {path}"
                    );
                } else {
                    output.push((path, blob.clone()));
                }
            }
        }
    }
    Ok(output)
}
