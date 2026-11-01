// SPDX-License-Identifier: GPL-3.0-or-later
//! Build a 0.31a2 rifle port for v0.26g (`hk416`: the H&K 416A5 and its
//! 5.56x45; `radon`: the MSBS Radon and its EOTech 552). See
//! `docs/weapons-031-026.md`.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// `hk416` or `radon`.
    #[arg(long)]
    port: String,
    /// An archive of the 0.31a2 files (`pack-donor-tree`).
    #[arg(long)]
    donor_db: PathBuf,
    /// The stock `resources.db`.
    #[arg(long)]
    target_db: PathBuf,
    /// The patches players have, laid over the target as the game mounts them.
    #[arg(long)]
    target_patch: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut layers = vec![a.target_db];
    layers.extend(a.target_patch);
    let port = vostok_resource_porter::weapons_026::port(&a.port)?;
    vostok_resource_porter::weapons_026::build(port, &a.donor_db, &layers, &a.output)
}
