// SPDX-License-Identifier: GPL-3.0-or-later
//! Bisecting runtime artefacts: a bundle replacing one client project with a
//! copy that drops the render instances whose lib_name contains any of the
//! given substrings (comma list; prefix with `!` to keep only those).
//!
//!     project_filter DB PROJECT OUT_BUNDLE PATTERNS
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (db, project, out) = (&a[0], &a[1], std::path::Path::new(&a[2]));
    let (keep, patterns) = match a[3].strip_prefix('!') { Some(p) => (true, p), None => (false, a[3].as_str()) };
    let patterns: Vec<&str> = patterns.split(',').collect();
    let mut db = ResourceDb::open(db).unwrap();
    let mut root = BinaryConfig::parse(&db.read(project).unwrap()).unwrap().root;
    let Some(ConfigValue::Indexed(v)) = root.named_mut("render_visuals") else { panic!() };
    let before = v.len();
    v.retain(|x| {
        let lib = match x.named("lib_name") { Some(ConfigValue::String(s)) => s.as_str(), _ => "" };
        patterns.iter().any(|p| lib.contains(p)) == keep
    });
    eprintln!("render instances {before} -> {}", v.len());
    let path = out.join("overlay").join(project);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = BinaryConfig { root }.to_bytes().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let rec = serde_json::json!({"files": [{"path": project, "source": "bisect filter", "size": bytes.len(),
        "sha256": format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes))}]});
    std::fs::write(out.join("port-manifest.json"), serde_json::to_string_pretty(&rec).unwrap()).unwrap();
}
