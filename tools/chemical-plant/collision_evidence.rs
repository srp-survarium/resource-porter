//! Extract only collision payloads and the project for a bounded offline audit.
use std::{env, fs, path::Path};
use vostok_resource_porter::ResourceDb;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = env::args().collect();
    if a.len() < 5 {
        return Err(
            "usage: collision_evidence DB MODEL_PREFIX PROJECT OUTPUT [EXTRA_PREFIX ...]".into(),
        );
    }
    let root = Path::new(&a[4]);
    if root.exists() {
        return Err("output already exists".into());
    }
    let mut db = ResourceDb::open(&a[1])?;
    let paths: Vec<_> = db
        .entries()
        .keys()
        .filter(|p| {
            db.is_file(p)
                && (*p == &a[3]
                    || a[5..].iter().any(|prefix| p.starts_with(prefix))
                    || (p.starts_with(&a[2])
                        && (p.contains("/collision/")
                            || p.contains("/walker_collision/")
                            || p.contains("/factory_boiler.model/")
                            || p.ends_with("/render/export_properties"))))
        })
        .cloned()
        .collect();
    let mut total = 0;
    for p in &paths {
        let dest = root.join(p);
        fs::create_dir_all(dest.parent().ok_or("missing parent")?)?;
        let b = db.read(p)?;
        total += b.len();
        fs::write(dest, b)?;
    }
    println!("{} files, {} bytes", paths.len(), total);
    Ok(())
}
