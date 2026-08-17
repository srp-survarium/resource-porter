//! Emit a v0.100b bundle from a compiled F.E.A.R. multiplayer world.
//!
//! This module owns the only side effects in the lilith path. The emit half
//! is the SoC path's, called directly -- model archives, materials, textures,
//! post-process and projects are written by the same code that built the
//! thirteen SoC maps, so the two families of ports cannot drift apart. What
//! differs is everything before that: the decode ([`super::codec`]) and the
//! conversion decisions ([`super::convert`]).
//!
//! The manifest is [`SocMapManifest`] unchanged. Its `source_level` names a
//! multiplayer world (`office`), and its `render_shader_families` keys are
//! `.fx` paths rather than X-Ray shader names; everything else -- templates,
//! bindings, material templates per family -- means exactly what it means for
//! a SoC map, which is the point.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::ResourceDb;
use crate::manifest::BuiltFile;
use crate::soc::build::{
    MapExtent, TexturePlan, write_materials, write_model_archive, write_post_process_material,
    write_projects, write_textures,
};
use crate::soc::convert::collision as soc_collision;
use crate::soc::convert::game_material::{self, CollisionMaterialBinding};
use crate::soc::convert::render_material::{MaterialPlan, RenderFamily};
use crate::soc::convert::surface as soc_surface;
use crate::soc::manifest::SocMapManifest;
use crate::soc::source as soc_source;
use crate::weapon::reject_duplicate_outputs;

use super::codec::{mat, world};
use super::convert::{collision, render_material, surface};
use super::probe;
use super::source::LilithSource;
use super::SCALE;

/// What one map build produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LilithMapBuildRecord {
    pub asset_id: String,
    pub source_world: String,
    pub project_name: String,
    pub client_project: String,
    pub server_project: String,
    pub models: Vec<String>,
    pub collision_models: Vec<String>,
    pub converted_surfaces: usize,
    pub rendered_triangles: usize,
    pub textures: usize,
    pub materials: usize,
    pub collision_pieces: usize,
    pub collision_vertices: usize,
    pub collision_triangles: usize,
    pub collision_materials: Vec<CollisionMaterialBinding>,
    pub collision_report: collision::CollisionReport,
    pub merge_report: surface::MergeReport,
    /// Materials whose shader the classification declined, with the shader.
    pub dropped_materials: BTreeMap<String, usize>,
    pub spawn_position: [f32; 3],
    /// Every authored `GameStartPoint`, in metres, for choosing spawns.
    pub start_points: Vec<(Option<String>, [f32; 3])>,
    pub map_extent: MapExtent,
    pub dropped_template_objects: BTreeMap<String, usize>,
    pub bound_normal_maps: usize,
    pub files: Vec<BuiltFile>,
}

/// Build the additive bundle for one multiplayer world.
pub fn build_bundle(
    manifest: &SocMapManifest,
    source_root: &Path,
    target_db: &Path,
    output: &Path,
) -> Result<LilithMapBuildRecord> {
    manifest.validate()?;
    if output.exists() {
        bail!("bundle output {} already exists", output.display());
    }
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)
        .with_context(|| format!("creating bundle overlay {}", overlay.display()))?;

    let mut source = LilithSource::open(source_root)?;
    let mut target = ResourceDb::open(target_db)?;

    // --- decode -----------------------------------------------------------
    let world_path = probe::resolve_world(&source, &manifest.source_level)?;
    let bytes = source.read(&world_path)?;
    let world = world::parse(&bytes, &world_path)?;

    let mut materials = Vec::with_capacity(world.render.materials.len());
    let mut missing = Vec::new();
    for path in &world.render.materials {
        match source.read(path) {
            Ok(bytes) => materials.push(Some(mat::parse(&bytes, path)?)),
            Err(_) => {
                missing.push(path.clone());
                materials.push(None);
            }
        }
    }
    if !missing.is_empty() {
        // Retail data carries a handful of dangling references (the engine
        // renders its own fallback there). They stay fail-closed until each
        // one is reviewed into `render_shader_families` under a substitute.
        bail!(
            "{world_path} references materials no mounted archive carries: {missing:?}"
        );
    }

    // --- convert ----------------------------------------------------------
    let overrides = &manifest.render_shader_families;
    let mut plans = Vec::new();
    let mut dropped = BTreeSet::new();
    let mut dropped_materials: BTreeMap<String, usize> = BTreeMap::new();
    for (index, material) in materials.iter().enumerate() {
        let material = material.as_ref().expect("missing materials bailed above");
        let surface_count = world
            .render
            .surfaces
            .iter()
            .filter(|surface| surface.material as usize == index)
            .count();
        if surface_count == 0 {
            continue;
        }
        let Some(family) = render_material::classify(&material.shader, overrides)? else {
            dropped.insert(index as u32);
            *dropped_materials
                .entry(material.shader.to_ascii_lowercase())
                .or_insert(0) += surface_count;
            continue;
        };
        // The forward emissive family binds what the source calls an emissive
        // map; everything else binds the diffuse. `specular_black.dds` is the
        // authored "none" texture, kept as-is: black adds nothing, exactly as
        // the source intended.
        let bound = match family {
            RenderFamily::Emissive => material.emissive().or(material.diffuse()),
            _ => material.diffuse(),
        };
        let Some(bound) = bound else {
            bail!(
                "{}: family {} has no texture to bind (shader {})",
                world.render.materials[index],
                family.as_str(),
                material.shader
            );
        };
        let source_diffuse = strip_dds(bound).to_owned();
        // Normal maps are bound per material by name -- materials share
        // diffuse textures (the black "none" texture above all) while
        // carrying their own normals, so the SoC `<diffuse>_bump` convention
        // cannot name them.
        let source_normal = (manifest.bind_normal_maps
            && family != RenderFamily::Emissive
            && family != RenderFamily::Water)
            .then(|| material.normal().map(strip_dds).map(str::to_owned))
            .flatten();
        plans.push(MaterialPlan {
            shader_id: u16::try_from(index).context("material index exceeds u16")?,
            name: format!("shader_{index:04}"),
            source_shader: material.shader.clone(),
            family,
            source_diffuse: Some(source_diffuse),
            source_normal,
            dropped_lightmaps: Vec::new(),
            surface_count,
        });
    }
    if plans.is_empty() {
        bail!("every material was dropped; nothing to convert");
    }

    let (surfaces, merge_report) = surface::merge_surfaces(&world.render, &dropped, &world_path)?;
    let solid_materials: BTreeSet<u32> = plans
        .iter()
        .filter(|plan| collision::family_is_solid(plan.family))
        .map(|plan| u32::from(plan.shader_id))
        .collect();
    let (cform, collision_report) = collision::build_cform(
        &world.world_models,
        &world.render,
        &solid_materials,
        &world_path,
    )?;

    let target_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let collision_target = manifest
        .collision_material_overrides
        .get(WORLD_MATERIAL)
        .map(String::as_str)
        .unwrap_or(DEFAULT_COLLISION_TARGET);
    let target_id = *target_materials.get(collision_target).with_context(|| {
        format!("the target database has no physical material {collision_target:?}")
    })?;
    let bindings = vec![CollisionMaterialBinding {
        source_id: 0,
        source_name: WORLD_MATERIAL.to_owned(),
        face_name: game_material::face_dictionary_name(0),
        target_id,
        target_name: collision_target.to_owned(),
        triangle_count: cform.faces.len(),
    }];
    let collision_pieces = soc_collision::split(&cform, manifest.max_collision_triangles)
        .iter()
        .map(|piece| soc_collision::encode(piece, &bindings, manifest.collision_face_dictionary))
        .collect::<Result<Vec<_>>>()?;

    let textures = plan_textures(manifest, &mut source, &plans)?;

    // --- emit -------------------------------------------------------------
    let mut files = Vec::new();
    let pieces = if manifest.emit_collision {
        collision_pieces.len()
    } else {
        0
    };
    let surfaces_per_model = if pieces > 1 {
        manifest.surfaces_per_model.min(surfaces.len() / pieces).max(1)
    } else {
        manifest.surfaces_per_model
    };
    let model_groups = soc_surface::assign_models(&surfaces, surfaces_per_model)?;
    if model_groups.len() < pieces {
        bail!(
            "{} collision pieces need at least that many model archives, but {} surfaces \
             produce only {}; lower max_collision_triangles or split the map",
            pieces,
            surfaces.len(),
            model_groups.len()
        );
    }
    let uv_scale: BTreeMap<u16, f32> = plans
        .iter()
        .filter_map(|plan| {
            manifest
                .family_uv_scale
                .get(&plan.family)
                .map(|scale| (plan.shader_id, *scale))
        })
        .collect();
    let lift: BTreeMap<u16, f32> = plans
        .iter()
        .filter(|plan| plan.family == RenderFamily::Decal)
        .map(|plan| (plan.shader_id, manifest.decal_lift))
        .collect();
    let mut models = Vec::with_capacity(model_groups.len());
    for (index, group) in model_groups.iter().enumerate() {
        let path = manifest.model_path(index);
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

    let uv_density = soc_surface::uv_world_density(&surfaces);
    // Every normal binding travels on its plan's `source_normal`; the
    // convention set the SoC path uses stays empty.
    write_materials(
        manifest,
        &mut target,
        &overlay,
        &plans,
        &BTreeSet::new(),
        None,
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

    let start_points = world
        .objects
        .iter()
        .filter(|object| object.class == "GameStartPoint")
        .map(|object| {
            (
                object.name().map(str::to_owned),
                object
                    .position()
                    .unwrap_or_default()
                    .map(|axis| axis * SCALE),
            )
        })
        .collect();

    let record = LilithMapBuildRecord {
        asset_id: manifest.asset_id.clone(),
        source_world: world_path,
        project_name: manifest.project_name.clone(),
        client_project: manifest.client_project_path(),
        server_project: manifest.server_project_path(),
        models,
        collision_models,
        converted_surfaces: surfaces.len(),
        rendered_triangles: merge_report.triangles,
        textures: textures.len(),
        materials: plans.len(),
        collision_pieces: collision_pieces.len(),
        collision_vertices: collision_pieces.iter().map(|piece| piece.vertex_count).sum(),
        collision_triangles: collision_pieces.iter().map(|piece| piece.triangle_count).sum(),
        collision_materials: bindings,
        collision_report,
        merge_report,
        dropped_materials,
        spawn_position: manifest.spawn_position,
        start_points,
        map_extent: extent,
        dropped_template_objects: dropped_objects,
        bound_normal_maps: plans
            .iter()
            .filter(|plan| plan.source_normal.is_some())
            .count(),
        files,
    };
    let manifest_path = output.join("port-manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("writing {}", manifest_path.display()))?;
    Ok(record)
}

/// The pseudo-material every collision face carries; see
/// [`super::convert::collision`].
const WORLD_MATERIAL: &str = "fear\\world";
/// Urban maps underfoot: concrete unless the manifest reviews otherwise.
const DEFAULT_COLLISION_TARGET: &str = "concrete";

fn strip_dds(path: &str) -> &str {
    path.strip_suffix(".dds")
        .or_else(|| path.strip_suffix(".DDS"))
        .unwrap_or(path)
}

/// Read, canonicalize and place every texture the plans bind.
fn plan_textures(
    manifest: &SocMapManifest,
    source: &mut LilithSource,
    plans: &[MaterialPlan],
) -> Result<Vec<TexturePlan>> {
    let mut planned: BTreeMap<String, TexturePlan> = BTreeMap::new();
    for plan in plans {
        let diffuse = plan
            .source_diffuse
            .as_ref()
            .expect("every lilith plan binds a texture");
        add_texture(manifest, source, &mut planned, diffuse, diffuse, false)?;
        if let Some(normal) = &plan.source_normal {
            add_texture(manifest, source, &mut planned, normal, normal, true)?;
        }
    }
    Ok(planned.into_values().collect())
}

/// Plan one texture under its target name, refusing conflicting sources.
fn add_texture(
    manifest: &SocMapManifest,
    source: &mut LilithSource,
    planned: &mut BTreeMap<String, TexturePlan>,
    target_name: &str,
    source_path: &str,
    is_normal: bool,
) -> Result<()> {
    let target_dds = soc_source::target_texture_path(&manifest.namespace, target_name)?;
    let bytes = source
        .read(&format!("{source_path}.dds"))
        .with_context(|| format!("reading source texture {source_path}.dds"))?;
    if let Some(existing) = planned.get(&target_dds) {
        if existing.source_name != source_path || existing.is_normal != is_normal {
            bail!(
                "{target_dds} would be written from both {} (normal: {}) and \
                 {source_path} (normal: {is_normal})",
                existing.source_name,
                existing.is_normal
            );
        }
        return Ok(());
    }
    let info = soc_source::parse_dds(&bytes, source_path)?;
    let bytes = soc_source::canonicalize_dds(&bytes, info, !is_normal)?;
    let target_options = target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    planned.insert(
        target_dds.clone(),
        TexturePlan {
            source_name: source_path.to_owned(),
            target_dds,
            target_options,
            info,
            bytes,
            is_normal,
        },
    );
    Ok(())
}
