//! 0.69d environment probes re-baked for 0.26.
//!
//! A 0.69d probe is a DXT5 cubemap in RGBM (colour normalised to its largest
//! channel, the magnitude in alpha: radiance = rgb * a^2) plus nine SH
//! coefficients of its radiance; 0.69d lights diffuse surfaces from the SH and
//! has no `_diffuse` cubemap. 0.26 reads a DXT5 cubemap whose texels at or
//! below 1 are stored as they are (alpha 1) and brighter ones divided by their
//! largest channel (alpha its reciprocal), and a 16x16 `_diffuse` cubemap of
//! irradiance over pi (0.26's own and 0.46c2's probes check out so: the
//! `_diffuse` is the cosine-weighted mean of the cubemap, to 3 %).
//!
//! The SH basis is the real one with the Condon-Shortley phase over the
//! world axes (checked on the lobby's probes: their L1 band points where
//! their cubemaps are brightest), the faces in D3D order (+x -x +y -y +z -z).

use anyhow::{Result, bail, ensure};

/// One decoded cubemap: six faces of `mips` levels, linear RGB.
pub struct Cube {
    pub size: usize,
    pub mips: usize,
    /// faces[face][mip] = size>>mip squared texels.
    pub faces: Vec<Vec<Vec<[f32; 3]>>>,
}

fn header(bytes: &[u8]) -> Result<(usize, usize, usize)> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS");
    let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    ensure!(&bytes[84..88] == b"DXT5", "probe cubemap is not DXT5");
    ensure!(u(112) & 0xfe00 == 0xfe00, "not a full cubemap");
    Ok((u(16), u(12), u(28).max(1)))
}

fn c565(c: u16) -> [f32; 3] {
    [((c >> 11) & 31) as f32 / 31.0, ((c >> 5) & 63) as f32 / 63.0, (c & 31) as f32 / 31.0]
}

fn decode_block(b: &[u8]) -> [[f32; 4]; 16] {
    let (a0, a1) = (b[0] as f32, b[1] as f32);
    let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
    let ap: [f32; 8] = if a0 > a1 {
        std::array::from_fn(|k| match k {
            0 => a0,
            1 => a1,
            k => ((8 - k) as f32 * a0 + (k - 1) as f32 * a1) / 7.0,
        })
    } else {
        std::array::from_fn(|k| match k {
            0 => a0,
            1 => a1,
            6 => 0.0,
            7 => 255.0,
            k => ((6 - k) as f32 * a0 + (k - 1) as f32 * a1) / 5.0,
        })
    };
    let c0 = c565(u16::from_le_bytes([b[8], b[9]]));
    let c1 = c565(u16::from_le_bytes([b[10], b[11]]));
    let cp = [c0, c1, std::array::from_fn(|i| (2.0 * c0[i] + c1[i]) / 3.0), std::array::from_fn(|i| (c0[i] + 2.0 * c1[i]) / 3.0)];
    let cbits = u32::from_le_bytes(b[12..16].try_into().unwrap());
    std::array::from_fn(|i| {
        let c = cp[((cbits >> (2 * i)) & 3) as usize];
        [c[0], c[1], c[2], ap[((bits >> (3 * i)) & 7) as usize] / 255.0]
    })
}

fn level_bytes(n: usize) -> usize {
    n.div_ceil(4).max(1).pow(2) * 16
}

/// Decode a 0.69d RGBM probe cubemap (radiance = rgb * a^2).
pub fn decode_rgbm(bytes: &[u8]) -> Result<Cube> {
    let (w, h, mips) = header(bytes)?;
    ensure!(w == h && w.is_power_of_two(), "probe cubemap {w}x{h}");
    let mut off = 128;
    let mut faces = Vec::new();
    for _ in 0..6 {
        let mut levels = Vec::new();
        for m in 0..mips {
            let n = (w >> m).max(1);
            let len = level_bytes(n);
            ensure!(bytes.len() >= off + len, "truncated cubemap");
            let bw = n.div_ceil(4);
            let mut texels = vec![[0.0; 3]; n * n];
            for by in 0..bw {
                for bx in 0..bw {
                    let block = decode_block(&bytes[off + (by * bw + bx) * 16..]);
                    for (i, p) in block.iter().enumerate() {
                        let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                        if x < n && y < n {
                            let a2 = p[3] * p[3];
                            texels[y * n + x] = [p[0] * a2, p[1] * a2, p[2] * a2];
                        }
                    }
                }
            }
            off += len;
            levels.push(texels);
        }
        faces.push(levels);
    }
    Ok(Cube { size: w, mips, faces })
}

/// Unit direction through the centre of texel (x, y) of a face of `n` texels.
pub fn direction(face: usize, n: usize, x: usize, y: usize) -> [f32; 3] {
    let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
    let v = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
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

fn solid_angle(n: usize, x: usize, y: usize) -> f32 {
    let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
    let v = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
    (2.0 / n as f32).powi(2) / (1.0 + u * u + v * v).powf(1.5)
}

/// Irradiance over pi in direction `d` from radiance SH (9 RGB coefficients).
pub fn sh_irradiance(sh: &[[f32; 3]; 9], d: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = d;
    // Basis with the Condon-Shortley phase, times the cosine lobe's band
    // factors (pi, 2pi/3, pi/4), over pi.
    let b = [
        0.282095,
        -0.488603 * y * (2.0 / 3.0),
        0.488603 * z * (2.0 / 3.0),
        -0.488603 * x * (2.0 / 3.0),
        1.092548 * x * y * 0.25,
        -1.092548 * y * z * 0.25,
        0.315392 * (3.0 * z * z - 1.0) * 0.25,
        -1.092548 * x * z * 0.25,
        0.546274 * (x * x - y * y) * 0.25,
    ];
    std::array::from_fn(|c| (0..9).map(|k| sh[k][c] * b[k]).sum::<f32>().max(0.0))
}

/// Mean over the sphere of a cubemap level (solid-angle weighted).
pub fn mean(cube: &Cube, mip: usize) -> [f32; 3] {
    let n = (cube.size >> mip).max(1);
    let mut acc = [0.0f32; 3];
    let mut w = 0.0;
    for face in &cube.faces {
        for y in 0..n {
            for x in 0..n {
                let s = solid_angle(n, x, y);
                let t = face[mip][y * n + x];
                for c in 0..3 {
                    acc[c] += t[c] * s;
                }
                w += s;
            }
        }
    }
    acc.map(|v| v / w)
}

fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The `_diffuse` cubemap (16 texels, 3 mips) from SH, and its scale: the
/// texels are irradiance over pi divided by the scale (the brightest is 1).
pub fn diffuse_from_sh(sh: &[[f32; 3]; 9]) -> (Cube, f32) {
    let (size, mips) = (16, 3);
    let mut faces = Vec::new();
    for f in 0..6 {
        let mut levels = Vec::new();
        for m in 0..mips {
            let n = size >> m;
            levels.push((0..n * n).map(|i| sh_irradiance(sh, direction(f, n, i % n, i / n))).collect::<Vec<_>>());
        }
        faces.push(levels);
    }
    let peak = faces.iter().flatten().flatten().flat_map(|t| t.iter().copied()).fold(0.0f32, f32::max).max(1e-6);
    for t in faces.iter_mut().flatten().flatten() {
        *t = t.map(|v| v / peak);
    }
    (Cube { size, mips, faces }, peak)
}

/// Scale a cubemap in place.
pub fn scale(cube: &mut Cube, k: f32) {
    for t in cube.faces.iter_mut().flatten().flatten() {
        *t = t.map(|v| v * k);
    }
}

/// The scale that makes the cubemap's mean luminance `target`.
pub fn match_mean(cube: &Cube, target: [f32; 3]) -> f32 {
    let m = luminance(mean(cube, cube.mips - 1));
    if m <= 0.0 { 1.0 } else { luminance(target) / m }
}

fn q565(c: [f32; 3]) -> u16 {
    let r = (c[0].clamp(0.0, 1.0) * 31.0).round() as u16;
    let g = (c[1].clamp(0.0, 1.0) * 63.0).round() as u16;
    let b = (c[2].clamp(0.0, 1.0) * 31.0).round() as u16;
    (r << 11) | (g << 5) | b
}

fn encode_block(px: &[[f32; 4]; 16], out: &mut Vec<u8>) {
    // Alpha: an 8-value ramp between its extremes.
    let amax = px.iter().map(|p| (p[3] * 255.0).round() as i32).max().unwrap().clamp(0, 255);
    let amin = px.iter().map(|p| (p[3] * 255.0).round() as i32).min().unwrap().clamp(0, 255);
    out.push(amax as u8);
    out.push(amin as u8);
    let mut bits = 0u64;
    if amax > amin {
        let pal: [f32; 8] = std::array::from_fn(|k| match k {
            0 => amax as f32,
            1 => amin as f32,
            k => ((8 - k) as f32 * amax as f32 + (k - 1) as f32 * amin as f32) / 7.0,
        });
        for (i, p) in px.iter().enumerate() {
            let a = p[3] * 255.0;
            let k = (0..8).min_by(|&x, &y| (pal[x] - a).abs().total_cmp(&(pal[y] - a).abs())).unwrap();
            bits |= (k as u64) << (3 * i);
        }
    }
    out.extend_from_slice(&bits.to_le_bytes()[..6]);
    // Colour: endpoints at the block's bounds, four-colour mode.
    let lo: [f32; 3] = std::array::from_fn(|c| px.iter().map(|p| p[c]).fold(f32::MAX, f32::min));
    let hi: [f32; 3] = std::array::from_fn(|c| px.iter().map(|p| p[c]).fold(f32::MIN, f32::max));
    let (mut c0, mut c1) = (q565(hi), q565(lo));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut cbits = 0u32;
    if c0 != c1 {
        let (p0, p1) = (c565(c0), c565(c1));
        let pal = [p0, p1, std::array::from_fn(|i| (2.0 * p0[i] + p1[i]) / 3.0), std::array::from_fn(|i| (p0[i] + 2.0 * p1[i]) / 3.0)];
        for (i, p) in px.iter().enumerate() {
            let d = |k: usize| (0..3).map(|c| (pal[k][c] - p[c]).powi(2)).sum::<f32>();
            let k = (0..4).min_by(|&x, &y| d(x).total_cmp(&d(y))).unwrap();
            cbits |= (k as u32) << (2 * i);
        }
    }
    out.extend_from_slice(&c0.to_le_bytes());
    out.extend_from_slice(&c1.to_le_bytes());
    out.extend_from_slice(&cbits.to_le_bytes());
}

/// Encode for 0.26 (texels above 1 divided by their largest channel, alpha
/// its reciprocal), with `header` (a 0.26 cubemap of the same size and mips).
pub fn encode_026(cube: &Cube, header: &[u8]) -> Result<Vec<u8>> {
    let (w, _, mips) = self::header(header)?;
    if w != cube.size || mips != cube.mips {
        bail!("header {w} px / {mips} mips for a {} px / {} mips cubemap", cube.size, cube.mips);
    }
    let mut out = header[..128].to_vec();
    for face in &cube.faces {
        for (m, level) in face.iter().enumerate() {
            let n = (cube.size >> m).max(1);
            let bw = n.div_ceil(4);
            for by in 0..bw {
                for bx in 0..bw {
                    let px: [[f32; 4]; 16] = std::array::from_fn(|i| {
                        let (x, y) = ((bx * 4 + i % 4).min(n - 1), (by * 4 + i / 4).min(n - 1));
                        let t = level[y * n + x];
                        let peak = t[0].max(t[1]).max(t[2]);
                        if peak > 1.0 { [t[0] / peak, t[1] / peak, t[2] / peak, 1.0 / peak] } else { [t[0], t[1], t[2], 1.0] }
                    });
                    encode_block(&px, &mut out);
                }
            }
        }
    }
    Ok(out)
}

/// A DDS header for a DXT5 cubemap of `size` px and `mips` levels, from a
/// 0.26 cubemap's header (only size, mips and linear size change).
pub fn cube_header(template: &[u8], size: usize, mips: usize) -> Result<Vec<u8>> {
    header(template)?;
    let mut h = template[..128].to_vec();
    h[12..16].copy_from_slice(&(size as u32).to_le_bytes());
    h[16..20].copy_from_slice(&(size as u32).to_le_bytes());
    h[20..24].copy_from_slice(&(level_bytes(size) as u32).to_le_bytes());
    h[28..32].copy_from_slice(&(mips as u32).to_le_bytes());
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_sh_gives_flat_irradiance() {
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0 / 0.282095, 0.5 / 0.282095, 0.25 / 0.282095];
        let (cube, peak) = diffuse_from_sh(&sh);
        assert!((peak - 1.0).abs() < 1e-4);
        let m = mean(&cube, 0);
        assert!((m[0] - 1.0).abs() < 1e-3 && (m[1] - 0.5).abs() < 1e-3 && (m[2] - 0.25).abs() < 1e-3);
    }

    #[test]
    fn l1_band_lights_its_side() {
        // Condon-Shortley phase: coefficient 3 is -x.
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0; 3];
        sh[3] = [-1.0; 3];
        let px = sh_irradiance(&sh, [1.0, 0.0, 0.0]);
        let nx = sh_irradiance(&sh, [-1.0, 0.0, 0.0]);
        assert!(px[0] > nx[0]);
    }

    #[test]
    fn encode_round_trips() {
        let template = {
            let mut h = vec![0u8; 128];
            h[..4].copy_from_slice(b"DDS ");
            h[84..88].copy_from_slice(b"DXT5");
            h[112..116].copy_from_slice(&0xfe00u32.to_le_bytes());
            h
        };
        let header = cube_header(&template, 4, 1).unwrap();
        let cube = Cube { size: 4, mips: 1, faces: (0..6).map(|f| vec![vec![[0.1 * f as f32, 0.5, 2.0]; 16]]).collect() };
        let bytes = encode_026(&cube, &header).unwrap();
        assert_eq!(bytes.len(), 128 + 6 * 16);
        // texel 2.0 in blue: rgb divided by 2, alpha 1/2
        let block = decode_block(&bytes[128 + 16 * 3..]);
        assert!((block[0][2] - 1.0).abs() < 0.02 && (block[0][3] - 0.5).abs() < 0.02);
    }
}
