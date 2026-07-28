use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::to_writer_pretty;
use sha2::{Digest, Sha256};

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::dependency;
use crate::manifest::{BuildRecord, BuiltFile, PortManifest, SrpContentManifest};

pub fn build_bundle(
    manifest: &PortManifest,
    target_db: &Path,
    donor_db: &Path,
    output: &Path,
) -> Result<BuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut target = ResourceDb::open(target_db)?;
    let mut donor = ResourceDb::open(donor_db)?;
    let mut files = Vec::new();

    let mut weapon = parse_config(
        &mut target,
        &manifest.weapon.template_config,
        "target weapon template",
    )?;
    // Parse donor configs as an explicit compatibility gate even though the
    // v0.100b schema comes from the old template.
    parse_config(&mut donor, &manifest.weapon.donor_config, "donor weapon")?;
    apply_patches(&mut weapon.root, &manifest.weapon.patch)?;
    write_generated_config(
        &overlay,
        &manifest.weapon.target_config,
        &weapon,
        "generated:weapon-config",
        &mut files,
    )?;

    if manifest.ammo.reuse_existing {
        parse_config(
            &mut target,
            &manifest.ammo.target_config,
            "existing target ammunition",
        )?;
        if !manifest.ammo.patch.is_empty() {
            bail!("reused ammunition cannot contain patches");
        }
    } else {
        let mut ammo = parse_config(
            &mut target,
            &manifest.ammo.template_config,
            "target ammunition template",
        )?;
        parse_config(&mut donor, &manifest.ammo.donor_config, "donor ammunition")?;
        apply_patches(&mut ammo.root, &manifest.ammo.patch)?;
        write_generated_config(
            &overlay,
            &manifest.ammo.target_config,
            &ammo,
            "generated:ammo-config",
            &mut files,
        )?;
    }

    let mut dictionary = parse_config(&mut target, &manifest.dictionary.path, "target dictionary")?;
    add_dictionary_item(
        &mut dictionary.root,
        manifest.weapon.dict_id,
        &manifest.weapon.target_config,
        manifest.weapon.item_category,
        false,
    )?;
    if manifest.ammo.reuse_existing {
        validate_dictionary_item(
            &dictionary.root,
            manifest.ammo.dict_id,
            &manifest.ammo.target_config,
        )?;
    } else {
        add_dictionary_item(
            &mut dictionary.root,
            manifest.ammo.dict_id,
            &manifest.ammo.target_config,
            manifest.ammo.item_category,
            true,
        )?;
    }
    write_generated_config(
        &overlay,
        &manifest.dictionary.path,
        &dictionary,
        "generated:dictionary",
        &mut files,
    )?;

    let mut localization = parse_config(
        &mut target,
        &manifest.localization.path,
        "target localization",
    )?;
    add_localization_strings(&mut localization.root, &manifest.localization.strings)?;
    write_generated_config(
        &overlay,
        &manifest.localization.path,
        &localization,
        "generated:localization",
        &mut files,
    )?;

    for prefix in &manifest.copy.prefixes {
        let paths: Vec<String> = donor
            .paths_with_prefix(prefix)
            .map(ToOwned::to_owned)
            .collect();
        if paths.is_empty() {
            bail!("copy prefix matched no donor resources: {prefix}");
        }
        for path in paths {
            copy_donor_file(&mut donor, &overlay, &path, &mut files)?;
        }
    }
    for path in &manifest.copy.files {
        copy_donor_file(&mut donor, &overlay, path, &mut files)?;
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    reject_duplicate_outputs(&files)?;

    let mut scan_roots = vec![manifest.weapon.target_config.clone()];
    if !manifest.ammo.reuse_existing {
        scan_roots.push(manifest.ammo.target_config.clone());
    }
    // Scan the source configs as a report. Generated configs also contain
    // target-era shared paths, which are validated separately below.
    let plan = dependency::scan(
        &mut donor,
        &mut target,
        [
            manifest.weapon.donor_config.clone(),
            manifest.ammo.donor_config.clone(),
        ],
    )?;
    let shared_dependencies =
        validate_generated_references(&overlay, &scan_roots, &mut donor, &mut target)?;

    let record = BuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        weapon_dict_id: manifest.weapon.dict_id,
        ammo_dict_id: manifest.ammo.dict_id,
        files,
        shared_dependencies: shared_dependencies.into_iter().collect(),
        unresolved_references: plan.unresolved_references.into_iter().collect(),
    };
    let record_path = output.join("port-manifest.json");
    let record_file = fs::File::create(&record_path)?;
    to_writer_pretty(record_file, &record)?;
    write_srp_content_manifest(manifest, output)?;
    Ok(record)
}

fn write_srp_content_manifest(manifest: &PortManifest, output: &Path) -> Result<()> {
    let weapon_dict_id = u16::try_from(manifest.weapon.dict_id)
        .context("weapon dictionary ID does not fit the SRP protocol")?;
    let ammo_dict_id = u16::try_from(manifest.ammo.dict_id)
        .context("ammunition dictionary ID does not fit the SRP protocol")?;
    let content = SrpContentManifest {
        format_version: 1,
        extra_weapon_dict_ids: vec![weapon_dict_id],
        extra_ammo_dict_ids: if manifest.ammo.reuse_existing {
            Vec::new()
        } else {
            vec![ammo_dict_id]
        },
        primary_weapon_dict_id: Some(weapon_dict_id),
        primary_ammo_dict_id: Some(ammo_dict_id),
    };
    let server_dir = output.join("server");
    fs::create_dir_all(&server_dir)?;
    let file = fs::File::create(server_dir.join("srp-content.json"))?;
    to_writer_pretty(file, &content)?;
    Ok(())
}

pub fn apply_bundle(
    base_db: &Path,
    bundle: &Path,
    work_dir: &Path,
    output_db: &Path,
    force: bool,
) -> Result<String> {
    if output_db.exists() && !force {
        bail!(
            "output database already exists (use --force to replace): {}",
            output_db.display()
        );
    }
    let tree = work_dir.join("tree");
    if tree.exists() {
        if !force {
            bail!(
                "work tree already exists (use --force to reuse): {}",
                tree.display()
            );
        }
    } else {
        fs::create_dir_all(&tree)?;
        let mut base = ResourceDb::open(base_db)?;
        base.extract_all(&tree)?;
    }
    copy_tree(&bundle.join("overlay"), &tree)?;
    let source = resources_db::pack::build_src_tree(&tree)?;
    let image = resources_db::pack::assemble(&source)?;
    if let Some(parent) = output_db.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_db, &image)?;
    let sha = encode_hex(&Sha256::digest(&image));
    verify_bundle(output_db, bundle)?;
    Ok(sha)
}

pub fn verify_bundle(db_path: &Path, bundle: &Path) -> Result<()> {
    let record: BuildRecord =
        serde_json::from_reader(fs::File::open(bundle.join("port-manifest.json"))?)?;
    let mut db = ResourceDb::open(db_path)?;
    for file in &record.files {
        let actual = db
            .read(&file.path)
            .with_context(|| format!("verifying packed resource {}", file.path))?;
        let sha = encode_hex(&Sha256::digest(&actual));
        if sha != file.sha256 {
            bail!(
                "packed resource hash mismatch for {}: expected {}, got {}",
                file.path,
                file.sha256,
                sha
            );
        }
    }
    Ok(())
}

fn parse_config(db: &mut ResourceDb, path: &str, role: &str) -> Result<BinaryConfig> {
    let bytes = db
        .read(path)
        .with_context(|| format!("reading {role} {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing {role} {path}"))
}

fn apply_patches(
    root: &mut ConfigValue,
    patches: &BTreeMap<String, serde_json::Value>,
) -> Result<()> {
    for (path, value) in patches {
        root.set_path(path, json_value(value).with_context(|| path.clone())?)?;
    }
    Ok(())
}

fn json_value(value: &serde_json::Value) -> Result<ConfigValue> {
    Ok(match value {
        serde_json::Value::String(value) => ConfigValue::String(value.clone()),
        serde_json::Value::Number(value) if value.is_i64() => {
            ConfigValue::Integer(value.as_i64().unwrap().try_into()?)
        }
        serde_json::Value::Number(value) => ConfigValue::Float(
            value
                .as_f64()
                .context("patch number cannot be represented as f64")? as f32,
        ),
        serde_json::Value::Bool(value) => ConfigValue::Boolean(*value),
        serde_json::Value::Array(values) => ConfigValue::Indexed(
            values
                .iter()
                .map(json_value)
                .collect::<Result<Vec<ConfigValue>>>()?,
        ),
        serde_json::Value::Object(_) => {
            bail!("patch values cannot be JSON objects; patch named fields separately")
        }
        serde_json::Value::Null => bail!("patch values cannot be null"),
    })
}

fn add_dictionary_item(
    root: &mut ConfigValue,
    dict_id: u32,
    config_path: &str,
    category: i32,
    stack: bool,
) -> Result<()> {
    let items = root
        .named_mut("items_dict")
        .context("dictionary has no items_dict table")?;
    let ConfigValue::Named(items) = items else {
        bail!("dictionary items_dict is not a named table");
    };
    let key = format!("{dict_id}_");
    for item in items.iter() {
        if item.name == key
            || item.value.named("dict_id") == Some(&ConfigValue::Integer(dict_id as i32))
        {
            bail!("dictionary ID is already occupied: {dict_id}");
        }
    }
    items.push(NamedValue {
        name: key,
        value: ConfigValue::Named(vec![
            named("cfg_name", ConfigValue::String(config_path.to_owned())),
            named("is_stack", ConfigValue::Boolean(stack)),
            named("item_category", ConfigValue::Integer(category)),
            named("dict_id", ConfigValue::Integer(dict_id as i32)),
            named("is_premium", ConfigValue::Boolean(false)),
        ]),
    });
    Ok(())
}

fn validate_dictionary_item(root: &ConfigValue, dict_id: u32, config_path: &str) -> Result<()> {
    let items = root
        .named("items_dict")
        .context("dictionary has no items_dict table")?;
    let ConfigValue::Named(items) = items else {
        bail!("dictionary items_dict is not a named table");
    };
    let expected_id = ConfigValue::Integer(dict_id as i32);
    let item = items
        .iter()
        .find(|item| item.value.named("dict_id") == Some(&expected_id))
        .with_context(|| format!("reused ammunition dictionary ID does not exist: {dict_id}"))?;
    let expected_path = ConfigValue::String(config_path.to_owned());
    if item.value.named("cfg_name") != Some(&expected_path) {
        bail!(
            "reused ammunition ID {dict_id} points to a different config than {config_path}"
        );
    }
    Ok(())
}

fn add_localization_strings(
    root: &mut ConfigValue,
    additions: &BTreeMap<String, String>,
) -> Result<()> {
    let strings = root
        .named_mut("strings")
        .context("localization has no strings table")?;
    let ConfigValue::Named(strings) = strings else {
        bail!("localization strings is not a named table");
    };
    for (name, value) in additions {
        if strings.iter().any(|entry| entry.name == *name) {
            bail!("localization key is already occupied: {name}");
        }
        strings.push(named(name, ConfigValue::String(value.clone())));
    }
    Ok(())
}

fn named(name: impl Into<String>, value: ConfigValue) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}

fn write_generated_config(
    overlay: &Path,
    path: &str,
    config: &BinaryConfig,
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = config.to_bytes()?;
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, &bytes)?;
    files.push(record_file(path, source, &bytes));
    Ok(())
}

fn copy_donor_file(
    donor: &mut ResourceDb,
    overlay: &Path,
    path: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = donor.read(path)?;
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, &bytes)?;
    files.push(record_file(path, "donor", &bytes));
    Ok(())
}

fn record_file(path: &str, source: &str, bytes: &[u8]) -> BuiltFile {
    BuiltFile {
        path: path.to_owned(),
        source: source.to_owned(),
        size: bytes.len() as u64,
        sha256: encode_hex(&Sha256::digest(bytes)),
    }
}

fn reject_duplicate_outputs(files: &[BuiltFile]) -> Result<()> {
    for pair in files.windows(2) {
        if pair[0].path == pair[1].path {
            bail!("multiple build operations emitted {}", pair[0].path);
        }
    }
    Ok(())
}

fn validate_generated_references(
    overlay: &Path,
    roots: &[String],
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
) -> Result<BTreeSet<String>> {
    let mut shared = BTreeSet::new();
    for root in roots {
        let bytes = fs::read(safe_join(overlay, root)?)?;
        let config = BinaryConfig::parse(&bytes)?;
        let mut strings = Vec::new();
        config.root.collect_strings(&mut strings);
        for value in strings {
            let normalized = value.trim_start_matches("resources/");
            if target.contains(normalized) {
                shared.insert(normalized.to_owned());
            } else if donor.contains(normalized) {
                // Donor-only direct references must have been copied into the
                // overlay. Prefix-derived object paths are covered by the
                // explicit copy manifest.
                if !safe_join(overlay, normalized)?.exists() {
                    shared.insert(format!("donor-prefix:{normalized}"));
                }
            }
        }
    }
    Ok(shared)
}

fn prepare_empty_output(output: &Path) -> Result<()> {
    if output.exists() && fs::read_dir(output)?.next().is_some() {
        bail!("bundle output directory is not empty: {}", output.display());
    }
    fs::create_dir_all(output)?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("bundle overlay does not exist: {}", source.display());
    }
    let mut pending = vec![source.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(source)?;
            let output = destination.join(relative);
            if entry.file_type()?.is_dir() {
                fs::create_dir_all(&output)?;
                pending.push(path);
            } else {
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&path, &output)?;
            }
        }
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_values_support_indexed_tables() {
        let value = json_value(&serde_json::json!([1, -1])).unwrap();
        assert_eq!(
            value,
            ConfigValue::Indexed(vec![ConfigValue::Integer(1), ConfigValue::Integer(-1)])
        );
    }
}
