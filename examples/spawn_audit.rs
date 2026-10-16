// SPDX-License-Identifier: GPL-3.0-or-later
//! Static spawn audit of a packed 0.26 project: for every respawn point,
//! the walkable support under it and the clearance of a standing player
//! capsule against the collision the character controller collides with
//! (instances with cgroup & 2 and cmask & 4, as `audit_battery_geometry`).
//!
//!     spawn_audit DB PROJECT [--radius 0.4] [--height 1.8]
//!
//! Collision instances use the same transform as the engine (Euler rotation
//! of `audit_collision_region.rotation`, `#[sx][sy][sz]` model scale).
//! Primitives are tested as their oriented boxes (conservative for spheres,
//! cylinders and capsules). This is geometry, not a runtime contact.
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

type V = [f32; 3];
fn sub(a: V, b: V) -> V { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn add(a: V, b: V) -> V { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn mul(a: V, s: f32) -> V { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V, b: V) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V, b: V) -> V { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn len(a: V) -> f32 { dot(a, a).sqrt() }

fn rotation(a: V) -> [[f32; 3]; 3] {
    let (sx, cx) = a[0].sin_cos();
    let (sy, cy) = a[1].sin_cos();
    let (sz, cz) = a[2].sin_cos();
    [
        [cy * cz, -cy * sz, sy],
        [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy],
        [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy],
    ]
}
fn transform(p: V, r: &[[f32; 3]; 3], t: V, s: V) -> V {
    let mut out = t;
    for j in 0..3 {
        for i in 0..3 {
            out[j] += p[i] * s[i] * r[i][j];
        }
    }
    out
}

fn f3(c: &ConfigValue, k: &str) -> V {
    match c.named(k) {
        Some(ConfigValue::Float3(v)) => *v,
        _ => [0.0; 3],
    }
}
fn int(c: &ConfigValue, k: &str) -> i64 {
    match c.named(k) {
        Some(ConfigValue::Integer(v)) => *v as i64,
        Some(ConfigValue::Float(v)) => *v as i64,
        _ => 0,
    }
}

fn chunk(bytes: &[u8], id: u32) -> Option<&[u8]> {
    let mut o = 0;
    while o + 8 <= bytes.len() {
        let k = u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let n = u32::from_le_bytes(bytes[o + 4..o + 8].try_into().unwrap()) as usize;
        if k == id {
            return bytes.get(o + 8..o + 8 + n);
        }
        o += 8 + n;
    }
    None
}

/// Closest distance between segment pq and triangle abc (sampled exactly
/// enough for a clearance audit: segment-triangle intersection, then edge
/// and endpoint distances).
fn segment_triangle_distance(p: V, q: V, t: &[V; 3]) -> f32 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let nl = len(n);
    let point_triangle = |x: V| -> f32 {
        if nl > 1e-9 {
            let nn = mul(n, 1.0 / nl);
            let d = dot(sub(x, t[0]), nn);
            let proj = sub(x, mul(nn, d));
            let inside = (0..3).all(|i| dot(cross(sub(t[(i + 1) % 3], t[i]), sub(proj, t[i])), nn) >= -1e-6);
            if inside {
                return d.abs();
            }
        }
        (0..3)
            .map(|i| segment_segment(x, x, t[i], t[(i + 1) % 3]))
            .fold(f32::MAX, f32::min)
    };
    if nl > 1e-9 {
        let dp = dot(sub(p, t[0]), n);
        let dq = dot(sub(q, t[0]), n);
        if dp * dq < 0.0 {
            let x = add(p, mul(sub(q, p), dp / (dp - dq)));
            if point_triangle(x) < 1e-5 {
                return 0.0;
            }
        }
    }
    let mut best = point_triangle(p).min(point_triangle(q));
    for i in 0..3 {
        best = best.min(segment_segment(p, q, t[i], t[(i + 1) % 3]));
    }
    best
}

fn segment_segment(p1: V, q1: V, p2: V, q2: V) -> f32 {
    let d1 = sub(q1, p1);
    let d2 = sub(q2, p2);
    let r = sub(p1, p2);
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (s, t);
    if a <= 1e-9 && e <= 1e-9 {
        return len(r);
    }
    if a <= 1e-9 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= 1e-9 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut ss = if denom > 1e-9 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 {
                tt = 0.0;
                ss = (-c / a).clamp(0.0, 1.0);
            } else if tt > 1.0 {
                tt = 1.0;
                ss = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = ss;
            t = tt;
        }
    }
    len(sub(add(p1, mul(d1, s)), add(p2, mul(d2, t))))
}

/// Ray (downward) / triangle hit distance.
fn ray_down(origin: V, t: &[V; 3]) -> Option<(f32, f32)> {
    let dir = [0.0, -1.0, 0.0];
    let e1 = sub(t[1], t[0]);
    let e2 = sub(t[2], t[0]);
    let h = cross(dir, e2);
    let a = dot(e1, h);
    if a.abs() < 1e-9 {
        return None;
    }
    let f = 1.0 / a;
    let s = sub(origin, t[0]);
    let u = f * dot(s, h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = f * dot(dir, q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let dist = f * dot(e2, q);
    let n = cross(e1, e2);
    let slope = (n[1].abs() / len(n)).clamp(0.0, 1.0).acos().to_degrees();
    (dist >= 0.0).then_some((dist, slope))
}

const BOX_FACES: [[usize; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let db_path = args.next().context("DB")?;
    let project_path = args.next().context("PROJECT")?;
    let radius: f32 = 0.4;
    let height: f32 = 1.8;
    let mut db = ResourceDb::open(&db_path)?;
    let project = BinaryConfig::parse(&db.read(&project_path)?)?.root;
    let spawns: Vec<ConfigValue> = match project.named("server_objects").and_then(|s| s.named("respawn_points")) {
        Some(ConfigValue::Indexed(v)) => v.clone(),
        _ => anyhow::bail!("no respawn points"),
    };
    let points: Vec<V> = spawns.iter().map(|s| f3(s, "position")).collect();
    let near = |lo: V, hi: V, reach: f32| points.iter().any(|p| (0..3).all(|i| p[i] >= lo[i] - reach && p[i] <= hi[i] + reach));
    let mut triangles: Vec<([V; 3], String)> = Vec::new();
    let mut cache: BTreeMap<String, Option<(Vec<V>, Vec<[usize; 3]>, Vec<(i64, V, V, V)>)>> = BTreeMap::new();
    let Some(ConfigValue::Indexed(collisions)) = project.named("collision_objects") else { anyhow::bail!("no collision") };
    let mut blocking = 0;
    for c in collisions {
        if !(int(c, "cgroup") & 2 != 0 && int(c, "cmask") & 4 != 0) {
            continue;
        }
        blocking += 1;
        let lib = match c.named("lib_name") { Some(ConfigValue::String(s)) => s.clone(), _ => continue };
        let mut parts = lib.trim_start_matches("resources/").splitn(2, '#');
        let dir = parts.next().unwrap().to_owned();
        let scale: V = parts
            .next()
            .map(|s| {
                let v: Vec<f32> = s.split(['[', ']']).filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
                [v[0], v[1], v[2]]
            })
            .unwrap_or([1.0; 3]);
        let geom = cache.entry(dir.clone()).or_insert_with(|| {
            let mut verts = Vec::new();
            let mut tris = Vec::new();
            if let (Ok(vb), Ok(ib)) = (db.read(&format!("{dir}/vertices")), db.read(&format!("{dir}/indices"))) {
                let v = chunk(&vb, 25)?;
                let i = chunk(&ib, 26)?;
                verts = v.chunks_exact(12).map(|c| [0, 4, 8].map(|o| f32::from_le_bytes(c[o..o + 4].try_into().unwrap()))).collect();
                tris = i.chunks_exact(12).map(|c| [0, 4, 8].map(|o| u32::from_le_bytes(c[o..o + 4].try_into().unwrap()) as usize)).collect();
            }
            let mut prims = Vec::new();
            if let Ok(pb) = db.read(&format!("{dir}/exported_primitives")) {
                if let Some(ConfigValue::Indexed(list)) = BinaryConfig::parse(&pb).ok()?.root.named("primitives").cloned() {
                    for p in list {
                        prims.push((int(&p, "type"), f3(&p, "position"), f3(&p, "rotation"), f3(&p, "scale")));
                    }
                }
            }
            Some((verts, tris, prims))
        });
        let Some((verts, tris, prims)) = geom.clone() else { continue };
        let r = rotation(f3(c, "rotation"));
        let t = f3(c, "position");
        let world: Vec<V> = verts.iter().map(|p| transform(*p, &r, t, scale)).collect();
        for tri in tris {
            let w = [world[tri[0]], world[tri[1]], world[tri[2]]];
            let lo = [0, 1, 2].map(|a| w.iter().map(|p| p[a]).fold(f32::MAX, f32::min));
            let hi = [0, 1, 2].map(|a| w.iter().map(|p| p[a]).fold(f32::MIN, f32::max));
            if near(lo, hi, 3.0) {
                triangles.push((w, dir.clone()));
            }
        }
        for (kind, pos, rot, dim) in prims {
            let half: V = match kind {
                0 => [dim[0]; 3],
                1 => dim,
                2 => [dim[0], dim[1], dim[0]],
                _ => [dim[0], dim[0] + dim[1] / 2.0, dim[0]],
            };
            let pr = rotation(rot);
            let corners: Vec<V> = (0..8)
                .map(|i| {
                    let local = [if i & 4 != 0 { half[0] } else { -half[0] }, if i & 2 != 0 { half[1] } else { -half[1] }, if i & 1 != 0 { half[2] } else { -half[2] }];
                    transform(transform(local, &pr, pos, [1.0; 3]), &r, t, scale)
                })
                .collect();
            for f in BOX_FACES {
                for tri in [[f[0], f[1], f[2]], [f[0], f[2], f[3]]] {
                    let w = tri.map(|i| corners[i]);
                    let lo = [0, 1, 2].map(|a| w.iter().map(|p| p[a]).fold(f32::MAX, f32::min));
                    let hi = [0, 1, 2].map(|a| w.iter().map(|p| p[a]).fold(f32::MIN, f32::max));
                    if near(lo, hi, 3.0) {
                        triangles.push((w, format!("{dir} primitive {kind}")));
                    }
                }
            }
        }
    }
    let mut failures = 0;
    let mut rows = Vec::new();
    for (spawn, p) in spawns.iter().zip(&points) {
        let name = match spawn.named("full_name") { Some(ConfigValue::String(s)) => s.clone(), _ => String::new() };
        // Support: nearest surface below the feet within 2 m (from 0.5 m above).
        let origin = [p[0], p[1] + 0.5, p[2]];
        let support = triangles
            .iter()
            .filter_map(|(t, d)| ray_down(origin, t).map(|(dist, slope)| (dist - 0.5, slope, d)))
            .filter(|(drop, _, _)| *drop <= 2.0)
            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        // Capsule: axis from radius above the support to height - radius.
        let foot = support.map_or(p[1], |s| p[1] - s.0);
        let a = [p[0], foot + radius + 0.05, p[2]];
        let b = [p[0], foot + height - radius, p[2]];
        let clearance = triangles
            .iter()
            .map(|(t, d)| (segment_triangle_distance(a, b, t), d))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        let supported = support.is_some_and(|s| s.0.abs() <= 0.6 && s.1 < 60.0);
        let clear = clearance.is_none_or(|c| c.0 >= radius);
        if !(supported && clear) {
            failures += 1;
        }
        rows.push(serde_json::json!({
            "spawn": name,
            "team": int(spawn, "team"),
            "position": p,
            "support_drop_m": support.map(|s| s.0),
            "support_slope_deg": support.map(|s| s.1),
            "support_resource": support.map(|s| s.2.clone()),
            "capsule_clearance_m": clearance.map(|c| c.0),
            "nearest_resource": clearance.map(|c| c.1.clone()),
            "ok": supported && clear,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "db": db_path, "project": project_path, "capsule_radius": radius, "capsule_height": height,
        "blocking_collision_instances": blocking, "triangles_near_spawns": triangles.len(),
        "spawns": rows.len(), "failures": failures, "results": rows,
    }))?);
    Ok(())
}
