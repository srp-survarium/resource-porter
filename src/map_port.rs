use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::dependency;
use crate::manifest::{
    LobbyVisualSpec, MapBuildRecord, MapPortManifest, PlayableLobbyManifest, StaticModelPortSpec,
    Stk2MapSliceManifest,
};
use crate::material_conversion::MaterialCatalog;
use crate::stk2_content;
use crate::weapon::{prepare_empty_output, record_file, reject_duplicate_outputs};

const STATIC_VERTEX_DECLARATION: [u8; 48] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // position: float3
    0x00, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x03, 0x00, // normal: D3DCOLOR
    0x00, 0x00, 0x10, 0x00, 0x04, 0x00, 0x06, 0x00, // tangent: D3DCOLOR
    0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x07, 0x00, // binormal: D3DCOLOR
    0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x05, 0x00, // uv: float2
    0xff, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, // D3DDECL_END
];

// Verified against the original v0.100b type=2 surface
// models/level/03/antenna/antenna_base.model/render/floor_linoleum_02_s_vc/converted_model.
// The target widens the donor half2 UV to float2 and moves COLOR0 from byte
// offset 28 to byte offset 32, producing a 36-byte colored-static vertex.
const COLORED_STATIC_VERTEX_DECLARATION: [u8; 56] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // position: float3
    0x00, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x03, 0x00, // normal: D3DCOLOR
    0x00, 0x00, 0x10, 0x00, 0x04, 0x00, 0x06, 0x00, // tangent: D3DCOLOR
    0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x07, 0x00, // binormal: D3DCOLOR
    0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x05, 0x00, // uv: float2
    0x00, 0x00, 0x20, 0x00, 0x04, 0x00, 0x0a, 0x00, // color: D3DCOLOR
    0xff, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, // D3DDECL_END
];

pub fn build_bundle(
    manifest: &MapPortManifest,
    donor_db: &Path,
    target_db: &Path,
    output: &Path,
) -> Result<MapBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut donor = ResourceDb::open(donor_db)?;
    let mut target = ResourceDb::open(target_db)?;
    let material_catalog = manifest
        .convert_materials_to_target_schema
        .then(|| MaterialCatalog::load(&mut target))
        .transpose()
        .context("indexing target-build material templates")?;
    let mut dropped_client_objects = std::collections::BTreeMap::<String, usize>::new();
    let mut applied_string_replacements = BTreeMap::<String, usize>::new();
    let mut converted_split_client_project = false;
    let mut dropped_client_object_types = BTreeMap::<String, usize>::new();
    let mut project_overrides = BTreeMap::<String, Vec<u8>>::new();
    for path in &manifest.roots {
        let output_path = manifest
            .path_replacements
            .get(path)
            .map_or(path.as_str(), String::as_str);
        if path.ends_with("/client_project")
            && (output_path != path
                || !manifest.drop_client_objects.is_empty()
                || manifest.convert_split_client_objects_to_legacy)
        {
            let bytes = donor
                .read(path)
                .with_context(|| format!("reading donor client project {path}"))?;
            let transformed = transform_client_project(
                manifest,
                &mut target,
                path,
                output_path,
                &bytes,
                &mut converted_split_client_project,
                &mut dropped_client_object_types,
                &mut dropped_client_objects,
                &mut applied_string_replacements,
            )?;
            project_overrides.insert(path.clone(), transformed);
        }
    }
    let plan = dependency::scan_with_forced_donor_and_overrides(
        &mut donor,
        &mut target,
        manifest.roots.clone(),
        manifest.forced_donor_paths.iter().cloned(),
        &project_overrides,
    )?;
    let forced_paths: Vec<&str> = manifest
        .roots
        .iter()
        .chain(manifest.forced_donor_paths.iter())
        .map(String::as_str)
        .collect();
    for path in &plan.donor_files {
        if target.is_file(path)
            && !forced_paths.iter().any(|root| {
                path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
            })
        {
            bail!(
                "dependency ownership invariant failed: shared target resource {path} \
                 was selected from the donor without an explicit override"
            );
        }
    }
    let unexpected_unresolved: Vec<_> = plan
        .unresolved_references
        .difference(&manifest.allowed_unresolved_references)
        .cloned()
        .collect();
    if !unexpected_unresolved.is_empty() {
        bail!(
            "map dependency scan has unresolved references: {:?}",
            unexpected_unresolved
        );
    }
    let unused_allowances: Vec<_> = manifest
        .allowed_unresolved_references
        .difference(&plan.unresolved_references)
        .cloned()
        .collect();
    if !unused_allowances.is_empty() {
        bail!(
            "map manifest permits references that are no longer unresolved: {:?}",
            unused_allowances
        );
    }

    let mut files = Vec::with_capacity(plan.donor_files.len() + manifest.target_copies.len());
    let mut converted_static_models = Vec::new();
    let mut converted_materials = std::collections::BTreeMap::new();
    let mut applied_material_overrides = BTreeSet::<(String, String)>::new();
    for path in &plan.donor_files {
        let transformed_project = project_overrides.get(path);
        let mut bytes = transformed_project.cloned().unwrap_or(donor.read(path)?);
        let mut source = format!("donor:{}", manifest.asset.donor_build);
        if transformed_project.is_some() {
            source.push_str("+transformed-client-project");
        }
        if path.starts_with("material_instances/") && path.ends_with(".material") {
            if let Some(catalog) = &material_catalog {
                let converted = catalog
                    .convert(
                        path,
                        &bytes,
                        manifest
                            .material_template_overrides
                            .get(path)
                            .map(String::as_str),
                        &donor,
                        &target,
                    )
                    .with_context(|| format!("converting map material {path}"))?;
                source = format!(
                    "{}+target-material-template:{}",
                    source, converted.template_path
                );
                converted_materials.insert(path.clone(), converted.template_path);
                bytes = converted.bytes;
            }
        }
        if transformed_project.is_none()
            && !manifest.config_string_replacements.is_empty()
            && let Ok(mut config) = BinaryConfig::parse(&bytes)
        {
            let replacements = replace_config_strings(
                &mut config.root,
                &manifest.config_string_replacements,
                &mut applied_string_replacements,
            );
            if replacements != 0 {
                bytes = config.to_bytes()?;
            }
        }
        if let Some(overrides) = manifest.model_material_overrides.get(path) {
            let mut settings = BinaryConfig::parse(&bytes)
                .with_context(|| format!("parsing overridden model settings {path}"))?;
            for (surface, material) in overrides {
                let target_material = format!("material_instances/{material}.material");
                if !target.is_file(&target_material) {
                    bail!(
                        "model material override {path}:{surface} references missing target material {target_material}"
                    );
                }
                set_model_surface_material(&mut settings.root, path, surface, material)?;
                applied_material_overrides.insert((path.clone(), surface.clone()));
            }
            bytes = settings.to_bytes()?;
        }
        if manifest.upgrade_stripped_static_vertex_declarations
            && path.contains(".model/")
            && !path.contains(".skinned_model/")
            && path.ends_with("/converted_model")
        {
            match upgrade_stripped_static_model(&bytes)
                .with_context(|| format!("upgrading static map model {path}"))?
            {
                StaticUpgrade::AlreadyCompatible => {}
                StaticUpgrade::Converted(converted) => {
                    bytes = converted;
                    converted_static_models.push(path.clone());
                }
            }
        }
        let output_path = manifest
            .path_replacements
            .get(path)
            .map_or(path.as_str(), String::as_str);
        write_bundle_resource(&overlay, output_path, &bytes, &source, &mut files)?;
    }
    if manifest.convert_split_client_objects_to_legacy && !converted_split_client_project {
        bail!("map requested split client-object conversion but emitted no client project");
    }
    let missing_object_type_drops: Vec<_> = manifest
        .drop_client_object_types
        .iter()
        .filter(|kind| !dropped_client_object_types.contains_key(*kind))
        .cloned()
        .collect();
    if !missing_object_type_drops.is_empty() {
        bail!(
            "declared client object types were not found and therefore not dropped: {:?}",
            missing_object_type_drops
        );
    }
    let unused_string_replacements: Vec<_> = manifest
        .config_string_replacements
        .keys()
        .filter(|source| !applied_string_replacements.contains_key(*source))
        .cloned()
        .collect();
    if !unused_string_replacements.is_empty() {
        bail!(
            "map config string replacements did not match any emitted donor config: {:?}",
            unused_string_replacements
        );
    }
    let unused_material_overrides: Vec<_> = manifest
        .model_material_overrides
        .iter()
        .flat_map(|(path, surfaces)| {
            surfaces
                .keys()
                .map(move |surface| (path.clone(), surface.clone()))
        })
        .filter(|entry| !applied_material_overrides.contains(entry))
        .collect();
    if !unused_material_overrides.is_empty() {
        bail!(
            "map model material overrides were not applied: {:?}",
            unused_material_overrides
        );
    }
    let missing_drops: Vec<_> = manifest
        .drop_client_objects
        .iter()
        .filter(|name| !dropped_client_objects.contains_key(*name))
        .cloned()
        .collect();
    if !missing_drops.is_empty() {
        bail!(
            "declared client scene objects were not found and therefore not dropped: {:?}",
            missing_drops
        );
    }
    for (source_path, output_path) in &manifest.target_copies {
        let bytes = target
            .read(source_path)
            .with_context(|| format!("reading target-side map resource {source_path}"))?;
        write_bundle_resource(
            &overlay,
            output_path,
            &bytes,
            &format!("target-copy:{source_path}"),
            &mut files,
        )?;
    }
    for settings_path in crate::model_validation::normalize_overlay_model_settings(&overlay)
        .context("normalizing donor model settings to emitted render surfaces")?
    {
        let output = safe_join(&overlay, &settings_path)?;
        let bytes = fs::read(&output)
            .with_context(|| format!("reading normalized model settings {}", output.display()))?;
        let file = files
            .iter_mut()
            .find(|file| file.path == settings_path)
            .with_context(|| {
                format!("normalized settings are absent from bundle record: {settings_path}")
            })?;
        let source = format!("{}+normalized-surface-contract", file.source);
        *file = record_file(&settings_path, &source, &bytes);
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;
    crate::model_validation::validate_effective_overlay(target_db, &overlay)
        .context("validating map bundle compatibility against the target database")?;

    let record = MapBuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        map_name: manifest.map_name.clone(),
        roots: manifest
            .roots
            .iter()
            .map(|root| {
                manifest
                    .path_replacements
                    .get(root)
                    .cloned()
                    .unwrap_or_else(|| root.clone())
            })
            .collect(),
        spawn_position: manifest.spawn_position,
        spawn_orientation: manifest.spawn_orientation,
        files,
        shared_dependencies: plan.target_files.into_iter().collect(),
        converted_static_models,
        converted_materials,
        dropped_client_objects: dropped_client_objects.keys().cloned().collect(),
        allowed_unresolved_references: plan.unresolved_references.into_iter().collect(),
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
fn transform_client_project(
    manifest: &MapPortManifest,
    target: &mut ResourceDb,
    path: &str,
    output_path: &str,
    bytes: &[u8],
    converted_split_client_project: &mut bool,
    dropped_client_object_types: &mut BTreeMap<String, usize>,
    dropped_client_objects: &mut BTreeMap<String, usize>,
    applied_string_replacements: &mut BTreeMap<String, usize>,
) -> Result<Vec<u8>> {
    let mut project = BinaryConfig::parse(bytes)
        .with_context(|| format!("parsing remapped client project {path}"))?;
    if manifest.convert_split_client_objects_to_legacy {
        convert_split_client_objects_to_legacy(&mut project.root, path)?;
        *converted_split_client_project = true;
        drop_client_object_types(
            &mut project.root,
            path,
            &manifest.drop_client_object_types,
            dropped_client_object_types,
        )?;
        convert_environment_probe_contracts(&mut project.root, path)?;
        if let Some(post_effect) = &manifest.legacy_environment_post_effect {
            let material_path = format!("material_instances/{post_effect}.material");
            if !target.contains(&material_path) {
                bail!(
                    "{path} legacy environment post effect is absent from the target DB: \
                     {material_path}"
                );
            }
        }
        convert_environment_contracts(
            &mut project.root,
            path,
            manifest.legacy_environment_post_effect.as_deref(),
        )?;
    }
    if let Some(template_path) = &manifest.legacy_client_project_template {
        let mut template = BinaryConfig::parse(
            &target
                .read(template_path)
                .with_context(|| format!("reading legacy client template {template_path}"))?,
        )
        .with_context(|| format!("parsing legacy client template {template_path}"))?;
        copy_project_sections(
            &project.root,
            &mut template.root,
            path,
            &manifest.legacy_client_project_sections,
        )?;
        project = template;
    }
    if !manifest.config_string_replacements.is_empty() {
        replace_config_strings(
            &mut project.root,
            &manifest.config_string_replacements,
            applied_string_replacements,
        );
    }
    // A declared drop removes an object the target cannot satisfy, so it must
    // happen before that object's contracts are checked.
    drop_named_objects(
        &mut project.root,
        &manifest.drop_client_objects,
        dropped_client_objects,
    );
    convert_object_sound_contracts(&mut project.root, path, |sound_name| {
        classify_target_sound_resource(target, sound_name)
    })?;
    if output_path != path && project.root.named("project_name").is_some() {
        project.root.set_path(
            "project_name",
            ConfigValue::String(manifest.map_name.clone()),
        )?;
    }
    if manifest.camera_at_spawn {
        crate::soc::build::set_camera_at_spawn(&mut project.root, manifest.spawn_position)
            .with_context(|| format!("placing the {path} camera at the spawn"))?;
    }
    project.to_bytes()
}

fn convert_split_client_objects_to_legacy(root: &mut ConfigValue, path: &str) -> Result<()> {
    let ConfigValue::Named(fields) = root else {
        bail!("{path} client project root is not a named table");
    };
    if fields
        .iter()
        .any(|field| field.name == "objects" || field.name == "respawn_points")
    {
        bail!("{path} already contains legacy objects or respawn_points");
    }

    let server_index = fields
        .iter()
        .position(|field| field.name == "server_objects")
        .with_context(|| format!("{path} has no split server_objects table"))?;
    let server_objects = fields.remove(server_index).value;
    let client_index = fields
        .iter()
        .position(|field| field.name == "client_objects")
        .with_context(|| format!("{path} has no split client_objects table"))?;
    let client_objects = fields.remove(client_index).value;

    let ConfigValue::Named(server_fields) = server_objects else {
        bail!("{path} server_objects is not a named table");
    };
    let respawn_points = server_fields
        .into_iter()
        .find(|field| field.name == "respawn_points")
        .with_context(|| format!("{path} server_objects has no respawn_points"))?
        .value;
    if !matches!(respawn_points, ConfigValue::Indexed(_)) {
        bail!("{path} server_objects.respawn_points is not an indexed table");
    }
    if !matches!(client_objects, ConfigValue::Indexed(_)) {
        bail!("{path} client_objects is not an indexed table");
    }

    fields.push(NamedValue {
        name: "respawn_points".into(),
        value: respawn_points,
    });
    fields.push(NamedValue {
        name: "objects".into(),
        value: client_objects,
    });
    Ok(())
}

fn drop_client_object_types(
    root: &mut ConfigValue,
    path: &str,
    dropped_types: &BTreeSet<String>,
    dropped: &mut BTreeMap<String, usize>,
) -> Result<()> {
    if dropped_types.is_empty() {
        return Ok(());
    }
    let ConfigValue::Indexed(objects) = root
        .named_mut("objects")
        .with_context(|| format!("{path} has no legacy objects table after conversion"))?
    else {
        bail!("{path} legacy objects is not an indexed table");
    };
    objects.retain(|object| {
        let Some(ConfigValue::String(kind)) = object.named("game_object_type") else {
            return true;
        };
        if dropped_types.contains(kind) {
            *dropped.entry(kind.clone()).or_default() += 1;
            false
        } else {
            true
        }
    });
    Ok(())
}

fn convert_environment_probe_contracts(root: &mut ConfigValue, path: &str) -> Result<usize> {
    let ConfigValue::Indexed(objects) = root
        .named_mut("objects")
        .with_context(|| format!("{path} has no legacy objects table after conversion"))?
    else {
        bail!("{path} legacy objects is not an indexed table");
    };

    let mut converted = 0;
    for (object_index, object) in objects.iter_mut().enumerate() {
        if !matches!(
            object.named("game_object_type"),
            Some(ConfigValue::String(kind)) if kind == "environment_probe"
        ) {
            continue;
        }
        let ConfigValue::Named(fields) = object else {
            bail!("{path} environment probe object {object_index} is not a named table");
        };
        if let Some(index_position) = fields
            .iter()
            .position(|field| field.name == "cubemap_resolution_index")
        {
            if fields
                .iter()
                .any(|field| field.name == "cubemap_resolution")
            {
                bail!(
                    "{path} environment probe object {object_index} contains both \
                     cubemap_resolution_index and cubemap_resolution"
                );
            }
            let index_field = &mut fields[index_position];
            let ConfigValue::Integer(index) = index_field.value else {
                bail!(
                    "{path} environment probe object {object_index} \
                     cubemap_resolution_index is not an integer"
                );
            };
            let resolution = match index {
                2 => 128,
                3 => 256,
                _ => bail!(
                    "{path} environment probe object {object_index} uses unproven \
                     cubemap_resolution_index {index}"
                ),
            };
            index_field.name = "cubemap_resolution".into();
            index_field.value = ConfigValue::Integer(resolution);
        }

        // v0.23 removed these fields from serialized probes, while v0.100b
        // unconditionally reads both. Supplying the legacy defaults avoids the
        // old binary-config reader spinning on a same-CRC field when a required
        // name is absent.
        for name in ["clip_by_normal", "with_shadows"] {
            match fields.iter().find(|field| field.name == name) {
                Some(NamedValue {
                    value: ConfigValue::Boolean(_),
                    ..
                }) => {}
                Some(_) => bail!(
                    "{path} environment probe object {object_index} field {name} \
                     is not a boolean"
                ),
                None => fields.push(NamedValue {
                    name: name.into(),
                    value: ConfigValue::Boolean(false),
                }),
            }
        }
        converted += 1;
    }
    Ok(converted)
}

fn convert_environment_contracts(
    root: &mut ConfigValue,
    path: &str,
    legacy_post_effect: Option<&str>,
) -> Result<usize> {
    let ConfigValue::Indexed(objects) = root
        .named_mut("objects")
        .with_context(|| format!("{path} has no legacy objects table after conversion"))?
    else {
        bail!("{path} legacy objects is not an indexed table");
    };

    let mut converted = 0;
    for (object_index, object) in objects.iter_mut().enumerate() {
        if !matches!(
            object.named("game_object_type"),
            Some(ConfigValue::String(kind)) if kind == "environment"
        ) {
            continue;
        }
        let ConfigValue::Named(fields) = object else {
            bail!("{path} environment object {object_index} is not a named table");
        };
        let has_legacy_post_effect = match fields.iter().find(|field| field.name == "post_effect") {
            Some(NamedValue {
                value: ConfigValue::String(_),
                ..
            }) => true,
            Some(_) => {
                bail!("{path} environment object {object_index} field post_effect is not a string")
            }
            None => false,
        };
        if !has_legacy_post_effect {
            let post_effect = legacy_post_effect.with_context(|| {
                format!(
                    "{path} environment object {object_index} has no v0.100b post_effect; \
                     declare legacy_environment_post_effect"
                )
            })?;
            fields.push(NamedValue {
                name: "post_effect".into(),
                value: ConfigValue::String(post_effect.into()),
            });
            converted += 1;
        }

        // This table is the newer replacement for post_effect. v0.100b does
        // not consume it, and retaining it would make dependency discovery
        // import incompatible resources that are unreachable at runtime.
        fields.retain(|field| field.name != "properties");
    }
    Ok(converted)
}

fn convert_object_sound_contracts(
    root: &mut ConfigValue,
    path: &str,
    mut classify: impl FnMut(&str) -> Result<i32>,
) -> Result<usize> {
    let ConfigValue::Indexed(objects) = root
        .named_mut("objects")
        .with_context(|| format!("{path} has no legacy objects table after conversion"))?
    else {
        bail!("{path} legacy objects is not an indexed table");
    };

    let mut converted = 0;
    for (object_index, object) in objects.iter_mut().enumerate() {
        if !matches!(
            object.named("game_object_type"),
            Some(ConfigValue::String(kind)) if kind == "object_sound"
        ) {
            continue;
        }
        let sound_name = match object.named("sound_name") {
            Some(ConfigValue::String(name)) => name.clone(),
            Some(_) => bail!("{path} sound object {object_index} sound_name is not a string"),
            None => bail!("{path} sound object {object_index} has no sound_name"),
        };
        let expected_resource_id = classify(&sound_name)
            .with_context(|| format!("{path} sound object {object_index} ({sound_name})"))?;
        let ConfigValue::Named(fields) = object else {
            bail!("{path} sound object {object_index} is not a named table");
        };
        match fields.iter_mut().find(|field| field.name == "resource_id") {
            Some(NamedValue {
                value: ConfigValue::Integer(resource_id),
                ..
            }) => {
                if *resource_id != expected_resource_id {
                    *resource_id = expected_resource_id;
                    converted += 1;
                }
            }
            Some(_) => bail!("{path} sound object {object_index} resource_id is not an integer"),
            None => {
                fields.push(NamedValue {
                    name: "resource_id".into(),
                    value: ConfigValue::Integer(expected_resource_id),
                });
                converted += 1;
            }
        }
    }
    Ok(converted)
}

fn classify_target_sound_resource(target: &ResourceDb, sound_name: &str) -> Result<i32> {
    let single_base = format!("sounds/single/{sound_name}");
    let single_payload = target.is_file(&format!("{single_base}.high.ogg"));
    let single_options = target.is_file(&format!("{single_base}.single_sound_options"));
    let collection = target.is_file(&format!("sounds/collections/{sound_name}.sound_collection"));

    match (single_payload, single_options, collection) {
        (true, true, false) => Ok(256),
        (false, false, true) => Ok(258),
        (false, false, false) => {
            bail!("target has neither a complete single sound nor a sound collection")
        }
        (payload, options, false) => {
            bail!("target single sound is incomplete (payload={payload}, options={options})")
        }
        (_, _, true) => {
            bail!("target sound is ambiguous: both single-sound and collection forms exist")
        }
    }
}

fn copy_project_sections(
    donor: &ConfigValue,
    target: &mut ConfigValue,
    donor_path: &str,
    sections: &BTreeSet<String>,
) -> Result<()> {
    for section in sections {
        let value = donor
            .named(section)
            .with_context(|| format!("{donor_path} has no project section {section}"))?
            .clone();
        target
            .set_path(section, value)
            .with_context(|| format!("legacy client template has no section {section}"))?;
    }
    Ok(())
}

fn replace_config_strings(
    value: &mut ConfigValue,
    replacements: &BTreeMap<String, String>,
    applied: &mut BTreeMap<String, usize>,
) -> usize {
    match value {
        ConfigValue::String(text) => {
            if let Some(replacement) = replacements.get(text) {
                *applied.entry(text.clone()).or_default() += 1;
                *text = replacement.clone();
                1
            } else {
                0
            }
        }
        ConfigValue::Named(values) => values
            .iter_mut()
            .map(|child| replace_config_strings(&mut child.value, replacements, applied))
            .sum(),
        ConfigValue::Indexed(values) => values
            .iter_mut()
            .map(|child| replace_config_strings(child, replacements, applied))
            .sum(),
        _ => 0,
    }
}

fn set_model_surface_material(
    root: &mut ConfigValue,
    settings_path: &str,
    surface: &str,
    material: &str,
) -> Result<()> {
    let ConfigValue::Named(material_settings) = root
        .named_mut("material_settings")
        .with_context(|| format!("{settings_path} has no material_settings table"))?
    else {
        bail!("{settings_path} material_settings is not a named table");
    };
    let surface_settings = material_settings
        .iter_mut()
        .find(|entry| entry.name == surface)
        .with_context(|| format!("{settings_path} has no material settings for {surface}"))?;
    let material_name = surface_settings
        .value
        .named_mut("material_name")
        .with_context(|| format!("{settings_path}:{surface} has no material_name"))?;
    *material_name = ConfigValue::String(material.to_owned());
    Ok(())
}

fn drop_named_objects(
    value: &mut ConfigValue,
    names: &std::collections::BTreeSet<String>,
    dropped: &mut std::collections::BTreeMap<String, usize>,
) {
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                drop_named_objects(&mut child.value, names, dropped);
            }
        }
        ConfigValue::Indexed(values) => {
            values.retain(|child| {
                let object_name = child.named("name").and_then(|value| match value {
                    ConfigValue::String(name) => Some(name.as_str()),
                    _ => None,
                });
                let Some(name) = object_name else {
                    return true;
                };
                if names.contains(name) {
                    *dropped.entry(name.to_owned()).or_default() += 1;
                    false
                } else {
                    true
                }
            });
            for child in values {
                drop_named_objects(child, names, dropped);
            }
        }
        _ => {}
    }
}

pub fn build_playable_lobby_bundle(
    manifest: &PlayableLobbyManifest,
    source_db: &Path,
    target_db: &Path,
    output: &Path,
) -> Result<MapBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut source = ResourceDb::open(source_db)?;
    let mut target = ResourceDb::open(target_db)?;
    let plan = dependency::scan(
        &mut source,
        &mut target,
        vec![manifest.source_client_project.clone()],
    )?;
    if !plan.unresolved_references.is_empty() {
        bail!(
            "playable lobby dependency scan has unresolved references: {:?}",
            plan.unresolved_references
        );
    }

    let mut files = Vec::with_capacity(plan.donor_files.len() + 1);
    for path in &plan.donor_files {
        if path == &manifest.source_client_project {
            continue;
        }
        let bytes = source.read(path)?;
        write_bundle_resource(
            &overlay,
            path,
            &bytes,
            &format!("source:{}", manifest.asset.donor_build),
            &mut files,
        )?;
    }

    let mut client = BinaryConfig::parse(&source.read(&manifest.source_client_project)?)
        .context("parsing playable lobby source client project")?;
    let collision_objects = client
        .root
        .named("collision_objects")
        .cloned()
        .context("playable lobby source has no collision_objects table")?;
    client.root.set_path(
        "project_name",
        ConfigValue::String(manifest.map_name.clone()),
    )?;
    write_bundle_resource(
        &overlay,
        &manifest.target_client_project,
        &client.to_bytes()?,
        "generated:playable-lobby-client-project",
        &mut files,
    )?;

    let mut server = BinaryConfig::parse(&target.read(&manifest.server_template_project)?)
        .context("parsing playable lobby server project template")?;
    let server_objects = server
        .root
        .named_mut("objects")
        .context("server project template has no objects table")?;
    server_objects.set_path("collision_objects", collision_objects)?;
    clear_category(server_objects, "collision_geometry")?;
    clear_category(server_objects, "damage_zones")?;
    replace_positioned_category(
        server_objects,
        "respawn_points",
        &manifest.team_spawns,
        Some(manifest.spawn_orientation),
        true,
    )?;
    if matches!(
        server_objects.named("team_base_points"),
        Some(ConfigValue::Indexed(templates)) if !templates.is_empty()
    ) {
        replace_positioned_category(
            server_objects,
            "team_base_points",
            &manifest.team_spawns,
            None,
            true,
        )?;
    } else {
        clear_category(server_objects, "team_base_points")?;
    }
    replace_positioned_category(
        server_objects,
        "victory_item_spawners",
        &manifest.victory_item_spawners,
        None,
        false,
    )?;
    replace_positioned_category(
        server_objects,
        "victory_items_containers",
        &manifest.victory_item_containers,
        None,
        true,
    )?;
    write_bundle_resource(
        &overlay,
        &manifest.target_server_project,
        &server.to_bytes()?,
        "generated:playable-lobby-server-project",
        &mut files,
    )?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;
    crate::model_validation::validate_effective_overlay(target_db, &overlay)
        .context("validating playable-lobby compatibility against the target database")?;
    let record = MapBuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        map_name: manifest.map_name.clone(),
        roots: vec![
            manifest.target_client_project.clone(),
            manifest.target_server_project.clone(),
        ],
        spawn_position: manifest.team_spawns[0],
        spawn_orientation: manifest.spawn_orientation,
        files,
        shared_dependencies: plan.target_files.into_iter().collect(),
        converted_static_models: Vec::new(),
        converted_materials: std::collections::BTreeMap::new(),
        dropped_client_objects: Vec::new(),
        allowed_unresolved_references: Vec::new(),
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

pub fn build_stk2_map_slice_bundle(
    manifest: &Stk2MapSliceManifest,
    source_root: &Path,
    sound_donor_db: Option<&Path>,
    target_db: &Path,
    output: &Path,
) -> Result<MapBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut target = ResourceDb::open(target_db)?;
    let target_game_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let mut files = Vec::new();
    let (models, mut visuals, texture_plans, source_objects) =
        derive_stk2_project_content(manifest, source_root, output)?;
    // Perform scene-schema projection before expensive texture/model
    // conversion so an incompatible editor object fails the build quickly.
    let mut client = BinaryConfig::parse(&target.read(&manifest.client_template_project)?)
        .context("parsing STK2 map-slice client project template")?;
    if manifest.convert_source_lights {
        replace_stk2_client_lighting(
            &mut client.root,
            &source_objects,
            manifest.source_origin,
            manifest.source_light_types.as_ref(),
            manifest.drop_target_environment_probes,
            &manifest.target_client_project,
        )?;
    }
    if manifest.convert_source_environment {
        convert_stk2_environment(
            manifest,
            source_root,
            &source_objects,
            &mut target,
            &mut client,
            &overlay,
            &mut files,
        )?;
    }
    add_stk2_ambient_sounds(
        manifest,
        sound_donor_db,
        &mut target,
        &mut client,
        &overlay,
        &mut files,
    )?;
    let terrain_cells = manifest
        .terrain_directory
        .as_deref()
        .map(|directory| stk2_content::load_stk2_terrain_cells(source_root, directory))
        .transpose()?
        .unwrap_or_default();
    let mut all_texture_plans = BTreeMap::new();
    for plan in texture_plans
        .into_iter()
        .chain(stk2_content::terrain_texture_plans(
            &terrain_cells,
            &manifest.map_name,
            source_root,
            &manifest.terrain_texture_aliases,
            &manifest.terrain_texture_binding_overrides,
        )?)
    {
        if let Some(existing) = all_texture_plans.insert(plan.target_material.clone(), plan.clone())
            && (existing.source_tga != plan.source_tga
                || existing.source_options != plan.source_options
                || existing.target_texture != plan.target_texture)
        {
            bail!(
                "STK2 target material {} has conflicting source plans",
                plan.target_material
            );
        }
    }
    let terrain_texture_names = stk2_content::terrain_texture_names(&terrain_cells);
    let stale_terrain_aliases: Vec<_> = manifest
        .terrain_texture_aliases
        .keys()
        .filter(|texture| !terrain_texture_names.contains(*texture))
        .collect();
    if !stale_terrain_aliases.is_empty() {
        bail!("unused STK2 terrain_texture_aliases entries: {stale_terrain_aliases:?}");
    }
    if !all_texture_plans.is_empty() {
        stk2_content::build_map_materials_and_textures(
            &all_texture_plans.into_values().collect::<Vec<_>>(),
            manifest
                .material_template
                .as_deref()
                .or_else(|| {
                    manifest
                        .material_templates
                        .as_ref()
                        .map(|templates| templates.opaque.as_str())
                })
                .unwrap(),
            manifest.texture_options_template.as_deref().unwrap(),
            source_root,
            &mut target,
            &overlay,
            &mut files,
        )?;
    }
    for model in &models {
        let collision_game_material = if manifest.convert_model_collision {
            let name = manifest
                .static_collision_game_material
                .as_deref()
                .context("validated STK2 static collision material is absent")?;
            let id = *target_game_materials.get(name).with_context(|| {
                format!("unknown target v0.100b static collision game material {name:?}")
            })?;
            Some((name, id))
        } else {
            None
        };
        stk2_content::build_static_model(
            &manifest.static_model,
            model,
            collision_game_material,
            source_root,
            &mut target,
            &overlay,
            &mut files,
        )?;
        if manifest.convert_model_collision {
            stk2_content::build_static_model_collision(model, source_root, &overlay, &mut files)?;
        }
    }
    if !terrain_cells.is_empty() {
        let terrain_game_material = manifest
            .terrain_collision_game_material
            .as_deref()
            .context("validated STK2 terrain collision material is absent")?;
        let terrain_game_material_id =
            *target_game_materials
                .get(terrain_game_material)
                .with_context(|| {
                    format!(
                        "unknown target v0.100b terrain collision game material {terrain_game_material:?}"
                    )
                })?;
        visuals.extend(stk2_content::build_stk2_terrain_models(
            &terrain_cells,
            &manifest.map_name,
            manifest.source_origin,
            &manifest.static_model,
            (terrain_game_material, terrain_game_material_id),
            &mut target,
            &overlay,
            &mut files,
        )?);
    }

    let mut collision_objects = client
        .root
        .named("collision_objects")
        .cloned()
        .context("STK2 map-slice client template has no collision_objects table")?;
    let ConfigValue::Indexed(template_objects) = &collision_objects else {
        bail!("STK2 map-slice client collision_objects is not indexed");
    };
    let collision_template = template_objects
        .first()
        .cloned()
        .context("STK2 map-slice client template has no collision object template")?;
    let mut generated_collisions = Vec::new();
    if let Some(scale) = manifest.flat_collision_scale {
        let mut ground = collision_template.clone();
        ground.set_path(
            "lib_name",
            ConfigValue::String("resources/models/test/mattest_cube01.model/collision".into()),
        )?;
        ground.set_path("position", ConfigValue::Float3([0.0, -1.0, 0.0]))?;
        ground.set_path("rotation", ConfigValue::Float3([0.0, 0.0, 0.0]))?;
        ground.set_path("scale", ConfigValue::Float3(scale))?;
        generated_collisions.push(ground);
    }
    if manifest.convert_model_collision {
        for visual in &visuals {
            let mut collision = collision_template.clone();
            collision.set_path(
                "lib_name",
                ConfigValue::String(format!(
                    "resources/models/{}.model/collision",
                    visual.library_name
                )),
            )?;
            collision.set_path("position", ConfigValue::Float3(visual.position))?;
            collision.set_path("rotation", ConfigValue::Float3(visual.rotation))?;
            collision.set_path("scale", ConfigValue::Float3(visual.scale))?;
            generated_collisions.push(collision);
        }
    }
    collision_objects = ConfigValue::Indexed(generated_collisions);
    client
        .root
        .set_path("collision_objects", collision_objects.clone())?;
    client.root.set_path(
        "project_name",
        ConfigValue::String(manifest.map_name.clone()),
    )?;
    client.root.set_path(
        "render_visuals",
        ConfigValue::Indexed(
            visuals
                .iter()
                .map(|visual| {
                    ConfigValue::Named(vec![
                        NamedValue {
                            name: "rotation".into(),
                            value: ConfigValue::Float3(visual.rotation),
                        },
                        NamedValue {
                            name: "position".into(),
                            value: ConfigValue::Float3(visual.position),
                        },
                        NamedValue {
                            name: "lib_name".into(),
                            value: ConfigValue::String(visual.library_name.clone()),
                        },
                        NamedValue {
                            name: "scale".into(),
                            value: ConfigValue::Float3(visual.scale),
                        },
                    ])
                })
                .collect(),
        ),
    )?;
    write_bundle_resource(
        &overlay,
        &manifest.target_client_project,
        &client.to_bytes()?,
        "generated:STK2-derived-client-project",
        &mut files,
    )?;

    let mut server = BinaryConfig::parse(&target.read(&manifest.server_template_project)?)
        .context("parsing STK2 map-slice server project template")?;
    let server_objects = server
        .root
        .named_mut("objects")
        .context("STK2 map-slice server template has no objects table")?;
    server_objects.set_path("collision_objects", collision_objects)?;
    clear_category(server_objects, "collision_geometry")?;
    clear_category(server_objects, "damage_zones")?;
    replace_positioned_category(
        server_objects,
        "respawn_points",
        &manifest.team_spawns,
        Some(manifest.spawn_orientation),
        true,
    )?;
    if matches!(
        server_objects.named("team_base_points"),
        Some(ConfigValue::Indexed(templates)) if !templates.is_empty()
    ) {
        replace_positioned_category(
            server_objects,
            "team_base_points",
            &manifest.team_spawns,
            None,
            true,
        )?;
    } else {
        clear_category(server_objects, "team_base_points")?;
    }
    replace_positioned_category(
        server_objects,
        "victory_item_spawners",
        &manifest.victory_item_spawners,
        None,
        false,
    )?;
    replace_positioned_category(
        server_objects,
        "victory_items_containers",
        &manifest.victory_item_containers,
        None,
        true,
    )?;
    write_bundle_resource(
        &overlay,
        &manifest.target_server_project,
        &server.to_bytes()?,
        "generated:STK2-derived-server-project",
        &mut files,
    )?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;
    crate::model_validation::validate_effective_overlay(target_db, &overlay)
        .context("validating STK2 map-slice compatibility against the target database")?;
    let record = MapBuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        map_name: manifest.map_name.clone(),
        roots: vec![
            manifest.target_client_project.clone(),
            manifest.target_server_project.clone(),
        ],
        spawn_position: manifest.team_spawns[0],
        spawn_orientation: manifest.spawn_orientation,
        files,
        shared_dependencies: vec![
            manifest.client_template_project.clone(),
            manifest.server_template_project.clone(),
        ],
        converted_static_models: models
            .iter()
            .map(|model| model.target_model.clone())
            .chain(terrain_cells.iter().map(|cell| {
                format!(
                    "models/level/{}/terrain/{}.model",
                    manifest.map_name, cell.name
                )
            }))
            .collect(),
        converted_materials: std::collections::BTreeMap::new(),
        dropped_client_objects: Vec::new(),
        allowed_unresolved_references: Vec::new(),
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

#[derive(Clone, Copy)]
enum Stk2EnvironmentValue {
    Boolean(bool),
    Number(f32),
    Float3([f32; 3]),
    Float4([f32; 4]),
}

fn convert_stk2_environment(
    manifest: &Stk2MapSliceManifest,
    source_root: &Path,
    source_objects: &[crate::stk2_project::ProjectObject],
    target: &mut ResourceDb,
    client: &mut BinaryConfig,
    overlay: &Path,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let environments = source_objects
        .iter()
        .filter(|object| object.kind == "environment")
        .collect::<Vec<_>>();
    if environments.len() != 1 {
        bail!(
            "{} source environment conversion requires exactly one environment object, found {}",
            manifest.target_client_project,
            environments.len()
        );
    }
    let source_environment = environments[0];
    let source_post_effect = match source_environment.properties.get("post_effect") {
        Some(crate::stk2_project::ProjectProperty::String(value)) => value,
        Some(_) => bail!("STK2 environment post_effect is not a string"),
        None => bail!("STK2 environment has no post_effect"),
    };
    let source_instance_path =
        format!("resources/sources/material_instances/{source_post_effect}.material");
    let source_instance = fs::read_to_string(safe_join(source_root, &source_instance_path)?)
        .with_context(|| format!("reading STK2 environment {source_instance_path}"))?;
    let base_material = lua_assignment_string_value(&source_instance, "base_material")
        .context("STK2 environment material has no base_material")?;
    let source_base_path = format!("resources/sources/materials/{base_material}.material");
    let source_base = fs::read_to_string(safe_join(source_root, &source_base_path)?)
        .with_context(|| format!("reading STK2 environment base {source_base_path}"))?;

    let template_path = manifest
        .environment_material_template
        .as_deref()
        .context("validated environment material template is absent")?;
    let target_path = manifest
        .target_environment_material
        .as_deref()
        .context("validated target environment material is absent")?;
    if target.is_file(target_path) {
        bail!("target environment material already exists: {target_path}");
    }
    let mut material = BinaryConfig::parse(&target.read(template_path)?)
        .with_context(|| format!("parsing target environment template {template_path}"))?;

    for field in [
        "adaptation_speed",
        "environment_ambient_color",
        "environment_far_fog_color",
        "environment_far_fog_distance",
        "environment_near_fog_distance",
        "environment_skylight_lower_color",
        "environment_skylight_lower_power",
        "environment_skylight_upper_color",
        "environment_skylight_upper_power",
        "environment_sun_color",
        "environment_use_skycolor",
        "frame_desaturation",
        "frame_height_lights",
        "frame_mid_tones",
        "frame_shadows",
        "use_color_grading_lut",
    ] {
        let value = parse_stk2_environment_value(&source_instance, field)
            .or_else(|| parse_stk2_environment_value(&source_base, field))
            .with_context(|| format!("STK2 environment property {field:?} is absent"))?;
        set_target_environment_value(&mut material.root, field, value)?;
    }
    let source_lut = parse_stk2_environment_string(&source_instance, "color_grading_texture")
        .or_else(|| parse_stk2_environment_string(&source_base, "color_grading_texture"))
        .context("STK2 environment property \"color_grading_texture\" is absent")?;
    let target_lut = manifest
        .target_environment_color_grading_texture
        .as_deref()
        .context("validated target environment color-grading texture is absent")?;
    convert_stk2_color_grading_lut(
        source_root,
        &source_lut,
        target,
        manifest
            .environment_color_grading_template
            .as_deref()
            .context("validated environment color-grading template is absent")?,
        target_lut,
        overlay,
        files,
    )?;
    material.root.set_path(
        "material.post_process.color_grading_texture.value",
        ConfigValue::String(target_lut.to_owned()),
    )?;
    write_bundle_resource(
        overlay,
        target_path,
        &material.to_bytes()?,
        &format!(
            "generated:target-schema-STK2-environment:{source_post_effect}:template:{template_path}"
        ),
        files,
    )?;

    let project_post_effect = target_path
        .strip_prefix("material_instances/")
        .and_then(|path| path.strip_suffix(".material"))
        .with_context(|| {
            format!(
                "target environment material must be material_instances/<name>.material: {target_path}"
            )
        })?;
    let ConfigValue::Indexed(objects) = client
        .root
        .named_mut("objects")
        .context("target client project has no objects table")?
    else {
        bail!("target client project objects table is not indexed");
    };
    let mut target_environments = objects
        .iter_mut()
        .filter(|object| project_object_kind(object) == Some("environment"))
        .collect::<Vec<_>>();
    if target_environments.len() != 1 {
        bail!(
            "{} target template requires exactly one environment object, found {}",
            manifest.client_template_project,
            target_environments.len()
        );
    }
    target_environments[0].set_path(
        "post_effect",
        ConfigValue::String(project_post_effect.to_owned()),
    )?;
    Ok(())
}

fn lua_assignment_string_value(text: &str, name: &str) -> Option<String> {
    let marker = format!("{name} = \"");
    let tail = text.split_once(&marker)?.1;
    Some(tail.split_once('"')?.0.replace('\\', "/"))
}

fn stk2_lua_property_block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("{name} ="))?;
    let tail = &text[start..];
    let end = tail.find(&format!("-- {name}"))?;
    Some(&tail[..end])
}

fn parse_stk2_environment_value(text: &str, name: &str) -> Option<Stk2EnvironmentValue> {
    let block = stk2_lua_property_block(text, name)?;
    let raw = block.split_once("value =")?.1.trim_start();
    if raw.starts_with("true") {
        return Some(Stk2EnvironmentValue::Boolean(true));
    }
    if raw.starts_with("false") {
        return Some(Stk2EnvironmentValue::Boolean(false));
    }
    if let Some(raw) = raw.strip_prefix("float3(") {
        let raw = raw.split_once(')')?.0;
        let values = raw
            .split(',')
            .map(|value| value.trim().parse::<f32>().ok())
            .collect::<Option<Vec<_>>>()?;
        if values.len() == 3 && values.iter().all(|value| value.is_finite()) {
            return Some(Stk2EnvironmentValue::Float3([
                values[0], values[1], values[2],
            ]));
        }
        return None;
    }
    if let Some(raw) = raw.strip_prefix("float4(") {
        let raw = raw.split_once(')')?.0;
        let values = raw
            .split(',')
            .map(|value| value.trim().parse::<f32>().ok())
            .collect::<Option<Vec<_>>>()?;
        if values.len() == 4 && values.iter().all(|value| value.is_finite()) {
            return Some(Stk2EnvironmentValue::Float4([
                values[0], values[1], values[2], values[3],
            ]));
        }
        return None;
    }
    let token = raw
        .split(|character: char| character == ',' || character.is_whitespace())
        .next()?;
    let value = token.parse::<f32>().ok()?;
    value
        .is_finite()
        .then_some(Stk2EnvironmentValue::Number(value))
}

fn set_target_environment_value(
    root: &mut ConfigValue,
    field: &str,
    value: Stk2EnvironmentValue,
) -> Result<()> {
    let path = format!("material.post_process.{field}.value");
    let current = config_value_at_path(root, &path)
        .with_context(|| format!("target environment template has no {path}"))?;
    let replacement = match (current, value) {
        (ConfigValue::Boolean(_), Stk2EnvironmentValue::Boolean(value)) => {
            ConfigValue::Boolean(value)
        }
        (ConfigValue::Integer(_), Stk2EnvironmentValue::Number(value))
            if value.fract() == 0.0 && value >= i32::MIN as f32 && value <= i32::MAX as f32 =>
        {
            ConfigValue::Integer(value as i32)
        }
        // Lua material values do not have a declared scalar type: integral
        // defaults cook as config integers even when the shader parameter is
        // real-valued. Preserve a fractional donor value by promoting that
        // self-describing node instead of rounding it back to the template.
        (ConfigValue::Integer(_), Stk2EnvironmentValue::Number(value)) => ConfigValue::Float(value),
        (ConfigValue::Float(_), Stk2EnvironmentValue::Number(value)) => ConfigValue::Float(value),
        (ConfigValue::Float3(_), Stk2EnvironmentValue::Float3(value)) => ConfigValue::Float3(value),
        (ConfigValue::Float4(_), Stk2EnvironmentValue::Float4(value)) => ConfigValue::Float4(value),
        (target, _) => bail!(
            "STK2 environment field {field:?} cannot be projected onto target type {target:?}"
        ),
    };
    root.set_path(&path, replacement)
}

fn parse_stk2_environment_string(text: &str, name: &str) -> Option<String> {
    let block = stk2_lua_property_block(text, name)?;
    let raw = block.split_once("value =")?.1.trim_start();
    let raw = raw.strip_prefix('"')?;
    Some(raw.split_once('"')?.0.replace('\\', "/"))
}

fn convert_stk2_color_grading_lut(
    source_root: &Path,
    source_texture: &str,
    target: &mut ResourceDb,
    template_dds_path: &str,
    target_texture: &str,
    overlay: &Path,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let source_tga_path = format!("resources/sources/textures/{source_texture}.tga");
    let source_tga = fs::read(safe_join(source_root, &source_tga_path)?)
        .with_context(|| format!("reading STK2 color-grading LUT {source_tga_path}"))?;
    let payload = stk2_color_grading_payload(&source_tga)
        .with_context(|| format!("converting STK2 color-grading LUT {source_tga_path}"))?;

    let mut template_dds = target
        .read(template_dds_path)
        .with_context(|| format!("reading target color-grading template {template_dds_path}"))?;
    validate_target_color_grading_template(&template_dds, template_dds_path)?;
    let payload_end = 128 + payload.len();
    template_dds[128..payload_end].copy_from_slice(&payload);
    // Resource DDS payloads carry one trailing sRGB policy byte.
    template_dds[payload_end] = 0;

    let target_dds_path = format!("textures/{target_texture}.dds");
    let target_options_path = format!("textures/{target_texture}.options");
    if target.is_file(&target_dds_path) || target.is_file(&target_options_path) {
        bail!("target color-grading texture already exists: {target_texture}");
    }
    write_bundle_resource(
        overlay,
        &target_dds_path,
        &template_dds,
        &format!("converted:STK2-volume-LUT:{source_tga_path}:template:{template_dds_path}"),
        files,
    )?;

    let template_options_path = format!(
        "{}.options",
        template_dds_path
            .strip_suffix(".dds")
            .context("environment_color_grading_template must end in .dds")?
    );
    let template_options = target.read(&template_options_path).with_context(|| {
        format!("reading target color-grading options template {template_options_path}")
    })?;
    write_bundle_resource(
        overlay,
        &target_options_path,
        &template_options,
        &format!("target-template:{template_options_path}"),
        files,
    )?;
    Ok(())
}

fn validate_target_color_grading_template(bytes: &[u8], label: &str) -> Result<()> {
    const PAYLOAD_SIZE: usize = 16 * 16 * 16 * 4 + 16 * 8 * 8 * 4 + 16 * 4 * 4 * 4;
    if bytes.len() != 128 + PAYLOAD_SIZE + 1
        || bytes.get(..4) != Some(b"DDS ")
        || read_u32(bytes, 4)? != 124
        || read_u32(bytes, 12)? != 16
        || read_u32(bytes, 16)? != 16
        || read_u32(bytes, 24)? != 16
        || read_u32(bytes, 28)? != 3
        || read_u32(bytes, 84)? != 0
        || read_u32(bytes, 88)? != 32
    {
        bail!(
            "{label}: expected an uncompressed 16x16x16 BGRA8 DDS volume with three fixed-depth mips"
        );
    }
    Ok(())
}

fn stk2_color_grading_payload(tga: &[u8]) -> Result<Vec<u8>> {
    if tga.len() < 18 {
        bail!("TGA header is truncated");
    }
    let id_length = usize::from(tga[0]);
    if tga[1] != 0 || tga[2] != 2 {
        bail!("only uncompressed true-color TGA LUTs are supported");
    }
    let width = usize::from(u16::from_le_bytes([tga[12], tga[13]]));
    let height = usize::from(u16::from_le_bytes([tga[14], tga[15]]));
    let channels = match tga[16] {
        24 => 3,
        32 => 4,
        depth => bail!("unsupported TGA LUT pixel depth {depth}"),
    };
    if width != 256 || height != 16 {
        bail!("STK2 color-grading LUT must be a 256x16 strip, got {width}x{height}");
    }
    let offset = 18_usize
        .checked_add(id_length)
        .context("TGA LUT pixel offset overflow")?;
    let byte_count = width
        .checked_mul(height)
        .and_then(|count| count.checked_mul(channels))
        .context("TGA LUT payload size overflow")?;
    let pixels = tga
        .get(offset..offset + byte_count)
        .context("TGA LUT pixel payload is truncated")?;
    let origin_top = tga[17] & 0x20 != 0;
    let origin_right = tga[17] & 0x10 != 0;

    // Convert the 16 horizontally packed 16x16 slices into canonical
    // top-left BGRA8 images before producing fixed-depth 2D mip levels.
    let mut slices = vec![vec![[0_u8; 4]; 16 * 16]; 16];
    for file_y in 0..16 {
        let y = if origin_top { file_y } else { 15 - file_y };
        for file_x in 0..256 {
            let x = if origin_right { 255 - file_x } else { file_x };
            let source = (file_y * 256 + file_x) * channels;
            let slice = x / 16;
            let slice_x = x % 16;
            slices[slice][y * 16 + slice_x] = [
                pixels[source],
                pixels[source + 1],
                pixels[source + 2],
                if channels == 4 {
                    pixels[source + 3]
                } else {
                    255
                },
            ];
        }
    }

    let mut payload = Vec::with_capacity(16 * (16 * 16 + 8 * 8 + 4 * 4) * 4);
    let mut level = slices;
    for size in [16_usize, 8, 4] {
        for slice in &level {
            for pixel in slice {
                payload.extend_from_slice(pixel);
            }
        }
        if size > 4 {
            level = level
                .iter()
                .map(|slice| {
                    let next_size = size / 2;
                    let mut next = vec![[0_u8; 4]; next_size * next_size];
                    for y in 0..next_size {
                        for x in 0..next_size {
                            for channel in 0..4 {
                                let sum = u32::from(slice[(y * 2) * size + x * 2][channel])
                                    + u32::from(slice[(y * 2) * size + x * 2 + 1][channel])
                                    + u32::from(slice[(y * 2 + 1) * size + x * 2][channel])
                                    + u32::from(slice[(y * 2 + 1) * size + x * 2 + 1][channel]);
                                next[y * next_size + x][channel] =
                                    u8::try_from((sum + 2) / 4).unwrap();
                            }
                        }
                    }
                    next
                })
                .collect();
        }
    }
    Ok(payload)
}

fn config_value_at_path<'a>(root: &'a ConfigValue, path: &str) -> Option<&'a ConfigValue> {
    let mut current = root;
    for component in path.split('.') {
        current = current.named(component)?;
    }
    Some(current)
}

fn add_stk2_ambient_sounds(
    manifest: &Stk2MapSliceManifest,
    sound_donor_db: Option<&Path>,
    target: &mut ResourceDb,
    client: &mut BinaryConfig,
    overlay: &Path,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    if manifest.ambient_sounds.is_empty() {
        return Ok(());
    }
    let donor_path = sound_donor_db.context(
        "STK2 map manifest declares ambient_sounds but --sound-donor-db was not supplied",
    )?;
    let mut donor = ResourceDb::open(donor_path)
        .with_context(|| format!("opening ambient sound donor {}", donor_path.display()))?;

    for sound in &manifest.ambient_sounds {
        let donor_ogg_path = format!("sounds/single/{}.high.ogg", sound.donor_sound);
        let target_ogg_path = format!("sounds/single/{}.high.ogg", sound.target_sound);
        let target_options_path =
            format!("sounds/single/{}.single_sound_options", sound.target_sound);
        if target.is_file(&target_ogg_path) || target.is_file(&target_options_path) {
            bail!(
                "STK2 ambient target {:?} already exists in the target database",
                sound.target_sound
            );
        }
        let ogg = donor
            .read(&donor_ogg_path)
            .with_context(|| format!("reading donor ambient sound {donor_ogg_path}"))?;
        if !ogg.starts_with(b"OggS") {
            bail!("donor ambient payload {donor_ogg_path} is not an Ogg stream");
        }
        write_bundle_resource(
            overlay,
            &target_ogg_path,
            &ogg,
            &format!("donor-survarium:{donor_ogg_path}"),
            files,
        )?;

        let options = target.read(&sound.options_template).with_context(|| {
            format!(
                "reading target ambient options template {}",
                sound.options_template
            )
        })?;
        BinaryConfig::parse(&options).with_context(|| {
            format!(
                "validating target ambient options template {}",
                sound.options_template
            )
        })?;
        write_bundle_resource(
            overlay,
            &target_options_path,
            &options,
            &format!("target-template:{}", sound.options_template),
            files,
        )?;

        let mut template_project = BinaryConfig::parse(
            &target.read(&sound.object_template_project)?,
        )
        .with_context(|| {
            format!(
                "parsing ambient object template project {}",
                sound.object_template_project
            )
        })?;
        let template_objects = template_project
            .root
            .named_mut("objects")
            .with_context(|| {
                format!(
                    "ambient object template project {} has no objects table",
                    sound.object_template_project
                )
            })?;
        let ConfigValue::Indexed(template_objects) = template_objects else {
            bail!(
                "ambient object template project {} objects table is not indexed",
                sound.object_template_project
            );
        };
        let mut object = template_objects
            .iter()
            .find(|object| {
                matches!(
                    object.named("game_object_type"),
                    Some(ConfigValue::String(kind)) if kind == "object_sound"
                ) && matches!(
                    object.named("sound_name"),
                    Some(ConfigValue::String(name)) if name == &sound.object_template_sound
                )
            })
            .cloned()
            .with_context(|| {
                format!(
                    "project {} has no object_sound for {:?}",
                    sound.object_template_project, sound.object_template_sound
                )
            })?;
        object.set_path(
            "sound_name",
            ConfigValue::String(sound.target_sound.clone()),
        )?;
        object.set_path("position", ConfigValue::Float3(sound.position))?;
        object.set_path("name", ConfigValue::String(sound.object_name.clone()))?;
        object.set_path(
            "full_name",
            ConfigValue::String(format!("sounds/{}", sound.target_sound)),
        )?;
        object.set_path("resource_id", ConfigValue::Integer(256))?;

        let objects = client.root.named_mut("objects").with_context(|| {
            format!(
                "{} client template has no objects table for ambient insertion",
                manifest.target_client_project
            )
        })?;
        let ConfigValue::Indexed(objects) = objects else {
            bail!(
                "{} client objects table is not indexed",
                manifest.target_client_project
            );
        };
        objects.push(object);
    }
    Ok(())
}

fn replace_stk2_client_lighting(
    root: &mut ConfigValue,
    source_objects: &[crate::stk2_project::ProjectObject],
    source_origin: [f32; 3],
    source_light_types: Option<&BTreeSet<i32>>,
    drop_target_environment_probes: bool,
    path: &str,
) -> Result<()> {
    let ConfigValue::Indexed(template_objects) = root
        .named("objects")
        .cloned()
        .with_context(|| format!("{path} client template has no objects table"))?
    else {
        bail!("{path} client template objects table is not indexed");
    };

    let mut light_templates = BTreeMap::<i32, Vec<ConfigValue>>::new();
    for object in &template_objects {
        if project_object_kind(object) != Some("light") {
            continue;
        }
        let light_type = match object.named("light_type") {
            Some(ConfigValue::Integer(value)) => *value,
            Some(_) => bail!("{path} target light template has non-integer light_type"),
            None => bail!("{path} target light template has no light_type"),
        };
        light_templates
            .entry(light_type)
            .or_default()
            .push(object.clone());
    }

    let mut output = Vec::new();
    let mut dropped_environment_probes = 0_usize;
    for object in template_objects {
        if drop_target_environment_probes
            && project_object_kind(&object) == Some("environment_probe")
        {
            dropped_environment_probes += 1;
            continue;
        }
        if project_object_kind(&object) != Some("light") {
            output.push(object);
            continue;
        }
        let light_type = match object.named("light_type") {
            Some(ConfigValue::Integer(value)) => *value,
            Some(_) => bail!("{path} target light template has non-integer light_type"),
            None => bail!("{path} target light template has no light_type"),
        };
        if source_light_types.is_some_and(|types| !types.contains(&light_type)) {
            output.push(object);
        }
    }
    if drop_target_environment_probes && dropped_environment_probes == 0 {
        bail!("{path} requested target environment-probe removal but template has no probes");
    }

    let source_lights = source_objects
        .iter()
        .filter(|object| object.kind == "light")
        .filter(|object| {
            source_light_types.is_none_or(|types| {
                matches!(
                    object.properties.get("light_type"),
                    Some(crate::stk2_project::ProjectProperty::Integer(value))
                        if types.contains(value)
                )
            })
        })
        .collect::<Vec<_>>();
    if source_lights.is_empty() {
        bail!("{path} source-light conversion was requested but no lights were parsed");
    }
    for source in source_lights {
        let light_type = match source.properties.get("light_type") {
            Some(crate::stk2_project::ProjectProperty::Integer(value)) => *value,
            Some(_) => bail!("STK2 light {} has non-integer light_type", source.id),
            None => bail!("STK2 light {} has no light_type", source.id),
        };
        let candidates = light_templates.get(&light_type).with_context(|| {
            format!(
                "{path} target client template has no light schema for STK2 light_type {light_type}"
            )
        })?;
        let mut ranked = candidates
            .iter()
            .map(|candidate| {
                (
                    project_property_incompatibilities(candidate, source),
                    candidate,
                )
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|(incompatibilities, _)| incompatibilities.len());
        let (incompatibilities, template) = &ranked[0];
        if !incompatibilities.is_empty() {
            bail!(
                "STK2 light {} cannot be represented by the target light_type {light_type} schemas \
                 (incompatible source properties: {incompatibilities:?})",
                source.id
            );
        }
        let mut light = (*template).clone();
        project_stk2_properties(&mut light, source, path)?;
        light.set_path(
            "position",
            ConfigValue::Float3([
                source.position[0] - source_origin[0],
                source.position[1] - source_origin[1],
                source.position[2] - source_origin[2],
            ]),
        )?;
        if light.named("full_name").is_some() {
            let name = match source.properties.get("name") {
                Some(crate::stk2_project::ProjectProperty::String(name)) => name.as_str(),
                _ => source.id.as_str(),
            };
            light.set_path(
                "full_name",
                ConfigValue::String(format!("environment_and_lights/{name}")),
            )?;
        }
        output.push(light);
    }

    root.set_path("objects", ConfigValue::Indexed(output))
        .with_context(|| format!("replacing {path} target client lighting objects"))
}

fn project_object_kind(object: &ConfigValue) -> Option<&str> {
    match object.named("game_object_type") {
        Some(ConfigValue::String(kind)) => Some(kind),
        _ => None,
    }
}

fn project_property_incompatibilities(
    template: &ConfigValue,
    source: &crate::stk2_project::ProjectObject,
) -> Vec<String> {
    let ConfigValue::Named(fields) = template else {
        return vec!["<template-is-not-named>".into()];
    };
    fields
        .iter()
        .filter(|field| !preserve_target_light_default(&field.name))
        .filter_map(|field| {
            source
                .properties
                .get(&field.name)
                .map(|source| (&field.name, &field.value, source))
        })
        .filter(|(_, target, source)| project_property_value(target, source).is_none())
        .map(|(name, _, source)| format!("{name}={source:?}"))
        .collect()
}

fn project_stk2_properties(
    target: &mut ConfigValue,
    source: &crate::stk2_project::ProjectObject,
    path: &str,
) -> Result<()> {
    let ConfigValue::Named(fields) = target else {
        bail!("{path} target project object template is not a named table");
    };
    for field in fields {
        if preserve_target_light_default(&field.name) {
            continue;
        }
        let Some(source_value) = source.properties.get(&field.name) else {
            continue;
        };
        field.value = project_property_value(&field.value, source_value).with_context(|| {
            format!(
                "{path} STK2 object {} property {:?} cannot be projected onto target type {:?}",
                source.id, field.name, field.value
            )
        })?;
    }
    Ok(())
}

fn preserve_target_light_default(name: &str) -> bool {
    // STK2 serializes a tiny floating depth bias (commonly 0.0001), whereas
    // v0.100b stores an integer policy value here. This is a semantic schema
    // change, not a numeric unit conversion; retain the reviewed old template
    // value until the renderer contract is recovered.
    // Newer STK2 likewise stores fractional shadow transparency while the
    // old client stores an integer policy. Both target fields therefore keep
    // the selected v0.100b template default.
    matches!(
        name,
        "lighting_model"
            | "z_bias"
            | "shadow_transparency"
            | "shadow_map_size_index"
            | "sun_shadow_map_size"
            | "num_sun_cascades"
            | "is_cast_shadow"
            | "is_cast_shadow_neg_x"
            | "is_cast_shadow_neg_y"
            | "is_cast_shadow_neg_z"
            | "is_cast_shadow_x"
            | "is_cast_shadow_y"
            | "is_cast_shadow_z"
            | "is_shadower"
            | "static_shadows"
            | "use_with_lpv"
    )
}

fn project_property_value(
    target: &ConfigValue,
    source: &crate::stk2_project::ProjectProperty,
) -> Option<ConfigValue> {
    use crate::stk2_project::ProjectProperty;
    match (target, source) {
        (ConfigValue::Boolean(_), ProjectProperty::Boolean(value)) => {
            Some(ConfigValue::Boolean(*value))
        }
        (ConfigValue::Integer(_), ProjectProperty::Integer(value)) => {
            Some(ConfigValue::Integer(*value))
        }
        (ConfigValue::Integer(_), ProjectProperty::Float(value))
            if value.fract() == 0.0 && *value >= i32::MIN as f32 && *value <= i32::MAX as f32 =>
        {
            Some(ConfigValue::Integer(*value as i32))
        }
        (ConfigValue::Float(_), ProjectProperty::Integer(value)) => {
            Some(ConfigValue::Float(*value as f32))
        }
        (ConfigValue::Float(_), ProjectProperty::Float(value)) => Some(ConfigValue::Float(*value)),
        (ConfigValue::String(_), ProjectProperty::String(value)) => {
            Some(ConfigValue::String(value.clone()))
        }
        (ConfigValue::Float3(_), ProjectProperty::Float3(value)) => {
            Some(ConfigValue::Float3(*value))
        }
        _ => None,
    }
}

#[derive(Serialize)]
struct Stk2ProjectBuildReport {
    source_project: String,
    parsed: crate::stk2_project::ProjectParseReport,
    direct_solid_visuals: usize,
    composite_solid_visuals: usize,
    converted_instances: usize,
    converted_models: usize,
    converted_lights: usize,
    source_texture_bindings: BTreeMap<String, Vec<String>>,
    empty_source_texture_bindings: Vec<String>,
    unresolved_source_texture_bindings: BTreeMap<String, Vec<String>>,
    omitted_object_kinds: BTreeMap<String, usize>,
    omitted_libraries: BTreeMap<String, String>,
}

fn derive_stk2_project_content(
    manifest: &Stk2MapSliceManifest,
    source_root: &Path,
    output: &Path,
) -> Result<(
    Vec<StaticModelPortSpec>,
    Vec<LobbyVisualSpec>,
    Vec<stk2_content::Stk2MapTexturePlan>,
    Vec<crate::stk2_project::ProjectObject>,
)> {
    let Some(source_project) = &manifest.source_project else {
        return Ok((
            manifest.models.clone(),
            manifest.visuals.clone(),
            Vec::new(),
            Vec::new(),
        ));
    };
    let project_path = safe_join(source_root, source_project)?;
    let composite_library = safe_join(
        source_root,
        "resources/sources/library/composite/composite.library",
    )?;
    let (objects, parsed) = if composite_library.is_file() {
        crate::stk2_project::parse_project_with_composites(&project_path, &composite_library)?
    } else {
        crate::stk2_project::parse_project(&project_path)?
    };
    if !parsed.unresolved_composite_children.is_empty() {
        bail!(
            "STK2 project contains {} unresolved composite children; first: {}",
            parsed.unresolved_composite_children.len(),
            parsed.unresolved_composite_children[0]
        );
    }
    let direct_solid_visuals = parsed
        .object_kinds
        .get("solid_visual")
        .copied()
        .unwrap_or_default();
    let composite_solid_visuals = parsed
        .composite_child_kinds
        .get("solid_visual")
        .copied()
        .unwrap_or_default();
    let libraries: BTreeSet<String> = objects
        .iter()
        .filter(|object| object.kind == "solid_visual")
        .filter_map(|object| object.library_name.clone())
        .collect();
    let material = manifest.auto_material.as_deref().unwrap();
    let target_prefix = format!("level/{}", manifest.map_name);
    let mut models = manifest.models.clone();
    let mut target_libraries = BTreeMap::<String, String>::new();
    let mut omitted_libraries = BTreeMap::new();
    let mut source_texture_bindings = BTreeMap::<String, Vec<String>>::new();
    let mut empty_source_texture_bindings = Vec::new();
    let mut unresolved_source_texture_bindings = BTreeMap::<String, Vec<String>>::new();
    let mut texture_plans = BTreeMap::<String, stk2_content::Stk2MapTexturePlan>::new();
    let source_material_catalog = manifest
        .material_templates
        .as_ref()
        .map(|_| stk2_content::Stk2SourceMaterialCatalog::load(source_root))
        .transpose()?;
    let mut used_fallback_source_textures = BTreeSet::new();
    let mut used_fallback_empty_surfaces = BTreeSet::new();
    let mut used_surface_material_overrides = BTreeSet::new();
    let mut used_source_material_template_overrides = BTreeSet::new();
    let mut used_model_archive_overrides = BTreeSet::new();
    let mut used_omitted_model_libraries = BTreeSet::new();

    for library in libraries {
        if !is_safe_stk2_library_name(&library) {
            omitted_libraries.insert(library, "unsafe library path".into());
            continue;
        }
        let default_archive_resource = format!("resources/exported/models/{library}.model");
        let archive_resource = if let Some(archive) = manifest.model_archive_overrides.get(&library)
        {
            used_model_archive_overrides.insert(library.clone());
            archive.clone()
        } else if safe_join(source_root, &default_archive_resource)?.is_file() {
            default_archive_resource
        } else {
            let original_archive_resource =
                format!("resources/exported/models/{library}.model.orig");
            if safe_join(source_root, &original_archive_resource)?.is_file() {
                original_archive_resource
            } else {
                default_archive_resource
            }
        };
        let archive = safe_join(source_root, &archive_resource)?;
        if !archive.is_file() {
            let reason = if let Some(reason) = manifest.omitted_model_libraries.get(&library) {
                used_omitted_model_libraries.insert(library.clone());
                reason.clone()
            } else {
                "exported model is absent".into()
            };
            omitted_libraries.insert(library, reason);
            continue;
        }
        let surface_textures = stk2_content::discover_static_model_surface_textures(&archive)
            .with_context(|| format!("validating active STK2 model library {library}"))?;
        let mut surface_materials = BTreeMap::new();
        for (surface, texture) in &surface_textures {
            let binding = format!("{library}#render/{surface}");
            if texture.raw.is_empty() {
                used_fallback_empty_surfaces.insert(binding.clone());
                empty_source_texture_bindings.push(binding);
                surface_materials.insert(surface.clone(), material.to_owned());
            } else if let Some(texture) = &texture.canonical {
                source_texture_bindings
                    .entry(texture.clone())
                    .or_default()
                    .push(binding.clone());
                let source_tga_lower = format!("resources/sources/textures/{texture}.tga");
                let source_tga_upper = format!("resources/sources/textures/{texture}.TGA");
                let source_tga = if safe_join(source_root, &source_tga_lower)?.is_file() {
                    Some(source_tga_lower)
                } else if safe_join(source_root, &source_tga_upper)?.is_file() {
                    Some(source_tga_upper)
                } else {
                    None
                };
                let source_options = format!("resources/sources/textures/{texture}.options");
                if let Some(source_tga) = source_tga
                    && safe_join(source_root, &source_options)?.is_file()
                {
                    let target_root = format!("ported/stk2/{}/{}", manifest.map_name, texture);
                    let source_material_override =
                        manifest.surface_material_overrides.get(&binding);
                    let resolved_material = if let Some(source_material_override) =
                        source_material_override
                    {
                        used_surface_material_overrides.insert(binding.clone());
                        let templates = manifest.material_templates.as_ref().with_context(|| {
                                format!(
                                    "surface material override {binding:?} requires material_templates"
                                )
                            })?;
                        Some(
                            source_material_catalog
                                .as_ref()
                                .context("STK2 source material catalog is absent")?
                                .resolve_exact(texture, source_material_override, templates)?,
                        )
                    } else {
                        manifest.material_templates.as_ref().and_then(|templates| {
                            source_material_catalog
                                .as_ref()
                                .and_then(|catalog| catalog.resolve(texture, templates))
                        })
                    };
                    let target_material =
                        if let Some(source_material_override) = source_material_override {
                            format!(
                                "ported/stk2/{}/source_materials/{}",
                                manifest.map_name, source_material_override
                            )
                        } else {
                            target_root.clone()
                        };
                    let resolved_source_material = resolved_material
                        .as_ref()
                        .map(|resolved| resolved.source_material.clone());
                    let (mut material_template, source_material, auxiliary_textures) =
                        if let Some(resolved) = resolved_material {
                            let auxiliary_textures = stk2_content::auxiliary_texture_plans(
                                &resolved,
                                &manifest.map_name,
                                source_root,
                            )?;
                            (
                                Some(resolved.template),
                                Some(resolved.source),
                                auxiliary_textures,
                            )
                        } else if let Some(templates) = &manifest.material_templates {
                            (
                                Some(templates.opaque.clone()),
                                Some("unresolved-or-ambiguous:opaque-fallback".to_owned()),
                                Vec::new(),
                            )
                        } else {
                            (None, None, Vec::new())
                        };
                    if let Some(source_material) = source_material_override
                        .map(String::as_str)
                        .or(resolved_source_material.as_deref())
                        && let Some(template) = manifest
                            .source_material_template_overrides
                            .get(source_material)
                    {
                        used_source_material_template_overrides.insert(source_material.to_owned());
                        material_template = Some(template.clone());
                    }
                    let plan = stk2_content::Stk2MapTexturePlan {
                        source_texture: texture.clone(),
                        source_tga,
                        source_options,
                        target_texture: target_root.clone(),
                        target_material: target_material.clone(),
                        material_template,
                        source_material,
                        auxiliary_textures,
                        target_texture_bindings: BTreeMap::new(),
                    };
                    if let Some(existing) =
                        texture_plans.insert(target_material.clone(), plan.clone())
                        && (existing.target_texture != plan.target_texture
                            || existing.source_tga != plan.source_tga
                            || existing.source_options != plan.source_options
                            || existing.material_template != plan.material_template
                            || existing.source_material != plan.source_material
                            || existing.auxiliary_textures.len() != plan.auxiliary_textures.len())
                    {
                        bail!("STK2 target material {target_material} received conflicting plans");
                    }
                    surface_materials.insert(surface.clone(), target_material);
                } else {
                    used_fallback_source_textures.insert(texture.clone());
                    surface_materials.insert(surface.clone(), material.to_owned());
                }
            } else {
                used_fallback_source_textures.insert(texture.raw.clone());
                unresolved_source_texture_bindings
                    .entry(texture.raw.clone())
                    .or_default()
                    .push(binding);
                surface_materials.insert(surface.clone(), material.to_owned());
            }
        }
        let target_library = format!("{target_prefix}/{library}");
        models.push(StaticModelPortSpec {
            donor_model_archive: archive_resource,
            target_model: format!("models/{target_library}.model"),
            surfaces: surface_materials,
        });
        target_libraries.insert(library, target_library);
    }

    let mut visuals = manifest.visuals.clone();
    for object in &objects {
        if object.kind != "solid_visual" {
            continue;
        }
        let Some(library) = &object.library_name else {
            continue;
        };
        let Some(target_library) = target_libraries.get(library) else {
            continue;
        };
        visuals.push(LobbyVisualSpec {
            library_name: target_library.clone(),
            position: [
                object.position[0] - manifest.source_origin[0],
                object.position[1] - manifest.source_origin[1],
                object.position[2] - manifest.source_origin[2],
            ],
            rotation: object.rotation,
            scale: object.scale,
        });
    }

    let converted_lights = manifest
        .convert_source_lights
        .then(|| {
            objects
                .iter()
                .filter(|object| object.kind == "light")
                .filter(|object| {
                    manifest.source_light_types.as_ref().is_none_or(|types| {
                        matches!(
                            object.properties.get("light_type"),
                            Some(crate::stk2_project::ProjectProperty::Integer(value))
                                if types.contains(value)
                        )
                    })
                })
                .count()
        })
        .unwrap_or_default();
    let mut omitted_object_kinds: BTreeMap<String, usize> = parsed
        .object_kinds
        .iter()
        .filter(|(kind, _)| {
            kind.as_str() != "solid_visual"
                && !(manifest.convert_source_lights && kind.as_str() == "light")
        })
        .map(|(kind, count)| (kind.clone(), *count))
        .collect();
    for (kind, count) in &parsed.composite_child_kinds {
        if kind != "solid_visual" {
            omitted_object_kinds.insert(format!("composite/{kind}"), *count);
        }
    }
    for (name, count) in &parsed.omitted_composite_local_entries {
        omitted_object_kinds.insert(format!("composite-local/{name}"), *count);
    }
    let report = Stk2ProjectBuildReport {
        source_project: source_project.clone(),
        parsed,
        direct_solid_visuals,
        composite_solid_visuals,
        converted_instances: visuals.len() - manifest.visuals.len(),
        converted_models: models.len() - manifest.models.len(),
        converted_lights,
        source_texture_bindings,
        empty_source_texture_bindings,
        unresolved_source_texture_bindings,
        omitted_object_kinds,
        omitted_libraries,
    };
    to_writer_pretty(
        fs::File::create(output.join("stk2-project-report.json"))?,
        &report,
    )?;
    if used_fallback_source_textures != manifest.fallback_source_textures {
        let missing: Vec<_> = used_fallback_source_textures
            .difference(&manifest.fallback_source_textures)
            .collect();
        let stale: Vec<_> = manifest
            .fallback_source_textures
            .difference(&used_fallback_source_textures)
            .collect();
        bail!(
            "STK2 fallback_source_textures mismatch; missing approvals: {missing:?}; stale approvals: {stale:?}"
        );
    }
    if used_fallback_empty_surfaces != manifest.fallback_empty_surfaces {
        let missing: Vec<_> = used_fallback_empty_surfaces
            .difference(&manifest.fallback_empty_surfaces)
            .collect();
        let stale: Vec<_> = manifest
            .fallback_empty_surfaces
            .difference(&used_fallback_empty_surfaces)
            .collect();
        bail!(
            "STK2 fallback_empty_surfaces mismatch; missing approvals: {missing:?}; stale approvals: {stale:?}"
        );
    }
    let declared_surface_material_overrides: BTreeSet<_> = manifest
        .surface_material_overrides
        .keys()
        .cloned()
        .collect();
    if used_surface_material_overrides != declared_surface_material_overrides {
        let unused: Vec<_> = declared_surface_material_overrides
            .difference(&used_surface_material_overrides)
            .collect();
        bail!("unused STK2 surface_material_overrides entries: {unused:?}");
    }
    let declared_source_material_template_overrides: BTreeSet<_> = manifest
        .source_material_template_overrides
        .keys()
        .cloned()
        .collect();
    if used_source_material_template_overrides != declared_source_material_template_overrides {
        let unused: Vec<_> = declared_source_material_template_overrides
            .difference(&used_source_material_template_overrides)
            .collect();
        bail!("unused STK2 source_material_template_overrides entries: {unused:?}");
    }
    let declared_model_archive_overrides: BTreeSet<_> =
        manifest.model_archive_overrides.keys().cloned().collect();
    if used_model_archive_overrides != declared_model_archive_overrides {
        let unused: Vec<_> = declared_model_archive_overrides
            .difference(&used_model_archive_overrides)
            .collect();
        bail!("unused STK2 model_archive_overrides entries: {unused:?}");
    }
    let declared_omitted_model_libraries: BTreeSet<_> =
        manifest.omitted_model_libraries.keys().cloned().collect();
    if used_omitted_model_libraries != declared_omitted_model_libraries {
        let unused: Vec<_> = declared_omitted_model_libraries
            .difference(&used_omitted_model_libraries)
            .collect();
        bail!("unused STK2 omitted_model_libraries entries: {unused:?}");
    }
    let unapproved_omitted_libraries: Vec<_> = report
        .omitted_libraries
        .keys()
        .filter(|library| !declared_omitted_model_libraries.contains(*library))
        .collect();
    if !unapproved_omitted_libraries.is_empty() {
        bail!(
            "STK2 project has {} unapproved omitted active model libraries; first: {:?}",
            unapproved_omitted_libraries.len(),
            unapproved_omitted_libraries.first()
        );
    }
    if visuals.is_empty() || models.is_empty() {
        bail!("STK2 source project produced no compatible static visuals");
    }
    Ok((
        models,
        visuals,
        texture_plans.into_values().collect(),
        objects,
    ))
}

fn is_safe_stk2_library_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && name.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

fn write_bundle_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let destination = safe_join(overlay, path)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(destination, bytes)?;
    files.push(record_file(path, source, bytes));
    Ok(())
}

fn clear_category(objects: &mut ConfigValue, category: &str) -> Result<()> {
    let current = objects
        .named(category)
        .with_context(|| format!("server project template has no {category} category"))?;
    let empty = match current {
        ConfigValue::Indexed(_) => ConfigValue::Indexed(Vec::new()),
        ConfigValue::Named(_) => ConfigValue::Named(Vec::new()),
        _ => bail!("server project category {category} is not a table"),
    };
    objects.set_path(category, empty)
}

fn replace_positioned_category(
    objects: &mut ConfigValue,
    category: &str,
    positions: &[[f32; 3]],
    orientation: Option<f32>,
    assign_teams: bool,
) -> Result<()> {
    let ConfigValue::Indexed(templates) = objects
        .named(category)
        .with_context(|| format!("server project template has no {category} category"))?
    else {
        bail!("server project category {category} is not an indexed table");
    };
    if templates.is_empty() {
        bail!("server project category {category} has no object template");
    }
    let templates = templates.clone();
    let mut generated = Vec::with_capacity(positions.len());
    for (index, position) in positions.iter().enumerate() {
        let mut object = templates[index % templates.len()].clone();
        object.set_path("position", ConfigValue::Float3(*position))?;
        if let Some(orientation) = orientation {
            object.set_path("rotation", ConfigValue::Float3([0.0, orientation, 0.0]))?;
        }
        if assign_teams {
            object.set_path("team", ConfigValue::Integer(i32::try_from(index)?))?;
        }
        if object.named("point_id").is_some() {
            object.set_path("point_id", ConfigValue::Integer(i32::try_from(index)?))?;
        }
        if object.named("id").is_some() {
            object.set_path("id", ConfigValue::Integer(i32::try_from(index)?))?;
        }
        clear_collision_references(&mut object);
        generated.push(object);
    }
    objects.set_path(category, ConfigValue::Indexed(generated))
}

fn clear_collision_references(value: &mut ConfigValue) {
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                if child.name == "collision_geometries" {
                    child.value = match child.value {
                        ConfigValue::Named(_) => ConfigValue::Named(Vec::new()),
                        _ => ConfigValue::Indexed(Vec::new()),
                    };
                } else {
                    clear_collision_references(&mut child.value);
                }
            }
        }
        ConfigValue::Indexed(values) => {
            for child in values {
                clear_collision_references(child);
            }
        }
        _ => {}
    }
}

pub(crate) enum StaticUpgrade {
    AlreadyCompatible,
    Converted(Vec<u8>),
}

pub(crate) fn upgrade_stripped_static_model(bytes: &[u8]) -> Result<StaticUpgrade> {
    if read_u32(bytes, 0)? != 3 {
        bail!("converted_model does not start with vertex chunk 3");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    if let Some(declaration_size) = explicit_vertex_declaration_size(bytes) {
        let declaration = bytes
            .get(8..8 + declaration_size)
            .ok_or_else(|| anyhow::anyhow!("explicit vertex declaration is truncated"))?;
        let expected_stride = if declaration == STATIC_VERTEX_DECLARATION {
            32
        } else if declaration == COLORED_STATIC_VERTEX_DECLARATION {
            36
        } else {
            bail!("explicit vertex declaration is not one of the proven v0.100b static layouts");
        };
        let count_offset = 8_usize
            .checked_add(declaration_size)
            .ok_or_else(|| anyhow::anyhow!("explicit vertex declaration offset overflow"))?;
        let vertex_count = usize::try_from(read_u32(bytes, count_offset)?)?;
        let payload_size = vertex_chunk_size
            .checked_sub(declaration_size + 4)
            .ok_or_else(|| anyhow::anyhow!("explicit vertex chunk is truncated"))?;
        if vertex_count == 0 || payload_size != vertex_count.saturating_mul(expected_stride) {
            bail!(
                "explicit vertex declaration has an invalid payload \
                 (chunk={vertex_chunk_size}, declaration={declaration_size}, \
                 vertices={vertex_count}, expected_stride={expected_stride})"
            );
        }
        let vertex_start = count_offset + 4;
        let index_chunk_offset = 8_usize
            .checked_add(vertex_chunk_size)
            .ok_or_else(|| anyhow::anyhow!("index chunk offset overflow"))?;
        validate_target_static_vertices(
            bytes
                .get(vertex_start..index_chunk_offset)
                .ok_or_else(|| anyhow::anyhow!("explicit static vertex payload is truncated"))?,
            expected_stride,
            vertex_count,
        )?;
        if read_u32(bytes, index_chunk_offset)? != 4 {
            bail!("converted_model has no index chunk 4 after its vertex chunk");
        }
        let index_chunk_size = usize::try_from(read_u32(bytes, index_chunk_offset + 4)?)?;
        let expected_size = index_chunk_offset
            .checked_add(8)
            .and_then(|size| size.checked_add(index_chunk_size))
            .ok_or_else(|| anyhow::anyhow!("converted_model size overflow"))?;
        if bytes.len() != expected_size {
            bail!(
                "converted_model has trailing or truncated data: {} bytes, expected {expected_size}",
                bytes.len()
            );
        }
        validate_static_indices(bytes, index_chunk_offset, index_chunk_size, vertex_count)?;
        return Ok(StaticUpgrade::AlreadyCompatible);
    }
    let vertex_count = usize::try_from(read_u32(bytes, 8)?)?;
    let vertex_payload_size = vertex_chunk_size
        .checked_sub(4)
        .ok_or_else(|| anyhow::anyhow!("static vertex chunk is shorter than its count"))?;
    let vertex_stride = if vertex_count > 0 && vertex_payload_size % vertex_count == 0 {
        vertex_payload_size / vertex_count
    } else {
        0
    };
    if !matches!(vertex_stride, 28 | 32) {
        bail!(
            "declaration-less static converted_model is not one of the supported donor \
             28/32-byte static layouts \
             (chunk={vertex_chunk_size}, vertices={vertex_count}, stride={vertex_stride})"
        );
    }
    let index_chunk_offset = 8_usize
        .checked_add(vertex_chunk_size)
        .ok_or_else(|| anyhow::anyhow!("index chunk offset overflow"))?;
    if read_u32(bytes, index_chunk_offset)? != 4 {
        bail!("converted_model has no index chunk 4 after its vertex chunk");
    }
    let index_chunk_size = usize::try_from(read_u32(bytes, index_chunk_offset + 4)?)?;
    let expected_size = index_chunk_offset
        .checked_add(8)
        .and_then(|size| size.checked_add(index_chunk_size))
        .ok_or_else(|| anyhow::anyhow!("converted_model size overflow"))?;
    if bytes.len() != expected_size {
        bail!(
            "converted_model has trailing or truncated data: {} bytes, expected {expected_size}",
            bytes.len()
        );
    }

    validate_stripped_static_vertices(&bytes[12..index_chunk_offset], vertex_stride, vertex_count)?;
    validate_static_indices(bytes, index_chunk_offset, index_chunk_size, vertex_count)?;

    let (target_declaration, target_stride) = match vertex_stride {
        28 => (STATIC_VERTEX_DECLARATION.as_slice(), 32_usize),
        32 => (COLORED_STATIC_VERTEX_DECLARATION.as_slice(), 36_usize),
        _ => unreachable!("unsupported stripped static stride passed validation"),
    };
    let upgraded_vertex_payload_size = vertex_count
        .checked_mul(target_stride)
        .ok_or_else(|| anyhow::anyhow!("upgraded static vertex payload overflow"))?;
    let upgraded_chunk_size = 4_usize
        .checked_add(target_declaration.len())
        .and_then(|size| size.checked_add(upgraded_vertex_payload_size))
        .ok_or_else(|| anyhow::anyhow!("upgraded static vertex chunk overflow"))?;
    let expansion = vertex_count
        .checked_mul(target_stride - vertex_stride)
        .and_then(|size| size.checked_add(target_declaration.len()))
        .ok_or_else(|| anyhow::anyhow!("upgraded converted_model size overflow"))?;
    let mut output = Vec::with_capacity(bytes.len() + expansion);
    output.extend_from_slice(&bytes[..4]);
    output.extend_from_slice(&u32::try_from(upgraded_chunk_size)?.to_le_bytes());
    output.extend_from_slice(target_declaration);
    output.extend_from_slice(&bytes[8..12]);
    for vertex in bytes[12..index_chunk_offset].chunks_exact(vertex_stride) {
        output.extend_from_slice(&vertex[..24]);
        let u = half_to_f32(u16::from_le_bytes(vertex[24..26].try_into().unwrap()));
        let v = half_to_f32(u16::from_le_bytes(vertex[26..28].try_into().unwrap()));
        if !u.is_finite() || !v.is_finite() {
            bail!("expanded donor static UV is not finite");
        }
        output.extend_from_slice(&u.to_le_bytes());
        output.extend_from_slice(&v.to_le_bytes());
        if vertex_stride == 32 {
            output.extend_from_slice(&vertex[28..32]);
        }
    }
    output.extend_from_slice(&bytes[index_chunk_offset..]);
    Ok(StaticUpgrade::Converted(output))
}

fn validate_target_static_vertices(
    payload: &[u8],
    stride: usize,
    vertex_count: usize,
) -> Result<()> {
    validate_stripped_static_vertices(payload, stride, vertex_count)?;
    for (index, vertex) in payload.chunks_exact(stride).enumerate() {
        for axis in 0..2 {
            let offset = 24 + axis * 4;
            let uv = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            if !uv.is_finite() {
                bail!("target static vertex {index} UV axis {axis} is not finite");
            }
        }
    }
    Ok(())
}

fn validate_stripped_static_vertices(
    payload: &[u8],
    stride: usize,
    vertex_count: usize,
) -> Result<()> {
    if payload.len() != stride.saturating_mul(vertex_count) {
        bail!(
            "static vertex payload size does not match its count and stride \
             (payload={}, vertices={vertex_count}, stride={stride})",
            payload.len()
        );
    }
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for (index, vertex) in payload.chunks_exact(stride).enumerate() {
        for axis in 0..3 {
            let offset = axis * 4;
            let position = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            if !position.is_finite() {
                bail!("static vertex {index} position axis {axis} is not finite");
            }
            minimum[axis] = minimum[axis].min(position);
            maximum[axis] = maximum[axis].max(position);
        }
    }
    if minimum
        .into_iter()
        .chain(maximum)
        .any(|component| !component.is_finite())
    {
        bail!("computed static vertex bounds are not finite");
    }
    Ok(())
}

fn validate_static_indices(
    bytes: &[u8],
    chunk_offset: usize,
    chunk_size: usize,
    vertex_count: usize,
) -> Result<()> {
    if chunk_size < 4 {
        bail!("static index chunk is shorter than its count");
    }
    let index_count = usize::try_from(read_u32(bytes, chunk_offset + 8)?)?;
    if index_count % 3 != 0 {
        bail!("static index count {index_count} is not divisible by three");
    }
    let expected_chunk_size = 4_usize
        .checked_add(
            index_count
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("static index payload size overflow"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("static index chunk size overflow"))?;
    if chunk_size != expected_chunk_size {
        bail!(
            "static index chunk size does not match its count \
             (chunk={chunk_size}, indices={index_count}, expected={expected_chunk_size})"
        );
    }
    let indices = bytes
        .get(chunk_offset + 12..chunk_offset + 8 + chunk_size)
        .ok_or_else(|| anyhow::anyhow!("static index payload is truncated"))?;
    for (position, raw) in indices.chunks_exact(2).enumerate() {
        let index = usize::from(u16::from_le_bytes(raw.try_into().unwrap()));
        if index >= vertex_count {
            bail!(
                "static index {position} references vertex {index}, but vertex_count is {vertex_count}"
            );
        }
    }
    Ok(())
}

fn explicit_vertex_declaration_size(bytes: &[u8]) -> Option<usize> {
    let declaration = bytes.get(8..)?;
    let first = declaration.get(..8)?;
    if u16::from_le_bytes(first[..2].try_into().ok()?) != 0
        || u16::from_le_bytes(first[2..4].try_into().ok()?) != 0
    {
        return None;
    }
    for (index, entry) in declaration.chunks_exact(8).take(16).enumerate() {
        let stream = u16::from_le_bytes(entry[..2].try_into().ok()?);
        if stream == 0xff && entry[4] == 0x11 {
            return Some((index + 1) * 8);
        }
        if stream != 0 || entry[4] >= 0x11 {
            return None;
        }
    }
    None
}

fn half_to_f32(value: u16) -> f32 {
    let sign = u32::from(value & 0x8000) << 16;
    let exponent = u32::from((value >> 10) & 0x1f);
    let fraction = u32::from(value & 0x03ff);
    let bits = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let leading = fraction.leading_zeros() - 22;
            let normalized = (fraction << (leading + 1)) & 0x03ff;
            sign | ((127 - 15 - leading) << 23) | (normalized << 13)
        }
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent + (127 - 15)) << 23) | (fraction << 13),
    };
    f32::from_bits(bits)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("converted_model is truncated at byte {offset}"))?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_newer_environment_probe_resolution_indices() {
        let probe = |index| {
            ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("environment_probe".into()),
                },
                NamedValue {
                    name: "cubemap_resolution_index".into(),
                    value: ConfigValue::Integer(index),
                },
            ])
        };
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![probe(2), probe(3)]),
        }]);

        assert_eq!(
            convert_environment_probe_contracts(&mut root, "projects/test/client_project").unwrap(),
            2
        );
        let ConfigValue::Indexed(objects) = root.named("objects").unwrap() else {
            panic!("objects must remain indexed");
        };
        assert_eq!(
            objects[0].named("cubemap_resolution"),
            Some(&ConfigValue::Integer(128))
        );
        assert_eq!(
            objects[1].named("cubemap_resolution"),
            Some(&ConfigValue::Integer(256))
        );
        assert!(objects[0].named("cubemap_resolution_index").is_none());
        assert!(objects[1].named("cubemap_resolution_index").is_none());
        for object in objects {
            assert_eq!(
                object.named("clip_by_normal"),
                Some(&ConfigValue::Boolean(false))
            );
            assert_eq!(
                object.named("with_shadows"),
                Some(&ConfigValue::Boolean(false))
            );
        }
    }

    #[test]
    fn rejects_unproven_environment_probe_resolution_indices() {
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("environment_probe".into()),
                },
                NamedValue {
                    name: "cubemap_resolution_index".into(),
                    value: ConfigValue::Integer(4),
                },
            ])]),
        }]);

        let error = convert_environment_probe_contracts(&mut root, "projects/test/client_project")
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unproven cubemap_resolution_index 4")
        );
    }

    #[test]
    fn supplies_legacy_post_effect_for_newer_environment_objects() {
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("environment".into()),
                },
                NamedValue {
                    name: "properties".into(),
                    value: ConfigValue::Named(Vec::new()),
                },
            ])]),
        }]);

        assert_eq!(
            convert_environment_contracts(
                &mut root,
                "projects/test/client_project",
                Some("post_process/level_03_day"),
            )
            .unwrap(),
            1
        );
        let ConfigValue::Indexed(objects) = root.named("objects").unwrap() else {
            panic!("objects must remain indexed");
        };
        assert_eq!(
            objects[0].named("post_effect"),
            Some(&ConfigValue::String("post_process/level_03_day".into()))
        );
        assert!(objects[0].named("properties").is_none());
    }

    #[test]
    fn replaces_only_template_lights_with_projected_stk2_lighting() {
        use crate::stk2_project::{ProjectObject, ProjectProperty};

        let object = |fields| ConfigValue::Named(fields);
        let named = |name: &str, value| NamedValue {
            name: name.into(),
            value,
        };
        let mut root = ConfigValue::Named(vec![named(
            "objects",
            ConfigValue::Indexed(vec![
                object(vec![named(
                    "game_object_type",
                    ConfigValue::String("sky".into()),
                )]),
                object(vec![
                    named(
                        "game_object_type",
                        ConfigValue::String("environment".into()),
                    ),
                    named(
                        "post_effect",
                        ConfigValue::String("post_process/lobby".into()),
                    ),
                    named("position", ConfigValue::Float3([0.0, 0.0, 0.0])),
                ]),
                object(vec![named(
                    "game_object_type",
                    ConfigValue::String("environment_probe".into()),
                )]),
                object(vec![named(
                    "game_object_type",
                    ConfigValue::String("decal".into()),
                )]),
                object(vec![
                    named("game_object_type", ConfigValue::String("light".into())),
                    named("light_type", ConfigValue::Integer(0)),
                    named("lighting_model", ConfigValue::Integer(3)),
                    named("is_cast_shadow", ConfigValue::Boolean(true)),
                    named("position", ConfigValue::Float3([0.0, 0.0, 0.0])),
                    named("intensity", ConfigValue::Float(1.0)),
                    named("z_bias", ConfigValue::Integer(0)),
                    named("shadow_transparency", ConfigValue::Integer(0)),
                    named("name", ConfigValue::String("template".into())),
                    named("full_name", ConfigValue::String("template".into())),
                ]),
            ]),
        )]);
        let source_objects = vec![
            ProjectObject {
                id: "environment".into(),
                kind: "environment".into(),
                library_name: Some("environment".into()),
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                properties: BTreeMap::from([
                    (
                        "game_object_type".into(),
                        ProjectProperty::String("environment".into()),
                    ),
                    (
                        "post_effect".into(),
                        ProjectProperty::String("source/incompatible".into()),
                    ),
                    ("position".into(), ProjectProperty::Float3([0.0, 0.0, 0.0])),
                ]),
            },
            ProjectObject {
                id: "light".into(),
                kind: "light".into(),
                library_name: Some("point_light".into()),
                position: [69.0, 14.0, -7.0],
                rotation: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                properties: BTreeMap::from([
                    (
                        "game_object_type".into(),
                        ProjectProperty::String("light".into()),
                    ),
                    ("light_type".into(), ProjectProperty::Integer(0)),
                    ("lighting_model".into(), ProjectProperty::Integer(0)),
                    ("is_cast_shadow".into(), ProjectProperty::Boolean(true)),
                    (
                        "position".into(),
                        ProjectProperty::Float3([69.0, 14.0, -7.0]),
                    ),
                    ("intensity".into(), ProjectProperty::Float(0.15)),
                    ("z_bias".into(), ProjectProperty::Float(0.0001)),
                    ("shadow_transparency".into(), ProjectProperty::Float(0.1)),
                    (
                        "name".into(),
                        ProjectProperty::String("source_light".into()),
                    ),
                ]),
            },
        ];

        replace_stk2_client_lighting(
            &mut root,
            &source_objects,
            [64.0, 0.0, 0.0],
            None,
            false,
            "projects/test/client_project",
        )
        .unwrap();

        let ConfigValue::Indexed(objects) = root.named("objects").unwrap() else {
            panic!("objects must remain indexed");
        };
        assert_eq!(objects.len(), 5);
        assert_eq!(project_object_kind(&objects[0]), Some("sky"));
        assert_eq!(project_object_kind(&objects[1]), Some("environment"));
        assert_eq!(
            objects[1].named("post_effect"),
            Some(&ConfigValue::String("post_process/lobby".into()))
        );
        assert_eq!(project_object_kind(&objects[2]), Some("environment_probe"));
        assert_eq!(project_object_kind(&objects[3]), Some("decal"));
        assert_eq!(project_object_kind(&objects[4]), Some("light"));
        assert_eq!(
            objects[4].named("position"),
            Some(&ConfigValue::Float3([5.0, 14.0, -7.0]))
        );
        assert_eq!(
            objects[4].named("intensity"),
            Some(&ConfigValue::Float(0.15))
        );
        assert_eq!(
            objects[4].named("lighting_model"),
            Some(&ConfigValue::Integer(3))
        );
        assert_eq!(
            objects[4].named("is_cast_shadow"),
            Some(&ConfigValue::Boolean(true))
        );
        assert_eq!(objects[4].named("z_bias"), Some(&ConfigValue::Integer(0)));
        assert_eq!(
            objects[4].named("shadow_transparency"),
            Some(&ConfigValue::Integer(0))
        );
    }

    #[test]
    fn rejects_newer_environment_without_explicit_legacy_post_effect() {
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![NamedValue {
                name: "game_object_type".into(),
                value: ConfigValue::String("environment".into()),
            }])]),
        }]);

        let error = convert_environment_contracts(&mut root, "projects/test/client_project", None)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("declare legacy_environment_post_effect")
        );
    }

    #[test]
    fn rewrites_sound_resource_class_with_the_target_sound_form() {
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("object_sound".into()),
                },
                NamedValue {
                    name: "sound_name".into(),
                    value: ConfigValue::String("scapes/creatures/creatures_line".into()),
                },
                NamedValue {
                    name: "resource_id".into(),
                    value: ConfigValue::Integer(258),
                },
            ])]),
        }]);

        assert_eq!(
            convert_object_sound_contracts(&mut root, "projects/test/client_project", |_| Ok(256),)
                .unwrap(),
            1
        );
        let ConfigValue::Indexed(objects) = root.named("objects").unwrap() else {
            panic!("objects must remain indexed");
        };
        assert_eq!(
            objects[0].named("resource_id"),
            Some(&ConfigValue::Integer(256))
        );
    }

    #[test]
    fn upgrades_newer_colored_static_model_with_target_color_declaration() {
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&36_u32.to_le_bytes());
        input.extend_from_slice(&1_u32.to_le_bytes());
        input.extend_from_slice(&[0; 24]);
        input.extend_from_slice(&0x3400_u16.to_le_bytes()); // 0.25
        input.extend_from_slice(&0x3800_u16.to_le_bytes()); // 0.5
        input.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&[0; 6]);

        let StaticUpgrade::Converted(output) = upgrade_stripped_static_model(&input).unwrap()
        else {
            panic!("expected conversion");
        };
        assert_eq!(read_u32(&output, 0).unwrap(), 3);
        assert_eq!(read_u32(&output, 4).unwrap(), 96);
        assert_eq!(&output[8..64], &COLORED_STATIC_VERTEX_DECLARATION);
        assert_eq!(read_u32(&output, 64).unwrap(), 1);
        assert_eq!(f32::from_bits(read_u32(&output, 92).unwrap()), 0.25);
        assert_eq!(f32::from_bits(read_u32(&output, 96).unwrap()), 0.5);
        assert_eq!(&output[100..104], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(read_u32(&output, 104).unwrap(), 4);
    }

    #[test]
    fn expands_half_precision_uvs_in_28_byte_static_vertices() {
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&32_u32.to_le_bytes());
        input.extend_from_slice(&1_u32.to_le_bytes());
        input.extend_from_slice(&[0x5a; 24]);
        input.extend_from_slice(&0x3400_u16.to_le_bytes()); // 0.25
        input.extend_from_slice(&0x3800_u16.to_le_bytes()); // 0.5
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&[0; 6]);

        let StaticUpgrade::Converted(output) = upgrade_stripped_static_model(&input).unwrap()
        else {
            panic!("expected conversion");
        };
        assert_eq!(read_u32(&output, 4).unwrap(), 84);
        assert_eq!(read_u32(&output, 56).unwrap(), 1);
        assert_eq!(f32::from_bits(read_u32(&output, 84).unwrap()), 0.25);
        assert_eq!(f32::from_bits(read_u32(&output, 88).unwrap()), 0.5);
        assert_eq!(read_u32(&output, 92).unwrap(), 4);
    }

    #[test]
    fn rejects_declarationless_36_byte_layout_in_static_adapter() {
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&40_u32.to_le_bytes());
        input.extend_from_slice(&1_u32.to_le_bytes());
        input.extend_from_slice(&[0x5a; 36]);
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&[0; 6]);

        let error = match upgrade_stripped_static_model(&input) {
            Err(error) => error,
            Ok(_) => panic!("expected ambiguous declaration-less stride rejection"),
        };
        assert!(error.to_string().contains("28/32-byte static layouts"));
    }

    #[test]
    fn leaves_explicit_extended_static_vertex_declarations_unchanged() {
        let declaration = COLORED_STATIC_VERTEX_DECLARATION.to_vec();
        let vertex_count = 2_u32;
        let vertex_stride = 36_usize;
        let vertex_chunk_size = declaration.len() + 4 + vertex_stride * vertex_count as usize;
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&(vertex_chunk_size as u32).to_le_bytes());
        input.extend_from_slice(&declaration);
        input.extend_from_slice(&vertex_count.to_le_bytes());
        input.extend_from_slice(&vec![0_u8; vertex_stride * vertex_count as usize]);
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(
            &[0_u16, 1_u16, 0_u16]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );

        assert!(matches!(
            upgrade_stripped_static_model(&input).unwrap(),
            StaticUpgrade::AlreadyCompatible
        ));
    }

    #[test]
    fn rejects_out_of_range_static_indices() {
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&32_u32.to_le_bytes());
        input.extend_from_slice(&1_u32.to_le_bytes());
        input.extend_from_slice(&[0; 28]);
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(
            &[0_u16, 1_u16, 0_u16]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );

        let error = match upgrade_stripped_static_model(&input) {
            Err(error) => error,
            Ok(_) => panic!("expected invalid index rejection"),
        };
        assert!(error.to_string().contains("references vertex 1"));
    }

    #[test]
    fn rejects_non_finite_static_positions() {
        let mut input = Vec::new();
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&32_u32.to_le_bytes());
        input.extend_from_slice(&1_u32.to_le_bytes());
        input.extend_from_slice(&f32::NAN.to_le_bytes());
        input.extend_from_slice(&[0; 24]);
        input.extend_from_slice(&4_u32.to_le_bytes());
        input.extend_from_slice(&10_u32.to_le_bytes());
        input.extend_from_slice(&3_u32.to_le_bytes());
        input.extend_from_slice(&[0; 6]);

        let error = match upgrade_stripped_static_model(&input) {
            Err(error) => error,
            Ok(_) => panic!("expected non-finite position rejection"),
        };
        assert!(error.to_string().contains("is not finite"));
    }
}
