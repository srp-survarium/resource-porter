// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads one path through a layered archive list, for studying a donor.
//!
//!     near-build-lab LIST leaves PATH      a config's leaves
//!     near-build-lab LIST ls PREFIX        file paths with sizes
//!     near-build-lab LIST raw PATH OUT     the file's bytes
use std::path::PathBuf;

use anyhow::{Context, Result};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

fn leaves(v: &ConfigValue, at: &str, out: &mut Vec<String>) {
    match v {
        ConfigValue::Named(f) => {
            for f in f {
                let p = if at.is_empty() { f.name.clone() } else { format!("{at}.{}", f.name) };
                leaves(&f.value, &p, out);
            }
            if f.is_empty() {
                out.push(format!("{at}\t{{}}"));
            }
        }
        ConfigValue::Indexed(i) => {
            for (n, x) in i.iter().enumerate() {
                leaves(x, &format!("{at}[{n}]"), out);
            }
            if i.is_empty() {
                out.push(format!("{at}\t[]"));
            }
        }
        ConfigValue::Boolean(b) => out.push(format!("{at}\tbool:{b}")),
        ConfigValue::Integer(i) => out.push(format!("{at}\ti32:{i}")),
        ConfigValue::Float(f) => out.push(format!("{at}\tf32:{f:?}")),
        ConfigValue::String(s) => out.push(format!("{at}\tstring:{s:?}")),
        ConfigValue::Float2(f) => out.push(format!("{at}\tfloat2:{f:?}")),
        ConfigValue::Float3(f) => out.push(format!("{at}\tfloat3:{f:?}")),
        ConfigValue::Float4(f) => out.push(format!("{at}\tfloat4:{f:?}")),
    }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let layers: Vec<PathBuf> = std::fs::read_to_string(&a[0])?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect();
    let mut db = ResourceDb::open_layers(&layers)?;
    match a[1].as_str() {
        "leaves" => {
            for path in &a[2..] {
                let root = BinaryConfig::parse(&db.read(path).with_context(|| path.clone())?)?.root;
                let mut out = Vec::new();
                leaves(&root, "", &mut out);
                if a.len() > 3 {
                    println!("== {path}");
                }
                println!("{}", out.join("\n"));
            }
        }
        "ls" => {
            let paths: Vec<String> = db.paths_with_prefix(&a[2]).map(str::to_owned).collect();
            for p in paths {
                if db.is_file(&p) {
                    println!("{}\t{p}", db.read(&p)?.len());
                }
            }
        }
        "raw" => std::fs::write(&a[3], db.read(&a[2])?)?,
        "old" => {
            let root = BinaryConfig::parse(&db.read(&a[2])?)?.root;
            let (old, report) = vostok_resource_porter::near_build::schema::to_old_form(&root)?;
            let mut out = Vec::new();
            leaves(&old, "", &mut out);
            println!("{}", out.join("\n"));
            eprintln!("{}", serde_json::to_string(&report)?);
        }
        _ => anyhow::bail!("leaves | ls | raw"),
    }
    Ok(())
}
