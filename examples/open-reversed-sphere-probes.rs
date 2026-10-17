//! Open the reversed 0.26 falloff interval of spherical environment probes in
//! the client projects of a content pack, in place (see
//! `survarium_026::open_reversed_sphere_probes`). The converter does it itself
//! with the manifest's `open_reversed_sphere_probes`; this is for a pack
//! already built.
//!
//!     cargo run --release --example open-reversed-sphere-probes -- PACK_DIR
use std::path::Path;

use vostok_resource_porter::BinaryConfig;

fn main() -> anyhow::Result<()> {
    let pack = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: PACK_DIR"))?;
    let projects = Path::new(&pack).join("files/projects");
    let mut changed = 0;
    for level in std::fs::read_dir(&projects)? {
        let level = level?.path();
        for variant in std::fs::read_dir(&level)? {
            let path = variant?.path().join("client_project");
            if !path.is_file() {
                continue;
            }
            let mut project = BinaryConfig::parse(&std::fs::read(&path)?)?;
            let notes = vostok_resource_porter::survarium_026::open_reversed_sphere_probes(&mut project.root)?;
            if !notes.is_empty() {
                std::fs::write(&path, project.to_bytes()?)?;
                changed += 1;
                for note in notes {
                    println!("{}: {note}", path.strip_prefix(&projects)?.display());
                }
            }
        }
    }
    println!("projects changed: {changed}");
    Ok(())
}
