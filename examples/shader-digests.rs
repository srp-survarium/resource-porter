// SPDX-License-Identifier: GPL-3.0-or-later
//! Print retail DXBC digests and resource paths for matching read-only traces.
use anyhow::{Context, Result};
use vostok_resource_porter::ResourceDb;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: shader-digests resources.db")?;
    let mut db = ResourceDb::open(path)?;
    let paths: Vec<_> = db
        .paths()
        .filter(|p| p.starts_with("shaders/") && (p.contains(".ps/") || p.contains(".vs/")))
        .map(str::to_owned)
        .collect();
    for path in paths {
        let blob = db.read(&path)?;
        if blob.len() < 4 {
            continue;
        }
        let count = u32::from_le_bytes(blob[..4].try_into()?) as usize;
        let Some(offset) = count.checked_mul(264).and_then(|n| n.checked_add(4)) else {
            continue;
        };
        if let Some(code) = blob.get(offset..) {
            if code.len() >= 20 && &code[..4] == b"DXBC" {
                for byte in &code[4..20] {
                    print!("{byte:02x}");
                }
                println!("\t{path}");
            }
        }
    }
    Ok(())
}
