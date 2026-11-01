// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a port's files come from: a [`Donor`] answers for one build, in
//! 0.26g's layouts, so the shared core ([`super::build`]) never sees a
//! build's own formats.
//!
//! A donor converts *layouts* (how a file is written): clip event headers,
//! mesh chunks 0.26g lacks, config records. What a config *means* (state
//! types, renamed keys, scales) is the build's [`Rules`], which the core
//! applies; the core also conforms materials and sound options to native
//! templates, as it needs the target for those.
//!
//! [`ArchiveDonor`] serves the builds that keep 0.26g's file formats and
//! ship their configs as files: 0.29a3 and 0.31a2 (one `resources.db` or a
//! split build's `.split` list of volumes). A build with another layout
//! (0.46c2's 16-byte configs and `game.options` items) is another impl.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::clips;
use crate::{BinaryConfig, ConfigValue, ResourceDb};

/// A build weapons are ported from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum DonorBuild {
    #[serde(rename = "0.29a3")]
    V029a3,
    #[serde(rename = "0.31a2")]
    V031a2,
    #[serde(rename = "0.46c2")]
    V046c2,
}

/// What a build's configs mean, against 0.26g's.
#[derive(Debug)]
pub struct Rules {
    /// The build's weapon state types and 0.26g's; a type not listed is the
    /// same in both (0.26g's 300..=311).
    pub state_types: &'static [(i32, i32)],
    /// The highest state type known to be the same in both builds.
    pub same_state_types_up_to: i32,
    /// Keys the build spells otherwise, by their last component: the
    /// build's name and 0.26g's.
    pub renames: &'static [(&'static str, &'static str)],
    /// Values on another scale: the config path and the factor to 0.26g's.
    pub scaled: &'static [(&'static str, f32)],
    /// The build's clips write 12-byte event headers.
    pub clips_12_byte_events: bool,
    /// The build's meshes may carry chunk 110 (4-byte statistics).
    pub mesh_statistics: bool,
    /// A donor config without `parameters.aim_offset` drops the template's
    /// (it fits the template's model): 0.29a3, 0.31a2 and 0.46c2. 0.46c2's
    /// configs have the key on no weapon, but its guns align by their own
    /// `aim` locator: the template's offsets put the SVU's sight 30 px and
    /// the Fort-401's front post 17 px low (the in-game look, 2026-11-01).
    pub aim_offset_in_config: bool,
}

/// 0.29 and 0.31 spell two of 0.26g's parameters otherwise, keep stamina
/// damage as a fraction (0.26g: percent; patch 19's 0.29a3 weapons x100),
/// and write 12-byte clip event headers.
const RENAMES: &[(&str, &str)] = &[
    ("health_modifier", "pain_health_modifier"),
    (
        "stamina_regeneration_speed_modifier",
        "stamina_regenation_speed_modifier",
    ),
];
const SCALED: &[(&str, f32)] = &[("parameters.stamina_damage", 100.0)];

/// 0.29a3: 0.26g's state types (patch 19's weapons keep the donor's).
pub const RULES_029A3: Rules = Rules {
    state_types: &[
        (312, 312),
        (313, 313),
        (314, 314),
        (315, 315),
        (316, 316),
        (321, 321),
    ],
    same_state_types_up_to: 311,
    renames: RENAMES,
    scaled: SCALED,
    clips_12_byte_events: true,
    mesh_statistics: false,
    aim_offset_in_config: true,
};

/// 0.31a2 renumbered the state types above 311 down by one (its reload of
/// two clips, tactical and empty, 313, is 0.26g's 314; grenade 315 is 316,
/// melee 320 is 321), as every weapon both builds ship shows.
pub const RULES_031A2: Rules = Rules {
    state_types: &[(312, 313), (313, 314), (314, 315), (315, 316), (320, 321)],
    same_state_types_up_to: 311,
    renames: RENAMES,
    scaled: SCALED,
    clips_12_byte_events: true,
    mesh_statistics: true,
    aim_offset_in_config: true,
};

/// 0.46c2: 0.31a2's state types (its AK-12, PM, SVD, Saiga and the ten
/// guns of `ports/weapons-046c2.json`), 12-byte clip event headers. Its
/// stamina key is spelt as 0.26g's after skhid-srp's weapon-balance-1, and
/// its stamina damage is no fraction of 0.26g's (0.1 against the live
/// AK-12's 20): a port takes that from the template.
pub const RULES_046C2: Rules = Rules {
    state_types: &[(312, 313), (313, 314), (314, 315), (315, 316), (320, 321)],
    same_state_types_up_to: 311,
    renames: &[("health_modifier", "pain_health_modifier")],
    scaled: &[],
    clips_12_byte_events: true,
    mesh_statistics: true,
    aim_offset_in_config: true,
};

impl DonorBuild {
    pub fn rules(self) -> &'static Rules {
        match self {
            Self::V029a3 => &RULES_029A3,
            Self::V031a2 => &RULES_031A2,
            Self::V046c2 => &RULES_046C2,
        }
    }
}

impl Rules {
    /// 0.26g's state type for the build's `kind`.
    pub fn state_type(&self, kind: i32) -> Result<i32> {
        if let Some((_, native)) = self.state_types.iter().find(|(theirs, _)| *theirs == kind) {
            return Ok(*native);
        }
        ensure!(
            (300..=self.same_state_types_up_to).contains(&kind),
            "state type {kind} has no known 0.26g counterpart"
        );
        Ok(kind)
    }
}

/// A build's files, in 0.26g's layouts.
pub trait Donor {
    fn build(&self) -> DonorBuild;
    fn rules(&self) -> &'static Rules {
        self.build().rules()
    }
    fn contains(&self, path: &str) -> bool;
    fn is_file(&self, path: &str) -> bool;
    /// The files under `prefix`, sorted.
    fn paths_with_prefix(&self, prefix: &str) -> Vec<String>;
    /// A file in 0.26g's layout.
    fn read(&mut self, path: &str) -> Result<Vec<u8>>;
    /// A config file, parsed.
    fn config(&mut self, path: &str) -> Result<ConfigValue> {
        let bytes = self.read(path)?;
        Ok(BinaryConfig::parse(&bytes)
            .with_context(|| format!("parsing donor {path}"))?
            .root)
    }
    /// An item's config, by the name a manifest gives it (a config path
    /// for a build that ships item configs as files).
    fn item_config(&mut self, item: &str) -> Result<ConfigValue> {
        self.config(item)
    }
    /// A lobby cell (171x83 RGBA PNG) of the donor's icon `png`, turned by
    /// `angle` degrees for a build that draws icons otherwise than 0.26g.
    fn lobby_cell(&mut self, png: &str, angle: Option<f32>) -> Result<Vec<u8>> {
        ensure!(angle.is_none(), "{png}: this build's icons are not turned");
        super::icons::cell_png(&self.read(png)?)
    }
    /// The build's `gameplay/db_static_dictionaries`.
    fn dictionary(&mut self) -> Result<ConfigValue> {
        self.config("gameplay/db_static_dictionaries")
    }
}

/// A build in 0.26g's formats: one archive or a split build's volumes.
pub struct ArchiveDonor {
    db: ResourceDb,
    build: DonorBuild,
}

impl ArchiveDonor {
    pub fn open(path: &Path, build: DonorBuild) -> Result<Self> {
        Ok(Self {
            db: ResourceDb::open(path)?,
            build,
        })
    }
}

impl Donor for ArchiveDonor {
    fn build(&self) -> DonorBuild {
        self.build
    }
    fn contains(&self, path: &str) -> bool {
        self.db.contains(path)
    }
    fn is_file(&self, path: &str) -> bool {
        self.db.is_file(path)
    }
    fn paths_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.db
            .paths_with_prefix(prefix)
            .filter(|p| self.db.is_file(p))
            .map(str::to_owned)
            .collect()
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let bytes = self
            .db
            .read(path)
            .with_context(|| format!("reading donor {path}"))?;
        let rules = self.rules();
        if path.ends_with(".b-spline") && rules.clips_12_byte_events {
            return clips::events_12_to_16(&bytes).with_context(|| format!("converting {path}"));
        }
        if path.ends_with("/converted_model") {
            return if rules.mesh_statistics {
                strip_mesh_statistics(&bytes).with_context(|| path.to_owned())
            } else {
                Ok(bytes)
            };
        }
        if is_config(path) {
            ensure!(
                !BinaryConfig::is_compact(&bytes),
                "{path} is not a 24-byte-record config"
            );
        }
        Ok(bytes)
    }
}

/// Whether `path` is a binary config (not a texture, sound, clip or mesh).
pub fn is_config(path: &str) -> bool {
    ![
        ".dds",
        ".ogg",
        ".rms",
        ".b-spline",
        "/bind_pose",
        "/converted_model",
        ".png",
        ".swf",
        ".spl",
    ]
    .iter()
    .any(|e| path.ends_with(e))
}

/// Removes chunk 110 (0.31's 4-byte mesh statistics) after 0.26g's chunks 3
/// and 4; any other chunk is refused.
pub fn strip_mesh_statistics(b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(b.len());
    let mut at = 0;
    let mut ids = vec![];
    while at < b.len() {
        let head = b.get(at..at + 8).context("truncated chunk header")?;
        let id = u32::from_le_bytes(head[..4].try_into()?);
        let size = u32::from_le_bytes(head[4..].try_into()?) as usize;
        let end = at + 8 + size;
        ensure!(end <= b.len(), "truncated chunk {id}");
        match id {
            3 | 4 => out.extend_from_slice(&b[at..end]),
            110 => ensure!(size == 4, "chunk 110 of {size} bytes"),
            other => bail!("unknown mesh chunk {other}"),
        }
        ids.push(id);
        at = end;
    }
    ensure!(ids.starts_with(&[3, 4]), "mesh chunks {ids:?}");
    Ok(out)
}
