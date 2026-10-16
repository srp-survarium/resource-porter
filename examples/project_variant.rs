// Write a bundle replacing one client project with a reduced copy, for
// bisecting load failures: KEEP_TYPES (comma list of game_object_type to keep
// in client_objects, or "all"), VISUALS (max render instances kept, or -1),
// COLLISIONS (max collision instances, or -1), PROBES_ONLY_GLOBAL (0/1).
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (db, project, out) = (&a[0], &a[1], std::path::Path::new(&a[2]));
    let keep: Vec<String> = a[3].split(',').map(str::to_owned).collect();
    let visuals: i64 = a[4].parse().unwrap();
    let collisions: i64 = a[5].parse().unwrap();
    let mut db = ResourceDb::open(db).unwrap();
    let mut root = BinaryConfig::parse(&db.read(project).unwrap()).unwrap().root;
    if keep[0] != "all" {
        if let Some(ConfigValue::Indexed(objects)) = root.named_mut("client_objects") {
            objects.retain(|o| matches!(o.named("game_object_type"), Some(ConfigValue::String(t)) if keep.contains(t)));
        }
    }
    for (group, limit) in [("render_visuals", visuals), ("collision_objects", collisions)] {
        if limit >= 0 {
            if let Some(ConfigValue::Indexed(v)) = root.named_mut(group) { v.truncate(limit as usize); }
        }
    }
    let path = out.join("overlay").join(project);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = BinaryConfig { root }.to_bytes().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let rec = serde_json::json!({"files": [{"path": project, "source": "bisect variant", "size": bytes.len(),
        "sha256": format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes))}]});
    std::fs::write(out.join("port-manifest.json"), serde_json::to_string_pretty(&rec).unwrap()).unwrap();
}
