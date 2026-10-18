// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in retail 0.21 atmosphere equations inside pinned native 0.26 shaders.
//! Retail bytecode is read from the user's databases, never shipped here.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

pub const PATH: &str = "shaders/sm_4_0/atmospheric_scattering_on_geometry_temporary.ps/___________________________________________________________________";
pub const SKY_PATH: &str = "shaders/sm_4_0/atmosphere.ps/___________________________________________________________________";
pub const CLOUD_PREFIX: &str = "shaders/sm_4_0/atmosphere_clouds.ps/";
pub const SCATTERING_PATH: &str = "shaders/sm_4_0/make_mie_rayleigh_texture.ps/___________________________________________________________________";
pub const PARTICLE_PREFIX: &str = "shaders/sm_4_0/vertex_base_offscreen_particle_lighting.vs/";
pub const PARTICLE_SOURCE: &str = "shaders/sm_4_0/vertex_base_offscreen_particle_lighting.vs/00__1_____________0___27___0____0_0________________0_0_____________";
pub const PARTICLE_TEMPLATE: &str = "shaders/sm_4_0/vertex_base_offscreen_particle_lighting.vs/0___1____________0___27___0____0_0_________________0_0_____________";
pub const WATER_PREFIX: &str = "shaders/sm_4_0/forward_water.ps/";
pub const WATER_SOURCE: &str = "shaders/sm_4_0/forward_water.ps/_3__1_1___________0____1________0_0________________0_0_____________";
pub const WATER_TEMPLATE: &str = "shaders/sm_4_0/forward_water.ps/_3__1_1__________0____1________0_0_________________0_0_____________";

pub struct ScatteringProfile {
    pub marker: f32,
    pub rayleigh_sun: f32,
}

#[derive(Clone)]
pub struct Profile {
    pub marker: f32,
    /// The original renderer uploads pow(authored RGB, 2.2).
    pub color: [f32; 3],
}

/// The native CPU substitutes sun intensity for the authored Rayleigh gain.
/// Restore that input and the original coefficients only for marked maps.
pub fn build_scattering(
    source: &[u8],
    target: &[u8],
    native_sky: &[u8],
    profiles: &[ScatteringProfile],
) -> Result<Vec<u8>> {
    ensure!(!profiles.is_empty(), "no original scattering profiles");
    let (_, source) = pinned_code(
        source,
        "721754b91581085aa3eb47c2fbce03db6c373183d52dceae469804669ff6f32e",
    )?;
    let (offset, native) = pinned_code(
        target,
        "d9b19759a64be27f4cf98e3b843a53f3398e48b8ac8217ffba69b11551567755",
    )?;
    let (_, sky) = pinned_code(
        native_sky,
        "8cad6019edbef9109e249ce181774cecbb0e32f810591618bd66664b8325c2e0",
    )?;
    let original = chunks(source)?;
    let mut entries = chunks(native)?;
    let sky = chunks(sky)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("missing scattering program")?;
    let original_words = words(
        &original
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("missing source scattering program")?
            .1,
    )?;
    let native_words = words(&entries[index].1)?;
    let source_ops = instructions(&original_words)?;
    let mut native_ops = instructions(&native_words)?;
    let temps = |ops: &[Vec<u32>]| {
        ops.iter()
            .position(|ins| ins[0] & 2047 == 104)
            .context("missing scattering temporaries")
    };
    let st = temps(&source_ops)?;
    let nt = temps(&native_ops)?;
    ensure!(
        source_ops[st] == [0x02000068, 13] && native_ops[nt] == source_ops[st],
        "unexpected scattering temporaries"
    );
    ensure!(
        source_ops[..st] == native_ops[..nt],
        "scattering declarations changed"
    );
    ensure!(
        original_words.len() == native_words.len()
            && original_words
                .iter()
                .zip(&native_words)
                .filter(|(a, b)| a != b)
                .count()
                == 2,
        "unexpected scattering equation changes"
    );

    // Reuse the native sky's exact static-global layout so the existing
    // environment marker is available during LUT generation. Both $Globals
    // layouts are two float4s; replace the second variable's name with the
    // lookup generator's name and retain only the two constant-buffer bindings.
    let ri = entries
        .iter()
        .position(|(tag, _)| tag == b"RDEF")
        .context("missing scattering reflection")?;
    let mut r = sky
        .iter()
        .find(|(tag, _)| tag == b"RDEF")
        .context("missing sky reflection")?
        .1
        .clone();
    fn name(r: &[u8], at: usize) -> Result<&str> {
        let tail = r.get(at..).context("missing reflection name")?;
        let n = tail
            .iter()
            .position(|c| *c == 0)
            .context("unterminated reflection name")?;
        Ok(std::str::from_utf8(&tail[..n])?)
    }
    let count = u32_at(&r, 0)? as usize;
    let buffers = u32_at(&r, 4)? as usize;
    ensure!(count == 2, "unexpected sky buffer count");
    let new_name = r.len() as u32;
    r.extend_from_slice(b"atmosphere_rayleigh_mie_parameters\0");
    let (mut renamed, mut marked) = (0, 0);
    for i in 0..count {
        let at = buffers + i * 24;
        let buffer_name = name(&r, u32_at(&r, at)? as usize)?.to_owned();
        let count = u32_at(&r, at + 4)? as usize;
        let variables = u32_at(&r, at + 8)? as usize;
        for j in 0..count {
            let at = variables + j * 24;
            let variable = name(&r, u32_at(&r, at)? as usize)?.to_owned();
            if buffer_name == "$Globals" && variable == "atmosphere_cloud_parameters" {
                ensure!(
                    u32_at(&r, at + 4)? == 16 && u32_at(&r, at + 8)? == 16,
                    "unexpected lookup parameter layout"
                );
                r[at..at + 4].copy_from_slice(&new_name.to_le_bytes());
                r[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
                renamed += 1;
            } else if buffer_name == "static_globals" {
                let flags = if variable == "atmosphere_parameters3" {
                    ensure!(
                        u32_at(&r, at + 4)? == 288 && u32_at(&r, at + 8)? == 16,
                        "unexpected scattering marker layout"
                    );
                    marked += 1;
                    2u32
                } else {
                    0
                };
                r[at + 12..at + 16].copy_from_slice(&flags.to_le_bytes());
            }
        }
    }
    ensure!(
        renamed == 1 && marked == 1,
        "scattering reflection binding missing"
    );
    let count = u32_at(&r, 8)? as usize;
    let resources = u32_at(&r, 12)? as usize;
    let mut bindings = Vec::new();
    for i in 0..count {
        let at = resources + i * 32;
        if u32_at(&r, at + 4)? == 0 {
            bindings.extend_from_slice(&r[at..at + 32]);
        }
    }
    ensure!(
        bindings.len() == 64,
        "unexpected scattering buffer bindings"
    );
    // Keep the binding table before its referenced names and type records.
    // The retail reflection reader does not accept a table appended after
    // those records, even though all absolute offsets remain in bounds.
    r[resources..resources + bindings.len()].copy_from_slice(&bindings);
    r[8..12].copy_from_slice(&2u32.to_le_bytes());
    entries[ri].1 = r;

    native_ops[nt][1] = 14;
    let mut updated = native_words[..2].to_vec();
    updated.extend(native_ops[..nt].iter().flatten());
    updated.extend([0x04000059, 0x00208e46, 1, 26]); // static_globals b1
    updated.extend(&native_ops[nt]);
    let mut seen = std::collections::BTreeSet::new();
    for profile in profiles {
        ensure!(
            profile.marker.is_finite()
                && profile.marker < -32000.0
                && seen.insert(profile.marker.to_bits()),
            "invalid scattering marker"
        );
        ensure!(
            profile.rayleigh_sun.is_finite() && profile.rayleigh_sun >= 0.0,
            "invalid original Rayleigh multiplier"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            13,
            0x0020803a,
            1,
            18,
            0x00004001,
            profile.marker.to_bits(),
            0x0304001f,
            0x0010003a,
            13,
        ]);
        let mut replacements = 0;
        for ins in &source_ops[st + 1..] {
            let mut ins = ins.clone();
            if let Some(at) = ins.windows(3).position(|w| w == [0x00208006, 0, 1]) {
                let value = profile.rayleigh_sun.to_bits();
                ins.splice(at..at + 3, [0x00004002, value, value, value, value]);
                ins[0] = (ins[0] & !0x7f000000) | (ins.len() as u32) << 24;
                replacements += 1;
            }
            updated.extend(ins);
        }
        ensure!(replacements == 1, "original Rayleigh binding changed");
        updated.push(0x01000015);
    }
    updated.extend(native_ops[nt + 1..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    let mut blob = target[..offset].to_vec();
    blob.extend(assemble(native, &entries)?);
    Ok(blob)
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
        let count = if words[at] & 2047 == 53 {
            *words.get(at + 1).context("truncated custom shader data")? as usize
        } else {
            ((words[at] >> 24) & 127) as usize
        };
        ensure!(
            count > 0 && at + count <= words.len(),
            "invalid shader instruction"
        );
        out.push(words[at..at + count].to_vec());
        at += count;
    }
    Ok(out)
}

/// Restore the original fog interpolator in the checked batched particle
/// variants. Particle geometry and lighting remain native. Other variants
/// return None; the caller checks the pinned retail family's variant count.
pub fn build_particle(
    source: &[u8],
    target: &[u8],
    template: &[u8],
    profiles: &[Profile],
) -> Result<Option<Vec<u8>>> {
    build_forward_fog(source, target, template, profiles, ForwardFog::Particle)
}

/// Replace only the terminal water fog block. Native waves, reflections,
/// lighting, material bindings and unmarked behavior remain intact.
pub fn build_water(
    source: &[u8],
    target: &[u8],
    template: &[u8],
    profiles: &[Profile],
) -> Result<Option<Vec<u8>>> {
    build_forward_fog(source, target, template, profiles, ForwardFog::Water)
}

#[derive(Clone, Copy, PartialEq)]
enum ForwardFog {
    Particle,
    Water,
}

fn build_forward_fog(
    source: &[u8],
    target: &[u8],
    template: &[u8],
    profiles: &[Profile],
    kind: ForwardFog,
) -> Result<Option<Vec<u8>>> {
    ensure!(!profiles.is_empty(), "no original forward fog profiles");
    let (source_hash, native_hash, source_range, native_range, scratch_count) = match kind {
        ForwardFog::Particle => (
            "955ddb23052e167e2842e990ff7698f3ed8ee05dd06ebda01096c21bb893e529",
            "28ab00cfa95506fefed79f14bc8474df332e804d6ffc6be11846c4f42fd72a5f",
            169..214,
            37..90,
            4,
        ),
        ForwardFog::Water => (
            "077372cb8294764877c1da9ae92e47a2a3a74fdb2f09b18a3476772e374459db",
            "e39fd1ff7ee8acd49bbb4a3af603d82fb69d38ab96aac9e722f548f54e180b92",
            306..354,
            319..375,
            3,
        ),
    };
    let (_, original) = pinned_code(source, source_hash)?;
    let (_, reference) = pinned_code(template, native_hash)?;
    let program = |code: &[u8]| -> Result<Vec<Vec<u32>>> {
        let entries = chunks(code)?;
        instructions(&words(
            &entries
                .iter()
                .find(|(t, _)| t == b"SHDR")
                .context("missing particle program")?
                .1,
        )?)
    };
    let source_ops = program(original)?;
    let template_ops = program(reference)?;
    let source_fog = &source_ops[source_range];
    let native_fog = &template_ops[native_range];
    if kind == ForwardFog::Particle {
        ensure!(
            source_fog[0]
                == [
                    0x09000000, 0x00100072, 2, 0x00101246, 0, 0x80208246, 0x41, 2, 20
                ]
                && source_fog[44] == [0x05002036, 0x00102082, 7, 0x0010003a, 0],
            "original particle fog boundary changed"
        );
    } else {
        ensure!(
            source_fog[0] == native_fog[0]
                && source_fog[0]
                    == [
                        0x09000000, 0x00100072, 1, 0x00101246, 4, 0x80208246, 0x41, 1, 20
                    ]
                && source_ops[354..] == [vec![0x0100003e]]
                && template_ops[375..] == [vec![0x0100003e]],
            "original water fog boundary changed"
        );
    }
    let offset = 4 + u32_at(target, 0)? as usize * 264;
    let native = target.get(offset..).context("truncated particle shader")?;
    ensure!(
        checksum(native)? == native[4..20],
        "particle checksum mismatch"
    );
    let mut entries = chunks(native)?;
    let index = entries
        .iter()
        .position(|(t, _)| t == b"SHDR")
        .context("missing particle program")?;
    let original_words = words(&entries[index].1)?;
    let mut native_ops = instructions(&original_words)?;
    let matches: Vec<_> = native_ops
        .windows(native_fog.len())
        .enumerate()
        .filter_map(|(i, block)| (block == native_fog).then_some(i))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    ensure!(matches.len() == 1, "ambiguous particle fog block");
    let start = matches[0];
    let end = start + native_fog.len();
    match kind {
        ForwardFog::Particle => ensure!(
            native_ops.get(end..end + 7) == Some(&template_ops[90..97]),
            "particle fog live-register boundary changed"
        ),
        ForwardFog::Water => ensure!(
            native_ops[end..] == [vec![0x0100003e]],
            "water fog must end at the return instruction"
        ),
    }
    let ti = native_ops
        .iter()
        .position(|ins| ins[0] & 2047 == 104)
        .context("missing particle temporaries")?;
    let base = native_ops[ti][1];
    ensure!(base <= 64, "unexpected particle temporaries");
    native_ops[ti][1] = base + scratch_count + 1;
    let profile_register = base + scratch_count;
    let mut body = Vec::new();
    let mut remaps = [0; 3];
    for ins in source_fog {
        let mut fixed = vec![ins[0]];
        let mut at = 1;
        while at < ins.len() {
            let mut token = ins[at];
            at += 1;
            let extended = token & 0x80000000 != 0;
            let extension = if extended {
                let value = ins[at];
                at += 1;
                Some(value)
            } else {
                None
            };
            let operand_kind = (token >> 12) & 255;
            let dimensions = ((token >> 20) & 3) as usize;
            let mut indices = ins[at..at + dimensions].to_vec();
            at += dimensions;
            if operand_kind == 4 {
                ensure!(dimensions == 0 && !extended, "unexpected particle literal");
                let count = if token & 3 == 2 { 4 } else { 1 };
                fixed.push(token);
                fixed.extend(&ins[at..at + count]);
                at += count;
                continue;
            }
            match operand_kind {
                0 => {
                    ensure!(
                        dimensions == 1 && indices[0] < scratch_count,
                        "unexpected original particle temporary"
                    );
                    indices[0] += base;
                }
                1 => {
                    if kind == ForwardFog::Particle {
                        ensure!(indices == [0], "unexpected original particle position");
                        indices[0] = 6;
                    } else {
                        ensure!(indices == [4], "unexpected original water position");
                    }
                }
                8 => {
                    ensure!(
                        dimensions == 2
                            && indices[0] == if kind == ForwardFog::Particle { 2 } else { 1 },
                        "unexpected particle constant buffer"
                    );
                    indices[0] = 1;
                    if indices[1] == 18 {
                        let component = (token >> 4) & 3;
                        if component == 0 {
                            token += 0x10;
                            remaps[0] += 1;
                        } else {
                            ensure!(
                                component == 1,
                                "unexpected original particle height operand"
                            );
                            token = (token & !(0x00300000 | 0x000ff000 | 0x30)) | 0x00100000 | 0x30;
                            indices = vec![profile_register];
                            remaps[1] += 1;
                        }
                    } else if indices[1] == 15 && (token >> 2) & 3 == 1 {
                        token = (token & !(0x00300000 | 0x000ff000)) | 0x00100000;
                        indices = vec![profile_register];
                        remaps[2] += 1;
                    }
                }
                2 => {}
                _ => anyhow::bail!("unexpected particle fog operand"),
            }
            fixed.push(token);
            fixed.extend(extension);
            fixed.extend(indices);
        }
        fixed[0] = (fixed[0] & !0x7f000000) | (fixed.len() as u32) << 24;
        body.push(fixed);
    }
    ensure!(remaps == [1, 2, 1], "particle fog bindings changed");
    let mut updated = original_words[..2].to_vec();
    updated.extend(native_ops[..start].iter().flatten());
    let mut seen = std::collections::BTreeSet::new();
    for profile in profiles {
        ensure!(
            profile.marker.is_finite()
                && profile.marker < -32000.0
                && seen.insert(profile.marker.to_bits())
                && profile.color.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid particle fog profile"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            profile_register,
            0x0020803a,
            1,
            18,
            0x00004001,
            profile.marker.to_bits(),
            0x0304001f,
            0x0010003a,
            profile_register,
        ]);
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
        if kind == ForwardFog::Water {
            // The original terminal block consumes the water color in r0.xyz.
            // Copy it to scratch, so the native branch retains its live inputs.
            updated.extend([0x05000036, 0x00100072, base, 0x00100246, 0]);
        }
        updated.extend(body.iter().flatten());
        updated.push(0x01000012); // else: next marked profile, then native fallback
    }
    updated.extend(native_fog.iter().flatten());
    updated.extend(std::iter::repeat_n(0x01000015, profiles.len()));
    updated.extend(native_ops[end..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    let mut result = target[..offset].to_vec();
    result.extend(assemble(native, &entries)?);
    Ok(Some(result))
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

pub fn cloud_source_path(lit: bool) -> String {
    format!("{CLOUD_PREFIX}_____0{}{}", u8::from(lit), "_".repeat(60))
}

/// Restore the original cloud fog blend and lighting, retaining native code
/// for unmarked maps. Texture/sampler slots and the sun row moved in 0.26.
pub fn build_cloud(source: &[u8], target: &[u8], markers: &[f32], lit: bool) -> Result<Vec<u8>> {
    ensure!(!markers.is_empty(), "no original cloud markers");
    let (_, source) = pinned_code(
        source,
        if lit {
            "4a1f1f60555082e2760943b27e88ce7f224598445f554ba2dfac9624f1456495"
        } else {
            "ef59da23ce56c9bfc9478c07357467ce433433f55e2fc76f79f3d1b220ea41dd"
        },
    )?;
    let hash = format!("{:x}", Sha256::digest(target));
    ensure!(
        if lit {
            matches!(
                hash.as_str(),
                "549764f07751703b0758d04a28d6a39df2a0f2fb3d70e21c80b746315dae4db4"
                    | "b645b48c5abb456cf5794892b75a9b1939cf4b2f2cd93dacf0b1a697dd5a2683"
            )
        } else {
            hash == "3610a53b1836c147bb2f7c82807cc6daba16d0017315e0af111f1a3625c08dac"
        },
        "unsupported native cloud variant"
    );
    let (offset, native) = pinned_code(target, &hash)?;
    let original = chunks(source)?;
    let mut entries = chunks(native)?;
    let chunk = |items: &[([u8; 4], Vec<u8>)], tag: &[u8; 4]| -> Result<usize> {
        items
            .iter()
            .position(|(t, _)| t == tag)
            .context("missing cloud chunk")
    };
    let index = chunk(&entries, b"SHDR")?;
    let native_words = words(&entries[index].1)?;
    let mut native_ops = instructions(&native_words)?;
    let source_ops = instructions(&words(&original[chunk(&original, b"SHDR")?].1)?)?;
    let temps = |ops: &[Vec<u32>]| -> Result<usize> {
        ops.iter()
            .position(|ins| ins[0] & 2047 == 104)
            .context("cloud temporary declaration missing")
    };
    let nt = temps(&native_ops)?;
    let st = temps(&source_ops)?;
    let scratch = native_ops[nt][1];
    ensure!(
        source_ops[st][1] <= scratch,
        "original cloud needs more temporaries"
    );
    // All source interpolants must already be supplied by the native vertex
    // shader/signature. No guessed varying or normal reconstruction.
    for ins in &source_ops[..st] {
        if matches!(ins[0] & 2047, 98..=100) {
            ensure!(
                native_ops[..nt].contains(ins),
                "original cloud interpolant is absent from native variant"
            );
        }
    }
    let reflection = chunk(&entries, b"RDEF")?;
    let rdef = &mut entries[reflection].1;
    let string = |data: &[u8], at: usize| -> Result<String> {
        let tail = data.get(at..).context("missing cloud reflection string")?;
        let len = tail
            .iter()
            .position(|b| *b == 0)
            .context("unterminated cloud reflection string")?;
        Ok(std::str::from_utf8(&tail[..len])?.to_owned())
    };
    let mut sampler = None;
    let mut texture = None;
    let resource_count = u32_at(rdef, 8)? as usize;
    let resources = u32_at(rdef, 12)? as usize;
    for i in 0..resource_count {
        let at = resources + i * 32;
        match string(rdef, u32_at(rdef, at)? as usize)?.as_str() {
            "s_linear" => sampler = Some(u32_at(rdef, at + 20)?),
            "sky_clouds_texture" => texture = Some(u32_at(rdef, at + 20)?),
            _ => (),
        }
    }
    let sampler = sampler.context("native cloud sampler missing")?;
    ensure!(texture == Some(1), "native cloud texture slot changed");
    let mut changed = 0;
    let count = u32_at(rdef, 0)? as usize;
    let buffers = u32_at(rdef, 4)? as usize;
    for i in 0..count {
        let at = buffers + i * 24;
        if string(rdef, u32_at(rdef, at)? as usize)? != "$Globals" {
            continue;
        }
        let count = u32_at(rdef, at + 4)? as usize;
        let variables = u32_at(rdef, at + 8)? as usize;
        for j in 0..count {
            let at = variables + j * 24;
            if string(rdef, u32_at(rdef, at)? as usize)? == "sky_clouds_parameters1" {
                ensure!(
                    u32_at(rdef, at + 4)? == 16
                        && u32_at(rdef, at + 8)? == 16
                        && u32_at(rdef, at + 12)? == 0,
                    "cloud fog row layout changed"
                );
                rdef[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
                changed += 1;
            }
        }
    }
    ensure!(changed == 1, "cloud fog binding missing");
    let mut body = source_ops[st + 1..].to_vec();
    let (mut textures, mut samplers, mut sun_rows) = (0, 0, 0);
    for ins in &mut body {
        for i in 1..ins.len() {
            if i + 1 < ins.len() && ins[i] == 0x00107e46 && ins[i + 1] == 0 {
                ins[i + 1] = 1;
                textures += 1;
            }
            if i + 1 < ins.len() && ins[i] == 0x00106000 && ins[i + 1] == 0 {
                ins[i + 1] = sampler;
                samplers += 1;
            }
            if i + 2 < ins.len() && ins[i] & 0xfffff000 == 0x00208000 && ins[i + 1..i + 3] == [0, 3]
            {
                ins[i + 2] = 4;
                sun_rows += 1;
            }
        }
    }
    ensure!(
        textures == 3 && samplers == 3 && sun_rows == usize::from(lit),
        "original cloud binding remap mismatch"
    );
    native_ops[nt][1] += 1;
    let mut updated = native_words[..2].to_vec();
    updated.extend(native_ops[..nt + 1].iter().flatten());
    let mut seen = std::collections::BTreeSet::new();
    for marker in markers {
        ensure!(
            marker.is_finite() && *marker < -32000.0 && seen.insert(marker.to_bits()),
            "invalid cloud marker"
        );
        updated.extend([
            0x08000018,
            0x00100082,
            scratch,
            0x0020803a,
            1,
            18,
            0x00004001,
            marker.to_bits(),
            0x0304001f,
            0x0010003a,
            scratch,
        ]);
        updated.extend(body.iter().flatten());
        updated.push(0x01000015);
    }
    updated.extend(native_ops[nt + 1..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[index].1 = bytes(&updated);
    let mut blob = target[..offset].to_vec();
    blob.extend(assemble(native, &entries)?);
    Ok(blob)
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
    fn custom_data_instruction_lengths_are_checked() {
        assert_eq!(
            instructions(&[0, 6, 53, 3, 0, 0x0100003e]).unwrap(),
            vec![vec![53, 3, 0], vec![0x0100003e]]
        );
        assert!(instructions(&[0, 3, 53]).is_err());
        assert!(instructions(&[0, 4, 53, 8]).is_err());
    }

    #[test]
    #[ignore = "requires WEATHER_SHADER_AUDIT with extracted pinned retail shader blobs"]
    fn retail_water_fog_preserves_material_and_native_fallback() {
        let root = std::path::PathBuf::from(std::env::var("WEATHER_SHADER_AUDIT").unwrap());
        let source =
            std::fs::read(root.join("original-e4f112a3322ff4772e0fa9404d5a7f9f.blob")).unwrap();
        let template =
            std::fs::read(root.join("native-915734a684b44d6d7f5bb3a2e8945e50.blob")).unwrap();
        let profiles = [
            Profile {
                marker: -32189.0,
                color: [0.841f32.powf(2.2); 3],
            },
            Profile {
                marker: -32198.0,
                color: [0.875f32.powf(2.2); 3],
            },
        ];
        let directory = root.join("native-water-pixels/overlay").join(WATER_PREFIX);
        let mut patched = 0;
        for entry in std::fs::read_dir(directory).unwrap() {
            let target = std::fs::read(entry.unwrap().path()).unwrap();
            let Some(result) = build_water(&source, &target, &template, &profiles).unwrap() else {
                continue;
            };
            patched += 1;
            let offset = 4 + u32_at(&target, 0).unwrap() as usize * 264;
            assert_eq!(&result[..offset], &target[..offset]);
            assert_eq!(
                checksum(&result[offset..]).unwrap(),
                result[offset + 4..offset + 20]
            );
            let old = chunks(&target[offset..]).unwrap();
            let new = chunks(&result[offset..]).unwrap();
            for ((tag, a), (new_tag, b)) in old.iter().zip(&new) {
                assert_eq!(tag, new_tag);
                if tag != b"SHDR" {
                    assert_eq!(a, b);
                    continue;
                }
                let old = instructions(&words(a).unwrap()).unwrap();
                let new = instructions(&words(b).unwrap()).unwrap();
                let start = old.len() - 57;
                for (a, b) in old[..start].iter().zip(&new) {
                    if a[0] & 2047 == 104 {
                        assert_eq!(b[1], a[1] + 4);
                    } else {
                        assert_eq!(a, b);
                    }
                }
                // All native lighting precedes the guard. Its complete fog
                // survives once as the fallback, followed only by endif/ret.
                assert_eq!(
                    new.windows(56)
                        .filter(|w| *w == &old[start..start + 56])
                        .count(),
                    1
                );
                assert!(new.ends_with(&[vec![0x01000015], vec![0x01000015], vec![0x0100003e]]));
            }
        }
        assert_eq!(patched, 8);
        assert!(build_water(&source, &template, &template, &[]).is_err());
        assert!(
            build_water(
                &source,
                &template,
                &template,
                &[profiles[0].clone(), profiles[0].clone()]
            )
            .is_err()
        );
        let mut corrupt = source.clone();
        corrupt[8] ^= 1;
        assert!(build_water(&corrupt, &template, &template, &profiles).is_err());
    }

    #[test]
    #[ignore = "requires WEATHER_SHADER_AUDIT with extracted pinned retail shader blobs"]
    fn retail_particle_fog_preserves_lighting_and_unmarked_path() {
        let root = std::path::PathBuf::from(std::env::var("WEATHER_SHADER_AUDIT").unwrap());
        let source =
            std::fs::read(root.join("original-9b11960c94a13675b92cbebf3d4dc919.blob")).unwrap();
        let template =
            std::fs::read(root.join("native-2a73464d45e748c014a63b25f5eaa660.blob")).unwrap();
        let profiles = [
            Profile {
                marker: -32188.0,
                color: [0.969f32.powf(2.2), 1.238f32.powf(2.2), 1.583f32.powf(2.2)],
            },
            Profile {
                marker: -32198.0,
                color: [0.875f32.powf(2.2); 3],
            },
        ];
        let directory = root
            .join("native-particle-vertices/overlay")
            .join(PARTICLE_PREFIX);
        let mut patched = 0;
        for entry in std::fs::read_dir(directory).unwrap() {
            let target = std::fs::read(entry.unwrap().path()).unwrap();
            let Some(result) = build_particle(&source, &target, &template, &profiles).unwrap()
            else {
                continue;
            };
            patched += 1;
            let offset = 4 + u32_at(&target, 0).unwrap() as usize * 264;
            assert_eq!(&result[..offset], &target[..offset]);
            let old = chunks(&target[offset..]).unwrap();
            let new = chunks(&result[offset..]).unwrap();
            assert_eq!(
                checksum(&result[offset..]).unwrap(),
                result[offset + 4..offset + 20]
            );
            for ((tag, a), (new_tag, b)) in old.iter().zip(&new) {
                assert_eq!(tag, new_tag);
                if tag != b"SHDR" {
                    assert_eq!(a, b);
                    continue;
                }
                let old = instructions(&words(a).unwrap()).unwrap();
                let new = instructions(&words(b).unwrap()).unwrap();
                let end = old
                    .iter()
                    .position(|i| i == &[0x05002036, 0x00102082, 7, 0x0010000a, 0])
                    .unwrap()
                    + 1;
                // Native lighting after fog survives exactly; native fog remains
                // as the unmarked branch, immediately before the inserted endif.
                assert!(new.ends_with(&old[end..]));
                assert!(
                    new.windows(54)
                        .any(|w| w[..53] == old[end - 53..end] && w[53] == [0x01000015])
                );
            }
        }
        assert_eq!(patched, 24);
        let bad = [Profile {
            marker: -32198.0,
            color: [f32::NAN; 3],
        }];
        assert!(build_particle(&source, &template, &template, &bad).is_err());
    }
    #[test]
    #[ignore = "requires WEATHER_SHADER_AUDIT with extracted pinned retail shader blobs"]
    fn retail_shader_adapters_preserve_native_fallbacks() {
        let root = std::path::PathBuf::from(std::env::var("WEATHER_SHADER_AUDIT").unwrap());
        let read = |name: &str| std::fs::read(root.join(format!("{name}.blob"))).unwrap();
        let profile = Profile {
            marker: -32197.0,
            color: [1.583f32.powf(2.2); 3],
        };
        let fog = build(
            &read("original-atmospheric_scattering_on_geometry_temporary.ps"),
            &read("target-atmospheric_scattering_on_geometry_temporary.ps"),
            &[profile],
        )
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(fog)),
            "7e810b1fdc6917591dee28f05b0adb8ec7179bf6f2cfba64b5ab3d42e2c7bbfd"
        );
        let sky = build_sky(
            &read("original-atmosphere.ps"),
            &read("target-atmosphere.ps"),
            &[-32197.0],
        )
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(sky)),
            "d3f3051d8765127b71f2f21a9727f6866f88a685c88c4faf71aeaac65cddaf68"
        );
        for variant in ["000", "010", "110"] {
            let lit = variant != "000";
            let native = read(&format!("native-cloud-{variant}"));
            let source = read(if lit {
                "original-cloud-01"
            } else {
                "original-cloud-00"
            });
            let result = build_cloud(&source, &native, &[-32197.0, -32188.0], lit).unwrap();
            let offset = 4 + u32_at(&native, 0).unwrap() as usize * 264;
            let old = chunks(&native[offset..]).unwrap();
            let new = chunks(&result[offset..]).unwrap();
            for ((ot, ob), (nt, nb)) in old.iter().zip(&new) {
                assert_eq!(ot, nt);
                if ot == b"SHDR" {
                    let a = instructions(&words(ob).unwrap()).unwrap();
                    let b = instructions(&words(nb).unwrap()).unwrap();
                    let body = a.iter().position(|ins| ins[0] & 2047 == 104).unwrap() + 1;
                    let len = a.len() - body;
                    assert_eq!(&a[body..], &b[b.len() - len..]);
                } else if ot != b"RDEF" {
                    assert_eq!(ob, nb);
                }
            }
            assert!(build_cloud(&source, &native, &[-32197.0, -32197.0], lit).is_err());
            let mut corrupted = native.clone();
            corrupted[offset + 60] ^= 1;
            assert!(build_cloud(&source, &corrupted, &[-32197.0], lit).is_err());
        }
        let source = read("original-make_mie_rayleigh_texture.ps");
        let native = read("target-make_mie_rayleigh_texture.ps");
        let sky = read("target-atmosphere.ps");
        let result = build_scattering(
            &source,
            &native,
            &sky,
            &[
                ScatteringProfile {
                    marker: -32197.0,
                    rayleigh_sun: 4.67,
                },
                ScatteringProfile {
                    marker: -32188.0,
                    rayleigh_sun: 0.0,
                },
            ],
        )
        .unwrap();
        let offset = 4 + u32_at(&native, 0).unwrap() as usize * 264;
        let old = chunks(&native[offset..]).unwrap();
        let new = chunks(&result[offset..]).unwrap();
        let reflection = |items: &[([u8; 4], Vec<u8>)]| {
            let r = &items.iter().find(|(tag, _)| tag == b"RDEF").unwrap().1;
            let name = |at: usize| {
                let end = r[at..].iter().position(|b| *b == 0).unwrap() + at;
                std::str::from_utf8(&r[at..end]).unwrap().to_owned()
            };
            let table_end = u32_at(r, 12).unwrap() + u32_at(r, 8).unwrap() * 32;
            let buffers = u32_at(r, 4).unwrap() as usize;
            let mut globals = Vec::new();
            for i in 0..u32_at(r, 0).unwrap() as usize {
                let at = buffers + i * 24;
                let no = u32_at(r, at).unwrap();
                assert!(
                    no >= table_end,
                    "binding table must precede reflection names"
                );
                if name(no as usize) != "$Globals" {
                    continue;
                }
                let vars = u32_at(r, at + 8).unwrap() as usize;
                for j in 0..u32_at(r, at + 4).unwrap() as usize {
                    let v = vars + j * 24;
                    globals.push((
                        name(u32_at(r, v).unwrap() as usize),
                        u32_at(r, v + 4).unwrap(),
                        u32_at(r, v + 8).unwrap(),
                        u32_at(r, v + 12).unwrap(),
                    ));
                }
            }
            globals
        };
        assert_eq!(
            reflection(&old),
            reflection(&new),
            "native lookup constant names/layout must be preserved"
        );
        for ((ot, ob), (nt, nb)) in old.iter().zip(&new) {
            assert_eq!(ot, nt);
            if ot == b"SHDR" {
                let a = instructions(&words(ob).unwrap()).unwrap();
                let b = instructions(&words(nb).unwrap()).unwrap();
                let start = a.iter().position(|i| i[0] & 2047 == 104).unwrap() + 1;
                assert_eq!(&a[start..], &b[b.len() - (a.len() - start)..]);
            } else if ot != b"RDEF" {
                assert_eq!(ob, nb);
            }
        }
        assert!(
            build_scattering(
                &source,
                &native,
                &sky,
                &[ScatteringProfile {
                    marker: -32197.0,
                    rayleigh_sun: f32::NAN
                }]
            )
            .is_err()
        );
    }

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
