use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{BinaryConfig, ConfigValue, NamedValue, ResourceDb};

#[derive(Clone, Copy, Debug, Default)]
pub struct ModelValidationReport {
    pub models: usize,
    pub overlay_configs: usize,
}

/// Validate the model directories visible after an additive overlay, before
/// writing an archive.
pub fn validate_effective_overlay(base_db: &Path, overlay: &Path) -> Result<ModelValidationReport> {
    let mut base = ResourceDb::open(base_db)?;
    let overlay_files = collect_overlay_files(overlay)?;
    validate_namespace_compatibility(&base, overlay_files.keys())?;

    let mut paths: BTreeSet<String> = base
        .entries()
        .keys()
        .filter(|path| base.is_file(path))
        .cloned()
        .collect();
    paths.extend(overlay_files.keys().cloned());
    validate_resource_pairs(&paths)?;
    let mut report = validate_paths(&mut base, &overlay_files, &paths)?;
    report.overlay_configs = validate_overlay_configs(&overlay_files)?;
    validate_overlay_collision_chunks(&overlay_files)?;
    Ok(report)
}

pub fn validate_database(db_path: &Path) -> Result<ModelValidationReport> {
    let mut db = ResourceDb::open(db_path)?;
    let paths: BTreeSet<String> = db
        .entries()
        .keys()
        .filter(|path| db.is_file(path))
        .cloned()
        .collect();
    validate_resource_pairs(&paths)?;
    validate_paths(&mut db, &BTreeMap::new(), &paths)
}

fn validate_resource_pairs(paths: &BTreeSet<String>) -> Result<()> {
    for path in paths {
        let counterpart = if path.starts_with("textures/") && path.ends_with(".dds") {
            Some(format!("{}.options", path.trim_end_matches(".dds")))
        } else if path.starts_with("textures/") && path.ends_with(".options") {
            Some(format!("{}.dds", path.trim_end_matches(".options")))
        } else if path.starts_with("sounds/single/") && path.ends_with(".high.ogg") {
            Some(format!(
                "{}.single_sound_options",
                path.trim_end_matches(".high.ogg")
            ))
        } else if path.starts_with("sounds/single/") && path.ends_with(".single_sound_options") {
            Some(format!(
                "{}.high.ogg",
                path.trim_end_matches(".single_sound_options")
            ))
        } else if path.starts_with("animations/single/") && path.ends_with(".b-spline") {
            Some(format!(
                "{}.bones_names",
                path.trim_end_matches(".b-spline")
            ))
        } else if path.starts_with("animations/single/") && path.ends_with(".bones_names") {
            Some(format!(
                "{}.b-spline",
                path.trim_end_matches(".bones_names")
            ))
        } else {
            None
        };
        if let Some(counterpart) = counterpart
            && !paths.contains(&counterpart)
        {
            bail!("paired resource {path} is missing required companion {counterpart}");
        }
    }
    Ok(())
}

/// Rewrite settings in a generated overlay when a donor model's settings name
/// a different surface set than its emitted render geometry.
pub fn normalize_overlay_model_settings(root: &Path) -> Result<Vec<String>> {
    let overlay_files = collect_overlay_files(root)?;
    let mut surfaces = BTreeMap::<String, BTreeSet<String>>::new();
    for path in overlay_files.keys() {
        let Some((model, surface)) = rendered_surface(path) else {
            continue;
        };
        surfaces.entry(model).or_default().insert(surface);
    }

    let mut normalized = Vec::new();
    for (model, actual) in surfaces {
        let settings_path = format!("{model}/settings");
        let Some(path) = overlay_files.get(&settings_path) else {
            continue;
        };
        let mut settings = BinaryConfig::parse(&fs::read(path)?)
            .with_context(|| format!("parsing generated model settings {settings_path}"))?;
        let expected = settings_surfaces(&settings, &settings_path)?;
        if expected == actual {
            continue;
        }
        normalize_material_settings(&mut settings, &settings_path, &actual)?;
        normalize_lod_hierarchy(&mut settings, &actual)?;
        fs::write(path, settings.to_bytes()?)
            .with_context(|| format!("writing normalized model settings {}", path.display()))?;
        normalized.push(settings_path);
    }
    Ok(normalized)
}

fn normalize_material_settings(
    settings: &mut BinaryConfig,
    settings_path: &str,
    actual: &BTreeSet<String>,
) -> Result<()> {
    let ConfigValue::Named(values) = settings
        .root
        .named_mut("material_settings")
        .with_context(|| format!("{settings_path} has no material_settings table"))?
    else {
        bail!("{settings_path} material_settings is not a named table");
    };
    let template = values
        .first()
        .map(|entry| entry.value.clone())
        .with_context(|| format!("{settings_path} has an empty material_settings table"))?;
    values.retain(|entry| actual.contains(&entry.name));
    let retained: BTreeSet<String> = values.iter().map(|entry| entry.name.clone()).collect();
    for surface in actual.difference(&retained) {
        values.push(NamedValue {
            name: surface.clone(),
            value: template.clone(),
        });
    }
    Ok(())
}

fn normalize_lod_hierarchy(settings: &mut BinaryConfig, actual: &BTreeSet<String>) -> Result<()> {
    let Some(ConfigValue::Named(levels)) = settings.root.named_mut("lod_hierrarchy") else {
        return Ok(());
    };
    for (level_name, level_prefix) in [("LOD0", ""), ("LOD1", "lod1_"), ("LOD2", "lod2_")] {
        let Some(level) = levels.iter_mut().find(|entry| entry.name == level_name) else {
            continue;
        };
        let ConfigValue::Named(fields) = &mut level.value else {
            continue;
        };
        let level_surfaces = ConfigValue::Indexed(
            actual
                .iter()
                .filter(|surface| match level_name {
                    "LOD1" => surface.starts_with(level_prefix),
                    "LOD2" => surface.starts_with(level_prefix),
                    _ => !surface.starts_with("lod1_") && !surface.starts_with("lod2_"),
                })
                .cloned()
                .map(ConfigValue::String)
                .collect(),
        );
        if let Some(surfaces) = fields.iter_mut().find(|entry| entry.name == "surfaces") {
            surfaces.value = level_surfaces;
        } else {
            fields.push(NamedValue {
                name: "surfaces".into(),
                value: level_surfaces,
            });
        }
    }
    Ok(())
}

fn validate_paths(
    base: &mut ResourceDb,
    overlay_files: &BTreeMap<String, PathBuf>,
    paths: &BTreeSet<String>,
) -> Result<ModelValidationReport> {
    let mut surfaces = BTreeMap::<String, BTreeSet<String>>::new();
    let mut material_vertex_color_gates = BTreeMap::<String, Option<bool>>::new();
    for path in paths {
        let Some((model, surface)) = rendered_surface(path) else {
            continue;
        };
        surfaces.entry(model).or_default().insert(surface);
    }

    for (model, actual) in &surfaces {
        let settings_path = format!("{model}/settings");
        let render_properties = format!("{model}/render/export_properties");
        if !paths.contains(&render_properties) {
            bail!("model {model} has rendered surfaces but no {render_properties}");
        }
        if model.ends_with(".skinned_model") {
            for required in [
                format!("{model}/render/bind_pose"),
                format!("{model}/skeleton"),
            ] {
                if !paths.contains(&required) {
                    bail!("skinned model {model} is missing required resource {required}");
                }
            }
        }
        for surface in actual {
            let properties = format!("{model}/render/{surface}/export_properties");
            if !paths.contains(&properties) {
                bail!("model {model} surface {surface} has no {properties}");
            }
        }
        let bytes = if let Some(path) = overlay_files.get(&settings_path) {
            fs::read(path).with_context(|| format!("reading {}", path.display()))?
        } else {
            base.read(&settings_path).with_context(|| {
                format!("model {model} has rendered surfaces but no effective settings")
            })?
        };
        let settings = BinaryConfig::parse(&bytes)
            .with_context(|| format!("parsing effective model settings {settings_path}"))?;
        let expected = settings_surfaces(&settings, &settings_path)?;
        if *actual != expected {
            let unexpected: Vec<_> = actual.difference(&expected).cloned().collect();
            let missing: Vec<_> = expected.difference(actual).cloned().collect();
            bail!(
                "incompatible model settings for {model}: render surfaces={:?}, \
                 settings surfaces={:?}, unexpected render surfaces={unexpected:?}, \
                missing render surfaces={missing:?}",
                actual,
                expected
            );
        }
        validate_runtime_lod_contract(&settings, &settings_path, actual)?;
        validate_surface_material_vertex_contracts(
            base,
            overlay_files,
            paths,
            model,
            &settings,
            actual,
            &mut material_vertex_color_gates,
        )?;
    }

    Ok(ModelValidationReport {
        models: surfaces.len(),
        overlay_configs: 0,
    })
}

fn validate_surface_material_vertex_contracts(
    base: &mut ResourceDb,
    overlay_files: &BTreeMap<String, PathBuf>,
    paths: &BTreeSet<String>,
    model: &str,
    settings: &BinaryConfig,
    surfaces: &BTreeSet<String>,
    material_vertex_color_gates: &mut BTreeMap<String, Option<bool>>,
) -> Result<()> {
    if !model.ends_with(".model") {
        return Ok(());
    }
    let ConfigValue::Named(material_settings) = settings
        .root
        .named("material_settings")
        .with_context(|| format!("{model}/settings has no material_settings table"))?
    else {
        bail!("{model}/settings material_settings is not a named table");
    };
    for surface in surfaces {
        let setting = material_settings
            .iter()
            .find(|entry| entry.name == *surface)
            .with_context(|| format!("{model} surface {surface} has no material setting"))?;
        let Some(ConfigValue::String(material_name)) = setting.value.named("material_name") else {
            bail!("{model} surface {surface} material_name is absent or not a string");
        };
        if material_name.is_empty() {
            continue;
        }
        let properties_path = format!("{model}/render/{surface}/export_properties");
        let properties = BinaryConfig::parse(&read_effective_resource(
            base,
            overlay_files,
            &properties_path,
        )?)
        .with_context(|| format!("parsing effective surface properties {properties_path}"))?;
        let Some(ConfigValue::Integer(mesh_type)) = properties.root.named("type") else {
            bail!("{properties_path}.type is absent or not an integer");
        };
        if *mesh_type != 2 {
            continue;
        }

        let material_path = format!("material_instances/{material_name}.material");
        if !paths.contains(&material_path) {
            bail!("{model} surface {surface} references missing material {material_path}");
        }
        let gate = if let Some(gate) = material_vertex_color_gates.get(&material_path) {
            *gate
        } else {
            let material = BinaryConfig::parse(&read_effective_resource(
                base,
                overlay_files,
                &material_path,
            )?)
            .with_context(|| format!("parsing effective material {material_path}"))?;
            let gate = material
                .root
                .named("material")
                .and_then(|material| material.named("g_stage"))
                .and_then(|stage| stage.named("with_static_vertex_color_meshes"))
                .and_then(|selector| selector.named("value"))
                .and_then(|value| match value {
                    ConfigValue::Boolean(value) => Some(*value),
                    _ => None,
                });
            material_vertex_color_gates.insert(material_path.clone(), gate);
            gate
        };
        if gate == Some(false) {
            bail!(
                "{model} surface {surface} uses static-vertex-color mesh type 2, but \
                 {material_path} disables material.g_stage.with_static_vertex_color_meshes; \
                 v0.100b will fail to load the surface material"
            );
        }
    }
    Ok(())
}

fn read_effective_resource(
    base: &mut ResourceDb,
    overlay_files: &BTreeMap<String, PathBuf>,
    resource: &str,
) -> Result<Vec<u8>> {
    if let Some(path) = overlay_files.get(resource) {
        fs::read(path).with_context(|| format!("reading {}", path.display()))
    } else {
        base.read(resource)
            .with_context(|| format!("reading effective resource {resource}"))
    }
}

fn validate_runtime_lod_contract(
    settings: &BinaryConfig,
    settings_path: &str,
    actual: &BTreeSet<String>,
) -> Result<()> {
    let ConfigValue::Named(levels) = settings.root.named("lod_hierrarchy").with_context(|| {
        format!(
            "{settings_path} has no lod_hierrarchy; v0.100b would leave model_lods_descriptor null"
        )
    })?
    else {
        bail!("{settings_path}.lod_hierrarchy is not a named table");
    };
    let mut lod0_count = 0;
    for level_name in ["LOD0", "LOD1", "LOD2"] {
        let level = levels
            .iter()
            .find(|entry| entry.name == level_name)
            .with_context(|| format!("{settings_path}.lod_hierrarchy has no {level_name}"))?;
        let ConfigValue::Named(fields) = &level.value else {
            bail!("{settings_path}.lod_hierrarchy.{level_name} is not a named table");
        };
        let surfaces = fields.iter().find(|entry| entry.name == "surfaces");
        let Some(surfaces) = surfaces else {
            continue;
        };
        let ConfigValue::Indexed(surfaces) = &surfaces.value else {
            bail!("{settings_path}.lod_hierrarchy.{level_name}.surfaces is not an indexed table");
        };
        if level_name == "LOD0" {
            lod0_count = surfaces.len();
        }
        let mut assigned_in_level = BTreeSet::new();
        for (index, surface) in surfaces.iter().enumerate() {
            let ConfigValue::String(surface) = surface else {
                bail!(
                    "{settings_path}.lod_hierrarchy.{level_name}.surfaces[{index}] is not string"
                );
            };
            if !actual.contains(surface) {
                bail!(
                    "{settings_path}.lod_hierrarchy.{level_name} names unknown render surface {surface:?}"
                );
            }
            if !assigned_in_level.insert(surface.clone()) {
                bail!(
                    "{settings_path}.lod_hierrarchy.{level_name} assigns render surface {surface:?} more than once"
                );
            }
        }
    }
    if lod0_count == 0 {
        bail!("{settings_path}.lod_hierrarchy.LOD0 has no render surfaces");
    }
    Ok(())
}

fn validate_overlay_configs(overlay_files: &BTreeMap<String, PathBuf>) -> Result<usize> {
    let mut validated = 0;
    for (resource, path) in overlay_files {
        if !is_known_binary_config(resource) || resource.ends_with("/settings") {
            continue;
        }
        let config = BinaryConfig::parse(
            &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing emitted self-describing config {resource}"))?;
        if resource.ends_with(".material") && config.root.named("material").is_none() {
            bail!("emitted material {resource} has no material table");
        }
        validated += 1;
    }
    Ok(validated)
}

fn validate_overlay_collision_chunks(overlay_files: &BTreeMap<String, PathBuf>) -> Result<()> {
    for (resource, path) in overlay_files {
        let leaf = resource.rsplit('/').next().unwrap_or(resource);
        let expected = match leaf {
            "vertices" if resource.contains("_collision/") || resource.contains("/collision/") => {
                Some((0x19, Some(12)))
            }
            "indices" if resource.contains("_collision/") || resource.contains("/collision/") => {
                Some((0x1a, Some(4)))
            }
            "face_data" if resource.contains("_collision/") || resource.contains("/collision/") => {
                Some((0x1b, None))
            }
            _ => None,
        };
        let Some((required_chunk, stride)) = expected else {
            continue;
        };
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        validate_chunked_resource(resource, &bytes, required_chunk, stride)?;
    }
    Ok(())
}

fn validate_chunked_resource(
    resource: &str,
    bytes: &[u8],
    required_chunk: u32,
    required_stride: Option<usize>,
) -> Result<()> {
    let mut offset = 0;
    let mut found = false;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .with_context(|| format!("{resource} has a truncated chunk header at {offset}"))?;
        let id = u32::from_le_bytes(header[..4].try_into().unwrap());
        let size = usize::try_from(u32::from_le_bytes(header[4..].try_into().unwrap()))?;
        let end = offset
            .checked_add(8)
            .and_then(|value| value.checked_add(size))
            .with_context(|| format!("{resource} chunk {id:#x} size overflows"))?;
        if end > bytes.len() {
            bail!(
                "{resource} chunk {id:#x} ends at {end}, beyond resource size {}",
                bytes.len()
            );
        }
        if id == required_chunk {
            found = true;
            if let Some(stride) = required_stride
                && size % stride != 0
            {
                bail!("{resource} chunk {id:#x} has {size} bytes, not a multiple of {stride}");
            }
        }
        offset = end;
    }
    if !found {
        bail!("{resource} has no required chunk {required_chunk:#x}");
    }
    Ok(())
}

fn is_known_binary_config(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    matches!(
        leaf,
        "client_project"
            | "server_project"
            | "export_properties"
            | "exported_primitives"
            | "db_static_dictionaries"
            | "localization"
    ) || [
        ".options",
        ".material",
        ".skeleton",
        ".clip",
        ".sound_collection",
        ".single_sound_options",
        ".particle",
        ".binary_particle",
    ]
    .iter()
    .any(|suffix| leaf.ends_with(suffix))
}

fn settings_surfaces(settings: &BinaryConfig, settings_path: &str) -> Result<BTreeSet<String>> {
    let ConfigValue::Named(values) = settings
        .root
        .named("material_settings")
        .with_context(|| format!("{settings_path} has no material_settings table"))?
    else {
        bail!("{settings_path} material_settings is not a named table");
    };
    Ok(values.iter().map(|value| value.name.clone()).collect())
}

fn rendered_surface(path: &str) -> Option<(String, String)> {
    let model_end = if let Some(index) = path.find(".skinned_model/render/") {
        index + ".skinned_model".len()
    } else if let Some(index) = path.find(".model/render/") {
        index + ".model".len()
    } else {
        return None;
    };
    let model = path[..model_end].to_owned();
    let rest = &path[model_end + "/render/".len()..];
    let surface = rest.strip_suffix("/converted_model")?;
    (!surface.is_empty() && !surface.contains('/')).then(|| (model, surface.to_owned()))
}

fn collect_overlay_files(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    collect_overlay_files_at(root, root, &mut files)?;
    Ok(files)
}

fn collect_overlay_files_at(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("reading overlay directory {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_overlay_files_at(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, path);
        } else {
            bail!(
                "overlay contains unsupported filesystem entry: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_namespace_compatibility<'a>(
    base: &ResourceDb,
    overlay_paths: impl IntoIterator<Item = &'a String>,
) -> Result<()> {
    for path in overlay_paths {
        let mut prefix = String::new();
        let components: Vec<_> = path.split('/').collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if base.is_file(&prefix) {
                bail!(
                    "additive overlay path {path} would replace stock file {prefix} with a folder"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_config::NamedValue;

    #[test]
    fn recognizes_direct_render_surfaces_only() {
        assert_eq!(
            rendered_surface("models/a.model/render/body/converted_model"),
            Some(("models/a.model".into(), "body".into()))
        );
        assert_eq!(
            rendered_surface("models/a.skinned_model/render/body/converted_model"),
            Some(("models/a.skinned_model".into(), "body".into()))
        );
        assert_eq!(
            rendered_surface("models/a.model/render/body/nested/converted_model"),
            None
        );
    }

    #[test]
    fn reads_material_setting_names_as_the_surface_contract() {
        let settings = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "material_settings".into(),
                value: ConfigValue::Named(vec![
                    NamedValue {
                        name: "body".into(),
                        value: ConfigValue::Named(Vec::new()),
                    },
                    NamedValue {
                        name: "lod1_body".into(),
                        value: ConfigValue::Named(Vec::new()),
                    },
                ]),
            }]),
        };
        assert_eq!(
            settings_surfaces(&settings, "model/settings").unwrap(),
            BTreeSet::from(["body".into(), "lod1_body".into()])
        );
    }

    #[test]
    fn normalizes_legacy_settings_to_emitted_surface_names() {
        let mut settings = BinaryConfig {
            root: ConfigValue::Named(vec![
                NamedValue {
                    name: "material_settings".into(),
                    value: ConfigValue::Named(vec![NamedValue {
                        name: "old_surface".into(),
                        value: ConfigValue::Named(vec![NamedValue {
                            name: "material_name".into(),
                            value: ConfigValue::String("material/example".into()),
                        }]),
                    }]),
                },
                NamedValue {
                    name: "lod_hierrarchy".into(),
                    value: ConfigValue::Named(vec![
                        NamedValue {
                            name: "LOD0".into(),
                            value: ConfigValue::Named(Vec::new()),
                        },
                        NamedValue {
                            name: "LOD1".into(),
                            value: ConfigValue::Named(Vec::new()),
                        },
                        NamedValue {
                            name: "LOD2".into(),
                            value: ConfigValue::Named(Vec::new()),
                        },
                    ]),
                },
            ]),
        };
        let actual = BTreeSet::from(["body".into(), "lod1_body".into()]);

        normalize_material_settings(&mut settings, "model/settings", &actual).unwrap();
        normalize_lod_hierarchy(&mut settings, &actual).unwrap();

        assert_eq!(
            settings_surfaces(&settings, "model/settings").unwrap(),
            actual
        );
        assert_eq!(
            settings
                .root
                .named("lod_hierrarchy")
                .and_then(|lod| lod.named("LOD0"))
                .and_then(|lod| lod.named("surfaces")),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "body".into()
            )]))
        );
        assert_eq!(
            settings
                .root
                .named("lod_hierrarchy")
                .and_then(|lod| lod.named("LOD1"))
                .and_then(|lod| lod.named("surfaces")),
            Some(&ConfigValue::Indexed(vec![ConfigValue::String(
                "lod1_body".into()
            )]))
        );
        validate_runtime_lod_contract(&settings, "model/settings", &actual).unwrap();
    }

    #[test]
    fn rejects_render_settings_without_the_runtime_lod_descriptor_source() {
        let settings = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "material_settings".into(),
                value: ConfigValue::Named(vec![NamedValue {
                    name: "body".into(),
                    value: ConfigValue::Named(Vec::new()),
                }]),
            }]),
        };
        let error = validate_runtime_lod_contract(
            &settings,
            "model/settings",
            &BTreeSet::from(["body".into()]),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("model_lods_descriptor null"));
    }
}
