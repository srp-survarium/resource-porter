//! Build the scoped original-linear wall blend from immutable retail programs.
//! vertex-blend-021-pack DONOR_DB STOCK_DB NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_blend};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 3, "DONOR_DB STOCK_DB NEW_OUTPUT");
    ensure!(!args[2].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[1], stock_hash)?;
    let mut donor = ResourceDb::open(&args[0])?;
    let mut stock = ResourceDb::open(&args[1])?;
    let source = donor.read(vertex_blend::SOURCE)?;
    let mut operations = Vec::new();
    for path in vertex_blend::PATHS {
        let before = stock.read(path)?;
        let after = vertex_blend::build_linear(&source, &before)?;
        let output = args[2].join("files").join(path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &after)?;
        println!("{path}: {} -> {}", sha256_hex(&before), sha256_hex(&after));
        operations.push(Operation::ReplaceFile {
            path: path.into(),
            source: None,
            base_sha256: sha256_hex(&before),
        });
    }
    let manifest = PackManifest {
        format_version: 1, id: "v021d-vertex-blend".into(), version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Original linear vertex-alpha blending for explicitly marked 0.21 Rudnya wall material; native fallback unchanged".into()),
        requires: vec!["maps-021d-rudnya".into()], base_db_sha256: Some(stock_hash.into()),
        operations, server: None,
        notes: vec!["Built by examples/vertex-blend-021-pack.rs from pinned original/native programs. constant_vertex_blended_strength=-21021 selects original linear weighting. Only the authored port material opts in; all native arithmetic is retained for every other value. This first scoped correction does not yet cover every 0.21 blend material.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
