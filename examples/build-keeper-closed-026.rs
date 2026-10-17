//! Writes the closed Keeper helmet pack (`docs/keeper-closed-026.md`).
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// The stock 0.26g `resources.db`.
    #[arg(long)]
    base_db: PathBuf,
    /// The players' `resources.db.patch_<n>` over it.
    #[arg(long = "base-patch")]
    base_patches: Vec<PathBuf>,
    /// The pack folder to write.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    std::fs::create_dir_all(&a.output)?;
    vostok_resource_porter::keeper_026::build(&a.base_db, &a.base_patches, &a.output)
}
