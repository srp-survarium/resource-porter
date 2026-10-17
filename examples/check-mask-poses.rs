// SPDX-License-Identifier: GPL-3.0-or-later
//! How close the native head comes to a fitted mask in the game's own
//! animations: both skinned through sampled poses of lobby and match clips.
//!
//! ```text
//! cargo run --release --example check-mask-poses -- --db RETAIL/resources.db \
//!     (--mask-db resources.db.patch_N | --mask-file converted_model) [--follow]
//! ```
use clap::Parser;
use std::path::PathBuf;
use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::bandit_026::{MASK_MODEL, pose_clearances};

#[derive(Parser)]
struct Args {
    /// The stock database (native head, rig, animations).
    #[arg(long)]
    db: PathBuf,
    /// The archive holding the mask (a patch or a whole database).
    #[arg(long, required_unless_present = "mask_file")]
    mask_db: Option<PathBuf>,
    /// Or the mask's `converted_model` itself (a bundle's overlay file).
    #[arg(long)]
    mask_file: Option<PathBuf>,
    /// Give the mask the head's skin first (`mask_fit::follow_head_weights`).
    #[arg(long)]
    follow: bool,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut db = ResourceDb::open(&a.db)?;
    let mut mask = match (&a.mask_file, &a.mask_db) {
        (Some(file), _) => std::fs::read(file)?,
        (None, Some(db)) => ResourceDb::open(db)?.read(MASK_MODEL)?,
        (None, None) => unreachable!("clap requires one"),
    };
    let head = db.read(vostok_resource_porter::bandit_026::HEAD_MODEL)?;
    if a.follow {
        let follow = vostok_resource_porter::mask_fit::follow_head_weights(&mut mask, &head)?;
        println!(
            "followed the head's skin: {} vertices changed, {} had off-head bones, farthest {:.1} mm",
            follow.changed,
            follow.off_head_bones,
            follow.max_distance * 1000.0
        );
    }
    let mut worst = f64::INFINITY;
    for (clip, c) in pose_clearances(&mut db, &mask, &head)? {
        worst = worst.min(c.min);
        println!(
            "{clip}: least gap {:+.2} mm (cheeks and jaw {:+.2} mm at {:+.3} {:.3} {:+.3}), {} head vertices through, at x {:+.3} y {:.3} z {:+.3} ({})",
            c.min * 1000.0,
            c.face_min * 1000.0,
            c.face_at[0],
            c.face_at[1],
            c.face_at[2],
            c.through,
            c.at[0],
            c.at[1],
            c.at[2],
            if c.at[0] < 0.0 { "left" } else { "right" }
        );
    }
    println!("worst {:+.2} mm", worst * 1000.0);
    Ok(())
}
