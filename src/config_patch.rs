use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::weapon::{prepare_empty_output, record_file};
use crate::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Debug, Serialize)]
struct ConfigPatchRecord {
    format_version: u32,
    asset_id: String,
    source_resource: String,
    boolean_patches: BTreeMap<String, bool>,
    retained_indexed_suffixes: BTreeMap<String, usize>,
    files: Vec<BuiltFile>,
}

pub fn build_bundle(
    source_db: &Path,
    resource_path: &str,
    boolean_specifications: &[String],
    suffix_specifications: &[String],
    output: &Path,
) -> Result<()> {
    validate_resource_path(resource_path)?;
    if boolean_specifications.is_empty() && suffix_specifications.is_empty() {
        bail!("a config patch requires at least one patch specification");
    }

    let mut patches = BTreeMap::new();
    for specification in boolean_specifications {
        let (path, raw_value) = specification.split_once('=').with_context(|| {
            format!("config patch entry must be PATH=BOOLEAN: {specification:?}")
        })?;
        if path.is_empty() || path.split('.').any(|component| component.is_empty()) {
            bail!("config patch path is invalid: {path:?}");
        }
        let value = match raw_value {
            "true" => true,
            "false" => false,
            _ => bail!("config patch value must be true or false: {specification:?}"),
        };
        if patches.insert(path.to_owned(), value).is_some() {
            bail!("duplicate config patch path: {path}");
        }
    }

    let mut suffixes = BTreeMap::new();
    for specification in suffix_specifications {
        let (path, raw_count) = specification.split_once('=').with_context(|| {
            format!("indexed suffix entry must be PATH=COUNT: {specification:?}")
        })?;
        if path.is_empty() || path.split('.').any(|component| component.is_empty()) {
            bail!("indexed suffix path is invalid: {path:?}");
        }
        let count = raw_count
            .parse::<usize>()
            .with_context(|| format!("indexed suffix count is invalid: {specification:?}"))?;
        if suffixes.insert(path.to_owned(), count).is_some() {
            bail!("duplicate indexed suffix path: {path}");
        }
    }

    let mut source = ResourceDb::open(source_db)?;
    let source_bytes = source
        .read(resource_path)
        .with_context(|| format!("reading source config {resource_path}"))?;
    let mut config = BinaryConfig::parse(&source_bytes)
        .with_context(|| format!("parsing source config {resource_path}"))?;

    for (path, value) in &patches {
        match value_at_path(&config.root, path) {
            Some(ConfigValue::Boolean(_)) => {}
            Some(actual) => bail!("config patch {path:?} targets {actual:?}, not a boolean"),
            None => bail!("config patch path does not exist: {path}"),
        }
        config.root.set_path(path, ConfigValue::Boolean(*value))?;
    }
    for (path, count) in &suffixes {
        let ConfigValue::Indexed(values) = value_at_path(&config.root, path)
            .with_context(|| format!("indexed suffix path does not exist: {path}"))?
        else {
            bail!("indexed suffix patch {path:?} does not target an indexed table");
        };
        if *count > values.len() {
            bail!(
                "cannot retain the last {count} entries of {path}; it contains only {}",
                values.len()
            );
        }
        let retained = values[values.len() - count..].to_vec();
        config.root.set_path(path, ConfigValue::Indexed(retained))?;
    }

    let patched_bytes = config.to_bytes()?;
    let reparsed = BinaryConfig::parse(&patched_bytes)
        .with_context(|| format!("reparsing patched config {resource_path}"))?;
    for (path, value) in &patches {
        if value_at_path(&reparsed.root, path) != Some(&ConfigValue::Boolean(*value)) {
            bail!("patched config did not preserve {path}={value}");
        }
    }
    for (path, count) in &suffixes {
        let Some(ConfigValue::Indexed(values)) = value_at_path(&reparsed.root, path) else {
            bail!("patched config did not preserve indexed table {path}");
        };
        if values.len() != *count {
            bail!(
                "patched config retained {} entries at {path}, expected {count}",
                values.len()
            );
        }
    }

    prepare_empty_output(output)?;
    let destination = safe_join(&output.join("overlay"), resource_path)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, &patched_bytes)?;
    let files = vec![record_file(
        resource_path,
        "binary-config:boolean-patch",
        &patched_bytes,
    )];
    let record = ConfigPatchRecord {
        format_version: 1,
        asset_id: "target-schema-binary-config-patch".into(),
        source_resource: resource_path.to_owned(),
        boolean_patches: patches,
        retained_indexed_suffixes: suffixes,
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
        bail!("config resource path is not safe: {path:?}");
    }
    Ok(())
}

fn value_at_path<'a>(root: &'a ConfigValue, path: &str) -> Option<&'a ConfigValue> {
    let mut current = root;
    for component in path.split('.') {
        current = match current {
            ConfigValue::Named(values) => values
                .iter()
                .find(|entry| entry.name == component)
                .map(|entry| &entry.value)?,
            ConfigValue::Indexed(values) => values.get(component.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}
