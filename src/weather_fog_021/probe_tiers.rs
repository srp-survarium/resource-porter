// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d's local probe tier: the three program families skhid's exe patch 203 draws a 0.69d
//! scene's local environment probes with (`environment_probe_tier_weight`, `_damp`, `_light`).
//!
//! 0.69d composes the probes of a tier as `sum(w L) / max(W, 1)` and shows the tier below only
//! where `1 - W` remains; 0.26g lerps every probe over the ones before it. Patch 203 draws a
//! scene's local probes (the 0.69d converter's, marked by a radius below 0) three more times
//! with techniques the four box effects compile in addition, each pass with one of these:
//! - weight: rt 32 minus `w/4` (0.69d's linear falloff over the side width, a cylinder's in its
//!   circumscribed box);
//! - damp: the light under the tier times `(1 - min(W, 1))^(w/W)`;
//! - light: the stock program's diffuse and specular times `w / max(W, 1)`.
//!
//! The programs are compiled from skhid's `srp-026g/shaders/sm_4_0/environment_probe_lighting.ps`
//! (`probes.sh build`: fxc /Zpr /O3 /T ps_4_0, the June 2010 SDK's, as every 0.26g blob), whose
//! stock program (TIER 0) compiles back to the shipped one (6 of the 8 byte-identical, the other
//! two one mul's operands apart: `probes.sh check`). Their keys are the stock family's (the same
//! mask), the 8 of geometry 1 and LOD_INDEX 0; each blob takes the stock blob's include records.
//! No scene without such probes draws them.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::ResourceDb;

/// The stock family the tier programs take their keys and include records from.
pub const STOCK_PREFIX: &str = "shaders/sm_4_0/environment_probe_lighting.ps/";

/// The stock family's mask (shaders/masks), the tier families' too.
pub const MASK: [&str; 6] = [
    "CONFIG_ENV_PROBE_GEOMETRY_TYPE",
    "CONFIG_LOD_INDEX",
    "CONFIG_TALPHABLENDED_DIFFUSE",
    "CONFIG_TDIFFUSE",
    "GLOBAL_LIGHTING_QUALITY",
    "CONFIG_SHADOWED_LIGHT",
];

pub const FAMILIES: [&str; 3] = ["environment_probe_tier_weight", "environment_probe_tier_damp", "environment_probe_tier_light"];

macro_rules! programs {
    ($($key:literal),*) => {
        /// The 8 keys, and each family's DXBC for it (weight, damp, light).
        const PROGRAMS: [(&str, [&[u8]; 3]); 8] = [$(
            ($key, [
                include_bytes!(concat!("../../assets/probe-tiers/environment_probe_tier_weight/", $key, ".dxbc")),
                include_bytes!(concat!("../../assets/probe-tiers/environment_probe_tier_damp/", $key, ".dxbc")),
                include_bytes!(concat!("../../assets/probe-tiers/environment_probe_tier_light/", $key, ".dxbc")),
            ]),
        )*];
    };
}

programs!(
    "___0_0____________________00___0_________1_________________________",
    "___0_0____________________01___0_________1_________________________",
    "___0_1____________________00___0_________1_________________________",
    "___0_1____________________01___0_________1_________________________",
    "___3_0____________________00___0_________1_________________________",
    "___3_0____________________01___0_________1_________________________",
    "___3_1____________________00___0_________1_________________________",
    "___3_1____________________01___0_________1_________________________"
);

/// The 24 programs as resources: `(path, blob)`, each the stock blob's include records (a u32
/// count, that many 264-byte records) and the tier's DXBC.
pub fn build(native: &mut ResourceDb) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for (key, dxbcs) in PROGRAMS {
        let stock = native.read(&format!("{STOCK_PREFIX}{key}")).with_context(|| format!("the stock probe program {key}"))?;
        ensure!(stock.len() >= 4, "{key}: a stock blob without its record count");
        let records = u32::from_le_bytes(stock[..4].try_into()?) as usize;
        let header = 4 + 264 * records;
        ensure!(stock.len() > header && &stock[header..header + 4] == b"DXBC", "{key}: no DXBC after the stock blob's records");
        for (family, dxbc) in FAMILIES.iter().zip(dxbcs) {
            ensure!(dxbc.starts_with(b"DXBC"), "{family}/{key}: not DXBC");
            let mut blob = stock[..header].to_vec();
            blob.extend_from_slice(dxbc);
            out.push((format!("shaders/sm_4_0/{family}.ps/{key}"), blob));
        }
    }
    Ok(out)
}

/// The `shaders/masks` config_set values: each tier family with the stock family's mask.
pub fn masks() -> serde_json::Map<String, Value> {
    FAMILIES.iter().map(|f| ((*f).to_owned(), json!({ "table": { "ps": MASK } }))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_is_dxbc_and_the_families_differ() {
        for (key, [weight, damp, light]) in PROGRAMS {
            for p in [weight, damp, light] {
                assert!(p.starts_with(b"DXBC"), "{key}");
            }
            assert!(weight != damp && damp != light, "{key}");
        }
        assert_eq!(masks().len(), 3);
    }
}
