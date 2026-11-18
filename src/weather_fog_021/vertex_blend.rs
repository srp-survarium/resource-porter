//! Original linear vertex-alpha blending, selected explicitly by a ported material.
//! The native height/mask formula and every other instruction remain in place.
use super::{assemble, bytes, chunks, instructions, pinned_code, words};
use anyhow::{Context, Result, ensure};
mod linear_set;
pub use linear_set::{LinearProgram, LinearPrograms};
mod masked;
pub use masked::{MASKED_SOURCE, MaskedFactor, MaskedProgram, MaskedPrograms};
pub mod consider_off;
pub mod normal_off;
pub mod normal_on;

/// Wire encoding in the otherwise nonnegative native blend-strength parameter.
/// Only an authored port material opts in; this is not an environment/global switch.
pub const ORIGINAL_LINEAR: f32 = -21021.0;
pub const SOURCE: &str = "shaders/sm_4_0/gbuffer_pass.ps/_3___1000000000__00____2_____0__1_0100____00000____000__________0__";
pub const PATHS: [&str; 4] = [
    "shaders/sm_4_0/gbuffer_pass.ps/_0___1100_00001_00____2_____0__0_01111____00001____000__________0__",
    "shaders/sm_4_0/gbuffer_pass.ps/_0___1100_00001_00____2_____0__2_11111____00001____000__________0__",
    "shaders/sm_4_0/gbuffer_pass.ps/_3___1100_00001_00____2_____0__0_01111____00001____000__________0__",
    "shaders/sm_4_0/gbuffer_pass.ps/_3___1100_00001_00____2_____0__2_11111____00001____000__________0__",
];
const SOURCE_HASH: &str = "e10df548669636b88eb841e62c681574d90ff3b5d71bfbd4ba23f0077d2b6bef";
const TARGET_HASH: &str = "477f2808ad02b8596a351ed0fb2bb8f3247de3334a7df9c8c667249382f5b793";

fn verify_original_linear(source: &[u8]) -> Result<()> {
    let (_, original) = pinned_code(source, SOURCE_HASH)?;
    let original_chunks = chunks(original)?;
    let original_ops = instructions(&words(
        &original_chunks
            .iter()
            .find(|(tag, _)| tag == b"SHDR")
            .context("original SHDR")?
            .1,
    )?)?;
    // Original diffuse = base + (1 - vertex_alpha) * (second - base).
    ensure!(
        original_ops.contains(&vec![
            0x08000000,
            0x00100012,
            0,
            0x8010100a,
            0x41,
            9,
            0x00004001,
            1.0f32.to_bits()
        ]),
        "original linear vertex-alpha formula changed"
    );
    Ok(())
}

pub fn build_linear(source: &[u8], target: &[u8]) -> Result<Vec<u8>> {
    verify_original_linear(source)?;
    let (offset, native) = pinned_code(target, TARGET_HASH)?;
    let mut entries = chunks(native)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("native SHDR")?;
    let old_words = words(&entries[index].1)?;
    let mut ops = instructions(&old_words)?;
    let temps = ops
        .iter()
        .position(|op| op == &[0x02000068, 7])
        .context("native temporaries changed")?;
    ops[temps][1] = 8;
    // Native weight is r0.w, consumed by both diffuse and normal interpolation.
    // Keep the entire native calculation, then override only an opted-in material.
    let cutoff = vec![0x08002038, 0x00100082, 0, 0x0010003a, 0, 0x0020801a, 0, 14];
    ensure!(
        ops.iter().filter(|op| **op == cutoff).count() == 1,
        "ambiguous native blend weight"
    );
    let at = ops
        .iter()
        .position(|op| *op == cutoff)
        .context("native weight")?
        + 1;
    ensure!(
        ops[at]
            == [
                0x0b000032, 0x00100072, 3, 0x00208246, 1, 0, 0x00100246, 3, 0x80100246, 0x41, 5
            ],
        "native diffuse blend boundary changed"
    );
    let guard = vec![
        vec![
            0x08000018,
            0x00100012,
            7,
            0x0020801a,
            0,
            14,
            0x00004001,
            ORIGINAL_LINEAR.to_bits(),
        ],
        vec![0x0304001f, 0x0010000a, 7],
        vec![0x05000036, 0x00100082, 0, 0x0010100a, 9],
        vec![0x01000015],
    ];
    ops.splice(at..at, guard);
    let mut program = old_words[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}
