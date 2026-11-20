// SPDX-License-Identifier: GPL-3.0-or-later
//! Print what 0.69d0's data says of each gun of a remodel sidecar: its own
//! parts (`*` one a fit can displace), its fits by kind and carrier chain,
//! and the refusals:
//! `plan-modules-069d <game.options file> <sidecar json> [<0.69d0 item>..]`.
use std::collections::BTreeMap;

use vostok_resource_porter::BinaryConfig;
use vostok_resource_porter::weapons_026::donor_069d::Spec069;
use vostok_resource_porter::weapons_026::donor_069d::catalog::Catalog;
use vostok_resource_porter::weapons_026::donor_069d::plan::{self, Kind};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let game = BinaryConfig::parse(&std::fs::read(&a[0])?)?.root;
    let catalog = Catalog::read(&game)?;
    let spec = Spec069::load(std::path::Path::new(&a[1]))?;
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    for (item, gun) in &spec.guns {
        if a.len() > 2 && !a[2..].contains(item) {
            continue;
        }
        let own = gun.own(catalog.gun(item)?);
        let offered = |m: &vostok_resource_porter::weapons_026::donor_069d::catalog::ModuleDef| {
            plan::kind(m) == Some(Kind::Scope) || catalog.has_stats(m)
        };
        let p = plan::plan(&catalog, item, &own, item, &offered)?;
        println!("{item}");
        for part in &p.parts {
            println!(
                "  {} {} in {} at {} of {:?}",
                if part.stock { "*" } else { " " },
                part.module,
                part.slot,
                part.site.locator,
                part.site.base
            );
        }
        for fit in &p.fits {
            let chain: Vec<String> = fit.chain.iter().map(|l| format!("{} in {}", l.module, l.slot)).collect();
            println!("  fit {:?} {} in {} [{}]", fit.kind, fit.module, fit.slot, chain.join(" > "));
            let needs = if fit.chain.is_empty() { "direct" } else { "carrier" };
            *totals.entry(format!("{:?} {needs}", fit.kind)).or_default() += 1;
        }
        for r in &p.refused {
            println!("  refused {:?} {}: {}", r.kind, r.module, r.why);
            *totals.entry(format!("{:?} refused", r.kind)).or_default() += 1;
        }
        *totals.entry("stock parts".into()).or_default() += p.stock().count();
    }
    println!("{totals:?}");
    Ok(())
}
