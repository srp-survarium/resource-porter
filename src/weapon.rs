// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::to_writer_pretty;
use sha2::{Digest, Sha256};

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::dependency;
use crate::manifest::{
    BuildRecord, BuiltFile, ComposedBundleRecord, PortManifest, SrpContentManifest,
};

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
    replace_strings(&mut weapon.root, &manifest.weapon.replace_strings);
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

    for generated in &manifest.generated_configs {
        let mut config = parse_config(
            &mut target,
            &generated.template_config,
            "target generated-config template",
        )?;
        parse_config(
            &mut donor,
            &generated.donor_config,
            "donor generated config",
        )?;
        apply_patches(&mut config.root, &generated.patch)?;
        replace_strings(&mut config.root, &generated.replace_strings);
        write_generated_config(
            &overlay,
            &generated.target_config,
            &config,
            "generated:converted-config",
            &mut files,
        )?;
    }

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
    scan_roots.extend(
        manifest
            .generated_configs
            .iter()
            .map(|config| config.target_config.clone()),
    );
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
    #[derive(serde::Deserialize)]
    struct ApplyRecord {
        #[serde(default)]
        target_db_sha256: Option<String>,
        #[serde(default)]
        materialize_target_hard_links: BTreeSet<String>,
    }

    if output_db.exists() && !force {
        bail!(
            "output database already exists (use --force to replace): {}",
            output_db.display()
        );
    }
    let overlay = bundle.join("overlay");
    if !overlay.is_dir() {
        bail!("bundle overlay does not exist: {}", overlay.display());
    }
    let apply_record: ApplyRecord =
        serde_json::from_reader(fs::File::open(bundle.join("port-manifest.json"))?)?;
    if let Some(expected) = &apply_record.target_db_sha256 {
        crate::survarium_026::check_hash(base_db, expected)?;
    }
    crate::model_validation::validate_effective_overlay(base_db, &overlay)
        .context("validating effective model surface/settings contracts before packing")?;
    fs::create_dir_all(work_dir)?;
    let donor_db = work_dir.join("bundle-overlay.db");
    if donor_db.exists() && !force {
        bail!(
            "bundle overlay database already exists (use --force to replace): {}",
            donor_db.display()
        );
    }
    let source = resources_db::pack::build_src_tree(&overlay)?;
    resources_db::pack::assemble_to_file(&source, &donor_db)?;
    if let Some(parent) = output_db.parent() {
        fs::create_dir_all(parent)?;
    }

    let materialized_paths: Vec<Vec<u8>> = apply_record
        .materialize_target_hard_links
        .into_iter()
        .map(String::into_bytes)
        .collect();
    resources_db::overlay::apply_db_overlay_with_materialized_paths(
        base_db,
        &donor_db,
        output_db,
        &materialized_paths,
    )?;
    let mut database = fs::File::open(output_db)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = database.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let sha = encode_hex(&digest.finalize());
    verify_bundle(output_db, bundle)?;
    Ok(sha)
}

pub fn compose_bundles(
    bundles: &[std::path::PathBuf],
    output: &Path,
) -> Result<ComposedBundleRecord> {
    if bundles.is_empty() {
        bail!("at least one input bundle is required");
    }
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    #[derive(serde::Deserialize)]
    struct InputRecord {
        files: Vec<BuiltFile>,
        #[serde(default)]
        materialize_target_hard_links: BTreeSet<String>,
    }

    let mut files = BTreeMap::<String, BuiltFile>::new();
    let mut overwritten_paths = BTreeSet::new();
    let mut owned_model_roots = BTreeSet::<String>::new();
    let mut bundle_names = Vec::with_capacity(bundles.len());
    let mut materialize_target_hard_links = BTreeSet::new();
    for bundle in bundles {
        let bundle_name = bundle
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| bundle.as_os_str().to_str().unwrap_or("<non-utf8-bundle>"))
            .to_owned();
        let record: InputRecord =
            serde_json::from_reader(fs::File::open(bundle.join("port-manifest.json"))?)
                .with_context(|| format!("reading bundle manifest {}", bundle.display()))?;
        materialize_target_hard_links.extend(record.materialize_target_hard_links.iter().cloned());
        let incoming_model_roots: BTreeSet<String> = record
            .files
            .iter()
            .filter_map(|file| model_root(&file.path))
            .collect();
        let accepted_model_roots: BTreeSet<String> = incoming_model_roots
            .into_iter()
            .filter(|root| owned_model_roots.insert(root.clone()))
            .collect();
        // Collision folders of suppressed models that no earlier bundle brought.
        let adopted_collision: BTreeSet<String> = record
            .files
            .iter()
            .filter(|file| {
                model_root(&file.path).is_some_and(|root| !accepted_model_roots.contains(&root))
            })
            .filter_map(|file| collision_folder(&file.path))
            .filter(|folder| {
                !files
                    .range(folder.clone()..)
                    .next()
                    .is_some_and(|(existing, _)| existing.starts_with(folder.as_str()))
            })
            .collect();
        for mut file in record.files {
            let input_path = safe_join(&bundle.join("overlay"), &file.path)?;
            let bytes = fs::read(&input_path)
                .with_context(|| format!("reading bundle resource {}", input_path.display()))?;
            let actual_sha = encode_hex(&Sha256::digest(&bytes));
            if actual_sha != file.sha256 {
                bail!(
                    "bundle resource hash mismatch for {} in {}: expected {}, got {}",
                    file.path,
                    bundle.display(),
                    file.sha256,
                    actual_sha
                );
            }
            if let Some(root) = model_root(&file.path)
                && !accepted_model_roots.contains(&root)
                && !collision_folder(&file.path)
                    .is_some_and(|folder| adopted_collision.contains(&folder))
            {
                overwritten_paths.insert(file.path);
                continue;
            }
            if files.contains_key(&file.path) {
                overwritten_paths.insert(file.path);
                continue;
            }
            let destination = safe_join(&overlay, &file.path)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(destination, &bytes)?;
            file.source = format!("compose:{bundle_name}:{}", file.source);
            files.insert(file.path.clone(), file);
        }
        bundle_names.push(bundle_name);
    }

    let record = ComposedBundleRecord {
        format_version: 1,
        asset_id: "all-versioned-maps-to-0100b".into(),
        bundles: bundle_names,
        overwritten_paths: overwritten_paths.into_iter().collect(),
        materialize_target_hard_links,
        files: files.into_values().collect(),
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

/// Lift already-packed resources back out of a database as a bundle.
///
/// For carrying a map forward from a verified release when its source is not
/// at hand: every file under one of `prefixes` is copied out with its hash, so
/// the result composes and applies like a freshly built bundle.
pub fn extract_bundle(db_path: &Path, prefixes: &[String], output: &Path) -> Result<usize> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut db = ResourceDb::open(db_path)?;
    let paths: Vec<String> = db
        .entries()
        .keys()
        .filter(|path| {
            db.is_file(path)
                && prefixes
                    .iter()
                    .any(|prefix| path.starts_with(prefix.as_str()))
        })
        .cloned()
        .collect();
    if paths.is_empty() {
        bail!("{} holds no file under {prefixes:?}", db_path.display());
    }
    let source = format!("extracted:{}", db_path.display());
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let bytes = db.read(path)?;
        let destination = safe_join(&overlay, path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &bytes)?;
        files.push(record_file(path, &source, &bytes));
    }
    #[derive(serde::Serialize)]
    struct ExtractedRecord {
        format_version: u32,
        asset_id: String,
        files: Vec<BuiltFile>,
    }
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &ExtractedRecord {
            format_version: 1,
            asset_id: "extracted".into(),
            files,
        },
    )?;
    Ok(paths.len())
}

pub fn verify_bundle(db_path: &Path, bundle: &Path) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct VerificationRecord {
        files: Vec<BuiltFile>,
    }
    let record: VerificationRecord =
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
    validate_touched_model_surfaces(&mut db, &record.files)?;
    Ok(())
}

fn validate_touched_model_surfaces(db: &mut ResourceDb, files: &[BuiltFile]) -> Result<()> {
    let model_roots: BTreeSet<String> = files
        .iter()
        .filter_map(|file| model_root(&file.path))
        .collect();
    for root in model_roots {
        let settings_path = format!("{root}/settings");
        if !db.is_file(&settings_path) {
            continue;
        }
        let render_prefix = format!("{root}/render/");
        let actual_surfaces: BTreeSet<String> = db
            .paths_with_prefix(&render_prefix)
            .filter_map(|path| {
                let relative = path.strip_prefix(&render_prefix)?;
                let (surface, leaf) = relative.split_once('/')?;
                (leaf == "converted_model").then(|| surface.to_owned())
            })
            .collect();
        if actual_surfaces.is_empty() {
            continue;
        }

        let settings = BinaryConfig::parse(&db.read(&settings_path)?)
            .with_context(|| format!("parsing model settings {settings_path}"))?;
        let Some(ConfigValue::Named(material_settings)) = settings.root.named("material_settings")
        else {
            continue;
        };
        let expected_surfaces: BTreeSet<String> = material_settings
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        ensure_matching_model_surfaces(&root, &expected_surfaces, &actual_surfaces)?;
    }
    Ok(())
}

/// The `collision` or `walker_collision` folder a model file sits in.
///
/// Collision is its own sub-resource, independent of the surfaces the
/// first-wins rule protects, so a suppressed bundle's collision folder is
/// still taken when no earlier bundle brought that folder. An earlier map that
/// only renders a shared model otherwise leaves a later map's project naming a
/// path with no node, and the client crashes cooking it.
fn collision_folder(path: &str) -> Option<String> {
    let root = model_root(path)?;
    let rest = path[root.len()..].strip_prefix('/')?;
    ["collision/", "walker_collision/"]
        .into_iter()
        .find(|folder| rest.starts_with(folder))
        .map(|folder| format!("{root}/{folder}"))
}

fn model_root(path: &str) -> Option<String> {
    [".skinned_model/", ".model/"]
        .into_iter()
        .find_map(|marker| {
            path.find(marker)
                .map(|offset| path[..offset + marker.len() - 1].to_owned())
        })
}

fn ensure_matching_model_surfaces(
    root: &str,
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    let missing_render_surfaces: Vec<&str> =
        expected.difference(actual).map(String::as_str).collect();
    let stale_render_surfaces: Vec<&str> =
        actual.difference(expected).map(String::as_str).collect();
    bail!(
        "model surface/settings mismatch for {root}: missing render surfaces \
         {missing_render_surfaces:?}, stale render surfaces {stale_render_surfaces:?}"
    )
}

pub(crate) fn parse_config(db: &mut ResourceDb, path: &str, role: &str) -> Result<BinaryConfig> {
    let bytes = db
        .read(path)
        .with_context(|| format!("reading {role} {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing {role} {path}"))
}

pub(crate) fn apply_patches(
    root: &mut ConfigValue,
    patches: &BTreeMap<String, serde_json::Value>,
) -> Result<()> {
    for (path, value) in patches {
        root.set_path(path, json_value(value).with_context(|| path.clone())?)?;
    }
    Ok(())
}

pub(crate) fn replace_strings(value: &mut ConfigValue, replacements: &BTreeMap<String, String>) {
    match value {
        ConfigValue::String(current) => {
            if let Some(replacement) = replacements.get(current) {
                current.clone_from(replacement);
            }
        }
        ConfigValue::Named(values) => {
            for value in values {
                replace_strings(&mut value.value, replacements);
            }
        }
        ConfigValue::Indexed(values) => {
            for value in values {
                replace_strings(value, replacements);
            }
        }
        _ => {}
    }
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

pub(crate) fn add_dictionary_item(
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
        bail!("reused ammunition ID {dict_id} points to a different config than {config_path}");
    }
    Ok(())
}

pub(crate) fn add_localization_strings(
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

pub(crate) fn write_generated_config(
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

pub(crate) fn record_file(path: &str, source: &str, bytes: &[u8]) -> BuiltFile {
    BuiltFile {
        path: path.to_owned(),
        source: source.to_owned(),
        size: bytes.len() as u64,
        sha256: encode_hex(&Sha256::digest(bytes)),
    }
}

pub(crate) fn reject_duplicate_outputs(files: &[BuiltFile]) -> Result<()> {
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

pub(crate) fn prepare_empty_output(output: &Path) -> Result<()> {
    if output.exists() && fs::read_dir(output)?.next().is_some() {
        bail!("bundle output directory is not empty: {}", output.display());
    }
    fs::create_dir_all(output)?;
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

    fn write_test_bundle(root: &Path, files: &[(&str, &[u8])]) {
        let overlay = root.join("overlay");
        fs::create_dir_all(&overlay).unwrap();
        let mut records = Vec::new();
        for (path, bytes) in files {
            let output = safe_join(&overlay, path).unwrap();
            fs::create_dir_all(output.parent().unwrap()).unwrap();
            fs::write(output, bytes).unwrap();
            records.push(record_file(path, "test", bytes));
        }
        fs::write(
            root.join("port-manifest.json"),
            serde_json::to_vec(&serde_json::json!({ "files": records })).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn patch_values_support_indexed_tables() {
        let value = json_value(&serde_json::json!([1, -1])).unwrap();
        assert_eq!(
            value,
            ConfigValue::Indexed(vec![ConfigValue::Integer(1), ConfigValue::Integer(-1)])
        );
    }

    #[test]
    fn extracts_static_and_skinned_model_roots() {
        assert_eq!(
            model_root("models/props/example.model/render/body/converted_model").as_deref(),
            Some("models/props/example.model")
        );
        assert_eq!(
            model_root("models/weapons/example.skinned_model/settings").as_deref(),
            Some("models/weapons/example.skinned_model")
        );
        assert_eq!(model_root("projects/level/client_project"), None);
    }

    #[test]
    fn rejects_stale_render_surfaces_left_by_an_additive_overlay() {
        let expected = BTreeSet::from(["lod1".to_owned(), "body".to_owned()]);
        let actual = BTreeSet::from([
            "lod1".to_owned(),
            "body".to_owned(),
            "removed_stock_surface".to_owned(),
        ]);
        let error =
            ensure_matching_model_surfaces("models/example.model", &expected, &actual).unwrap_err();
        assert!(error.to_string().contains("removed_stock_surface"));
    }

    #[test]
    fn compose_keeps_the_first_complete_model_namespace() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = temp.path().join("first");
        let later = temp.path().join("later");
        let output = temp.path().join("composed");
        write_test_bundle(
            &first,
            &[
                ("models/shared.model/settings", b"first settings"),
                (
                    "models/shared.model/render/body/converted_model",
                    b"first body",
                ),
            ],
        );
        write_test_bundle(
            &later,
            &[
                ("models/shared.model/settings", b"later settings"),
                (
                    "models/shared.model/render/body/converted_model",
                    b"later body",
                ),
                (
                    "models/shared.model/render/lod1/converted_model",
                    b"later-only lod",
                ),
            ],
        );

        let record = compose_bundles(&[first, later], &output).unwrap();

        assert_eq!(
            fs::read(output.join("overlay/models/shared.model/render/body/converted_model"))
                .unwrap(),
            b"first body"
        );
        assert!(
            !output
                .join("overlay/models/shared.model/render/lod1/converted_model")
                .exists()
        );
        assert_eq!(
            record
                .files
                .iter()
                .filter(|file| file.path.starts_with("models/shared.model/"))
                .count(),
            2
        );
    }

    #[test]
    fn compose_keeps_first_bundle_partial_collision_namespace() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = temp.path().join("first");
        let later = temp.path().join("later");
        let output = temp.path().join("composed");
        write_test_bundle(
            &first,
            &[(
                "models/shared.model/collision/exported_primitives",
                b"old collision",
            )],
        );
        write_test_bundle(
            &later,
            &[
                ("models/shared.model/settings", b"later settings"),
                (
                    "models/shared.model/render/body/converted_model",
                    b"later body",
                ),
            ],
        );

        let record = compose_bundles(&[first, later], &output).unwrap();

        assert_eq!(
            fs::read(output.join("overlay/models/shared.model/collision/exported_primitives"))
                .unwrap(),
            b"old collision"
        );
        assert!(!output.join("overlay/models/shared.model/settings").exists());
        assert!(
            record
                .overwritten_paths
                .iter()
                .any(|path| path == "models/shared.model/render/body/converted_model")
        );
    }

    #[test]
    fn compose_adopts_a_later_collision_folder_the_owner_lacks() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = temp.path().join("first");
        let later = temp.path().join("later");
        let output = temp.path().join("composed");
        write_test_bundle(
            &first,
            &[
                ("models/shared.model/settings", b"first settings"),
                (
                    "models/shared.model/render/a/converted_model",
                    b"first body",
                ),
            ],
        );
        write_test_bundle(
            &later,
            &[
                ("models/shared.model/settings", b"later settings"),
                (
                    "models/shared.model/render/b/converted_model",
                    b"later body",
                ),
                ("models/shared.model/collision/vertices", b"later collision"),
            ],
        );

        compose_bundles(&[first, later], &output).unwrap();

        assert_eq!(
            fs::read(output.join("overlay/models/shared.model/settings")).unwrap(),
            b"first settings"
        );
        assert!(!output.join("overlay/models/shared.model/render/b").exists());
        assert_eq!(
            fs::read(output.join("overlay/models/shared.model/collision/vertices")).unwrap(),
            b"later collision"
        );
    }

    #[test]
    fn exact_string_replacements_walk_nested_tables() {
        let mut value = ConfigValue::Named(vec![NamedValue {
            name: "states".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::String("old/path".into())]),
        }]);
        let replacements = BTreeMap::from([("old/path".into(), "new/path".into())]);

        replace_strings(&mut value, &replacements);

        assert_eq!(
            value.named("states"),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "new/path".into()
            )]))
        );
    }
}
