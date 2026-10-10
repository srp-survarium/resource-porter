// SPDX-License-Identifier: GPL-3.0-or-later
//! `level.cform`: the compiled collision mesh of an X-Ray 1.x level.
//!
//! The file is a flat header followed by two packed arrays. Unlike the render
//! geometry it is not chunked, so every bound is checked against the header
//! counts rather than a chunk size.

use anyhow::{Context, Result, bail};

use super::chunks::Cursor;

/// `CFORM_CURRENT_VERSION`.
pub const CFORM_VERSION: u32 = 4;

/// `sizeof(CDB::TRI)`.
const FACE_SIZE: usize = 16;

/// `hdrCFORM`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CformHeader {
    pub version: u32,
    pub vertex_count: u32,
    pub face_count: u32,
    pub bounding_box: ([f32; 3], [f32; 3]),
}

/// One collision triangle.
///
/// The trailing word packs a 14-bit game-material identifier, two rendering
/// suppression flags, and the owning sector. Only the material survives the
/// port: v0.100b resolves physical behaviour through its own dictionary, and
/// has no sector graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CformFace {
    pub vertices: [u32; 3],
    pub material: u16,
    pub suppress_shadows: bool,
    pub suppress_wallmarks: bool,
    pub sector: u16,
}

/// A parsed `level.cform`.
#[derive(Clone, Debug)]
pub struct Cform {
    pub header: CformHeader,
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<CformFace>,
}

impl Cform {
    /// Every distinct source material identifier, in ascending order.
    pub fn material_ids(&self) -> Vec<u16> {
        let mut ids: Vec<u16> = self.faces.iter().map(|face| face.material).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Triangle count per source material identifier.
    pub fn material_triangle_counts(&self) -> std::collections::BTreeMap<u16, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for face in &self.faces {
            *counts.entry(face.material).or_insert(0) += 1;
        }
        counts
    }
}

/// Parse `level.cform`.
pub fn parse(bytes: &[u8]) -> Result<Cform> {
    let mut cursor = Cursor::new(bytes, "level.cform");
    let header = CformHeader {
        version: cursor.u32()?,
        vertex_count: cursor.u32()?,
        face_count: cursor.u32()?,
        bounding_box: (cursor.float3()?, cursor.float3()?),
    };
    if header.version != CFORM_VERSION {
        bail!(
            "level.cform is version {}, expected {CFORM_VERSION}",
            header.version
        );
    }
    if header.vertex_count == 0 || header.face_count == 0 {
        bail!("level.cform declares an empty collision mesh");
    }

    let vertex_count = usize::try_from(header.vertex_count)?;
    let face_count = usize::try_from(header.face_count)?;

    let mut vertices = Vec::with_capacity(vertex_count);
    for index in 0..vertex_count {
        let position = cursor
            .float3()
            .with_context(|| format!("reading collision vertex {index}"))?;
        if !position.into_iter().all(f32::is_finite) {
            bail!("collision vertex {index} is non-finite");
        }
        vertices.push(position);
    }

    let mut faces = Vec::with_capacity(face_count);
    for index in 0..face_count {
        let record = cursor
            .take(FACE_SIZE)
            .with_context(|| format!("reading collision face {index}"))?;
        let vertex =
            |slot: usize| u32::from_le_bytes(record[slot * 4..slot * 4 + 4].try_into().unwrap());
        let vertices_of_face = [vertex(0), vertex(1), vertex(2)];
        for reference in vertices_of_face {
            if usize::try_from(reference)? >= vertex_count {
                bail!(
                    "collision face {index} references vertex {reference}, but only {vertex_count} exist"
                );
            }
        }
        let packed = u32::from_le_bytes(record[12..16].try_into().unwrap());
        faces.push(CformFace {
            vertices: vertices_of_face,
            material: (packed & 0x3fff) as u16,
            suppress_shadows: packed & (1 << 14) != 0,
            suppress_wallmarks: packed & (1 << 15) != 0,
            sector: (packed >> 16) as u16,
        });
    }

    // Retail levels append nothing after the face array; a cache block would
    // belong to a later engine and must not be silently ignored.
    if !cursor.is_empty() {
        bail!(
            "level.cform has {} trailing bytes after {face_count} faces",
            cursor.remaining()
        );
    }

    Ok(Cform {
        header,
        vertices,
        faces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(version: u32, material: u16, sector: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        for value in [0.0_f32, 0.0, 0.0, 1.0, 1.0, 1.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for position in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in [0_u32, 1, 2] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        let packed = u32::from(material) | (1 << 15) | (u32::from(sector) << 16);
        bytes.extend_from_slice(&packed.to_le_bytes());
        bytes
    }

    #[test]
    fn unpacks_material_flags_and_sector() {
        let cform = parse(&sample(CFORM_VERSION, 39, 7)).unwrap();
        assert_eq!(cform.header.vertex_count, 3);
        assert_eq!(cform.faces.len(), 1);
        let face = cform.faces[0];
        assert_eq!(face.vertices, [0, 1, 2]);
        assert_eq!(face.material, 39);
        assert!(!face.suppress_shadows);
        assert!(face.suppress_wallmarks);
        assert_eq!(face.sector, 7);
        assert_eq!(cform.material_ids(), vec![39]);
        assert_eq!(cform.material_triangle_counts()[&39], 1);
    }

    #[test]
    fn rejects_a_foreign_cform_version() {
        let error = parse(&sample(3, 0, 0)).unwrap_err().to_string();
        assert!(error.contains("version 3"), "{error}");
    }

    #[test]
    fn rejects_a_face_referencing_a_missing_vertex() {
        let mut bytes = sample(CFORM_VERSION, 0, 0);
        let offset = bytes.len() - FACE_SIZE;
        bytes[offset..offset + 4].copy_from_slice(&9_u32.to_le_bytes());
        let error = parse(&bytes).unwrap_err().to_string();
        assert!(error.contains("references vertex 9"), "{error}");
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = sample(CFORM_VERSION, 0, 0);
        bytes.push(0);
        assert!(parse(&bytes).is_err());
    }
}
