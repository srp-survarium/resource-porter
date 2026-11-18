//! Missing ordinary alpha-test permutations with native parallax and no normal map.
//! Native dirt programs supply the height calculation and finish. Only their dead
//! dirt mask is removed before the ordinary alpha test and original painted guard.
use super::{assemble, bytes, chunks, geometry, instructions, op, pinned_code, words};
use crate::{ResourceDb, weather_fog_021::u32_at};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    vertex: u8,
    high_path: String,
    high_sha256: String,
    low_path: String,
    low_sha256: String,
    base_register: u32,
    removed_mask: Vec<Vec<u32>>,
    targets: Vec<String>,
}

fn name_at(data: &[u8], at: usize) -> Result<&[u8]> {
    let start = u32_at(data, at)? as usize;
    let tail = data.get(start..).context("reflection string bounds")?;
    Ok(&tail[..tail
        .iter()
        .position(|b| *b == 0)
        .context("reflection string terminator")?])
}

fn ordinary_high(blob: &[u8], row: &Variant) -> Result<Vec<u8>> {
    let (offset, native) = pinned_code(blob, &row.high_sha256)?;
    let mut entries = chunks(native)?;
    let index = entries
        .iter()
        .position(|(t, _)| t == b"SHDR")
        .context("native program")?;
    let old = words(&entries[index].1)?;
    let mut ops = instructions(&old)?;
    let discards: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| o[0] & 2047 == 13)
        .map(|(i, _)| i)
        .collect();
    ensure!(discards.len() == 1, "ambiguous native dirt discard");
    let discard = discards[0];
    let temporary = ops
        .iter_mut()
        .find(|o| o[0] == 0x02000068)
        .context("native temps")?;
    let scratch = temporary[1];
    ensure!(scratch < 64, "unexpected temporary layout");
    temporary[1] += 1;
    let dst = [0x00100012, scratch];
    let src = [0x0010000a, scratch];
    let cutoff = [
        op(
            0,
            &[
                &dst,
                &[0x0010003a, row.base_register],
                &[0x8020802a, 0x41, 1, 15],
            ],
        ),
        op(49, &[&dst, &src, &[0x4001, 0]]),
        op(0x4000d, &[&src]),
    ];
    let variable_mask = matches!(row.vertex, 1 | 2);
    if variable_mask {
        ensure!(
            row.removed_mask.len() == 14 && discard >= 16,
            "mask extent changed"
        );
        ensure!(
            ops[discard - 16..discard - 2] == row.removed_mask,
            "pinned dead mask changed"
        );
        ensure!(
            ops[discard - 2] == [0x07000000, 0x00100082, 0, 0x0010003a, 0, 0x4001, 0xbecccccd]
                && ops[discard - 1] == [0x07000031, 0x00100082, 0, 0x0010003a, 0, 0x4001, 0]
                && ops[discard] == [0x0304000d, 0x0010003a, 0],
            "native mask cutoff changed"
        );
        ops.splice(discard - 16..=discard, cutoff);
        let declarations: Vec<_> = ops
            .iter()
            .enumerate()
            .filter(|(_, o)| o[0] & 2047 == 88 && o.get(2) == Some(&2))
            .map(|(i, _)| i)
            .collect();
        ensure!(declarations.len() == 1, "mask texture declaration changed");
        ops.remove(declarations[0]);
    } else {
        ensure!(
            row.removed_mask.is_empty() && ops[discard] == [0x0304000d, 0x4001, u32::MAX],
            "constant dirt discard changed"
        );
        ops.splice(discard..=discard, cutoff);
    }
    let mut program = old[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let reflection = &mut entries
        .iter_mut()
        .find(|(t, _)| t == b"RDEF")
        .context("native reflection")?
        .1;
    let mut changed = BTreeSet::new();
    for buffer in 0..u32_at(reflection, 0)? as usize {
        let at = u32_at(reflection, 4)? as usize + buffer * 24;
        for field in 0..u32_at(reflection, at + 4)? as usize {
            let variable = u32_at(reflection, at + 8)? as usize + field * 24;
            let (offset, before, after): (u32, u32, u32) = match name_at(reflection, variable)? {
                b"alpha_ref_parameter" => (248, 0, 2),
                b"vertex_blend_tiled_mask_parameters" if variable_mask => (208, 2, 0),
                b"vertex_blend_parameters" if variable_mask => (224, 2, 0),
                _ => continue,
            };
            ensure!(
                u32_at(reflection, variable + 4)? == offset
                    && u32_at(reflection, variable + 12)? == before,
                "native variable contract changed"
            );
            ensure!(changed.insert(offset), "duplicate variable");
            reflection[variable + 12..variable + 16].copy_from_slice(&after.to_le_bytes());
        }
    }
    ensure!(
        changed
            == if variable_mask {
                BTreeSet::from([208, 224, 248])
            } else {
                BTreeSet::from([248])
            },
        "missing native variable"
    );
    if variable_mask {
        let count = u32_at(reflection, 8)? as usize;
        let table = u32_at(reflection, 12)? as usize;
        let matches: Vec<_> = (0..count)
            .filter(|i| {
                let at = table + i * 32;
                name_at(reflection, at).ok() == Some(b"t_vertex_blended_tiled_mask".as_slice())
            })
            .collect();
        ensure!(matches.len() == 1, "mask resource binding changed");
        let at = table + matches[0] * 32;
        ensure!(
            u32_at(reflection, at + 4)? == 2
                && u32_at(reflection, at + 20)? == 2
                && u32_at(reflection, at + 24)? == 1,
            "mask resource contract changed"
        );
        // Keep string/type offsets intact; compact only the fixed-size binding table.
        reflection.copy_within(at + 32..table + count * 32, at);
        reflection[table + (count - 1) * 32..table + count * 32].fill(0);
        reflection[8..12].copy_from_slice(&((count - 1) as u32).to_le_bytes());
    }
    let mut output = blob[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

pub fn build(donor: &mut ResourceDb, stock: &mut ResourceDb) -> Result<Vec<(String, Vec<u8>)>> {
    let rows: Vec<Variant> = serde_json::from_str(include_str!("parallax_programs.json"))?;
    ensure!(
        rows.len() == 7
            && rows.iter().map(|r| r.vertex).collect::<BTreeSet<_>>()
                == BTreeSet::from([0, 1, 2, 10, 12, 13, 14]),
        "incomplete vertex family"
    );
    let mut output = Vec::new();
    for row in rows {
        let high = ordinary_high(&stock.read(&row.high_path)?, &row)?;
        let low = stock.read(&row.low_path)?;
        pinned_code(&low, &row.low_sha256)?;
        let high = geometry::adapt_template(&high, row.vertex, donor)?;
        let low = geometry::adapt_template(&low, row.vertex, donor)?;
        ensure!(row.targets.len() == 4, "incomplete quality/alias family");
        for path in row.targets {
            ensure!(
                !stock.contains(&path),
                "ordinary parallax path already exists: {path}"
            );
            let bytes = if path.contains("/_3_") { &high } else { &low };
            output.push((path, bytes.clone()));
        }
    }
    Ok(output)
}
