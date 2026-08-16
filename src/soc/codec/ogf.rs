//! OGF v4 visual headers as embedded in a compiled level.
//!
//! Level visuals are a narrow slice of the OGF format: they never carry their
//! own vertex payload, only a `OGF_GCONTAINER` reference into the shared
//! `level.geom` buffers. Tree visuals additionally carry the placement matrix
//! that `xrLC` factored out so identical foliage could share one buffer range.

use anyhow::{Context, Result, bail};

use super::chunks::{Chunks, Cursor};

/// `OGF_Chuncks` from `xray-16/src/xrCore/FMesh.hpp`.
pub const OGF_HEADER: u32 = 1;
pub const OGF_SWIDATA: u32 = 6;
pub const OGF_SWICONTAINER: u32 = 20;
pub const OGF_GCONTAINER: u32 = 21;
pub const OGF_TREEDEF2: u32 = 12;

/// `xrOGF_FormatVersion`.
pub const OGF_FORMAT_VERSION: u8 = 4;

/// `MT`: the visual class the engine instantiates for this chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualKind {
    /// `MT_NORMAL`: an ordinary static surface.
    Normal,
    /// `MT_PROGRESSIVE`: static geometry with a sliding-window LOD chain. The
    /// container range is the full-detail mesh.
    Progressive,
    /// `MT_TREE_ST` / `MT_TREE_PM`: static geometry plus a placement matrix and
    /// baked wind-animation constants.
    Tree,
    /// Anything the map converter does not render.
    Unsupported(u8),
}

impl VisualKind {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Normal,
            2 => Self::Progressive,
            7 | 11 => Self::Tree,
            other => Self::Unsupported(other),
        }
    }

    /// Whether this visual contributes static render geometry.
    pub fn is_static(self) -> bool {
        matches!(self, Self::Normal | Self::Progressive | Self::Tree)
    }

    pub fn label(self) -> String {
        match self {
            Self::Normal => "normal".into(),
            Self::Progressive => "progressive".into(),
            Self::Tree => "tree".into(),
            Self::Unsupported(value) => format!("mt_{value}"),
        }
    }
}

/// A `OGF_GCONTAINER` reference into the shared level buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryContainer {
    pub vertex_buffer: u32,
    pub vertex_base: u32,
    pub vertex_count: u32,
    pub index_buffer: u32,
    pub index_base: u32,
    pub index_count: u32,
}

/// One parsed level visual.
#[derive(Clone, Copy, Debug)]
pub struct Visual {
    pub kind: VisualKind,
    pub shader_id: u16,
    pub bounding_box: ([f32; 3], [f32; 3]),
    pub geometry: Option<GeometryContainer>,
    /// Row-major placement matrix from `OGF_TREEDEF2`, translation in row 3.
    pub tree_transform: Option<[f32; 16]>,
    /// `OGF_SWICONTAINER`: index of this visual's record in `fsL_SWIS`.
    ///
    /// Tree visuals keep their detail levels in the level's shared sliding
    /// window table rather than in their own chunk, so the container range has
    /// to be narrowed against that table once it is loaded.
    pub sliding_window: Option<u32>,
}

/// `OGF_Chuncks` a visual carries, in file order.
///
/// Diagnostic only: the converter acts on the three chunks it understands, and
/// this reports what a visual it declines actually contained. A visual kind
/// with no geometry chunk is a grouping node and loses nothing when dropped; a
/// kind carrying `OGF_CHILDREN` (9) holds its geometry in embedded children,
/// and dropping it loses that geometry outright.
pub fn visual_chunk_ids(payload: &[u8]) -> Vec<u32> {
    Chunks::new(payload, "level visual")
        .iter()
        .filter_map(|chunk| chunk.ok().map(|chunk| chunk.id))
        .collect()
}

/// Parse one visual chunk from `fsL_VISUALS`.
pub fn parse_visual(payload: &[u8], index: usize) -> Result<Visual> {
    let chunks = Chunks::new(payload, "level visual");
    let header = chunks
        .require(OGF_HEADER)
        .with_context(|| format!("visual {index} has no OGF header"))?;
    let mut cursor = Cursor::new(header, "OGF header");
    let format_version = cursor.u8()?;
    if format_version != OGF_FORMAT_VERSION {
        bail!("visual {index} uses OGF format version {format_version}, expected 4");
    }
    let kind = VisualKind::from_raw(cursor.u8()?);
    let shader_id = cursor.u16()?;
    let minimum = cursor.float3()?;
    let maximum = cursor.float3()?;
    // The bounding sphere is recomputed from converted geometry, so it is read
    // only to prove the header length.
    let _sphere_center = cursor.float3()?;
    let _sphere_radius = cursor.f32()?;

    let geometry = match chunks.find(OGF_GCONTAINER)? {
        Some(container) => {
            let mut cursor = Cursor::new(container, "OGF geometry container");
            let container = GeometryContainer {
                vertex_buffer: cursor.u32()?,
                vertex_base: cursor.u32()?,
                vertex_count: cursor.u32()?,
                index_buffer: cursor.u32()?,
                index_base: cursor.u32()?,
                index_count: cursor.u32()?,
            };
            if container.index_count % 3 != 0 {
                bail!(
                    "visual {index} declares {} indices, which is not a whole number of triangles",
                    container.index_count
                );
            }
            if container.vertex_count == 0 || container.index_count == 0 {
                bail!("visual {index} declares an empty geometry range");
            }
            Some(container)
        }
        None => None,
    };

    // A progressive mesh keeps every detail level in one container range, and
    // the engine draws exactly one of them:
    //
    // ```cpp
    // // xrRender/FProgressive.cpp
    // FSlideWindow& SW = nSWI.sw[lod_id];
    // cmd_list.Render( D3DPT_TRIANGLELIST, vBase, 0, SW.num_verts,
    //                  iBase + SW.offset, SW.num_tris );
    // ```
    //
    // with `lod_id = iFloor((1 - LOD) * (count - 1) + 0.5)`, so window 0 is the
    // full-detail mesh. Taking the whole container instead emits every level at
    // once, stacked in place: the object is drawn complete *and* simplified,
    // interpenetrating, with each level's own texture mapping.
    let geometry = match (geometry, chunks.find(OGF_SWIDATA)?) {
        (Some(container), Some(payload)) => {
            let mut cursor = Cursor::new(payload, "OGF sliding window data");
            for _ in 0..4 {
                cursor.u32()?; // reserved[4]
            }
            let count = cursor.u32()?;
            if count == 0 {
                bail!("visual {index} declares sliding window data with no windows");
            }
            let offset = cursor.u32()?;
            let triangles = u32::from(cursor.u16()?);
            let vertices = u32::from(cursor.u16()?);
            if triangles == 0 || vertices == 0 {
                bail!("visual {index} declares an empty full-detail sliding window");
            }
            if offset + triangles * 3 > container.index_count {
                bail!(
                    "visual {index} window 0 runs to index {} of a {}-index container",
                    offset + triangles * 3,
                    container.index_count
                );
            }
            Some(GeometryContainer {
                index_base: container.index_base + offset,
                index_count: triangles * 3,
                vertex_count: vertices.min(container.vertex_count),
                ..container
            })
        }
        (geometry, _) => geometry,
    };

    let tree_transform = match chunks.find(OGF_TREEDEF2)? {
        Some(treedef) => {
            let mut cursor = Cursor::new(treedef, "OGF tree definition");
            let mut transform = [0.0_f32; 16];
            for value in &mut transform {
                *value = cursor.f32()?;
            }
            if !transform.iter().all(|value| value.is_finite()) {
                bail!("visual {index} has a non-finite tree placement matrix");
            }
            // The remaining `c_scale`/`c_bias` colours are baked X-Ray wind and
            // hemisphere lighting; v0.100b lights the surface itself.
            Some(transform)
        }
        None => None,
    };

    if kind == VisualKind::Tree && tree_transform.is_none() {
        bail!("visual {index} is a tree visual without an OGF_TREEDEF2 placement matrix");
    }

    let sliding_window = match chunks.find(OGF_SWICONTAINER)? {
        Some(payload) => Some(Cursor::new(payload, "OGF sliding window container").u32()?),
        None => None,
    };

    Ok(Visual {
        kind,
        shader_id,
        bounding_box: (minimum, maximum),
        geometry,
        tree_transform,
        sliding_window,
    })
}

/// Transform a position by a row-major matrix with translation in row 3.
pub fn transform_position(matrix: &[f32; 16], position: [f32; 3]) -> [f32; 3] {
    [
        position[0] * matrix[0] + position[1] * matrix[4] + position[2] * matrix[8] + matrix[12],
        position[0] * matrix[1] + position[1] * matrix[5] + position[2] * matrix[9] + matrix[13],
        position[0] * matrix[2] + position[1] * matrix[6] + position[2] * matrix[10] + matrix[14],
    ]
}

/// Transform a direction by the rotation/scale part of the same matrix.
pub fn transform_direction(matrix: &[f32; 16], direction: [f32; 3]) -> [f32; 3] {
    let rotated = [
        direction[0] * matrix[0] + direction[1] * matrix[4] + direction[2] * matrix[8],
        direction[0] * matrix[1] + direction[1] * matrix[5] + direction[2] * matrix[9],
        direction[0] * matrix[2] + direction[1] * matrix[6] + direction[2] * matrix[10],
    ];
    let length =
        (rotated[0] * rotated[0] + rotated[1] * rotated[1] + rotated[2] * rotated[2]).sqrt();
    if length > f32::EPSILON {
        [
            rotated[0] / length,
            rotated[1] / length,
            rotated[2] / length,
        ]
    } else {
        direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn header_payload(kind: u8, shader_id: u16) -> Vec<u8> {
        let mut payload = vec![OGF_FORMAT_VERSION, kind];
        payload.extend_from_slice(&shader_id.to_le_bytes());
        for value in [-1.0_f32, -2.0, -3.0, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 4.0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload
    }

    fn container_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        for value in [0_u32, 100, 8, 0, 300, 12] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload
    }

    #[test]
    fn parses_a_static_visual() {
        let mut bytes = chunk(OGF_HEADER, &header_payload(0, 7));
        bytes.extend(chunk(OGF_GCONTAINER, &container_payload()));
        let visual = parse_visual(&bytes, 0).unwrap();
        assert_eq!(visual.kind, VisualKind::Normal);
        assert_eq!(visual.shader_id, 7);
        assert_eq!(visual.bounding_box.1, [1.0, 2.0, 3.0]);
        let geometry = visual.geometry.unwrap();
        assert_eq!(geometry.vertex_base, 100);
        assert_eq!(geometry.index_count, 12);
        assert!(visual.tree_transform.is_none());
    }

    #[test]
    fn requires_a_placement_matrix_for_tree_visuals() {
        let mut bytes = chunk(OGF_HEADER, &header_payload(7, 3));
        bytes.extend(chunk(OGF_GCONTAINER, &container_payload()));
        let error = parse_visual(&bytes, 4).unwrap_err().to_string();
        assert!(error.contains("OGF_TREEDEF2"), "{error}");
    }

    #[test]
    fn reads_the_tree_placement_matrix() {
        let mut identity = [0.0_f32; 16];
        identity[0] = 1.0;
        identity[5] = 1.0;
        identity[10] = 1.0;
        identity[15] = 1.0;
        identity[12] = 10.0;
        identity[13] = 20.0;
        identity[14] = 30.0;
        let mut treedef = Vec::new();
        for value in identity {
            treedef.extend_from_slice(&value.to_le_bytes());
        }
        // `c_scale` and `c_bias` follow as five floats each.
        treedef.extend(std::iter::repeat_n(0_u8, 40));

        let mut bytes = chunk(OGF_HEADER, &header_payload(7, 3));
        bytes.extend(chunk(OGF_GCONTAINER, &container_payload()));
        bytes.extend(chunk(OGF_TREEDEF2, &treedef));

        let visual = parse_visual(&bytes, 0).unwrap();
        let matrix = visual.tree_transform.unwrap();
        assert_eq!(transform_position(&matrix, [1.0, 1.0, 1.0]), [11.0, 21.0, 31.0]);
        assert_eq!(transform_direction(&matrix, [0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn rejects_an_index_range_that_is_not_whole_triangles() {
        let mut container = Vec::new();
        for value in [0_u32, 0, 8, 0, 0, 11] {
            container.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = chunk(OGF_HEADER, &header_payload(0, 1));
        bytes.extend(chunk(OGF_GCONTAINER, &container));
        let error = parse_visual(&bytes, 2).unwrap_err().to_string();
        assert!(error.contains("whole number of triangles"), "{error}");
    }

    #[test]
    fn classifies_unsupported_visual_types() {
        let bytes = chunk(OGF_HEADER, &header_payload(8, 1));
        let visual = parse_visual(&bytes, 0).unwrap();
        assert_eq!(visual.kind, VisualKind::Unsupported(8));
        assert!(!visual.kind.is_static());
        assert_eq!(visual.kind.label(), "mt_8");
    }
}
