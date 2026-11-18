// SPDX-License-Identifier: GPL-3.0-or-later
//! The original linear blend for the native parallax blend programs.
//! vertex-blend-parallax-021-pack DONOR_DB STOCK_DB NEW_OUTPUT
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
    let programs = vertex_blend::LinearPrograms::parallax(&donor.read(vertex_blend::SOURCE)?)?;
    let mut operations = Vec::new();
    for program in programs.programs() {
        let native = stock.read(program.path())?;
        let adapted = program.build(&native)?;
        let output = args[2].join("files").join(program.path());
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &adapted)?;
        println!(
            "{}: {} -> {}",
            program.path(),
            sha256_hex(&native),
            sha256_hex(&adapted)
        );
        operations.push(Operation::ReplaceFile {
            path: program.path().into(),
            source: None,
            base_sha256: sha256_hex(&native),
        });
    }
    let count = operations.len();
    let manifest = PackManifest {
        format_version: 1,
        id: "v021d-vertex-blend-parallax".into(),
        version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some(format!(
            "Original linear vertex-alpha blending in the {count} native parallax blend programs; explicit material selector, native fallback unchanged"
        )),
        requires: vec!["v021d-vertex-blend".into()],
        base_db_sha256: Some(stock_hash.into()),
        operations,
        server: None,
        notes: vec!["Built by examples/vertex-blend-parallax-021-pack.rs from weather_fog_021/vertex_blend/parallax-linear-programs.tsv. A parallax program computes the blend weight as every other native blend program does, with the material constants in cb1; the same four-instruction guard replaces the weight by the painted vertex alpha for a material whose strength is the original-linear selector. The masked equation is not added: no masked material uses parallax.".into()],
    };
    fs::write(
        args[2].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
