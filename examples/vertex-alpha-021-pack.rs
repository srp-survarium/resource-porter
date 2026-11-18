//! Build original visible and shadow alpha rules without activating materials.
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
    let mut donor = ResourceDb::open(&args[0])?;
    let mut stock = ResourceDb::open(&args[1])?;
    let mut shaders = vertex_alpha::geometry::build(&mut donor, &mut stock)?;
    for (path, data) in vertex_alpha::depth::build(&mut donor, &mut stock)? {
        ensure!(
            shaders.insert(path, data).is_none(),
            "duplicate shader path"
        );
    }
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
    let manifest=PackManifest {
        format_version:1,id:"v021d-vertex-alpha".into(),version:"1".into(),target_build:TARGET_BUILD.into(),
        description:Some("Original painted-alpha visibility and ordinary shadow cutoffs for private 0.21 materials".into()),
        requires:vec!["maps-021d-rudnya".into()],base_db_sha256:Some(stock_hash.into()),operations,server:None,
        notes:vec!["Built by examples/vertex-alpha-021-pack.rs from pinned original/native shaders. The 1364 visible paths retain native finish and default arithmetic, adding only the original painted cutoff for five explicit alpha_ref selectors. The 78 depth paths decode those selectors to the original ordinary shadow threshold. Matching private material edits must select ordinary native blending and preserve original active inputs; this pack activates no materials by itself.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "1364 visible + 78 depth paths built; material integration and runtime comparisons pending"
    );
    Ok(())
}
