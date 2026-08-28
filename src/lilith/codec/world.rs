//! The Jupiter EX compiled world, `.World00p` version 113.
//!
//! One file carries everything the engine loads for a map, in five sections
//! the header points at:
//!
//! ```text
//! header       version, four section offsets, world bounds
//! world models brush BSPs by name, one physics shape per BSP, streaming assets
//! render       one world mesh: raw vertex/index blocks, vertex layout
//!              definitions, surfaces referencing materials by name
//! sectors      portal visibility -- decoded structurally, unused by the port
//! objects      every placed entity as a class name plus typed properties
//! blind data   keyframers, AI navmesh, shatter/scatter -- not read at all
//! ```
//!
//! The layout follows Five-Damned-Dollarz's public 010Editor template
//! (gist `56fb8f497056ec443021ea4aad71409c`), corrected where the F.E.A.R.
//! retail worlds disagree; divergences are flagged inline. The eight counts at
//! the head of the world-model section are XORed with a magic derived from the
//! version -- `113 + 'F'+'E'+'A'+'R' = 399` -- which is obfuscation, not
//! encryption.
//!
//! Sections decode independently: a probe of the object list must not depend
//! on the physics decoder surviving every world, so [`parse`] is a
//! convenience over the four public section parsers.

use anyhow::{Context, Result, bail};

use crate::soc::codec::chunks::Cursor;

pub const VERSION: u32 = 113;
/// `version + 'F' + 'E' + 'A' + 'R'`.
const MAGIC_XOR: u32 = VERSION + 70 + 69 + 65 + 82;

// ---------------------------------------------------------------------------
// header

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHeader {
    pub render_offset: u32,
    pub sector_offset: u32,
    pub object_offset: u32,
    pub blind_data_offset: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// The template calls this `offset`; every observed world has zeroes.
    pub world_offset: [f32; 3],
}

pub fn parse_header(bytes: &[u8], label: &str) -> Result<WorldHeader> {
    let mut cursor = Cursor::new(bytes, label);
    let version = cursor.u32()?;
    if version != VERSION {
        bail!("{label}: world version {version}, only {VERSION} is understood");
    }
    let header = WorldHeader {
        render_offset: cursor.u32()?,
        sector_offset: cursor.u32()?,
        object_offset: cursor.u32()?,
        blind_data_offset: cursor.u32()?,
        min: cursor.float3()?,
        max: cursor.float3()?,
        world_offset: cursor.float3()?,
    };
    let ordered = [
        header.render_offset,
        header.sector_offset,
        header.object_offset,
        header.blind_data_offset,
    ];
    if ordered.windows(2).any(|pair| pair[0] >= pair[1])
        || (header.blind_data_offset as usize) > bytes.len()
    {
        bail!(
            "{label}: section offsets {ordered:?} are not ascending within {} bytes",
            bytes.len()
        );
    }
    Ok(header)
}

// ---------------------------------------------------------------------------
// world models

/// One polygon of a brush BSP.
#[derive(Clone, Debug)]
pub struct BspPolygon {
    pub surface_flags: u16,
    /// Index into [`WorldModels::planes`].
    pub plane: u32,
    pub plane_distance: f32,
    /// Indices into the owning BSP's `points`.
    pub vertices: Vec<u32>,
}

/// One named brush model -- the world itself is BSP 0.
#[derive(Clone, Debug)]
pub struct Bsp {
    pub id: u32,
    pub half_extents: [f32; 3],
    pub center: [f32; 3],
    pub polygons: Vec<BspPolygon>,
    pub points: Vec<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct PhysicsMesh {
    pub mass: f32,
    pub density: f32,
    pub points: Vec<[f32; 3]>,
    /// Triangle list, three indices per triangle.
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct PhysicsHull {
    pub mass: f32,
    pub density: f32,
    pub vertices: Vec<[f32; 3]>,
    /// Plane normals with distances.
    pub planes: Vec<([f32; 3], f32)>,
}

#[derive(Clone, Debug)]
pub enum PhysicsShapeKind {
    Null,
    Mesh(PhysicsMesh),
    Obb {
        mass: f32,
        density: f32,
        dimensions: [f32; 3],
    },
    Sphere {
        mass: f32,
        density: f32,
        radius: f32,
    },
    Hull(PhysicsHull),
    Capsule {
        mass: f32,
        density: f32,
        radius: f32,
        ends: [[f32; 3]; 2],
    },
    SubShapes(Vec<PhysicsShape>),
}

#[derive(Clone, Debug)]
pub struct PhysicsShape {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub kind: PhysicsShapeKind,
}

impl PhysicsShapeKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Mesh(_) => "mesh",
            Self::Obb { .. } => "obb",
            Self::Sphere { .. } => "sphere",
            Self::Hull(_) => "hull",
            Self::Capsule { .. } => "capsule",
            Self::SubShapes(_) => "sub_shapes",
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorldModels {
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Shared plane normals the BSP polygons index.
    pub planes: Vec<[f32; 3]>,
    /// BSP name per BSP index, from the section's string table.
    pub names: Vec<String>,
    pub bsps: Vec<Bsp>,
    /// One shape per BSP, same order.
    pub physics: Vec<PhysicsShape>,
    /// Authored movement clips: invisible polygons that fence the playable
    /// area, each a planar fan of world-space points.
    pub blockers: Vec<Vec<[f32; 3]>>,
    /// `folder\file` pairs of every asset the world streams -- the world's
    /// own material and texture census.
    pub streaming_assets: Vec<String>,
}

pub fn parse_world_models(bytes: &[u8], header: &WorldHeader, label: &str) -> Result<WorldModels> {
    // The section begins immediately after the 56-byte header.
    let mut cursor = Cursor::new(
        bytes
            .get(56..header.render_offset as usize)
            .with_context(|| format!("{label}: world-model section out of range"))?,
        label,
    );
    let min = cursor.float3()?;
    let max = cursor.float3()?;
    let model_count = cursor.u32()? as usize;
    let zero = cursor.u32()?;
    if zero != 0 {
        bail!("{label}: world-model section reserved word is {zero}, not 0");
    }
    cursor.take(model_count.div_ceil(8))?; // subdivision bitflags

    let name_count = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let names_length = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let plane_count = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let bsp_count = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let total_nodes = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let total_polygons = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let total_unknown_counts = (cursor.u32()? ^ MAGIC_XOR) as usize;
    let total_points = (cursor.u32()? ^ MAGIC_XOR) as usize;

    let names_data = cursor.take(names_length)?.to_vec();
    let mut names = vec![String::new(); name_count];
    for slot in 0..name_count {
        let offset = cursor.u32()? as usize;
        let id = cursor.u32()? as usize;
        let tail = names_data.get(offset..).with_context(|| {
            format!("{label}: BSP name offset {offset} outside the {names_length}-byte table")
        })?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("{label}: unterminated BSP name at offset {offset}"))?;
        let name = std::str::from_utf8(&tail[..end])
            .with_context(|| format!("{label}: non-ASCII BSP name at offset {offset}"))?;
        let entry = names
            .get_mut(id)
            .with_context(|| format!("{label}: BSP name {slot} names id {id} of {name_count}"))?;
        *entry = name.to_owned();
    }

    let mut planes = Vec::with_capacity(plane_count);
    for _ in 0..plane_count {
        planes.push(cursor.float3()?);
    }

    let mut bsps = Vec::with_capacity(bsp_count);
    let (mut seen_nodes, mut seen_polygons, mut seen_refs, mut seen_points) = (0, 0, 0, 0);
    for index in 0..bsp_count {
        let id = cursor.u32()?;
        let point_count = cursor.u32()? as usize;
        let polygon_count = cursor.u32()? as usize;
        // The template calls this `unk_count_1`. On small brush models it
        // equals the polygon vertex-reference sum; on the world BSP it is
        // larger -- close to, but not exactly, the fan-triangulated index
        // count. Uninterpreted, but summed against the section total below.
        let unknown_count = cursor.u32()? as usize;
        let node_count = cursor.u32()? as usize;
        let half_extents = cursor.float3()?;
        let center = cursor.float3()?;
        // The template calls this `zero`, but empty brush models carry -2
        // here -- a node sentinel, not padding -- so it is read, not checked.
        let _root = cursor.i32()?;
        let vertex_counts = cursor.take(polygon_count)?.to_vec();
        let mut polygons = Vec::with_capacity(polygon_count);
        for &count in &vertex_counts {
            let _unk = cursor.take(2)?;
            let surface_flags = cursor.u16()?;
            let plane = cursor.u32()?;
            if plane as usize >= plane_count {
                bail!("{label}: BSP {index} polygon names plane {plane} of {plane_count}");
            }
            let plane_distance = cursor.f32()?;
            let mut vertices = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let vertex = cursor.u32()?;
                if vertex as usize >= point_count {
                    bail!("{label}: BSP {index} polygon names point {vertex} of {point_count}");
                }
                vertices.push(vertex);
            }
            polygons.push(BspPolygon {
                surface_flags,
                plane,
                plane_distance,
                vertices,
            });
        }
        cursor.take(node_count * 12)?; // BSP nodes: three i32s each, unused
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            points.push(cursor.float3()?);
        }
        seen_nodes += node_count;
        seen_polygons += polygon_count;
        seen_refs += unknown_count;
        seen_points += point_count;
        bsps.push(Bsp {
            id,
            half_extents,
            center,
            polygons,
            points,
        });
    }
    // The section totals exist to be checked: a drifted cursor produces
    // plausible-looking garbage, and these four sums are the tripwire.
    if (seen_nodes, seen_polygons, seen_refs, seen_points)
        != (
            total_nodes,
            total_polygons,
            total_unknown_counts,
            total_points,
        )
    {
        bail!(
            "{label}: section declares {total_nodes} nodes / {total_polygons} polygons / \
             {total_unknown_counts} unknown-counts / {total_points} points, but the BSPs \
             carry {seen_nodes} / {seen_polygons} / {seen_refs} / {seen_points}"
        );
    }

    // Blocker polygons: authored movement clips. The plane is redundant with
    // the points and dropped; the points are the collision.
    let blocker_count = cursor.u32()? as usize;
    let _unknown = cursor.u32()?;
    let mut blockers = Vec::with_capacity(blocker_count);
    for _ in 0..blocker_count {
        cursor.take(16)?; // plane
        let vec_count = cursor.u32()? as usize;
        let mut points = Vec::with_capacity(vec_count);
        for _ in 0..vec_count {
            points.push(cursor.float3()?);
        }
        blockers.push(points);
    }

    let shape_count = cursor.u32()? as usize;
    if shape_count != bsp_count {
        bail!("{label}: {shape_count} physics shapes for {bsp_count} BSPs");
    }
    // A nested capsule's optional trailer has no reliable local discriminator
    // -- see `parse_nested_shape` -- so ambiguous sites are resolved by
    // search: parse the rest of the section under each combination of
    // trailer decisions until the terminal padding and asset table validate.
    // Worlds carry a handful of capsules, so the space is tiny.
    let mut solved = None;
    let mut last_error = None;
    let mut flips: u64 = 0;
    let mut widest_sites = 0;
    loop {
        let mut attempt = cursor;
        let mut choices = TrailerChoices { flips, sites: 0 };
        let result = (|| -> Result<(Vec<PhysicsShape>, Vec<String>)> {
            let mut physics = Vec::with_capacity(shape_count);
            for index in 0..shape_count {
                physics.push(parse_physics_shape(
                    &mut attempt,
                    label,
                    index,
                    &mut choices,
                )?);
            }
            let assets = parse_streaming_assets(&mut attempt, label)?;
            Ok((physics, assets))
        })();
        match result {
            Ok(parsed) => {
                solved = Some(parsed);
                break;
            }
            Err(error) => {
                widest_sites = widest_sites.max(choices.sites);
                last_error = Some(error);
                flips += 1;
                if widest_sites >= 10 || flips >= 1 << widest_sites {
                    break;
                }
            }
        }
    }
    let (physics, streaming_assets) = match solved {
        Some(parsed) => parsed,
        None => {
            return Err(last_error.unwrap().context(format!(
                "{label}: no combination of capsule-trailer decisions parses the \
                 physics shapes"
            )));
        }
    };
    Ok(WorldModels {
        min,
        max,
        planes,
        names,
        bsps,
        physics,
        blockers,
        streaming_assets,
    })
}

/// The trailer decisions one parse attempt runs under; see
/// [`parse_nested_shape`].
struct TrailerChoices {
    /// Bit `n` flips the heuristic's answer at the `n`-th ambiguous site.
    flips: u64,
    /// Ambiguous sites encountered so far in this attempt.
    sites: usize,
}

fn parse_physics_shape(
    cursor: &mut Cursor,
    label: &str,
    index: usize,
    choices: &mut TrailerChoices,
) -> Result<PhysicsShape> {
    let position = cursor.float3()?;
    let rotation = [cursor.f32()?, cursor.f32()?, cursor.f32()?, cursor.f32()?];
    let kind = cursor.u32()?;
    let kind = match kind {
        0 => PhysicsShapeKind::Null,
        1 => {
            let mass = cursor.f32()?;
            let density = cursor.f32()?;
            cursor.take(4 * 12)?; // four vectors the template leaves unnamed
            cursor.take(8)?;
            let point_count = cursor.u32()? as usize;
            let index_count = cursor.u32()? as usize;
            let mut points = Vec::with_capacity(point_count);
            for _ in 0..point_count {
                points.push(cursor.float3()?);
            }
            let mut indices = Vec::with_capacity(index_count);
            if point_count < 0x10000 {
                for _ in 0..index_count {
                    indices.push(u32::from(cursor.u16()?));
                }
            } else {
                for _ in 0..index_count {
                    indices.push(cursor.u32()?);
                }
            }
            if index_count % 3 != 0 {
                bail!(
                    "{label}: physics mesh {index} has {index_count} indices, \
                     which is not a triangle list"
                );
            }
            if let Some(out_of_range) = indices.iter().find(|&&i| i as usize >= point_count) {
                bail!("{label}: physics mesh {index} names point {out_of_range} of {point_count}");
            }
            cursor.take(16)?; // physics_props
            let trailing = cursor.u32()? as usize;
            cursor.take(trailing)?;
            PhysicsShapeKind::Mesh(PhysicsMesh {
                mass,
                density,
                points,
                indices,
            })
        }
        2 => {
            let mass = cursor.f32()?;
            let density = cursor.f32()?;
            let _unknown = cursor.f32()?;
            PhysicsShapeKind::Obb {
                mass,
                density,
                dimensions: cursor.float3()?,
            }
        }
        3 => PhysicsShapeKind::Sphere {
            mass: cursor.f32()?,
            density: cursor.f32()?,
            radius: cursor.f32()?,
        },
        4 => {
            let mass = cursor.f32()?;
            let density = cursor.f32()?;
            cursor.take(4 * 12)?;
            cursor.take(8)?;
            let vertex_count = cursor.u32()? as usize;
            let plane_count = cursor.u32()? as usize;
            let mut vertices = Vec::with_capacity(vertex_count);
            for _ in 0..vertex_count {
                vertices.push(cursor.float3()?);
            }
            let mut planes = Vec::with_capacity(plane_count);
            for _ in 0..plane_count {
                planes.push((cursor.float3()?, cursor.f32()?));
            }
            PhysicsShapeKind::Hull(PhysicsHull {
                mass,
                density,
                vertices,
                planes,
            })
        }
        6 => {
            let nested_count = cursor.u32()? as usize;
            let mut nested = Vec::with_capacity(nested_count);
            for sub in 0..nested_count {
                nested.push(parse_nested_shape(
                    cursor,
                    label,
                    index * 1000 + sub,
                    choices,
                )?);
            }
            PhysicsShapeKind::SubShapes(nested)
        }
        7 => {
            let mass = cursor.f32()?;
            let density = cursor.f32()?;
            let radius = cursor.f32()?;
            let ends = [cursor.float3()?, cursor.float3()?];
            PhysicsShapeKind::Capsule {
                mass,
                density,
                radius,
                ends,
            }
        }
        other => bail!("{label}: physics shape {index} has unknown type {other}"),
    };
    Ok(PhysicsShape {
        position,
        rotation,
        kind,
    })
}

/// One entry of a sub-shape container.
///
/// The grammar differs from the top-level array, recovered byte by byte from
/// `Office.World00p` and `MP_05_Train_Yard.World00p` (the 010 template
/// resorts to a lookahead hack here and misreads these very containers):
///
/// ```text
/// u32 5    a marker, then an ordinary transform + type + payload record
/// u32 7    a capsule with no leading transform: mass, density, radius,
///          both end points -- then, on some capsules only, a trailer of
///          `pos, quat, u32 count, count * vec3`
/// ```
///
/// Nothing before the trailer says whether it is present, and the local
/// plausibility test -- small count, unit quaternion -- can be satisfied by
/// the *next shape's* record when that shape sits at the identity transform,
/// as Train Yard's does. So a plausible site consults [`TrailerChoices`]:
/// the section-level search re-parses under flipped decisions until the
/// terminal padding and asset table validate, which a wrong decision cannot
/// reach.
fn parse_nested_shape(
    cursor: &mut Cursor,
    label: &str,
    index: usize,
    choices: &mut TrailerChoices,
) -> Result<PhysicsShape> {
    let marker = cursor.u32()?;
    match marker {
        5 => parse_physics_shape(cursor, label, index, choices),
        7 => {
            let mass = cursor.f32()?;
            let density = cursor.f32()?;
            let radius = cursor.f32()?;
            let ends = [cursor.float3()?, cursor.float3()?];
            let mut position = [0.0f32; 3];
            let mut rotation = [0.0, 0.0, 0.0, 1.0f32];
            if cursor.remaining() >= 32 {
                let mut peek = *cursor;
                let trailer_position = peek.float3()?;
                let trailer_rotation = [peek.f32()?, peek.f32()?, peek.f32()?, peek.f32()?];
                let count = peek.u32()?;
                let norm: f32 = trailer_rotation.iter().map(|axis| axis * axis).sum();
                if (1..=16).contains(&count) && (0.98..=1.02).contains(&norm) {
                    let flipped = choices.flips >> choices.sites.min(63) & 1 == 1;
                    choices.sites += 1;
                    if !flipped {
                        position = trailer_position;
                        rotation = trailer_rotation;
                        peek.take(count as usize * 12)?;
                        *cursor = peek;
                    }
                }
            }
            Ok(PhysicsShape {
                position,
                rotation,
                kind: PhysicsShapeKind::Capsule {
                    mass,
                    density,
                    radius,
                    ends,
                },
            })
        }
        other => bail!(
            "{label}: nested physics shape {index} starts with marker {other}, \
             neither 5 (transformed shape) nor 7 (capsule)"
        ),
    }
}

/// The streaming-assets table: every sound, texture and material the world
/// references, as `folder\file` pairs of offsets into one string blob.
///
/// The 010 template's eleven-word header does not exist in F.E.A.R. retail
/// worlds. What is actually on disk, recovered from `Office.World00p`:
///
/// ```text
/// u32 0, i32 -1, u32 0          three words, constant in observed worlds
/// string blob                   NUL-terminated, 4-byte-aligned, *unsized*
/// u32 pair_count
/// { u32 folder, u32 file } * pair_count
/// u64 0                         section terminator
/// ```
///
/// Nothing states the blob's length forwards, so it is solved backwards from
/// the section end: the terminator is fixed, the pairs are eight bytes each,
/// and the count must equal the one stored before them. The solution is
/// checked for uniqueness and every offset must land on a terminated ASCII
/// string, so a coincidental count cannot slip through.
fn parse_streaming_assets(cursor: &mut Cursor, label: &str) -> Result<Vec<String>> {
    // Two header variants exist in retail: `(0, -1, 0)` with an unsized blob
    // (Office), and `(blob_length, 0, 0)` (DeadWood). When the length is
    // declared it is cross-checked against the solved layout below.
    let word = cursor.u32()? as usize;
    let sentinel = cursor.i32()?;
    let word2 = cursor.u32()?;
    let declared_blob = match (word, sentinel, word2) {
        (0, -1, 0) => None,
        (length, 0, 0) if length > 0 => {
            // The sized variant carries the 010 template's full eleven-word
            // header -- the template was evidently written from such a world
            // -- so eight more words follow: a string count and padding.
            cursor.take(32)?;
            Some(length)
        }
        other => bail!(
            "{label}: streaming-asset header is {other:?}, neither (0, -1, 0) \
             nor a declared blob length"
        ),
    };
    let region = cursor.take(cursor.remaining())?;
    let terminator = region
        .len()
        .checked_sub(8)
        .with_context(|| format!("{label}: streaming-asset region too small"))?;
    if region[terminator..] != [0u8; 8] {
        bail!(
            "{label}: world-model section does not end in eight zero bytes; \
             a physics shape was misread"
        );
    }
    let body = &region[..terminator];
    // A shorter table is a suffix of a longer candidate, so a coincidental
    // small count can also fit; the true table is the largest one that does.
    let mut solution = None;
    for count in (0..=(body.len().saturating_sub(4)) / 8).rev() {
        let count_at = body.len() - 4 - 8 * count;
        if count_at % 4 != 0 {
            continue;
        }
        let stored = u32::from_le_bytes(body[count_at..count_at + 4].try_into().unwrap()) as usize;
        if stored != count {
            continue;
        }
        let blob = &body[..count_at];
        let pairs = &body[count_at + 4..];
        let all_resolve = pairs.chunks_exact(8).all(|pair| {
            let folder = u32::from_le_bytes(pair[..4].try_into().unwrap()) as usize;
            let file = u32::from_le_bytes(pair[4..].try_into().unwrap()) as usize;
            string_at(blob, folder, label).is_ok() && string_at(blob, file, label).is_ok()
        });
        if all_resolve {
            solution = Some((blob, pairs));
            break;
        }
    }
    let (blob, pairs) =
        solution.with_context(|| format!("{label}: no streaming-asset count fits the section"))?;
    if let Some(declared) = declared_blob
        && declared != blob.len()
    {
        bail!(
            "{label}: streaming-asset header declares a {declared}-byte string \
             blob, but the table layout gives {}",
            blob.len()
        );
    }
    let mut assets = Vec::with_capacity(pairs.len() / 8);
    for pair in pairs.chunks_exact(8) {
        let folder = u32::from_le_bytes(pair[..4].try_into().unwrap()) as usize;
        let file = u32::from_le_bytes(pair[4..].try_into().unwrap()) as usize;
        let mut path = string_at(blob, folder, label)?;
        let name = string_at(blob, file, label)?;
        if !path.is_empty() {
            path.push('\\');
        }
        path.push_str(&name);
        assets.push(path);
    }
    Ok(assets)
}

fn string_at(data: &[u8], offset: usize, label: &str) -> Result<String> {
    let tail = data.get(offset..).with_context(|| {
        format!(
            "{label}: string offset {offset} outside a {}-byte table",
            data.len()
        )
    })?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .with_context(|| format!("{label}: unterminated string at offset {offset}"))?;
    Ok(std::str::from_utf8(&tail[..end])
        .with_context(|| format!("{label}: non-ASCII string at offset {offset}"))?
        .to_owned())
}

// ---------------------------------------------------------------------------
// render

/// Where one vertex attribute lives inside a vertex layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexAttribute {
    pub offset: u16,
    /// 1 = float2, 2 = float3, 3 = float4, 4 = u32 colour, 5 = skeletal index.
    pub kind: u8,
    /// 0 position, 3 normal, 5 texcoord, 6 tangent, 7 binormal, 10 colour.
    pub usage: u8,
    /// Distinguishes repeated usages, e.g. a second texcoord set.
    pub usage_index: u8,
}

/// One vertex layout the surfaces reference by index.
#[derive(Clone, Debug, Default)]
pub struct VertexLayout {
    pub attributes: Vec<VertexAttribute>,
}

/// One draw batch: a vertex range, an index range, a material.
#[derive(Clone, Copy, Debug)]
pub struct RenderSurface {
    pub vertices_start: u32,
    pub vertices_count: u32,
    pub vertex_size: u32,
    /// First index in the shared pool, in *indices*.
    pub indices_start: u32,
    pub indices_offset: u32,
    /// The 010 template calls this `indices_count`, but consecutive surfaces
    /// advance `indices_start` by three times it: it counts triangles.
    pub triangle_count: u32,
    /// Index into [`RenderMesh::materials`].
    pub material: u32,
    /// Index into [`RenderMesh::layouts`].
    pub layout: u32,
}

#[derive(Clone, Debug)]
pub struct RenderMesh {
    /// Raw interleaved vertex data; surfaces slice it by their own stride.
    pub vertex_block: Vec<u8>,
    /// One shared 16-bit index pool.
    pub indices: Vec<u16>,
    pub layouts: Vec<VertexLayout>,
    pub surfaces: Vec<RenderSurface>,
    /// Material paths relative to the archive root, `.Mat00` files.
    pub materials: Vec<String>,
}

/// One surface's vertices with the attributes the port binds.
///
/// Attribute vectors are either empty or one entry per vertex; a layout
/// without normals produces an empty `normals`, never a padded one.
#[derive(Clone, Debug, Default)]
pub struct DecodedSurface {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// The first texture-coordinate set.
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 3]>,
    pub binormals: Vec<[f32; 3]>,
    /// Triangle list, rebased to this surface's own vertex range.
    pub indices: Vec<u32>,
}

impl RenderMesh {
    /// Decode one surface's vertex range through its declared layout.
    pub fn decode_surface(&self, index: usize, label: &str) -> Result<DecodedSurface> {
        let surface = self
            .surfaces
            .get(index)
            .with_context(|| format!("{label}: no surface {index}"))?;
        let layout = &self.layouts[surface.layout as usize];
        let stride = surface.vertex_size as usize;
        let start = surface.vertices_start as usize * stride;
        let count = surface.vertices_count as usize;
        let block = self
            .vertex_block
            .get(start..start + count * stride)
            .with_context(|| format!("{label}: surface {index} vertex range out of block"))?;

        let mut decoded = DecodedSurface::default();
        for attribute in &layout.attributes {
            let offset = attribute.offset as usize;
            let read3 = |vertex: usize| -> Result<[f32; 3]> {
                floats_at::<3>(block, vertex * stride + offset)
            };
            match (attribute.usage, attribute.kind, attribute.usage_index) {
                (0, 2, 0) => {
                    for vertex in 0..count {
                        decoded.positions.push(read3(vertex)?);
                    }
                }
                (3, 2, 0) => {
                    for vertex in 0..count {
                        decoded.normals.push(read3(vertex)?);
                    }
                }
                (5, 1, 0) => {
                    for vertex in 0..count {
                        decoded
                            .uvs
                            .push(floats_at::<2>(block, vertex * stride + offset)?);
                    }
                }
                (6, 2, 0) => {
                    for vertex in 0..count {
                        decoded.tangents.push(read3(vertex)?);
                    }
                }
                (7, 2, 0) => {
                    for vertex in 0..count {
                        decoded.binormals.push(read3(vertex)?);
                    }
                }
                // Secondary UV sets, colours and skeletal weights have no
                // binding in the port; leaving them behind is deliberate.
                _ => {}
            }
        }
        // SpillKill declares one layout with no position attribute at all:
        // normal at 12, texcoord at 24, tangent, binormal -- every entry
        // leaves bytes 0..12 untouched. The position is implicit at offset 0,
        // which is only accepted when the declared attributes actually leave
        // room for it.
        if decoded.positions.is_empty()
            && stride >= 12
            && layout
                .attributes
                .iter()
                .all(|attribute| attribute.offset >= 12)
        {
            for vertex in 0..count {
                decoded
                    .positions
                    .push(floats_at::<3>(block, vertex * stride)?);
            }
        }
        if decoded.positions.len() != count {
            bail!(
                "{label}: surface {index} layout {} carries no float3 position; \
                 its attributes are {:?}",
                surface.layout,
                layout.attributes
            );
        }

        let base = i64::from(surface.vertices_start) - i64::from(surface.indices_offset);
        let range = surface.indices_start as usize
            ..(surface.indices_start + surface.triangle_count * 3) as usize;
        for &stored in &self.indices[range] {
            let local = i64::from(stored) - base;
            if !(0..count as i64).contains(&local) {
                bail!(
                    "{label}: surface {index} index {stored} rebases to {local} \
                     of {count} (vertices_start {}, indices_offset {})",
                    surface.vertices_start,
                    surface.indices_offset
                );
            }
            decoded.indices.push(local as u32);
        }
        Ok(decoded)
    }
}

pub fn parse_render(bytes: &[u8], header: &WorldHeader, label: &str) -> Result<RenderMesh> {
    let section = bytes
        .get(header.render_offset as usize..header.sector_offset as usize)
        .with_context(|| format!("{label}: render section out of range"))?;
    let mut cursor = Cursor::new(section, label);
    let _branch_count = cursor.u32()?;
    let _node_count = cursor.u32()?;
    let _unknown = cursor.u32()?;
    let mesh_count = cursor.u32()?;
    if mesh_count != 1 {
        bail!("{label}: {mesh_count} render meshes, every reviewed world has exactly 1");
    }
    let material_count = cursor.u32()? as usize;
    let _mesh_count_again = cursor.u32()?;
    cursor.take(16)?;

    let mesh_kind = cursor.u32()?;
    if mesh_kind != 0 {
        bail!("{label}: render mesh kind {mesh_kind}, not the world mesh");
    }
    let _surface_count_dupe = cursor.u32()?;
    let _material_count_dupe = cursor.u32()?;
    let vertex_block_size = cursor.u32()? as usize;
    let index_block_size = cursor.u32()? as usize;
    let vertex_block = cursor.take(vertex_block_size)?.to_vec();
    if index_block_size % 2 != 0 {
        bail!("{label}: index block of {index_block_size} bytes is not 16-bit indices");
    }
    let raw_indices = cursor.take(index_block_size)?;
    let indices: Vec<u16> = raw_indices
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();

    let layout_count = cursor.u32()? as usize;
    let mut layouts = Vec::with_capacity(layout_count);
    for index in 0..layout_count {
        let size = cursor.u32()? as usize;
        if size % 8 != 0 {
            bail!("{label}: vertex layout {index} declares {size} bytes, not a whole entry count");
        }
        let mut attributes = Vec::new();
        for _ in 0..size / 8 {
            let stream = cursor.u16()?;
            let offset = cursor.u16()?;
            let kind = cursor.u8()?;
            let _method = cursor.u8()?;
            let usage = cursor.u8()?;
            let usage_index = cursor.u8()?;
            if stream == 0xFF || kind == 17 {
                continue; // D3DDECL_END
            }
            if stream != 0 {
                bail!("{label}: vertex layout {index} uses stream {stream}");
            }
            attributes.push(VertexAttribute {
                offset,
                kind,
                usage,
                usage_index,
            });
        }
        layouts.push(VertexLayout { attributes });
    }

    let surface_count = cursor.u32()? as usize;
    let mut surfaces = Vec::with_capacity(surface_count);
    for index in 0..surface_count {
        let surface = RenderSurface {
            vertices_start: cursor.u32()?,
            vertices_count: cursor.u32()?,
            vertex_size: cursor.u32()?,
            indices_start: cursor.u32()?,
            indices_offset: cursor.u32()?,
            triangle_count: cursor.u32()?,
            material: cursor.u32()?,
            layout: {
                let _unknown = cursor.i32()?;
                cursor.u32()?
            },
        };
        if surface.material as usize >= material_count {
            bail!(
                "{label}: surface {index} names material {} of {material_count}",
                surface.material
            );
        }
        if surface.layout as usize >= layout_count {
            bail!(
                "{label}: surface {index} names vertex layout {} of {layout_count}",
                surface.layout
            );
        }
        let vertex_end = (surface.vertices_start as usize + surface.vertices_count as usize)
            * surface.vertex_size as usize;
        if vertex_end > vertex_block.len() {
            bail!(
                "{label}: surface {index} runs to vertex byte {vertex_end} of {}",
                vertex_block.len()
            );
        }
        let index_end = surface.indices_start as usize + surface.triangle_count as usize * 3;
        if index_end > indices.len() {
            bail!(
                "{label}: surface {index} runs to index {index_end} of {}",
                indices.len()
            );
        }
        surfaces.push(surface);
    }

    let mut materials = Vec::with_capacity(material_count);
    for _ in 0..material_count {
        let length = cursor.u16()? as usize;
        let bytes = cursor.take(length)?;
        materials.push(
            std::str::from_utf8(bytes)
                .context("non-ASCII material name")?
                .to_owned(),
        );
    }
    // The render tree follows; the port has no use for its culling hierarchy.
    Ok(RenderMesh {
        vertex_block,
        indices,
        layouts,
        surfaces,
        materials,
    })
}

// ---------------------------------------------------------------------------
// objects

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    String(String),
    Vector([f32; 3]),
    Color([f32; 3]),
    Float(f32),
    Integer(i32),
    Flags(u32),
    Quaternion([f32; 4]),
    Command(String),
    Text(String),
}

/// One placed entity: a class and its authored properties.
#[derive(Clone, Debug)]
pub struct WorldObject {
    pub class: String,
    pub properties: Vec<(String, PropertyValue)>,
}

impl WorldObject {
    fn value(&self, name: &str) -> Option<&PropertyValue> {
        self.properties
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// The instance name, distinct from the class.
    pub fn name(&self) -> Option<&str> {
        match self.value("Name")? {
            PropertyValue::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn position(&self) -> Option<[f32; 3]> {
        match self.value("Pos")? {
            PropertyValue::Vector(vector) => Some(*vector),
            _ => None,
        }
    }

    pub fn rotation(&self) -> Option<[f32; 4]> {
        match self.value("Rotation")? {
            PropertyValue::Quaternion(quaternion) => Some(*quaternion),
            _ => None,
        }
    }

    pub fn string(&self, name: &str) -> Option<&str> {
        match self.value(name)? {
            PropertyValue::String(text)
            | PropertyValue::Command(text)
            | PropertyValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn float(&self, name: &str) -> Option<f32> {
        match self.value(name)? {
            PropertyValue::Float(value) => Some(*value),
            _ => None,
        }
    }

    pub fn vector(&self, name: &str) -> Option<[f32; 3]> {
        match self.value(name)? {
            PropertyValue::Vector(vector) | PropertyValue::Color(vector) => Some(*vector),
            _ => None,
        }
    }
}

pub fn parse_objects(bytes: &[u8], header: &WorldHeader, label: &str) -> Result<Vec<WorldObject>> {
    let section = bytes
        .get(header.object_offset as usize..header.blind_data_offset as usize)
        .with_context(|| format!("{label}: object section out of range"))?;
    let mut cursor = Cursor::new(section, label);
    let count = cursor.u32()? as usize;
    let mut objects = Vec::with_capacity(count);
    for index in 0..count {
        let class_length = cursor.u16()? as usize;
        let class = std::str::from_utf8(cursor.take(class_length)?)
            .with_context(|| format!("{label}: object {index} has a non-ASCII class"))?
            .to_owned();
        let property_count = cursor.u32()? as usize;
        let blob_size = cursor.u32()? as usize;
        let blob = cursor.take(blob_size)?.to_vec();
        let mut properties = Vec::with_capacity(property_count);
        for slot in 0..property_count {
            let name_offset = cursor.u32()? as usize;
            let kind = cursor.u32()?;
            let name = string_at(&blob, name_offset, label).with_context(|| {
                format!("{label}: object {index} ({class}) property {slot} name")
            })?;
            let context = || format!("{label}: object {index} ({class}) property {name}");
            // Floats, integers and flags carry their value in the record
            // itself -- a WeaponItem's AmmoAmount of -1 cannot be a blob
            // offset, whatever the 010 template says. Everything wider lives
            // in the blob at the offset the record carries.
            let value = match kind {
                3 => PropertyValue::Float(cursor.f32()?),
                4 => PropertyValue::Integer(cursor.i32()?),
                5 => PropertyValue::Flags(cursor.u32()?),
                _ => {
                    let offset = cursor.u32()? as usize;
                    match kind {
                        0 => PropertyValue::String(
                            string_at(&blob, offset, label).with_context(context)?,
                        ),
                        1 => PropertyValue::Vector(
                            floats_at::<3>(&blob, offset).with_context(context)?,
                        ),
                        2 => PropertyValue::Color(
                            floats_at::<3>(&blob, offset).with_context(context)?,
                        ),
                        6 => PropertyValue::Quaternion(
                            floats_at::<4>(&blob, offset).with_context(context)?,
                        ),
                        7 => PropertyValue::Command(
                            string_at(&blob, offset, label).with_context(context)?,
                        ),
                        8 => PropertyValue::Text(
                            string_at(&blob, offset, label).with_context(context)?,
                        ),
                        other => bail!("{}: unknown property type {other}", context()),
                    }
                }
            };
            properties.push((name, value));
        }
        objects.push(WorldObject { class, properties });
    }
    Ok(objects)
}

fn bytes_at<const N: usize>(blob: &[u8], offset: usize) -> Result<[u8; N]> {
    Ok(blob
        .get(offset..offset + N)
        .with_context(|| {
            format!(
                "value at offset {offset} runs past the {}-byte blob",
                blob.len()
            )
        })?
        .try_into()
        .unwrap())
}

fn floats_at<const N: usize>(blob: &[u8], offset: usize) -> Result<[f32; N]> {
    let mut floats = [0f32; N];
    for (index, slot) in floats.iter_mut().enumerate() {
        *slot = f32::from_le_bytes(bytes_at::<4>(blob, offset + index * 4)?);
    }
    Ok(floats)
}

// ---------------------------------------------------------------------------
// whole file

pub struct World {
    pub header: WorldHeader,
    pub world_models: WorldModels,
    pub render: RenderMesh,
    pub objects: Vec<WorldObject>,
}

pub fn parse(bytes: &[u8], label: &str) -> Result<World> {
    let header = parse_header(bytes, label)?;
    Ok(World {
        world_models: parse_world_models(bytes, &header, label)?,
        render: parse_render(bytes, &header, label)?,
        objects: parse_objects(bytes, &header, label)?,
        header,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_matches_the_template() {
        assert_eq!(MAGIC_XOR, 399);
    }

    /// The object property encoding, reproduced from the `WorldProperties`
    /// record at the head of `Office.World00p`'s object section.
    #[test]
    fn decodes_typed_object_properties() {
        let mut blob = Vec::new();
        let name_offset = blob.len() as u32;
        blob.extend_from_slice(b"Name\0");
        while blob.len() % 4 != 0 {
            blob.push(0);
        }
        let value_offset = blob.len() as u32;
        blob.extend_from_slice(b"WorldProperties00\0");
        while blob.len() % 4 != 0 {
            blob.push(0);
        }
        let pos_offset = blob.len() as u32;
        blob.extend_from_slice(b"Pos\0");
        let pos_value = blob.len() as u32;
        for value in [6038.0f32, -1124.0, -512.0] {
            blob.extend_from_slice(&value.to_le_bytes());
        }

        let mut section = Vec::new();
        section.extend_from_slice(&1u32.to_le_bytes());
        section.extend_from_slice(&15u16.to_le_bytes());
        section.extend_from_slice(b"WorldProperties");
        section.extend_from_slice(&3u32.to_le_bytes());
        section.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        section.extend_from_slice(&blob);
        for (name, kind, value) in [
            (name_offset, 0u32, value_offset),
            (pos_offset, 1, pos_value),
            (name_offset, 3, 2.5f32.to_bits()),
        ] {
            section.extend_from_slice(&name.to_le_bytes());
            section.extend_from_slice(&kind.to_le_bytes());
            section.extend_from_slice(&value.to_le_bytes());
        }

        let mut bytes = vec![0u8; 56];
        let object_offset = bytes.len() as u32;
        bytes.extend_from_slice(&section);
        let header = WorldHeader {
            render_offset: 0,
            sector_offset: 0,
            object_offset,
            blind_data_offset: bytes.len() as u32,
            min: [0.0; 3],
            max: [0.0; 3],
            world_offset: [0.0; 3],
        };
        let objects = parse_objects(&bytes, &header, "test").unwrap();
        assert_eq!(objects.len(), 1);
        let object = &objects[0];
        assert_eq!(object.class, "WorldProperties");
        assert_eq!(object.name(), Some("WorldProperties00"));
        assert_eq!(object.position(), Some([6038.0, -1124.0, -512.0]));
        assert_eq!(object.properties[2].1, PropertyValue::Float(2.5));
    }

    #[test]
    fn header_rejects_disordered_sections() {
        let mut bytes = vec![0u8; 60];
        bytes[..4].copy_from_slice(&VERSION.to_le_bytes());
        bytes[4..8].copy_from_slice(&40u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&30u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&50u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&55u32.to_le_bytes());
        let error = parse_header(&bytes, "test").unwrap_err().to_string();
        assert!(error.contains("not ascending"), "{error}");
    }
}
