//! Build all pinned native layouts of both original vertex-blend equations.
//! vertex-blend-masked-021-pack DONOR_DB STOCK_DB NEW_OUTPUT
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
    let masked = donor.read(vertex_blend::MASKED_SOURCE)?;
    let programs = vertex_blend::MaskedPrograms::new(&source, &masked)?;
    let mut operations = Vec::new();
    for program in programs.programs() {
        let path = program.path();
        let before = stock.read(path)?;
        let after = program.build(&before).with_context(|| path.to_owned())?;
        let output = args[2].join("files").join(path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &after)?;
        println!(
            "{path}: {} -> {}",
            program.native_sha256(),
            sha256_hex(&after)
        );
        operations.push(Operation::ReplaceFile {
            path: path.into(),
            source: None,
            base_sha256: program.native_sha256().into(),
        });
    }
    let manifest = PackManifest {
        format_version: 1, id: "v021d-vertex-blend".into(), version: "3".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Original linear and masked vertex-alpha blending across all pinned native blend layouts; explicit per-material selector and unchanged native fallback".into()),
        requires: vec!["maps-021d-rudnya".into()], base_db_sha256: Some(stock_hash.into()),
        operations, server: None,
        notes: vec!["Built by examples/vertex-blend-masked-021-pack.rs. Each native blob, vertex-alpha input and weight destination is pinned in weather_fog_021/vertex_blend/linear-programs.tsv. Strength -21021 selects linear mode. Masked mode uses exact negative factor*2^23 in the otherwise unused interval [-2^24,-2^23], retaining authored falloff. Both original programs are pinned and native fallbacks remain unchanged.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
