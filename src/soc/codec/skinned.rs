// SPDX-License-Identifier: GPL-3.0-or-later
//! The render geometry of an X-Ray 1.x skinned actor model (`.ogf`).
//!
//! An actor `.ogf` (`MT_SKELETON_ANIM`) holds its meshes as children of
//! `OGF_CHILDREN`, one per surface: each child names its texture and shader
//! (`OGF_TEXTURE`) and carries its own vertices (`OGF_VERTICES`) and
//! triangle indices (`OGF_INDICES`). The bone each vertex is bound to is an
//! index into the model's `OGF_S_BONE_NAMES` ([`super::skeleton`]).
//!
//! Vertices are stored in model space, in the bind pose. Their layout is
//! named by the vertex format id; Shadow of Chernobyl writes the long ids
//! for one and two bones, Clear Sky and Call of Pripyat the short ones
//! (`xrCore/FMesh.hpp`, `Layers/xrRender/SkeletonX.cpp`):
//!
//! | bones | ids | layout | bytes |
//! | --- | --- | --- | --- |
//! | 1 | `0x12071980`, 1 | P N T B uv, u32 bone | 60 |
//! | 2 | `0x240e3300`, 2 | u16 bone[2], P N T B, w (bone 1), uv | 64 |
//! | 3 | 3 | u16 bone[3], P N T B, w[2], uv | 70 |
//! | 4 | 4 | u16 bone[4], P N T B, w[3], uv | 76 |
//!
//! The last bone takes the rest of the weight.

use anyhow::{Context, Result, bail, ensure};

use super::chunks::{Chunks, Cursor};

/// `OGF_Chuncks` from `xrCore/FMesh.hpp`.
pub const OGF_TEXTURE: u32 = 2;
pub const OGF_VERTICES: u32 = 3;
pub const OGF_INDICES: u32 = 4;
pub const OGF_CHILDREN: u32 = 9;

/// One vertex of a skinned mesh, in model space at the bind pose.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub uv: [f32; 2],
    /// (bone index, weight), weights summing to one.
    pub bones: Vec<(u16, f32)>,
}

/// One surface of a skinned model.
#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    /// `OGF_TEXTURE`: the texture, e.g. `act\act_mp_team_blue_1`.
    pub texture: String,
    /// `OGF_TEXTURE`: the shader, e.g. `models\model`.
    pub shader: String,
    /// Bones a vertex of this mesh carries (1-4).
    pub links: usize,
    pub vertices: Vec<SkinnedVertex>,
    pub triangles: Vec<[u16; 3]>,
}

/// Every child mesh of an actor model, in file order.
pub fn parse_meshes(bytes: &[u8], label: &str) -> Result<Vec<SkinnedMesh>> {
    let children = Chunks::new(bytes, label)
        .require(OGF_CHILDREN)
        .with_context(|| format!("{label}: no OGF_CHILDREN: not a multi-surface actor model"))?;
    Chunks::new(children, label)
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            parse_mesh(chunk?.payload, label).with_context(|| format!("{label}: child {index}"))
        })
        .collect()
}

fn parse_mesh(bytes: &[u8], label: &str) -> Result<SkinnedMesh> {
    let chunks = Chunks::new(bytes, label);
    let mut texture = Cursor::new(chunks.require(OGF_TEXTURE)?, label);
    let (texture, shader) = (texture.stringz()?, texture.stringz()?);
    let mut cursor = Cursor::new(chunks.require(OGF_VERTICES)?, label);
    let format = cursor.u32()?;
    let count = usize::try_from(cursor.u32()?)?;
    let links = match format {
        1 | 0x1207_1980 => 1,
        2 | 0x240e_3300 => 2,
        3 => 3,
        4 => 4,
        other => bail!("vertex format {other:#x} is not a skinned layout"),
    };
    let mut vertices = Vec::with_capacity(count);
    for _ in 0..count {
        let mut ids = [0_u16; 4];
        if links > 1 {
            for id in ids.iter_mut().take(links) {
                *id = cursor.u16()?;
            }
        }
        let position = cursor.float3()?;
        let normal = cursor.float3()?;
        let tangent = cursor.float3()?;
        let binormal = cursor.float3()?;
        let mut weights = [0.0_f32; 4];
        match links {
            1 => {}
            2 => weights[1] = cursor.f32()?,
            n => {
                for w in weights.iter_mut().take(n - 1) {
                    *w = cursor.f32()?;
                }
            }
        }
        let uv = [cursor.f32()?, cursor.f32()?];
        if links == 1 {
            ids[0] = u16::try_from(cursor.u32()?).context("bone index above 65535")?;
        }
        let bones = match links {
            1 => vec![(ids[0], 1.0)],
            2 => vec![(ids[0], 1.0 - weights[1]), (ids[1], weights[1])],
            n => {
                let rest = 1.0 - weights[..n - 1].iter().sum::<f32>();
                (0..n)
                    .map(|k| (ids[k], if k == n - 1 { rest } else { weights[k] }))
                    .collect()
            }
        };
        vertices.push(SkinnedVertex {
            position,
            normal,
            tangent,
            binormal,
            uv,
            bones,
        });
    }
    ensure!(
        cursor.is_empty(),
        "{} bytes after {count} {links}-bone vertices",
        cursor.remaining()
    );
    let mut cursor = Cursor::new(chunks.require(OGF_INDICES)?, label);
    let indices = usize::try_from(cursor.u32()?)?;
    ensure!(
        indices % 3 == 0,
        "{indices} indices are not whole triangles"
    );
    let mut triangles = Vec::with_capacity(indices / 3);
    for _ in 0..indices / 3 {
        let t = [cursor.u16()?, cursor.u16()?, cursor.u16()?];
        ensure!(
            t.iter().all(|&i| usize::from(i) < count),
            "triangle {t:?} indexes past {count} vertices"
        );
        triangles.push(t);
    }
    Ok(SkinnedMesh {
        texture,
        shader,
        links,
        vertices,
        triangles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn floats(out: &mut Vec<u8>, values: &[f32]) {
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn child(format: u32, vertex: &[u8], count: u32) -> Vec<u8> {
        let mut vertices = format.to_le_bytes().to_vec();
        vertices.extend_from_slice(&count.to_le_bytes());
        for _ in 0..count {
            vertices.extend_from_slice(vertex);
        }
        let mut indices = 3_u32.to_le_bytes().to_vec();
        for i in [0_u16, 1, 2] {
            indices.extend_from_slice(&i.to_le_bytes());
        }
        let mut bytes = chunk(1, &[4, 3, 0, 0]);
        bytes.extend(chunk(OGF_TEXTURE, b"act\\band\0models\\selflightl\0"));
        bytes.extend(chunk(OGF_VERTICES, &vertices));
        bytes.extend(chunk(OGF_INDICES, &indices));
        bytes
    }

    fn model(children: &[Vec<u8>]) -> Vec<u8> {
        let mut list = vec![];
        for (i, c) in children.iter().enumerate() {
            list.extend(chunk(i as u32, c));
        }
        chunk(OGF_CHILDREN, &list)
    }

    #[test]
    fn reads_three_and_four_bone_vertices_of_clear_sky() {
        let mut v3 = vec![];
        for b in [5_u16, 6, 7] {
            v3.extend_from_slice(&b.to_le_bytes());
        }
        floats(
            &mut v3,
            &[1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        );
        floats(&mut v3, &[0.5, 0.25, 0.1, 0.9]);
        assert_eq!(v3.len(), 70);
        let mut v4 = vec![];
        for b in [1_u16, 2, 3, 4] {
            v4.extend_from_slice(&b.to_le_bytes());
        }
        floats(&mut v4, &[0.0; 12]);
        floats(&mut v4, &[0.4, 0.3, 0.2, 0.5, 0.5]);
        assert_eq!(v4.len(), 76);
        let meshes = parse_meshes(&model(&[child(3, &v3, 3), child(4, &v4, 3)]), "test").unwrap();
        assert_eq!(meshes[0].texture, "act\\band");
        assert_eq!(meshes[0].shader, "models\\selflightl");
        let v = &meshes[0].vertices[0];
        assert_eq!(v.position, [1.0, 2.0, 3.0]);
        assert_eq!(v.uv, [0.1, 0.9]);
        assert_eq!(v.bones, vec![(5, 0.5), (6, 0.25), (7, 0.25)]);
        assert_eq!(meshes[1].links, 4);
        let w: Vec<u16> = meshes[1].vertices[0].bones.iter().map(|b| b.0).collect();
        assert_eq!(w, [1, 2, 3, 4]);
        assert!((meshes[1].vertices[0].bones[3].1 - 0.1).abs() < 1e-6);
        assert_eq!(meshes[1].triangles, vec![[0, 1, 2]]);
    }

    #[test]
    fn reads_the_one_and_two_bone_layouts_of_shadow_of_chernobyl() {
        let mut v1 = vec![];
        floats(&mut v1, &[0.0; 12]);
        floats(&mut v1, &[0.5, 0.5]);
        v1.extend_from_slice(&9_u32.to_le_bytes());
        assert_eq!(v1.len(), 60);
        let mut v2 = vec![];
        for b in [2_u16, 8] {
            v2.extend_from_slice(&b.to_le_bytes());
        }
        floats(&mut v2, &[0.0; 12]);
        floats(&mut v2, &[0.75, 0.5, 0.5]);
        assert_eq!(v2.len(), 64);
        let meshes = parse_meshes(
            &model(&[child(0x1207_1980, &v1, 3), child(0x240e_3300, &v2, 3)]),
            "t",
        )
        .unwrap();
        assert_eq!(meshes[0].vertices[0].bones, vec![(9, 1.0)]);
        assert_eq!(meshes[1].vertices[0].bones, vec![(2, 0.25), (8, 0.75)]);
    }

    #[test]
    fn rejects_an_index_past_the_vertices() {
        let mut v1 = vec![];
        floats(&mut v1, &[0.0; 14]);
        v1.extend_from_slice(&0_u32.to_le_bytes());
        let mut c = child(1, &v1, 2);
        // The index chunk is last: point its third index past the two vertices.
        let n = c.len();
        c[n - 2..].copy_from_slice(&5_u16.to_le_bytes());
        let error = format!("{:#}", parse_meshes(&model(&[c]), "t").unwrap_err());
        assert!(error.contains("indexes past"), "{error}");
    }
}
