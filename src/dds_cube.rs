// SPDX-License-Identifier: GPL-3.0-or-later
//! DXT5 (BC3) cube maps: decode, encode and the DDS container both builds'
//! environment probes use (a 256 or 16 texel cube, mips down to 4x4, the
//! engine's trailing sRGB option byte).
use anyhow::{Result, ensure};

/// One RGBA8 image per face and mip: `faces[face][mip]`, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeRgba8 {
    pub size: usize,
    pub faces: Vec<Vec<Vec<[u8; 4]>>>,
}

impl CubeRgba8 {
    pub fn mips(&self) -> usize {
        self.faces[0].len()
    }
}

/// Caps, height, width, pitch, pixel format, mip count, linear size: the
/// header every 0.26 and 0.69d probe carries.
const DDSD_FLAGS: u32 = 0x000A_100F;
const DDSCAPS_COMPLEX_MIPMAP_TEXTURE: u32 = 0x0040_1008;
const DDSCAPS2_CUBEMAP_ALL_FACES: u32 = 0x0000_FE00;

fn mip_size(size: usize, mip: usize) -> usize {
    (size >> mip).max(1)
}

fn blocks(size: usize) -> usize {
    size.div_ceil(4) * size.div_ceil(4)
}

/// Decode a DXT5 cube map DDS.
pub fn decode_cube(bytes: &[u8]) -> Result<CubeRgba8> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS file");
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    ensure!(&bytes[84..88] == b"DXT5", "not a DXT5 texture");
    ensure!(u32_at(112) & DDSCAPS2_CUBEMAP_ALL_FACES == DDSCAPS2_CUBEMAP_ALL_FACES, "not a full cube map");
    let (height, width) = (u32_at(12) as usize, u32_at(16) as usize);
    ensure!(width == height && width > 0, "cube faces must be square");
    let mips = u32_at(28).max(1) as usize;
    let per_face: usize = (0..mips).map(|m| blocks(mip_size(width, m)) * 16).sum();
    let payload = bytes.len() - 128;
    ensure!(payload == per_face * 6 || payload == per_face * 6 + 1, "DXT5 cube payload length does not match its header");
    let mut offset = 128;
    let mut faces = Vec::with_capacity(6);
    for _ in 0..6 {
        let mut chain = Vec::with_capacity(mips);
        for mip in 0..mips {
            let s = mip_size(width, mip);
            let mut image = vec![[0u8; 4]; s * s];
            for by in 0..s.div_ceil(4) {
                for bx in 0..s.div_ceil(4) {
                    let block = decode_block(bytes[offset..offset + 16].try_into().unwrap());
                    offset += 16;
                    for (i, texel) in block.iter().enumerate() {
                        let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                        if x < s && y < s {
                            image[y * s + x] = *texel;
                        }
                    }
                }
            }
            chain.push(image);
        }
        faces.push(chain);
    }
    Ok(CubeRgba8 { size: width, faces })
}

/// Encode a cube map as DXT5 DDS with the engine's trailing option byte
/// (`srgb`, 0 for probes).
pub fn encode_cube(cube: &CubeRgba8, srgb: u8) -> Vec<u8> {
    let mips = cube.mips();
    let mut out = vec![0u8; 128];
    let mut put = |o: usize, v: u32| out[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put(0, u32::from_le_bytes(*b"DDS "));
    put(4, 124);
    put(8, DDSD_FLAGS);
    put(12, cube.size as u32);
    put(16, cube.size as u32);
    put(28, mips as u32);
    put(76, 32);
    put(80, 4); // DDPF_FOURCC
    put(84, u32::from_le_bytes(*b"DXT5"));
    put(108, DDSCAPS_COMPLEX_MIPMAP_TEXTURE);
    put(112, DDSCAPS2_CUBEMAP_ALL_FACES);
    for face in &cube.faces {
        for (mip, image) in face.iter().enumerate() {
            let s = mip_size(cube.size, mip);
            for by in 0..s.div_ceil(4) {
                for bx in 0..s.div_ceil(4) {
                    let mut block = [[0u8; 4]; 16];
                    for (i, texel) in block.iter_mut().enumerate() {
                        let (x, y) = ((bx * 4 + i % 4).min(s - 1), (by * 4 + i / 4).min(s - 1));
                        *texel = image[y * s + x];
                    }
                    out.extend_from_slice(&encode_block(&block));
                }
            }
        }
    }
    out.push(srgb);
    // The engine writes the payload length with its option byte as the pitch.
    let pitch = (out.len() - 128) as u32;
    out[20..24].copy_from_slice(&pitch.to_le_bytes());
    out
}

fn rgb565(c: u16) -> [f32; 3] {
    let r = ((c >> 11) & 31) as f32;
    let g = ((c >> 5) & 63) as f32;
    let b = (c & 31) as f32;
    [r * 255.0 / 31.0, g * 255.0 / 63.0, b * 255.0 / 31.0]
}

fn colour_palette(c0: u16, c1: u16) -> [[f32; 3]; 4] {
    // BC3's colour half always interpolates four colours.
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |t: f32| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t);
    [a, b, mix(1.0 / 3.0), mix(2.0 / 3.0)]
}

fn alpha_palette(a0: u8, a1: u8) -> [f32; 8] {
    let (a, b) = (a0 as f32, a1 as f32);
    let mut p = [a, b, 0., 0., 0., 0., 0., 255.];
    if a0 > a1 {
        for i in 1..7 {
            p[i + 1] = ((7 - i) as f32 * a + i as f32 * b) / 7.;
        }
    } else {
        for i in 1..5 {
            p[i + 1] = ((5 - i) as f32 * a + i as f32 * b) / 5.;
        }
    }
    p
}

pub fn decode_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let ap = alpha_palette(block[0], block[1]);
    let abits = u64::from_le_bytes([block[2], block[3], block[4], block[5], block[6], block[7], 0, 0]);
    let c0 = u16::from_le_bytes([block[8], block[9]]);
    let c1 = u16::from_le_bytes([block[10], block[11]]);
    let cp = colour_palette(c0, c1);
    let cbits = u32::from_le_bytes(block[12..16].try_into().unwrap());
    let mut out = [[0u8; 4]; 16];
    for (i, texel) in out.iter_mut().enumerate() {
        let c = cp[((cbits >> (2 * i)) & 3) as usize];
        let a = ap[((abits >> (3 * i)) & 7) as usize];
        *texel = [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, a.round() as u8];
    }
    out
}

fn quantise565(c: [f32; 3]) -> u16 {
    let r = (c[0].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    let g = (c[1].clamp(0.0, 255.0) * 63.0 / 255.0).round() as u16;
    let b = (c[2].clamp(0.0, 255.0) * 31.0 / 255.0).round() as u16;
    (r << 11) | (g << 5) | b
}

fn fit_indices(texels: &[[f32; 3]; 16], c0: u16, c1: u16) -> (u32, f32) {
    let p = colour_palette(c0, c1);
    let mut bits = 0u32;
    let mut error = 0.0;
    for (i, t) in texels.iter().enumerate() {
        let d = |k: usize| (0..3).map(|j| (p[k][j] - t[j]).powi(2)).sum::<f32>();
        let k = (0..4).min_by(|&x, &y| d(x).total_cmp(&d(y))).unwrap();
        error += d(k);
        bits |= (k as u32) << (2 * i);
    }
    (bits, error)
}

/// Order the endpoints for four-colour mode (c0 > c1), swapping indices.
fn ordered(c0: u16, c1: u16, bits: u32) -> (u16, u16, u32) {
    if c0 > c1 {
        return (c0, c1, bits);
    }
    if c0 == c1 {
        return (c0, c1, 0);
    }
    // Swap endpoints: index 0<->1, 2<->3.
    let mut swapped = 0u32;
    for i in 0..16 {
        swapped |= (((bits >> (2 * i)) & 3) ^ 1) << (2 * i);
    }
    (c1, c0, swapped)
}

fn encode_colour(texels: &[[f32; 3]; 16]) -> [u8; 8] {
    let mean = [0, 1, 2].map(|k| texels.iter().map(|t| t[k]).sum::<f32>() / 16.0);
    let mut cov = [[0f32; 3]; 3];
    for t in texels {
        let d = [0, 1, 2].map(|k| t[k] - mean[k]);
        for a in 0..3 {
            for b in 0..3 {
                cov[a][b] += d[a] * d[b];
            }
        }
    }
    let mut axis = [1.0f32, 1.0, 1.0];
    for _ in 0..8 {
        let next = [0, 1, 2].map(|a| (0..3).map(|b| cov[a][b] * axis[b]).sum::<f32>());
        let n = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if n < 1e-12 {
            break;
        }
        axis = next.map(|v| v / n);
    }
    let project = |t: &[f32; 3]| (0..3).map(|k| (t[k] - mean[k]) * axis[k]).sum::<f32>();
    let (lo, hi) = texels.iter().fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(project(t)), hi.max(project(t))));
    let at = |s: f32| [0, 1, 2].map(|k| mean[k] + axis[k] * s);
    let mut best: Option<(u16, u16, u32, f32)> = None;
    let consider = |best: &mut Option<(u16, u16, u32, f32)>, c0: u16, c1: u16| {
        let (bits, error) = fit_indices(texels, c0, c1);
        if best.is_none_or(|b| error < b.3) {
            *best = Some((c0, c1, bits, error));
        }
    };
    let (mut e0, mut e1) = (at(hi), at(lo));
    consider(&mut best, quantise565(e0), quantise565(e1));
    // Least-squares refinement of the endpoints for the chosen indices.
    for _ in 0..2 {
        let (_, _, bits, _) = best.unwrap();
        let weights = [1.0f32, 0.0, 2.0 / 3.0, 1.0 / 3.0];
        let (mut aa, mut bb, mut ab) = (0f32, 0f32, 0f32);
        let (mut ax, mut bx) = ([0f32; 3], [0f32; 3]);
        for (i, t) in texels.iter().enumerate() {
            let w = weights[((bits >> (2 * i)) & 3) as usize];
            let v = 1.0 - w;
            aa += w * w;
            bb += v * v;
            ab += w * v;
            for k in 0..3 {
                ax[k] += w * t[k];
                bx[k] += v * t[k];
            }
        }
        let det = aa * bb - ab * ab;
        if det.abs() < 1e-6 {
            break;
        }
        e0 = [0, 1, 2].map(|k| (ax[k] * bb - bx[k] * ab) / det);
        e1 = [0, 1, 2].map(|k| (bx[k] * aa - ax[k] * ab) / det);
        consider(&mut best, quantise565(e0), quantise565(e1));
    }
    let (c0, c1, bits, _) = best.unwrap();
    let (c0, c1, bits) = ordered(c0, c1, bits);
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&bits.to_le_bytes());
    out
}

fn encode_alpha(alpha: &[u8; 16]) -> [u8; 8] {
    let min = *alpha.iter().min().unwrap();
    let max = *alpha.iter().max().unwrap();
    let (a0, a1) = if max > min { (max, min) } else { (min, min) };
    let p = alpha_palette(a0, a1);
    let mut bits = 0u64;
    for (i, &v) in alpha.iter().enumerate() {
        let d = |k: usize| (p[k] - v as f32).abs();
        let k = (0..8).min_by(|&x, &y| d(x).total_cmp(&d(y))).unwrap();
        bits |= (k as u64) << (3 * i);
    }
    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    out[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
    out
}

pub fn encode_block(texels: &[[u8; 4]; 16]) -> [u8; 16] {
    let colour = texels.map(|t| [t[0] as f32, t[1] as f32, t[2] as f32]);
    let alpha = texels.map(|t| t[3]);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&encode_alpha(&alpha));
    out[8..].copy_from_slice(&encode_colour(&colour));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_round_trip_closely() {
        let mut block = [[0u8; 4]; 16];
        for (i, t) in block.iter_mut().enumerate() {
            *t = [(40 + i * 9) as u8, (90 + i * 5) as u8, (200 - i * 7) as u8, (10 + i * 15) as u8];
        }
        let decoded = decode_block(&encode_block(&block));
        for (a, b) in block.iter().zip(decoded.iter()) {
            for k in 0..3 {
                // Four levels over a 135-wide ramp: up to a sixth of it.
                assert!((a[k] as i32 - b[k] as i32).abs() <= 24, "{a:?} {b:?}");
            }
            assert!((a[3] as i32 - b[3] as i32).abs() <= 18, "{a:?} {b:?}");
        }
        let flat = [[128u8, 64, 32, 200]; 16];
        let decoded = decode_block(&encode_block(&flat));
        for t in decoded {
            assert!((t[0] as i32 - 128).abs() <= 4 && (t[1] as i32 - 64).abs() <= 2 && (t[2] as i32 - 32).abs() <= 4 && t[3] == 200, "{t:?}");
        }
    }

    #[test]
    fn cube_round_trips_its_container() {
        let size = 16;
        let faces = (0..6)
            .map(|f| (0..3).map(|m| vec![[f as u8 * 40, 100, 50, 255]; mip_size(size, m).pow(2)]).collect())
            .collect();
        let cube = CubeRgba8 { size, faces };
        let bytes = encode_cube(&cube, 0);
        assert_eq!(bytes.len(), 2145);
        let back = decode_cube(&bytes).unwrap();
        assert_eq!(back.size, 16);
        assert_eq!(back.mips(), 3);
        assert!((back.faces[5][2][0][0] as i32 - 200).abs() <= 4);
    }
}
