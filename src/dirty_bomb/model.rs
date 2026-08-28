//! Cooked UE3 `UModel` (BSP) decoder used by Dirty Bomb map packages.

use anyhow::{Context, Result, bail};

use super::package::{ExportEntry, Package};

const MAX_ARRAY_ITEMS: usize = 100_000_000;

pub const PF_INVISIBLE: u32 = 0x0000_0001;
pub const PF_NOT_SOLID: u32 = 0x0000_0008;
pub const PF_TWO_SIDED: u32 = 0x0000_0100;
pub const PF_PORTAL: u32 = 0x0400_0000;

#[derive(Clone, Copy, Debug)]
pub struct BspNode {
    pub first_vertex: usize,
    pub surface: usize,
    pub vertex_buffer_start: usize,
    pub vertices: usize,
    pub flags: u8,
}

#[derive(Clone, Debug)]
pub struct BspSurface {
    pub material: Option<String>,
    pub poly_flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Model {
    pub bounds_origin: [f32; 3],
    pub bounds_extent: [f32; 3],
    pub nodes: Vec<BspNode>,
    pub surfaces: Vec<BspSurface>,
    pub vertices: Vec<ModelVertex>,
}

pub fn parse(package: &Package, export: &ExportEntry) -> Result<Model> {
    if export.class_name != "Model" {
        bail!(
            "{} is {}, not a Model export",
            export.object_name,
            export.class_name
        );
    }
    let tagged = package.tagged_properties(export)?;
    let mut reader = Cursor::new(tagged.native_data, &export.object_name);

    let bounds_origin = reader.float3()?;
    let bounds_extent = reader.float3()?;
    reader.f32()?; // bounding sphere radius

    reader.bulk_array(Some(12), "model vectors")?;
    reader.bulk_array(Some(12), "model points")?;

    let node_bytes = reader.bulk_array(Some(64), "BSP nodes")?;
    let mut nodes = Vec::with_capacity(node_bytes.len() / 64);
    for bytes in node_bytes.chunks_exact(64) {
        let i_vert_pool = i32_at(bytes, 16);
        let i_surf = i32_at(bytes, 20);
        let i_vertex_index = i32_at(bytes, 24);
        let num_vertices = usize::from(bytes[54]);
        if i_vert_pool < 0 || i_surf < 0 || i_vertex_index < 0 {
            bail!(
                "{}: BSP node has negative geometry index ({i_vert_pool}, {i_surf}, {i_vertex_index})",
                export.object_name
            );
        }
        nodes.push(BspNode {
            first_vertex: usize::try_from(i_vert_pool)?,
            surface: usize::try_from(i_surf)?,
            vertex_buffer_start: usize::try_from(i_vertex_index)?,
            vertices: num_vertices,
            flags: bytes[55],
        });
    }

    // Surfs is a TTransArray. Unlike the three BulkSerialize calls above,
    // its regular archive operator persists the transaction owner first.
    reader.i32()?; // TTransArray owner (normally this UModel)
    let surface_count = reader.count("BSP surface count")?;
    let mut surfaces = Vec::with_capacity(surface_count);
    for _ in 0..surface_count {
        let material_index = reader.i32()?;
        let material = (material_index != 0)
            .then(|| package.object_path(material_index))
            .transpose()?;
        let poly_flags = reader.u32()?;
        reader.take(20)?; // pBase, vNormal, vTextureU, vTextureV, iBrushPoly
        reader.i32()?; // owning brush actor
        reader.take(16)?; // plane
        reader.f32()?; // shadow map scale
        reader.u32()?; // lighting channels
        reader.i32()?; // Lightmass settings index
        surfaces.push(BspSurface {
            material,
            poly_flags,
        });
    }

    // FVert is only needed by editor rebuild and collision tree code. Cooked
    // rendering addresses the already-built FModelVertex buffer via each
    // FBspNode's iVertexIndex.
    reader.bulk_array(Some(24), "BSP vertex pool")?;
    reader.i32()?; // NumSharedSides
    let zone_count = reader.count("BSP zone count")?;
    // FZoneProperties: actor object ref, connectivity/visibility FZoneSets,
    // and LastRenderTime. Dirty Bomb's 64-zone UE3 branch uses two u64 sets.
    reader.take(
        zone_count
            .checked_mul(24)
            .context("BSP zone table size overflow")?,
    )?;
    reader.i32()?; // Polys object ref
    reader.bulk_array(Some(4), "leaf hulls")?;
    reader.bulk_array(Some(4), "leaves")?;
    reader.bool32()?; // RootOutside
    reader.bool32()?; // Linked
    reader.bulk_array(Some(4), "portal nodes")?;
    reader.u32()?; // NumUniqueVertices

    let vertex_bytes = reader.bulk_array(Some(36), "BSP render vertices")?;
    let mut vertices = Vec::with_capacity(vertex_bytes.len() / 36);
    for bytes in vertex_bytes.chunks_exact(36) {
        let tangent = packed_normal(&bytes[12..16]);
        let normal = packed_normal(&bytes[16..20]);
        let sign = if bytes[19] < 128 { -1.0 } else { 1.0 };
        let cross = [
            normal[1] * tangent[2] - normal[2] * tangent[1],
            normal[2] * tangent[0] - normal[0] * tangent[2],
            normal[0] * tangent[1] - normal[1] * tangent[0],
        ];
        vertices.push(ModelVertex {
            position: float3_at(bytes, 0),
            normal,
            tangent,
            binormal: cross.map(|component| component * sign),
            uv: [f32_at(bytes, 20), f32_at(bytes, 24)],
        });
    }

    // The remaining fields are lighting-only: LightingGuid and the serialized
    // LightmassSettings array. They are deliberately bounded and retained as
    // a known tail instead of being mistaken for geometry.
    if reader.remaining() < 20 {
        bail!(
            "{}: UModel lighting tail is only {} bytes",
            export.object_name,
            reader.remaining()
        );
    }
    reader.take(16)?;
    let lightmass_count = reader.count("Lightmass settings count")?;
    if lightmass_count == 0 {
        if !reader.is_empty() {
            bail!(
                "{}: {} unexpected bytes after empty UModel LightmassSettings",
                export.object_name,
                reader.remaining()
            );
        }
    } else if reader.remaining() % lightmass_count != 0 {
        bail!(
            "{}: {} Lightmass settings leave a non-integral {}-byte tail",
            export.object_name,
            lightmass_count,
            reader.remaining()
        );
    }

    for (index, node) in nodes.iter().enumerate() {
        if node.surface >= surfaces.len() {
            bail!(
                "{}: BSP node {index} surface {} exceeds {} surfaces",
                export.object_name,
                node.surface,
                surfaces.len()
            );
        }
        let end = node
            .vertex_buffer_start
            .checked_add(node.vertices)
            .context("BSP node vertex range overflow")?;
        if end > vertices.len() {
            bail!(
                "{}: BSP node {index} vertex range {}..{end} exceeds {} render vertices",
                export.object_name,
                node.vertex_buffer_start,
                vertices.len()
            );
        }
    }

    Ok(Model {
        bounds_origin,
        bounds_extent,
        nodes,
        surfaces,
        vertices,
    })
}

fn packed_normal(bytes: &[u8]) -> [f32; 3] {
    [
        f32::from(bytes[0]) / 127.5 - 1.0,
        f32::from(bytes[1]) / 127.5 - 1.0,
        f32::from(bytes[2]) / 127.5 - 1.0,
    ]
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn float3_at(bytes: &[u8], offset: usize) -> [f32; 3] {
    [
        f32_at(bytes, offset),
        f32_at(bytes, offset + 4),
        f32_at(bytes, offset + 8),
    ]
}

#[derive(Clone, Copy, Debug)]
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    label: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], label: &'a str) -> Self {
        Self {
            bytes,
            offset: 0,
            label,
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("UModel cursor overflow")?;
        let bytes = self.bytes.get(self.offset..end).with_context(|| {
            format!(
                "{}: wanted {count} UModel bytes at {:#x}, only {} remain",
                self.label,
                self.offset,
                self.remaining()
            )
        })?;
        self.offset = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn float3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    fn bool32(&mut self) -> Result<bool> {
        match self.i32()? {
            0 => Ok(false),
            1 => Ok(true),
            value => bail!("{}: invalid serialized UBOOL {value}", self.label),
        }
    }

    fn count(&mut self, field: &str) -> Result<usize> {
        let value = self.i32()?;
        if value < 0 {
            bail!("{}: {field} is negative ({value})", self.label);
        }
        let count = usize::try_from(value)?;
        if count > MAX_ARRAY_ITEMS {
            bail!("{}: implausible {field} {count}", self.label);
        }
        Ok(count)
    }

    fn bulk_array(
        &mut self,
        expected_element_size: Option<usize>,
        field: &str,
    ) -> Result<&'a [u8]> {
        let element_size = self.count(&format!("{field} element size"))?;
        let count = self.count(&format!("{field} count"))?;
        if let Some(expected) = expected_element_size
            && element_size != expected
        {
            bail!(
                "{}: {field} element size {element_size}, expected {expected}",
                self.label
            );
        }
        self.take(
            element_size
                .checked_mul(count)
                .context("UModel bulk array size overflow")?,
        )
    }
}
