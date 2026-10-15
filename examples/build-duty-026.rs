// SPDX-License-Identifier: GPL-3.0-or-later
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
/// Build one complete Duty exoskeleton item for Survarium 0.26g.
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    texture_dir: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    #[arg(long)]
    icons_swf: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::duty_026::build(
        &a.source_root,
        &a.texture_dir,
        &a.target_db,
        a.icons_swf.as_deref(),
        &a.output,
    )
}
