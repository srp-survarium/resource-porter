// SPDX-License-Identifier: GPL-3.0-or-later
//! Writes posed copies of skinned surfaces, for renders of a fit in the
//! game's poses.
//!
//! ```text
//! cargo run --release --example pose-stk2-outfits-026 -- --db RETAIL/resources.db \
//!     [--patch resources.db.patch_N]... [--clip ak/player/crouch/3rd_view/on_site_idle --at 0.5] \
//!     --out DIR  MODEL=BIND_POSE...
//! ```
//! A `MODEL` or `BIND_POSE` starting `db:` is read from the database (with
//! its patches); anything else from disk. Each posed surface is written to
//! `DIR/<k>/converted_model`, in the order given.
use clap::Parser;
use std::path::PathBuf;
use vostok_resource_porter::stk2_outfits_026::{PoseInput, pose_surfaces};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    db: PathBuf,
    #[arg(long)]
    patch: Vec<PathBuf>,
    /// A clip under `animations/single/weapons/`; the bind pose without one.
    #[arg(long)]
    clip: Option<String>,
    /// Where in the clip, 0 to 1.
    #[arg(long, default_value_t = 0.5)]
    at: f32,
    #[arg(long)]
    out: PathBuf,
    surfaces: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut view = vostok_resource_porter::ResourceDb::open(&a.db)?;
    let mut patched: Vec<vostok_resource_porter::ResourceDb> =
        a.patch.iter().map(vostok_resource_porter::ResourceDb::open).collect::<Result<_, _>>()?;
    let mut read = |spec: &str| -> anyhow::Result<Vec<u8>> {
        match spec.strip_prefix("db:") {
            Some(path) => {
                for p in patched.iter_mut().rev() {
                    if let Ok(bytes) = p.read(path) {
                        return Ok(bytes);
                    }
                }
                Ok(view.read(path)?)
            }
            None => Ok(std::fs::read(spec)?),
        }
    };
    let mut inputs = vec![];
    for s in &a.surfaces {
        let (model, bind) = s.split_once('=').ok_or_else(|| anyhow::anyhow!("{s}: not MODEL=BIND_POSE"))?;
        inputs.push(PoseInput { model: read(model)?, bind_pose: read(bind)? });
    }
    let posed = pose_surfaces(&a.db, &a.patch, &inputs, a.clip.as_deref(), a.at)?;
    for (k, bytes) in posed.iter().enumerate() {
        let dir = a.out.join(k.to_string());
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("converted_model"), bytes)?;
    }
    println!("{} surfaces posed into {}", posed.len(), a.out.display());
    Ok(())
}
