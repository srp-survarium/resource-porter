//! Missing native normal-blend permutations, derived from pinned native neighbors.
//! Existing programs are never replaced. Native shader ABI and finish stay intact.
use super::{MaskedPrograms, assemble, bytes, chunks, instructions, words};
use crate::ResourceDb;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    Base,
    Blended,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Conversion {
    MaskOff { mask_slot: u32 },
    AlphaOn { coverage: Coverage },
    MaskOffAlphaOn { mask_slot: u32, coverage: Coverage },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    path: String,
    source_path: String,
    source_sha256: String,
    base_sampler: u32,
    conversion: Conversion,
}

fn word(raw: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        raw.get(at..at + 4)
            .context("reflection bounds")?
            .try_into()?,
    ))
}

fn name(raw: &[u8], at: usize) -> Result<&str> {
    let start = word(raw, at)? as usize;
    let tail = raw.get(start..).context("reflection name bounds")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("reflection terminator")?;
    Ok(std::str::from_utf8(&tail[..end])?)
}

fn variable_usage(
    raw: &mut [u8],
    target: &str,
    offset: u32,
    before: u32,
    after: u32,
) -> Result<()> {
    let mut matches = Vec::new();
    for i in 0..word(raw, 0)? as usize {
        let cb = word(raw, 4)? as usize + i * 24;
        for j in 0..word(raw, cb + 4)? as usize {
            let at = word(raw, cb + 8)? as usize + j * 24;
            if name(raw, at)? == target {
                matches.push(at);
            }
        }
    }
    ensure!(matches.len() == 1, "ambiguous material constant: {target}");
    let at = matches[0];
    ensure!(
        word(raw, at + 4)? == offset && word(raw, at + 12)? == before,
        "material constant ABI changed: {target}"
    );
    raw[at + 12..at + 16].copy_from_slice(&after.to_le_bytes());
    Ok(())
}

fn remove_mask_binding(raw: &mut [u8], slot: u32) -> Result<()> {
    let count = word(raw, 8)? as usize;
    let start = word(raw, 12)? as usize;
    let mut matches = Vec::new();
    for i in 0..count {
        let at = start + i * 32;
        if name(raw, at)? == "t_vertex_blended_tiled_mask" {
            ensure!(
                word(raw, at + 4)? == 2 && word(raw, at + 20)? == slot && word(raw, at + 24)? == 1,
                "mask resource ABI changed"
            );
            matches.push(i);
        }
    }
    ensure!(matches.len() == 1, "ambiguous tiled mask binding");
    let at = start + matches[0] * 32;
    raw.copy_within(at + 32..start + count * 32, at);
    raw[start + (count - 1) * 32..start + count * 32].fill(0);
    raw[8..12].copy_from_slice(&u32::try_from(count - 1)?.to_le_bytes());
    variable_usage(raw, "vertex_blend_tiled_mask_parameters", 208, 2, 0)
}

fn base_alpha_sample(destination: &[u32], sampler: u32) -> Vec<u32> {
    vec![
        0x09000045,
        destination[0],
        destination[1],
        0x00101046,
        7,
        0x00107ff6,
        0,
        0x00106000,
        sampler,
    ]
}

fn without_mask(ops: &mut Vec<Vec<u32>>, slot: u32, sampler: u32) -> Result<()> {
    let sites: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.len() == 9 && op[0] == 0x09000045 && op[6] == slot)
        .map(|(i, _)| i)
        .collect();
    ensure!(sites.len() == 1, "ambiguous tiled mask sample");
    let at = sites[0];
    ensure!(at >= 6, "missing mask coordinates");
    let kinds: Vec<_> = ops[at - 6..at].iter().map(|op| op[0] & 2047).collect();
    ensure!(
        kinds == [56, 0, 15, 15, 0, 56] || kinds == [0, 56, 15, 15, 0, 56],
        "mask coordinate sequence changed"
    );
    let sample = base_alpha_sample(&ops[at][1..3], sampler);
    // The catalog derivation independently proves every other component
    // written by these six coordinate instructions is dead after the sample.
    ops.splice(at - 6..=at, [sample]);
    let declarations: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op[0] & 2047 == 88 && op[2] == slot)
        .map(|(i, _)| i)
        .collect();
    ensure!(declarations.len() == 1, "ambiguous mask declaration");
    ops.remove(declarations[0]);
    Ok(())
}

fn with_alpha(ops: &mut Vec<Vec<u32>>, sampler: u32, coverage: Coverage) -> Result<()> {
    ensure!(
        !ops.iter().any(|op| op[0] & 2047 == 13),
        "source already alpha-tested"
    );
    let temps: Vec<_> = ops.iter().filter(|op| op[0] & 2047 == 104).collect();
    ensure!(
        temps.len() == 1 && temps[0][1] > 1,
        "temporary declaration changed"
    );
    // The existing original-blend adapter owns the last scratch register;
    // both of its guarded branches have completed at this insertion point.
    let scratch = temps[0][1] - 1;
    let sites: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.len() == 8 && op[0] == 0x08002038 && op[5..] == [0x0020801a, 0, 14])
        .map(|(i, _)| i)
        .collect();
    ensure!(sites.len() == 1, "blend weight changed");
    let at = sites[0];
    ensure!(
        ops[at + 1][0] & 2047 == 24 && ops[at + 24] == [0x01000015],
        "original blend override boundary changed"
    );
    let component = ((ops[at][1] >> 4) & 15).trailing_zeros();
    ensure!(component < 4, "weight component changed");
    let weight = [0x0010000a | (component << 4), ops[at][2]];
    let mut added = vec![base_alpha_sample(&[0x00100012, scratch], sampler)];
    if let Coverage::Blended = coverage {
        added.push(vec![
            0x07000000,
            0x00100012,
            scratch,
            0x0010000a,
            scratch,
            0x00004001,
            (-1.0f32).to_bits(),
        ]);
        added.push(vec![
            0x09000032,
            0x00100012,
            scratch,
            weight[0],
            weight[1],
            0x0010000a,
            scratch,
            0x00004001,
            1.0f32.to_bits(),
        ]);
    }
    added.extend([
        vec![
            0x09000000, 0x00100012, scratch, 0x0010000a, scratch, 0x8020802a, 0x41, 0, 15,
        ],
        vec![
            0x07000031, 0x00100012, scratch, 0x0010000a, scratch, 0x00004001, 0,
        ],
        vec![0x0304000d, 0x0010000a, scratch],
    ]);
    ops.splice(at + 25..at + 25, added);
    Ok(())
}

fn convert(blob: &[u8], contract: &Contract) -> Result<Vec<u8>> {
    let offset = 4 + word(blob, 0)? as usize * 264;
    let code = blob.get(offset..).context("shader wrapper")?;
    let mut entries = chunks(code)?;
    let shdr = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("SHDR")?;
    let rdef = entries
        .iter()
        .position(|(tag, _)| tag == b"RDEF")
        .context("RDEF")?;
    let old = words(&entries[shdr].1)?;
    let mut ops = instructions(&old)?;
    match contract.conversion {
        Conversion::MaskOff { mask_slot } | Conversion::MaskOffAlphaOn { mask_slot, .. } => {
            without_mask(&mut ops, mask_slot, contract.base_sampler)?;
            remove_mask_binding(&mut entries[rdef].1, mask_slot)?;
        }
        Conversion::AlphaOn { .. } => {}
    }
    match contract.conversion {
        Conversion::AlphaOn { coverage } | Conversion::MaskOffAlphaOn { coverage, .. } => {
            with_alpha(&mut ops, contract.base_sampler, coverage)?;
            variable_usage(&mut entries[rdef].1, "alpha_ref_parameter", 248, 0, 2)?;
        }
        Conversion::MaskOff { .. } => {}
    }
    let mut updated = old[..2].to_vec();
    updated.extend(ops.iter().flatten());
    updated[1] = u32::try_from(updated.len())?;
    instructions(&updated)?;
    entries[shdr].1 = bytes(&updated);
    let mut result = blob[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(result)
}

pub fn build(donor: &mut ResourceDb, stock: &mut ResourceDb) -> Result<BTreeMap<String, Vec<u8>>> {
    let linear = donor.read(super::SOURCE)?;
    let masked = donor.read(super::MASKED_SOURCE)?;
    let programs = MaskedPrograms::new(&linear, &masked)?;
    let source: BTreeMap<_, _> = programs.programs().map(|p| (p.path(), p)).collect();
    let contracts: Vec<Contract> = serde_json::from_str(include_str!("normal-on-programs.json"))?;
    ensure!(contracts.len() == 704, "incomplete normal-blend closure");
    let mut result = BTreeMap::new();
    for contract in contracts {
        ensure!(
            !stock.paths().any(|p| p == contract.path),
            "existing native shader: {}",
            contract.path
        );
        let program = source
            .get(contract.source_path.as_str())
            .context("unpinned native source")?;
        ensure!(
            program.native_sha256() == contract.source_sha256,
            "source fingerprint differs"
        );
        let native = stock.read(&contract.source_path)?;
        let adapted = program.build(&native)?;
        let output = convert(&adapted, &contract).with_context(|| contract.path.clone())?;
        ensure!(
            result.insert(contract.path, output).is_none(),
            "duplicate alias"
        );
    }
    Ok(result)
}
