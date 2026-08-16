//! Emit a v0.100b bundle from a compiled Shadow of Chernobyl level.
//!
//! This module owns the only side effects in the SoC path: reading the source
//! tree and target database, writing the overlay, and recording provenance.
//! Every decision it acts on was already made in [`super::convert`], and every
//! target structure it writes is either encoded there or cloned from a template
//! named by the manifest.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::{ResourceDb, safe_join};
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::manifest::BuiltFile;
use crate::weapon::{record_file, reject_duplicate_outputs};

use super::codec::{blender, cform, gamemtl, level, ogf};
use super::convert::collision;
use super::convert::game_material::{self, CollisionMaterialBinding};
use super::convert::render_material::{self, MaterialPlan};
use super::convert::surface::{self, MergeReport, MergedSurface};
use super::manifest::SocMapManifest;
use super::source::{self, DdsInfo, SocSource};

/// What one map build produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SocMapBuildRecord {
    pub asset_id: String,
    pub source_level: String,
    pub project_name: String,
    pub client_project: String,
    pub server_project: String,
    pub models: Vec<String>,
    pub collision_model: String,
    /// Source visuals that produced render geometry.
    pub rendered_surfaces: usize,
    /// Merged target surfaces.
    pub converted_surfaces: usize,
    pub rendered_triangles: usize,
    /// Source visuals placed by their own tree matrix.
    pub tree_surfaces: usize,
    pub textures: usize,
    pub materials: usize,
    pub collision_vertices: usize,
    pub collision_triangles: usize,
    pub collision_materials: Vec<CollisionMaterialBinding>,
    pub material_plans: Vec<MaterialPlan>,
    pub merge_report: MergeReport,
    pub dropped_lightmaps: usize,
    pub spawn_position: [f32; 3],
    /// World extent of the converted map.
    pub map_extent: MapExtent,
    /// Template scene objects dropped because they belong to the template map,
    /// counted by `game_object_type`.
    pub dropped_template_objects: BTreeMap<String, usize>,
    /// Source normal maps bound into the target materials.
    pub bound_normal_maps: usize,
    pub files: Vec<BuiltFile>,
}

/// One converted texture.
struct TexturePlan {
    source_name: String,
    target_dds: String,
    target_options: String,
    info: DdsInfo,
    bytes: Vec<u8>,
}

/// Build the additive bundle for one map.
pub fn build_bundle(
    manifest: &SocMapManifest,
    source_root: &Path,
    target_db: &Path,
    output: &Path,
) -> Result<SocMapBuildRecord> {
    manifest.validate()?;
    if output.exists() {
        bail!("bundle output {} already exists", output.display());
    }
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)
        .with_context(|| format!("creating bundle overlay {}", overlay.display()))?;

    let mut source = SocSource::open(source_root)?;
    let mut target = ResourceDb::open(target_db)?;

    // --- decode -----------------------------------------------------------
    let decoded = decode_level(&mut source, &manifest.source_level)?;
    let DecodedLevel {
        shader_table,
        visuals,
        buffers,
        cform,
        library,
        blenders,
    } = decoded;

    // --- convert ----------------------------------------------------------
    let (surfaces, merge_report) = surface::merge_visuals(&visuals, &buffers)?;
    if surfaces.is_empty() {
        bail!("the level produced no convertible render surfaces");
    }

    let mut used_shaders: BTreeMap<u16, usize> = BTreeMap::new();
    for merged in &surfaces {
        *used_shaders.entry(merged.shader_id).or_insert(0) += 1;
    }
    let overrides = manifest.effective_name_overrides();
    let material_plans =
        render_material::plan_materials(&used_shaders, &shader_table, &blenders, &overrides)?;
    let bound_shaders: BTreeSet<String> = material_plans
        .iter()
        .map(|plan| plan.source_shader.clone())
        .collect();
    render_material::validate_override_usage(&manifest.render_shader_families, &bound_shaders)?;

    let used_materials = cform.material_triangle_counts();
    game_material::validate_override_usage(
        &manifest.collision_material_overrides,
        &library,
        &used_materials,
    )?;
    let target_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let collision_materials = game_material::resolve_bindings(
        &used_materials,
        &library,
        &manifest.effective_correspondence(),
        &target_materials,
    )?;
    let collision_payloads = collision::encode(&cform, &collision_materials)?;

    let textures = plan_textures(manifest, &mut source, &material_plans)?;
    let normals: BTreeSet<String> = textures
        .iter()
        .filter(|texture| texture.source_name.ends_with("_bump"))
        .map(|texture| texture.source_name.clone())
        .collect();

    // --- emit -------------------------------------------------------------
    let mut files = Vec::new();
    let model_groups = surface::assign_models(&surfaces, manifest.surfaces_per_model)?;
    let mut models = Vec::with_capacity(model_groups.len());
    for (index, group) in model_groups.iter().enumerate() {
        let path = manifest.model_path(index);
        write_model_archive(
            manifest,
            &mut target,
            &overlay,
            &path,
            &surfaces,
            group,
            &mut files,
        )?;
        models.push(path);
    }

    let collision_model = manifest.collision_model_path();
    write_collision_archive(
        manifest,
        &mut target,
        &overlay,
        &collision_model,
        &collision_payloads,
        &collision_materials,
        &mut files,
    )?;

    write_materials(manifest, &mut target, &overlay, &material_plans, &normals, &mut files)?;
    write_textures(manifest, &mut target, &overlay, &textures, &mut files)?;
    let extent = MapExtent::of(&surfaces, cform.header.bounding_box)?;
    let dropped_objects = write_projects(
        manifest,
        &mut target,
        &overlay,
        &models,
        &collision_model,
        &mut files,
    )?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    reject_duplicate_outputs(&files)?;

    let record = SocMapBuildRecord {
        asset_id: manifest.asset_id.clone(),
        source_level: manifest.source_level.clone(),
        project_name: manifest.project_name.clone(),
        client_project: manifest.client_project_path(),
        server_project: manifest.server_project_path(),
        models,
        collision_model,
        rendered_surfaces: merge_report.rendered_visuals,
        converted_surfaces: surfaces.len(),
        rendered_triangles: merge_report.triangles,
        tree_surfaces: merge_report.tree_visuals,
        textures: textures.len(),
        materials: material_plans.len(),
        collision_vertices: collision_payloads.vertex_count,
        collision_triangles: collision_payloads.triangle_count,
        dropped_lightmaps: material_plans
            .iter()
            .map(|plan| plan.dropped_lightmaps.len())
            .sum(),
        collision_materials,
        material_plans,
        merge_report,
        spawn_position: manifest.spawn_position,
        map_extent: extent,
        dropped_template_objects: dropped_objects,
        bound_normal_maps: normals.len(),
        files,
    };

    let manifest_path = output.join("port-manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("writing {}", manifest_path.display()))?;
    Ok(record)
}

/// The world extent a derived lighting volume has to contain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapExtent {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl MapExtent {
    /// Union the converted render surfaces with the source collision bounds.
    ///
    /// Collision reaches below the visible mesh in places — ground has
    /// thickness, and ravine floors and blockers sit under the render surface —
    /// so a volume sized to visible geometry alone can leave the lowest
    /// walkable spots outside every ambient source.
    pub fn of(surfaces: &[MergedSurface], collision: ([f32; 3], [f32; 3])) -> Result<Self> {
        let mut min = collision.0;
        let mut max = collision.1;
        for merged in surfaces {
            let bounds = merged.bounds();
            for axis in 0..3 {
                min[axis] = min[axis].min(bounds.min[axis]);
                max[axis] = max[axis].max(bounds.max[axis]);
            }
        }
        if !min.iter().chain(&max).all(|value| value.is_finite()) {
            bail!("map extent is not finite");
        }
        if (0..3).any(|axis| max[axis] < min[axis]) {
            bail!("map extent is inverted: {min:?}..{max:?}");
        }
        Ok(Self { min, max })
    }

    pub fn center(&self) -> [f32; 3] {
        std::array::from_fn(|axis| (self.min[axis] + self.max[axis]) * 0.5)
    }

    pub fn size(&self) -> [f32; 3] {
        std::array::from_fn(|axis| self.max[axis] - self.min[axis])
    }

    /// Side lengths grown by `margin` on every side.
    pub fn padded_size(&self, margin: f32) -> [f32; 3] {
        let size = self.size();
        std::array::from_fn(|axis| size[axis] + margin * 2.0)
    }
}

/// One level decoded into everything the converter needs.
pub struct DecodedLevel {
    pub shader_table: level::ShaderTable,
    pub visuals: Vec<ogf::Visual>,
    pub buffers: level::GeometryBuffers,
    pub cform: cform::Cform,
    pub library: BTreeMap<u16, gamemtl::GameMaterial>,
    /// The shader library `shaders.xr` records, keyed by shader name.
    pub blenders: BTreeMap<String, blender::Blender>,
}

/// Decode one compiled level and the physical-material library.
pub fn decode_level(source: &mut SocSource, level_name: &str) -> Result<DecodedLevel> {
    let level_bytes = source.read_level_file(level_name, "level")?;
    let geom_bytes = source.read_level_file(level_name, "level.geom")?;
    let cform_bytes = source.read_level_file(level_name, "level.cform")?;

    level::parse_header(&level_bytes)?;
    let shader_table = level::parse_shaders(&level_bytes)?;
    let buffers = level::parse_geometry(&geom_bytes)?;
    let visual_payloads = level::visual_payloads(&level_bytes)?;
    let mut visuals = Vec::with_capacity(visual_payloads.len());
    for (index, payload) in visual_payloads.iter().enumerate() {
        visuals.push(ogf::parse_visual(payload, index)?);
    }
    let cform = cform::parse(&cform_bytes)?;
    let library = gamemtl::parse_library(&source.read_game_materials()?)?;
    let blenders = blender::parse_library(&source.read_shaders()?)?;

    Ok(DecodedLevel {
        shader_table,
        visuals,
        buffers,
        cform,
        library,
        blenders,
    })
}

/// What decoding and merging one level produced, without emitting anything.
///
/// This is the shape the recovered `port-manifest.json` files record, so a
/// probe of Cordon can be compared directly against the original port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LevelProbe {
    pub level: String,
    pub shader_table_entries: usize,
    pub visuals: usize,
    pub rendered_surfaces: usize,
    pub tree_surfaces: usize,
    pub converted_surfaces: usize,
    pub rendered_triangles: usize,
    /// Of `rendered_triangles`, how many came from tree visuals.
    pub tree_triangles: usize,
    /// Triangles counted once per distinct source index range.
    ///
    /// `xrLC` lets many visuals share one buffer range and place it by matrix,
    /// so this is the size of the level's unique geometry rather than of the
    /// geometry a merge emits.
    pub distinct_source_triangles: usize,
    pub distinct_source_ranges: usize,
    /// Triangles in the *union* of the index space the level references.
    ///
    /// Distinct ranges can overlap — a progressive visual and a plain one can
    /// share a prefix — so this is the tightest measure of unique geometry.
    pub covered_source_triangles: usize,
    pub bound_shaders: usize,
    pub source_textures: usize,
    pub missing_textures: Vec<String>,
    pub dropped_lightmaps: usize,
    pub collision_vertices: usize,
    pub collision_triangles: usize,
    pub collision_materials: Vec<(u16, String, usize)>,
    /// Collision materials the level uses that X-Ray marks climbable, with
    /// their triangle counts: ladders, and climbable fences.
    pub climbable_materials: Vec<(String, usize)>,
    /// Materials the actor walks through rather than collides with.
    pub passable_materials: Vec<(String, usize)>,
    pub unclassified_shaders: Vec<String>,
    pub dropped_by_kind: BTreeMap<String, usize>,
    pub vertex_buffers: usize,
    pub index_buffers: usize,
    /// World extent of the converted render geometry.
    pub render_bounds: (
        [f32; 3],
        [f32; 3],
    ),
    /// World extent of the source collision mesh, from `hdrCFORM`.
    pub collision_bounds: ([f32; 3], [f32; 3]),
    /// The union of both: what a derived lighting volume has to contain.
    pub world_bounds: ([f32; 3], [f32; 3]),
    /// Side lengths of `world_bounds`, in engine units.
    pub world_size: [f32; 3],
}

/// Decode and merge a level, reporting what the conversion would contain.
///
/// Needs no target database, so it can run against a bare installation. It is
/// the check that proves the decoders against real data.
pub fn probe_level(source: &mut SocSource, level_name: &str) -> Result<LevelProbe> {
    let decoded = decode_level(source, level_name)?;
    let (surfaces, merge_report) = surface::merge_visuals(&decoded.visuals, &decoded.buffers)?;

    let mut used_shaders: BTreeMap<u16, usize> = BTreeMap::new();
    for merged in &surfaces {
        *used_shaders.entry(merged.shader_id).or_insert(0) += 1;
    }

    let mut distinct_ranges = BTreeSet::new();
    for visual in &decoded.visuals {
        if !visual.kind.is_static() {
            continue;
        }
        if let Some(container) = visual.geometry {
            distinct_ranges.insert((
                container.index_buffer,
                container.index_base,
                container.index_count,
            ));
        }
    }
    let distinct_source_triangles: usize = distinct_ranges
        .iter()
        .map(|(_, _, count)| *count as usize / 3)
        .sum();

    let mut by_buffer: BTreeMap<u32, Vec<(u64, u64)>> = BTreeMap::new();
    for (buffer, base, count) in &distinct_ranges {
        by_buffer
            .entry(*buffer)
            .or_default()
            .push((u64::from(*base), u64::from(*base) + u64::from(*count)));
    }
    let mut covered_indices = 0_u64;
    for intervals in by_buffer.values_mut() {
        intervals.sort_unstable();
        let mut current: Option<(u64, u64)> = None;
        for (start, end) in intervals.iter().copied() {
            match &mut current {
                Some((_, open_end)) if start <= *open_end => *open_end = (*open_end).max(end),
                Some((open_start, open_end)) => {
                    covered_indices += *open_end - *open_start;
                    current = Some((start, end));
                }
                None => current = Some((start, end)),
            }
        }
        if let Some((start, end)) = current {
            covered_indices += end - start;
        }
    }
    let covered_source_triangles = (covered_indices / 3) as usize;

    // The extent a derived sky-AO or ambient volume has to contain. Collision
    // is unioned in because it reaches below visible geometry in places, and a
    // lighting volume that stops at the render bounds would leave those spots
    // outside every ambient source.
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for merged in &surfaces {
        let bounds = merged.bounds();
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(bounds.min[axis]);
            maximum[axis] = maximum[axis].max(bounds.max[axis]);
        }
    }
    let render_bounds = (minimum, maximum);
    let collision_bounds = decoded.cform.header.bounding_box;
    for axis in 0..3 {
        minimum[axis] = minimum[axis].min(collision_bounds.0[axis]);
        maximum[axis] = maximum[axis].max(collision_bounds.1[axis]);
    }
    let world_size = [
        maximum[0] - minimum[0],
        maximum[1] - minimum[1],
        maximum[2] - minimum[2],
    ];

    let overrides = render_material::default_name_overrides();
    let mut unclassified = BTreeSet::new();
    let mut source_textures = BTreeSet::new();
    let mut dropped_lightmaps = 0;
    for shader_id in used_shaders.keys() {
        let entry = decoded.shader_table.get(*shader_id)?;
        let classified = match decoded.blenders.get(&entry.shader) {
            Some(found) => render_material::classify(&entry.shader, found, &overrides),
            None => overrides.get(&entry.shader).copied(),
        };
        if classified.is_none() {
            unclassified.insert(entry.shader.clone());
        }
        if let Some(texture) = entry.base_texture() {
            source_textures.insert(texture.to_owned());
        }
        dropped_lightmaps += entry.lightmaps().len();
    }

    let mut missing_textures = Vec::new();
    for texture in &source_textures {
        if source.read_texture(texture)?.is_none() {
            missing_textures.push(texture.clone());
        }
    }

    let counts = decoded.cform.material_triangle_counts();
    let mut collision_materials = Vec::with_capacity(counts.len());
    for (id, triangles) in &counts {
        let name = decoded
            .library
            .get(id)
            .map(|material| material.name.clone())
            .unwrap_or_else(|| format!("<undefined id {id}>"));
        collision_materials.push((*id, name, *triangles));
    }

    let flagged = |flag: u32| -> Vec<(String, usize)> {
        counts
            .iter()
            .filter_map(|(id, triangles)| {
                let material = decoded.library.get(id)?;
                material.has(flag).then(|| (material.name.clone(), *triangles))
            })
            .collect()
    };
    let climbable_materials = flagged(gamemtl::flags::CLIMABLE);
    let passable_materials = flagged(gamemtl::flags::PASSABLE);

    Ok(LevelProbe {
        level: level_name.to_owned(),
        shader_table_entries: decoded.shader_table.len(),
        visuals: merge_report.visuals,
        rendered_surfaces: merge_report.rendered_visuals,
        tree_surfaces: merge_report.tree_visuals,
        converted_surfaces: surfaces.len(),
        rendered_triangles: merge_report.triangles,
        tree_triangles: merge_report.tree_triangles,
        distinct_source_triangles,
        distinct_source_ranges: distinct_ranges.len(),
        covered_source_triangles,
        bound_shaders: used_shaders.len(),
        source_textures: source_textures.len(),
        missing_textures,
        dropped_lightmaps,
        collision_vertices: decoded.cform.vertices.len(),
        collision_triangles: decoded.cform.faces.len(),
        collision_materials,
        climbable_materials,
        passable_materials,
        unclassified_shaders: unclassified.into_iter().collect(),
        dropped_by_kind: merge_report.dropped_by_kind,
        render_bounds,
        collision_bounds,
        world_bounds: (minimum, maximum),
        world_size,
        vertex_buffers: decoded.buffers.vertex_buffers.len(),
        index_buffers: decoded.buffers.index_buffers.len(),
    })
}

fn plan_textures(
    manifest: &SocMapManifest,
    source: &mut SocSource,
    plans: &[MaterialPlan],
) -> Result<Vec<TexturePlan>> {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for plan in plans {
        if let Some(diffuse) = &plan.source_diffuse {
            wanted.insert(diffuse.clone());
        }
    }
    let mut stale: Vec<&String> = manifest
        .texture_aliases
        .keys()
        .filter(|name| !wanted.contains(*name))
        .collect();
    if !stale.is_empty() {
        stale.sort();
        bail!("texture aliases no longer apply to any bound texture: {stale:?}");
    }

    // Shadow of Chernobyl stores a normal map beside its diffuse, as
    // `<name>_bump`. Where one exists it is a free fidelity gain: the payload
    // is a DDS the target already reads, and the binding is one field.
    if manifest.bind_normal_maps {
        let with_normals: Vec<String> = wanted
            .iter()
            .map(|name| format!("{name}_bump"))
            .filter(|candidate| {
                source
                    .read_texture(candidate)
                    .map(|found| found.is_some())
                    .unwrap_or(false)
            })
            .collect();
        wanted.extend(with_normals);
    }

    let mut textures = Vec::with_capacity(wanted.len());
    let mut missing = Vec::new();
    for name in &wanted {
        let effective = manifest.texture_aliases.get(name).unwrap_or(name).clone();
        let Some(bytes) = source.read_texture(&effective)? else {
            missing.push(name.clone());
            continue;
        };
        let info = source::parse_dds(&bytes, name)?;
        // Source chains run to 1x1; the target stops at the last complete 4x4
        // block and expects a trailing colour-space byte. Rewriting the
        // container is what the recovered port's `identity:SoC-DDS` copy did
        // not do.
        let is_normal = name.ends_with("_bump");
        let bytes = if manifest.allow_untrimmed_mip_chains {
            bytes
        } else {
            source::canonicalize_dds(&bytes, info, !is_normal)
                .with_context(|| format!("canonicalizing {name}"))?
        };
        // The target path always keeps the *declared* name, so an alias stays
        // visible in the manifest without moving the binding.
        let target_dds = source::target_texture_path(&manifest.namespace, name)?;
        let target_options = target_dds
            .strip_suffix(".dds")
            .expect("target texture paths end in .dds")
            .to_owned()
            + ".options";
        textures.push(TexturePlan {
            source_name: name.clone(),
            target_dds,
            target_options,
            info,
            bytes,
        });
    }
    if !missing.is_empty() {
        bail!(
            "the source tree has no texture for {missing:?}; add reviewed texture_aliases entries"
        );
    }
    Ok(textures)
}

fn write_model_archive(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    model: &str,
    surfaces: &[MergedSurface],
    group: &[usize],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let surface_template = load_template(target, &manifest.templates.surface_properties)?;
    let mut bindings = BTreeMap::new();

    for index in group {
        let merged = &surfaces[*index];
        let name = merged.name();
        let reference = format!("{}/{}", manifest.namespace, merged.material_name());
        bindings.insert(name.clone(), reference.clone());

        write_resource(
            overlay,
            &format!("{model}/render/{name}/converted_model"),
            &merged.encode_converted_model()?,
            "converted:SoC-OGF-static-visuals",
            files,
        )?;

        let bounds = merged.bounds();
        let mut properties = surface_template.clone();
        properties.root.set_path(
            "source.mesh_name",
            ConfigValue::String(format!("|SOC|{name}")),
        )?;
        properties
            .root
            .set_path("source.texture_name", ConfigValue::String(reference))?;
        properties.root.set_path(
            "source.vertices",
            ConfigValue::Integer(i32::try_from(merged.vertices.len())?),
        )?;
        properties.root.set_path(
            "source.indices",
            ConfigValue::Integer(i32::try_from(merged.indices.len())?),
        )?;
        properties.root.set_path(
            "source.primitives",
            ConfigValue::Integer(i32::try_from(merged.triangle_count())?),
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
        write_config(
            overlay,
            &format!("{model}/render/{name}/export_properties"),
            &properties,
            "generated:v0.100b-surface-properties",
            files,
        )?;
    }

    let render_properties = load_template(target, &manifest.templates.model_render_properties)?;
    write_config(
        overlay,
        &format!("{model}/render/export_properties"),
        &render_properties,
        "generated:v0.100b-model-properties",
        files,
    )?;

    let mut settings = load_template(target, &manifest.templates.model_settings)?;
    // The one-to-one rule: the surface list in settings and the surface
    // directories carrying geometry must be the same set.
    // Every LOD level the template declares gets our surface list. The merge
    // produces one detail level, but the target's LOD switching expects the
    // levels the template had, and the one-to-one rule -- surfaces named in
    // settings == surface directories carrying geometry -- makes a level left
    // naming the template's surfaces a load failure.
    let surface_names: Vec<ConfigValue> =
        bindings.keys().cloned().map(ConfigValue::String).collect();
    let levels: Vec<String> = match settings.root.named("lod_hierrarchy") {
        Some(ConfigValue::Named(levels)) => levels.iter().map(|l| l.name.clone()).collect(),
        _ => bail!("{model}: template settings have no lod_hierrarchy"),
    };
    for level in levels {
        settings.root.set_path(
            &format!("lod_hierrarchy.{level}.surfaces"),
            ConfigValue::Indexed(surface_names.clone()),
        )?;
    }
    settings.root.set_path(
        "material_settings",
        ConfigValue::Named(
            bindings
                .iter()
                .map(|(surface, material)| NamedValue {
                    name: surface.clone(),
                    value: ConfigValue::Named(vec![
                        NamedValue {
                            name: "material_name".into(),
                            value: ConfigValue::String(material.clone()),
                        },
                        NamedValue {
                            name: "used_in_portals".into(),
                            value: ConfigValue::Boolean(false),
                        },
                    ]),
                })
                .collect(),
        ),
    )?;
    write_config(
        overlay,
        &format!("{model}/settings"),
        &settings,
        "generated:v0.100b-model-settings",
        files,
    )
}

fn write_collision_archive(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    model: &str,
    payloads: &collision::CollisionPayloads,
    bindings: &[CollisionMaterialBinding],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    write_resource(
        overlay,
        &format!("{model}/collision/vertices"),
        &payloads.vertices,
        "converted:SoC-CFORM-vertices",
        files,
    )?;
    write_resource(
        overlay,
        &format!("{model}/collision/indices"),
        &payloads.indices,
        "converted:SoC-CFORM-indices",
        files,
    )?;
    write_resource(
        overlay,
        &format!("{model}/collision/face_data"),
        &payloads.face_data,
        "converted:SoC-CFORM-per-triangle-materials",
        files,
    )?;

    let mut settings = load_template(target, &manifest.templates.collision_settings)?;
    // v0.100b ships no collision-only model, so the template necessarily has
    // render surfaces. A collision archive declares none, and the one-to-one
    // rule -- surfaces in settings == surface directories with geometry -- makes
    // leaving them in a crash rather than a cosmetic flaw.
    for key in ["material_settings", "lod_hierrarchy", "lod_switching"] {
        remove_named(&mut settings.root, key);
    }
    settings.root.set_path(
        "game_material_settings",
        ConfigValue::Named(
            bindings
                .iter()
                .map(|binding| NamedValue {
                    name: binding.face_name.clone(),
                    value: ConfigValue::Named(vec![
                        NamedValue {
                            name: "game_material_id".into(),
                            value: ConfigValue::Integer(binding.target_id),
                        },
                        NamedValue {
                            name: "game_material_name".into(),
                            value: ConfigValue::String(binding.target_name.clone()),
                        },
                    ]),
                })
                .collect(),
        ),
    )?;
    write_config(
        overlay,
        &format!("{model}/settings"),
        &settings,
        "generated:v0.100b-collision-model-settings",
        files,
    )
}

/// Rewrite one texture binding, keeping the reference style the template uses.
fn bind_texture(
    material: &mut BinaryConfig,
    path: &str,
    target_texture: &str,
    template_path: &str,
) -> Result<()> {
    let existing = named_path(&material.root, path)
        .with_context(|| format!("material template {template_path} has no {path} binding"))?;
    let ConfigValue::String(existing) = existing else {
        bail!("material template {template_path} binds {path} as a non-string value");
    };
    let reference = texture_reference_like(existing, target_texture);
    material.root.set_path(path, ConfigValue::String(reference))
}

fn write_materials(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    plans: &[MaterialPlan],
    normals: &BTreeSet<String>,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut templates = BTreeMap::new();
    for plan in plans {
        let template_path = manifest
            .material_templates
            .get(&plan.family)
            .expect("validated manifests have a template for every family");
        if !templates.contains_key(template_path) {
            templates.insert(template_path.clone(), load_template(target, template_path)?);
        }
        let mut material = templates[template_path].clone();
        if let Some(diffuse) = &plan.source_diffuse {
            let diffuse_path = manifest
                .family_diffuse_paths
                .get(&plan.family)
                .unwrap_or(&manifest.diffuse_path);
            bind_texture(
                &mut material,
                diffuse_path,
                &source::target_texture_path(&manifest.namespace, diffuse)?,
                template_path,
            )?;

            // The `_bump` companion, where the source has one. The enable flag
            // has to move with it: a template chosen for its diffuse may have
            // normal mapping switched off.
            if manifest.bind_normal_maps && normals.contains(&format!("{diffuse}_bump")) {
                bind_texture(
                    &mut material,
                    &manifest.normal_path,
                    &source::target_texture_path(
                        &manifest.namespace,
                        &format!("{diffuse}_bump"),
                    )?,
                    template_path,
                )?;
                material
                    .root
                    .set_path(&manifest.normal_enable_path, ConfigValue::Boolean(true))?;
            }
        }
        write_config(
            overlay,
            &plan.target_path(&manifest.namespace),
            &material,
            &format!("generated:v0.100b-material-from-SoC:{}", plan.source_shader),
            files,
        )?;
    }
    Ok(())
}

fn write_textures(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    textures: &[TexturePlan],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let template = load_template(target, &manifest.templates.texture_options)?;
    for texture in textures {
        // The pixels are X-Ray's; the container around them is rewritten to the
        // target's convention — chain trimmed to the last complete block, and a
        // trailing colour-space byte.
        write_resource(
            overlay,
            &texture.target_dds,
            &texture.bytes,
            "converted:SoC-DDS-to-v0.100b-container",
            files,
        )?;

        // `.options` is what the cooker reads to decide how to interpret the
        // payload; a format mismatch here is a missing texture at runtime.
        // v0.100b treats a normal map as its own format rather than as a DXT5
        // with a convention attached.
        let is_normal = texture.source_name.ends_with("_bump");
        let mut options = template.clone();
        options.root.set_path(
            "options.format",
            ConfigValue::String(texture.info.target_format(is_normal)?.to_owned()),
        )?;
        options.root.set_path(
            "options.has_mip_levels",
            ConfigValue::Integer(i32::from(texture.info.mip_levels > 1)),
        )?;
        options.root.set_path(
            "options.srgb",
            ConfigValue::Integer(i32::from(!is_normal)),
        )?;
        write_config(
            overlay,
            &texture.target_options,
            &options,
            "generated:v0.100b-texture-options",
            files,
        )
        .with_context(|| format!("writing options for {}", texture.source_name))?;
    }
    Ok(())
}

fn write_projects(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    models: &[String],
    collision_model: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<BTreeMap<String, usize>> {
    let bindings = &manifest.project_bindings;

    let mut client = load_template(target, &manifest.templates.client_project)?;
    let dropped_objects = filter_template_objects(manifest, &mut client.root)?;
    client.root.set_path(
        &bindings.project_name_path,
        ConfigValue::String(manifest.project_name.clone()),
    )?;
    let visuals = rebuild_instances(
        &client.root,
        &bindings.visual_array_path,
        &bindings.instance_model_path,
        models,
    )?;
    client
        .root
        .set_path(&bindings.visual_array_path, ConfigValue::Indexed(visuals))?;
    let collision = rebuild_instances(
        &client.root,
        &bindings.collision_array_path,
        &bindings.instance_model_path,
        std::slice::from_ref(&collision_model.to_owned()),
    )?;
    client
        .root
        .set_path(&bindings.collision_array_path, ConfigValue::Indexed(collision))?;
    write_config(
        overlay,
        &manifest.client_project_path(),
        &client,
        "generated:SoC-client-project",
        files,
    )?;

    let mut server = load_template(target, &manifest.templates.server_project)?;
    server.root.set_path(
        &bindings.server_project_name_path,
        ConfigValue::String(manifest.project_name.clone()),
    )?;
    let respawns = named_path(&server.root, &bindings.server_respawn_array_path)?;
    let ConfigValue::Indexed(entries) = respawns else {
        bail!(
            "server template {} is not an indexed array",
            bindings.server_respawn_array_path
        );
    };
    if entries.is_empty() {
        bail!(
            "server template {} has no respawn entry to place",
            bindings.server_respawn_array_path
        );
    }
    let mut placed = entries.clone();
    for entry in &mut placed {
        entry.set_path(
            &bindings.respawn_position_path,
            ConfigValue::Float3(manifest.spawn_position),
        )?;
    }
    server.root.set_path(
        &bindings.server_respawn_array_path,
        ConfigValue::Indexed(placed),
    )?;
    write_config(
        overlay,
        &manifest.server_project_path(),
        &server,
        "generated:SoC-server-project",
        files,
    )?;
    Ok(dropped_objects)
}

/// Drop every scene object whose meaning is positional.
///
/// v0.100b keeps lights, decals, environment probes, collision geometry,
/// sounds, vegetation, sky and fog in one flat array discriminated by
/// `game_object_type`. Cloning a template project inherits all of them at the
/// template map's coordinates: `projects/level_03/client_project` carries 248,
/// including 36 environment probes, 110 decals and 47 collision geometries.
///
/// Only globally-meaningful types survive — sky, environment, fog — because
/// they describe the whole scene rather than a place in it. Everything else
/// would be furniture from another map scattered through this one, and the
/// probes in particular are what put a hard black lighting boundary through
/// the STK2 A01 ravine.
///
/// Returns how many objects of each type were dropped, for the build record.
fn filter_template_objects(
    manifest: &SocMapManifest,
    root: &mut ConfigValue,
) -> Result<BTreeMap<String, usize>> {
    let bindings = &manifest.project_bindings;
    let (Some(array_path), Some(type_path)) = (
        bindings.objects_array_path.as_ref(),
        bindings.object_type_path.as_ref(),
    ) else {
        return Ok(BTreeMap::new());
    };

    let ConfigValue::Indexed(entries) = named_path(root, array_path)? else {
        bail!("project template {array_path} is not an indexed array");
    };
    let retained: BTreeSet<&str> = bindings
        .retained_object_types
        .iter()
        .map(String::as_str)
        .collect();

    let mut kept = Vec::new();
    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in entries {
        let kind = match entry.named(type_path) {
            Some(ConfigValue::String(kind)) => kind.clone(),
            // An object without the discriminator cannot be judged, so it goes:
            // keeping it would be keeping something unidentified from another
            // map.
            _ => "<untyped>".to_owned(),
        };
        seen.insert(kind.clone());
        if retained.contains(kind.as_str()) {
            kept.push(entry.clone());
        } else {
            *dropped.entry(kind).or_insert(0) += 1;
        }
    }

    // A retain entry that matches nothing is a stale review, same posture as
    // every other override in this converter.
    let stale: Vec<&String> = bindings
        .retained_object_types
        .iter()
        .filter(|kind| !seen.contains(*kind))
        .collect();
    if !stale.is_empty() {
        bail!("retained_object_types names types the template does not contain: {stale:?}");
    }

    root.set_path(array_path, ConfigValue::Indexed(kept))?;
    Ok(dropped)
}

/// Clone the template's first array entry once per model, so the generated
/// instances keep every field of a real project entry.
fn rebuild_instances(
    root: &ConfigValue,
    array_path: &str,
    model_path: &str,
    models: &[String],
) -> Result<Vec<ConfigValue>> {
    let ConfigValue::Indexed(entries) = named_path(root, array_path)? else {
        bail!("project template {array_path} is not an indexed array");
    };
    let prototype = entries
        .first()
        .with_context(|| format!("project template {array_path} has no entry to clone"))?;
    let existing = prototype
        .named(model_path)
        .with_context(|| format!("project template {array_path} entry has no {model_path}"))?;
    let ConfigValue::String(existing) = existing else {
        bail!("project template {array_path} entry binds {model_path} as a non-string value");
    };
    let existing = existing.clone();
    models
        .iter()
        .map(|model| {
            let mut entry = prototype.clone();
            entry.set_path(
                model_path,
                ConfigValue::String(model_reference_like(&existing, model)),
            )?;
            Ok(entry)
        })
        .collect()
}

/// Drop one named field, if the config has it.
fn remove_named(root: &mut ConfigValue, key: &str) {
    if let ConfigValue::Named(values) = root {
        values.retain(|value| value.name != key);
    }
}

/// Resolve a dotted path against a decoded config.
fn named_path<'a>(root: &'a ConfigValue, path: &str) -> Result<&'a ConfigValue> {
    let mut current = root;
    for part in path.split('.') {
        current = current
            .named(part)
            .with_context(|| format!("project template has no field {path}"))?;
    }
    Ok(current)
}

/// Match the reference style a template already uses.
///
/// v0.100b projects and materials are inconsistent about whether a reference
/// carries its family prefix and extension, so the converter copies whatever
/// the reviewed template does instead of guessing.
fn texture_reference_like(template_value: &str, target_path: &str) -> String {
    let keeps_prefix = template_value.starts_with("textures/");
    let keeps_extension = template_value.ends_with(".dds");
    let mut reference = target_path.to_owned();
    if !keeps_extension {
        reference = reference
            .strip_suffix(".dds")
            .unwrap_or(&reference)
            .to_owned();
    }
    if !keeps_prefix {
        reference = reference
            .strip_prefix("textures/")
            .unwrap_or(&reference)
            .to_owned();
    }
    reference
}

fn model_reference_like(template_value: &str, target_path: &str) -> String {
    let keeps_prefix = template_value.starts_with("models/");
    let keeps_extension = template_value.ends_with(".model");
    let mut reference = target_path.to_owned();
    if !keeps_extension {
        reference = reference
            .strip_suffix(".model")
            .unwrap_or(&reference)
            .to_owned();
    }
    if !keeps_prefix {
        reference = reference
            .strip_prefix("models/")
            .unwrap_or(&reference)
            .to_owned();
    }
    reference
}

fn load_template(target: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    let bytes = target
        .read(path)
        .with_context(|| format!("reading template {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing template {path}"))
}

fn write_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    origin: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, bytes).with_context(|| format!("writing {}", output.display()))?;
    files.push(record_file(path, origin, bytes));
    Ok(())
}

fn write_config(
    overlay: &Path,
    path: &str,
    config: &BinaryConfig,
    origin: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let bytes = config
        .to_bytes()
        .with_context(|| format!("encoding {path}"))?;
    write_resource(overlay, path, &bytes, origin, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_the_reference_style_of_a_template() {
        assert_eq!(
            texture_reference_like("textures/stock/wall.dds", "textures/ported/soc/cordon/a.dds"),
            "textures/ported/soc/cordon/a.dds"
        );
        assert_eq!(
            texture_reference_like("stock/wall", "textures/ported/soc/cordon/a.dds"),
            "ported/soc/cordon/a"
        );
        assert_eq!(
            model_reference_like("models/stock/x.model", "models/ported/soc/cordon/y.model"),
            "models/ported/soc/cordon/y.model"
        );
        assert_eq!(
            model_reference_like("stock/x", "models/ported/soc/cordon/y.model"),
            "ported/soc/cordon/y"
        );
    }

    fn project_template() -> ConfigValue {
        ConfigValue::Named(vec![
            NamedValue {
                name: "project_name".into(),
                value: ConfigValue::String("level_03".into()),
            },
            NamedValue {
                name: "objects".into(),
                value: ConfigValue::Named(vec![NamedValue {
                    name: "visuals".into(),
                    value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                        NamedValue {
                            name: "model".into(),
                            value: ConfigValue::String("stock/antenna".into()),
                        },
                        NamedValue {
                            name: "cast_shadow".into(),
                            value: ConfigValue::Boolean(true),
                        },
                    ])]),
                }]),
            },
        ])
    }

    #[test]
    fn clones_every_field_of_the_prototype_instance() {
        let models = vec![
            "models/ported/soc/cordon/cordon_000.model".to_owned(),
            "models/ported/soc/cordon/cordon_001.model".to_owned(),
        ];
        let instances =
            rebuild_instances(&project_template(), "objects.visuals", "model", &models).unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(
            instances[1].named("model"),
            Some(&ConfigValue::String(
                "ported/soc/cordon/cordon_001".into()
            ))
        );
        // Fields the converter does not understand survive untouched.
        assert_eq!(
            instances[0].named("cast_shadow"),
            Some(&ConfigValue::Boolean(true))
        );
    }

    #[test]
    fn fails_on_a_declared_path_the_template_does_not_have() {
        let error = rebuild_instances(&project_template(), "objects.lights", "model", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("objects.lights"), "{error}");
    }
    /// A client project shaped like `projects/level_03/client_project`: one
    /// flat `objects` array mixing global and positional scene objects.
    fn objects_template() -> ConfigValue {
        let object = |kind: &str, name: &str| {
            ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String(kind.into()),
                },
                NamedValue {
                    name: "name".into(),
                    value: ConfigValue::String(name.into()),
                },
                NamedValue {
                    name: "position".into(),
                    value: ConfigValue::Float3([900.0, 400.0, 900.0]),
                },
            ])
        };
        ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![
                object("environment_probe", "probe_a"),
                object("environment_probe", "probe_b"),
                object("decal", "decal_a"),
                object("light", "light_a"),
                object("collision_geometry", "collision_a"),
                object("sky", "the_sky"),
                object("environment", "the_environment"),
                object("volume_fog", "the_fog"),
            ]),
        }])
    }

    /// Cordon's real extent, as measured from retail: 1498 x 212 x 2417, with
    /// ground down at Y -96.9.
    const CORDON: MapExtent = MapExtent {
        min: [-749.0, -96.9, -1208.0],
        max: [749.0, 115.4, 1209.0],
    };

    #[test]
    fn map_extent_unions_render_and_collision_bounds() {
        let extent = MapExtent::of(&[], ([-10.0, -5.0, -10.0], [10.0, 5.0, 10.0])).unwrap();
        assert_eq!(extent.size(), [20.0, 10.0, 20.0]);
        assert_eq!(extent.center(), [0.0, 0.0, 0.0]);
        assert_eq!(extent.padded_size(16.0), [52.0, 42.0, 52.0]);
    }

    #[test]
    fn map_extent_measures_cordon() {
        assert_eq!(CORDON.size().map(|v| v.round()), [1498.0, 212.0, 2417.0]);
    }

    #[test]
    fn map_extent_rejects_an_inverted_box() {
        assert!(MapExtent::of(&[], ([10.0, 0.0, 0.0], [-10.0, 0.0, 0.0])).is_err());
    }

    #[test]
    fn positional_template_objects_are_dropped_and_global_ones_kept() {
        let manifest = crate::soc::manifest::sample_manifest();
        let mut root = objects_template();
        let dropped = filter_template_objects(&manifest, &mut root).unwrap();

        // Everything that carries a place in the template's map is gone.
        assert_eq!(dropped["environment_probe"], 2);
        assert_eq!(dropped["decal"], 1);
        assert_eq!(dropped["light"], 1);
        assert_eq!(dropped["collision_geometry"], 1);

        // Whole-scene settings survive.
        let ConfigValue::Indexed(kept) = named_path(&root, "objects").unwrap() else {
            panic!("objects is not indexed");
        };
        let names: Vec<&str> = kept
            .iter()
            .filter_map(|entry| match entry.named("game_object_type") {
                Some(ConfigValue::String(kind)) => Some(kind.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["sky", "environment", "volume_fog"]);
    }

    #[test]
    fn an_object_without_a_type_is_dropped_rather_than_guessed_at() {
        let mut manifest = crate::soc::manifest::sample_manifest();
        manifest.project_bindings.retained_object_types = vec!["sky".into()];
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![
                ConfigValue::Named(vec![NamedValue {
                    name: "name".into(),
                    value: ConfigValue::String("mystery".into()),
                }]),
                ConfigValue::Named(vec![NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("sky".into()),
                }]),
            ]),
        }]);
        let dropped = filter_template_objects(&manifest, &mut root).unwrap();
        assert_eq!(dropped["<untyped>"], 1);
    }

    #[test]
    fn a_stale_retained_type_is_a_build_failure() {
        let mut manifest = crate::soc::manifest::sample_manifest();
        manifest
            .project_bindings
            .retained_object_types
            .push("terrain".into());
        let mut root = objects_template();
        let error = filter_template_objects(&manifest, &mut root)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not contain"), "{error}");
    }

    #[test]
    fn a_material_binding_keeps_the_templates_reference_style() {
        let mut material = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "effect".into(),
                value: ConfigValue::Named(vec![NamedValue {
                    name: "texture_diffuse".into(),
                    value: ConfigValue::Named(vec![NamedValue {
                        name: "value".into(),
                        // The target writes namespace-relative, extension-less.
                        value: ConfigValue::String("tiled/roof/roof_03".into()),
                    }]),
                }]),
            }]),
        };
        bind_texture(
            &mut material,
            "effect.texture_diffuse.value",
            "textures/ported/soc/cordon/prop/bochka.dds",
            "tiled/roof/roof_03",
        )
        .unwrap();
        assert_eq!(
            named_path(&material.root, "effect.texture_diffuse.value").unwrap(),
            &ConfigValue::String("ported/soc/cordon/prop/bochka".into())
        );
    }

    #[test]
    fn fails_when_the_template_array_has_nothing_to_clone() {
        let template = ConfigValue::Named(vec![NamedValue {
            name: "visuals".into(),
            value: ConfigValue::Indexed(Vec::new()),
        }]);
        let error = rebuild_instances(&template, "visuals", "model", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("no entry to clone"), "{error}");
    }
}
