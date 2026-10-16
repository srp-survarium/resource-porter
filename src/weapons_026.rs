// SPDX-License-Identifier: GPL-3.0-or-later
//! Two assault rifles of Survarium 0.69d0 (2022) for v0.26g: the H&K 416A5
//! (1820) and the MSBS Radon (1821). See `docs/weapons-069d-026.md`.
//!
//! The 0.69d client carries the weapons' art (models, skeletons, bind poses,
//! animations, attachment modules, materials, textures, sounds, names) but not
//! their item configs, which its server sent. Each item config is therefore a
//! native 0.26g rifle's (the AK-12's: the same states, sounds and shared
//! user animations) with the ported clips, sounds and model in it, and
//! provisional statistics.
//!
//! Everything the 0.69d art needs is converted, never substituted, except
//! where 0.26g has no counterpart (listed in the doc): the 0.69d attachment
//! system (modules placed at the weapon's locators by the server's config)
//! becomes surfaces baked into the weapon at those locators, and the 0.69d
//! per-location weapon reverbs (`sound_environment` layers, which no 0.26g map
//! defines) become the native rifles' tails.
pub mod convert;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde_json::json;

use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::manifest::BuiltFile;
use crate::weapon::{parse_config, prepare_empty_output, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use convert::Skinned;

const DICT: &str = "gameplay/db_static_dictionaries";
/// The native rifle every item config starts from: its states are the ones
/// the ported clips fill (show, hide, idle, fire, reload; melee and grenade
/// stay the AK family's), and it shares `rifles.user_animations` (aiming,
/// moving, sprinting, crouching) with every 0.26g rifle.
const CONFIG_TEMPLATE: &str = "gameplay/items/weapons/wpn_ak_12.options";
const SETTINGS_TEMPLATE: &str = "models/weapons/ak_12/ak_12.skinned_model/settings";
const MATERIAL_TEMPLATE: &str = "material_instances/weapons/ak_12.material";
/// A native rifle's clip: the bones every ported hands clip must drive.
const NATIVE_HANDS_CLIP: &str =
    "animations/single/weapons/ak_12/player/stand/3rd_view/on_site/hands_only/reload.bones_names";
/// 5.45x39 7N10 and 7N22, the AK-74M's: 0.26g has no 5.56x45.
pub const AMMO: [i32; 2] = [217, 331];
const SOURCE: &str = "converted:0.69d0";

/// 0.69d weapon rigs keep their meshes rotated half a turn about Y from the
/// frame their bind pose and locators are written in (x and z negated).
fn mesh_from_bind(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], -p[2]]
}

/// An attachment module baked into the weapon.
#[derive(Clone, Copy, Debug)]
pub struct Module {
    /// `models/weapons/modules/<model>.model`.
    pub model: &'static str,
    /// The weapon locator the module sits at.
    pub locator: &'static str,
    /// The surface it becomes in the weapon.
    pub surface: &'static str,
    /// For a sight: the height of its aperture centre or post tip above
    /// the socket, measured on the module's mesh from behind (the rear view
    /// in the gallery). The aim locator goes on that line.
    pub sight_height: Option<f32>,
}

/// One layer of a weapon's firing sound.
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    /// Sound name under `sounds/collections/` (resource type 258).
    pub collection: &'static str,
    pub start_offset: i32,
}

/// Provisional statistics (owner to confirm).
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub faction: i32,
    pub level: i32,
    pub drop_weight: i32,
    pub base_cost: i32,
    pub rounds_per_minute: i32,
    pub bullet_damage: f32,
    pub bullet_pierce: f32,
    pub stopping_power: f32,
    pub weight: f32,
    pub reload_time: f32,
    pub aim_time: f32,
    pub base_dispersion: f32,
    pub magazine: i32,
    pub min_damage_distance: i32,
    pub max_damage_distance: i32,
    /// -1 automatic, 1 single, 3 burst.
    pub fire_modes: &'static [i32],
}

#[derive(Clone, Copy, Debug)]
pub struct Rifle {
    pub key: &'static str,
    pub dict_id: i32,
    /// `items_dict` name.
    pub name: &'static str,
    /// The donor's folder name (models, animations, skeleton, textures).
    pub donor: &'static str,
    pub config: &'static str,
    /// The donor's skinned surfaces, drawn in this order; the magazine is
    /// one (the donor's default magazine module is the same mesh, placed).
    pub surfaces: &'static [&'static str],
    pub modules: &'static [Module],
    /// The module the muzzle locator moves to the end of.
    pub muzzle_module: &'static str,
    /// The locator `handguard_point` is copied from.
    pub handguard: &'static str,
    /// Sound collections of the reload state, by index.
    pub reload_sounds: &'static [&'static str],
    /// `sound_events` indices of the empty (`reload`) and tactical
    /// (`reload_tactic`) clips; `None` keeps the donor's.
    pub empty_events: Option<&'static [u8]>,
    pub tactical_events: Option<&'static [u8]>,
    pub fire_first: &'static [Layer],
    pub fire_first_master: Option<i32>,
    pub fire_third: &'static [Layer],
    pub fire_third_master: Option<i32>,
    /// Composite sound names (`sounds/composite/<name>`), first and third view.
    pub composites: [&'static str; 2],
    pub stats: Stats,
    pub texts: [(&'static str, Text); 2],
    /// Cell [row, col] of the lobby's weapon atlas (`LobbyMenu_WeaponIcons`).
    pub icon: [i32; 2],
    /// Kill-feed icon: a native rifle's (the HUD atlas is not extended).
    pub combat_log_icon: i32,
}

/// The six clips 0.69d gives each rifle, each for the item and the hands.
pub const CLIPS: [&str; 6] = ["show", "hide", "idle", "reload", "reload_tactic", "shoot"];

pub const HK416: Rifle = Rifle {
    key: "hk416",
    dict_id: 1820,
    name: "hk416_a5",
    donor: "hk416_a5",
    config: "gameplay/items/weapons/wpn_hk416_a5.options",
    surfaces: &[
        "hk416_receiver_s",
        "hk416_handguard_s",
        "hk416_grip_s",
        "hk416_buttstock_s",
        "hk416_barrel_s",
        "hk416_magazine_s",
    ],
    modules: &[
        Module {
            model: "muzzlebrakes/hk416a5_i05x28_muzzlebrake",
            locator: "muzzle_point",
            surface: "hk416_muzzlebrake_s",
            sight_height: None,
        },
        Module {
            model: "ironsights/hk416_a5_rear_ironsight_on",
            locator: "picatinny_ironsight_rear_point",
            surface: "hk416_rear_sight_s",
            sight_height: Some(0.0330),
        },
        Module {
            model: "ironsights/hk416_a5_front_ironsight_on",
            locator: "picatinny_ironsight_front_point",
            surface: "hk416_front_sight_s",
            sight_height: Some(0.0350),
        },
    ],
    muzzle_module: "muzzlebrakes/hk416a5_i05x28_muzzlebrake",
    handguard: "handguard_point",
    // The empty reload's seven cues in clip order (frames 20, 21, 47, 90,
    // 110, 137, 140); the tactical clip cues 1..4 of them (frames 17, 43, 90,
    // 110), which is what leaves the falling magazine and the bolt out.
    reload_sounds: &[
        "weapons/hk416/mag_fall",
        "weapons/hk416/mag_eject",
        "weapons/hk416/wear_line",
        "weapons/hk416/mag_set",
        "weapons/hk416/mag_set_ready",
        "weapons/hk416/bolt_button",
        "weapons/hk416/bolt_close",
    ],
    empty_events: None,
    tactical_events: None,
    // The donor's own layers, then the AK-12's first-person tails in place
    // of the 0.69d reverb layer.
    fire_first: &[
        Layer {
            collection: "weapons/hk416/hk416_shutter",
            start_offset: 60,
        },
        Layer {
            collection: "weapons/hk416/hk416_loud",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_close_forest",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_main_layer",
            start_offset: 0,
        },
    ],
    fire_first_master: Some(1),
    // The donor's (both native Vityaz collections), then the Vityaz's
    // distance and forest tails in place of the 0.69d reverb layer.
    fire_third: &[
        Layer {
            collection: "weapons/vityaz/vityaz_m_close",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/vityaz/vityaz_m_shutter",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/vityaz/vityaz_m_distant",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/vityaz/vityaz_m_close_forest",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/vityaz/vityaz_m_distant_forest",
            start_offset: 0,
        },
    ],
    fire_third_master: None,
    composites: ["weapons/hk416", "weapons/hk416_m"],
    stats: Stats {
        faction: 4,
        level: 13,
        drop_weight: 30,
        base_cost: 82_000,
        rounds_per_minute: 850,
        bullet_damage: 0.5,
        bullet_pierce: 0.35,
        stopping_power: 0.15,
        weight: 3.5,
        reload_time: 3.0,
        aim_time: 0.3,
        base_dispersion: 0.18,
        magazine: 30,
        min_damage_distance: 50,
        max_damage_distance: 20,
        fire_modes: &[-1, 1],
    },
    texts: [
        (
            "st_hk416_026_name",
            Text {
                english: "H&K 416A5",
                russian: "H&K 416A5",
                ukrainian: "H&K 416A5",
                german: "H&K 416A5",
                polish: "H&K 416A5",
            },
        ),
        (
            "st_hk416_026_description",
            Text {
                english: "German assault rifle on the AR-15 pattern with a short-stroke gas piston, rebarrelled here for the 5.45x39 ammunition of the Zone. Supports single shot and automatic fire modes.",
                russian: "Немецкая штурмовая винтовка по схеме AR-15 с газовым поршнем с коротким ходом, здесь переделанная под патрон 5.45×39. Поддерживает одиночный и автоматический режим стрельбы.",
                ukrainian: "Німецька штурмова гвинтівка за схемою AR-15 з газовим поршнем із коротким ходом, тут перероблена під набій 5.45×39. Підтримує одиночний і автоматичний режим стрільби.",
                german: "Deutsches Sturmgewehr nach AR-15-Muster mit Kurzhub-Gaskolben, hier auf die 5,45x39-Munition der Zone umgebaut. Einzel- und Dauerfeuer.",
                polish: "Niemiecki karabin szturmowy w układzie AR-15 z tłokiem gazowym o krótkim skoku, tu przerobiony na amunicję 5,45x39. Ogień pojedynczy i ciągły.",
            },
        ),
    ],
    icon: [7, 3],
    combat_log_icon: 49,
};

pub const RADON: Rifle = Rifle {
    key: "radon",
    dict_id: 1821,
    name: "msbs_radon",
    donor: "msbs_carbine",
    config: "gameplay/items/weapons/wpn_msbs_radon.options",
    surfaces: &[
        "msbs_carbine_receiver_s",
        "msbs_carbine_grip_s",
        "msbs_carbine_body_s",
        "msbs_carbine_mag_s",
    ],
    modules: &[
        Module {
            model: "muzzlebrakes/msbsradon_i05x28_muzzlebrake",
            locator: "muzzle_point",
            surface: "msbs_carbine_muzzlebrake_s",
            sight_height: None,
        },
        // 0.69d's Radon came with no sights of its own; the Troy micro
        // battle sights are the set its Summer 2021 skin has a variant of.
        Module {
            model: "ironsights/troy_micronattlesight_rear_ironsight_on",
            locator: "picatinny_ironsight_rear_point",
            surface: "msbs_carbine_rear_sight_s",
            sight_height: Some(0.0340),
        },
        Module {
            model: "ironsights/troy_micronattlesight_front_ironsight_on",
            locator: "picatinny_ironsight_front_point",
            surface: "msbs_carbine_front_sight_s",
            sight_height: Some(0.0339),
        },
    ],
    muzzle_module: "muzzlebrakes/msbsradon_i05x28_muzzlebrake",
    handguard: "LeftHand_idle",
    // 0.69d gave the Radon no reload sounds of its own (its server named
    // other weapons'); these are the H&K's, cued by the clips' own frames.
    // Empty: release (1), out (26), fall (39), in (98), seat (118), bolt
    // release (154), bolt (156). Tactical: release (20), out (26), stow (38),
    // in (100), seat (116).
    reload_sounds: &[
        "weapons/hk416/bolt_button",
        "weapons/hk416/mag_eject",
        "weapons/hk416/mag_fall",
        "weapons/hk416/mag_set",
        "weapons/hk416/mag_set_ready",
        "weapons/hk416/bolt_button",
        "weapons/hk416/bolt_close",
        "weapons/hk416/wear_line",
    ],
    empty_events: Some(&[0, 1, 2, 3, 4, 5, 6]),
    tactical_events: Some(&[0, 1, 7, 3, 4]),
    fire_first: &[
        Layer {
            collection: "weapons/akm_n/akmn_base",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akmn_layer",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akmn_loud",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akmn_shutter",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/ak_12/ak_12_shutter",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_close_forest",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_main_layer",
            start_offset: 0,
        },
    ],
    fire_first_master: Some(2),
    fire_third: &[
        Layer {
            collection: "weapons/akm_n/akmn_loud_m",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akmn_base_m",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akmn_layer_m",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/ak_12/ak_12_shutter_m",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_m_distant",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_m_close_forest",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_m_distant_forest",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_most_distant",
            start_offset: 0,
        },
        Layer {
            collection: "weapons/akm_n/akm_n_most_distant_echo",
            start_offset: 0,
        },
    ],
    fire_third_master: None,
    composites: ["weapons/msbs", "weapons/msbs_m"],
    stats: Stats {
        faction: 4,
        level: 12,
        drop_weight: 40,
        base_cost: 72_000,
        rounds_per_minute: 700,
        bullet_damage: 0.5,
        bullet_pierce: 0.3,
        stopping_power: 0.15,
        weight: 3.4,
        reload_time: 3.0,
        aim_time: 0.3,
        base_dispersion: 0.2,
        magazine: 30,
        min_damage_distance: 50,
        max_damage_distance: 20,
        fire_modes: &[-1, 1],
    },
    texts: [
        (
            "st_msbs_radon_026_name",
            Text {
                english: "MSBS Radon",
                russian: "MSBS Radon",
                ukrainian: "MSBS Radon",
                german: "MSBS Radon",
                polish: "MSBS Radon",
            },
        ),
        (
            "st_msbs_radon_026_description",
            Text {
                english: "Polish modular assault rifle, adopted by the Polish army in 2017 as the Grot. Converted to fire 5.45x39 ammunition. Supports single shot and automatic fire modes.",
                russian: "Польская модульная штурмовая винтовка, принятая на вооружение польской армии в 2017 году под названием Grot. Переделана под патрон 5.45×39. Поддерживает одиночный и автоматический режим стрельбы.",
                ukrainian: "Польська модульна штурмова гвинтівка, прийнята на озброєння польської армії 2017 року під назвою Grot. Перероблена під набій 5.45×39. Підтримує одиночний і автоматичний режим стрільби.",
                german: "Polnisches modulares Sturmgewehr, 2017 als Grot bei der polnischen Armee eingeführt. Auf 5,45x39-Munition umgebaut. Einzel- und Dauerfeuer.",
                polish: "Polski modułowy karabin szturmowy, przyjęty na uzbrojenie Wojska Polskiego w 2017 roku jako Grot. Przerobiony na amunicję 5,45x39. Ogień pojedynczy i ciągły.",
            },
        ),
    ],
    icon: [8, 3],
    combat_log_icon: 31,
};

pub const RIFLES: [&Rifle; 2] = [&HK416, &RADON];

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}
fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}

struct Out<'a> {
    overlay: &'a Path,
    /// Scratch files of the texture encoder (outside the overlay).
    work: &'a Path,
    files: Vec<BuiltFile>,
    written: BTreeSet<String>,
}

impl Out<'_> {
    fn raw(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        let dest = crate::archive::safe_join(self.overlay, path)?;
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
        self.files.push(record_file(path, SOURCE, bytes));
        Ok(())
    }
    fn config(&mut self, path: &str, root: V) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        write_generated_config(
            self.overlay,
            path,
            &BinaryConfig { root },
            SOURCE,
            &mut self.files,
        )
    }
    fn has(&self, path: &str) -> bool {
        self.written.contains(path)
    }
}

fn donor_config(donor: &mut ResourceDb, path: &str) -> Result<BinaryConfig> {
    let bytes = donor
        .read(path)
        .with_context(|| format!("reading donor {path}"))?;
    ensure!(
        BinaryConfig::is_compact(&bytes),
        "{path} is not a 0.69d (16-byte) config"
    );
    BinaryConfig::parse(&bytes).with_context(|| format!("parsing donor {path}"))
}

fn float3(v: Option<&V>) -> Result<[f32; 3]> {
    match v {
        Some(V::Float3(f)) => Ok(*f),
        other => bail!("expected a float3, found {other:?}"),
    }
}
fn string_of<'v>(v: Option<&'v V>) -> Result<&'v str> {
    match v {
        Some(V::String(s)) => Ok(s),
        other => bail!("expected a string, found {other:?}"),
    }
}
fn int_of(v: Option<&V>) -> Result<i32> {
    match v {
        Some(V::Integer(i)) => Ok(*i),
        other => bail!("expected an integer, found {other:?}"),
    }
}

/// A bind pose: per bone its name and translation (the rotation of every
/// 0.69d weapon bone is the root's half turn, checked).
fn bind_pose(b: &[u8]) -> Result<Vec<(String, [f32; 3])>> {
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
        let half_turn = (f[3].abs() - std::f32::consts::PI).abs() < 1e-3
            && f[4].abs() < 1e-3
            && (f[5].abs() - std::f32::consts::PI).abs() < 1e-3;
        ensure!(half_turn, "bone {name} is not in the rig's half-turn frame");
        ensure!(f[6..9].iter().all(|s| (s - 1.0).abs() < 1e-4), "bone {name} is scaled");
        out.push((name, [f[0], f[1], f[2]]));
    }
    ensure!(at == b.len(), "bind pose has trailing bytes");
    Ok(out)
}

/// `bones_names` of a clip.
fn clip_bones(bytes: &[u8]) -> Result<Vec<String>> {
    let config = BinaryConfig::parse(bytes)?;
    match config.root.named("bones_names") {
        Some(V::Indexed(names)) => names
            .iter()
            .map(|n| match n {
                V::String(s) => Ok(s.clone()),
                _ => bail!("bones_names holds a non-string"),
            })
            .collect(),
        _ => bail!("no bones_names"),
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

struct Locator {
    name: String,
    bone: String,
    bone_idx: i32,
    position: [f32; 3],
    rotation: [f32; 3],
}

fn locators(properties: &V) -> Result<Vec<Locator>> {
    let Some(V::Indexed(list)) = properties.named("locators") else {
        bail!("the rig has no locators");
    };
    list.iter()
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

fn locator_value(l: &Locator) -> V {
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
    ensure!(
        loc.rotation.iter().all(|r| r.abs() < 1e-6),
        "{} is rotated; only unrotated sockets are baked",
        loc.name
    );
    let (_, t) = bind
        .iter()
        .find(|(n, _)| *n == loc.bone)
        .with_context(|| format!("{} names an unknown bone {}", loc.name, loc.bone))?;
    let p = mesh_from_bind(loc.position);
    Ok([t[0] + p[0], t[1] + p[1], t[2] + p[2]])
}

/// The weapon's surfaces: the donor's skinned ones and the baked modules.
struct Model {
    surfaces: Vec<(String, Skinned, String)>,
    locators: Vec<Locator>,
}

fn model_root(rifle: &Rifle) -> String {
    format!(
        "models/weapons/assault/{0}/{0}.skinned_model",
        rifle.donor
    )
}

fn build_model(rifle: &Rifle, donor: &mut ResourceDb, target: &mut ResourceDb, out: &mut Out) -> Result<Model> {
    let root = model_root(rifle);
    let bind_bytes = donor.read(&format!("{root}/render/bind_pose"))?;
    let bind = bind_pose(&bind_bytes)?;
    let skeleton_path = format!(
        "animations/skeletons/weapons/{0}/{0}.skeleton",
        rifle.donor
    );
    let skeleton = donor_config(donor, &skeleton_path)?;
    // The bind pose names every skeleton bone, in a depth-first order.
    let mut skeleton_bones = vec![];
    collect_bones(&skeleton.root, &mut skeleton_bones);
    ensure!(
        skeleton_bones.first().map(String::as_str) == Some("object_movement"),
        "the skeleton does not start at object_movement"
    );
    let bind_names: BTreeSet<&str> = bind.iter().map(|(n, _)| n.as_str()).collect();
    let skel_names: BTreeSet<&str> = skeleton_bones[1..].iter().map(String::as_str).collect();
    ensure!(bind_names == skel_names, "bind pose and skeleton disagree");

    let settings = donor_config(donor, &format!("{root}/settings"))?;
    let properties = donor_config(donor, &format!("{root}/render/export_properties"))?;
    ensure!(
        int_of(properties.root.named("type"))? == 40,
        "the rig is not a type-40 skinned model"
    );
    let all = locators(&properties.root)?;
    let find = |name: &str| {
        all.iter()
            .find(|l| l.name == name)
            .with_context(|| format!("{} has no {name} locator", rifle.donor))
    };

    let mut model = Model {
        surfaces: vec![],
        locators: vec![],
    };
    for surface in rifle.surfaces {
        // The magazine is no LOD0 surface of the rig (0.69d drew the magazine
        // module in its place), so it is not in the settings; its material is
        // the one of the same name.
        let listed = settings
            .root
            .named("material_settings")
            .and_then(|m| m.named(surface))
            .and_then(|m| m.named("material_name"));
        let material = match listed {
            Some(_) => string_of(listed)?.to_owned(),
            None => {
                let name = format!(
                    "weapons/assault/{}/{}",
                    rifle.donor,
                    surface.trim_end_matches("_s")
                );
                ensure!(
                    donor.contains(&format!("material_instances/{name}.material")),
                    "{surface} has no material"
                );
                name
            }
        };
        let bytes = donor.read(&format!("{root}/render/{surface}/converted_model"))?;
        let mesh = convert::skinned_069_to_026(&bytes, bind.len())?;
        model.surfaces.push(((*surface).into(), mesh, material));
    }
    let mut sights = vec![];
    for module in rifle.modules {
        let path = format!("models/weapons/modules/{}.model", module.model);
        let msettings = donor_config(donor, &format!("{path}/settings"))?;
        let Some(V::Named(lod)) = msettings
            .root
            .named("lod_hierrarchy")
            .and_then(|l| l.named("LOD0"))
        else {
            bail!("{path} has no LOD0");
        };
        let Some(V::Indexed(surfaces)) = lod.iter().find(|v| v.name == "surfaces").map(|v| &v.value)
        else {
            bail!("{path} LOD0 lists no surfaces");
        };
        ensure!(surfaces.len() == 1, "{path} has {} surfaces, not one", surfaces.len());
        let V::String(surface) = &surfaces[0] else {
            bail!("{path} surface is not a name");
        };
        let material = string_of(
            msettings
                .root
                .named("material_settings")
                .and_then(|m| m.named(surface))
                .and_then(|m| m.named("material_name")),
        )?
        .to_owned();
        let loc = find(module.locator)?;
        let at = socket(loc, &bind)?;
        let bone = bind
            .iter()
            .position(|(n, _)| *n == loc.bone)
            .context("socket bone")? as u32;
        let bytes = donor.read(&format!("{path}/render/{surface}/converted_model"))?;
        let mesh = convert::static_069_to_skinned(&bytes, at, bone)?;
        if let Some(h) = module.sight_height {
            sights.push(at[1] + h);
        }
        if module.model == rifle.muzzle_module {
            // The muzzle moves to the brake's far end, on the bore.
            let (_, hi) = mesh.bounds();
            let muzzle = find("muzzle_point")?;
            let (_, t) = bind
                .iter()
                .find(|(n, _)| *n == muzzle.bone)
                .context("muzzle bone")?;
            let end = [at[0] - t[0], at[1] - t[1], hi[2] - t[2]];
            model.locators.push(Locator {
                name: "barrel_point".into(),
                bone: muzzle.bone.clone(),
                bone_idx: muzzle.bone_idx,
                position: mesh_from_bind(end),
                rotation: [0.0; 3],
            });
        }
        model
            .surfaces
            .push((module.surface.into(), mesh, material));
    }
    ensure!(
        model.locators.iter().any(|l| l.name == "barrel_point"),
        "no muzzle module"
    );
    // The aim point stays where the rig puts it along the bore; it rises
    // to the iron sights' line (aperture centre and post tip, which
    // co-witness within 0.2 mm on both rifles).
    let mut aim = find("aim")?.clone_locator();
    ensure!(sights.len() == 2, "a rifle needs a rear and a front sight");
    ensure!(
        (sights[0] - sights[1]).abs() < 0.002,
        "the sights do not co-witness: {sights:?}"
    );
    let line = (sights[0] + sights[1]) / 2.0;
    let aim_bone = bind.iter().find(|(n, _)| *n == aim.bone).context("aim bone")?.1;
    aim.position[1] = line - aim_bone[1];
    model.locators.push(aim);
    let scope = find("picatinny_scope_point")?;
    model.locators.push(Locator {
        name: "scope_point".into(),
        ..scope.clone_locator()
    });
    let handguard = find(rifle.handguard)?;
    model.locators.push(Locator {
        name: "handguard_point".into(),
        rotation: [0.0; 3],
        ..handguard.clone_locator()
    });
    let hands: Vec<&Locator> = all.iter().filter(|l| l.name.ends_with("_idle")).collect();
    ensure!(hands.len() == 32, "{} hand locators, not 32", hands.len());
    model.locators.extend(hands.into_iter().map(Locator::clone_locator));

    // Files.
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
    native_settings.root.set_path(
        "material_settings",
        V::Named(
            model
                .surfaces
                .iter()
                .map(|(n, _, m)| {
                    field(
                        n,
                        V::Named(vec![
                            field("material_name", s(m)),
                            field("used_in_portals", V::Boolean(false)),
                        ]),
                    )
                })
                .collect(),
        ),
    )?;
    out.config(&format!("{root}/settings"), native_settings.root)?;
    out.config(&format!("{root}/skeleton"), donor_config(donor, &format!("{root}/skeleton"))?.root)?;
    out.raw(&format!("{root}/render/bind_pose"), &bind_bytes)?;
    out.config(&skeleton_path, skeleton.root)?;
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
        let (src_mesh, a_box, b_box) = (name, a, b);
        let props = V::Named(vec![
            field("source", V::Named(vec![
                field("indices", V::Integer(i32::try_from(mesh.indices.len())?)),
                field("primitives", V::Integer(i32::try_from(mesh.indices.len() / 3)?)),
                field("mesh_name", s(format!("|{}|0.69d0|{src_mesh}", rifle.donor))),
                field("texture_name", s("")),
                field("vertices", V::Integer(i32::try_from(mesh.vertices.len())?)),
            ])),
            field("type", V::Integer(44)),
            field("version", V::Integer(0)),
            field("bounding_box", V::Named(vec![
                field("max", V::Float3(b_box)),
                field("min", V::Float3(a_box)),
            ])),
        ]);
        out.config(&format!("{root}/render/{name}/export_properties"), props)?;
        out.raw(&format!("{root}/render/{name}/converted_model"), &mesh.to_bytes()?)?;
    }
    let props = V::Named(vec![
        field(
            "src_filename",
            s(format!("Survarium 0.69d0: {root}")),
        ),
        field("type", V::Integer(40)),
        field("version", V::Integer(0)),
        field(
            "locators",
            V::Indexed(model.locators.iter().map(locator_value).collect()),
        ),
        field("bounding_box", V::Named(vec![
            field("max", V::Float3(hi)),
            field("min", V::Float3(lo)),
        ])),
    ]);
    out.config(&format!("{root}/render/export_properties"), props)?;
    Ok(model)
}

impl Locator {
    fn clone_locator(&self) -> Locator {
        Locator {
            name: self.name.clone(),
            bone: self.bone.clone(),
            bone_idx: self.bone_idx,
            position: self.position,
            rotation: self.rotation,
        }
    }
}

fn collect_bones(v: &V, out: &mut Vec<String>) {
    if let V::Named(children) = v {
        for c in children {
            if c.name != "mask" {
                out.push(c.name.clone());
                collect_bones(&c.value, out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Materials and textures
// ---------------------------------------------------------------------------

/// The donor map roles and the 0.26g ones they feed.
const MAP_ROLES: [(&str, &str, Option<convert::ToneCurve>); 4] = [
    ("t_albedo", "texture_diffuse", Some(convert::ALBEDO)),
    ("t_normal", "texture_normal", None),
    ("t_smoothness", "texture_roughness", Some(convert::SMOOTHNESS_TO_GLOSS)),
    ("t_metalness", "texture_fresnel", Some(convert::METALNESS_TO_FRESNEL)),
];

fn build_material(
    name: &str,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<()> {
    let path = format!("material_instances/{name}.material");
    if out.has(&path) {
        return Ok(());
    }
    ensure!(!target.contains(&path), "{path} exists in the target");
    let source = donor_config(donor, &path)?;
    let effect = source
        .root
        .named("material")
        .and_then(|m| m.named("g_stage"))
        .and_then(|g| g.named("effect"))
        .with_context(|| format!("{path} has no g_stage effect"))?;
    let mut material = parse_config(target, MATERIAL_TEMPLATE, "native rifle material")?;
    let mut names = BTreeMap::new();
    for (role, native, _) in MAP_ROLES {
        let texture = string_of(effect.named(role).and_then(|r| r.named("value")))
            .with_context(|| format!("{path}: {role}"))?
            .to_owned();
        ensure!(!texture.is_empty(), "{path} binds no {role}");
        material.root.set_path(
            &format!("material.g_stage.effect.{native}.value"),
            s(&texture),
        )?;
        names.insert(role, texture);
    }
    for (role, _, curve) in MAP_ROLES {
        let texture = &names[role];
        let dds = format!("textures/{texture}.dds");
        if out.has(&dds) {
            continue;
        }
        ensure!(!target.contains(&dds), "{dds} exists in the target");
        let bytes = donor.read(&dds)?;
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
        out.config(&format!("textures/{texture}.options"), options.root)?;
    }
    // The detail normal is the native weapons' (both builds name det/weap_nmap).
    ensure!(
        string_of(effect.named("t_normal_detail").and_then(|r| r.named("value")))? == "det/weap_nmap",
        "{path} has another detail normal"
    );
    out.config(&path, material.root)
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
// Animations
// ---------------------------------------------------------------------------

fn clip_path(rifle: &Rifle, hands: bool, clip: &str) -> String {
    let base = format!("animations/single/weapons/{}", rifle.donor);
    if hands {
        format!("{base}/player/stand/3rd_view/on_site/hands_only/{clip}")
    } else {
        format!("{base}/item/stand/3rd_view/{clip}")
    }
}

fn build_clips(rifle: &Rifle, donor: &mut ResourceDb, target: &mut ResourceDb, out: &mut Out) -> Result<()> {
    let hands_rig = clip_bones(&target.read(NATIVE_HANDS_CLIP)?)?;
    let bind = bind_pose(&donor.read(&format!("{}/render/bind_pose", model_root(rifle)))?)?;
    let mut item_rig: Vec<String> = vec!["object_movement".into()];
    item_rig.extend(bind.iter().map(|(n, _)| n.clone()));
    for clip in CLIPS {
        for hands in [false, true] {
            let path = clip_path(rifle, hands, clip);
            let names = donor_config(donor, &format!("{path}.bones_names"))?;
            let bones = clip_bones(&names.to_bytes()?)?;
            if hands {
                ensure!(
                    bones == hands_rig,
                    "{path} drives other bones than 0.26g's player rig"
                );
            } else {
                let set: BTreeSet<&String> = bones.iter().collect();
                ensure!(
                    set == item_rig.iter().collect(),
                    "{path} drives other bones than the weapon's"
                );
            }
            let mut bytes = convert::bspline_069_to_026(&donor.read(&format!("{path}.b-spline"))?)?;
            ensure!(
                convert::clip_header(&bytes)?.bones == bones.len(),
                "{path}: curves and bones_names disagree"
            );
            if !hands {
                let indices = match clip {
                    "reload" => rifle.empty_events,
                    "reload_tactic" => rifle.tactical_events,
                    _ => None,
                };
                if let Some(indices) = indices {
                    convert::set_sound_events(&mut bytes, indices)?;
                }
                if clip.starts_with("reload") {
                    for e in convert::events_026(&bytes)? {
                        if e.name == "sound_events" {
                            for d in e.domains {
                                ensure!(
                                    d == 0xff || usize::from(d) < rifle.reload_sounds.len(),
                                    "{path} cues sound {d} of {}",
                                    rifle.reload_sounds.len()
                                );
                            }
                        }
                    }
                }
            }
            out.raw(&format!("{path}.b-spline"), &bytes)?;
            out.config(&format!("{path}.bones_names"), names.root)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------------

/// Copies a sound collection and its sounds from the donor unless the target
/// has it. Returns whether it was carried.
fn carry_collection(name: &str, donor: &mut ResourceDb, target: &mut ResourceDb, out: &mut Out) -> Result<bool> {
    let path = format!("sounds/collections/{name}.sound_collection");
    if target.contains(&path) || out.has(&path) {
        return Ok(false);
    }
    let config = donor_config(donor, &path)?;
    let Some(V::Indexed(items)) = config
        .root
        .named("collection")
        .and_then(|c| c.named("sound_items"))
    else {
        bail!("{path} lists no sounds");
    };
    for item in items {
        let file = string_of(item.named("filename"))?;
        match item.named("resource_type") {
            None | Some(V::Integer(256)) => {}
            other => bail!("{path}: {file} is a {other:?}, not a single sound"),
        }
        let single = format!("sounds/single/{file}");
        if target.contains(&format!("{single}.high.ogg")) || out.has(&format!("{single}.high.ogg")) {
            continue;
        }
        for ext in ["high.ogg", "high.rms"] {
            out.raw(&format!("{single}.{ext}"), &donor.read(&format!("{single}.{ext}"))?)?;
        }
        out.config(
            &format!("{single}.raw_options"),
            donor_config(donor, &format!("{single}.raw_options"))?.root,
        )?;
    }
    out.config(&path, config.root)?;
    Ok(true)
}

fn composite(layers: &[Layer], master: Option<i32>) -> V {
    V::Named(vec![field(
        "composite_sound",
        V::Named(vec![
            field("use_master_layer_index", V::Boolean(master.is_some())),
            field("master_layer_index", V::Integer(master.unwrap_or(0))),
            field(
                "sound_items",
                V::Indexed(
                    layers
                        .iter()
                        .map(|l| {
                            V::Named(vec![
                                field("filename", s(l.collection)),
                                field("enabled", V::Boolean(true)),
                                field("max_distance", V::Integer(2000)),
                                field("resource_type", V::Integer(258)),
                                field("start_offset", V::Integer(l.start_offset)),
                                field("start_offset_max", V::Integer(l.start_offset)),
                                field("min_distance", V::Integer(0)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    )])
}

fn build_sounds(rifle: &Rifle, donor: &mut ResourceDb, target: &mut ResourceDb, out: &mut Out, carried: &mut BTreeSet<String>) -> Result<()> {
    let mut names: Vec<&str> = rifle.reload_sounds.to_vec();
    names.extend(rifle.fire_first.iter().map(|l| l.collection));
    names.extend(rifle.fire_third.iter().map(|l| l.collection));
    for name in names {
        if carry_collection(name, donor, target, out)? {
            carried.insert(name.into());
        }
        let path = format!("sounds/collections/{name}.sound_collection");
        ensure!(target.contains(&path) || out.has(&path), "{path} is missing");
    }
    for (name, layers, master) in [
        (rifle.composites[0], rifle.fire_first, rifle.fire_first_master),
        (rifle.composites[1], rifle.fire_third, rifle.fire_third_master),
    ] {
        let path = format!("sounds/composite/{name}.composite_sound");
        ensure!(!target.contains(&path), "{path} exists in the target");
        out.config(&path, composite(layers, master))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Item config
// ---------------------------------------------------------------------------

fn res(path: &str) -> V {
    s(format!("resources/{path}"))
}

fn sound_list(names: &[&str]) -> V {
    V::Indexed(
        names
            .iter()
            .map(|n| V::Indexed(vec![V::Integer(258), s(*n)]))
            .collect(),
    )
}

fn item_config(rifle: &Rifle, target: &mut ResourceDb, model: &str, icons: bool) -> Result<V> {
    let mut cfg = parse_config(target, CONFIG_TEMPLATE, "native rifle config")?.root;
    let st = &rifle.stats;
    for (key, value) in [
        ("faction", V::Integer(st.faction)),
        ("item_level", V::Integer(st.level)),
        ("match_making_level", V::Integer(st.level)),
        ("drop_weight", V::Integer(st.drop_weight)),
        ("rounds_per_minute", V::Integer(st.rounds_per_minute)),
        ("bullet_damage", V::Float(st.bullet_damage)),
        ("bullet_pierce", V::Float(st.bullet_pierce)),
        ("stopping_power", V::Float(st.stopping_power)),
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
    ] {
        ensure!(
            cfg.named("parameters").and_then(|p| p.named(key)).is_some(),
            "the template has no parameters.{key}"
        );
        cfg.set_path(&format!("parameters.{key}"), value)?;
    }
    cfg.set_path("dispersion.base_dispersion", V::Float(st.base_dispersion))?;
    let hands = |clip: &str| res(&clip_path(rifle, true, clip));
    let item = |clip: &str| res(&clip_path(rifle, false, clip));
    for (state, clip, slots) in [
        ("show", "show", 4),
        ("hide", "hide", 4),
        ("idle", "idle", 8),
        ("fire", "shoot", 8),
    ] {
        cfg.set_path(
            &format!("states.{state}.user_animations"),
            V::Indexed((0..slots).map(|_| hands(clip)).collect()),
        )?;
        cfg.set_path(&format!("states.{state}.animations"), V::Indexed(vec![item(clip)]))?;
    }
    // Eight reload slots in chambered/empty pairs and two item clips, as the
    // native rifles with an empty reload (Kiparis, TT-33) list them.
    cfg.set_path(
        "states.reload.user_animations",
        V::Indexed(
            (0..8)
                .map(|i| hands(if i % 2 == 0 { "reload_tactic" } else { "reload" }))
                .collect(),
        ),
    )?;
    cfg.set_path(
        "states.reload.animations",
        V::Indexed(vec![item("reload_tactic"), item("reload")]),
    )?;
    for view in ["first_view", "third_view"] {
        cfg.set_path(
            &format!("states.reload.sounds.{view}"),
            sound_list(rifle.reload_sounds),
        )?;
    }
    cfg.set_path(
        "states.reload.sounds.simultaneous_sounds_queue_size",
        V::Integer(i32::try_from(rifle.reload_sounds.len())?),
    )?;
    cfg.set_path(
        "states.fire.sounds.first_view",
        V::Indexed(vec![V::Indexed(vec![V::Integer(257), s(rifle.composites[0])])]),
    )?;
    cfg.set_path(
        "states.fire.sounds.third_view",
        V::Indexed(vec![V::Indexed(vec![V::Integer(257), s(rifle.composites[1])])]),
    )?;
    cfg.set_path("user_animations_in_place.preview_item", item("idle"))?;
    cfg.set_path("object.model", s(model))?;
    cfg.set_path("object.skeleton", s(format!("weapons/{0}/{0}", rifle.donor)))?;
    cfg.set_path("ui_desc.text_descriptions.name", s(rifle.texts[0].0))?;
    cfg.set_path("ui_desc.text_descriptions.description", s(rifle.texts[1].0))?;
    cfg.set_path("ui_desc.combat_log_icon", V::Integer(rifle.combat_log_icon))?;
    if icons {
        cfg.set_path(
            "ui_desc.icon",
            V::Indexed(rifle.icon.iter().map(|v| V::Integer(*v)).collect()),
        )?;
    }
    // The card: the AK-12's eight properties, with these values.
    let fmt = |v: f32| {
        let t = format!("{v:.2}");
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let props = [
        (1, format!("{}", (st.bullet_damage * 100.0).round() as i32)),
        (3, format!("{}", (st.bullet_pierce * 100.0).round() as i32)),
        (133, fmt(st.aim_time)),
        (132, format!("{}", st.max_damage_distance)),
        (2, fmt(st.base_dispersion)),
        (24, format!("{}", st.min_damage_distance)),
        (4, format!("{}", st.rounds_per_minute)),
        (15, fmt(st.weight)),
    ];
    let Some(V::Indexed(template_props)) = cfg.named("ui_desc").and_then(|u| u.named("props_list")) else {
        bail!("the template card has no props_list");
    };
    let ids: Vec<i32> = template_props
        .iter()
        .map(|p| int_of(p.named("prop_id")))
        .collect::<Result<_>>()?;
    ensure!(
        ids == props.iter().map(|p| p.0).collect::<Vec<_>>(),
        "the template card lists {ids:?}"
    );
    cfg.set_path(
        "ui_desc.props_list",
        V::Indexed(
            props
                .iter()
                .map(|(id, value)| {
                    V::Named(vec![
                        field("prop_value", s(value)),
                        field("prop_id", V::Integer(*id)),
                    ])
                })
                .collect(),
        ),
    )?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

fn dictionary_rows(dictionary: &mut V, rifles: &[&Rifle]) -> Result<Vec<String>> {
    let V::Indexed(items) = dictionary.named_mut("items_dict").context("items_dict")? else {
        bail!("items_dict is not indexed");
    };
    let template = items
        .iter()
        .find(|v| v.named("cfg_name") == Some(&s(CONFIG_TEMPLATE)))
        .context("no AK-12 row")?
        .clone();
    for ammo in AMMO {
        let row = items
            .iter()
            .find(|v| v.named("dict_id") == Some(&V::Integer(ammo)))
            .with_context(|| format!("ammo {ammo} is not in the dictionary"))?;
        ensure!(
            string_of(row.named("name"))?.starts_with("ammo_5.45x39"),
            "item {ammo} is not 5.45x39"
        );
    }
    for rifle in rifles {
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(rifle.dict_id))),
            "id {} is taken",
            rifle.dict_id
        );
        ensure!(
            !items.iter().any(|v| v.named("cfg_name") == Some(&s(rifle.config))
                || v.named("name") == Some(&s(rifle.name))),
            "{} is taken",
            rifle.name
        );
        let mut row = template.clone();
        row.set_path("cfg_name", s(rifle.config))?;
        row.set_path("name", s(rifle.name))?;
        row.set_path("dict_id", V::Integer(rifle.dict_id))?;
        row.set_path("base_cost", V::Integer(rifle.stats.base_cost))?;
        row.set_path("is_premium", V::Boolean(false))?;
        items.push(row);
    }
    let V::Named(compat) = dictionary
        .named_mut("items_compatibility")
        .context("items_compatibility")?
    else {
        bail!("items_compatibility is not a table");
    };
    let next = compat
        .iter()
        .filter_map(|c| c.name.strip_prefix("compatibility_")?.parse::<u32>().ok())
        .max()
        .context("no compatibility rows")?
        + 1;
    let mut added = vec![];
    for (k, (rifle, ammo)) in rifles
        .iter()
        .flat_map(|r| AMMO.iter().map(move |a| (r, *a)))
        .enumerate()
    {
        let name = format!("compatibility_{}", next + k as u32);
        compat.push(field(
            &name,
            V::Named(vec![
                field("second_item", V::Integer(ammo)),
                field("first_item", V::Integer(rifle.dict_id)),
            ]),
        ));
        added.push(name);
    }
    Ok(added)
}

/// Builds the bundle (`overlay/`, `port-manifest.json`, `equipment.json`)
/// over `target` (the stock archive with the patches players have) from
/// `donor`, an archive of the 0.69d0 files the rifles use.
pub fn build(
    donor_db: &Path,
    target_layers: &[std::path::PathBuf],
    icons_swf: Option<&Path>,
    output: &Path,
) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut donor = ResourceDb::open(donor_db)?;
    let work = output.join("work");
    let mut out = Out {
        overlay: &overlay,
        work: &work,
        files: vec![],
        written: BTreeSet::new(),
    };
    let mut carried = BTreeSet::new();
    let mut registration = vec![];
    let mut configs = vec![];
    for rifle in RIFLES {
        ensure!(!target.contains(rifle.config), "{} exists", rifle.config);
        let model = build_model(rifle, &mut donor, &mut target, &mut out)?;
        let materials: BTreeSet<String> = model.surfaces.iter().map(|(_, _, m)| m.clone()).collect();
        for material in &materials {
            build_material(material, &mut donor, &mut target, &mut out)?;
        }
        build_clips(rifle, &mut donor, &mut target, &mut out)?;
        build_sounds(rifle, &mut donor, &mut target, &mut out, &mut carried)?;
        let model_name = model_root(rifle)
            .trim_start_matches("models/")
            .trim_end_matches(".skinned_model")
            .to_owned();
        let cfg = item_config(rifle, &mut target, &model_name, icons_swf.is_some())?;
        out.config(rifle.config, cfg.clone())?;
        configs.push(cfg);
        registration.push(json!({
            "dict_id": rifle.dict_id,
            "name": rifle.name,
            "config": rifle.config,
            "faction": rifle.stats.faction,
            "item_level": rifle.stats.level,
            "ammo": AMMO,
            "surfaces": model.surfaces.iter().map(|(n, m, _)| json!({"name": n, "vertices": m.vertices.len()})).collect::<Vec<_>>(),
        }));
    }
    let mut dictionary = parse_config(&mut target, DICT, "0.26g dictionary")?;
    let compat = dictionary_rows(&mut dictionary.root, &RIFLES)?;
    out.config(DICT, dictionary.root)?;
    let mut localizations = vec![];
    for lang in LANGUAGES {
        let path = format!("localization/{lang}/localization");
        let mut loc = parse_config(&mut target, &path, "localization")?;
        for rifle in RIFLES {
            for (key, text) in &rifle.texts {
                set_string(&mut loc.root, key, text.get(lang)?)?;
            }
        }
        localizations.push((lang, loc.root));
    }
    let views: Vec<(&str, &V)> = localizations.iter().map(|(l, r)| (*l, r)).collect();
    for cfg in &configs {
        check_item_texts(cfg, &views)?;
    }
    for (lang, root) in localizations {
        out.config(&format!("localization/{lang}/localization"), root)?;
    }
    if let Some(path) = icons_swf {
        out.raw("flash_movies/lobby_menu.swf", &fs::read(path)?)?;
    }
    if work.exists() {
        fs::remove_dir(&work).context("the encoder left files behind")?;
    }
    // Every reference the new files make resolves.
    check_references(&mut target, &out)?;
    let mut files = out.files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": "weapons-hk416-radon",
            "files": files,
            "donor_db": donor_db,
            "target": target_layers,
            "items": registration,
            "compatibility": compat,
            "carried_sound_collections": carried,
            "limitations": [
                "Provisional statistics (owner to confirm)",
                "Attachment modules are baked into the model; 0.26g rifles have none",
                "0.69d per-location reverbs replaced by native rifle tails",
                "Kill-feed icon is a native rifle's",
            ],
        }))?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(
            &RIFLES
                .iter()
                .map(|r| json!({"dict_id": r.dict_id}))
                .collect::<Vec<_>>(),
        )?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {}: {} files",
        RIFLES.map(|r| r.name).join(", "),
        files.len()
    );
    Ok(())
}

/// Every resource path the new configs name exists in the target or the
/// overlay: models, skeletons, clips, materials, textures, sounds.
fn check_references(target: &mut ResourceDb, out: &Out) -> Result<()> {
    // What the native templates name is native (built-ins such as
    // `editor/default_bump` included).
    let mut native = BTreeSet::new();
    for template in [CONFIG_TEMPLATE, SETTINGS_TEMPLATE, MATERIAL_TEMPLATE] {
        let config = parse_config(target, template, "native template")?;
        let mut strings = vec![];
        config.root.collect_strings(&mut strings);
        native.extend(strings.into_iter().map(str::to_owned));
    }
    let exists = |p: &str, target: &ResourceDb| target.contains(p) || out.has(p);
    for path in out.written.clone() {
        let bytes = fs::read(crate::archive::safe_join(out.overlay, &path)?)?;
        let Ok(config) = BinaryConfig::parse(&bytes) else {
            continue;
        };
        if path.starts_with("localization/") || path == DICT {
            continue;
        }
        let mut strings = vec![];
        config.root.collect_strings(&mut strings);
        for s in strings {
            if native.contains(s) {
                continue;
            }
            let candidates: Vec<String> = if let Some(r) = s.strip_prefix("resources/animations/single/") {
                vec![format!("animations/single/{r}.b-spline")]
            } else if path.ends_with(".material") && s.contains('/') && !s.contains(':') {
                vec![format!("textures/{s}.dds")]
            } else if path.ends_with("/settings") && s.contains('/') {
                vec![
                    format!("material_instances/{s}.material"),
                    format!("animations/skeletons/{s}.skeleton"),
                    format!("{s}.skeleton"),
                ]
            } else if path.ends_with(".composite_sound") && s.contains('/') {
                vec![format!("sounds/collections/{s}.sound_collection")]
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
