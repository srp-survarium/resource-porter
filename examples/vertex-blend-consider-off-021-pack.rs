// SPDX-License-Identifier: GPL-3.0-or-later
//! Supply the absent alpha-tested blend programs whose cutoff ignores the blend.
//! vertex-blend-consider-off-021-pack DONOR_DB STOCK_DB MOUNTED_LAYERS_TEXT PACK_DIRS_TEXT NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, fs, path::PathBuf};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_blend};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 5,
        "DONOR_DB STOCK_DB MOUNTED_LAYERS_TEXT PACK_DIRS_TEXT NEW_OUTPUT"
    );
    ensure!(!args[4].exists(), "output must be new");
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    vostok_resource_porter::survarium_026::check_hash(&args[1], stock_hash)?;
    // A sibling counts as present in any published patch or any other listed pack.
    let mut present = BTreeSet::new();
    for archive in fs::read_to_string(&args[2])?
        .lines()
        .filter(|l| !l.is_empty())
    {
        present.extend(ResourceDb::open(archive)?.paths().map(str::to_owned));
    }
    for pack in fs::read_to_string(&args[3])?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let manifest: PackManifest = serde_json::from_slice(
            &fs::read(PathBuf::from(pack).join("pack.json")).with_context(|| pack.to_owned())?,
        )?;
        for operation in manifest.operations {
            if let Operation::AddFile { path, .. } | Operation::ReplaceFile { path, .. } = operation
            {
                present.insert(path);
            }
        }
    }
    let shaders = vertex_blend::consider_off::build(
        &mut ResourceDb::open(&args[0])?,
        &mut ResourceDb::open(&args[1])?,
        &|path| present.contains(path),
    )?;
    ensure!(!shaders.is_empty(), "nothing absent");
    let count = shaders.len();
    let mut operations = Vec::new();
    for (path, bytes) in shaders {
        let output = args[4].join("files").join(&path);
        fs::create_dir_all(output.parent().context("shader parent")?)?;
        fs::write(output, &bytes)?;
        println!("{path}: {}", sha256_hex(&bytes));
        operations.push(Operation::AddFile { path, source: None });
    }
    let manifest = PackManifest {
        format_version: 1,
        id: "v021d-vertex-blend-consider-off".into(),
        version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some(format!(
            "Base-alpha cutoff for the {count} absent alpha-tested blend programs; no existing shader resource changes"
        )),
        requires: vec!["v021d-vertex-blend".into()],
        base_db_sha256: Some(stock_hash.into()),
        operations,
        server: None,
        notes: vec!["Built by examples/vertex-blend-consider-off-021-pack.rs. Each program is the consider-on program of the same key with the original blend adapter of v021d-vertex-blend, and one changed instruction: the native blend-aware alpha 1 + w * (a - 1) becomes the base alpha a before the unchanged alpha_ref comparison. Every addition is absent across stock, the supplied mounted archives and the supplied packs.".into()],
    };
    fs::write(
        args[4].join("pack.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
