//! Regression proof: shared helper preserves every published native adapter byte.
use anyhow::{Result, ensure};
use std::{fs, path::PathBuf};
use vostok_resource_porter::weather_fog_021::vertex_blend::MaskedPrograms;
fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 4,
        "ORIGINAL_LINEAR_BLOB ORIGINAL_MASKED_BLOB PINNED_NATIVE_BLOBS EXISTING_PACK"
    );
    let programs = MaskedPrograms::new(&fs::read(&args[0])?, &fs::read(&args[1])?)?;
    let mut count = 0;
    for program in programs.programs() {
        let source = fs::read(args[2].join(format!("{}.blob", program.native_sha256())))?;
        ensure!(
            program.build(&source)? == fs::read(args[3].join("files").join(program.path()))?,
            "changed existing alias: {}",
            program.path()
        );
        count += 1;
    }
    println!("All {count} existing native blend adapter payloads are byte-identical");
    Ok(())
}
