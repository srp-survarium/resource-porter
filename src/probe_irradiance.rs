// SPDX-License-Identifier: GPL-3.0-or-later
//! The diffuse cube 0.26 reads next to every environment probe.
//!
//! 0.26 ships each probe as two cubes: `<probe>.dds` (reflections) and
//! `<probe>_diffuse.dds`, a 16x16 cube with three mips that the deferred
//! light pass samples along the pixel normal and scales by the probe's
//! `diffuse_multiplier`. Native diffuse cubes are irradiance: across each
//! face the brightest texel is within 1.1-1.3x of the face mean.
//!
//! Early builds (0.10b, 0.11a, 0.11e) have no diffuse cube. Taking the
//! source probe's 16x16 mip tail kept window and sky hot spots at 3.5-7.5x
//! the face mean. Under the early multipliers (6-20) a normal-mapped floor
//! then lights each bump from a different hot spot, and rubble and gravel
//! read as crumpled foil (v0.11 Radar hangars, Chemical's halls).
//!
//! This builds the cube as the cosine-weighted average of the source
//! probe's radiance over the hemisphere around each texel direction. A
//! uniform environment comes back unchanged, so a probe's overall
//! brightness and colour stay what the early map authored.
use anyhow::{Context, Result, bail, ensure};

/// Output edge and mip count of a native diffuse cube.
const SIZE: usize = 16;
const MIPS: usize = 3;
/// Source radiance is box-filtered to this edge before the convolution.
const SOURCE: usize = 32;

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Dxt1,
    Dxt5,
}

impl Format {
    fn block(self) -> usize {
        match self {
            Self::Dxt1 => 8,
            Self::Dxt5 => 16,
        }
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

/// Unit direction through the centre of texel (x, y) of a D3D cube face.
fn direction(face: usize, x: usize, y: usize, size: usize) -> [f32; 3] {
    let u = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

/// Solid angle of texel (x, y), up to the constant texel area.
fn solid_angle(x: usize, y: usize, size: usize) -> f32 {
    let u = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    (1.0 + u * u + v * v).powf(-1.5)
}

fn rgb565(c: u16) -> [f32; 3] {
    [
        ((c >> 11) & 31) as f32 / 31.0,
        ((c >> 5) & 63) as f32 / 63.0,
        (c & 31) as f32 / 31.0,
    ]
}

fn decode_color(block: &[u8], format: Format) -> [[f32; 3]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: f32, wb: f32| std::array::from_fn(|i| a[i] * wa + b[i] * wb);
    let palette: [[f32; 3]; 4] = if format == Format::Dxt5 || c0 > c1 {
        [a, b, mix(2.0 / 3.0, 1.0 / 3.0), mix(1.0 / 3.0, 2.0 / 3.0)]
    } else {
        [a, b, mix(0.5, 0.5), [0.0; 3]]
    };
    let bits = u32::from_le_bytes(block[4..8].try_into().unwrap());
    std::array::from_fn(|i| palette[((bits >> (2 * i)) & 3) as usize])
}

fn decode_alpha(block: &[u8]) -> [f32; 16] {
    let (a0, a1) = (block[0] as f32, block[1] as f32);
    let mut p = [a0, a1, 0.0, 0.0, 0.0, 0.0, 0.0, 255.0];
    if block[0] > block[1] {
        for i in 1..7 {
            p[i + 1] = ((7 - i) as f32 * a0 + i as f32 * a1) / 7.0;
        }
    } else {
        for i in 1..5 {
            p[i + 1] = ((5 - i) as f32 * a0 + i as f32 * a1) / 5.0;
        }
    }
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&block[2..8]);
    let bits = u64::from_le_bytes(bits);
    std::array::from_fn(|i| p[((bits >> (3 * i)) & 7) as usize] / 255.0)
}

fn quantize565(c: [f32; 3]) -> u16 {
    let q = |v: f32, max: f32| (v.clamp(0.0, 1.0) * max).round() as u16;
    (q(c[0], 31.0) << 11) | (q(c[1], 63.0) << 5) | q(c[2], 31.0)
}

/// Four-colour BC1 block (c0 > c1) fitted along the block's luminance range.
fn encode_color(texels: &[[f32; 3]; 16]) -> [u8; 8] {
    let lum = |c: &[f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let lo = texels.iter().min_by(|a, b| lum(a).total_cmp(&lum(b))).unwrap();
    let hi = texels.iter().max_by(|a, b| lum(a).total_cmp(&lum(b))).unwrap();
    let (mut c0, mut c1) = (quantize565(*hi), quantize565(*lo));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut out = [0u8; 8];
    out[..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    if c0 == c1 {
        return out;
    }
    let (a, b) = (rgb565(c0), rgb565(c1));
    let palette: [[f32; 3]; 4] = [
        a,
        b,
        std::array::from_fn(|i| (2.0 * a[i] + b[i]) / 3.0),
        std::array::from_fn(|i| (a[i] + 2.0 * b[i]) / 3.0),
    ];
    let mut bits = 0u32;
    for (i, t) in texels.iter().enumerate() {
        let error = |p: &[f32; 3]| (0..3).map(|k| (p[k] - t[k]).powi(2)).sum::<f32>();
        let index = (0..4)
            .min_by(|&x, &y| error(&palette[x]).total_cmp(&error(&palette[y])))
            .unwrap();
        bits |= (index as u32) << (2 * i);
    }
    out[4..].copy_from_slice(&bits.to_le_bytes());
    out
}

/// BC3 alpha block holding one value.
fn encode_alpha(alpha: f32) -> [u8; 8] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    [a, a, 0, 0, 0, 0, 0, 0]
}

/// Builds the 16x16, three-mip diffuse cube for an early probe cube. The
/// result keeps the source's compression and trailing colour-space byte.
pub fn diffuse_cube(bytes: &[u8]) -> Result<Vec<u8>> {
    diffuse_cube_scaled(bytes, 1.0)
}

/// `diffuse_cube` with the irradiance (not the alpha) multiplied by `gain`.
pub fn diffuse_cube_scaled(bytes: &[u8], gain: f32) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() >= 129 && &bytes[..4] == b"DDS ",
        "invalid probe DDS"
    );
    let size = u32_at(bytes, 12);
    ensure!(
        size == u32_at(bytes, 16) && size.is_power_of_two() && (SIZE..=4096).contains(&size),
        "unsupported probe size"
    );
    ensure!(
        u32_at(bytes, 112) & 0xfe00 == 0xfe00,
        "probe is not a six-face cubemap"
    );
    let format = match &bytes[84..88] {
        b"DXT1" => Format::Dxt1,
        b"DXT5" => Format::Dxt5,
        _ => bail!("unsupported probe compression"),
    };
    let srgb = bytes[bytes.len() - 1];
    ensure!(srgb <= 1, "probe has no colour-space byte");
    let mips = u32_at(bytes, 28).max(1);
    let face_bytes: usize = (0..mips)
        .map(|m| ((size >> m) / 4).max(1).pow(2) * format.block())
        .sum();
    ensure!(
        bytes.len() == 128 + face_bytes * 6 + 1,
        "probe cube length mismatch"
    );
    let to_linear = |v: f32| if srgb == 1 { v.powf(2.2) } else { v };
    let to_stored = |v: f32| if srgb == 1 { v.max(0.0).powf(1.0 / 2.2) } else { v };

    // Radiance (r, g, b, a) per source texel, box-filtered to `edge`.
    let edge = SOURCE.min(size);
    let step = size / edge;
    let mut radiance = vec![[0f32; 4]; 6 * edge * edge];
    for face in 0..6 {
        let base = 128 + face * face_bytes;
        let blocks = size / 4;
        for by in 0..blocks {
            for bx in 0..blocks {
                let o = base + (by * blocks + bx) * format.block();
                let block = &bytes[o..o + format.block()];
                let (alpha, color) = match format {
                    Format::Dxt5 => (decode_alpha(&block[..8]), decode_color(&block[8..], format)),
                    Format::Dxt1 => ([1.0; 16], decode_color(block, format)),
                };
                for i in 0..16 {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    let cell = &mut radiance[(face * edge + y / step) * edge + x / step];
                    for k in 0..3 {
                        cell[k] += to_linear(color[i][k]);
                    }
                    cell[3] += alpha[i];
                }
            }
        }
    }
    let samples = (step * step) as f32;
    let source: Vec<([f32; 3], f32, [f32; 4])> = (0..6 * edge * edge)
        .map(|i| {
            let (face, y, x) = (i / (edge * edge), i / edge % edge, i % edge);
            let r = radiance[i].map(|v| v / samples);
            (direction(face, x, y, edge), solid_angle(x, y, edge), r)
        })
        .collect();

    let irradiance = |n: [f32; 3]| -> [f32; 4] {
        let (mut sum, mut weight) = ([0f32; 4], 0f32);
        for (d, omega, r) in &source {
            let cos = n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
            if cos > 0.0 {
                let w = cos * omega;
                for k in 0..4 {
                    sum[k] += r[k] * w;
                }
                weight += w;
            }
        }
        let mut e = sum.map(|v| v / weight);
        for v in &mut e[..3] {
            *v *= gain;
        }
        e
    };

    let mut result = bytes[..128].to_vec();
    let total: usize = (0..MIPS).map(|m| ((SIZE >> m) / 4).pow(2) * format.block()).sum();
    for (o, n) in [(12, SIZE), (16, SIZE), (20, total * 6), (28, MIPS)] {
        result[o..o + 4].copy_from_slice(&(n as u32).to_le_bytes());
    }
    for face in 0..6 {
        for mip in 0..MIPS {
            let out = SIZE >> mip;
            let texels: Vec<[f32; 4]> = (0..out * out)
                .map(|i| irradiance(direction(face, i % out, i / out, out)))
                .collect();
            for by in 0..out / 4 {
                for bx in 0..out / 4 {
                    let at = |i: usize| texels[(by * 4 + i / 4) * out + bx * 4 + i % 4];
                    let color: [[f32; 3]; 16] =
                        std::array::from_fn(|i| std::array::from_fn(|k| to_stored(at(i)[k])));
                    if format == Format::Dxt5 {
                        let alpha = (0..16).map(|i| at(i)[3]).sum::<f32>() / 16.0;
                        result.extend(encode_alpha(alpha));
                    }
                    result.extend(encode_color(&color));
                }
            }
        }
    }
    result.push(srgb);
    Ok(result)
}

/// Brightest texel of each top-level face over that face's mean luminance.
pub fn face_peak_ratios(bytes: &[u8]) -> Result<[f32; 6]> {
    let size = u32_at(bytes, 12);
    let format = match &bytes[84..88] {
        b"DXT1" => Format::Dxt1,
        b"DXT5" => Format::Dxt5,
        _ => bail!("unsupported probe compression"),
    };
    let mips = u32_at(bytes, 28).max(1);
    let face_bytes: usize = (0..mips)
        .map(|m| ((size >> m) / 4).max(1).pow(2) * format.block())
        .sum();
    let mut ratios = [0f32; 6];
    for (face, ratio) in ratios.iter_mut().enumerate() {
        let base = 128 + face * face_bytes;
        let (mut sum, mut peak, mut n) = (0f32, 0f32, 0f32);
        for b in 0..(size / 4).pow(2) {
            let o = base + b * format.block();
            let block = bytes.get(o..o + format.block()).context("short probe")?;
            let color = match format {
                Format::Dxt5 => decode_color(&block[8..], format),
                Format::Dxt1 => decode_color(block, format),
            };
            for c in color {
                let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                sum += l;
                peak = peak.max(l);
                n += 1.0;
            }
        }
        *ratio = if sum > 0.0 { peak / (sum / n) } else { 1.0 };
    }
    Ok(ratios)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DXT5 cube of `size` whose texels come from `color(face, x, y)`.
    fn cube(size: usize, srgb: u8, color: impl Fn(usize, usize, usize) -> [f32; 3]) -> Vec<u8> {
        let mips = size.ilog2() as usize - 1;
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        for (o, n) in [(4, 124), (12, size), (16, size), (28, mips), (112, 0xfe00)] {
            bytes[o..o + 4].copy_from_slice(&(n as u32).to_le_bytes());
        }
        bytes[84..88].copy_from_slice(b"DXT5");
        for face in 0..6 {
            for mip in 0..mips {
                let edge = size >> mip;
                let scale = size / edge;
                for by in 0..edge / 4 {
                    for bx in 0..edge / 4 {
                        let texels: [[f32; 3]; 16] = std::array::from_fn(|i| {
                            color(face, (bx * 4 + i % 4) * scale, (by * 4 + i / 4) * scale)
                        });
                        bytes.extend(encode_alpha(1.0));
                        bytes.extend(encode_color(&texels));
                    }
                }
            }
        }
        bytes.push(srgb);
        bytes
    }

    fn mean_color(bytes: &[u8]) -> [f32; 3] {
        let mut sum = [0f32; 3];
        let blocks = (u32_at(bytes, 12) / 4).pow(2);
        let face_bytes: usize = (0..u32_at(bytes, 28))
            .map(|m| ((u32_at(bytes, 12) >> m) / 4).pow(2) * 16)
            .sum();
        for face in 0..6 {
            for b in 0..blocks {
                let o = 128 + face * face_bytes + b * 16;
                for c in decode_color(&bytes[o + 8..o + 16], Format::Dxt5) {
                    for k in 0..3 {
                        sum[k] += c[k] / (6 * blocks * 16) as f32;
                    }
                }
            }
        }
        sum
    }

    #[test]
    fn the_cube_has_the_native_diffuse_shape() {
        let source = cube(128, 0, |_, _, _| [0.2, 0.3, 0.4]);
        let result = diffuse_cube(&source).unwrap();
        assert_eq!((u32_at(&result, 12), u32_at(&result, 16), u32_at(&result, 28)), (16, 16, 3));
        assert_eq!(result.len(), 128 + 6 * (16 + 4 + 1) * 16 + 1);
        assert_eq!(&result[84..88], b"DXT5");
        assert_eq!(result.last(), Some(&0));
        assert!(diffuse_cube(&source[..source.len() - 1]).is_err());
    }

    #[test]
    fn a_uniform_environment_keeps_its_colour() {
        let result = diffuse_cube(&cube(64, 0, |_, _, _| [0.2, 0.3, 0.4])).unwrap();
        let mean = mean_color(&result);
        for (got, want) in mean.iter().zip([0.2, 0.3, 0.4]) {
            assert!((got - want).abs() < 0.02, "{mean:?}");
        }
    }

    #[test]
    fn the_gain_scales_colour_but_not_alpha() {
        let result = diffuse_cube_scaled(&cube(64, 0, |_, _, _| [0.2, 0.3, 0.4]), 1.25).unwrap();
        let mean = mean_color(&result);
        for (got, want) in mean.iter().zip([0.25, 0.375, 0.5]) {
            assert!((got - want).abs() < 0.02, "{mean:?}");
        }
        assert_eq!(&result[128..130], &[255, 255]);
    }

    #[test]
    fn a_window_hot_spot_is_spread_over_the_hemisphere() {
        // A dark room with one bright window patch on +X, as the early
        // Radar hangar probes are.
        let source = cube(128, 0, |face, x, y| {
            if face == 0 && (48..80).contains(&x) && (48..80).contains(&y) {
                [1.0, 1.0, 0.9]
            } else {
                [0.05, 0.05, 0.05]
            }
        });
        let tail = face_peak_ratios(&source).unwrap();
        assert!(tail[0] > 5.0, "{tail:?}");
        let result = diffuse_cube(&source).unwrap();
        let ratios = face_peak_ratios(&result).unwrap();
        assert!(ratios.iter().all(|r| *r < 1.6), "{ratios:?}");
        // The window still lights the side that faces it most.
        let side = |bytes: &[u8], face: usize| {
            let o = 128 + face * (16 + 4 + 1) * 16 + 5 * 16;
            decode_color(&bytes[o + 8..o + 16], Format::Dxt5)[0][0]
        };
        assert!(side(&result, 0) > 1.5 * side(&result, 1));
    }
}
