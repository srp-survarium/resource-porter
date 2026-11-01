// SPDX-License-Identifier: GPL-3.0-or-later
//! Weapons of Survarium 0.69d0 (2022) for v0.26g, read from the build's
//! split volumes. See `docs/weapons-069d-026.md`.
//!
//! The 0.69d item configs (`gameplay/game.options`, `weapons.<key>`) name each
//! weapon's model, rig, attachment modules, clips and sounds. A ported weapon
//! is a native 0.26g weapon's config (its `template`: the states, melee,
//! grenade, recoil and lobby poses of a live peer) with:
//!
//! * the donor's model converted ([`convert`]): its skinned surfaces and the
//!   **structural** attachment modules baked in at their locators (receiver
//!   parts, stocks, pistol grips, magazines, bolts, the bare barrel). Muzzle
//!   devices, foregrips and scopes are never baked: 0.26g's weapon kit
//!   attaches them at the locators the port records (`attach-points.json`);
//!   suppressors are not ported at all;
//! * either the live peer's rig and clips ([`Rig::Live`]: the donor rig is the
//!   same skeleton, bone for bone) or the donor's own skeleton and clips
//!   ([`Rig::Donor`]), state by state ([`StatePlan`]);
//! * the donor's sounds where 0.26g lacks them, without 0.69d's
//!   per-location reverb layers (`weapons/environments/*`, which no 0.26g map
//!   defines) and without suppressed variants;
//! * statistics fitted to the live balance (owner, 2026-11-01), ammo of a live
//!   weapon of the same calibre or a new ammo item.
//!
//! Every new file is under a `v069d` folder (models, materials, textures,
//! skeletons, clips, sounds), so no path is shared with another port.
pub mod convert;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::json;

use crate::manifest::BuiltFile;
use crate::weapon::{parse_config, prepare_empty_output, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use convert::Skinned;

pub(crate) const DICT: &str = "gameplay/db_static_dictionaries";
const MATERIAL_TEMPLATE: &str = "material_instances/weapons/ak_12.material";
const SETTINGS_TEMPLATE: &str = "models/weapons/ak_12/ak_12.skinned_model/settings";
/// A native weapon's hands clip: the 0.26g player rig every hands clip drives.
const NATIVE_HANDS_CLIP: &str =
    "animations/single/weapons/ak_12/player/stand/3rd_view/on_site/hands_only/reload.bones_names";
pub(crate) const SOURCE: &str = "converted:0.69d0";
/// The folder every new file goes under, after its kind's root.
pub const NS: &str = "v069d";

/// 0.69d weapon rigs keep their meshes rotated half a turn about Y from the
/// frame their bind pose and locators are written in (x and z negated).
pub(crate) fn mesh_from_bind(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], -p[2]]
}

/// Whose skeleton and clips the weapon uses.
#[derive(Clone, Copy, Debug)]
pub enum Rig {
    /// The live weapon's: the donor rig is the same skeleton (bone names and
    /// order checked). `model` is the live skinned model whose bind pose,
    /// skeleton file and hand locators the port takes, so the live clips
    /// play on it as on the live weapon.
    Live { model: &'static str },
    /// The donor's own skeleton and clips, converted.
    Donor,
}

/// Where a state's clips or sounds come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Template,
    Donor,
}

/// One state of the item config.
#[derive(Clone, Copy, Debug)]
pub struct StatePlan {
    pub state: &'static str,
    pub clips: Src,
    pub sounds: Src,
}

const fn donor(state: &'static str) -> StatePlan {
    StatePlan {
        state,
        clips: Src::Donor,
        sounds: Src::Donor,
    }
}
const fn template(state: &'static str) -> StatePlan {
    StatePlan {
        state,
        clips: Src::Template,
        sounds: Src::Template,
    }
}
const fn donor_sounds(state: &'static str) -> StatePlan {
    StatePlan {
        state,
        clips: Src::Template,
        sounds: Src::Donor,
    }
}

/// The ammo a weapon fires.
#[derive(Clone, Copy, Debug)]
pub enum Ammo {
    /// Every ammo a live weapon (its `items_dict` name) takes.
    Like(&'static str),
    /// A new ammo item of this port ([`NewAmmo`]).
    New(&'static NewAmmo),
}

/// A new ammo item: a live ammo's config with its own name, damage and icon.
#[derive(Clone, Copy, Debug)]
pub struct NewAmmo {
    pub dict_id: i32,
    pub name: &'static str,
    pub config: &'static str,
    /// The live ammo the config and dictionary row start from.
    pub template: &'static str,
    pub base_cost: i32,
    pub buckshot: i32,
    /// `parameters.damage` and `data.k_damage`: a pellet's share.
    pub damage: f32,
    /// `parameters.arp` and `data.k_arp`.
    pub arp: f32,
    pub clip_size: i32,
    pub clip_weight: f32,
    /// Cell [row, col] of `LobbyMenu_AmmoIcons`.
    pub icon: [i32; 2],
    /// Text keys `<text>_name` and `<text>_desc`.
    pub text: &'static str,
}

/// Statistics fitted to the live balance (`docs/weapons-069d-026.md`).
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub faction: i32,
    pub level: i32,
    pub drop_weight: i32,
    pub base_cost: i32,
    pub rounds_per_minute: i32,
    pub bullet_damage: f32,
    pub bullet_pierce: f32,
    pub weight: f32,
    pub reload_time: f32,
    pub aim_time: f32,
    pub base_dispersion: f32,
    pub magazine: i32,
    pub min_damage_distance: i32,
    pub max_damage_distance: i32,
    /// -1 automatic, 1 single, 3 burst.
    pub fire_modes: &'static [i32],
    /// Pump and bolt guns only.
    pub chamber_a_round_time: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Gun {
    /// `weapons.<key>` of 0.69d's `gameplay/game.options`.
    pub key: &'static str,
    pub dict_id: i32,
    /// `items_dict` name (and the folder of its files).
    pub name: &'static str,
    pub config: &'static str,
    /// The live config every item config starts from.
    pub template: &'static str,
    pub rig: Rig,
    pub states: &'static [StatePlan],
    /// Module ids (`weapon_modules.<id>`) of the donor's bundled and default
    /// modules that are **not** baked: muzzle devices (the weapon kit's), and
    /// any foregrip, scope or suppressor. Every other one is structural.
    pub separable: &'static [&'static str],
    /// Baked modules turned about the vertical axis (degrees) before they are
    /// placed: meshes 0.69d authored facing another way than the weapon.
    pub module_turns: &'static [(&'static str, f32)],
    pub ammo: Ammo,
    pub stats: Stats,
    /// Cell [row, col] of `LobbyMenu_WeaponIcons`.
    pub icon: [i32; 2],
    /// Kill-feed icon (`ui_desc.combat_log_icon`).
    pub combat_log_icon: i32,
}

impl Gun {
    pub fn name_key(&self) -> String {
        format!("st_srp_weapon069_{}_name", self.name)
    }
    pub fn description_key(&self) -> String {
        format!("st_srp_weapon069_{}_desc", self.name)
    }
    pub fn model_ref(&self) -> String {
        format!("weapons/{NS}/{0}/{0}", self.name)
    }
    pub fn model_root(&self) -> String {
        format!("models/{}.skinned_model", self.model_ref())
    }
}

const SHOW_HIDE_IDLE_FIRE_RELOAD: [StatePlan; 5] = [
    donor("show"),
    donor("hide"),
    donor("idle"),
    donor("fire"),
    donor("reload"),
];

/// 23x75 mm R buckshot for the KS-23M: 0.69d's `ammo_23mm_r` (1251) is its
/// 12/70 buckshot's twin (12 pellets of 0.08334 each, as `ammo_12mm_buck`),
/// so here it is the live 12x70 buckshot's twin too.
pub const AMMO_23MM: NewAmmo = NewAmmo {
    dict_id: 2253,
    name: "ammo_23mm_r",
    config: "gameplay/items/weapons/ammo/ammo_23mm_r.options",
    template: "ammo_12mm_buck",
    base_cost: 20,
    buckshot: 10,
    damage: 0.2,
    arp: 0.2,
    clip_size: 15,
    clip_weight: 0.6,
    icon: [22, 4],
    text: "st_srp_ammo069_23mm_r",
};

pub const AKMSU: Gun = Gun {
    key: "akmsu",
    dict_id: 2290,
    name: "akmsu",
    config: "gameplay/items/weapons/wpn_akmsu.options",
    template: "gameplay/items/weapons/wpn_ak_74u.options",
    rig: Rig::Live {
        model: "models/weapons/ak74u.skinned_model",
    },
    // The AK family's own clips and reload sounds (the AKS-74U's); 0.69d's
    // AKM firing sound (`weapons/akm_n`, 0.26g's AKM's).
    states: &[
        template("show"),
        template("hide"),
        template("idle"),
        donor_sounds("fire"),
        template("reload"),
    ],
    separable: &["akmsu_m24x15_muzzlebrake"],
    module_turns: &[],
    ammo: Ammo::Like("akmn"),
    stats: Stats {
        faction: 3,
        level: 6,
        drop_weight: 100,
        base_cost: 19_000,
        rounds_per_minute: 625,
        bullet_damage: 0.45,
        bullet_pierce: 0.4,
        weight: 2.9,
        reload_time: 3.167,
        aim_time: 0.3,
        base_dispersion: 0.3,
        magazine: 30,
        min_damage_distance: 40,
        max_damage_distance: 15,
        fire_modes: &[-1, 1],
        chamber_a_round_time: None,
    },
    icon: [17, 5],
    combat_log_icon: 96,
};

pub const FAMAS: Gun = Gun {
    key: "famas",
    dict_id: 2291,
    name: "famas",
    config: "gameplay/items/weapons/wpn_famas.options",
    template: "gameplay/items/weapons/wpn_ash_12.options",
    rig: Rig::Live {
        model: "models/weapons/ash_12/ash_12.skinned_model",
    },
    // 0.69d plays the Ash-12's clips but for the reload (its own, on the
    // Ash-12 rig), and the M4A1's firing sound.
    states: &[
        template("show"),
        template("hide"),
        template("idle"),
        donor_sounds("fire"),
        donor("reload"),
    ],
    separable: &["famas_default_muzzlebrake"],
    module_turns: &[],
    ammo: Ammo::Like("hk416"),
    stats: Stats {
        faction: 1,
        level: 14,
        drop_weight: 70,
        base_cost: 100_000,
        rounds_per_minute: 900,
        bullet_damage: 0.5,
        bullet_pierce: 0.32,
        weight: 3.4,
        reload_time: 4.0,
        aim_time: 0.3,
        base_dispersion: 0.2,
        magazine: 30,
        min_damage_distance: 50,
        max_damage_distance: 20,
        fire_modes: &[-1, 1, 3],
        chamber_a_round_time: None,
    },
    icon: [18, 5],
    combat_log_icon: 97,
};

pub const G3SG1: Gun = Gun {
    key: "g3a3",
    dict_id: 2292,
    name: "g3sg1",
    config: "gameplay/items/weapons/wpn_g3sg1.options",
    template: "gameplay/items/weapons/wpn_socom16.options",
    rig: Rig::Donor,
    states: &SHOW_HIDE_IDLE_FIRE_RELOAD,
    separable: &["g3a3_default_muzzlebrake"],
    module_turns: &[],
    ammo: Ammo::Like("rem_700"),
    stats: Stats {
        faction: 2,
        level: 15,
        drop_weight: 40,
        base_cost: 80_000,
        rounds_per_minute: 240,
        bullet_damage: 0.64,
        bullet_pierce: 0.4,
        weight: 5.2,
        reload_time: 3.2,
        aim_time: 0.3,
        base_dispersion: 0.2,
        magazine: 10,
        min_damage_distance: 100,
        max_damage_distance: 30,
        fire_modes: &[1, 3],
        chamber_a_round_time: None,
    },
    icon: [19, 5],
    combat_log_icon: 98,
};

pub const MAUSER: Gun = Gun {
    key: "mauser",
    dict_id: 2293,
    name: "mauser_m712",
    config: "gameplay/items/weapons/wpn_mauser_m712.options",
    template: "gameplay/items/weapons/wpn_aps.options",
    rig: Rig::Donor,
    states: &SHOW_HIDE_IDLE_FIRE_RELOAD,
    separable: &[],
    module_turns: &[],
    ammo: Ammo::Like("tt_33"),
    stats: Stats {
        faction: 3,
        level: 6,
        drop_weight: 100,
        base_cost: 13_000,
        rounds_per_minute: 580,
        bullet_damage: 0.35,
        bullet_pierce: 0.25,
        weight: 1.7,
        reload_time: 1.5,
        aim_time: 0.25,
        base_dispersion: 0.35,
        magazine: 10,
        min_damage_distance: 40,
        max_damage_distance: 15,
        fire_modes: &[-1, 1],
        chamber_a_round_time: None,
    },
    icon: [20, 5],
    combat_log_icon: 86,
};

pub const KS23M: Gun = Gun {
    key: "ks_23",
    dict_id: 2294,
    name: "ks_23m",
    config: "gameplay/items/weapons/wpn_ks_23m.options",
    template: "gameplay/items/weapons/wpn_rem_870.options",
    rig: Rig::Donor,
    states: &[
        donor("show"),
        donor("hide"),
        donor("idle"),
        donor("fire"),
        donor("reload"),
        donor("chamber_a_round"),
    ],
    separable: &[],
    module_turns: &[],
    ammo: Ammo::New(&AMMO_23MM),
    stats: Stats {
        faction: 3,
        level: 7,
        drop_weight: 90,
        base_cost: 26_000,
        rounds_per_minute: 180,
        bullet_damage: 1.2,
        bullet_pierce: 0.1,
        weight: 3.7,
        reload_time: 1.7,
        aim_time: 0.25,
        base_dispersion: 1.8,
        magazine: 4,
        min_damage_distance: 30,
        max_damage_distance: 10,
        fire_modes: &[1],
        chamber_a_round_time: Some(0.75),
    },
    icon: [21, 5],
    combat_log_icon: 85,
};

pub const PPSH_DRUM: Gun = Gun {
    key: "ppsh_drum",
    dict_id: 2295,
    name: "ppsh_drum",
    config: "gameplay/items/weapons/wpn_ppsh_drum.options",
    template: "gameplay/items/weapons/wpn_ppsh.options",
    rig: Rig::Live {
        model: "models/weapons/ppsh/ppsh.skinned_model",
    },
    // The PPSh-41's clips, but the drum's own reload.
    states: &[
        template("show"),
        template("hide"),
        template("idle"),
        template("fire"),
        donor("reload"),
    ],
    separable: &[],
    // The 71-round drum's mesh lies with its faces fore and aft (its box,
    // 148 x 158 x 55 mm, is widest across); the PPSh-41 carries it with its
    // faces to the sides, as 0.69d's own icon shows it.
    module_turns: &[("ppsh_762x25_71_round_mag", 90.0)],
    ammo: Ammo::Like("ppsh"),
    stats: Stats {
        faction: 1,
        level: 2,
        drop_weight: 120,
        base_cost: 6_000,
        rounds_per_minute: 1000,
        bullet_damage: 0.3,
        bullet_pierce: 0.25,
        weight: 3.9,
        reload_time: 4.6,
        aim_time: 0.3,
        base_dispersion: 0.4,
        magazine: 71,
        min_damage_distance: 40,
        max_damage_distance: 15,
        fire_modes: &[-1, 1],
        chamber_a_round_time: None,
    },
    icon: [22, 5],
    combat_log_icon: 33,
};

pub const GUNS: [&Gun; 6] = [&AKMSU, &FAMAS, &G3SG1, &MAUSER, &KS23M, &PPSH_DRUM];

pub fn gun(key: &str) -> Result<&'static Gun> {
    GUNS.iter()
        .copied()
        .find(|g| g.key == key || g.name == key)
        .with_context(|| format!("no 0.69d gun {key}"))
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

pub(crate) fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}
pub(crate) fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}
pub(crate) fn float3(v: Option<&V>) -> Result<[f32; 3]> {
    match v {
        Some(V::Float3(f)) => Ok(*f),
        other => bail!("expected a float3, found {other:?}"),
    }
}
pub(crate) fn string_of(v: Option<&V>) -> Result<&str> {
    match v {
        Some(V::String(s)) => Ok(s),
        other => bail!("expected a string, found {other:?}"),
    }
}
pub(crate) fn int_of(v: Option<&V>) -> Result<i32> {
    match v {
        Some(V::Integer(i)) => Ok(*i),
        other => bail!("expected an integer, found {other:?}"),
    }
}
pub(crate) fn at<'v>(root: &'v V, path: &str) -> Option<&'v V> {
    path.split('.').try_fold(root, |v, k| v.named(k))
}
pub(crate) fn list(v: Option<&V>) -> &[V] {
    match v {
        Some(V::Indexed(items)) => items,
        _ => &[],
    }
}

/// `weapons/<rest>` -> `weapons/v069d/<rest>`: a donor resource name in this
/// port's own folder.
pub fn ns_name(name: &str) -> Result<String> {
    let rest = name
        .strip_prefix("weapons/")
        .with_context(|| format!("{name} is not under weapons/"))?;
    Ok(format!("weapons/{NS}/{rest}"))
}

/// A clip `resources/animations/single/weapons/<rest>` in this port's folder.
fn ns_clip(path: &str) -> Result<String> {
    let rest = path
        .strip_prefix("resources/animations/single/weapons/")
        .with_context(|| format!("{path} is not a weapon clip"))?;
    Ok(format!("resources/animations/single/weapons/{NS}/{rest}"))
}

pub(crate) struct Out<'a> {
    pub(crate) overlay: &'a Path,
    pub(crate) work: &'a Path,
    pub(crate) files: Vec<BuiltFile>,
    pub(crate) written: BTreeSet<String>,
}

impl Out<'_> {
    pub(crate) fn raw(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        let dest = crate::archive::safe_join(self.overlay, path)?;
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
        self.files.push(record_file(path, SOURCE, bytes));
        Ok(())
    }
    pub(crate) fn config(&mut self, path: &str, root: V) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        write_generated_config(
            self.overlay,
            path,
            &BinaryConfig { root },
            SOURCE,
            &mut self.files,
        )
    }
    pub(crate) fn has(&self, path: &str) -> bool {
        self.written.contains(path)
    }
    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>> {
        Ok(fs::read(crate::archive::safe_join(self.overlay, path)?)?)
    }
}

pub(crate) fn donor_config(donor: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    let bytes = donor
        .read(path)
        .with_context(|| format!("reading donor {path}"))?;
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing donor {path}"))
}

/// A bind pose: per bone its name and translation.
pub(crate) fn bind_pose(b: &[u8]) -> Result<Vec<(String, [f32; 3])>> {
    ensure!(b.len() >= 2, "bind pose shorter than its count");
    let n = usize::from(u16::from_le_bytes([b[0], b[1]]));
    let mut at = 2;
    let mut out = vec![];
    for _ in 0..n {
        let end = b[at..]
            .iter()
            .position(|c| *c == 0)
            .context("unterminated bone name")?;
        let name = String::from_utf8(b[at..at + end].to_vec())?;
        at += end + 1;
        let f: Vec<f32> = b
            .get(at..at + 36)
            .context("truncated bind frame")?
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        at += 36;
        ensure!(
            f[6..9].iter().all(|s| (s - 1.0).abs() < 1e-4),
            "bone {name} is scaled"
        );
        out.push((name, [f[0], f[1], f[2]]));
    }
    ensure!(at == b.len(), "bind pose has trailing bytes");
    Ok(out)
}

/// `bones_names` of a clip.
fn clip_bones(bytes: &[u8]) -> Result<Vec<String>> {
    let config = BinaryConfig::parse(bytes)?;
    list(config.root.named("bones_names"))
        .iter()
        .map(|n| match n {
            V::String(s) => Ok(s.clone()),
            _ => bail!("bones_names holds a non-string"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The donor's item config and modules
// ---------------------------------------------------------------------------

/// 0.69d's item configs: `gameplay/game.options`.
pub struct Game {
    pub(crate) root: V,
}

impl Game {
    pub fn load(donor: &mut ResourceDb) -> Result<Self> {
        Ok(Self {
            root: donor_config(donor, "gameplay/game.options")?.root,
        })
    }
    pub(crate) fn weapon(&self, key: &str) -> Result<&V> {
        at(&self.root, &format!("weapons.{key}"))
            .with_context(|| format!("0.69d has no weapons.{key}"))
    }
    pub(crate) fn module(&self, id: &str) -> Result<&V> {
        at(&self.root, &format!("weapon_modules.{id}"))
            .with_context(|| format!("0.69d has no weapon_modules.{id}"))
    }
}

/// One of the donor's bundled or default modules, resolved to its model and
/// the rig locator it sits at.
#[derive(Clone, Debug)]
pub struct PlacedModule {
    pub id: String,
    pub model: String,
    pub slot: String,
    pub locator: String,
    pub baked: bool,
}

/// The donor weapon's modules in their attach order (bundled, then default),
/// each at the locator its slot names after the slot overrides of the baked
/// modules before it.
fn placed_modules(game: &Game, gun: &Gun) -> Result<Vec<PlacedModule>> {
    let w = game.weapon(gun.key)?;
    let mut slots: BTreeMap<String, String> = BTreeMap::new();
    for slot in list(w.named("slots")) {
        slots.insert(
            string_of(slot.named("id"))?.into(),
            string_of(slot.named("locator"))?.into(),
        );
    }
    let mut ids = vec![];
    for kind in ["bundled_modules", "default_modules"] {
        for m in list(w.named(kind)) {
            ids.push(string_of(m.named("module"))?.to_owned());
        }
    }
    for id in gun.separable {
        ensure!(
            ids.iter().any(|i| i == id),
            "{}: {id} is none of its modules",
            gun.key
        );
    }
    let mut out = vec![];
    for id in ids {
        let module = game.module(&id)?;
        let model = match at(module, &format!("exceptions.{}.model", gun.key)) {
            Some(v) => string_of(Some(v))?.to_owned(),
            None => string_of(module.named("model"))?.to_owned(),
        };
        let attach = list(module.named("slots_to_attach_to"));
        ensure!(attach.len() == 1, "{id} attaches to {} slots", attach.len());
        let slot = string_of(attach[0].named("slot"))?.to_owned();
        let locator = slots
            .get(&slot)
            .with_context(|| format!("{}: {id}'s slot {slot} has no locator", gun.key))?
            .clone();
        let baked = !gun.separable.contains(&id.as_str());
        ensure!(
            !(baked && is_attachment(&id, &model)),
            "{}: {id} ({model}) is an attachment, not structural; list it as separable",
            gun.key
        );
        if baked {
            for key in ["slots_to_override", "slots_to_add"] {
                for o in list(at(module, &format!("attach_params.{key}"))) {
                    slots.insert(
                        string_of(o.named("id"))?.into(),
                        string_of(o.named("locator"))?.into(),
                    );
                }
            }
        }
        out.push(PlacedModule {
            id,
            model,
            slot,
            locator,
            baked,
        });
    }
    Ok(out)
}

/// Muzzle devices, suppressors, foregrips and optics: never baked.
fn is_attachment(id: &str, model: &str) -> bool {
    const KINDS: [&str; 7] = [
        "muzzlebrake",
        "flashhider",
        "silencer",
        "suppressor",
        "foregrip",
        "scope",
        "collimator",
    ];
    KINDS
        .iter()
        .any(|k| id.contains(k) || model.contains(&format!("/{k}")))
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Locator {
    pub(crate) name: String,
    pub(crate) bone: String,
    pub(crate) bone_idx: i32,
    pub(crate) position: [f32; 3],
    pub(crate) rotation: [f32; 3],
}

pub(crate) fn locators(properties: &V) -> Result<Vec<Locator>> {
    list(properties.named("locators"))
        .iter()
        .map(|l| {
            Ok(Locator {
                name: string_of(l.named("name"))?.into(),
                bone: string_of(l.named("bone_name"))?.into(),
                bone_idx: int_of(l.named("bone_idx"))?,
                position: float3(l.named("position"))?,
                rotation: float3(l.named("rotation"))?,
            })
        })
        .collect()
}

pub(crate) fn locator_value(l: &Locator) -> V {
    // Native field order (the AK-12's).
    V::Named(vec![
        field("rotation", V::Float3(l.rotation)),
        field("position", V::Float3(l.position)),
        field("name", s(&l.name)),
        field("bone_idx", V::Integer(l.bone_idx)),
        field("bone_name", s(&l.bone)),
    ])
}

/// Where a module's origin lands in mesh space: the socket bone's bind
/// translation plus the locator's position, both in the bind frame.
fn socket(loc: &Locator, bind: &[(String, [f32; 3])]) -> Result<[f32; 3]> {
    let (_, t) = bind
        .iter()
        .find(|(n, _)| *n == loc.bone)
        .with_context(|| format!("{} names an unknown bone {}", loc.name, loc.bone))?;
    let p = mesh_from_bind(loc.position);
    Ok([t[0] + p[0], t[1] + p[1], t[2] + p[2]])
}

/// A socket's turn in mesh space. Only a turn about the bore's crosswise
/// axis (x) is met (the PPSh-41's drum, -3.7 degrees); in mesh space, half a
/// turn about y from the bind frame, it is the opposite angle about x.
fn socket_rotation(loc: &Locator) -> Result<[[f32; 3]; 3]> {
    let [x, y, z] = loc.rotation;
    ensure!(
        y.abs() < 1e-5 && z.abs() < 1e-5,
        "{} turns about y or z ({:?}); only turns about x are baked",
        loc.name,
        loc.rotation
    );
    let a = -x * SOCKET_TURN_SIGN;
    let (s, c) = a.sin_cos();
    Ok([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
}

fn turn_about_y(a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

/// `a` after `b`: the matrix applying `b` first.
fn multiply3(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// The sense of a socket's turn about x: checked on the PPSh-41's drum,
/// which then sits in the magazine well as 0.69d draws it.
const SOCKET_TURN_SIGN: f32 = 1.0;

/// A point the weapon kit may attach to: a donor locator (its bone and its
/// bind-frame position from that bone) and the donor module left out there.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AttachPoint {
    pub locator: String,
    pub bone: String,
    pub position: [f32; 3],
    pub module: Option<String>,
}

struct Model {
    surfaces: Vec<(String, Skinned, String)>,
    attach_points: Vec<AttachPoint>,
    modules: Vec<PlacedModule>,
}

/// Locator names of the 0.26g contract that are made, not copied.
const CONTRACT: [&str; 4] = ["aim", "barrel_point", "scope_point", "handguard_point"];
/// Donor locators a kit attachment can use.
const ATTACH_LOCATORS: [&str; 6] = [
    "muzzle_point",
    "fire_point",
    "scope_picatinnyrail_point",
    "picatinny_scope_point",
    "grip_picatinnyrail_point",
    "tactical_picatinnyrail_point",
];

/// A static model's LOD0 surfaces and their materials.
pub(crate) fn lod0_surfaces(settings: &V, path: &str) -> Result<Vec<(String, String)>> {
    let surfaces = list(at(settings, "lod_hierrarchy.LOD0.surfaces"));
    ensure!(!surfaces.is_empty(), "{path} has no LOD0 surface");
    surfaces
        .iter()
        .map(|surface| {
            let surface = string_of(Some(surface))?.to_owned();
            let material = string_of(at(
                settings,
                &format!("material_settings.{surface}.material_name"),
            ))?
            .to_owned();
            Ok((surface, material))
        })
        .collect()
}

fn is_hand_locator(name: &str) -> bool {
    ["_idle", "_lobby", "_reload", "_reload_pivot"]
        .iter()
        .any(|k| name.ends_with(k))
}

#[allow(clippy::too_many_lines)]
fn build_model(
    gun: &Gun,
    game: &Game,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<Model> {
    let w = game.weapon(gun.key)?;
    let donor_root = format!("models/{}.skinned_model", string_of(at(w, "object.model"))?);
    let bind_bytes = donor.read(&format!("{donor_root}/render/bind_pose"))?;
    let bind = bind_pose(&bind_bytes)?;
    let properties = donor_config(donor, &format!("{donor_root}/render/export_properties"))?;
    let all = locators(&properties.root)?;
    let find = |name: &str| {
        all.iter()
            .find(|l| l.name == name)
            .with_context(|| format!("{} has no {name} locator", gun.key))
    };
    // The bind pose shipped: the live model's for a live rig (its clips are
    // written for it), else the donor's.
    let (ship_bind, live_props) = match gun.rig {
        Rig::Live { model } => {
            let live = target.read(&format!("{model}/render/bind_pose"))?;
            let names: Vec<String> = bind_pose(&live)?.into_iter().map(|(n, _)| n).collect();
            let ours: Vec<String> = bind.iter().map(|(n, _)| n.clone()).collect();
            ensure!(
                names == ours,
                "{}: the donor rig's bones {ours:?} are not {model}'s {names:?}",
                gun.key
            );
            let props = parse_config(
                target,
                &format!("{model}/render/export_properties"),
                "live rig",
            )?;
            (live, Some(props.root))
        }
        Rig::Donor => (bind_bytes.clone(), None),
    };

    let ship_bones = bind_pose(&ship_bind)?;
    let settings = donor_config(donor, &format!("{donor_root}/settings"))?;
    let mut model = Model {
        surfaces: vec![],
        attach_points: vec![],
        modules: placed_modules(game, gun)?,
    };
    for surface in list(at(&settings.root, "lod_hierrarchy.LOD0.surfaces")) {
        let surface = string_of(Some(surface))?;
        let material = string_of(at(
            &settings.root,
            &format!("material_settings.{surface}.material_name"),
        ))?;
        let bytes = donor.read(&format!("{donor_root}/render/{surface}/converted_model"))?;
        let mesh = convert::skinned_069_to_026(&bytes, bind.len())?;
        model.surfaces.push((surface.into(), mesh, material.into()));
    }
    for module in model.modules.clone() {
        let loc = find(&module.locator)?;
        if !module.baked {
            model.attach_points.push(AttachPoint {
                locator: loc.name.clone(),
                bone: loc.bone.clone(),
                position: loc.position,
                module: Some(module.id.clone()),
            });
            continue;
        }
        let path = format!("models/{}.model", module.model);
        let msettings = donor_config(donor, &format!("{path}/settings"))?;
        // In the game a bone's vertices are drawn at clip * bind^-1 of the
        // shipped rig; 0.69d drew the module at clip * locator. So the
        // module sits at the shipped bind frame of its bone (the live rig's
        // on a live rig: the AKMSU's magazine bone rests 15 cm off in 0.69d's
        // bind pose, where the AK clips never take it).
        let origin = socket(loc, &ship_bones)?;
        let mut rotation = socket_rotation(loc)?;
        if let Some((_, degrees)) = gun.module_turns.iter().find(|(id, _)| *id == module.id) {
            rotation = multiply3(&rotation, &turn_about_y(degrees.to_radians()));
        }
        let bone = bind
            .iter()
            .position(|(n, _)| *n == loc.bone)
            .context("socket bone")? as u32;
        for (surface, material) in lod0_surfaces(&msettings.root, &path)? {
            let bytes = donor.read(&format!("{path}/render/{surface}/converted_model"))?;
            let mesh = convert::static_069_to_skinned(&bytes, rotation, origin, bone)?;
            let module_name = module.model.rsplit('/').next().unwrap();
            let name = format!("{module_name}_{surface}");
            ensure!(
                !model.surfaces.iter().any(|(n, _, _)| *n == name),
                "{}: two surfaces {name}",
                gun.key
            );
            model.surfaces.push((name, mesh, material));
        }
    }
    for name in ATTACH_LOCATORS {
        if let Ok(loc) = find(name)
            && !model.attach_points.iter().any(|a| a.locator == name)
        {
            model.attach_points.push(AttachPoint {
                locator: loc.name.clone(),
                bone: loc.bone.clone(),
                position: loc.position,
                module: None,
            });
        }
    }

    // Locators: 0.26g's contract, made from the donor's geometry (the bare
    // barrel's end is `muzzle_point`, where 0.69d mounts its devices), and
    // the hand locators (the live model's on a live rig).
    // A gun with no muzzle device slot (the PPSh-41, whose compensator is
    // its jacket) fires from its `fire_point`.
    let muzzle = find("muzzle_point").or_else(|_| find("fire_point"))?;
    let aim = find("aim")?;
    let mut locs = vec![
        Locator {
            name: "aim".into(),
            ..aim.clone()
        },
        Locator {
            name: "barrel_point".into(),
            rotation: [0.0; 3],
            ..muzzle.clone()
        },
    ];
    let scope = ["scope_picatinnyrail_point", "picatinny_scope_point"]
        .iter()
        .find_map(|n| find(n).ok())
        .unwrap_or(aim);
    locs.push(Locator {
        name: "scope_point".into(),
        rotation: [0.0; 3],
        ..scope.clone()
    });
    let handguard = ["handguard_point", "forend_point", "LeftHand_idle"]
        .iter()
        .find_map(|n| find(n).ok())
        .context("no handguard locator")?;
    locs.push(Locator {
        name: "handguard_point".into(),
        rotation: [0.0; 3],
        ..handguard.clone()
    });
    match &live_props {
        Some(live) => {
            let live = locators(live)?;
            locs.extend(
                live.into_iter()
                    .filter(|l| !CONTRACT.contains(&l.name.as_str())),
            );
        }
        None => locs.extend(all.iter().filter(|l| is_hand_locator(&l.name)).cloned()),
    }
    ensure!(
        locs.iter().filter(|l| l.name.ends_with("_idle")).count() == 32,
        "{}: not 32 idle hand locators",
        gun.key
    );

    // Files.
    let root = gun.model_root();
    let mut native_settings = parse_config(target, SETTINGS_TEMPLATE, "native rifle settings")?;
    native_settings.root.set_path(
        "lod_hierrarchy",
        V::Named(vec![field(
            "LOD0",
            V::Named(vec![field(
                "surfaces",
                V::Indexed(model.surfaces.iter().map(|(n, _, _)| s(n)).collect()),
            )]),
        )]),
    )?;
    let mut material_settings = vec![];
    for (n, _, m) in &model.surfaces {
        material_settings.push(field(
            n,
            V::Named(vec![
                field("material_name", s(ns_name(m)?)),
                field("used_in_portals", V::Boolean(false)),
            ]),
        ));
    }
    native_settings
        .root
        .set_path("material_settings", V::Named(material_settings))?;
    out.config(&format!("{root}/settings"), native_settings.root)?;
    match gun.rig {
        Rig::Live { model } => {
            let skeleton =
                parse_config(target, &format!("{model}/skeleton"), "live model skeleton")?;
            out.config(&format!("{root}/skeleton"), skeleton.root)?;
        }
        Rig::Donor => {
            let mut skeleton = donor_config(donor, &format!("{donor_root}/skeleton"))?.root;
            redirect_skeleton(&mut skeleton)?;
            out.config(&format!("{root}/skeleton"), skeleton)?;
            let name = string_of(at(w, "object.skeleton"))?;
            let ours = format!("animations/skeletons/{}.skeleton", ns_name(name)?);
            if !out.has(&ours) {
                out.config(
                    &ours,
                    donor_config(donor, &format!("animations/skeletons/{name}.skeleton"))?.root,
                )?;
            }
        }
    }
    out.raw(&format!("{root}/render/bind_pose"), &ship_bind)?;
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for (name, mesh, _) in &model.surfaces {
        let (a, b) = mesh.bounds();
        // Boxes are written in the bind frame, as the rig's (and native) ones.
        let (a, b) = (mesh_from_bind(a), mesh_from_bind(b));
        let (a, b) = (
            [a[0].min(b[0]), a[1], a[2].min(b[2])],
            [a[0].max(b[0]), b[1], a[2].max(b[2])],
        );
        for k in 0..3 {
            lo[k] = lo[k].min(a[k]);
            hi[k] = hi[k].max(b[k]);
        }
        let props = V::Named(vec![
            field(
                "source",
                V::Named(vec![
                    field("indices", V::Integer(i32::try_from(mesh.indices.len())?)),
                    field(
                        "primitives",
                        V::Integer(i32::try_from(mesh.indices.len() / 3)?),
                    ),
                    field("mesh_name", s(format!("|{}|0.69d0|{name}", gun.key))),
                    field("texture_name", s("")),
                    field("vertices", V::Integer(i32::try_from(mesh.vertices.len())?)),
                ]),
            ),
            field("type", V::Integer(44)),
            field("version", V::Integer(0)),
            field(
                "bounding_box",
                V::Named(vec![field("max", V::Float3(b)), field("min", V::Float3(a))]),
            ),
        ]);
        out.config(&format!("{root}/render/{name}/export_properties"), props)?;
        out.raw(
            &format!("{root}/render/{name}/converted_model"),
            &mesh.to_bytes()?,
        )?;
    }
    let props = V::Named(vec![
        field("src_filename", s(format!("Survarium 0.69d0: {donor_root}"))),
        field("type", V::Integer(40)),
        field("version", V::Integer(0)),
        field(
            "locators",
            V::Indexed(locs.iter().map(locator_value).collect()),
        ),
        field(
            "bounding_box",
            V::Named(vec![
                field("max", V::Float3(hi)),
                field("min", V::Float3(lo)),
            ]),
        ),
    ]);
    out.config(&format!("{root}/render/export_properties"), props)?;
    Ok(model)
}

/// A skinned model's `skeleton` file names its skeleton resource
/// (`resources/animations/skeletons/weapons/<x>`); point it at this port's
/// copy.
fn redirect_skeleton(v: &mut V) -> Result<()> {
    const PREFIX: &str = "resources/animations/skeletons/";
    let Some(V::String(text)) = v.named_mut("skeleton") else {
        bail!("the skeleton file names no skeleton");
    };
    let name = text
        .strip_prefix(PREFIX)
        .with_context(|| format!("skeleton {text} is not under {PREFIX}"))?;
    *text = format!("{PREFIX}{}", ns_name(name)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Materials and textures
// ---------------------------------------------------------------------------

/// The donor map roles and the 0.26g ones they feed.
const MAP_ROLES: [(&str, &str, Option<convert::ToneCurve>); 4] = [
    ("t_albedo", "texture_diffuse", Some(convert::ALBEDO)),
    ("t_normal", "texture_normal", None),
    (
        "t_smoothness",
        "texture_roughness",
        Some(convert::SMOOTHNESS_TO_GLOSS),
    ),
    (
        "t_metalness",
        "texture_fresnel",
        Some(convert::METALNESS_TO_FRESNEL),
    ),
];

pub(crate) fn build_material(
    name: &str,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<()> {
    let ours = format!("material_instances/{}.material", ns_name(name)?);
    if out.has(&ours) {
        return Ok(());
    }
    ensure!(!target.contains(&ours), "{ours} exists in the target");
    let path = format!("material_instances/{name}.material");
    let source = donor_config(donor, &path)?;
    let effect = at(&source.root, "material.g_stage.effect")
        .with_context(|| format!("{path} has no g_stage effect"))?;
    let mut material = parse_config(target, MATERIAL_TEMPLATE, "native rifle material")?;
    let mut names = BTreeMap::new();
    for (role, native, _) in MAP_ROLES {
        let texture = string_of(at(effect, &format!("{role}.value")))
            .with_context(|| format!("{path}: {role}"))?
            .to_owned();
        ensure!(!texture.is_empty(), "{path} binds no {role}");
        material.root.set_path(
            &format!("material.g_stage.effect.{native}.value"),
            s(ns_name(&texture)?),
        )?;
        names.insert(role, texture);
    }
    for (role, _, curve) in MAP_ROLES {
        let texture = &names[role];
        let dds = format!("textures/{}.dds", ns_name(texture)?);
        if out.has(&dds) {
            continue;
        }
        let bytes = donor.read(&format!("textures/{texture}.dds"))?;
        let bytes = if role == "t_albedo" {
            let metalness = donor.read(&format!("textures/{}.dds", names["t_metalness"]))?;
            diffuse_dds(&bytes, &metalness, &dds, out.work)?
        } else {
            match curve {
                Some(curve) => convert::remap_dds(&bytes, curve)?,
                None => bytes,
            }
        };
        out.raw(&dds, &bytes)?;
        let options = donor_config(donor, &format!("textures/{texture}.options"))?;
        out.config(
            &format!("textures/{}.options", ns_name(texture)?),
            options.root,
        )?;
    }
    // The detail normal stays the template's (`det/weap_nmap`, the one the
    // 0.69d weapons name too; the cartridges name another, not ported).
    out.config(&ours, material.root)
}

/// The 0.26g diffuse of a 0.69d albedo and metalness pair (see
/// [`convert::diffuse_from_albedo`]), DXT1 with mips like a native weapon's.
fn diffuse_dds(albedo: &[u8], metalness: &[u8], name: &str, work: &Path) -> Result<Vec<u8>> {
    let (w, h, a) = convert::decode_dxt1(albedo)?;
    let (mw, mh, m) = convert::decode_dxt1(metalness)?;
    let px: Vec<[u8; 3]> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let metal = m[(y * mh / h) * mw + x * mw / w][0];
            convert::diffuse_from_albedo(a[i], metal)
        })
        .collect();
    fs::create_dir_all(work)?;
    let stem = name.replace('/', "_");
    let tga = work.join(format!("{stem}.tga"));
    let dds = work.join(format!("{stem}.dds"));
    fs::write(&tga, convert::tga(w, h, &px)?)?;
    crate::armor::convert_texture(&tga, &dds, crate::manifest::TextureCompression::Dxt1)?;
    let out = crate::duty_026::texture_dds(&fs::read(&dds)?, name, true)?;
    fs::remove_file(&tga)?;
    fs::remove_file(&dds)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------

struct Rigs {
    /// The weapon's bones (`object_movement` first).
    item: Vec<String>,
    /// 0.26g's player rig.
    hands: Vec<String>,
}

/// Converts one donor clip (`resources/animations/single/...`) into this
/// port's folder, checking the bones it drives and, for a reload item clip,
/// that its sound cues index the state's list (`reload_sounds` long);
/// returns the new reference.
fn carry_clip(
    reference: &str,
    rigs: &Rigs,
    reload_sounds: Option<usize>,
    donor: &mut ResourceDb,
    out: &mut Out,
) -> Result<String> {
    let ours = ns_clip(reference)?;
    let src = reference.strip_prefix("resources/").unwrap();
    let dst = ours.strip_prefix("resources/").unwrap();
    if out.has(&format!("{dst}.b-spline")) {
        return Ok(ours);
    }
    let names = donor_config(donor, &format!("{src}.bones_names"))?;
    let bones = clip_bones(&names.to_bytes()?)?;
    let hands = src.contains("/player/");
    if hands {
        ensure!(
            bones == rigs.hands,
            "{src} drives other bones than 0.26g's player rig"
        );
    } else {
        let set: BTreeSet<&String> = bones.iter().collect();
        ensure!(
            set == rigs.item.iter().collect(),
            "{src} drives {bones:?}, not the weapon's {:?}",
            rigs.item
        );
    }
    let bytes = convert::bspline_069_to_026(&donor.read(&format!("{src}.b-spline"))?)?;
    ensure!(
        convert::clip_header(&bytes)?.bones == bones.len(),
        "{src}: curves and bones_names disagree"
    );
    if let (false, Some(count)) = (hands, reload_sounds) {
        for e in convert::events_026(&bytes)? {
            if e.name == "sound_events" {
                for d in e.domains {
                    ensure!(
                        d == 0xff || usize::from(d) < count,
                        "{src} cues sound {d} of {count}"
                    );
                }
            }
        }
    }
    out.raw(&format!("{dst}.b-spline"), &bytes)?;
    out.config(&format!("{dst}.bones_names"), names.root)?;
    Ok(ours)
}

/// Clips of 0.69d-only states, left out: the aimed shots (`_calim`) and the
/// pump's start of a reload into an empty chamber (`reload_finalround`; 0.26g's
/// pumps start every reload alike).
fn drops_clip(reference: &str) -> bool {
    reference.ends_with("_calim") || reference.ends_with("/reload_finalround")
}

fn clip_list(
    v: Option<&V>,
    rigs: &Rigs,
    reload_sounds: Option<usize>,
    donor: &mut ResourceDb,
    target: &ResourceDb,
    out: &mut Out,
) -> Result<V> {
    let mut items = vec![];
    for clip in list(v) {
        let reference = string_of(Some(clip))?;
        if drops_clip(reference) {
            continue;
        }
        let path = reference
            .strip_prefix("resources/")
            .context("clip outside resources/")?;
        if target.contains(&format!("{path}.b-spline")) {
            items.push(s(reference));
        } else {
            items.push(s(carry_clip(reference, rigs, reload_sounds, donor, out)?));
        }
    }
    Ok(V::Indexed(items))
}

// ---------------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------------

const ENVIRONMENTS: &str = "weapons/environments/";

fn sound_path(kind: i32, name: &str) -> Result<String> {
    Ok(match kind {
        256 => format!("sounds/single/{name}.high.ogg"),
        257 => format!("sounds/composite/{name}.composite_sound"),
        258 => format!("sounds/collections/{name}.sound_collection"),
        other => bail!("sound {name} of unknown kind {other}"),
    })
}

/// The name a donor sound has in the target: its own when 0.26g has it, else
/// this port's copy (carried with what it plays).
fn carry_sound(
    kind: i32,
    name: &str,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<String> {
    if target.contains(&sound_path(kind, name)?) {
        return Ok(name.into());
    }
    let ours = ns_name(name)?;
    let path = sound_path(kind, &ours)?;
    if out.has(&path) {
        return Ok(ours);
    }
    match kind {
        256 => {
            for ext in ["high.ogg", "high.rms"] {
                out.raw(
                    &format!("sounds/single/{ours}.{ext}"),
                    &donor.read(&format!("sounds/single/{name}.{ext}"))?,
                )?;
            }
            out.config(
                &format!("sounds/single/{ours}.raw_options"),
                donor_config(donor, &format!("sounds/single/{name}.raw_options"))?.root,
            )?;
        }
        258 => {
            let mut config = donor_config(donor, &sound_path(258, name)?)?.root;
            let Some(V::Indexed(items)) = config
                .named_mut("collection")
                .and_then(|c| c.named_mut("sound_items"))
            else {
                bail!("{name} lists no sounds");
            };
            let mut files = vec![];
            for item in items.iter() {
                ensure!(
                    matches!(item.named("resource_type"), None | Some(V::Integer(256))),
                    "{name}: an item is no single sound"
                );
                files.push(string_of(item.named("filename"))?.to_owned());
            }
            let mut carried = vec![];
            for file in &files {
                carried.push(carry_sound(256, file, donor, target, out)?);
            }
            let Some(V::Indexed(items)) = config
                .named_mut("collection")
                .and_then(|c| c.named_mut("sound_items"))
            else {
                unreachable!()
            };
            for (item, name) in items.iter_mut().zip(carried) {
                item.set_path("filename", s(name))?;
            }
            out.config(&path, config)?;
        }
        257 => {
            let mut config = donor_config(donor, &sound_path(257, name)?)?.root;
            let Some(V::Indexed(items)) = config
                .named_mut("composite_sound")
                .and_then(|c| c.named_mut("sound_items"))
            else {
                bail!("{name} lists no layers");
            };
            // 0.69d's per-location reverb layers: no 0.26g map defines them.
            items.retain(|item| {
                !string_of(item.named("filename")).is_ok_and(|f| f.starts_with(ENVIRONMENTS))
            });
            ensure!(!items.is_empty(), "{name} has only reverb layers");
            let layers: Vec<(i32, String)> = items
                .iter()
                .map(|item| {
                    Ok((
                        int_of(item.named("resource_type")).unwrap_or(258),
                        string_of(item.named("filename"))?.to_owned(),
                    ))
                })
                .collect::<Result<_>>()?;
            let count = layers.len();
            let mut carried = vec![];
            for (kind, file) in &layers {
                carried.push(carry_sound(*kind, file, donor, target, out)?);
            }
            let Some(V::Indexed(items)) = config
                .named_mut("composite_sound")
                .and_then(|c| c.named_mut("sound_items"))
            else {
                unreachable!()
            };
            for (item, name) in items.iter_mut().zip(carried) {
                item.set_path("filename", s(name))?;
            }
            if let Some(V::Integer(master)) = at(&config, "composite_sound.master_layer_index")
                && usize::try_from(*master).unwrap_or(usize::MAX) >= count
            {
                config.set_path("composite_sound.master_layer_index", V::Integer(0))?;
                config.set_path("composite_sound.use_master_layer_index", V::Boolean(false))?;
            }
            out.config(&path, config)?;
        }
        _ => unreachable!(),
    }
    Ok(ours)
}

/// A donor sound list (`[[kind, name], ..]`) for 0.26g: suppressed variants
/// left out, every other sound carried.
fn sound_list(
    v: Option<&V>,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<V> {
    let mut items = vec![];
    for pair in list(v) {
        let pair = list(Some(pair));
        ensure!(pair.len() == 2, "a sound entry is not [kind, name]");
        let kind = int_of(Some(&pair[0]))?;
        let name = string_of(Some(&pair[1]))?;
        if name.contains("silencer") {
            continue;
        }
        let carried = carry_sound(kind, name, donor, target, out)?;
        items.push(V::Indexed(vec![V::Integer(kind), s(carried)]));
    }
    Ok(V::Indexed(items))
}

// ---------------------------------------------------------------------------
// Item config
// ---------------------------------------------------------------------------

/// One state (or substate) of the donor laid over the template's.
#[allow(clippy::too_many_arguments)]
fn take_state(
    cfg: &mut V,
    path: &str,
    from: &V,
    plan: StatePlan,
    rigs: &Rigs,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<()> {
    ensure!(at(cfg, path).is_some(), "the template has no {path}");
    let reload_sounds = match at(from, "sounds.first_view") {
        Some(V::Indexed(l)) => Some(l.len()),
        _ => None,
    };
    if plan.clips == Src::Donor {
        for key in ["animations", "user_animations"] {
            if let Some(value) = from.named(key) {
                let cues = if plan.sounds == Src::Donor {
                    reload_sounds
                } else {
                    None
                };
                let clips = clip_list(Some(value), rigs, cues, donor, target, out)?;
                cfg.set_path(&format!("{path}.{key}"), clips)?;
            }
        }
    }
    if plan.sounds == Src::Donor {
        for view in ["first_view", "third_view"] {
            if let Some(value) = at(from, &format!("sounds.{view}")) {
                let sounds = sound_list(Some(value), donor, target, out)?;
                cfg.set_path(&format!("{path}.sounds.{view}"), sounds)?;
            }
        }
        if let Some(V::Integer(n)) = at(from, "sounds.simultaneous_sounds_queue_size") {
            cfg.set_path(
                &format!("{path}.sounds.simultaneous_sounds_queue_size"),
                V::Integer(*n),
            )?;
        }
    }
    if let V::Named(children) = from {
        for child in children {
            if child.name.ends_with("_substate") {
                let sub = format!("{path}.{}", child.name);
                take_state(cfg, &sub, &child.value, plan, rigs, donor, target, out)?;
            }
        }
    }
    Ok(())
}

/// What the live weapons' configs get from skhid-srp's edit lists
/// `weapon-camo-1` and `weapon-decals-1` (client patches 44 and 47): a
/// camouflage (`parameters.camo_available`, `data.camo_scale` 4.0, the stock
/// bakes' tiling) and a decal (`parameters.decal_available`, an empty
/// `data.decals`). The port's configs carry them themselves, so the shared
/// edit lists need no rows for them.
fn weapon_kit(cfg: &mut V) -> Result<()> {
    ensure!(
        cfg.named("data").is_none(),
        "the template already has a data table"
    );
    let V::Named(fields) = cfg else {
        bail!("the config is not a table");
    };
    fields.push(field(
        "data",
        V::Named(vec![
            field("camo_scale", V::Float(4.0)),
            field("decals", V::Named(vec![])),
        ]),
    ));
    let Some(V::Named(params)) = cfg.named_mut("parameters") else {
        bail!("the config has no parameters");
    };
    for key in ["camo_available", "decal_available"] {
        ensure!(
            !params.iter().any(|f| f.name == key),
            "the template already has parameters.{key}"
        );
        params.push(field(key, V::Boolean(true)));
    }
    Ok(())
}

fn trimmed(v: f32) -> String {
    let t = format!("{v:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[allow(clippy::too_many_arguments)]
fn item_config(
    gun: &Gun,
    game: &Game,
    rigs: &Rigs,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
    icons: bool,
) -> Result<V> {
    let mut cfg = parse_config(target, gun.template, "live weapon config")?.root;
    let w = game.weapon(gun.key)?.clone();
    let st = &gun.stats;
    let mut params = vec![
        ("faction", V::Integer(st.faction)),
        ("item_level", V::Integer(st.level)),
        ("match_making_level", V::Integer(st.level)),
        ("drop_weight", V::Integer(st.drop_weight)),
        ("rounds_per_minute", V::Integer(st.rounds_per_minute)),
        ("bullet_damage", V::Float(st.bullet_damage)),
        ("bullet_pierce", V::Float(st.bullet_pierce)),
        ("weight", V::Float(st.weight)),
        ("reload_time", V::Float(st.reload_time)),
        ("aim_time", V::Float(st.aim_time)),
        ("base_dispersion", V::Float(st.base_dispersion)),
        ("magazine_capacity", V::Integer(st.magazine)),
        ("min_damage_distance", V::Integer(st.min_damage_distance)),
        ("max_damage_distance", V::Integer(st.max_damage_distance)),
        (
            "fire_queue_types",
            V::Indexed(st.fire_modes.iter().map(|m| V::Integer(*m)).collect()),
        ),
    ];
    if let Some(t) = st.chamber_a_round_time {
        params.push(("chamber_a_round_time", V::Float(t)));
    }
    for (key, value) in params {
        ensure!(
            at(&cfg, &format!("parameters.{key}")).is_some(),
            "{} has no parameters.{key}",
            gun.template
        );
        cfg.set_path(&format!("parameters.{key}"), value)?;
    }
    if at(&cfg, "dispersion.base_dispersion").is_some() {
        cfg.set_path("dispersion.base_dispersion", V::Float(st.base_dispersion))?;
    }
    for plan in gun.states {
        let path = format!("states.{}", plan.state);
        let from = at(&w, &path)
            .with_context(|| format!("0.69d's {} has no {path}", gun.key))?
            .clone();
        take_state(&mut cfg, &path, &from, *plan, rigs, donor, target, out)?;
    }
    if matches!(gun.rig, Rig::Donor) {
        let preview = string_of(at(&w, "user_animations_in_place.preview_item"))?.to_owned();
        let clips = clip_list(
            Some(&V::Indexed(vec![s(preview)])),
            rigs,
            None,
            donor,
            target,
            out,
        )?;
        cfg.set_path(
            "user_animations_in_place.preview_item",
            list(Some(&clips))[0].clone(),
        )?;
        cfg.set_path(
            "object.skeleton",
            s(ns_name(string_of(at(&w, "object.skeleton"))?)?),
        )?;
    }
    weapon_kit(&mut cfg)?;
    cfg.set_path("object.model", s(gun.model_ref()))?;
    cfg.set_path("ui_desc.text_descriptions.name", s(gun.name_key()))?;
    cfg.set_path(
        "ui_desc.text_descriptions.description",
        s(gun.description_key()),
    )?;
    cfg.set_path("ui_desc.combat_log_icon", V::Integer(gun.combat_log_icon))?;
    if icons {
        cfg.set_path(
            "ui_desc.icon",
            V::Indexed(gun.icon.iter().map(|v| V::Integer(*v)).collect()),
        )?;
    }
    // The card: the template's properties, these values.
    let value_of = |id: i32| -> Option<String> {
        Some(match id {
            1 => format!("{}", (st.bullet_damage * 100.0).round() as i32),
            3 => format!("{}", (st.bullet_pierce * 100.0).round() as i32),
            133 => trimmed(st.aim_time),
            132 => format!("{}", st.max_damage_distance),
            2 => trimmed(st.base_dispersion),
            24 => format!("{}", st.min_damage_distance),
            4 => format!("{}", st.rounds_per_minute),
            15 => trimmed(st.weight),
            16 => format!("{}", st.magazine),
            _ => return None,
        })
    };
    let mut props = list(at(&cfg, "ui_desc.props_list")).to_vec();
    for p in &mut props {
        if let Some(value) = value_of(int_of(p.named("prop_id"))?) {
            p.set_path("prop_value", s(value))?;
        }
    }
    cfg.set_path("ui_desc.props_list", V::Indexed(props))?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Ammo
// ---------------------------------------------------------------------------

fn ammo_config(
    ammo: &NewAmmo,
    target: &mut ResourceDb,
    template_cfg: &str,
    icons: bool,
) -> Result<V> {
    let mut cfg = parse_config(target, template_cfg, "live ammo config")?.root;
    for (path, value) in [
        ("parameters.damage", V::Float(ammo.damage)),
        ("data.k_damage", V::Float(ammo.damage)),
        ("parameters.arp", V::Float(ammo.arp)),
        ("data.k_arp", V::Float(ammo.arp)),
        ("parameters.buckshot", V::Integer(ammo.buckshot)),
        ("data.buck_shot", V::Integer(ammo.buckshot)),
        ("parameters.clip_size", V::Integer(ammo.clip_size)),
        ("parameters.clip_weight", V::Float(ammo.clip_weight)),
        (
            "ui_desc.ammo_indicator_name",
            s(format!("{}_name", ammo.text)),
        ),
        (
            "ui_desc.text_descriptions.name",
            s(format!("{}_name", ammo.text)),
        ),
        (
            "ui_desc.text_descriptions.description",
            s(format!("{}_desc", ammo.text)),
        ),
    ] {
        ensure!(at(&cfg, path).is_some(), "{template_cfg} has no {path}");
        cfg.set_path(path, value)?;
    }
    if icons {
        cfg.set_path(
            "ui_desc.icon",
            V::Indexed(ammo.icon.iter().map(|v| V::Integer(*v)).collect()),
        )?;
    }
    let mut props = list(at(&cfg, "ui_desc.props_list")).to_vec();
    for p in &mut props {
        let value = match int_of(p.named("prop_id"))? {
            21 => format!(
                "{}",
                (ammo.damage * ammo.buckshot as f32 * 100.0).round() as i32
            ),
            22 => format!("{}", (ammo.arp * 100.0).round() as i32),
            16 => format!("{}", ammo.clip_size),
            15 => trimmed(ammo.clip_weight),
            _ => continue,
        };
        p.set_path("prop_value", s(value))?;
    }
    cfg.set_path("ui_desc.props_list", V::Indexed(props))?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Dictionary
// ---------------------------------------------------------------------------

fn row_of<'a>(items: &'a [V], key: &str, value: &V) -> Option<&'a V> {
    items.iter().find(|v| v.named(key) == Some(value))
}

/// Adds the guns' (and new ammo's) `items_dict` rows and their ammo's
/// `items_compatibility` rows. Returns the compatibility rows' names.
pub fn dictionary_rows(dictionary: &mut V, guns: &[&Gun]) -> Result<Vec<String>> {
    let items = list(dictionary.named("items_dict")).to_vec();
    ensure!(!items.is_empty(), "no items_dict");
    let id_of = |name: &str| -> Result<i32> {
        int_of(
            row_of(&items, "name", &s(name))
                .with_context(|| format!("no {name} row"))?
                .named("dict_id"),
        )
    };
    let mut new_rows = vec![];
    let mut seen_ammo = BTreeSet::new();
    for gun in guns {
        let mut row = row_of(&items, "cfg_name", &s(gun.template))
            .with_context(|| format!("no row for {}", gun.template))?
            .clone();
        row.set_path("cfg_name", s(gun.config))?;
        row.set_path("name", s(gun.name))?;
        row.set_path("dict_id", V::Integer(gun.dict_id))?;
        row.set_path("base_cost", V::Integer(gun.stats.base_cost))?;
        row.set_path("is_premium", V::Boolean(false))?;
        new_rows.push(row);
        if let Ammo::New(ammo) = gun.ammo
            && seen_ammo.insert(ammo.name)
        {
            let mut row = row_of(&items, "name", &s(ammo.template))
                .with_context(|| format!("no {} row", ammo.template))?
                .clone();
            row.set_path("cfg_name", s(ammo.config))?;
            row.set_path("name", s(ammo.name))?;
            row.set_path("dict_id", V::Integer(ammo.dict_id))?;
            row.set_path("base_cost", V::Integer(ammo.base_cost))?;
            new_rows.push(row);
        }
    }
    for row in &new_rows {
        for key in ["dict_id", "name", "cfg_name"] {
            let value = row.named(key).context("row field")?;
            ensure!(
                row_of(&items, key, value).is_none(),
                "{key} {value:?} is taken"
            );
        }
    }
    let V::Named(compat) = dictionary
        .named("items_compatibility")
        .context("items_compatibility")?
    else {
        bail!("items_compatibility is not a table");
    };
    let mut pairs = vec![];
    for gun in guns {
        match gun.ammo {
            Ammo::Like(peer) => {
                let peer_id = id_of(peer)?;
                let is_ammo = |id: i32| {
                    row_of(&items, "dict_id", &V::Integer(id))
                        .and_then(|r| r.named("name"))
                        .is_some_and(|n| matches!(n, V::String(n) if n.starts_with("ammo_")))
                };
                // Rows name a weapon and its ammo either way round (and
                // weapons' scopes, which are not the ammo's).
                let mut ammo = vec![];
                for c in compat {
                    let (Ok(a), Ok(b)) = (
                        int_of(c.value.named("first_item")),
                        int_of(c.value.named("second_item")),
                    ) else {
                        bail!("{} is no pair of items", c.name);
                    };
                    let other = if a == peer_id {
                        b
                    } else if b == peer_id {
                        a
                    } else {
                        continue;
                    };
                    if is_ammo(other) && !ammo.contains(&other) {
                        ammo.push(other);
                    }
                }
                ensure!(!ammo.is_empty(), "{peer} takes no ammo");
                pairs.extend(ammo.into_iter().map(|a| (gun.dict_id, a)));
            }
            Ammo::New(ammo) => pairs.push((gun.dict_id, ammo.dict_id)),
        }
    }
    let Some(V::Indexed(rows)) = dictionary.named_mut("items_dict") else {
        unreachable!()
    };
    rows.extend(new_rows);
    let Some(V::Named(compat)) = dictionary.named_mut("items_compatibility") else {
        unreachable!()
    };
    let mut added = vec![];
    for (first, second) in pairs {
        // A name of this port's own: other packs add rows too, and the
        // game's `compatibility_<n>` numbers would collide.
        let name = format!("compatibility_{NS}_{first}_{second}");
        ensure!(!compat.iter().any(|c| c.name == name), "{name} is taken");
        compat.push(field(
            &name,
            V::Named(vec![
                field("second_item", V::Integer(second)),
                field("first_item", V::Integer(first)),
            ]),
        ));
        added.push(name);
    }
    Ok(added)
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Builds the bundle (`overlay/`, `port-manifest.json`, `equipment.json`,
/// `attach-points.json`) of `guns` over `target` (the stock archive with the
/// patches players have) from `donor` (0.69d0's split volumes, a `.split`
/// list). With `icons`, the configs name their icon cells (the icons are
/// drawn by `scripts/render-weapons-069d-icons.py` into `<bundle>/icons`).
pub fn build(
    guns: &[&Gun],
    donor_db: &Path,
    target_layers: &[PathBuf],
    icons: bool,
    output: &Path,
) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut donor = ResourceDb::open(donor_db)?;
    let game = Game::load(&mut donor)?;
    let work = output.join("work");
    let mut out = Out {
        overlay: &overlay,
        work: &work,
        files: vec![],
        written: BTreeSet::new(),
    };
    let hands_rig = clip_bones(&target.read(NATIVE_HANDS_CLIP)?)?;
    let mut registration = vec![];
    let mut attach = serde_json::Map::new();
    let mut seen_ammo = BTreeSet::new();
    let mut ammo_items = vec![];
    for gun in guns {
        ensure!(!target.contains(gun.config), "{} exists", gun.config);
        let model = build_model(gun, &game, &mut donor, &mut target, &mut out)
            .with_context(|| format!("{}: model", gun.name))?;
        let materials: BTreeSet<String> =
            model.surfaces.iter().map(|(_, _, m)| m.clone()).collect();
        for material in &materials {
            build_material(material, &mut donor, &mut target, &mut out)
                .with_context(|| format!("{}: material {material}", gun.name))?;
        }
        let bind = bind_pose(&out.read(&format!("{}/render/bind_pose", gun.model_root()))?)?;
        let mut item: Vec<String> = vec!["object_movement".into()];
        item.extend(bind.into_iter().map(|(n, _)| n));
        let rigs = Rigs {
            item,
            hands: hands_rig.clone(),
        };
        let cfg = item_config(gun, &game, &rigs, &mut donor, &mut target, &mut out, icons)
            .with_context(|| format!("{}: item config", gun.name))?;
        out.config(gun.config, cfg)?;
        if let Ammo::New(ammo) = gun.ammo
            && seen_ammo.insert(ammo.name)
        {
            let template_cfg = format!("gameplay/items/weapons/ammo/{}.options", ammo.template);
            // 0.69d draws the 23 mm round with its 12/70 buckshot's icon
            // (both `ui_desc.icon` [1, 1]); so does the port, in a cell of its own.
            let template = parse_config(&mut target, &template_cfg, "live ammo config")?;
            let icon_like: Vec<i32> = list(at(&template.root, "ui_desc.icon"))
                .iter()
                .map(|v| int_of(Some(v)))
                .collect::<Result<_>>()?;
            ammo_items.push(json!({"name": ammo.name, "dict_id": ammo.dict_id, "icon": ammo.icon, "icon_like": icon_like}));
            let cfg = ammo_config(ammo, &mut target, &template_cfg, icons)?;
            out.config(ammo.config, cfg)?;
        }
        attach.insert(gun.name.into(), serde_json::to_value(&model.attach_points)?);
        registration.push(json!({
            "dict_id": gun.dict_id,
            "name": gun.name,
            "donor": gun.key,
            "config": gun.config,
            "template": gun.template,
            "faction": gun.stats.faction,
            "item_level": gun.stats.level,
            "model_root": gun.model_root(),
            "icon": gun.icon,
            "modules": model.modules.iter().map(|m| json!({"id": m.id, "model": m.model, "locator": m.locator, "baked": m.baked})).collect::<Vec<_>>(),
            "surfaces": model.surfaces.iter().map(|(n, m, _)| json!({"name": n, "vertices": m.vertices.len()})).collect::<Vec<_>>(),
        }));
    }
    let mut dictionary = parse_config(&mut target, DICT, "0.26g dictionary")?;
    let compat = dictionary_rows(&mut dictionary.root, guns)?;
    out.config(DICT, dictionary.root)?;
    if work.exists() {
        fs::remove_dir(&work).context("the encoder left files behind")?;
    }
    check_references(&mut target, &out, guns)?;
    let mut files = out.files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": "weapons-069",
            "files": files,
            "donor_db": donor_db,
            "target": target_layers,
            "items": registration,
            "ammo": ammo_items,
            "compatibility": compat,
        }))?,
    )?;
    fs::write(
        output.join("attach-points.json"),
        serde_json::to_vec_pretty(&attach)?,
    )?;
    let mut ids: Vec<i32> = guns.iter().map(|g| g.dict_id).collect();
    for gun in guns {
        if let Ammo::New(a) = gun.ammo
            && !ids.contains(&a.dict_id)
        {
            ids.push(a.dict_id);
        }
    }
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(
            &ids.iter()
                .map(|id| json!({"dict_id": id}))
                .collect::<Vec<_>>(),
        )?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {}: {} files",
        guns.iter().map(|g| g.name).collect::<Vec<_>>().join(", "),
        files.len()
    );
    Ok(())
}

/// Every resource path the new configs name exists in the target or the
/// overlay: models, skeletons, clips, materials, textures, sounds.
fn check_references(target: &mut ResourceDb, out: &Out, guns: &[&Gun]) -> Result<()> {
    // What the native templates name is native.
    let mut native = BTreeSet::new();
    let mut templates: Vec<&str> = vec![SETTINGS_TEMPLATE, MATERIAL_TEMPLATE];
    templates.extend(guns.iter().map(|g| g.template));
    for template in templates {
        let config = parse_config(target, template, "native template")?;
        let mut strings = vec![];
        config.root.collect_strings(&mut strings);
        native.extend(strings.into_iter().map(str::to_owned));
    }
    let exists = |p: &str, target: &ResourceDb| target.contains(p) || out.has(p);
    for path in out.written.clone() {
        if path.starts_with("localization/") || path == DICT {
            continue;
        }
        let bytes = out.read(&path)?;
        let Ok(config) = BinaryConfig::parse(&bytes) else {
            continue;
        };
        let mut strings = vec![];
        config.root.collect_strings(&mut strings);
        for s in strings {
            if native.contains(s) {
                continue;
            }
            let candidates: Vec<String> =
                if let Some(r) = s.strip_prefix("resources/animations/single/") {
                    vec![format!("animations/single/{r}.b-spline")]
                } else if path.ends_with(".material") && s.contains('/') && !s.contains(':') {
                    vec![format!("textures/{s}.dds")]
                } else if path.ends_with("/settings") && s.contains('/') {
                    vec![format!("material_instances/{s}.material")]
                } else if let Some(r) = s.strip_prefix("resources/animations/skeletons/") {
                    vec![format!("animations/skeletons/{r}.skeleton")]
                } else if path.ends_with(".composite_sound") && s.contains('/') {
                    vec![
                        format!("sounds/collections/{s}.sound_collection"),
                        format!("sounds/single/{s}.high.ogg"),
                    ]
                } else if path.ends_with(".sound_collection") && s.contains('/') {
                    vec![format!("sounds/single/{s}.high.ogg")]
                } else if path.starts_with("gameplay/items/weapons/") && s.starts_with("weapons/") {
                    vec![
                        format!("sounds/collections/{s}.sound_collection"),
                        format!("sounds/composite/{s}.composite_sound"),
                        format!("sounds/single/{s}.high.ogg"),
                        format!("animations/skeletons/{s}.skeleton"),
                        format!("models/{s}.skinned_model/settings"),
                    ]
                } else {
                    continue;
                };
            ensure!(
                candidates.iter().any(|c| exists(c, target)),
                "{path} names {s}, which resolves to none of {candidates:?}"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
