use clap::Parser;
use std::path::PathBuf;

/// Builds the STK2 outfits' bundles (resource-porter `stk2_outfits_026`):
/// `<output>/<outfit>/` for each `--outfit`, every outfit fitted against
/// the released game and the others.
#[derive(Parser)]
struct Args {
    /// The STK2 characters export (`___surva_stk2_characters_port`): its
    /// `resources/exported` and `resources/platforms/pc_dx11` folders.
    #[arg(long)]
    source_root: PathBuf,
    /// A folder with STK2's `resources/sources/textures/character/human`
    /// TGAs (`bandit/bandit_01`, `neutral/neutral_01`,
    /// `neutral/neutral_01_novice`).
    #[arg(long)]
    textures: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>` archives over the target database.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long = "outfit", default_values_t = ["bandit".to_owned(), "novice".to_owned(), "loner".to_owned()])]
    outfits: Vec<String>,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let outfits: Vec<&str> = a.outfits.iter().map(String::as_str).collect();
    vostok_resource_porter::stk2_outfits_026::build(
        &a.source_root,
        &a.textures,
        &a.target_db,
        &a.patches,
        &a.output,
        &outfits,
    )
}
