// SPDX-License-Identifier: GPL-3.0-or-later
//! Pictures of the faces' fit: a software z-buffer render of converted
//! models (any vertex stride) in the bind pose, each with its colour map
//! (a DXT1 or DXT5 DDS), one PNG a tile.
//!
//! ```text
//! cargo run --release --example render-faces-026 -- --plan plan.json
//! ```
//!
//! `plan.json`: `{"size": 480, "centre": [0, 1.64, 0], "height": 0.46,
//! "tiles": [{"out": "a.png", "yaw": 0, "pitch": 0, "meshes": [{"surface":
//! "path", "diffuse": "path" | null, "two_sided": false, "alpha_test":
//! false}]}]}`; yaw 0 looks at the face, 180 at the back of the head.
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    plan: PathBuf,
}

#[derive(Deserialize)]
struct Plan {
    size: usize,
    centre: [f64; 3],
    /// The world height the picture spans (m).
    height: f64,
    /// Width:height of the picture (1 when left out).
    #[serde(default)]
    aspect: Option<f64>,
    /// A clear background (RGBA, for icons) instead of a dark one.
    #[serde(default)]
    transparent: bool,
    tiles: Vec<Tile>,
}

#[derive(Deserialize)]
struct Tile {
    out: PathBuf,
    yaw: f64,
    #[serde(default)]
    pitch: f64,
    meshes: Vec<MeshSpec>,
}

#[derive(Deserialize, Clone)]
struct MeshSpec {
    surface: PathBuf,
    diffuse: Option<PathBuf>,
    #[serde(default)]
    two_sided: bool,
    #[serde(default)]
    alpha_test: bool,
    /// Multiplies the colour (a tint to tell layers apart), default white.
    #[serde(default)]
    tint: Option<[f64; 3]>,
}

struct Mesh {
    pos: Vec<[f64; 3]>,
    uv: Vec<[f64; 2]>,
    tris: Vec<[usize; 3]>,
}

fn u32_at(b: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).context("short")?.try_into()?) as usize)
}

fn f32_at(b: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes(b[at..at + 4].try_into().unwrap()))
}

fn read_mesh(b: &[u8]) -> Result<Mesh> {
    ensure!(u32_at(b, 0)? == 3, "no vertex chunk");
    let n = u32_at(b, 8)?;
    let stride = (u32_at(b, 4)? - 4) / n;
    let mut pos = vec![];
    let mut uv = vec![];
    for i in 0..n {
        let at = 12 + i * stride;
        pos.push([f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8)]);
        uv.push([f32_at(b, at + stride - 8), f32_at(b, at + stride - 4)]);
    }
    let ic = 12 + n * stride;
    ensure!(u32_at(b, ic)? == 4, "no index chunk");
    let count = u32_at(b, ic + 8)?;
    let idx: Vec<usize> = (0..count)
        .map(|k| usize::from(u16::from_le_bytes([b[ic + 12 + 2 * k], b[ic + 13 + 2 * k]])))
        .collect();
    Ok(Mesh {
        pos,
        uv,
        tris: idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
    })
}

struct Texture {
    w: usize,
    h: usize,
    rgba: Vec<[f64; 4]>,
}

fn read_dds(b: &[u8]) -> Result<Texture> {
    ensure!(&b[0..4] == b"DDS ", "not a DDS");
    let h = u32_at(b, 12)?;
    let w = u32_at(b, 16)?;
    let four = &b[84..88];
    let (block, alpha) = match four {
        b"DXT1" => (8, false),
        b"DXT5" => (16, true),
        _ => bail!("{}", String::from_utf8_lossy(four)),
    };
    let mut rgba = vec![[1.0; 4]; w * h];
    let bw = w.div_ceil(4);
    for by in 0..h.div_ceil(4) {
        for bx in 0..bw {
            let at = 128 + (by * bw + bx) * block;
            let blk = b.get(at..at + block).context("short DDS")?;
            let mut alphas = [1.0_f64; 16];
            if alpha {
                let (a0, a1) = (f64::from(blk[0]) / 255.0, f64::from(blk[1]) / 255.0);
                let mut bits = 0u64;
                for k in 0..6 {
                    bits |= u64::from(blk[2 + k]) << (8 * k);
                }
                for (t, a) in alphas.iter_mut().enumerate() {
                    let i = ((bits >> (3 * t)) & 7) as usize;
                    *a = match (i, blk[0] > blk[1]) {
                        (0, _) => a0,
                        (1, _) => a1,
                        (i, true) => (a0 * (8 - i) as f64 + a1 * (i - 1) as f64) / 7.0,
                        (6, false) => 0.0,
                        (7, false) => 1.0,
                        (i, false) => (a0 * (6 - i) as f64 + a1 * (i - 1) as f64) / 5.0,
                    };
                }
            }
            let c = &blk[block - 8..];
            let rgb = |v: u16| {
                [
                    f64::from((v >> 11) & 31) / 31.0,
                    f64::from((v >> 5) & 63) / 63.0,
                    f64::from(v & 31) / 31.0,
                ]
            };
            let (v0, v1) = (
                u16::from_le_bytes([c[0], c[1]]),
                u16::from_le_bytes([c[2], c[3]]),
            );
            let (a, z) = (rgb(v0), rgb(v1));
            let four_colour = alpha || v0 > v1;
            let pal = [
                a,
                z,
                [0, 1, 2].map(|k| {
                    if four_colour {
                        (2.0 * a[k] + z[k]) / 3.0
                    } else {
                        (a[k] + z[k]) / 2.0
                    }
                }),
                [0, 1, 2].map(|k| {
                    if four_colour {
                        (a[k] + 2.0 * z[k]) / 3.0
                    } else {
                        0.0
                    }
                }),
            ];
            let bits = u32::from_le_bytes(c[4..8].try_into()?);
            for t in 0..16 {
                let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
                if x < w && y < h {
                    let p = pal[((bits >> (2 * t)) & 3) as usize];
                    rgba[y * w + x] = [p[0], p[1], p[2], alphas[t]];
                }
            }
        }
    }
    Ok(Texture { w, h, rgba })
}

impl Texture {
    fn at(&self, uv: [f64; 2]) -> [f64; 4] {
        let x = ((uv[0].rem_euclid(1.0)) * self.w as f64) as usize % self.w;
        let y = ((uv[1].rem_euclid(1.0)) * self.h as f64) as usize % self.h;
        self.rgba[y * self.w + x]
    }
}

fn main() -> Result<()> {
    let a = Args::parse();
    let base = a
        .plan
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let plan: Plan = serde_json::from_slice(&fs::read(&a.plan)?)?;
    let mut meshes: HashMap<PathBuf, Mesh> = HashMap::new();
    let mut textures: HashMap<PathBuf, Texture> = HashMap::new();
    let ss = 2;
    let size = plan.size * ss;
    let width = (plan.size as f64 * plan.aspect.unwrap_or(1.0)).round() as usize * ss;
    let scale = size as f64 / plan.height;
    for tile in &plan.tiles {
        let (sy, cy) = tile.yaw.to_radians().sin_cos();
        let (sp, cp) = tile.pitch.to_radians().sin_cos();
        let view = |p: [f64; 3]| {
            let d = [
                p[0] - plan.centre[0],
                p[1] - plan.centre[1],
                p[2] - plan.centre[2],
            ];
            let x = d[0] * cy - d[2] * sy;
            let z = d[0] * sy + d[2] * cy;
            let y = d[1] * cp - z * sp;
            let z = d[1] * sp + z * cp;
            [x, y, z]
        };
        let mut depth = vec![f64::NEG_INFINITY; width * size];
        let mut colour = vec![[0.17, 0.18, 0.2]; width * size];
        let light = {
            let l: [f64; 3] = [0.35, 0.5, 0.8];
            let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
            l.map(|v| v / n)
        };
        for spec in &tile.meshes {
            let sp = base.join(&spec.surface);
            if !meshes.contains_key(&sp) {
                meshes.insert(
                    sp.clone(),
                    read_mesh(&fs::read(&sp).with_context(|| sp.display().to_string())?)?,
                );
            }
            let tex = match &spec.diffuse {
                Some(d) => {
                    let dp = base.join(d);
                    if !textures.contains_key(&dp) {
                        textures.insert(
                            dp.clone(),
                            read_dds(&fs::read(&dp).with_context(|| dp.display().to_string())?)?,
                        );
                    }
                    Some(dp)
                }
                None => None,
            };
            let mesh = &meshes[&sp];
            let tex = tex.map(|t| &textures[&t]);
            let tint = spec.tint.unwrap_or([1.0; 3]);
            let vp: Vec<[f64; 3]> = mesh.pos.iter().map(|&p| view(p)).collect();
            for t in &mesh.tris {
                let [a, b, c] = t.map(|i| vp[i]);
                let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let mut n = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len < 1e-14 {
                    continue;
                }
                n = n.map(|v| v / len);
                // The native winding faces out along cross(e1, e2); the
                // camera looks along -z.
                if n[2] < 0.0 {
                    if !spec.two_sided {
                        continue;
                    }
                    n = n.map(|v| -v);
                }
                let shade =
                    0.42 + 0.58 * (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
                let sx = |p: [f64; 3]| width as f64 / 2.0 + p[0] * scale;
                let syy = |p: [f64; 3]| size as f64 / 2.0 - p[1] * scale;
                let (xs, ys) = ([sx(a), sx(b), sx(c)], [syy(a), syy(b), syy(c)]);
                let x0 = xs
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min)
                    .floor()
                    .max(0.0) as usize;
                let x1 = (xs.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() as isize)
                    .min(width as isize - 1);
                let y0 = ys
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min)
                    .floor()
                    .max(0.0) as usize;
                let y1 = (ys.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() as isize)
                    .min(size as isize - 1);
                if x1 < 0 || y1 < 0 {
                    continue;
                }
                let d = (ys[1] - ys[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (ys[0] - ys[2]);
                if d.abs() < 1e-12 {
                    continue;
                }
                for y in y0..=y1 as usize {
                    for x in x0..=x1 as usize {
                        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                        let w0 =
                            ((ys[1] - ys[2]) * (px - xs[2]) + (xs[2] - xs[1]) * (py - ys[2])) / d;
                        let w1 =
                            ((ys[2] - ys[0]) * (px - xs[2]) + (xs[0] - xs[2]) * (py - ys[2])) / d;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6 {
                            continue;
                        }
                        let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                        let k = y * width + x;
                        if z <= depth[k] {
                            continue;
                        }
                        let rgba = match tex {
                            Some(tex) => {
                                let uv = [0, 1].map(|j| {
                                    w0 * mesh.uv[t[0]][j]
                                        + w1 * mesh.uv[t[1]][j]
                                        + w2 * mesh.uv[t[2]][j]
                                });
                                tex.at(uv)
                            }
                            None => [0.6, 0.6, 0.6, 1.0],
                        };
                        if spec.alpha_test && rgba[3] < 0.5 {
                            continue;
                        }
                        depth[k] = z;
                        colour[k] = [0, 1, 2].map(|j| (rgba[j] * shade * tint[j]).min(1.0));
                    }
                }
            }
        }
        // Down to the plan's size, 2x2 averaged.
        let (n, nw) = (plan.size, width / ss);
        let channels = if plan.transparent { 4 } else { 3 };
        let mut px = vec![0u8; nw * n * channels];
        for y in 0..n {
            for x in 0..nw {
                let mut sum = [0.0; 3];
                let mut covered = 0.0;
                for dy in 0..ss {
                    for dx in 0..ss {
                        let k = (y * ss + dy) * width + x * ss + dx;
                        if depth[k] > f64::NEG_INFINITY || !plan.transparent {
                            for j in 0..3 {
                                sum[j] += colour[k][j];
                            }
                            covered += 1.0;
                        }
                    }
                }
                let at = (y * nw + x) * channels;
                for j in 0..3 {
                    let v = if covered > 0.0 {
                        (sum[j] / covered).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    px[at + j] = (v.powf(1.0 / 1.15) * 255.0).round() as u8;
                }
                if plan.transparent {
                    px[at + 3] = (covered / (ss * ss) as f64 * 255.0).round() as u8;
                }
            }
        }
        let out = base.join(&tile.out);
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        let file = fs::File::create(&out)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), nw as u32, n as u32);
        enc.set_color(if plan.transparent {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&px)?;
    }
    println!("rendered {} tiles", plan.tiles.len());
    Ok(())
}
