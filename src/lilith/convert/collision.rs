//! Turn a world's visible geometry into the SoC pipeline's collision mesh.
//!
//! Jupiter EX carries three collision-shaped things per world, and none of
//! the two that *look* like collision is complete:
//!
//! * the **physics shapes** -- per-model rigid-body proxies (mesh soups,
//!   OBBs, spheres, capsules) with mass and density. Probing Office's spawn
//!   column finds nothing: the first shipped build converted these, and the
//!   player fell 4.5 km through the map before the theory died;
//! * the **brush BSP polygons**. Probing the same column finds the building
//!   here -- but the engine's real collision is the solid-leaf BSP *tree*,
//!   and the stored polygons alone are not a closed surface: one column
//!   inside Office ([67.5, -2.0]) has no polygon at all where the floor
//!   visibly is, one metre from three coincident copies of the same slab
//!   face. The second shipped build walked until that hole;
//! * the **render mesh** -- complete by construction, because it is what
//!   the player sees. A player walks on what is drawn.
//!
//! So the conversion takes the **union**: render surfaces of the families
//! that read as solid -- the caller says which, since family classification
//! lives with the materials -- plus the brush BSP polygons, plus the
//! authored blocker polygons. Each covers the others' gaps: the BSP carries
//! the invisible ramp clips mappers laid over stairs (the visible treads are
//! unconverted prop models) and the seals the render never draws, while the
//! render carries the floors the polygon soup lost. Coincident faces where
//! the two agree are harmless. Physics shapes are not converted.
//!
//! Every face takes physical material id 0: F.E.A.R. attaches surface types
//! to materials rather than collision faces, and translating that per-face
//! is future work. One material means one face-dictionary entry, which is
//! also the only shape the engine has been proven to load.

use std::collections::BTreeSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::lilith::SCALE;
use crate::lilith::codec::world::{RenderMesh, WorldModels};
use crate::soc::codec::cform::{Cform, CformFace, CformHeader};
use crate::soc::convert::render_material::RenderFamily;

/// Whether a render family's surfaces block movement.
///
/// Glass does: a shot-out pane never shatters in the port, so it stays a
/// wall. Water does not, for the same reason the SoC path drops X-Ray's
/// water collision: converting it makes a pool walkable. Emissive glows and
/// decals are overlays on other surfaces -- an additive light shaft made
/// solid is an invisible wall across a room.
pub fn family_is_solid(family: RenderFamily) -> bool {
    matches!(
        family,
        RenderFamily::Opaque
            | RenderFamily::VertexLit
            | RenderFamily::AlphaTested
            | RenderFamily::VertexLitAlphaTested
            | RenderFamily::Translucent
            | RenderFamily::Foliage
            | RenderFamily::Bark
            | RenderFamily::Terrain
    )
}

/// What the conversion took from each source, for the build record.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CollisionReport {
    /// Triangles from render surfaces of solid families.
    pub render_triangles: usize,
    /// Render surfaces excluded because their family is not solid.
    pub passable_surfaces: usize,
    /// Triangles from brush BSP polygons, world and placed models alike.
    pub bsp_triangles: usize,
    /// Brush models contributing no polygons (empty BSPs exist in retail).
    pub empty_models: usize,
    /// Triangles from authored blocker polygons.
    pub blocker_triangles: usize,
    /// Polygons skipped for having fewer than three vertices.
    pub degenerate_polygons: usize,
}

/// Longest triangle edge the emitted collision keeps, in metres.
///
/// Brush faces arrive as a handful of huge triangles -- a floor slab is two
/// triangles thirty metres long -- where X-Ray collision, the only shape this
/// engine is known to cook correctly, is dense and small. Shipped unsplit,
/// the cooked result behaved like a bad spatial hash: standable near parts
/// of a slab, holes at its far corners, and a phantom solid where the spawn
/// room's interior should be. Subdividing to X-Ray-like density removes the
/// difference in kind between this source and the one the cooker grew up on.
const MAX_EDGE_METRES: f32 = 4.0;

/// Collect every walkable triangle into one collision mesh, in metres.
///
/// `solid_materials` holds the render-material indices whose family blocks
/// movement; surfaces of any other material are counted and left passable.
pub fn build_cform(
    world_models: &WorldModels,
    render: &RenderMesh,
    solid_materials: &BTreeSet<u32>,
    label: &str,
) -> Result<(Cform, CollisionReport)> {
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut faces: Vec<CformFace> = Vec::new();
    let mut report = CollisionReport::default();

    for (index, surface) in render.surfaces.iter().enumerate() {
        if !solid_materials.contains(&surface.material) {
            report.passable_surfaces += 1;
            continue;
        }
        let decoded = render.decode_surface(index, label)?;
        for triangle in decoded.indices.chunks_exact(3) {
            let corner =
                |slot: usize| decoded.positions[triangle[slot] as usize].map(|axis| axis * SCALE);
            report.render_triangles += push_subdivided(
                &mut vertices,
                &mut faces,
                [corner(0), corner(1), corner(2)],
                0,
            );
        }
    }

    for bsp in &world_models.bsps {
        if bsp.polygons.is_empty() {
            report.empty_models += 1;
            continue;
        }
        for polygon in &bsp.polygons {
            if polygon.vertices.len() < 3 {
                report.degenerate_polygons += 1;
                continue;
            }
            let point = |corner: usize| {
                bsp.points[polygon.vertices[corner] as usize].map(|axis| axis * SCALE)
            };
            for corner in 1..polygon.vertices.len() - 1 {
                report.bsp_triangles += push_subdivided(
                    &mut vertices,
                    &mut faces,
                    [point(0), point(corner + 1), point(corner)],
                    0,
                );
            }
        }
    }

    for blocker in &world_models.blockers {
        if blocker.len() < 3 {
            report.degenerate_polygons += 1;
            continue;
        }
        let point = |corner: usize| blocker[corner].map(|axis| axis * SCALE);
        // A blocker fences movement from both sides; emit both windings so
        // neither approach direction depends on how the cooker reads facing.
        for corner in 1..blocker.len() - 1 {
            report.blocker_triangles += push_subdivided(
                &mut vertices,
                &mut faces,
                [point(0), point(corner + 1), point(corner)],
                0,
            );
            report.blocker_triangles += push_subdivided(
                &mut vertices,
                &mut faces,
                [point(0), point(corner), point(corner + 1)],
                0,
            );
        }
    }

    if faces.is_empty() {
        bail!("{label}: the world's brush geometry produced no collision triangles");
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    let header = CformHeader {
        version: 4,
        vertex_count: u32::try_from(vertices.len())?,
        face_count: u32::try_from(faces.len())?,
        bounding_box: (min, max),
    };
    Ok((
        Cform {
            header,
            vertices,
            faces,
        },
        report,
    ))
}

/// Emit one triangle, split until no edge exceeds [`MAX_EDGE_METRES`].
/// Returns how many triangles were emitted.
fn push_subdivided(
    vertices: &mut Vec<[f32; 3]>,
    faces: &mut Vec<CformFace>,
    triangle: [[f32; 3]; 3],
    depth: usize,
) -> usize {
    let edge = |a: [f32; 3], b: [f32; 3]| -> f32 {
        (0..3).map(|axis| (a[axis] - b[axis]).powi(2)).sum::<f32>()
    };
    let lengths = [
        edge(triangle[1], triangle[2]),
        edge(triangle[0], triangle[2]),
        edge(triangle[0], triangle[1]),
    ];
    let longest = (0..3).max_by(|&a, &b| lengths[a].total_cmp(&lengths[b])).unwrap();
    // The depth guard only exists to make degenerate float behaviour finite;
    // 4m edges under a 300m map are eight halvings away.
    if lengths[longest] <= MAX_EDGE_METRES * MAX_EDGE_METRES || depth >= 12 {
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&triangle);
        faces.push(CformFace {
            vertices: [base, base + 1, base + 2],
            material: 0,
            suppress_shadows: false,
            suppress_wallmarks: false,
            sector: 0,
        });
        return 1;
    }
    // Split the longest edge at its midpoint; the two halves keep the
    // original winding.
    let (a, b) = ((longest + 1) % 3, (longest + 2) % 3);
    let midpoint: [f32; 3] =
        std::array::from_fn(|axis| (triangle[a][axis] + triangle[b][axis]) * 0.5);
    let mut first = triangle;
    first[b] = midpoint;
    let mut second = triangle;
    second[a] = midpoint;
    push_subdivided(vertices, faces, first, depth + 1)
        + push_subdivided(vertices, faces, second, depth + 1)
}
