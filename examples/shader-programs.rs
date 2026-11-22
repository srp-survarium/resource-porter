// SPDX-License-Identifier: GPL-3.0-or-later
//! Each shader blob's program, by the hash of its code chunk (SHDR) alone:
//! two builds' blobs with the same code are the same program, whatever their
//! reflection data says. Also the names its reflection binds (constant
//! buffers' variables, textures, samplers), hashed, to tell blobs whose code
//! is equal but whose bindings are not.
//!
//!     shader-programs ARCHIVES.list > programs.tsv   (code hash, bindings hash, path)
use std::path::PathBuf;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use vostok_resource_porter::ResourceDb;

fn chunk<'a>(dxbc: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let u = |at: usize| Some(u32::from_le_bytes(dxbc.get(at..at + 4)?.try_into().ok()?) as usize);
    for index in 0..u(28)? {
        let at = u(32 + 4 * index)?;
        if dxbc.get(at..at + 4)? == tag {
            return dxbc.get(at + 8..at + 8 + u(at + 4)?);
        }
    }
    None
}

fn short(bytes: &[u8]) -> String {
    Sha256::digest(bytes)[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> Result<()> {
    let list = std::env::args().nth(1).context("usage: shader-programs ARCHIVES.list")?;
    let layers: Vec<PathBuf> = std::fs::read_to_string(&list)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect();
    let mut db = ResourceDb::open_layers(&layers)?;
    let paths: Vec<String> = db.paths_with_prefix("shaders/sm_4_0/").map(str::to_owned).collect();
    for path in paths {
        if !db.is_file(&path) {
            continue;
        }
        let blob = db.read(&path)?;
        let Some(count) = blob.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else {
            continue;
        };
        let Some(dxbc) = blob.get(4 + count * 264..).filter(|d| d.starts_with(b"DXBC")) else {
            continue;
        };
        let code = chunk(dxbc, b"SHDR").or_else(|| chunk(dxbc, b"SHEX"));
        let rdef = chunk(dxbc, b"RDEF");
        println!(
            "{}\t{}\t{path}",
            code.map_or("-".into(), short),
            rdef.map_or("-".into(), short)
        );
    }
    Ok(())
}
