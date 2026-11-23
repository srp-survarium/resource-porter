//! 0.69d's cut of an alpha-tested vertex blend by its second layer's alpha.
//!
//! 0.26g's alpha-tested blend programs (alpha_test_consider_vertex_blend on)
//! test `1 + W (base.a - 1)`: the second layer counts as opaque. 0.69d's
//! `g_stage_default.ps` (technique 1) tests `lerp(B', A', W)` with
//! `A' = 1` when `consider_albedo_opaque`, else the base layer's alpha source,
//! and `B' = 1` when `consider_blended_albedo_opaque`, else the second layer's
//! alpha, so a material that does not consider its second layer opaque is cut
//! through where that layer shows:
//!
//! * no tiled mask, base opaque (permutation `1001110100110001010030000`):
//!   `W = w` (the raw vertex alpha) and `alpha = B.a + w (1 - B.a)`;
//! * tiled mask, neither opaque (`1001110001110011010030000`):
//!   `W = sat(pow(w, (1 - m)^2 falloff) strength)` and
//!   `alpha = B.a + W (m - B.a)`, `m` the tiled mask's red (0.69d reads the
//!   mask there, not the base albedo's alpha).
//!
//! The ported material selects one of them by its (integer) blend strength,
//! as `ORIGINAL_LINEAR` and the masked factor do, in a range disjoint from
//! both and from every native strength: `CUT_LINEAR` exactly, or
//! `-(2^25 + strength * 2^17)` for the masked form with its own strength.
//! The program keeps every native and v021d instruction; two guarded blocks
//! are added around the blend-aware alpha (the weight before it, the alpha
//! after it), with two temporaries of their own.
use super::{assemble, bytes, chunks, instructions, words};
use anyhow::{Context, Result, bail, ensure};

/// The linear form (no tiled mask, base layer opaque).
pub const CUT_LINEAR: f32 = -21069.0;
const MASKED_TOP: f32 = -33_554_432.0; // -2^25
const MASKED_BOTTOM: f32 = -67_108_864.0; // -2^26
const MASKED_SCALE: f32 = 131_072.0; // 2^17

/// The material strength that selects the masked form with `strength`.
pub fn masked_strength(strength: f32) -> Result<i32> {
    ensure!((0.0..256.0).contains(&strength), "blend strength {strength} out of the masked cut's range");
    let encoded = -(33_554_432.0f64 + strength as f64 * MASKED_SCALE as f64);
    ensure!(encoded == encoded.round() && (encoded as f32) as f64 == encoded, "blend strength {strength} is not exact in the masked cut's encoding");
    Ok(encoded as i32)
}

/// The strength the masked form decodes from a material value (as the program does).
pub fn decoded_strength(material: i32) -> f32 {
    material as f32 * (-1.0 / MASKED_SCALE) - 256.0
}

const ONE: u32 = 0x3f80_0000;

fn reg_operand(op: &[u32], at: usize) -> Option<(u32, u32, u32)> {
    // (type, component of a select-1 or the mask, index) of a one-index operand
    let tok = *op.get(at)?;
    if tok & 0x8000_0000 != 0 {
        return None;
    }
    Some(((tok >> 12) & 0xff, (tok >> 2) & 3, *op.get(at + 1)?))
}

/// Tokens of one instruction of fxc's listing syntax (the operands the guards use).
fn asm(line: &str) -> Result<Vec<u32>> {
    let (name, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    let (base, sat) = match name.strip_suffix("_sat") {
        Some(b) => (b, true),
        None => (name, false),
    };
    let (base, nz) = match base.strip_suffix("_nz") {
        Some(b) => (b, true),
        None => (base, false),
    };
    let code = match base {
        "add" => 0x00,
        "and" => 0x01,
        "endif" => 0x15,
        "eq" => 0x18,
        "exp" => 0x19,
        "ge" => 0x1d,
        "if" => 0x1f,
        "log" => 0x2f,
        "lt" => 0x31,
        "mad" => 0x32,
        "mov" => 0x36,
        "mul" => 0x38,
        "or" => 0x3c,
        "sample" => 0x45,
        "dp2" => 0x0f,
        other => bail!("no encoding for {other}"),
    };
    let operands = split(rest);
    let mut body = Vec::new();
    for (k, text) in operands.iter().enumerate() {
        let dest = k == 0 && base != "if";
        body.extend(operand(text, dest && !text.starts_with('t'))?);
    }
    let mut tok = code | if sat { 0x2000 } else { 0 } | if nz { 1 << 18 } else { 0 };
    tok |= ((1 + body.len()) as u32) << 24;
    let mut out = vec![tok];
    out.extend(body);
    Ok(out)
}

fn split(text: &str) -> Vec<String> {
    let (mut out, mut depth, mut cur) = (Vec::new(), 0, String::new());
    for ch in text.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            out.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

fn operand(text: &str, dest: bool) -> Result<Vec<u32>> {
    let mut t = text;
    let mut modifier = 0;
    if let Some(r) = t.strip_prefix('-') {
        modifier |= 1;
        t = r;
    }
    if let Some(r) = t.strip_prefix('|').and_then(|r| r.strip_suffix('|')) {
        modifier |= 2;
        t = r;
    }
    if let Some(values) = t.strip_prefix("l(").and_then(|r| r.strip_suffix(')')) {
        let values: Vec<u32> = values.split(',').map(|v| v.trim().parse::<f32>().map(f32::to_bits)).collect::<Result<_, _>>()?;
        ensure!(modifier == 0 && matches!(values.len(), 1 | 4), "literal {text}");
        let mut out = vec![(4 << 12) | if values.len() == 1 { 1 } else { 2 }];
        out.extend(values);
        return Ok(out);
    }
    let (kind, rest) = if let Some(r) = t.strip_prefix("cb") { (8, r) } else { (match &t[..1] { "r" => 0, "v" => 1, "o" => 2, "s" => 6, "t" => 7, _ => bail!("operand {text}") }, &t[1..]) };
    let (index, comps) = rest.split_once('.').unwrap_or((rest, ""));
    let indices: Vec<u32> = match index.split_once('[') {
        Some((a, b)) => vec![a.parse()?, b.trim_end_matches(']').parse()?],
        None => vec![index.parse()?],
    };
    let comp = |c: char| "xyzw".find(c).map(|i| i as u32).context("component");
    let mut tok = (kind << 12) | ((indices.len() as u32) << 20);
    if kind != 6 {
        tok |= 2;
        if dest {
            let mut mask = 0;
            for c in (if comps.is_empty() { "xyzw" } else { comps }).chars() {
                mask |= 1 << comp(c)?;
            }
            tok |= mask << 4;
        } else if comps.len() == 1 {
            tok |= (2 << 2) | (comp(comps.chars().next().unwrap())? << 4);
        } else {
            let comps = if comps.is_empty() { "xyzw" } else { comps };
            ensure!(comps.len() == 4, "swizzle {text}");
            let mut sw = 0;
            for (k, c) in comps.chars().enumerate() {
                sw |= comp(c)? << (2 * k);
            }
            tok |= (1 << 2) | (sw << 4);
        }
    }
    let mut out = vec![tok];
    if modifier != 0 {
        out[0] |= 0x8000_0000;
        out.push(1 | (modifier << 6));
    }
    out.extend(indices);
    Ok(out)
}

fn temp_count(ops: &[Vec<u32>]) -> Result<usize> {
    ops.iter().position(|op| op[0] & 0x7ff == 0x68).context("no dcl_temps")
}

/// The adapted program, or None when the program has no blend-aware alpha
/// test (not of the family).
pub fn build(program: &[u8]) -> Result<Option<Vec<u8>>> {
    let offset = 4 + u32::from_le_bytes(program.get(..4).context("shader wrapper")?.try_into()?) as usize * 264;
    let code = program.get(offset..).context("shader wrapper")?;
    let mut entries = chunks(code)?;
    let shdr = entries.iter().position(|(tag, _)| tag == b"SHDR").context("SHDR")?;
    let old = words(&entries[shdr].1)?;
    let mut ops = instructions(&old)?;
    // The blend-aware alpha: mad a, W, a - 1, 1, then the alpha_ref compare and the discard.
    let mixes: Vec<usize> = (0..ops.len())
        .filter(|&i| {
            let op = &ops[i];
            op.len() == 9
                && op[0] == 0x0900_0032
                && op[7..] == [0x0000_4001, ONE]
                && ops.get(i + 1).is_some_and(|n| n[0] & 0x7ff == 0x00)
                && ops.get(i + 2).is_some_and(|n| n[0] & 0x7ff == 0x31)
                && ops.get(i + 3).is_some_and(|n| n[0] & 0x7ff == 0x0d)
        })
        .collect();
    if mixes.is_empty() {
        return Ok(None);
    }
    ensure!(mixes.len() == 1, "two blend-aware alphas");
    let mix = mixes[0];
    let op = ops[mix].clone();
    let alpha_comp = ((op[1] >> 4) & 15).trailing_zeros();
    ensure!((op[1] >> 4) & 15 == 1 << alpha_comp && op[1] & 0xffff_f00f == 0x0010_0002, "alpha is not one temporary component");
    let alpha_reg = op[2];
    let (wtype, wmode, wreg) = reg_operand(&op, 3).context("weight operand")?;
    ensure!(wtype == 0 && wmode == 2, "weight is not a temporary component");
    let wcomp = (op[3] >> 4) & 3;
    let c = |i: u32| ["x", "y", "z", "w"][i as usize];
    let w = format!("r{wreg}.{}", c(wcomp));
    let a = format!("r{alpha_reg}.{}", c(alpha_comp));
    // The weight's vertex alpha: log r, |vN.x| (once).
    let inputs: Vec<u32> = ops
        .iter()
        .filter(|op| op[0] & 0x7ff == 0x2f && op.len() == 6 && op[3] & 0x8000_0000 != 0 && (op[3] >> 12) & 0xff == 1 && op[4] == 0x81)
        .map(|op| op[5])
        .collect();
    if inputs.is_empty() {
        // a layout without a vertex colour (its weight reads the UV): not drawn by a ported blend
        return Ok(None);
    }
    ensure!(inputs.iter().all(|i| *i == inputs[0]), "two vertex-alpha inputs");
    let vertex = inputs[0];
    let sample_t = |op: &[u32], slot: u32| op[0] & 0x7ff == 0x45 && op.len() == 9 && (op[5] >> 12) & 0xff == 7 && op[6] == slot;
    // The second layer's UV: mul r, vU.xyxy-ish, cb0[14].zzzz (its tile).
    let uvs: Vec<u32> = ops
        .iter()
        .filter(|op| op[0] & 0x7ff == 0x38 && op.len() == 8 && (op[3] >> 12) & 0xff == 1 && op.ends_with(&[0, 14]) && (op[5] >> 4) & 0xff == 0xaa)
        .map(|op| op[4])
        .collect();
    ensure!(!uvs.is_empty() && uvs.iter().all(|u| *u == uvs[0]), "second layer UV not found");
    let uv = uvs[0];
    if vertex == uv {
        // a layout without a vertex colour (its weight reads the UV): never a ported blend
        return Ok(None);
    }
    let blended = ops.iter().find(|op| sample_t(op, 1)).context("no second-layer sample")?;
    let sampler = blended[8];
    // The tiled mask (when the layout has one): six instructions on one
    // temporary ending in its t2 sample, moved to r9.
    let pattern = [0x38, 0x00, 0x0f, 0x0f, 0x00, 0x38, 0x45];
    let chain: Option<Vec<Vec<u32>>> = (6..ops.len())
        .filter(|&end| sample_t(&ops[end], 2) && (0..7).all(|k| ops[end - 6 + k][0] & 0x7ff == pattern[k]))
        .map(|end| ops[end - 6..=end].to_vec())
        .next();
    let moved: Option<Vec<Vec<u32>>> = chain.map(|chain| {
        let mask_reg = chain[0][2];
        chain
            .iter()
            .map(|op| {
                let mut op = op.clone();
                let mut k = 1;
                while k < op.len() {
                    let tok = op[k];
                    let ext = tok & 0x8000_0000 != 0;
                    let kind = (tok >> 12) & 0xff;
                    let dims = (tok >> 20) & 3;
                    let start = k + 1 + ext as usize;
                    if kind == 4 {
                        k = start + if tok & 3 == 1 { 1 } else { 4 };
                        continue;
                    }
                    if kind == 0 && dims == 1 && op[start] == mask_reg {
                        op[start] = 9;
                    }
                    k = start + dims as usize;
                }
                op
            })
            .collect()
    });
    // Native weight anchor: mul_sat W, W, cb0[14].y before the mix.
    ensure!(
        ops[..mix].iter().any(|o| o[0] & 0x7ff == 0x38 && o[0] & 0x2000 != 0 && o[2] == wreg && o.ends_with(&[0, 14])),
        "native blend strength product not found"
    );
    let lines = |text: &str| -> Result<Vec<Vec<u32>>> { text.lines().filter(|l| !l.trim().is_empty()).map(asm).collect() };
    let mut weight = lines(&format!(
        "eq r8.x, cb0[14].y, l({CUT_LINEAR:.6})
         if_nz r8.x
         mov {w}, v{vertex}.x
         endif"
    ))?;
    if let Some(moved) = &moved {
        weight.extend(lines(&format!(
            "ge r8.x, l({MASKED_TOP:.6}), cb0[14].y
             lt r8.y, l({MASKED_BOTTOM:.6}), cb0[14].y
             and r8.x, r8.x, r8.y
             if_nz r8.x"
        ))?);
        weight.extend(moved.iter().cloned());
        weight.extend(lines(&format!(
            "add r9.x, -r9.x, l(1.000000)
             mul r9.x, r9.x, r9.x
             mul r9.x, r9.x, cb0[14].x
             log r9.y, |v{vertex}.x|
             mul r9.x, r9.y, r9.x
             exp r9.x, r9.x
             mad r9.y, cb0[14].y, l({}), l(-256.000000)
             mul_sat {w}, r9.x, r9.y
             endif",
            -1.0 / MASKED_SCALE
        ))?);
    }
    let mut cut = match &moved {
        Some(_) => lines(&format!(
            "eq r8.x, cb0[14].y, l({CUT_LINEAR:.6})
             ge r8.y, l({MASKED_TOP:.6}), cb0[14].y
             lt r8.z, l({MASKED_BOTTOM:.6}), cb0[14].y
             and r8.y, r8.y, r8.z
             or r8.x, r8.x, r8.y"
        ))?,
        None => lines(&format!("eq r8.x, cb0[14].y, l({CUT_LINEAR:.6})"))?,
    };
    cut.extend(lines(&format!(
        "if_nz r8.x
         mul r8.zw, v{uv}.xxxy, cb0[14].zzzz
         sample r8.w, r8.zwzz, t1.wwww, s{sampler}
         mov r8.z, l(1.000000)"
    ))?);
    if let Some(moved) = &moved {
        cut.extend(lines("if_nz r8.y")?);
        cut.extend(moved.iter().cloned());
        cut.extend(lines("mov r8.z, r9.x\n endif")?);
    }
    cut.extend(lines(&format!(
        "add r8.z, r8.z, -r8.w
         mad {a}, {w}, r8.z, r8.w
         endif"
    ))?);
    // r8 and r9 are the guards' own.
    let temps = temp_count(&ops)?;
    ensure!(ops[temps].len() == 2 && ops[temps][1] <= 8, "temporaries already above r7");
    ops[temps][1] = 10;
    ops.splice(mix + 1..mix + 1, cut);
    ops.splice(mix..mix, weight);
    let mut updated = old[..2].to_vec();
    updated.extend(ops.iter().flatten());
    updated[1] = u32::try_from(updated.len())?;
    instructions(&updated)?;
    entries[shdr].1 = bytes(&updated);
    let mut output = program[..offset].to_vec();
    output.extend(assemble(code, &entries)?);
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guards_assemble_as_fxc_lists_them() {
        // fxc's own tokens for these lines (v021d's guard in the same programs).
        assert_eq!(asm("eq r7.x, cb0[14].y, l(-21021.000000)").unwrap(), vec![0x08000018, 0x00100012, 7, 0x0020801a, 0, 14, 0x00004001, (-21021.0f32).to_bits()]);
        assert_eq!(asm("if_nz r7.x").unwrap(), vec![0x0304001f, 0x0010000a, 7]);
        assert_eq!(asm("mov r0.w, v9.x").unwrap(), vec![0x05000036, 0x00100082, 0, 0x0010100a, 9]);
        assert_eq!(asm("endif").unwrap(), vec![0x01000015]);
    }

    #[test]
    fn the_masked_strength_round_trips_and_stays_apart() {
        for s in [1.0, 3.0, 4.0, 12.0, 0.5] {
            let m = masked_strength(s).unwrap();
            assert_eq!(decoded_strength(m), s);
            assert!((m as f32) < MASKED_TOP && (m as f32) > MASKED_BOTTOM);
        }
        // apart from v021d's linear marker and masked factors [-2^24, -2^23]
        assert!(CUT_LINEAR != -21021.0 && MASKED_TOP < -16_777_216.0);
    }
}
