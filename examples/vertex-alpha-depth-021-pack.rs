//! Build the ordinary depth cutoff half of the original painted-alpha adapter.
//! It must be composed with the visible-pass adapter before marking materials.
use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD};
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_alpha};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 3, "DONOR_DB STOCK_DB NEW_OUTPUT");
    ensure!(!args[2].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[1], stock_hash)?;
    let shaders = vertex_alpha::depth::build(
        &mut ResourceDb::open(&args[0])?,
        &mut ResourceDb::open(&args[1])?,
    )?;
    let mut operations = Vec::new();
    for (path, (base_sha256, bytes)) in shaders {
        let file = args[2].join("files").join(&path);
        fs::create_dir_all(file.parent().context("shader parent")?)?;
        fs::write(file, bytes)?;
        operations.push(Operation::ReplaceFile {
            path,
            source: None,
            base_sha256,
        });
    }
    let manifest = PackManifest {
        format_version: 1, id: "v021d-vertex-alpha-depth".into(), version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Preserve original ordinary shadow thresholds for private painted-alpha materials".into()),
        requires: vec!["maps-021d-rudnya".into()], base_db_sha256: Some(stock_hash.into()),
        operations, server: None,
        notes: vec!["Built by examples/vertex-alpha-depth-021-pack.rs from 21 pinned native programs and 14 checked original contracts. Five selectors decode to their original alpha_ref (.25 or .5); every other value keeps native sampling and cutoff. No material is activated by this pack. A complete visible-pass adapter and material edits are required for deployment.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!("78 depth shader paths built; visible-pass integration remains required");
    Ok(())
}
