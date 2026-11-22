// SPDX-License-Identifier: GPL-3.0-or-later
//! How a donor's shader programs stand against 0.26g's, family by family:
//! of the keys players have, how many the donor has under the translated
//! key and with the same code; of the donor's keys players lack (in 0.26g's
//! key space), how many 0.26g could be fed (`programs::Supplier`'s gate).
//!
//!     near-build-programs DONOR.list SHIPPED.list TARGET_ORDER.tsv DONOR_ORDER.tsv FAMILY...
use std::{collections::BTreeSet, path::PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use vostok_resource_porter::{
    ResourceDb,
    near_build::programs::{KeyOrder, Keys, Supplier, translate},
};

fn layers(list: &str) -> Result<Vec<PathBuf>> {
    Ok(std::fs::read_to_string(list)
        .with_context(|| format!("reading {list}"))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect())
}

/// The code of a blob: its SHDR chunk (two builds' blobs of one program
/// differ in their reflection and include records).
fn code(blob: &[u8]) -> Vec<u8> {
    let count = u32::from_le_bytes(blob[..4].try_into().unwrap()) as usize;
    let dxbc = &blob[4 + count * 264..];
    let u = |at: usize| u32::from_le_bytes(dxbc[at..at + 4].try_into().unwrap()) as usize;
    for index in 0..u(28) {
        let at = u(32 + 4 * index);
        if &dxbc[at..at + 4] == b"SHDR" {
            return Sha256::digest(&dxbc[at + 8..at + 8 + u(at + 4)]).to_vec();
        }
    }
    Sha256::digest(dxbc).to_vec()
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(a.len() >= 5, "DONOR.list SHIPPED.list TARGET_ORDER.tsv DONOR_ORDER.tsv FAMILY...");
    let mut donor = ResourceDb::open_layers(&layers(&a[0])?)?;
    let mut shipped = ResourceDb::open_layers(&layers(&a[1])?)?;
    let order = |f: &str| -> Result<KeyOrder> { KeyOrder::parse(&std::fs::read_to_string(f)?) };
    let target = Keys::read(order(&a[2])?, &mut shipped)?;
    let donor_keys = Keys::read(order(&a[3])?, &mut donor)?;
    for family in &a[4..] {
        let prefix = format!("shaders/sm_4_0/{family}/");
        let ours: Vec<String> = shipped.paths_with_prefix(&prefix).filter(|p| shipped.is_file(p)).map(str::to_owned).collect();
        let (mut there, mut same, mut absent, mut untranslatable) = (0, 0, 0, 0);
        let mut reached = BTreeSet::new();
        for path in &ours {
            match translate(&path[prefix.len()..], family, &target, &donor_keys) {
                Ok(key) => {
                    let theirs = format!("{prefix}{key}");
                    if donor.is_file(&theirs) {
                        there += 1;
                        if code(&donor.read(&theirs)?) == code(&shipped.read(path)?) {
                            same += 1;
                        }
                        reached.insert(theirs);
                    } else {
                        absent += 1;
                    }
                }
                Err(error) => {
                    if untranslatable == 0 {
                        println!("  first key that does not translate: {error:#}");
                    }
                    untranslatable += 1;
                }
            }
        }
        println!("{family}: players have {}; the donor has {there} of them under the translated key ({same} with the same code), lacks {absent}, {untranslatable} do not translate", ours.len());
        // The donor's other programs, as 0.26g would ask for them: the
        // donor's key with its own macros off, read back in 0.26g's order.
        let theirs: Vec<String> = donor.paths_with_prefix(&prefix).filter(|p| donor.is_file(p) && !reached.contains(*p)).map(str::to_owned).collect();
        let mut supplier = Supplier::new(&target, &donor_keys);
        let (mut fed, mut refused) = (0, 0);
        let mut why = std::collections::BTreeMap::<String, usize>::new();
        for path in &theirs {
            let Ok(back) = translate(&path[prefix.len()..], family, &donor_keys, &target) else { continue };
            let wanted = format!("{prefix}{back}");
            if shipped.is_file(&wanted) {
                continue;
            }
            let (supply, blob) = supplier.supply(&mut shipped, &mut donor, &wanted)?;
            if blob.is_some() {
                fed += 1;
            } else {
                refused += 1;
                let reason = supply.not_supplied.unwrap_or_default();
                let reason = reason.split("reads what").nth(1).map_or(reason.clone(), |r| r.to_owned());
                *why.entry(reason).or_default() += 1;
            }
        }
        println!("  the donor's other programs 0.26g could ask for: {fed} it can feed, {refused} it cannot");
        let mut reasons: Vec<_> = why.into_iter().collect();
        reasons.sort_by_key(|r| std::cmp::Reverse(r.1));
        for (reason, n) in reasons.iter().take(6) {
            println!("    {n}: {}", &reason[..reason.len().min(300)]);
        }
    }
    Ok(())
}
