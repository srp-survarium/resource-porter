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
    let shaders = vertex_alpha::parallax::build(&mut donor, &mut stock)?;
    ensure!(shaders.len() == 28, "incomplete ordinary parallax family");
    let mut operations = Vec::new();
    for (path, bytes) in shaders {
        let file = args[2].join("files").join(&path);
        fs::create_dir_all(file.parent().context("shader parent")?)?;
        fs::write(file, bytes)?;
        operations.push(Operation::AddFile { path, source: None });
    }
    let manifest=PackManifest {
        format_version:1,id:"v021d-vertex-alpha-parallax".into(),version:"1".into(),target_build:TARGET_BUILD.into(),
        description:Some("Missing ordinary parallax alpha shaders with original painted coverage".into()),
        requires:vec!["maps-021d-rudnya".into()],base_db_sha256:Some(stock_hash.into()),operations,server:None,
        notes:vec!["Built by examples/vertex-alpha-parallax-021-pack.rs. Adds 28 absent ordinary alpha permutations for seven vertex classes, two quality levels and two native aliases. High quality retains native height calculations and finish, removing only unused dirt-mask computation and binding; low quality retains the native ordinary non-parallax program because native low-quality parallax is inactive. Both use the independently checked original painted cutoff guard. No existing shader is replaced.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!("28 missing ordinary parallax paths built; comparisons pending");
    Ok(())
}
