// SPDX-License-Identifier: GPL-3.0-or-later
//! Preserve complete original mipchains for shared alpha-sensitive textures.
//! alpha-textures-021-pack DONOR_DB STOCK_DB NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{fs, path::PathBuf};
use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::native_equivalence::{TextureMatch, TextureUsage, texture_match};

#[derive(Deserialize)]
struct Pins {
    donor_db_sha256: String,
    stock_db_sha256: String,
    textures: Vec<Texture>,
}

#[derive(Deserialize)]
struct Texture {
    texture: String,
    original_sha256: String,
    native_sha256: String,
    options_sha256: String,
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 3, "DONOR_DB STOCK_DB NEW_OUTPUT");
    ensure!(!args[2].exists(), "output must be new");
    let source = include_bytes!("../src/alpha_texture_021_pins.json");
    let pins: Pins = serde_json::from_slice(source)?;
    vostok_resource_porter::survarium_026::check_hash(&args[0], &pins.donor_db_sha256)?;
    vostok_resource_porter::survarium_026::check_hash(&args[1], &pins.stock_db_sha256)?;
    let mut donor = ResourceDb::open(&args[0])?;
    let mut native = ResourceDb::open(&args[1])?;
    let mut operations = Vec::new();
    for texture in &pins.textures {
        let path = format!("textures/{}.dds", texture.texture);
        let options = format!("textures/{}.options", texture.texture);
        let original = donor.read(&path)?;
        let stock = native.read(&path)?;
        let original_options = donor.read(&options)?;
        let native_options = native.read(&options)?;
        ensure!(
            sha256_hex(&original) == texture.original_sha256,
            "original texture changed: {path}"
        );
        ensure!(
            sha256_hex(&stock) == texture.native_sha256,
            "native texture changed: {path}"
        );
        ensure!(
            sha256_hex(&original_options) == texture.options_sha256,
            "original options changed: {options}"
        );
        ensure!(
            original_options == native_options,
            "options differ: {options}"
        );
        ensure!(
            texture_match(
                &original,
                Some(&original_options),
                &stock,
                Some(&native_options),
                TextureUsage::Colour
            ) == Some(TextureMatch::SameTopLevel),
            "not a shared regenerated mipchain: {path}"
        );
        ensure!(
            texture_match(
                &original,
                Some(&original_options),
                &stock,
                Some(&native_options),
                TextureUsage::Alpha
            )
            .is_none(),
            "visibility unexpectedly native: {path}"
        );
        for (extension, bytes) in [("dds", original), ("options", original_options)] {
            let target = format!(
                "textures/ported/v021d/original_alpha_mips/{}.{extension}",
                texture.texture
            );
            let output = args[2].join("files").join(&target);
            fs::create_dir_all(output.parent().context("texture parent")?)?;
            fs::write(output, bytes)?;
            operations.push(Operation::AddFile {
                path: target,
                source: None,
            });
        }
    }
    let pack = PackManifest {
        format_version: 1,
        id: "v021d-alpha-textures".into(),
        version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some("Exact original 0.21 alpha-sensitive DDS mipchains and options in a private namespace; native resources remain unchanged".into()),
        requires: vec!["maps-021d-rudnya".into()],
        base_db_sha256: Some(pins.stock_db_sha256),
        operations,
        server: None,
        notes: vec![format!("{} pinned original textures. Authored material edits separately bind only imported 0.21 materials. No mip regeneration, alpha cutoff retune, or shared native texture override. Source pins SHA256 {}", pins.textures.len(), sha256_hex(source))],
    };
    fs::write(args[2].join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    fs::write(args[2].join("source-pins.json"), source)?;
    println!(
        "{} original visibility texture mipchains",
        pins.textures.len()
    );
    Ok(())
}
