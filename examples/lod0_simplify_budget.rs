// SPDX-License-Identifier: GPL-3.0-or-later
//! What error-bounded LOD0 simplification would save: simplifies every
//! LOD0 surface of the imported models below a prefix and sums the vertex
//! and index bytes before and after.
//!
//!     lod0_simplify_budget DB PREFIX RATIO MAX_ERROR [MIN_TRIANGLES]
use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::survarium_026::later::simplify_mesh;
fn bytes(m: &[u8]) -> usize {
    let u = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap()) as usize;
    u(4) - 4 + u(8 + u(4) + 8) * 2
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let (ratio, error): (f32, f32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let min_tris: usize = a.get(4).map(|s| s.parse().unwrap()).unwrap_or(500);
    let paths: Vec<String> = db.paths_with_prefix(&a[1]).filter(|p| p.ends_with("/converted_model") && !p.contains("srp_cluster") && !p.contains("/landscape/")).map(str::to_owned).collect();
    let (mut before, mut after, mut n) = (0usize, 0usize, 0usize);
    for p in paths {
        let surface = p.rsplit('/').nth(1).unwrap();
        if surface.starts_with("lod") || surface.starts_with("srp_lod") { continue }
        let m = db.read(&p).unwrap();
        if u32::from_le_bytes(m[0..4].try_into().unwrap()) != 3 { continue }
        let u = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap()) as usize;
        let tris = u(8 + u(4) + 8) / 3;
        before += bytes(&m);
        if tris < min_tris { after += bytes(&m); continue }
        match simplify_mesh(&m, ratio, error) {
            Ok(Some(s)) => { after += bytes(&s); n += 1 }
            _ => after += bytes(&m),
        }
    }
    println!("ratio {ratio} error {error}: LOD0 {:.1} MiB -> {:.1} MiB ({n} surfaces simplified)", before as f64 / 1048576.0, after as f64 / 1048576.0);
}
