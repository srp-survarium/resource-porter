// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the H&K 416A5 and MSBS Radon bundle for v0.26g from 0.69d0 files.
//! See `docs/weapons-069d-026.md`.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// An archive of the 0.69d0 files the rifles use (`pack-donor-tree`).
    #[arg(long)]
    donor_db: PathBuf,
    /// The stock `resources.db`.
    #[arg(long)]
    target_db: PathBuf,
    /// The patches players have, laid over the target as the game mounts them.
    #[arg(long)]
    target_patch: Vec<PathBuf>,
    /// A `lobby_menu.swf` with the rifles' icons drawn (from the icon renderer).
    #[arg(long)]
    icons_swf: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut layers = vec![a.target_db];
    layers.extend(a.target_patch);
    vostok_resource_porter::weapons_026::build(&a.donor_db, &layers, a.icons_swf.as_deref(), &a.output)
}
