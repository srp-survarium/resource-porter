use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::weapon::{prepare_empty_output, record_file};
use crate::ResourceDb;

#[derive(Debug, Serialize)]
struct ResourceCopyPatchRecord {
    format_version: u32,
    asset_id: String,
    copied_resources: BTreeMap<String, String>,
    files: Vec<BuiltFile>,
}

pub fn build_bundle(source_db: &Path, specifications: &[String], output: &Path) -> Result<()> {
    if specifications.is_empty() {
        bail!("a resource-copy patch requires at least one SOURCE=DESTINATION specification");
    }
    let mut copies = BTreeMap::new();
    for specification in specifications {
        let (source, destination) = specification.split_once('=').with_context(|| {
            format!("resource copy must be SOURCE=DESTINATION: {specification:?}")
        })?;
        validate_resource_path(source)?;
        validate_resource_path(destination)?;
        if copies
            .insert(destination.to_owned(), source.to_owned())
            .is_some()
        {
            bail!("duplicate resource-copy destination: {destination}");
        }
    }

    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut database = ResourceDb::open(source_db)?;
    let mut files = Vec::with_capacity(copies.len());
    for (destination, source) in &copies {
        let bytes = database
            .read(source)
            .with_context(|| format!("reading source resource {source}"))?;
        let output_path = safe_join(&overlay, destination)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output_path, &bytes)?;
        files.push(record_file(
            destination,
            &format!("exact-resource-copy:{source}"),
            &bytes,
        ));
    }

    let record = ResourceCopyPatchRecord {
        format_version: 1,
        asset_id: "exact-resource-copy-patch".into(),
        copied_resources: copies,
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(())
}

fn validate_resource_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("resource path is not safe: {path:?}");
    }
    Ok(())
}

