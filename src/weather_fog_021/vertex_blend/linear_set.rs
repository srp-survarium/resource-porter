//! Pinned native layouts for the original unmasked vertex-alpha equation.
use super::{ORIGINAL_LINEAR, assemble, bytes, chunks, instructions, pinned_code, words};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum AlphaInput {
    V7,
    V9,
}

impl AlphaInput {
    fn register(self) -> u32 {
        match self {
            Self::V7 => 7,
            Self::V9 => 9,
        }
    }
}

#[derive(Clone, Copy)]
enum Component {
    X,
    Y,
    W,
}

impl Component {
    fn index(self) -> u32 {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::W => 3,
        }
    }

    fn destination(self) -> u32 {
        0x00100002 | (1 << (4 + self.index()))
    }

    fn scalar_source(self) -> u32 {
        0x0010000a | (self.index() << 4)
    }
}

#[derive(Clone, Copy)]
struct Temporary(u32);

/// A program can only be obtained from a source-verified `LinearPrograms`.
pub struct LinearProgram {
    path: &'static str,
    fingerprint: &'static str,
    alpha: AlphaInput,
    weight_register: Temporary,
    weight_component: Component,
    scratch: Temporary,
}

/// Verifies the original equation once, then owns all parsed native contracts.
pub struct LinearPrograms {
    programs: Vec<LinearProgram>,
}

impl LinearPrograms {
    pub fn new(original: &[u8]) -> Result<Self> {
        super::verify_original_linear(original)?;
        let mut paths = BTreeSet::new();
        let mut programs = Vec::new();
        for line in include_str!("linear-programs.tsv")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let fields: Vec<_> = line.split('\t').collect();
            ensure!(fields.len() == 6, "invalid native linear-blend contract");
            let path = fields[0];
            ensure!(
                path.starts_with("shaders/sm_4_0/gbuffer_pass.ps/"),
                "unexpected stage"
            );
            ensure!(paths.insert(path), "duplicate native shader path: {path}");
            let fingerprint = fields[1];
            ensure!(
                fingerprint.len() == 64 && fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid native hash"
            );
            let alpha = match fields[2] {
                "7" => AlphaInput::V7,
                "9" => AlphaInput::V9,
                _ => anyhow::bail!("unsupported native vertex-alpha input"),
            };
            let weight_register = Temporary(fields[3].parse()?);
            let weight_component = match fields[4] {
                "x" => Component::X,
                "y" => Component::Y,
                "w" => Component::W,
                _ => anyhow::bail!("unsupported native blend-weight component"),
            };
            let scratch = Temporary(fields[5].parse()?);
            ensure!(
                scratch.0 > weight_register.0 && scratch.0 < 4096,
                "invalid temporary-register range"
            );
            programs.push(LinearProgram {
                path,
                fingerprint,
                alpha,
                weight_register,
                weight_component,
                scratch,
            });
        }
        ensure!(!programs.is_empty(), "empty native shader set");
        Ok(Self { programs })
    }

    pub fn programs(&self) -> &[LinearProgram] {
        &self.programs
    }
}

impl LinearProgram {
    pub fn path(&self) -> &'static str {
        self.path
    }

    pub fn native_sha256(&self) -> &'static str {
        self.fingerprint
    }

    pub fn build(&self, target: &[u8]) -> Result<Vec<u8>> {
        let (offset, native) = pinned_code(target, self.fingerprint)?;
        let mut entries = chunks(native)?;
        let index = entries
            .iter()
            .position(|(tag, _)| tag == b"SHDR")
            .context("native SHDR")?;
        let old_words = words(&entries[index].1)?;
        let mut ops = instructions(&old_words)?;
        let temps: Vec<_> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| op.as_slice() == [0x02000068, self.scratch.0])
            .map(|(i, _)| i)
            .collect();
        ensure!(temps.len() == 1, "native temporary declaration changed");
        let cutoff = vec![
            0x08002038,
            self.weight_component.destination(),
            self.weight_register.0,
            self.weight_component.scalar_source(),
            self.weight_register.0,
            0x0020801a,
            0,
            14,
        ];
        let sites: Vec<_> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| **op == cutoff)
            .map(|(i, _)| i)
            .collect();
        ensure!(sites.len() == 1, "native blend-weight site changed");
        let site = sites[0];
        let alpha_reads = ops[site.saturating_sub(5)..site]
            .iter()
            .filter(|op| {
                op.len() == 6
                    && op[0] == 0x0600002f
                    && op[3..] == [0x8010100a, 0x81, self.alpha.register()]
            })
            .count();
        ensure!(alpha_reads == 1, "native vertex-alpha operand changed");
        ops[temps[0]][1] += 1;
        // The complete native calculation stays in place. Only the explicit
        // original-linear marker replaces its final weight before any consumer.
        let guard = [
            vec![
                0x08000018,
                0x00100012,
                self.scratch.0,
                0x0020801a,
                0,
                14,
                0x00004001,
                ORIGINAL_LINEAR.to_bits(),
            ],
            vec![0x0304001f, 0x0010000a, self.scratch.0],
            vec![
                0x05000036,
                self.weight_component.destination(),
                self.weight_register.0,
                0x0010100a,
                self.alpha.register(),
            ],
            vec![0x01000015],
        ];
        ops.splice(site + 1..site + 1, guard);
        let mut program = old_words[..2].to_vec();
        program.extend(ops.iter().flatten());
        program[1] = program.len() as u32;
        instructions(&program)?;
        entries[index].1 = bytes(&program);
        let mut result = target[..offset].to_vec();
        result.extend(assemble(native, &entries)?);
        Ok(result)
    }
}
