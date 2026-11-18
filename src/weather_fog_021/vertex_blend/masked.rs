//! Original base-alpha masked blending, with the native path left unchanged.
use super::{LinearProgram, LinearPrograms, ORIGINAL_LINEAR};
use super::{assemble, bytes, chunks, instructions, pinned_code, words};
use anyhow::{Context, Result, ensure};

// Original normal-blended, base-alpha-masked program; all aliases share this blob.
pub const MASKED_SOURCE: &str = "shaders/sm_4_0/gbuffer_pass.ps/_3___1100000001__00____2_____0__0_0111____00000____000__________0__";
const SOURCE_HASH: &str = "fcf8789de938e808ea75c899a220f88a725679601e75dbe1b107fda796c8ce9e";
const FACTOR_SCALE: f32 = 8_388_608.0;

/// The original authored factor is carried losslessly by an otherwise unused
/// negative integer strength. This interval is disjoint from original-linear
/// mode and every nonnegative native material strength.
pub struct MaskedFactor(i32);

impl MaskedFactor {
    pub fn from_original(factor: f32) -> Result<Self> {
        ensure!(
            (1.0..=2.0).contains(&factor),
            "unsupported original masked factor"
        );
        let integer = (factor * FACTOR_SCALE) as i32;
        ensure!(
            (integer as f32 / FACTOR_SCALE).to_bits() == factor.to_bits(),
            "masked factor is not exactly representable"
        );
        Ok(Self(-integer))
    }

    pub fn material_strength(&self) -> i32 {
        self.0
    }
}

/// No masked program is exposed until both original equations are verified.
pub struct MaskedPrograms {
    linear: LinearPrograms,
}

pub struct MaskedProgram<'a> {
    linear: &'a LinearProgram,
}

/// Which texture's coefficient the caller's existing interpolation stores.
pub(super) enum WeightOrientation {
    Base,
    Second,
}

/// Parsed binding contract of the caller's actual base sample/material buffer.
pub(super) struct BaseSample {
    pub coordinates: [u32; 2],
    pub texture: u32,
    pub sampler: u32,
    pub material_buffer: u32,
}

pub(super) fn masked_override(
    alpha: u32,
    sample: BaseSample,
    scratch: u32,
    weight: [u32; 2],
    orientation: WeightOrientation,
) -> Vec<Vec<u32>> {
    let dx = destination(scratch, 0);
    let dy = destination(scratch, 1);
    let dz = destination(scratch, 2);
    let dw = destination(scratch, 3);
    let x = scalar(scratch, 0);
    let y = scalar(scratch, 1);
    let z = scalar(scratch, 2);
    let w = scalar(scratch, 3);
    let strength = [0x0020801a, sample.material_buffer, 14];
    let falloff = [0x0020800a, sample.material_buffer, 14];
    vec![
        operation(29, &[&dx, &strength, &literal(-2.0 * FACTOR_SCALE)]),
        operation(29, &[&dy, &literal(-FACTOR_SCALE), &strength]),
        operation(1, &[&dx, &x, &y]),
        operation(0x4001f, &[&x]),
        // An extra alpha-only sample stays inside the uniform material
        // branch. Native scheduling often samples base after the weight;
        // duplicating it avoids moving native instructions or assuming a
        // live alpha register. Coordinates and sampler match native base.
        operation(
            69,
            &[
                &dw,
                &sample.coordinates,
                &[0x00107ff6, sample.texture],
                &[0x00106000, sample.sampler],
            ],
        ),
        operation(0, &[&dx, &[0x8010100a, 0x41, alpha], &literal(1.0)]),
        operation(47, &[&dy, &x]),
        operation(56, &[&dy, &y, &falloff]),
        operation(25, &[&dy, &y]),
        operation(0, &[&dz, &negative_scalar(scratch, 3), &literal(1.0)]),
        operation(50, &[&dx, &x, &z, &w]),
        operation(56, &[&dx, &x, &y]),
        operation(56, &[&dy, &strength, &literal(-1.0 / FACTOR_SCALE)]),
        operation(0, &[&dz, &y, &literal(1.0)]),
        operation(0x2038, &[&dx, &x, &z]),
        operation(47, &[&dx, &x]),
        operation(56, &[&dx, &x, &y]),
        operation(25, &[&dx, &x]),
        match orientation {
            WeightOrientation::Base => {
                operation(0, &[&weight, &negative_scalar(scratch, 0), &literal(1.0)])
            }
            WeightOrientation::Second => operation(54, &[&weight, &x]),
        },
        operation(21, &[]),
    ]
}

impl MaskedPrograms {
    pub fn new(original_linear: &[u8], original_masked: &[u8]) -> Result<Self> {
        // The pinned program's contiguous original block calculates:
        // q=1-v; blend=pow(saturate((q*(1-a)+a)*pow(q,h)*(f+1)),f).
        // It consumes base texture alpha, not a separate tiled-mask texture.
        pinned_code(original_masked, SOURCE_HASH)?;
        Ok(Self {
            linear: LinearPrograms::new(original_linear)?,
        })
    }

    pub fn programs(&self) -> impl Iterator<Item = MaskedProgram<'_>> {
        self.linear
            .programs()
            .iter()
            .map(|linear| MaskedProgram { linear })
    }
}

fn destination(register: u32, component: u32) -> [u32; 2] {
    [0x00100002 | (1 << (4 + component)), register]
}

fn scalar(register: u32, component: u32) -> [u32; 2] {
    [0x0010000a | (component << 4), register]
}

fn negative_scalar(register: u32, component: u32) -> [u32; 3] {
    [0x8010000a | (component << 4), 0x41, register]
}

fn literal(value: f32) -> [u32; 2] {
    [0x00004001, value.to_bits()]
}

fn operation(opcode: u32, operands: &[&[u32]]) -> Vec<u32> {
    let length = 1 + operands.iter().map(|part| part.len()).sum::<usize>();
    let mut result = vec![opcode | ((length as u32) << 24)];
    result.extend(operands.iter().flat_map(|part| part.iter()).copied());
    result
}

impl MaskedProgram<'_> {
    pub fn path(&self) -> &'static str {
        self.linear.path()
    }

    pub fn native_sha256(&self) -> &'static str {
        self.linear.native_sha256()
    }

    pub fn build(&self, target: &[u8]) -> Result<Vec<u8>> {
        // Start with the already verified linear adapter. Both modes share its
        // one scratch register, whose lifetime ends at the compatibility block.
        let linear = self.linear.build(target)?;
        let offset = 4 + u32::from_le_bytes(linear[..4].try_into()?) as usize * 264;
        let native = &linear[offset..];
        let mut entries = chunks(native)?;
        let index = entries
            .iter()
            .position(|(tag, _)| tag == b"SHDR")
            .context("SHDR")?;
        let old_words = words(&entries[index].1)?;
        let mut ops = instructions(&old_words)?;
        let branch = ops
            .iter()
            .position(|op| {
                op.len() == 8
                    && op[0] == 0x08000018
                    && op[6..] == [0x00004001, ORIGINAL_LINEAR.to_bits()]
            })
            .context("linear guard")?;
        let scratch = ops[branch][2];
        let assign = &ops[branch + 2];
        ensure!(
            assign.len() == 5
                && assign[0] == 0x05000036
                && assign[3] == 0x0010100a
                && ops[branch + 3] == [0x01000015],
            "linear boundary changed"
        );
        let weight = [assign[1], assign[2]];
        let alpha = assign[4];
        let samples: Vec<_> = ops
            .iter()
            .filter(|op| {
                op.len() == 9
                    && op[0] == 0x09000045
                    && op[3..5] == [0x00101046, 7]
                    && op[6] == 0
                    && matches!(op[5], 0x00107e46 | 0x00107936)
                    && op[7] == 0x00106000
                    && op[8] <= 1
            })
            .collect();
        ensure!(samples.len() == 1, "native base-sample contract changed");
        let sampler = samples[0][8];
        let added = masked_override(
            alpha,
            BaseSample {
                coordinates: [0x00101046, 7],
                texture: 0,
                sampler,
                material_buffer: 0,
            },
            scratch,
            weight,
            WeightOrientation::Base,
        );
        ops.splice(branch + 4..branch + 4, added);
        let mut program = old_words[..2].to_vec();
        program.extend(ops.iter().flatten());
        program[1] = program.len() as u32;
        instructions(&program)?;
        entries[index].1 = bytes(&program);
        let mut output = linear[..offset].to_vec();
        output.extend(assemble(native, &entries)?);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_factor_survives_integer_config_and_gpu_conversion() -> Result<()> {
        // Include both authored factors and adjacent floats: equality of the
        // decoded f32 bits is the wire contract, not an approximate tolerance.
        for factor in [
            1.0_f32,
            f32::from_bits(1.0_f32.to_bits() + 1),
            1.85,
            f32::from_bits(2.0_f32.to_bits() - 1),
            2.0,
        ] {
            let strength = MaskedFactor::from_original(factor)?.material_strength();
            assert!((-16_777_216..=-8_388_608).contains(&strength));
            assert_eq!(
                (-(strength as f32) / FACTOR_SCALE).to_bits(),
                factor.to_bits()
            );
        }
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, 0.99, 2.01] {
            assert!(MaskedFactor::from_original(invalid).is_err());
        }
        Ok(())
    }
}
