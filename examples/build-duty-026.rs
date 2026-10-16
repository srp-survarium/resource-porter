// SPDX-License-Identifier: GPL-3.0-or-later
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
/// Build an STK2 exoskeleton set (Duty, or its Freedom repaint) for Survarium 0.26g.
struct Args {
    /// `duty` (body 1805, helmet 1800) or `freedom` (body 1807, helmet 1808).
    #[arg(long, default_value = "duty")]
    variant: String,
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
    vostok_resource_porter::duty_026::build_variant(
        vostok_resource_porter::duty_026::variant(&a.variant)?,
        &a.source_root,
        &a.texture_dir,
        &a.target_db,
        a.icons_swf.as_deref(),
        &a.output,
    )
}
