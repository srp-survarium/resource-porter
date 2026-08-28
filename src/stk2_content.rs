use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::to_writer_pretty;

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::armor::{convert_texture, validate_dds};
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::manifest::{
    BuiltFile, LobbyVisualSpec, SrpContentManifest, StaticModelPortSpec, Stk2ContentBuildRecord,
    Stk2ContentManifest, Stk2MaterialTemplates, VisualWeaponSpec,
};
use crate::weapon::{
    add_dictionary_item, add_localization_strings, apply_patches, parse_config,
    prepare_empty_output, record_file, reject_duplicate_outputs, replace_strings,
    write_generated_config,
};

const STATIC_VERTEX_DECLARATION: [u8; 48] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // position: float3
    0x00, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x03, 0x00, // normal: ubyte4n
    0x00, 0x00, 0x10, 0x00, 0x04, 0x00, 0x06, 0x00, // tangent: ubyte4n
    0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x07, 0x00, // binormal: ubyte4n
    0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x05, 0x00, // uv: float2
    0xff, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, // D3DDECL_END
];

#[derive(Clone, Debug)]
pub(crate) struct Stk2MapTexturePlan {
    pub source_texture: String,
    pub source_tga: String,
    pub source_options: String,
    pub target_texture: String,
    pub target_material: String,
    pub material_template: Option<String>,
    pub source_material: Option<String>,
    pub auxiliary_textures: Vec<Stk2MapAuxiliaryTexturePlan>,
    /// Reviewed target-owned texture bindings that require no source
    /// conversion (for example the stock flat normal used by lossy terrain).
    pub target_texture_bindings: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Stk2MapAuxiliaryTexturePlan {
    pub material_property: String,
    pub source_texture: String,
    pub source_tga: String,
    pub source_options: String,
    pub target_texture: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Stk2SourceMaterialContract {
    source_material: String,
    base_material: String,
    alpha_test: bool,
    two_sided: bool,
    metal: bool,
    decal: bool,
    texture_roles: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Stk2ResolvedSourceMaterial {
    pub template: String,
    pub source_material: String,
    pub source: String,
    pub texture_roles: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Stk2SourceMaterialCatalog {
    by_diffuse: BTreeMap<String, Vec<Stk2SourceMaterialContract>>,
}

pub(crate) fn auxiliary_texture_plans(
    resolved: &Stk2ResolvedSourceMaterial,
    map_name: &str,
    source_root: &Path,
) -> Result<Vec<Stk2MapAuxiliaryTexturePlan>> {
    let mut plans = Vec::new();
    for (material_property, source_texture) in &resolved.texture_roles {
        let source_tga_lower = format!("resources/sources/textures/{source_texture}.tga");
        let source_tga_upper = format!("resources/sources/textures/{source_texture}.TGA");
        let source_tga = if safe_join(source_root, &source_tga_lower)?.is_file() {
            source_tga_lower
        } else if safe_join(source_root, &source_tga_upper)?.is_file() {
            source_tga_upper
        } else {
            bail!(
                "STK2 material {} references missing auxiliary texture {}",
                resolved.source,
                source_texture
            );
        };
        let source_options = format!("resources/sources/textures/{source_texture}.options");
        if !safe_join(source_root, &source_options)?.is_file() {
            bail!(
                "STK2 material {} auxiliary texture {} has no options",
                resolved.source,
                source_texture
            );
        }
        plans.push(Stk2MapAuxiliaryTexturePlan {
            material_property: material_property.clone(),
            source_texture: source_texture.clone(),
            source_tga,
            source_options,
            target_texture: format!("ported/stk2/{map_name}/{source_texture}"),
        });
    }
    Ok(plans)
}

impl Stk2SourceMaterialCatalog {
    pub(crate) fn load(source_root: &Path) -> Result<Self> {
        let instances = safe_join(source_root, "resources/sources/material_instances")?;
        let bases = safe_join(source_root, "resources/sources/materials")?;
        if !instances.is_dir() || !bases.is_dir() {
            return Ok(Self::default());
        }

        let mut paths = Vec::new();
        collect_files_with_extension(&instances, "material", &mut paths)?;
        paths.sort();
        let mut base_texts = BTreeMap::new();
        let mut catalog = Self::default();
        for path in paths {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("reading STK2 source material {}", path.display()))?;
            let Some(diffuse) = lua_string_property(&text, "texture_diffuse") else {
                continue;
            };
            if diffuse.is_empty() {
                continue;
            }
            let Some(base_material) = lua_assignment_string(&text, "base_material") else {
                continue;
            };
            let base_text = if let Some(text) = base_texts.get(&base_material) {
                text
            } else {
                let base_path = bases.join(format!("{base_material}.material"));
                let text = fs::read_to_string(&base_path).with_context(|| {
                    format!(
                        "reading base material {base_material:?} for {}",
                        path.display()
                    )
                })?;
                base_texts.entry(base_material.clone()).or_insert(text)
            };
            let alpha_test = lua_bool_property(&text, "use_alpha_test")
                .or_else(|| lua_bool_property(base_text, "use_alpha_test"))
                .unwrap_or(false);
            let two_sided = lua_bool_property(&text, "two_sided")
                .or_else(|| lua_bool_property(base_text, "two_sided"))
                .unwrap_or(false);
            let source_material = path
                .strip_prefix(&instances)
                .unwrap_or(&path)
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            let lower_base = base_material.to_ascii_lowercase();
            let metal = lower_base.contains("metal")
                || matches!(
                    lower_base.as_str(),
                    "bronze" | "chrome" | "metal_alum_brushed" | "wire_base_lighted"
                );
            let decal = lower_base.contains("decal");
            let texture_roles = [
                "texture_normal",
                "texture_detail_normal",
                "texture_specular_intensity",
                "texture_specular_power",
                "texture_emissive",
                "texture_translucency",
                "texture_bump",
            ]
            .into_iter()
            .filter_map(|property| {
                lua_string_property(&text, property)
                    .filter(|texture| !texture.is_empty())
                    .map(|texture| (property.to_owned(), normalize_stk2_resource_name(&texture)))
            })
            .collect();
            catalog
                .by_diffuse
                .entry(normalize_stk2_resource_name(&diffuse))
                .or_default()
                .push(Stk2SourceMaterialContract {
                    source_material,
                    base_material,
                    alpha_test,
                    two_sided,
                    metal,
                    decal,
                    texture_roles,
                });
        }
        Ok(catalog)
    }

    pub(crate) fn resolve(
        &self,
        diffuse: &str,
        templates: &Stk2MaterialTemplates,
    ) -> Option<Stk2ResolvedSourceMaterial> {
        let contracts = self
            .by_diffuse
            .get(&normalize_stk2_resource_name(diffuse))?;
        let first = contracts.first()?;
        if contracts.iter().any(|contract| {
            contract.alpha_test != first.alpha_test
                || contract.two_sided != first.two_sided
                || contract.metal != first.metal
                || contract.decal != first.decal
                || contract.texture_roles != first.texture_roles
        }) {
            return None;
        }
        Some(resolve_stk2_source_material(first, templates))
    }

    pub(crate) fn resolve_exact(
        &self,
        diffuse: &str,
        source_material: &str,
        templates: &Stk2MaterialTemplates,
    ) -> Result<Stk2ResolvedSourceMaterial> {
        let normalized_diffuse = normalize_stk2_resource_name(diffuse);
        let contracts = self.by_diffuse.get(&normalized_diffuse).with_context(|| {
            format!("STK2 diffuse {diffuse:?} has no source material instances")
        })?;
        let normalized_source = normalize_stk2_resource_name(source_material);
        let contract = contracts
            .iter()
            .find(|contract| {
                normalize_stk2_resource_name(&contract.source_material) == normalized_source
            })
            .with_context(|| {
                let candidates: Vec<_> = contracts
                    .iter()
                    .map(|contract| contract.source_material.as_str())
                    .collect();
                format!(
                    "STK2 diffuse {diffuse:?} is not used by override material {source_material:?}; candidates: {candidates:?}"
                )
            })?;
        Ok(resolve_stk2_source_material(contract, templates))
    }
}

fn resolve_stk2_source_material(
    contract: &Stk2SourceMaterialContract,
    templates: &Stk2MaterialTemplates,
) -> Stk2ResolvedSourceMaterial {
    let template = if contract.decal {
        &templates.decal
    } else if contract.metal && contract.alpha_test && contract.two_sided {
        &templates.metal_alpha_test_two_sided
    } else if contract.metal && contract.alpha_test {
        &templates.metal_alpha_test
    } else if contract.metal && contract.two_sided {
        &templates.metal_two_sided
    } else if contract.metal {
        &templates.metal
    } else if contract.alpha_test && contract.two_sided {
        &templates.alpha_test_two_sided
    } else if contract.alpha_test {
        &templates.alpha_test
    } else if contract.two_sided {
        &templates.two_sided
    } else {
        &templates.opaque
    };
    Stk2ResolvedSourceMaterial {
        template: template.clone(),
        source_material: contract.source_material.clone(),
        source: format!("{} ({})", contract.source_material, contract.base_material),
        texture_roles: contract.texture_roles.clone(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Stk2TerrainCell {
    pub name: String,
    row_size: usize,
    physical_size: f32,
    textures: Vec<String>,
    transform: [[f32; 3]; 4],
    records: Vec<Stk2TerrainVertex>,
}

#[derive(Clone, Copy, Debug)]
struct Stk2TerrainVertex {
    height: f32,
    texture_shift: [f32; 2],
    texture_indices: [u8; 3],
    texture_alphas: [u8; 3],
    _color: u32,
}

#[link(name = "z")]
unsafe extern "C" {
    fn uncompress(
        destination: *mut u8,
        destination_length: *mut usize,
        source: *const u8,
        source_length: usize,
    ) -> i32;
}

pub(crate) fn load_stk2_terrain_cells(
    source_root: &Path,
    terrain_directory: &str,
) -> Result<Vec<Stk2TerrainCell>> {
    let directory = safe_join(source_root, terrain_directory)?;
    let mut paths = fs::read_dir(&directory)
        .with_context(|| format!("reading STK2 terrain directory {}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("terr"));
    paths.sort();
    if paths.is_empty() {
        bail!("STK2 terrain directory {} is empty", directory.display());
    }
    paths
        .iter()
        .map(|path| parse_stk2_terrain_cell(path))
        .collect()
}

pub(crate) fn terrain_texture_names(cells: &[Stk2TerrainCell]) -> BTreeSet<String> {
    cells
        .iter()
        .flat_map(|cell| cell.textures.iter().cloned())
        .collect()
}

pub(crate) fn terrain_texture_plans(
    cells: &[Stk2TerrainCell],
    map_name: &str,
    source_root: &Path,
    aliases: &BTreeMap<String, String>,
    texture_binding_overrides: &BTreeMap<String, String>,
) -> Result<Vec<Stk2MapTexturePlan>> {
    let unique_textures: BTreeSet<_> = cells
        .iter()
        .flat_map(|cell| cell.textures.iter())
        .cloned()
        .collect();
    let mut missing = Vec::new();
    for texture in &unique_textures {
        let source_texture = aliases.get(texture).unwrap_or(texture);
        let lower = format!("resources/sources/textures/{source_texture}.tga");
        let upper = format!("resources/sources/textures/{source_texture}.TGA");
        let options = format!("resources/sources/textures/{source_texture}.options");
        if (!safe_join(source_root, &lower)?.is_file()
            && !safe_join(source_root, &upper)?.is_file())
            || !safe_join(source_root, &options)?.is_file()
        {
            missing.push(format!("{texture} (source {source_texture})"));
        }
    }
    if !missing.is_empty() {
        bail!("STK2 terrain textures have missing TGA/options payloads: {missing:?}");
    }

    let mut plans = BTreeMap::new();
    for texture in cells.iter().flat_map(|cell| cell.textures.iter()) {
        let source_texture = aliases.get(texture).unwrap_or(texture);
        let source_tga_lower = format!("resources/sources/textures/{source_texture}.tga");
        let source_tga_upper = format!("resources/sources/textures/{source_texture}.TGA");
        let source_tga = if safe_join(source_root, &source_tga_lower)?.is_file() {
            source_tga_lower
        } else if safe_join(source_root, &source_tga_upper)?.is_file() {
            source_tga_upper
        } else {
            bail!("STK2 terrain texture {texture:?} (source {source_texture:?}) has no source TGA");
        };
        let source_options = format!("resources/sources/textures/{source_texture}.options");
        if !safe_join(source_root, &source_options)?.is_file() {
            bail!(
                "STK2 terrain texture {texture:?} (source {source_texture:?}) has no source options"
            );
        }
        let target_root = format!("ported/stk2/{map_name}/{texture}");
        let mut target_texture_bindings =
            BTreeMap::from([("texture_normal".into(), "editor/editor_dummy_nmap".into())]);
        target_texture_bindings.extend(texture_binding_overrides.clone());
        plans.insert(
            texture.clone(),
            Stk2MapTexturePlan {
                source_texture: source_texture.clone(),
                source_tga,
                source_options,
                target_texture: target_root.clone(),
                target_material: target_root,
                material_template: None,
                source_material: Some("terrain-dominant-layer:flat-target-normal".into()),
                auxiliary_textures: Vec::new(),
                target_texture_bindings,
            },
        );
    }
    Ok(plans.into_values().collect())
}

pub(crate) fn build_stk2_terrain_models(
    cells: &[Stk2TerrainCell],
    map_name: &str,
    source_origin: [f32; 3],
    template: &crate::manifest::StaticModelTemplateSpec,
    collision_game_material: (&str, i32),
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<Vec<LobbyVisualSpec>> {
    let mut visuals = Vec::with_capacity(cells.len());
    for cell in cells {
        let target_library = format!("level/{map_name}/terrain/{}", cell.name);
        let target_root = format!("models/{target_library}.model");
        let positions = terrain_positions(cell)?;
        let bases = terrain_bases(cell, &positions)?;
        let groups = terrain_triangle_groups(cell)?;
        let mut material_bindings = BTreeMap::new();

        let mut render_properties = parse_config(
            target,
            &template.render_properties_template,
            "terrain render-properties template",
        )?;
        render_properties.root.set_path(
            "src_filename",
            ConfigValue::String(format!("STK2/terrain/{}.terr", cell.name)),
        )?;
        write_generated_config(
            overlay,
            &format!("{target_root}/render/export_properties"),
            &render_properties,
            "generated:STK2-terrain-render-properties",
            files,
        )?;

        for (texture_index, triangles) in groups {
            let surface = format!("terrain_layer_{texture_index}");
            let texture = cell
                .textures
                .get(texture_index)
                .with_context(|| format!("terrain layer {texture_index} is out of range"))?;
            let material = format!("ported/stk2/{map_name}/{texture}");
            material_bindings.insert(surface.clone(), material.clone());
            let (vertices, indices) =
                make_terrain_surface_source(cell, &positions, &bases, &triangles)?;
            validate_static_geometry(&vertices, &indices, &surface)?;
            let converted = convert_static_surface(&vertices, &indices)?;
            write_raw_resource(
                overlay,
                &format!("{target_root}/render/{surface}/converted_model"),
                &converted,
                "generated:STK2-terrain-static-model",
                files,
            )?;
            let properties = make_surface_properties(
                parse_config(
                    target,
                    &template.surface_properties_template,
                    "terrain surface-properties template",
                )?,
                &surface,
                &material,
                read_u32(&vertices, 0)?,
                read_u32(&indices, 0)?,
                geometry_bounds(&vertices)?,
                Some(0),
            )?;
            write_generated_config(
                overlay,
                &format!("{target_root}/render/{surface}/export_properties"),
                &properties,
                "generated:STK2-terrain-surface-properties",
                files,
            )?;
        }

        let settings_template = parse_config(
            target,
            &template.settings_template,
            "terrain settings template",
        )?;
        let mut settings = replace_settings_surfaces(settings_template, &material_bindings, false)?;
        set_collision_game_material(
            &mut settings,
            collision_game_material.0,
            collision_game_material.1,
        )?;
        write_generated_config(
            overlay,
            &format!("{target_root}/settings"),
            &settings,
            "generated:STK2-terrain-settings",
            files,
        )?;
        write_terrain_collision(&target_root, cell, &positions, overlay, files)?;
        visuals.push(LobbyVisualSpec {
            library_name: target_library,
            position: [
                cell.transform[3][0] - source_origin[0],
                cell.transform[3][1] - source_origin[1],
                cell.transform[3][2] - source_origin[2],
            ],
            rotation: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        });
    }
    Ok(visuals)
}

fn parse_stk2_terrain_cell(path: &Path) -> Result<Stk2TerrainCell> {
    let bytes = fs::read(path)?;
    if read_u32(&bytes, 0)? != 103
        || usize::try_from(read_u32(&bytes, 4)?)? != bytes.len().saturating_sub(8)
    {
        bail!("{}: malformed terrain chunk 103", path.display());
    }
    let payload = zlib_uncompress(&bytes[8..])
        .with_context(|| format!("decompressing STK2 terrain {}", path.display()))?;
    let mut offset = 0_usize;
    let row_size = usize::try_from(read_u32(&payload, offset)?)?;
    offset += 4;
    let physical_size = read_f32(&payload, offset)?;
    offset += 4;
    let texture_count = usize::try_from(read_u32(&payload, offset)?)?;
    offset += 4;
    if row_size < 2 || row_size > 1024 || !physical_size.is_finite() || physical_size <= 0.0 {
        bail!(
            "{}: invalid terrain dimensions {row_size}/{physical_size}",
            path.display()
        );
    }
    let mut textures = Vec::with_capacity(texture_count);
    for _ in 0..texture_count {
        let end = payload[offset..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|relative| offset + relative)
            .context("terrain texture name is not NUL terminated")?;
        let texture = std::str::from_utf8(&payload[offset..end])?.to_owned();
        if texture.is_empty()
            || texture.starts_with('/')
            || texture.split('/').any(|component| {
                component.is_empty()
                    || component == "."
                    || component == ".."
                    || !component
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
        {
            bail!("{}: unsafe terrain texture {texture:?}", path.display());
        }
        textures.push(texture);
        offset = end + 1;
    }
    let mut transform = [[0.0_f32; 3]; 4];
    for row in &mut transform {
        *row = read_float3(&payload, offset)?;
        offset += 12;
        if !row.iter().all(|value| value.is_finite()) {
            bail!("{}: non-finite terrain transform", path.display());
        }
    }
    let record_count = row_size
        .checked_mul(row_size)
        .context("terrain record count overflow")?;
    let expected = offset
        .checked_add(record_count * 22)
        .context("terrain payload size overflow")?;
    if payload.len() != expected {
        bail!(
            "{}: terrain payload has {} bytes, expected {expected}",
            path.display(),
            payload.len()
        );
    }
    let mut records = Vec::with_capacity(record_count);
    for record_index in 0..record_count {
        let start = offset + record_index * 22;
        let record = Stk2TerrainVertex {
            height: read_f32(&payload, start)?,
            texture_shift: [
                read_f32(&payload, start + 4)?,
                read_f32(&payload, start + 8)?,
            ],
            texture_indices: [
                payload[start + 12],
                payload[start + 13],
                payload[start + 14],
            ],
            texture_alphas: [
                payload[start + 15],
                payload[start + 16],
                payload[start + 17],
            ],
            _color: read_u32(&payload, start + 18)?,
        };
        if !record.height.is_finite()
            || !record.texture_shift.into_iter().all(f32::is_finite)
            || record
                .texture_indices
                .into_iter()
                .any(|index| usize::from(index) >= textures.len())
        {
            bail!("{}: invalid terrain record {record_index}", path.display());
        }
        records.push(record);
    }
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .context("terrain cell has no UTF-8 file stem")?
        .to_owned();
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'_' | b'-'))
    {
        bail!("unsafe terrain cell name {name:?}");
    }
    Ok(Stk2TerrainCell {
        name,
        row_size,
        physical_size,
        textures,
        transform,
        records,
    })
}

fn zlib_uncompress(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut capacity = compressed.len().saturating_mul(8).max(128 * 1024);
    loop {
        let mut output = vec![0_u8; capacity];
        let mut output_length = output.len();
        // SAFETY: both buffers remain allocated for the duration of the call,
        // and zlib receives their exact lengths.
        let result = unsafe {
            uncompress(
                output.as_mut_ptr(),
                &mut output_length,
                compressed.as_ptr(),
                compressed.len(),
            )
        };
        match result {
            0 => {
                output.truncate(output_length);
                return Ok(output);
            }
            -5 if capacity < 64 * 1024 * 1024 => {
                capacity = capacity
                    .checked_mul(2)
                    .context("terrain decompression buffer overflow")?;
            }
            code => bail!("zlib uncompress failed with code {code}"),
        }
    }
}

fn terrain_positions(cell: &Stk2TerrainCell) -> Result<Vec<[f32; 3]>> {
    let step = cell.physical_size / (cell.row_size - 1) as f32;
    let mut positions = Vec::with_capacity(cell.records.len());
    for row in 0..cell.row_size {
        for column in 0..cell.row_size {
            let local = [
                column as f32 * step,
                cell.records[row * cell.row_size + column].height,
                -(row as f32) * step,
            ];
            let transformed = [
                cell.transform[0][0] * local[0]
                    + cell.transform[1][0] * local[1]
                    + cell.transform[2][0] * local[2],
                cell.transform[0][1] * local[0]
                    + cell.transform[1][1] * local[1]
                    + cell.transform[2][1] * local[2],
                cell.transform[0][2] * local[0]
                    + cell.transform[1][2] * local[1]
                    + cell.transform[2][2] * local[2],
            ];
            if !transformed.into_iter().all(f32::is_finite) {
                bail!("terrain {} produced a non-finite position", cell.name);
            }
            positions.push(transformed);
        }
    }
    Ok(positions)
}

fn terrain_bases(cell: &Stk2TerrainCell, positions: &[[f32; 3]]) -> Result<Vec<[[f32; 3]; 3]>> {
    let mut bases = Vec::with_capacity(positions.len());
    for row in 0..cell.row_size {
        for column in 0..cell.row_size {
            let left = positions[row * cell.row_size + column.saturating_sub(1)];
            let right = positions[row * cell.row_size + (column + 1).min(cell.row_size - 1)];
            let up = positions[row.saturating_sub(1) * cell.row_size + column];
            let down = positions[(row + 1).min(cell.row_size - 1) * cell.row_size + column];
            let tangent = normalize3(sub3(right, left))
                .with_context(|| format!("terrain {} has a degenerate X tangent", cell.name))?;
            let row_direction = normalize3(sub3(down, up))
                .with_context(|| format!("terrain {} has a degenerate Z tangent", cell.name))?;
            let normal = normalize3(cross3(tangent, row_direction))
                .with_context(|| format!("terrain {} has a degenerate normal", cell.name))?;
            let binormal = normalize3(cross3(normal, tangent))
                .with_context(|| format!("terrain {} has a degenerate binormal", cell.name))?;
            bases.push([normal, tangent, binormal]);
        }
    }
    Ok(bases)
}

fn terrain_triangle_groups(cell: &Stk2TerrainCell) -> Result<BTreeMap<usize, Vec<[u16; 3]>>> {
    let mut groups = BTreeMap::<usize, Vec<[u16; 3]>>::new();
    for row in 0..cell.row_size - 1 {
        for column in 0..cell.row_size - 1 {
            let top_left = row * cell.row_size + column;
            let top_right = top_left + 1;
            let bottom_left = top_left + cell.row_size;
            let bottom_right = bottom_left + 1;
            for triangle in [
                [top_left, top_right, bottom_left],
                [top_right, bottom_right, bottom_left],
            ] {
                let mut weights = vec![0_u32; cell.textures.len()];
                for vertex_index in triangle {
                    let record = cell.records[vertex_index];
                    for layer in 0..3 {
                        weights[usize::from(record.texture_indices[layer])] +=
                            u32::from(record.texture_alphas[layer]);
                    }
                }
                let texture_index = weights
                    .iter()
                    .enumerate()
                    .max_by_key(|(index, weight)| (**weight, std::cmp::Reverse(*index)))
                    .map(|(index, _)| index)
                    .context("terrain cell has no texture layers")?;
                groups.entry(texture_index).or_default().push([
                    u16::try_from(triangle[0])?,
                    u16::try_from(triangle[1])?,
                    u16::try_from(triangle[2])?,
                ]);
            }
        }
    }
    Ok(groups)
}

fn make_terrain_surface_source(
    cell: &Stk2TerrainCell,
    positions: &[[f32; 3]],
    bases: &[[[f32; 3]; 3]],
    triangles: &[[u16; 3]],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut remap = BTreeMap::<u16, u16>::new();
    for triangle in triangles {
        for source_index in triangle {
            let next = u16::try_from(remap.len())?;
            remap.entry(*source_index).or_insert(next);
        }
    }
    let mut ordered = vec![0_u16; remap.len()];
    for (source, target) in &remap {
        ordered[usize::from(*target)] = *source;
    }
    let mut vertices = Vec::with_capacity(4 + ordered.len() * 56);
    push_u32(&mut vertices, u32::try_from(ordered.len())?);
    let step = cell.physical_size / (cell.row_size - 1) as f32;
    for source_index in ordered {
        let index = usize::from(source_index);
        for component in positions[index] {
            push_f32(&mut vertices, component);
        }
        for basis in bases[index] {
            for component in basis {
                push_f32(&mut vertices, component);
            }
        }
        let row = index / cell.row_size;
        let column = index % cell.row_size;
        let shift = cell.records[index].texture_shift;
        push_f32(&mut vertices, column as f32 * step / 4.0 + shift[0]);
        push_f32(&mut vertices, row as f32 * step / 4.0 + shift[1]);
    }
    let mut indices = Vec::with_capacity(4 + triangles.len() * 6);
    push_u32(&mut indices, u32::try_from(triangles.len() * 3)?);
    for triangle in triangles {
        for source_index in triangle {
            indices.extend_from_slice(&remap[source_index].to_le_bytes());
        }
    }
    Ok((vertices, indices))
}

fn write_terrain_collision(
    target_root: &str,
    cell: &Stk2TerrainCell,
    positions: &[[f32; 3]],
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut vertices = Vec::with_capacity(8 + positions.len() * 12);
    push_u32(&mut vertices, 0x19);
    push_u32(&mut vertices, u32::try_from(positions.len() * 12)?);
    for position in positions {
        for component in position {
            push_f32(&mut vertices, *component);
        }
    }
    let triangle_count = (cell.row_size - 1) * (cell.row_size - 1) * 2;
    let mut indices = Vec::with_capacity(8 + triangle_count * 12);
    push_u32(&mut indices, 0x1a);
    push_u32(&mut indices, u32::try_from(triangle_count * 12)?);
    for row in 0..cell.row_size - 1 {
        for column in 0..cell.row_size - 1 {
            let top_left = row * cell.row_size + column;
            let top_right = top_left + 1;
            let bottom_left = top_left + cell.row_size;
            let bottom_right = bottom_left + 1;
            for index in [
                top_left,
                top_right,
                bottom_left,
                top_right,
                bottom_right,
                bottom_left,
            ] {
                push_u32(&mut indices, u32::try_from(index)?);
            }
        }
    }
    validate_stk2_collision(&vertices, &indices, target_root)?;
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/vertices"),
        &vertices,
        "generated:STK2-terrain-collision-vertices",
        files,
    )?;
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/indices"),
        &indices,
        "generated:STK2-terrain-collision-indices",
        files,
    )?;
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/face_data"),
        &make_stk2_collision_face_data(triangle_count)?,
        "generated:STK2-terrain-collision-face-data",
        files,
    )?;
    Ok(())
}

pub fn build_bundle(
    manifest: &Stk2ContentManifest,
    source_root: &Path,
    target_db: &Path,
    output: &Path,
    preserve_lobby: bool,
) -> Result<Stk2ContentBuildRecord> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut target = ResourceDb::open(target_db)?;
    let mut files = Vec::new();

    build_materials_and_textures(manifest, source_root, &mut target, &overlay, &mut files)?;

    let mut dictionary = parse_config(&mut target, &manifest.dictionary.path, "target dictionary")?;
    for weapon in &manifest.weapons {
        build_weapon(
            manifest,
            weapon,
            source_root,
            &mut target,
            &overlay,
            &mut files,
        )?;
        add_dictionary_item(
            &mut dictionary.root,
            weapon.dict_id,
            &weapon.target_config,
            weapon.item_category,
            false,
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

    if !preserve_lobby {
        for model in &manifest.map_models {
            build_static_model(
                &manifest.static_model,
                model,
                None,
                source_root,
                &mut target,
                &overlay,
                &mut files,
            )?;
        }
        replace_lobby(manifest, &mut target, &overlay, &mut files)?;
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;
    let record = Stk2ContentBuildRecord {
        format_version: 1,
        asset_id: manifest.asset.id.clone(),
        donor_build: manifest.asset.donor_build.clone(),
        target_build: manifest.asset.target_build.clone(),
        weapon_dict_ids: manifest
            .weapons
            .iter()
            .map(|weapon| weapon.dict_id)
            .collect(),
        replaced_project: if preserve_lobby {
            format!("preserved:{}", manifest.lobby.template_project)
        } else {
            manifest.lobby.target_project.clone()
        },
        files,
    };
    to_writer_pretty(
        fs::File::create(output.join("port-manifest.json"))?,
        &record,
    )?;

    let weapon_ids = manifest
        .weapons
        .iter()
        .map(|weapon| {
            u16::try_from(weapon.dict_id)
                .with_context(|| format!("weapon ID {} exceeds the SRP protocol", weapon.dict_id))
        })
        .collect::<Result<Vec<_>>>()?;
    let primary = weapon_ids.first().copied();
    let server_dir = output.join("server");
    fs::create_dir_all(&server_dir)?;
    to_writer_pretty(
        fs::File::create(server_dir.join("srp-content.json"))?,
        &SrpContentManifest {
            format_version: 1,
            extra_weapon_dict_ids: weapon_ids,
            extra_ammo_dict_ids: Vec::new(),
            primary_weapon_dict_id: primary,
            primary_ammo_dict_id: None,
        },
    )?;
    Ok(record)
}

fn build_materials_and_textures(
    manifest: &Stk2ContentManifest,
    source_root: &Path,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    for material_spec in &manifest.materials {
        let mut material =
            parse_config(target, &material_spec.template, "target material template")?;
        replace_strings(&mut material.root, &material_spec.replace_strings);
        write_generated_config(
            overlay,
            &material_spec.target,
            &material,
            "generated:target-schema-material",
            files,
        )?;
    }

    for texture in &manifest.textures {
        let source = safe_join(source_root, &texture.source_file)?;
        if !source.is_file() {
            bail!("STK2 texture does not exist: {}", source.display());
        }
        let dds_path = format!("{}.dds", texture.target);
        let output = safe_join(overlay, &dds_path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        convert_texture(&source, &output, texture.compression)
            .with_context(|| format!("converting STK2 texture {}", source.display()))?;
        let dds = fs::read(&output)?;
        validate_dds(&dds, texture.compression, &dds_path)?;
        files.push(record_file(
            &dds_path,
            &format!("converted:STK2:{}", texture.source_file),
            &dds,
        ));

        let options_path = format!("{}.options", texture.target);
        let options = target.read(&texture.template_options)?;
        write_raw_resource(
            overlay,
            &options_path,
            &options,
            &format!("target-template:{}", texture.template_options),
            files,
        )?;
    }
    Ok(())
}

pub(crate) fn build_map_materials_and_textures(
    plans: &[Stk2MapTexturePlan],
    material_template: &str,
    options_template: &str,
    source_root: &Path,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut converted_diffuse_textures = BTreeMap::<String, (String, String)>::new();
    let mut converted_auxiliary_textures = BTreeSet::new();
    for plan in plans {
        let source_options = safe_join(source_root, &plan.source_options)?;
        let options_text = fs::read_to_string(&source_options).with_context(|| {
            format!(
                "reading STK2 source texture options {}",
                source_options.display()
            )
        })?;
        let source_format = source_option_string(&options_text, "format")?;
        let srgb = source_option_integer(&options_text, "srgb")?;
        let has_mips = source_option_integer(&options_text, "has_mip_levels")?;
        let dimension = source_option_integer(&options_text, "texture_dimension")?;
        if !matches!(srgb, 0 | 1) || has_mips != 1 || dimension != 1 {
            bail!(
                "{}: unsupported source texture options srgb={srgb}, has_mip_levels={has_mips}, texture_dimension={dimension}",
                plan.source_texture
            );
        }
        let format = Stk2DiffuseFormat::parse(&source_format)
            .with_context(|| format!("source texture {}", plan.source_texture))?;
        let diffuse_source = (plan.source_tga.clone(), plan.source_options.clone());
        if let Some(existing) = converted_diffuse_textures.get(&plan.target_texture) {
            if existing != &diffuse_source {
                bail!(
                    "target diffuse texture {} has conflicting STK2 sources {:?} and {:?}",
                    plan.target_texture,
                    existing,
                    diffuse_source
                );
            }
        } else {
            converted_diffuse_textures.insert(plan.target_texture.clone(), diffuse_source);
            let source_tga = safe_join(source_root, &plan.source_tga)?;
            let dds_path = format!("textures/{}.dds", plan.target_texture);
            let dds_output = safe_join(overlay, &dds_path)?;
            if let Some(parent) = dds_output.parent() {
                fs::create_dir_all(parent)?;
            }
            convert_stk2_diffuse_texture(&source_tga, &dds_output, format, srgb as u8)?;
            let dds = fs::read(&dds_output)?;
            files.push(record_file(
                &dds_path,
                &format!("converted:STK2:{}", plan.source_tga),
                &dds,
            ));

            let mut options =
                parse_config(target, options_template, "target texture options template")?;
            options.root.set_path(
                "options.format",
                ConfigValue::String(format.target_options_name().to_owned()),
            )?;
            options
                .root
                .set_path("options.srgb", ConfigValue::Integer(srgb))?;
            options
                .root
                .set_path("options.has_mip_levels", ConfigValue::Integer(has_mips))?;
            write_generated_config(
                overlay,
                &format!("textures/{}.options", plan.target_texture),
                &options,
                "generated:target-schema-STK2-texture-options",
                files,
            )?;
        }

        let selected_template = plan
            .material_template
            .as_deref()
            .unwrap_or(material_template);
        let mut material = parse_config(target, selected_template, "target material template")?;
        material.root.set_path(
            "material.g_stage.effect.texture_diffuse.value",
            ConfigValue::String(plan.target_texture.clone()),
        )?;
        for auxiliary in &plan.auxiliary_textures {
            if converted_auxiliary_textures.insert(auxiliary.target_texture.clone()) {
                let source_options = safe_join(source_root, &auxiliary.source_options)?;
                let options_text = fs::read_to_string(&source_options).with_context(|| {
                    format!(
                        "reading STK2 auxiliary texture options {}",
                        source_options.display()
                    )
                })?;
                let source_format = source_option_string(&options_text, "format")?;
                let srgb = source_option_integer(&options_text, "srgb")?;
                let has_mips = source_option_integer(&options_text, "has_mip_levels")?;
                let dimension = source_option_integer(&options_text, "texture_dimension")?;
                if !matches!(srgb, 0 | 1) || has_mips != 1 || dimension != 1 {
                    bail!(
                        "{}: unsupported auxiliary texture options srgb={srgb}, has_mip_levels={has_mips}, texture_dimension={dimension}",
                        auxiliary.source_texture
                    );
                }
                let format = Stk2DiffuseFormat::parse(&source_format).with_context(|| {
                    format!("source auxiliary texture {}", auxiliary.source_texture)
                })?;
                let source_tga = safe_join(source_root, &auxiliary.source_tga)?;
                let dds_path = format!("textures/{}.dds", auxiliary.target_texture);
                let dds_output = safe_join(overlay, &dds_path)?;
                if let Some(parent) = dds_output.parent() {
                    fs::create_dir_all(parent)?;
                }
                convert_stk2_diffuse_texture(&source_tga, &dds_output, format, srgb as u8)?;
                let dds = fs::read(&dds_output)?;
                files.push(record_file(
                    &dds_path,
                    &format!("converted:STK2:{}", auxiliary.source_tga),
                    &dds,
                ));

                let mut options =
                    parse_config(target, options_template, "target texture options template")?;
                options.root.set_path(
                    "options.format",
                    ConfigValue::String(format.target_options_name().to_owned()),
                )?;
                options
                    .root
                    .set_path("options.srgb", ConfigValue::Integer(srgb))?;
                options
                    .root
                    .set_path("options.has_mip_levels", ConfigValue::Integer(has_mips))?;
                write_generated_config(
                    overlay,
                    &format!("textures/{}.options", auxiliary.target_texture),
                    &options,
                    "generated:target-schema-STK2-auxiliary-texture-options",
                    files,
                )?;
            }
            material.root.set_path(
                &format!(
                    "material.g_stage.effect.{}.value",
                    auxiliary.material_property
                ),
                ConfigValue::String(auxiliary.target_texture.clone()),
            )?;
        }
        for (material_property, target_texture) in &plan.target_texture_bindings {
            let dds = format!("textures/{target_texture}.dds");
            let options = format!("textures/{target_texture}.options");
            if !target.is_file(&dds) || !target.is_file(&options) {
                bail!(
                    "target-owned material binding {material_property} references absent {target_texture}"
                );
            }
            material.root.set_path(
                &format!("material.g_stage.effect.{material_property}.value"),
                ConfigValue::String(target_texture.clone()),
            )?;
        }
        // Shader feature booleans are part of the old build's precompiled
        // permutation key. Preserve the reviewed target template byte-for-byte
        // except for texture bindings; synthesizing a new boolean combination
        // can request a shader_source that v0.100b never shipped.
        write_generated_config(
            overlay,
            &format!("material_instances/{}.material", plan.target_material),
            &material,
            &format!(
                "generated:target-schema-STK2-material:{}",
                plan.source_material.as_deref().unwrap_or("legacy-template")
            ),
            files,
        )?;
    }
    Ok(())
}

fn collect_files_with_extension(
    directory: &Path,
    extension: &str,
    output: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("reading directory {}", directory.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, output)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            output.push(path);
        }
    }
    Ok(())
}

fn lua_assignment_string(text: &str, name: &str) -> Option<String> {
    let marker = format!("{name} = \"");
    let tail = text.split_once(&marker)?.1;
    Some(tail.split_once('"')?.0.replace('\\', "/"))
}

fn lua_property_block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name} =");
    let start = text.find(&marker)?;
    let tail = &text[start..];
    let end_marker = format!("-- {name}");
    let end = tail.find(&end_marker)?;
    Some(&tail[..end])
}

fn lua_string_property(text: &str, name: &str) -> Option<String> {
    let block = lua_property_block(text, name)?;
    lua_assignment_string(block, "value")
}

fn lua_bool_property(text: &str, name: &str) -> Option<bool> {
    let block = lua_property_block(text, name)?;
    let marker = "value =";
    let tail = block.split_once(marker)?.1.trim_start();
    if tail.starts_with("true") {
        Some(true)
    } else if tail.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn normalize_stk2_resource_name(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug)]
enum Stk2DiffuseFormat {
    Dxt1,
    Dxt1Alpha,
    Dxt3,
    Dxt5,
    NormalMap,
}

impl Stk2DiffuseFormat {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "DXT1" => Ok(Self::Dxt1),
            "DXT1a" => Ok(Self::Dxt1Alpha),
            "DXT3" => Ok(Self::Dxt3),
            "DXT5" => Ok(Self::Dxt5),
            "NORMAL_MAP" => Ok(Self::NormalMap),
            _ => bail!("unsupported diffuse texture format {value:?}"),
        }
    }

    fn imagemagick_name(self) -> &'static str {
        match self {
            Self::Dxt1 => "dxt1",
            // ImageMagick's DDS writer emits four-color BC1 and discards the
            // one-bit alpha mask even when the source TGA has alpha. DXT5 is
            // a valid v0.100b alpha-test input and preserves the cutout.
            Self::Dxt1Alpha => "dxt5",
            // ImageMagick accepts dxt3 but writes DXT5. Make that supported
            // alpha-preserving transcode explicit in payload and options.
            Self::Dxt3 | Self::Dxt5 | Self::NormalMap => "dxt5",
        }
    }

    fn fourcc(self) -> &'static [u8; 4] {
        match self {
            Self::Dxt1 => b"DXT1",
            Self::Dxt1Alpha | Self::Dxt3 | Self::Dxt5 | Self::NormalMap => b"DXT5",
        }
    }

    fn target_options_name(self) -> &'static str {
        match self {
            Self::Dxt1 => "DXT1",
            Self::Dxt1Alpha | Self::Dxt3 | Self::Dxt5 => "DXT5",
            Self::NormalMap => "NORMAL_MAP",
        }
    }

    fn block_bytes(self) -> usize {
        match self {
            Self::Dxt1 => 8,
            Self::Dxt1Alpha | Self::Dxt3 | Self::Dxt5 | Self::NormalMap => 16,
        }
    }
}

fn convert_stk2_diffuse_texture(
    source: &Path,
    output: &Path,
    format: Stk2DiffuseFormat,
    srgb: u8,
) -> Result<()> {
    let mut command = Command::new("magick");
    command.arg(source);
    if matches!(format, Stk2DiffuseFormat::NormalMap) {
        // Vostok's A2D5 convention stores tangent-space X in alpha and Y in
        // green. Preserve the source TGA's red channel in alpha before DXT5
        // compression; the runtime reconstructs Z.
        command
            .arg("-alpha")
            .arg("on")
            .arg("-channel")
            .arg("A")
            .arg("-fx")
            .arg("r")
            .arg("-channel")
            .arg("R")
            .arg("-evaluate")
            .arg("set")
            .arg("100%")
            .arg("+channel");
    }
    let result = command
        .arg("-define")
        .arg(format!("dds:compression={}", format.imagemagick_name()))
        .arg(output)
        .output()
        .with_context(|| "starting ImageMagick from the repository Nix shell")?;
    if !result.status.success() {
        bail!(
            "ImageMagick failed with {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    let mut dds = fs::read(output)?;
    validate_imagemagick_stk2_diffuse_dds(&dds, format, output)?;
    canonicalize_v0100b_bc_dds(&mut dds, format, output)?;
    validate_stk2_diffuse_dds(&dds, format, output)?;
    if matches!(format, Stk2DiffuseFormat::NormalMap) {
        dds.get_mut(88..92)
            .context("normal-map DDS is too short for the A2D5 marker")?
            .copy_from_slice(b"A2D5");
    }
    dds.push(srgb);
    fs::write(output, dds)?;
    Ok(())
}

fn validate_imagemagick_stk2_diffuse_dds(
    bytes: &[u8],
    format: Stk2DiffuseFormat,
    label: &Path,
) -> Result<()> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || read_u32(bytes, 4)? != 124 {
        bail!("{}: invalid DDS header", label.display());
    }
    if bytes.get(84..88) != Some(format.fourcc()) {
        bail!(
            "{}: expected {} DDS FourCC",
            label.display(),
            String::from_utf8_lossy(format.fourcc())
        );
    }
    if read_u32(bytes, 112)? != 0 {
        bail!(
            "{}: generated DDS is unexpectedly a cubemap",
            label.display()
        );
    }
    let mut width = usize::try_from(read_u32(bytes, 16)?)?;
    let mut height = usize::try_from(read_u32(bytes, 12)?)?;
    let mip_count = usize::try_from(read_u32(bytes, 28)?.max(1))?;
    if width == 0 || height == 0 || mip_count < 2 {
        bail!(
            "{}: invalid generated DDS dimensions/mips {width}x{height}/{mip_count}",
            label.display()
        );
    }
    let mut payload = 0_usize;
    for _ in 0..mip_count {
        let blocks_wide = width.div_ceil(4).max(1);
        let blocks_high = height.div_ceil(4).max(1);
        payload = payload
            .checked_add(blocks_wide * blocks_high * format.block_bytes())
            .context("generated DDS payload size overflow")?;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    if bytes.len() != 128 + payload {
        bail!(
            "{}: generated DDS has {} bytes, expected {} before the Vostok sRGB byte",
            label.display(),
            bytes.len(),
            128 + payload
        );
    }
    Ok(())
}

fn target_v0100b_bc_mip_count(width: usize, height: usize) -> usize {
    let mut width = width;
    let mut height = height;
    let mut count = 1_usize;
    // Stock v0.100b block-compressed textures stop at the last mip whose
    // dimensions are both at least one complete 4x4 BC block. In particular,
    // 1024x1024 has 9 levels (down to 4x4), not the 11 levels emitted by
    // ImageMagick (which also includes 2x2 and 1x1 blocks).
    while width > 4 && height > 4 {
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        count += 1;
    }
    count
}

fn bc_mip_payload_size(
    mut width: usize,
    mut height: usize,
    mip_count: usize,
    block_bytes: usize,
) -> Result<usize> {
    let mut payload = 0_usize;
    for _ in 0..mip_count {
        let blocks_wide = width.div_ceil(4).max(1);
        let blocks_high = height.div_ceil(4).max(1);
        payload = payload
            .checked_add(
                blocks_wide
                    .checked_mul(blocks_high)
                    .and_then(|blocks| blocks.checked_mul(block_bytes))
                    .context("generated DDS mip size overflow")?,
            )
            .context("generated DDS payload size overflow")?;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    Ok(payload)
}

fn write_dds_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<()> {
    bytes
        .get_mut(offset..offset + 4)
        .context("generated DDS header is truncated")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn canonicalize_v0100b_bc_dds(
    bytes: &mut Vec<u8>,
    format: Stk2DiffuseFormat,
    label: &Path,
) -> Result<()> {
    let width = usize::try_from(read_u32(bytes, 16)?)?;
    let height = usize::try_from(read_u32(bytes, 12)?)?;
    let target_mip_count = target_v0100b_bc_mip_count(width, height);
    let payload = bc_mip_payload_size(width, height, target_mip_count, format.block_bytes())?;
    let target_len = 128_usize
        .checked_add(payload)
        .context("target v0.100b DDS size overflow")?;
    if bytes.len() < target_len {
        bail!(
            "{}: ImageMagick DDS is too short for the v0.100b mip chain",
            label.display()
        );
    }
    bytes.truncate(target_len);

    // Match the native header contract byte-for-byte: DDSD_LINEARSIZE is set,
    // dwPitchOrLinearSize describes the complete retained payload plus
    // Vostok's trailing sRGB byte, and the reserved ImageMagick producer tag
    // is cleared.
    let header_flags = read_u32(bytes, 8)? | 0x8;
    write_dds_u32(bytes, 8, header_flags)?;
    write_dds_u32(
        bytes,
        20,
        u32::try_from(
            payload
                .checked_add(1)
                .context("target Vostok DDS size overflow")?,
        )?,
    )?;
    write_dds_u32(bytes, 28, u32::try_from(target_mip_count)?)?;
    bytes
        .get_mut(32..76)
        .context("generated DDS reserved header is truncated")?
        .fill(0);
    Ok(())
}

fn validate_stk2_diffuse_dds(bytes: &[u8], format: Stk2DiffuseFormat, label: &Path) -> Result<()> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || read_u32(bytes, 4)? != 124 {
        bail!("{}: invalid target DDS header", label.display());
    }
    if bytes.get(84..88) != Some(format.fourcc()) {
        bail!(
            "{}: expected {} target DDS FourCC",
            label.display(),
            String::from_utf8_lossy(format.fourcc())
        );
    }
    if read_u32(bytes, 112)? != 0 {
        bail!("{}: target DDS is unexpectedly a cubemap", label.display());
    }
    let width = usize::try_from(read_u32(bytes, 16)?)?;
    let height = usize::try_from(read_u32(bytes, 12)?)?;
    let mip_count = usize::try_from(read_u32(bytes, 28)?.max(1))?;
    let expected_mip_count = target_v0100b_bc_mip_count(width, height);
    if mip_count != expected_mip_count {
        bail!(
            "{}: target DDS has {mip_count} mips, expected {expected_mip_count} for {width}x{height}",
            label.display()
        );
    }
    let payload = bc_mip_payload_size(width, height, mip_count, format.block_bytes())?;
    if read_u32(bytes, 8)? & 0x8 == 0 || usize::try_from(read_u32(bytes, 20)?)? != payload + 1 {
        bail!(
            "{}: target DDS has a non-native linear-size header",
            label.display()
        );
    }
    if bytes
        .get(32..76)
        .is_none_or(|reserved| reserved.iter().any(|byte| *byte != 0))
    {
        bail!(
            "{}: target DDS retains non-native reserved header data",
            label.display()
        );
    }
    if bytes.len() != 128 + payload {
        bail!(
            "{}: target DDS has {} bytes, expected {} before the Vostok sRGB byte",
            label.display(),
            bytes.len(),
            128 + payload
        );
    }
    if matches!(format, Stk2DiffuseFormat::Dxt1Alpha) && !dxt5_has_nonopaque_alpha(&bytes[128..]) {
        bail!(
            "{}: DXT1a source transcode lost its alpha cutout",
            label.display()
        );
    }
    Ok(())
}

fn dxt5_has_nonopaque_alpha(payload: &[u8]) -> bool {
    payload.chunks_exact(16).any(|block| {
        let alpha0 = block[0];
        let alpha1 = block[1];
        let mut palette = [0_u8; 8];
        palette[0] = alpha0;
        palette[1] = alpha1;
        if alpha0 > alpha1 {
            for index in 1..7 {
                palette[index + 1] = u8::try_from(
                    ((7 - index) * usize::from(alpha0) + index * usize::from(alpha1)) / 7,
                )
                .unwrap();
            }
        } else {
            for index in 1..5 {
                palette[index + 1] = u8::try_from(
                    ((5 - index) * usize::from(alpha0) + index * usize::from(alpha1)) / 5,
                )
                .unwrap();
            }
            palette[6] = 0;
            palette[7] = 255;
        }
        let indices = u64::from_le_bytes([
            block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
        ]);
        (0..16).any(|pixel| {
            let index = usize::try_from((indices >> (pixel * 3)) & 7).unwrap();
            palette[index] < 255
        })
    })
}

fn source_option_string(text: &str, name: &str) -> Result<String> {
    let prefix = format!("{name} = \"");
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix("\","))
        })
        .map(ToOwned::to_owned)
        .with_context(|| format!("source texture options lack {name}"))
}

fn source_option_integer(text: &str, name: &str) -> Result<i32> {
    let prefix = format!("{name} = ");
    text.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(','))
        })
        .with_context(|| format!("source texture options lack {name}"))?
        .parse()
        .with_context(|| format!("source texture option {name} is not an integer"))
}

fn build_weapon(
    manifest: &Stk2ContentManifest,
    weapon: &VisualWeaponSpec,
    source_root: &Path,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut config = parse_config(target, &weapon.template_config, "target weapon template")?;
    apply_patches(&mut config.root, &weapon.patch)?;
    replace_strings(&mut config.root, &weapon.replace_strings);
    write_generated_config(
        overlay,
        &weapon.target_config,
        &config,
        "generated:weapon-config",
        files,
    )?;

    let source_archive = safe_join(source_root, &weapon.donor_model_archive)?;
    let mut donor = ResourceDb::open(&source_archive)?;
    validate_surface_set(&donor, &weapon.surfaces, &source_archive)?;
    let target_root = weapon.target_model.trim_end_matches('/');
    let template = &manifest.weapon_model;

    for (source_path, target_path) in [
        (
            &template.bind_pose_template,
            format!("{target_root}/render/bind_pose"),
        ),
        (
            &template.skeleton_template,
            format!("{target_root}/skeleton"),
        ),
    ] {
        let bytes = target.read(source_path)?;
        write_raw_resource(
            overlay,
            &target_path,
            &bytes,
            &format!("target-template:{source_path}"),
            files,
        )?;
    }

    let mut render_properties = parse_config(
        target,
        &template.render_properties_template,
        "weapon render-properties template",
    )?;
    render_properties.root.set_path(
        "src_filename",
        ConfigValue::String(format!("STK2/{}", weapon.donor_model_archive)),
    )?;
    write_generated_config(
        overlay,
        &format!("{target_root}/render/export_properties"),
        &render_properties,
        "generated:weapon-render-properties",
        files,
    )?;

    let settings_template = parse_config(
        target,
        &template.settings_template,
        "weapon settings template",
    )?;
    let settings = replace_settings_surfaces(settings_template, &weapon.surfaces, false)?;
    write_generated_config(
        overlay,
        &format!("{target_root}/settings"),
        &settings,
        "generated:weapon-settings",
        files,
    )?;

    for (surface, material) in &weapon.surfaces {
        let vertices = donor.read(&format!("render/{surface}/vertices"))?;
        let indices = donor.read(&format!("render/{surface}/indices"))?;
        validate_static_geometry(&vertices, &indices, surface)?;
        let converted = convert_static_to_skinned(&vertices, &indices, template.rigid_bone_index)?;
        write_raw_resource(
            overlay,
            &format!("{target_root}/render/{surface}/converted_model"),
            &converted,
            "converted:STK2-static-to-rigid-skinned",
            files,
        )?;
        let bounds = geometry_bounds(&vertices)?;
        let properties = make_surface_properties(
            parse_config(
                target,
                &template.surface_properties_template,
                "weapon surface-properties template",
            )?,
            surface,
            material,
            read_u32(&vertices, 0)?,
            read_u32(&indices, 0)?,
            bounds,
            None,
        )?;
        write_generated_config(
            overlay,
            &format!("{target_root}/render/{surface}/export_properties"),
            &properties,
            "generated:weapon-surface-properties",
            files,
        )?;
    }
    Ok(())
}

pub(crate) fn build_static_model(
    template: &crate::manifest::StaticModelTemplateSpec,
    model: &StaticModelPortSpec,
    collision_game_material: Option<(&str, i32)>,
    source_root: &Path,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let source_archive = safe_join(source_root, &model.donor_model_archive)?;
    let mut donor = ResourceDb::open(&source_archive)?;
    validate_surface_set(&donor, &model.surfaces, &source_archive)?;
    let target_root = model.target_model.trim_end_matches('/');
    let mut render_properties = parse_config(
        target,
        &template.render_properties_template,
        "static render-properties template",
    )?;
    render_properties.root.set_path(
        "src_filename",
        ConfigValue::String(format!("STK2/{}", model.donor_model_archive)),
    )?;
    write_generated_config(
        overlay,
        &format!("{target_root}/render/export_properties"),
        &render_properties,
        "generated:static-render-properties",
        files,
    )?;

    let settings_template = parse_config(
        target,
        &template.settings_template,
        "static settings template",
    )?;
    let mut settings = replace_settings_surfaces(settings_template, &model.surfaces, true)?;
    if let Some((name, id)) = collision_game_material {
        set_collision_game_material(&mut settings, name, id)?;
    }
    write_generated_config(
        overlay,
        &format!("{target_root}/settings"),
        &settings,
        "generated:static-settings",
        files,
    )?;

    for (surface, material) in &model.surfaces {
        let vertices = donor.read(&format!("render/{surface}/vertices"))?;
        let indices = donor.read(&format!("render/{surface}/indices"))?;
        validate_static_geometry(&vertices, &indices, surface)?;
        let converted = convert_static_surface(&vertices, &indices)?;
        write_raw_resource(
            overlay,
            &format!("{target_root}/render/{surface}/converted_model"),
            &converted,
            "converted:STK2-static",
            files,
        )?;
        let bounds = geometry_bounds(&vertices)?;
        let properties = make_surface_properties(
            parse_config(
                target,
                &template.surface_properties_template,
                "static surface-properties template",
            )?,
            surface,
            material,
            read_u32(&vertices, 0)?,
            read_u32(&indices, 0)?,
            bounds,
            Some(0),
        )?;
        write_generated_config(
            overlay,
            &format!("{target_root}/render/{surface}/export_properties"),
            &properties,
            "generated:static-surface-properties",
            files,
        )?;
    }
    Ok(())
}

pub(crate) fn build_static_model_collision(
    model: &StaticModelPortSpec,
    source_root: &Path,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let source_archive = safe_join(source_root, &model.donor_model_archive)?;
    let mut donor = ResourceDb::open(&source_archive)?;
    let vertices = donor
        .read("collision/vertices")
        .with_context(|| format!("{} lacks low collision vertices", source_archive.display()))?;
    let indices = donor
        .read("collision/indices")
        .with_context(|| format!("{} lacks low collision indices", source_archive.display()))?;
    let triangle_count = validate_stk2_collision(&vertices, &indices, &model.target_model)?;
    let target_root = model.target_model.trim_end_matches('/');
    let settings_path = safe_join(overlay, &format!("{target_root}/settings"))?;
    let settings = BinaryConfig::parse(&fs::read(&settings_path)?)
        .with_context(|| format!("parsing generated settings {}", settings_path.display()))?;
    let collision_material = settings
        .root
        .named("game_material_settings")
        .and_then(|settings| settings.named("collisionSG"))
        .with_context(|| {
            format!(
                "{}: generated model settings lack game_material_settings.collisionSG",
                model.target_model
            )
        })?;
    let collision_material_id = match collision_material.named("game_material_id") {
        Some(ConfigValue::Integer(id)) => *id,
        _ => bail!(
            "{}: collisionSG has no integer game_material_id",
            model.target_model
        ),
    };
    let collision_material_name = match collision_material.named("game_material_name") {
        Some(ConfigValue::String(name)) => name,
        _ => bail!(
            "{}: collisionSG has no string game_material_name",
            model.target_model
        ),
    };
    if collision_material_id < 0
        || collision_material_id == 65_535
        || collision_material_name.is_empty()
        || collision_material_name == "not assigned or deleted"
    {
        bail!(
            "{}: collisionSG retains invalid game material {:?}/{}",
            model.target_model,
            collision_material_name,
            collision_material_id
        );
    }
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/vertices"),
        &vertices,
        "identity-proof:STK2-collision-float3-chunk-0x19",
        files,
    )?;
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/indices"),
        &indices,
        "identity-proof:STK2-collision-u32-chunk-0x1a",
        files,
    )?;
    let face_data = make_stk2_collision_face_data(triangle_count)?;
    write_raw_resource(
        overlay,
        &format!("{target_root}/collision/face_data"),
        &face_data,
        "generated:target-collision-face-data:collisionSG",
        files,
    )?;
    Ok(())
}

fn validate_stk2_collision(vertices: &[u8], indices: &[u8], label: &str) -> Result<usize> {
    let vertex_payload = validate_chunk(vertices, 0x19, 12, label)?;
    let index_payload = validate_chunk(indices, 0x1a, 4, label)?;
    let vertex_count = vertex_payload.len() / 12;
    let index_count = index_payload.len() / 4;
    if vertex_count == 0 || index_count == 0 || index_count % 3 != 0 {
        bail!("{label}: invalid collision vertex/index counts {vertex_count}/{index_count}");
    }
    for vertex_index in 0..vertex_count {
        let position = read_float3(vertex_payload, vertex_index * 12)?;
        if !position.into_iter().all(f32::is_finite) {
            bail!("{label}: collision vertex {vertex_index} is non-finite");
        }
    }
    for index_position in 0..index_count {
        let index = usize::try_from(read_u32(index_payload, index_position * 4)?)?;
        if index >= vertex_count {
            bail!(
                "{label}: collision index {index_position} references {index}, but only {vertex_count} vertices exist"
            );
        }
    }
    Ok(index_count / 3)
}

fn validate_chunk<'a>(
    bytes: &'a [u8],
    expected_id: u32,
    stride: usize,
    label: &str,
) -> Result<&'a [u8]> {
    let id = read_u32(bytes, 0)?;
    let size = usize::try_from(read_u32(bytes, 4)?)?;
    if id != expected_id || bytes.len() != 8 + size || size % stride != 0 {
        bail!(
            "{label}: malformed collision chunk id={id:#x}, size={size}, bytes={}, stride={stride}",
            bytes.len()
        );
    }
    Ok(&bytes[8..])
}

fn make_stk2_collision_face_data(triangle_count: usize) -> Result<Vec<u8>> {
    let dictionary = b"collisionSG\0";
    let dictionary_size = 2_usize
        .checked_add(dictionary.len())
        .context("collision material dictionary size overflow")?;
    let face_bytes = triangle_count
        .checked_mul(2)
        .context("collision face-data size overflow")?;
    let mut output = Vec::with_capacity(16 + dictionary_size + face_bytes);
    push_u32(&mut output, 0x1b);
    push_u32(&mut output, u32::try_from(dictionary_size)?);
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(dictionary);
    push_u32(&mut output, 0x1c);
    push_u32(&mut output, u32::try_from(face_bytes)?);
    output.resize(output.len() + face_bytes, 0);
    Ok(output)
}

pub(crate) fn discover_static_model_surface_textures(
    archive: &Path,
) -> Result<BTreeMap<String, Stk2SourceTextureReference>> {
    let mut donor = ResourceDb::open(archive)
        .with_context(|| format!("opening STK2 exported model {}", archive.display()))?;
    let surfaces: BTreeSet<String> = donor
        .paths_with_prefix("render/")
        .filter_map(|path| {
            path.strip_prefix("render/")
                .and_then(|path| path.strip_suffix("/vertices"))
                .map(ToOwned::to_owned)
        })
        .collect();
    if surfaces.is_empty() {
        bail!("{} has no static render surfaces", archive.display());
    }
    let mut surface_textures = BTreeMap::new();
    for surface in &surfaces {
        let vertices = donor.read(&format!("render/{surface}/vertices"))?;
        let indices = donor.read(&format!("render/{surface}/indices"))?;
        validate_static_geometry(&vertices, &indices, surface)?;
        let properties = donor.read(&format!("render/{surface}/export_properties"))?;
        let texture = parse_stk2_source_texture_name(&properties, surface)?;
        surface_textures.insert(surface.clone(), texture);
    }
    Ok(surface_textures)
}

#[derive(Clone, Debug)]
pub(crate) struct Stk2SourceTextureReference {
    pub raw: String,
    pub canonical: Option<String>,
}

fn parse_stk2_source_texture_name(
    bytes: &[u8],
    surface: &str,
) -> Result<Stk2SourceTextureReference> {
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("{surface}: source export properties are not UTF-8"))?;
    let mut values = text.lines().filter_map(|line| {
        line.trim()
            .strip_prefix("texture_name = \"")
            .and_then(|value| value.strip_suffix("\","))
    });
    let value = values
        .next()
        .with_context(|| format!("{surface}: source export properties lack texture_name"))?;
    if values.next().is_some() {
        bail!("{surface}: source export properties contain multiple texture_name fields");
    }
    if value.is_empty() {
        return Ok(Stk2SourceTextureReference {
            raw: String::new(),
            canonical: Some(String::new()),
        });
    }
    if value.starts_with('/') || value.ends_with('/') {
        return Ok(Stk2SourceTextureReference {
            raw: value.to_owned(),
            canonical: None,
        });
    }
    let components = value
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let canonical = components
        .iter()
        .all(|component| {
            *component != "."
                && *component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .then(|| components.join("/"));
    Ok(Stk2SourceTextureReference {
        raw: value.to_owned(),
        canonical,
    })
}

fn validate_surface_set(
    donor: &ResourceDb,
    declared: &BTreeMap<String, String>,
    archive: &Path,
) -> Result<()> {
    let actual: BTreeSet<String> = donor
        .paths_with_prefix("render/")
        .filter_map(|path| {
            path.strip_prefix("render/")
                .and_then(|path| path.strip_suffix("/vertices"))
                .map(ToOwned::to_owned)
        })
        .collect();
    let expected: BTreeSet<String> = declared.keys().cloned().collect();
    if actual != expected {
        bail!(
            "surface declaration differs from {}: archive={actual:?}, manifest={expected:?}",
            archive.display()
        );
    }
    Ok(())
}

fn replace_settings_surfaces(
    mut settings: BinaryConfig,
    surfaces: &BTreeMap<String, String>,
    used_in_portals: bool,
) -> Result<BinaryConfig> {
    let surface_names = surfaces.keys().cloned().map(ConfigValue::String).collect();
    let material_settings = surfaces
        .iter()
        .map(|(surface, material)| {
            named(
                surface,
                ConfigValue::Named(vec![
                    named("material_name", ConfigValue::String(material.clone())),
                    named("used_in_portals", ConfigValue::Boolean(used_in_portals)),
                ]),
            )
        })
        .collect();
    settings.root.set_path(
        "lod_hierrarchy.LOD0.surfaces",
        ConfigValue::Indexed(surface_names),
    )?;
    settings
        .root
        .set_path("material_settings", ConfigValue::Named(material_settings))?;
    Ok(settings)
}

fn make_surface_properties(
    mut properties: BinaryConfig,
    surface: &str,
    material: &str,
    vertex_count: u32,
    index_count: u32,
    bounds: Bounds,
    model_type: Option<i32>,
) -> Result<BinaryConfig> {
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
        ConfigValue::String(format!("|STK2|{surface}")),
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
        .set_path("bounding_box.min", ConfigValue::Float3(bounds.min))?;
    properties
        .root
        .set_path("bounding_box.max", ConfigValue::Float3(bounds.max))?;
    if properties.root.named("bounding_sphere").is_some() {
        properties
            .root
            .set_path("bounding_sphere.center", ConfigValue::Float3(bounds.center))?;
        properties
            .root
            .set_path("bounding_sphere.radius", ConfigValue::Float(bounds.radius))?;
    }
    if let Some(model_type) = model_type {
        properties
            .root
            .set_path("type", ConfigValue::Integer(model_type))?;
    }
    Ok(properties)
}

fn set_collision_game_material(settings: &mut BinaryConfig, name: &str, id: i32) -> Result<()> {
    if name.is_empty() || id < 0 || id == 65_535 {
        bail!("invalid target collision game material {name:?} with id {id}");
    }
    settings.root.set_path(
        "game_material_settings.collisionSG.game_material_id",
        ConfigValue::Integer(id),
    )?;
    settings.root.set_path(
        "game_material_settings.collisionSG.game_material_name",
        ConfigValue::String(name.to_owned()),
    )?;
    Ok(())
}

fn replace_lobby(
    manifest: &Stk2ContentManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut project = parse_config(
        target,
        &manifest.lobby.template_project,
        "target lobby project template",
    )?;
    let template_collision_count = match project.root.named("collision_objects") {
        Some(ConfigValue::Indexed(objects)) if !objects.is_empty() => objects.len(),
        Some(ConfigValue::Indexed(_)) => bail!("target lobby template has no collision objects"),
        Some(_) => bail!("target lobby template collision_objects is not an indexed table"),
        None => bail!("target lobby template has no collision_objects table"),
    };
    debug_assert!(template_collision_count > 0);
    project.root.set_path(
        "render_visuals",
        ConfigValue::Indexed(
            manifest
                .lobby
                .visuals
                .iter()
                .map(lobby_visual_value)
                .collect(),
        ),
    )?;
    if let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("objects") {
        objects.retain(|object| {
            matches!(
                object.named("game_object_type"),
                Some(ConfigValue::String(kind))
                    if matches!(kind.as_str(), "light" | "environment" | "environment_probe" | "sky")
            )
        });
    }
    write_generated_config(
        overlay,
        &manifest.lobby.target_project,
        &project,
        "generated:STK2-lobby-project",
        files,
    )?;

    let lua = config_to_lua(&project.root)?;
    write_raw_resource(
        overlay,
        &manifest.lobby.target_project_lua,
        lua.as_bytes(),
        "generated:STK2-lobby-project-lua",
        files,
    )?;
    Ok(())
}

fn lobby_visual_value(visual: &LobbyVisualSpec) -> ConfigValue {
    ConfigValue::Named(vec![
        named("rotation", ConfigValue::Float3(visual.rotation)),
        named("position", ConfigValue::Float3(visual.position)),
        named("lib_name", ConfigValue::String(visual.library_name.clone())),
        named("scale", ConfigValue::Float3(visual.scale)),
    ])
}

fn validate_static_geometry(vertices: &[u8], indices: &[u8], surface: &str) -> Result<()> {
    let vertex_count = usize::try_from(read_u32(vertices, 0)?)?;
    if vertex_count == 0 {
        bail!("{surface}: static surface has no vertices");
    }
    let expected_vertices = 4_usize
        .checked_add(
            vertex_count
                .checked_mul(56)
                .context("static vertex payload overflow")?,
        )
        .context("static vertex payload overflow")?;
    if vertices.len() != expected_vertices {
        bail!(
            "{surface}: {} static vertex bytes, expected {expected_vertices}",
            vertices.len()
        );
    }
    for vertex_index in 0..vertex_count {
        let source = &vertices[4 + vertex_index * 56..][..56];
        let position = read_float3(source, 0)?;
        let normal = read_float3(source, 12)?;
        let tangent = read_float3(source, 24)?;
        let binormal = read_float3(source, 36)?;
        let uv = [read_f32(source, 48)?, read_f32(source, 52)?];
        if !position
            .into_iter()
            .chain(normal)
            .chain(tangent)
            .chain(binormal)
            .chain(uv)
            .all(f32::is_finite)
        {
            bail!("{surface}: vertex {vertex_index} contains a non-finite component");
        }
    }
    let index_count = usize::try_from(read_u32(indices, 0)?)?;
    let expected_indices = 4_usize
        .checked_add(
            index_count
                .checked_mul(2)
                .context("index payload overflow")?,
        )
        .context("index payload overflow")?;
    if indices.len() != expected_indices || index_count == 0 || index_count % 3 != 0 {
        bail!(
            "{surface}: invalid index payload size/count ({}, {index_count})",
            indices.len()
        );
    }
    for index_position in 0..index_count {
        let offset = 4 + index_position * 2;
        let index = usize::from(read_u16(indices, offset)?);
        if index >= vertex_count {
            bail!(
                "{surface}: index {index_position} references vertex {index}, but only {vertex_count} vertices exist"
            );
        }
    }
    repaired_static_bases(vertices, indices, surface)?;
    Ok(())
}

fn convert_static_surface(vertices: &[u8], indices: &[u8]) -> Result<Vec<u8>> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    let bases = repaired_static_bases(vertices, indices, "converted static surface")?;
    let vertex_chunk_size = STATIC_VERTEX_DECLARATION
        .len()
        .checked_add(4 + count * 32)
        .context("static converted vertex chunk overflow")?;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + indices.len());
    push_u32(&mut output, 3);
    push_u32(&mut output, u32::try_from(vertex_chunk_size)?);
    output.extend_from_slice(&STATIC_VERTEX_DECLARATION);
    push_u32(&mut output, u32::try_from(count)?);
    for index in 0..count {
        let source = &vertices[4 + index * 56..][..56];
        output.extend_from_slice(&source[..12]);
        push_u32(&mut output, packed_basis(bases[index][0], 127));
        push_u32(&mut output, packed_basis(bases[index][1], 255));
        push_u32(&mut output, packed_basis(bases[index][2], 255));
        output.extend_from_slice(&source[48..56]);
    }
    push_u32(&mut output, 4);
    push_u32(&mut output, u32::try_from(indices.len())?);
    output.extend_from_slice(indices);
    Ok(output)
}

fn convert_static_to_skinned(
    vertices: &[u8],
    indices: &[u8],
    rigid_bone_index: u8,
) -> Result<Vec<u8>> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    let bases = repaired_static_bases(vertices, indices, "converted rigid surface")?;
    let vertex_chunk_size = 4_usize
        .checked_add(
            count
                .checked_mul(36)
                .context("skinned vertex chunk overflow")?,
        )
        .context("skinned vertex chunk overflow")?;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + indices.len());
    push_u32(&mut output, 3);
    push_u32(&mut output, u32::try_from(vertex_chunk_size)?);
    push_u32(&mut output, u32::try_from(count)?);
    for index in 0..count {
        let source = &vertices[4 + index * 56..][..56];
        output.extend_from_slice(&source[..12]);
        output.extend_from_slice(&[rigid_bone_index, 0, 0, 0]);
        let uv_x = read_f32(source, 48)?;
        let uv_y = read_f32(source, 52)?;
        push_u32(&mut output, packed_basis(bases[index][0], 127));
        push_u32(
            &mut output,
            packed_basis(bases[index][1], quantized_uv_fraction(uv_x)),
        );
        push_u32(
            &mut output,
            packed_basis(bases[index][2], quantized_uv_fraction(uv_y)),
        );
        output.extend_from_slice(&source[48..56]);
    }
    push_u32(&mut output, 4);
    push_u32(&mut output, u32::try_from(indices.len())?);
    output.extend_from_slice(indices);
    Ok(output)
}

fn repaired_static_bases(
    vertices: &[u8],
    indices: &[u8],
    surface: &str,
) -> Result<Vec<[[f32; 3]; 3]>> {
    let vertex_count = usize::try_from(read_u32(vertices, 0)?)?;
    let index_count = usize::try_from(read_u32(indices, 0)?)?;
    let mut accumulated_normal = vec![[0.0_f32; 3]; vertex_count];
    let mut accumulated_tangent = vec![[0.0_f32; 3]; vertex_count];
    let mut accumulated_binormal = vec![[0.0_f32; 3]; vertex_count];
    let mut strongest_normal = vec![[0.0_f32; 3]; vertex_count];
    let mut strongest_tangent = vec![[0.0_f32; 3]; vertex_count];
    let mut strongest_binormal = vec![[0.0_f32; 3]; vertex_count];

    for triangle in 0..index_count / 3 {
        let mut vertex_indices = [0_usize; 3];
        let mut positions = [[0.0_f32; 3]; 3];
        let mut uvs = [[0.0_f32; 2]; 3];
        for corner in 0..3 {
            let vertex_index = usize::from(read_u16(indices, 4 + (triangle * 3 + corner) * 2)?);
            if vertex_index >= vertex_count {
                bail!(
                    "{surface}: triangle {triangle} references vertex {vertex_index}, but only {vertex_count} vertices exist"
                );
            }
            vertex_indices[corner] = vertex_index;
            let source = &vertices[4 + vertex_index * 56..][..56];
            positions[corner] = read_float3(source, 0)?;
            uvs[corner] = [read_f32(source, 48)?, read_f32(source, 52)?];
        }
        let edge1 = sub3(positions[1], positions[0]);
        let edge2 = sub3(positions[2], positions[0]);
        let face_normal = cross3(edge1, edge2);
        if normalize3(face_normal).is_some() {
            for vertex_index in vertex_indices {
                add_assign3(&mut accumulated_normal[vertex_index], face_normal);
                replace_if_longer(&mut strongest_normal[vertex_index], face_normal);
            }
        }

        let uv1 = [uvs[1][0] - uvs[0][0], uvs[1][1] - uvs[0][1]];
        let uv2 = [uvs[2][0] - uvs[0][0], uvs[2][1] - uvs[0][1]];
        let determinant = uv1[0] * uv2[1] - uv1[1] * uv2[0];
        if determinant.is_finite() && determinant.abs() > 1.0e-12 {
            let reciprocal = determinant.recip();
            let tangent = scale3(
                sub3(scale3(edge1, uv2[1]), scale3(edge2, uv1[1])),
                reciprocal,
            );
            let binormal = scale3(
                sub3(scale3(edge2, uv1[0]), scale3(edge1, uv2[0])),
                reciprocal,
            );
            if normalize3(tangent).is_some() && normalize3(binormal).is_some() {
                for vertex_index in vertex_indices {
                    add_assign3(&mut accumulated_tangent[vertex_index], tangent);
                    add_assign3(&mut accumulated_binormal[vertex_index], binormal);
                    replace_if_longer(&mut strongest_tangent[vertex_index], tangent);
                    replace_if_longer(&mut strongest_binormal[vertex_index], binormal);
                }
            }
        }
    }

    let mut position_bases = BTreeMap::<[u32; 3], [[f32; 3]; 3]>::new();
    for vertex_index in 0..vertex_count {
        let source = &vertices[4 + vertex_index * 56..][..56];
        let position = read_float3(source, 0)?;
        let source_normal = read_float3(source, 12)?;
        let source_tangent = read_float3(source, 24)?;
        let source_binormal = read_float3(source, 36)?;
        let entry = position_bases
            .entry([
                position[0].to_bits(),
                position[1].to_bits(),
                position[2].to_bits(),
            ])
            .or_insert([[0.0; 3]; 3]);
        fill_if_degenerate(
            &mut entry[0],
            [
                source_normal,
                cross3(source_tangent, source_binormal),
                accumulated_normal[vertex_index],
                strongest_normal[vertex_index],
            ],
        );
        fill_if_degenerate(
            &mut entry[1],
            [
                source_tangent,
                accumulated_tangent[vertex_index],
                strongest_tangent[vertex_index],
                [0.0; 3],
            ],
        );
        fill_if_degenerate(
            &mut entry[2],
            [
                source_binormal,
                accumulated_binormal[vertex_index],
                strongest_binormal[vertex_index],
                [0.0; 3],
            ],
        );
    }

    let mut result = Vec::with_capacity(vertex_count);
    for vertex_index in 0..vertex_count {
        let source = &vertices[4 + vertex_index * 56..][..56];
        let position = read_float3(source, 0)?;
        let position_basis = position_bases
            .get(&[
                position[0].to_bits(),
                position[1].to_bits(),
                position[2].to_bits(),
            ])
            .expect("position basis was populated for every vertex");
        let source_normal = read_float3(source, 12)?;
        let source_tangent = read_float3(source, 24)?;
        let source_binormal = read_float3(source, 36)?;
        let normal = normalize3(source_normal)
            .or_else(|| normalize3(cross3(source_tangent, source_binormal)))
            .or_else(|| normalize3(accumulated_normal[vertex_index]))
            .or_else(|| normalize3(strongest_normal[vertex_index]))
            .or_else(|| normalize3(position_basis[0]))
            .with_context(|| {
                format!("{surface}: vertex {vertex_index} has no repairable normal")
            })?;
        let tangent_source = normalize3(source_tangent)
            .or_else(|| normalize3(accumulated_tangent[vertex_index]))
            .or_else(|| normalize3(strongest_tangent[vertex_index]))
            .or_else(|| normalize3(position_basis[1]))
            .unwrap_or_else(|| deterministic_tangent(normal));
        let tangent = normalize3(sub3(
            tangent_source,
            scale3(normal, dot3(tangent_source, normal)),
        ))
        .unwrap_or_else(|| deterministic_tangent(normal));
        // Very nearly parallel source vectors can survive the first
        // normalization because of float cancellation, yet still yield a
        // numerically zero cross product. In that case the source contains no
        // meaningful tangent direction, so use the same deterministic
        // orthogonal basis as a directly degenerate tangent.
        let (tangent, canonical_binormal) =
            if let Some(binormal) = normalize3(cross3(normal, tangent)) {
                (tangent, binormal)
            } else {
                let tangent = deterministic_tangent(normal);
                let binormal = normalize3(cross3(normal, tangent)).with_context(|| {
                    format!("{surface}: vertex {vertex_index} has no repairable binormal")
                })?;
                (tangent, binormal)
            };
        let handed_reference = normalize3(source_binormal)
            .or_else(|| normalize3(accumulated_binormal[vertex_index]))
            .or_else(|| normalize3(strongest_binormal[vertex_index]))
            .or_else(|| normalize3(position_basis[2]));
        let handedness = match handed_reference {
            Some(reference) if dot3(canonical_binormal, reference) < 0.0 => -1.0,
            _ => 1.0,
        };
        result.push([normal, tangent, scale3(canonical_binormal, handedness)]);
    }
    Ok(result)
}

fn add_assign3(target: &mut [f32; 3], value: [f32; 3]) {
    for axis in 0..3 {
        target[axis] += value[axis];
    }
}

fn replace_if_longer(target: &mut [f32; 3], value: [f32; 3]) {
    if dot3(value, value) > dot3(*target, *target) {
        *target = value;
    }
}

fn fill_if_degenerate(target: &mut [f32; 3], candidates: [[f32; 3]; 4]) {
    if normalize3(*target).is_some() {
        return;
    }
    if let Some(candidate) = candidates
        .into_iter()
        .find(|candidate| normalize3(*candidate).is_some())
    {
        *target = candidate;
    }
}

fn sub3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale3(value: [f32; 3], scale: f32) -> [f32; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize3(value: [f32; 3]) -> Option<[f32; 3]> {
    let squared_length = dot3(value, value);
    if !squared_length.is_finite() || squared_length <= 1.0e-20 {
        return None;
    }
    Some(scale3(value, squared_length.sqrt().recip()))
}

fn deterministic_tangent(normal: [f32; 3]) -> [f32; 3] {
    let axis = if normal[0].abs() <= normal[1].abs() && normal[0].abs() <= normal[2].abs() {
        [1.0, 0.0, 0.0]
    } else if normal[1].abs() <= normal[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    normalize3(cross3(axis, normal)).expect("axis selection guarantees a nonparallel tangent")
}

#[derive(Clone, Copy)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
    center: [f32; 3],
    radius: f32,
}

fn geometry_bounds(vertices: &[u8]) -> Result<Bounds> {
    let count = usize::try_from(read_u32(vertices, 0)?)?;
    if count == 0 {
        bail!("cannot calculate bounds for an empty surface");
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut positions = Vec::with_capacity(count);
    for index in 0..count {
        let position = read_float3(vertices, 4 + index * 56)?;
        if !position.into_iter().all(f32::is_finite) {
            bail!("surface vertex {index} has a non-finite position");
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
        positions.push(position);
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let radius = positions
        .iter()
        .map(|position| {
            ((position[0] - center[0]).powi(2)
                + (position[1] - center[1]).powi(2)
                + (position[2] - center[2]).powi(2))
            .sqrt()
        })
        .fold(0.0_f32, f32::max);
    Ok(Bounds {
        min,
        max,
        center,
        radius,
    })
}

fn config_to_lua(root: &ConfigValue) -> Result<String> {
    let ConfigValue::Named(values) = root else {
        bail!("lobby project root is not a named table");
    };
    let mut output = String::new();
    for value in values {
        write!(&mut output, "{} = ", lua_key(&value.name))?;
        write_lua_value(&mut output, &value.value, 0)?;
        output.push_str("\n\n");
    }
    Ok(output)
}

fn write_lua_value(output: &mut String, value: &ConfigValue, depth: usize) -> Result<()> {
    match value {
        ConfigValue::Boolean(value) => output.push_str(if *value { "true" } else { "false" }),
        ConfigValue::Integer(value) => write!(output, "{value}")?,
        ConfigValue::Float(value) => write!(output, "{value:?}")?,
        ConfigValue::String(value) => write!(output, "{:?}", value)?,
        ConfigValue::Float2(value) => write!(output, "float2( {:?}, {:?} )", value[0], value[1])?,
        ConfigValue::Float3(value) => write!(
            output,
            "float3( {:?}, {:?}, {:?} )",
            value[0], value[1], value[2]
        )?,
        ConfigValue::Float4(value) => write!(
            output,
            "float4( {:?}, {:?}, {:?}, {:?} )",
            value[0], value[1], value[2], value[3]
        )?,
        ConfigValue::Named(values) => {
            output.push_str("{\n");
            for value in values {
                write_indent(output, depth + 1);
                write!(output, "{} = ", lua_key(&value.name))?;
                write_lua_value(output, &value.value, depth + 1)?;
                output.push_str(",\n");
            }
            write_indent(output, depth);
            output.push('}');
        }
        ConfigValue::Indexed(values) => {
            output.push_str("{\n");
            for value in values {
                write_indent(output, depth + 1);
                write_lua_value(output, value, depth + 1)?;
                output.push_str(",\n");
            }
            write_indent(output, depth);
            output.push('}');
        }
    }
    Ok(())
}

fn lua_key(key: &str) -> String {
    let valid = !key.is_empty()
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        });
    if valid {
        key.to_owned()
    } else {
        format!("[{key:?}]")
    }
}

fn write_indent(output: &mut String, depth: usize) {
    for _ in 0..depth {
        output.push('\t');
    }
}

fn write_raw_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes)?;
    files.push(record_file(path, source, bytes));
    Ok(())
}

fn named(name: impl Into<String>, value: ConfigValue) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}

fn packed_basis(mut vector: [f32; 3], alpha: u8) -> u32 {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > f32::EPSILON {
        for component in &mut vector {
            *component /= length;
        }
    }
    let component = |value: f32| (((value + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
    ((alpha as u32) << 24)
        | (component(vector[0]) << 16)
        | (component(vector[1]) << 8)
        | component(vector[2])
}

fn quantized_uv_fraction(value: f32) -> u8 {
    let rebased = value * 1024.0;
    let primary = rebased.floor().clamp(-32768.0, 32767.0);
    (255.5 * (rebased - primary)).floor().clamp(0.0, 255.0) as u8
}

fn read_float3(bytes: &[u8], offset: usize) -> Result<[f32; 3]> {
    Ok([
        read_f32(bytes, offset)?,
        read_f32(bytes, offset + 4)?,
        read_f32(bytes, offset + 8)?,
    ])
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let bytes = bytes
        .get(offset..offset + 4)
        .with_context(|| format!("missing u32 at byte {offset}"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let bytes = bytes
        .get(offset..offset + 2)
        .with_context(|| format!("missing u16 at byte {offset}"))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(output: &mut Vec<u8>, value: f32) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_old_static_vertex_declaration_and_chunks() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        for value in [
            1.0_f32, 2.0, 3.0, // position
            0.0, 1.0, 0.0, // normal
            1.0, 0.0, 0.0, // tangent
            0.0, 0.0, 1.0, // binormal
            0.25, 0.5, // uv
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        indices.extend_from_slice(&[0; 6]);

        let converted = convert_static_surface(&vertices, &indices).unwrap();
        assert_eq!(read_u32(&converted, 0).unwrap(), 3);
        assert_eq!(read_u32(&converted, 4).unwrap(), 84);
        assert_eq!(&converted[8..56], &STATIC_VERTEX_DECLARATION);
        assert_eq!(read_u32(&converted, 56).unwrap(), 1);
        assert_eq!(read_u32(&converted, 92).unwrap(), 4);
        assert_eq!(read_u32(&converted, 96).unwrap(), 10);
    }

    #[test]
    fn makes_rigid_weapon_vertices_use_requested_bone() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        for value in [
            0.0_f32, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        indices.extend_from_slice(&[0; 6]);
        let converted = convert_static_to_skinned(&vertices, &indices, 1).unwrap();
        assert_eq!(read_u32(&converted, 4).unwrap(), 40);
        assert_eq!(&converted[24..28], &[1, 0, 0, 0]);
        assert_eq!(read_u32(&converted, 48).unwrap(), 4);
        assert_eq!(read_u32(&converted, 52).unwrap(), 10);
    }

    #[test]
    fn rejects_out_of_range_static_indices() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        for value in [
            0.0_f32, 0.0, 0.0, // position
            0.0, 1.0, 0.0, // normal
            1.0, 0.0, 0.0, // tangent
            0.0, 0.0, 1.0, // binormal
            0.0, 0.0, // uv
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        indices.extend_from_slice(&0_u16.to_le_bytes());
        indices.extend_from_slice(&1_u16.to_le_bytes());
        indices.extend_from_slice(&0_u16.to_le_bytes());
        let error = validate_static_geometry(&vertices, &indices, "bad").unwrap_err();
        assert!(error.to_string().contains("references vertex 1"));
    }

    #[test]
    fn rejects_unrepairable_static_basis() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        for value in [
            0.0_f32, 0.0, 0.0, // position
            0.0, 0.0, 0.0, // normal
            0.0, 0.0, 0.0, // tangent
            0.0, 0.0, 0.0, // binormal
            0.0, 0.0, // uv
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        indices.extend_from_slice(&[0; 6]);
        let error = validate_static_geometry(&vertices, &indices, "bad").unwrap_err();
        assert!(error.to_string().contains("no repairable normal"));
    }

    #[test]
    fn repairs_normal_from_tangent_and_binormal() {
        let mut vertices = Vec::new();
        push_u32(&mut vertices, 1);
        for value in [
            0.0_f32, 0.0, 0.0, // position
            0.0, 0.0, 0.0, // missing normal
            1.0, 0.0, 0.0, // tangent
            0.0, 0.0, 1.0, // binormal
            0.0, 0.0, // uv
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let mut indices = Vec::new();
        push_u32(&mut indices, 3);
        indices.extend_from_slice(&[0; 6]);
        let bases = repaired_static_bases(&vertices, &indices, "repairable").unwrap();
        assert_eq!(bases[0][0], [0.0, -1.0, 0.0]);
    }

    #[test]
    fn reads_source_texture_name_from_lua_properties() {
        let properties = br#"source = {
	texture_name = "tiled/concrete/concrete02_tile01",
} -- source
"#;
        let reference = parse_stk2_source_texture_name(properties, "surface").unwrap();
        assert_eq!(
            reference.canonical.as_deref(),
            Some("tiled/concrete/concrete02_tile01")
        );
        let reference = parse_stk2_source_texture_name(
            b"source = {\n\ttexture_name = \"forge//zabor_\",\n}\n",
            "surface",
        )
        .unwrap();
        assert_eq!(reference.canonical.as_deref(), Some("forge/zabor_"));
        let reference = parse_stk2_source_texture_name(
            b"source = {\n\ttexture_name = \"d:work\\\\broken\",\n}\n",
            "surface",
        )
        .unwrap();
        assert_eq!(reference.canonical, None);
        assert_eq!(reference.raw, r"d:work\\broken");
    }

    #[test]
    fn reads_stk2_material_property_overrides_without_matching_references() {
        let material = r#"
texture_diffuse = create_property_table ( {
    value = "nature/bush/grass_custom",
}, this ), -- texture_diffuse
two_sided = create_reference( "base.two_sided", this ),
use_alpha_test = create_property_table ( {
    value = true,
}, this ), -- use_alpha_test
instance = {
    base_material = "speedtree_base",
}
"#;
        assert_eq!(
            lua_string_property(material, "texture_diffuse").as_deref(),
            Some("nature/bush/grass_custom")
        );
        assert_eq!(lua_bool_property(material, "use_alpha_test"), Some(true));
        assert_eq!(lua_bool_property(material, "two_sided"), None);
        assert_eq!(
            lua_assignment_string(material, "base_material").as_deref(),
            Some("speedtree_base")
        );
    }

    #[test]
    fn resolves_proven_material_family_and_rejects_ambiguous_diffuse() {
        let templates = Stk2MaterialTemplates {
            opaque: "opaque".into(),
            alpha_test: "alpha".into(),
            alpha_test_two_sided: "foliage".into(),
            two_sided: "two-sided".into(),
            decal: "decal".into(),
            metal: "metal".into(),
            metal_alpha_test: "metal-alpha-one-sided".into(),
            metal_alpha_test_two_sided: "metal-alpha".into(),
            metal_two_sided: "metal-two-sided".into(),
        };
        let metal = Stk2SourceMaterialContract {
            source_material: "props/gate".into(),
            base_material: "metal_rusted".into(),
            alpha_test: true,
            two_sided: true,
            metal: true,
            decal: false,
            texture_roles: BTreeMap::new(),
        };
        let mut catalog = Stk2SourceMaterialCatalog::default();
        catalog
            .by_diffuse
            .insert("props/gate".into(), vec![metal.clone()]);
        assert_eq!(
            catalog.resolve("props/gate", &templates).unwrap().template,
            "metal-alpha"
        );
        let mut incompatible = metal;
        incompatible.alpha_test = false;
        catalog
            .by_diffuse
            .get_mut("props/gate")
            .unwrap()
            .push(incompatible);
        assert!(catalog.resolve("props/gate", &templates).is_none());
    }

    #[test]
    fn generates_target_collision_face_dictionary() {
        let face_data = make_stk2_collision_face_data(3).unwrap();
        assert_eq!(read_u32(&face_data, 0).unwrap(), 0x1b);
        assert_eq!(read_u32(&face_data, 4).unwrap(), 14);
        assert_eq!(&face_data[10..22], b"collisionSG\0");
        assert_eq!(read_u32(&face_data, 22).unwrap(), 0x1c);
        assert_eq!(read_u32(&face_data, 26).unwrap(), 6);
        assert_eq!(&face_data[30..], &[0; 6]);
    }

    #[test]
    fn detects_preserved_dxt5_alpha_cutouts() {
        let mut opaque = [0_u8; 16];
        opaque[0] = 255;
        opaque[1] = 255;
        assert!(!dxt5_has_nonopaque_alpha(&opaque));

        let mut cutout = [0_u8; 16];
        cutout[0] = 0;
        cutout[1] = 255;
        assert!(dxt5_has_nonopaque_alpha(&cutout));
    }

    #[test]
    fn canonicalizes_bc_mips_and_header_for_v0100b() {
        assert_eq!(target_v0100b_bc_mip_count(1024, 1024), 9);
        assert_eq!(target_v0100b_bc_mip_count(2048, 512), 8);
        assert_eq!(target_v0100b_bc_mip_count(16, 16), 3);

        // ImageMagick emits the complete 16, 8, 4, 2, 1 chain and writes its
        // producer name into the reserved DDS header fields.
        let source_payload = bc_mip_payload_size(16, 16, 5, 8).unwrap();
        let mut dds = vec![0_u8; 128 + source_payload];
        dds[..4].copy_from_slice(b"DDS ");
        write_dds_u32(&mut dds, 4, 124).unwrap();
        write_dds_u32(&mut dds, 8, 0x000a_1007).unwrap();
        write_dds_u32(&mut dds, 12, 16).unwrap();
        write_dds_u32(&mut dds, 16, 16).unwrap();
        write_dds_u32(&mut dds, 20, 128).unwrap();
        write_dds_u32(&mut dds, 28, 5).unwrap();
        dds[32..43].copy_from_slice(b"IMAGEMAGICK");
        write_dds_u32(&mut dds, 76, 32).unwrap();
        write_dds_u32(&mut dds, 80, 4).unwrap();
        dds[84..88].copy_from_slice(b"DXT1");
        write_dds_u32(&mut dds, 108, 0x0040_1008).unwrap();

        validate_imagemagick_stk2_diffuse_dds(
            &dds,
            Stk2DiffuseFormat::Dxt1,
            Path::new("probe.dds"),
        )
        .unwrap();
        canonicalize_v0100b_bc_dds(&mut dds, Stk2DiffuseFormat::Dxt1, Path::new("probe.dds"))
            .unwrap();
        validate_stk2_diffuse_dds(&dds, Stk2DiffuseFormat::Dxt1, Path::new("probe.dds")).unwrap();

        assert_eq!(read_u32(&dds, 8).unwrap(), 0x000a_100f);
        assert_eq!(read_u32(&dds, 20).unwrap(), 169);
        assert_eq!(read_u32(&dds, 28).unwrap(), 3);
        assert_eq!(dds.len(), 128 + 168);
        assert!(dds[32..76].iter().all(|byte| *byte == 0));
    }
}
