//! `UStaticMesh` native serializer for Dirty Bomb's cooked PC packages.
//!
//! The layout follows `UStaticMesh::Serialize` and
//! `FStaticMeshRenderData::Serialize` from the available UE3 source. Arrays
//! retain their serialized element-size sentinels and every sentinel is
//! checked before a vertex or index is accepted.

use anyhow::{Context, Result, bail};

use super::package::{ExportEntry, Package};

const MAX_ARRAY_ITEMS: usize = 100_000_000;
const BULKDATA_STORE_IN_SEPARATE_FILE: u32 = 1 << 0;

#[derive(Clone, Debug)]
pub struct StaticMeshElement {
    pub material: Option<String>,
    pub enable_collision: bool,
    pub enable_shadow_casting: bool,
    pub first_index: usize,
    pub triangles: usize,
    pub min_vertex: usize,
    pub max_vertex: usize,
    pub material_index: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct StaticMeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct StaticMeshLod {
    pub elements: Vec<StaticMeshElement>,
    pub vertices: Vec<StaticMeshVertex>,
    /// Raw cooked `FColor` bytes for meshes authored with vertex painting.
    /// UE3 stores no buffer at all for the common unpainted case.
    pub vertex_colors: Option<Vec<[u8; 4]>>,
    pub indices: Vec<u16>,
    pub texture_coordinates: usize,
    pub full_precision_uvs: bool,
}

#[derive(Clone, Debug)]
pub struct StaticMesh {
    pub bounds_origin: [f32; 3],
    pub bounds_extent: [f32; 3],
    pub bounds_radius: f32,
    pub body_setup: Option<String>,
    pub internal_version: i32,
    pub lods: Vec<StaticMeshLod>,
}

pub fn parse(package: &Package, export: &ExportEntry) -> Result<StaticMesh> {
    if export.class_name != "StaticMesh" {
        bail!(
            "{} is {}, not a StaticMesh export",
            export.object_name,
            export.class_name
        );
    }
    let tagged = package.tagged_properties(export)?;
    let mut reader = Cursor::new(
        tagged.native_data,
        tagged.native_offset,
        &export.object_name,
    );
    let bounds_origin = reader.float3()?;
    let bounds_extent = reader.float3()?;
    let bounds_radius = reader.f32()?;
    let body_setup_index = reader.i32()?;
    let body_setup = (body_setup_index != 0)
        .then(|| package.object_path(body_setup_index))
        .transpose()?;

    // TkDOPTreeCompact: FkDOP RootBound (six floats), followed by bulk
    // arrays of compact nodes and WORD collision triangles.
    reader.take(24)?;
    reader.skip_bulk_array(None, None, "kDOP nodes")?;
    reader.skip_bulk_array(None, None, "kDOP triangles")?;

    let internal_version = reader.i32()?;
    if internal_version != 18 {
        bail!(
            "{}: unsupported internal static-mesh version {internal_version}",
            export.object_name
        );
    }

    // FStaticMeshSourceData is editor source. Cooked packages normally clear
    // it, but accepting and structurally decoding it costs little.
    let has_source_data = reader.bool32()?;
    if has_source_data {
        parse_render_data(package, &mut reader)?;
    }

    let optimization_settings = reader.count("optimization settings")?;
    // v868 is after VER_ADDED_EXTRA_MESH_OPTIMIZATION_SETTINGS (863).
    reader.take(
        optimization_settings
            .checked_mul(24)
            .context("optimization settings size overflow")?,
    )?;
    reader.bool32()?; // bHasBeenSimplified
    // Dirty Bomb's licensee branch predates the generic
    // VER_TAG_MESH_PROXIES field despite carrying a numerically newer package
    // version. Its cooked stream proceeds directly to LODModels here.

    let lod_count = reader.count("static mesh LOD count")?;
    if lod_count == 0 {
        bail!("{} has no render LODs", export.object_name);
    }
    let mut lods = Vec::with_capacity(lod_count);
    for _ in 0..lod_count {
        lods.push(parse_render_data(package, &mut reader)?);
    }

    // FStaticMeshLODInfo serializes no fields in a persistent archive, but
    // its containing TArray still writes the element count.
    reader.count("LOD info count")?;
    reader.take(12)?; // ThumbnailAngle FRotator
    reader.f32()?; // ThumbnailDistance
    reader.fstring()?; // HighResSourceMeshName
    reader.u32()?; // HighResSourceMeshCRC
    reader.take(16)?; // LightingGuid
    reader.i32()?; // VertexPositionVersionNumber
    let streaming_factors = reader.count("streaming texture factors")?;
    reader.take(
        streaming_factors
            .checked_mul(4)
            .context("streaming texture factors size overflow")?,
    )?;
    reader.bool32()?; // bRemoveDegenerates
    // This branch predates the generic per-LOD instancing-lighting and console
    // preallocation tail fields despite its higher licensee package number.
    if !reader.is_empty() {
        bail!(
            "{}: {} unconsumed bytes after UStaticMesh::Serialize",
            export.object_name,
            reader.remaining()
        );
    }
    Ok(StaticMesh {
        bounds_origin,
        bounds_extent,
        bounds_radius,
        body_setup,
        internal_version,
        lods,
    })
}

fn parse_render_data(package: &Package, reader: &mut Cursor<'_>) -> Result<StaticMeshLod> {
    reader.skip_untyped_bulk_data("raw triangles")?;

    let element_count = reader.count("static mesh element count")?;
    let mut elements = Vec::with_capacity(element_count);
    for _ in 0..element_count {
        let material_index = reader.i32()?;
        let material = (material_index != 0)
            .then(|| package.object_path(material_index))
            .transpose()?;
        let enable_collision = reader.bool32()?;
        reader.bool32()?; // OldEnableCollision
        let enable_shadow_casting = reader.bool32()?;
        let first_index = reader.usize32("first index")?;
        let triangles = reader.usize32("triangle count")?;
        let min_vertex = reader.usize32("minimum vertex index")?;
        let max_vertex = reader.usize32("maximum vertex index")?;
        let material_slot = reader.i32()?;
        let fragment_count = reader.count("fragment range count")?;
        reader.take(
            fragment_count
                .checked_mul(8)
                .context("fragment range size overflow")?,
        )?;
        let has_platform_data = reader.u8()?;
        if has_platform_data != 0 {
            bail!("PC static mesh unexpectedly carries PS3 platform data");
        }
        elements.push(StaticMeshElement {
            material,
            enable_collision,
            enable_shadow_casting,
            first_index,
            triangles,
            min_vertex,
            max_vertex,
            material_index: material_slot,
        });
    }

    let position_stride = reader.usize32("position stride")?;
    let position_count = reader.usize32("position count")?;
    if position_stride != 12 {
        bail!("unsupported position vertex stride {position_stride}");
    }
    let position_bytes =
        reader.bulk_array(Some(position_stride), Some(position_count), "positions")?;
    let mut positions = Vec::with_capacity(position_count);
    for vertex in position_bytes.chunks_exact(position_stride) {
        positions.push([
            f32::from_le_bytes(vertex[0..4].try_into().unwrap()),
            f32::from_le_bytes(vertex[4..8].try_into().unwrap()),
            f32::from_le_bytes(vertex[8..12].try_into().unwrap()),
        ]);
    }

    let texture_coordinates = reader.usize32("texture coordinate count")?;
    let vertex_stride = reader.usize32("static mesh vertex stride")?;
    let vertex_count = reader.usize32("static mesh vertex count")?;
    let full_precision_uvs = reader.bool32()?;
    if !(1..=4).contains(&texture_coordinates) {
        bail!("unsupported texture-coordinate count {texture_coordinates}");
    }
    let expected_stride = 8 + texture_coordinates * if full_precision_uvs { 8 } else { 4 };
    if vertex_stride != expected_stride {
        bail!(
            "static mesh vertex stride {vertex_stride}, expected {expected_stride} for {texture_coordinates} UV sets (full precision: {full_precision_uvs})"
        );
    }
    if vertex_count != position_count {
        bail!("position/attribute vertex counts differ: {position_count}/{vertex_count}");
    }
    let vertex_bytes = reader.bulk_array(
        Some(vertex_stride),
        Some(vertex_count),
        "static mesh vertices",
    )?;
    let mut vertices = Vec::with_capacity(vertex_count);
    for (index, vertex) in vertex_bytes.chunks_exact(vertex_stride).enumerate() {
        let tangent = packed_normal(&vertex[0..4]);
        let normal = packed_normal(&vertex[4..8]);
        let sign = if vertex[7] < 128 { -1.0 } else { 1.0 };
        let cross = [
            normal[1] * tangent[2] - normal[2] * tangent[1],
            normal[2] * tangent[0] - normal[0] * tangent[2],
            normal[0] * tangent[1] - normal[1] * tangent[0],
        ];
        let binormal = cross.map(|component| component * sign);
        let uv = if full_precision_uvs {
            [
                f32::from_le_bytes(vertex[8..12].try_into().unwrap()),
                f32::from_le_bytes(vertex[12..16].try_into().unwrap()),
            ]
        } else {
            [
                half_to_f32(u16::from_le_bytes(vertex[8..10].try_into().unwrap())),
                half_to_f32(u16::from_le_bytes(vertex[10..12].try_into().unwrap())),
            ]
        };
        vertices.push(StaticMeshVertex {
            position: positions[index],
            normal,
            tangent,
            binormal,
            uv,
        });
    }

    let color_stride = reader.usize32("color stride")?;
    let color_count = reader.usize32("color count")?;
    let vertex_colors = if color_count > 0 {
        if color_stride != 4 || color_count != vertex_count {
            bail!(
                "invalid color buffer stride/count {color_stride}/{color_count}, expected 4/{vertex_count}"
            );
        }
        let bytes = reader.bulk_array(Some(4), Some(color_count), "vertex colors")?;
        Some(
            bytes
                .chunks_exact(4)
                .map(|color| color.try_into().expect("a four-byte color chunk"))
                .collect(),
        )
    } else if color_stride != 0 {
        bail!("empty color buffer has nonzero stride {color_stride}");
    } else {
        None
    };

    let declared_vertices = reader.usize32("LOD vertex count")?;
    if declared_vertices != vertex_count {
        bail!("LOD declares {declared_vertices} vertices, buffers contain {vertex_count}");
    }
    let index_bytes = reader.bulk_array(Some(2), None, "render indices")?;
    let indices: Vec<u16> = index_bytes
        .chunks_exact(2)
        .map(|index| u16::from_le_bytes(index.try_into().unwrap()))
        .collect();
    reader.skip_bulk_array(Some(2), None, "wireframe indices")?;
    // Dirty Bomb's licensee branch does not carry the generic UE3 adjacency
    // buffer here; the stream proceeds directly to the next LOD or LODInfo.

    for (section_index, element) in elements.iter().enumerate() {
        let index_end = element
            .first_index
            .checked_add(
                element
                    .triangles
                    .checked_mul(3)
                    .context("section index count overflow")?,
            )
            .context("section index range overflow")?;
        if index_end > indices.len() {
            bail!(
                "section {section_index} index range {}..{index_end} exceeds {} indices",
                element.first_index,
                indices.len()
            );
        }
        if element.min_vertex > element.max_vertex || element.max_vertex >= vertex_count {
            bail!(
                "section {section_index} vertex range {}..{} exceeds {vertex_count} vertices",
                element.min_vertex,
                element.max_vertex
            );
        }
    }
    if let Some(index) = indices
        .iter()
        .find(|&&index| usize::from(index) >= vertex_count)
    {
        bail!("render index {index} exceeds {vertex_count} vertices");
    }
    Ok(StaticMeshLod {
        elements,
        vertices,
        vertex_colors,
        indices,
        texture_coordinates,
        full_precision_uvs,
    })
}

fn packed_normal(bytes: &[u8]) -> [f32; 3] {
    [
        f32::from(bytes[0]) / 127.5 - 1.0,
        f32::from(bytes[1]) / 127.5 - 1.0,
        f32::from(bytes[2]) / 127.5 - 1.0,
    ]
}

fn half_to_f32(value: u16) -> f32 {
    let sign = u32::from(value & 0x8000) << 16;
    let exponent = u32::from((value >> 10) & 0x1f);
    let fraction = u32::from(value & 0x03ff);
    let bits = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let leading = fraction.leading_zeros() - 22;
            let normalized = (fraction << (leading + 1)) & 0x03ff;
            sign | ((127 - 15 - leading) << 23) | (normalized << 13)
        }
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent + (127 - 15)) << 23) | (fraction << 13),
    };
    f32::from_bits(bits)
}

#[derive(Clone, Copy, Debug)]
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    base_offset: usize,
    label: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], base_offset: usize, label: &'a str) -> Self {
        Self {
            bytes,
            offset: 0,
            base_offset,
            label,
        }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn absolute_offset(&self) -> Result<usize> {
        self.base_offset
            .checked_add(self.offset)
            .context("absolute package offset overflow")
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("static-mesh cursor overflow")?;
        let bytes = self.bytes.get(self.offset..end).with_context(|| {
            format!(
                "{}: wanted {count} native bytes at {:#x}, only {} remain",
                self.label,
                self.offset,
                self.remaining()
            )
        })?;
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
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

    fn usize32(&mut self, field: &str) -> Result<usize> {
        let value = self.i32()?;
        if value < 0 {
            bail!("{}: {field} is negative ({value})", self.label);
        }
        usize::try_from(value).context("nonnegative i32 does not fit usize")
    }

    fn count(&mut self, field: &str) -> Result<usize> {
        let count = self.usize32(field)?;
        if count > MAX_ARRAY_ITEMS {
            bail!("{}: implausible {field} {count}", self.label);
        }
        Ok(count)
    }

    fn fstring(&mut self) -> Result<()> {
        let count = self.i32()?;
        let bytes = if count >= 0 {
            usize::try_from(count)?
        } else {
            usize::try_from(
                count
                    .checked_neg()
                    .with_context(|| format!("{}: invalid FString length", self.label))?,
            )?
            .checked_mul(2)
            .context("FString size overflow")?
        };
        self.take(bytes)?;
        Ok(())
    }

    fn bulk_array(
        &mut self,
        expected_element_size: Option<usize>,
        expected_count: Option<usize>,
        field: &str,
    ) -> Result<&'a [u8]> {
        let element_size = self.usize32(&format!("{field} element size"))?;
        let count = self.count(&format!("{field} count"))?;
        if let Some(expected) = expected_element_size
            && element_size != expected
        {
            bail!(
                "{}: {field} element size {element_size}, expected {expected}",
                self.label
            );
        }
        if let Some(expected) = expected_count
            && count != expected
        {
            bail!("{}: {field} count {count}, expected {expected}", self.label);
        }
        let size = element_size
            .checked_mul(count)
            .context("bulk array size overflow")?;
        self.take(size)
    }

    fn skip_bulk_array(
        &mut self,
        expected_element_size: Option<usize>,
        expected_count: Option<usize>,
        field: &str,
    ) -> Result<()> {
        self.bulk_array(expected_element_size, expected_count, field)?;
        Ok(())
    }

    fn skip_untyped_bulk_data(&mut self, field: &str) -> Result<()> {
        let flags = self.u32()?;
        let elements = self.usize32(&format!("{field} element count"))?;
        let size_on_disk = self.usize32(&format!("{field} disk size"))?;
        let data_offset = self.i32()?;
        if flags & BULKDATA_STORE_IN_SEPARATE_FILE != 0 {
            return Ok(());
        }
        if size_on_disk == 0 {
            return Ok(());
        }
        let absolute = self.absolute_offset()?;
        if data_offset < 0 || usize::try_from(data_offset)? != absolute {
            bail!(
                "{}: {field} payload offset {data_offset:#x}, expected inline {absolute:#x}",
                self.label
            );
        }
        if elements == 0 {
            bail!("{}: {field} has payload bytes but no elements", self.label);
        }
        self.take(size_on_disk)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_packed_normals() {
        let value = packed_normal(&[0, 128, 255, 0]);
        assert!((value[0] + 1.0).abs() < 0.0001);
        assert!(value[1].abs() < 0.01);
        assert!((value[2] - 1.0).abs() < 0.0001);
    }

    #[test]
    fn decodes_half_values() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
    }
}
