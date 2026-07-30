use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;
use sha2::{Digest, Sha256};

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::manifest::{BuiltFile, TargetNativeMapManifest};
use crate::map_port::{StaticUpgrade, upgrade_stripped_static_model};
use crate::material_conversion::{MaterialCatalog, canonical_texture_reference};
use crate::weapon::{prepare_empty_output, record_file, reject_duplicate_outputs};

#[derive(Clone, Debug, Serialize)]
pub struct TargetNativeMapBuildRecord {
    pub format_version: u32,
    pub asset_id: String,
    pub donor_build: String,
    pub target_build: String,
    pub map_name: String,
    pub namespace: String,
    pub target_client_project: String,
    pub target_server_project: String,
    pub material_template_overrides: BTreeMap<String, String>,
    pub material_compatibility_templates: BTreeMap<String, String>,
    pub material_texture_overrides: BTreeMap<String, BTreeMap<String, String>>,
    pub empty_render_material_overrides: BTreeMap<String, BTreeMap<String, String>>,
    pub preserved_empty_render_materials: BTreeMap<String, BTreeSet<String>>,
    pub dropped_empty_render_models: BTreeSet<String>,
    pub dropped_unsupported_render_models: BTreeMap<String, String>,
    pub dropped_empty_render_surfaces: BTreeMap<String, BTreeSet<String>>,
    pub target_texture_fallbacks: BTreeSet<String>,
    pub target_texture_aliases: BTreeMap<String, String>,
    pub game_material_name_overrides: BTreeMap<String, String>,
    pub game_material_binding_overrides: BTreeMap<String, BTreeMap<String, String>>,
    pub auto_proven_game_material_bindings: BTreeMap<String, BTreeMap<String, String>>,
    pub collision_binding_preflight_report: String,
    pub dropped_empty_collision_families: Vec<String>,
    pub materialize_target_hard_links: BTreeSet<String>,
    pub converted_models: Vec<ModelAdapterRecord>,
    pub converted_collision_models: Vec<CollisionModelAdapterRecord>,
    pub converted_materials: BTreeMap<String, String>,
    pub converted_textures: BTreeMap<String, String>,
    pub reused_target_textures: BTreeMap<String, String>,
    pub dropped_categories: BTreeMap<String, String>,
    pub files: Vec<BuiltFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelAdapterRecord {
    pub source: String,
    pub target: String,
    pub ordinary_surfaces: usize,
    pub colored_surfaces: usize,
    pub collision_families: BTreeMap<String, CollisionAdapterRecord>,
    pub omitted_siblings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CollisionAdapterRecord {
    pub triangles: Option<usize>,
    pub primitives: Option<usize>,
    pub materials: BTreeSet<String>,
    pub fallback_zero_materials: BTreeSet<String>,
    pub default_sentinel_materials: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CollisionModelAdapterRecord {
    pub source: String,
    pub target: String,
    pub families: BTreeMap<String, CollisionAdapterRecord>,
    pub omitted_siblings: Vec<String>,
}

#[derive(Clone, Debug)]
struct ModelRequest {
    render_referenced: bool,
    target: String,
    collision_families: BTreeSet<String>,
    collision_targets: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DdsInfo {
    width: u32,
    height: u32,
    mip_count: u32,
    fourcc: [u8; 4],
    srgb: u8,
    dimension: i32,
}

#[derive(Clone, Debug)]
struct SurfaceProperties {
    mesh_type: i32,
    vertices: usize,
    indices: usize,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
}

#[derive(Clone, Debug, Serialize)]
struct CollisionBindingPreflightReport {
    format_version: u32,
    asset_id: String,
    entries: Vec<CollisionBindingPreflightEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct CollisionBindingPreflightEntry {
    model: String,
    family: String,
    key: String,
    donor_member_hashes: BTreeMap<String, String>,
    stock_family_available: bool,
    stock_member_hashes: BTreeMap<String, String>,
    donor_setting: Option<GameMaterialEvidence>,
    stock_setting: Option<GameMaterialEvidence>,
    donor_regular_collision: Option<MaterialHistogramEvidence>,
    stock_regular_collision: Option<MaterialHistogramEvidence>,
    parent_assigned_target_materials: BTreeSet<String>,
    status: String,
    reason: String,
    suggested_target_binding: Option<String>,
    #[serde(skip)]
    identity_candidate: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct GameMaterialEvidence {
    name: String,
    id: i32,
}

#[derive(Clone, Debug, Serialize)]
struct MaterialHistogramEvidence {
    key_counts: BTreeMap<String, usize>,
    target_counts: BTreeMap<String, usize>,
    fully_resolved: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CollisionMaterialResolution {
    fallback_zero: BTreeSet<String>,
    default_sentinel: BTreeSet<String>,
}

pub fn build_bundle(
    manifest: &TargetNativeMapManifest,
    donor_db: &Path,
    target_db: &Path,
    compatibility_db: Option<&Path>,
    output: &Path,
) -> Result<TargetNativeMapBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut donor = ResourceDb::open(donor_db)?;
    let mut target = ResourceDb::open(target_db)?;
    let mut compatibility = compatibility_db.map(ResourceDb::open).transpose()?;
    if !manifest.material_compatibility_templates.is_empty() && compatibility.is_none() {
        bail!(
            "manifest declares compatibility material templates but no compatibility DB was supplied"
        );
    }
    let material_catalog =
        MaterialCatalog::load(&mut target).context("indexing v0.100b material templates")?;
    let target_game_materials = load_game_materials(&mut target)?;
    let target_dds_formats = load_target_dds_formats(&mut target)?;

    let donor_project = BinaryConfig::parse(
        &donor
            .read(&manifest.donor_client_project)
            .with_context(|| {
                format!(
                    "reading donor client project {}",
                    manifest.donor_client_project
                )
            })?,
    )
    .context("parsing donor client project")?;
    let mut target_project = BinaryConfig::parse(
        &target
            .read(&manifest.target_client_template)
            .with_context(|| {
                format!(
                    "reading target client template {}",
                    manifest.target_client_template
                )
            })?,
    )
    .context("parsing target client template")?;

    let donor_render_visuals = donor_project
        .root
        .named("render_visuals")
        .context("donor project has no render_visuals")?;
    let donor_collision_objects = donor_project
        .root
        .named("collision_objects")
        .context("donor project has no collision_objects")?;
    let mut model_map = BTreeMap::<String, ModelRequest>::new();
    collect_model_requests(donor_render_visuals, InstanceKind::Render, &mut model_map)?;
    collect_model_requests(
        donor_collision_objects,
        InstanceKind::Collision,
        &mut model_map,
    )?;
    let requested_models: BTreeSet<&str> = model_map.keys().map(String::as_str).collect();
    let stale_game_binding_models: Vec<_> = manifest
        .game_material_binding_overrides
        .keys()
        .filter(|model| !requested_models.contains(model.as_str()))
        .cloned()
        .collect();
    if !stale_game_binding_models.is_empty() {
        bail!(
            "game-material binding overrides target unrequested models: {stale_game_binding_models:?}"
        );
    }
    let stale_render_binding_models: Vec<_> = manifest
        .empty_render_material_overrides
        .keys()
        .filter(|model| !requested_models.contains(model.as_str()))
        .cloned()
        .collect();
    if !stale_render_binding_models.is_empty() {
        bail!(
            "empty render-material overrides target unrequested models: {stale_render_binding_models:?}"
        );
    }
    let stale_preserved_empty_models: Vec<_> = manifest
        .preserve_empty_render_materials
        .keys()
        .filter(|model| !requested_models.contains(model.as_str()))
        .cloned()
        .collect();
    if !stale_preserved_empty_models.is_empty() {
        bail!(
            "preserved empty render-material entries target unrequested models: {stale_preserved_empty_models:?}"
        );
    }
    let stale_dropped_render_models: Vec<_> = manifest
        .drop_empty_render_models
        .iter()
        .filter(|model| {
            !model_map
                .get(*model)
                .is_some_and(|request| request.render_referenced)
        })
        .cloned()
        .collect();
    if !stale_dropped_render_models.is_empty() {
        bail!(
            "dropped empty render models are not render-referenced: {stale_dropped_render_models:?}"
        );
    }
    let stale_unsupported_render_models: Vec<_> = manifest
        .drop_unsupported_render_models
        .keys()
        .filter(|model| {
            !model_map
                .get(*model)
                .is_some_and(|request| request.render_referenced)
        })
        .cloned()
        .collect();
    if !stale_unsupported_render_models.is_empty() {
        bail!(
            "unsupported render-model drops are not render-referenced: \
             {stale_unsupported_render_models:?}"
        );
    }
    for (source_model, reason) in &manifest.drop_unsupported_render_models {
        if reason.trim().is_empty() {
            bail!("unsupported render-model drop {source_model} has no audit reason");
        }
        let request = model_map
            .get(source_model)
            .context("validated unsupported render model disappeared")?;
        debug_assert!(request.render_referenced);
        model_map
            .get_mut(source_model)
            .context("validated unsupported render model disappeared")?
            .render_referenced = false;
    }
    let stale_dropped_render_surface_models: Vec<_> = manifest
        .drop_empty_render_surfaces
        .keys()
        .filter(|model| {
            !model_map
                .get(*model)
                .is_some_and(|request| request.render_referenced)
        })
        .cloned()
        .collect();
    if !stale_dropped_render_surface_models.is_empty() {
        bail!(
            "dropped empty render surfaces target models that are not render-referenced: \
             {stale_dropped_render_surface_models:?}"
        );
    }
    for source_model in &manifest.drop_empty_render_models {
        let request = model_map
            .get(source_model)
            .context("validated dropped render model disappeared")?;
        if request.collision_families.is_empty() {
            bail!(
                "dropped empty render model {source_model} has no collision instances to preserve"
            );
        }
        validate_all_render_materials_empty(&mut donor, source_model, "donor")?;
        validate_all_render_materials_empty(&mut target, source_model, "target")?;
        model_map
            .get_mut(source_model)
            .context("validated dropped render model disappeared")?
            .render_referenced = false;
    }
    let mut dropped_empty_collision_families = Vec::new();
    for (source_model, request) in &mut model_map {
        let relative = source_model
            .strip_prefix("models/")
            .and_then(|path| path.strip_suffix(".model"))
            .context("normalized project model has an invalid path")?;
        let versioned = format!("models/{}/{relative}.model", manifest.namespace);
        request.target = versioned.clone();
        let declared_families = request.collision_families.clone();
        request.collision_families.clear();
        request.collision_targets.clear();
        for family in declared_families {
            let prefix = format!("{source_model}/{family}/");
            let has_payload = donor
                .paths_with_prefix(&prefix)
                .any(|path| donor.is_file(path));
            let family_path = format!("{source_model}/{family}");
            if has_payload {
                request.collision_families.insert(family.clone());
                request.collision_targets.insert(family, versioned.clone());
                continue;
            }
            if manifest
                .drop_empty_collision_families
                .contains(&family_path)
            {
                dropped_empty_collision_families.push(family_path);
                continue;
            }
            bail!(
                "{family_path} is a declared empty collision family; add an audited fallback or explicit drop"
            );
        }
    }
    let auto_proven_game_material_bindings = preflight_collision_material_bindings(
        &model_map,
        manifest,
        &mut donor,
        &mut target,
        &target_game_materials,
        output,
    )?;
    let mut effective_game_material_binding_overrides =
        manifest.game_material_binding_overrides.clone();
    for (model, bindings) in &auto_proven_game_material_bindings {
        let effective = effective_game_material_binding_overrides
            .entry(model.clone())
            .or_default();
        for (key, target_name) in bindings {
            if let Some(declared) = effective.insert(key.clone(), target_name.clone())
                && declared != *target_name
            {
                bail!(
                    "auto-proven binding {model}/{key}={target_name} conflicts with manifest {declared}"
                );
            }
        }
    }
    let render_visuals =
        rebuild_project_instances(donor_render_visuals, InstanceKind::Render, &model_map)?;
    let collision_objects =
        rebuild_project_instances(donor_collision_objects, InstanceKind::Collision, &model_map)?;
    target_project
        .root
        .set_path("render_visuals", render_visuals)?;
    target_project
        .root
        .set_path("collision_objects", collision_objects)?;
    for section in ["camera", "levelmap_min", "levelmap_max"] {
        if let Some(value) = donor_project.root.named(section) {
            target_project.root.set_path(section, value.clone())?;
        }
    }
    if target_project.root.named("project_name").is_some() {
        target_project.root.set_path(
            "project_name",
            ConfigValue::String(manifest.map_name.clone()),
        )?;
    }

    let mut files = Vec::new();
    write_resource(
        &overlay,
        &manifest.target_client_project,
        &target_project.to_bytes()?,
        "adapter:target-native-project-v020f-to-v0100b",
        &mut files,
    )?;
    let server_bytes = target
        .read(&manifest.target_server_template)
        .with_context(|| {
            format!(
                "reading target server template {}",
                manifest.target_server_template
            )
        })?;
    write_resource(
        &overlay,
        &manifest.target_server_project,
        &server_bytes,
        &format!("target-copy:{}", manifest.target_server_template),
        &mut files,
    )?;

    let mut converted_models = Vec::new();
    let mut converted_collision_models = Vec::new();
    let mut requested_materials = BTreeSet::<String>::new();
    for (source_model, request) in &model_map {
        let versioned_collision_families = request.collision_families.clone();
        if request.render_referenced {
            converted_models.push(convert_model(
                source_model,
                &request.target,
                &versioned_collision_families,
                &manifest.namespace,
                &mut donor,
                &target_game_materials,
                &manifest.game_material_name_overrides,
                effective_game_material_binding_overrides.get(source_model),
                manifest.empty_render_material_overrides.get(source_model),
                manifest.preserve_empty_render_materials.get(source_model),
                manifest.drop_empty_render_surfaces.get(source_model),
                versioned_collision_families.is_empty(),
                &overlay,
                &mut files,
                &mut requested_materials,
            )?);
        } else if !versioned_collision_families.is_empty() {
            let target_model = request
                .collision_targets
                .values()
                .next()
                .context("versioned collision family has no target model")?
                .clone();
            let mut families = BTreeMap::new();
            let source_paths: Vec<String> = donor
                .paths_with_prefix(&format!("{source_model}/"))
                .filter(|path| donor.is_file(path))
                .map(ToOwned::to_owned)
                .collect();
            let settings_path = format!("{source_model}/settings");
            if !source_paths.contains(&settings_path) {
                bail!("collision-only model {source_model} has no parent settings");
            }
            let donor_settings = BinaryConfig::parse(&donor.read(&settings_path)?)
                .with_context(|| format!("parsing collision-only settings {settings_path}"))?;
            let game_material_settings = donor_settings
                .root
                .named("game_material_settings")
                .with_context(|| {
                    format!(
                        "collision-only settings {settings_path} have no game_material_settings"
                    )
                })?
                .clone();
            let mut minimal_settings = BinaryConfig {
                root: ConfigValue::Named(vec![NamedValue {
                    name: "game_material_settings".into(),
                    value: game_material_settings,
                }]),
            };
            let collision_materials = remap_game_material_settings(
                &mut minimal_settings.root,
                &settings_path,
                &target_game_materials,
                &manifest.game_material_name_overrides,
                effective_game_material_binding_overrides.get(source_model),
            )?;
            let collision_materials = apply_game_material_binding_overrides(
                &mut minimal_settings.root,
                &settings_path,
                collision_materials,
                effective_game_material_binding_overrides.get(source_model),
                &target_game_materials,
            )?;
            strip_game_material_audit_names(&mut minimal_settings.root, &settings_path)?;
            write_resource(
                &overlay,
                &format!("{target_model}/settings"),
                &minimal_settings.to_bytes()?,
                "adapter:collision-only-settings-v020f-to-v0100b",
                &mut files,
            )?;
            for family in versioned_collision_families {
                families.insert(
                    family.clone(),
                    convert_collision_family(
                        source_model,
                        &target_model,
                        &family,
                        &source_paths,
                        &mut donor,
                        &overlay,
                        &mut files,
                        &collision_materials,
                    )?,
                );
            }
            let recognized: BTreeSet<String> = std::iter::once(settings_path)
                .chain(
                    source_paths
                        .iter()
                        .filter(|path| {
                            families
                                .keys()
                                .any(|family| path.contains(&format!("/{family}/")))
                        })
                        .cloned(),
                )
                .collect();
            let (omitted_siblings, unknown) =
                classify_model_siblings(source_model, &source_paths, &recognized, true);
            if !unknown.is_empty() {
                bail!(
                    "collision-only model {source_model} has unsupported resource kinds: {unknown:?}"
                );
            }
            validate_binding_override_usage(
                source_model,
                manifest.game_material_binding_overrides.get(source_model),
                families.values(),
            )?;
            converted_collision_models.push(CollisionModelAdapterRecord {
                source: source_model.clone(),
                target: target_model,
                families,
                omitted_siblings,
            });
        }
    }

    let mut converted_materials = BTreeMap::new();
    let mut converted_textures = BTreeMap::new();
    let mut reused_target_textures = BTreeMap::new();
    let mut used_target_texture_fallbacks = BTreeSet::new();
    let stale_material_template_overrides: Vec<_> = manifest
        .material_template_overrides
        .keys()
        .filter(|path| !requested_materials.contains(*path))
        .cloned()
        .collect();
    if !stale_material_template_overrides.is_empty() {
        bail!(
            "material template overrides were not used by the converted model graph: {stale_material_template_overrides:?}"
        );
    }
    let stale_material_texture_overrides: Vec<_> = manifest
        .material_texture_overrides
        .keys()
        .filter(|path| !requested_materials.contains(*path))
        .cloned()
        .collect();
    if !stale_material_texture_overrides.is_empty() {
        bail!(
            "material texture overrides were not used by the converted model graph: {stale_material_texture_overrides:?}"
        );
    }
    let stale_material_compatibility_templates: Vec<_> = manifest
        .material_compatibility_templates
        .keys()
        .filter(|path| !requested_materials.contains(*path))
        .cloned()
        .collect();
    if !stale_material_compatibility_templates.is_empty() {
        bail!(
            "compatibility material templates were not used by the converted model graph: \
             {stale_material_compatibility_templates:?}"
        );
    }
    let donor_missing_material_template_overrides: Vec<_> = manifest
        .material_template_overrides
        .keys()
        .filter(|path| !donor.is_file(path))
        .cloned()
        .collect();
    if !donor_missing_material_template_overrides.is_empty() {
        bail!(
            "material template overrides target donor-missing materials that require exact target-copy fallback instead: {donor_missing_material_template_overrides:?}"
        );
    }
    for donor_material in requested_materials {
        convert_material_and_textures(
            manifest,
            &donor_material,
            &material_catalog,
            &mut donor,
            &mut target,
            compatibility.as_mut(),
            &target_dds_formats,
            &overlay,
            &mut files,
            &mut converted_materials,
            &mut converted_textures,
            &mut reused_target_textures,
            &mut used_target_texture_fallbacks,
        )?;
    }
    for (target_dds, source_dds) in &manifest.target_texture_aliases {
        if target.is_file(target_dds) {
            bail!("target texture alias destination already exists: {target_dds}");
        }
        if !target.is_file(source_dds) {
            bail!("target texture alias source does not exist: {source_dds}");
        }
        let target_options = format!("{}.options", target_dds.trim_end_matches(".dds"));
        let source_options = format!("{}.options", source_dds.trim_end_matches(".dds"));
        if target.is_file(&target_options) {
            bail!("target texture alias options destination already exists: {target_options}");
        }
        if !target.is_file(&source_options) {
            bail!("target texture alias source lacks options payload: {source_options}");
        }
        write_resource(
            &overlay,
            target_dds,
            &target.read(source_dds)?,
            &format!("target-texture-alias:{source_dds}"),
            &mut files,
        )?;
        write_resource(
            &overlay,
            &target_options,
            &target.read(&source_options)?,
            &format!("target-texture-alias:{source_options}"),
            &mut files,
        )?;
    }
    let stale_target_texture_fallbacks: Vec<_> = manifest
        .target_texture_fallbacks
        .difference(&used_target_texture_fallbacks)
        .cloned()
        .collect();
    if !stale_target_texture_fallbacks.is_empty() {
        bail!(
            "target texture fallbacks were not used by the converted material graph: {stale_target_texture_fallbacks:?}"
        );
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;
    crate::model_validation::validate_effective_overlay(target_db, &overlay)
        .context("validating target-native map overlay")?;

    let record = TargetNativeMapBuildRecord {
        format_version: 2,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        map_name: manifest.map_name.clone(),
        namespace: manifest.namespace.clone(),
        target_client_project: manifest.target_client_project.clone(),
        target_server_project: manifest.target_server_project.clone(),
        material_template_overrides: manifest.material_template_overrides.clone(),
        material_compatibility_templates: manifest.material_compatibility_templates.clone(),
        material_texture_overrides: manifest.material_texture_overrides.clone(),
        empty_render_material_overrides: manifest.empty_render_material_overrides.clone(),
        preserved_empty_render_materials: manifest.preserve_empty_render_materials.clone(),
        dropped_empty_render_models: manifest.drop_empty_render_models.clone(),
        dropped_unsupported_render_models: manifest.drop_unsupported_render_models.clone(),
        dropped_empty_render_surfaces: manifest.drop_empty_render_surfaces.clone(),
        target_texture_fallbacks: manifest.target_texture_fallbacks.clone(),
        target_texture_aliases: manifest.target_texture_aliases.clone(),
        game_material_name_overrides: manifest.game_material_name_overrides.clone(),
        game_material_binding_overrides: manifest.game_material_binding_overrides.clone(),
        auto_proven_game_material_bindings,
        collision_binding_preflight_report: "collision-binding-preflight.json".into(),
        dropped_empty_collision_families,
        materialize_target_hard_links: manifest.materialize_target_hard_links.clone(),
        converted_models,
        converted_collision_models,
        converted_materials,
        converted_textures,
        reused_target_textures,
        dropped_categories: BTreeMap::from([
            (
                "client_objects".into(),
                "stock v0.100b objects retained; donor gameplay objects are unsupported".into(),
            ),
            (
                "server_objects".into(),
                "known-good stock v0.100b server project copied for SRP only".into(),
            ),
            (
                "vegetation".into(),
                "v0.20f grass_layer_data ABI differs; no identity adapter".into(),
            ),
            (
                "particles_anomalies_effect_zones".into(),
                "unsupported donor gameplay/effect categories are not reachable".into(),
            ),
            (
                "model_locators".into(),
                "donor render locators are removed; stock v0.100b equivalents omit them".into(),
            ),
            (
                "model_editor_metadata".into(),
                "excluded_surfaces metadata is validated then omitted; runtime-required lod_hierrarchy and lod_switching are reconstructed separately".into(),
            ),
        ]),
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(record)
}

#[derive(Clone, Copy)]
enum InstanceKind {
    Render,
    Collision,
}

fn rebuild_project_instances(
    value: &ConfigValue,
    kind: InstanceKind,
    models: &BTreeMap<String, ModelRequest>,
) -> Result<ConfigValue> {
    let ConfigValue::Indexed(entries) = value else {
        bail!("project instance section is not an indexed table");
    };
    let mut output = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        validate_instance(entry, kind, index)?;
        let ConfigValue::String(library) = entry
            .named("lib_name")
            .context("project instance has no string lib_name")?
        else {
            bail!("project instance lib_name is not a string");
        };
        let (source_model, suffix) = model_from_library(library, kind)?;
        let request = models
            .get(&source_model)
            .with_context(|| format!("model request was not collected for {source_model}"))?;
        if matches!(kind, InstanceKind::Render) && !request.render_referenced {
            continue;
        }
        let mut rebuilt = entry.clone();
        let target_library = match kind {
            InstanceKind::Render => request
                .target
                .strip_prefix("models/")
                .and_then(|path| path.strip_suffix(".model"))
                .context("normalized project model has an invalid path")?
                .to_owned(),
            InstanceKind::Collision => {
                let family = suffix
                    .strip_prefix('/')
                    .and_then(|value| value.split(['#', '/']).next())
                    .context("collision instance has no resource family")?;
                let Some(target_model) = request.collision_targets.get(family) else {
                    continue;
                };
                format!("resources/{target_model}{suffix}")
            }
        };
        rebuilt.set_path("lib_name", ConfigValue::String(target_library))?;
        output.push(rebuilt);
    }
    Ok(ConfigValue::Indexed(output))
}

fn collect_model_requests(
    value: &ConfigValue,
    kind: InstanceKind,
    models: &mut BTreeMap<String, ModelRequest>,
) -> Result<()> {
    let ConfigValue::Indexed(entries) = value else {
        bail!("project instance section is not an indexed table");
    };
    for (index, entry) in entries.iter().enumerate() {
        validate_instance(entry, kind, index)?;
        let ConfigValue::String(library) = entry
            .named("lib_name")
            .context("project instance has no string lib_name")?
        else {
            bail!("project instance lib_name is not a string");
        };
        let (source_model, suffix) = model_from_library(library, kind)?;
        let request = models.entry(source_model).or_insert_with(|| ModelRequest {
            render_referenced: false,
            target: String::new(),
            collision_families: BTreeSet::new(),
            collision_targets: BTreeMap::new(),
        });
        if matches!(kind, InstanceKind::Render) {
            request.render_referenced = true;
        } else {
            let family = suffix
                .strip_prefix('/')
                .and_then(|value| value.split(['#', '/']).next())
                .context("collision instance has no resource family")?;
            request.collision_families.insert(family.to_owned());
        }
    }
    Ok(())
}

fn validate_instance(value: &ConfigValue, kind: InstanceKind, index: usize) -> Result<()> {
    let ConfigValue::Named(fields) = value else {
        bail!("project instance {index} is not a named table");
    };
    let expected: BTreeSet<&str> = match kind {
        InstanceKind::Render => ["lib_name", "scale", "rotation", "position"]
            .into_iter()
            .collect(),
        InstanceKind::Collision => [
            "lib_name", "cgroup", "cmask", "scale", "rotation", "position",
        ]
        .into_iter()
        .collect(),
    };
    let actual: BTreeSet<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    if actual.len() != fields.len() {
        bail!("project instance {index} contains duplicate named fields");
    }
    if actual != expected {
        bail!(
            "project instance {index} has unsupported fields: expected {expected:?}, got {actual:?}"
        );
    }
    for field in ["scale", "rotation", "position"] {
        let ConfigValue::Float3(values) = value
            .named(field)
            .with_context(|| format!("project instance {index} has no {field}"))?
        else {
            bail!("project instance {index}.{field} is not float3");
        };
        if values.iter().any(|value| !value.is_finite()) {
            bail!("project instance {index}.{field} is not finite");
        }
    }
    if matches!(kind, InstanceKind::Collision) {
        for field in ["cgroup", "cmask"] {
            if !matches!(value.named(field), Some(ConfigValue::Integer(_))) {
                bail!("collision instance {index}.{field} is not integer");
            }
        }
    }
    Ok(())
}

fn model_from_library(library: &str, kind: InstanceKind) -> Result<(String, String)> {
    match kind {
        InstanceKind::Render => {
            let normalized = library
                .trim_start_matches("resources/")
                .trim_start_matches("models/");
            if normalized.is_empty() || normalized.contains('#') || normalized.ends_with(".model") {
                bail!("unsupported render_visual library name {library:?}");
            }
            Ok((format!("models/{normalized}.model"), String::new()))
        }
        InstanceKind::Collision => {
            let normalized = library.trim_start_matches("resources/");
            let model_end = normalized
                .rfind(".model")
                .map(|index| index + ".model".len())
                .with_context(|| format!("collision library has no .model root: {library}"))?;
            let model = &normalized[..model_end];
            let suffix = parse_collision_suffix(&normalized[model_end..])
                .with_context(|| format!("unsupported collision library name {library:?}"))?;
            if !model.starts_with("models/") {
                bail!("unsupported collision library name {library:?}");
            }
            Ok((model.to_owned(), suffix))
        }
    }
}

fn parse_collision_suffix(suffix: &str) -> Result<String> {
    let Some((family, scale)) = suffix.split_once('#') else {
        if !matches!(suffix, "/collision" | "/walker_collision") {
            bail!("collision suffix has an unsupported family");
        }
        return Ok(suffix.to_owned());
    };
    if !matches!(family, "/collision" | "/walker_collision") {
        bail!("collision suffix has an unsupported family");
    }
    if scale.is_empty() {
        bail!("collision scale marker has no values");
    }
    let mut rest = scale;
    let mut values = [0.0_f32; 3];
    for value in &mut values {
        let inner = rest
            .strip_prefix('[')
            .and_then(|tail| tail.split_once(']'))
            .context("collision scale is not exactly three bracketed floats")?;
        *value = inner
            .0
            .parse::<f32>()
            .context("collision scale contains a non-float")?;
        if !value.is_finite() {
            bail!("collision scale is not finite");
        }
        rest = inner.1;
    }
    if !rest.is_empty() {
        bail!("collision scale has trailing data");
    }
    Ok(format!(
        "{family}#[{}][{}][{}]",
        values[0], values[1], values[2]
    ))
}

#[allow(clippy::too_many_arguments)]
fn convert_model(
    source_model: &str,
    target_model: &str,
    requested_collision_families: &BTreeSet<String>,
    namespace: &str,
    donor: &mut ResourceDb,
    game_materials: &BTreeMap<String, i32>,
    game_material_name_overrides: &BTreeMap<String, String>,
    game_material_binding_overrides: Option<&BTreeMap<String, String>>,
    empty_render_material_overrides: Option<&BTreeMap<String, String>>,
    preserve_empty_render_materials: Option<&BTreeSet<String>>,
    drop_empty_render_surfaces: Option<&BTreeSet<String>>,
    allow_missing_game_material_settings: bool,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
    requested_materials: &mut BTreeSet<String>,
) -> Result<ModelAdapterRecord> {
    let source_paths: Vec<String> = donor
        .paths_with_prefix(&format!("{source_model}/"))
        .filter(|path| donor.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    if source_paths.is_empty() {
        bail!("donor model unit is absent: {source_model}");
    }
    if source_paths.iter().any(|path| path.contains("speedtree")) {
        bail!("model {source_model} contains unsupported speedtree render resources");
    }
    let dropped_surfaces = drop_empty_render_surfaces.cloned().unwrap_or_default();
    validate_declared_empty_render_surfaces(donor, source_model, &dropped_surfaces)?;

    let settings_path = format!("{source_model}/settings");
    let render_properties_path = format!("{source_model}/render/export_properties");
    if !source_paths.contains(&settings_path) || !source_paths.contains(&render_properties_path) {
        bail!("model {source_model} lacks settings or render export properties");
    }
    let surface_paths: BTreeMap<String, (String, String)> =
        discover_model_surfaces(source_model, &source_paths)?;
    if surface_paths.is_empty() {
        bail!("model {source_model} has no static render surfaces");
    }

    let mut ordinary_surfaces = 0;
    let mut colored_surfaces = 0;
    let mut surface_names = BTreeSet::new();
    let mut model_min = [f32::INFINITY; 3];
    let mut model_max = [f32::NEG_INFINITY; 3];
    for (surface, (converted_path, properties_path)) in &surface_paths {
        let mut properties =
            parse_surface_properties(&donor.read(properties_path)?, properties_path)?;
        let converted = donor.read(converted_path)?;
        let donor_stride = stripped_vertex_stride(&converted)?;
        match (properties.mesh_type, donor_stride) {
            (0 | 1, 28) => ordinary_surfaces += 1,
            (2, 32) => colored_surfaces += 1,
            (99 | 200, _) => {
                bail!(
                    "model {source_model} surface {surface} uses unsupported mesh type {}",
                    properties.mesh_type
                )
            }
            pair => bail!(
                "model {source_model} surface {surface} has incompatible type/stride {pair:?}"
            ),
        }
        validate_source_counts_and_bounds(&converted, &properties, properties_path)?;
        let StaticUpgrade::Converted(target_bytes) = upgrade_stripped_static_model(&converted)
            .with_context(|| format!("converting target-native static surface {converted_path}"))?
        else {
            bail!("donor static surface unexpectedly used a target-native declaration");
        };
        let target_counts = target_static_counts(&target_bytes, properties.mesh_type)?;
        if target_counts != (properties.vertices, properties.indices) {
            bail!(
                "converted surface count mismatch for {converted_path}: \
                 properties=({}, {}), emitted={target_counts:?}",
                properties.vertices,
                properties.indices
            );
        }
        for axis in 0..3 {
            model_min[axis] = model_min[axis].min(properties.bounds_min[axis]);
            model_max[axis] = model_max[axis].max(properties.bounds_max[axis]);
        }
        surface_names.insert(surface.clone());
        let target_converted = converted_path.replacen(source_model, target_model, 1);
        write_resource(
            overlay,
            &target_converted,
            &target_bytes,
            "adapter:static-surface-v020f-to-v0100b",
            files,
        )?;
        let target_properties = properties_path.replacen(source_model, target_model, 1);
        write_resource(
            overlay,
            &target_properties,
            &properties.config_bytes()?,
            "adapter:surface-properties-v020f-to-v0100b",
            files,
        )?;
    }

    let mut settings =
        BinaryConfig::parse(&donor.read(&settings_path)?).context("parsing model settings")?;
    let collision_materials = synthesize_minimal_settings(
        &mut settings.root,
        &settings_path,
        namespace,
        &surface_names,
        game_materials,
        game_material_name_overrides,
        game_material_binding_overrides,
        empty_render_material_overrides,
        preserve_empty_render_materials,
        &dropped_surfaces,
        allow_missing_game_material_settings,
        requested_materials,
    )?;
    write_resource(
        overlay,
        &settings_path.replacen(source_model, target_model, 1),
        &settings.to_bytes()?,
        "adapter:model-settings-v020f-to-v0100b",
        files,
    )?;

    let mut render_properties = BinaryConfig::parse(&donor.read(&render_properties_path)?)
        .context("parsing model render export properties")?;
    validate_render_properties(
        &mut render_properties.root,
        &render_properties_path,
        model_min,
        model_max,
    )?;
    write_resource(
        overlay,
        &render_properties_path.replacen(source_model, target_model, 1),
        &render_properties.to_bytes()?,
        "adapter:model-properties-v020f-to-v0100b",
        files,
    )?;

    let mut collision_families = BTreeMap::new();
    for family in requested_collision_families {
        collision_families.insert(
            family.clone(),
            convert_collision_family(
                source_model,
                target_model,
                family,
                &source_paths,
                donor,
                overlay,
                files,
                &collision_materials,
            )?,
        );
    }
    validate_binding_override_usage(
        source_model,
        game_material_binding_overrides,
        collision_families.values(),
    )?;

    let recognized: BTreeSet<String> = std::iter::once(settings_path)
        .chain(std::iter::once(render_properties_path))
        .chain(
            surface_paths
                .values()
                .flat_map(|(converted, properties)| [converted.clone(), properties.clone()]),
        )
        .chain(
            source_paths
                .iter()
                .filter(|path| {
                    requested_collision_families
                        .iter()
                        .any(|family| path.contains(&format!("/{family}/")))
                })
                .cloned(),
        )
        .collect();
    let (omitted_siblings, unknown) =
        classify_model_siblings(source_model, &source_paths, &recognized, false);
    if !unknown.is_empty() {
        bail!("model {source_model} has unsupported resource kinds: {unknown:?}");
    }

    Ok(ModelAdapterRecord {
        source: source_model.to_owned(),
        target: target_model.to_owned(),
        ordinary_surfaces,
        colored_surfaces,
        collision_families,
        omitted_siblings,
    })
}

fn classify_model_siblings(
    source_model: &str,
    source_paths: &[String],
    recognized: &BTreeSet<String>,
    allow_render_omission: bool,
) -> (Vec<String>, Vec<String>) {
    let prefix = format!("{source_model}/");
    let mut omitted = Vec::new();
    let mut unknown = Vec::new();
    for path in source_paths {
        if recognized.contains(path) {
            continue;
        }
        let rest = path.strip_prefix(&prefix).unwrap_or(path);
        let known_omission = rest.starts_with("collision/")
            || rest.starts_with("walker_collision/")
            || rest.starts_with("locator")
            || (allow_render_omission && rest.starts_with("render/"));
        if known_omission {
            omitted.push(path.clone());
        } else {
            unknown.push(path.clone());
        }
    }
    (omitted, unknown)
}

fn validate_binding_override_usage<'a>(
    source_model: &str,
    overrides: Option<&BTreeMap<String, String>>,
    families: impl Iterator<Item = &'a CollisionAdapterRecord>,
) -> Result<()> {
    let Some(overrides) = overrides else {
        return Ok(());
    };
    let used: BTreeSet<&str> = families
        .flat_map(|family| family.materials.iter().map(String::as_str))
        .collect();
    let stale: Vec<_> = overrides
        .keys()
        .filter(|surface| !used.contains(surface.as_str()))
        .cloned()
        .collect();
    if !stale.is_empty() {
        bail!(
            "{source_model} has stale game-material binding overrides not used by selected collision families: {stale:?}"
        );
    }
    Ok(())
}

fn discover_model_surfaces(
    model: &str,
    paths: &[String],
) -> Result<BTreeMap<String, (String, String)>> {
    let prefix = format!("{model}/render/");
    let mut converted = BTreeMap::new();
    let mut properties = BTreeMap::new();
    for path in paths {
        let Some(rest) = path.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(surface) = rest.strip_suffix("/converted_model")
            && !surface.contains('/')
        {
            converted.insert(surface.to_owned(), path.clone());
        } else if let Some(surface) = rest.strip_suffix("/export_properties")
            && !surface.is_empty()
            && !surface.contains('/')
        {
            properties.insert(surface.to_owned(), path.clone());
        }
    }
    let names: BTreeSet<_> = converted.keys().chain(properties.keys()).cloned().collect();
    let mut result = BTreeMap::new();
    for name in names {
        result.insert(
            name.clone(),
            (
                converted
                    .remove(&name)
                    .with_context(|| format!("{model} surface {name} has no converted_model"))?,
                properties
                    .remove(&name)
                    .with_context(|| format!("{model} surface {name} has no export_properties"))?,
            ),
        );
    }
    Ok(result)
}

fn validate_declared_empty_render_surfaces(
    donor: &ResourceDb,
    model: &str,
    surfaces: &BTreeSet<String>,
) -> Result<()> {
    for surface in surfaces {
        if surface.is_empty() || surface.contains('/') {
            bail!("{model} has invalid declared empty render surface {surface:?}");
        }
        let folder = format!("{model}/render/{surface}");
        if !donor.contains(&folder) || donor.is_file(&folder) {
            bail!(
                "{model} declared empty render surface {surface:?}, but {folder} is not an archive folder"
            );
        }
        let prefix = format!("{folder}/");
        if donor
            .paths_with_prefix(&prefix)
            .any(|path| donor.is_file(path))
        {
            bail!(
                "{model} declared empty render surface {surface:?}, but its archive folder has payload"
            );
        }
    }
    Ok(())
}

fn parse_surface_properties(bytes: &[u8], path: &str) -> Result<SurfacePropertiesWithConfig> {
    let config = BinaryConfig::parse(bytes)
        .with_context(|| format!("parsing surface export properties {path}"))?;
    let allowed: BTreeSet<&str> = [
        "bounding_sphere",
        "source",
        "type",
        "version",
        "bounding_box",
    ]
    .into_iter()
    .collect();
    validate_named_field_set(&config.root, path, &allowed)?;
    let mesh_type = integer(&config.root, "type", path)?;
    let source = config
        .root
        .named("source")
        .with_context(|| format!("{path} has no source table"))?;
    let source_allowed: BTreeSet<&str> = [
        "indices",
        "primitives",
        "mesh_name",
        "texture_name",
        "vertices",
    ]
    .into_iter()
    .collect();
    validate_named_field_set(source, &format!("{path}.source"), &source_allowed)?;
    let vertices = usize::try_from(integer(source, "vertices", path)?)?;
    let indices = usize::try_from(integer(source, "indices", path)?)?;
    let primitives = usize::try_from(integer(source, "primitives", path)?)?;
    if primitives.checked_mul(3) != Some(indices) {
        bail!("{path} primitive count does not match its index count");
    }
    let bounds = config
        .root
        .named("bounding_box")
        .with_context(|| format!("{path} has no bounding_box"))?;
    let bounds_min = float3(bounds, "min", path)?;
    let bounds_max = float3(bounds, "max", path)?;
    validate_bounds(bounds_min, bounds_max, path)?;
    Ok(SurfacePropertiesWithConfig {
        properties: SurfaceProperties {
            mesh_type,
            vertices,
            indices,
            bounds_min,
            bounds_max,
        },
        config,
    })
}

struct SurfacePropertiesWithConfig {
    properties: SurfaceProperties,
    config: BinaryConfig,
}

impl std::ops::Deref for SurfacePropertiesWithConfig {
    type Target = SurfaceProperties;
    fn deref(&self) -> &Self::Target {
        &self.properties
    }
}

impl SurfacePropertiesWithConfig {
    fn config_bytes(&mut self) -> Result<Vec<u8>> {
        self.config.to_bytes()
    }
}

fn stripped_vertex_stride(bytes: &[u8]) -> Result<usize> {
    if read_u32(bytes, 0)? != 3 {
        bail!("static surface has no leading chunk 3");
    }
    let size = usize::try_from(read_u32(bytes, 4)?)?;
    let count = usize::try_from(read_u32(bytes, 8)?)?;
    if count == 0 || size < 4 || (size - 4) % count != 0 {
        bail!("static surface has invalid declaration-less vertex count/size");
    }
    Ok((size - 4) / count)
}

fn validate_source_counts_and_bounds(
    bytes: &[u8],
    properties: &SurfaceProperties,
    path: &str,
) -> Result<()> {
    let stride = stripped_vertex_stride(bytes)?;
    let vertex_count = usize::try_from(read_u32(bytes, 8)?)?;
    let vertex_end = 12 + vertex_count * stride;
    let vertices = bytes
        .get(12..vertex_end)
        .with_context(|| format!("{path} vertex payload is truncated"))?;
    let (minimum, maximum) = compute_bounds(vertices, stride)?;
    if vertex_count != properties.vertices {
        bail!("{path} source vertex count does not match export properties");
    }
    compare_bounds(
        minimum,
        maximum,
        properties.bounds_min,
        properties.bounds_max,
        path,
    )?;
    if read_u32(bytes, vertex_end)? != 4 {
        bail!("{path} has no chunk 4");
    }
    let index_size = usize::try_from(read_u32(bytes, vertex_end + 4)?)?;
    if index_size < 4 {
        bail!("{path} index chunk is truncated");
    }
    let index_count = usize::try_from(read_u32(bytes, vertex_end + 8)?)?;
    if index_count != properties.indices {
        bail!("{path} source index count does not match export properties");
    }
    Ok(())
}

fn target_static_counts(bytes: &[u8], mesh_type: i32) -> Result<(usize, usize)> {
    let declaration = match mesh_type {
        0 | 1 => 48,
        2 => 56,
        _ => bail!("unsupported target static mesh type {mesh_type}"),
    };
    let vertex_count = usize::try_from(read_u32(bytes, 8 + declaration)?)?;
    let chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    let index_offset = 8 + chunk_size;
    let index_count = usize::try_from(read_u32(bytes, index_offset + 8)?)?;
    Ok((vertex_count, index_count))
}

fn compute_bounds(bytes: &[u8], stride: usize) -> Result<([f32; 3], [f32; 3])> {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in bytes.chunks_exact(stride) {
        for axis in 0..3 {
            let offset = axis * 4;
            let value = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            if !value.is_finite() {
                bail!("model vertex position is not finite");
            }
            minimum[axis] = minimum[axis].min(value);
            maximum[axis] = maximum[axis].max(value);
        }
    }
    validate_bounds(minimum, maximum, "computed geometry")?;
    Ok((minimum, maximum))
}

fn compare_bounds(
    actual_min: [f32; 3],
    actual_max: [f32; 3],
    declared_min: [f32; 3],
    declared_max: [f32; 3],
    path: &str,
) -> Result<()> {
    const TOLERANCE: f32 = 0.002;
    for axis in 0..3 {
        if (actual_min[axis] - declared_min[axis]).abs() > TOLERANCE
            || (actual_max[axis] - declared_max[axis]).abs() > TOLERANCE
        {
            bail!(
                "{path} bounding_box does not enclose emitted vertices within {TOLERANCE}: \
                 actual=({actual_min:?}, {actual_max:?}), declared=({declared_min:?}, {declared_max:?})"
            );
        }
    }
    Ok(())
}

fn synthesize_minimal_settings(
    root: &mut ConfigValue,
    path: &str,
    namespace: &str,
    surfaces: &BTreeSet<String>,
    game_materials: &BTreeMap<String, i32>,
    game_material_name_overrides: &BTreeMap<String, String>,
    game_material_binding_overrides: Option<&BTreeMap<String, String>>,
    empty_render_material_overrides: Option<&BTreeMap<String, String>>,
    preserve_empty_render_materials: Option<&BTreeSet<String>>,
    drop_empty_render_surfaces: &BTreeSet<String>,
    allow_missing_game_material_settings: bool,
    requested_materials: &mut BTreeSet<String>,
) -> Result<BTreeMap<String, (String, i32)>> {
    let ConfigValue::Named(source_materials) = root
        .named("material_settings")
        .with_context(|| format!("{path} has no material_settings"))?
    else {
        bail!("{path}.material_settings is not named");
    };
    let declared: BTreeSet<String> = source_materials
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let stale_drops: Vec<_> = drop_empty_render_surfaces
        .difference(&declared)
        .cloned()
        .collect();
    if !stale_drops.is_empty() {
        bail!("{path} has stale dropped empty render surfaces: {stale_drops:?}");
    }
    let actual: BTreeSet<String> = declared
        .difference(drop_empty_render_surfaces)
        .cloned()
        .collect();
    if actual != *surfaces || declared.len() != source_materials.len() {
        bail!("{path} render/material surface coverage differs or contains duplicates");
    }
    let mut emitted_materials = Vec::with_capacity(source_materials.len());
    for surface in source_materials {
        if drop_empty_render_surfaces.contains(&surface.name) {
            continue;
        }
        let allowed: BTreeSet<&str> = ["material_name", "used_in_portals"].into_iter().collect();
        validate_named_field_set(
            &surface.value,
            &format!("{path}.material_settings.{}", surface.name),
            &allowed,
        )?;
        let ConfigValue::String(material) = surface
            .value
            .named("material_name")
            .with_context(|| format!("{path}.{} has no material_name", surface.name))?
        else {
            bail!("{path}.{}.material_name is not string", surface.name);
        };
        let effective_material = if material.is_empty() {
            if let Some(override_material) =
                empty_render_material_overrides.and_then(|overrides| overrides.get(&surface.name))
            {
                override_material.as_str()
            } else if preserve_empty_render_materials
                .is_some_and(|surfaces| surfaces.contains(&surface.name))
            {
                ""
            } else {
                bail!(
                    "{path}.material_settings.{} has an empty render material reference; add an audited empty_render_material_overrides or preserve_empty_render_materials entry",
                    surface.name
                );
            }
        } else {
            if empty_render_material_overrides
                .is_some_and(|overrides| overrides.contains_key(&surface.name))
            {
                bail!(
                    "{path}.material_settings.{} has a donor material, so its empty-only override is stale",
                    surface.name
                );
            }
            if preserve_empty_render_materials
                .is_some_and(|surfaces| surfaces.contains(&surface.name))
            {
                bail!(
                    "{path}.material_settings.{} has a donor material, so its empty-sentinel preservation is stale",
                    surface.name
                );
            }
            material
        };
        if !effective_material.is_empty() {
            let source_path = format!("material_instances/{effective_material}.material");
            requested_materials.insert(source_path);
        }
        emitted_materials.push(NamedValue {
            name: surface.name.clone(),
            value: ConfigValue::Named(vec![NamedValue {
                name: "material_name".into(),
                value: ConfigValue::String(target_material_reference(
                    namespace,
                    effective_material,
                )),
            }]),
        });
    }
    let lod_hierarchy =
        synthesize_runtime_lod_hierarchy(root, path, surfaces, drop_empty_render_surfaces)?;
    let lod_switching = synthesize_runtime_lod_switching(root, path)?;
    let game_material_settings =
        runtime_game_material_settings(root, path, allow_missing_game_material_settings)?;
    let mut minimal = ConfigValue::Named(vec![
        NamedValue {
            name: "lod_hierrarchy".into(),
            value: lod_hierarchy,
        },
        NamedValue {
            name: "material_settings".into(),
            value: ConfigValue::Named(emitted_materials),
        },
        NamedValue {
            name: "game_material_settings".into(),
            value: game_material_settings,
        },
        NamedValue {
            name: "lod_switching".into(),
            value: lod_switching,
        },
    ]);
    if let Some(overrides) = empty_render_material_overrides {
        let stale: Vec<_> = overrides
            .keys()
            .filter(|surface| !surfaces.contains(*surface))
            .cloned()
            .collect();
        if !stale.is_empty() {
            bail!("{path} has stale empty render-material surface overrides: {stale:?}");
        }
    }
    if let Some(preserved) = preserve_empty_render_materials {
        let stale: Vec<_> = preserved
            .iter()
            .filter(|surface| !surfaces.contains(*surface))
            .cloned()
            .collect();
        if !stale.is_empty() {
            bail!("{path} has stale preserved empty render-material surfaces: {stale:?}");
        }
    }
    let resolved = remap_game_material_settings(
        &mut minimal,
        path,
        game_materials,
        game_material_name_overrides,
        game_material_binding_overrides,
    )?;
    let resolved = apply_game_material_binding_overrides(
        &mut minimal,
        path,
        resolved,
        game_material_binding_overrides,
        game_materials,
    )?;
    strip_game_material_audit_names(&mut minimal, path)?;
    *root = minimal;
    Ok(resolved)
}

fn runtime_game_material_settings(
    root: &ConfigValue,
    path: &str,
    allow_missing: bool,
) -> Result<ConfigValue> {
    match root.named("game_material_settings") {
        Some(settings) => Ok(settings.clone()),
        None if allow_missing => Ok(ConfigValue::Named(Vec::new())),
        None => bail!("{path} has no game_material_settings"),
    }
}

fn synthesize_runtime_lod_hierarchy(
    root: &ConfigValue,
    path: &str,
    surfaces: &BTreeSet<String>,
    drop_empty_render_surfaces: &BTreeSet<String>,
) -> Result<ConfigValue> {
    let hierarchy = root
        .named("lod_hierrarchy")
        .with_context(|| format!("{path} has no runtime-required lod_hierrarchy"))?;
    let allowed_levels: BTreeSet<&str> =
        ["unassigned", "LOD0", "LOD1", "LOD2"].into_iter().collect();
    validate_named_field_set(
        hierarchy,
        &format!("{path}.lod_hierrarchy"),
        &allowed_levels,
    )?;

    if let Some(unassigned) = hierarchy.named("unassigned") {
        validate_named_field_set(
            unassigned,
            &format!("{path}.lod_hierrarchy.unassigned"),
            &BTreeSet::new(),
        )?;
    }

    let mut emitted_levels = Vec::with_capacity(3);
    for level in ["LOD0", "LOD1", "LOD2"] {
        let level_path = format!("{path}.lod_hierrarchy.{level}");
        let value = hierarchy
            .named(level)
            .with_context(|| format!("{path}.lod_hierrarchy has no {level}"))?;
        let allowed_fields: BTreeSet<&str> = ["surfaces"].into_iter().collect();
        validate_named_field_set(value, &level_path, &allowed_fields)?;
        let mut assigned_in_level = BTreeSet::new();
        let level_surfaces = match value.named("surfaces") {
            Some(ConfigValue::Indexed(values)) => {
                let mut result = Vec::with_capacity(values.len());
                for (index, value) in values.iter().enumerate() {
                    let ConfigValue::String(surface) = value else {
                        bail!("{level_path}.surfaces[{index}] is not string");
                    };
                    if drop_empty_render_surfaces.contains(surface) {
                        continue;
                    }
                    if !surfaces.contains(surface) {
                        bail!(
                            "{level_path}.surfaces[{index}] names unknown render surface {surface:?}"
                        );
                    }
                    if !assigned_in_level.insert(surface.clone()) {
                        bail!("{level_path} assigns render surface {surface:?} more than once");
                    }
                    result.push(ConfigValue::String(surface.clone()));
                }
                result
            }
            None => Vec::new(),
            Some(_) => bail!("{level_path}.surfaces is not indexed"),
        };
        emitted_levels.push(NamedValue {
            name: level.into(),
            value: ConfigValue::Named(vec![NamedValue {
                name: "surfaces".into(),
                value: ConfigValue::Indexed(level_surfaces),
            }]),
        });
    }
    if emitted_levels[0]
        .value
        .named("surfaces")
        .is_none_or(|value| matches!(value, ConfigValue::Indexed(values) if values.is_empty()))
    {
        bail!("{path}.lod_hierrarchy.LOD0 has no surfaces");
    }
    Ok(ConfigValue::Named(emitted_levels))
}

fn synthesize_runtime_lod_switching(root: &ConfigValue, path: &str) -> Result<ConfigValue> {
    let Some(switching) = root.named("lod_switching") else {
        return Ok(ConfigValue::Named(vec![
            NamedValue {
                name: "type".into(),
                value: ConfigValue::Integer(0),
            },
            NamedValue {
                name: "default_params".into(),
                value: ConfigValue::Boolean(true),
            },
        ]));
    };
    let allowed: BTreeSet<&str> = ["type", "default_params", "param0", "param1", "param2"]
        .into_iter()
        .collect();
    validate_named_field_set(switching, &format!("{path}.lod_switching"), &allowed)?;

    let lod_type = integer(switching, "type", &format!("{path}.lod_switching"))?;
    if !matches!(lod_type, 0 | 1) {
        bail!("{path}.lod_switching.type {lod_type} is unsupported by v0.100b");
    }
    let ConfigValue::Boolean(default_params) = switching
        .named("default_params")
        .with_context(|| format!("{path}.lod_switching has no default_params"))?
    else {
        bail!("{path}.lod_switching.default_params is not boolean");
    };

    let mut fields = vec![
        NamedValue {
            name: "type".into(),
            value: ConfigValue::Integer(lod_type),
        },
        NamedValue {
            name: "default_params".into(),
            value: ConfigValue::Boolean(*default_params),
        },
    ];
    if !default_params {
        for parameter in ["param0", "param1"] {
            fields.push(NamedValue {
                name: parameter.into(),
                value: finite_lod_parameter(switching, parameter, path)?.clone(),
            });
        }
        if switching.named("param2").is_some() {
            fields.push(NamedValue {
                name: "param2".into(),
                value: finite_lod_parameter(switching, "param2", path)?.clone(),
            });
        }
    }
    Ok(ConfigValue::Named(fields))
}

fn finite_lod_parameter<'a>(
    switching: &'a ConfigValue,
    parameter: &str,
    path: &str,
) -> Result<&'a ConfigValue> {
    let value = switching
        .named(parameter)
        .with_context(|| format!("{path}.lod_switching has no {parameter}"))?;
    let finite = match value {
        ConfigValue::Integer(_) => true,
        ConfigValue::Float(value) => value.is_finite(),
        _ => false,
    };
    if !finite {
        bail!("{path}.lod_switching.{parameter} is not a finite scalar");
    }
    Ok(value)
}

fn target_material_reference(namespace: &str, material: &str) -> String {
    if material.is_empty() {
        String::new()
    } else {
        format!("{namespace}/{material}")
    }
}

fn validate_all_render_materials_empty(
    db: &mut ResourceDb,
    model: &str,
    build: &str,
) -> Result<()> {
    let settings_path = format!("{model}/settings");
    let settings = BinaryConfig::parse(
        &db.read(&settings_path)
            .with_context(|| format!("reading {build} settings for {model}"))?,
    )
    .with_context(|| format!("parsing {build} settings for {model}"))?;
    let ConfigValue::Named(materials) = settings
        .root
        .named("material_settings")
        .with_context(|| format!("{build} {settings_path} has no material_settings"))?
    else {
        bail!("{build} {settings_path}.material_settings is not named");
    };
    if materials.is_empty() {
        bail!("{build} {settings_path} has no render material entries");
    }
    for surface in materials {
        if !matches!(
            surface.value.named("material_name"),
            Some(ConfigValue::String(material)) if material.is_empty()
        ) {
            bail!(
                "{build} {settings_path}.material_settings.{} is not an empty render sentinel",
                surface.name
            );
        }
    }
    Ok(())
}

fn remap_game_material_settings(
    root: &mut ConfigValue,
    path: &str,
    game_materials: &BTreeMap<String, i32>,
    game_material_name_overrides: &BTreeMap<String, String>,
    game_material_binding_overrides: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, (String, i32)>> {
    let ConfigValue::Named(entries) = root
        .named_mut("game_material_settings")
        .with_context(|| format!("{path} has no game_material_settings"))?
    else {
        bail!("{path}.game_material_settings is not named");
    };
    let mut resolved = BTreeMap::new();
    for entry in entries {
        let fields: BTreeSet<&str> = ["game_material_id", "game_material_name"]
            .into_iter()
            .collect();
        validate_named_field_set(
            &entry.value,
            &format!("{path}.game_material_settings.{}", entry.name),
            &fields,
        )?;
        let ConfigValue::String(source_name) = entry
            .value
            .named("game_material_name")
            .with_context(|| format!("{path} game material {} has no name", entry.name))?
        else {
            bail!("{path} game material {} name is not string", entry.name);
        };
        let source_name = source_name.clone();
        let scoped_target_name =
            game_material_binding_overrides.and_then(|overrides| overrides.get(&entry.name));
        let (target_name, target_id) = if let Some(target_name) = scoped_target_name {
            let target_id = *game_materials.get(target_name).with_context(|| {
                format!(
                    "{path} override for {:?} names unknown v0.100b material {target_name:?}",
                    entry.name
                )
            })?;
            (target_name.clone(), target_id)
        } else if source_name.is_empty() || source_name == "not assigned or deleted" {
            if integer(&entry.value, "game_material_id", path)? != 65_535 {
                bail!(
                    "{path} game material {} uses an unassigned name without the stock 65535 sentinel",
                    entry.name
                );
            }
            (String::new(), 65_535)
        } else {
            let target_name = game_material_name_overrides
                .get(&source_name)
                .cloned()
                .unwrap_or(source_name);
            let target_id = *game_materials.get(&target_name).with_context(|| {
                format!("{path} game material {target_name:?} does not resolve to v0.100b")
            })?;
            (target_name, target_id)
        };
        entry.value.set_path(
            "game_material_name",
            ConfigValue::String(target_name.clone()),
        )?;
        entry
            .value
            .set_path("game_material_id", ConfigValue::Integer(target_id))?;
        if resolved
            .insert(entry.name.clone(), (target_name, target_id))
            .is_some()
        {
            bail!("{path} has duplicate game-material surface {}", entry.name);
        }
    }
    Ok(resolved)
}

fn apply_game_material_binding_overrides(
    root: &mut ConfigValue,
    path: &str,
    mut resolved: BTreeMap<String, (String, i32)>,
    overrides: Option<&BTreeMap<String, String>>,
    game_materials: &BTreeMap<String, i32>,
) -> Result<BTreeMap<String, (String, i32)>> {
    let Some(overrides) = overrides else {
        return Ok(resolved);
    };
    let ConfigValue::Named(entries) = root
        .named_mut("game_material_settings")
        .with_context(|| format!("{path} has no game_material_settings"))?
    else {
        bail!("{path}.game_material_settings is not named");
    };
    for (surface, target_name) in overrides {
        let target_id = *game_materials.get(target_name).with_context(|| {
            format!(
                "{path} override for {surface:?} names unknown v0.100b material {target_name:?}"
            )
        })?;
        let replacement = ConfigValue::Named(vec![
            NamedValue {
                name: "game_material_id".into(),
                value: ConfigValue::Integer(target_id),
            },
            NamedValue {
                name: "game_material_name".into(),
                value: ConfigValue::String(target_name.clone()),
            },
        ]);
        if let Some(entry) = entries.iter_mut().find(|entry| entry.name == *surface) {
            entry.value = replacement;
        } else {
            entries.push(NamedValue {
                name: surface.clone(),
                value: replacement,
            });
        }
        resolved.insert(surface.clone(), (target_name.clone(), target_id));
    }
    Ok(resolved)
}

fn strip_game_material_audit_names(root: &mut ConfigValue, path: &str) -> Result<()> {
    let ConfigValue::Named(entries) = root
        .named_mut("game_material_settings")
        .with_context(|| format!("{path} has no game_material_settings"))?
    else {
        bail!("{path}.game_material_settings is not named");
    };
    for entry in entries {
        let ConfigValue::Named(fields) = &mut entry.value else {
            bail!("{path}.game_material_settings.{} is not named", entry.name);
        };
        fields.retain(|field| field.name == "game_material_id");
        if fields.len() != 1 || !matches!(fields[0].value, ConfigValue::Integer(_)) {
            bail!(
                "{path}.game_material_settings.{} has no target game_material_id",
                entry.name
            );
        }
    }
    Ok(())
}

fn validate_render_properties(
    root: &mut ConfigValue,
    path: &str,
    expected_min: [f32; 3],
    expected_max: [f32; 3],
) -> Result<()> {
    let allowed: BTreeSet<&str> = [
        "src_filename",
        "type",
        "version",
        "bounding_box",
        "locators",
    ]
    .into_iter()
    .collect();
    validate_named_field_set(root, path, &allowed)?;
    if let Some(locators) = root.named("locators") {
        let ConfigValue::Indexed(locators) = locators else {
            bail!("{path}.locators is not indexed");
        };
        let fields: BTreeSet<&str> = ["rotation", "position", "name", "bone_idx", "bone_name"]
            .into_iter()
            .collect();
        for (index, locator) in locators.iter().enumerate() {
            let locator_path = format!("{path}.locators[{index}]");
            validate_named_field_set(locator, &locator_path, &fields)?;
            for field in ["rotation", "position"] {
                if float3(locator, field, &locator_path)?
                    .into_iter()
                    .any(|value| !value.is_finite())
                {
                    bail!("{locator_path}.{field} is not finite");
                }
            }
            integer(locator, "bone_idx", &locator_path)?;
            for field in ["name", "bone_name"] {
                if !matches!(locator.named(field), Some(ConfigValue::String(_))) {
                    bail!("{locator_path}.{field} is not string");
                }
            }
        }
        let ConfigValue::Named(fields) = root else {
            unreachable!();
        };
        fields.retain(|field| field.name != "locators");
    }
    let bounds = root
        .named("bounding_box")
        .with_context(|| format!("{path} has no bounding_box"))?;
    let minimum = float3(bounds, "min", path)?;
    let maximum = float3(bounds, "max", path)?;
    validate_bounds(minimum, maximum, path)?;
    root.set_path("bounding_box.min", ConfigValue::Float3(expected_min))?;
    root.set_path("bounding_box.max", ConfigValue::Float3(expected_max))?;
    if root.named("src_filename").is_some() {
        root.set_path(
            "src_filename",
            ConfigValue::String("target-native-reconstruction".into()),
        )?;
    }
    Ok(())
}

pub(crate) fn load_game_materials(target: &mut ResourceDb) -> Result<BTreeMap<String, i32>> {
    let config = BinaryConfig::parse(&target.read("game_materials/game.materials")?)
        .context("parsing target game-material dictionary")?;
    let ConfigValue::Indexed(materials) = config
        .root
        .named("materials")
        .context("target game-material dictionary has no materials")?
    else {
        bail!("target game-material materials is not indexed");
    };
    let mut result = BTreeMap::new();
    for material in materials {
        let ConfigValue::String(name) = material
            .named("name")
            .context("target game material has no name")?
        else {
            bail!("target game material name is not string");
        };
        let id = integer(material, "id", "game_materials/game.materials")?;
        if result.insert(name.clone(), id).is_some() {
            bail!("duplicate target game material name {name}");
        }
    }
    Ok(result)
}

fn preflight_collision_material_bindings(
    models: &BTreeMap<String, ModelRequest>,
    manifest: &TargetNativeMapManifest,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    target_game_materials: &BTreeMap<String, i32>,
    output: &Path,
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut entries = Vec::<CollisionBindingPreflightEntry>::new();
    for (source_model, request) in models {
        if request.collision_families.is_empty() {
            continue;
        }
        let settings_path = format!("{source_model}/settings");
        let donor_settings = BinaryConfig::parse(&donor.read(&settings_path)?)
            .with_context(|| format!("preflighting collision settings {settings_path}"))?;
        let donor_bindings = read_game_material_evidence(&donor_settings.root, &settings_path)?;
        let stock_bindings = if target.is_file(&settings_path) {
            let stock_settings =
                BinaryConfig::parse(&target.read(&settings_path)?).with_context(|| {
                    format!("preflighting stock collision settings {settings_path}")
                })?;
            read_optional_game_material_evidence(&stock_settings.root, &settings_path)?
        } else {
            BTreeMap::new()
        };
        let scoped_bindings = manifest.game_material_binding_overrides.get(source_model);
        let donor_regular_collision = collision_material_histogram(
            donor,
            &format!("{source_model}/collision"),
            &donor_bindings,
            scoped_bindings,
            &manifest.game_material_name_overrides,
            target_game_materials,
            false,
        )?;
        let stock_regular_collision = collision_material_histogram(
            target,
            &format!("{source_model}/collision"),
            &stock_bindings,
            None,
            &BTreeMap::new(),
            target_game_materials,
            true,
        )?;
        let parent_assigned_target_materials: BTreeSet<String> = donor_bindings
            .iter()
            .filter_map(|(key, evidence)| {
                resolve_game_material_key(
                    key,
                    evidence,
                    scoped_bindings,
                    &manifest.game_material_name_overrides,
                    target_game_materials,
                    false,
                )
            })
            .collect();
        for family in &request.collision_families {
            let base = format!("{source_model}/{family}");
            let donor_member_hashes = collision_family_hashes(donor, &base)?;
            let stock_member_hashes = collision_family_hashes(target, &base)?;
            let stock_family_available = !stock_member_hashes.is_empty();
            let family_is_identical =
                stock_family_available && donor_member_hashes == stock_member_hashes;
            let mut used_keys = BTreeSet::new();
            let face_data_path = format!("{base}/face_data");
            let indices_path = format!("{base}/indices");
            if donor.is_file(&face_data_path) {
                let index_bytes = donor.read(&indices_path)?;
                let index_payload = required_single_chunk(&indices_path, &index_bytes, 0x1a)?;
                if index_payload.len() % 12 != 0 {
                    bail!(
                        "{indices_path} cannot preflight face bindings: index byte count {} is not triangle-aligned",
                        index_payload.len()
                    );
                }
                let names = validate_face_data(
                    &face_data_path,
                    &donor.read(&face_data_path)?,
                    index_payload.len() / 12,
                )?;
                used_keys.extend(names);
            }

            let primitives_path = format!("{base}/exported_primitives");
            if donor.is_file(&primitives_path) {
                let primitives = BinaryConfig::parse(&donor.read(&primitives_path)?)
                    .with_context(|| format!("preflighting {primitives_path}"))?;
                let (_, names) = validate_exported_primitives(&primitives.root, &primitives_path)?;
                used_keys.extend(names);
            }

            for key in used_keys {
                let declared = manifest
                    .game_material_binding_overrides
                    .get(source_model)
                    .and_then(|bindings| bindings.get(&key));
                let donor_setting = donor_bindings.get(&key).cloned();
                let donor_resolves = donor_setting
                    .as_ref()
                    .and_then(|binding| {
                        resolved_target_game_material(
                            binding,
                            &manifest.game_material_name_overrides,
                            target_game_materials,
                        )
                    })
                    .is_some();
                if declared.is_none() && donor_resolves {
                    continue;
                }
                let stock_setting = stock_bindings.get(&key).cloned();
                let identity_candidate = if family_is_identical {
                    stock_setting.as_ref().and_then(|binding| {
                        exact_stock_game_material(binding, target_game_materials)
                    })
                } else {
                    None
                };
                entries.push(CollisionBindingPreflightEntry {
                    model: source_model.clone(),
                    family: family.clone(),
                    key,
                    donor_member_hashes: donor_member_hashes.clone(),
                    stock_family_available,
                    stock_member_hashes: stock_member_hashes.clone(),
                    donor_setting,
                    stock_setting,
                    donor_regular_collision: donor_regular_collision.clone(),
                    stock_regular_collision: stock_regular_collision.clone(),
                    parent_assigned_target_materials: parent_assigned_target_materials.clone(),
                    status: "pending".into(),
                    reason: String::new(),
                    suggested_target_binding: declared.cloned(),
                    identity_candidate,
                });
            }
        }
    }

    let mut grouped = BTreeMap::<(String, String), Vec<usize>>::new();
    for (index, entry) in entries.iter().enumerate() {
        grouped
            .entry((entry.model.clone(), entry.key.clone()))
            .or_default()
            .push(index);
    }
    let mut auto_proven = BTreeMap::<String, BTreeMap<String, String>>::new();
    for ((model, key), indices) in grouped {
        if let Some(declared) = manifest
            .game_material_binding_overrides
            .get(&model)
            .and_then(|bindings| bindings.get(&key))
        {
            for index in indices {
                entries[index].status = "manifest_override".into();
                entries[index].reason =
                    "explicit audited model/key binding from source manifest".into();
                entries[index].suggested_target_binding = Some(declared.clone());
            }
            continue;
        }
        let mut pending = Vec::new();
        for index in indices {
            if entries[index]
                .donor_setting
                .as_ref()
                .is_some_and(is_explicit_default_sentinel)
            {
                entries[index].status = "target_runtime_default_sentinel".into();
                entries[index].reason =
                    "donor parent settings explicitly preserve game material ID 65535; recovered v0.100b collision cookers retain 0xffff and the runtime material manager safely resolves an unknown ID through its default material".into();
                entries[index].suggested_target_binding = None;
            } else if entries[index].family == "walker_collision" {
                entries[index].status = "target_runtime_fallback_zero".into();
                entries[index].reason =
                    "recovered v0.100b walker cooker intentionally maps a missing family-local setting key to material ID 0; it does not inherit regular collision materials".into();
                entries[index].suggested_target_binding = None;
            } else {
                pending.push(index);
            }
        }
        if pending.is_empty() {
            continue;
        }
        let candidates: BTreeSet<String> = pending
            .iter()
            .filter_map(|index| entries[*index].identity_candidate.clone())
            .collect();
        let every_family_proves_identity = pending
            .iter()
            .all(|index| entries[*index].identity_candidate.is_some());
        if every_family_proves_identity && candidates.len() == 1 {
            let target_name = candidates.into_iter().next().unwrap();
            auto_proven
                .entry(model.clone())
                .or_default()
                .insert(key.clone(), target_name.clone());
            for index in pending {
                entries[index].status = "auto_proven_identical_family".into();
                entries[index].reason =
                    "donor and stock collision-family member hashes are identical and the same stock dictionary key has an assigned target binding".into();
                entries[index].suggested_target_binding = Some(target_name.clone());
            }
            continue;
        }
        for index in pending {
            entries[index].status = "unresolved".into();
            entries[index].reason = if !entries[index].stock_family_available {
                "corresponding stock collision family is absent".into()
            } else if entries[index].donor_member_hashes != entries[index].stock_member_hashes {
                "donor and stock collision-family member hashes differ".into()
            } else if entries[index].stock_setting.is_none() {
                "identical stock family has no setting for this dictionary key".into()
            } else {
                "stock dictionary key is unassigned, inconsistent with the target dictionary, or conflicts across selected families".into()
            };
        }
    }

    entries.sort_by(|left, right| {
        (&left.model, &left.family, &left.key).cmp(&(&right.model, &right.family, &right.key))
    });
    let report = CollisionBindingPreflightReport {
        format_version: 2,
        asset_id: manifest.asset.id.clone(),
        entries,
    };
    let report_path = output.join("collision-binding-preflight.json");
    to_writer_pretty(fs::File::create(&report_path)?, &report)?;
    let unresolved: Vec<_> = report
        .entries
        .iter()
        .filter(|entry| entry.status == "unresolved")
        .map(|entry| format!("{}/{}/{}", entry.model, entry.family, entry.key))
        .collect();
    if !unresolved.is_empty() {
        bail!(
            "collision material binding preflight has {} unresolved model/family/key entries; see {}:\n  {}",
            unresolved.len(),
            report_path.display(),
            unresolved.join("\n  ")
        );
    }
    Ok(auto_proven)
}

fn read_game_material_evidence(
    root: &ConfigValue,
    path: &str,
) -> Result<BTreeMap<String, GameMaterialEvidence>> {
    let ConfigValue::Named(entries) = root
        .named("game_material_settings")
        .with_context(|| format!("{path} has no game_material_settings"))?
    else {
        bail!("{path}.game_material_settings is not named");
    };
    let mut result = BTreeMap::new();
    for entry in entries {
        let ConfigValue::String(name) = entry
            .value
            .named("game_material_name")
            .with_context(|| format!("{path} game material {} has no name", entry.name))?
        else {
            bail!("{path} game material {} name is not string", entry.name);
        };
        let evidence = GameMaterialEvidence {
            name: name.clone(),
            id: integer(&entry.value, "game_material_id", path)?,
        };
        if result.insert(entry.name.clone(), evidence).is_some() {
            bail!("{path} has duplicate game-material surface {}", entry.name);
        }
    }
    Ok(result)
}

fn read_optional_game_material_evidence(
    root: &ConfigValue,
    path: &str,
) -> Result<BTreeMap<String, GameMaterialEvidence>> {
    if root.named("game_material_settings").is_none() {
        return Ok(BTreeMap::new());
    }
    read_game_material_evidence(root, path)
}

fn is_explicit_default_sentinel(binding: &GameMaterialEvidence) -> bool {
    binding.id == 65_535 && (binding.name.is_empty() || binding.name == "not assigned or deleted")
}

fn resolved_target_game_material(
    evidence: &GameMaterialEvidence,
    name_overrides: &BTreeMap<String, String>,
    target_game_materials: &BTreeMap<String, i32>,
) -> Option<String> {
    if evidence.id == 65_535
        || evidence.name.is_empty()
        || evidence.name == "not assigned or deleted"
    {
        return None;
    }
    let target_name = name_overrides
        .get(&evidence.name)
        .cloned()
        .unwrap_or_else(|| evidence.name.clone());
    target_game_materials
        .contains_key(&target_name)
        .then_some(target_name)
}

fn exact_stock_game_material(
    evidence: &GameMaterialEvidence,
    target_game_materials: &BTreeMap<String, i32>,
) -> Option<String> {
    if evidence.id == 65_535 || evidence.name.is_empty() {
        return None;
    }
    matches!(
        target_game_materials.get(&evidence.name),
        Some(target_id) if *target_id == evidence.id
    )
    .then_some(evidence.name.clone())
}

fn resolve_game_material_key(
    key: &str,
    evidence: &GameMaterialEvidence,
    scoped_bindings: Option<&BTreeMap<String, String>>,
    name_overrides: &BTreeMap<String, String>,
    target_game_materials: &BTreeMap<String, i32>,
    require_exact_target_id: bool,
) -> Option<String> {
    if let Some(target_name) = scoped_bindings.and_then(|bindings| bindings.get(key)) {
        return target_game_materials
            .contains_key(target_name)
            .then_some(target_name.clone());
    }
    if require_exact_target_id {
        exact_stock_game_material(evidence, target_game_materials)
    } else {
        resolved_target_game_material(evidence, name_overrides, target_game_materials)
    }
}

fn collision_material_histogram(
    db: &mut ResourceDb,
    base: &str,
    bindings: &BTreeMap<String, GameMaterialEvidence>,
    scoped_bindings: Option<&BTreeMap<String, String>>,
    name_overrides: &BTreeMap<String, String>,
    target_game_materials: &BTreeMap<String, i32>,
    require_exact_target_id: bool,
) -> Result<Option<MaterialHistogramEvidence>> {
    let Some(key_counts) = collision_material_key_counts(db, base)? else {
        return Ok(None);
    };
    let mut target_counts = BTreeMap::new();
    let mut fully_resolved = true;
    for (key, count) in &key_counts {
        let Some(evidence) = bindings.get(key) else {
            fully_resolved = false;
            continue;
        };
        let Some(target_name) = resolve_game_material_key(
            key,
            evidence,
            scoped_bindings,
            name_overrides,
            target_game_materials,
            require_exact_target_id,
        ) else {
            fully_resolved = false;
            continue;
        };
        *target_counts.entry(target_name).or_insert(0) += count;
    }
    Ok(Some(MaterialHistogramEvidence {
        key_counts,
        target_counts,
        fully_resolved,
    }))
}

fn collision_material_key_counts(
    db: &mut ResourceDb,
    base: &str,
) -> Result<Option<BTreeMap<String, usize>>> {
    let mut counts = BTreeMap::new();
    let face_data_path = format!("{base}/face_data");
    if db.is_file(&face_data_path) {
        let indices_path = format!("{base}/indices");
        let index_bytes = db.read(&indices_path)?;
        let index_payload = required_single_chunk(&indices_path, &index_bytes, 0x1a)?;
        if index_payload.len() % 12 != 0 {
            bail!("{indices_path} is not triangle-aligned");
        }
        let face_bytes = db.read(&face_data_path)?;
        validate_face_data(&face_data_path, &face_bytes, index_payload.len() / 12)?;
        let dictionary_size = usize::try_from(read_u32(&face_bytes, 4)?)?;
        let dictionary = &face_bytes[8..8 + dictionary_size];
        let mut names = Vec::new();
        let mut cursor = 2;
        while cursor < dictionary.len() {
            let end = dictionary[cursor..]
                .iter()
                .position(|byte| *byte == 0)
                .map(|length| cursor + length)
                .context("validated collision dictionary lost its terminator")?;
            names.push(std::str::from_utf8(&dictionary[cursor..end])?.to_owned());
            cursor = end + 1;
        }
        let face_offset = 8 + dictionary_size + 8;
        for raw in face_bytes[face_offset..].chunks_exact(2) {
            let index = usize::from(u16::from_le_bytes(raw.try_into().unwrap()));
            *counts.entry(names[index].clone()).or_insert(0) += 1;
        }
    }

    let primitives_path = format!("{base}/exported_primitives");
    if db.is_file(&primitives_path) {
        let primitives = BinaryConfig::parse(&db.read(&primitives_path)?)
            .with_context(|| format!("parsing histogram source {primitives_path}"))?;
        validate_exported_primitives(&primitives.root, &primitives_path)?;
        let ConfigValue::Indexed(materials) = primitives.root.named("mtl_list").unwrap() else {
            unreachable!();
        };
        let material_names: Vec<&str> = materials
            .iter()
            .map(|value| {
                let ConfigValue::String(name) = value else {
                    unreachable!();
                };
                name.as_str()
            })
            .collect();
        let ConfigValue::Indexed(entries) = primitives.root.named("primitives").unwrap() else {
            unreachable!();
        };
        for primitive in entries {
            let index = usize::try_from(integer(primitive, "mtl", &primitives_path)?)?;
            *counts.entry(material_names[index].to_owned()).or_insert(0) += 1;
        }
    }
    Ok((!counts.is_empty()).then_some(counts))
}

fn collision_family_hashes(db: &mut ResourceDb, base: &str) -> Result<BTreeMap<String, String>> {
    let prefix = format!("{base}/");
    let paths: Vec<String> = db
        .paths_with_prefix(&prefix)
        .filter(|path| db.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    let mut result = BTreeMap::new();
    for path in paths {
        let relative = path
            .strip_prefix(&prefix)
            .context("collision family member escaped its prefix")?;
        result.insert(
            relative.to_owned(),
            format!("{:x}", Sha256::digest(db.read(&path)?)),
        );
    }
    Ok(result)
}

fn convert_collision_family(
    source_model: &str,
    target_model: &str,
    family: &str,
    paths: &[String],
    donor: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
    collision_materials: &BTreeMap<String, (String, i32)>,
) -> Result<CollisionAdapterRecord> {
    if !matches!(family, "collision" | "walker_collision") {
        bail!("unsupported collision family {family:?}");
    }
    let family_marker = format!("/{family}/");
    let collision_paths: Vec<_> = paths
        .iter()
        .filter(|path| path.contains(&family_marker))
        .cloned()
        .collect();
    if collision_paths.is_empty() {
        bail!("{source_model} is referenced through {family}, but that family is absent");
    }
    let base = format!("{source_model}/{family}");
    let vertices_path = format!("{base}/vertices");
    let indices_path = format!("{base}/indices");
    let face_data_path = format!("{base}/face_data");
    let mesh_members = [&vertices_path, &indices_path, &face_data_path];
    let present_mesh_members = mesh_members
        .iter()
        .filter(|required| collision_paths.contains(required))
        .count();
    if present_mesh_members != 0 && present_mesh_members != mesh_members.len() {
        bail!("collision family {base} contains a partial triangle-mesh family");
    }
    let mut used_materials = BTreeSet::new();
    let mut fallback_zero_materials = BTreeSet::new();
    let mut default_sentinel_materials = BTreeSet::new();
    let triangles = if present_mesh_members == mesh_members.len() {
        let vertices = donor.read(&vertices_path)?;
        let indices = donor.read(&indices_path)?;
        let face_data = donor.read(&face_data_path)?;
        let vertex_payload = required_single_chunk(&vertices_path, &vertices, 0x19)?;
        if vertex_payload.len() % 12 != 0 {
            bail!("{vertices_path} payload is not float3 aligned");
        }
        let vertex_count = vertex_payload.len() / 12;
        compute_bounds(vertex_payload, 12)?;
        let index_payload = required_single_chunk(&indices_path, &indices, 0x1a)?;
        if index_payload.len() % 4 != 0 {
            bail!("{indices_path} payload is not u32 aligned");
        }
        let index_count = index_payload.len() / 4;
        if index_count % 3 != 0 {
            bail!("{indices_path} index count is not divisible by three");
        }
        for raw in index_payload.chunks_exact(4) {
            let index = usize::try_from(u32::from_le_bytes(raw.try_into().unwrap()))?;
            if index >= vertex_count {
                bail!("{indices_path} contains out-of-range vertex {index}/{vertex_count}");
            }
        }
        let face_materials = validate_face_data(&face_data_path, &face_data, index_count / 3)?;
        let resolution = validate_collision_material_names(
            &face_materials,
            collision_materials,
            &face_data_path,
            family == "walker_collision",
        )?;
        fallback_zero_materials.extend(resolution.fallback_zero);
        default_sentinel_materials.extend(resolution.default_sentinel);
        used_materials.extend(face_materials);
        for source in mesh_members {
            let target_path = source.replacen(source_model, target_model, 1);
            write_resource(
                overlay,
                &target_path,
                &donor.read(source)?,
                "adapter:collision-mesh-identity-v020f-v0100b-validated",
                files,
            )?;
        }
        Some(index_count / 3)
    } else {
        None
    };

    let primitives_path = format!("{base}/exported_primitives");
    let primitives = if collision_paths.contains(&primitives_path) {
        let config = BinaryConfig::parse(&donor.read(&primitives_path)?)
            .with_context(|| format!("parsing {primitives_path}"))?;
        let (count, primitive_materials) =
            validate_exported_primitives(&config.root, &primitives_path)?;
        let resolution = validate_collision_material_names(
            &primitive_materials,
            collision_materials,
            &primitives_path,
            family == "walker_collision",
        )?;
        fallback_zero_materials.extend(resolution.fallback_zero);
        default_sentinel_materials.extend(resolution.default_sentinel);
        used_materials.extend(primitive_materials);
        write_resource(
            overlay,
            &primitives_path.replacen(source_model, target_model, 1),
            &config.to_bytes()?,
            "adapter:collision-primitives-v020f-v0100b-validated",
            files,
        )?;
        Some(count)
    } else {
        None
    };
    if triangles.is_none() && primitives.is_none() {
        bail!("collision family {base} contains no validated payload");
    }
    let extras: Vec<_> = collision_paths
        .iter()
        .filter(|path| !mesh_members.contains(path) && path.as_str() != primitives_path)
        .cloned()
        .collect();
    if !extras.is_empty() {
        bail!("collision family {base} has unsupported resources: {extras:?}");
    }
    Ok(CollisionAdapterRecord {
        triangles,
        primitives,
        materials: used_materials,
        fallback_zero_materials,
        default_sentinel_materials,
    })
}

fn validate_exported_primitives(
    root: &ConfigValue,
    path: &str,
) -> Result<(usize, BTreeSet<String>)> {
    let allowed: BTreeSet<&str> = ["primitives", "mtl_list"].into_iter().collect();
    validate_named_field_set(root, path, &allowed)?;
    let ConfigValue::Indexed(materials) = root
        .named("mtl_list")
        .with_context(|| format!("{path} has no mtl_list"))?
    else {
        bail!("{path}.mtl_list is not indexed");
    };
    if materials.is_empty()
        || materials
            .iter()
            .any(|value| !matches!(value, ConfigValue::String(name) if !name.is_empty()))
    {
        bail!("{path}.mtl_list contains an empty or non-string material");
    }
    let material_names: BTreeSet<String> = materials
        .iter()
        .map(|value| {
            let ConfigValue::String(name) = value else {
                unreachable!();
            };
            name.clone()
        })
        .collect();
    if material_names.len() != materials.len() {
        bail!("{path}.mtl_list contains duplicate materials");
    }
    let ConfigValue::Indexed(primitives) = root
        .named("primitives")
        .with_context(|| format!("{path} has no primitives"))?
    else {
        bail!("{path}.primitives is not indexed");
    };
    if primitives.is_empty() {
        bail!("{path}.primitives is empty");
    }
    let primitive_fields: BTreeSet<&str> = ["rotation", "position", "type", "mtl", "scale"]
        .into_iter()
        .collect();
    for (index, primitive) in primitives.iter().enumerate() {
        let primitive_path = format!("{path}.primitives[{index}]");
        validate_named_field_set(primitive, &primitive_path, &primitive_fields)?;
        for field in ["rotation", "position", "scale"] {
            let values = float3(primitive, field, &primitive_path)?;
            if values.into_iter().any(|value| !value.is_finite()) {
                bail!("{primitive_path}.{field} is not finite");
            }
        }
        let primitive_type = integer(primitive, "type", &primitive_path)?;
        if !matches!(primitive_type, 0..=3) {
            bail!(
                "{primitive_path}.type {primitive_type} is unsupported by the v0.100b physics factory"
            );
        }
        let scale = float3(primitive, "scale", &primitive_path)?;
        match primitive_type {
            0 => {
                if scale[0] <= 0.0 || scale[1] != 0.0 || scale[2] != 0.0 {
                    bail!(
                        "{primitive_path}.scale {scale:?} is not canonical sphere [radius>0, 0, 0]"
                    );
                }
            }
            1 => {
                if scale.iter().any(|value| *value < 0.0) {
                    bail!("{primitive_path}.scale {scale:?} is not a non-negative box half-extent");
                }
            }
            2 => {
                if scale[0] <= 0.0 || scale[1] <= 0.0 || scale[2] != 0.0 {
                    bail!(
                        "{primitive_path}.scale {scale:?} is not canonical cylinder [half-length>0, radius>0, 0]"
                    );
                }
            }
            3 => {
                if scale[0] <= 0.0 || scale[1] < 0.0 || scale[2] != 0.0 {
                    bail!(
                        "{primitive_path}.scale {scale:?} is not canonical capsule [radius>0, height>=0, 0]"
                    );
                }
            }
            _ => unreachable!(),
        }
        let material = usize::try_from(integer(primitive, "mtl", &primitive_path)?)?;
        if material >= materials.len() {
            bail!(
                "{primitive_path}.mtl {material} is outside mtl_list/{}",
                materials.len()
            );
        }
    }
    Ok((primitives.len(), material_names))
}

fn validate_collision_material_names(
    names: &BTreeSet<String>,
    settings: &BTreeMap<String, (String, i32)>,
    path: &str,
    allow_fallback_zero: bool,
) -> Result<CollisionMaterialResolution> {
    let mut resolution = CollisionMaterialResolution::default();
    let mut invalid = BTreeSet::new();
    for name in names {
        match settings.get(name) {
            Some((_, 65_535)) => {
                resolution.default_sentinel.insert(name.clone());
            }
            Some((target_name, _)) if !target_name.is_empty() => {}
            None if allow_fallback_zero => {
                resolution.fallback_zero.insert(name.clone());
            }
            _ => {
                invalid.insert(name.clone());
            }
        }
    }
    if !invalid.is_empty() {
        bail!(
            "{path} material dictionary has absent or unassigned parent game_material_settings: {invalid:?}"
        );
    }
    Ok(resolution)
}

fn required_single_chunk<'a>(path: &str, bytes: &'a [u8], id: u32) -> Result<&'a [u8]> {
    if read_u32(bytes, 0)? != id {
        bail!("{path} has unexpected chunk id, expected {id:#x}");
    }
    let size = usize::try_from(read_u32(bytes, 4)?)?;
    if bytes.len() != 8 + size {
        bail!("{path} has trailing or truncated chunk payload");
    }
    Ok(&bytes[8..])
}

fn validate_face_data(path: &str, bytes: &[u8], triangles: usize) -> Result<BTreeSet<String>> {
    let mut offset = 0;
    if read_u32(bytes, offset)? != 0x1b {
        bail!("{path} has no material dictionary chunk 0x1b");
    }
    let dictionary_size = usize::try_from(read_u32(bytes, offset + 4)?)?;
    let dictionary = bytes
        .get(offset + 8..offset + 8 + dictionary_size)
        .context("collision material dictionary is truncated")?;
    if dictionary.len() < 2 {
        bail!("{path} material dictionary is empty");
    }
    let material_count = usize::from(u16::from_le_bytes(dictionary[..2].try_into().unwrap()));
    let mut names = BTreeSet::new();
    let mut cursor = 2;
    while cursor < dictionary.len() {
        let end = dictionary[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|length| cursor + length)
            .context("collision material dictionary has unterminated name")?;
        if end == cursor {
            bail!("collision material dictionary has an empty name");
        }
        let name = std::str::from_utf8(&dictionary[cursor..end])
            .with_context(|| format!("{path} material name is not UTF-8"))?;
        if !names.insert(name.to_owned()) {
            bail!("{path} material dictionary contains duplicate {name:?}");
        }
        cursor = end + 1;
    }
    if names.len() != material_count {
        bail!(
            "{path} dictionary count {material_count} differs from {} names",
            names.len()
        );
    }
    offset += 8 + dictionary_size;
    if read_u32(bytes, offset)? != 0x1c {
        bail!("{path} has no face-material chunk 0x1c");
    }
    let face_size = usize::try_from(read_u32(bytes, offset + 4)?)?;
    if bytes.len() != offset + 8 + face_size || face_size != triangles * 2 {
        bail!("{path} face-material count does not match {triangles} triangles");
    }
    for raw in bytes[offset + 8..].chunks_exact(2) {
        let material = usize::from(u16::from_le_bytes(raw.try_into().unwrap()));
        if material >= material_count {
            bail!("{path} face references material {material}/{material_count}");
        }
    }
    Ok(names)
}

#[allow(clippy::too_many_arguments)]
fn convert_material_and_textures(
    manifest: &TargetNativeMapManifest,
    donor_path: &str,
    catalog: &MaterialCatalog,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    compatibility: Option<&mut ResourceDb>,
    target_dds_formats: &BTreeSet<[u8; 4]>,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
    materials: &mut BTreeMap<String, String>,
    textures: &mut BTreeMap<String, String>,
    reused_target_textures: &mut BTreeMap<String, String>,
    used_target_texture_fallbacks: &mut BTreeSet<String>,
) -> Result<()> {
    if materials.contains_key(donor_path) {
        return Ok(());
    }
    let relative = donor_path
        .strip_prefix("material_instances/")
        .and_then(|path| path.strip_suffix(".material"))
        .context("invalid donor material path")?;
    let target_path = format!(
        "material_instances/{}/{relative}.material",
        manifest.namespace
    );
    if !donor.is_file(donor_path) {
        if !target.is_file(donor_path) {
            bail!("donor and target both lack referenced material {donor_path}");
        }
        let bytes = target.read(donor_path)?;
        write_resource(
            overlay,
            &target_path,
            &bytes,
            &format!("target-copy:{donor_path}"),
            files,
        )?;
        materials.insert(donor_path.to_owned(), target_path);
        return Ok(());
    }
    let donor_bytes = donor
        .read(donor_path)
        .with_context(|| format!("reading donor material {donor_path}"))?;
    let compatibility_path = manifest
        .material_compatibility_templates
        .get(donor_path)
        .map(String::as_str);
    let conversion_bytes = if let Some(compatibility_path) = compatibility_path {
        let compatibility = compatibility.with_context(|| {
            format!(
                "material {donor_path} declares compatibility template {compatibility_path} \
                 but no compatibility database is open"
            )
        })?;
        if !compatibility.is_file(compatibility_path) {
            bail!(
                "compatibility database lacks declared material template {compatibility_path} \
                 for {donor_path}"
            );
        }
        compatibility
            .read(compatibility_path)
            .with_context(|| format!("reading compatibility material {compatibility_path}"))?
    } else {
        donor_bytes.clone()
    };
    let converted = catalog.convert(
        donor_path,
        &conversion_bytes,
        manifest
            .material_template_overrides
            .get(donor_path)
            .map(String::as_str),
        donor,
        target,
    )?;
    let mut config = BinaryConfig::parse(&converted.bytes)?;
    let mut texture_bindings = converted.donor_texture_bindings.clone();
    if let Some(overrides) = manifest.material_texture_overrides.get(donor_path) {
        for (field_path, texture) in overrides {
            texture_bindings.insert(field_path.clone(), texture.clone());
        }
    }
    let explicitly_bound_texture_fields: BTreeSet<String> =
        texture_bindings.keys().cloned().collect();
    for (field_path, value) in &texture_bindings {
        let base = canonical_texture_reference(value)?;
        let donor_dds = format!("textures/{base}.dds");
        if manifest.target_texture_fallbacks.contains(&donor_dds) {
            let target_dds = format!("textures/{base}.dds");
            let target_options = format!("textures/{base}.options");
            if !target.is_file(&target_dds) || !target.is_file(&target_options) {
                bail!(
                    "declared target texture fallback {donor_dds} lacks its target DDS/options pair"
                );
            }
            config
                .root
                .set_path(field_path, ConfigValue::String(base.clone()))?;
            reused_target_textures.insert(
                format!("{donor_path}#{field_path}"),
                format!("{target_dds} (manifest fallback)"),
            );
            used_target_texture_fallbacks.insert(donor_dds);
        } else if donor.is_file(&donor_dds) {
            let target_base = format!("{}/{}", manifest.namespace, base);
            let target_dds = format!("textures/{target_base}.dds");
            if let Some(existing) = textures.get(&donor_dds) {
                if existing != &target_dds {
                    bail!(
                        "donor texture {donor_dds} resolved to conflicting targets {existing} and {target_dds}"
                    );
                }
            } else {
                convert_texture_pair(
                    &donor_dds,
                    &target_base,
                    donor,
                    target_dds_formats,
                    overlay,
                    files,
                )?;
                textures.insert(donor_dds, target_dds);
            }
            config
                .root
                .set_path(field_path, ConfigValue::String(target_base.clone()))
                .with_context(|| {
                    format!(
                        "rewriting donor texture field {field_path} in converted material {donor_path}"
                    )
                })?;
        } else if target.is_file(&format!("textures/{base}.dds"))
            && target.is_file(&format!("textures/{base}.options"))
        {
            config
                .root
                .set_path(field_path, ConfigValue::String(base.clone()))?;
            reused_target_textures.insert(
                format!("{donor_path}#{field_path}"),
                format!("textures/{base}.dds"),
            );
        } else {
            bail!("merged donor texture reference {value:?} has no DDS payload");
        }
    }
    let inherited_texture_bindings = collect_material_texture_bindings(&config.root);
    for (field_path, value) in inherited_texture_bindings {
        if explicitly_bound_texture_fields.contains(&field_path) {
            continue;
        }
        let base = canonical_texture_reference(&value)?;
        let target_dds = format!("textures/{base}.dds");
        let target_options = format!("textures/{base}.options");
        if target.is_file(&target_dds) && target.is_file(&target_options) {
            reused_target_textures.insert(
                format!("{donor_path}#{field_path}"),
                format!("{target_dds} (inherited template binding)"),
            );
            continue;
        }
        let donor_dds = format!("textures/{base}.dds");
        let donor_options = format!("textures/{base}.options");
        if !donor.is_file(&donor_dds) || !donor.is_file(&donor_options) {
            bail!(
                "inherited template texture reference {value:?} in {donor_path}#{field_path} \
                 has no complete target or donor DDS/options pair"
            );
        }
        let target_base = format!("{}/{}", manifest.namespace, base);
        let namespaced_dds = format!("textures/{target_base}.dds");
        if let Some(existing) = textures.get(&donor_dds) {
            if existing != &namespaced_dds {
                bail!(
                    "donor texture {donor_dds} resolved to conflicting targets \
                     {existing} and {namespaced_dds}"
                );
            }
        } else {
            convert_texture_pair(
                &donor_dds,
                &target_base,
                donor,
                target_dds_formats,
                overlay,
                files,
            )?;
            textures.insert(donor_dds, namespaced_dds);
        }
        config
            .root
            .set_path(&field_path, ConfigValue::String(target_base))
            .with_context(|| {
                format!(
                    "rewriting inherited texture field {field_path} in converted material {donor_path}"
                )
            })?;
    }
    validate_active_normal_map_references(&config.root, donor_path, donor, target, overlay)?;
    write_resource(
        overlay,
        &target_path,
        &config.to_bytes()?,
        &format!(
            "adapter:target-material-template:{}{}",
            converted.template_path,
            compatibility_path
                .map(|path| format!(":compatibility-baseline:{path}"))
                .unwrap_or_default()
        ),
        files,
    )?;
    materials.insert(donor_path.to_owned(), target_path);
    Ok(())
}

fn collect_material_texture_bindings(value: &ConfigValue) -> BTreeMap<String, String> {
    fn visit(value: &ConfigValue, path: &str, output: &mut BTreeMap<String, String>) {
        match value {
            ConfigValue::Named(fields) => {
                for field in fields {
                    let field_path = if path.is_empty() {
                        field.name.clone()
                    } else {
                        format!("{path}.{}", field.name)
                    };
                    // Target templates contain optional texture-like
                    // parameters whose feature switch is disabled even when
                    // is_model_parameter is true. Some stock defaults point
                    // at intentionally absent payloads. The diffuse slot is
                    // always sampled and is the inherited closure that made
                    // grass_l01 disappear at runtime.
                    if field.name == "texture_diffuse"
                        && let Some(ConfigValue::String(texture)) = field.value.named("value")
                        && !texture.is_empty()
                        && matches!(
                            field.value.named("is_model_parameter"),
                            Some(ConfigValue::Boolean(true))
                        )
                    {
                        output.insert(format!("{field_path}.value"), texture.clone());
                    }
                    visit(&field.value, &field_path, output);
                }
            }
            ConfigValue::Indexed(values) => {
                for (index, value) in values.iter().enumerate() {
                    let field_path = if path.is_empty() {
                        index.to_string()
                    } else {
                        format!("{path}.{index}")
                    };
                    visit(value, &field_path, output);
                }
            }
            _ => {}
        }
    }

    let mut result = BTreeMap::new();
    visit(value, "", &mut result);
    result
}

fn validate_active_normal_map_references(
    value: &ConfigValue,
    material_path: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
    overlay: &Path,
) -> Result<()> {
    match value {
        ConfigValue::Named(fields) => {
            let use_nmap = fields
                .iter()
                .find(|field| field.name == "use_nmap")
                .and_then(|field| field.value.named("value"));
            if matches!(use_nmap, Some(ConfigValue::Boolean(true))) {
                for (field_name, normal) in active_normal_map_bindings(fields, material_path)? {
                    let base = canonical_texture_reference(&normal).with_context(|| {
                        format!(
                            "{material_path} has an invalid active normal-map reference in {field_name}"
                        )
                    })?;
                    let dds = format!("textures/{base}.dds");
                    let options = format!("textures/{base}.options");
                    let overlay_dds = safe_join(overlay, &dds)?;
                    let overlay_options = safe_join(overlay, &options)?;
                    if !((overlay_dds.is_file() && overlay_options.is_file())
                        || (donor.is_file(&dds) && donor.is_file(&options))
                        || (target.is_file(&dds) && target.is_file(&options)))
                    {
                        bail!(
                            "{material_path} enables use_nmap but active texture {base:?} from {field_name} has no DDS/options pair"
                        );
                    }
                }
            }
            for field in fields {
                validate_active_normal_map_references(
                    &field.value,
                    material_path,
                    donor,
                    target,
                    overlay,
                )?;
            }
        }
        ConfigValue::Indexed(values) => {
            for value in values {
                validate_active_normal_map_references(
                    value,
                    material_path,
                    donor,
                    target,
                    overlay,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn active_normal_map_bindings(
    fields: &[NamedValue],
    material_path: &str,
) -> Result<Vec<(String, String)>> {
    let terrain_layer_count = fields
        .iter()
        .find(|field| field.name == "num_used_terrain_layers")
        .and_then(|field| field.value.named("value"))
        .and_then(|value| match value {
            ConfigValue::Integer(value) => usize::try_from(*value).ok(),
            _ => None,
        });
    let has_single_normal = fields.iter().any(|field| field.name == "texture_normal");
    let active_fields: Vec<_> = fields
        .iter()
        .filter(|field| {
            if field.name == "texture_normal" {
                return true;
            }
            let Some(layer) = field
                .name
                .strip_prefix("texture_normal_")
                .and_then(|layer| layer.parse::<usize>().ok())
            else {
                return false;
            };
            if has_single_normal || terrain_layer_count.is_some_and(|count| layer >= count) {
                return false;
            }
            terrain_layer_count.is_some()
                || matches!(
                    field.value.named("value"),
                    Some(ConfigValue::String(value)) if !value.is_empty()
                )
        })
        .collect();
    if active_fields.is_empty() {
        bail!(
            "{material_path} enables use_nmap without an active texture_normal.value or texture_normal_N.value"
        );
    }
    active_fields
        .into_iter()
        .map(|field| {
            let normal = field.value.named("value").with_context(|| {
                format!("{material_path} active {}.value is absent", field.name)
            })?;
            let ConfigValue::String(normal) = normal else {
                bail!(
                    "{material_path} active {}.value is not a string",
                    field.name
                );
            };
            if normal.is_empty() {
                bail!(
                    "{material_path} active {}.value is an empty string",
                    field.name
                );
            }
            Ok((field.name.clone(), normal.clone()))
        })
        .collect()
}

fn convert_texture_pair(
    donor_dds_path: &str,
    target_base: &str,
    donor: &mut ResourceDb,
    target_formats: &BTreeSet<[u8; 4]>,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = donor.read(donor_dds_path)?;
    let info = parse_dds(&bytes, donor_dds_path)?;
    if !target_formats.contains(&info.fourcc) {
        let format = if info.fourcc == [0; 4] {
            "UNCOMPRESSED".into()
        } else {
            format!("FourCC {:?}", String::from_utf8_lossy(&info.fourcc))
        };
        bail!("{donor_dds_path} uses {format}, not observed in stock v0.100b");
    }
    let donor_options_path = format!("{}.options", donor_dds_path.trim_end_matches(".dds"));
    let options_bytes = donor
        .read(&donor_options_path)
        .with_context(|| format!("missing texture options {donor_options_path}"))?;
    let options = validate_texture_options(&options_bytes, &donor_options_path, info)?;
    write_resource(
        overlay,
        &format!("textures/{target_base}.dds"),
        &bytes,
        "adapter:validated-legacy-dds-v020f-to-v0100b",
        files,
    )?;
    write_resource(
        overlay,
        &format!("textures/{target_base}.options"),
        &options.to_bytes()?,
        "adapter:texture-options-v020f-to-v0100b",
        files,
    )?;
    Ok(())
}

fn load_target_dds_formats(target: &mut ResourceDb) -> Result<BTreeSet<[u8; 4]>> {
    let paths: Vec<String> = target
        .paths_with_prefix("textures/")
        .filter(|path| path.ends_with(".dds") && target.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    let mut result = BTreeSet::new();
    for path in paths {
        if let Ok(info) = parse_dds(&target.read(&path)?, &path) {
            result.insert(info.fourcc);
        }
    }
    if result.is_empty() {
        bail!("stock v0.100b contains no supported legacy DDS formats");
    }
    Ok(result)
}

fn parse_dds(bytes: &[u8], path: &str) -> Result<DdsInfo> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || read_u32(bytes, 4)? != 124 {
        bail!("{path} is not a legacy DDS file");
    }
    let height = read_u32(bytes, 12)?;
    let width = read_u32(bytes, 16)?;
    let depth = read_u32(bytes, 24)?;
    let raw_mip_count = read_u32(bytes, 28)?;
    if width == 0 || height == 0 || read_u32(bytes, 76)? != 32 {
        bail!("{path} has invalid DDS dimensions/pixel format");
    }
    let max_mip_count = u32::BITS - width.max(height).leading_zeros();
    let mip_count = raw_mip_count.max(1);
    if mip_count > max_mip_count {
        bail!(
            "{path} has impossible 2D mip count {raw_mip_count} for {width}x{height} \
             (normalized={mip_count}, maximum={max_mip_count})"
        );
    }
    let pixel_flags = read_u32(bytes, 80)?;
    let fourcc: [u8; 4] = if pixel_flags & 0x4 != 0 {
        let fourcc = bytes[84..88].try_into().unwrap();
        if &fourcc == b"DX10" || !matches!(&fourcc, b"DXT1" | b"DXT3" | b"DXT5") {
            bail!(
                "{path} uses unsupported DDS FourCC {:?}",
                String::from_utf8_lossy(&fourcc)
            );
        }
        fourcc
    } else {
        if pixel_flags != 0x41
            || read_u32(bytes, 88)? != 32
            || read_u32(bytes, 92)? != 0x00ff_0000
            || read_u32(bytes, 96)? != 0x0000_ff00
            || read_u32(bytes, 100)? != 0x0000_00ff
            || read_u32(bytes, 104)? != 0xff00_0000
        {
            bail!("{path} is not stock-compatible uncompressed BGRA8 DDS");
        }
        [0; 4]
    };
    let caps2 = read_u32(bytes, 112)?;
    if depth != 0 || !matches!(caps2, 0 | 0xfe00) {
        bail!("{path} is not a supported 2D/cubemap DDS (depth={depth}, caps2={caps2:#x})");
    }
    let (faces, dimension) = if caps2 == 0xfe00 {
        (6_usize, 3)
    } else {
        (1_usize, 1)
    };
    let block_size = match &fourcc {
        b"DXT1" => Some(8_usize),
        b"DXT3" | b"DXT5" => Some(16_usize),
        _ => None,
    };
    let mut payload_size = 0_usize;
    let mut w = usize::try_from(width)?;
    let mut h = usize::try_from(height)?;
    for _ in 0..mip_count {
        let mip_size = if let Some(block_size) = block_size {
            w.div_ceil(4)
                .max(1)
                .checked_mul(h.div_ceil(4).max(1))
                .and_then(|blocks| blocks.checked_mul(block_size))
        } else {
            w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4))
        }
        .context("DDS mip payload size overflow")?;
        payload_size = payload_size
            .checked_add(mip_size)
            .context("DDS payload size overflow")?;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    payload_size = payload_size
        .checked_mul(faces)
        .context("DDS cubemap payload size overflow")?;
    let vostok_payload_size = payload_size
        .checked_add(1)
        .context("Vostok DDS payload size overflow")?;
    let header_linear_size = usize::try_from(read_u32(bytes, 20)?)?;
    let expected_linear_size = if block_size.is_some() {
        vostok_payload_size
    } else {
        0
    };
    if header_linear_size != expected_linear_size {
        bail!(
            "{path} DDS linear size is not target-native: {header_linear_size} != {expected_linear_size}"
        );
    }
    let required = 128_usize
        .checked_add(vostok_payload_size)
        .context("Vostok DDS file size overflow")?;
    if bytes.len() != required {
        bail!(
            "{path} Vostok DDS payload length is not exact: {} != {required}",
            bytes.len()
        );
    }
    let srgb = bytes[required - 1];
    if srgb > 1 {
        bail!("{path} has invalid trailing Vostok sRGB byte {srgb}");
    }
    Ok(DdsInfo {
        width,
        height,
        mip_count,
        fourcc,
        srgb,
        dimension,
    })
}

fn validate_texture_options(bytes: &[u8], path: &str, dds: DdsInfo) -> Result<BinaryConfig> {
    let config =
        BinaryConfig::parse(bytes).with_context(|| format!("parsing texture options {path}"))?;
    let options = config
        .root
        .named("options")
        .with_context(|| format!("{path} has no options table"))?;
    let allowed: BTreeSet<&str> = [
        "ver",
        "has_mip_levels",
        "srgb",
        "texture_dimension",
        "bump_mode",
        "high_quality",
        "format",
    ]
    .into_iter()
    .collect();
    validate_named_field_set(options, path, &allowed)?;
    if !matches!(options.named("ver"), Some(ConfigValue::String(value)) if value == "1.2") {
        bail!("{path} is not texture options schema 1.2");
    }
    let expected_format = if dds.fourcc == [0; 4] {
        "UNCOMPRESSED"
    } else {
        std::str::from_utf8(&dds.fourcc)?
    };
    let ConfigValue::String(options_format) = options
        .named("format")
        .with_context(|| format!("{path} has no format"))?
    else {
        bail!("{path}.format is not string");
    };
    let format_matches = options_format == expected_format
        || (options_format == "NORMAL_MAP" && &dds.fourcc == b"DXT5" && dds.srgb == 0);
    if !format_matches {
        bail!(
            "{path} format {options_format:?} does not agree with its DDS FourCC {expected_format}"
        );
    }
    let has_mips = integer(options, "has_mip_levels", path)?;
    if (dds.mip_count > 1) != (has_mips != 0) {
        bail!("{path} mip flag does not agree with its DDS");
    }
    for field in ["srgb", "texture_dimension"] {
        integer(options, field, path)?;
    }
    for field in ["bump_mode", "high_quality"] {
        if options.named(field).is_some() {
            integer(options, field, path)?;
        }
    }
    if integer(options, "srgb", path)? != i32::from(dds.srgb) {
        bail!(
            "{path} sRGB option does not agree with its trailing DDS byte {}",
            dds.srgb
        );
    }
    if integer(options, "texture_dimension", path)? != dds.dimension {
        bail!(
            "{path} texture dimension does not agree with its DDS: {} != {}",
            integer(options, "texture_dimension", path)?,
            dds.dimension
        );
    }
    Ok(config)
}

fn validate_named_field_set(
    value: &ConfigValue,
    path: &str,
    allowed: &BTreeSet<&str>,
) -> Result<()> {
    let ConfigValue::Named(fields) = value else {
        bail!("{path} is not a named table");
    };
    let actual: BTreeSet<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    if actual.len() != fields.len() {
        bail!("{path} contains duplicate named fields");
    }
    let unknown: Vec<_> = actual.difference(allowed).copied().collect();
    if !unknown.is_empty() {
        bail!("{path} has unsupported fields {unknown:?}");
    }
    Ok(())
}

fn integer(value: &ConfigValue, field: &str, path: &str) -> Result<i32> {
    let ConfigValue::Integer(result) = value
        .named(field)
        .with_context(|| format!("{path} has no {field}"))?
    else {
        bail!("{path}.{field} is not integer");
    };
    Ok(*result)
}

fn float3(value: &ConfigValue, field: &str, path: &str) -> Result<[f32; 3]> {
    let ConfigValue::Float3(result) = value
        .named(field)
        .with_context(|| format!("{path} has no {field}"))?
    else {
        bail!("{path}.{field} is not float3");
    };
    Ok(*result)
}

fn validate_bounds(minimum: [f32; 3], maximum: [f32; 3], path: &str) -> Result<()> {
    if minimum
        .into_iter()
        .chain(maximum)
        .any(|value| !value.is_finite())
        || (0..3).any(|axis| minimum[axis] > maximum[axis])
    {
        bail!("{path} has invalid bounds {minimum:?}..{maximum:?}");
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw = bytes
        .get(offset..offset + 4)
        .with_context(|| format!("binary payload is truncated at {offset}"))?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

fn write_resource(
    overlay: &Path,
    resource: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let path = safe_join(overlay, resource)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    files.push(record_file(resource, source, bytes));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_config::NamedValue;

    fn named(entries: impl IntoIterator<Item = (&'static str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            entries
                .into_iter()
                .map(|(name, value)| NamedValue {
                    name: name.into(),
                    value,
                })
                .collect(),
        )
    }

    fn collision_instance(library: &str) -> ConfigValue {
        named([
            ("lib_name", ConfigValue::String(library.into())),
            ("cgroup", ConfigValue::Integer(1)),
            ("cmask", ConfigValue::Integer(2)),
            ("scale", ConfigValue::Float3([1.0; 3])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
            ("position", ConfigValue::Float3([2.0, 3.0, 4.0])),
        ])
    }

    #[test]
    fn typed_project_rewrite_tracks_walker_collision_family() {
        let source = ConfigValue::Indexed(vec![collision_instance(
            "resources/models/props/test.model/walker_collision",
        )]);
        let mut models = BTreeMap::new();
        collect_model_requests(&source, InstanceKind::Collision, &mut models).unwrap();
        let request = models.get_mut("models/props/test.model").unwrap();
        request.target = "models/ported/v020f/level03/props/test.model".into();
        request.collision_targets.insert(
            "walker_collision".into(),
            "models/ported/v020f/level03/props/test.model".into(),
        );
        assert_eq!(
            request.target,
            "models/ported/v020f/level03/props/test.model"
        );
        assert_eq!(
            request.collision_families,
            BTreeSet::from(["walker_collision".into()])
        );
        let converted =
            rebuild_project_instances(&source, InstanceKind::Collision, &models).unwrap();
        let ConfigValue::Indexed(entries) = converted else {
            panic!("expected indexed instances");
        };
        assert_eq!(
            entries[0].named("lib_name"),
            Some(&ConfigValue::String(
                "resources/models/ported/v020f/level03/props/test.model/walker_collision".into()
            ))
        );
        assert_eq!(
            entries[0].named("position"),
            Some(&ConfigValue::Float3([2.0, 3.0, 4.0]))
        );
    }

    #[test]
    fn project_rewrite_omits_an_explicitly_non_rendered_model() {
        let source = ConfigValue::Indexed(vec![named([
            (
                "lib_name",
                ConfigValue::String("level/03/level_border".into()),
            ),
            ("scale", ConfigValue::Float3([1.0; 3])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
            ("position", ConfigValue::Float3([0.0; 3])),
        ])]);
        let models = BTreeMap::from([(
            "models/level/03/level_border.model".into(),
            ModelRequest {
                render_referenced: false,
                target: "models/ported/v020f/level03/level/03/level_border.model".into(),
                collision_families: BTreeSet::from(["collision".into()]),
                collision_targets: BTreeMap::new(),
            },
        )]);
        assert_eq!(
            rebuild_project_instances(&source, InstanceKind::Render, &models).unwrap(),
            ConfigValue::Indexed(Vec::new())
        );
    }

    #[test]
    fn project_rewrite_can_omit_an_unsupported_pure_visual_model() {
        let source = ConfigValue::Indexed(vec![named([
            ("lib_name", ConfigValue::String("fx/anomaly_smoke".into())),
            ("scale", ConfigValue::Float3([1.0; 3])),
            ("rotation", ConfigValue::Float3([0.0; 3])),
            ("position", ConfigValue::Float3([0.0; 3])),
        ])]);
        let models = BTreeMap::from([(
            "models/fx/anomaly_smoke.model".into(),
            ModelRequest {
                render_referenced: false,
                target: "models/ported/v020f/level04/fx/anomaly_smoke.model".into(),
                collision_families: BTreeSet::new(),
                collision_targets: BTreeMap::new(),
            },
        )]);

        assert_eq!(
            rebuild_project_instances(&source, InstanceKind::Render, &models).unwrap(),
            ConfigValue::Indexed(Vec::new())
        );
    }

    fn primitive_config(primitive_type: i32) -> ConfigValue {
        let scale = match primitive_type {
            0 => [1.0, 0.0, 0.0],
            1 => [1.0; 3],
            2 | 3 => [1.0, 1.0, 0.0],
            _ => [1.0; 3],
        };
        named([
            (
                "primitives",
                ConfigValue::Indexed(vec![named([
                    ("rotation", ConfigValue::Float3([0.0; 3])),
                    ("position", ConfigValue::Float3([0.0; 3])),
                    ("type", ConfigValue::Integer(primitive_type)),
                    ("mtl", ConfigValue::Integer(0)),
                    ("scale", ConfigValue::Float3(scale)),
                ])]),
            ),
            (
                "mtl_list",
                ConfigValue::Indexed(vec![ConfigValue::String("default".into())]),
            ),
        ])
    }

    #[test]
    fn primitive_whitelist_accepts_zero_through_three_only() {
        for primitive_type in 0..=3 {
            assert_eq!(
                validate_exported_primitives(
                    &primitive_config(primitive_type),
                    "exported_primitives"
                )
                .unwrap()
                .0,
                1
            );
        }
        assert!(validate_exported_primitives(&primitive_config(4), "exported_primitives").is_err());
        assert!(
            validate_exported_primitives(&primitive_config(99), "exported_primitives").is_err()
        );
    }

    #[test]
    fn collision_face_data_requires_exact_dictionary_and_triangle_indices() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1b_u32.to_le_bytes());
        bytes.extend_from_slice(&6_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(b"a\0b\0");
        bytes.extend_from_slice(&0x1c_u32.to_le_bytes());
        bytes.extend_from_slice(&4_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        validate_face_data("face_data", &bytes, 2).unwrap();

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(validate_face_data("face_data", &trailing, 2).is_err());
        let end = bytes.len();
        bytes[end - 2..].copy_from_slice(&2_u16.to_le_bytes());
        assert!(validate_face_data("face_data", &bytes, 2).is_err());
    }

    #[test]
    fn scoped_binding_can_replace_an_unknown_newer_material_name() {
        let mut settings = named([(
            "game_material_settings",
            named([(
                "coll_metall_thinSG",
                named([
                    ("game_material_id", ConfigValue::Integer(46)),
                    (
                        "game_material_name",
                        ConfigValue::String("metal_car".into()),
                    ),
                ]),
            )]),
        )]);
        let target_materials = BTreeMap::from([("metall_thin".into(), 17)]);
        let binding_overrides =
            BTreeMap::from([("coll_metall_thinSG".into(), "metall_thin".into())]);
        let resolved = remap_game_material_settings(
            &mut settings,
            "kamaz/settings",
            &target_materials,
            &BTreeMap::new(),
            Some(&binding_overrides),
        )
        .unwrap();
        assert_eq!(
            resolved.get("coll_metall_thinSG"),
            Some(&("metall_thin".into(), 17))
        );
    }

    #[test]
    fn stock_render_only_settings_are_valid_without_collision_material_evidence() {
        let settings = named([(
            "material_settings",
            named([(
                "pallet_s",
                named([(
                    "material_name",
                    ConfigValue::String("props/garage/pallet".into()),
                )]),
            )]),
        )]);

        assert!(
            read_optional_game_material_evidence(&settings, "pallet/settings")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn render_only_models_receive_an_empty_runtime_collision_material_table() {
        let settings = named([(
            "material_settings",
            named([(
                "background",
                named([("material_name", ConfigValue::String("backgr/trees".into()))]),
            )]),
        )]);

        assert_eq!(
            runtime_game_material_settings(&settings, "background/settings", true).unwrap(),
            named([])
        );
        assert!(runtime_game_material_settings(&settings, "background/settings", false).is_err());
    }

    #[test]
    fn preserved_empty_render_material_remains_the_runtime_sentinel() {
        assert_eq!(
            target_material_reference("ported/v020f/level03", ""),
            String::new()
        );
        assert_eq!(
            target_material_reference("ported/v020f/level03", "tiled/metal/metal_02"),
            "ported/v020f/level03/tiled/metal/metal_02"
        );
    }

    #[test]
    fn collects_inherited_template_texture_fields_for_closure_validation() {
        let material = named([(
            "material",
            named([(
                "g_stage",
                named([(
                    "effect",
                    named([
                        (
                            "texture_diffuse",
                            named([
                                ("value", ConfigValue::String("flora/grass_l01".into())),
                                ("is_model_parameter", ConfigValue::Boolean(true)),
                            ]),
                        ),
                        (
                            "constant_color",
                            named([("value", ConfigValue::Float4([1.0; 4]))]),
                        ),
                    ]),
                )]),
            )]),
        )]);

        assert_eq!(
            collect_material_texture_bindings(&material),
            BTreeMap::from([(
                "material.g_stage.effect.texture_diffuse.value".into(),
                "flora/grass_l01".into()
            )])
        );
    }

    #[test]
    fn active_normal_map_bindings_follow_the_terrain_layer_count() {
        let effect = named([
            ("use_nmap", named([("value", ConfigValue::Boolean(true))])),
            (
                "num_used_terrain_layers",
                named([("value", ConfigValue::Integer(2))]),
            ),
            (
                "texture_normal_0",
                named([("value", ConfigValue::String("terrain/asphalt_nmap".into()))]),
            ),
            (
                "texture_normal_1",
                named([("value", ConfigValue::String("terrain/grass_nmap".into()))]),
            ),
            (
                "texture_normal_2",
                named([("value", ConfigValue::String(String::new()))]),
            ),
        ]);
        let ConfigValue::Named(fields) = effect else {
            unreachable!();
        };

        assert_eq!(
            active_normal_map_bindings(&fields, "terrain.material").unwrap(),
            vec![
                ("texture_normal_0".into(), "terrain/asphalt_nmap".into()),
                ("texture_normal_1".into(), "terrain/grass_nmap".into()),
            ]
        );
    }

    #[test]
    fn reconstructs_the_runtime_lod_descriptor_inputs() {
        let settings = named([
            (
                "lod_hierrarchy",
                named([
                    ("unassigned", named([])),
                    (
                        "LOD0",
                        named([(
                            "surfaces",
                            ConfigValue::Indexed(vec![ConfigValue::String("body".into())]),
                        )]),
                    ),
                    (
                        "LOD1",
                        named([(
                            "surfaces",
                            ConfigValue::Indexed(vec![ConfigValue::String("lod1_body".into())]),
                        )]),
                    ),
                    (
                        "LOD2",
                        named([(
                            "surfaces",
                            ConfigValue::Indexed(vec![ConfigValue::String("lod1_body".into())]),
                        )]),
                    ),
                ]),
            ),
            (
                "lod_switching",
                named([
                    ("param1", ConfigValue::Integer(100)),
                    ("param0", ConfigValue::Integer(50)),
                    ("default_params", ConfigValue::Boolean(false)),
                    ("type", ConfigValue::Integer(0)),
                    ("param2", ConfigValue::Integer(240)),
                ]),
            ),
        ]);
        let surfaces = BTreeSet::from(["body".into(), "lod1_body".into(), "unused".into()]);

        let hierarchy = synthesize_runtime_lod_hierarchy(
            &settings,
            "model/settings",
            &surfaces,
            &BTreeSet::new(),
        )
        .unwrap();
        let switching = synthesize_runtime_lod_switching(&settings, "model/settings").unwrap();

        assert_eq!(
            hierarchy
                .named("LOD0")
                .and_then(|lod| lod.named("surfaces")),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "body".into()
            )]))
        );
        assert_eq!(
            hierarchy
                .named("LOD2")
                .and_then(|lod| lod.named("surfaces")),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "lod1_body".into()
            )]))
        );
        assert_eq!(switching.named("param2"), Some(&ConfigValue::Integer(240)));
    }

    #[test]
    fn missing_lod_hierarchy_is_rejected_before_packing() {
        let error = synthesize_runtime_lod_hierarchy(
            &named([]),
            "model/settings",
            &BTreeSet::from(["body".into()]),
            &BTreeSet::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("runtime-required lod_hierrarchy"));
    }

    #[test]
    fn explicitly_dropped_empty_surface_is_removed_from_lod_hierarchy() {
        let settings = named([(
            "lod_hierrarchy",
            named([
                ("unassigned", named([])),
                (
                    "LOD0",
                    named([(
                        "surfaces",
                        ConfigValue::Indexed(vec![ConfigValue::String("body".into())]),
                    )]),
                ),
                (
                    "LOD1",
                    named([(
                        "surfaces",
                        ConfigValue::Indexed(vec![
                            ConfigValue::String("lod1_body".into()),
                            ConfigValue::String("lod1_empty_glass".into()),
                        ]),
                    )]),
                ),
                ("LOD2", named([])),
            ]),
        )]);
        let hierarchy = synthesize_runtime_lod_hierarchy(
            &settings,
            "model/settings",
            &BTreeSet::from(["body".into(), "lod1_body".into()]),
            &BTreeSet::from(["lod1_empty_glass".into()]),
        )
        .unwrap();

        assert_eq!(
            hierarchy
                .named("LOD1")
                .and_then(|lod| lod.named("surfaces")),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "lod1_body".into()
            )]))
        );
    }

    #[test]
    fn missing_lod_switching_uses_the_retail_default() {
        assert_eq!(
            synthesize_runtime_lod_switching(&named([]), "model/settings").unwrap(),
            named([
                ("type", ConfigValue::Integer(0)),
                ("default_params", ConfigValue::Boolean(true)),
            ])
        );
    }

    #[test]
    fn explicit_collision_material_sentinel_is_valid_for_regular_families() {
        let names = BTreeSet::from(["default".into()]);
        let settings = BTreeMap::from([("default".into(), (String::new(), 65_535))]);
        assert_eq!(
            validate_collision_material_names(&names, &settings, "collision", false).unwrap(),
            CollisionMaterialResolution {
                default_sentinel: names,
                ..Default::default()
            }
        );
    }

    #[test]
    fn only_walker_families_allow_a_missing_material_key_to_fall_back_to_zero() {
        let names = BTreeSet::from(["missing".into()]);
        let settings = BTreeMap::new();
        assert!(validate_collision_material_names(&names, &settings, "collision", false).is_err());
        assert_eq!(
            validate_collision_material_names(&names, &settings, "walker_collision", true).unwrap(),
            CollisionMaterialResolution {
                fallback_zero: names,
                ..Default::default()
            }
        );
    }

    #[test]
    fn stock_normal_map_option_is_valid_for_linear_dxt5() {
        let options = BinaryConfig {
            root: named([(
                "options",
                named([
                    ("ver", ConfigValue::String("1.2".into())),
                    ("has_mip_levels", ConfigValue::Integer(1)),
                    ("srgb", ConfigValue::Integer(0)),
                    ("texture_dimension", ConfigValue::Integer(1)),
                    ("bump_mode", ConfigValue::Integer(0)),
                    ("high_quality", ConfigValue::Integer(0)),
                    ("format", ConfigValue::String("NORMAL_MAP".into())),
                ]),
            )]),
        };
        validate_texture_options(
            &options.to_bytes().unwrap(),
            "rag_nmap.options",
            DdsInfo {
                width: 4,
                height: 4,
                mip_count: 3,
                fourcc: *b"DXT5",
                srgb: 0,
                dimension: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn stock_texture_options_may_omit_optional_quality_and_bump_fields() {
        let options = BinaryConfig {
            root: named([(
                "options",
                named([
                    ("ver", ConfigValue::String("1.2".into())),
                    ("has_mip_levels", ConfigValue::Integer(1)),
                    ("srgb", ConfigValue::Integer(1)),
                    ("texture_dimension", ConfigValue::Integer(1)),
                    ("format", ConfigValue::String("DXT1".into())),
                ]),
            )]),
        };
        validate_texture_options(
            &options.to_bytes().unwrap(),
            "default_spec.options",
            DdsInfo {
                width: 4,
                height: 4,
                mip_count: 3,
                fourcc: *b"DXT1",
                srgb: 1,
                dimension: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn stock_uncompressed_bgra8_dds_uses_full_mip_payload_and_zero_pitch() {
        let mut bytes = vec![0_u8; 128 + 4 * 4 * 4 + 1];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124_u32),
            (12, 4),
            (16, 4),
            (28, 1),
            (76, 32),
            (80, 0x41),
            (88, 32),
            (92, 0x00ff_0000),
            (96, 0x0000_ff00),
            (100, 0x0000_00ff),
            (104, 0xff00_0000),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let info = parse_dds(&bytes, "metal_egoza.dds").unwrap();
        assert_eq!(info.fourcc, [0; 4]);
        assert_eq!(info.srgb, 0);
        assert_eq!(info.dimension, 1);
    }

    #[test]
    fn stock_cubemap_dds_accounts_for_all_six_faces() {
        let mut bytes = vec![0_u8; 128 + 6 * 16 + 1];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124_u32),
            (12, 4),
            (16, 4),
            (20, 97),
            (28, 1),
            (76, 32),
            (80, 0x4),
            (84, u32::from_le_bytes(*b"DXT5")),
            (112, 0xfe00),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let info = parse_dds(&bytes, "probe.dds").unwrap();
        assert_eq!(info.fourcc, *b"DXT5");
        assert_eq!(info.dimension, 3);
    }
}
