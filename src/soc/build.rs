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
use super::convert::render_material::{self, MaterialPlan, RenderFamily};
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
    pub collision_models: Vec<String>,
    /// Source visuals that produced render geometry.
    pub rendered_surfaces: usize,
    /// Merged target surfaces.
    pub converted_surfaces: usize,
    pub rendered_triangles: usize,
    /// Source visuals placed by their own tree matrix.
    pub tree_surfaces: usize,
    pub textures: usize,
    pub materials: usize,
    pub collision_pieces: usize,
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
///
/// `pub(crate)` because the F.E.A.R. path plans its own textures -- explicit
/// paths instead of X-Ray naming conventions -- and feeds them to the same
/// writer.
pub(crate) struct TexturePlan {
    pub(crate) source_name: String,
    pub(crate) target_dds: String,
    pub(crate) target_options: String,
    pub(crate) info: DdsInfo,
    pub(crate) bytes: Vec<u8>,
    /// Written into `.options` as the NORMAL_MAP format and linear colour
    /// space. SoC infers this from the `_bump` suffix; F.E.A.R. knows it from
    /// the material binding.
    pub(crate) is_normal: bool,
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

    // Drop the triangles X-Ray never blocks movement with before anything
    // downstream sees them: water, foliage and trigger volumes are in
    // `level.cform` like everything else, and converting them makes a pool
    // walkable and a bush a wall. See `GameMaterial::blocks_movement`.
    let mut cform = cform;
    let passable: BTreeSet<u16> = cform
        .material_triangle_counts()
        .keys()
        .copied()
        .filter(|id| {
            library
                .get(id)
                .is_some_and(|material| !material.blocks_movement())
        })
        .collect();
    if !passable.is_empty() {
        let before = cform.faces.len();
        cform.faces.retain(|face| !passable.contains(&face.material));
        let dropped = before - cform.faces.len();
        let names: Vec<&str> = passable
            .iter()
            .filter_map(|id| library.get(id).map(|material| material.name.as_str()))
            .collect();
        println!(
            "passable_collision_triangles_dropped={dropped} materials={}",
            names.join(",")
        );
    }

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
    // One level-sized mesh does not cook; retail ships collision as many small
    // per-object models. Split before encoding, and let the piece count set a
    // floor on how many model archives we emit, so every piece rides in an
    // archive that also carries render surfaces -- the only shape retail has.
    let collision_pieces = collision::split(&cform, manifest.max_collision_triangles)
        .iter()
        .map(|piece| {
            collision::encode(piece, &collision_materials, manifest.collision_face_dictionary)
        })
        .collect::<Result<Vec<_>>>()?;

    let textures = plan_textures(manifest, &mut source, &material_plans)?;
    let normals: BTreeSet<String> = textures
        .iter()
        .filter(|texture| texture.source_name.ends_with("_bump"))
        .map(|texture| texture.source_name.clone())
        .collect();

    // --- emit -------------------------------------------------------------
    let mut files = Vec::new();
    let pieces = if manifest.emit_collision {
        collision_pieces.len()
    } else {
        0
    };
    // Enough archives to carry every collision piece, without exceeding the
    // declared surfaces-per-archive.
    let surfaces_per_model = if pieces > 1 {
        // Floor, not ceiling: we need at least as many archives as pieces, and
        // ceiling rounds the archive count down below the piece count.
        manifest.surfaces_per_model.min(surfaces.len() / pieces).max(1)
    } else {
        manifest.surfaces_per_model
    };
    let model_groups = surface::assign_models(&surfaces, surfaces_per_model)?;
    if model_groups.len() < pieces {
        bail!(
            "{} collision pieces need at least that many model archives, but {} surfaces \
             produce only {}; lower max_collision_triangles or split the map",
            pieces,
            surfaces.len(),
            model_groups.len()
        );
    }
    // A family-wide texture-coordinate multiplier, resolved per shader.
    let uv_scale: BTreeMap<u16, f32> = material_plans
        .iter()
        .filter_map(|plan| {
            manifest
                .family_uv_scale
                .get(&plan.family)
                .map(|scale| (plan.shader_id, *scale))
        })
        .collect();
    // Decals sit exactly on the surface they decorate and v0.100b exposes no
    // depth bias, so they are lifted along their normals instead.
    let lift: BTreeMap<u16, f32> = material_plans
        .iter()
        .filter(|plan| plan.family == RenderFamily::Decal)
        .map(|plan| (plan.shader_id, manifest.decal_lift))
        .collect();
    let mut models = Vec::with_capacity(model_groups.len());
    for (index, group) in model_groups.iter().enumerate() {
        let path = manifest.model_path(index);
        // Each piece rides in an archive that also carries render surfaces.
        let collision = (index < pieces).then(|| &collision_pieces[index]);
        write_model_archive(
            manifest,
            &mut target,
            &overlay,
            &path,
            &surfaces,
            group,
            collision,
            &uv_scale,
            &lift,
            &mut files,
        )?;
        models.push(path);
    }

    let collision_models = models[..pieces].to_vec();

    // The level's own ground needs two generated control textures, and only if
    // the level has terrain surfaces at all — three of the thirteen maps are
    // carved out of a single-player level and reach no `levels\*` shader.
    let terrain_controls = material_plans
        .iter()
        .any(|plan| plan.family == RenderFamily::Terrain)
        .then(|| write_terrain_controls(manifest, &mut target, &overlay, &mut files))
        .transpose()?;
    let uv_density = surface::uv_world_density(&surfaces);
    write_materials(
        manifest,
        &mut target,
        &overlay,
        &material_plans,
        &normals,
        terrain_controls.as_ref(),
        &uv_density,
        &mut files,
    )?;
    write_textures(manifest, &mut target, &overlay, &textures, &mut files)?;
    let extent = MapExtent::of(&surfaces, cform.header.bounding_box)?;
    let post_effect =
        write_post_process_material(manifest, &mut target, &overlay, &extent, &mut files)?;
    let dropped_objects = write_projects(
        manifest,
        &mut target,
        &overlay,
        &models,
        &collision_models,
        &extent,
        &post_effect,
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
        collision_models,
        rendered_surfaces: merge_report.rendered_visuals,
        converted_surfaces: surfaces.len(),
        rendered_triangles: merge_report.triangles,
        tree_surfaces: merge_report.tree_visuals,
        textures: textures.len(),
        materials: material_plans.len(),
        collision_vertices: collision_pieces.iter().map(|piece| piece.vertex_count).sum(),
        collision_triangles: collision_pieces.iter().map(|piece| piece.triangle_count).sum(),
        collision_pieces: collision_pieces.len(),
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
        // Terrain wants a second texture the shader table never names: the
        // tiled ground detail X-Ray blended over its colour map.
        if plan.family == RenderFamily::Terrain {
            wanted.insert(terrain_layer_texture(manifest, &plan.source_shader));
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
            is_normal: name.ends_with("_bump"),
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

pub(crate) fn write_model_archive(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    model: &str,
    surfaces: &[MergedSurface],
    group: &[usize],
    collision: Option<&collision::CollisionPayloads>,
    uv_scale: &BTreeMap<u16, f32>,
    lift: &BTreeMap<u16, f32>,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let surface_template = load_template(target, &manifest.templates.surface_properties)?;
    let mut archive_min = [f32::INFINITY; 3];
    let mut archive_max = [f32::NEG_INFINITY; 3];
    let mut bindings = BTreeMap::new();

    for index in group {
        let merged = &surfaces[*index];
        let name = merged.name();
        let reference = format!("{}/{}", manifest.namespace, merged.material_name());
        bindings.insert(name.clone(), reference.clone());

        write_resource(
            overlay,
            &format!("{model}/render/{name}/converted_model"),
            &merged.encode_converted_model_lifted(
                uv_scale.get(&merged.shader_id).copied().unwrap_or(1.0),
                lift.get(&merged.shader_id).copied().unwrap_or(0.0),
            )?,
            "converted:SoC-OGF-static-visuals",
            files,
        )?;

        let bounds = merged.bounds();
        for axis in 0..3 {
            archive_min[axis] = archive_min[axis].min(bounds.min[axis]);
            archive_max[axis] = archive_max[axis].max(bounds.max[axis]);
        }
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

    // The template's bounding box describes the template's map. Cloning it
    // verbatim leaves every archive claiming level_03's extent -- for
    // `terrain_p_09` that is x 38.6..116.076, z -22.691..70.817, which barely
    // touches a converted map at all. Render-only archives merely cull wrong;
    // an archive carrying collision has to be cooked against these bounds.
    let mut render_properties = load_template(target, &manifest.templates.model_render_properties)?;
    if let Some(collision) = collision {
        let (min, max) = collision.bounds;
        for axis in 0..3 {
            archive_min[axis] = archive_min[axis].min(min[axis]);
            archive_max[axis] = archive_max[axis].max(max[axis]);
        }
    }
    if !archive_min.iter().chain(&archive_max).all(|value| value.is_finite()) {
        bail!("model archive {model} produced no finite geometry to bound");
    }
        render_properties
            .root
            .set_path("bounding_box.min", ConfigValue::Float3(archive_min))?;
        render_properties
            .root
            .set_path("bounding_box.max", ConfigValue::Float3(archive_max))?;

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
    // A surface appears exactly once in the hierarchy: retail's `terrain_p_09`
    // leaves `unassigned` empty and names its surfaces in LOD0. Assigning the
    // impostors to LOD1 instead would be the faithful shape, but a map whose
    // collision forces one surface per archive cannot use it -- an impostor and
    // the object it stands for land in different models, so nothing can swap
    // between them, and the impostor-only archives would declare an empty LOD0.
    let mut assigned = false;
    for level in levels {
        let surfaces = if level == "unassigned" || assigned {
            Vec::new()
        } else {
            assigned = true;
            surface_names.clone()
        };
        settings
            .root
            .set_path(&format!("lod_hierrarchy.{level}.surfaces"), ConfigValue::Indexed(surfaces))?;
    }
    if !assigned {
        bail!("{model}: template lod_hierrarchy declares no level to carry surfaces");
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

    // Collision lives *inside* a render archive, never beside one.
    //
    // v0.100b has 326 models, 236 of them carry collision, and not one is
    // collision-only. A standalone collision archive is a shape the engine has
    // never been asked to load, and it faults on a null resource while the map
    // loads. The recovered Cordon bundles tell the same story from the other
    // side: v1 kept render and collision in a single `cordon.model` and v3
    // split them, and no bundle from v3 onward was ever shown to run.
    let mut emitted_bindings = false;
    if let Some(payloads) = collision {
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
        emitted_bindings = true;
    }
    if let Some(payloads) = collision.filter(|_| emitted_bindings) {
        settings.root.set_path(
            "game_material_settings",
            ConfigValue::Named(
                payloads
                    .bindings
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
    } else {
        // No collision payloads means no face dictionary to bind; leaving the
        // template's would name faces this archive does not have.
        remove_named(&mut settings.root, "game_material_settings");
    }

    write_config(
        overlay,
        &format!("{model}/settings"),
        &settings,
        "generated:v0.100b-model-settings",
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

/// Emit this map's post-process material, with the skylight band re-anchored.
///
/// Sky light is a world-height ramp, not a constant: `stage_ambient_lighting`
/// hands the shader `(skylight_lower, skylight_upper - skylight_lower,
/// skylight_power)` and the surface is lit in proportion to where it sits
/// between the two. `level_03_day` sets `lower = 0`, `upper = 5`, tuned for
/// terrain that runs y = -0.6..3.97.
///
/// A Shadow of Chernobyl map does not share that convention. `mp_pool`'s deck
/// is at y = 0 and its pool floor at y = -4.69, so under the inherited band the
/// ground receives *exactly* zero sky light, the pool receives zero, and a
/// player's own hands go dark on the way down -- a sharp, sourceless cutoff at
/// the floor. With `power = 0.1` the ramp then rises so steeply that a scrap of
/// litter a few units up is already fully lit, which is what makes the edge
/// look like a hard line rather than a gradient.
///
/// The band's width is the artist's decision and is preserved; only its anchor
/// moves, to sit just under the map's lowest geometry.
pub(crate) fn write_post_process_material(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    extent: &MapExtent,
    files: &mut Vec<BuiltFile>,
) -> Result<String> {
    let mut material = load_template(target, &manifest.templates.post_process_material)?;

    let read = |material: &BinaryConfig, name: &str| -> Result<f32> {
        match named_path(&material.root, &format!("{SKYLIGHT_PREFIX}.{name}.value")) {
            Ok(ConfigValue::Float(value)) => Ok(*value),
            Ok(ConfigValue::Integer(value)) => Ok(*value as f32),
            _ => bail!(
                "post-process template {} has no numeric {name}",
                manifest.templates.post_process_material
            ),
        }
    };
    let band = (read(&material, "skylight_upper")? - read(&material, "skylight_lower")?).max(1.0);

    // Clear of the lowest geometry, so the deepest floor still catches the
    // bottom of the ramp instead of sitting exactly on zero.
    let lower = extent.min[1] - SKYLIGHT_FLOOR_CLEARANCE;
    material
        .root
        .set_path(&format!("{SKYLIGHT_PREFIX}.skylight_lower.value"), ConfigValue::Float(lower))?;
    material
        .root
        .set_path(&format!("{SKYLIGHT_PREFIX}.skylight_upper.value"), ConfigValue::Float(lower + band))?;

    let path = format!("material_instances/{}/post_process.material", manifest.namespace);
    write_config(
        overlay,
        &path,
        &material,
        "generated:v0.100b-post-process",
        files,
    )?;
    println!("skylight_band={lower:.2}..{:.2}", lower + band);
    Ok(format!("{}/post_process", manifest.namespace))
}

/// Where `g_stage_terrain` reads its layer blend weights.
const TERRAIN_MASK_TEXTURE: &str = "material.g_stage.effect.texture_mask.value";

/// How a forward material combines with what is already on screen.
///
/// `blend_mode.h`: 0 opaque, 1 translucent, 2 additive, 3 modulate, 4
/// subtractive. It is read straight into render state by
/// `effect_material_base.cpp` rather than turned into a shader macro, so
/// unlike almost everything else on a material it can be changed freely.
const FORWARD_BLEND_MODE: &str = "material.forward.effect.blend_mode.value";
const BLEND_MODE_ADDITIVE: i32 = 2;

/// The rest of a forward emissive material's colour contract.
const FORWARD_TRANSPARENCY_TEXTURE: &str = "material.forward.effect.texture_transparency.value";
const FORWARD_EMISSIVE_MULTIPLIER: &str =
    "material.forward.effect.constant_emissive_multiplier.value";

/// How often one terrain layer may repeat across its surface.
///
/// A repeat count, not a distance: the largest retail value is 60, and a
/// four-figure count on a level-sized surface aliases into noise long before
/// the constant itself becomes a problem.
const MAX_TERRAIN_TILE: i32 = 2000;

/// Where the tile count for one terrain layer sits.
fn terrain_tile_path(layer: usize) -> String {
    format!("material.g_stage.effect.constant_tile_{layer}.value")
}

/// The tiled ground texture for one terrain shader: declared, or derived.
fn terrain_layer_texture(manifest: &SocMapManifest, shader: &str) -> String {
    manifest
        .terrain_layer_textures
        .get(shader)
        .cloned()
        .unwrap_or_else(|| render_material::terrain_detail_texture(shader).to_owned())
}

/// Where a forward water material binds its reflection probe.
const WATER_REFLECTION_PATH: &str = "material.forward.effect.texture_reflection.value";

/// Where the skylight parameters sit inside a post-process material.
const SKYLIGHT_PREFIX: &str = "material.post_process";

/// How far under the map's lowest geometry the skylight ramp starts.
const SKYLIGHT_FLOOR_CLEARANCE: f32 = 1.0;

pub(crate) fn write_materials(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    plans: &[MaterialPlan],
    normals: &BTreeSet<String>,
    terrain: Option<&TerrainControls>,
    uv_density: &BTreeMap<u16, f32>,
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

        // Water is rendered by the target's own shader, so nothing from the
        // source is bound to it -- see `RenderFamily::Water`. What it does need
        // is a reflection probe that exists: `water/water_green` points at
        // `environment_probes/untitled_project/probe_50043953794066488`, which
        // this database does not contain, and a material that samples a missing
        // texture shows the engine's `missing texture` fallback.
        if plan.family == RenderFamily::Water {
            if named_path(&material.root, WATER_REFLECTION_PATH).is_ok() {
                material.root.set_path(
                    WATER_REFLECTION_PATH,
                    ConfigValue::String(manifest.water_reflection_probe.clone()),
                )?;
            }
            write_config(
                overlay,
                &plan.target_path(&manifest.namespace),
                &material,
                &format!("generated:v0.100b-water-from-SoC:{}", plan.source_shader),
                files,
            )?;
            continue;
        }

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
            //
            // Terrain is exempt. Its normals are per layer — `texture_normal_0`
            // through `_3` — and layer 0 is the colour map at tile 1, so a
            // normal map bound there is stretched over the whole level and
            // perturbs nothing. The bumps that matter are the template's, on
            // the tiled layers, and they are already bound.
            //
            // Emissive is exempt because it is forward-rendered: it has no
            // `g_stage` block at all, so `normal_path` names a field that does
            // not exist. An additive glow is not lit anyway.
            let normal_source = plan.source_normal.clone().or_else(|| {
                let bump = format!("{diffuse}_bump");
                normals.contains(&bump).then_some(bump)
            });
            if manifest.bind_normal_maps
                && plan.family != RenderFamily::Terrain
                && plan.family != RenderFamily::Emissive
                && let Some(normal_source) = normal_source
            {
                bind_texture(
                    &mut material,
                    &manifest.normal_path,
                    &source::target_texture_path(&manifest.namespace, &normal_source)?,
                    template_path,
                )?;
                material
                    .root
                    .set_path(&manifest.normal_enable_path, ConfigValue::Boolean(true))?;
            }
        }
        // Emissive surfaces are *added* to the scene, not placed in it.
        //
        // X-Ray's `effects\lightplanes` are the fake light shafts a renderer
        // without volumetrics used: a bright cone painted on a black field,
        // drawn additively, so the black contributes nothing. Given an opaque
        // material instead, the black is what you see -- mp_agroprom's shafts
        // became two storey-high black walls across the middle of a building,
        // with the lamp still glowing at their apex.
        //
        // `blend_mode` is the fix and it is free: `effect_material_base.cpp`
        // reads it into render state rather than into the shader key, so unlike
        // the `use_*` flags it can differ from the template. The template is
        // `weapons/tracer`, chosen for its base material (`forward_emissive`)
        // and translucent by default because a tracer round is.
        if plan.family == RenderFamily::Emissive {
            material.root.set_path(
                FORWARD_BLEND_MODE,
                ConfigValue::Integer(BLEND_MODE_ADDITIVE),
            )?;
            // The template multiplies a tracer by ten to make it carry at
            // range. A light shaft is already the brightness it should be.
            material
                .root
                .set_path(FORWARD_EMISSIVE_MULTIPLIER, ConfigValue::Integer(1))?;
            // `use_ttransparency` is a shader flag and stays as the template
            // set it, so the slot has to hold something: the same texture, as
            // the template itself does.
            if let Some(diffuse) = &plan.source_diffuse {
                bind_texture(
                    &mut material,
                    FORWARD_TRANSPARENCY_TEXTURE,
                    &source::target_texture_path(&manifest.namespace, diffuse)?,
                    template_path,
                )?;
            }
        }

        // Terrain: the source colour map becomes layer 0, and the template's
        // own tiled layers supply the detail the colour map cannot carry.
        //
        // X-Ray stretches one unique image across the whole level -- Cordon's
        // `escape_grass` covers 1,445 world units in one UV unit, about 1.4m
        // per texel -- and blends a tiled detail texture over it at close
        // range. Converted as a plain diffuse the second half is simply lost,
        // and the ground reads as flat brown with the triangle facets showing
        // through: what looks like low-poly geometry is in fact
        // S.T.A.L.K.E.R.'s own 153,706 triangles under a texture with no
        // detail left at walking distance.
        //
        // `g_stage_terrain` rebuilds the same idea from the target's parts.
        // Layer 0 is the colour map at tile 1, layers 1..3 are the template's
        // tiled ground textures, and the generated mask decides the mix. Only
        // textures and tile constants change: `use_tdetail`, `use_nmap`,
        // `use_height_map_*`, `num_used_terrain_layers` and
        // `terrain_blend_mode` are the config values
        // `effect_gstage_terrain_materials::compile` turns into shader macros,
        // and each one is part of the permutation key in `shaders/masks`.
        if plan.family == RenderFamily::Terrain {
            let controls = terrain.expect("a terrain plan implies generated terrain controls");
            material.root.set_path(
                TERRAIN_MASK_TEXTURE,
                ConfigValue::String(controls.mask.clone()),
            )?;
            // The colour map is not tiled: X-Ray laid it out to cover the level
            // exactly once.
            material.root.set_path(
                &terrain_tile_path(0),
                ConfigValue::Integer(manifest.terrain_base_tile),
            )?;
            // Layer 1 is X-Ray's own ground detail for this shader, which is
            // the half of its terrain the colour map never carried.
            let detail = terrain_layer_texture(manifest, &plan.source_shader);
            bind_texture(
                &mut material,
                "material.g_stage.effect.texture_diffuse_1.value",
                &source::target_texture_path(&manifest.namespace, &detail)?,
                template_path,
            )?;
            if manifest.bind_normal_maps && normals.contains(&format!("{detail}_bump")) {
                bind_texture(
                    &mut material,
                    "material.g_stage.effect.texture_normal_1.value",
                    &source::target_texture_path(
                        &manifest.namespace,
                        &format!("{detail}_bump"),
                    )?,
                    template_path,
                )?;
            }
            // The tiled layers repeat at a fixed size on the ground rather than
            // a fixed number of times, because the UV range a terrain shader
            // covers differs per shader and per map. `constant_tile_N` is a
            // repeat count over the mesh UV, so the count that gives a
            // metre-sized repeat is the shader's world span per UV unit divided
            // by the size wanted.
            let density = uv_density.get(&plan.shader_id).copied().unwrap_or(0.0);
            if density > 0.0 && manifest.terrain_layer_metres > 0.0 {
                let tile = (density / manifest.terrain_layer_metres).round();
                let tile = (tile as i32).clamp(1, MAX_TERRAIN_TILE);
                for layer in 1..4 {
                    material
                        .root
                        .set_path(&terrain_tile_path(layer), ConfigValue::Integer(tile))?;
                }
            }
            // `use_tdetail` cannot be cleared, so the slot is rebound to a flat
            // grey rather than left holding level_03's ground photo.
            for slot in 0..2 {
                material.root.set_path(
                    &format!("material.g_stage.effect.texture_detail_{slot}.value"),
                    ConfigValue::String(controls.detail.clone()),
                )?;
                material.root.set_path(
                    &format!("material.g_stage.effect.constant_detail_tile_{slot}.value"),
                    ConfigValue::Integer(1),
                )?;
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

/// One `4x4` DXT5 image of a single colour, in the target's container.
///
/// Both terrain control textures are constants — a blend mask with no spatial
/// variation, and a flat exposure multiplier — so this generates them rather
/// than shipping image files. DXT5 rather than the smaller uncompressed layout
/// because it is the format retail already uses in these slots
/// (`terrain/03_base_mask` is DXT5) and so is the one the loader is known to
/// accept; a constant colour costs one block either way.
///
/// A DXT5 block whose two endpoints are equal decodes to that colour for every
/// index, so both index fields are left zero. Endpoint precision is 5:6:5, and
/// the mask survives it because the shader `normalize()`s the weights and only
/// their ratio matters.
fn write_constant_texture(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    name: &str,
    rgba: [u8; 4],
    label: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<String> {
    const SIDE: u32 = 4;
    const BLOCK: u32 = 16;

    let [red, green, blue, alpha] = rgba;
    let quantise = |value: u8, bits: u32| -> u16 {
        let steps = f32::from((1_u16 << bits) - 1);
        (f32::from(value) / 255.0 * steps).round() as u16
    };
    let colour = (quantise(red, 5) << 11) | (quantise(green, 6) << 5) | quantise(blue, 5);

    let mut block = Vec::with_capacity(BLOCK as usize);
    block.extend_from_slice(&[alpha, alpha]); // both alpha endpoints
    block.extend_from_slice(&[0; 6]); // alpha indices: endpoint 0 throughout
    block.extend_from_slice(&colour.to_le_bytes());
    block.extend_from_slice(&colour.to_le_bytes());
    block.extend_from_slice(&[0; 4]); // colour indices: endpoint 0 throughout

    // The same header shape `source::canonicalize_dds` writes: linear size over
    // pitch, an explicit mip count, a cleared reserved block, and one trailing
    // colour-space byte.
    const DDSD_CAPS: u32 = 0x1;
    const DDSD_HEIGHT: u32 = 0x2;
    const DDSD_WIDTH: u32 = 0x4;
    const DDSD_PIXELFORMAT: u32 = 0x1000;
    const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
    const DDSD_LINEARSIZE: u32 = 0x8_0000;

    let mut dds = Vec::with_capacity(128 + block.len() + 1);
    dds.extend_from_slice(b"DDS ");
    dds.extend_from_slice(&124_u32.to_le_bytes());
    dds.extend_from_slice(
        &(DDSD_CAPS
            | DDSD_HEIGHT
            | DDSD_WIDTH
            | DDSD_PIXELFORMAT
            | DDSD_MIPMAPCOUNT
            | DDSD_LINEARSIZE)
            .to_le_bytes(),
    );
    dds.extend_from_slice(&SIDE.to_le_bytes()); // height
    dds.extend_from_slice(&SIDE.to_le_bytes()); // width
    dds.extend_from_slice(&BLOCK.to_le_bytes()); // linear size
    dds.extend_from_slice(&0_u32.to_le_bytes()); // depth
    dds.extend_from_slice(&1_u32.to_le_bytes()); // mip levels
    dds.extend_from_slice(&[0; 44]); // reserved, cleared
    dds.extend_from_slice(&32_u32.to_le_bytes()); // pixel format size
    dds.extend_from_slice(&0x4_u32.to_le_bytes()); // DDPF_FOURCC
    dds.extend_from_slice(b"DXT5");
    dds.extend_from_slice(&[0; 20]); // bit counts and channel masks
    dds.extend_from_slice(&0x1000_u32.to_le_bytes()); // caps: TEXTURE
    dds.extend_from_slice(&[0; 16]); // caps2..reserved2
    dds.extend_from_slice(&block);
    // Control data, not colour: sampled raw, never de-gamma'd. Retail agrees --
    // every `terrain/*` texture records `srgb: 0`.
    dds.push(0);

    write_resource(
        overlay,
        &format!("textures/{}/{name}.dds", manifest.namespace),
        &dds,
        label,
        files,
    )?;

    let mut options = load_template(target, &manifest.templates.texture_options)?;
    options
        .root
        .set_path("options.format", ConfigValue::String("DXT5".into()))?;
    options
        .root
        .set_path("options.has_mip_levels", ConfigValue::Integer(0))?;
    options.root.set_path("options.srgb", ConfigValue::Integer(0))?;
    write_config(
        overlay,
        &format!("textures/{}/{name}.options", manifest.namespace),
        &options,
        label,
        files,
    )?;

    Ok(format!("{}/{name}", manifest.namespace))
}

/// The two constants a converted terrain material needs, and their paths.
pub(crate) struct TerrainControls {
    mask: String,
    detail: String,
}

/// Generate the blend mask and the exposure multiplier for this map's terrain.
///
/// `terrain_gbuffer_pass` weights its four layers by a mask sampled at the
/// *untiled* mesh UV, and level_03's mask is a 2048x2048 painting of level_03's
/// own ground — white where its terrain is, black under its buildings.
/// Inherited, it stamps another map's layout onto this one, so the port
/// generates its own: one colour, every texel.
///
/// The second constant is there because `use_tdetail` cannot be turned off.
/// It is one of the fourteen macros in `shaders/masks` that form this shader's
/// permutation, so clearing it asks for a shader build 802 never compiled; the
/// slot has to stay bound, and the template binds `terrain/03_base` — level_03's
/// ground photo — stretched across whatever UV range it is given. A flat grey
/// takes its place, which also pays for the mask's normalisation:
///
/// ```hlsl
/// mask = normalize(mask);           // L2, so live weights sum to > 1
/// diffuse = mask.x * diffuse_0 + mask.y * diffuse_1 + ...;
/// out_color = min(detail_0.x, detail_1.x) * (float4(diffuse, 1) + ...);
/// ```
///
/// A 60/40 mask sums to 1.34 after normalisation, and the ground would render
/// a third too bright. The grey is `1 / that sum`, so the two layers add back
/// to exactly the exposure their textures were painted at.
fn write_terrain_controls(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<TerrainControls> {
    let weights = manifest.terrain_layer_weights;
    let length = weights
        .iter()
        .map(|weight| f32::from(*weight).powi(2))
        .sum::<f32>()
        .sqrt();
    if length <= 0.0 {
        bail!("terrain_layer_weights are all zero, so every terrain layer is weighted out");
    }
    let weight_sum = weights.iter().map(|weight| f32::from(*weight)).sum::<f32>() / length;

    let exposure = (manifest.terrain_exposure / weight_sum).clamp(0.0, 1.0);
    let grey = (exposure * 255.0).round() as u8;
    println!(
        "terrain_mask={weights:?} normalised_sum={weight_sum:.3} exposure_grey={grey}"
    );

    Ok(TerrainControls {
        mask: write_constant_texture(
            manifest,
            target,
            overlay,
            "terrain_mask",
            weights,
            "generated:v0.100b-terrain-blend-mask",
            files,
        )?,
        detail: write_constant_texture(
            manifest,
            target,
            overlay,
            "terrain_exposure",
            [grey, grey, grey, 255],
            "generated:v0.100b-terrain-exposure",
            files,
        )?,
    })
}

pub(crate) fn write_textures(
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
        let is_normal = texture.is_normal;
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

pub(crate) fn write_projects(
    manifest: &SocMapManifest,
    target: &mut ResourceDb,
    overlay: &Path,
    models: &[String],
    collision_models: &[String],
    extent: &MapExtent,
    post_effect: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<BTreeMap<String, usize>> {
    let bindings = &manifest.project_bindings;

    let mut client = load_template(target, &manifest.templates.client_project)?;
    let dropped_objects =
        filter_template_objects(manifest, &mut client.root, extent, post_effect)?;
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
    // With the collision archive suppressed the template keeps its own
    // collision, which belongs to another map -- deliberately wrong, and only
    // useful for isolating the archive.
    if manifest.emit_collision {
        let collision = rebuild_instances(
            &client.root,
            &bindings.collision_array_path,
            &bindings.instance_model_path,
            collision_models,
        )?;
        client
            .root
            .set_path(&bindings.collision_array_path, ConfigValue::Indexed(collision))?;
    }
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
    extent: &MapExtent,
    post_effect: &str,
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
    let mut anchored = 0_usize;
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
        // A `light` is positional -- except the sun. The template's 19 lights
        // are one directional sun (`lib_name: "sun"`, `light_type: 4`, four
        // shadow cascades) and eighteen local lamps placed around level_03.
        // Dropping the lot leaves a map lit only by sky ambient, which reaches
        // open geometry but not enclosed ground: the map goes black below the
        // height where the sky stops being visible. The sun's own position is
        // meaningless, so it travels; the lamps do not.
        let is_positional_lamp = kind == "light"
            && !matches!(entry.named("lib_name"), Some(ConfigValue::String(name)) if name == "sun");
        if retained.contains(kind.as_str()) && !is_positional_lamp {
            // A retained object is retained for its settings -- but the
            // atmosphere objects anchor those settings to a *place*, and the
            // place they carry is the template map's.
            //
            // `environment` sits at (0, 0, 0) in `level_03`, whose terrain
            // runs y = -0.595..3.967, so that map's ground is above its own
            // anchor. Imported unmoved into a map whose floor is at y = 0, the
            // anchor lands exactly on the floor: the floor and the pool below
            // it lose ambient entirely, a scrap of paper a few units higher
            // keeps it, and a player's own hands go dark on the way down. The
            // cutoff is perfectly sharp and has no visible source.
            //
            // `volume_fog` carries the same problem in its own terms: its
            // position is the height fog collects at.
            //
            // So the atmosphere is re-anchored under this map, clear of its
            // lowest geometry by [`SocMapManifest::lighting_volume_margin`] --
            // a volume whose floor coincides with the map's own leaves the
            // lowest surfaces on the boundary, where a rounding difference
            // decides whether they are lit. The sun is exempt: it is
            // directional, and its position means nothing.
            let mut entry = entry.clone();
            if kind != "light" && entry.named("position").is_some() {
                entry.set_path(
                    "position",
                    ConfigValue::Float3([
                        extent.center()[0],
                        extent.min[1] - manifest.lighting_volume_margin,
                        extent.center()[2],
                    ]),
                )?;
                anchored += 1;
            }
            // The environment object names the post-process material that
            // carries the skylight band; point it at this map's own.
            if entry.named("post_effect").is_some() {
                entry.set_path("post_effect", ConfigValue::String(post_effect.to_owned()))?;
            }
            kept.push(entry);
        } else {
            // Distinguish a lamp dropped for its place from a type that was
            // never retained at all.
            let label = if is_positional_lamp && retained.contains(kind.as_str()) {
                "light:lamp".to_owned()
            } else {
                kind
            };
            *dropped.entry(label).or_insert(0) += 1;
        }
    }
    if anchored > 0 {
        println!("re_anchored_template_objects={anchored}");
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
            // The prototype is one of the template map's own objects and
            // carries its placement -- for `level_03` that is a lamp at
            // (-49.834, 8.897, 8.074). A converted map's geometry is already
            // in world coordinates, so an inherited transform silently offsets
            // the entire map from the coordinates the server spawns players in.
            for (field, identity) in [
                ("position", [0.0, 0.0, 0.0]),
                ("rotation", [0.0, 0.0, 0.0]),
                ("scale", [1.0, 1.0, 1.0]),
            ] {
                if entry.named(field).is_some() {
                    entry.set_path(field, ConfigValue::Float3(identity))?;
                }
            }
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

/// Rewrite a model reference in the shape the surrounding array already uses.
///
/// The two arrays of a client project do not agree. `render_visuals` names a
/// model as `props/light/green_lamp_with_wire`, while `collision_objects` names
/// the same model as
/// `resources/models/props/light/green_lamp_with_wire.model/collision` -- a
/// full resource path, down to the sub-resource. Writing the render shape into
/// the collision array asks the engine to cook a path that resolves to nothing,
/// and it faults on the result it never checks:
///
/// ```text
/// resources <Warning>  cook of 'ported/soc/mp_pool/mp_pool_000' [...] failed
/// debug     <ERROR>    ACCESS_VIOLATION
/// ```
///
/// So the template value is read for all four of its conventions -- the
/// `resources/` root, the `models/` prefix, the `.model` extension and any
/// sub-resource suffix -- and each is reproduced.
fn model_reference_like(template_value: &str, target_path: &str) -> String {
    let (root, rest) = match template_value.strip_prefix("resources/") {
        Some(rest) => ("resources/", rest),
        None => ("", template_value),
    };
    let (core, suffix) = match rest.find(".model") {
        Some(at) => rest.split_at(at + ".model".len()),
        None => (rest, ""),
    };

    let mut reference = target_path.to_owned();
    if !core.ends_with(".model") {
        reference = reference
            .strip_suffix(".model")
            .unwrap_or(&reference)
            .to_owned();
    }
    if !core.starts_with("models/") {
        reference = reference
            .strip_prefix("models/")
            .unwrap_or(&reference)
            .to_owned();
    }
    format!("{root}{reference}{suffix}")
}

pub(crate) fn load_template(target: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    let bytes = target
        .read(path)
        .with_context(|| format!("reading template {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing template {path}"))
}

pub(crate) fn write_resource(
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

pub(crate) fn write_config(
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

    /// The collision array names a model by full resource path down to the
    /// sub-resource, unlike the render array. Reproducing only the render
    /// shape names a path that resolves to nothing, and the cook fails.
    #[test]
    fn reproduces_a_full_collision_sub_resource_reference() {
        assert_eq!(
            model_reference_like(
                "resources/models/props/light/green_lamp_with_wire.model/collision",
                "models/ported/soc/cordon/y.model"
            ),
            "resources/models/ported/soc/cordon/y.model/collision"
        );
        assert_eq!(
            model_reference_like(
                "models/props/light/lamp.model/collision",
                "models/ported/soc/cordon/y.model"
            ),
            "models/ported/soc/cordon/y.model/collision"
        );
        // The render array's own shape must survive unchanged.
        assert_eq!(
            model_reference_like(
                "props/light/green_lamp_with_wire",
                "models/ported/soc/cordon/y.model"
            ),
            "ported/soc/cordon/y"
        );
    }

    /// A cloned prototype carries the template map's placement; a converted
    /// map's geometry is already in world coordinates.
    #[test]
    fn clears_the_prototype_placement() {
        let float3 = |name: &str, value: [f32; 3]| NamedValue {
            name: name.into(),
            value: ConfigValue::Float3(value),
        };
        // A prototype shaped like `level_03`'s: a lamp, placed where that map
        // put it, referenced down to its collision sub-resource.
        let root = ConfigValue::Named(vec![NamedValue {
            name: "collision_objects".into(),
            value: ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                float3("position", [-49.834, 8.897, 8.074]),
                float3("rotation", [0.0, 90.0, 0.0]),
                float3("scale", [2.0, 2.0, 2.0]),
                NamedValue {
                    name: "lib_name".into(),
                    value: ConfigValue::String(
                        "resources/models/props/light/green_lamp_with_wire.model/collision".into(),
                    ),
                },
            ])]),
        }]);
        let entries = rebuild_instances(
            &root,
            "collision_objects",
            "lib_name",
            &["models/ported/soc/cordon/cordon_000.model".to_owned()],
        )
        .unwrap();
        assert_eq!(
            entries[0].named("lib_name"),
            Some(&ConfigValue::String(
                "resources/models/ported/soc/cordon/cordon_000.model/collision".into()
            ))
        );
        let entry = entries.first().expect("one instance");
        assert_eq!(entry.named("position"), Some(&ConfigValue::Float3([0.0; 3])));
        assert_eq!(entry.named("rotation"), Some(&ConfigValue::Float3([0.0; 3])));
        assert_eq!(entry.named("scale"), Some(&ConfigValue::Float3([1.0; 3])));
    }

    fn test_extent() -> MapExtent {
        MapExtent {
            min: [-10.0, -4.0, -20.0],
            max: [10.0, 6.0, 20.0],
        }
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

    /// The sun is the one light worth importing: it is directional, so its
    /// position means nothing, and without it a map is lit by sky ambient
    /// alone and goes black wherever the sky is not visible.
    #[test]
    fn keeps_the_sun_but_not_the_lamps() {
        let light = |lib_name: &str| {
            ConfigValue::Named(vec![
                NamedValue {
                    name: "game_object_type".into(),
                    value: ConfigValue::String("light".into()),
                },
                NamedValue {
                    name: "lib_name".into(),
                    value: ConfigValue::String(lib_name.into()),
                },
                NamedValue {
                    name: "position".into(),
                    value: ConfigValue::Float3([14.132, 2.792, 7.879]),
                },
            ])
        };
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "objects".into(),
            value: ConfigValue::Indexed(vec![light("sun"), light("green_lamp"), light("lamp_02")]),
        }]);

        let mut manifest = crate::soc::manifest::sample_manifest();
        manifest.project_bindings.retained_object_types = vec!["light".to_owned()];
        let dropped = filter_template_objects(&manifest, &mut root, &test_extent(), "ported/soc/x/post_process").unwrap();
        assert_eq!(dropped["light:lamp"], 2);

        let ConfigValue::Indexed(kept) = named_path(&root, "objects").unwrap() else {
            panic!("objects is not indexed");
        };
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].named("lib_name"),
            Some(&ConfigValue::String("sun".into()))
        );
        // The sun is not fog: its position is left exactly as authored.
        assert_eq!(
            kept[0].named("position"),
            Some(&ConfigValue::Float3([14.132, 2.792, 7.879]))
        );
    }

    #[test]
    fn positional_template_objects_are_dropped_and_global_ones_kept() {
        let manifest = crate::soc::manifest::sample_manifest();
        let mut root = objects_template();
        let dropped = filter_template_objects(&manifest, &mut root, &test_extent(), "ported/soc/x/post_process").unwrap();

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
        let dropped = filter_template_objects(&manifest, &mut root, &test_extent(), "ported/soc/x/post_process").unwrap();
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
        let error = filter_template_objects(&manifest, &mut root, &test_extent(), "ported/soc/x/post_process")
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
