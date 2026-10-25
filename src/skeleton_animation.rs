// SPDX-License-Identifier: GPL-3.0-or-later
//! Write a v0.100b skeleton animation.
//!
//! A clip is two resources. `<clip>.bones_names` is a binary config listing the
//! bones a clip drives, in channel order. `<clip>.b-spline` is a raw memory
//! image the engine fixes up in place:
//!
//! ```text
//! u16 bones_count, u8 event_channels_count, u8 animation_type
//! bones_count    * 0x48 bytes   -- nine pointer slots, overwritten on load
//! event_channels * 0x10 bytes   -- knots pointer, counts, type
//! per bone, per channel:  u32 knots_count, knots_count * (f32 knot, f32 value)
//! per event channel:      knots, domains, NUL-terminated name, padded to 4
//! ```
//!
//! The nine channels are translation x/y/z, rotation x/y/z as Euler angles, and
//! scale x/y/z, in that order (`animation/anim_track_common.h`).
//!
//! The `(knot, value)` pairs are **not** samples. They are the knot vector and
//! control points of a non-uniform cubic B-spline, which the engine converts to
//! per-span cubic polynomials on load. Authoring a clip therefore means solving
//! for control points whose curve passes through the poses we want, so
//! [`Channel::evaluate`] reimplements the engine's own construction
//! (`poly_curve_inline.h get_spline_params` and `create_in_place_internals`)
//! and [`Channel::fit`] inverts it. Fitting against the engine's basis rather
//! than a textbook one is the point: a clip is correct when *this* runtime
//! draws the pose we asked for.

use anyhow::{Context, Result, bail};

use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};

/// `channel_max`: translation xyz, rotation xyz, scale xyz.
pub const CHANNELS: usize = 9;

/// The engine reads four control points per span, so a curve needs at least
/// that many pairs plus the end knot.
const MIN_PAIRS: usize = 5;

/// `bi_spline_bone_animation_baked`: nine platform pointers.
const BONE_RECORD_BYTES: usize = 0x48;

/// `poly_curve_inline.h bi_spline_knots_resolution_eps`.
const KNOT_EPS: f32 = 0.0000001;

/// The value retail writes in the final, unreferenced pair.
///
/// The engine builds spans only up to `points_number() - 2`, so the last
/// control point is never read; only its knot, which ends the curve.
const UNUSED_POINT: f32 = -1_000_000_000.0;

/// One scalar channel: a knot vector and its control points, stored as pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub knots: Vec<f32>,
    pub points: Vec<f32>,
}

impl Channel {
    /// A channel that holds one value for the whole clip.
    ///
    /// Written the way retail writes a bone that never moves: four control
    /// points at the start knot, then the end knot carrying the unused value.
    pub fn constant(value: f32, end: f32) -> Self {
        Self {
            knots: vec![0.0, 0.0, 0.0, 0.0, end],
            points: vec![value, value, value, value, UNUSED_POINT],
        }
    }

    pub fn pairs(&self) -> usize {
        self.knots.len()
    }

    /// `poly_curve::evaluate`: find the span holding `time`, then evaluate that
    /// span's cubic in local parameter.
    pub fn evaluate(&self, time: f32) -> f32 {
        let spans = self.spans();
        if spans.is_empty() {
            return self.points.first().copied().unwrap_or_default();
        }
        let first = spans[0].0;
        let last = *self.knots.last().expect("a channel has knots");
        let time = time.clamp(first, last);
        let span = spans
            .iter()
            .rev()
            .find(|(start, _)| *start <= time)
            .unwrap_or(&spans[0]);
        let local = time - span.0;
        let c = span.1;
        local * (local * (local * c[3] + c[2]) + c[1]) + c[0]
    }

    /// Every span the engine would build, as `(start knot, cubic coefficients)`.
    ///
    /// Mirrors `create_in_place_internals`: spans run from index 3 to the
    /// second-to-last pair, and a repeated knot produces no span at all.
    fn spans(&self) -> Vec<(f32, [f32; 4])> {
        let mut spans = Vec::new();
        if self.knots.len() < MIN_PAIRS {
            return spans;
        }
        for index in 3..self.knots.len() - 1 {
            let start = self.knots[index];
            let end = self.knots[index + 1];
            if end - start < KNOT_EPS {
                continue;
            }
            spans.push((start, self.span_coefficients(index)));
        }
        spans
    }

    /// `get_spline_params`, transcribed.
    fn span_coefficients(&self, index: usize) -> [f32; 4] {
        let dt = [
            self.knot(index as isize - 1) - self.knot(index as isize - 2),
            self.knot(index as isize) - self.knot(index as isize - 1),
            self.knot(index as isize + 1) - self.knot(index as isize),
            self.knot(index as isize + 2) - self.knot(index as isize + 1),
            self.knot(index as isize + 3) - self.knot(index as isize + 2),
        ];
        let (dm1, d0, d1, d2, d3) = (dt[0], dt[1], dt[2], dt[3], dt[4]);

        let k0 = 1.0 / (d1 * (d1 + d0) * (d1 + d0 + dm1));
        let k10 = k0;
        let k11 = 1.0 / (d1 * (d1 + d0) * (d1 + d0 + d2));
        let k12 = 1.0 / (d1 * (d1 + d2) * (d1 + d0 + d2));
        let k20 = k11;
        let k21 = k12;
        let k22 = 1.0 / (d1 * (d1 + d2) * (d1 + d2 + d3));
        let k3 = k22;

        let factors = [
            [k3, -k20 - k21 - k22, k10 + k11 + k12, -k0],
            [
                0.0,
                k20 * (d1 - 2.0 * d0) + k21 * (d1 + d2 - d0) + k22 * (d1 + d2 + d3),
                k10 * (dm1 + d0 - 2.0 * d1)
                    + k11 * (-2.0 * d1 - d2 + d0)
                    + k12 * (-2.0 * (d1 + d2)),
                k0 * 3.0 * d1,
            ],
            [
                0.0,
                k20 * (2.0 * d0 * d1 - d0 * d0) + k21 * (d0 * (d1 + d2)),
                k10 * (d1 * d1 - 2.0 * d1 * (dm1 + d0))
                    + k11 * (-d0 * d1 + (d1 - d0) * (d1 + d2))
                    + k12 * ((d1 + d2) * (d1 + d2)),
                -k0 * 3.0 * d1 * d1,
            ],
            [
                0.0,
                k20 * (d1 * d0 * d0),
                k10 * (d1 * d1 * (dm1 + d0)) + k11 * (d1 * d0 * (d1 + d2)),
                k0 * d1 * d1 * d1,
            ],
        ];

        let p = [
            self.point(index as isize),
            self.point(index as isize - 1),
            self.point(index as isize - 2),
            self.point(index as isize - 3),
        ];
        let combine =
            |row: &[f32; 4]| row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3] * p[3];
        [
            combine(&factors[3]),
            combine(&factors[2]),
            combine(&factors[1]),
            combine(&factors[0]),
        ]
    }

    /// `bi_spline_channel_animation_baked::knot`, including its clamp: any
    /// index past the end — which a negative index becomes when the engine
    /// reads it as unsigned — returns the last entry.
    fn knot(&self, index: isize) -> f32 {
        let last = self.knots.len() - 1;
        let index = if index < 0 || index as usize > last {
            last
        } else {
            index as usize
        };
        self.knots[index]
    }

    fn point(&self, index: isize) -> f32 {
        let last = self.points.len() - 1;
        let index = if index < 0 || index as usize > last {
            last
        } else {
            index as usize
        };
        self.points[index]
    }

    /// Solve for control points whose curve passes through `values` at `times`.
    ///
    /// The curve is linear in its control points, so the basis is recovered by
    /// evaluating the engine's own construction once per control point and the
    /// fit is an ordinary least-squares solve. A small ridge term keeps the
    /// system well conditioned: the layout carries one more control point than
    /// there are samples, so without it the extra degree of freedom is
    /// unconstrained.
    pub fn fit(times: &[f32], values: &[f32]) -> Result<Self> {
        if times.len() != values.len() {
            bail!(
                "fitting a channel needs one value per time, got {} and {}",
                times.len(),
                values.len()
            );
        }
        if times.len() < 2 {
            let value = values.first().copied().unwrap_or_default();
            let end = times.first().copied().unwrap_or_default();
            return Ok(Self::constant(value, end.max(1.0)));
        }
        if values.windows(2).all(|pair| pair[0] == pair[1]) {
            return Ok(Self::constant(values[0], *times.last().expect("two times")));
        }

        // Knots: the start repeated four times so the curve begins there, then
        // one knot per interior sample, then the final time.
        let mut knots = vec![times[0]; 4];
        knots.extend(times[1..].iter().copied());
        let pairs = knots.len();
        let unknowns = pairs - 1;

        // Basis: column j is the curve produced by a unit control point j.
        let mut basis = vec![vec![0.0_f64; unknowns]; times.len()];
        for column in 0..unknowns {
            let mut points = vec![0.0_f32; pairs];
            points[column] = 1.0;
            points[pairs - 1] = 0.0;
            let probe = Channel {
                knots: knots.clone(),
                points,
            };
            for (row, time) in times.iter().enumerate() {
                basis[row][column] = f64::from(probe.evaluate(*time));
            }
        }

        let solved = least_squares(&basis, values, 1e-6)?;
        let mut points: Vec<f32> = solved.into_iter().map(|value| value as f32).collect();
        points.push(UNUSED_POINT);
        Ok(Self { knots, points })
    }

    /// The largest error between this curve and the samples it was fitted to.
    pub fn residual(&self, times: &[f32], values: &[f32]) -> f32 {
        times
            .iter()
            .zip(values)
            .map(|(time, value)| (self.evaluate(*time) - value).abs())
            .fold(0.0_f32, f32::max)
    }
}

/// Solve `min ||A x - b||^2 + ridge * ||x||^2` by normal equations.
fn least_squares(a: &[Vec<f64>], b: &[f32], ridge: f64) -> Result<Vec<f64>> {
    let columns = a.first().map(Vec::len).unwrap_or_default();
    if columns == 0 {
        bail!("least squares needs at least one unknown");
    }
    let mut normal = vec![vec![0.0_f64; columns + 1]; columns];
    for row in 0..columns {
        for column in 0..columns {
            let mut sum = 0.0;
            for equation in a {
                sum += equation[row] * equation[column];
            }
            normal[row][column] = sum;
        }
        normal[row][row] += ridge;
        let mut sum = 0.0;
        for (equation, value) in a.iter().zip(b) {
            sum += equation[row] * f64::from(*value);
        }
        normal[row][columns] = sum;
    }

    // Gaussian elimination with partial pivoting.
    for pivot in 0..columns {
        let best = (pivot..columns)
            .max_by(|left, right| {
                normal[*left][pivot]
                    .abs()
                    .total_cmp(&normal[*right][pivot].abs())
            })
            .expect("a pivot row");
        normal.swap(pivot, best);
        let head = normal[pivot][pivot];
        if head.abs() < 1e-12 {
            bail!("the channel fit is singular at unknown {pivot}");
        }
        for row in pivot + 1..columns {
            let factor = normal[row][pivot] / head;
            if factor == 0.0 {
                continue;
            }
            for column in pivot..=columns {
                normal[row][column] -= factor * normal[pivot][column];
            }
        }
    }
    let mut solved = vec![0.0_f64; columns];
    for row in (0..columns).rev() {
        let mut value = normal[row][columns];
        for column in row + 1..columns {
            value -= normal[row][column] * solved[column];
        }
        solved[row] = value / normal[row][row];
    }
    Ok(solved)
}

/// One bone's nine channels.
#[derive(Clone, Debug)]
pub struct BoneTracks {
    pub channels: [Channel; CHANNELS],
}

impl BoneTracks {
    /// A bone that holds its rest frame for the whole clip.
    pub fn rest(translation: [f32; 3], rotation: [f32; 3], end: f32) -> Self {
        Self {
            channels: [
                Channel::constant(translation[0], end),
                Channel::constant(translation[1], end),
                Channel::constant(translation[2], end),
                Channel::constant(rotation[0], end),
                Channel::constant(rotation[1], end),
                Channel::constant(rotation[2], end),
                Channel::constant(1.0, end),
                Channel::constant(1.0, end),
                Channel::constant(1.0, end),
            ],
        }
    }
}

/// A whole clip: the bones it drives and their curves.
#[derive(Clone, Debug)]
pub struct Clip {
    /// Channel order, `object_movement` first, exactly as `.bones_names` lists.
    pub bones: Vec<String>,
    pub tracks: Vec<BoneTracks>,
    /// `animation_type`; retail player clips use 0.
    pub animation_type: u8,
}

impl Clip {
    /// The `.b-spline` memory image.
    ///
    /// Event channels are emitted empty: they carry footstep sounds and
    /// interval marks, and a clip without them plays silently rather than
    /// wrongly.
    pub fn to_bspline(&self) -> Result<Vec<u8>> {
        if self.bones.len() != self.tracks.len() {
            bail!(
                "clip names {} bones but carries {} tracks",
                self.bones.len(),
                self.tracks.len()
            );
        }
        let bones = u16::try_from(self.bones.len())?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&bones.to_le_bytes());
        bytes.push(0); // event channels
        bytes.push(self.animation_type);
        bytes.extend(std::iter::repeat_n(
            0u8,
            self.tracks.len() * BONE_RECORD_BYTES,
        ));
        for track in &self.tracks {
            for channel in &track.channels {
                if channel.knots.len() != channel.points.len() {
                    bail!(
                        "a channel has {} knots and {} points",
                        channel.knots.len(),
                        channel.points.len()
                    );
                }
                if channel.pairs() < MIN_PAIRS {
                    bail!(
                        "a channel has {} pairs; the engine reads four control points per span",
                        channel.pairs()
                    );
                }
                bytes.extend_from_slice(&u32::try_from(channel.pairs())?.to_le_bytes());
                for (knot, point) in channel.knots.iter().zip(&channel.points) {
                    bytes.extend_from_slice(&knot.to_le_bytes());
                    bytes.extend_from_slice(&point.to_le_bytes());
                }
            }
        }
        Ok(bytes)
    }

    /// Reads a clip back from its `.b-spline` image and `.bones_names`, the
    /// layout [`Self::to_bspline`] writes (retail clips use it too). Event
    /// channels, which follow the bone curves, are skipped.
    pub fn parse(bspline: &[u8], bones_names: &[u8]) -> Result<Self> {
        let config = BinaryConfig::parse(bones_names)?;
        let Some(ConfigValue::Indexed(names)) = config.root.named("bones_names") else {
            bail!("bones_names lists no bones");
        };
        let bones: Vec<String> = names
            .iter()
            .map(|name| match name {
                ConfigValue::String(name) => Ok(name.clone()),
                _ => bail!("bones_names holds a non-string"),
            })
            .collect::<Result<_>>()?;
        let header = bspline
            .get(..4)
            .context("b-spline shorter than its header")?;
        let count = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let events = usize::from(header[2]);
        let animation_type = header[3];
        if count != bones.len() {
            bail!(
                "b-spline drives {count} bones; bones_names lists {}",
                bones.len()
            );
        }
        let mut at = 4 + count * BONE_RECORD_BYTES + events * 0x10;
        let read_u32 = |at: &mut usize| -> Result<u32> {
            let bytes = bspline.get(*at..*at + 4).context("truncated b-spline")?;
            *at += 4;
            Ok(u32::from_le_bytes(bytes.try_into()?))
        };
        let mut tracks = Vec::with_capacity(count);
        for bone in &bones {
            let mut channels = Vec::with_capacity(CHANNELS);
            for _ in 0..CHANNELS {
                let pairs = read_u32(&mut at)? as usize;
                let bytes = bspline
                    .get(at..at + pairs * 8)
                    .with_context(|| format!("truncated curve of {bone}"))?;
                at += pairs * 8;
                let value = |k: usize| f32::from_le_bytes(bytes[k..k + 4].try_into().unwrap());
                channels.push(Channel {
                    knots: (0..pairs).map(|i| value(i * 8)).collect(),
                    points: (0..pairs).map(|i| value(i * 8 + 4)).collect(),
                });
            }
            tracks.push(BoneTracks {
                channels: channels.try_into().expect("nine channels"),
            });
        }
        if events == 0 && at != bspline.len() {
            bail!("b-spline has {} bytes after its curves", bspline.len() - at);
        }
        Ok(Self {
            bones,
            tracks,
            animation_type,
        })
    }

    /// The clip's length: its longest curve's end knot.
    pub fn duration(&self) -> f32 {
        self.tracks
            .iter()
            .flat_map(|track| &track.channels)
            .filter_map(|channel| channel.knots.last().copied())
            .fold(0.0, f32::max)
    }

    /// The `.bones_names` binary config.
    pub fn to_bones_names(&self) -> Result<Vec<u8>> {
        let config = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "bones_names".to_owned(),
                value: ConfigValue::Indexed(
                    self.bones
                        .iter()
                        .map(|bone| ConfigValue::String(bone.clone()))
                        .collect(),
                ),
            }]),
        };
        config.to_bytes()
    }
}

/// The root-motion channel a clip carries ahead of the rig's own bones. It
/// appears in a `.skeleton` as the outermost name but has no bind frame.
pub const ROOT_MOTION_CHANNEL: &str = "object_movement";

/// A 3x3 rotation in the engine's row-vector convention: a row vector is
/// multiplied on the left, so `multiply(a, b)` applies `a` first.
pub type Basis = [[f32; 3]; 3];

/// A bone's model-space frame: its basis and translation.
pub type Frame = (Basis, [f32; 3]);

pub const IDENTITY_BASIS: Basis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// `math_float4x4_inline.h create_rotation(float3 const& angles)`.
pub fn euler_to_basis(angles: [f32; 3]) -> Basis {
    let (sx, cx) = (angles[0].sin(), angles[0].cos());
    let (sy, cy) = (angles[1].sin(), angles[1].cos());
    let (sz, cz) = (angles[2].sin(), angles[2].cos());
    [
        [cy * cz, -cy * sz, sy],
        [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy],
        [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy],
    ]
}

/// `math_quaternion_inline.h quaternion(float3 angles)`, as `[x, y, z, w]`.
fn engine_quaternion(angles: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = angles.map(|a| (a * 0.5).sin_cos());
    let (xs_yc, xc_ys) = (x.0 * y.1, x.1 * y.0);
    let (xc_yc, xs_ys) = (x.1 * y.1, x.0 * y.0);
    [
        -xs_yc * z.1 - xc_ys * z.0,
        -xc_ys * z.1 + xs_yc * z.0,
        -xc_yc * z.0 - xs_ys * z.1,
        -xc_yc * z.1 + xs_ys * z.0,
    ]
}

/// `math_quaternion_inline.h operator *`.
fn hamilton(l: [f32; 4], r: [f32; 4]) -> [f32; 4] {
    let ([lx, ly, lz, lw], [rx, ry, rz, rw]) = (l, r);
    [
        lw * rx + lx * rw + ly * rz - lz * ry,
        lw * ry - lx * rz + ly * rw + lz * rx,
        lw * rz + lx * ry - ly * rx + lz * rw,
        lw * rw - lx * rx - ly * ry - lz * rz,
    ]
}

/// `math_float4x4_inline.h create_matrix(quaternion, 0)`'s rotation.
fn quaternion_basis([x, y, z, w]: [f32; 4]) -> Basis {
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
    ]
}

/// The inverse of [`euler_to_basis`], reading the angles back out of the basis.
///
/// At a gimbal lock the Z angle is unrecoverable, so it is pinned to zero and
/// the X angle absorbs the rotation — the same choice the engine's own
/// `get_angles_xyz` makes.
pub fn basis_to_euler(basis: &Basis) -> [f32; 3] {
    let sy = basis[0][2].clamp(-1.0, 1.0);
    let y = sy.asin();
    let cy = (1.0 - sy * sy).sqrt();
    if cy < 1e-6 {
        let x = basis[1][0].atan2(basis[1][1]);
        return [x, y, 0.0];
    }
    let x = (-basis[1][2]).atan2(basis[2][2]);
    let z = (-basis[0][1]).atan2(basis[0][0]);
    [x, y, z]
}

/// `a` applied first, then `b`.
pub fn multiply(a: &Basis, b: &Basis) -> Basis {
    let mut out = [[0.0_f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            out[row][column] =
                a[row][0] * b[0][column] + a[row][1] * b[1][column] + a[row][2] * b[2][column];
        }
    }
    out
}

/// The inverse of a rotation.
pub fn transpose(basis: &Basis) -> Basis {
    let mut out = [[0.0_f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            out[row][column] = basis[column][row];
        }
    }
    out
}

/// A row vector times a basis.
pub fn transform(vector: [f32; 3], basis: &Basis) -> [f32; 3] {
    let mut out = [0.0_f32; 3];
    for axis in 0..3 {
        out[axis] =
            vector[0] * basis[0][axis] + vector[1] * basis[1][axis] + vector[2] * basis[2][axis];
    }
    out
}

/// One bone of a target rig.
#[derive(Clone, Debug, PartialEq)]
pub struct RigBone {
    pub name: String,
    /// Index into [`Rig::bones`], or `None` for the rig's root.
    pub parent: Option<usize>,
    /// Model-space rest position, straight out of `render/bind_pose`.
    pub rest_translation: [f32; 3],
    /// Model-space rest orientation as engine Euler angles.
    pub rest_rotation: [f32; 3],
}

/// A target character rig: the hierarchy from a `.skeleton` and the rest pose
/// from a model's `render/bind_pose`.
///
/// The two resources describe the same skeleton from different angles — the
/// skeleton nests bone names to give parentage and a blend mask, the bind pose
/// lists model-space frames in the order a model indexes them — and a retarget
/// needs both.
#[derive(Clone, Debug)]
pub struct Rig {
    pub bones: Vec<RigBone>,
}

impl Rig {
    pub fn parse(bind_pose: &[u8], skeleton: &[u8]) -> Result<Self> {
        let frames = parse_bind_pose(bind_pose)?;
        let parents = parse_skeleton_parents(skeleton)?;
        let mut bones = Vec::with_capacity(frames.len());
        for (name, translation, rotation) in &frames {
            let parent = match parents.get(name) {
                // The skeleton hangs everything off the root-motion channel,
                // which is not a bone and carries no bind frame: a bone
                // parented to it is a root as far as the rest pose goes.
                Some(Some(parent)) if parent == ROOT_MOTION_CHANNEL => None,
                Some(Some(parent)) => Some(
                    frames
                        .iter()
                        .position(|(candidate, _, _)| candidate == parent)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "skeleton parents {name} to {parent}, which the bind pose lacks"
                            )
                        })?,
                ),
                Some(None) => None,
                None => bail!("the skeleton does not name bind-pose bone {name}"),
            };
            bones.push(RigBone {
                name: name.clone(),
                parent,
                rest_translation: *translation,
                rest_rotation: *rotation,
            });
        }
        Ok(Self { bones })
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|bone| bone.name == name)
    }

    pub fn rest_basis(&self, index: usize) -> Basis {
        euler_to_basis(self.bones[index].rest_rotation)
    }

    /// Every bone's model-space frame `(basis, translation)` at `time` of
    /// `clip`, in bind-pose order. A bone the clip drives takes its local
    /// translation and angles from the curves (scale is ignored); any other
    /// keeps its rest frame relative to its parent. Root motion
    /// (`object_movement`) is left out: the pose stands where the bind pose does.
    pub fn pose(&self, clip: &Clip, time: f32) -> Vec<Frame> {
        let mut frames: Vec<Option<Frame>> = vec![None; self.bones.len()];
        fn resolve(
            rig: &Rig,
            clip: &Clip,
            time: f32,
            index: usize,
            frames: &mut [Option<Frame>],
        ) -> Frame {
            if let Some(frame) = frames[index] {
                return frame;
            }
            let (translation, angles) = match clip
                .bones
                .iter()
                .position(|name| *name == rig.bones[index].name)
            {
                Some(track) => {
                    let c = &clip.tracks[track].channels;
                    (
                        [
                            c[0].evaluate(time),
                            c[1].evaluate(time),
                            c[2].evaluate(time),
                        ],
                        [
                            c[3].evaluate(time),
                            c[4].evaluate(time),
                            c[5].evaluate(time),
                        ],
                    )
                }
                None => rig.local_rest(index),
            };
            let local = euler_to_basis(angles);
            let frame = match rig.bones[index].parent {
                None => (local, translation),
                Some(parent) => {
                    let (parent_basis, parent_at) = resolve(rig, clip, time, parent, frames);
                    let offset = transform(translation, &parent_basis);
                    (
                        multiply(&local, &parent_basis),
                        [0, 1, 2].map(|k| offset[k] + parent_at[k]),
                    )
                }
            };
            frames[index] = Some(frame);
            frame
        }
        (0..self.bones.len())
            .map(|index| resolve(self, clip, time, index, &mut frames))
            .collect()
    }

    /// Like [`Self::pose`], with additive clips (`animation_type` 1: a
    /// third-person weapon hold's `hands_only` clips, the `_look` clips)
    /// layered on the full one as the engine's `bone_matrices_computer`
    /// does: per bone, translations add and rotations compose as
    /// quaternions, `base * layer` in order. A bone a layer does not drive
    /// is left as it is.
    pub fn pose_layered(&self, base: (&Clip, f32), layers: &[(&Clip, f32)]) -> Vec<Frame> {
        let sample = |clip: &Clip, time: f32, index: usize| {
            clip.bones
                .iter()
                .position(|name| *name == self.bones[index].name)
                .map(|track| {
                    let c = &clip.tracks[track].channels;
                    (
                        [0, 1, 2].map(|k| c[k].evaluate(time)),
                        [3, 4, 5].map(|k| c[k].evaluate(time)),
                    )
                })
        };
        let locals: Vec<([f32; 3], Basis)> = (0..self.bones.len())
            .map(|index| {
                let (mut translation, angles) =
                    sample(base.0, base.1, index).unwrap_or_else(|| self.local_rest(index));
                let mut rotation = engine_quaternion(angles);
                for (clip, time) in layers {
                    if let Some((t, a)) = sample(clip, *time, index) {
                        translation = [0, 1, 2].map(|k| translation[k] + t[k]);
                        rotation = hamilton(rotation, engine_quaternion(a));
                    }
                }
                (translation, quaternion_basis(rotation))
            })
            .collect();
        let mut frames: Vec<Option<Frame>> = vec![None; self.bones.len()];
        fn resolve(
            rig: &Rig,
            locals: &[([f32; 3], Basis)],
            index: usize,
            frames: &mut [Option<Frame>],
        ) -> Frame {
            if let Some(frame) = frames[index] {
                return frame;
            }
            let (translation, local) = locals[index];
            let frame = match rig.bones[index].parent {
                None => (local, translation),
                Some(parent) => {
                    let (parent_basis, parent_at) = resolve(rig, locals, parent, frames);
                    let offset = transform(translation, &parent_basis);
                    (
                        multiply(&local, &parent_basis),
                        [0, 1, 2].map(|k| offset[k] + parent_at[k]),
                    )
                }
            };
            frames[index] = Some(frame);
            frame
        }
        (0..self.bones.len())
            .map(|index| resolve(self, &locals, index, &mut frames))
            .collect()
    }

    /// Every bone's model-space rest frame, as [`Self::pose`] gives a pose.
    pub fn rest(&self) -> Vec<Frame> {
        (0..self.bones.len())
            .map(|index| (self.rest_basis(index), self.bones[index].rest_translation))
            .collect()
    }

    /// A bone's rest frame relative to its parent, as `(translation, angles)`.
    pub fn local_rest(&self, index: usize) -> ([f32; 3], [f32; 3]) {
        let bone = &self.bones[index];
        let Some(parent) = bone.parent else {
            return (bone.rest_translation, bone.rest_rotation);
        };
        let parent_basis = self.rest_basis(parent);
        let inverse = transpose(&parent_basis);
        let offset = [
            bone.rest_translation[0] - self.bones[parent].rest_translation[0],
            bone.rest_translation[1] - self.bones[parent].rest_translation[1],
            bone.rest_translation[2] - self.bones[parent].rest_translation[2],
        ];
        (
            transform(offset, &inverse),
            basis_to_euler(&multiply(&self.rest_basis(index), &inverse)),
        )
    }
}

/// `render/bind_pose`: a count, then per bone a name and a model-space frame.
fn parse_bind_pose(bytes: &[u8]) -> Result<Vec<(String, [f32; 3], [f32; 3])>> {
    if bytes.len() < 2 {
        bail!("bind pose is shorter than its bone count");
    }
    let count = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
    let mut offset = 2;
    let mut bones = Vec::with_capacity(count);
    for index in 0..count {
        let tail = bytes
            .get(offset..)
            .with_context(|| format!("bind pose is missing bone {index}"))?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("bind pose bone {index} has an unterminated name"))?;
        let name = std::str::from_utf8(&tail[..end])
            .with_context(|| format!("bind pose bone {index} has a non-UTF-8 name"))?
            .to_owned();
        offset += end + 1;
        let frame = bytes
            .get(offset..offset + 36)
            .with_context(|| format!("bind pose bone {index} has a truncated frame"))?;
        let read =
            |at: usize| f32::from_le_bytes(frame[at..at + 4].try_into().expect("four bytes"));
        let translation = [read(0), read(4), read(8)];
        let rotation = [read(12), read(16), read(20)];
        offset += 36;
        bones.push((name, translation, rotation));
    }
    if offset != bytes.len() {
        bail!(
            "bind pose has {} trailing bytes after {count} bones",
            bytes.len() - offset
        );
    }
    Ok(bones)
}

/// A `.skeleton` nests bone names as config tables, one level per generation,
/// with a `mask` leaf marking each bone. The nesting *is* the hierarchy.
fn parse_skeleton_parents(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, Option<String>>> {
    let config = BinaryConfig::parse(bytes)?;
    let mut parents = std::collections::BTreeMap::new();
    collect_skeleton_parents(&config.root, None, &mut parents);
    if parents.is_empty() {
        bail!("the skeleton names no bones");
    }
    Ok(parents)
}

fn collect_skeleton_parents(
    value: &ConfigValue,
    parent: Option<&str>,
    parents: &mut std::collections::BTreeMap<String, Option<String>>,
) {
    let ConfigValue::Named(entries) = value else {
        return;
    };
    for entry in entries {
        if entry.name == "mask" {
            continue;
        }
        parents
            .entry(entry.name.clone())
            .or_insert_with(|| parent.map(str::to_owned));
        collect_skeleton_parents(&entry.value, Some(&entry.name), parents);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's quaternion of a set of angles is the rotation its
    /// Euler matrix is, and the matrix of `a * b` applies `a`'s turn
    /// first, then `b`'s.
    #[test]
    fn engine_quaternions_match_the_euler_matrices() {
        let close =
            |a: &Basis, b: &Basis| (0..3).all(|i| (0..3).all(|j| (a[i][j] - b[i][j]).abs() < 1e-5));
        let a = [0.4, -1.1, 2.3];
        let b = [-0.7, 0.2, 0.9];
        let qa = engine_quaternion(a);
        assert!(close(&quaternion_basis(qa), &euler_to_basis(a)));
        let composed = quaternion_basis(hamilton(qa, engine_quaternion(b)));
        assert!(close(
            &composed,
            &multiply(&euler_to_basis(a), &euler_to_basis(b))
        ));
    }

    /// Retail writes a bone that never moves as four control points and an end
    /// knot; the curve it produces must be exactly that value everywhere.
    #[test]
    fn a_constant_channel_holds_its_value() {
        let channel = Channel::constant(0.25, 24.0);
        assert_eq!(channel.pairs(), 5);
        for time in [0.0, 1.0, 12.0, 23.9, 24.0] {
            assert!((channel.evaluate(time) - 0.25).abs() < 1e-6, "at {time}");
        }
    }

    /// The whole reason the fit exists: control points are not samples, so a
    /// curve built by pasting samples in as points would miss them.
    #[test]
    fn fitting_reproduces_the_samples_the_engine_will_read() {
        let times: Vec<f32> = (0..26).map(|frame| frame as f32).collect();
        let values: Vec<f32> = times
            .iter()
            .map(|time| (time * 0.24).sin() * 0.5 + 0.1 * time)
            .collect();

        let channel = Channel::fit(&times, &values).unwrap();
        let residual = channel.residual(&times, &values);
        assert!(residual < 1e-3, "residual {residual}");

        let pasted = Channel {
            knots: channel.knots.clone(),
            points: {
                let mut points = vec![values[0]; 3];
                points.extend(values.iter().copied());
                points.push(UNUSED_POINT);
                points
            },
        };
        assert!(
            pasted.residual(&times, &values) > residual,
            "pasting samples as control points should be worse"
        );
    }

    /// A clip whose values already sit on a straight line still has to come
    /// back exactly: the ridge term must not pull the curve off the samples.
    #[test]
    fn fitting_a_ramp_is_exact() {
        let times: Vec<f32> = (0..10).map(|frame| frame as f32).collect();
        let values: Vec<f32> = times.iter().map(|time| 2.0 * time - 1.0).collect();
        let channel = Channel::fit(&times, &values).unwrap();
        assert!(channel.residual(&times, &values) < 1e-4);
    }

    #[test]
    fn an_unchanging_track_collapses_to_a_constant() {
        let times: Vec<f32> = (0..26).map(|frame| frame as f32).collect();
        let values = vec![1.5_f32; 26];
        let channel = Channel::fit(&times, &values).unwrap();
        assert_eq!(channel.pairs(), 5);
        assert!((channel.evaluate(13.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn writes_the_baked_header_and_channel_stream() {
        let clip = Clip {
            bones: vec!["object_movement".to_owned(), "Root".to_owned()],
            tracks: vec![
                BoneTracks::rest([0.0; 3], [0.0; 3], 24.0),
                BoneTracks::rest([0.0, 1.0, 0.0], [0.0; 3], 24.0),
            ],
            animation_type: 0,
        };
        let bytes = clip.to_bspline().unwrap();

        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 2);
        assert_eq!(bytes[2], 0, "event channels");
        assert_eq!(bytes[3], 0, "animation type");

        // Header, two bone records, then 18 channels of five pairs each.
        let expected = 4 + 2 * BONE_RECORD_BYTES + 2 * CHANNELS * (4 + 5 * 8);
        assert_eq!(bytes.len(), expected);

        let first_channel = 4 + 2 * BONE_RECORD_BYTES;
        assert_eq!(
            u32::from_le_bytes(bytes[first_channel..first_channel + 4].try_into().unwrap()),
            5
        );
    }

    #[test]
    fn names_bones_in_channel_order() {
        let clip = Clip {
            bones: vec!["object_movement".to_owned(), "Hip".to_owned()],
            tracks: vec![
                BoneTracks::rest([0.0; 3], [0.0; 3], 1.0),
                BoneTracks::rest([0.0; 3], [0.0; 3], 1.0),
            ],
            animation_type: 0,
        };
        let bytes = clip.to_bones_names().unwrap();
        let parsed = BinaryConfig::parse(&bytes).unwrap();
        let mut strings = Vec::new();
        parsed.root.collect_strings(&mut strings);
        assert!(strings.contains(&"object_movement"));
        assert!(strings.contains(&"Hip"));
    }

    #[test]
    fn rejects_a_track_count_mismatch() {
        let clip = Clip {
            bones: vec!["object_movement".to_owned(), "Hip".to_owned()],
            tracks: vec![BoneTracks::rest([0.0; 3], [0.0; 3], 1.0)],
            animation_type: 0,
        };
        let error = clip.to_bspline().unwrap_err().to_string();
        assert!(error.contains("names 2 bones"), "{error}");
    }

    #[test]
    fn a_written_clip_reads_back() {
        let clip = Clip {
            bones: vec!["object_movement".to_owned(), "Hip".to_owned()],
            tracks: vec![
                BoneTracks::rest([0.0; 3], [0.0; 3], 2.0),
                BoneTracks::rest([0.1, 0.9, 0.0], [0.2, 0.0, -0.3], 2.0),
            ],
            animation_type: 0,
        };
        let parsed =
            Clip::parse(&clip.to_bspline().unwrap(), &clip.to_bones_names().unwrap()).unwrap();
        assert_eq!(parsed.bones, clip.bones);
        assert_eq!(parsed.duration(), 2.0);
        for (a, b) in parsed.tracks.iter().zip(&clip.tracks) {
            assert_eq!(a.channels, b.channels);
        }
        // A clip that names fewer bones than it drives is refused.
        let one = Clip {
            bones: vec!["Hip".to_owned()],
            ..clip.clone()
        };
        assert!(Clip::parse(&clip.to_bspline().unwrap(), &one.to_bones_names().unwrap()).is_err());
    }

    /// A clip that holds every bone at its local rest frame poses the rig in
    /// its bind pose: the pose composes local frames as `local_rest` splits them.
    #[test]
    fn a_rest_clip_poses_the_bind_pose() {
        let rig = Rig {
            bones: vec![
                RigBone {
                    name: "Hip".into(),
                    parent: None,
                    rest_translation: [0.0, 1.0, 0.0],
                    rest_rotation: [0.1, 0.2, 0.3],
                },
                RigBone {
                    name: "Spin".into(),
                    parent: Some(0),
                    rest_translation: [0.05, 1.2, 0.02],
                    rest_rotation: [-0.4, 0.5, 0.1],
                },
                RigBone {
                    name: "Head".into(),
                    parent: Some(1),
                    rest_translation: [0.0, 1.7, 0.1],
                    rest_rotation: [0.3, -0.2, 0.6],
                },
            ],
        };
        let clip = Clip {
            bones: vec!["Hip".into(), "Spin".into()],
            tracks: (0..2)
                .map(|i| {
                    let (t, r) = rig.local_rest(i);
                    BoneTracks::rest(t, r, 1.0)
                })
                .collect(),
            animation_type: 0,
        };
        // Head is not in the clip and keeps its rest frame under Spin.
        for (posed, rest) in rig.pose(&clip, 0.5).iter().zip(rig.rest()) {
            for k in 0..3 {
                assert!((posed.1[k] - rest.1[k]).abs() < 1e-5);
                for c in 0..3 {
                    assert!((posed.0[k][c] - rest.0[k][c]).abs() < 1e-5);
                }
            }
        }
    }
}
