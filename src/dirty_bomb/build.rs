// SPDX-License-Identifier: GPL-3.0-or-later
//! Emit a Survarium v0.100b bundle from a cooked Dirty Bomb map graph.
//!
//! Geometry, placement and collision are source-derived. The first milestone
//! intentionally clones one declared opaque target material per UE3 material
//! slot; texture and material-instance translation can replace those clones
//! without changing the model/project contract proven here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::ResourceDb;
use crate::manifest::BuiltFile;
use crate::soc::build::{
    MapExtent, write_materials, write_model_archive, write_post_process_material, write_projects,
    write_textures,
};
use crate::soc::convert::collision as soc_collision;
use crate::soc::convert::game_material::{self, CollisionMaterialBinding};
use crate::soc::convert::render_material::RenderFamily;
use crate::soc::convert::surface as soc_surface;
use crate::soc::manifest::SocMapManifest;
use crate::weapon::reject_duplicate_outputs;

use super::convert::{self, ConversionReport};
use super::material::{self, MaterialAssetReport};
use super::source::DirtyBombSource;

const COLLISION_SOURCE: &str = "dirty_bomb_default";
const DEFAULT_COLLISION_TARGET: &str = "concrete";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirtyBombMapBuildRecord {
    pub asset_id: String,
    pub source_map: String,
    pub project_name: String,
    pub client_project: String,
    pub server_project: String,
    pub models: Vec<String>,
    pub collision_models: Vec<String>,
    pub converted_surfaces: usize,
    pub rendered_triangles: usize,
    pub materials: usize,
    pub textures: usize,
    /// This milestone preserves source material-slot identity but deliberately
    /// uses the manifest's opaque target template until UE3 texture graphs are
    /// translated.
    pub placeholder_materials: bool,
    pub source_materials: BTreeMap<String, u16>,
    pub material_assets: MaterialAssetReport,
    pub collision_pieces: usize,
    pub collision_vertices: usize,
    pub collision_triangles: usize,
    pub collision_materials: Vec<CollisionMaterialBinding>,
    pub conversion: ConversionReport,
    pub spawn_position: [f32; 3],
    pub map_extent: MapExtent,
    pub dropped_template_objects: BTreeMap<String, usize>,
    pub files: Vec<BuiltFile>,
}

pub fn build_bundle(
    manifest: &SocMapManifest,
    source_root: &Path,
    target_db: &Path,
    output: &Path,
) -> Result<DirtyBombMapBuildRecord> {
    manifest.validate()?;
    if output.exists() {
        bail!("bundle output {} already exists", output.display());
    }
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)
        .with_context(|| format!("creating bundle overlay {}", overlay.display()))?;

    let source = DirtyBombSource::open(source_root)?;
    let converted = convert::convert_map(&source, &manifest.source_level)?;
    let mut target = ResourceDb::open(target_db)?;

    let mut surface_counts = BTreeMap::new();
    for surface in &converted.surfaces {
        *surface_counts.entry(surface.shader_id).or_insert(0) += 1;
    }
    let material_assets = material::plan_assets(
        &source,
        manifest,
        &converted.report.materials,
        &surface_counts,
    )?;
    let material_plans = &material_assets.materials;

    let target_materials = crate::target_native_map::load_game_materials(&mut target)?;
    let collision_target = manifest
        .collision_material_overrides
        .get(COLLISION_SOURCE)
        .map(String::as_str)
        .unwrap_or(DEFAULT_COLLISION_TARGET);
    let target_id = *target_materials.get(collision_target).with_context(|| {
        format!("the target database has no physical material {collision_target:?}")
    })?;
    let collision_bindings = vec![CollisionMaterialBinding {
        source_id: 0,
        source_name: COLLISION_SOURCE.to_owned(),
        face_name: game_material::face_dictionary_name(0),
        target_id,
        target_name: collision_target.to_owned(),
        triangle_count: converted.collision.faces.len(),
    }];
    let collision_pieces =
        soc_collision::split(&converted.collision, manifest.max_collision_triangles)
            .iter()
            .map(|piece| {
                soc_collision::encode(
                    piece,
                    &collision_bindings,
                    manifest.collision_face_dictionary,
                )
            })
            .collect::<Result<Vec<_>>>()?;

    let pieces = if manifest.emit_collision {
        collision_pieces.len()
    } else {
        0
    };
    let surfaces_per_model = if pieces > 1 {
        manifest
            .surfaces_per_model
            .min(converted.surfaces.len() / pieces)
            .max(1)
    } else {
        manifest.surfaces_per_model
    };
    let model_groups = soc_surface::assign_models(&converted.surfaces, surfaces_per_model)?;
    if model_groups.len() < pieces {
        bail!(
            "{} collision pieces need at least that many render archives, but {} surfaces produce only {}; increase max_collision_triangles",
            pieces,
            converted.surfaces.len(),
            model_groups.len()
        );
    }

    let uv_scale: BTreeMap<u16, f32> = material_plans
        .iter()
        .filter_map(|plan| {
            manifest
                .family_uv_scale
                .get(&plan.family)
                .map(|scale| (plan.shader_id, *scale))
        })
        .collect();
    // Dirty Bomb's decals, grime strips and soft dirt-mound edges are thin
    // static meshes laid almost exactly on architecture. v0.100b exposes no
    // material depth bias, so use the same reviewed normal lift as the X-Ray
    // map path to prevent distance-dependent z fighting.
    let lift: BTreeMap<u16, f32> = material_plans
        .iter()
        .filter(|plan| plan.family == RenderFamily::Decal)
        .map(|plan| (plan.shader_id, manifest.decal_lift))
        .collect();

    let mut files = Vec::new();
    let mut models = Vec::with_capacity(model_groups.len());
    for (index, group) in model_groups.iter().enumerate() {
        let path = manifest.model_path(index);
        let collision = (index < pieces).then(|| &collision_pieces[index]);
        write_model_archive(
            manifest,
            &mut target,
            &overlay,
            &path,
            &converted.surfaces,
            group,
            collision,
            &uv_scale,
            &lift,
            &mut files,
        )?;
        models.push(path);
    }
    let collision_models = models[..pieces].to_vec();

    let uv_density = soc_surface::uv_world_density(&converted.surfaces);
    write_materials(
        manifest,
        &mut target,
        &overlay,
        material_plans,
        &BTreeSet::new(),
        None,
        &uv_density,
        &mut files,
    )?;
    write_textures(
        manifest,
        &mut target,
        &overlay,
        &material_assets.textures,
        &mut files,
    )?;
    let mut extent = MapExtent::of(&[], converted.collision.header.bounding_box)?;
    for axis in 0..3 {
        extent.min[axis] = extent.min[axis].min(converted.playable_render_bounds.0[axis]);
        extent.max[axis] = extent.max[axis].max(converted.playable_render_bounds.1[axis]);
    }
    let post_effect =
        write_post_process_material(manifest, &mut target, &overlay, &extent, &mut files)?;
    let dropped_template_objects = write_projects(
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
    let record = DirtyBombMapBuildRecord {
        asset_id: manifest.asset_id.clone(),
        source_map: converted.map,
        project_name: manifest.project_name.clone(),
        client_project: manifest.client_project_path(),
        server_project: manifest.server_project_path(),
        models,
        collision_models,
        converted_surfaces: converted.surfaces.len(),
        rendered_triangles: converted.report.render_triangles,
        materials: material_plans.len(),
        textures: material_assets.textures.len(),
        placeholder_materials: material_assets.report.diffuse_bound_materials
            < material_plans.len(),
        source_materials: converted.report.materials.clone(),
        material_assets: material_assets.report,
        collision_pieces: collision_pieces.len(),
        collision_vertices: collision_pieces
            .iter()
            .map(|piece| piece.vertex_count)
            .sum(),
        collision_triangles: collision_pieces
            .iter()
            .map(|piece| piece.triangle_count)
            .sum(),
        collision_materials: collision_bindings,
        conversion: converted.report,
        spawn_position: manifest.spawn_position,
        map_extent: extent,
        dropped_template_objects,
        files,
    };
    let manifest_path = output.join("port-manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("writing {}", manifest_path.display()))?;
    Ok(record)
}
