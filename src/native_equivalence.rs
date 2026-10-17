// SPDX-License-Identifier: GPL-3.0-or-later
//! Donor textures that 0.26g already has.
//!
//! Of the 803 textures RLS Vostok and Mamayev Kurgan use, 0.26g ships 768
//! under the same path: 408 byte-identical; 178 with the same image at full
//! resolution (header and top mip level identical, the trailing colour-space
//! byte and the `.options` too) whose smaller mip levels 0.26 regenerated;
//! 42 that 0.26 re-flagged from sRGB to linear; 10 of the same image with
//! other `.options`; 130 re-encoded or re-authored (117 differ by less than
//! 35 dB PSNR at full resolution, 5 by 35 to 45 dB, 8 have another format or
//! size). Importing the first two kinds copies 0.26g's own textures into the
//! namespace (773 MB); a map references the native ones instead.
//!
//! The others stay the donor's: the colour space and the conversion options
//! change what the shader samples, and re-encoded or re-authored art is the
//! donor's art.
use serde::Serialize;

/// Why a donor texture may be replaced by the target's of the same path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureMatch {
    /// The `.dds` and the `.options` are byte-identical.
    Identical,
    /// The same size, header, full-resolution level, colour space and
    /// `.options`; only smaller mip levels differ (0.26 regenerated them).
    /// RLS Vostok's and Mamayev Kurgan's 178: identical colour in every mip
    /// level for 122, colour PSNR at least 42 dB in every level for the
    /// other 56; alpha differs only in the smaller levels.
    SameTopLevel,
}

/// Header length and the byte size of the full-resolution level.
fn top_level(dds: &[u8]) -> Option<(usize, usize)> {
    if dds.len() < 128 || &dds[..4] != b"DDS " {
        return None;
    }
    let u32_at = |at: usize| u32::from_le_bytes(dds[at..at + 4].try_into().unwrap()) as usize;
    let (height, width) = (u32_at(12), u32_at(16));
    let flags = u32_at(80);
    let fourcc = &dds[84..88];
    let blocks = width.div_ceil(4).max(1) * height.div_ceil(4).max(1);
    let (header, size) = match fourcc {
        b"DXT1" | b"ATI1" | b"BC4U" | b"BC4S" => (128, blocks * 8),
        b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" | b"BC5S" => (128, blocks * 16),
        _ if flags & 0x4 == 0 => {
            let bits = u32_at(88);
            (bits % 8 == 0 && bits > 0).then_some((128, width * height * bits / 8))?
        }
        _ => return None,
    };
    (header + size < dds.len()).then_some((header, size))
}

/// Whether a donor texture (its `.dds` and `.options`, if any) is the
/// target's of the same path.
pub fn texture_match(
    donor_dds: &[u8],
    donor_options: Option<&[u8]>,
    target_dds: &[u8],
    target_options: Option<&[u8]>,
) -> Option<TextureMatch> {
    if donor_options != target_options {
        return None;
    }
    if donor_dds == target_dds {
        return Some(TextureMatch::Identical);
    }
    let (header, size) = top_level(donor_dds)?;
    (donor_dds.len() == target_dds.len()
        && donor_dds[..header + size] == target_dds[..header + size]
        && donor_dds.last() == target_dds.last())
    .then_some(TextureMatch::SameTopLevel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8x8 DXT1 texture with two mip levels, `level0` and `level1` bytes,
    /// then the archive's trailing colour-space byte.
    fn dxt1(level0: u8, level1: u8, srgb: u8) -> Vec<u8> {
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&8u32.to_le_bytes());
        dds[16..20].copy_from_slice(&8u32.to_le_bytes());
        dds[28..32].copy_from_slice(&2u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend(std::iter::repeat_n(level0, 4 * 8));
        dds.extend(std::iter::repeat_n(level1, 8));
        dds.push(srgb);
        dds
    }

    #[test]
    fn regenerated_mips_match_but_colour_space_and_art_do_not() {
        let options = Some(&b"opts"[..]);
        let native = dxt1(1, 2, 0);
        assert_eq!(
            texture_match(&native, options, &native, options),
            Some(TextureMatch::Identical)
        );
        assert_eq!(
            texture_match(&dxt1(1, 9, 0), options, &native, options),
            Some(TextureMatch::SameTopLevel)
        );
        // 0.26 re-flagged it linear: the shader samples other values.
        assert_eq!(
            texture_match(&dxt1(1, 2, 1), options, &native, options),
            None
        );
        // Different art at full resolution.
        assert_eq!(
            texture_match(&dxt1(7, 2, 0), options, &native, options),
            None
        );
        // Different conversion options.
        assert_eq!(
            texture_match(&native, Some(&b"srgb"[..]), &native, options),
            None
        );
    }
}
