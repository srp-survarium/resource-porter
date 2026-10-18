// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in retail 0.21 fog and sky equations inside pinned native 0.26 shaders.
//! Retail bytecode is read from the user's databases, never shipped here.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

pub const PATH: &str = "shaders/sm_4_0/atmospheric_scattering_on_geometry_temporary.ps/___________________________________________________________________";
pub const SKY_PATH: &str = "shaders/sm_4_0/atmosphere.ps/___________________________________________________________________";

pub struct Profile {
    pub marker: f32,
    /// The original renderer uploads pow(authored RGB, 2.2).
    pub color: [f32; 3],
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated shader")?
            .try_into()?,
    ))
}
fn words(bytes: &[u8]) -> Result<Vec<u32>> {
    ensure!(bytes.len() % 4 == 0, "unaligned shader words");
    bytes
        .chunks_exact(4)
        .map(|b| Ok(u32::from_le_bytes(b.try_into()?)))
        .collect()
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn instructions(words: &[u32]) -> Result<Vec<Vec<u32>>> {
    ensure!(
        words.len() >= 2 && words[1] as usize == words.len(),
        "shader length mismatch"
    );
    let mut out = Vec::new();
    let mut at = 2;
    while at < words.len() {
        let count = ((words[at] >> 24) & 127) as usize;
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid shader instruction"
        );
        out.push(words[at..at + count].to_vec());
        at += count;
    }
    Ok(out)
}
fn chunks(code: &[u8]) -> Result<Vec<([u8; 4], Vec<u8>)>> {
    ensure!(
        code.starts_with(b"DXBC") && u32_at(code, 24)? as usize == code.len(),
        "invalid DXBC"
    );
    let count = u32_at(code, 28)? as usize;
    ensure!(count <= 32, "unexpected shader chunk count");
    (0..count)
        .map(|i| {
            let at = u32_at(code, 32 + i * 4)? as usize;
            let size = u32_at(code, at + 4)? as usize;
            let tag = code
                .get(at..at + 4)
                .context("missing chunk tag")?
                .try_into()?;
            Ok((
                tag,
                code.get(at + 8..at + 8 + size)
                    .context("truncated chunk")?
                    .to_vec(),
            ))
        })
        .collect()
}
fn pinned_code<'a>(blob: &'a [u8], expected: &str) -> Result<(usize, &'a [u8])> {
    ensure!(
        format!("{:x}", Sha256::digest(blob)) == expected,
        "unsupported retail atmosphere shader"
    );
    let offset = 4 + u32_at(blob, 0)? as usize * 264;
    let code = blob
        .get(offset..)
        .context("truncated retail shader header")?;
    ensure!(
        checksum(code)? == code[4..20],
        "retail DXBC checksum mismatch"
    );
    Ok((offset, code))
}

/// Each marked map executes the original arithmetic; other maps execute the
/// unchanged native body. Only camera reconstruction follows the target UV ABI.
pub fn build(source: &[u8], target: &[u8], profiles: &[Profile]) -> Result<Vec<u8>> {
    ensure!(!profiles.is_empty(), "no legacy fog profiles");
    let (_, source) = pinned_code(
        source,
        "216fc657befc72b202c4bc10075300223a1198c37fb3a378ce073154397c2a04",
    )?;
    let (offset, native) = pinned_code(
        target,
        "6aa56bbc4f3c13775c155d50b436f910b09f82656eda17c1c326e0ddfa953b8d",
    )?;
    let original = chunks(source)?;
    let mut entries = chunks(native)?;
    let source_words = words(
        &original
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("missing original shader")?
            .1,
    )?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("missing native shader")?;
    let native_words = words(&entries[index].1)?;
    let source_ops = instructions(&source_words)?;
    let native_ops = instructions(&native_words)?;
    let body_start = |ops: &[Vec<u32>]| -> Result<usize> {
        Ok(ops
            .iter()
            .position(|i| i[0] & 2047 == 104)
            .context("missing temp declaration")?
            + 1)
    };
    let sb = body_start(&source_ops)?;
    let nb = body_start(&native_ops)?;
    let source_body = &source_ops[sb..];
    let native_body = &native_ops[nb..];
    ensure!(
        source_body.len() == 56 && native_body.len() == 63 && native_ops[nb - 1] == [0x02000068, 3],
        "unexpected fog program"
    );
    ensure!(
        source_body[10][0] & 2047 == 0 && native_body[9][0] & 2047 == 0,
        "world position boundary changed"
    );
    // Retail source uses two temporaries. r2 is free for the original color
    // and the observed infinite height multiplier (the original lerp bug).
    let mut tail = source_body[11..].to_vec();
    let substitutions: &[(&[u32], &[u32], usize)] = &[
        (&[0x0020801a, 1, 18], &[0x0010003a, 2], 1), // height -> r2.w
        (&[0x8020801a, 0x41, 1, 18], &[0x8010003a, 0x41, 2], 1), // -height
        (&[0x0020800a, 1, 18], &[0x0020801a, 1, 18], 1), // far moved x -> y
        (&[0x00208906, 1, 15], &[0x00100906, 2], 1), // Rayleigh RGB -> r2
    ];
    // Match against the original instruction, so the far-distance remapping
    // cannot itself be mistaken for a height operand on a later pass.
    let mut counts = vec![0; substitutions.len()];
    for ins in &mut tail {
        let mut fixed = vec![ins[0]];
        let mut at = 1;
        while at < ins.len() {
            if let Some((j, (from, to, _))) = substitutions
                .iter()
                .enumerate()
                .find(|(_, (from, _, _))| ins[at..].starts_with(from))
            {
                fixed.extend_from_slice(to);
                counts[j] += 1;
                at += from.len();
            } else {
                fixed.push(ins[at]);
                at += 1;
            }
        }
        fixed[0] = (fixed[0] & !0x7f000000) | (fixed.len() as u32) << 24;
        *ins = fixed;
    }
    for (count, (_, _, expected)) in counts.iter().zip(substitutions) {
        ensure!(count == expected, "original fog binding remap mismatch");
    }
    let mut updated = native_words[..2].to_vec();
    updated.extend(native_ops[..nb].iter().flatten());
    let mut markers = std::collections::BTreeSet::new();
    for profile in profiles {
        ensure!(
            profile.marker.is_finite()
                && profile.marker < -32000.0
                && markers.insert(profile.marker.to_bits()),
            "invalid or duplicate fog marker"
        );
        ensure!(
            profile.color.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid original fog color"
        );
        // eq r2.w, cb1[18].w, marker; if_nz r2.w
        updated.extend([
            0x08000018,
            0x00100082,
            2,
            0x0020803a,
            1,
            18,
            0x00004001,
            profile.marker.to_bits(),
            0x0304001f,
            0x0010003a,
            2,
        ]);
        updated.extend([
            0x08000036,
            0x001000f2,
            2,
            0x00004002,
            profile.color[0].to_bits(),
            profile.color[1].to_bits(),
            profile.color[2].to_bits(),
            f32::INFINITY.to_bits(),
        ]);
        updated.extend(native_body[..10].iter().flatten());
        updated.extend(tail.iter().flatten()); // includes the original ret
        updated.push(0x01000015); // endif
    }
    let native_at = updated.len();
    updated.extend(native_body.iter().flatten());
    updated[1] = updated.len() as u32;
    // The unmarked path and every native declaration must survive byte-for-byte.
    ensure!(
        updated[native_at..] == native_body.iter().flatten().copied().collect::<Vec<_>>(),
        "native body changed"
    );
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    let result = assemble(native, &entries)?;
    for (i, (old, new)) in chunks(native)?.iter().zip(chunks(&result)?).enumerate() {
        if i != index {
            ensure!(old == &new, "native shader metadata changed");
        }
    }
    let mut blob = target[..offset].to_vec();
    blob.extend(result);
    Ok(blob)
}

fn assemble(native: &[u8], entries: &[([u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    let mut result = native[..32 + entries.len() * 4].to_vec();
    for (i, (tag, data)) in entries.iter().enumerate() {
        let chunk_offset = (result.len() as u32).to_le_bytes();
        result[32 + i * 4..36 + i * 4].copy_from_slice(&chunk_offset);
        result.extend(tag);
        result.extend((data.len() as u32).to_le_bytes());
        result.extend(data);
    }
    let size = (result.len() as u32).to_le_bytes();
    result[24..28].copy_from_slice(&size);
    let digest = checksum(&result)?;
    result[4..20].copy_from_slice(&digest);
    Ok(result)
}

/// The original sky has the same varying/texture ABI as 0.26. Preserve its
/// complete arithmetic and select it only for maps already using legacy fog.
pub fn build_sky(source: &[u8], target: &[u8], markers: &[f32]) -> Result<Vec<u8>> {
    ensure!(!markers.is_empty(), "no original sky markers");
    let (_, source) = pinned_code(
        source,
        "bd9cff731c4e3f3b506dc7283b9ce9b0ba70d7280dd207ea69bb3bd0e7494bad",
    )?;
    let (offset, native) = pinned_code(
        target,
        "8cad6019edbef9109e249ce181774cecbb0e32f810591618bd66664b8325c2e0",
    )?;
    let original = chunks(source)?;
    let mut entries = chunks(native)?;
    let chunk = |name: &[u8; 4], items: &[([u8; 4], Vec<u8>)]| -> Result<usize> {
        items
            .iter()
            .position(|(tag, _)| tag == name)
            .context("missing sky shader chunk")
    };
    for name in [b"ISGN", b"OSGN"] {
        ensure!(
            original[chunk(name, &original)?].1 == entries[chunk(name, &entries)?].1,
            "sky signature changed"
        );
    }
    let index = chunk(b"SHDR", &entries)?;
    let native_words = words(&entries[index].1)?;
    let native_ops = instructions(&native_words)?;
    let source_ops = instructions(&words(&original[chunk(b"SHDR", &original)?].1)?)?;
    let body_start = |ops: &[Vec<u32>]| -> Result<usize> {
        let at = ops
            .iter()
            .position(|i| i[0] & 2047 == 104)
            .context("missing sky temporaries")?;
        ensure!(ops[at] == [0x02000068, 3], "sky temporary count changed");
        Ok(at + 1)
    };
    let nb = body_start(&native_ops)?;
    let sb = body_start(&source_ops)?;
    let mut updated = native_words[..2].to_vec();
    updated.extend(native_ops[..nb].iter().flatten());
    let mut seen = std::collections::BTreeSet::new();
    for marker in markers {
        ensure!(
            marker.is_finite() && *marker < -32000.0 && seen.insert(marker.to_bits()),
            "invalid or duplicate sky marker"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            2,
            0x0020803a,
            1,
            18,
            0x00004001,
            marker.to_bits(),
            0x0304001f,
            0x0010003a,
            2,
        ]);
        updated.extend(source_ops[sb..].iter().flatten());
        updated.push(0x01000015);
    }
    updated.extend(native_ops[nb..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    // The layout stays native. Mark the original-only constants and marker
    // as used so reflection-based binding also supplies the new branch.
    let reflection = chunk(b"RDEF", &entries)?;
    let rdef = &mut entries[reflection].1;
    let count = u32_at(rdef, 0)? as usize;
    let buffers = u32_at(rdef, 4)? as usize;
    let string = |data: &[u8], at: usize| -> Result<String> {
        let tail = data.get(at..).context("missing reflection string")?;
        let len = tail
            .iter()
            .position(|b| *b == 0)
            .context("unterminated reflection string")?;
        Ok(std::str::from_utf8(&tail[..len])?.to_owned())
    };
    let mut changed = 0;
    for i in 0..count {
        let at = buffers + i * 24;
        if string(rdef, u32_at(rdef, at)? as usize)? != "static_globals" {
            continue;
        }
        let vars = u32_at(rdef, at + 4)? as usize;
        let start = u32_at(rdef, at + 8)? as usize;
        for j in 0..vars {
            let at = start + j * 24;
            let name = string(rdef, u32_at(rdef, at)? as usize)?;
            let expected = match name.as_str() {
                "atmosphere_parameters1" => Some((256, 16)),
                "atmosphere_parameters2" => Some((272, 16)),
                "atmosphere_parameters3" => Some((288, 16)),
                "to_sun_direction" => Some((352, 12)),
                _ => None,
            };
            if let Some((offset, size)) = expected {
                ensure!(
                    u32_at(rdef, at + 4)? == offset
                        && u32_at(rdef, at + 8)? == size
                        && u32_at(rdef, at + 12)? == 0,
                    "sky constant layout changed"
                );
                rdef[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
                changed += 1;
            }
        }
    }
    ensure!(changed == 4, "sky constant binding mismatch");
    let result = assemble(native, &entries)?;
    let mut blob = target[..offset].to_vec();
    blob.extend(result);
    Ok(blob)
}

/// DXBC's MD5 padding variant. The compression rounds are standard MD5.
/// Padding reference: Wine libs/vkd3d/libs/vkd3d-shader/checksum.c.
fn checksum(code: &[u8]) -> Result<[u8; 16]> {
    ensure!(
        code.starts_with(b"DXBC") && code.len() >= 20,
        "invalid checksum input"
    );
    let payload = &code[20..];
    let bit_len = (payload.len() as u32).wrapping_mul(8);
    let complete = payload.len() / 64 * 64;
    let mut blocks = payload[..complete].to_vec();
    let mut tail = payload[complete..].to_vec();
    tail.push(0x80);
    let mut last;
    if tail.len() > 56 {
        tail.resize(64, 0);
        blocks.extend(tail);
        last = vec![0; 60];
    } else {
        last = vec![0; 4];
        last.extend(tail);
        last.resize(60, 0);
    }
    last.extend(((bit_len >> 2) | 1).to_le_bytes());
    last[..4].copy_from_slice(&bit_len.to_le_bytes());
    blocks.extend(last);
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let shifts = [
        [7, 12, 17, 22],
        [5, 9, 14, 20],
        [4, 11, 16, 23],
        [6, 10, 15, 21],
    ];
    let constants: [u32; 64] =
        std::array::from_fn(|i| (((i + 1) as f64).sin().abs() * 4294967296.0) as u32);
    for block in blocks.chunks_exact(64) {
        let w = words(block)?;
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..16 => ((b & c) | (!b & d), i),
                16..32 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..48 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(constants[i])
                    .wrapping_add(w[g])
                    .rotate_left(shifts[i / 16][i % 4]),
            );
            (a, b, c, d) = (d, next, b, c);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    Ok(bytes(&state).try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_padding_boundaries() {
        // Independent Python implementation, also checked against retail DXBC.
        for (len, expected) in [
            (0, "140d60f6b775e2ba4e4abed401b2e9a1"),
            (55, "842e55534ba93daa93e94bd5af9b0c03"),
            (56, "00f9cc964ff2ec81959d4a3f092ce63f"),
            (63, "4400d428a41d0e75dc98b936693f7bfb"),
            (64, "f42eb06ca921c878e435e3b8d4f92b13"),
            (65, "9ce1a374e3f259448f5777113a2edb57"),
            (120, "882fce4f4c95194392e7adefd53fe343"),
        ] {
            let mut code = b"DXBC".to_vec();
            code.extend([0; 16]);
            code.extend((0..len).map(|i| (i % 251) as u8));
            let actual: String = checksum(&code)
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(actual, expected);
        }
        assert!(checksum(b"DXBC").is_err());
    }
}
