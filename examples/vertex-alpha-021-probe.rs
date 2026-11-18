//! Build the first observed original vertex-alpha layout for causal validation.
//! This is a diagnostic pack, not complete map or shadow-pass coverage.
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
    let source = ResourceDb::open(&args[0])?.read(vertex_alpha::SOURCE)?;
    let mut stock = ResourceDb::open(&args[1])?;
    let mut operations = Vec::new();
    for quality in [0, 3] {
        for (mode, blend, tail) in [
            (0, "00000", "000"),
            (1, "00000", "000"),
            (1, "00000", "001"),
            (2, "10000", "000"),
        ] {
            let path = format!(
                "shaders/sm_4_0/gbuffer_pass.ps/_{quality}___1001_00001_00____2_____0__{mode}_{blend}____00000____{tail}__________0__"
            );
            let target = stock.read(&path)?;
            let after = vertex_alpha::build_observed(&source, &target)?;
            let output = args[2].join("files").join(&path);
            fs::create_dir_all(output.parent().context("shader parent")?)?;
            fs::write(output, after)?;
            operations.push(Operation::ReplaceFile {
                path,
                source: None,
                base_sha256: vertex_alpha::NATIVE_HASH.into(),
            });
        }
    }
    let manifest = PackManifest {
        format_version: 1, id: "v021d-vertex-alpha-probe".into(), version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Diagnostic: original vertex-alpha cutout in the observed Vostok dirt shader".into()),
        requires: vec!["maps-021d-rudnya".into()], base_db_sha256: Some(stock_hash.into()),
        operations, server: None,
        notes: vec!["Built by examples/vertex-alpha-021-probe.rs. Five original authored profiles use explicit negative alpha_ref selectors; native alpha cutoff remains unchanged otherwise. Only the observed class2 program and its identical aliases are covered. Other layouts and shadows still require implementation and validation.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
