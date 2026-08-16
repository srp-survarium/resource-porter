//! Encode a v0.100b collision family from a parsed `level.cform`.
//!
//! Collision is an atomic family: vertices, indices, the per-face material
//! dictionary and the face indices must be written together, and the parent
//! model settings must bind every dictionary entry to a target physical
//! material. Emitting any one of them alone produces a model that loads and
//! then behaves as the wrong substance.

use anyhow::{Context, Result, bail};

use crate::soc::codec::cform::Cform;
use super::game_material::CollisionMaterialBinding;

/// Chunk identifiers of the target collision family.
pub const CHUNK_VERTICES: u32 = 0x19;
pub const CHUNK_INDICES: u32 = 0x1a;
pub const CHUNK_FACE_DICTIONARY: u32 = 0x1b;
pub const CHUNK_FACE_INDICES: u32 = 0x1c;

/// The three encoded collision payloads.
#[derive(Clone, Debug)]
pub struct CollisionPayloads {
    pub vertices: Vec<u8>,
    pub indices: Vec<u8>,
    pub face_data: Vec<u8>,
    pub vertex_count: usize,
    pub triangle_count: usize,
}

/// Encode the collision family.
///
/// `bindings` must cover every material the level's faces actually use; its
/// order fixes the face-dictionary order, and the face indices point into it.
pub fn encode(cform: &Cform, bindings: &[CollisionMaterialBinding]) -> Result<CollisionPayloads> {
    if bindings.is_empty() {
        bail!("collision has no resolved material bindings");
    }
    let dictionary_index = |material: u16| {
        bindings
            .iter()
            .position(|binding| binding.source_id == material)
    };

    let mut vertices = Vec::with_capacity(8 + cform.vertices.len() * 12);
    vertices.extend_from_slice(&CHUNK_VERTICES.to_le_bytes());
    let vertex_bytes = cform
        .vertices
        .len()
        .checked_mul(12)
        .context("collision vertex payload overflow")?;
    vertices.extend_from_slice(&u32::try_from(vertex_bytes)?.to_le_bytes());
    for position in &cform.vertices {
        for value in position {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
    }

    let index_count = cform
        .faces
        .len()
        .checked_mul(3)
        .context("collision index count overflow")?;
    let mut indices = Vec::with_capacity(8 + index_count * 4);
    indices.extend_from_slice(&CHUNK_INDICES.to_le_bytes());
    let index_bytes = index_count
        .checked_mul(4)
        .context("collision index payload overflow")?;
    indices.extend_from_slice(&u32::try_from(index_bytes)?.to_le_bytes());
    for face in &cform.faces {
        for vertex in face.vertices {
            indices.extend_from_slice(&vertex.to_le_bytes());
        }
    }

    let mut dictionary = Vec::new();
    dictionary.extend_from_slice(&u16::try_from(bindings.len())?.to_le_bytes());
    for binding in bindings {
        if !binding.face_name.is_ascii() {
            bail!("collision face name {:?} is not ASCII", binding.face_name);
        }
        dictionary.extend_from_slice(binding.face_name.as_bytes());
        dictionary.push(0);
    }

    let face_bytes = cform
        .faces
        .len()
        .checked_mul(2)
        .context("collision face-data payload overflow")?;
    let mut face_data = Vec::with_capacity(16 + dictionary.len() + face_bytes);
    face_data.extend_from_slice(&CHUNK_FACE_DICTIONARY.to_le_bytes());
    face_data.extend_from_slice(&u32::try_from(dictionary.len())?.to_le_bytes());
    face_data.extend_from_slice(&dictionary);
    face_data.extend_from_slice(&CHUNK_FACE_INDICES.to_le_bytes());
    face_data.extend_from_slice(&u32::try_from(face_bytes)?.to_le_bytes());
    for (index, face) in cform.faces.iter().enumerate() {
        let slot = dictionary_index(face.material).with_context(|| {
            format!(
                "collision face {index} uses source material {}, which has no resolved binding",
                face.material
            )
        })?;
        face_data.extend_from_slice(&u16::try_from(slot)?.to_le_bytes());
    }

    Ok(CollisionPayloads {
        vertices,
        indices,
        face_data,
        vertex_count: cform.vertices.len(),
        triangle_count: cform.faces.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::codec::cform::{CformFace, CformHeader};
    use crate::soc::convert::game_material::face_dictionary_name;

    fn binding(source_id: u16, target_id: i32, target_name: &str) -> CollisionMaterialBinding {
        CollisionMaterialBinding {
            source_id,
            source_name: format!("materials\\{target_name}"),
            face_name: face_dictionary_name(source_id),
            target_id,
            target_name: target_name.to_owned(),
            triangle_count: 0,
        }
    }

    fn cform(vertex_count: usize, materials: &[u16]) -> Cform {
        Cform {
            header: CformHeader {
                version: 4,
                vertex_count: vertex_count as u32,
                face_count: materials.len() as u32,
                bounding_box: ([0.0; 3], [1.0; 3]),
            },
            vertices: (0..vertex_count).map(|i| [i as f32, 0.0, 0.0]).collect(),
            faces: materials
                .iter()
                .map(|material| CformFace {
                    vertices: [0, 1, 2],
                    material: *material,
                    suppress_shadows: false,
                    suppress_wallmarks: false,
                    sector: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn encodes_the_three_family_payloads() {
        let cform = cform(3, &[3, 39, 3]);
        let bindings = [binding(3, 19, "concrete"), binding(39, 13, "wood_silent")];
        let payloads = encode(&cform, &bindings).unwrap();

        assert_eq!(payloads.vertices.len(), 8 + 3 * 12);
        assert_eq!(
            u32::from_le_bytes(payloads.vertices[0..4].try_into().unwrap()),
            CHUNK_VERTICES
        );
        assert_eq!(payloads.indices.len(), 8 + 3 * 3 * 4);
        assert_eq!(
            u32::from_le_bytes(payloads.indices[0..4].try_into().unwrap()),
            CHUNK_INDICES
        );

        // dictionary: u16 count + two 13-byte NUL-terminated names
        let dictionary_size = 2 + 2 * 13;
        assert_eq!(payloads.face_data.len(), 8 + dictionary_size + 8 + 3 * 2);
        let face_indices = &payloads.face_data[8 + dictionary_size + 8..];
        assert_eq!(face_indices, &[0, 0, 1, 0, 0, 0]);
    }

    /// The recovered `cordon-v8` manifest records exact payload sizes. They are
    /// a byte-level contract for this encoder.
    #[test]
    fn reproduces_the_recorded_cordon_payload_sizes() {
        const VERTICES: usize = 698_013;
        const TRIANGLES: usize = 1_375_470;
        const MATERIALS: usize = 22;

        let vertices = 8 + VERTICES * 12;
        let indices = 8 + TRIANGLES * 3 * 4;
        // Every recorded face name is `soc_mtl_NNNN`: twelve bytes and a NUL.
        let face_data = 8 + (2 + MATERIALS * 13) + 8 + TRIANGLES * 2;

        assert_eq!(vertices, 8_376_164);
        assert_eq!(indices, 16_505_648);
        assert_eq!(face_data, 2_751_244);
    }

    #[test]
    fn fails_when_a_face_material_has_no_binding() {
        let cform = cform(3, &[3, 7]);
        let error = encode(&cform, &[binding(3, 19, "concrete")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("no resolved binding"), "{error}");
    }

    #[test]
    fn fails_without_any_binding() {
        let error = encode(&cform(3, &[3]), &[]).unwrap_err().to_string();
        assert!(error.contains("no resolved material bindings"), "{error}");
    }
}
