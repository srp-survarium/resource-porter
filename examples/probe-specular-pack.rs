// SPDX-License-Identifier: GPL-3.0-or-later
//! Reflection-only environment probes (`probe_specular`) as a content pack: every
//! `affect_diffuse = false` permutation of 0.26g's `environment_probe_lighting.ps`, taken from
//! the layers players have (the stock db first, pinned by its hash), the diffuse output's alpha zero.
//!
//! With `--parallax M1,M2,...` it is pack `london-probes`: the box permutations also get
//! 0.31a2's box-projected reflections behind those scene markers (`probe_parallax`).
//!
//! probe-specular-pack STOCK_SHA256 NEW_OUTPUT [--parallax M1,M2,...] LAYER...
use std::{fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::json;
use vostok_resource_porter::{
    ResourceDb,
    content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex},
    probe_parallax, probe_specular,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(args.len() >= 3, "STOCK_SHA256 NEW_OUTPUT [--parallax M1,M2,...] LAYER...");
    let (stock_sha, output) = (&args[0], PathBuf::from(&args[1]));
    let (markers, rest): (Vec<f32>, &[String]) = if args[2] == "--parallax" {
        (
            args[3].split(',').map(|m| m.parse::<f32>()).collect::<Result<_, _>>()?,
            &args[4..],
        )
    } else {
        (vec![], &args[2..])
    };
    let layers: Vec<PathBuf> = rest.iter().map(PathBuf::from).collect();
    ensure!(!output.exists(), "output must be new");
    vostok_resource_porter::survarium_026::check_hash(&layers[0], stock_sha)?;
    let mut db = ResourceDb::open_layers(&layers)?;
    let key_of = |p: &str| p[probe_specular::FAMILY.len()..].to_owned();
    let keys: Vec<String> = db
        .paths_with_prefix(probe_specular::FAMILY)
        .filter(|p| {
            probe_specular::is_reflection_only(&key_of(p))
                || (!markers.is_empty() && probe_parallax::is_box(&key_of(p)))
        })
        .map(str::to_owned)
        .collect();
    let expected = if markers.is_empty() { 16 } else { 24 };
    ensure!(keys.len() == expected, "{} permutations, expected {expected}", keys.len());
    let mut operations = Vec::new();
    for path in &keys {
        let shipped = db.read(path)?;
        let key = key_of(path);
        let mut built = shipped.clone();
        if probe_specular::is_reflection_only(&key) {
            built = probe_specular::build(&built).with_context(|| path.clone())?;
        }
        if !markers.is_empty() && probe_parallax::is_box(&key) {
            built = probe_parallax::build(&built, &markers).with_context(|| path.clone())?;
        }
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
        id: if markers.is_empty() { "probe-specular" } else { "london-probes" }.into(),
        version: "1".into(),
        target_build: TARGET_BUILD.into(),
        description: Some(if !markers.is_empty() {
            format!("London's environment probes as 0.31a2 draws them: reflection-only probes (affect_diffuse = false) add their reflection and leave the diffuse light; box probes look their reflection up box-projected (parallax-corrected) on scenes marked {markers:?} (mie_fog_base_height). Native maps draw neither: no other map has reflection-only probes, and the box projection runs only behind London's markers.")
        } else {
            "Reflection-only environment probes (affect_diffuse = false) as 0.31a2 draws them: the probe adds its reflection and leaves the diffuse light as it was; 0.26g's shader wrote the surface's own colour into it. Only London's ten window probes are such probes."
                .into()
        }),
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
