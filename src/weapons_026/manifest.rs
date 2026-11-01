// SPDX-License-Identifier: GPL-3.0-or-later
//! A weapon port, as data: `ports/weapons-*.json` (`docs/weapons-026.md`
//! says how to write one). Everything a port decides (ids, cells, the
//! template, the balance) is here; how a donor's files become 0.26g's is the
//! core's and the donor's.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::donor::DonorBuild;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponPort {
    pub format_version: u32,
    /// The bundle's asset id (`weapons-031`).
    pub id: String,
    pub donor_build: DonorBuild,
    #[serde(default)]
    pub weapons: Vec<Weapon>,
    #[serde(default)]
    pub modules: Vec<Module>,
    /// Ammunition ids the guns take that another patch adds (not the
    /// target's dictionary): patch 19's own rounds, for its comparison.
    #[serde(default)]
    pub external_ammo: Vec<i32>,
}

/// One gun: a new item of 0.26g's.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    /// Its `items_dict` name.
    pub name: String,
    pub dict_id: i32,
    /// The donor's item (its config path, or its item name for a build that
    /// keeps items in one file).
    pub donor_item: String,
    /// Where its config goes (`gameplay/items/weapons/wpn_<name>.options`:
    /// the edit lists' `wpn_*` globs then cover it).
    pub config: String,
    /// A native config of the same class: its keys, the states and parts
    /// that play clips 0.26g lacks, its dictionary row, the native systems'
    /// values ([`super::config::NATIVE_PATHS`]).
    pub template: String,
    /// Silver (its dictionary row's `base_cost`).
    pub base_cost: i32,
    /// A premium item (its dictionary row's `is_premium`; gold, not sold
    /// for silver).
    #[serde(default)]
    pub premium: bool,
    /// The ammunition it takes (`items_compatibility`), 0.26g dict ids.
    pub ammo: Vec<i32>,
    /// Config values, by dotted path, over everything else (the balance:
    /// faction, levels, damage, ...). Typed as the template has them.
    #[serde(default)]
    pub set: BTreeMap<String, serde_json::Value>,
    /// A skin of the template's own weapon (a premium copy): its states,
    /// clips and sounds, are the template's (the donor's may add clips to
    /// 0.26g's own sets, which would play beside the native ones).
    #[serde(default)]
    pub copy_of_template: bool,
    /// Paths taken whole from the template, beyond the core's own.
    #[serde(default)]
    pub from_template: Vec<String>,
    /// Values over a carried config's (a material, a model's settings), by
    /// its path then dotted key: a fix of the donor's file.
    #[serde(default)]
    pub file_values: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// The lobby icon: a cell of `LobbyMenu_WeaponIcons` and the donor's
    /// image (`None`: the bundle draws none).
    pub lobby_icon: Option<LobbyIcon>,
    /// The kill feed icon (`ui_desc.combat_log_icon`).
    pub kill_feed: KillFeed,
    /// Why the port is as it is, for the record.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LobbyIcon {
    pub row: u32,
    pub col: u32,
    /// The donor's icon image (a PNG no larger than a cell).
    pub donor_png: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KillFeed {
    /// The icon number: frame (icon + 1) of the hud's weapon clip, or the
    /// hud's `CombatLogWeapon<icon>` bitmap past it.
    pub icon: i32,
    /// The donor's kill log image the bundle carries for the hud
    /// (`kill-feed/CombatLogWeapon<icon>.png`); `None` for a stock icon.
    pub donor_png: Option<String>,
}

/// An attachable module (the attachments track): a mesh and its item.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub kind: ModuleKind,
    pub dict_id: i32,
    /// The donor's static model (`models/<model>.model`).
    pub donor_model: String,
    /// Where the model goes (`models/<model>.model`), and its item config.
    pub model: String,
    pub config: String,
    /// A native config of the kind (its keys and dictionary row).
    pub template: String,
    pub base_cost: i32,
    /// The locator of the gun it attaches at, by kind unless given.
    pub locator: Option<String>,
    #[serde(default)]
    pub set: BTreeMap<String, serde_json::Value>,
}

/// The kinds of module the kit carries. No suppressors (the owner,
/// 2026-11-01): a muzzle device is a flash hider or a muzzle brake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    Scope,
    FlashHider,
    MuzzleBrake,
    Grip,
}

impl ModuleKind {
    /// The gun's locator the module's origin goes to (0.69d0's, which our
    /// meshes share: `docs/weapons-026.md`).
    pub fn locator(self) -> &'static str {
        match self {
            Self::Scope => "scope_point",
            Self::FlashHider | Self::MuzzleBrake => "muzzle_point",
            Self::Grip => "grip_point",
        }
    }
}

impl WeaponPort {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let port: Self =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        port.check()?;
        Ok(port)
    }

    /// Ids, names, configs and cells distinct within the port.
    pub fn check(&self) -> Result<()> {
        ensure!(self.format_version == 1, "format_version 1 only");
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut configs = std::collections::BTreeSet::new();
        let mut cells = std::collections::BTreeSet::new();
        for w in &self.weapons {
            ensure!(ids.insert(w.dict_id), "dict id {} twice", w.dict_id);
            ensure!(names.insert(w.name.clone()), "{} twice", w.name);
            ensure!(configs.insert(w.config.clone()), "{} twice", w.config);
            if let Some(icon) = &w.lobby_icon {
                ensure!(
                    cells.insert((icon.row, icon.col)),
                    "lobby cell {},{} twice",
                    icon.row,
                    icon.col
                );
            }
            ensure!(!w.ammo.is_empty(), "{} takes no ammo", w.name);
        }
        for m in &self.modules {
            ensure!(ids.insert(m.dict_id), "dict id {} twice", m.dict_id);
            ensure!(names.insert(m.name.clone()), "{} twice", m.name);
            ensure!(configs.insert(m.config.clone()), "{} twice", m.config);
        }
        Ok(())
    }
}
