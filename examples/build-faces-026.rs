// SPDX-License-Identifier: GPL-3.0-or-later
use clap::Parser;
use std::path::PathBuf;

/// Builds the player faces' bundle (resource-porter `faces_026`): the
/// 0.69d0 faces and the STK2 Novice's with their helmet and mask models,
/// the helmets and masks made for a face, and `gameplay/bodyparts/default`'s
/// `heads`, for `vostok-port pack from-bundle`. Stops if a face shows
/// through an item, or an item stands off a face, that the open list does
/// not name.
#[derive(Parser)]
struct Args {
    /// 0.69d0's files, extracted at their archive paths
    /// (`models/character/human/head`, `material_instances/characters/human/head`,
    /// `textures/character/human/head`).
    #[arg(long)]
    source_069d: PathBuf,
    /// The STK2 characters export (`___surva_stk2_characters_port`).
    #[arg(long)]
    stk2: PathBuf,
    /// A folder with STK2's `resources/sources/textures/character/human`.
    #[arg(long)]
    stk2_textures: PathBuf,
    /// The stock 0.26g resources.db.
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>` archives over the target database.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    /// Only measure, writing `faces-fit.json`.
    #[arg(long)]
    measure_only: bool,
    /// The open list (`ports/faces-026-open.tsv`): the misfits known and
    /// why. One it does not name stops the build.
    #[arg(long)]
    open: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    vostok_resource_porter::faces_026::build(
        &vostok_resource_porter::faces_026::Sources {
            v069: &a.source_069d,
            stk2: &a.stk2,
            stk2_textures: &a.stk2_textures,
        },
        &a.target_db,
        &a.patches,
        &a.output,
        a.measure_only,
        a.open.as_deref(),
    )
}
