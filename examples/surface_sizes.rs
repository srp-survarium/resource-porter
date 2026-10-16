// SPDX-License-Identifier: GPL-3.0-or-later
//! Largest static render surfaces under a prefix: vertex count, stride and
//! index count (0.26 loads a surface's buffers through the main thread's
//! stack, so oversized merged surfaces overflow it).
//!
//!     surface_sizes DB [PREFIX] [TOP]
use vostok_resource_porter::ResourceDb;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let prefix = a.get(1).cloned().unwrap_or_else(|| "models/".into());
    let top: usize = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(15);
    let paths: Vec<String> = db.paths_with_prefix(&prefix).filter(|p| p.ends_with("/converted_model")).map(str::to_owned).collect();
    let mut rows = Vec::new();
    for p in &paths {
        let Ok(b) = db.read(p) else { continue };
        let u = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
        let (Some(3), Some(size), Some(count)) = (u(0), u(4), u(8)) else { continue };
        if count == 0 { continue }
        let indices = u(8 + size + 8).unwrap_or(0);
        rows.push((size - 4, count, (size - 4) / count, indices, p.clone()));
    }
    rows.sort_by(|x, y| y.0.cmp(&x.0));
    println!("{} static surfaces under {prefix}", rows.len());
    for (bytes, count, stride, indices, p) in rows.iter().take(top) {
        println!("{bytes}\t{count} v\tstride {stride}\t{indices} i\t{p}");
    }
}
