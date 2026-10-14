// SPDX-License-Identifier: GPL-3.0-or-later
//! Re-encode BC2 (DXT3) textures as BC3 (DXT5) for the 0.26g streamer.
//!
//! 0.26g build 2777 creates its texture pool with BC1_TYPELESS and
//! BC3_TYPELESS entries only (pool initialisation at 0x11b090). Streaming
//! registration (`scene::gather_streamable_textures`, check at 0x1550b3)
//! skips any format/dimension key the pool does not hold, so a BC2 texture
//! stays at the 64x64 standalone mip the loader creates for it. Retail 0.26
//! ships no DXT3 texture at all; 0.10b, 0.11e and 0.21d ship a few.
//!
//! BC2 and BC3 share the 8-byte colour half of each block (both always use
//! four-colour interpolation), so colour blocks are copied verbatim. Only the
//! explicit 4-bit alpha half is fitted to a BC3 interpolated alpha block.
//! Dimensions, mip chain, cube faces and the engine's trailing sRGB option
//! byte are retained; the payload length is unchanged.
use anyhow::{Result, ensure};

/// Alpha-test references used by early and native materials (0.25, 0.5).
/// Encoding avoids moving a texel across them whenever possible.
const THRESHOLDS: [f32; 2] = [63.75, 127.5];

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Bc3Report {
    pub blocks: usize,
    pub threshold_crossings: usize,
    pub max_alpha_error: f32,
}

/// Returns `None` for any texture that is not BC2.
pub fn bc2_to_bc3(bytes: &[u8]) -> Result<Option<(Vec<u8>, Bc3Report)>> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || &bytes[84..88] != b"DXT3" {
        return Ok(None);
    }
    let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    ensure!(u32_at(4) == 124, "unexpected DDS header size");
    let (height, width) = (u32_at(12) as usize, u32_at(16) as usize);
    ensure!(width > 0 && height > 0, "empty BC2 texture");
    let mips = u32_at(28).max(1) as usize;
    let caps2 = u32_at(112);
    ensure!(
        caps2 & 0x0020_0000 == 0,
        "volume BC2 textures are unsupported"
    );
    let faces = if caps2 & 0x200 != 0 {
        ensure!(
            caps2 & 0xfe00 == 0xfe00,
            "partial BC2 cube maps are unsupported"
        );
        6
    } else {
        1
    };
    let mut blocks = 0;
    for mip in 0..mips {
        let (w, h) = ((width >> mip).max(1), (height >> mip).max(1));
        blocks += w.div_ceil(4) * h.div_ceil(4);
    }
    blocks *= faces;
    let payload = bytes.len() - 128;
    ensure!(
        payload == blocks * 16 || payload == blocks * 16 + 1,
        "BC2 payload length does not match its header"
    );
    let mut result = bytes.to_vec();
    result[84..88].copy_from_slice(b"DXT5");
    let mut report = Bc3Report {
        blocks,
        ..Bc3Report::default()
    };
    for block in result[128..128 + blocks * 16].chunks_exact_mut(16) {
        let mut alpha = [0u8; 16];
        for (i, value) in alpha.iter_mut().enumerate() {
            *value = ((block[i / 2] >> (4 * (i % 2))) & 15) * 17;
        }
        let (encoded, crossings, error) = encode_alpha(&alpha);
        block[..8].copy_from_slice(&encoded);
        report.threshold_crossings += crossings;
        report.max_alpha_error = report.max_alpha_error.max(error);
    }
    Ok(Some((result, report)))
}

fn palette(a0: u8, a1: u8) -> [f32; 8] {
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

fn side(value: f32) -> [bool; 2] {
    THRESHOLDS.map(|t| value >= t)
}

/// Encode one 4x4 alpha block: (bytes, threshold crossings, max error).
fn encode_alpha(alpha: &[u8; 16]) -> ([u8; 8], usize, f32) {
    let min = *alpha.iter().min().unwrap();
    let max = *alpha.iter().max().unwrap();
    let inner: Vec<u8> = alpha
        .iter()
        .copied()
        .filter(|v| *v != 0 && *v != 255)
        .collect();
    let mut candidates = Vec::new();
    if max > min {
        candidates.push((max, min));
    }
    match (inner.iter().min(), inner.iter().max()) {
        (Some(&lo), Some(&hi)) => candidates.push((lo, hi)),
        _ => candidates.push((min, min)),
    }
    let mut best: Option<([u8; 8], usize, f32, f32)> = None;
    for (a0, a1) in candidates {
        let p = palette(a0, a1);
        let mut bits = 0u64;
        let (mut crossings, mut sse, mut worst) = (0, 0f32, 0f32);
        for (i, &value) in alpha.iter().enumerate() {
            let value = value as f32;
            let distance = |k: usize| (p[k] - value).abs();
            let same_side = (0..8)
                .filter(|&k| side(p[k]) == side(value))
                .min_by(|&x, &y| distance(x).total_cmp(&distance(y)));
            let index = same_side.unwrap_or_else(|| {
                (0..8)
                    .min_by(|&x, &y| distance(x).total_cmp(&distance(y)))
                    .unwrap()
            });
            crossings += usize::from(same_side.is_none());
            sse += distance(index).powi(2);
            worst = worst.max(distance(index));
            bits |= (index as u64) << (3 * i);
        }
        let mut encoded = [0u8; 8];
        encoded[0] = a0;
        encoded[1] = a1;
        encoded[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
        if best.as_ref().is_none_or(|b| (crossings, sse) < (b.1, b.3)) {
            best = Some((encoded, crossings, worst, sse));
        }
    }
    let (encoded, crossings, worst, _) = best.unwrap();
    (encoded, crossings, worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_bc3_alpha(block: &[u8]) -> [f32; 16] {
        let p = palette(block[0], block[1]);
        let mut bits = [0u8; 8];
        bits[..6].copy_from_slice(&block[2..8]);
        let bits = u64::from_le_bytes(bits);
        std::array::from_fn(|i| p[((bits >> (3 * i)) & 7) as usize])
    }

    fn dds(
        width: u32,
        height: u32,
        mips: u32,
        caps2: u32,
        blocks: &[[u8; 16]],
        srgb: bool,
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124),
            (12, height),
            (16, width),
            (28, mips),
            (112, caps2),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[84..88].copy_from_slice(b"DXT3");
        for block in blocks {
            bytes.extend(block);
        }
        if srgb {
            bytes.push(1);
        }
        bytes
    }

    fn bc2_block(alpha: [u8; 16], color: [u8; 8]) -> [u8; 16] {
        let mut block = [0u8; 16];
        for (i, a) in alpha.iter().enumerate() {
            block[i / 2] |= (a & 15) << (4 * (i % 2));
        }
        block[8..].copy_from_slice(&color);
        block
    }

    #[test]
    fn colour_blocks_and_length_are_kept_and_cutout_alpha_stays_on_its_side() {
        // The remains textures use alpha levels 0..7 and 15 (cutouts with
        // alpha_ref 0.25): every level must stay below/above 0.25 and 0.5.
        let cutout = [0, 1, 2, 3, 4, 5, 6, 7, 15, 15, 15, 15, 0, 7, 15, 3];
        let ramp: [u8; 16] = std::array::from_fn(|i| i as u8);
        let flat = [9u8; 16];
        let colour = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];
        let blocks: Vec<_> = [cutout, ramp, flat]
            .iter()
            .cycle()
            .take(4 + 1 + 1)
            .map(|a| bc2_block(*a, colour))
            .collect();
        // 8x8 with three mips: 4 + 1 + 1 blocks, plus the sRGB option byte.
        let input = dds(8, 8, 3, 0, &blocks, true);
        let (output, report) = bc2_to_bc3(&input).unwrap().unwrap();
        assert_eq!(output.len(), input.len());
        assert_eq!(&output[84..88], b"DXT5");
        assert_eq!(output[..84], input[..84]);
        assert_eq!(output[88..128], input[88..128]);
        assert_eq!(output.last(), Some(&1));
        assert_eq!(report.blocks, 6);
        assert_eq!(report.threshold_crossings, 0);
        for (i, (out, src)) in output[128..128 + 96]
            .chunks_exact(16)
            .zip(blocks.iter())
            .enumerate()
        {
            assert_eq!(&out[8..], &src[8..], "colour block {i}");
            let decoded = decode_bc3_alpha(out);
            let source: [u8; 16] =
                std::array::from_fn(|t| ((src[t / 2] >> (4 * (t % 2))) & 15) * 17);
            for (d, s) in decoded.iter().zip(source) {
                assert_eq!(side(*d), side(s as f32));
                // Endpoints are exact; interpolated levels are within half a
                // BC3 step of the widest possible block (255 / 7 / 2).
                assert!((d - s as f32).abs() <= 255. / 14. + 1e-3);
            }
        }
        // Flat and 0/255 values are exact.
        assert!(
            decode_bc3_alpha(&output[128 + 2 * 16..])
                .iter()
                .all(|v| *v == 9. * 17.)
        );
    }

    #[test]
    fn cube_maps_are_walked_per_face_and_other_formats_are_untouched() {
        let block = bc2_block([15; 16], [1; 8]);
        // 4x4, two mips (4x4, 2x2 -> one block each), six faces.
        let cube = dds(4, 4, 2, 0xfe00, &vec![block; 12], false);
        let (output, report) = bc2_to_bc3(&cube).unwrap().unwrap();
        assert_eq!(report.blocks, 12);
        assert_eq!(output.len(), cube.len());
        assert!(bc2_to_bc3(&cube[..cube.len() - 16]).is_err());
        let mut dxt5 = cube.clone();
        dxt5[84..88].copy_from_slice(b"DXT5");
        assert!(bc2_to_bc3(&dxt5).unwrap().is_none());
    }
}
