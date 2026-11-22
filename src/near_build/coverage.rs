// SPDX-License-Identifier: GPL-3.0-or-later
//! Whether a material that keeps its donor switch set has its g-buffer
//! programs, for every runtime choice.
//!
//! A g-buffer program's key is the material's macros (what its switches
//! select) and the runtime's (the shading quality, the mesh's vertex input,
//! the level of detail, instancing, displacement, sub-uv). The material's
//! part is read from its switches (`part`, the mapping 0.26g's g-buffer
//! effect compile makes: it gives the key of 1 700 of the stock game's 1 711
//! g-stage materials); the runtime choices are those the donor compiled for
//! that part. A kept set is covered when each of those keys is a program of
//! players' or of the pack. One that is not (a donor program 0.26g cannot
//! feed: 0.44a5's parallax at shading quality 3 reads
//! `parallax_uv_scale_factors`; or a part no donor program has) takes the
//! nearest set players have programs for (`files::Importer::refit_kept`).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use super::programs::Keys;
use crate::{ResourceDb, binary_config::ConfigValue};

pub const FAMILY: &str = "gbuffer_pass.ps";
const ROOT: &str = "shaders/sm_4_0/gbuffer_pass.ps/";

/// Macros a draw or a player's setting chooses, not the material.
const RUNTIME: [&str; 6] =
    ["GLOBAL_SHADING_QUALITY", "CONFIG_USE_SUBUV", "CONFIG_VERTEX_INPUT_TYPE", "CONFIG_LOD_INDEX", "CONFIG_USE_INSTANCING", "CONFIG_USE_DISPLACEMENT"];

/// The material's g-buffer macros and the switch each one is.
const SWITCHES: [(&str, &str); 21] = [
    ("CONFIG_TDIFFUSE", "use_tdiffuse"),
    ("CONFIG_TNORMAL", "use_nmap"),
    ("CONFIG_TDETAIL_NORMAL", "use_detail_nmap"),
    ("CONFIG_ALPHA_TEST", "use_alpha_test"),
    ("CONFIG_PARALLAX", "use_parallax"),
    ("CONFIG_TRANSLUCENCY", "use_ttranslucency"),
    ("CONFIG_TSPECULAR_COLOR", "use_tspecular_color"),
    ("CONFIG_TFRESNEL", "use_tfresnel"),
    ("CONFIG_TROUGHNESS", "use_troughness"),
    ("CONFIG_TDETAIL", "use_tditail_diffuse"),
    ("CONFIG_TALPHABLENDED_NORMAL", "use_alphablended_normal"),
    ("CONFIG_VERTEX_BLENDED_TECHNIQUE", "vertex_blend_technique"),
    ("CONFIG_VERTEX_BLENDED_TILED_MASK", "use_tiled_vertex_blended_mask"),
    ("CONFIG_VERTEX_BLENDED_NORMAL", "use_vertex_blended_normal"),
    ("CONFIG_USE_DIFFUSE_MASKED_COLOR", "use_masked_diffuse_color"),
    ("CONFIG_USE_UV_SCROLLING", "use_uv_scrolling"),
    ("CONFIG_USE_NORMAL_WAVES", "use_normal_waves"),
    ("CONFIG_USE_FUZZINESS", "use_fuzziness"),
    ("CONFIG_USE_OLTA", "use_olta"),
    ("CONFIG_WIND_MOTION", "wind_motion"),
    ("CONFIG_SEQUENCE", "use_sequence"),
];

type Values = Vec<String>;

/// The g-buffer keys there are, as material part to runtime choices.
#[derive(Default)]
pub struct Coverage {
    /// The target's material macros, in its order.
    material: Vec<String>,
    /// Players' and the pack's programs.
    have: BTreeMap<Values, BTreeSet<Values>>,
    /// The donor's programs (in the target's macros).
    donor: BTreeMap<Values, BTreeSet<Values>>,
}

fn split(target: &Keys, material: &[String], named: &BTreeMap<&str, &str>) -> (Values, Values) {
    let get = |name: &str| named.get(name).map_or("0".to_owned(), |v| (*v).to_owned());
    let runtime = target.order.names().iter().filter(|n| RUNTIME.contains(&n.as_str()) && target.reads(FAMILY, n)).map(|n| get(n)).collect();
    (material.iter().map(|n| get(n)).collect(), runtime)
}

impl Coverage {
    /// From players' archives, the pack's programs (`out`) and the donor's.
    pub fn read(target: &Keys, donor: &Keys, shipped: &ResourceDb, out: &BTreeMap<String, Vec<u8>>, donor_db: &ResourceDb) -> Result<Self> {
        let material: Vec<String> =
            target.order.names().iter().filter(|n| target.reads(FAMILY, n) && !RUNTIME.contains(&n.as_str())).cloned().collect();
        let mut coverage = Self { material, ..Self::default() };
        let theirs = shipped.paths_with_prefix(ROOT).filter(|p| shipped.is_file(p)).map(str::to_owned);
        let ours = out.keys().filter(|p| p.starts_with(ROOT)).cloned();
        for path in theirs.chain(ours).collect::<Vec<_>>() {
            let key = &path[ROOT.len()..];
            let values = target.values(FAMILY, key)?;
            let named: BTreeMap<&str, &str> = target.order.names().iter().map(String::as_str).zip(values).collect();
            let (m, r) = split(target, &coverage.material, &named);
            coverage.have.entry(m).or_default().insert(r);
        }
        for path in donor_db.paths_with_prefix(ROOT).filter(|p| donor_db.is_file(p)).map(str::to_owned).collect::<Vec<_>>() {
            let key = &path[ROOT.len()..];
            let Ok(values) = donor.values(FAMILY, key) else { continue };
            let named: BTreeMap<&str, &str> = donor.order.names().iter().map(String::as_str).zip(values).collect();
            // A macro only the donor's family reads, on: no key 0.26g computes.
            if named.iter().any(|(n, v)| !target.reads(FAMILY, n) && donor.reads(FAMILY, n) && v.bytes().any(|c| c != b'0')) {
                continue;
            }
            let (m, r) = split(target, &coverage.material, &named);
            coverage.donor.entry(m).or_default().insert(r);
        }
        Ok(coverage)
    }

    /// The material part of an effect's fields (`<field>` to its `value`).
    pub fn part(&self, effect: &[crate::binary_config::NamedValue]) -> Values {
        let value = |field: &str| -> String {
            match effect.iter().find(|f| f.name == field).and_then(|f| f.value.named("value")) {
                Some(ConfigValue::Boolean(true)) => "1".into(),
                Some(ConfigValue::Integer(i)) => i.to_string(),
                _ => "0".into(),
            }
        };
        let technique = value("vertex_blend_technique");
        self.material
            .iter()
            .map(|name| match name.as_str() {
                // Both are the technique's: the blend's own alpha test with
                // technique 1, its normal with any.
                "CONFIG_ALPHA_TEST_CONSIDER_VERTEX_BLEND" => if technique == "1" { "1" } else { "0" }.to_owned(),
                "CONFIG_VERTEX_BLENDED_NORMAL" if technique == "0" => "0".to_owned(),
                _ => SWITCHES.iter().find(|(m, _)| m == name).map_or("0".to_owned(), |(_, s)| value(s)),
            })
            .collect()
    }

    /// The runtime choices of `part` the donor compiled and nothing ships;
    /// None when no donor program has the part (the set is not known to
    /// draw at all).
    pub fn lacking(&self, part: &Values) -> Option<usize> {
        let need = self.donor.get(part)?;
        let have = self.have.get(part);
        Some(need.iter().filter(|r| !have.is_some_and(|h| h.contains(*r))).count())
    }

    /// The part's macros that are on, by name (for the report).
    pub fn describe(&self, part: &Values) -> String {
        self.material.iter().zip(part).filter(|(_, v)| *v != "0").map(|(n, v)| format!("{}={v}", n.trim_start_matches("CONFIG_"))).collect::<Vec<_>>().join(",")
    }
}
