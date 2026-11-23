// SPDX-License-Identifier: GPL-3.0-or-later
//! v021d-vertex-blend@4: version 3's programs, and in its alpha-tested
//! blend programs 0.69d's cut by the second layer's alpha behind its own
//! strength selector (`weather_fog_021::vertex_blend::blend_cut`).
//! vertex-blend-069d-cut-pack PACK_3_DIR STOCK_DB NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, sha256_hex};
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_blend::blend_cut};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 3, "PACK_3_DIR STOCK_DB NEW_OUTPUT");
    ensure!(!args[2].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[1], stock_hash)?;
    let mut manifest: PackManifest = serde_json::from_slice(&fs::read(args[0].join("pack.json"))?)?;
    ensure!(manifest.id == "v021d-vertex-blend" && manifest.version == "3", "not v021d-vertex-blend@3");
    let mut covered = BTreeSet::new();
    let mut adapted = 0;
    for operation in &manifest.operations {
        let Operation::ReplaceFile { path, .. } = operation else { continue };
        covered.insert(path.clone());
        let bytes = fs::read(args[0].join("files").join(path)).with_context(|| path.clone())?;
        let after = match blend_cut::build(&bytes).with_context(|| path.clone())? {
            Some(after) => {
                adapted += 1;
                println!("{path}: {} -> {}", sha256_hex(&bytes), sha256_hex(&after));
                after
            }
            None => bytes,
        };
        let output = args[2].join("files").join(path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &after)?;
    }
    // Every stock program of the family is one version 3 already carries.
    let mut stock = ResourceDb::open(&args[1])?;
    let paths: Vec<String> = stock.paths().filter(|p| p.starts_with("shaders/sm_4_0/gbuffer_pass.ps/")).map(str::to_owned).collect();
    for path in paths.iter().filter(|p| !covered.contains(*p)) {
        ensure!(blend_cut::build(&stock.read(path)?)?.is_none(), "{path}: an alpha-tested blend program version 3 does not carry");
    }
    ensure!(adapted > 0, "no program adapted");
    manifest.version = "4".into();
    manifest.description = Some(format!(
        "{}; @4: 0.69d's cut of an alpha-tested blend by its second layer's alpha in the {adapted} alpha-tested blend programs, selected by the strength {} (linear, base opaque) or -(2^25 + strength * 2^17) (tiled mask)",
        manifest.description.as_deref().unwrap_or(""),
        blend_cut::CUT_LINEAR
    ));
    manifest.notes.push(format!(
        "@4 built by examples/vertex-blend-069d-cut-pack.rs from @3's own files: {adapted} alpha-tested blend programs (alpha_test_consider_vertex_blend on, vertex-colour layouts) gain two guarded blocks around the blend-aware alpha (weight before it, alpha after it, temporaries r8 and r9); every other instruction and every other file is @3's."
    ));
    fs::write(args[2].join("pack.json"), serde_json::to_vec_pretty(&manifest)?)?;
    println!("{adapted} programs adapted");
    Ok(())
}
