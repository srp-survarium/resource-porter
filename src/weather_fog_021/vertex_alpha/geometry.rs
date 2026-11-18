//! Original painted cutoff inside ordinary native material shaders.
//! Private materials select the ordinary path because their original blend was
//! inactive. Other material features and the unmarked native path are preserved.
use super::{
    assemble, bytes, chunks, instructions, literal, op, pinned_code, profiles::AuthoredProfile,
    words,
};
use crate::{ResourceDb, weather_fog_021::u32_at};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Original {
    vertex: u8,
    path: String,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    vertex: u8,
    paths: Vec<String>,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    original: Vec<Original>,
    native: Vec<Native>,
}

#[derive(Clone, Copy)]
enum PaintedInput {
    Zero,
    PackedUv,
    ColorAlpha,
}
impl PaintedInput {
    fn from_vertex(vertex: u8) -> Result<Self> {
        match vertex {
            0 | 10 | 12 | 13 | 14 => Ok(Self::Zero),
            1 => Ok(Self::PackedUv),
            2 => Ok(Self::ColorAlpha),
            _ => anyhow::bail!("unreviewed painted-alpha vertex class"),
        }
    }
    fn register(self) -> Option<u32> {
        match self {
            Self::Zero => None,
            Self::PackedUv => Some(7),
            Self::ColorAlpha => Some(9),
        }
    }
    fn operand(self) -> [u32; 2] {
        self.register()
            .map_or_else(|| literal(0.0), |r| [0x0010100a, r])
    }
}

fn signature_entry(raw: &[u8], register: u32) -> Result<usize> {
    let mut found = Vec::new();
    for i in 0..u32_at(raw, 0)? as usize {
        let at = 8 + i * 24;
        if u32_at(raw, at + 16)? == register {
            found.push(at);
        }
    }
    ensure!(found.len() == 1, "ambiguous painted-alpha input signature");
    Ok(found[0])
}
fn semantic(raw: &[u8], at: usize) -> Result<&[u8]> {
    let start = u32_at(raw, at)? as usize;
    let tail = raw.get(start..).context("semantic bounds")?;
    Ok(&tail[..tail
        .iter()
        .position(|b| *b == 0)
        .context("semantic terminator")?])
}
fn enable_input(
    entries: &mut [([u8; 4], Vec<u8>)],
    ops: &mut Vec<Vec<u32>>,
    original: &[([u8; 4], Vec<u8>)],
    input: PaintedInput,
) -> Result<()> {
    let Some(register) = input.register() else {
        return Ok(());
    };
    let source_sig = &original
        .iter()
        .find(|(tag, _)| tag == b"ISGN")
        .context("original signature")?
        .1;
    let target_sig = &mut entries
        .iter_mut()
        .find(|(tag, _)| tag == b"ISGN")
        .context("native signature")?
        .1;
    let source_at = signature_entry(source_sig, register)?;
    let target_at = signature_entry(target_sig, register)?;
    ensure!(
        semantic(source_sig, source_at)? == semantic(target_sig, target_at)?
            && source_sig[source_at + 4..source_at + 20]
                == target_sig[target_at + 4..target_at + 20]
            && target_sig[target_at + 20] & 1 == 1
            && source_sig[source_at + 21] & 1 == 1,
        "native vertex contract does not supply original painted alpha"
    );
    target_sig[target_at + 21] |= 1;
    let source_ops = instructions(&words(
        &original
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("original program")?
            .1,
    )?)?;
    let source_decl = source_ops
        .iter()
        .find(|o| o.len() == 3 && o[0] & 2047 == 98 && o[2] == register)
        .context("original alpha declaration")?;
    ensure!(
        source_decl[1] & 0x10 != 0,
        "original declaration lacks alpha component"
    );
    if let Some(existing) = ops
        .iter_mut()
        .find(|o| o.len() == 3 && o[0] & 2047 == 98 && o[2] == register)
    {
        ensure!(
            existing[0] == source_decl[0] && existing[1] & !0xf0 == source_decl[1] & !0xf0,
            "painted alpha interpolation changed"
        );
        existing[1] |= 0x10;
    } else {
        let mut decl = source_decl.clone();
        decl[1] = (decl[1] & !0xf0) | 0x10;
        let at = ops
            .iter()
            .position(|o| o[0] == 0x02000068)
            .context("native temps")?;
        ops.insert(at, decl);
    }
    Ok(())
}

fn convert(blob: &[u8], contract: &Native, original: &[u8]) -> Result<Vec<u8>> {
    let input = PaintedInput::from_vertex(contract.vertex)?;
    let (offset, native) = pinned_code(blob, &contract.sha256)?;
    let mut entries = chunks(native)?;
    let original_entries = chunks(original)?;
    let index = entries
        .iter()
        .position(|(t, _)| t == b"SHDR")
        .context("native program")?;
    let old = words(&entries[index].1)?;
    let mut ops = instructions(&old)?;
    enable_input(&mut entries, &mut ops, &original_entries, input)?;
    let temp_decl = ops
        .iter_mut()
        .find(|o| o[0] == 0x02000068)
        .context("native temps")?;
    let scratch = temp_decl[1];
    ensure!(scratch < 64, "unexpected temporary layout");
    temp_decl[1] += 1;
    let cutoffs: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            o.len() == 9
                && o[0] == 0x09000000
                && o.windows(4)
                    .any(|v| v[0] == 0x8020802a && v[1] == 0x41 && v[3] == 15)
        })
        .map(|(i, _)| i)
        .collect();
    ensure!(cutoffs.len() == 1, "ambiguous alpha cutoff");
    let at = cutoffs[0];
    let cutoff = ops[at].clone();
    let (alpha, slot) = if cutoff[3] == 0x8020802a {
        ensure!(
            cutoff[7..] == [0x00004001, 1.0f32.to_bits()],
            "constant alpha changed"
        );
        (cutoff[7..9].to_vec(), cutoff[5])
    } else {
        ensure!(
            cutoff[5] == 0x8020802a && (cutoff[3] >> 12) & 255 == 0,
            "base alpha input changed"
        );
        (cutoff[3..5].to_vec(), cutoff[7])
    };
    ensure!(
        slot <= 1 && ops[at + 1][0] & 2047 == 49 && ops[at + 2][0] & 2047 == 13,
        "cutoff boundary changed"
    );
    let alpha_ref = [0x0020802a, slot, 15];
    let dst = |c: u32| [0x00100002 | (1 << (4 + c)), scratch];
    let src = |c: u32| [0x0010000a | (c << 4), scratch];
    let mut added = Vec::new();
    for profile in AuthoredProfile::ALL {
        let (dark, light, multiply) = profile.mask_parameters();
        added.extend([
            op(24, &[&dst(0), &alpha_ref, &literal(profile.marker())]),
            op(0x4001f, &[&src(0)]),
            op(47, &[&dst(0), &input.operand()]),
            op(56, &[&dst(1), &src(0), &literal(light)]),
            op(56, &[&dst(1), &src(1), &literal(7.0)]),
            op(25, &[&dst(1), &src(1)]),
            op(56, &[&dst(2), &literal(dark), &literal(0.7)]),
            op(56, &[&dst(0), &src(0), &src(2)]),
            op(25, &[&dst(0), &src(0)]),
            op(50, &[&dst(0), &alpha, &src(0), &literal(0.2)]),
            op(0, &[&dst(0), &src(1), &src(0)]),
            op(0x2038, &[&dst(1), &alpha, &literal(3.0)]),
            op(0, &[&dst(1), &src(1), &literal(-1.0)]),
            op(
                50,
                &[
                    &dst(1),
                    &literal(if multiply { 1.0 } else { 0.0 }),
                    &src(1),
                    &literal(1.0),
                ],
            ),
            op(50, &[&cutoff[1..3], &src(0), &src(1), &literal(-0.4)]),
            op(18, &[]),
        ]);
    }
    added.push(cutoff);
    added.extend((0..AuthoredProfile::ALL.len()).map(|_| op(21, &[])));
    ops.splice(at..=at, added);
    let mut program = old[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let mut output = blob[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

/// Apply the same proven cutoff to an ordinary template constructed and checked
/// by the sibling parallax closure. The donor vertex contract remains pinned.
pub(super) fn adapt_template(blob: &[u8], vertex: u8, donor: &mut ResourceDb) -> Result<Vec<u8>> {
    let catalog: Catalog = serde_json::from_str(include_str!("geometry_programs.json"))?;
    let row = catalog
        .original
        .iter()
        .find(|r| r.vertex == vertex)
        .context("missing original vertex layout")?;
    let original = donor.read(&row.path)?;
    let (_, code) = pinned_code(&original, &row.sha256)?;
    convert(
        blob,
        &Native {
            vertex,
            paths: Vec::new(),
            sha256: crate::content_pack::sha256_hex(blob),
        },
        code,
    )
}

pub fn build(
    donor: &mut ResourceDb,
    stock: &mut ResourceDb,
) -> Result<BTreeMap<String, (String, Vec<u8>)>> {
    let catalog: Catalog = serde_json::from_str(include_str!("geometry_programs.json"))?;
    ensure!(
        catalog.original.len() == 7 && catalog.native.len() == 259,
        "painted alpha family coverage"
    );
    let mut originals = BTreeMap::new();
    for row in catalog.original {
        PaintedInput::from_vertex(row.vertex)?;
        let blob = donor.read(&row.path)?;
        let (_, code) = pinned_code(&blob, &row.sha256)?;
        ensure!(
            originals.insert(row.vertex, code.to_vec()).is_none(),
            "duplicate original vertex layout"
        );
    }
    let expected: BTreeSet<_> = stock
        .paths()
        .filter(|path| {
            if !path.starts_with("shaders/sm_4_0/gbuffer_pass.ps/") {
                return false;
            }
            let parts: Vec<_> = path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .split('_')
                .filter(|s| !s.is_empty())
                .collect();
            parts.len() == 11
                && parts[1].ends_with('1')
                && parts[7] == "00000"
                && matches!(parts[4], "0" | "1" | "2" | "10" | "12" | "13" | "14")
        })
        .map(str::to_owned)
        .collect();
    let mut output = BTreeMap::new();
    for row in catalog.native {
        let original = originals
            .get(&row.vertex)
            .context("missing original vertex layout")?;
        for path in &row.paths {
            let blob = stock.read(path)?;
            let after = convert(&blob, &row, original)?;
            ensure!(
                output
                    .insert(path.clone(), (row.sha256.clone(), after))
                    .is_none(),
                "duplicate painted-alpha path"
            );
        }
    }
    ensure!(
        expected.len() == 854
            && expected.iter().all(|p| output.contains_key(p))
            && output.len() == 1364,
        "ordinary stock family is incomplete"
    );
    Ok(output)
}
