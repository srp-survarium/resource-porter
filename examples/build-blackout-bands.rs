// SPDX-License-Identifier: GPL-3.0-or-later
//! Blackout's team armbands, a blue and a red cloth band made on the sleeves
//! of every torso: `vostok_resource_porter::blackout_bands`.
//!
//! ```text
//! cargo run --release --example build-blackout-bands -- \
//!     --target-db RETAIL/resources.db [--patch RETAIL/resources.db.patch_N ...] \
//!     --output .work/blackout-bands
//! vostok-port pack from-bundle --base-db RETAIL/resources.db [--base-patch ...] \
//!     --bundle .work/blackout-bands --output PACK --id blackout-bands --version 2
//! ```
use clap::Parser;
use std::path::PathBuf;
use vostok_resource_porter::blackout_bands::{COLOURS, SURFACE, band_model, build};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>`: their torsos get bands too.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let r = build(&a.target_db, &a.patches, &a.output)?;
    let mm = |m: f64| m * 1000.0;
    println!(
        "Bands at {:.2}-{:.2} of the {:.0} mm upper arm, {:.0} mm out of the sleeve, on {} torsos:",
        r.window[0],
        r.window[1],
        mm(r.upper_arm),
        mm(r.offset),
        r.bands.len()
    );
    for b in &r.bands {
        println!(
            "  {:62} {:4}+{:<4} vertices, around the arm {:3.0}% / {:3.0}%",
            b.base_model,
            b.vertices[0],
            b.vertices[1],
            b.coverage[0] * 100.0,
            b.coverage[1] * 100.0
        );
    }
    println!("Skipped {}:", r.skipped.len());
    for (m, why) in &r.skipped {
        println!("  {m}: {why}");
    }
    println!("Band to sleeve in the game's clips (least / mean / largest):");
    for p in &r.poses {
        println!(
            "  {:52} {:+5.1} / {:4.1} / {:4.1} mm, sleeve over {} of {} (worst {} arm, {})",
            p.clip,
            mm(p.least),
            mm(p.mean),
            mm(p.largest),
            p.covered,
            p.measured,
            p.side,
            p.worst
        );
        let most: Vec<String> = p
            .most_covered
            .iter()
            .map(|(m, f)| format!("{m} {:.1}%", f * 100.0))
            .collect();
        println!("    most covered: {}", most.join(", "));
    }
    if let Some(b) = r.bands.first() {
        for c in &COLOURS {
            println!(
                "e.g. {} -> base_model \"{}\", part_name \"{SURFACE}\", material \"{}\"",
                b.base_model,
                band_model(&b.base_model, c.name),
                c.material()
            );
        }
    }
    Ok(())
}
