// SPDX-License-Identifier: GPL-3.0-or-later
//! What shader programs read (`programs::reads`): the constants and bound
//! names of each path, or of every program of a family put together.
//!
//!     near-build-reads ARCHIVES.list PATH_OR_FAMILY...
use std::path::PathBuf;

use anyhow::Result;
use vostok_resource_porter::{ResourceDb, near_build::programs};

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let layers: Vec<PathBuf> = std::fs::read_to_string(&a[0])?.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(PathBuf::from).collect();
    let mut db = ResourceDb::open_layers(&layers)?;
    for name in &a[1..] {
        let paths: Vec<String> = if db.is_file(name) {
            vec![name.clone()]
        } else {
            let prefix = format!("shaders/sm_4_0/{name}/");
            db.paths_with_prefix(&prefix).filter(|p| db.is_file(p)).map(str::to_owned).collect()
        };
        let mut constants = std::collections::BTreeSet::new();
        let mut bound = std::collections::BTreeSet::new();
        let mut indexed = 0;
        for path in &paths {
            let one = programs::reads(&db.read(path)?)?;
            constants.extend(one.constants.into_iter().map(|c| format!("{}:{}", c.name, c.size)));
            bound.extend(one.bound);
            indexed += usize::from(one.bytes.is_none());
        }
        println!("{name}: {} programs, {indexed} index a constant buffer by a computed element", paths.len());
        println!("  constants: {}", constants.into_iter().collect::<Vec<_>>().join(" "));
        println!("  bound: {}", bound.into_iter().collect::<Vec<_>>().join(" "));
    }
    Ok(())
}
