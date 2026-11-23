// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::ambient_facing::{Facing, Falloff, encode};

/// One pixel of an SM 4.0 pixel program: the instructions these programs use.
struct Pixel<'a> {
    r: Vec<[u32; 4]>,
    o: [[u32; 4]; 2],
    v0: [f32; 4],
    cb: [Vec<[u32; 4]>; 2],
    /// The texel each texture slot returns.
    texel: &'a dyn Fn(u32) -> [f32; 4],
}

struct Operand {
    ty: u32,
    mode: u32,
    sel: u32,
    modifier: u32,
    index: Vec<u32>,
    imm: [u32; 4],
}

fn operand(words: &[u32], at: &mut usize) -> Operand {
    let t = words[*at];
    *at += 1;
    let mut modifier = 0;
    if t & 0x8000_0000 != 0 {
        modifier = (words[*at] >> 6) & 0xff;
        *at += 1;
    }
    let (comps, mode, sel, ty, dim) = (t & 3, (t >> 2) & 3, (t >> 4) & 0xff, (t >> 12) & 0xff, (t >> 20) & 3);
    assert_eq!((t >> 22) & 0x1ff, 0, "relative indexing");
    let mut imm = [0; 4];
    if ty == 4 {
        let n = if comps == 1 { 1 } else { 4 };
        for v in imm.iter_mut().take(n) {
            *v = words[*at];
            *at += 1;
        }
        if n == 1 {
            imm = [imm[0]; 4];
        }
    }
    let index = (0..dim)
        .map(|_| {
            *at += 1;
            words[*at - 1]
        })
        .collect();
    Operand { ty, mode, sel, modifier, index, imm }
}

fn f(v: u32) -> f32 {
    f32::from_bits(v)
}
fn u(v: f32) -> u32 {
    v.to_bits()
}

impl Pixel<'_> {
    fn new<'a>(texel: &'a dyn Fn(u32) -> [f32; 4]) -> Pixel<'a> {
        Pixel { r: vec![[0; 4]; 16], o: [[0; 4]; 2], v0: [0.0; 4], cb: [vec![[0; 4]; 32], vec![[0; 4]; 32]], texel }
    }

    fn read(&self, o: &Operand) -> [u32; 4] {
        let raw = match o.ty {
            0 => self.r[o.index[0] as usize],
            1 => self.v0.map(u),
            4 => o.imm,
            8 => self.cb[o.index[0] as usize][o.index[1] as usize],
            7 => (self.texel)(o.index[0]).map(u),
            6 => [0; 4],
            ty => panic!("operand type {ty}"),
        };
        let v = match o.mode {
            0 => raw,
            1 => std::array::from_fn(|i| raw[((o.sel >> (2 * i)) & 3) as usize]),
            _ => [raw[(o.sel & 3) as usize]; 4],
        };
        v.map(|x| match o.modifier {
            0 => x,
            1 => x ^ 0x8000_0000,
            2 => x & 0x7fff_ffff,
            _ => x | 0x8000_0000,
        })
    }

    fn write(&mut self, o: &Operand, v: [u32; 4], sat: bool) {
        assert_eq!(o.mode, 0);
        let target = match o.ty {
            0 => &mut self.r[o.index[0] as usize],
            2 => &mut self.o[o.index[0] as usize],
            ty => panic!("destination type {ty}"),
        };
        for i in 0..4 {
            if o.sel & (1 << i) != 0 {
                target[i] = if sat { u(if f(v[i]).is_nan() { 0.0 } else { f(v[i]).clamp(0.0, 1.0) }) } else { v[i] };
            }
        }
    }

    fn run(&mut self, ops: &[Vec<u32>]) {
        let mut active = true;
        let mut stack = Vec::new();
        for op in ops {
            let code = op[0] & 2047;
            if declaration(op) {
                continue;
            }
            let mut at = 1;
            let mut operands = Vec::new();
            while at < op.len() {
                operands.push(operand(op, &mut at));
            }
            match code {
                IF => {
                    let c = self.read(&operands[0])[0];
                    let taken = if op[0] & 0x4_0000 != 0 { c != 0 } else { c == 0 };
                    stack.push((active, taken));
                    active = active && taken;
                    continue;
                }
                ELSE => {
                    let (parent, taken) = *stack.last().unwrap();
                    active = parent && !taken;
                    continue;
                }
                ENDIF => {
                    active = stack.pop().unwrap().0;
                    continue;
                }
                RET if active => return,
                _ if !active => continue,
                _ => {}
            }
            let sat = op[0] & 0x2000 != 0;
            let s: Vec<[u32; 4]> = operands[1..].iter().map(|o| self.read(o)).collect();
            let lanes = |g: &dyn Fn(usize) -> u32| -> [u32; 4] { std::array::from_fn(g) };
            let fl = |g: &dyn Fn(f32, f32, f32) -> f32| -> [u32; 4] {
                std::array::from_fn(|i| {
                    let a = |k: usize| s.get(k).map_or(0.0, |v| f(v[i]));
                    u(g(a(0), a(1), a(2)))
                })
            };
            let dot = |n: usize| -> [u32; 4] {
                let sum = (1..n).fold(f(s[0][0]) * f(s[1][0]), |sum, i| sum + f(s[0][i]) * f(s[1][i]));
                [u(sum); 4]
            };
            let flag = |b: bool| if b { u32::MAX } else { 0 };
            let v = match code {
                ADD => fl(&|a, b, _| a + b),
                MUL => fl(&|a, b, _| a * b),
                MAD => fl(&|a, b, c| a * b + c),
                DIV => fl(&|a, b, _| a / b),
                MIN => fl(&|a, b, _| a.min(b)),
                MAX => fl(&|a, b, _| a.max(b)),
                SQRT => fl(&|a, _, _| a.sqrt()),
                0x44 => fl(&|a, _, _| 1.0 / a.sqrt()),
                LOG => fl(&|a, _, _| a.log2()),
                EXP => fl(&|a, _, _| a.exp2()),
                MOV => s[0],
                0x37 => lanes(&|i| if s[0][i] != 0 { s[1][i] } else { s[2][i] }),
                0x0f => dot(2),
                DP3 => dot(3),
                DP4 => dot(4),
                0x31 => lanes(&|i| flag(f(s[0][i]) < f(s[1][i]))),
                0x1d => lanes(&|i| flag(f(s[0][i]) >= f(s[1][i]))),
                EQ => lanes(&|i| flag(f(s[0][i]) == f(s[1][i]))),
                AND => lanes(&|i| s[0][i] & s[1][i]),
                OR => lanes(&|i| s[0][i] | s[1][i]),
                IADD => lanes(&|i| s[0][i].wrapping_add(s[1][i])),
                USHR => lanes(&|i| s[0][i] >> (s[1][i] & 31)),
                ULT => lanes(&|i| flag(s[0][i] < s[1][i])),
                UTOF => lanes(&|i| u(s[0][i] as f32)),
                ITOF => lanes(&|i| u(s[0][i] as i32 as f32)),
                0x48 => s[1],
                code => panic!("opcode {code:#x}"),
            };
            self.write(&operands[0], v, sat);
        }
        panic!("no ret");
    }
}

const MARKERS: [f32; 2] = [33001.0, 33003.0];

#[test]
fn the_decode_and_facing_blocks_compute_0_69ds_term() {
    let mut ops = vec![op(DCL_TEMPS, &[vec![2]])];
    ops.extend(condition(0, &MARKERS));
    ops.push(if_nz(rc(0, "x")));
    ops.extend(decode(0, Shape::Sphere));
    ops.push(op(MOV, &[r(1, "x"), cbc(0, 0, "x")]));
    ops.extend(facing(0, 1, "x"));
    ops.push(op(MOV, &[o(0, "x"), rc(1, "x")]));
    ops.push(op(MOV, &[o(0, "yzw"), rs(0, "xyzw")]));
    ops.push(op(ENDIF, &[]));
    ops.push(op(RET, &[]));
    let texel = |_: u32| [0.0f32; 4];
    for (cutoff, power) in [(0.0, 0.1), (0.0, 0.5), (0.0, 1.0), (0.0, 4.0), (0.0, 10.0), (0.3, 2.0), (-0.5, 1.5), (1.0, 1.0)] {
        let facing = Facing { cutoff, power };
        let e = encode(Falloff::Sphere { power: 2.75 }, facing).unwrap();
        for x in [-1.0f32, -0.7, -0.2, 0.0, 0.1, 0.33, 0.8, 1.0] {
            for (marker, marked) in [(33003.0, true), (33002.0, false), (-32281.0, false)] {
                let mut p = Pixel::new(&texel);
                p.cb[0][0] = [u(x); 4];
                p.cb[0][12] = [0, 0, 0, u(e)];
                p.cb[1][18] = [0, 0, 0, u(marker)];
                p.run(&ops);
                if marked {
                    let carried = crate::ambient_facing::decode(e, Shape::Sphere).unwrap();
                    assert_eq!(f(p.o[0][0]).to_bits(), carried.facing.eval(x).to_bits(), "{cutoff} {power} {x}");
                    assert_eq!([f(p.o[0][1]), f(p.o[0][2]), f(p.o[0][3])], [2.75, carried.facing.cutoff, carried.facing.power]);
                    assert!((carried.facing.cutoff - cutoff).abs() < 1e-7 && carried.falloff == Falloff::Sphere { power: 2.75 });
                } else {
                    assert_eq!(p.o[0], [0; 4]);
                }
            }
            // a plain power is not decoded in a marked scene
            let mut p = Pixel::new(&texel);
            p.cb[0][12] = [0, 0, 0, u(4.0)];
            p.cb[1][18] = [0, 0, 0, u(33001.0)];
            p.run(&ops);
            assert_eq!(p.o[0], [0; 4]);
        }
    }
}

fn header() -> Vec<Vec<u32>> {
    vec![
        vec![0x0400_0059, 0x0020_8e46, 0, 15],
        vec![0x0400_0059, 0x0020_8e46, 1, 15],
        op(DCL_TEMPS, &[vec![4]]),
    ]
}

/// A sphere-shaped native body: `max(N.L, 0) * 0.5^p` into o0.w.
fn sphere_native(n: [f32; 3], d: [f32; 3]) -> Vec<Vec<u32>> {
    let mut ops = header();
    ops[2] = op(DCL_TEMPS, &[vec![3]]);
    ops.extend([
        op(MOV, &[r(1, "xyz"), l4([n[0], n[1], n[2], 0.0])]),
        op(MOV, &[r(0, "xyz"), l4([d[0], d[1], d[2], 0.0])]),
        op(DP3, &[r(0, "w"), rs(0, "xyzx"), rs(0, "xyzx")]),
        op(SQRT, &[r(1, "w"), rc(0, "w")]),
        op(DIV, &[r(0, "xyz"), rs(0, "xyzx"), rs(1, "wwww")]),
        op(DP3, &[r(0, "x"), rs(1, "xyzx"), rs(0, "xyzx")]),
        op(MAX, &[r(0, "x"), rc(0, "x"), l(0.0)]),
        op(MOV, &[r(0, "y"), l(0.5)]),
        op(LOG, &[r(0, "y"), rc(0, "y")]),
        op(MUL, &[r(0, "y"), rc(0, "y"), cbc(0, 12, "w")]),
        op(EXP, &[r(0, "y"), rc(0, "y")]),
        op(MUL, &[o(0, "w"), rc(0, "x"), rc(0, "y")]),
        op(MOV, &[o(0, "xyz"), cb(0, 12, "xyzx")]),
        op(RET, &[]),
    ]);
    ops
}

/// A box-shaped native body: the normal in r1 (then overwritten), the local position of
/// `p` in r2, a constant alpha.
fn box_native(n: [f32; 3], p: [f32; 3]) -> Vec<Vec<u32>> {
    let mut ops = header();
    ops.extend([
        op(MOV, &[r(1, "xyz"), l4([n[0], n[1], n[2], 0.0])]),
        op(MIN, &[r(1, "xyz"), rs(1, "xyzx"), l4([1.0, 1.0, 1.0, 0.0])]),
        op(MUL, &[r(1, "xyz"), rs(1, "xyzx"), l(0.5)]),
        op(MOV, &[r(0, "xyzw"), l4([p[0], p[1], p[2], 1.0])]),
        op(DP4, &[r(2, "x"), cb(0, 7, "xyzw"), rs(0, "xyzw")]),
        op(DP4, &[r(2, "y"), cb(0, 8, "xyzw"), rs(0, "xyzw")]),
        op(DP4, &[r(2, "z"), cb(0, 9, "xyzw"), rs(0, "xyzw")]),
        op(MOV, &[r(0, "x"), l(0.25)]),
        op(MOV, &[o(0, "w"), rc(0, "x")]),
        op(MOV, &[o(1, "w"), rc(0, "x")]),
        op(MOV, &[o(0, "xyz"), cb(0, 12, "xyzx")]),
        op(MOV, &[o(1, "xyz"), l4([0.0; 4])]),
        op(RET, &[]),
    ]);
    ops
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot3(a, a).sqrt();
    a.map(|c| c / l)
}

/// 0.69d's cosine to the centre: `dot(N, d / max(|d|, 0.01))`.
fn cosine(n: [f32; 3], d: [f32; 3]) -> f32 {
    let l = dot3(d, d).sqrt().max(0.01);
    dot3(n, d.map(|c| c / l))
}

/// A box's `view_to_local` rows: axes `basis` over `half`, centred on `centre`.
fn view_to_local(basis: [[f32; 3]; 3], half: [f32; 3], centre: [f32; 3]) -> [[u32; 4]; 3] {
    std::array::from_fn(|i| {
        let a = basis[i].map(|c| c / half[i]);
        [u(a[0]), u(a[1]), u(a[2]), u(-dot3(a, centre))]
    })
}

fn basis() -> [[f32; 3]; 3] {
    let (a, b) = (0.6f32, 0.3f32);
    let r = [[a.cos(), a.sin(), 0.0], [-a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]];
    let q = [[1.0, 0.0, 0.0], [0.0, b.cos(), b.sin()], [0.0, -b.sin(), b.cos()]];
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| r[i][k] * q[k][j]).sum()))
}

#[test]
fn a_marked_sphere_draws_0_69ds_facing_and_others_the_native_body() {
    let texel = |_: u32| [0.0f32; 4];
    let n = normalize([0.3, -0.5, 0.8]);
    for d in [[1.0, 2.0, -0.5], [-0.3, 0.1, -2.0], [0.0, 0.004, 0.002], [0.5, -0.9, 1.1]] {
        let native = sphere_native(n, d);
        let ops = splice(&native, Shape::Sphere, &MARKERS).unwrap();
        // the native body is the else branch, unchanged
        let at = ops.iter().position(|i| *i == op(ELSE, &[])).unwrap();
        assert_eq!(&ops[at + 1..ops.len() - 2], &native[3..native.len() - 1]);
        assert_eq!(ops[2], op(DCL_TEMPS, &[vec![4]]));
        assert_eq!(ops[1][3], 19);
        for (cutoff, power) in [(0.0, 4.0), (0.0, 0.1), (0.25, 2.0)] {
            let facing = Facing { cutoff, power };
            let e = encode(Falloff::Sphere { power: 3.5 }, facing).unwrap();
            for (marker, power_bits) in [(33001.0, e), (33002.0, e), (33001.0, 3.5), (-32281.0, 2.0)] {
                let run = |ops: &[Vec<u32>]| {
                    let mut p = Pixel::new(&texel);
                    p.cb[0][12] = [u(0.2), u(0.4), u(0.6), u(power_bits)];
                    p.cb[1][18] = [0, 0, 0, u(marker)];
                    p.run(ops);
                    p.o
                };
                let out = run(&ops);
                if MARKERS.contains(&marker) && power_bits == e {
                    let want = facing.eval(cosine(n, d)) * 0.5f32.powf(3.5);
                    assert!((f(out[0][3]) - want).abs() < 1e-6, "{d:?} {cutoff} {power}: {} {want}", f(out[0][3]));
                    assert_eq!(&out[0][..3], &[u(0.2), u(0.4), u(0.6)]);
                } else {
                    assert_eq!(out, run(&native));
                }
            }
        }
    }
}

/// 0.69d's box falloff (`ambient_light_*.ps`) at the local position `l` (unit coordinates) of a
/// box of half extents `half` and side widths `side` (over the half extents).
fn falloff_069d(l: [f32; 3], half: [f32; 3], side: [f32; 3], power: f32, gradient: bool) -> f32 {
    let fall: f32 = (0..3)
        .map(|i| {
            let inner = (1.0 - side[i]).max(0.01 / half[i]);
            let t = ((l[i].abs().min(1.0) - inner) / (1.0 - inner)).clamp(0.0, 1.0);
            (1.0 - t).powf(power)
        })
        .product();
    if gradient { fall * (0.5 - 0.5 * l[1]).clamp(0.0, 1.0).powf(power) } else { fall }
}

#[test]
fn a_marked_box_draws_0_69ds_falloff_and_gradient_times_its_facing_to_its_centre() {
    let texel = |_: u32| [0.0f32; 4];
    let n = normalize([-0.2, 0.9, 0.3]);
    let (half, centre, side) = ([2.0, 0.7, 3.5], [1.0, -2.0, 6.0], [0.4, 0.9, 0.2]);
    let rows = view_to_local(basis(), half, centre);
    for offset in [[0.3, 0.2, -1.0], [-1.2, -0.1, 2.9], [0.0, 0.0, 0.001], [1.5, 0.5, -3.0]] {
        let p: [f32; 3] = std::array::from_fn(|i| centre[i] + (0..3).map(|k| basis()[k][i] * offset[k]).sum::<f32>());
        let native = box_native(n, p);
        let ops = splice(&native, Shape::Box, &MARKERS).unwrap();
        let at = ops.iter().position(|i| *i == op(ELSE, &[])).unwrap();
        assert_eq!(&ops[at + 1..ops.len() - 2], &native[3..native.len() - 1]);
        for (falloff_power, gradient, facing) in [(1.0, false, Facing { cutoff: 0.0, power: 4.0 }), (3.0, true, Facing::NONE), (0.5, true, Facing { cutoff: 0.0, power: 1.0 })] {
        let e = encode(Falloff::Box { power: falloff_power, gradient }, facing).unwrap();
        for (marker, power_bits) in [(33003.0, e), (33002.0, e), (33003.0, 1.0)] {
            let run = |ops: &[Vec<u32>]| {
                let mut px = Pixel::new(&texel);
                px.cb[0][7..10].copy_from_slice(&rows);
                px.cb[0][12] = [u(0.9), u(0.8), u(0.7), u(power_bits)];
                px.cb[0][14] = [u(side[0]), u(side[1]), u(side[2]), 0];
                px.cb[1][18] = [0, 0, 0, u(marker)];
                px.run(ops);
                px.o
            };
            let out = run(&ops);
            if marker == 33003.0 && power_bits == e {
                let l: [f32; 3] = std::array::from_fn(|i| offset[i] / half[i]);
                let fall = falloff_069d(l, half, side, falloff_power, gradient);
                let d: [f32; 3] = std::array::from_fn(|i| centre[i] - p[i]);
                let want = fall * facing.eval(cosine(n, d));
                assert!((f(out[0][3]) - want).abs() < 1e-5, "{offset:?} {falloff_power} {gradient}: {} {want}", f(out[0][3]));
                assert_eq!(out[1][3], out[0][3]);
                assert_eq!(&out[0][..3], &[u(0.9), u(0.8), u(0.7)]);
            } else {
                assert_eq!(out, run(&native));
            }
        }
        }
    }
}

/// 0.69d's cylinder (`ambient_light_*.ps`, geometry 2) at the local position `l` of the box
/// around it (half extents `r`, `h`, `r`), side widths over the half extents `side`.
fn cylinder_069d(l: [f32; 3], half: [f32; 3], side: [f32; 3], power: f32) -> f32 {
    let fall = |d: f32, i: usize| {
        let inner = (1.0 - side[i]).max(0.01 / half[i]);
        (1.0 - ((d.min(1.0) - inner) / (1.0 - inner)).clamp(0.0, 1.0)).powf(power)
    };
    fall((l[0] * l[0] + l[2] * l[2]).sqrt(), 0) * fall(l[1].abs(), 1)
}

#[test]
fn a_marked_cylinder_fades_by_the_distance_from_its_axis() {
    let texel = |_: u32| [0.0f32; 4];
    let (half, centre, side) = ([2.4, 1.1, 2.4], [3.0, 1.0, -2.0], [0.125, 0.27, 0.125]);
    let rows = view_to_local(basis(), half, centre);
    let e = encode(Falloff::Cylinder { power: 1.0, gradient: false }, Facing::NONE).unwrap();
    // inside the disc, in its band, and in the box's corner outside the disc (0.69d: nothing)
    for offset in [[0.3, 0.2, -1.0], [1.6, -0.5, 1.6], [2.2, 0.0, 2.2], [0.0, 0.9, 2.3], [-1.0, 0.0, 0.5]] {
        let p: [f32; 3] = std::array::from_fn(|i| centre[i] + (0..3).map(|k| basis()[k][i] * offset[k]).sum::<f32>());
        let ops = splice(&box_native([0.0, 0.0, 1.0], p), Shape::Box, &MARKERS).unwrap();
        let mut px = Pixel::new(&texel);
        px.cb[0][7..10].copy_from_slice(&rows);
        px.cb[0][12] = [0, 0, 0, u(e)];
        px.cb[0][14] = [u(side[0]), u(side[1]), u(side[2]), 0];
        px.cb[1][18] = [0, 0, 0, u(33001.0)];
        px.run(&ops);
        let l: [f32; 3] = std::array::from_fn(|i| offset[i] / half[i]);
        let want = cylinder_069d(l, half, side, 1.0);
        assert!((f(px.o[0][3]) - want).abs() < 1e-5, "{offset:?}: {} {want}", f(px.o[0][3]));
        if offset == [2.2, 0.0, 2.2] {
            assert_eq!(want, 0.0);
        }
    }
}

#[test]
fn the_gradient_runs_from_full_at_minus_y_to_nothing_at_plus_y() {
    let texel = |_: u32| [0.0f32; 4];
    let (half, centre) = ([1.0, 2.0, 1.0], [0.0, 0.0, 0.0]);
    let ident = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let rows = view_to_local(ident, half, centre);
    let e = encode(Falloff::Box { power: 2.0, gradient: true }, Facing::NONE).unwrap();
    let mut last = f32::INFINITY;
    for y in [-1.99f32, -1.0, 0.0, 1.0, 1.99] {
        let ops = splice(&box_native([0.0, 0.0, 1.0], [0.0, y, 0.0]), Shape::Box, &MARKERS).unwrap();
        let mut px = Pixel::new(&texel);
        px.cb[0][7..10].copy_from_slice(&rows);
        px.cb[0][12] = [0, 0, 0, u(e)];
        px.cb[0][14] = [u(0.01), u(0.005), u(0.01), 0];
        px.cb[1][18] = [0, 0, 0, u(33001.0)];
        px.run(&ops);
        let a = f(px.o[0][3]);
        let want = (0.5 - 0.25 * y).powi(2);
        assert!((a - want).abs() < 1e-3, "{y}: {a} {want}");
        assert!(a < last);
        last = a;
    }
}

#[test]
fn a_body_with_flow_control_is_refused() {
    let mut ops = sphere_native([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
    let at = ops.len() - 1;
    ops.insert(at, op(RET, &[]));
    assert!(splice(&ops, Shape::Sphere, &MARKERS).is_err());
    assert!(splice(&box_native([0.0, 0.0, 1.0], [0.0; 3]), Shape::Sphere, &MARKERS).is_err());
}

/// The program of a retail blob.
fn program(blob: &[u8], hash: &str) -> Vec<Vec<u32>> {
    let (_, code) = pinned_code(blob, hash).unwrap();
    let entries = chunks(code).unwrap();
    instructions(&words(&entries.iter().find(|(t, _)| t == b"SHDR").unwrap().1).unwrap()).unwrap()
}

/// The 0.26g normal (`ambient_light.ps`'s decode) of a G-buffer texel.
fn native_normal(t: [f32; 2]) -> [f32; 3] {
    let n = [t[0] * 4.0 - 2.0, t[1] * 4.0 - 2.0];
    let q = n[0] * n[0] + n[1] * n[1];
    let a = (1.0 - q * 0.25).abs().sqrt();
    normalize([n[0] * a, n[1] * a, -(1.0 - q * 0.5)]).map(|c| c.clamp(-1.0, 1.0))
}

#[test]
#[ignore = "requires WEATHER_SHADER_AUDIT with the stock ambient_light.ps blobs (ambient_light-<sha256>.blob)"]
fn retail_dot_normal_variants_draw_0_69ds_facing_and_keep_the_native_path() {
    let root = std::path::PathBuf::from(std::env::var("WEATHER_SHADER_AUDIT").unwrap());
    let read = |hash: &str| std::fs::read(root.join(format!("ambient_light-{hash}.blob"))).unwrap();
    // the sphere variant without dot_normal: the falloff 0.69d's term multiplies (a box draws
    // 0.69d's own falloff)
    let plain_sphere = program(&read("e8d229fce0059c24b6ddccadeeeac7cbe3041e2fc9ded8ececa73b8fc89f6e71"), "e8d229fce0059c24b6ddccadeeeac7cbe3041e2fc9ded8ececa73b8fc89f6e71");
    let (screen, ray, depth) = ([64.0f32, 48.0], [1.1f32, 0.8, 1.0], 6.0f32);
    let (mut cases, mut lit) = (0, 0);
    for (variant, shape, hash) in DOT_NORMAL_VARIANTS {
        let blob = read(hash);
        let native = program(&blob, hash);
        let built = build_ambient_wrap(&blob, variant, &MARKERS).unwrap();
        let (_, code) = pinned_code(&blob, hash).unwrap();
        let header = blob.len() - code.len();
        assert_eq!(&built[..header], &blob[..header]);
        let entries = chunks(&built[header..]).unwrap();
        let ops = instructions(&words(&entries.iter().find(|(t, _)| t == b"SHDR").unwrap().1).unwrap()).unwrap();
        let start = native.iter().position(|i| !declaration(i)).unwrap();
        let at = ops.iter().position(|i| *i == op(ELSE, &[])).unwrap();
        assert_eq!(&ops[at + 1..ops.len() - 2], &native[start..native.len() - 1], "{variant}");
        for normal in [[0.5, 0.5], [0.62, 0.41], [0.31, 0.7], [0.55, 0.2], [0.8, 0.55]] {
            for pixel in [[32.0, 24.0], [10.5, 40.5], [60.0, 3.0]] {
                let uv = [pixel[0] * (1.0 / screen[0]), pixel[1] * (1.0 / screen[1])];
                let pos = [(1.0 - 2.0 * uv[0]) * ray[0] / ray[2] * depth, (1.0 - 2.0 * uv[1]) * ray[1] / ray[2] * depth, depth];
                let n = native_normal(normal);
                for offset in [[0.8, -0.4, 0.6], [-1.3, 0.2, -0.9], [0.1, 1.2, 0.3], [0.0, 0.0, 0.005]] {
                    let centre: [f32; 3] = std::array::from_fn(|i| pos[i] + offset[i]);
                    for (cutoff, power) in [(0.0, 4.0), (0.0, 0.5), (0.0, 10.0), (0.0, 1.0)] {
                        let facing = Facing { cutoff, power };
                        let falloff = match shape {
                            Shape::Sphere => Falloff::Sphere { power: 2.25 },
                            Shape::Box => Falloff::Box { power: 2.25, gradient: power > 2.0 },
                        };
                        let e = encode(falloff, facing).unwrap();
                        let run = |ops: &[Vec<u32>], marker: f32, power_bits: f32, dot: bool| {
                            // the dot variants read the normal from t0 and depth from t1
                            let texel = |slot: u32| if dot && slot == 0 { [normal[0], normal[1], 0.0, 0.0] } else { [depth, 0.0, 0.0, 0.0] };
                            let mut p = Pixel::new(&texel);
                            p.v0 = [pixel[0], pixel[1], 0.5, 1.0];
                            p.cb[1][14] = [0, 0, u(1.0 / screen[0]), u(1.0 / screen[1])];
                            p.cb[1][18] = [0, 0, 0, u(marker)];
                            p.cb[0][13] = [u(ray[0]), u(ray[1]), u(ray[2]), 0];
                            p.cb[0][11] = [u(centre[0]), u(centre[1]), u(centre[2]), u(3.0)];
                            p.cb[0][12] = [u(0.3), u(0.5), u(0.7), u(power_bits)];
                            p.cb[0][7..10].copy_from_slice(&view_to_local(basis(), [2.5, 1.5, 3.0], centre));
                            p.cb[0][14] = [u(0.5), u(0.6), u(0.3), 0];
                            p.run(ops);
                            p.o
                        };
                        let out = run(&ops, 33003.0, e, true);
                        let fall = match falloff {
                            Falloff::Sphere { .. } => f(run(&plain_sphere, 0.0, 2.25, false)[0][3]),
                            Falloff::Box { power, gradient } => {
                                let rows = view_to_local(basis(), [2.5, 1.5, 3.0], centre);
                                let l: [f32; 3] = std::array::from_fn(|i| (0..3).map(|k| f(rows[i][k]) * pos[k]).sum::<f32>() + f(rows[i][3]));
                                falloff_069d(l, [2.5, 1.5, 3.0], [0.5, 0.6, 0.3], power, gradient)
                            }
                            Falloff::Cylinder { .. } => unreachable!("the retail cases use no cylinder"),
                        };
                        let d: [f32; 3] = std::array::from_fn(|i| centre[i] - pos[i]);
                        let want = fall * facing.eval(cosine(n, d));
                        assert!((f(out[0][3]) - want).abs() < 2e-5, "{variant} {normal:?} {pixel:?} {offset:?} {power}: {} {want}", f(out[0][3]));
                        // every other scene and light: the native program, bit for bit
                        for (marker, bits) in [(33002.0, e), (-32281.0, e), (33001.0, 2.25), (0.0, 1.0)] {
                            assert_eq!(run(&ops, marker, bits, true), run(&native, marker, bits, true), "{variant}");
                        }
                        cases += 1;
                        lit += usize::from(want > 0.01);
                    }
                }
            }
        }
    }
    assert_eq!(cases, 8 * 5 * 3 * 4 * 4);
    assert!(lit * 3 > cases, "{lit} of {cases} lit");
}
