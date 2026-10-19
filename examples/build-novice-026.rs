use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>` archives over the target database:
    /// the masks the cap is fitted over and the trousers and gloves the
    /// jacket is fitted to include their ported items.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::novice_026::build(&a.source_root, &a.target_db, &a.patches, &a.output)
}
