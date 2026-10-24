// SPDX-License-Identifier: GPL-3.0-or-later
//! Blackout's team armbands, from S.T.A.L.K.E.R. Clear Sky / Call of Pripyat
//! multiplayer: `vostok_resource_porter::blackout_bands`.
//!
//! ```text
//! cargo run --release --example build-blackout-bands -- \
//!     --source-root "STEAM/common/Stalker Call of Pripyat" \
//!     --target-db RETAIL/resources.db [--patch RETAIL/resources.db.patch_N ...] \
//!     --output .work/blackout-bands
//! vostok-port pack from-bundle --base-db RETAIL/resources.db [--base-patch ...] \
//!     --bundle .work/blackout-bands --output PACK --id blackout-bands --version 1
//! ```
use clap::Parser;
use std::path::PathBuf;
use vostok_resource_porter::blackout_bands::{CLEARANCE, Placement, SURFACE, TEAMS, build};

#[derive(Parser)]
struct Args {
    /// A Clear Sky or Call of Pripyat installation (or unpacked gamedata).
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    target_db: PathBuf,
    /// Released `resources.db.patch_<n>`: their torsos are fitted over too.
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    /// The band's lower edge as a fraction of the upper arm (default 0.88).
    #[arg(long)]
    distal: Option<f64>,
    /// The quantile of the torsos' sleeves the band goes over (default 1:
    /// all of them; lower is snugger on most and sinks into the bulkiest).
    #[arg(long)]
    cover: Option<f64>,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let mut placement = Placement::default();
    placement.distal = a.distal.unwrap_or(placement.distal);
    placement.cover = a.cover.unwrap_or(placement.cover);
    let r = build(
        &a.source_root,
        &a.target_db,
        &a.patches,
        placement,
        &a.output,
    )?;
    let mm = |m: f64| m * 1000.0;
    println!(
        "Band: {:.2}-{:.2} of the {:.0} mm upper arm ({:.0} mm below the shoulder joint), {:.0} mm wide, {:.0} / {:.0} mm around (left / right), emblem at {:.0} / {:.0} deg; {} vertices, {} triangles; pushed {} rounds",
        r.proximal,
        r.distal,
        mm(r.upper_arm),
        mm(r.below_shoulder),
        mm(r.width),
        mm(r.circumference[0]),
        mm(r.circumference[1]),
        r.emblem_degrees[0],
        r.emblem_degrees[1],
        r.vertices,
        r.triangles,
        r.push_rounds
    );
    println!(
        "Gaps over {} torso models (clearance {:.0} mm), thinnest sleeves first:",
        r.torsos.len(),
        mm(CLEARANCE)
    );
    for g in &r.torsos {
        println!(
            "  {:72} least {:5.1} mean {:5.1} largest {:5.1} mm, sleeve under {:3}% of the band",
            g.model,
            mm(g.least),
            mm(g.mean),
            mm(g.largest),
            100 * g.covered / g.samples.max(1)
        );
    }
    println!("Least gap in the game's clips (band and sleeve skinned):");
    for p in &r.poses {
        println!(
            "  {:58} {:+6.1} mm ({} arm, {}, at {:.2} {:+.0} deg)",
            p.clip,
            mm(p.least),
            p.side,
            p.torso,
            p.at,
            p.angle
        );
    }
    for t in &TEAMS {
        println!(
            "Part: base_model \"{}\", part_name \"{SURFACE}\", material \"{}\"",
            t.base_model(),
            t.material()
        );
    }
    Ok(())
}
