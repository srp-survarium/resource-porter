// SPDX-License-Identifier: GPL-3.0-or-later
//! Merge level visuals into target render surfaces.
//!
//! A compiled X-Ray level is cut into thousands of small visuals so the sector
//! and portal graph can reject them individually. v0.100b has no such graph and
//! charges per surface, so the port regroups the world by shader:
//!
//! ```text
//! N visuals  ->  one surface per (shader, part)  ->  M model archives
//! ```
//!
//! Parts exist because target indices are `u16`: a shader used by more of the
//! world than fits in one index space is split at exactly that boundary and
//! nowhere else, so the split is reproducible.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::soc::codec::level::GeometryBuffers;
use crate::soc::codec::ogf::{
    GeometryContainer, Visual, VisualKind, transform_direction, transform_position,
};
use crate::soc::codec::vertex::SocVertex;

/// One `u16` index space.
pub const MAX_SURFACE_VERTICES: usize = 65_536;

/// The target static vertex declaration, byte-identical to the one the weapon
/// and STK2 importers already emit.
const STATIC_VERTEX_DECLARATION: [u8; 48] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // position: float3
    0x00, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x03, 0x00, // normal: D3DCOLOR
    0x00, 0x00, 0x10, 0x00, 0x04, 0x00, 0x06, 0x00, // tangent: D3DCOLOR
    0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x07, 0x00, // binormal: D3DCOLOR
    0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x05, 0x00, // uv: float2
    0xff, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, // D3DDECL_END
];

/// A surface built from one or more source visuals sharing a shader.
#[derive(Clone, Debug)]
pub struct MergedSurface {
    pub shader_id: u16,
    pub part: usize,
    pub vertices: Vec<SocVertex>,
    pub indices: Vec<u16>,
    /// How many source visuals were welded into this surface.
    pub visual_count: usize,
    /// How many of those were tree visuals placed by their own matrix.
    pub tree_visual_count: usize,
}

impl MergedSurface {
    pub fn name(&self) -> String {
        format!("shader_{:04}_part_{:03}", self.shader_id, self.part)
    }

    pub fn material_name(&self) -> String {
        format!("shader_{:04}", self.shader_id)
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Axis-aligned bounds and the sphere the renderer culls against.
    /// World units spanned by one texture-coordinate unit, horizontally.
    ///
    /// The measure a tiled overlay needs: `constant_tile_N` is a repeat *count*
    /// over the mesh UV, so turning a wanted repeat size in metres into a count
    /// needs to know how much ground one UV unit covers. Horizontal only —
    /// terrain is close enough to flat that height would only add the cliffs.
    ///
    /// `None` when the surface has no measurable UV extent, which a single
    /// degenerate triangle can produce.
    pub fn uv_world_density(&self) -> Option<f32> {
        let mut world = [f32::NEG_INFINITY; 2];
        let mut world_min = [f32::INFINITY; 2];
        let mut uv = [f32::NEG_INFINITY; 2];
        let mut uv_min = [f32::INFINITY; 2];
        for vertex in &self.vertices {
            for (index, axis) in [0, 2].into_iter().enumerate() {
                world[index] = world[index].max(vertex.position[axis]);
                world_min[index] = world_min[index].min(vertex.position[axis]);
            }
            for axis in 0..2 {
                uv[axis] = uv[axis].max(vertex.uv[axis]);
                uv_min[axis] = uv_min[axis].min(vertex.uv[axis]);
            }
        }
        let world_span = (world[0] - world_min[0]).max(world[1] - world_min[1]);
        let uv_span = (uv[0] - uv_min[0]).max(uv[1] - uv_min[1]);
        (uv_span > f32::EPSILON && world_span.is_finite() && world_span > 0.0)
            .then(|| world_span / uv_span)
    }

    pub fn bounds(&self) -> Bounds {
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for vertex in &self.vertices {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(vertex.position[axis]);
                maximum[axis] = maximum[axis].max(vertex.position[axis]);
            }
        }
        let center = [
            (minimum[0] + maximum[0]) * 0.5,
            (minimum[1] + maximum[1]) * 0.5,
            (minimum[2] + maximum[2]) * 0.5,
        ];
        let mut radius = 0.0_f32;
        for vertex in &self.vertices {
            let delta = [
                vertex.position[0] - center[0],
                vertex.position[1] - center[1],
                vertex.position[2] - center[2],
            ];
            radius = radius
                .max((delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt());
        }
        Bounds {
            min: minimum,
            max: maximum,
            center,
            radius,
        }
    }

    /// Encode the target `converted_model` payload: a vertex chunk carrying the
    /// declaration, then an index chunk.
    /// Encode with an explicit texture-coordinate multiplier.
    ///
    /// A few X-Ray shaders do not get their detail from the base texture, so
    /// the level authors had no reason to give them a sensible tiling.
    /// `effects\water` is the clear case: `mp_pool`'s water is one flat plane
    /// of 7,296 triangles spanning 42 world units, and its UVs span 3 -- one
    /// repeat every fourteen units. X-Ray gets away with that because
    /// `water.vs` takes its ripples from two normal maps offset by world
    /// position (`watermove_tc(tbase * W_DISTORT_BASE_TILE, P.xz, AMP)`) and
    /// animated per frame. Ported without that shader, the base texture is
    /// left magnified about fourteenfold and reads as a few smooth lobes.
    ///
    /// Scaling here is an approximation, not a correction: it trades the
    /// source's exact mapping for a texel density the target can show.
    /// Encode, optionally lifting every vertex along its own normal.
    ///
    /// X-Ray's decals — posters, litter, wall grime — are separate surfaces laid
    /// *exactly* on the surface they decorate, and the engine biased them at
    /// draw time. v0.100b exposes no depth bias on a material instance, so two
    /// coplanar surfaces fight for the depth buffer and flicker as the camera
    /// moves.
    ///
    /// The engine's own decal material is not a substitute: `default_multiply_decal`
    /// is a *projected* decal used by the decal system, and instantiating it as
    /// a surface material renders nothing at all.
    ///
    /// So the geometry is moved instead of the render state: a decal is lifted
    /// a few millimetres along its vertex normal, which is the direction it
    /// faces away from its host surface. Small enough to be invisible, large
    /// enough to settle the depth test.
    pub fn encode_converted_model_lifted(&self, uv_scale: f32, lift: f32) -> Result<Vec<u8>> {
        let count = self.vertices.len();
        let vertex_chunk_size = STATIC_VERTEX_DECLARATION
            .len()
            .checked_add(4 + count * 32)
            .context("converted vertex chunk overflow")?;
        let index_chunk_size = 4_usize
            .checked_add(self.indices.len() * 2)
            .context("converted index chunk overflow")?;
        let mut output = Vec::with_capacity(16 + vertex_chunk_size + index_chunk_size);

        output.extend_from_slice(&3_u32.to_le_bytes());
        output.extend_from_slice(&u32::try_from(vertex_chunk_size)?.to_le_bytes());
        output.extend_from_slice(&STATIC_VERTEX_DECLARATION);
        output.extend_from_slice(&u32::try_from(count)?.to_le_bytes());
        for vertex in &self.vertices {
            for axis in 0..3 {
                let value = vertex.position[axis] + vertex.normal[axis] * lift;
                output.extend_from_slice(&value.to_le_bytes());
            }
            // The alpha bytes are target render policy, not source data: the
            // source carried a hemisphere sample and the texture-coordinate
            // remainder there, both already consumed during decoding.
            output.extend_from_slice(&packed_basis(vertex.normal, 127).to_le_bytes());
            output.extend_from_slice(&packed_basis(vertex.tangent, 255).to_le_bytes());
            output.extend_from_slice(&packed_basis(vertex.binormal, 255).to_le_bytes());
            for value in vertex.uv {
                output.extend_from_slice(&(value * uv_scale).to_le_bytes());
            }
        }

        output.extend_from_slice(&4_u32.to_le_bytes());
        output.extend_from_slice(&u32::try_from(index_chunk_size)?.to_le_bytes());
        output.extend_from_slice(&u32::try_from(self.indices.len())?.to_le_bytes());
        for index in &self.indices {
            output.extend_from_slice(&index.to_le_bytes());
        }
        Ok(output)
    }

    /// Encode with the source mapping and placement unchanged.
    pub fn encode_converted_model(&self) -> Result<Vec<u8>> {
        self.encode_converted_model_lifted(1.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub center: [f32; 3],
    pub radius: f32,
}

/// Counters describing what the merge accepted and rejected.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MergeReport {
    pub visuals: usize,
    pub rendered_visuals: usize,
    pub tree_visuals: usize,
    pub merged_surfaces: usize,
    pub triangles: usize,
    /// Triangles contributed by tree visuals, which `xrLC` placed by their own
    /// matrix. Tracked apart because they dominate a wooded level and because
    /// the recovered Cordon manifests appear to have counted only the rest.
    pub tree_triangles: usize,
    /// Source visual kinds with no target adapter, counted by label.
    pub dropped_by_kind: BTreeMap<String, usize>,
    /// Static visuals that carried no geometry container.
    pub dropped_without_geometry: usize,
}

/// One source visual resolved into world-space geometry.
struct ResolvedVisual {
    shader_id: u16,
    vertices: Vec<SocVertex>,
    indices: Vec<u16>,
    is_tree: bool,
}

/// Weld every static visual into per-shader surfaces of at most one `u16`
/// index space.
pub fn merge_visuals(
    visuals: &[Visual],
    buffers: &GeometryBuffers,
) -> Result<(Vec<MergedSurface>, MergeReport)> {
    merge_visuals_limited(visuals, buffers, MAX_SURFACE_VERTICES)
}

/// Weld every static visual into per-shader surfaces of at most
/// `max_vertices` vertices (at most one `u16` index space).
pub fn merge_visuals_limited(
    visuals: &[Visual],
    buffers: &GeometryBuffers,
    max_vertices: usize,
) -> Result<(Vec<MergedSurface>, MergeReport)> {
    if max_vertices == 0 || max_vertices > MAX_SURFACE_VERTICES {
        bail!("a surface may hold 1..={MAX_SURFACE_VERTICES} vertices, not {max_vertices}");
    }
    let mut report = MergeReport {
        visuals: visuals.len(),
        ..MergeReport::default()
    };
    let mut by_shader: BTreeMap<u16, Vec<ResolvedVisual>> = BTreeMap::new();

    for (index, visual) in visuals.iter().enumerate() {
        if !visual.kind.is_static() {
            *report
                .dropped_by_kind
                .entry(visual.kind.label())
                .or_insert(0) += 1;
            continue;
        }
        let Some(container) = visual.geometry else {
            report.dropped_without_geometry += 1;
            continue;
        };
        let resolved = resolve_visual(visual, &container, buffers)
            .with_context(|| format!("resolving level visual {index}"))?;
        report.rendered_visuals += 1;
        if resolved.is_tree {
            report.tree_visuals += 1;
            report.tree_triangles += resolved.indices.len() / 3;
        }
        by_shader
            .entry(resolved.shader_id)
            .or_default()
            .push(resolved);
    }

    let mut surfaces = Vec::new();
    for (shader_id, group) in by_shader {
        let mut part = 0;
        let mut current: Option<MergedSurface> = None;
        for resolved in group {
            if resolved.vertices.len() > max_vertices {
                bail!(
                    "a single visual for shader {shader_id} has {} vertices, which exceeds the \
                     {max_vertices}-vertex surface limit",
                    resolved.vertices.len()
                );
            }
            let needs_new_part = match &current {
                Some(surface) => {
                    surface.vertices.len() + resolved.vertices.len() > max_vertices
                }
                None => true,
            };
            if needs_new_part {
                if let Some(surface) = current.take() {
                    surfaces.push(surface);
                    part += 1;
                }
                current = Some(MergedSurface {
                    shader_id,
                    part,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    visual_count: 0,
                    tree_visual_count: 0,
                });
            }
            let surface = current.as_mut().expect("a part was just started");
            let base = u16::try_from(surface.vertices.len())
                .expect("part vertex counts stay inside one u16 index space");
            surface.vertices.extend(resolved.vertices);
            surface
                .indices
                .extend(resolved.indices.iter().map(|index| index + base));
            surface.visual_count += 1;
            if resolved.is_tree {
                surface.tree_visual_count += 1;
            }
        }
        if let Some(surface) = current {
            surfaces.push(surface);
        }
    }

    report.merged_surfaces = surfaces.len();
    report.triangles = surfaces.iter().map(MergedSurface::triangle_count).sum();
    Ok((surfaces, report))
}

fn resolve_visual(
    visual: &Visual,
    container: &GeometryContainer,
    buffers: &GeometryBuffers,
) -> Result<ResolvedVisual> {
    // A tree visual keeps its detail levels in the level's shared `fsL_SWIS`
    // table and names its record by `OGF_SWICONTAINER`, so the narrowing that
    // `OGF_SWIDATA` does for a progressive mesh has to happen here instead:
    //
    // ```cpp
    // // xrRender/FTreeVisual.cpp
    // u32 ID = data->r_u32();
    // pSWI   = RImplementation.getSWI( ID );
    // // FTreeVisual_PM::Render
    // FSlideWindow& SW = pSWI->sw[lod_id];
    // cmd_list.Render( D3DPT_TRIANGLELIST, vBase, 0, SW.num_verts,
    //                  iBase + SW.offset, SW.num_tris );
    // ```
    //
    // Without it the whole container is drawn, which is every detail level of
    // the mesh at once.
    let narrowed;
    let container = match visual.sliding_window {
        Some(id) => {
            let window = buffers
                .sliding_windows
                .get(usize::try_from(id)?)
                .with_context(|| format!("sliding window record {id} does not exist"))?;
            let triangles = u32::from(window.triangles);
            if triangles == 0 {
                bail!("sliding window record {id} declares no triangles at full detail");
            }
            if window.offset + triangles * 3 > container.index_count {
                bail!(
                    "sliding window record {id} runs to index {} of a {}-index container",
                    window.offset + triangles * 3,
                    container.index_count
                );
            }
            narrowed = GeometryContainer {
                index_base: container.index_base + window.offset,
                index_count: triangles * 3,
                vertex_count: u32::from(window.vertices).min(container.vertex_count),
                ..*container
            };
            &narrowed
        }
        None => container,
    };

    let vertex_buffer = buffers
        .vertex_buffers
        .get(usize::try_from(container.vertex_buffer)?)
        .with_context(|| format!("vertex buffer {} does not exist", container.vertex_buffer))?;
    let index_buffer = buffers
        .index_buffers
        .get(usize::try_from(container.index_buffer)?)
        .with_context(|| format!("index buffer {} does not exist", container.index_buffer))?;

    let vertex_base = usize::try_from(container.vertex_base)?;
    let vertex_count = usize::try_from(container.vertex_count)?;
    let index_base = usize::try_from(container.index_base)?;
    let index_count = usize::try_from(container.index_count)?;

    if vertex_base + vertex_count > vertex_buffer.count {
        bail!(
            "vertex range {vertex_base}..{} exceeds buffer {} of {} vertices",
            vertex_base + vertex_count,
            container.vertex_buffer,
            vertex_buffer.count
        );
    }
    let indices = index_buffer
        .get(index_base..index_base + index_count)
        .with_context(|| {
            format!(
                "index range {index_base}..{} exceeds buffer {} of {} indices",
                index_base + index_count,
                container.index_buffer,
                index_buffer.len()
            )
        })?;

    let mut vertices = Vec::with_capacity(vertex_count);
    for offset in 0..vertex_count {
        let record = vertex_buffer.record(vertex_base + offset)?;
        let mut vertex = vertex_buffer.layout.decode_as(
            record,
            if visual.kind == VisualKind::Tree {
                crate::soc::codec::vertex::TexCoords::Tree
            } else {
                crate::soc::codec::vertex::TexCoords::Level
            },
        )?;
        // Tree geometry is stored once in its own space and placed by the
        // matrix `xrLC` factored out.
        if let Some(matrix) = &visual.tree_transform {
            vertex.position = transform_position(matrix, vertex.position);
            vertex.normal = transform_direction(matrix, vertex.normal);
            vertex.tangent = transform_direction(matrix, vertex.tangent);
            vertex.binormal = transform_direction(matrix, vertex.binormal);
        }
        vertices.push(vertex);
    }

    // Container indices are relative to `vertex_base`, which the engine passes
    // to the draw call as the start vertex.
    for index in indices {
        if usize::from(*index) >= vertex_count {
            bail!(
                "index {index} is outside the visual's own {vertex_count}-vertex range; the \
                 container range and index buffer disagree"
            );
        }
    }

    Ok(ResolvedVisual {
        shader_id: visual.shader_id,
        vertices,
        indices: indices.to_vec(),
        is_tree: visual.kind == VisualKind::Tree,
    })
}

/// World units one texture-coordinate unit covers, per shader.
///
/// A shader is split into as many surfaces as the merge needed, and its layers
/// have to tile at one size across all of them, so the parts are summed rather
/// than measured apart: the density is the widest part's, which is the one that
/// establishes the scale a player reads.
pub fn uv_world_density(surfaces: &[MergedSurface]) -> BTreeMap<u16, f32> {
    let mut densities: BTreeMap<u16, f32> = BTreeMap::new();
    for surface in surfaces {
        if let Some(density) = surface.uv_world_density() {
            let entry = densities.entry(surface.shader_id).or_insert(0.0);
            *entry = entry.max(density);
        }
    }
    densities
}

/// Distribute surfaces over model archives, at most `per_model` each.
pub fn assign_models(surfaces: &[MergedSurface], per_model: usize) -> Result<Vec<Vec<usize>>> {
    if per_model == 0 {
        bail!("surfaces per model archive must be positive");
    }
    Ok(surfaces
        .chunks(per_model)
        .scan(0_usize, |start, chunk| {
            let range = (*start..*start + chunk.len()).collect();
            *start += chunk.len();
            Some(range)
        })
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::codec::level::VertexBuffer;

    use crate::soc::codec::chunks::Cursor;
    use crate::soc::codec::vertex::{SocVertexLayout, parse_declaration};

    fn float2_layout() -> (SocVertexLayout, usize) {
        let mut bytes = Vec::new();
        for (offset, kind, usage) in [(0_u16, 2_u8, 0_u8), (12, 4, 3), (16, 4, 6), (20, 4, 7)] {
            let mut element = [0_u8; 8];
            element[2..4].copy_from_slice(&offset.to_le_bytes());
            element[4] = kind;
            element[6] = usage;
            bytes.extend(element);
        }
        let mut texcoord = [0_u8; 8];
        texcoord[2..4].copy_from_slice(&24_u16.to_le_bytes());
        texcoord[4] = 1; // FLOAT2
        texcoord[6] = 5; // TEXCOORD
        bytes.extend(texcoord);
        let mut end = [0_u8; 8];
        end[0..2].copy_from_slice(&0xff_u16.to_le_bytes());
        end[4] = 17;
        bytes.extend(end);
        let mut cursor = Cursor::new(&bytes, "decl");
        let layout = SocVertexLayout::resolve(&parse_declaration(&mut cursor).unwrap()).unwrap();
        let stride = layout.stride;
        (layout, stride)
    }

    fn buffers(vertex_count: usize, indices: Vec<u16>) -> GeometryBuffers {
        let (layout, stride) = float2_layout();
        let mut data = vec![0_u8; vertex_count * stride];
        for index in 0..vertex_count {
            let record = &mut data[index * stride..(index + 1) * stride];
            record[0..4].copy_from_slice(&(index as f32).to_le_bytes());
            // +x normal, +y tangent, +z binormal
            record[12..16].copy_from_slice(&0x00_ff_80_80_u32.to_le_bytes());
            record[16..20].copy_from_slice(&0x00_80_ff_80_u32.to_le_bytes());
            record[20..24].copy_from_slice(&0x00_80_80_ff_u32.to_le_bytes());
        }
        GeometryBuffers {
            sliding_windows: Vec::new(),
            vertex_buffers: vec![VertexBuffer {
                layout,
                count: vertex_count,
                data,
            }],
            index_buffers: vec![indices],
        }
    }

    fn visual(kind: VisualKind, shader_id: u16, base: u32, count: u32) -> Visual {
        Visual {
            sliding_window: None,
            kind,
            shader_id,
            bounding_box: ([0.0; 3], [1.0; 3]),
            geometry: Some(GeometryContainer {
                vertex_buffer: 0,
                vertex_base: base,
                vertex_count: count,
                index_buffer: 0,
                index_base: 0,
                index_count: 3,
            }),
            tree_transform: None,
        }
    }

    #[test]
    fn welds_visuals_that_share_a_shader() {
        let buffers = buffers(6, vec![0, 1, 2]);
        let visuals = vec![
            visual(VisualKind::Normal, 4, 0, 3),
            visual(VisualKind::Normal, 4, 3, 3),
        ];
        let (surfaces, report) = merge_visuals(&visuals, &buffers).unwrap();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].name(), "shader_0004_part_000");
        assert_eq!(surfaces[0].material_name(), "shader_0004");
        assert_eq!(surfaces[0].vertices.len(), 6);
        // The second visual's indices are rebased onto the merged buffer.
        assert_eq!(surfaces[0].indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(report.rendered_visuals, 2);
        assert_eq!(report.triangles, 2);
    }

    #[test]
    fn starts_a_new_part_at_the_vertex_limit() {
        let buffers = buffers(9, vec![0, 1, 2]);
        let visuals = vec![
            visual(VisualKind::Normal, 4, 0, 3),
            visual(VisualKind::Normal, 4, 3, 3),
            visual(VisualKind::Normal, 4, 6, 3),
        ];
        let (surfaces, _) = merge_visuals_limited(&visuals, &buffers, 6).unwrap();
        let parts: Vec<(String, usize)> =
            surfaces.iter().map(|s| (s.name(), s.vertices.len())).collect();
        assert_eq!(
            parts,
            vec![("shader_0004_part_000".into(), 6), ("shader_0004_part_001".into(), 3)]
        );
        assert_eq!(surfaces[1].indices, vec![0, 1, 2]);
        assert!(merge_visuals_limited(&visuals, &buffers, 2).is_err());
        assert!(merge_visuals_limited(&visuals, &buffers, MAX_SURFACE_VERTICES + 1).is_err());
    }

    #[test]
    fn keeps_distinct_shaders_apart() {
        let buffers = buffers(6, vec![0, 1, 2]);
        let visuals = vec![
            visual(VisualKind::Normal, 9, 0, 3),
            visual(VisualKind::Normal, 4, 3, 3),
        ];
        let (surfaces, _) = merge_visuals(&visuals, &buffers).unwrap();
        let names: Vec<String> = surfaces.iter().map(MergedSurface::name).collect();
        assert_eq!(names, vec!["shader_0004_part_000", "shader_0009_part_000"]);
    }

    #[test]
    fn records_unsupported_visual_kinds_as_drops() {
        let buffers = buffers(3, vec![0, 1, 2]);
        let mut particle = visual(VisualKind::Unsupported(8), 1, 0, 3);
        particle.kind = VisualKind::Unsupported(8);
        let visuals = vec![visual(VisualKind::Normal, 1, 0, 3), particle];
        let (surfaces, report) = merge_visuals(&visuals, &buffers).unwrap();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(report.dropped_by_kind["mt_8"], 1);
    }

    #[test]
    fn places_tree_geometry_by_its_matrix() {
        let buffers = buffers(3, vec![0, 1, 2]);
        let mut identity = [0.0_f32; 16];
        identity[0] = 1.0;
        identity[5] = 1.0;
        identity[10] = 1.0;
        identity[15] = 1.0;
        identity[13] = 5.0;
        let mut tree = visual(VisualKind::Tree, 2, 0, 3);
        tree.tree_transform = Some(identity);
        let (surfaces, report) = merge_visuals(&[tree], &buffers).unwrap();
        assert_eq!(report.tree_visuals, 1);
        assert!(
            surfaces[0]
                .vertices
                .iter()
                .all(|vertex| vertex.position[1] == 5.0)
        );
        assert_eq!(surfaces[0].tree_visual_count, 1);
    }

    #[test]
    fn encodes_the_target_static_surface_layout() {
        let buffers = buffers(3, vec![0, 1, 2]);
        let (surfaces, _) =
            merge_visuals(&[visual(VisualKind::Normal, 1, 0, 3)], &buffers).unwrap();
        let payload = surfaces[0].encode_converted_model().unwrap();
        assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 3);
        assert_eq!(&payload[8..56], &STATIC_VERTEX_DECLARATION);
        assert_eq!(u32::from_le_bytes(payload[56..60].try_into().unwrap()), 3);
        let vertex_chunk = 48 + 4 + 3 * 32;
        assert_eq!(
            u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize,
            vertex_chunk
        );
        let index_chunk_offset = 8 + vertex_chunk;
        assert_eq!(
            u32::from_le_bytes(
                payload[index_chunk_offset..index_chunk_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            4
        );
        assert_eq!(payload.len(), 8 + vertex_chunk + 8 + 4 + 3 * 2);
    }

    #[test]
    fn computes_bounds_from_converted_positions() {
        let buffers = buffers(3, vec![0, 1, 2]);
        let (surfaces, _) =
            merge_visuals(&[visual(VisualKind::Normal, 1, 0, 3)], &buffers).unwrap();
        let bounds = surfaces[0].bounds();
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [2.0, 0.0, 0.0]);
        assert_eq!(bounds.center, [1.0, 0.0, 0.0]);
        assert!((bounds.radius - 1.0).abs() < 1e-6);
    }

    #[test]
    fn splits_model_archives_at_the_declared_surface_budget() {
        let buffers = buffers(3, vec![0, 1, 2]);
        let visuals: Vec<Visual> = (0..5)
            .map(|shader| visual(VisualKind::Normal, shader, 0, 3))
            .collect();
        let (surfaces, _) = merge_visuals(&visuals, &buffers).unwrap();
        let models = assign_models(&surfaces, 2).unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0], vec![0, 1]);
        assert_eq!(models[2], vec![4]);
    }
}
