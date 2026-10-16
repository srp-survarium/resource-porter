// SPDX-License-Identifier: GPL-3.0-or-later
//! Surfaces per LOD against distinct materials per LOD for the render
//! instances of a project: how many draws merging same-material surfaces
//! would save, weighted by instance count.
//!
//! With CELL=m, also how many LOD2 draws merging same-material surfaces
//! across the instances of each m-sized cell would leave.
//!
//!     lod_surfaces DB PROJECT [PREFIX]
use std::collections::{BTreeMap, BTreeSet};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn arr(c: Option<&ConfigValue>) -> &[ConfigValue] { match c { Some(ConfigValue::Indexed(v)) => v, _ => &[] } }
fn main() {
    let mut a = std::env::args().skip(1);
    let mut db = ResourceDb::open(a.next().unwrap()).unwrap();
    let project = a.next().unwrap();
    let prefix = a.next().unwrap_or_default();
    let root = BinaryConfig::parse(&db.read(&project).unwrap()).unwrap().root;
    let mut instances: BTreeMap<String, usize> = BTreeMap::new();
    let mut placed: Vec<(String, [f32; 3])> = Vec::new();
    for v in arr(root.named("render_visuals")) {
        if let Some(ConfigValue::String(l)) = v.named("lib_name") {
            if l.starts_with(&prefix) {
                *instances.entry(l.replace('\\', "/")).or_default() += 1;
                if let Some(ConfigValue::Float3(p)) = v.named("position") { placed.push((l.replace('\\', "/"), *p)); }
            }
        }
    }
    let mut lod2_materials: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut surfaces, mut materials) = ([0usize; 4], [0usize; 4]);
    let mut rows = Vec::new();
    for (lib, n) in &instances {
        let Ok(bytes) = db.read(&format!("models/{lib}.model/settings")) else { continue };
        let c = BinaryConfig::parse(&bytes).unwrap().root;
        let mat = |surface: &str| -> String {
            match c.named("material_settings").and_then(|m| m.named(surface)).and_then(|e| e.named("material_name")) {
                Some(ConfigValue::String(s)) => s.clone(),
                _ => format!("?{surface}"),
            }
        };
        let mut row = [(0usize, 0usize); 4];
        for (i, lod) in ["LOD0", "LOD1", "LOD2", "LOD3"].iter().enumerate() {
            let Some(l) = c.named("lod_hierrarchy").and_then(|h| h.named(lod)) else { continue };
            let names: Vec<String> = arr(l.named("surfaces")).iter().filter_map(|x| match x { ConfigValue::String(s) => Some(s.clone()), _ => None }).collect();
            let distinct: BTreeSet<String> = names.iter().map(|s| mat(s)).collect();
            row[i] = (names.len(), distinct.len());
            if i == 2 || (i == 0 && c.named("lod_hierrarchy").and_then(|h| h.named("LOD1")).is_none()) {
                lod2_materials.insert(lib.clone(), names.iter().map(|s| mat(s)).collect());
            }
            surfaces[i] += names.len() * n;
            materials[i] += distinct.len() * n;
        }
        rows.push((n * (row[2].0 - row[2].1), *n, row, lib.clone()));
    }
    rows.sort_by(|x, y| y.0.cmp(&x.0));
    for (saved, n, row, lib) in rows.iter().take(25) { eprintln!("{saved}\t{n} inst\t{row:?}\t{lib}"); }
    if let Ok(cell) = std::env::var("CELL") {
        let cell: f32 = cell.parse().unwrap();
        let mut cells: BTreeMap<(i32, i32), (usize, BTreeSet<String>)> = BTreeMap::new();
        for (lib, p) in &placed {
            let Some(m) = lod2_materials.get(lib) else { continue };
            let e = cells.entry(((p[0] / cell).floor() as i32, (p[2] / cell).floor() as i32)).or_default();
            e.0 += m.len();
            e.1.extend(m.iter().cloned());
        }
        let (before, after) = cells.values().fold((0, 0), |(b, a), (n, set)| (b + n, a + set.len()));
        println!("cell {cell} m: far draws {before} -> {after} merged by material");
    }
    println!("instance-weighted surfaces per LOD {surfaces:?}; distinct materials per LOD {materials:?}");
}
