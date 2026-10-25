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
use vostok_resource_porter::blackout_bands::{COLOURS, Look, SURFACE, band_model, build};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>`: their torsos get bands too.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    /// The cloth: `plain` (pack 2), `cloth` (the game's grey fabric tinted
    /// to the team colour), `worn` (worn colours with dirt), `worn-cloth`.
    #[arg(long, default_value = "worn-cloth")]
    look: String,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let look = match a.look.as_str() {
        "plain" => Look::Plain,
        "cloth" => Look::Cloth,
        "worn" => Look::Worn,
        "worn-cloth" => Look::WornCloth,
        other => anyhow::bail!("no look {other}: plain, cloth, worn or worn-cloth"),
    };
    let r = build(&a.target_db, &a.patches, look, &a.output)?;
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
            b.coverage[1] * 100.0,
        );
    }
    println!("Skipped {}:", r.skipped.len());
    for (m, why) in &r.skipped {
        println!("  {m}: {why}");
    }
    println!("Band to sleeve in the game's clips (least / mean / largest):");
    for p in &r.poses {
        println!(
            "  {:52} {:+6.1} / {:4.1} / {:5.1} mm, off the sleeve {:.1}% (under it {:.1}%) (least: {} arm, {})",
            p.clip,
            mm(p.least),
            mm(p.mean),
            mm(p.largest),
            100.0 * p.off as f64 / p.measured.max(1) as f64,
            100.0 * p.covered as f64 / p.measured.max(1) as f64,
            p.side,
            p.worst
        );
        let most: Vec<String> = p
            .most_off
            .iter()
            .map(|(m, f)| format!("{m} {:.1}%", f * 100.0))
            .collect();
        println!("    most off: {}", most.join(", "));
        println!(
            "    edges off square to the upper arm: worst {:.1} deg ({} arm, {}), mean {:.2} deg",
            p.tilt_worst, p.tilt_worst_side, p.tilt_worst_torso, p.tilt_mean
        );
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
