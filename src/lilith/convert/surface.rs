//! Regroup a world's render surfaces into target static surfaces.
//!
//! F.E.A.R. batches draw calls per material already, so unlike the SoC path
//! there is no visual tree to walk: every render surface belonging to one
//! material is welded into one target surface, split into parts only when the
//! 16-bit index format demands it. The material index doubles as the target
//! surface's `shader_id`, so each source material keeps its own textures --
//! finer-grained than the SoC ports, which merged per shader.
//!
//! Everything here scales positions by [`crate::lilith::SCALE`]: LithTech
//! centimetres in, target metres out.

use anyhow::{Context, Result, bail};

use crate::lilith::SCALE;
use crate::lilith::codec::world::RenderMesh;
use crate::soc::codec::vertex::SocVertex;
use crate::soc::convert::surface::MergedSurface;

use serde::{Deserialize, Serialize};

/// What the merge kept and dropped, for the build record.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MergeReport {
    /// Source render surfaces converted.
    pub converted_surfaces: usize,
    /// Source render surfaces dropped with their family, by shader.
    pub dropped_surfaces: usize,
    pub dropped_triangles: usize,
    pub triangles: usize,
}

/// Weld each material's render surfaces into target surfaces.
///
/// `dropped` holds the material indices whose shader the classification
/// declined; their surfaces are counted and left behind.
pub fn merge_surfaces(
    render: &RenderMesh,
    dropped: &std::collections::BTreeSet<u32>,
    label: &str,
) -> Result<(Vec<MergedSurface>, MergeReport)> {
    let mut report = MergeReport::default();
    let mut merged = Vec::new();
    let material_count = render.materials.len();
    for material in 0..material_count {
        let material = material as u32;
        let surfaces: Vec<usize> = render
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, surface)| surface.material == material)
            .map(|(index, _)| index)
            .collect();
        if surfaces.is_empty() {
            continue;
        }
        if dropped.contains(&material) {
            report.dropped_surfaces += surfaces.len();
            report.dropped_triangles += surfaces
                .iter()
                .map(|&index| render.surfaces[index].triangle_count as usize)
                .sum::<usize>();
            continue;
        }
        let shader_id = u16::try_from(material)
            .with_context(|| format!("{label}: material index {material} exceeds u16"))?;

        let mut part = PartBuilder::new(shader_id, merged.len());
        let mut parts = Vec::new();
        for &index in &surfaces {
            let decoded = render.decode_surface(index, label)?;
            let vertices = convert_vertices(&decoded)?;
            if part.vertices.len() + vertices.len() > u16::MAX as usize {
                if vertices.len() > u16::MAX as usize {
                    bail!(
                        "{label}: one render surface carries {} vertices, past the \
                         16-bit index format",
                        vertices.len()
                    );
                }
                parts.push(part.finish(&mut report));
                part = PartBuilder::new(shader_id, parts.len());
            }
            part.append(&vertices, &decoded.indices, label)?;
            report.converted_surfaces += 1;
        }
        parts.push(part.finish(&mut report));
        merged.extend(parts.into_iter());
    }
    // Re-number parts per material: the builder numbered them within the
    // material, but `MergedSurface::name` wants a stable unique name, which
    // `shader_id` plus `part` already is.
    if merged.is_empty() {
        bail!("{label}: every render surface was dropped; nothing to convert");
    }
    Ok((merged, report))
}

struct PartBuilder {
    shader_id: u16,
    part: usize,
    vertices: Vec<SocVertex>,
    indices: Vec<u16>,
}

impl PartBuilder {
    fn new(shader_id: u16, part: usize) -> Self {
        Self {
            shader_id,
            part,
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn append(&mut self, vertices: &[SocVertex], indices: &[u32], label: &str) -> Result<()> {
        let base = self.vertices.len();
        self.vertices.extend_from_slice(vertices);
        for &index in indices {
            let rebased = base + index as usize;
            let rebased = u16::try_from(rebased)
                .with_context(|| format!("{label}: part index {rebased} exceeds u16"))?;
            self.indices.push(rebased);
        }
        Ok(())
    }

    fn finish(self, report: &mut MergeReport) -> MergedSurface {
        report.triangles += self.indices.len() / 3;
        MergedSurface {
            shader_id: self.shader_id,
            part: self.part,
            vertices: self.vertices,
            indices: self.indices,
            visual_count: 1,
            tree_visual_count: 0,
        }
    }
}

/// One decoded surface's vertices in the target's layout and scale.
///
/// Attributes the source layout lacks are synthesised rather than zeroed:
///
/// * normals -- averaged face normals, because a zero normal renders black;
/// * tangent frames -- any orthonormal pair around the normal. The families
///   without authored tangents are unlit glass and additive glows, which
///   never sample a normal map, so the frame only has to be valid, not
///   UV-aligned.
fn convert_vertices(decoded: &crate::lilith::codec::world::DecodedSurface) -> Result<Vec<SocVertex>> {
    let count = decoded.positions.len();
    let mut normals = if decoded.normals.len() == count {
        decoded.normals.clone()
    } else {
        face_normals(&decoded.positions, &decoded.indices)
    };
    for normal in &mut normals {
        let length =
            (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if length > f32::EPSILON {
            for axis in normal.iter_mut() {
                *axis /= length;
            }
        } else {
            *normal = [0.0, 1.0, 0.0];
        }
    }

    let mut vertices = Vec::with_capacity(count);
    for index in 0..count {
        let normal = normals[index];
        let (tangent, binormal) = if decoded.tangents.len() == count
            && decoded.binormals.len() == count
        {
            (decoded.tangents[index], decoded.binormals[index])
        } else {
            orthonormal_frame(normal)
        };
        vertices.push(SocVertex {
            position: decoded.positions[index].map(|value| value * SCALE),
            normal,
            tangent,
            binormal,
            uv: if decoded.uvs.len() == count {
                decoded.uvs[index]
            } else {
                [0.0, 0.0]
            },
        });
    }
    Ok(vertices)
}

fn face_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0f32; 3]; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            positions[triangle[0] as usize],
            positions[triangle[1] as usize],
            positions[triangle[2] as usize],
        ];
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for &corner in triangle {
            for axis in 0..3 {
                normals[corner as usize][axis] += face[axis];
            }
        }
    }
    normals
}

/// Any orthonormal (tangent, binormal) pair around `normal`.
fn orthonormal_frame(normal: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let reference = if normal[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let tangent = [
        normal[1] * reference[2] - normal[2] * reference[1],
        normal[2] * reference[0] - normal[0] * reference[2],
        normal[0] * reference[1] - normal[1] * reference[0],
    ];
    let length =
        (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt();
    let tangent = tangent.map(|axis| axis / length.max(f32::EPSILON));
    let binormal = [
        normal[1] * tangent[2] - normal[2] * tangent[1],
        normal[2] * tangent[0] - normal[0] * tangent[2],
        normal[0] * tangent[1] - normal[1] * tangent[0],
    ];
    (tangent, binormal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_orthonormal() {
        for normal in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.577, 0.577, 0.577]] {
            let (tangent, binormal) = orthonormal_frame(normal);
            let dot_nt: f32 = (0..3).map(|axis| normal[axis] * tangent[axis]).sum();
            let dot_nb: f32 = (0..3).map(|axis| normal[axis] * binormal[axis]).sum();
            assert!(dot_nt.abs() < 1e-3, "{normal:?} {tangent:?}");
            assert!(dot_nb.abs() < 1e-3, "{normal:?} {binormal:?}");
        }
    }
}
