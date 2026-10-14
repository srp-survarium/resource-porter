//! Print the FourCC of every DDS under a prefix: dds_formats DB PREFIX
use std::env;
use vostok_resource_porter::ResourceDb;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut db = ResourceDb::open(&args[0])?;
    let paths: Vec<String> = db
        .paths_with_prefix(&args[1])
        .filter(|p| p.ends_with(".dds"))
        .map(str::to_owned)
        .collect();
    for path in paths {
        if !db.is_file(&path) {
            continue;
        }
        let bytes = db.read(&path)?;
        let fourcc = bytes
            .get(84..88)
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        println!("{fourcc}\t{path}");
    }
    Ok(())
}
