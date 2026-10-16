// SPDX-License-Identifier: GPL-3.0-or-later
//! List walker-collision surfaces too steep for the 0.26 character
//! controller (max slope pi/3, step height 0.6 m) that still look like a
//! climbing route: steep triangles (60..85 degrees from horizontal) whose
//! connected height span exceeds the step height. 0.26 has no ladder state,
//! so such a route cannot be climbed.
//!
//!     steep_walker DB PROJECT
use std::collections::BTreeMap;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn chunk(b: &[u8], id: u32) -> Option<&[u8]> {
    let mut o = 0;
    while o + 8 <= b.len() {
        let k = u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let n = u32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap()) as usize;
        if k == id { return b.get(o + 8..o + 8 + n); }
        o += 8 + n;
    }
    None
}
fn rotation(a: [f32; 3]) -> [[f32; 3]; 3] {
    let (sx, cx) = a[0].sin_cos(); let (sy, cy) = a[1].sin_cos(); let (sz, cz) = a[2].sin_cos();
    [[cy * cz, -cy * sz, sy], [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy], [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy]]
}
fn f3(c: &ConfigValue, k: &str) -> [f32; 3] { match c.named(k) { Some(ConfigValue::Float3(v)) => *v, _ => [0.0; 3] } }
fn int(c: &ConfigValue, k: &str) -> i64 { match c.named(k) { Some(ConfigValue::Integer(v)) => *v as i64, _ => 0 } }
fn main() {
    let mut a = std::env::args().skip(1);
    let mut db = ResourceDb::open(a.next().unwrap()).unwrap();
    let project = BinaryConfig::parse(&db.read(&a.next().unwrap()).unwrap()).unwrap().root;
    let Some(ConfigValue::Indexed(objects)) = project.named("collision_objects") else { return };
    let mut report: BTreeMap<String, (usize, f32, f32, usize)> = BTreeMap::new();
    for o in objects {
        if !(int(o, "cgroup") & 2 != 0 && int(o, "cmask") & 4 != 0) { continue }
        let Some(ConfigValue::String(lib)) = o.named("lib_name") else { continue };
        let mut parts = lib.trim_start_matches("resources/").splitn(2, '#');
        let dir = parts.next().unwrap().to_owned();
        let scale: [f32; 3] = parts.next().map(|s| { let v: Vec<f32> = s.split(['[', ']']).filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect(); [v[0], v[1], v[2]] }).unwrap_or([1.0; 3]);
        let (Ok(vb), Ok(ib)) = (db.read(&format!("{dir}/vertices")), db.read(&format!("{dir}/indices"))) else { continue };
        let (Some(v), Some(i)) = (chunk(&vb, 25), chunk(&ib, 26)) else { continue };
        let r = rotation(f3(o, "rotation")); let t = f3(o, "position");
        let world: Vec<[f32; 3]> = v.chunks_exact(12).map(|c| {
            let p = [0, 4, 8].map(|k| f32::from_le_bytes(c[k..k + 4].try_into().unwrap()));
            std::array::from_fn(|j| (0..3).map(|q| p[q] * scale[q] * r[q][j]).sum::<f32>() + t[j])
        }).collect();
        let (mut lo, mut hi, mut n, mut area) = (f32::MAX, f32::MIN, 0, 0.0f32);
        let mut ramps: Vec<(f32, f32, f32, [f32; 3])> = Vec::new();
        for tri in i.chunks_exact(12) {
            let ix = [0, 4, 8].map(|k| u32::from_le_bytes(tri[k..k + 4].try_into().unwrap()) as usize);
            let (p, q, s) = (world[ix[0]], world[ix[1]], world[ix[2]]);
            let e1 = [q[0] - p[0], q[1] - p[1], q[2] - p[2]]; let e2 = [s[0] - p[0], s[1] - p[1], s[2] - p[2]];
            let nrm = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let l = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt(); if l < 1e-6 { continue }
            let slope = (nrm[1].abs() / l).acos().to_degrees();
            // Ladder-like ramp: a steep triangle that rises over 2 m while
            // being under 1.5 m wide horizontally across its slope.
            let ys = [p[1], q[1], s[1]];
            let rise = ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
            if (55.0..85.0).contains(&slope) && rise > 2.0 && nrm[1] > 0.0 {
                let horizontal = [nrm[0] / l, nrm[2] / l];
                let h = (horizontal[0].powi(2) + horizontal[1].powi(2)).sqrt().max(1e-6);
                let across = [-horizontal[1] / h, horizontal[0] / h];
                let proj: Vec<f32> = [p, q, s].iter().map(|v| v[0] * across[0] + v[2] * across[1]).collect();
                let width = proj.iter().cloned().fold(f32::MIN, f32::max) - proj.iter().cloned().fold(f32::MAX, f32::min);
                if width < 1.5 {
                    ramps.push((slope, rise, width, [(p[0] + q[0] + s[0]) / 3.0, (p[1] + q[1] + s[1]) / 3.0, (p[2] + q[2] + s[2]) / 3.0]));
                }
            }
            if (60.0..85.0).contains(&slope) {
                n += 1; area += l / 2.0;
                for y in [p[1], q[1], s[1]] { lo = lo.min(y); hi = hi.max(y); }
            }
        }
        for (slope, rise, width, c) in &ramps {
            println!("RAMP\t{dir}\tslope {slope:.0} deg\trise {rise:.1} m\twidth {width:.2} m\tat [{:.1}, {:.1}, {:.1}]", c[0], c[1], c[2]);
        }
        if std::env::var("ALL").is_ok() && n > 0 && hi - lo > 0.6 && area > 0.5 {
            let e = report.entry(dir.clone()).or_insert((0, 0.0, 0.0, 0));
            e.0 += n; e.1 = e.1.max(hi - lo); e.2 += area; e.3 += 1;
        }
    }
    for (d, (n, span, area, inst)) in report { println!("{inst} inst\t{n} steep tris\theight span {span:.1} m\tarea {area:.1} m2\t{d}"); }
}
