// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the 0.69d0 weapons' bundle for v0.26g. See `docs/weapons-069d-026.md`.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// 0.69d0's split volumes (a `.split` list, mount order).
    #[arg(long)]
    donor: PathBuf,
    /// The stock `resources.db`.
    #[arg(long)]
    target_db: PathBuf,
    /// The patches players have, laid over the target as the game mounts them.
    #[arg(long)]
    target_patch: Vec<PathBuf>,
    /// Only these guns (0.69d keys or item names); all by default.
    #[arg(long)]
    gun: Vec<String>,
    /// The configs name their icon cells (icons drawn into `<output>/icons`).
    #[arg(long)]
    icons: bool,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    use vostok_resource_porter::weapons_069d as w;
    let a = Args::parse();
    let mut layers = vec![a.target_db];
    layers.extend(a.target_patch);
    let guns: Vec<&w::Gun> = if a.gun.is_empty() {
        w::GUNS.to_vec()
    } else {
        a.gun
            .iter()
            .map(|k| w::gun(k))
            .collect::<anyhow::Result<_>>()?
    };
    w::build(&guns, &a.donor, &layers, a.icons, &a.output)
}
