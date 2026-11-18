//! Original forward fog for the three passes observed on 0.21 vehicle glass and
//! for the particle base programs. Lighting and transparency stay native; only
//! exact matching fog blocks change.
use super::{Profile, assemble, bytes, checksum, chunks, instructions, pinned_code, u32_at, words};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy)]
pub enum Kind {
    Base,
    /// Particle base programs: their tail blends towards the fog colour.
    Particle,
    Probe,
    Direct,
}

impl Kind {
    pub fn paths(self) -> (&'static str, &'static str) {
        match self {
            Self::Base => (
                "shaders/sm_4_0/forward_base.ps/______0________1__010__1________0_0_____________1__0_0_0________0__",
                "shaders/sm_4_0/forward_base.ps/______0________1_010__1________0_0______________1__0_0_0________0__",
            ),
            Self::Particle => (
                "shaders/sm_4_0/forward_base.ps/______0________2__010__3________0_0_____________0__0_0_0________0__",
                "shaders/sm_4_0/forward_base.ps/______0________2_010__3________0_0______________0__0_0_0________0__",
            ),
            Self::Probe => (
                "shaders/sm_4_0/forward_probe_lighting.ps/_0___11_0___001___00___1________0_0_____________0__0_0_____________",
                "shaders/sm_4_0/forward_probe_lighting.ps/_____11_0___001__00___1________0_0______________0__0_0__________0__",
            ),
            Self::Direct => (
                "shaders/sm_4_0/forward_lighting.ps/_0__111_0___001__000___1___0____0_0_____________0__0_0_____________",
                "shaders/sm_4_0/forward_lighting.ps/____111_0___001_000___1___0____0_0______________0__0_0__________0__",
            ),
        }
    }

    fn specification(
        self,
    ) -> (
        &'static str,
        &'static str,
        std::ops::Range<usize>,
        std::ops::Range<usize>,
        u32,
    ) {
        match self {
            Self::Base => (
                "a3ca3f7366221b6b80b00c4fe7a9c79b7a359730c29ba1f2780af7ca984855a9",
                "11a0d17017593951bf8c850e2a5b5f7646413f03a196a1b236995822f5c88028",
                13..61,
                14..70,
                2,
            ),
            Self::Particle => (
                "22698bac066933e07f1785714f743ee0767809bd2579d0c55c70ab134454d6d9",
                "3c12ca61033cb5c2bef08e2e96839a40bedce5ce3e6ce5a4e17912b469715bc2",
                10..55,
                11..64,
                2,
            ),
            Self::Probe => (
                "7d8b804f706e6d3bacb1bc3cffb46a97131b3492d0d6bd83ee84719c129b8437",
                "b15ac0326fad1fbecb7b6e5457f05543a98d2461e2f9b241575136e98fc42046",
                134..183,
                135..192,
                3,
            ),
            Self::Direct => (
                "ae3cf994899c05c015fed0ec4ec0701391045f844eac9b9fb0d710edec8d1a66",
                "891830b564aa6b58fcdfaa6bd0c453b497483b274191ad96f9194d36df842299",
                389..439,
                393..451,
                3,
            ),
        }
    }
}

fn program(code: &[u8]) -> Result<Vec<Vec<u32>>> {
    let entries = chunks(code)?;
    instructions(&words(
        &entries
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?
            .1,
    )?)
}

/// Transplant the pinned original block into scratch registers behind the
/// existing scene marker. None means this permutation uses a different block.
pub fn build(
    kind: Kind,
    source: &[u8],
    template: &[u8],
    target: &[u8],
    profiles: &[Profile],
) -> Result<Option<Vec<u8>>> {
    ensure!(!profiles.is_empty(), "no forward fog profiles");
    let (source_hash, native_hash, source_range, native_range, globals) = kind.specification();
    let (_, original) = pinned_code(source, source_hash)?;
    let (_, reference) = pinned_code(template, native_hash)?;
    let original_ops = program(original)?;
    let reference_ops = program(reference)?;
    let original_block = &original_ops[source_range];
    let native_block = &reference_ops[native_range];
    let offset = 4 + u32_at(target, 0)? as usize * 264;
    let code = target.get(offset..).context("truncated forward shader")?;
    ensure!(checksum(code)? == code[4..20], "forward checksum mismatch");
    let mut entries = chunks(code)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("SHDR")?;
    let old_words = words(&entries[index].1)?;
    let mut ops = instructions(&old_words)?;
    let matches: Vec<_> = ops
        .windows(native_block.len())
        .enumerate()
        .filter_map(|(i, block)| (block == native_block).then_some(i))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    ensure!(matches.len() == 1, "ambiguous forward fog block");
    let start = matches[0];
    let end = start + native_block.len();
    match kind {
        Kind::Base => {
            if ops.get(end)
                != Some(&vec![
                    0x08000038, 0x00100062, 0, 0x00101106, 0, 0x00208ba6, 2, 14,
                ])
            {
                // Another transparency variant may share its fog prefix. Leave
                // it native until its live-register continuation is reviewed.
                return Ok(None);
            }
        }
        Kind::Particle => {
            // The reviewed tails read the fog colour r0.xzw and amount r0.y
            // and write every other temporary before reading it. They start
            // with the texture coordinates or with `add r0.xzw, r0.xxzw, -cb1[0].xxyz`.
            let reviewed = [
                reference_ops[kind.specification().3.end].clone(),
                vec![
                    0x09000000, 0x001000d2, 0, 0x00100e06, 0, 0x80208906, 0x41, 1, 0,
                ],
            ];
            if !ops.get(end).is_some_and(|next| reviewed.contains(next)) {
                return Ok(None);
            }
        }
        Kind::Probe | Kind::Direct => ensure!(
            ops[end..] == [vec![0x0100003e]],
            "lighting fog must end before return"
        ),
    }
    let temp = ops
        .iter()
        .position(|op| op[0] & 2047 == 104)
        .context("temporaries")?;
    let scratch = ops[temp][1];
    ensure!(scratch <= 64, "unexpected forward temporaries");
    let profile_register = scratch + 4;
    ops[temp][1] = scratch + 5;
    let mut body = Vec::new();
    let mut substitutions = [0; 3];
    for instruction in original_block {
        let mut fixed = vec![instruction[0]];
        let mut at = 1;
        while at < instruction.len() {
            let mut token = instruction[at];
            at += 1;
            let extension = if token & 0x80000000 != 0 {
                let v = instruction[at];
                at += 1;
                Some(v)
            } else {
                None
            };
            let operand = (token >> 12) & 255;
            let dimensions = ((token >> 20) & 3) as usize;
            let mut indices = instruction[at..at + dimensions].to_vec();
            at += dimensions;
            if operand == 4 {
                ensure!(
                    dimensions == 0 && extension.is_none(),
                    "unexpected forward literal"
                );
                let count = if token & 3 == 2 { 4 } else { 1 };
                fixed.push(token);
                fixed.extend(&instruction[at..at + count]);
                at += count;
                continue;
            }
            match operand {
                0 => {
                    ensure!(
                        dimensions == 1 && indices[0] < 4,
                        "unexpected source temporary"
                    );
                    indices[0] += scratch;
                }
                8 if indices[0] == globals => {
                    ensure!(dimensions == 2, "unexpected globals operand");
                    if indices[1] == 18 {
                        let component = (token >> 4) & 3;
                        if component == 0 {
                            token += 0x10;
                            substitutions[0] += 1;
                        } else {
                            ensure!(component == 1, "unexpected source height operand");
                            token = (token & !(0x00300000 | 0x000ff000 | 0x30)) | 0x00100000 | 0x30;
                            indices = vec![profile_register];
                            substitutions[1] += 1;
                        }
                    } else if indices[1] == 15 && (token >> 2) & 3 == 1 {
                        token = (token & !(0x00300000 | 0x000ff000)) | 0x00100000;
                        indices = vec![profile_register];
                        substitutions[2] += 1;
                    }
                }
                1 | 2 | 8 => {}
                _ => anyhow::bail!("unexpected forward fog operand {operand}"),
            }
            fixed.push(token);
            fixed.extend(extension);
            fixed.extend(indices);
        }
        fixed[0] = (fixed[0] & !0x7f000000) | (fixed.len() as u32) << 24;
        body.push(fixed);
    }
    ensure!(
        substitutions == [1, 2, 1],
        "original forward bindings changed"
    );
    let mut updated = old_words[..2].to_vec();
    updated.extend(ops[..start].iter().flatten());
    let mut markers = std::collections::BTreeSet::new();
    for profile in profiles {
        ensure!(
            profile.marker.is_finite()
                && profile.marker < -32000.0
                && markers.insert(profile.marker.to_bits())
                && profile.color.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid forward profile"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            profile_register,
            0x0020803a,
            globals,
            18,
            0x00004001,
            profile.marker.to_bits(),
            0x0304001f,
            0x0010003a,
            profile_register,
        ]);
        // Same observed original lerp bug and schema conversion as the day
        // fullscreen/particle/water adapters; do not invent a new fog setting.
        updated.extend([
            0x08000036,
            0x001000f2,
            profile_register,
            0x00004002,
            profile.color[0].to_bits(),
            profile.color[1].to_bits(),
            profile.color[2].to_bits(),
            f32::INFINITY.to_bits(),
        ]);
        if matches!(kind, Kind::Probe | Kind::Direct) {
            updated.extend([0x05000036, 0x001000f2, scratch, 0x00100e46, 0]);
        }
        updated.extend(body.iter().flatten());
        match kind {
            // The unchanged native transparency tail consumes only r0.x.
            Kind::Base => updated.extend([0x05000036, 0x00100012, 0, 0x0010000a, scratch]),
            // The original leaves the colour in r0.xyz and the amount in r1.y.
            Kind::Particle => updated.extend([
                0x05000036,
                0x001000d2,
                0,
                0x00100906,
                scratch,
                0x05000036,
                0x00100022,
                0,
                0x0010001a,
                scratch + 1,
            ]),
            Kind::Probe | Kind::Direct => {}
        }
        updated.push(0x01000012);
    }
    updated.extend(native_block.iter().flatten());
    updated.extend(std::iter::repeat_n(0x01000015, profiles.len()));
    updated.extend(ops[end..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    let mut result = target[..offset].to_vec();
    result.extend(assemble(code, &entries)?);
    Ok(Some(result))
}
