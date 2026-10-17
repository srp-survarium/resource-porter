// SPDX-License-Identifier: GPL-3.0-or-later
//! Pack a folder of resources extracted from a later build into one
//! `resources.db`, so the porter reads them like any archive.
//!
//! `pack-donor-tree <folder> <out.db>`: every file under the folder at its
//! path relative to it. The archive is read back and compared file by file.

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, out] = args.as_slice() else {
        bail!("usage: pack-donor-tree <folder> <out.db>");
    };
    let (bytes, files) = resources_db::pack::pack(std::path::Path::new(dir))
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("packing {dir}"))?;
    std::fs::write(out, &bytes)?;
    println!(
        "packed {} files into {out} ({} bytes)",
        files.len(),
        bytes.len()
    );
    Ok(())
}
