//! Resolve cooked `StaticMeshComponent` objects into world placements.

use anyhow::{Context, Result};

use super::package::{Package, Property};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4 {
    /// UE3 row-vector matrix (`point * matrix`).
    pub rows: [[f32; 4]; 4],
}

impl Matrix4 {
    pub fn transform_point(self, point: [f32; 3]) -> [f32; 3] {
        [
            point[0] * self.rows[0][0]
                + point[1] * self.rows[1][0]
                + point[2] * self.rows[2][0]
                + self.rows[3][0],
            point[0] * self.rows[0][1]
                + point[1] * self.rows[1][1]
                + point[2] * self.rows[2][1]
                + self.rows[3][1],
            point[0] * self.rows[0][2]
                + point[1] * self.rows[1][2]
                + point[2] * self.rows[2][2]
                + self.rows[3][2],
        ]
    }

    pub fn transform_vector(self, vector: [f32; 3]) -> [f32; 3] {
        normalize([
            vector[0] * self.rows[0][0] + vector[1] * self.rows[1][0] + vector[2] * self.rows[2][0],
            vector[0] * self.rows[0][1] + vector[1] * self.rows[1][1] + vector[2] * self.rows[2][1],
            vector[0] * self.rows[0][2] + vector[1] * self.rows[1][2] + vector[2] * self.rows[2][2],
        ])
    }

    /// Determinant of the affine transform's linear 3x3 portion.
    ///
    /// A negative value means UE3 mirrored this instance with a negative
    /// actor/component scale, which reverses its triangle winding.
    pub fn determinant3(self) -> f32 {
        let m = self.rows;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
}

#[derive(Clone, Debug)]
pub struct StaticMeshPlacement {
    pub actor: String,
    pub actor_class: String,
    pub component: String,
    pub mesh: String,
    pub mesh_index: i32,
    pub local_mesh: bool,
    /// Per-section component overrides. A null entry means "use the mesh's
    /// authored material" for that slot.
    pub material_overrides: Vec<Option<String>>,
    pub visible: bool,
    pub collision_enabled: bool,
    pub absolute_translation: bool,
    pub absolute_rotation: bool,
    pub absolute_scale: bool,
    /// Bounds authored by custom actors such as `SDStitchingProxy`. Dirty
    /// Bomb names the property `CombinedBounds`, but does not encode its
    /// coordinate space in the property itself; the conversion report keeps
    /// both direct and actor-transformed comparisons.
    pub reported_bounds: Option<([f32; 3], [f32; 3])>,
    pub actor_transform: Matrix4,
    pub transform: Matrix4,
}

pub fn placements(package: &Package) -> Result<Vec<StaticMeshPlacement>> {
    let mut placements = Vec::new();
    for component in &package.exports {
        if component.class_name != "StaticMeshComponent" {
            continue;
        }
        let component_properties = package
            .properties(component)
            .with_context(|| format!("decoding component {}", component.object_name))?;
        let Some(mesh_index) = component_properties
            .iter()
            .find(|property| property.name == "StaticMesh")
            .map(|property| package.property_object(property))
            .transpose()?
            .flatten()
        else {
            // The class-default component owns no render asset.
            continue;
        };
        if mesh_index == 0 {
            continue;
        }
        let owner = package.export(component.outer_index).with_context(|| {
            format!(
                "{} has non-export owner {}",
                component.object_name, component.outer_index
            )
        })?;
        let actor_properties = package
            .properties(owner)
            .with_context(|| format!("decoding actor {}", owner.object_name))?;
        let actor_transform = actor_matrix(package, &actor_properties);
        let absolute_translation =
            boolean(&component_properties, "AbsoluteTranslation").unwrap_or(false);
        let absolute_rotation = boolean(&component_properties, "AbsoluteRotation").unwrap_or(false);
        let absolute_scale = boolean(&component_properties, "AbsoluteScale").unwrap_or(false);
        let component_transform = relative_component_matrix(package, &component_properties);
        let component_parent = apply_absolute_component_flags(
            actor_transform,
            absolute_translation,
            absolute_rotation,
            absolute_scale,
        );
        let transform = multiply(component_transform, component_parent);
        let material_overrides = component_properties
            .iter()
            .rev()
            .find(|property| property.name == "Materials")
            .map(|property| package.property_object_array(property))
            .transpose()?
            .flatten()
            .unwrap_or_default()
            .into_iter()
            .map(|index| (index != 0).then(|| package.object_path(index)).transpose())
            .collect::<Result<Vec<_>>>()?;
        let hidden = boolean(&actor_properties, "bHidden").unwrap_or(false)
            || boolean(&component_properties, "HiddenGame").unwrap_or(false);
        let actor_collision = boolean(&actor_properties, "bCollideActors").unwrap_or(true);
        let component_collision = boolean(&component_properties, "CollideActors").unwrap_or(true);
        let reported_bounds_property = actor_properties
            .iter()
            .rev()
            .find(|property| property.name == "CombinedBounds");
        let reported_bounds = reported_bounds_property
            .and_then(|property| package.property_box_sphere_bounds(property));
        placements.push(StaticMeshPlacement {
            actor: owner.object_name.clone(),
            actor_class: owner.class_name.clone(),
            component: component.object_name.clone(),
            mesh: package.object_path(mesh_index)?,
            mesh_index,
            local_mesh: mesh_index > 0,
            material_overrides,
            visible: !hidden,
            collision_enabled: actor_collision && component_collision,
            absolute_translation,
            absolute_rotation,
            absolute_scale,
            reported_bounds,
            actor_transform,
            transform,
        });
    }
    Ok(placements)
}

fn actor_matrix(package: &Package, properties: &[Property<'_>]) -> Matrix4 {
    let location = vector(package, properties, "Location").unwrap_or([0.0; 3]);
    let pre_pivot = vector(package, properties, "PrePivot").unwrap_or([0.0; 3]);
    let rotation = rotator(package, properties, "Rotation").unwrap_or([0; 3]);
    let uniform_scale = float(package, properties, "DrawScale").unwrap_or(1.0);
    let mut scale = vector(package, properties, "DrawScale3D").unwrap_or([1.0; 3]);
    for component in &mut scale {
        *component *= uniform_scale;
    }
    scale_rotation_translation(scale, rotation, location, pre_pivot)
}

fn relative_component_matrix(package: &Package, properties: &[Property<'_>]) -> Matrix4 {
    let location = vector(package, properties, "Translation").unwrap_or([0.0; 3]);
    let rotation = rotator(package, properties, "Rotation").unwrap_or([0; 3]);
    let uniform_scale = float(package, properties, "Scale").unwrap_or(1.0);
    let mut scale = vector(package, properties, "Scale3D").unwrap_or([1.0; 3]);
    for component in &mut scale {
        *component *= uniform_scale;
    }
    scale_rotation_translation(scale, rotation, location, [0.0; 3])
}

/// Apply the three absolute-transform flags exactly as
/// `UPrimitiveComponent::SetTransformedToWorld` does before multiplying the
/// component's relative SRT by its parent matrix.
///
/// These flags matter for Dirty Bomb's generated `SDStitchingProxy` actors:
/// some of their cooked components deliberately keep a world translation,
/// rotation, or scale independent of the owning actor. Applying the complete
/// actor transform to those components moves otherwise valid geometry tens of
/// metres outside the actor's serialized `CombinedBounds`.
fn apply_absolute_component_flags(
    mut parent: Matrix4,
    absolute_translation: bool,
    absolute_rotation: bool,
    absolute_scale: bool,
) -> Matrix4 {
    if absolute_translation {
        parent.rows[3][..3].fill(0.0);
    }

    if absolute_rotation || absolute_scale {
        let mut axes = [
            [parent.rows[0][0], parent.rows[0][1], parent.rows[0][2]],
            [parent.rows[1][0], parent.rows[1][1], parent.rows[1][2]],
            [parent.rows[2][0], parent.rows[2][1], parent.rows[2][2]],
        ];
        if absolute_scale {
            axes = axes.map(normalize);
        }
        if absolute_rotation {
            let lengths = axes.map(vector_length);
            axes = [
                [lengths[0], 0.0, 0.0],
                [0.0, lengths[1], 0.0],
                [0.0, 0.0, lengths[2]],
            ];
        }
        for (row, axis) in axes.into_iter().enumerate() {
            parent.rows[row][..3].copy_from_slice(&axis);
        }
    }

    parent
}

/// Byte-for-byte algebraic equivalent of `AActor::LocalToWorld` in AActor.h.
fn scale_rotation_translation(
    scale: [f32; 3],
    rotation: [i32; 3],
    location: [f32; 3],
    pre_pivot: [f32; 3],
) -> Matrix4 {
    let angle = |units: i32| (units as f32) * std::f32::consts::TAU / 65_536.0;
    let (sp, cp) = angle(rotation[0]).sin_cos();
    let (sy, cy) = angle(rotation[1]).sin_cos();
    let (sr, cr) = angle(rotation[2]).sin_cos();
    let [dx, dy, dz] = scale;
    let [lx, ly, lz] = location;
    let [px, py, pz] = pre_pivot;
    Matrix4 {
        rows: [
            [cp * cy * dx, cp * dx * sy, dx * sp, 0.0],
            [
                dy * (cy * sp * sr - cr * sy),
                dy * (cr * cy + sp * sr * sy),
                -cp * dy * sr,
                0.0,
            ],
            [
                -dz * (cr * cy * sp + sr * sy),
                dz * (cy * sr - cr * sp * sy),
                cp * cr * dz,
                0.0,
            ],
            [
                lx - cp * cy * dx * px + cr * cy * dz * pz * sp - cy * dy * py * sp * sr
                    + cr * dy * py * sy
                    + dz * pz * sr * sy,
                ly - (cr * cy * dy * py + cy * dz * pz * sr + cp * dx * px * sy
                    - cr * dz * pz * sp * sy
                    + dy * py * sp * sr * sy),
                lz - (cp * cr * dz * pz + dx * px * sp - cp * dy * py * sr),
                1.0,
            ],
        ],
    }
}

fn multiply(left: Matrix4, right: Matrix4) -> Matrix4 {
    let mut rows = [[0.0; 4]; 4];
    for (row, values) in rows.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = (0..4)
                .map(|middle| left.rows[row][middle] * right.rows[middle][column])
                .sum();
        }
    }
    Matrix4 { rows }
}

fn vector(package: &Package, properties: &[Property<'_>], name: &str) -> Option<[f32; 3]> {
    properties
        .iter()
        .rev()
        .find(|property| property.name == name)
        .and_then(|property| package.property_vector(property))
}

fn rotator(package: &Package, properties: &[Property<'_>], name: &str) -> Option<[i32; 3]> {
    properties
        .iter()
        .rev()
        .find(|property| property.name == name)
        .and_then(|property| package.property_rotator(property))
}

fn float(package: &Package, properties: &[Property<'_>], name: &str) -> Option<f32> {
    properties
        .iter()
        .rev()
        .find(|property| property.name == name)
        .and_then(|property| package.property_float(property))
}

fn boolean(properties: &[Property<'_>], name: &str) -> Option<bool> {
    properties
        .iter()
        .rev()
        .find(|property| property.name == name)
        .and_then(|property| property.bool_value)
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = vector_length(vector);
    if length > f32::EPSILON {
        vector.map(|component| component / length)
    } else {
        vector
    }
}

fn vector_length(vector: [f32; 3]) -> f32 {
    (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_actor_matrix_keeps_points() {
        let matrix = scale_rotation_translation([1.0; 3], [0; 3], [0.0; 3], [0.0; 3]);
        assert_eq!(matrix.transform_point([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn determinant_identifies_mirrored_scale() {
        let regular = scale_rotation_translation([1.0; 3], [0; 3], [0.0; 3], [0.0; 3]);
        let mirrored = scale_rotation_translation([-1.0, 1.0, 1.0], [0; 3], [0.0; 3], [0.0; 3]);
        assert!(regular.determinant3() > 0.0);
        assert!(mirrored.determinant3() < 0.0);
    }

    #[test]
    fn pre_pivot_is_applied_before_scale_and_translation() {
        let matrix =
            scale_rotation_translation([2.0; 3], [0; 3], [10.0, 20.0, 30.0], [1.0, 2.0, 3.0]);
        assert_eq!(matrix.transform_point([1.0, 2.0, 3.0]), [10.0, 20.0, 30.0]);
    }

    #[test]
    fn absolute_component_flags_match_ue3_parent_filtering() {
        let parent = Matrix4 {
            rows: [
                [0.0, 2.0, 0.0, 0.0],
                [-3.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 4.0, 0.0],
                [10.0, 20.0, 30.0, 1.0],
            ],
        };

        let translation = apply_absolute_component_flags(parent, true, false, false);
        assert_eq!(translation.rows[3], [0.0, 0.0, 0.0, 1.0]);

        let scale = apply_absolute_component_flags(parent, false, false, true);
        assert_eq!(scale.rows[0], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(scale.rows[1], [-1.0, 0.0, 0.0, 0.0]);
        assert_eq!(scale.rows[2], [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(scale.rows[3], parent.rows[3]);

        let rotation = apply_absolute_component_flags(parent, false, true, false);
        assert_eq!(rotation.rows[0], [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(rotation.rows[1], [0.0, 3.0, 0.0, 0.0]);
        assert_eq!(rotation.rows[2], [0.0, 0.0, 4.0, 0.0]);

        let all = apply_absolute_component_flags(parent, true, true, true);
        assert_eq!(
            all.rows,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
    }
}
