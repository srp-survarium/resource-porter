// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode-and-report helpers behind the `lilith-*` CLI probes.
//!
//! The probe is the regression net for the world decoder: it decodes every
//! render surface through its declared vertex layout and rebases every index,
//! so a drifted cursor or a misread layout fails here with coordinates,
//! before any conversion exists to hide it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::SCALE;
use super::codec::world::{self, World};
use super::source::LilithSource;

/// Resolve `office` or `Worlds\...\Office.World00p` to a mounted world path.
pub fn resolve_world(source: &LilithSource, requested: &str) -> Result<String> {
    if requested.to_ascii_lowercase().ends_with(".world00p") {
        let normalized = super::codec::arch::normalize(requested);
        if !source.contains(&normalized) {
            bail!("no mounted archive carries {requested}");
        }
        return Ok(normalized);
    }
    let wanted = format!("\\{}.world00p", requested.to_ascii_lowercase());
    let matches: Vec<String> = source
        .multiplayer_worlds()
        .into_iter()
        .filter(|path| path.ends_with(&wanted))
        .collect();
    match matches.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!(
            "no multiplayer world is named {requested}; lilith-inventory --list-worlds lists them"
        ),
        many => bail!("{requested} is ambiguous: {}", many.join(", ")),
    }
}

/// What one world would give a port.
#[derive(Debug)]
pub struct WorldProbe {
    pub world_models: usize,
    pub physics_shapes: BTreeMap<&'static str, usize>,
    /// Triangles across every physics mesh shape.
    pub physics_triangles: usize,
    pub render_surfaces: usize,
    pub render_materials: usize,
    pub render_vertices: usize,
    pub render_triangles: usize,
    /// Human-readable vertex layouts, e.g. `pos3 nrm3 uv2 tan3 bin3 (44b)`.
    pub layouts: Vec<String>,
    pub objects_by_class: BTreeMap<String, usize>,
    /// `GameStartPoint` positions with instance names, in metres.
    pub start_points: Vec<(Option<String>, [f32; 3])>,
    pub streaming_assets: usize,
    /// World bounds in metres.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

pub fn probe_world(source: &mut LilithSource, world_path: &str) -> Result<WorldProbe> {
    let bytes = source.read(world_path)?;
    let world = world::parse(&bytes, world_path)?;
    probe_parsed(&world, world_path)
}

fn probe_parsed(world: &World, label: &str) -> Result<WorldProbe> {
    let mut physics_shapes = BTreeMap::new();
    let mut physics_triangles = 0;
    for shape in &world.world_models.physics {
        tally_shape(&shape.kind, &mut physics_shapes, &mut physics_triangles);
    }

    let render = &world.render;
    let mut render_vertices = 0;
    let mut render_triangles = 0;
    for index in 0..render.surfaces.len() {
        let decoded = render.decode_surface(index, label)?;
        render_vertices += decoded.positions.len();
        render_triangles += decoded.indices.len() / 3;
    }

    let mut objects_by_class = BTreeMap::new();
    let mut start_points = Vec::new();
    for object in &world.objects {
        *objects_by_class.entry(object.class.clone()).or_insert(0) += 1;
        if object.class == "GameStartPoint" {
            let position = object
                .position()
                .with_context(|| format!("{label}: a GameStartPoint has no Pos"))?;
            start_points.push((
                object.name().map(str::to_owned),
                position.map(|axis| axis * SCALE),
            ));
        }
    }

    Ok(WorldProbe {
        world_models: world.world_models.bsps.len(),
        physics_shapes,
        physics_triangles,
        render_surfaces: render.surfaces.len(),
        render_materials: render.materials.len(),
        render_vertices,
        render_triangles,
        layouts: render.layouts.iter().map(describe_layout).collect(),
        objects_by_class,
        start_points,
        streaming_assets: world.world_models.streaming_assets.len(),
        min: world.header.min.map(|axis| axis * SCALE),
        max: world.header.max.map(|axis| axis * SCALE),
    })
}

fn tally_shape(
    kind: &world::PhysicsShapeKind,
    census: &mut BTreeMap<&'static str, usize>,
    triangles: &mut usize,
) {
    *census.entry(kind.name()).or_insert(0) += 1;
    match kind {
        world::PhysicsShapeKind::Mesh(mesh) => *triangles += mesh.indices.len() / 3,
        world::PhysicsShapeKind::SubShapes(nested) => {
            for shape in nested {
                tally_shape(&shape.kind, census, triangles);
            }
        }
        _ => {}
    }
}

fn describe_layout(layout: &world::VertexLayout) -> String {
    let mut text = String::new();
    for attribute in &layout.attributes {
        let usage = match attribute.usage {
            0 => "pos",
            1 => "blendw",
            2 => "blendi",
            3 => "nrm",
            5 => "uv",
            6 => "tan",
            7 => "bin",
            10 => "col",
            other => return format!("unknown usage {other}"),
        };
        let width = match attribute.kind {
            1 => "2",
            2 => "3",
            3 => "4",
            4 => "c",
            5 => "s",
            other => return format!("unknown kind {other}"),
        };
        if !text.is_empty() {
            text.push(' ');
        }
        let _ = write!(text, "{usage}{width}");
        if attribute.usage_index != 0 {
            let _ = write!(text, ".{}", attribute.usage_index);
        }
    }
    text
}

/// One collision layer found over a probed column.
#[derive(Debug)]
pub struct GroundLayer {
    /// Height of the triangle where it covers the probe point, in metres.
    pub y: f32,
    /// Whether the face's normal points up (standable) or down (ceiling).
    pub upward: bool,
    /// Which conversion source produced it.
    pub source: &'static str,
}

/// Every triangle of the *converted* collision over `(x, z)` metres, highest
/// first -- exactly what the port ships, so a hole here is a hole in the map.
///
/// This probe is what buried the physics-shape theory: the first shipped
/// build converted physics shapes, this column showed them empty where the
/// BSP polygons carried the building, and the player had already measured
/// the difference by falling 4.5 km.
pub fn probe_ground(
    source: &mut LilithSource,
    world_path: &str,
    x: f32,
    z: f32,
) -> Result<Vec<GroundLayer>> {
    let bytes = source.read(world_path)?;
    let world = world::parse(&bytes, world_path)?;
    let mut layers = Vec::new();

    // Solid families as the build sees them, with unreadable or unreviewed
    // materials treated as solid: the probe's job is to say what a player
    // stands on, and an over-approximation is visible where a silent gap
    // is not.
    let mut solid = std::collections::BTreeSet::new();
    for (index, path) in world.render.materials.iter().enumerate() {
        let is_solid = match source.read(path) {
            Ok(bytes) => match super::codec::mat::parse(&bytes, path) {
                Ok(material) => {
                    match super::convert::render_material::classify(
                        &material.shader,
                        &BTreeMap::new(),
                    ) {
                        Ok(Some(family)) => super::convert::collision::family_is_solid(family),
                        Ok(None) => false,
                        Err(_) => true,
                    }
                }
                Err(_) => true,
            },
            Err(_) => true,
        };
        if is_solid {
            solid.insert(index as u32);
        }
    }
    let (cform, _) = super::convert::collision::build_cform(
        &world.world_models,
        &world.render,
        &solid,
        world_path,
    )?;
    for face in &cform.faces {
        let triangle = face.vertices.map(|index| cform.vertices[index as usize]);
        if let Some((y, upward)) = height_over(&triangle, x, z) {
            layers.push(GroundLayer {
                y,
                upward,
                source: "cform",
            });
        }
    }

    layers.sort_by(|left, right| right.y.total_cmp(&left.y));
    Ok(layers)
}

/// The height at which a triangle's XZ footprint covers `(x, z)`, if it does.
fn height_over(triangle: &[[f32; 3]; 3], x: f32, z: f32) -> Option<(f32, bool)> {
    let [a, b, c] = triangle;
    let denominator = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if denominator.abs() < 1e-12 {
        return None;
    }
    let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / denominator;
    let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / denominator;
    let w = 1.0 - u - v;
    if u < 0.0 || v < 0.0 || w < 0.0 {
        return None;
    }
    let y = u * a[1] + v * b[1] + w * c[1];
    // The face's vertical orientation, from the triangle normal's y sign,
    // with "up" calibrated to the winding the engine lets a player stand on.
    let normal_y = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
    Some((y, normal_y > 0.0))
}

/// Every brush BSP polygon over `(x, z)` metres with its raw surface flags,
/// highest first — for working out what a flag bit means by looking at what
/// carries it. `(height, flags, bsp_index)`.
pub fn probe_bsp_flags(
    source: &mut LilithSource,
    world_path: &str,
    x: f32,
    z: f32,
) -> Result<Vec<(f32, u16, usize)>> {
    let bytes = source.read(world_path)?;
    let world = world::parse(&bytes, world_path)?;
    let mut hits = Vec::new();
    for (index, bsp) in world.world_models.bsps.iter().enumerate() {
        for polygon in &bsp.polygons {
            let points: Vec<[f32; 3]> = polygon
                .vertices
                .iter()
                .map(|&vertex| bsp.points[vertex as usize].map(|axis| axis * SCALE))
                .collect();
            for corner in 1..points.len().saturating_sub(1) {
                let triangle = [points[0], points[corner], points[corner + 1]];
                if let Some((y, _)) = height_over(&triangle, x, z) {
                    hits.push((y, polygon.surface_flags, index));
                }
            }
        }
    }
    hits.sort_by(|left, right| right.0.total_cmp(&left.0));
    hits.dedup();
    Ok(hits)
}

/// How much floor-to-ceiling clearance a spawn needs, in metres.
///
/// Empirical, from the client: the position the engine takes is the capsule's
/// *feet* (the walk telemetry tracks floor heights exactly), and Office's
/// 3.5 m rooms rejected a spawn authored 1.5 m up -- 2.0 m of headroom was
/// not enough -- while bath's 2.7 m of headroom in the SoC port was. So the
/// capsule is 2.0..2.7 m tall; a spawn placed [`SPAWN_FLOOR_CLEARANCE`]
/// above the floor fits wherever the room clears this.
const SPAWN_MIN_ROOM: f32 = 3.3;
/// Metres above the floor the adjusted spawn is placed.
const SPAWN_FLOOR_CLEARANCE: f32 = 0.55;

/// One authored start point judged against the converted collision.
#[derive(Debug)]
pub struct SpawnCandidate {
    pub name: Option<String>,
    /// Authored position, metres.
    pub position: [f32; 3],
    /// The floor under the point, if any.
    pub floor: Option<f32>,
    /// The first collision above the floor, if any.
    pub ceiling: Option<f32>,
    /// Where to actually spawn: just above the floor, clear of the ceiling.
    pub adjusted: Option<[f32; 3]>,
    /// Whether the capsule fits -- see [`SPAWN_MIN_ROOM`].
    pub usable: bool,
}

/// Judge every `GameStartPoint` of a world for spawn usability.
pub fn spawn_candidates(
    source: &mut LilithSource,
    world_path: &str,
) -> Result<Vec<SpawnCandidate>> {
    // (floor under the authored point, first layer above that floor).
    let room_at = |cform: &crate::soc::codec::cform::Cform,
                   x: f32,
                   z: f32,
                   authored: f32|
     -> (Option<f32>, Option<f32>) {
        let mut floor: Option<f32> = None;
        let mut ceiling: Option<f32> = None;
        for face in &cform.faces {
            let triangle = face.vertices.map(|index| cform.vertices[index as usize]);
            if let Some((y, _)) = height_over(&triangle, x, z) {
                if y <= authored + 0.1 {
                    floor = Some(floor.map_or(y, |current: f32| current.max(y)));
                } else {
                    ceiling = Some(ceiling.map_or(y, |current: f32| current.min(y)));
                }
            }
        }
        (floor, ceiling)
    };

    let bytes = source.read(world_path)?;
    let world = world::parse(&bytes, world_path)?;
    let mut solid = std::collections::BTreeSet::new();
    for (index, path) in world.render.materials.iter().enumerate() {
        let is_solid = match source.read(path) {
            Ok(bytes) => match super::codec::mat::parse(&bytes, path) {
                Ok(material) => match super::convert::render_material::classify(
                    &material.shader,
                    &BTreeMap::new(),
                ) {
                    Ok(Some(family)) => super::convert::collision::family_is_solid(family),
                    Ok(None) => false,
                    Err(_) => true,
                },
                Err(_) => true,
            },
            Err(_) => true,
        };
        if is_solid {
            solid.insert(index as u32);
        }
    }
    let (cform, _) = super::convert::collision::build_cform(
        &world.world_models,
        &world.render,
        &solid,
        world_path,
    )?;

    let mut candidates = Vec::new();
    for object in &world.objects {
        if object.class != "GameStartPoint" {
            continue;
        }
        let position = object
            .position()
            .with_context(|| format!("{world_path}: a GameStartPoint has no Pos"))?
            .map(|axis| axis * SCALE);
        let (floor, ceiling) = room_at(&cform, position[0], position[2], position[1]);
        let usable = floor.is_some_and(|floor| {
            // The floor must be the point's own -- authored ~1.5 m under it,
            // not a storey further down through a hole in the mesh.
            let authored_gap = position[1] - floor;
            let room = ceiling.map_or(f32::INFINITY, |ceiling| ceiling - floor);
            (0.2..=2.6).contains(&authored_gap) && room >= SPAWN_MIN_ROOM
        });
        let adjusted = floor
            .filter(|_| usable)
            .map(|floor| [position[0], floor + SPAWN_FLOOR_CLEARANCE, position[2]]);
        candidates.push(SpawnCandidate {
            name: object.name().map(str::to_owned),
            position,
            floor,
            ceiling,
            adjusted,
            usable,
        });
    }
    // Usable points first, each group in authored order.
    candidates.sort_by_key(|candidate| !candidate.usable);
    Ok(candidates)
}

/// Which shaders the mounted worlds' materials use, and how often.
///
/// The FEAR analogue of `soc-audit-shaders`: every `.Mat00` a world's render
/// mesh references is parsed and grouped by its `.fx` path, because the
/// shader path is what a port family classification will key on.
pub fn audit_shaders(
    source: &mut LilithSource,
    worlds: &[String],
) -> Result<BTreeMap<String, ShaderUse>> {
    let mut census: BTreeMap<String, ShaderUse> = BTreeMap::new();
    for world_path in worlds {
        let bytes = source.read(world_path)?;
        let header = world::parse_header(&bytes, world_path)?;
        let render = world::parse_render(&bytes, &header, world_path)?;
        for material_path in &render.materials {
            let Ok(bytes) = source.read(material_path) else {
                census
                    .entry(format!("<missing material> {material_path}"))
                    .or_default()
                    .materials += 1;
                continue;
            };
            let material = super::codec::mat::parse(&bytes, material_path)?;
            let entry = census
                .entry(material.shader.to_ascii_lowercase())
                .or_default();
            entry.materials += 1;
            if entry.example.is_empty() {
                entry.example = material_path.clone();
            }
            if material.diffuse().is_some() {
                entry.with_diffuse += 1;
            }
            if material.normal().is_some() {
                entry.with_normal += 1;
            }
            if material.specular().is_some() {
                entry.with_specular += 1;
            }
        }
    }
    Ok(census)
}

/// One shader's usage across the audited worlds.
#[derive(Debug, Default)]
pub struct ShaderUse {
    pub materials: usize,
    pub with_diffuse: usize,
    pub with_normal: usize,
    pub with_specular: usize,
    pub example: String,
}

/// Write one world's render geometry as OBJ, one group per surface, positions
/// in metres. Returns (surfaces, vertices, triangles).
pub fn dump_obj(
    source: &mut LilithSource,
    world_path: &str,
    output: &Path,
) -> Result<(usize, usize, usize)> {
    let bytes = source.read(world_path)?;
    let world = world::parse(&bytes, world_path)?;
    let render = &world.render;

    let mut text = String::new();
    let _ = writeln!(text, "# {world_path}");
    let mut base = 1usize; // OBJ indices are 1-based
    let mut vertices = 0;
    let mut triangles = 0;
    for (index, surface) in render.surfaces.iter().enumerate() {
        let decoded = render.decode_surface(index, world_path)?;
        let material = &render.materials[surface.material as usize];
        let _ = writeln!(text, "o surface_{index:04}");
        let _ = writeln!(text, "usemtl {}", material.replace(['\\', ' '], "_"));
        let textured = decoded.uvs.len() == decoded.positions.len();
        let lit = decoded.normals.len() == decoded.positions.len();
        for position in &decoded.positions {
            let _ = writeln!(
                text,
                "v {} {} {}",
                position[0] * SCALE,
                position[1] * SCALE,
                position[2] * SCALE
            );
        }
        for uv in &decoded.uvs {
            let _ = writeln!(text, "vt {} {}", uv[0], 1.0 - uv[1]);
        }
        for normal in &decoded.normals {
            let _ = writeln!(text, "vn {} {} {}", normal[0], normal[1], normal[2]);
        }
        for triangle in decoded.indices.chunks_exact(3) {
            let corner = |vertex: u32| {
                let id = base + vertex as usize;
                match (textured, lit) {
                    (true, true) => format!("{id}/{id}/{id}"),
                    (true, false) => format!("{id}/{id}"),
                    (false, true) => format!("{id}//{id}"),
                    (false, false) => format!("{id}"),
                }
            };
            let _ = writeln!(
                text,
                "f {} {} {}",
                corner(triangle[0]),
                corner(triangle[1]),
                corner(triangle[2])
            );
        }
        base += decoded.positions.len();
        vertices += decoded.positions.len();
        triangles += decoded.indices.len() / 3;
    }
    fs::write(output, text).with_context(|| format!("writing {}", output.display()))?;
    Ok((render.surfaces.len(), vertices, triangles))
}
