//! Original painted dirt cutouts and ordinary shadow thresholds.
//! `geometry`, `depth`, and `profiles` implement the full active family. This
//! file also retains the isolated first Vostok probe so its causal capture is
//! reproducible; its diagnostic profiles must not activate production materials.
use super::{assemble, bytes, chunks, instructions, pinned_code, words};
use anyhow::{Context, Result, ensure};

pub mod depth;
pub mod geometry;
pub mod parallax;
pub mod profiles;

pub const SOURCE: &str = "shaders/sm_4_0/gbuffer_pass.ps/_3___1001100001__00____2_____0__0_0000____00000____000__________0__";
pub const TARGET: &str = "shaders/sm_4_0/gbuffer_pass.ps/_3___1001_00001_00____2_____0__0_00000____00000____000__________0__";
pub const NATIVE_HASH: &str = "afca0c61fa9e91c15231da2816ae5023d7fcaef0a111d49917fa7b6e965e39fe";

/// Initial diagnostic profiles, including one alpha-disabled original material.
/// Production uses `profiles::AuthoredProfile`, which also preserves each
/// ordinary shadow threshold and excludes the inactive diagnostic selector.
#[derive(Clone, Copy)]
pub enum Profile {
    Moss,
    Stucco,
    Dirt,
    Roots,
    MossMultiply,
}

impl Profile {
    pub fn from_authored(dark: f32, light: f32, multiply: bool) -> Result<Self> {
        [
            Self::Moss,
            Self::Stucco,
            Self::Dirt,
            Self::Roots,
            Self::MossMultiply,
        ]
        .into_iter()
        .find(|p| p.parameters() == (dark, light, multiply))
        .context("unrepresented original vertex-alpha profile")
    }

    pub fn alpha_ref(self) -> f32 {
        match self {
            Self::Moss => -21210.0,
            Self::Stucco => -21211.0,
            Self::Dirt => -21212.0,
            Self::Roots => -21213.0,
            Self::MossMultiply => -21214.0,
        }
    }

    fn parameters(self) -> (f32, f32, bool) {
        match self {
            Self::Moss => (0.1, 0.5, false),
            Self::Stucco => (0.5, 1.0, false),
            Self::Dirt => (1.0, 1.0, false),
            Self::Roots => (1.0, 1.0, true),
            Self::MossMultiply => (1.5, 1.0, true),
        }
    }
}

fn dst(component: u32) -> [u32; 2] {
    [0x00100002 | (1 << (4 + component)), 2]
}
fn src(component: u32) -> [u32; 2] {
    [0x0010000a | (component << 4), 2]
}
fn literal(value: f32) -> [u32; 2] {
    [0x00004001, value.to_bits()]
}
fn op(code: u32, args: &[&[u32]]) -> Vec<u32> {
    let mut out = vec![code | (((1 + args.iter().map(|a| a.len()).sum::<usize>()) as u32) << 24)];
    out.extend(args.iter().flat_map(|a| a.iter()).copied());
    out
}

/// Restore the original cutoff in the observed stock class-2 alpha program.
/// All native arithmetic remains in its original order on the default branch.
pub fn build_observed(source: &[u8], target: &[u8]) -> Result<Vec<u8>> {
    let (_, original) = pinned_code(
        source,
        "34fd69c289b490e2db71ff8d31a4f3a037ca85ef67f4aca70f21cd9f1f7f4b45",
    )?;
    let (offset, native) = pinned_code(target, NATIVE_HASH)?;
    let original_chunks = chunks(original)?;
    let mut entries = chunks(native)?;
    let index = entries
        .iter()
        .position(|(tag, _)| tag == b"SHDR")
        .context("SHDR")?;
    let old = words(&entries[index].1)?;
    let mut ops = instructions(&old)?;
    let temps = ops
        .iter_mut()
        .find(|o| o[0] == 0x02000068)
        .context("temps")?;
    ensure!(temps[1] == 2, "native scratch registers changed");
    temps[1] = 3;
    // The pinned original signature differs by precisely the painted alpha
    // input v9.x. The observed native vertex shader already exports that value.
    let original_ops = instructions(&words(
        &original_chunks
            .iter()
            .find(|(t, _)| t == b"SHDR")
            .context("original SHDR")?
            .1,
    )?)?;
    let declaration = original_ops
        .iter()
        .find(|o| o.len() == 3 && o[1..] == [0x00101012, 9])
        .context("original alpha declaration")?;
    let declaration_at = ops
        .iter()
        .position(|o| o[0] == 0x02000068)
        .context("temps")?;
    ops.insert(declaration_at, declaration.clone());
    let signature = original_chunks
        .iter()
        .find(|(t, _)| t == b"ISGN")
        .context("original ISGN")?;
    entries
        .iter_mut()
        .find(|(t, _)| t == b"ISGN")
        .context("native ISGN")?
        .1 = signature.1.clone();
    let cutoff = vec![
        0x09000000, 0x00100012, 0, 0x0010000a, 0, 0x8020802a, 0x41, 0, 15,
    ];
    let at = ops
        .iter()
        .position(|o| *o == cutoff)
        .context("native alpha cutoff")?;
    let alpha_ref = [0x0020802a, 0, 15];
    let alpha = [0x0010000a, 0];
    let output = [0x00100012, 0];
    let mut added = Vec::new();
    for profile in [
        Profile::Moss,
        Profile::Stucco,
        Profile::Dirt,
        Profile::Roots,
        Profile::MossMultiply,
    ] {
        let (dark, light, multiply) = profile.parameters();
        added.extend([
            op(24, &[&dst(0), &alpha_ref, &literal(profile.alpha_ref())]),
            op(0x4001f, &[&src(0)]),
            op(47, &[&dst(0), &[0x0010100a, 9]]),
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
            op(50, &[&output, &src(0), &src(1), &literal(-0.4)]),
            op(18, &[]),
        ]);
    }
    added.push(cutoff);
    added.extend((0..5).map(|_| op(21, &[])));
    ops.splice(at..=at, added);
    let mut program = old[..2].to_vec();
    program.extend(ops.iter().flatten());
    program[1] = program.len() as u32;
    instructions(&program)?;
    entries[index].1 = bytes(&program);
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}
