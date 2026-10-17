// SPDX-License-Identifier: GPL-3.0-or-later
//! Static render meshes of a project's render instances with suspicious
//! data: non-finite or far positions, indices outside the vertex buffer,
//! vertices outside the surface's export_properties bounds, a stride the
//! static formats do not have or that disagrees with the surface `type`, or
//! stale `source` counts.
//!
//!     vertex_sanity DB PROJECT
use std::collections::BTreeSet;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn f3(c: &ConfigValue, k: &str) -> Option<[f32; 3]> { match c.named(k) { Some(ConfigValue::Float3(v)) => Some(*v), _ => None } }
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let root = BinaryConfig::parse(&db.read(&a[1]).unwrap()).unwrap().root;
    let Some(ConfigValue::Indexed(visuals)) = root.named("render_visuals") else { return };
    let models: BTreeSet<String> = visuals.iter().filter_map(|v| match v.named("lib_name") { Some(ConfigValue::String(s)) => Some(s.replace('\\', "/")), _ => None }).collect();
    let mut bad = 0;
    for m in &models {
        let prefix = format!("models/{m}.model/render/");
        let paths: Vec<String> = db.paths_with_prefix(&prefix).filter(|p| p.ends_with("/converted_model")).map(str::to_owned).collect();
        for p in paths {
            let b = db.read(&p).unwrap();
            let u = |o: usize| b.get(o..o + 4).map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
            if u(0) != 3 { println!("{p}: not a static mesh (chunk {})", u(0)); bad += 1; continue }
            let (size, count) = (u(4), u(8));
            if count == 0 { println!("{p}: no vertices"); bad += 1; continue }
            let stride = (size - 4) / count;
            let mut issues = Vec::new();
            if (size - 4) % count != 0 || !matches!(stride, 28 | 32) { issues.push(format!("stride {stride}")); }
            let at = 8 + size;
            if u(at) != 4 { issues.push("no index chunk".into()); }
            let n = u(at + 8);
            let properties = BinaryConfig::parse(&db.read(&p.replace("/converted_model", "/export_properties")).unwrap_or_default()).ok();
            let bounds = properties.as_ref().and_then(|c| c.root.named("bounding_box").and_then(|bb| Some((f3(bb, "min")?, f3(bb, "max")?))));
            // 0.26 reads the vertex layout from the surface's `type`: 2 has a colour (32 bytes).
            if let Some(ConfigValue::Integer(t)) = properties.as_ref().and_then(|c| c.root.named("type")) {
                if (*t == 2) != (stride == 32) { issues.push(format!("type {t} with stride {stride}")); }
            }
            if let Some(ConfigValue::Integer(v)) = properties.as_ref().and_then(|c| c.root.named("source")).and_then(|s| s.named("vertices")) {
                if *v as usize != count { issues.push(format!("source.vertices {v} for {count} vertices")); }
            }
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for k in 0..count {
                for i in 0..3 {
                    let x = f32::from_le_bytes(b[12 + k * stride + 4 * i..16 + k * stride + 4 * i].try_into().unwrap());
                    if !x.is_finite() || x.abs() > 5000.0 { issues.push(format!("vertex {k} coordinate {x}")); break }
                    lo[i] = lo[i].min(x); hi[i] = hi[i].max(x);
                }
                if issues.len() > 3 { break }
            }
            let max_index = (0..n).map(|k| u16::from_le_bytes([b[at + 12 + 2 * k], b[at + 13 + 2 * k]]) as usize).max().unwrap_or(0);
            if max_index >= count { issues.push(format!("index {max_index} >= {count} vertices")); }
            if let Some((bl, bh)) = bounds {
                let slack = 0.05 * (0..3).map(|i| bh[i] - bl[i]).fold(1.0f32, f32::max);
                if (0..3).any(|i| lo[i] < bl[i] - slack || hi[i] > bh[i] + slack) { issues.push(format!("vertices {lo:?}..{hi:?} outside bounds {bl:?}..{bh:?}")); }
            } else { issues.push("no bounds".into()); }
            if !issues.is_empty() { bad += 1; if bad < 60 { println!("{p}: {}", issues.join("; ")); } }
        }
    }
    println!("{bad} suspicious surfaces in {} models", models.len());
}
