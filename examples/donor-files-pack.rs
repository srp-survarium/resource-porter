// SPDX-License-Identifier: GPL-3.0-or-later
//! A ported map's own files as its donor build has them: each listed file was first shipped
//! by one of the map's own patches (not in the stock db), players have another version of it,
//! and the donor's bytes replace it (pinned by the players' version's hash). For files the
//! port took from elsewhere, e.g. London's office furniture texture, which patch 14 shipped
//! in another colour than 0.31a2's. `new_files` are the donor's files players do not have at
//! all, added as they are (e.g. a texture only a donor material set names).
//!
//! donor-files-pack MANIFEST.json NEW_OUTPUT
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use vostok_resource_porter::{
    ResourceDb,
    content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    version: String,
    description: String,
    donor: Vec<PathBuf>,
    /// What players have: the stock db, then every offered patch in order.
    shipped: Vec<PathBuf>,
    stock_sha256: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    new_files: Vec<String>,
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 2, "MANIFEST.json NEW_OUTPUT");
    let m: Manifest = serde_json::from_slice(&fs::read(&args[0])?)?;
    let output = &args[1];
    ensure!(!output.exists(), "output must be new");
    vostok_resource_porter::survarium_026::check_hash(&m.shipped[0], &m.stock_sha256)?;
    let mut donor = ResourceDb::open_layers(&m.donor)?;
    let stock = ResourceDb::open(&m.shipped[0])?;
    let mut shipped = ResourceDb::open_layers(&m.shipped)?;
    let mut operations = Vec::new();
    for path in &m.files {
        ensure!(!stock.contains(path), "{path}: a stock file, shared with other maps");
        let players = shipped.read(path).with_context(|| format!("{path}: players do not have it"))?;
        let own = donor.read(path).with_context(|| format!("{path}: the donor does not have it"))?;
        ensure!(players != own, "{path}: players have the donor's version already");
        let at = vostok_resource_porter::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, &own)?;
        operations.push(Operation::ReplaceFile {
            path: path.clone(),
            source: None,
            base_sha256: sha256_hex(&players),
        });
        println!("{path}: the donor's {} bytes for players' {}", own.len(), players.len());
    }
    for path in &m.new_files {
        ensure!(!shipped.contains(path), "{path}: players have it already");
        let own = donor.read(path).with_context(|| format!("{path}: the donor does not have it"))?;
        let at = vostok_resource_porter::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, &own)?;
        operations.push(Operation::AddFile { path: path.clone(), source: None });
        println!("{path}: the donor's {} bytes, new to players", own.len());
    }
    let pack = PackManifest {
        format_version: 1,
        id: m.id.clone(),
        version: m.version.clone(),
        target_build: TARGET_BUILD.into(),
        description: Some(m.description.clone()),
        requires: vec![],
        base_db_sha256: Some(m.stock_sha256.clone()),
        operations,
        server: None,
        notes: vec!["Built by examples/donor-files-pack.rs: each file first shipped by the map's own patches, or new to players, as its donor build has it.".into()],
    };
    fs::write(output.join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    Ok(())
}
