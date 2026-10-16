// Rendered-triangle and draw estimate at the spawns of a project, using the
// 0.26 LOD rule (render_model_static.cpp select_lod: distance 20/40/240 m or
// projected size 0.4/0.2/0.001, x0.75 at low geometry quality not applied).
// Omnidirectional (no frustum or occlusion culling): an upper bound that is
// comparable between maps.
use std::collections::BTreeMap;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn u32_at(b: &[u8], o: usize) -> usize { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize }
fn s<'a>(c: &'a ConfigValue, k: &str) -> Option<&'a str> { match c.named(k) { Some(ConfigValue::String(s)) => Some(s), _ => None } }
fn f3(c: &ConfigValue, k: &str) -> [f32; 3] { match c.named(k) { Some(ConfigValue::Float3(v)) => *v, _ => [0.0; 3] } }
fn num(c: &ConfigValue, k: &str) -> Option<f32> { match c.named(k)? { ConfigValue::Float(v) => Some(*v), ConfigValue::Integer(v) => Some(*v as f32), _ => None } }
fn arr(c: Option<&ConfigValue>) -> &[ConfigValue] { match c { Some(ConfigValue::Indexed(v)) => v, _ => &[] } }
fn surface_radius(db: &mut ResourceDb, path: &str) -> f32 {
    let Ok(b) = db.read(path) else { return 0.0 };
    let p = BinaryConfig::parse(&b).unwrap().root;
    let Some(bb) = p.named("bounding_box") else { return 0.0 };
    let (lo, hi) = (f3(bb, "min"), f3(bb, "max"));
    ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0
}
struct Model { lods: [(usize, usize); 4], center: [f32; 3], radius: f32, kind: u8, params: Option<[f32; 3]> }
fn main() {
    let mut a = std::env::args().skip(1);
    let mut db = ResourceDb::open(a.next().unwrap()).unwrap();
    let project = a.next().unwrap();
    let root = BinaryConfig::parse(&db.read(&project).unwrap()).unwrap().root;
    let mut models: BTreeMap<String, Model> = BTreeMap::new();
    let visuals = arr(root.named("render_visuals"));
    for v in visuals {
        let lib = s(v, "lib_name").unwrap().replace('\\', "/");
        if models.contains_key(&lib) { continue }
        let m = format!("models/{lib}.model");
        let c = BinaryConfig::parse(&db.read(&format!("{m}/settings")).unwrap()).unwrap().root;
        let mut lods = [(0, 0); 4];
        let mut far: Vec<(usize, f32)> = Vec::new();
        for (i, lod) in ["LOD0", "LOD1", "LOD2", "LOD3"].iter().enumerate() {
            if let Some(l) = c.named("lod_hierrarchy").and_then(|h| h.named(lod)) {
                for x in arr(l.named("surfaces")) { if let ConfigValue::String(n) = x {
                    if let Ok(g) = db.read(&format!("{m}/render/{n}/converted_model")) { let size = u32_at(&g, 4); if g.len() > 8 + size + 12 {
                        lods[i].0 += u32_at(&g, 8 + size + 8) / 3; lods[i].1 += 1;
                        if i == 2 { far.push((u32_at(&g, 8 + size + 8) / 3, surface_radius(&mut db, &format!("{m}/render/{n}/export_properties")))); }
                    } }
                } }
            }
        }
        let p = BinaryConfig::parse(&db.read(&format!("{m}/render/export_properties")).unwrap()).unwrap().root;
        let bb = p.named("bounding_box").unwrap();
        let (lo, hi) = (f3(bb, "min"), f3(bb, "max"));
        let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0;
        let sw = c.named("lod_switching");
        let kind = sw.and_then(|w| num(w, "type")).unwrap_or(0.0) as u8;
        let default = sw.map(|w| !matches!(w.named("default_params"), Some(ConfigValue::Boolean(false)))).unwrap_or(true);
        let params = (!default).then(|| [num(sw.unwrap(), "param0").unwrap_or(0.0), num(sw.unwrap(), "param1").unwrap_or(0.0), num(sw.unwrap(), "param2").unwrap_or(0.0)]);
        let (mut kind, mut params) = (kind, params);
        if let Ok(o) = std::env::var("FORCE") {
            if lib.starts_with("ported/") && lods[1].1 > 0 {
                let v: Vec<f32> = o.split(',').map(|x| x.parse().unwrap()).collect();
                kind = 0; params = Some([v[0], v[1], v[2]]);
            }
        }
        // CULLK=k,min: imported models with LODs stop drawing beyond k * radius
        // (at least min m), the builder's `cull_radius_factor`.
        if let Ok(o) = std::env::var("CULLK") {
            if lib.starts_with("ported/") && lods[1].1 > 0 && kind == 0 {
                let v: Vec<f32> = o.split(',').map(|x| x.parse().unwrap()).collect();
                if let Some(p) = params.as_mut() { p[2] = p[2].min((v[0] * radius).max(v[1])); }
            }
        }
        // LOD3SIM=D,T: imported models with LODs get a LOD3 from D m on that
        // keeps only the LOD2 surfaces of radius >= T m (the builder's `far_lod`).
        if let Ok(o) = std::env::var("LOD3SIM") {
            if lib.starts_with("ported/") && lods[1].1 > 0 && kind == 0 && lods[3].1 == 0 {
                let v: Vec<f32> = o.split(',').map(|x| x.parse().unwrap()).collect();
                if let Some(p) = params.as_mut() { p[2] = p[2].min(v[0]); }
                for (t, r) in &far { if *r >= v[1] { lods[3].0 += t; lods[3].1 += 1; } }
            }
        }
        if std::env::var("TERRAIN4").is_ok() && lib.contains("landscape/segment") { lods[0].0 /= 4; }
        models.insert(lib, Model { lods, center, radius, kind, params });
    }
    let spawns = arr(root.named("server_objects").and_then(|s| s.named("respawn_points")));
    let tan = (60f32.to_radians() / 2.0).tan();
    let (mut worst, mut sum, mut worst_draws, mut sum_draws) = (0usize, 0usize, 0usize, 0usize);
    // Per chosen LOD, split into imported (ported/) and native models: (tris, draws).
    let mut split = [[(0usize, 0usize); 4]; 2];
    let mut contrib: BTreeMap<String, (usize, usize, u8, Option<[f32; 3]>, [usize; 4])> = BTreeMap::new();
    for sp in spawns {
        let eye = f3(sp, "position"); let eye = [eye[0], eye[1] + 1.7, eye[2]];
        let (mut t, mut d) = (0, 0);
        for v in visuals {
            let m = &models[&s(v, "lib_name").unwrap().replace('\\', "/")];
            let pos = f3(v, "position"); let sc = f3(v, "scale");
            // Rotation ignored for the centre offset of the bounding box (upper-bound-neutral).
            let c = [pos[0] + m.center[0] * sc[0], pos[1] + m.center[1] * sc[1], pos[2] + m.center[2] * sc[2]];
            let r = m.radius * sc.iter().cloned().fold(0.0, f32::max);
            let dist = ((c[0] - eye[0]).powi(2) + (c[1] - eye[1]).powi(2) + (c[2] - eye[2]).powi(2)).sqrt();
            let lod = if m.lods[1].1 == 0 { 0 } else if m.kind == 1 {
                let p = m.params.unwrap_or([0.4, 0.2, 0.001]);
                let size = r / (dist.max(0.1) * tan);
                if size < p[2] { 3 } else if size < p[1] { 2 } else if size < p[0] { 1 } else { 0 }
            } else {
                let p = m.params.unwrap_or([20.0, 40.0, 240.0]);
                if dist < p[0] { 0 } else if dist < p[1] { 1 } else if dist < p[2] { 2 } else { 3 }
            };
            t += m.lods[lod].0; d += m.lods[lod].1;
            let side = &mut split[s(v, "lib_name").unwrap().starts_with("ported/") as usize][lod];
            side.0 += m.lods[lod].0; side.1 += m.lods[lod].1;
            let e = contrib.entry(s(v, "lib_name").unwrap().to_owned()).or_insert((0usize, 0usize, m.kind, m.params, [0usize; 4]));
            e.0 += m.lods[lod].0; e.1 += m.lods[lod].1; e.4[lod] += 1;
        }
        worst = worst.max(t); sum += t; worst_draws = worst_draws.max(d); sum_draws += d;
    }
    let n = spawns.len().max(1);
    if std::env::var("TOP").is_ok() {
        let mut v: Vec<_> = contrib.iter().collect(); v.sort_by_key(|x| std::cmp::Reverse(x.1.0));
        for (k, (t, d, kind, p, lods)) in v.iter().take(std::env::var("TOP").unwrap().parse().unwrap_or(30)) { eprintln!("{}\t{}\ttype{} {:?} lods {:?}\t{}", t / n, d / n, kind, p, lods, k); }
        let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
        for (k, (t, _, kind, p, _)) in &contrib { *by_kind.entry(format!("type{} custom={}", kind, p.is_some())).or_default() += t / n; let _ = k; }
        eprintln!("{by_kind:?}");
    }
    if std::env::var("SPLIT").is_ok() {
        for (i, side) in ["native", "imported"].iter().enumerate() {
            eprintln!("{side}: per LOD (tris, draws) per spawn {:?}", split[i].map(|(t, d)| (t / n, d / n)));
        }
    }
    println!("{project}: spawns {} mean_tris {} worst_tris {} mean_draws {} worst_draws {}", spawns.len(), sum / n, worst, sum_draws / n, worst_draws);
}
