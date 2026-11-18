//! Missing "alpha test ignores the blend" siblings of native alpha-tested blend programs.
//!
//! An original alpha-tested blend material cuts out by its base texture alone, so its
//! port disables `alpha_test_consider_vertex_blend`. Stock ships that selector only for
//! some feature sets; the rest are derived here from the consider-on program of the same
//! key (already carrying the original blend adapter): its cutoff `1 + w * (a - 1)`
//! becomes the base alpha `a`. Existing programs are never replaced.
use super::{MaskedPrograms, assemble, bytes, chunks, instructions, words};
use crate::ResourceDb;
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

const ONE: u32 = 0x3f80_0000;
const MINUS_ONE: u32 = 0xbf80_0000;
/// `____FFFFF____TTT__________E__` follows the five blend selectors.
const AFTER_BLEND: usize = 29;

/// The sibling key with consider-blend off, for an alpha-tested consider-on key.
pub fn sibling(path: &str) -> Option<String> {
    let (folder, key) = path.rsplit_once('/')?;
    let at = key.len().checked_sub(AFTER_BLEND + 2)?;
    let bytes = key.as_bytes();
    (bytes.get(8) == Some(&b'1') && bytes[at] == b'1' && bytes[at - 2] == b'1').then(|| {
        let mut key = key.to_owned();
        key.replace_range(at..=at, "0");
        format!("{folder}/{key}")
    })
}

fn base_alpha_cutoff(ops: &mut [Vec<u32>]) -> Result<()> {
    let discards: Vec<_> = (0..ops.len()).filter(|i| ops[*i][0] & 2047 == 13).collect();
    ensure!(
        discards.len() == 1 && discards[0] >= 3,
        "ambiguous alpha test"
    );
    let mix = discards[0] - 3;
    let op = ops[mix].clone();
    // mad a, w, a, 1: the native blend-aware alpha, compared with alpha_ref next.
    ensure!(
        op.len() == 9 && op[0] == 0x0900_0032 && op[7..] == [0x0000_4001, ONE],
        "blend-aware alpha changed"
    );
    let (target, register) = (op[1], op[2]);
    let component = ((target >> 4) & 15).trailing_zeros();
    ensure!(
        (target >> 4) & 15 == 1 << component,
        "alpha is not a scalar"
    );
    let source = [0x0010_000a | (component << 4), register];
    ensure!(
        op[5..7] == source && op[3..5] != source,
        "alpha operand moved"
    );
    ensure!(
        ops[mix + 1][0] & 2047 == 0
            && ops[mix + 1][1..3] == [target, register]
            && ops[mix + 2][0] & 2047 == 49,
        "alpha reference comparison changed"
    );
    // That operand is `a - 1`, written once and untouched until the mix.
    let minus_one = [
        0x0700_0000,
        target,
        register,
        source[0],
        register,
        0x0000_4001,
        MINUS_ONE,
    ];
    let from: Vec<_> = (0..mix).filter(|i| ops[*i] == minus_one).collect();
    ensure!(from.len() == 1, "base alpha offset changed");
    ensure!(
        ops[from[0] + 1..mix].iter().all(|op| {
            let writes = matches!(op.get(1), Some(t) if t & 0xffff_f00f == 0x0010_0002);
            !(writes && op[2] == register && (op[1] >> 4) & (1 << component) != 0)
        }),
        "base alpha overwritten before the test"
    );
    ops[mix] = vec![
        0x0700_0000,
        target,
        register,
        source[0],
        register,
        0x0000_4001,
        ONE,
    ];
    Ok(())
}

/// Every absent sibling; `exists` answers for stock's other layers and packs.
pub fn build(
    donor: &mut ResourceDb,
    stock: &mut ResourceDb,
    exists: &dyn Fn(&str) -> bool,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let linear = donor.read(super::SOURCE)?;
    let masked = donor.read(super::MASKED_SOURCE)?;
    let programs = MaskedPrograms::new(&linear, &masked)?;
    let mut result = BTreeMap::new();
    for program in programs.programs() {
        let Some(path) = sibling(program.path()) else {
            continue;
        };
        if stock.paths().any(|p| p == path) || exists(&path) {
            continue;
        }
        let adapted = program.build(&stock.read(program.path())?)?;
        let offset = 4 + u32::from_le_bytes(adapted[..4].try_into()?) as usize * 264;
        let code = adapted.get(offset..).context("shader wrapper")?;
        let mut entries = chunks(code)?;
        let shdr = entries
            .iter()
            .position(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?;
        let old = words(&entries[shdr].1)?;
        let mut ops = instructions(&old)?;
        base_alpha_cutoff(&mut ops).with_context(|| path.clone())?;
        let mut updated = old[..2].to_vec();
        updated.extend(ops.iter().flatten());
        updated[1] = u32::try_from(updated.len())?;
        instructions(&updated)?;
        entries[shdr].1 = bytes(&updated);
        let mut output = adapted[..offset].to_vec();
        output.extend(assemble(code, &entries)?);
        ensure!(result.insert(path, output).is_none(), "duplicate sibling");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn sibling_turns_only_consider_off() {
        let on = "shaders/sm_4_0/gbuffer_pass.ps/_3___1001_00101_00____2_____0__1_01010____10000____000__________0__";
        let off = "shaders/sm_4_0/gbuffer_pass.ps/_3___1001_00101_00____2_____0__1_01000____10000____000__________0__";
        assert_eq!(super::sibling(on).as_deref(), Some(off));
        assert_eq!(super::sibling(off), None);
        // not alpha-tested, and a two-digit vertex class
        assert_eq!(super::sibling(&on.replace("_1001_", "_1000_")), None);
        let wide = on.replace("____2_____", "____12_____");
        assert_eq!(
            super::sibling(&wide),
            Some(off.replace("____2_____", "____12_____"))
        );
    }
}
