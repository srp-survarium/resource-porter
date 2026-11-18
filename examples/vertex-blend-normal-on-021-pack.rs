//! Supply only absent native paths with their original normal-on linear shaders.
//! vertex-blend-normal-on-021-pack DONOR_DB STOCK_DB MOUNTED_LAYERS_TEXT NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_blend};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 4,
        "DONOR_DB STOCK_DB MOUNTED_LAYERS_TEXT NEW_OUTPUT"
    );
    ensure!(!args[3].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[1], stock_hash)?;
    let shaders = vertex_blend::normal_on::build(
        &mut ResourceDb::open(&args[0])?,
        &mut ResourceDb::open(&args[1])?,
    )?;
    // Absence must hold over published compatibility patches as well as stock.
    for archive in fs::read_to_string(&args[2])?
        .lines()
        .filter(|line| !line.is_empty())
    {
        let db = ResourceDb::open(archive)?;
        for path in db.paths() {
            ensure!(
                !shaders.contains_key(path),
                "existing shader in {archive}: {path}"
            );
        }
    }
    let mut operations = Vec::new();
    for (path, bytes) in shaders {
        let output = args[3].join("files").join(&path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &bytes)?;
        println!("{path}: {}", sha256_hex(&bytes));
        operations.push(Operation::AddFile { path, source: None });
    }
    let manifest = PackManifest {
        format_version: 1, id: "v021d-vertex-blend-normal-on".into(), version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Original normal-on linear blend shaders for the 704 absent native aliases; no existing shader resource changes".into()),
        requires: vec!["maps-021d-rudnya".into()], base_db_sha256: Some(stock_hash.into()),
        operations, server: None,
        notes: vec!["Built from pinned native programs by examples/vertex-blend-normal-on-021-pack.rs. Missing mask-off permutations replace the dead tiled-mask UV/sample dependency with native base-alpha weighting and remove its binding. Missing alpha-test permutations use the exact native base/blended cutoff and alpha_ref ABI, after original blend selection. All existing native programs are untouched; source layouts preserve native scene-time, normal, texture and finish bindings. Every addition must be absent across the supplied mounted archive stack.".into()],
    };
    fs::write(
        args[3].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
