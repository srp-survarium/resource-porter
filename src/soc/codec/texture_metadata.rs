// SPDX-License-Identifier: GPL-3.0-or-later
//! The detail association in X-Ray texture thumbnails (.thm).
//! STextureParams::Load reads chunk 0x0815 as a zero-terminated texture name
//! followed by a float scale. CTextureDescrMngr uses it only when a detail
//! flag is set. Clear Sky ships these instead of textures.ltx associations.

use anyhow::{ensure, Result};

use super::chunks::{Chunks, Cursor};

pub fn detail_scale(bytes: &[u8]) -> Result<Option<f32>> {
    let chunks = Chunks::new(bytes, "texture thumbnail");
    let Some(detail) = chunks.find(0x0815)? else {
        return Ok(None);
    };
    let mut params = Cursor::new(chunks.require(0x0812)?, "texture parameters");
    let _format = params.u32()?;
    let flags = params.u32()?;
    let mut detail = Cursor::new(detail, "texture detail association");
    let name = detail.stringz()?;
    let scale = detail.f32()?;
    if name.is_empty() || flags & ((1 << 23) | (1 << 26)) == 0 {
        return Ok(None);
    }
    ensure!(
        scale.is_finite() && scale > 0.,
        "invalid texture detail scale {scale}"
    );
    Ok(Some(scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumbnail(flags: u32, name: &[u8], scale: f32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let params = [0u32.to_le_bytes(), flags.to_le_bytes()].concat();
        let detail = [name, &[0], &scale.to_le_bytes()].concat();
        for (id, payload) in [(0x0812u32, params), (0x0815, detail)] {
            bytes.extend(id.to_le_bytes());
            bytes.extend((payload.len() as u32).to_le_bytes());
            bytes.extend(payload);
        }
        bytes
    }

    #[test]
    fn reads_military_warehouses_scale_and_fractional_scales() {
        for scale in [57., 57.25] {
            assert_eq!(
                detail_scale(&thumbnail(1 << 23, b"detail\\grass", scale)).unwrap(),
                Some(scale)
            );
            assert_eq!(
                detail_scale(&thumbnail(1 << 26, b"detail\\grass", scale)).unwrap(),
                Some(scale)
            );
        }
    }

    #[test]
    fn ignores_disabled_or_absent_associations() {
        assert_eq!(detail_scale(&[]).unwrap(), None);
        assert_eq!(
            detail_scale(&thumbnail(0, b"detail\\grass", 57.)).unwrap(),
            None
        );
        assert_eq!(detail_scale(&thumbnail(1 << 23, b"", 57.)).unwrap(), None);
    }

    #[test]
    fn rejects_bad_or_truncated_scales() {
        for scale in [0., -1., f32::NAN, f32::INFINITY] {
            assert!(detail_scale(&thumbnail(1 << 23, b"detail\\grass", scale)).is_err());
        }
        let mut bytes = thumbnail(1 << 23, b"detail\\grass", 57.);
        bytes.pop();
        assert!(detail_scale(&bytes).is_err());
    }
}
