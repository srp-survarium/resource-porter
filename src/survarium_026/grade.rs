// SPDX-License-Identifier: GPL-3.0-or-later
//! A deterministic colour grade of a block-compressed diffuse texture.
//!
//! The grade c' = mean + contrast * (L + saturation * (c - L) - mean), with L
//! the Rec. 709 luminance of c and `mean` the texture's mean luminance, is
//! affine in RGB, and BC1 interpolates its palette linearly between the two
//! block endpoints. Grading the endpoints therefore grades every texel of
//! every mip exactly, up to 5:6:5 rounding, without re-encoding the indices.
//! A block whose endpoint order flips is swapped and its indices remapped so
//! it keeps its 4-colour or 3-colour (transparent) mode.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    /// 1 keeps colour, 0 is greyscale.
    pub saturation: f32,
    /// 1 keeps contrast around the texture's mean luminance.
    pub contrast: f32,
    /// Why, for the build record.
    pub reason: String,
}

fn unpack(c: u16) -> [f32; 3] {
    [
        ((c >> 11) & 31) as f32 / 31.0,
        ((c >> 5) & 63) as f32 / 63.0,
        (c & 31) as f32 / 31.0,
    ]
}

fn pack(c: [f32; 3]) -> u16 {
    let q = |v: f32, m: f32| (v.clamp(0.0, 1.0) * m).round() as u16;
    (q(c[0], 31.0) << 11) | (q(c[1], 63.0) << 5) | q(c[2], 31.0)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Grade a DXT1 or DXT5 DDS (2D, any mip count). Returns None for other
/// formats.
pub fn grade_dds(bytes: &[u8], grade: &Grade) -> Result<Option<Vec<u8>>> {
    ensure!(bytes.len() > 128 && &bytes[..4] == b"DDS ", "not a DDS");
    ensure!(
        grade.saturation.is_finite() && grade.contrast.is_finite()
            && (0.0..=1.5).contains(&grade.saturation) && (0.0..=1.5).contains(&grade.contrast),
        "grade out of range"
    );
    let (block, colour_at) = match &bytes[84..88] {
        b"DXT1" => (8usize, 0usize),
        b"DXT5" => (16, 8),
        _ => return Ok(None),
    };
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    ensure!(u32_at(112) & 0xfe00 == 0, "cube maps are not graded");
    let (h, w, mips) = (u32_at(12), u32_at(16), u32_at(28).max(1));
    let mut size = 0;
    for mip in 0..mips {
        size += ((w >> mip).max(1)).div_ceil(4) * ((h >> mip).max(1)).div_ceil(4) * block;
    }
    ensure!(bytes.len() >= 128 + size, "DDS shorter than its mip chain");
    let blocks = &bytes[128..128 + size];
    // Mean luminance of the top mip (mean of block endpoint midpoints).
    let top = w.div_ceil(4) * h.div_ceil(4);
    let mut sum = 0.0f64;
    for b in blocks.chunks_exact(block).take(top) {
        let c0 = unpack(u16::from_le_bytes([b[colour_at], b[colour_at + 1]]));
        let c1 = unpack(u16::from_le_bytes([b[colour_at + 2], b[colour_at + 3]]));
        sum += 0.5 * f64::from(luma(c0) + luma(c1));
    }
    let mean = (sum / top as f64) as f32;
    let apply = |c: [f32; 3]| {
        let l = luma(c);
        c.map(|v| mean + grade.contrast * (l + grade.saturation * (v - l) - mean))
    };
    let mut out = bytes.to_vec();
    for b in out[128..128 + size].chunks_exact_mut(block) {
        let b = &mut b[colour_at..colour_at + 8];
        let (o0, o1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
        let four = o0 > o1;
        let (mut n0, mut n1) = (pack(apply(unpack(o0))), pack(apply(unpack(o1))));
        let mut indices = u32::from_le_bytes(b[4..8].try_into().unwrap());
        let swap = if four { n0 < n1 } else { n0 > n1 };
        if swap {
            std::mem::swap(&mut n0, &mut n1);
            let mut remapped = 0u32;
            for i in 0..16 {
                let k = (indices >> (2 * i)) & 3;
                let k = if four { [1, 0, 3, 2][k as usize] } else { [1, 0, 2, 3][k as usize] };
                remapped |= k << (2 * i);
            }
            indices = remapped;
        }
        if four && n0 == n1 {
            // Collapsed to one colour: 3-colour mode, every texel index 0.
            indices = 0;
        }
        b[0..2].copy_from_slice(&n0.to_le_bytes());
        b[2..4].copy_from_slice(&n1.to_le_bytes());
        b[4..8].copy_from_slice(&indices.to_le_bytes());
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dds(blocks: &[[u8; 8]]) -> Vec<u8> {
        let mut d = vec![0u8; 128];
        d[..4].copy_from_slice(b"DDS ");
        d[12..16].copy_from_slice(&4u32.to_le_bytes());
        d[16..20].copy_from_slice(&(4 * blocks.len() as u32).to_le_bytes());
        d[28..32].copy_from_slice(&1u32.to_le_bytes());
        d[84..88].copy_from_slice(b"DXT1");
        for b in blocks {
            d.extend(b);
        }
        d.push(1);
        d
    }

    #[test]
    fn grade_desaturates_and_keeps_block_modes() {
        let red = pack([0.9, 0.2, 0.1]);
        let grey = pack([0.4, 0.4, 0.4]);
        let (hi, lo) = if red > grey { (red, grey) } else { (grey, red) };
        let mut four = [0u8; 8];
        four[0..2].copy_from_slice(&hi.to_le_bytes());
        four[2..4].copy_from_slice(&lo.to_le_bytes());
        four[4..8].copy_from_slice(&0x1b1b_1b1bu32.to_le_bytes());
        let input = dds(&[four]);
        let g = Grade { saturation: 0.5, contrast: 0.8, reason: String::new() };
        let out = grade_dds(&input, &g).unwrap().unwrap();
        assert_eq!(out.len(), input.len());
        assert_eq!(out[..128], input[..128]);
        assert_eq!(*out.last().unwrap(), 1);
        let c0 = u16::from_le_bytes([out[128], out[129]]);
        let c1 = u16::from_le_bytes([out[130], out[131]]);
        assert!(c0 > c1, "4-colour mode kept");
        // Chroma of the red endpoint shrinks.
        let chroma = |c: [f32; 3]| c.iter().cloned().fold(0.0f32, f32::max) - c.iter().cloned().fold(1.0f32, f32::min);
        let before = chroma(unpack(red));
        let after = [unpack(c0), unpack(c1)].into_iter().map(chroma).fold(0.0f32, f32::max);
        assert!(after < before * 0.7, "{after} vs {before}");
        // Identity grade is a no-op.
        let id = Grade { saturation: 1.0, contrast: 1.0, reason: String::new() };
        assert_eq!(grade_dds(&input, &id).unwrap().unwrap(), input);
        let mut dxt3 = input.clone();
        dxt3[84..88].copy_from_slice(b"DXT3");
        assert!(grade_dds(&dxt3, &g).unwrap().is_none());
    }
}
