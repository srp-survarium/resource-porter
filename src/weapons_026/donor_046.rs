// SPDX-License-Identifier: GPL-3.0-or-later
//! Survarium 0.46c2 (April 2017) as a weapon donor for v0.26g.
//!
//! 0.46c2 is between 0.31a2 and 0.69d0: 36 `resources.NN.db` volumes (the
//! last ones its patches), every item config inside `gameplay/game.options`,
//! and every config in the later 16-byte value records; but its weapon
//! models are complete (no module system yet) and in 0.26g's vertex layout.
//! [`Donor046::read`] answers each path with the bytes 0.26g reads:
//!
//! * configs (models' `settings`, `skeleton` and `export_properties`,
//!   skeletons, `.bones_names`, materials, sounds, texture options): the same
//!   tree in 0.26g's 24-byte records;
//! * clips (`.b-spline`): event channel headers of 16 bytes
//!   ([`super::clips`]);
//! * skinned surfaces (`converted_model`): only chunks 3 and 4 (vertices and
//!   indices); 0.46c2 adds 110 (4-byte statistics) and 113 (72 bytes);
//! * a skinned model's `render/export_properties`: 0.26g's exe finds the
//!   muzzle and the sight by the locators `barrel_point` and `scope_point`
//!   (strings of `survarium.exe`; no config names them). 0.46c2's muzzle is
//!   `fire_point` (the end of the muzzle, past any device built into the
//!   mesh): 0.26g's AK-12 and H&K 416 put `barrel_point` within 2.5 cm of
//!   it. 0.46c2's own `barrel_point`, where a few models (SVU, Glock 17,
//!   Colt M1911) attach a barrel module, is mid-gun: it becomes
//!   `barrel_mount_point` and `barrel_point` is the muzzle. A model without
//!   `scope_point` gets its `picatini_scope_point`, or 0 as 0.26g's AK-12.
//! * textures, sounds (`.ogg`, `.rms`) and bind poses: as they are.
//!
//! [`Donor046::item_config`] is the `weapons.<name>` table of
//! `gameplay/game.options`, whose integral floats the 16-byte records keep
//! as integers (a config writer conforming to a 0.26g template turns them
//! back, `super::conform`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::ResourceDb;
use crate::binary_config::{BinaryConfig, ConfigValue as V};
use super::clips;

/// The file every 0.46c2 item config is in.
pub const GAME_OPTIONS: &str = "gameplay/game.options";
/// Mesh chunks 0.26g reads (vertices, indices) and the ones 0.46c2 adds.
const MESH_CHUNKS: [u32; 2] = [3, 4];
const LATER_MESH_CHUNKS: [u32; 2] = [110, 113];
/// The locators 0.26g's weapon code reads, and where 0.46c2 has the same
/// place under another name (first found).
const LOCATORS: [(&str, &[&str]); 2] = [
    ("barrel_point", &["fire_point", "muzzle_point"]),
    ("scope_point", &["scope_point", "picatini_scope_point"]),
];
/// 0.46c2's `barrel_point` (a barrel module's mount) is kept under this name.
pub const BARREL_MOUNT: &str = "barrel_mount_point";

/// The 0.46c2 game's archives and item configs.
pub struct Donor046 {
    db: ResourceDb,
    weapons: BTreeMap<String, V>,
}

/// The volumes of a 0.46c2 game folder (`resources.00.db` ..), in mount
/// order, or of a `.split` list.
pub fn volumes(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_dir() {
        let mut found: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("resources.")
                        && n.ends_with(".db")
                        && n["resources.".len()..n.len() - 3]
                            .chars()
                            .all(|c| c.is_ascii_digit())
                })
            })
            .collect();
        found.sort();
        ensure!(!found.is_empty(), "{}: no resources.NN.db", path.display());
        return Ok(found);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading volume list {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let found: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| dir.join(l))
        .collect();
    ensure!(!found.is_empty(), "{} names no volumes", path.display());
    Ok(found)
}

impl Donor046 {
    /// The game at `path`: its folder, or a `.split` list of its volumes.
    pub fn open(path: &Path) -> Result<Self> {
        let mut db = ResourceDb::open_layers(&volumes(path)?)?;
        let game = BinaryConfig::parse(&db.read(GAME_OPTIONS)?)
            .context("parsing 0.46c2's game.options")?;
        let Some(V::Named(weapons)) = game.root.named("weapons") else {
            bail!("{GAME_OPTIONS} has no weapons table");
        };
        let weapons = weapons
            .iter()
            .map(|w| (w.name.clone(), w.value.clone()))
            .collect();
        Ok(Self { db, weapons })
    }

    pub fn contains(&self, path: &str) -> bool {
        self.db.contains(path)
    }

    pub fn is_file(&self, path: &str) -> bool {
        self.db.is_file(path)
    }

    pub fn paths_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.db.paths_with_prefix(prefix)
    }

    /// A weapon's config (`weapons.<name>` of `gameplay/game.options`).
    pub fn item_config(&self, name: &str) -> Result<V> {
        self.weapons
            .get(name)
            .cloned()
            .with_context(|| format!("0.46c2 has no weapon {name}"))
    }

    /// The file at `path` as the donor has it (icons, for instance).
    pub fn read_raw(&mut self, path: &str) -> Result<Vec<u8>> {
        self.db.read(path)
    }

    /// The file at `path` in 0.26g's layout (see the module docs).
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let bytes = self.db.read(path)?;
        native_layout(path, &bytes).with_context(|| format!("0.46c2 {path}"))
    }
}

/// `bytes` of the 0.46c2 file `path` in 0.26g's layout.
pub fn native_layout(path: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    if path.ends_with(".b-spline") {
        return clips::events_12_to_16(bytes);
    }
    if path.ends_with("/converted_model") {
        return mesh_chunks(bytes);
    }
    if [".dds", ".ogg", ".rms", "/bind_pose", ".png"]
        .iter()
        .any(|e| path.ends_with(e))
    {
        return Ok(bytes.to_vec());
    }
    let mut config = BinaryConfig::parse(bytes)?;
    if path.ends_with(".skinned_model/render/export_properties") {
        add_locators(&mut config.root)?;
    }
    config.to_bytes()
}

/// A skinned surface with chunks 3 and 4 only.
pub fn mesh_chunks(b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(b.len());
    let mut ids = vec![];
    let mut at = 0;
    while at < b.len() {
        let head = b.get(at..at + 8).context("truncated chunk header")?;
        let id = u32::from_le_bytes(head[..4].try_into()?);
        let size = u32::from_le_bytes(head[4..].try_into()?) as usize;
        let end = at + 8 + size;
        ensure!(end <= b.len(), "truncated chunk {id}");
        if MESH_CHUNKS.contains(&id) {
            out.extend_from_slice(&b[at..end]);
        } else {
            ensure!(LATER_MESH_CHUNKS.contains(&id), "unknown mesh chunk {id}");
        }
        ids.push(id);
        at = end;
    }
    ensure!(ids.starts_with(&MESH_CHUNKS), "mesh chunks {ids:?}");
    Ok(out)
}

fn locator_name(l: &V) -> Option<&str> {
    match l.named("name") {
        Some(V::String(s)) => Some(s),
        _ => None,
    }
}

/// `barrel_point` at the muzzle and `scope_point` at the sight mount, as
/// copies of 0.46c2's locators at those places ([`LOCATORS`]); 0.46c2's own
/// `barrel_point` is kept as [`BARREL_MOUNT`]; a model with no sight locator
/// gets `scope_point` at 0, as 0.26g's AK-12. Idempotent.
pub fn add_locators(root: &mut V) -> Result<()> {
    let Some(V::Indexed(locators)) = root.named_mut("locators") else {
        bail!("export_properties without locators");
    };
    let muzzle = LOCATORS[0]
        .1
        .iter()
        .find_map(|m| locators.iter().find(|l| locator_name(l) == Some(m)))
        .and_then(|m| m.named("position"))
        .cloned();
    let barrel_at_muzzle = locators.iter().any(|b| {
        locator_name(b) == Some("barrel_point")
            && muzzle.is_some()
            && b.named("position") == muzzle.as_ref()
    });
    if !barrel_at_muzzle {
        for l in locators.iter_mut() {
            if locator_name(l) == Some("barrel_point") {
                l.set_path("name", V::String(BARREL_MOUNT.into()))?;
            }
        }
    }
    for (name, sources) in LOCATORS {
        if locators.iter().any(|l| locator_name(l) == Some(name)) {
            continue;
        }
        let source = sources
            .iter()
            .find_map(|s| locators.iter().find(|l| locator_name(l) == Some(s)));
        let mut copy = match (source, name) {
            (Some(l), _) => l.clone(),
            (None, "scope_point") => {
                let bone = locators
                    .iter()
                    .find(|l| locator_name(l) == Some("aim"))
                    .context("no aim locator to take the bone from")?;
                let mut l = bone.clone();
                l.set_path("position", V::Float3([0.0; 3]))?;
                l.set_path("rotation", V::Float3([0.0; 3]))?;
                l
            }
            (None, _) => bail!("no locator for {name} ({sources:?})"),
        };
        copy.set_path("name", V::String(name.into()))?;
        locators.push(copy);
    }
    Ok(())
}

/// A locator's position, for the record (`muzzle_point` for the muzzle
/// device step).
pub fn locator_position(root: &V, name: &str) -> Option<[f32; 3]> {
    let V::Indexed(locators) = root.named("locators")? else {
        return None;
    };
    locators
        .iter()
        .find(|l| locator_name(l) == Some(name))
        .and_then(|l| match l.named("position") {
            Some(V::Float3(p)) => Some(*p),
            _ => None,
        })
}

/// A lobby cell (171x83, RGBA PNG) of a 0.46c2 item icon. 0.46c2 draws a
/// weapon on a 128x128 square, diagonally, muzzle up and to the left (the
/// weapon's right side); 0.26g's weapon atlas draws it level, muzzle to the
/// right (its left side), as wide as the cell allows. The icon is turned
/// by `angle` (degrees, clockwise from level) or else its silhouette's main
/// axis (the alpha-weighted principal axis: a long gun's barrel; a pistol's
/// grip pulls it off its slide, so a pistol's is given), mirrored, cropped
/// to what it draws and scaled to fit `fit` (width, height), centred in the
/// cell.
pub fn lobby_cell(png_bytes: &[u8], fit: (usize, usize), angle: Option<f32>) -> Result<Vec<u8>> {
    let (w, h, rgba) = decode_rgba(png_bytes)?;
    // Premultiplied, so that resampling does not bleed transparent colours.
    let pre: Vec<[f32; 4]> = rgba
        .chunks(4)
        .map(|p| {
            let a = f32::from(p[3]) / 255.0;
            [
                f32::from(p[0]) * a,
                f32::from(p[1]) * a,
                f32::from(p[2]) * a,
                a,
            ]
        })
        .collect();
    let angle = match angle {
        Some(degrees) => degrees.to_radians(),
        None => main_axis(&pre, w, h)?,
    };
    // Turn by -angle about the centre (so the axis lies along +x), then mirror:
    // sample the source at the rotated position of each output pixel. The
    // output square holds the whole turned icon.
    let side = ((w * w + h * h) as f32).sqrt().ceil() as usize + 2;
    let (cs, sn) = (angle.cos(), angle.sin());
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let half = side as f32 / 2.0;
    let mut turned = vec![[0f32; 4]; side * side];
    for y in 0..side {
        for x in 0..side {
            // Mirrored: output x runs against the axis.
            let (u, v) = (half - (x as f32 + 0.5), y as f32 + 0.5 - half);
            let (sx, sy) = (cx + u * cs - v * sn, cy + u * sn + v * cs);
            turned[y * side + x] = bilinear(&pre, w, h, sx - 0.5, sy - 0.5);
        }
    }
    let (x0, y0, x1, y1) = bounds(&turned, side, side).context("an empty icon")?;
    let (cw, ch) = (x1 - x0, y1 - y0);
    let scale = (fit.0 as f32 / cw as f32).min(fit.1 as f32 / ch as f32);
    let (ow, oh) = (
        ((cw as f32 * scale).round() as usize).max(1),
        ((ch as f32 * scale).round() as usize).max(1),
    );
    let (ox, oy) = ((171 - ow) / 2, (83 - oh) / 2);
    let mut cell = vec![0u8; 171 * 83 * 4];
    for y in 0..oh {
        for x in 0..ow {
            // Box filter over the source area this pixel covers.
            let (fx0, fx1) = (x as f32 / scale, (x + 1) as f32 / scale);
            let (fy0, fy1) = (y as f32 / scale, (y + 1) as f32 / scale);
            let mut sum = [0f32; 4];
            let mut n = 0f32;
            let mut sy = fy0;
            while sy < fy1 {
                let mut sx = fx0;
                while sx < fx1 {
                    let p = bilinear(&turned, side, side, x0 as f32 + sx, y0 as f32 + sy);
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                    n += 1.0;
                    sx += 0.5;
                }
                sy += 0.5;
            }
            let a = sum[3] / n;
            let at = ((oy + y) * 171 + ox + x) * 4;
            for c in 0..3 {
                cell[at + c] = if a > 0.0 {
                    (sum[c] / n / a).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                };
            }
            cell[at + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, 171, 83);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&cell)?;
    }
    Ok(out)
}

/// An 8-bit PNG (palette, grey or colour, with or without alpha) as RGBA.
pub fn decode_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    ensure!(info.bit_depth == png::BitDepth::Eight, "icon is not 8-bit");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2]
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => bail!("icon colour type {other:?} after expansion"),
    };
    Ok((w, h, rgba))
}

/// The angle (radians, image coordinates, y down) of the alpha-weighted
/// principal axis, within (-pi/2, pi/2].
fn main_axis(px: &[[f32; 4]], w: usize, h: usize) -> Result<f32> {
    let (mut sum, mut sx, mut sy) = (0f64, 0f64, 0f64);
    for y in 0..h {
        for x in 0..w {
            let a = f64::from(px[y * w + x][3]);
            sum += a;
            sx += a * x as f64;
            sy += a * y as f64;
        }
    }
    ensure!(sum > 0.0, "an empty icon");
    let (mx, my) = (sx / sum, sy / sum);
    let (mut xx, mut yy, mut xy) = (0f64, 0f64, 0f64);
    for y in 0..h {
        for x in 0..w {
            let a = f64::from(px[y * w + x][3]);
            let (dx, dy) = (x as f64 - mx, y as f64 - my);
            xx += a * dx * dx;
            yy += a * dy * dy;
            xy += a * dx * dy;
        }
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    Ok(angle as f32)
}

fn bilinear(px: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: f32, yi: f32| -> [f32; 4] {
        if xi < 0.0 || yi < 0.0 || xi >= w as f32 || yi >= h as f32 {
            [0.0; 4]
        } else {
            px[yi as usize * w + xi as usize]
        }
    };
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1.0, y0),
        at(x0, y0 + 1.0),
        at(x0 + 1.0, y0 + 1.0),
    );
    let mut out = [0f32; 4];
    for i in 0..4 {
        out[i] =
            (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    out
}

/// The box (x0, y0, x1, y1) of the pixels at least slightly opaque.
fn bounds(px: &[[f32; 4]], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[y * w + x][3] > 0.02 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

#[cfg(test)]
mod tests;
