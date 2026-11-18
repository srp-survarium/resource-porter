//! Add the 28 native shader paths needed by original 0.21 dirt alpha.
//! visibility-021-pack STOCK_DB NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::{ResourceDb, weather_fog_021::visibility};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 2, "STOCK_DB NEW_OUTPUT");
    ensure!(!args[1].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[0], stock_hash)?;
    let mut stock = ResourceDb::open(&args[0])?;
    let mut operations = Vec::new();
    let shaders = visibility::build(&mut stock)?;
    ensure!(shaders.len() == 28, "expected complete28-path dirt family");
    for (path, bytes) in shaders {
        let output = args[1].join("files").join(&path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &bytes)?;
        println!("{path}: {}", sha256_hex(&bytes));
        operations.push(Operation::AddFile { path, source: None });
    }
    let pack = PackManifest {
        format_version: 1, id: "v021d-material-visibility".into(), version: "3".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("All28 absent native shader paths for original 0.21 dirt alpha-test with parallax; original material selectors are authored separately".into()),
        requires: vec!["maps-021d-rudnya".into()],
        base_db_sha256: Some(stock_hash.into()), operations, server: None,
        notes: vec!["Built by examples/visibility-021-pack.rs from fingerprinted stock programs. The0-prefixed program reuses byte-identical native alpha code; the3-prefixed program adds the existing native alpha-test formula. Seven vertex classes each have two native aliases with identical source bytes; classes without vertex alpha retain the shipped zero-mask alpha formula. No blend arithmetic or existing shader resource is changed.".into()],
    };
    fs::write(args[1].join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    Ok(())
}
