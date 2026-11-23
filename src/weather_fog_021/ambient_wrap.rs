// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's facing term on the ambient lights of a 0.69d scene.
//!
//! 0.26g's `dot_normal` ambient lights (`ambient_light.ps`, the 8 variants with the facing
//! digit 1) draw `max(dot(N, L), 0)` times the falloff (a sphere) or fold a facing of the
//! box's faces into it (a box); 0.69d draws `saturate(saturate(x/2 + 1/2) (1 + cutoff) -
//! cutoff)^power` times the falloff, `x` the cosine to the light's centre
//! (`crate::ambient_facing`). A light converted from 0.69d carries the cutoff, the power
//! and 0.26g's distance power in its `attenuation_power` (`ambient_facing::encode`).
//!
//! Each variant runs, for a scene whose `cb1[18].w` (`atmosphere_parameters3.w`, the
//! environment's `mie_fog_base_height`) is one of the given markers and a light whose power
//! is so encoded, a copy of its native program that draws 0.69d's term:
//! - a sphere: the native program with `max(x, 0)` replaced by 0.69d's term, the decoded
//!   distance power for `cb0[12].w`, and 0.69d's `max(|d|, 0.01)` before the division
//!   that makes the direction;
//! - a box: the native normal and position, then the box's falloff as the variant without
//!   `dot_normal` draws it (`1 - saturate(1 - prod(1 - smoothstep(t_i)))`), times 0.69d's
//!   term, `x` taken to the box's centre: `view_to_local`'s rows are the box's axes over its
//!   half extents (exe 0x181C6F: the inverse of the light's scaled world matrix times the
//!   view's), so `P - C = sum_i p_i a_i / |a_i|^2` of the local position `p`.
//!
//! Every other scene and light runs the native instructions, unchanged, in the other branch.
use super::{assemble, bytes, chunks, instructions, pinned_code, u32_at, words};
use crate::ambient_facing::{BASE, CUTOFF_OFFSET, CUTOFF_SHIFT, CUTOFF_STEP, FACING_POWER_SHIFT, FACING_POWER_STEP, PAYLOAD_LIMIT, POWER_BITS, POWER_STEP};
use anyhow::{Context, Result, bail, ensure};

pub const AMBIENT_LIGHT_PREFIX: &str = "shaders/sm_4_0/ambient_light.ps/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sphere,
    Box,
}

/// The 8 `dot_normal` variants of stock 0.26g (key digits: affect_specular, facing 1,
/// geometry, mode), their shape and sha256. Modes 0 (multiplying, two targets) and 1
/// (adding, one); affect_specular only changes the specular mask's constant.
pub const DOT_NORMAL_VARIANTS: [(&str, Shape, &str); 8] = [
    ("____________0________1____0____0__0________________________________", Shape::Sphere, "5f44d9a20c1027b0781128c7eec3b88d062253218032daf039a2597d30775754"),
    ("____________0________1____0____0__1________________________________", Shape::Sphere, "0b5660d3292b36f9265afc0a95d8e7a9491ef8f40b4161f513e1b649181524f4"),
    ("____________0________1____0____1__0________________________________", Shape::Box, "4cb73f0385fa2f291d13b97b2c9ee37671ded9b77a695cf7d52e74e8c9cb39ac"),
    ("____________0________1____0____1__1________________________________", Shape::Box, "1989d60f3dc33459886b28f56785d9578ab2232698041aa87b64499fdf0c7722"),
    ("____________1________1____0____0__0________________________________", Shape::Sphere, "ed5209d96b2a7b810d1e4a64f7792a96a6c04ea7f00aa4ea80198d2e1157cb81"),
    ("____________1________1____0____0__1________________________________", Shape::Sphere, "0b5660d3292b36f9265afc0a95d8e7a9491ef8f40b4161f513e1b649181524f4"),
    ("____________1________1____0____1__0________________________________", Shape::Box, "8fd88d6e7dbd92b5cdf35070d9d2053fc98d1a5e357318231207d1435859d55b"),
    ("____________1________1____0____1__1________________________________", Shape::Box, "1989d60f3dc33459886b28f56785d9578ab2232698041aa87b64499fdf0c7722"),
];

/// Tokens of the few SM 4.0 instructions the branch writes.
mod asm {
    pub const ADD: u32 = 0x00;
    pub const AND: u32 = 0x01;
    pub const DIV: u32 = 0x0e;
    pub const DP3: u32 = 0x10;
    pub const DP4: u32 = 0x11;
    pub const ELSE: u32 = 0x12;
    pub const ENDIF: u32 = 0x15;
    pub const EQ: u32 = 0x18;
    pub const EXP: u32 = 0x19;
    pub const IADD: u32 = 0x1e;
    pub const IF: u32 = 0x1f;
    pub const ITOF: u32 = 0x2b;
    pub const LOG: u32 = 0x2f;
    pub const MAD: u32 = 0x32;
    pub const MIN: u32 = 0x33;
    pub const MAX: u32 = 0x34;
    pub const MOV: u32 = 0x36;
    pub const MUL: u32 = 0x38;
    pub const OR: u32 = 0x3c;
    pub const RET: u32 = 0x3e;
    pub const SQRT: u32 = 0x4b;
    pub const ULT: u32 = 0x4f;
    pub const USHR: u32 = 0x55;
    pub const UTOF: u32 = 0x56;
    pub const DCL_CONSTANT_BUFFER: u32 = 0x59;
    pub const DCL_TEMPS: u32 = 0x68;

    fn comp(c: u8) -> u32 {
        match c {
            b'x' => 0,
            b'y' => 1,
            b'z' => 2,
            b'w' => 3,
            _ => panic!("component {c}"),
        }
    }
    fn mask(m: &str) -> u32 {
        m.bytes().map(|c| 1 << comp(c)).fold(0, |a, b| a | b)
    }
    fn swizzle(s: &str) -> u32 {
        s.bytes().enumerate().map(|(i, c)| comp(c) << (2 * i)).sum()
    }
    /// Operand types and index dimensions: temp 0, output 2, constant buffer 8.
    const TEMP: u32 = 0x0010_0000;
    #[cfg(test)]
    const OUTPUT: u32 = 0x0010_2000;
    const CB: u32 = 0x0020_8000;

    pub fn r(i: u32, m: &str) -> Vec<u32> {
        vec![TEMP | mask(m) << 4 | 2, i]
    }
    #[cfg(test)]
    pub fn o(i: u32, m: &str) -> Vec<u32> {
        vec![OUTPUT | mask(m) << 4 | 2, i]
    }
    /// A temp read with a swizzle (`r0.xyzx`).
    pub fn rs(i: u32, s: &str) -> Vec<u32> {
        vec![TEMP | swizzle(s) << 4 | 0x6, i]
    }
    /// A temp read as one component (`r0.x`).
    pub fn rc(i: u32, c: &str) -> Vec<u32> {
        vec![TEMP | comp(c.as_bytes()[0]) << 4 | 0xa, i]
    }
    pub fn cb(slot: u32, reg: u32, s: &str) -> Vec<u32> {
        vec![CB | swizzle(s) << 4 | 0x6, slot, reg]
    }
    pub fn cbc(slot: u32, reg: u32, c: &str) -> Vec<u32> {
        vec![CB | comp(c.as_bytes()[0]) << 4 | 0xa, slot, reg]
    }
    pub fn l(v: f32) -> Vec<u32> {
        vec![0x0000_4001, v.to_bits()]
    }
    pub fn lu(v: u32) -> Vec<u32> {
        vec![0x0000_4001, v]
    }
    pub fn l4(v: [f32; 4]) -> Vec<u32> {
        let mut out = vec![0x0000_4002];
        out.extend(v.map(f32::to_bits));
        out
    }
    fn modify(mut operand: Vec<u32>, m: u32) -> Vec<u32> {
        operand[0] |= 0x8000_0000;
        operand.insert(1, 1 | m << 6);
        operand
    }
    pub fn neg(operand: Vec<u32>) -> Vec<u32> {
        modify(operand, 1)
    }
    pub fn abs(operand: Vec<u32>) -> Vec<u32> {
        modify(operand, 2)
    }
    fn build(token: u32, operands: &[Vec<u32>]) -> Vec<u32> {
        let mut out = vec![0];
        for operand in operands {
            out.extend(operand);
        }
        out[0] = token | (out.len() as u32) << 24;
        out
    }
    pub fn op(code: u32, operands: &[Vec<u32>]) -> Vec<u32> {
        build(code, operands)
    }
    pub fn op_sat(code: u32, operands: &[Vec<u32>]) -> Vec<u32> {
        build(code | 0x2000, operands)
    }
    /// `if_nz`.
    pub fn if_nz(operand: Vec<u32>) -> Vec<u32> {
        build(IF | 0x4_0000, &[operand])
    }
}
use asm::*;

/// Is `ops[i]` a declaration (or the program's custom data)?
fn declaration(op: &[u32]) -> bool {
    let code = op[0] & 2047;
    code >= 0x58 || code == 0x35
}

/// `rT.x` = the scene is one of `markers` and the light's power carries a facing term.
fn condition(t: u32, markers: &[f32]) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for (i, m) in markers.iter().enumerate() {
        let into = if i == 0 { "x" } else { "y" };
        out.push(op(EQ, &[r(t, into), cbc(1, 18, "w"), l(*m)]));
        if i > 0 {
            out.push(op(OR, &[r(t, "x"), rc(t, "x"), rc(t, "y")]));
        }
    }
    out.push(op(IADD, &[r(t, "y"), cbc(0, 12, "w"), lu(BASE.wrapping_neg())]));
    out.push(op(ULT, &[r(t, "z"), rc(t, "y"), lu(PAYLOAD_LIMIT)]));
    out.push(op(AND, &[r(t, "x"), rc(t, "x"), rc(t, "z")]));
    out
}

/// The payload in `rT.y` into `rT.y` = distance power, `rT.z` = cutoff, `rT.w` = facing
/// power (`ambient_facing::decode`).
fn decode(t: u32) -> Vec<Vec<u32>> {
    vec![
        op(USHR, &[r(t, "z"), rc(t, "y"), lu(CUTOFF_SHIFT)]),
        op(IADD, &[r(t, "z"), rc(t, "z"), lu((-CUTOFF_OFFSET) as u32)]),
        op(ITOF, &[r(t, "z"), rc(t, "z")]),
        op(MUL, &[r(t, "z"), rc(t, "z"), l(CUTOFF_STEP)]),
        op(USHR, &[r(t, "w"), rc(t, "y"), lu(FACING_POWER_SHIFT)]),
        op(AND, &[r(t, "w"), rc(t, "w"), lu(0xff)]),
        op(UTOF, &[r(t, "w"), rc(t, "w")]),
        op(MUL, &[r(t, "w"), rc(t, "w"), l(FACING_POWER_STEP)]),
        op(AND, &[r(t, "y"), rc(t, "y"), lu((1 << POWER_BITS) - 1)]),
        op(UTOF, &[r(t, "y"), rc(t, "y")]),
        op(MUL, &[r(t, "y"), rc(t, "y"), l(POWER_STEP)]),
    ]
}

/// 0.69d's term of the cosine in `reg.c`, in place (its program's six instructions, its
/// constants from `rT`; `rT.x` is free once the branch is taken).
fn facing(t: u32, reg: u32, c: &str) -> Vec<Vec<u32>> {
    vec![
        op_sat(MAD, &[r(reg, c), rc(reg, c), l(0.5), l(0.5)]),
        op(ADD, &[r(t, "x"), rc(t, "z"), l(1.0)]),
        op_sat(MAD, &[r(reg, c), rc(reg, c), rc(t, "x"), neg(rc(t, "z"))]),
        op(LOG, &[r(reg, c), rc(reg, c)]),
        op(MUL, &[r(reg, c), rc(reg, c), rc(t, "w")]),
        op(EXP, &[r(reg, c), rc(reg, c)]),
    ]
}

fn find(body: &[Vec<u32>], wanted: &[u32], what: &str) -> Result<usize> {
    let mut at = body.iter().enumerate().filter(|(_, i)| i.as_slice() == wanted).map(|(n, _)| n);
    let first = at.next().with_context(|| format!("no `{what}` in the native program"))?;
    ensure!(at.next().is_none(), "two `{what}` in the native program");
    Ok(first)
}

/// The sphere's body drawing 0.69d's term.
fn sphere(body: &[Vec<u32>], t: u32) -> Result<Vec<Vec<u32>>> {
    let direction = find(body, &op(DIV, &[r(0, "xyz"), rs(0, "xyzx"), rs(1, "wwww")]), "div r0.xyz, r0.xyzx, r1.wwww")?;
    let cosine = find(body, &op(DP3, &[r(0, "x"), rs(1, "xyzx"), rs(0, "xyzx")]), "dp3 r0.x, r1.xyzx, r0.xyzx")?;
    let clamp = find(body, &op(MAX, &[r(0, "x"), rc(0, "x"), l(0.0)]), "max r0.x, r0.x, l(0)")?;
    let power = find(body, &op(MUL, &[r(0, "y"), rc(0, "y"), cbc(0, 12, "w")]), "mul r0.y, r0.y, cb0[12].w")?;
    ensure!(direction + 1 == cosine && cosine + 1 == clamp && clamp < power, "the sphere's facing moved");
    let mut out = body[..direction].to_vec();
    out.push(op(MAX, &[r(1, "w"), rc(1, "w"), l(0.01)]));
    out.extend_from_slice(&body[direction..clamp]);
    out.extend(facing(t, 0, "x"));
    out.extend_from_slice(&body[clamp + 1..power]);
    out.push(op(MUL, &[r(0, "y"), rc(0, "y"), rc(t, "y")]));
    out.extend_from_slice(&body[power + 1..]);
    Ok(out)
}

/// Does `op` write an output register?
fn writes_output(op: &[u32]) -> bool {
    op.len() > 2 && op[0] & 0x8000_0000 == 0 && (op[1] >> 12) & 0xff == 2
}

/// The box's body drawing its falloff times 0.69d's term.
fn cube(body: &[Vec<u32>], t: u32) -> Result<Vec<Vec<u32>>> {
    let s = t + 1;
    let normal = find(body, &op(MIN, &[r(1, "xyz"), rs(1, "xyzx"), l4([1.0, 1.0, 1.0, 0.0])]), "min r1.xyz, r1.xyzx, l(1, 1, 1, 0)")?;
    let local = find(body, &op(DP4, &[r(2, "z"), cb(0, 9, "xyzw"), rs(0, "xyzw")]), "dp4 r2.z, cb0[9].xyzw, r0.xyzw")?;
    ensure!(normal < local, "the box's normal follows its position");
    let outputs: Vec<&Vec<u32>> = body[local + 1..].iter().filter(|i| writes_output(i)).collect();
    ensure!(body[body.len() - outputs.len()..].iter().all(|i| writes_output(i)), "the box writes its targets before its end");
    let one = l4([1.0, 1.0, 1.0, 0.0]);
    let mut out = body[..=normal].to_vec();
    out.push(op(MOV, &[r(s, "xyz"), rs(1, "xyzx")]));
    out.extend_from_slice(&body[normal + 1..=local]);
    // the falloff of the variant without dot_normal, from the local position r2
    out.extend([
        op(MIN, &[r(0, "xyz"), abs(rs(2, "xyzx")), one.clone()]),
        op(ADD, &[r(1, "xyz"), neg(cb(0, 14, "xyzx")), one.clone()]),
        op(ADD, &[r(0, "xyz"), rs(0, "xyzx"), neg(rs(1, "xyzx"))]),
        op(ADD, &[r(1, "xyz"), neg(rs(1, "xyzx")), one.clone()]),
        op(DIV, &[r(1, "xyz"), l4([1.0; 4]), rs(1, "xyzx")]),
        op_sat(MUL, &[r(0, "xyz"), rs(0, "xyzx"), rs(1, "xyzx")]),
        op(MAD, &[r(1, "xyz"), rs(0, "xyzx"), l4([-2.0, -2.0, -2.0, 0.0]), l4([3.0, 3.0, 3.0, 0.0])]),
        op(MUL, &[r(0, "xyz"), rs(0, "xyzx"), rs(0, "xyzx")]),
        op(MAD, &[r(0, "xyz"), neg(rs(1, "xyzx")), rs(0, "xyzx"), one.clone()]),
        op(MUL, &[r(0, "x"), rc(0, "y"), rc(0, "x")]),
        op_sat(MAD, &[r(0, "x"), neg(rc(0, "x")), rc(0, "z"), l(1.0)]),
        op(ADD, &[r(0, "x"), neg(rc(0, "x")), l(1.0)]),
    ]);
    // P - C = sum_i p_i a_i / |a_i|^2 into r1, the direction to the centre, the cosine
    for (axis, row) in ["x", "y", "z"].into_iter().zip(7..) {
        out.push(op(MOV, &[r(3, "xyz"), cb(0, row, "xyzx")]));
        out.push(op(DP3, &[r(3, "w"), rs(3, "xyzx"), rs(3, "xyzx")]));
        out.push(op(DIV, &[r(3, "w"), rc(2, axis), rc(3, "w")]));
        out.push(if axis == "x" {
            op(MUL, &[r(1, "xyz"), rs(3, "wwww"), rs(3, "xyzx")])
        } else {
            op(MAD, &[r(1, "xyz"), rs(3, "wwww"), rs(3, "xyzx"), rs(1, "xyzx")])
        });
    }
    out.extend([
        op(DP3, &[r(1, "w"), rs(1, "xyzx"), rs(1, "xyzx")]),
        op(SQRT, &[r(1, "w"), rc(1, "w")]),
        op(MAX, &[r(1, "w"), rc(1, "w"), l(0.01)]),
        op(DIV, &[r(1, "xyz"), neg(rs(1, "xyzx")), rs(1, "wwww")]),
        op(DP3, &[r(0, "y"), rs(s, "xyzx"), rs(1, "xyzx")]),
    ]);
    out.extend(facing(t, 0, "y"));
    out.push(op(MUL, &[r(0, "x"), rc(0, "x"), rc(0, "y")]));
    for o in outputs {
        let alpha = (o[1] >> 4) & 0xf == 8;
        out.push(if alpha { op(MOV, &[vec![o[1], o[2]], rc(0, "x")]) } else { o.clone() });
    }
    Ok(out)
}

/// The native instructions with the marked scenes' branch round their body.
fn splice(ops: &[Vec<u32>], shape: Shape, markers: &[f32]) -> Result<Vec<Vec<u32>>> {
    let start = ops.iter().position(|i| !declaration(i)).context("no program body")?;
    ensure!(ops[start..].iter().all(|i| !declaration(i)), "a declaration inside the body");
    let (last, body) = ops[start..].split_last().context("empty body")?;
    ensure!(*last == op(RET, &[]), "the body does not end with ret");
    ensure!(
        body.iter().all(|i| !matches!(i[0] & 2047, IF | ELSE | ENDIF | RET | 0x3f | 0x30 | 0x16 | 0x4c | 0x17)),
        "the native body has flow control"
    );
    let temps = ops[..start].iter().position(|i| i[0] & 2047 == DCL_TEMPS).context("no temporaries")?;
    let t = ops[temps][1];
    let marked = match shape {
        Shape::Sphere => sphere(body, t)?,
        Shape::Box => cube(body, t)?,
    };
    let mut out = ops[..start].to_vec();
    out[temps] = vec![ops[temps][0], t + if shape == Shape::Box { 2 } else { 1 }];
    let cb1 = out
        .iter_mut()
        .find(|i| i[0] & 2047 == DCL_CONSTANT_BUFFER && i.len() == 4 && i[2] == 1)
        .context("no static_globals declaration")?;
    cb1[3] = cb1[3].max(19);
    out.extend(condition(t, markers));
    out.push(if_nz(rc(t, "x")));
    out.extend(decode(t));
    out.extend(marked);
    out.push(op(ELSE, &[]));
    out.extend_from_slice(body);
    out.push(op(ENDIF, &[]));
    out.push(op(RET, &[]));
    Ok(out)
}

/// A `dot_normal` variant of `ambient_light.ps` drawing 0.69d's facing in the scenes of
/// `markers`.
pub fn build_ambient_wrap(target: &[u8], variant: &str, markers: &[f32]) -> Result<Vec<u8>> {
    ensure!(!markers.is_empty(), "no 0.69d scene markers");
    ensure!(markers.iter().all(|m| m.is_finite()), "invalid marker");
    let Some(&(_, shape, hash)) = DOT_NORMAL_VARIANTS.iter().find(|(name, _, _)| *name == variant) else {
        bail!("not a dot_normal ambient light variant");
    };
    let (offset, native) = pinned_code(target, hash)?;
    let mut entries = chunks(native)?;
    let program = entries.iter_mut().find(|(t, _)| t == b"SHDR").context("missing ambient light program")?;
    let native_words = words(&program.1)?;
    let ops = instructions(&native_words)?;
    let spliced = splice(&ops, shape, markers)?;
    let mut updated = native_words[..2].to_vec();
    updated.extend(spliced.iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    program.1 = bytes(&updated);
    let reflection = entries.iter_mut().find(|(t, _)| t == b"RDEF").context("missing ambient light reflection")?;
    mark_used(&mut reflection.1, "static_globals", "atmosphere_parameters3", 288, 16)?;
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(output)
}

#[cfg(test)]
mod tests;

/// Marks a constant of a buffer used (the game binds what the reflection says is used).
fn mark_used(rdef: &mut [u8], buffer: &str, variable: &str, offset: u32, size: u32) -> Result<()> {
    let string = |data: &[u8], at: usize| -> Result<String> {
        let tail = data.get(at..).context("missing reflection string")?;
        let len = tail.iter().position(|b| *b == 0).context("unterminated reflection string")?;
        Ok(std::str::from_utf8(&tail[..len])?.to_owned())
    };
    let count = u32_at(rdef, 0)? as usize;
    let buffers = u32_at(rdef, 4)? as usize;
    for i in 0..count {
        let at = buffers + i * 24;
        if string(rdef, u32_at(rdef, at)? as usize)? != buffer {
            continue;
        }
        let vars = u32_at(rdef, at + 4)? as usize;
        let start = u32_at(rdef, at + 8)? as usize;
        for j in 0..vars {
            let at = start + j * 24;
            if string(rdef, u32_at(rdef, at)? as usize)? == variable {
                ensure!(u32_at(rdef, at + 4)? == offset && u32_at(rdef, at + 8)? == size, "{variable} moved");
                rdef[at + 12..at + 16].copy_from_slice(&2u32.to_le_bytes());
                return Ok(());
            }
        }
    }
    anyhow::bail!("no {buffer}.{variable}")
}
