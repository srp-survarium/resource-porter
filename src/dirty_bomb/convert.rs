//! Convert Dirty Bomb's placed UE3 static meshes into Vostok world geometry.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::soc::codec::cform::{CFORM_VERSION, Cform, CformFace, CformHeader};
use crate::soc::codec::vertex::SocVertex;
use crate::soc::convert::surface::{MAX_SURFACE_VERTICES, MergedSurface};

use super::model::{self, BspNode, Model, PF_INVISIBLE, PF_NOT_SOLID, PF_PORTAL};
use super::package::Package;
use super::probe::ue_to_vostok;
use super::scene::{Matrix4, StaticMeshPlacement};
use super::source::DirtyBombSource;
use super::static_mesh::{self, StaticMesh, StaticMeshElement, StaticMeshLod};

const UNBOUND_MATERIAL: &str = "DirtyBomb.None";
const LARGE_PLACEMENT_THRESHOLD_METRES: f32 = 1_000.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LargePlacement {
    pub actor: String,
    pub actor_class: String,
    pub component: String,
    pub mesh: String,
    pub visible: bool,
    pub collidable: bool,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportedBoundsComparison {
    pub actor: String,
    pub actor_class: String,
    pub component: String,
    pub mesh: String,
    pub absolute_translation: bool,
    pub absolute_rotation: bool,
    pub absolute_scale: bool,
    pub reported_min: [f32; 3],
    pub reported_max: [f32; 3],
    pub actor_transformed_reported_min: [f32; 3],
    pub actor_transformed_reported_max: [f32; 3],
    pub computed_min: [f32; 3],
    pub computed_max: [f32; 3],
    pub center_error_metres: f32,
    pub extent_error_metres: f32,
    /// Maximum distance by which the computed mesh bounds escape the actor's
    /// reported union bounds. Zero means the mesh is fully contained.
    pub outside_distance_metres: f32,
    /// Same containment check after interpreting `CombinedBounds` as
    /// actor-local and transforming its eight corners into world space.
    pub actor_transformed_outside_distance_metres: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpawnCandidate {
    pub package: String,
    pub object: String,
    pub actor_position: [f32; 3],
    pub floor: Option<f32>,
    pub target_position: Option<[f32; 3]>,
}

/// Source attribution for one rendered material section. Keeping these bounds
/// in the report lets a live target position be traced back to the UE3 actor,
/// mesh and material without trying to reverse the later surface merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialRegion {
    pub package: String,
    pub kind: String,
    pub actor: String,
    pub component: String,
    pub mesh: String,
    pub section: usize,
    pub material: String,
    pub shader_id: u16,
    pub triangles: usize,
    pub vertex_colors: bool,
    pub texture_coordinates: usize,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Non-default per-vertex inputs present in one decoded source mesh LOD0.
/// The initial target path consumes only UV0 and no color, so this inventory
/// measures exactly where UE3 layered/vertex-painted materials lose data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshVertexData {
    pub package: String,
    pub mesh: String,
    pub vertices: usize,
    pub texture_coordinates: usize,
    pub full_precision_uvs: bool,
    pub colored_vertices: usize,
    pub non_white_vertices: usize,
    pub channel_min: [u8; 4],
    pub channel_max: [u8; 4],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConversionReport {
    pub packages: usize,
    pub placements: usize,
    pub visible_placements: usize,
    pub rendered_placements: usize,
    pub collidable_placements: usize,
    pub mirrored_placements: usize,
    pub absolute_translation_placements: usize,
    pub absolute_rotation_placements: usize,
    pub absolute_scale_placements: usize,
    pub skipped_imported_mesh_placements: usize,
    pub bsp_models: usize,
    pub bsp_nodes: usize,
    pub bsp_render_nodes: usize,
    pub bsp_render_vertices: usize,
    pub bsp_render_triangles: usize,
    pub bsp_collision_nodes: usize,
    pub bsp_collision_vertices: usize,
    pub bsp_collision_triangles: usize,
    pub render_sections: usize,
    pub render_vertices: usize,
    pub render_triangles: usize,
    pub vertex_colored_meshes: usize,
    pub vertex_colored_render_sections: usize,
    pub vertex_colored_render_triangles: usize,
    pub vertex_colored_materials: BTreeMap<String, usize>,
    pub multi_uv_meshes: usize,
    pub multi_uv_render_sections: usize,
    pub multi_uv_render_triangles: usize,
    pub multi_uv_materials: BTreeMap<String, usize>,
    pub mesh_vertex_data: Vec<MeshVertexData>,
    pub collision_sections: usize,
    pub collision_vertices: usize,
    pub collision_triangles: usize,
    pub collision_degenerate_triangles_dropped: usize,
    pub background_collision_triangles_dropped: usize,
    pub materials: BTreeMap<String, u16>,
    pub material_regions: Vec<MaterialRegion>,
    pub spawn_candidates: Vec<SpawnCandidate>,
    /// Placements whose transformed LOD0 reaches outside a one-kilometre cube.
    /// These are normally sky/background meshes and need explicit policy.
    pub large_placements: Vec<LargePlacement>,
    /// Cross-checks computed placement bounds against union bounds serialized
    /// by Dirty Bomb's custom actor.
    pub reported_bounds_comparisons: Vec<ReportedBoundsComparison>,
}

pub struct ConvertedScene {
    pub map: String,
    pub surfaces: Vec<MergedSurface>,
    pub collision: Cform,
    /// Visible non-background bounds. Sky domes remain renderable but must not
    /// stretch lighting and probe volumes over kilometres.
    pub playable_render_bounds: ([f32; 3], [f32; 3]),
    pub report: ConversionReport,
}

/// Decode and convert one complete streamed map graph.
pub fn convert_map(source: &DirtyBombSource, requested: &str) -> Result<ConvertedScene> {
    let map = source.resolve_map(requested)?;
    let paths = source.map_packages(&map)?;
    let mut report = ConversionReport {
        packages: paths.len(),
        ..ConversionReport::default()
    };
    let mut by_material: BTreeMap<u16, Vec<MergedSurface>> = BTreeMap::new();
    let mut material_ids = BTreeMap::new();
    let mut collision_vertices = Vec::new();
    let mut collision_faces = Vec::new();
    let mut playable_render_min = [f32::INFINITY; 3];
    let mut playable_render_max = [f32::NEG_INFINITY; 3];

    for path in paths {
        let label = source.relative_path(&path);
        let package =
            Package::read(&path).with_context(|| format!("decoding Dirty Bomb package {label}"))?;
        let placements = super::scene::placements(&package)
            .with_context(|| format!("resolving mesh placements in {label}"))?;
        let mut meshes = decode_placed_meshes(&package, &placements)
            .with_context(|| format!("decoding placed meshes in {label}"))?;
        for (mesh_index, mesh) in &meshes {
            let Some(lod) = mesh.lods.first() else {
                continue;
            };
            let colored_vertices = lod.vertex_colors.as_ref().map_or(0, Vec::len);
            if colored_vertices == 0 && lod.texture_coordinates == 1 {
                continue;
            }
            let mut channel_min = [u8::MAX; 4];
            let mut channel_max = [u8::MIN; 4];
            let mut non_white_vertices = 0;
            if let Some(colors) = &lod.vertex_colors {
                for color in colors {
                    non_white_vertices += usize::from(*color != [u8::MAX; 4]);
                    for channel in 0..4 {
                        channel_min[channel] = channel_min[channel].min(color[channel]);
                        channel_max[channel] = channel_max[channel].max(color[channel]);
                    }
                }
            } else {
                channel_min = [u8::MAX; 4];
                channel_max = [u8::MAX; 4];
            }
            report.vertex_colored_meshes += usize::from(colored_vertices > 0);
            report.multi_uv_meshes += usize::from(lod.texture_coordinates > 1);
            report.mesh_vertex_data.push(MeshVertexData {
                package: label.clone(),
                mesh: package.object_path(*mesh_index)?,
                vertices: lod.vertices.len(),
                texture_coordinates: lod.texture_coordinates,
                full_precision_uvs: lod.full_precision_uvs,
                colored_vertices,
                non_white_vertices,
                channel_min,
                channel_max,
            });
        }

        for export in &package.exports {
            if export.class_name != "PlayerStart" {
                continue;
            }
            let properties = package.properties(export).with_context(|| {
                format!("decoding PlayerStart {} in {label}", export.object_name)
            })?;
            if let Some(location) = properties
                .iter()
                .rev()
                .find(|property| property.name == "Location")
                .and_then(|property| package.property_vector(property))
            {
                report.spawn_candidates.push(SpawnCandidate {
                    package: label.clone(),
                    object: export.object_name.clone(),
                    actor_position: ue_to_vostok(location),
                    floor: None,
                    target_position: None,
                });
            }
        }

        for placement in placements {
            report.placements += 1;
            report.visible_placements += usize::from(placement.visible);
            report.collidable_placements += usize::from(placement.collision_enabled);
            report.absolute_translation_placements += usize::from(placement.absolute_translation);
            report.absolute_rotation_placements += usize::from(placement.absolute_rotation);
            report.absolute_scale_placements += usize::from(placement.absolute_scale);
            let mirrored = placement.transform.determinant3() < 0.0;
            report.mirrored_placements += usize::from(mirrored);
            if !placement.local_mesh {
                report.skipped_imported_mesh_placements += 1;
                continue;
            }
            let mesh = meshes.get_mut(&placement.mesh_index).with_context(|| {
                format!(
                    "{} has no decoded local mesh {}",
                    placement.component, placement.mesh
                )
            })?;
            let lod = mesh
                .lods
                .first()
                .with_context(|| format!("{} has no LOD0", placement.mesh))?;
            let (placement_min, placement_max) = transformed_bounds(lod, placement.transform)?;
            if let Some((origin, extent)) = placement.reported_bounds {
                let (reported_min, reported_max) = convert_source_bounds(origin, extent);
                let (actor_transformed_reported_min, actor_transformed_reported_max) =
                    convert_transformed_source_bounds(origin, extent, placement.actor_transform);
                report
                    .reported_bounds_comparisons
                    .push(ReportedBoundsComparison {
                        actor: placement.actor.clone(),
                        actor_class: placement.actor_class.clone(),
                        component: placement.component.clone(),
                        mesh: placement.mesh.clone(),
                        absolute_translation: placement.absolute_translation,
                        absolute_rotation: placement.absolute_rotation,
                        absolute_scale: placement.absolute_scale,
                        reported_min,
                        reported_max,
                        actor_transformed_reported_min,
                        actor_transformed_reported_max,
                        computed_min: placement_min,
                        computed_max: placement_max,
                        center_error_metres: bounds_center_error(
                            reported_min,
                            reported_max,
                            placement_min,
                            placement_max,
                        ),
                        extent_error_metres: bounds_extent_error(
                            reported_min,
                            reported_max,
                            placement_min,
                            placement_max,
                        ),
                        outside_distance_metres: bounds_outside_distance(
                            reported_min,
                            reported_max,
                            placement_min,
                            placement_max,
                        ),
                        actor_transformed_outside_distance_metres: bounds_outside_distance(
                            actor_transformed_reported_min,
                            actor_transformed_reported_max,
                            placement_min,
                            placement_max,
                        ),
                    });
            }
            let is_background = placement_min
                .iter()
                .chain(&placement_max)
                .any(|value| value.abs() > LARGE_PLACEMENT_THRESHOLD_METRES);
            if is_background {
                report.large_placements.push(LargePlacement {
                    actor: placement.actor.clone(),
                    actor_class: placement.actor_class.clone(),
                    component: placement.component.clone(),
                    mesh: placement.mesh.clone(),
                    visible: placement.visible,
                    collidable: placement.collision_enabled
                        && lod.elements.iter().any(|section| section.enable_collision),
                    min: placement_min,
                    max: placement_max,
                });
            } else if placement.visible {
                include_bounds(
                    &mut playable_render_min,
                    &mut playable_render_max,
                    placement_min,
                    placement_max,
                );
            }
            let mut rendered = false;
            for (section_index, section) in lod.elements.iter().enumerate() {
                let material = effective_material(&placement, section)
                    .unwrap_or_else(|| UNBOUND_MATERIAL.to_owned());
                let material_id = material_id(&mut material_ids, &material)?;
                if placement.visible {
                    let (section_min, section_max) =
                        transformed_section_bounds(lod, section, placement.transform)?;
                    report.material_regions.push(MaterialRegion {
                        package: label.clone(),
                        kind: "static_mesh".to_owned(),
                        actor: placement.actor.clone(),
                        component: placement.component.clone(),
                        mesh: placement.mesh.clone(),
                        section: section_index,
                        material: material.clone(),
                        shader_id: material_id,
                        triangles: section.triangles,
                        vertex_colors: lod.vertex_colors.is_some(),
                        texture_coordinates: lod.texture_coordinates,
                        min: section_min,
                        max: section_max,
                    });
                    if lod.vertex_colors.is_some() {
                        report.vertex_colored_render_sections += 1;
                        report.vertex_colored_render_triangles += section.triangles;
                        *report
                            .vertex_colored_materials
                            .entry(material.clone())
                            .or_default() += section.triangles;
                    }
                    if lod.texture_coordinates > 1 {
                        report.multi_uv_render_sections += 1;
                        report.multi_uv_render_triangles += section.triangles;
                        *report
                            .multi_uv_materials
                            .entry(material.clone())
                            .or_default() += section.triangles;
                    }
                    append_render_section(
                        by_material.entry(material_id).or_default(),
                        material_id,
                        lod,
                        section,
                        placement.transform,
                        mirrored,
                    )
                    .with_context(|| {
                        format!(
                            "rendering {} section {section_index} for {}",
                            placement.mesh, placement.actor
                        )
                    })?;
                    report.render_sections += 1;
                    report.render_triangles += section.triangles;
                    rendered = true;
                }
                if is_background && placement.collision_enabled && section.enable_collision {
                    report.background_collision_triangles_dropped += section.triangles;
                } else if placement.collision_enabled && section.enable_collision {
                    let dropped = append_collision_section(
                        &mut collision_vertices,
                        &mut collision_faces,
                        lod,
                        section,
                        placement.transform,
                        mirrored,
                    )
                    .with_context(|| {
                        format!(
                            "building collision from {} section {section_index} for {}",
                            placement.mesh, placement.actor
                        )
                    })?;
                    report.collision_sections += 1;
                    report.collision_triangles += section.triangles - dropped;
                    report.collision_degenerate_triangles_dropped += dropped;
                }
            }
            report.rendered_placements += usize::from(rendered);
        }

        for export in &package.exports {
            if export.class_name != "Model" || export.outer_index <= 0 {
                continue;
            }
            let owner = package.export(export.outer_index)?;
            if owner.class_name != "Level" {
                continue;
            }
            let bsp = model::parse(&package, export)
                .with_context(|| format!("decoding world BSP {} in {label}", export.object_name))?;
            report.bsp_models += 1;
            report.bsp_nodes += bsp.nodes.len();
            for (node_index, node) in bsp.nodes.iter().enumerate() {
                if node.vertices < 3 {
                    continue;
                }
                let surface = &bsp.surfaces[node.surface];
                if surface.poly_flags & (PF_INVISIBLE | PF_PORTAL) == 0 {
                    let material = surface
                        .material
                        .clone()
                        .unwrap_or_else(|| UNBOUND_MATERIAL.to_owned());
                    let shader_id = material_id(&mut material_ids, &material)?;
                    let (vertices, triangles, node_min, node_max) = append_bsp_render_node(
                        by_material.entry(shader_id).or_default(),
                        shader_id,
                        &bsp,
                        node,
                        // Cooked BSP node winding is not consistent across
                        // streamed Dirty Bomb levels. Its collision remains
                        // correct while isolated floors and walls can face
                        // away from the player. Architectural BSP is a closed
                        // shell, so emitting the reverse fan is the faithful,
                        // deterministic substitute for UE3's BSP render path.
                        true,
                    )
                    .with_context(|| {
                        format!(
                            "rendering BSP node {node_index} from {}",
                            export.object_name
                        )
                    })?;
                    include_bounds(
                        &mut playable_render_min,
                        &mut playable_render_max,
                        node_min,
                        node_max,
                    );
                    report.material_regions.push(MaterialRegion {
                        package: label.clone(),
                        kind: "bsp".to_owned(),
                        actor: owner.object_name.clone(),
                        component: export.object_name.clone(),
                        mesh: export.object_name.clone(),
                        section: node_index,
                        material,
                        shader_id,
                        triangles,
                        vertex_colors: false,
                        texture_coordinates: 1,
                        min: node_min,
                        max: node_max,
                    });
                    report.bsp_render_nodes += 1;
                    report.bsp_render_vertices += vertices;
                    report.bsp_render_triangles += triangles;
                    report.render_sections += 1;
                    report.render_triangles += triangles;
                }
                if surface.poly_flags & (PF_NOT_SOLID | PF_PORTAL) == 0 {
                    let (vertices, triangles, dropped) = append_bsp_collision_node(
                        &mut collision_vertices,
                        &mut collision_faces,
                        &bsp,
                        node,
                    )
                    .with_context(|| {
                        format!(
                            "building collision from BSP node {node_index} in {}",
                            export.object_name
                        )
                    })?;
                    report.bsp_collision_nodes += 1;
                    report.bsp_collision_vertices += vertices;
                    report.bsp_collision_triangles += triangles;
                    report.collision_sections += 1;
                    report.collision_triangles += triangles;
                    report.collision_degenerate_triangles_dropped += dropped;
                }
            }
        }
    }

    let mut surfaces = Vec::new();
    for (_, mut parts) in by_material {
        surfaces.append(&mut parts);
    }
    if surfaces.is_empty() {
        bail!("{map} produced no visible local static-mesh geometry");
    }
    if collision_faces.is_empty() {
        bail!("{map} produced no collidable static-mesh geometry");
    }
    if !playable_render_min
        .iter()
        .chain(&playable_render_max)
        .all(|value| value.is_finite())
    {
        bail!("{map} produced no finite non-background render bounds");
    }
    report.render_vertices = surfaces.iter().map(|surface| surface.vertices.len()).sum();
    report.collision_vertices = collision_vertices.len();
    report.materials = material_ids;
    let collision_bounds = bounds(&collision_vertices)?;
    let collision = Cform {
        header: CformHeader {
            version: CFORM_VERSION,
            vertex_count: u32::try_from(collision_vertices.len())?,
            face_count: u32::try_from(collision_faces.len())?,
            bounding_box: collision_bounds,
        },
        vertices: collision_vertices,
        faces: collision_faces,
    };
    for candidate in &mut report.spawn_candidates {
        candidate.floor = floor_below(&collision, candidate.actor_position);
        candidate.target_position = candidate.floor.map(|floor| {
            [
                candidate.actor_position[0],
                floor + 0.05,
                candidate.actor_position[2],
            ]
        });
    }
    Ok(ConvertedScene {
        map,
        surfaces,
        collision,
        playable_render_bounds: (playable_render_min, playable_render_max),
        report,
    })
}

/// Highest collision layer at or below a UE3 actor position.
///
/// UE3 PlayerStart locations refer to the pawn/cylinder center while Vostok's
/// network spawn is a feet position. Snapping to the source collision avoids
/// preserving the UE3 pawn half-height as an unintended hover offset.
pub fn floor_below(collision: &Cform, position: [f32; 3]) -> Option<f32> {
    let mut best = None::<f32>;
    for face in &collision.faces {
        let triangle = face
            .vertices
            .map(|index| collision.vertices[index as usize]);
        let [a, b, c] = triangle;
        let denominator = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        if denominator.abs() <= 1.0e-7 {
            continue;
        }
        let first = ((b[2] - c[2]) * (position[0] - c[0]) + (c[0] - b[0]) * (position[2] - c[2]))
            / denominator;
        let second = ((c[2] - a[2]) * (position[0] - c[0]) + (a[0] - c[0]) * (position[2] - c[2]))
            / denominator;
        let third = 1.0 - first - second;
        if first < -1.0e-4 || second < -1.0e-4 || third < -1.0e-4 {
            continue;
        }
        let height = first * a[1] + second * b[1] + third * c[1];
        if height > position[1] + 0.25 {
            continue;
        }
        best = Some(best.map_or(height, |current| current.max(height)));
    }
    best
}

fn append_bsp_render_node(
    parts: &mut Vec<MergedSurface>,
    shader_id: u16,
    model: &Model,
    node: &BspNode,
    two_sided: bool,
) -> Result<(usize, usize, [f32; 3], [f32; 3])> {
    let source =
        &model.vertices[node.vertex_buffer_start..node.vertex_buffer_start + node.vertices];
    let needs_part = parts
        .last()
        .is_none_or(|part| part.vertices.len() + source.len() > MAX_SURFACE_VERTICES);
    if needs_part {
        parts.push(MergedSurface {
            shader_id,
            part: parts.len(),
            vertices: Vec::new(),
            indices: Vec::new(),
            visual_count: 0,
            tree_visual_count: 0,
        });
    }
    let part = parts
        .last_mut()
        .expect("a BSP render part was just created");
    let base = u16::try_from(part.vertices.len())
        .expect("a nonempty BSP append starts below the u16 vertex limit");
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in source {
        let position = ue_to_vostok(vertex.position);
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
        part.vertices.push(SocVertex {
            position,
            normal: rotate_ue_basis(vertex.normal),
            tangent: rotate_ue_basis(vertex.tangent),
            binormal: rotate_ue_basis(vertex.binormal),
            uv: vertex.uv,
        });
    }
    let local = |index: usize| -> Result<u16> {
        Ok(base
            .checked_add(u16::try_from(index)?)
            .context("BSP surface index overflow")?)
    };
    let mut triangles = 0;
    for index in 2..source.len() {
        // UModel's cooked vertex buffer is emitted as [0, i, i-1], and the
        // v0.100b renderer accepts that fan order directly. Static-mesh index
        // buffers use the opposite target convention below; treating BSP the
        // same way makes floors visible only from underneath.
        let front = [local(0)?, local(index)?, local(index - 1)?];
        part.indices.extend(front);
        triangles += 1;
        if two_sided {
            part.indices.extend([front[0], front[2], front[1]]);
            triangles += 1;
        }
    }
    part.visual_count += 1;
    Ok((source.len(), triangles, min, max))
}

fn append_bsp_collision_node(
    vertices: &mut Vec<[f32; 3]>,
    faces: &mut Vec<CformFace>,
    model: &Model,
    node: &BspNode,
) -> Result<(usize, usize, usize)> {
    let source =
        &model.vertices[node.vertex_buffer_start..node.vertex_buffer_start + node.vertices];
    let base = u32::try_from(vertices.len()).context("collision vertex count exceeds u32")?;
    vertices.extend(source.iter().map(|vertex| ue_to_vostok(vertex.position)));
    let mut triangles = 0;
    let mut dropped = 0;
    for index in 2..source.len() {
        let mapped = [
            base,
            base + u32::try_from(index - 1)?,
            base + u32::try_from(index)?,
        ];
        let points = mapped.map(|index| vertices[index as usize]);
        if triangle_area_squared(points) <= f32::EPSILON {
            dropped += 1;
            continue;
        }
        faces.push(CformFace {
            vertices: mapped,
            material: 0,
            suppress_shadows: false,
            suppress_wallmarks: false,
            sector: 0,
        });
        triangles += 1;
    }
    Ok((source.len(), triangles, dropped))
}

fn decode_placed_meshes(
    package: &Package,
    placements: &[StaticMeshPlacement],
) -> Result<HashMap<i32, StaticMesh>> {
    let mut meshes = HashMap::new();
    for placement in placements.iter().filter(|placement| placement.local_mesh) {
        if meshes.contains_key(&placement.mesh_index) {
            continue;
        }
        let export = package.export(placement.mesh_index)?;
        let mesh = static_mesh::parse(package, export)
            .with_context(|| format!("decoding {}", placement.mesh))?;
        meshes.insert(placement.mesh_index, mesh);
    }
    Ok(meshes)
}

fn effective_material(
    placement: &StaticMeshPlacement,
    section: &StaticMeshElement,
) -> Option<String> {
    usize::try_from(section.material_index)
        .ok()
        .and_then(|index| placement.material_overrides.get(index))
        .and_then(Clone::clone)
        .or_else(|| section.material.clone())
}

fn material_id(materials: &mut BTreeMap<String, u16>, material: &str) -> Result<u16> {
    if let Some(id) = materials.get(material) {
        return Ok(*id);
    }
    let id = u16::try_from(materials.len()).context("Dirty Bomb material count exceeds u16")?;
    materials.insert(material.to_owned(), id);
    Ok(id)
}

fn append_render_section(
    parts: &mut Vec<MergedSurface>,
    shader_id: u16,
    lod: &StaticMeshLod,
    section: &StaticMeshElement,
    transform: Matrix4,
    mirrored: bool,
) -> Result<()> {
    let indices = section_indices(lod, section)?;
    let (source_vertices, remapped) = remap_section(indices, lod.vertices.len())?;
    if source_vertices.len() > MAX_SURFACE_VERTICES {
        bail!(
            "section uses {} vertices, exceeding a target u16 index space",
            source_vertices.len()
        );
    }
    let needs_part = parts
        .last()
        .is_none_or(|part| part.vertices.len() + source_vertices.len() > MAX_SURFACE_VERTICES);
    if needs_part {
        parts.push(MergedSurface {
            shader_id,
            part: parts.len(),
            vertices: Vec::new(),
            indices: Vec::new(),
            visual_count: 0,
            tree_visual_count: 0,
        });
    }
    let part = parts.last_mut().expect("a render part was just created");
    let base = u16::try_from(part.vertices.len())
        .expect("a nonempty append starts below the u16 vertex limit");
    for index in source_vertices {
        let vertex = lod.vertices[usize::from(index)];
        let (normal, tangent, binormal) =
            transformed_basis(transform, vertex.normal, vertex.tangent, vertex.binormal);
        part.vertices.push(SocVertex {
            position: ue_to_vostok(transform.transform_point(vertex.position)),
            normal: rotate_ue_basis(normal),
            tangent: rotate_ue_basis(tangent),
            binormal: rotate_ue_basis(binormal),
            uv: vertex.uv,
        });
    }
    // The UE-to-Vostok conversion is a proper rotation, so reverse only for
    // Vostok's opposite rasterizer front-face convention. A negative-scale
    // UE3 placement has already reversed the face, so do not reverse twice.
    for triangle in remapped.chunks_exact(3) {
        let mapped = if mirrored {
            [triangle[0] + base, triangle[1] + base, triangle[2] + base]
        } else {
            [triangle[0] + base, triangle[2] + base, triangle[1] + base]
        };
        part.indices.extend(mapped);
    }
    part.visual_count += 1;
    Ok(())
}

fn append_collision_section(
    vertices: &mut Vec<[f32; 3]>,
    faces: &mut Vec<CformFace>,
    lod: &StaticMeshLod,
    section: &StaticMeshElement,
    transform: Matrix4,
    mirrored: bool,
) -> Result<usize> {
    let indices = section_indices(lod, section)?;
    let (source_vertices, remapped) = remap_section(indices, lod.vertices.len())?;
    let base = u32::try_from(vertices.len()).context("collision vertex count exceeds u32")?;
    for index in source_vertices {
        vertices.push(ue_to_vostok(
            transform.transform_point(lod.vertices[usize::from(index)].position),
        ));
    }
    let mut dropped = 0;
    for triangle in remapped.chunks_exact(3) {
        let mapped = if mirrored {
            [
                u32::from(triangle[0]) + base,
                u32::from(triangle[1]) + base,
                u32::from(triangle[2]) + base,
            ]
        } else {
            [
                u32::from(triangle[0]) + base,
                u32::from(triangle[2]) + base,
                u32::from(triangle[1]) + base,
            ]
        };
        let points = mapped.map(|index| vertices[index as usize]);
        if mapped[0] == mapped[1]
            || mapped[1] == mapped[2]
            || mapped[2] == mapped[0]
            || triangle_area_squared(points) <= f32::EPSILON
        {
            dropped += 1;
            continue;
        }
        faces.push(CformFace {
            vertices: mapped,
            material: 0,
            suppress_shadows: false,
            suppress_wallmarks: false,
            sector: 0,
        });
    }
    Ok(dropped)
}

fn section_indices<'a>(lod: &'a StaticMeshLod, section: &StaticMeshElement) -> Result<&'a [u16]> {
    let count = section
        .triangles
        .checked_mul(3)
        .context("section index count overflow")?;
    lod.indices
        .get(section.first_index..section.first_index + count)
        .context("static-mesh section index range exceeds LOD buffer")
}

fn remap_section(indices: &[u16], vertex_count: usize) -> Result<(Vec<u16>, Vec<u16>)> {
    let mut slots = HashMap::new();
    let mut source_vertices = Vec::new();
    let mut remapped = Vec::with_capacity(indices.len());
    for source in indices {
        if usize::from(*source) >= vertex_count {
            bail!("section index {source} exceeds {vertex_count} vertices");
        }
        let next = u16::try_from(source_vertices.len())
            .context("section uses more than 65,536 vertices")?;
        let target = *slots.entry(*source).or_insert_with(|| {
            source_vertices.push(*source);
            next
        });
        remapped.push(target);
    }
    Ok((source_vertices, remapped))
}

fn transformed_basis(
    transform: Matrix4,
    normal: [f32; 3],
    tangent: [f32; 3],
    binormal: [f32; 3],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let tangent = transform.transform_vector(tangent);
    let binormal = transform.transform_vector(binormal);
    let approximate_normal = transform.transform_vector(normal);
    let mut normal = normalize(cross(tangent, binormal));
    if dot(normal, approximate_normal) < 0.0 {
        normal = normal.map(|component| -component);
    }
    (normal, tangent, binormal)
}

fn rotate_ue_basis(vector: [f32; 3]) -> [f32; 3] {
    [vector[0], vector[2], -vector[1]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = dot(vector, vector).sqrt();
    if length > f32::EPSILON {
        vector.map(|component| component / length)
    } else {
        vector
    }
}

fn triangle_area_squared(points: [[f32; 3]; 3]) -> f32 {
    let ab = std::array::from_fn(|axis| points[1][axis] - points[0][axis]);
    let ac = std::array::from_fn(|axis| points[2][axis] - points[0][axis]);
    dot(cross(ab, ac), cross(ab, ac))
}

fn bounds(vertices: &[[f32; 3]]) -> Result<([f32; 3], [f32; 3])> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in vertices {
        if !point.iter().all(|value| value.is_finite()) {
            bail!("converted collision contains a non-finite vertex");
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Ok((min, max))
}

fn transformed_bounds(lod: &StaticMeshLod, transform: Matrix4) -> Result<([f32; 3], [f32; 3])> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &lod.vertices {
        let point = ue_to_vostok(transform.transform_point(vertex.position));
        if !point.iter().all(|value| value.is_finite()) {
            bail!("transformed render geometry contains a non-finite vertex");
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Ok((min, max))
}

fn transformed_section_bounds(
    lod: &StaticMeshLod,
    section: &StaticMeshElement,
    transform: Matrix4,
) -> Result<([f32; 3], [f32; 3])> {
    let indices = section_indices(lod, section)?;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for index in indices {
        let vertex = lod
            .vertices
            .get(usize::from(*index))
            .context("static-mesh section index exceeds its vertex buffer")?;
        let point = ue_to_vostok(transform.transform_point(vertex.position));
        if !point.iter().all(|value| value.is_finite()) {
            bail!("transformed render section contains a non-finite vertex");
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Ok((min, max))
}

fn include_bounds(
    target_min: &mut [f32; 3],
    target_max: &mut [f32; 3],
    min: [f32; 3],
    max: [f32; 3],
) {
    for axis in 0..3 {
        target_min[axis] = target_min[axis].min(min[axis]);
        target_max[axis] = target_max[axis].max(max[axis]);
    }
}

fn convert_source_bounds(origin: [f32; 3], extent: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let source_min = std::array::from_fn(|axis| origin[axis] - extent[axis]);
    let source_max = std::array::from_fn(|axis| origin[axis] + extent[axis]);
    let converted = [ue_to_vostok(source_min), ue_to_vostok(source_max)];
    (
        std::array::from_fn(|axis| converted[0][axis].min(converted[1][axis])),
        std::array::from_fn(|axis| converted[0][axis].max(converted[1][axis])),
    )
}

fn convert_transformed_source_bounds(
    origin: [f32; 3],
    extent: [f32; 3],
    transform: Matrix4,
) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for x in [-extent[0], extent[0]] {
        for y in [-extent[1], extent[1]] {
            for z in [-extent[2], extent[2]] {
                let point = ue_to_vostok(transform.transform_point([
                    origin[0] + x,
                    origin[1] + y,
                    origin[2] + z,
                ]));
                for axis in 0..3 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
        }
    }
    (min, max)
}

fn bounds_center_error(
    left_min: [f32; 3],
    left_max: [f32; 3],
    right_min: [f32; 3],
    right_max: [f32; 3],
) -> f32 {
    let difference: [f32; 3] = std::array::from_fn(|axis| {
        (left_min[axis] + left_max[axis] - right_min[axis] - right_max[axis]) * 0.5
    });
    dot(difference, difference).sqrt()
}

fn bounds_extent_error(
    left_min: [f32; 3],
    left_max: [f32; 3],
    right_min: [f32; 3],
    right_max: [f32; 3],
) -> f32 {
    let difference: [f32; 3] = std::array::from_fn(|axis| {
        (left_max[axis] - left_min[axis] - right_max[axis] + right_min[axis]) * 0.5
    });
    dot(difference, difference).sqrt()
}

fn bounds_outside_distance(
    outer_min: [f32; 3],
    outer_max: [f32; 3],
    inner_min: [f32; 3],
    inner_max: [f32; 3],
) -> f32 {
    (0..3)
        .flat_map(|axis| {
            [
                outer_min[axis] - inner_min[axis],
                inner_max[axis] - outer_max[axis],
            ]
        })
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_rotation_preserves_triangle_winding() {
        let source = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let converted = source.map(ue_to_vostok);
        let normal = cross(
            std::array::from_fn(|axis| converted[1][axis] - converted[0][axis]),
            std::array::from_fn(|axis| converted[2][axis] - converted[0][axis]),
        );
        assert!(normal[1] > 0.0);
    }

    #[test]
    fn remaps_only_vertices_used_by_a_section() {
        let (vertices, indices) = remap_section(&[4, 2, 4, 2, 3, 4], 5).unwrap();
        assert_eq!(vertices, vec![4, 2, 3]);
        assert_eq!(indices, vec![0, 1, 0, 1, 2, 0]);
    }

    #[test]
    fn finds_floor_below_actor_center() {
        let collision = Cform {
            header: CformHeader {
                version: CFORM_VERSION,
                vertex_count: 3,
                face_count: 1,
                bounding_box: ([0.0, 2.0, 0.0], [2.0, 2.0, 2.0]),
            },
            vertices: vec![[0.0, 2.0, 0.0], [0.0, 2.0, 2.0], [2.0, 2.0, 0.0]],
            faces: vec![CformFace {
                vertices: [0, 1, 2],
                material: 0,
                suppress_shadows: false,
                suppress_wallmarks: false,
                sector: 0,
            }],
        };
        assert_eq!(floor_below(&collision, [0.5, 3.0, 0.5]), Some(2.0));
        assert_eq!(floor_below(&collision, [3.0, 3.0, 3.0]), None);
    }
}
