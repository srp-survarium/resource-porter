//! Lighting fixes of the v0.11 maps on a content pack already built, in
//! place: the reversed 0.26 falloff interval of spherical environment probes
//! is opened (`survarium_026::open_reversed_sphere_probes`), and, given the
//! port manifest, each map's reviewed `probe_overrides` are applied to its
//! projects (`projects/<prefix>_<level>/*`). The converter does both itself.
//!
//!     cargo run --release --example open-reversed-sphere-probes -- PACK_DIR [MANIFEST]
use std::path::Path;

use anyhow::Context;
use vostok_resource_porter::BinaryConfig;
use vostok_resource_porter::survarium_026::{Manifest, apply_probe_overrides, open_reversed_sphere_probes};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let pack = args.next().context("usage: PACK_DIR [MANIFEST]")?;
    let manifest: Option<Manifest> = match args.next() {
        Some(path) => Some(serde_json::from_reader(std::fs::File::open(&path)?)?),
        None => None,
    };
    let projects = Path::new(&pack).join("files/projects");
    let mut changed = 0;
    let mut overridden = std::collections::BTreeSet::new();
    for level in std::fs::read_dir(&projects)? {
        let level = level?.path();
        let level_name = level.file_name().unwrap().to_string_lossy().into_owned();
        for variant in std::fs::read_dir(&level)? {
            let path = variant?.path().join("client_project");
            if !path.is_file() {
                continue;
            }
            let mut project = BinaryConfig::parse(&std::fs::read(&path)?)?;
            let mut notes = Vec::new();
            for map in manifest.iter().flat_map(|m| &m.maps) {
                if level_name.ends_with(&format!("_{}", map.level)) && !map.probe_overrides.is_empty() {
                    notes.extend(apply_probe_overrides(&mut project.root, &map.probe_overrides)?);
                    overridden.insert(map.level.clone());
                }
            }
            notes.extend(open_reversed_sphere_probes(&mut project.root)?);
            if !notes.is_empty() {
                std::fs::write(&path, project.to_bytes()?)?;
                changed += 1;
                for note in notes {
                    println!("{}: {note}", path.strip_prefix(&projects)?.display());
                }
            }
        }
    }
    for map in manifest.iter().flat_map(|m| &m.maps) {
        anyhow::ensure!(
            map.probe_overrides.is_empty() || overridden.contains(&map.level),
            "the pack has no project of {}, which has probe overrides",
            map.level
        );
    }
    println!("projects changed: {changed}");
    Ok(())
}
