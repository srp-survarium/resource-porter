//! Merge additive map bundles without discarding another bundle's registrations.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Read,
    path::PathBuf,
};
use vostok_resource_porter::{
    BinaryConfig, ConfigValue, ResourceDb, archive::safe_join, manifest::BuiltFile,
};

const DICTIONARY: &str = "gameplay/db_static_dictionaries";

#[derive(Deserialize)]
struct Input {
    target_db_sha256: String,
    maps: Vec<Map>,
    files: Vec<BuiltFile>,
    #[serde(default)]
    materialize_target_hard_links: BTreeSet<String>,
}
#[derive(Deserialize)]
struct Map {
    id: i32,
}
#[derive(Deserialize)]
struct Patch {
    target_db_sha256: String,
    files: Vec<Replacement>,
}
#[derive(Deserialize)]
struct Replacement {
    #[serde(flatten)]
    file: BuiltFile,
    expected_sha256: String,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn rows(config: &ConfigValue) -> Result<&Vec<ConfigValue>> {
    match config.named("maps_dict") {
        Some(ConfigValue::Indexed(rows)) => Ok(rows),
        _ => anyhow::bail!("maps_dict is absent or is not an array"),
    }
}
fn id(row: &ConfigValue) -> Result<i32> {
    match row.named("id") {
        Some(ConfigValue::Integer(value)) => Ok(*value),
        _ => anyhow::bail!("map row has no integer id"),
    }
}

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let mut check_only = false;
    let mut patches = Vec::new();
    let mut args = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--check-only" {
            check_only = true;
        } else if argument == "--patch" {
            patches.push(PathBuf::from(
                arguments.next().context("--patch needs a bundle path")?,
            ));
        } else {
            args.push(PathBuf::from(argument));
        }
    }
    ensure!(
        args.len() >= 3,
        "usage: compose-day-map-bundles [--check-only] [--patch PATCH_BUNDLE] BASE_DB OUTPUT_BUNDLE INPUT_BUNDLE..."
    );
    let base_path = &args[0];
    let output = &args[1];
    ensure!(!output.exists(), "output already exists");
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(base_path)?;
    let mut block = vec![0; 8 * 1024 * 1024];
    loop {
        let count = file.read(&mut block)?;
        if count == 0 {
            break;
        }
        hasher.update(&block[..count]);
    }
    let base_hash = format!("{:x}", hasher.finalize());
    let mut base = ResourceDb::open(base_path)?;
    let mut dictionary = BinaryConfig::parse(&base.read(DICTIONARY)?)?;
    let original_rows = rows(&dictionary.root)?.clone();
    let mut registrations = BTreeMap::new();
    let mut selected: BTreeMap<String, (BuiltFile, PathBuf)> = BTreeMap::new();
    let mut provenance = Vec::new();
    let mut materialize_target_hard_links = BTreeSet::new();
    for bundle in &args[2..] {
        let manifest_path = bundle.join("port-manifest.json");
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: Input = serde_json::from_slice(&manifest_bytes)?;
        ensure!(
            manifest.target_db_sha256 == base_hash,
            "target hash mismatch: {}",
            bundle.display()
        );
        materialize_target_hard_links.extend(manifest.materialize_target_hard_links);
        let mut incoming_dictionary = None;
        for entry in manifest.files {
            let path = safe_join(&bundle.join("overlay"), &entry.path)?;
            let bytes = fs::read(&path)?;
            ensure!(
                bytes.len() as u64 == entry.size && sha(&bytes) == entry.sha256,
                "bundle resource mismatch: {}",
                path.display()
            );
            if entry.path == DICTIONARY {
                ensure!(incoming_dictionary.is_none(), "duplicate dictionary entry");
                incoming_dictionary = Some(BinaryConfig::parse(&bytes)?);
                continue;
            }
            if let Some((previous, _)) = selected.get(&entry.path) {
                ensure!(
                    previous.sha256 == entry.sha256 && previous.size == entry.size,
                    "conflicting shared resource {}",
                    entry.path
                );
            } else {
                selected.insert(entry.path.clone(), (entry, path));
            }
        }
        let mut incoming = incoming_dictionary.context("bundle has no dictionary")?;
        let incoming_rows = rows(&incoming.root)?.clone();
        let map_ids: Vec<_> = manifest.maps.iter().map(|map| map.id).collect();
        ensure!(!map_ids.is_empty(), "bundle declares no maps");
        for map_id in &map_ids {
            ensure!(
                !original_rows
                    .iter()
                    .any(|row| id(row).ok() == Some(*map_id)),
                "map id {map_id} replaces a native registration"
            );
            let matching: Vec<_> = incoming_rows
                .iter()
                .filter(|row| id(row).ok() == Some(*map_id))
                .collect();
            ensure!(
                matching.len() == 1,
                "missing or duplicate registration {map_id}"
            );
            ensure!(
                registrations.insert(*map_id, matching[0].clone()).is_none(),
                "multiple bundles own map id {map_id}"
            );
        }
        let retained: Vec<_> = incoming_rows
            .into_iter()
            .filter(|row| !map_ids.contains(&id(row).unwrap_or(i32::MIN)))
            .collect();
        ensure!(
            retained == original_rows,
            "bundle modifies undeclared map registrations"
        );
        incoming
            .root
            .set_path("maps_dict", ConfigValue::Indexed(original_rows.clone()))?;
        ensure!(
            incoming == dictionary,
            "bundle modifies other dictionary sections"
        );
        provenance.push(serde_json::json!({"bundle":bundle,"manifest_sha256":sha(&manifest_bytes),"map_ids":map_ids}));
    }
    for patch in patches {
        let manifest_bytes = fs::read(patch.join("port-manifest.json"))?;
        let manifest: Patch = serde_json::from_slice(&manifest_bytes)?;
        ensure!(
            manifest.target_db_sha256 == base_hash,
            "patch target hash mismatch"
        );
        let mut replaced = BTreeSet::new();
        for replacement in manifest.files {
            let entry = replacement.file;
            ensure!(
                entry.path != DICTIONARY,
                "patch cannot replace map registrations"
            );
            ensure!(
                replaced.insert(entry.path.clone()),
                "duplicate patch resource"
            );
            let (original, _) = selected
                .get(&entry.path)
                .context("patch resource is not in the map bundles")?;
            ensure!(
                original.sha256 == replacement.expected_sha256,
                "patch baseline mismatch: {}",
                entry.path
            );
            let path = safe_join(&patch.join("overlay"), &entry.path)?;
            let bytes = fs::read(&path)?;
            ensure!(
                bytes.len() as u64 == entry.size && sha(&bytes) == entry.sha256,
                "patch resource mismatch: {}",
                path.display()
            );
            selected.insert(entry.path.clone(), (entry, path));
        }
        provenance.push(serde_json::json!({"patch":patch,"manifest_sha256":sha(&manifest_bytes),"replaced_paths":replaced}));
    }
    let mut merged_rows = original_rows;
    merged_rows.extend(registrations.values().cloned());
    dictionary
        .root
        .set_path("maps_dict", ConfigValue::Indexed(merged_rows))?;
    let dictionary_bytes = dictionary.to_bytes()?;
    ensure!(
        BinaryConfig::parse(&dictionary_bytes)? == dictionary,
        "dictionary roundtrip mismatch"
    );
    if check_only {
        println!(
            "Verified {} resources; would merge map IDs {:?}; no files written",
            selected.len() + 1,
            registrations.keys().collect::<Vec<_>>()
        );
        return Ok(());
    }
    fs::create_dir_all(output.join("overlay/gameplay"))?;
    let mut records = Vec::new();
    for (_, (record, input)) in selected {
        let destination = safe_join(&output.join("overlay"), &record.path)?;
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::copy(&input, &destination)?;
        ensure!(
            sha(&fs::read(&destination)?) == record.sha256,
            "copied resource changed"
        );
        records.push(record);
    }
    fs::write(output.join("overlay").join(DICTIONARY), &dictionary_bytes)?;
    records.push(BuiltFile {
        path: DICTIONARY.into(),
        source: "merged map registrations".into(),
        size: dictionary_bytes.len() as u64,
        sha256: sha(&dictionary_bytes),
    });
    let result = serde_json::json!({"format_version":1,"asset_id":"composed-day-maps",
        "target_db_sha256":base_hash,"maps":registrations.keys().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>(),
        "inputs":provenance,"files":records,"materialize_target_hard_links":materialize_target_hard_links});
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!(
        "Verified {} resources; merged map IDs {:?}",
        records.len(),
        registrations.keys().collect::<Vec<_>>()
    );
    Ok(())
}
