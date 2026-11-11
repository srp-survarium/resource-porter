// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a module baked into a gun lands, by the engine's own frame
//! arithmetic.
//!
//! Vostok's matrices act on row vectors (`math_float4x4_inline.h`): a point
//! goes through `A * B` as through A, then through B.
//!
//! * A bone's bind frame is `create_rotation(rotation) *
//!   create_translation(translation)` (`animation::frame_matrix`; scale 1).
//! * A locator's frame in its bone's is `create_rotation(rotation) *
//!   create_translation(position)` (`render_model::load_properties`).
//! * 0.69d0 draws a module at `locator * bone`, and a module is modelled
//!   half a turn about Y from its socket's frame: 0.69d0's weapon rigs bind
//!   their bones half a turn about Y from the mesh, and a module is modelled
//!   in the mesh's directions (muzzle along +Z).
//!
//! So a module's vertex `v` lands in the gun's mesh space at
//! `(v * H) * locator * bone`, with `H = diag(-1, 1, -1)`.
//!
//! Established on 0.69d0's own files (2026-11-10):
//!
//! * The VSK-94's `offset` bone is bound at (179.154, 0.001, -179.652)
//!   degrees and its sockets are turned (0.846, 0.006, 0.348) degrees:
//!   `locator * bone` is the plain half turn on all three axes, to 1e-4
//!   degrees. With the locator's angles negated it is 1.83 degrees off.
//! * The PPSh's `mag_point` is turned -3.74 degrees about X. By this rule
//!   its 71-round drum's neck stands 0.4 mm behind the magazine well's front
//!   wall at every height, and its 30-round box fills the well to within
//!   0.4 mm of the rear wall. With the angle negated (skhid-srp's
//!   weapons-069@15) the drum's neck is 6 to 8 mm inside the front wall and
//!   9 to 10 mm short of the rear one.

use super::BindFrame;

/// The rows i, j, k of a rotation acting on row vectors.
type Rows = [[f64; 3]; 3];

/// Sine and cosine of an angle of the files. Their angles are f32: a half
/// turn is stored as f32 pi, whose sine is 8.7e-8. A sine or cosine within
/// 1e-6 of -1, 0 or 1 is that value, so a bone bound a plain half turn turns
/// a module exactly half a turn.
fn sine_cosine(angle: f32) -> (f64, f64) {
    let exact = |v: f64| {
        [-1.0, 0.0, 1.0]
            .into_iter()
            .find(|e| (v - e).abs() < 1e-6)
            .unwrap_or(v)
    };
    let (s, c) = f64::from(angle).sin_cos();
    (exact(s), exact(c))
}

/// The engine's `create_rotation( float3 const& angles )`.
fn create_rotation(angles: [f32; 3]) -> Rows {
    let [(xs, xc), (ys, yc), (zs, zc)] = angles.map(sine_cosine);
    [
        [yc * zc, -yc * zs, ys],
        [ys * xs * zc + xc * zs, -xs * ys * zs + xc * zc, -xs * yc],
        [-xc * ys * zc + xs * zs, xc * ys * zs + xs * zc, xc * yc],
    ]
}

/// `a * b`: a row vector goes through `a`, then through `b`.
fn product(a: &Rows, b: &Rows) -> Rows {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// A module's place in a gun's mesh space: its vertex `v` lands at
/// `rotation * v + origin` (column vectors, as
/// [`super::convert::static_069_to_skinned`] takes them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub rotation: [[f32; 3]; 3],
    pub origin: [f32; 3],
}

/// The place of a module at a locator (its rotation and position in its
/// bone's frame) of a bone bound at `bone`.
pub fn placement(
    locator_rotation: [f32; 3],
    locator_position: [f32; 3],
    bone: &BindFrame,
) -> Placement {
    let locator = create_rotation(locator_rotation);
    let bind = create_rotation(bone.rotation);
    // H * locator: H negates the x and z rows.
    let turned = [locator[0].map(|c| -c), locator[1], locator[2].map(|c| -c)];
    let rows = product(&turned, &bind);
    // The same turn on column vectors is the transpose. Adding 0.0 leaves no
    // negative zero.
    let rotation = std::array::from_fn(|r| std::array::from_fn(|c| (rows[c][r] + 0.0) as f32));
    // The locator's position goes through the bone's frame alone.
    let origin = std::array::from_fn(|k| {
        let turned: f64 = (0..3)
            .filter(|r| bind[*r][k] != 0.0)
            .map(|r| f64::from(locator_position[r]) * bind[r][k])
            .sum();
        (turned + f64::from(bone.translation[k])) as f32
    });
    Placement { rotation, origin }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0.69d0's weapon bones: half a turn about Y, written (-pi, 0, -pi).
    const HALF_TURN: [f32; 3] = [-std::f32::consts::PI, 0.0, -std::f32::consts::PI];

    fn apply(p: &Placement, v: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| {
            p.rotation[r][0] * v[0]
                + p.rotation[r][1] * v[1]
                + p.rotation[r][2] * v[2]
                + p.origin[r]
        })
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn an_unturned_socket_of_a_half_turned_bone_moves_a_module_by_its_locator_alone() {
        let bone = BindFrame {
            translation: [0.0, -0.0295, 0.2518],
            rotation: HALF_TURN,
        };
        let p = placement([0.0; 3], [0.01, -0.02, -0.03], &bone);
        assert_eq!(
            p.rotation,
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        // The locator's x and z are the bone's, half a turn from the mesh's.
        assert_eq!(p.origin, [-0.01, -0.0295 + -0.02, 0.2518 + 0.03]);
    }

    #[test]
    fn the_ppsh_magazine_well_leans_its_magazine_to_the_butt_at_the_top() {
        // 0.69d0's models/weapons/smg/ppsh: mag_point on the bone magazin.
        let bone = BindFrame {
            translation: [0.0, -0.0295, 0.2518],
            rotation: HALF_TURN,
        };
        let p = placement(
            [-0.065_265_5, 0.0, 0.0],
            [0.0, -0.019_640_053, -0.021_128_776],
            &bone,
        );
        let (s, c) = 0.065_265_5_f32.sin_cos();
        assert_eq!(p.rotation[0], [1.0, 0.0, 0.0]);
        assert!(
            close(p.rotation[1][1], c) && close(p.rotation[1][2], s),
            "{p:?}"
        );
        assert!(
            close(p.rotation[2][1], -s) && close(p.rotation[2][2], c),
            "{p:?}"
        );
        // 5 cm up the module is 3.3 mm nearer the butt (-z): the well's
        // walls lean so.
        let (foot, up) = (apply(&p, [0.0; 3]), apply(&p, [0.0, 0.05, 0.0]));
        assert!(close(up[2] - foot[2], -0.05 * s), "{foot:?} {up:?}");
        assert!(close(foot[1], -0.0295 - 0.019_640_053), "{foot:?}");
        assert!(close(foot[2], 0.2518 + 0.021_128_776), "{foot:?}");
    }

    #[test]
    fn a_socket_turn_that_undoes_its_bones_tilt_leaves_the_module_unturned() {
        // 0.69d0's models/weapons/sniper/vsk_94/vsk_94_camo: muzzle_point on
        // the bone offset, both a little off on every axis.
        let bone = BindFrame {
            translation: [-0.009, 0.021, 0.001],
            rotation: [3.126_827_2, 1.736_587_2e-5, -3.135_518_8],
        };
        let p = placement(
            [0.014_765_107, 0.000_107_129_1, 0.006_072_826],
            [-0.000_213_197_8, 0.040_955_573, -0.295_189_68],
            &bone,
        );
        for r in 0..3 {
            for c in 0..3 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((p.rotation[r][c] - want).abs() < 2e-6, "{p:?}");
            }
        }
    }

    #[test]
    fn a_placement_is_a_rigid_turn() {
        let bone = BindFrame {
            translation: [0.1, 0.2, 0.3],
            rotation: [2.9, 0.4, -3.0],
        };
        let r = placement([0.3, -1.2, 1.570_796_4], [0.0; 3], &bone).rotation;
        for a in 0..3 {
            for b in 0..3 {
                let dot: f32 = (0..3).map(|k| r[k][a] * r[k][b]).sum();
                let want = if a == b { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-6, "{r:?}");
            }
        }
        let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
        assert!((det - 1.0).abs() < 1e-6, "{det}");
    }
}
