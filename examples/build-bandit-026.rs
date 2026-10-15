use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::bandit_026::build(&a.source_root, &a.target_db, &a.output)
}
