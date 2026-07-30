use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::material_conversion::MaterialCatalog;
use crate::weapon::{prepare_empty_output, record_file};
use crate::ResourceDb;

#[derive(Debug, Serialize)]
struct MaterialPatchRecord {
    format_version: u32,
    asset_id: String,
    source_materials: BTreeMap<String, String>,
    files: Vec<BuiltFile>,
}

pub fn build_bundle(
    source_db: &Path,
    target_db: &Path,
    specifications: &[String],
    output: &Path,
) -> Result<()> {
    if specifications.is_empty() {
        bail!("a material patch requires at least one PATH=TEMPLATE specification");
    }

    let mut material_templates = BTreeMap::new();
    for specification in specifications {
        let (path, template) = specification.split_once('=').with_context(|| {
            format!("material patch entry must be PATH=TEMPLATE: {specification:?}")
        })?;
        for (label, value) in [("material path", path), ("template", template)] {
            if !value.starts_with("material_instances/")
                || !value.ends_with(".material")
                || value
                    .split('/')
                    .any(|component| component.is_empty() || matches!(component, "." | ".."))
            {
                bail!("{label} is not a safe material-instance resource: {value:?}");
            }
        }
        if material_templates
            .insert(path.to_owned(), template.to_owned())
            .is_some()
        {
            bail!("duplicate material patch output path: {path}");
        }
    }

    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut source = ResourceDb::open(source_db)?;
    let mut target = ResourceDb::open(target_db)?;
    let catalog = MaterialCatalog::load(&mut target)?;
    let mut files = Vec::new();

    for (path, template) in &material_templates {
        let source_bytes = source
            .read(path)
            .with_context(|| format!("reading source material {path}"))?;
        let converted =
            catalog.convert(path, &source_bytes, Some(template), &source, &target)?;
        let destination = safe_join(&overlay, path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &converted.bytes)?;
        files.push(record_file(
            path,
            &format!("material-template:{}", converted.template_path),
            &converted.bytes,
        ));
    }

    let record = MaterialPatchRecord {
        format_version: 1,
        asset_id: "target-schema-material-patch".into(),
        source_materials: material_templates,
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(())
}
