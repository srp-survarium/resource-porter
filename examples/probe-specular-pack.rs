// SPDX-License-Identifier: GPL-3.0-or-later
//! Reflection-only environment probes (`probe_specular`) as a content pack: every
//! `affect_diffuse = false` permutation of 0.26g's `environment_probe_lighting.ps`, taken from
//! the layers players have (the stock db first, pinned by its hash), the diffuse output's alpha zero.
//!
//! probe-specular-pack STOCK_SHA256 NEW_OUTPUT LAYER...
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::json;
use vostok_resource_porter::{
    ResourceDb,
    content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex},
    probe_specular,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(args.len() >= 3, "STOCK_SHA256 NEW_OUTPUT LAYER...");
    let (stock_sha, output) = (&args[0], PathBuf::from(&args[1]));
    let layers: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
    ensure!(!output.exists(), "output must be new");
    vostok_resource_porter::survarium_026::check_hash(&layers[0], stock_sha)?;
    let mut db = ResourceDb::open_layers(&layers)?;
    let keys: Vec<String> = db
        .paths_with_prefix(probe_specular::FAMILY)
        .filter(|p| probe_specular::is_reflection_only(&p[probe_specular::FAMILY.len()..]))
        .map(str::to_owned)
        .collect();
    ensure!(keys.len() == 16, "{} reflection-only permutations, expected 16", keys.len());
    let mut operations = Vec::new();
    for path in &keys {
        let shipped = db.read(path)?;
        let built = probe_specular::build(&shipped).with_context(|| path.clone())?;
        let at = vostok_resource_porter::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, built)?;
        operations.push(Operation::ReplaceFile {
            path: path.clone(),
            source: None,
            base_sha256: sha256_hex(&shipped),
        });
    }
    let pack = PackManifest {
        format_version: 1,
        id: "probe-specular".into(),
        version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some(
            "Reflection-only environment probes (affect_diffuse = false) as 0.31a2 draws them: the probe adds its reflection and leaves the diffuse light as it was; 0.26g's shader wrote the surface's own colour into it. Only London's ten window probes are such probes."
                .into(),
        ),
        requires: vec![],
        base_db_sha256: Some(stock_sha.clone()),
        operations,
        server: None,
        notes: vec![json!({"built_by": "examples/probe-specular-pack.rs", "layers": layers}).to_string()],
    };
    fs::write(output.join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    println!("{} permutations", keys.len());
    Ok(())
}
