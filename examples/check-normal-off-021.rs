//! Read-only debug preflight of every pinned original/native shader contract.
use anyhow::{Result, ensure};
use std::path::PathBuf;
use vostok_resource_porter::{ResourceDb, weather_fog_021::vertex_blend};
fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 2, "DONOR_DB STOCK_DB");
    let programs = vertex_blend::normal_off::build(
        &mut ResourceDb::open(&args[0])?,
        &mut ResourceDb::open(&args[1])?,
    )?;
    println!(
        "All {} pinned normal-off contracts build; no files written",
        programs.len()
    );
    Ok(())
}
