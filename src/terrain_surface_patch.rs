use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::to_writer_pretty;

use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::weapon::{prepare_empty_output, record_file};
use crate::{BinaryConfig, ConfigValue, NamedValue, ResourceDb};

const STATIC_DECLARATION_SIZE: usize = 48;
const STATIC_VERTEX_STRIDE: usize = 32;

#[derive(Debug, Serialize)]
struct TerrainSurfacePatchRecord {
    format_version: u32,
    asset_id: String,
    source_prefix: String,
    target_prefix: String,
    project: String,
    material: String,
    models: usize,
    source_surfaces: usize,
    files: Vec<BuiltFile>,
}

struct StaticSurface {
    declaration: Vec<u8>,
    vertices: Vec<u8>,
    indices: Vec<u16>,
}

pub fn build_single_surface_bundle(
    source_db: &Path,
    source_prefix: &str,
    target_prefix: &str,
    project_path: &str,
    material: &str,
    output: &Path,
) -> Result<()> {
    validate_prefix(source_prefix)?;
    validate_prefix(target_prefix)?;
    if source_prefix == target_prefix {
        bail!("single-surface terrain diagnostic requires a distinct target prefix");
    }

    let mut source = ResourceDb::open(source_db)?;
    let mut models = BTreeMap::<String, Vec<String>>::new();
    for path in source.paths_with_prefix(&format!("{source_prefix}/")) {
        let Some((model_root, rest)) = path.split_once("/render/") else {
            continue;
        };
        if rest.contains('/') && rest.ends_with("/converted_model") {
            models
                .entry(model_root.to_owned())
                .or_default()
                .push(path.to_owned());
        }
    }
    if models.is_empty() {
        bail!("no static terrain surfaces found below {source_prefix:?}");
    }

    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let mut files = Vec::new();
    let mut source_surface_count = 0_usize;
    let mut model_rewrites = BTreeMap::new();

    for (source_model, mut surface_paths) in models {
        surface_paths.sort();
        source_surface_count += surface_paths.len();
        let suffix = source_model
            .strip_prefix(source_prefix)
            .with_context(|| format!("{source_model} is outside {source_prefix}"))?;
        let target_model = format!("{target_prefix}{suffix}");
        model_rewrites.insert(
            source_model
                .strip_prefix("models/")
                .unwrap_or(&source_model)
                .trim_end_matches(".model")
                .to_owned(),
            target_model
                .strip_prefix("models/")
                .unwrap_or(&target_model)
                .trim_end_matches(".model")
                .to_owned(),
        );

        let mut surfaces = Vec::with_capacity(surface_paths.len());
        for path in &surface_paths {
            surfaces.push(parse_static_surface(&source.read(path)?, path)?);
        }
        let merged = merge_static_surfaces(&surfaces, &source_model)?;
        write_resource(
            &overlay,
            &format!("{target_model}/render/terrain/converted_model"),
            &merged,
            "diagnostic:merged-single-terrain-surface",
            &mut files,
        )?;

        let bounds = static_bounds(&merged, &source_model)?;
        let (vertex_count, index_count) = static_counts(&merged, &source_model)?;
        let first_surface_root = surface_paths[0]
            .strip_suffix("/converted_model")
            .context("converted-model path has no leaf")?;
        let mut properties = BinaryConfig::parse(
            &source.read(&format!("{first_surface_root}/export_properties"))?,
        )?;
        properties.root.set_path(
            "source.indices",
            ConfigValue::Integer(i32::try_from(index_count)?),
        )?;
        properties.root.set_path(
            "source.primitives",
            ConfigValue::Integer(i32::try_from(index_count / 3)?),
        )?;
        properties.root.set_path(
            "source.mesh_name",
            ConfigValue::String("|STK2|terrain".into()),
        )?;
        properties.root.set_path(
            "source.texture_name",
            ConfigValue::String(material.to_owned()),
        )?;
        properties.root.set_path(
            "source.vertices",
            ConfigValue::Integer(i32::try_from(vertex_count)?),
        )?;
        properties
            .root
            .set_path("bounding_box.min", ConfigValue::Float3(bounds.0))?;
        properties
            .root
            .set_path("bounding_box.max", ConfigValue::Float3(bounds.1))?;
        let center = [
            (bounds.0[0] + bounds.1[0]) * 0.5,
            (bounds.0[1] + bounds.1[1]) * 0.5,
            (bounds.0[2] + bounds.1[2]) * 0.5,
        ];
        let radius = ((bounds.1[0] - center[0]).powi(2)
            + (bounds.1[1] - center[1]).powi(2)
            + (bounds.1[2] - center[2]).powi(2))
        .sqrt();
        properties
            .root
            .set_path("bounding_sphere.center", ConfigValue::Float3(center))?;
        properties
            .root
            .set_path("bounding_sphere.radius", ConfigValue::Float(radius))?;
        write_config(
            &overlay,
            &format!("{target_model}/render/terrain/export_properties"),
            &properties,
            "diagnostic:single-terrain-surface-properties",
            &mut files,
        )?;

        copy_resource(
            &mut source,
            &overlay,
            &format!("{source_model}/render/export_properties"),
            &format!("{target_model}/render/export_properties"),
            &mut files,
        )?;

        let mut settings =
            BinaryConfig::parse(&source.read(&format!("{source_model}/settings"))?)?;
        settings.root.set_path(
            "lod_hierrarchy.LOD0.surfaces",
            ConfigValue::Indexed(vec![ConfigValue::String("terrain".into())]),
        )?;
        settings.root.set_path(
            "material_settings",
            ConfigValue::Named(vec![NamedValue {
                name: "terrain".into(),
                value: ConfigValue::Named(vec![
                    NamedValue {
                        name: "material_name".into(),
                        value: ConfigValue::String(material.to_owned()),
                    },
                    NamedValue {
                        name: "used_in_portals".into(),
                        value: ConfigValue::Boolean(false),
                    },
                ]),
            }]),
        )?;
        settings
            .root
            .set_path("lod_switching.default_params", ConfigValue::Boolean(false))?;
        settings
            .root
            .set_path("lod_switching.param0", ConfigValue::Integer(80))?;
        settings
            .root
            .set_path("lod_switching.param1", ConfigValue::Integer(200))?;
        settings
            .root
            .set_path("lod_switching.param2", ConfigValue::Integer(300))?;
        settings
            .root
            .set_path("collision_enabled", ConfigValue::Boolean(true))?;
        settings.root.set_path(
            "walker_collision_enabled",
            ConfigValue::Boolean(true),
        )?;
        settings.root.set_path(
            "collision_use_raytrace_for_walker",
            ConfigValue::Boolean(true),
        )?;
        write_config(
            &overlay,
            &format!("{target_model}/settings"),
            &settings,
            "diagnostic:native-terrain-model-settings-contract",
            &mut files,
        )?;

        let collision_prefix = format!("{source_model}/collision/");
        let collision_paths = source
            .paths_with_prefix(&collision_prefix)
            .filter(|path| source.is_file(path))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for path in collision_paths {
            let destination = path.replacen(&source_model, &target_model, 1);
            copy_resource(&mut source, &overlay, &path, &destination, &mut files)?;
        }
    }

    let mut project = BinaryConfig::parse(&source.read(project_path)?)?;
    let rewrites = rewrite_project_model_names(&mut project.root, &model_rewrites);
    if rewrites < model_rewrites.len() * 2 {
        bail!(
            "project rewrite touched only {rewrites} model references for {} terrain cells",
            model_rewrites.len()
        );
    }
    write_config(
        &overlay,
        project_path,
        &project,
        "diagnostic:single-terrain-model-project-rewrite",
        &mut files,
    )?;

    let record = TerrainSurfacePatchRecord {
        format_version: 1,
        asset_id: "single-surface-native-contract-terrain-diagnostic".into(),
        source_prefix: source_prefix.to_owned(),
        target_prefix: target_prefix.to_owned(),
        project: project_path.to_owned(),
        material: material.to_owned(),
        models: model_rewrites.len(),
        source_surfaces: source_surface_count,
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;
    Ok(())
}

fn rewrite_project_model_names(
    value: &mut ConfigValue,
    rewrites: &BTreeMap<String, String>,
) -> usize {
    match value {
        ConfigValue::String(text) => {
            for (source, target) in rewrites {
                if text.contains(source) {
                    *text = text.replacen(source, target, 1);
                    return 1;
                }
            }
            0
        }
        ConfigValue::Named(values) => values
            .iter_mut()
            .map(|entry| rewrite_project_model_names(&mut entry.value, rewrites))
            .sum(),
        ConfigValue::Indexed(values) => values
            .iter_mut()
            .map(|entry| rewrite_project_model_names(entry, rewrites))
            .sum(),
        _ => 0,
    }
}

fn parse_static_surface(bytes: &[u8], path: &str) -> Result<StaticSurface> {
    if read_u32(bytes, 0)? != 3 {
        bail!("{path}: missing static vertex chunk");
    }
    let vertex_chunk_size = usize::try_from(read_u32(bytes, 4)?)?;
    let vertex_count = usize::try_from(read_u32(bytes, 8 + STATIC_DECLARATION_SIZE)?)?;
    let expected_vertex_chunk = STATIC_DECLARATION_SIZE + 4 + vertex_count * STATIC_VERTEX_STRIDE;
    if vertex_chunk_size != expected_vertex_chunk {
        bail!("{path}: unexpected static vertex chunk size");
    }
    let vertices_start = 8 + STATIC_DECLARATION_SIZE + 4;
    let vertices_end = vertices_start + vertex_count * STATIC_VERTEX_STRIDE;
    if read_u32(bytes, vertices_end)? != 4 {
        bail!("{path}: missing static index chunk");
    }
    let index_chunk_size = usize::try_from(read_u32(bytes, vertices_end + 4)?)?;
    let index_count = usize::try_from(read_u32(bytes, vertices_end + 8)?)?;
    if index_chunk_size != 4 + index_count * 2
        || bytes.len() != vertices_end + 8 + index_chunk_size
        || index_count % 3 != 0
    {
        bail!("{path}: malformed static index chunk");
    }
    let mut indices = Vec::with_capacity(index_count);
    for offset in (vertices_end + 12..bytes.len()).step_by(2) {
        let index = u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
        if usize::from(index) >= vertex_count {
            bail!("{path}: out-of-range static index");
        }
        indices.push(index);
    }
    Ok(StaticSurface {
        declaration: bytes[8..8 + STATIC_DECLARATION_SIZE].to_vec(),
        vertices: bytes[vertices_start..vertices_end].to_vec(),
        indices,
    })
}

fn merge_static_surfaces(surfaces: &[StaticSurface], path: &str) -> Result<Vec<u8>> {
    let declaration = &surfaces[0].declaration;
    let vertex_count: usize = surfaces
        .iter()
        .map(|surface| surface.vertices.len() / STATIC_VERTEX_STRIDE)
        .sum();
    if vertex_count > usize::from(u16::MAX) {
        bail!("{path}: merged terrain has {vertex_count} vertices, exceeding u16 indices");
    }
    let index_count: usize = surfaces.iter().map(|surface| surface.indices.len()).sum();
    let vertex_chunk_size = STATIC_DECLARATION_SIZE + 4 + vertex_count * STATIC_VERTEX_STRIDE;
    let index_chunk_size = 4 + index_count * 2;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + index_chunk_size);
    output.extend_from_slice(&3_u32.to_le_bytes());
    output.extend_from_slice(&u32::try_from(vertex_chunk_size)?.to_le_bytes());
    output.extend_from_slice(declaration);
    output.extend_from_slice(&u32::try_from(vertex_count)?.to_le_bytes());
    for surface in surfaces {
        if &surface.declaration != declaration {
            bail!("{path}: terrain surfaces use different vertex declarations");
        }
        output.extend_from_slice(&surface.vertices);
    }
    output.extend_from_slice(&4_u32.to_le_bytes());
    output.extend_from_slice(&u32::try_from(index_chunk_size)?.to_le_bytes());
    output.extend_from_slice(&u32::try_from(index_count)?.to_le_bytes());
    let mut vertex_base = 0_usize;
    for surface in surfaces {
        for index in &surface.indices {
            let merged = vertex_base + usize::from(*index);
            output.extend_from_slice(&u16::try_from(merged)?.to_le_bytes());
        }
        vertex_base += surface.vertices.len() / STATIC_VERTEX_STRIDE;
    }
    Ok(output)
}

fn static_counts(bytes: &[u8], path: &str) -> Result<(usize, usize)> {
    let surface = parse_static_surface(bytes, path)?;
    Ok((
        surface.vertices.len() / STATIC_VERTEX_STRIDE,
        surface.indices.len(),
    ))
}

fn static_bounds(bytes: &[u8], path: &str) -> Result<([f32; 3], [f32; 3])> {
    let surface = parse_static_surface(bytes, path)?;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in surface.vertices.chunks_exact(STATIC_VERTEX_STRIDE) {
        for axis in 0..3 {
            let offset = axis * 4;
            let value = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            if !value.is_finite() {
                bail!("{path}: non-finite position");
            }
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    Ok((min, max))
}

fn copy_resource(
    source: &mut ResourceDb,
    overlay: &Path,
    source_path: &str,
    target_path: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = source.read(source_path)?;
    write_resource(
        overlay,
        target_path,
        &bytes,
        &format!("diagnostic:copy:{source_path}"),
        files,
    )
}

fn write_config(
    overlay: &Path,
    path: &str,
    config: &BinaryConfig,
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    write_resource(overlay, path, &config.to_bytes()?, source, files)
}

fn write_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let destination = safe_join(overlay, path)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, bytes)?;
    files.push(record_file(path, source, bytes));
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated u32")?
            .try_into()
            .unwrap(),
    ))
}

fn validate_prefix(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("resource prefix is not safe: {path:?}");
    }
    Ok(())
}
