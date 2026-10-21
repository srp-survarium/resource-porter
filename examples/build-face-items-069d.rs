use clap::Parser;
use std::path::PathBuf;

/// Builds the seven 0.69d masks as a bundle (`overlay/`, `port-manifest.json`,
/// `equipment.json`) for `vostok-port pack from-bundle`.
#[derive(Parser)]
struct Args {
    /// The 0.69d resources the port reads, extracted at their archive paths.
    #[arg(long)]
    source_root: PathBuf,
    /// The stock 0.26g resources.db.
    #[arg(long)]
    target_db: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::face_items_069d::build(&a.source_root, &a.target_db, &a.output)
}
