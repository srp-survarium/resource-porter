//! Decode painted-alpha selectors to the original ordinary shadow threshold.
//! All original alpha-discard depth programs use base alpha - alpha_ref; none
//! uses the visible pass's painted-alpha equation. Native sampling stays intact.
use super::{
    assemble, bytes, chunks, instructions, literal, op, pinned_code, profiles::AuthoredProfile,
    words,
};
use crate::ResourceDb;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Program {
    sha256: String,
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    original: Vec<Program>,
    native: Vec<Program>,
}

fn cutoff() -> Vec<u32> {
    vec![
        0x09000000, 0x00100012, 0, 0x0010003a, 0, 0x8020802a, 0x41, 0, 15,
    ]
}

/// Check the entire reviewed executable instruction sequence, including the
/// shared cb0[15].z alpha_ref ABI. Only the sampled UV varies across layouts.
fn validate(ops: &[Vec<u32>]) -> Result<usize> {
    let executable: Vec<_> = ops.iter().filter(|o| (o[0] & 2047) < 88).collect();
    ensure!(executable.len() == 6, "depth executable sequence changed");
    let sample = executable[0];
    let texture_at = match sample.len() {
        11 => {
            ensure!(
                sample[0] == 0x0b000048 && (sample[3] >> 12) & 255 == 1,
                "depth vertex UV changed"
            );
            5
        }
        14 => {
            ensure!(
                sample[0] == 0x0e000048 && sample[3..8] == [0x00004002, 0, 0, 0, 0],
                "depth constant UV changed"
            );
            8
        }
        _ => anyhow::bail!("depth sample length changed"),
    };
    ensure!(
        sample[1..3] == [0x001000f2, 0]
            && sample[texture_at..] == [0x00107e46, 0, 0x00106000, 0, 0x00004001, 0],
        "depth base-alpha sample changed"
    );
    ensure!(*executable[1] == cutoff(), "depth cutoff changed");
    ensure!(
        *executable[2] == [0x07000031, 0x00100012, 0, 0x0010000a, 0, 0x00004001, 0],
        "depth comparison changed"
    );
    ensure!(
        *executable[3] == [0x0304000d, 0x0010000a, 0],
        "depth discard changed"
    );
    ensure!(
        *executable[4] == [0x08000036, 0x001020f2, 0, 0x00004002, 0, 0, 0, 0]
            && *executable[5] == [0x0100003e],
        "depth output changed"
    );
    ops.iter()
        .position(|o| *o == cutoff())
        .context("depth cutoff")
}

fn convert(blob: &[u8], sha256: &str) -> Result<Vec<u8>> {
    let (offset, native) = pinned_code(blob, sha256)?;
    let mut entries = chunks(native)?;
    let index = entries
        .iter()
        .position(|(t, _)| t == b"SHDR")
        .context("depth SHDR")?;
    let old = words(&entries[index].1)?;
    let mut ops = instructions(&old)?;
    let at = validate(&ops)?;
    let temps = ops
        .iter_mut()
        .find(|o| o[0] == 0x02000068)
        .context("depth temps")?;
    ensure!(temps[1] == 1, "depth temporary layout changed");
    temps[1] = 2;
    let alpha_ref = [0x0020802a, 0, 15];
    let threshold_dst = [0x00100012, 1];
    let threshold_src = [0x0010000a, 1];
    let condition_dst = [0x00100022, 1];
    let condition_src = [0x0010001a, 1];
    let mut replacement = vec![op(54, &[&threshold_dst, &alpha_ref])];
    for profile in AuthoredProfile::ALL {
        replacement.push(op(
            24,
            &[&condition_dst, &alpha_ref, &literal(profile.marker())],
        ));
        replacement.push(op(
            55,
            &[
                &threshold_dst,
                &condition_src,
                &literal(profile.shadow_threshold()),
                &threshold_src,
            ],
        ));
    }
    replacement.push(op(
        0,
        &[&[0x00100012, 0], &[0x0010003a, 0], &[0x8010000a, 0x41, 1]],
    ));
    ops.splice(at..=at, replacement);
    let mut program = old[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let mut output = blob[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

/// Returns (native base hash, replacement bytes) for all 78 alpha-tested paths.
/// Read both donor and stock at build time; no released patch supplies shaders.
pub fn build(
    donor: &mut ResourceDb,
    stock: &mut ResourceDb,
) -> Result<BTreeMap<String, (String, Vec<u8>)>> {
    let catalog: Catalog = serde_json::from_str(include_str!("depth_programs.json"))?;
    ensure!(
        catalog.original.len() == 14 && catalog.native.len() == 21,
        "depth catalog coverage"
    );
    for original in catalog.original {
        for path in original.paths {
            let blob = donor.read(&path)?;
            let (_, code) = pinned_code(&blob, &original.sha256)?;
            let entries = chunks(code)?;
            let program = &entries
                .iter()
                .find(|(t, _)| t == b"SHDR")
                .context("original depth SHDR")?
                .1;
            validate(&instructions(&words(program)?)?)?;
        }
    }
    let mut output = BTreeMap::new();
    for native in catalog.native {
        for path in native.paths {
            let blob = stock.read(&path)?;
            let replacement = convert(&blob, &native.sha256)?;
            ensure!(
                output
                    .insert(path, (native.sha256.clone(), replacement))
                    .is_none(),
                "duplicate depth path"
            );
        }
    }
    ensure!(output.len() == 78, "depth path coverage");
    Ok(output)
}
