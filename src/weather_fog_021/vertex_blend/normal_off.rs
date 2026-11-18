//! Original base-normal programs with explicit original masked selection.
//! These are additions only: every existing native shader is left untouched.
use super::masked::{BaseSample, WeightOrientation, masked_override};
use super::{assemble, bytes, chunks, instructions, pinned_code, words};
use crate::ResourceDb;
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Contract {
    quality: u8,
    vertex: u8,
    original_path: String,
    original_sha256: String,
    native_reference_path: String,
    native_reference_sha256: String,
    paths: Vec<String>,
    global_layout: GlobalLayout,
    blend_layout: BlendLayout,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GlobalLayout {
    Identical,
    #[serde(rename = "scene_time_396_to_400")]
    SceneTime396To400,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BlendLayout {
    ConstantZero,
    VertexAlpha {
        alpha_register: u32,
        weight_operand: [u32; 2],
        sample_coordinates: [u32; 2],
        sample_texture: u32,
        sample_sampler: u32,
        material_buffer: u32,
    },
}

fn value(raw: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        raw.get(at..at + 4)
            .context("reflection bounds")?
            .try_into()?,
    ))
}

fn string(raw: &[u8], at: usize) -> Result<&str> {
    let start = value(raw, at)? as usize;
    let tail = raw.get(start..).context("reflection name bounds")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("reflection terminator")?;
    Ok(std::str::from_utf8(&tail[..end])?)
}

fn reflected_variable(raw: &[u8], buffer: &str, variable: &str) -> Result<(usize, usize)> {
    let mut found = Vec::new();
    for i in 0..value(raw, 0)? as usize {
        let cb = value(raw, 4)? as usize + i * 24;
        if string(raw, cb)? != buffer {
            continue;
        }
        for j in 0..value(raw, cb + 4)? as usize {
            let at = value(raw, cb + 8)? as usize + j * 24;
            if string(raw, at)? == variable {
                found.push((cb, at));
            }
        }
    }
    ensure!(
        found.len() == 1,
        "ambiguous reflected variable {buffer}::{variable}"
    );
    Ok(found[0])
}

fn binding_slot(raw: &[u8], target: &str, kind: u32) -> Result<u32> {
    let mut found = Vec::new();
    for i in 0..value(raw, 8)? as usize {
        let at = value(raw, 12)? as usize + i * 32;
        if string(raw, at)? == target {
            ensure!(
                value(raw, at + 4)? == kind && value(raw, at + 24)? == 1,
                "binding kind/count changed"
            );
            found.push(value(raw, at + 20)?);
        }
    }
    ensure!(found.len() == 1, "ambiguous binding {target}");
    Ok(found[0])
}

fn adapt(original: &[u8], contract: &Contract) -> Result<Vec<u8>> {
    let offset = 4 + value(original, 0)? as usize * 264;
    let native = original.get(offset..).context("shader wrapper")?;
    let mut entries = chunks(native)?;
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
    if let GlobalLayout::SceneTime396To400 = contract.global_layout {
        let slot = binding_slot(&entries[rdef].1, "static_globals", 0)?;
        let mut reads = 0;
        for op in &mut ops {
            for at in 1..op.len().saturating_sub(2) {
                if op[at..at + 3] == [0x0020803a, slot, 24] {
                    op[at..at + 3].copy_from_slice(&[0x0020800a, slot, 25]);
                    reads += 1;
                }
            }
        }
        ensure!(reads == 1, "original scene-time read changed");
        let declarations: Vec<_> = ops
            .iter_mut()
            .filter(|op| op.len() == 4 && op[0] == 0x04000059 && op[2] == slot)
            .collect();
        ensure!(declarations.len() == 1, "static-global declaration changed");
        let declaration = declarations.into_iter().next().context("static globals")?;
        ensure!(declaration[3] == 25, "original static-global size changed");
        declaration[3] = 26;
        let raw = &mut entries[rdef].1;
        let (cb, at) = reflected_variable(raw, "static_globals", "scene_time")?;
        ensure!(
            value(raw, cb + 12)? == 400
                && value(raw, at + 4)? == 396
                && value(raw, at + 8)? == 4
                && value(raw, at + 12)? == 2,
            "original scene-time reflection changed"
        );
        raw[cb + 12..cb + 16].copy_from_slice(&416_u32.to_le_bytes());
        raw[at + 4..at + 8].copy_from_slice(&400_u32.to_le_bytes());
    }
    match contract.blend_layout {
        BlendLayout::ConstantZero => {
            // Original constant-alpha classes have q=1. All authored masked
            // factors are in[1,2], so pow(saturate(f+1),f)=1 exactly, matching
            // the original unmasked program's second-texture coefficient.
            ensure!(
                matches!(contract.vertex, 0 | 10 | 12 | 13 | 14),
                "unexpected constant-alpha class"
            );
        }
        BlendLayout::VertexAlpha {
            alpha_register,
            weight_operand,
            sample_coordinates,
            sample_texture,
            sample_sampler,
            material_buffer,
        } => {
            ensure!(
                matches!((contract.vertex, alpha_register), (1, 7) | (2, 9)),
                "vertex-alpha semantic changed"
            );
            let weight = [
                0x08000000,
                weight_operand[0],
                weight_operand[1],
                0x8010100a,
                0x41,
                alpha_register,
                0x00004001,
                1_f32.to_bits(),
            ];
            let sites: Vec<_> = ops
                .iter()
                .enumerate()
                .filter(|(_, op)| op.as_slice() == weight)
                .map(|(i, _)| i)
                .collect();
            ensure!(sites.len() == 1, "original interpolation site changed");
            let samples: Vec<_> = ops
                .iter()
                .filter(|op| op.len() == 9 && op[0] == 0x09000045 && op[6] == sample_texture)
                .collect();
            ensure!(
                samples.len() == 1
                    && samples[0][3..5] == sample_coordinates
                    && samples[0][7..] == [0x00106000, sample_sampler],
                "original base-sample contract changed"
            );
            ensure!(
                binding_slot(&entries[rdef].1, "t_base", 2)? == sample_texture
                    && binding_slot(&entries[rdef].1, "deferred_material_parameters", 0)?
                        == material_buffer,
                "original binding slot changed"
            );
            let declarations: Vec<_> = ops.iter_mut().filter(|op| op[0] & 2047 == 104).collect();
            ensure!(declarations.len() == 1, "temporary declaration changed");
            let declaration = declarations.into_iter().next().context("temporaries")?;
            let scratch = declaration[1];
            declaration[1] += 1;
            let body = masked_override(
                alpha_register,
                BaseSample {
                    coordinates: sample_coordinates,
                    texture: sample_texture,
                    sampler: sample_sampler,
                    material_buffer,
                },
                scratch,
                weight_operand,
                WeightOrientation::Second,
            );
            ops.splice(sites[0] + 1..sites[0] + 1, body);
            let raw = &mut entries[rdef].1;
            let (_, at) = reflected_variable(
                raw,
                "deferred_material_parameters",
                "vertex_blend_parameters",
            )?;
            ensure!(
                value(raw, at + 4)? == 160
                    && value(raw, at + 8)? == 12
                    && value(raw, at + 12)? == 0,
                "original unused blend binding changed"
            );
            // Native binder supplies(falloff, encoded factor, tiling), at224.
            raw[at + 4..at + 8].copy_from_slice(&224_u32.to_le_bytes());
            raw[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
        }
    }
    let mut result = old[..2].to_vec();
    result.extend(ops.iter().flatten());
    result[1] = u32::try_from(result.len())?;
    instructions(&result)?;
    entries[shdr].1 = bytes(&result);
    let mut output = original[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

type Input = (String, u32, u32, u32, u32);

fn input_signature(raw: &[u8]) -> Result<BTreeMap<Input, u8>> {
    let word = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            raw.get(at..at + 4)
                .ok_or_else(|| anyhow::anyhow!("signature bounds"))?
                .try_into()?,
        ))
    };
    let count = word(0)? as usize;
    ensure!(
        word(4)? == 8 && count <= 32 && raw.len() >= 8 + count * 24,
        "invalid signature header"
    );
    let mut result = BTreeMap::new();
    for i in 0..count {
        let at = 8 + i * 24;
        let name = word(at)? as usize;
        let tail = raw
            .get(name..)
            .ok_or_else(|| anyhow::anyhow!("semantic name bounds"))?;
        let end = tail
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| anyhow::anyhow!("semantic terminator"))?;
        let name = std::str::from_utf8(&tail[..end])?.to_owned();
        let mask = raw[at + 20];
        ensure!(raw[at + 21] & !mask == 0, "invalid input component mask");
        let key = (
            name,
            word(at + 4)?,
            word(at + 8)?,
            word(at + 12)?,
            word(at + 16)?,
        );
        ensure!(result.insert(key, mask).is_none(), "duplicate semantic");
    }
    Ok(result)
}

pub fn build(donor: &mut ResourceDb, stock: &mut ResourceDb) -> Result<BTreeMap<String, Vec<u8>>> {
    let contracts: Vec<Contract> = serde_json::from_str(include_str!("normal-off-programs.json"))?;
    ensure!(contracts.len() == 348, "incomplete normal-off family");
    let shipped: BTreeSet<_> = stock.paths().map(str::to_owned).collect();
    let mut identities = BTreeSet::new();
    let mut result = BTreeMap::new();
    for contract in contracts {
        ensure!(
            matches!(contract.quality, 0 | 3)
                && matches!(contract.vertex, 0 | 1 | 2 | 10 | 12 | 13 | 14),
            "unknown original layout"
        );
        identities.insert((contract.quality, contract.vertex));
        ensure!(!contract.paths.is_empty(), "empty alias family");
        let original = donor.read(&contract.original_path)?;
        let native = stock.read(&contract.native_reference_path)?;
        let (_, original_code) = pinned_code(&original, &contract.original_sha256)?;
        let (_, native_code) = pinned_code(&native, &contract.native_reference_sha256)?;
        let original_chunks = chunks(original_code)?;
        let native_chunks = chunks(native_code)?;
        for tag in [b"ISGN", b"OSGN"] {
            let original_signature = original_chunks.iter().find(|(name, _)| name == tag);
            let native_signature = native_chunks.iter().find(|(name, _)| name == tag);
            let (_, original_signature) =
                original_signature.ok_or_else(|| anyhow::anyhow!("missing original signature"))?;
            let (_, native_signature) =
                native_signature.ok_or_else(|| anyhow::anyhow!("missing native signature"))?;
            if tag == b"OSGN" {
                ensure!(
                    original_signature == native_signature,
                    "output signature mismatch"
                );
            } else {
                let original_inputs = input_signature(original_signature)?;
                let native_inputs = input_signature(native_signature)?;
                // A normal-off consumer reads only tangent Z where a native
                // normal-blended shader needs XYZ. Exact semantic/register/type
                // identity plus component inclusion proves compatible inputs.
                ensure!(
                    original_inputs.iter().all(|(key, mask)| native_inputs
                        .get(key)
                        .is_some_and(|available| mask & !available == 0)),
                    "original input is not supplied by the native vertex contract"
                );
            }
        }
        let output = adapt(&original, &contract)?;
        for path in contract.paths {
            ensure!(
                !shipped.contains(&path),
                "refusing to replace an existing native shader: {path}"
            );
            ensure!(
                result.insert(path, output.clone()).is_none(),
                "duplicate output path"
            );
        }
    }
    ensure!(
        identities.len() == 14 && result.len() == 1218,
        "incomplete normal-off closure"
    );
    Ok(result)
}
