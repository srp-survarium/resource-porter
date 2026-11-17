//! Original point/spot programs with native screen-position and resource bindings.
//! The complete native body remains the fallback for every unmarked scene.
use super::*;
use std::collections::BTreeMap;

const PINS: &str = include_str!("local_lights.tsv");

pub fn local_light_pairs() -> Result<Vec<(&'static str, &'static str)>> {
    PINS.lines()
        .filter(|l| !l.starts_with('#'))
        .map(|line| {
            let p: Vec<_> = line.split('\t').collect();
            ensure!(p.len() == 4, "invalid local-light pin row");
            Ok((p[0], p[1]))
        })
        .collect()
}

fn name(r: &[u8], at: usize) -> Result<&str> {
    let tail = r.get(at..).context("missing local-light reflection name")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated reflection name")?;
    Ok(std::str::from_utf8(&tail[..end])?)
}

// Reflection maps resource names to binding slots, and records real used
// constants. Compare names, byte offsets and sizes before adapting operands.
fn bindings(r: &[u8]) -> Result<BTreeMap<(u32, String), (u32, u32)>> {
    let mut result = BTreeMap::new();
    for i in 0..u32_at(r, 8)? as usize {
        let at = u32_at(r, 12)? as usize + 32 * i;
        let key = (
            u32_at(r, at + 4)?,
            name(r, u32_at(r, at)? as usize)?.to_owned(),
        );
        ensure!(
            result
                .insert(key, (u32_at(r, at + 20)?, u32_at(r, at + 24)?))
                .is_none(),
            "duplicate binding"
        );
    }
    Ok(result)
}

struct Constant {
    record: usize,
    byte_offset: u32,
    size: u32,
    flags: u32,
}

fn variables(r: &[u8]) -> Result<BTreeMap<(String, String), Constant>> {
    let mut result = BTreeMap::new();
    for i in 0..u32_at(r, 0)? as usize {
        let cb = u32_at(r, 4)? as usize + 24 * i;
        let buffer = name(r, u32_at(r, cb)? as usize)?;
        for j in 0..u32_at(r, cb + 4)? as usize {
            let at = u32_at(r, cb + 8)? as usize + 24 * j;
            let key = (
                buffer.to_owned(),
                name(r, u32_at(r, at)? as usize)?.to_owned(),
            );
            ensure!(
                result
                    .insert(
                        key,
                        Constant {
                            record: at,
                            byte_offset: u32_at(r, at + 4)?,
                            size: u32_at(r, at + 8)?,
                            flags: u32_at(r, at + 12)?,
                        }
                    )
                    .is_none(),
                "duplicate constant"
            );
        }
    }
    Ok(result)
}

pub fn build_local_light(
    path: &str,
    source: &[u8],
    target: &[u8],
    markers: &[f32],
) -> Result<Vec<u8>> {
    let pins = PINS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|p| p.get(1) == Some(&path))
        .context("unknown local-light permutation")?;
    ensure!(
        pins.len() == 4 && !markers.is_empty(),
        "invalid local-light request"
    );
    let (_, original) = pinned_code(source, pins[2])?;
    let (offset, native) = pinned_code(target, pins[3])?;
    let source_chunks = chunks(original)?;
    let mut entries = chunks(native)?;
    let chunk = |items: &[([u8; 4], Vec<u8>)], tag: &[u8; 4]| -> Result<usize> {
        items
            .iter()
            .position(|(t, _)| t == tag)
            .context("missing local-light chunk")
    };
    let si = chunk(&entries, b"SHDR")?;
    let ri = chunk(&entries, b"RDEF")?;
    ensure!(
        source_chunks[chunk(&source_chunks, b"OSGN")?].1 == entries[chunk(&entries, b"OSGN")?].1,
        "local-light output targets changed"
    );
    let source_reflection = &source_chunks[chunk(&source_chunks, b"RDEF")?].1;
    let native_bindings = bindings(&entries[ri].1)?;
    let mut remap = BTreeMap::new();
    for (key, (old_slot, count)) in bindings(source_reflection)? {
        let (slot, native_count) = native_bindings
            .get(&key)
            .context("missing native light binding")?;
        ensure!(
            count == 1 && *native_count == 1,
            "unexpected light resource array"
        );
        // D3D reflection types: cbuffer=0, texture=2, sampler=3.
        let operand = match key.0 {
            0 => 8,
            2 => 7,
            3 => 6,
            _ => anyhow::bail!("unsupported light resource"),
        };
        remap.insert((operand, old_slot), *slot);
    }
    let native_variables = variables(&entries[ri].1)?;
    for (key, original_constant) in variables(source_reflection)? {
        if original_constant.flags & 2 == 0 {
            continue;
        }
        let constant = native_variables
            .get(&key)
            .context("missing native light constant")?;
        ensure!(
            (original_constant.byte_offset, original_constant.size)
                == (constant.byte_offset, constant.size),
            "light constant layout differs: {key:?}"
        );
        entries[ri].1[constant.record + 12..constant.record + 16]
            .copy_from_slice(&(constant.flags | 2).to_le_bytes());
    }
    let marker_key = (
        "static_globals".to_owned(),
        "atmosphere_parameters3".to_owned(),
    );
    let marker_constant = native_variables
        .get(&marker_key)
        .context("missing light marker constant")?;
    ensure!(
        marker_constant.byte_offset == 288 && marker_constant.size == 16,
        "light marker layout changed"
    );
    entries[ri].1[marker_constant.record + 12..marker_constant.record + 16]
        .copy_from_slice(&(marker_constant.flags | 2).to_le_bytes());

    let source_ops = instructions(&words(&source_chunks[chunk(&source_chunks, b"SHDR")?].1)?)?;
    let native_words = words(&entries[si].1)?;
    let mut native_ops = instructions(&native_words)?;
    let temps = |ops: &[Vec<u32>]| -> Result<usize> {
        ops.iter()
            .position(|i| i[0] & 2047 == 104)
            .context("missing light temporaries")
    };
    let st = temps(&source_ops)?;
    let nt = temps(&native_ops)?;
    let scratch = source_ops[st][1].max(native_ops[nt][1]);
    native_ops[nt][1] = scratch + 1;
    let cb = native_ops
        .iter_mut()
        .find(|i| i.as_slice() == [0x04000059, 0x00208e46, 1, 15])
        .context("native light static globals changed")?;
    cb[3] = 19;
    // Both reconstruct the same screen UV. The original interpolated clip XY/W
    // input was replaced by SV_Position in 0.26; use its exact native first op.
    ensure!(
        source_ops[st + 1] == [0x0700000e, 0x00100032, 0, 0x00101046, 1, 0x00101ff6, 1],
        "original light screen UV changed"
    );
    let mut uv = native_ops[nt + 1..].iter().filter(|op| {
        op.len() == 8
            && op[..2] == [0x08000038, 0x00100032]
            && op[3..] == [0x00101046, 0, 0x00208ae6, 1, 14]
    });
    let mut screen_uv = uv.next().context("missing native light screen UV")?.clone();
    ensure!(uv.next().is_none(), "ambiguous native light screen UV");
    screen_uv[2] = 0; // original destination, also for native SSDO variants
    let mut body = vec![screen_uv];
    for ins in &source_ops[st + 2..] {
        ensure!(ins[0] & 0x80000000 == 0, "extended light instruction");
        let mut fixed = vec![ins[0]];
        let mut at = 1;
        while at < ins.len() {
            let token = ins[at];
            fixed.push(token);
            at += 1;
            if token & 0x80000000 != 0 {
                let extension = *ins.get(at).context("truncated operand modifier")?;
                ensure!(extension & 0x80000000 == 0, "chained operand modifier");
                fixed.push(extension);
                at += 1;
            }
            let kind = (token >> 12) & 255;
            let dimensions = ((token >> 20) & 3) as usize;
            ensure!(token & 0x7fc00000 == 0, "relative light operand");
            if kind == 4 {
                ensure!(
                    dimensions == 0 && matches!(token & 3, 1 | 2),
                    "invalid light literal"
                );
                let count = if token & 3 == 2 { 4 } else { 1 };
                fixed.extend_from_slice(ins.get(at..at + count).context("truncated literal")?);
                at += count;
                continue;
            }
            let mut indices = ins
                .get(at..at + dimensions)
                .context("truncated light operand")?
                .to_vec();
            at += dimensions;
            match kind {
                6..=8 => {
                    ensure!(
                        dimensions == if kind == 8 { 2 } else { 1 },
                        "unexpected light binding operand"
                    );
                    indices[0] = *remap
                        .get(&(kind, indices[0]))
                        .context("unbound original light resource")?;
                }
                0 => ensure!(
                    dimensions == 1 && indices[0] < scratch,
                    "invalid light temporary"
                ),
                2 => ensure!(dimensions == 1 && indices[0] < 2, "invalid light output"),
                _ => anyhow::bail!("unexpected original light operand {kind}"),
            }
            fixed.extend(indices);
        }
        ensure!(fixed.len() == ins.len(), "light instruction length changed");
        body.push(fixed);
    }
    let mut updated = native_words[..2].to_vec();
    updated.extend(native_ops[..=nt].iter().flatten());
    let mut seen = std::collections::BTreeSet::new();
    for marker in markers {
        ensure!(
            marker.is_finite() && *marker < -32000.0 && seen.insert(marker.to_bits()),
            "invalid light marker"
        );
        updated.extend([
            0x08000018,
            0x00100012,
            scratch,
            0x0020803a,
            1,
            18,
            0x00004001,
            marker.to_bits(),
            0x0304001f,
            0x0010000a,
            scratch,
        ]);
        updated.extend(body.iter().flatten()); // original ret ends only the marked path
        updated.push(0x01000015);
    }
    updated.extend(native_ops[nt + 1..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[si].1 = bytes(&updated);
    let mut blob = target[..offset].to_vec();
    blob.extend(assemble(native, &entries)?);
    Ok(blob)
}
