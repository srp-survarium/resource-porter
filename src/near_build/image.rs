// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether two builds' files of one texture show the same image. The decoder
//! and the test are the 0.69d port's (`survarium_026/later.rs` of the Rudnya
//! branch, where 0.69d's re-encoded textures were told from its retouched
//! ones); the thresholds are unchanged.

use anyhow::{Result, bail, ensure};

fn dds_info(bytes: &[u8]) -> Result<(u32, u32, u32, [u8; 4])> {
    ensure!(bytes.len() >= 128 && &bytes[..4] == b"DDS ", "not a DDS");
    let get = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let mut fourcc = [0; 4];
    fourcc.copy_from_slice(&bytes[84..88]);
    Ok((get(16), get(12), get(28).max(1), fourcc))
}

/// Decode one BC1/BC3 texel block row-major into RGBA (top mip only).
fn decode_bc(bytes: &[u8]) -> Result<(usize, usize, Vec<[u8; 4]>)> {
    let (width, height, _, fourcc) = dds_info(bytes)?;
    let (w, h) = (width as usize, height as usize);
    let block = match &fourcc {
        b"DXT1" => 8,
        b"DXT5" => 16,
        _ => bail!("cannot decode {:?}", String::from_utf8_lossy(&fourcc)),
    };
    ensure!(w % 4 == 0 && h % 4 == 0, "odd texture size");
    ensure!(bytes.len() >= 128 + (w / 4) * (h / 4) * block, "truncated texture");
    let mut out = vec![[0u8; 4]; w * h];
    let c565 = |c: u16| {
        let r = ((c >> 11) & 31) as u32;
        let g = ((c >> 5) & 63) as u32;
        let b = (c & 31) as u32;
        [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
    };
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            let o = 128 + (by * (w / 4) + bx) * block;
            let mut alpha = [255u8; 16];
            let co = if block == 16 {
                let a0 = bytes[o] as u32;
                let a1 = bytes[o + 1] as u32;
                let bits = u64::from_le_bytes([
                    bytes[o + 2], bytes[o + 3], bytes[o + 4], bytes[o + 5], bytes[o + 6], bytes[o + 7], 0, 0,
                ]);
                let palette: [u32; 8] = if a0 > a1 {
                    [a0, a1, (6 * a0 + a1) / 7, (5 * a0 + 2 * a1) / 7, (4 * a0 + 3 * a1) / 7, (3 * a0 + 4 * a1) / 7, (2 * a0 + 5 * a1) / 7, (a0 + 6 * a1) / 7]
                } else {
                    [a0, a1, (4 * a0 + a1) / 5, (3 * a0 + 2 * a1) / 5, (2 * a0 + 3 * a1) / 5, (a0 + 4 * a1) / 5, 0, 255]
                };
                for (i, a) in alpha.iter_mut().enumerate() {
                    *a = palette[((bits >> (3 * i)) & 7) as usize] as u8;
                }
                o + 8
            } else {
                o
            };
            let c0 = u16::from_le_bytes([bytes[co], bytes[co + 1]]);
            let c1 = u16::from_le_bytes([bytes[co + 2], bytes[co + 3]]);
            let bits = u32::from_le_bytes(bytes[co + 4..co + 8].try_into().unwrap());
            let (p0, p1) = (c565(c0), c565(c1));
            let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
            let palette: [[u8; 3]; 4] = if c0 > c1 || block == 16 {
                [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 2, 1)), std::array::from_fn(|i| mix(p0[i], p1[i], 1, 2))]
            } else {
                [p0, p1, std::array::from_fn(|i| mix(p0[i], p1[i], 1, 1)), [0, 0, 0]]
            };
            for i in 0..16 {
                let p = palette[((bits >> (2 * i)) & 3) as usize];
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                out[y * w + x] = [p[0], p[1], p[2], alpha[i]];
            }
        }
    }
    Ok((w, h, out))
}

/// Whether a donor texture shows the same image as the file players have:
/// the same size, and per 4x4 block (one compressed block, whose mean colour
/// a re-encode of the same source keeps) the mean RGBA within 2 levels on
/// average, no channel's mean over the image moved by 1.5 or more, and 99% of
/// blocks within 12 levels in every channel (an edit painted into a corner is
/// not the same image). A format the decoder does not read (anything but
/// DXT1 and DXT5) is the same image only when the bytes are.
pub fn same_image(donor: &[u8], native: &[u8]) -> Result<bool> {
    let (Ok((w, h, a)), Ok((nw, nh, b))) = (decode_bc(donor), decode_bc(native)) else {
        return Ok(false);
    };
    if (w, h) != (nw, nh) {
        return Ok(false);
    }
    let blocks = (w / 4) * (h / 4);
    ensure!(blocks > 0, "empty texture");
    let mut block_max = Vec::with_capacity(blocks);
    let (mut sum, mut means) = (0.0f64, [0.0f64; 8]);
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            let mut m = [0.0f32; 8];
            for i in 0..16 {
                let o = (by * 4 + i / 4) * w + bx * 4 + i % 4;
                for c in 0..4 {
                    m[c] += f32::from(a[o][c]) / 16.0;
                    m[4 + c] += f32::from(b[o][c]) / 16.0;
                }
            }
            let d: [f32; 4] = std::array::from_fn(|c| (m[c] - m[4 + c]).abs());
            sum += f64::from(d.iter().sum::<f32>()) / 4.0;
            block_max.push(d.iter().copied().fold(0.0f32, f32::max));
            for (acc, v) in means.iter_mut().zip(m) {
                *acc += f64::from(v);
            }
        }
    }
    let n = blocks as f64;
    let shift = (0..4).map(|c| (means[c] - means[4 + c]).abs() / n).fold(0.0f64, f64::max);
    block_max.sort_by(f32::total_cmp);
    let p99 = block_max[(blocks - 1) * 99 / 100];
    Ok(sum / n < 2.0 && shift < 1.5 && p99 < 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_dxt1(colours: [u16; 4]) -> Vec<u8> {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&8u32.to_le_bytes());
        dds[16..20].copy_from_slice(&8u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        for c in colours {
            dds.extend_from_slice(&c.to_le_bytes());
            dds.extend_from_slice(&c.to_le_bytes());
            dds.extend_from_slice(&[0; 4]);
        }
        dds
    }

    #[test]
    fn a_re_encode_is_the_same_image_and_a_repaint_is_not() {
        let grey = 0b10000_100000_10000u16;
        let nearly = 0b10000_100001_10000u16;
        let red = 0b11111_000000_00000u16;
        assert!(same_image(&flat_dxt1([grey; 4]), &flat_dxt1([nearly, grey, grey, grey])).unwrap());
        assert!(!same_image(&flat_dxt1([grey; 4]), &flat_dxt1([grey, grey, grey, red])).unwrap());
    }
}
