// Static render budget of a client project: LOD0 triangles and surface
// draws over all render instances, unique textures and their payload bytes,
// vegetation instances, decals, probes, lights and particles.
use std::collections::{BTreeMap, BTreeSet};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn u32_at(b: &[u8], o: usize) -> usize { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize }
fn s<'a>(c: &'a ConfigValue, k: &str) -> Option<&'a str> { match c.named(k) { Some(ConfigValue::String(s)) => Some(s), _ => None } }
fn arr(c: Option<&ConfigValue>) -> &[ConfigValue] { match c { Some(ConfigValue::Indexed(v)) => v, _ => &[] } }
fn tex_fields(c: &ConfigValue, out: &mut BTreeSet<String>) {
    match c {
        ConfigValue::Named(v) => for f in v {
            if f.name.starts_with("texture_") || f.name.ends_with("_texture") {
                if let Some(ConfigValue::String(t)) = Some(f.value.named("value").unwrap_or(&f.value)) { if !t.is_empty() { out.insert(t.replace('\\', "/")); } }
            } else { tex_fields(&f.value, out) }
        },
        ConfigValue::Indexed(v) => for f in v { tex_fields(f, out) },
        _ => {}
    }
}
fn main() {
    let mut a = std::env::args().skip(1);
    let mut db = ResourceDb::open(a.next().unwrap()).unwrap();
    let project = a.next().unwrap();
    let root = BinaryConfig::parse(&db.read(&project).unwrap()).unwrap().root;
    let mut model_cache: BTreeMap<String, (usize, usize, BTreeSet<String>)> = BTreeMap::new();
    let (mut tris, mut draws, mut instances) = (0usize, 0usize, 0usize);
    let mut materials = BTreeSet::new();
    let mut missing = 0;
    let mut per_model: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for v in arr(root.named("render_visuals")) {
        let lib = s(v, "lib_name").unwrap_or("").replace('\\', "/");
        let model = format!("models/{lib}.model");
        let entry = model_cache.entry(model.clone()).or_insert_with(|| {
            let Ok(b) = db.read(&format!("{model}/settings")) else { return (usize::MAX, 0, BTreeSet::new()) };
            let c = BinaryConfig::parse(&b).unwrap().root;
            let lod0: Vec<String> = c.named("lod_hierrarchy").and_then(|l| l.named("LOD0")).map(|l| arr(l.named("surfaces")).iter().filter_map(|x| if let ConfigValue::String(s) = x { Some(s.clone()) } else { None }).collect()).unwrap_or_default();
            let mut t = 0; let mut mats = BTreeSet::new();
            for surface in &lod0 {
                if let Ok(g) = db.read(&format!("{model}/render/{surface}/converted_model")) {
                    let size = u32_at(&g, 4);
                    if g.len() > 8 + size + 12 { t += u32_at(&g, 8 + size + 8) / 3; }
                }
                if let Some(m) = c.named("material_settings").and_then(|m| m.named(surface)).and_then(|e| s(e, "material_name")) { mats.insert(m.replace('\\', "/")); }
            }
            (t, lod0.len(), mats)
        });
        if entry.0 == usize::MAX { missing += 1; continue; }
        tris += entry.0; draws += entry.1; instances += 1; materials.extend(entry.2.iter().cloned());
        *per_model.entry(model.clone()).or_insert((0, 0)) = { let e = per_model.get(&model).copied().unwrap_or((0, 0)); (e.0 + entry.0, e.1 + 1) };
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut veg_files = Vec::new();
    for o in arr(root.named("client_objects")) {
        let k = s(o, "game_object_type").unwrap_or("?").to_owned();
        if k == "decal" { if let Some(m) = s(o, "decal_material") { materials.insert(m.replace('\\', "/")); } }
        if k == "vegetation" { for l in arr(o.named("layers")) { if let Some(f) = s(l, "filename") { veg_files.push(f.replace('\\', "/")); } } }
        *counts.entry(k).or_default() += 1;
    }
    let mut textures = BTreeSet::new();
    for m in &materials { if let Ok(b) = db.read(&format!("material_instances/{m}.material")) { tex_fields(&BinaryConfig::parse(&b).unwrap().root, &mut textures); } }
    let mut tex_bytes = 0usize; let mut tex_found = 0;
    for t in &textures { let p = format!("textures/{t}.dds"); if db.is_file(&p) { tex_bytes += db.entries()[&p].size_in_db as usize; tex_found += 1; } }
    let level_dir = project.trim_end_matches("/client_project").rsplit_once('/').unwrap().0.to_owned();
    let mut veg_instances = 0usize;
    for f in &veg_files {
        let Ok(b) = db.read(&format!("{level_dir}/{f}")) else { continue };
        let mut o = 0;
        while o + 8 <= b.len() { let id = u32_at(&b, o); let size = u32_at(&b, o + 4); if id == 0x58 { veg_instances += size / 12; } o += 8 + size; }
    }
    if std::env::var("TOP").is_ok() { let mut v: Vec<_> = per_model.iter().collect(); v.sort_by_key(|x| std::cmp::Reverse(x.1.0)); for (m, (t, n)) in v.iter().take(40) { eprintln!("{t}\t{n}\t{m}"); } }
    println!("{{\"project\":\"{project}\",\"render_instances\":{instances},\"missing_models\":{missing},\"unique_models\":{},\"lod0_triangles\":{tris},\"lod0_surface_draws\":{draws},\"materials\":{},\"textures\":{tex_found},\"texture_mib\":{:.1},\"vegetation_instances\":{veg_instances},\"objects\":{counts:?}}}",
        model_cache.len(), materials.len(), tex_bytes as f64 / 1048576.0);
}
