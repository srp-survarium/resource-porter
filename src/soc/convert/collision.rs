//! Encode a v0.100b collision family from a parsed `level.cform`.
//!
//! Collision is an atomic family: vertices, indices, the per-face material
//! dictionary and the face indices must be written together, and the parent
//! model settings must bind every dictionary entry to a target physical
//! material. Emitting any one of them alone produces a model that loads and
//! then behaves as the wrong substance.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use super::game_material::CollisionMaterialBinding;
use crate::soc::codec::cform::Cform;

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
    /// The bindings the face dictionary actually names, in dictionary order.
    ///
    /// The parent settings must bind exactly these and no others, so callers
    /// take the list from here rather than from what they passed in.
    pub bindings: Vec<CollisionMaterialBinding>,
    /// World-space extent of this piece, as `(min, max)`.
    ///
    /// The archive carrying a piece has to declare a bounding box that
    /// contains it; the caller unions this with its render surfaces.
    pub bounds: ([f32; 3], [f32; 3]),
}

/// How many entries the per-face material dictionary carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceDictionary {
    /// One entry per source material, with each face pointing at its own.
    ///
    /// This is what the source data supports and what the port wants. It is
    /// also unproven: the recovered Cordon bundles only produced it in `v8`,
    /// the last one, and no bundle from `v3` onward was ever shown to load.
    PerMaterial,
    /// A single entry that every face points at.
    ///
    /// The whole mesh becomes one substance, which is a real loss — footsteps,
    /// bullet impacts and wallmarks stop distinguishing metal from foliage.
    /// It is the only collision shape this engine is known to have loaded: the
    /// runtime-tested STK2 maps emitted exactly this, and so did Cordon `v3`
    /// through `v7`.
    Single,
}

/// The largest collision mesh v0.100b is known to load.
///
/// Measured across every collision model in the retail database: the biggest
/// is `models/level/03/antenna_base.model` at 6,911 triangles, and the biggest
/// terrain tile is `terrain_p_08` at 4,751. Nothing ships a level's collision
/// as one mesh -- a map's collision is many small per-object models.
///
/// A Shadow of Chernobyl `level.cform` is the opposite: one mesh for the whole
/// level, up to 1,375,470 triangles for Cordon. Handing that to the cooker
/// fails, and the engine does not check the result before dereferencing it:
///
/// ```text
/// resources <Warning>  cook of 'ported/soc/mp_pool/mp_pool_000' [...] failed
/// debug     <ERROR>    ACCESS_VIOLATION
/// ```
///
/// The cook runs inside a fixed 1,179,648-byte `cook allocator` arena, which
/// is the size an acceleration structure over a few thousand triangles needs.
pub const MAX_TRIANGLES_PER_MODEL: usize = 6_000;

/// Split one level-sized collision mesh into independently cookable pieces.
///
/// The split is spatial -- recursive median bisection along the longest axis
/// of the piece's bounding box -- so each piece stays a compact region rather
/// than a scattering of triangles from all over the map. That matters for the
/// broadphase: a piece's bounding box is what the engine tests first, and
/// pieces that overlap the whole level would defeat it.
///
/// Vertices are re-indexed per piece, so a piece references only its own.
pub fn split(cform: &Cform, max_triangles: usize) -> Vec<Cform> {
    assert!(max_triangles > 0, "max_triangles must be positive");

    let centroid = |face: &crate::soc::codec::cform::CformFace| {
        let [a, b, c] = face.vertices.map(|index| cform.vertices[index as usize]);
        [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]
    };

    let mut pieces = Vec::new();
    let mut pending = vec![(0..cform.faces.len()).collect::<Vec<usize>>()];
    while let Some(mut faces) = pending.pop() {
        if faces.len() <= max_triangles {
            pieces.push(faces);
            continue;
        }

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for index in &faces {
            let point = centroid(&cform.faces[*index]);
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        let axis = (0..3)
            .max_by(|a, b| {
                (max[*a] - min[*a])
                    .partial_cmp(&(max[*b] - min[*b]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("three axes");

        // Degenerate extent: the centroids coincide on every axis, so no split
        // separates them. Cut the list in half rather than recursing forever.
        let middle = faces.len() / 2;
        if !(max[axis] - min[axis]).is_finite() || max[axis] <= min[axis] {
            let tail = faces.split_off(middle);
            pending.push(faces);
            pending.push(tail);
            continue;
        }

        faces.sort_unstable_by(|a, b| {
            centroid(&cform.faces[*a])[axis]
                .partial_cmp(&centroid(&cform.faces[*b])[axis])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let tail = faces.split_off(middle);
        pending.push(faces);
        pending.push(tail);
    }

    // Bisection pops depth-first, so restore a stable spatial order.
    pieces.sort_unstable_by_key(|faces| faces.first().copied().unwrap_or(usize::MAX));

    pieces
        .into_iter()
        .map(|faces| {
            let mut remap = BTreeMap::new();
            let mut vertices = Vec::new();
            let mut rebuilt = Vec::with_capacity(faces.len());
            for index in faces {
                let face = &cform.faces[index];
                let mapped = face.vertices.map(|vertex| {
                    *remap.entry(vertex).or_insert_with(|| {
                        vertices.push(cform.vertices[vertex as usize]);
                        (vertices.len() - 1) as u32
                    })
                });
                rebuilt.push(crate::soc::codec::cform::CformFace {
                    vertices: mapped,
                    ..*face
                });
            }
            Cform {
                header: crate::soc::codec::cform::CformHeader {
                    vertex_count: vertices.len() as u32,
                    face_count: rebuilt.len() as u32,
                    ..cform.header
                },
                vertices,
                faces: rebuilt,
            }
        })
        .collect()
}

/// Encode the collision family.
///
/// `bindings` must cover every material the level's faces actually use; its
/// order fixes the face-dictionary order, and the face indices point into it.
pub fn encode(
    cform: &Cform,
    bindings: &[CollisionMaterialBinding],
    dictionary: FaceDictionary,
) -> Result<CollisionPayloads> {
    if bindings.is_empty() {
        bail!("collision has no resolved material bindings");
    }

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

    // In single-entry mode the dictionary names only the material covering the
    // most triangles, and every face points at it.
    let entries: Vec<&CollisionMaterialBinding> = match dictionary {
        // Only the materials this piece's faces actually use: retail's
        // `garage.model` names its two and no more.
        FaceDictionary::PerMaterial => {
            let used: BTreeSet<u16> = cform.faces.iter().map(|face| face.material).collect();
            bindings
                .iter()
                .filter(|binding| used.contains(&binding.source_id))
                .collect()
        }
        FaceDictionary::Single => vec![
            bindings
                .iter()
                .max_by_key(|binding| binding.triangle_count)
                .expect("bindings are not empty"),
        ],
    };

    let mut dictionary_bytes = Vec::new();
    dictionary_bytes.extend_from_slice(&u16::try_from(entries.len())?.to_le_bytes());
    for binding in &entries {
        if !binding.face_name.is_ascii() {
            bail!("collision face name {:?} is not ASCII", binding.face_name);
        }
        dictionary_bytes.extend_from_slice(binding.face_name.as_bytes());
        dictionary_bytes.push(0);
    }

    let face_bytes = cform
        .faces
        .len()
        .checked_mul(2)
        .context("collision face-data payload overflow")?;
    let mut face_data = Vec::with_capacity(16 + dictionary_bytes.len() + face_bytes);
    face_data.extend_from_slice(&CHUNK_FACE_DICTIONARY.to_le_bytes());
    face_data.extend_from_slice(&u32::try_from(dictionary_bytes.len())?.to_le_bytes());
    face_data.extend_from_slice(&dictionary_bytes);
    face_data.extend_from_slice(&CHUNK_FACE_INDICES.to_le_bytes());
    face_data.extend_from_slice(&u32::try_from(face_bytes)?.to_le_bytes());
    for (index, face) in cform.faces.iter().enumerate() {
        let slot = match dictionary {
            FaceDictionary::Single => 0,
            FaceDictionary::PerMaterial => entries
                .iter()
                .position(|binding| binding.source_id == face.material)
                .with_context(|| {
                    format!(
                        "collision face {index} uses source material {}, which has no resolved binding",
                        face.material
                    )
                })?,
        };
        face_data.extend_from_slice(&u16::try_from(slot)?.to_le_bytes());
    }

    Ok(CollisionPayloads {
        vertices,
        indices,
        face_data,
        vertex_count: cform.vertices.len(),
        triangle_count: cform.faces.len(),
        bindings: entries.into_iter().cloned().collect(),
        bounds: {
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for position in &cform.vertices {
                for axis in 0..3 {
                    min[axis] = min[axis].min(position[axis]);
                    max[axis] = max[axis].max(position[axis]);
                }
            }
            (min, max)
        },
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
        let payloads = encode(&cform, &bindings, FaceDictionary::PerMaterial).unwrap();

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

    /// The single-entry dictionary names the material covering the most
    /// triangles, points every face at it, and reports only that one binding
    /// so the parent settings cannot bind materials the dictionary omits.
    #[test]
    fn single_entry_dictionary_collapses_every_face() {
        let cform = cform(3, &[3, 39, 3]);
        let mut concrete = binding(3, 19, "concrete");
        concrete.triangle_count = 2;
        let mut wood = binding(39, 13, "wood_silent");
        wood.triangle_count = 1;
        let payloads = encode(&cform, &[wood, concrete], FaceDictionary::Single).unwrap();

        assert_eq!(payloads.bindings.len(), 1);
        assert_eq!(payloads.bindings[0].target_name, "concrete");
        let dictionary_size = 2 + 13;
        assert_eq!(payloads.face_data.len(), 8 + dictionary_size + 8 + 3 * 2);
        assert_eq!(&payloads.face_data[8 + dictionary_size + 8..], &[0; 6]);
    }

    /// A face whose material has no binding is fatal only when the dictionary
    /// is per-material; collapsing to one entry makes every face resolvable.
    #[test]
    fn single_entry_dictionary_accepts_unbound_face_materials() {
        let payloads = encode(
            &cform(3, &[3, 7]),
            &[binding(3, 19, "concrete")],
            FaceDictionary::Single,
        )
        .unwrap();
        assert_eq!(&payloads.face_data[8 + 2 + 13 + 8..], &[0; 4]);
    }

    /// Every piece must be independently cookable, and together the pieces must
    /// still be the same mesh: no triangle added, dropped or duplicated.
    #[test]
    fn split_bounds_every_piece_and_preserves_the_mesh() {
        let count = 1_000;
        let mut source = cform(count * 3, &vec![3; count]);
        for (index, face) in source.faces.iter_mut().enumerate() {
            let base = (index * 3) as u32;
            face.vertices = [base, base + 1, base + 2];
        }
        for (index, position) in source.vertices.iter_mut().enumerate() {
            *position = [(index / 3) as f32, 0.0, (index % 7) as f32];
        }

        let pieces = split(&source, 64);
        assert!(pieces.len() >= 16, "{} pieces", pieces.len());
        assert!(pieces.iter().all(|piece| piece.faces.len() <= 64));
        assert_eq!(
            pieces.iter().map(|piece| piece.faces.len()).sum::<usize>(),
            count
        );

        // Re-indexed vertices must still describe the original triangles.
        let mut rebuilt: Vec<[[f32; 3]; 3]> = pieces
            .iter()
            .flat_map(|piece| {
                piece.faces.iter().map(|face| {
                    assert!(
                        face.vertices
                            .iter()
                            .all(|v| (*v as usize) < piece.vertices.len())
                    );
                    face.vertices.map(|v| piece.vertices[v as usize])
                })
            })
            .collect();
        let mut original: Vec<[[f32; 3]; 3]> = source
            .faces
            .iter()
            .map(|face| face.vertices.map(|v| source.vertices[v as usize]))
            .collect();
        let key = |t: &[[f32; 3]; 3]| format!("{t:?}");
        rebuilt.sort_by_key(key);
        original.sort_by_key(key);
        assert_eq!(rebuilt, original);
    }

    /// Coincident centroids have no axis to split on; the bisection must still
    /// terminate rather than recurse on an undivided list.
    #[test]
    fn split_terminates_on_coincident_triangles() {
        let source = cform(3, &vec![3; 500]);
        let pieces = split(&source, 16);
        assert_eq!(pieces.iter().map(|p| p.faces.len()).sum::<usize>(), 500);
        assert!(pieces.iter().all(|piece| piece.faces.len() <= 16));
    }

    /// A piece names only the materials its own faces use, as retail's
    /// two-entry `garage.model` does.
    #[test]
    fn dictionary_names_only_the_materials_a_piece_uses() {
        let payloads = encode(
            &cform(3, &[39, 39]),
            &[binding(3, 19, "concrete"), binding(39, 13, "wood_silent")],
            FaceDictionary::PerMaterial,
        )
        .unwrap();
        assert_eq!(payloads.bindings.len(), 1);
        assert_eq!(payloads.bindings[0].target_name, "wood_silent");
        assert_eq!(&payloads.face_data[8 + 2 + 13 + 8..], &[0; 4]);
    }

    #[test]
    fn fails_when_a_face_material_has_no_binding() {
        let cform = cform(3, &[3, 7]);
        let error = encode(
            &cform,
            &[binding(3, 19, "concrete")],
            FaceDictionary::PerMaterial,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("no resolved binding"), "{error}");
    }

    #[test]
    fn fails_without_any_binding() {
        let error = encode(&cform(3, &[3]), &[], FaceDictionary::PerMaterial)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no resolved material bindings"), "{error}");
    }
}
