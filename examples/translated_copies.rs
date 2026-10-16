// SPDX-License-Identifier: GPL-3.0-or-later
//! Imported models whose render geometry equals another's up to a
//! translation (0.69d exports some repeated structures, e.g. the Duga
//! antenna sections, as separate models at their own offsets).
//!
//!     translated_copies DB [PREFIX]
use std::collections::BTreeMap;
use vostok_resource_porter::ResourceDb;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let prefix = a.get(1).cloned().unwrap_or_else(|| "models/ported/v069d/".into());
    let mut by_model: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    let paths: Vec<String> = db.paths_with_prefix(&prefix).filter(|p| p.ends_with("/converted_model") && !p.contains("srp_cluster")).map(str::to_owned).collect();
    for p in paths {
        let (model, rest) = p.split_once(".model/render/").unwrap();
        let surface = rest.trim_end_matches("/converted_model").to_owned();
        by_model.entry(model.to_owned()).or_default().push((surface, db.read(&p).unwrap()));
    }
    let mut groups: BTreeMap<Vec<u8>, Vec<(String, [f32; 3], usize)>> = BTreeMap::new();
    for (model, mut surfaces) in by_model {
        surfaces.sort();
        let lod0: Vec<&(String, Vec<u8>)> = surfaces.iter().filter(|(s, _)| !s.starts_with("lod") && !s.starts_with("srp_lod")).collect();
        let Some(first) = lod0.first() else { continue };
        let b = &first.1;
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        if u(0) != 3 || u(8) == 0 { continue }
        let stride = (u(4) - 4) / u(8);
        let origin: [f32; 3] = std::array::from_fn(|i| f32::from_le_bytes(b[12 + 4 * i..16 + 4 * i].try_into().unwrap()));
        let mut key = Vec::new();
        let mut bytes = 0;
        for (name, m) in &lod0 {
            key.extend(name.as_bytes());
            let u = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap()) as usize;
            if u(0) != 3 { continue }
            let (size, count) = (u(4), u(8));
            for v in m[12..8 + size].chunks_exact(stride) {
                for i in 0..3 {
                    let x = f32::from_le_bytes(v[4 * i..4 * i + 4].try_into().unwrap()) - origin[i];
                    key.extend(((x * 1000.0).round() as i32).to_le_bytes());
                }
                key.extend(&v[12..]);
            }
            key.extend(&m[8 + size..]);
            bytes += m.len();
            let _ = count;
        }
        groups.entry(key).or_default().push((model, origin, bytes));
    }
    let mut saved = 0;
    for members in groups.values().filter(|g| g.len() > 1) {
        saved += members[0].2 * (members.len() - 1);
        println!("{} copies, {:.2} MiB LOD0 each: {}", members.len(), members[0].2 as f64 / 1048576.0,
                 members.iter().map(|(m, o, _)| format!("{} @[{:.2},{:.2},{:.2}]", m.trim_start_matches(&prefix), o[0], o[1], o[2])).collect::<Vec<_>>().join(", "));
    }
    println!("LOD0 bytes in duplicate copies: {:.1} MiB", saved as f64 / 1048576.0);
}
