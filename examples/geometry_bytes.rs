// SPDX-License-Identifier: GPL-3.0-or-later
//! Static render geometry a project loads: the vertex and index bytes of
//! every surface of every distinct render model (all LODs), against 0.26's
//! fixed geometry pool (108 MB in client logs).
//!
//!     geometry_bytes DB PROJECT
use std::collections::BTreeSet;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let root = BinaryConfig::parse(&db.read(&a[1]).unwrap()).unwrap().root;
    let Some(ConfigValue::Indexed(visuals)) = root.named("render_visuals") else { return };
    let models: BTreeSet<String> = visuals.iter().filter_map(|v| match v.named("lib_name") { Some(ConfigValue::String(s)) => Some(s.replace('\\', "/")), _ => None }).collect();
    let (mut vertex, mut index, mut surfaces) = (0usize, 0usize, 0usize);
    let mut per_model: Vec<(usize, String)> = Vec::new();
    for m in &models {
        let before = vertex + index;
        let prefix = format!("models/{m}.model/render/");
        let paths: Vec<String> = db.paths_with_prefix(&prefix).filter(|p| p.ends_with("/converted_model")).map(str::to_owned).collect();
        for p in paths {
            let Ok(b) = db.read(&p) else { continue };
            let u = |o: usize| b.get(o..o + 4).map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
            if u(0) != 3 { continue }
            vertex += u(4) - 4;
            index += u(8 + u(4) + 8) * 2;
            surfaces += 1;
        }
        per_model.push((vertex + index - before, m.clone()));
    }
    if let Ok(n) = std::env::var("TOP") {
        per_model.sort_by(|x, y| y.0.cmp(&x.0));
        for (bytes, m) in per_model.iter().take(n.parse().unwrap()) {
            eprintln!("{:.2} MiB\t{m}", *bytes as f64 / 1048576.0);
        }
    }
    println!("{}: {} models, {surfaces} surfaces, vertex {:.1} MiB, index {:.1} MiB, total {:.1} MiB", a[1], models.len(), vertex as f64 / 1048576.0, index as f64 / 1048576.0, (vertex + index) as f64 / 1048576.0);
}
