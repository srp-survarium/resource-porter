// SPDX-License-Identifier: GPL-3.0-or-later
//! The STK2 balaclava with one wide eye window, as the Renaissance Army's
//! mask (1823): `vostok_resource_porter::army_026`.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// The STK2 characters export (holds `resources/exported/...`).
    #[arg(long)]
    source_root: PathBuf,
    /// STK2's `bandit_01` texture folder (`bandit_01_attach.tga`, `_nmap.tga`).
    #[arg(long)]
    textures: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>` archives over the target database.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::army_026::build(
        &a.source_root,
        &a.textures,
        &a.target_db,
        &a.patches,
        &a.output,
    )
}
