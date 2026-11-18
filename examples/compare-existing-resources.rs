// SPDX-License-Identifier: GPL-3.0-or-later
//! Verify that a later local overlay preserves every earlier overlay resource.
//! Used to retain visual evidence when only absent shader aliases are added.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};
use vostok_resource_porter::{ResourceDb, content_pack::sha256_hex};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 3, "EARLIER_PATCH LATER_PATCH REPORT_JSON");
    let mut before = ResourceDb::open(&args[0])?;
    let mut after = ResourceDb::open(&args[1])?;
    let old: BTreeSet<String> = before.paths().map(str::to_owned).collect();
    let new: BTreeSet<String> = after.paths().map(str::to_owned).collect();
    let removed: Vec<_> = old.difference(&new).cloned().collect();
    let added: Vec<_> = new.difference(&old).cloned().collect();
    let mut changed = Vec::new();
    let mut same_bytes = 0usize;
    for path in old.intersection(&new) {
        let a = before
            .read(path)
            .with_context(|| format!("earlier {path}"))?;
        let b = after.read(path).with_context(|| format!("later {path}"))?;
        if a != b {
            changed.push(json!({"path": path, "earlier_sha256": sha256_hex(&a), "later_sha256": sha256_hex(&b)}));
        } else {
            same_bytes += a.len();
        }
    }
    let report = json!({
        "purpose": "Existing resource identity for reuse of immutable native capture evidence",
        "earlier_patch_sha256": sha256_hex(&fs::read(&args[0])?),
        "later_patch_sha256": sha256_hex(&fs::read(&args[1])?),
        "earlier_resources": old.len(), "later_resources": new.len(),
        "unchanged_bytes": same_bytes, "removed": removed, "changed": changed, "added": added,
        "existing_resources_identical": removed.is_empty() && changed.is_empty()
    });
    fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        removed.is_empty() && changed.is_empty(),
        "later overlay changes existing resources; capture reuse needs an affected-resource audit"
    );
    println!(
        "All {} earlier resources unchanged ({} bytes); {} additive paths",
        old.len(),
        same_bytes,
        added.len()
    );
    Ok(())
}
