//! Vegetation (`.veg`) files for 0.26g.
//!
//! A `.veg` file is a list of chunks, each `u32 id, u32 size, size bytes`.
//! The 0.26g client reads only two of them: 0x58, the float3 positions, and
//! 0x59, the float3 normals. Every chunk lookup in the exe that names a
//! vegetation chunk asks for those two (the grass layer loader at 0x15FB36
//! and the two readers at 0x12AB95 / 0x12AD21); none asks for 0x4D or 0x4E.
//!
//! 0x4D is the editor's paint mask, stored uncompressed. The 0.21d and 0.11e
//! scenes carry it, and it is almost all of the file: 54 of the 54.4 MiB of
//! Mamayev's suh_middle.veg, of which 99.9% are zero bytes. The client loads
//! the whole file into the managed resources arena and keeps it there with
//! the map, so the mask only takes room. A single 54.4 MiB block of it is
//! what the 2026-10-18 22:58 crash waited for ("10 sec passed and no
//! deallocation in managed arena"). The 81 native 0.26g files have no 0x4D
//! at all: they keep the editor's compressed mask, 0x4E, a few KiB.
//!
//! So 0x4D is dropped, and every other chunk stays as it was, in its order.

use anyhow::{ensure, Context, Result};

/// The editor's uncompressed paint mask: never read by the 0.26g client.
pub const PAINT_MASK: u32 = 0x4d;
const POSITIONS: u32 = 0x58;
const NORMALS: u32 = 0x59;

/// `(id, payload)` of every chunk, in file order.
pub fn chunks(bytes: &[u8]) -> Result<Vec<(u32, &[u8])>> {
    let mut chunks = Vec::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        ensure!(remaining.len() >= 8, "truncated vegetation chunk header");
        let id = u32::from_le_bytes(remaining[..4].try_into().unwrap());
        let size = u32::from_le_bytes(remaining[4..8].try_into().unwrap()) as usize;
        ensure!(size <= remaining.len() - 8, "truncated vegetation chunk");
        ensure!(
            chunks.iter().all(|&(seen, _)| seen != id),
            "duplicate vegetation chunk {id:#x}"
        );
        chunks.push((id, &remaining[8..8 + size]));
        remaining = &remaining[8 + size..];
    }
    Ok(chunks)
}

/// The file without its 0x4D chunk. The positions and normals the client
/// reads come out byte for byte the same; a file without 0x4D is returned
/// unchanged.
pub fn drop_paint_mask(bytes: &[u8]) -> Result<Vec<u8>> {
    let all = chunks(bytes)?;
    let runtime = |list: &[(u32, &[u8])]| -> Result<(Vec<u8>, Vec<u8>)> {
        let get = |id| {
            list.iter()
                .find(|&&(seen, _)| seen == id)
                .map(|&(_, payload)| payload.to_vec())
                .with_context(|| format!("missing vegetation chunk {id:#x}"))
        };
        Ok((get(POSITIONS)?, get(NORMALS)?))
    };
    let before = runtime(&all)?;
    let mut result = Vec::with_capacity(bytes.len());
    for &(id, payload) in &all {
        if id != PAINT_MASK {
            result.extend(id.to_le_bytes());
            result.extend((payload.len() as u32).to_le_bytes());
            result.extend(payload);
        }
    }
    ensure!(
        runtime(&chunks(&result)?)? == before,
        "vegetation positions or normals changed"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_le_bytes().to_vec();
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn drops_only_the_paint_mask() {
        let positions: Vec<u8> = (0..24).collect();
        let normals: Vec<u8> = (100..124).collect();
        let source = [
            chunk(0x4d, &[0; 4096]),
            chunk(0x4e, b"compressed"),
            chunk(0x58, &positions),
            chunk(0x59, &normals),
        ]
        .concat();
        let result = drop_paint_mask(&source).unwrap();
        assert_eq!(
            result,
            [
                chunk(0x4e, b"compressed"),
                chunk(0x58, &positions),
                chunk(0x59, &normals),
            ]
            .concat()
        );
    }

    #[test]
    fn a_native_file_is_unchanged() {
        let native = [
            chunk(0x4e, b"mask"),
            chunk(0x58, &[1; 12]),
            chunk(0x59, &[2; 12]),
        ]
        .concat();
        assert_eq!(drop_paint_mask(&native).unwrap(), native);
    }

    #[test]
    fn rejects_damaged_files() {
        let good = [chunk(0x4d, &[0; 8]), chunk(0x58, &[1; 12]), chunk(0x59, &[2; 12])].concat();
        assert!(drop_paint_mask(&good[..good.len() - 1]).is_err());
        assert!(drop_paint_mask(&[chunk(0x4d, &[0; 8]), chunk(0x58, &[1; 12])].concat()).is_err());
        assert!(drop_paint_mask(&[good.clone(), chunk(0x58, &[3; 12])].concat()).is_err());
    }
}
