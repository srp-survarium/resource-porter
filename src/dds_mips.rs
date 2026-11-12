// SPDX-License-Identifier: GPL-3.0-or-later
//! Select existing BC1/BC3 mips that fit 0.26's native streaming pools.
use anyhow::{Result, ensure};
use serde::Deserialize;

/// An explicit native pool size; non-streamed sky textures need no such limit.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "u32")]
pub struct NativeMipLimit(u32);

impl TryFrom<u32> for NativeMipLimit {
    type Error = String;
    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        if !(4..=2048).contains(&value) || !value.is_power_of_two() {
            return Err("native mip limit must be a power of two from 4 through 2048".into());
        }
        Ok(Self(value))
    }
}

/// Keep the selected mip chain byte for byte, including Vostok's sRGB trailer.
/// Reject redundant requests so stale manifest exceptions cannot pass silently.
pub fn select_native_mips(bytes: &[u8], limit: NativeMipLimit) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() >= 129 && &bytes[..4] == b"DDS ",
        "expected Vostok DDS"
    );
    let field = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    ensure!(field(4) == 124 && field(76) == 32, "unexpected DDS header");
    ensure!(
        field(112) == 0 && field(24) == 0,
        "expected a 2D, non-cube texture"
    );
    let (width, height, count) = (field(16), field(12), field(28));
    ensure!(
        width.is_power_of_two() && height.is_power_of_two(),
        "expected power-of-two dimensions"
    );
    ensure!(
        width.max(height) <= 65536,
        "texture dimensions exceed bounded DDS input"
    );
    ensure!(
        count > 0 && count <= width.max(height).ilog2() + 1,
        "invalid mip count"
    );
    ensure!(
        width.max(height) > limit.0,
        "texture already fits the requested pool"
    );
    let block_bytes = match &bytes[84..88] {
        b"DXT1" => 8,
        b"DXT5" => 16,
        _ => anyhow::bail!("expected BC1 or BC3 texture"),
    };
    let sizes: Vec<usize> = (0..count)
        .map(|mip| {
            let w = (width >> mip).max(1).div_ceil(4) as usize;
            let h = (height >> mip).max(1).div_ceil(4) as usize;
            w * h * block_bytes
        })
        .collect();
    ensure!(
        bytes.len() == 129 + sizes.iter().sum::<usize>() && bytes[bytes.len() - 1] <= 1,
        "invalid mip payload or sRGB trailer"
    );
    let skip = width.max(height).ilog2() - limit.0.ilog2();
    ensure!(skip < count, "requested mip is missing");
    let offset = 128 + sizes[..skip as usize].iter().sum::<usize>();
    let mut output = bytes[..128].to_vec();
    output.extend_from_slice(&bytes[offset..]);
    let payload = u32::try_from(output.len() - 128)?;
    for (at, value) in [
        (12, (height >> skip).max(1)),
        (16, (width >> skip).max(1)),
        (20, payload),
        (28, count - skip),
    ] {
        output[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[84..88].copy_from_slice(b"DXT5");
        for (offset, value) in [(4, 124u32), (12, 8), (16, 16), (28, 3), (76, 32)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[11; 128]); // 16x8
        bytes.extend_from_slice(&[22; 32]); // 8x4
        bytes.extend_from_slice(&[33; 16]); // 4x2
        bytes.push(1);
        bytes
    }

    #[test]
    fn retains_authored_lower_mips_and_color_space() {
        let bytes = fixture();
        let result = select_native_mips(&bytes, NativeMipLimit::try_from(8).unwrap()).unwrap();
        assert_eq!(&result[128..], &bytes[256..]);
        assert_eq!(u32::from_le_bytes(result[12..16].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(result[16..20].try_into().unwrap()), 8);
        assert_eq!(u32::from_le_bytes(result[20..24].try_into().unwrap()), 49);
        assert_eq!(u32::from_le_bytes(result[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn rejects_unsafe_or_stale_mip_requests() {
        for value in [0, 1, 3, 4096] {
            assert!(NativeMipLimit::try_from(value).is_err());
        }
        let bytes = fixture();
        assert!(select_native_mips(&bytes, NativeMipLimit::try_from(16).unwrap()).is_err());
        let limit = NativeMipLimit::try_from(8).unwrap();
        assert!(select_native_mips(&bytes[..bytes.len() - 1], limit).is_err());
        let mut cube = bytes.clone();
        cube[112..116].copy_from_slice(&0x200u32.to_le_bytes());
        assert!(select_native_mips(&cube, limit).is_err());
        let mut missing = bytes[..256].to_vec();
        missing.push(1);
        missing[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(select_native_mips(&missing, limit).is_err());
    }
}
