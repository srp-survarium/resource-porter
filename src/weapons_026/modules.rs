// SPDX-License-Identifier: GPL-3.0-or-later
//! Attachable modules (the attachments track, release 41): scopes, flash
//! hiders, muzzle brakes and grips, each a static model and its item,
//! placed at the gun's locator of its kind. No suppressors (the owner,
//! 2026-11-01): [`ModuleKind`] has none, and [`check_module`] refuses one.
//!
//! The rules (`docs/weapons-026.md`, from the muzzle study of 2026-10-31):
//!
//! * **Placement:** a module's origin goes to the gun's locator of its kind
//!   (`muzzle_point`, `picatinny_grip_point`, `picatinny_scope_point`), the
//!   donor build's own; retail 0.69d0's where our mesh is retail's (0.26g's
//!   gun meshes are 0.69d0's base meshes plus the stock modules), so the
//!   offset between the donor's gun and ours is zero ([`placement`]).
//! * **A bare gun:** the device a 0.26g mesh has built in is cut by the
//!   removal rule: drop our triangles whose corners are all missing from the
//!   0.69d0 base mesh and whose centre lies inside the placed stock module
//!   ([`cut_builtin`]); fused devices (AK-12, vz. 58) and integral
//!   suppressors are left alone.
//! * **Verification:** a side view of the gun with the module placed
//!   ([`render_side`]), one a gun, to look at before a game does.
//!
//! The meshes are 0.26g's (`converted_model`: chunk 3 the vertices, each's
//! position first; chunk 4 the 16-bit indices), as a [`super::donor`] serves
//! them.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};

use super::manifest::{Module, ModuleKind};
use super::{field, s};
use crate::ConfigValue as V;

/// A mesh's positions and triangles, and what it takes to write it back.
#[derive(Clone, Debug)]
pub struct Mesh {
    pub stride: usize,
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u16; 3]>,
    vertex_chunk: Vec<u8>,
    tail: Vec<(u32, Vec<u8>)>,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated mesh")?.try_into()?,
    ))
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

impl Mesh {
    /// Parses a 0.26g `converted_model`.
    pub fn parse(b: &[u8]) -> Result<Self> {
        let (mut vertices, mut indices, mut tail) = (None, None, vec![]);
        let mut at = 0;
        while at < b.len() {
            let id = u32_at(b, at)?;
            let size = u32_at(b, at + 4)? as usize;
            let body = b.get(at + 8..at + 8 + size).context("truncated chunk")?;
            match id {
                3 => vertices = Some(body.to_vec()),
                4 => indices = Some(body.to_vec()),
                other => tail.push((other, body.to_vec())),
            }
            at += 8 + size;
        }
        let vertex_chunk = vertices.context("no vertex chunk")?;
        let n = u32_at(&vertex_chunk, 0)? as usize;
        ensure!(n > 0, "no vertices");
        let stride = (vertex_chunk.len() - 4) / n;
        ensure!(
            stride >= 12 && 4 + n * stride == vertex_chunk.len(),
            "vertex stride"
        );
        let positions = (0..n)
            .map(|i| {
                let at = 4 + i * stride;
                [
                    f32_at(&vertex_chunk, at),
                    f32_at(&vertex_chunk, at + 4),
                    f32_at(&vertex_chunk, at + 8),
                ]
            })
            .collect();
        let indices = indices.context("no index chunk")?;
        let count = u32_at(&indices, 0)? as usize;
        ensure!(
            count.is_multiple_of(3) && indices.len() >= 4 + 2 * count,
            "index chunk"
        );
        let index = |k: usize| u16::from_le_bytes([indices[4 + 2 * k], indices[5 + 2 * k]]);
        let triangles = (0..count / 3)
            .map(|t| [index(3 * t), index(3 * t + 1), index(3 * t + 2)])
            .collect();
        Ok(Self {
            stride,
            positions,
            triangles,
            vertex_chunk,
            tail,
        })
    }

    /// Vertex `i`'s bytes (its position first, then its layout's own
    /// fields).
    pub fn vertex(&self, i: usize) -> &[u8] {
        let at = 4 + i * self.stride;
        &self.vertex_chunk[at..at + self.stride]
    }

    /// The mesh with only `keep`'s triangles (the vertices stay, as does
    /// their skinning).
    pub fn with_triangles(&self, keep: &[[u16; 3]]) -> Vec<u8> {
        let mut out = vec![];
        let mut chunk = |id: u32, body: &[u8]| {
            out.extend(id.to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
        };
        chunk(3, &self.vertex_chunk);
        let mut idx = ((keep.len() * 3) as u32).to_le_bytes().to_vec();
        for t in keep {
            for i in t {
                idx.extend(i.to_le_bytes());
            }
        }
        chunk(4, &idx);
        for (id, body) in &self.tail {
            chunk(*id, body);
        }
        out
    }
}

/// A position rounded to 0.1 mm, to match two builds' vertices.
fn key(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c * 1e4).round() as i32)
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub fn of(points: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        Some(it.fold(
            Self {
                min: first,
                max: first,
            },
            |b, p| Self {
                min: [0, 1, 2].map(|k| b.min[k].min(p[k])),
                max: [0, 1, 2].map(|k| b.max[k].max(p[k])),
            },
        ))
    }
    pub fn grown(self, by: f32) -> Self {
        Self {
            min: self.min.map(|c| c - by),
            max: self.max.map(|c| c + by),
        }
    }
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|k| p[k] >= self.min[k] && p[k] <= self.max[k])
    }
}

/// Where a module's origin goes on our gun: the donor's locator of its
/// kind, moved by the difference of our gun's reference point from the
/// donor's (zero when our mesh is the donor's base: the 0.69d0 guns).
pub fn placement(donor_locator: [f32; 3], donor_gun: [f32; 3], our_gun: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| donor_locator[k] + our_gun[k] - donor_gun[k])
}

/// The module's positions at `at`.
pub fn placed(module: &Mesh, at: [f32; 3]) -> Vec<[f32; 3]> {
    module
        .positions
        .iter()
        .map(|p| [p[0] + at[0], p[1] + at[1], p[2] + at[2]])
        .collect()
}

/// The removal rule: our triangles whose corners the base mesh has none of
/// and whose centre lies inside the placed stock device's box (3 mm of
/// slack) are the built-in device; the rest is the bare gun. The kept
/// triangles and how many went.
pub fn cut_builtin(ours: &Mesh, base: &[[f32; 3]], device: &[[f32; 3]]) -> (Vec<[u16; 3]>, usize) {
    let known: BTreeSet<[i32; 3]> = base.iter().map(|p| key(*p)).collect();
    let Some(bounds) = Bounds::of(device.iter().copied()) else {
        return (ours.triangles.clone(), 0);
    };
    let bounds = bounds.grown(0.003);
    let absent = |i: u16| !known.contains(&key(ours.positions[i as usize]));
    let mut kept = vec![];
    let mut cut = 0;
    for t in &ours.triangles {
        let centre = [0, 1, 2].map(|k| {
            t.iter()
                .map(|&i| ours.positions[i as usize][k])
                .sum::<f32>()
                / 3.0
        });
        if t.iter().all(|&i| absent(i)) && bounds.contains(centre) {
            cut += 1;
        } else {
            kept.push(*t);
        }
    }
    (kept, cut)
}

/// One part of a render: positions, triangles, colour.
pub type Part<'a> = (&'a [[f32; 3]], &'a [[u16; 3]], [u8; 3]);

/// A side view (the gun's length, z, across; its height, y, up; the
/// viewer at +x) of `parts`, depth-tested and shaded by facing: an RGBA
/// image `width` wide, scaled to fit.
pub fn render_side(parts: &[Part], width: usize) -> Result<(usize, usize, Vec<u8>)> {
    let all = parts.iter().flat_map(|(p, _, _)| p.iter().copied());
    let b = Bounds::of(all).context("nothing to render")?;
    render_side_in(parts, width, b)
}

/// [`render_side`] of the box `b` alone (what lies outside is clipped): views
/// of one gun at one scale.
pub fn render_side_in(parts: &[Part], width: usize, b: Bounds) -> Result<(usize, usize, Vec<u8>)> {
    let span_x = (b.max[2] - b.min[2]).max(1e-4);
    let span_y = (b.max[1] - b.min[1]).max(1e-4);
    let scale = (width - 2) as f32 / span_x;
    let height = ((span_y * scale) as usize + 2).max(2);
    let mut rgba = vec![0u8; width * height * 4];
    let mut depth = vec![f32::INFINITY; width * height];
    for (positions, triangles, colour) in parts {
        for t in *triangles {
            let p = t.map(|i| positions[i as usize]);
            let q = p.map(|v| {
                [
                    (v[2] - b.min[2]) * scale + 1.0,
                    (b.max[1] - v[1]) * scale + 1.0,
                    v[0],
                ]
            });
            let (u, v) = (sub(p[1], p[0]), sub(p[2], p[0]));
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let light = if len > 0.0 {
                0.35 + 0.65 * (n[0] / len).abs()
            } else {
                0.35
            };
            let lo = |k: usize| q.iter().map(|v| v[k]).fold(f32::MAX, f32::min);
            let hi = |k: usize| q.iter().map(|v| v[k]).fold(f32::MIN, f32::max);
            let ys = lo(1).floor().max(0.0) as usize..=(hi(1).ceil() as usize).min(height - 1);
            for y in ys {
                let xs = lo(0).floor().max(0.0) as usize..=(hi(0).ceil() as usize).min(width - 1);
                for x in xs {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w = [
                        edge(q[1], q[2], px, py),
                        edge(q[2], q[0], px, py),
                        edge(q[0], q[1], px, py),
                    ];
                    let area = w[0] + w[1] + w[2];
                    let inside = w.iter().all(|e| *e >= 0.0) || w.iter().all(|e| *e <= 0.0);
                    if area.abs() < 1e-9 || !inside {
                        continue;
                    }
                    let near = -(w[0] * q[0][2] + w[1] * q[1][2] + w[2] * q[2][2]) / area;
                    let at = y * width + x;
                    if near < depth[at] {
                        depth[at] = near;
                        for c in 0..3 {
                            rgba[at * 4 + c] = (f32::from(colour[c]) * light) as u8;
                        }
                        rgba[at * 4 + 3] = 255;
                    }
                }
            }
        }
    }
    Ok((width, height, rgba))
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn edge(a: [f32; 3], b: [f32; 3], x: f32, y: f32) -> f32 {
    (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])
}

/// An RGBA image as a PNG.
pub fn png(width: usize, height: usize, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(rgba)?;
    }
    Ok(out)
}

/// The item kind (`parameters.type`) of a module of the kit: srp's own
/// names, as the AK-74M's brake (#766: `srp_muzl`).
pub fn item_type(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::Scope => "scope",
        ModuleKind::FlashHider | ModuleKind::MuzzleBrake => "srp_muzl",
        ModuleKind::Grip => "srp_grip",
    }
}

/// A module's item config: the native template of its kind, its model and
/// locator, the port's values. A scope's model is its idle and aimed view
/// (`data.idle_model`, `data.aimed_model`, as 0.26g's scopes); a muzzle
/// device's and a grip's are the kit's keys (`data.srp_<slot>_model`,
/// `data.srp_locator`), which the kit's patch reads.
pub fn module_config(module: &Module, template: &V) -> Result<V> {
    check_module(module)?;
    let mut cfg = template.clone();
    let model = s(&module.model);
    match module.kind {
        ModuleKind::Scope => {
            for key in ["data.idle_model", "data.aimed_model"] {
                ensure!(
                    super::get(&cfg, key).is_some(),
                    "the scope template has no {key}"
                );
                cfg.set_path(key, model.clone())?;
            }
        }
        kind => {
            let slot = if kind == ModuleKind::Grip {
                "grip"
            } else {
                "muzzle"
            };
            let locator = module.locator.as_deref().unwrap_or(kind.locator());
            let add = [
                field(&format!("srp_{slot}_model"), model),
                field("srp_locator", s(locator)),
            ];
            match cfg.named_mut("data") {
                Some(V::Named(fields)) => {
                    for f in add {
                        fields.retain(|x| x.name != f.name);
                        fields.push(f);
                    }
                }
                _ => cfg.set_path("data", V::Named(add.to_vec()))?,
            }
            if super::get(&cfg, "parameters.type").is_some() {
                cfg.set_path("parameters.type", s(item_type(kind)))?;
            }
        }
    }
    for (path, value) in &module.set {
        super::config::set_typed(&mut cfg, path, value)?;
    }
    Ok(cfg)
}

/// A module the kit may carry: a muzzle device is a flash hider or a brake,
/// never a suppressor (by its model's name, as the builds name theirs).
pub fn check_module(module: &Module) -> Result<()> {
    if is_suppressor(&module.donor_model) {
        bail!(
            "{}: suppressors are not ported (the owner, 2026-11-01)",
            module.name
        );
    }
    Ok(())
}

/// A model or module name that names a suppressor, as the builds name
/// theirs: never ported (the owner, 2026-11-01).
pub fn is_suppressor(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["silencer", "suppressor", "pbs", "glush"]
        .iter()
        .any(|w| name.contains(w))
}
