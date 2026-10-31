// SPDX-License-Identifier: GPL-3.0-or-later
//! The rest of the STK2 characters export (`___surva_stk2_characters_port`)
//! as 0.26g equipment: three outfits, one content pack each.
//!
//! - `bandit` (`bandit_01`): the leather bomber, the track pants and the
//!   sneakers, Black Market. Its two balaclavas shipped already (1810, 1823);
//!   `bandit_01_head_03` is a bare face and its hands are bare skin (with a
//!   watch and a bracelet): not equipment.
//! - `novice` (`neutral_01`, which wears STK2's `neutral_01_novice`
//!   textures): the hood, the two field jackets (`torso_s*` with the canteen
//!   and knife of its attachment, `torso_v2_s*` with two belt pouches), the
//!   backpack, the jeans, the boots and the fingerless gloves, Scavengers.
//!   Its head is the donor's face under the knit cap that shipped as 1821.
//! - `loner` (`neutral_03`, which wears STK2's `neutral_01` textures): the
//!   hooded jacket under its vest with four pouches, the camouflage pants
//!   with their pouch, holster and nuts bag, the boots and the fingerless
//!   gloves, Scavengers. Its head is a bare face.
//!
//! The export's models are already 0.26g skinned models on the native
//! 69-bone rig, turned 180 degrees about Y ([`crate::duty_026::rotate_mesh`]).
//! Their `_s1`, `_s2` and `_s3` are byte-identical copies (the variant names
//! of the native torsos and trousers, never cut): each piece is built here
//! in the variants the game draws, as stock pieces are:
//! - a jacket's sleeves over wrist-length gloves (`data_full`) and into
//!   gauntlets (`data_short`, `data_long`), its waist over every trousers and
//!   its collar over the native neck and every mask;
//! - trousers over low boots (`data_full`) and tucked into medium and tall
//!   boots (`data_medium`, `data_long`);
//! - gloves inside every wrist-length sleeve; boots inside every full-length
//!   trousers hem (all three outfits' boots are worn so) or over every
//!   tucked trousers leg;
//! - the hood over the native head, as the knit cap is; it covers the mask
//!   slot, as the stock helmets that cannot go over a mask do.
//!
//! The fits see the released game's pieces and each other's, so any mix of
//! the three outfits with stock equipment is fitted. Items copy a stock
//! piece of the same slot at their level whole (stats, level, price, drop
//! weight, bonuses, card), with only the model, icon and texts their own,
//! and no faction: they are the Independent traders', sold by no one, and
//! only drop from matches (the owner, 2026-10-31).
use crate::body_fit::{BodyFit, Cover};
use crate::limb_fit::{Layer, LimbFit};
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::manifest::BuiltFile;
use crate::mask_fit::{Mesh, Shell, bone_origins, dot, parse_mesh, read_u32, sub};
use crate::outfit_026::{
    Finish, Writer, converted_bounds, keep_triangles, remap_uvs, s, tga_size, uv_crop,
};
use crate::weapon::{parse_config, prepare_empty_output, record_file};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const BASE: &str = "models/character/human/base/base_character_boot.skinned_model";
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";
const DICTIONARY: &str = "gameplay/db_static_dictionaries";
/// The export's models, by `<folder>/<model>` under `human/`.
const PLATFORM: &str = "resources/platforms/pc_dx11/models/character/human";
const EXPORTED: &str = "resources/exported/models/character/human";
/// STK2's character textures under `--textures`.
const TEXTURES: &str = "resources/sources/textures/character/human";
/// The native character maps are 1024 texels; larger donor maps are halved
/// down to it.
const MAX_MAP: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Helmet,
    Torso,
    Back,
    Legs,
    Gloves,
    Boots,
}

impl Kind {
    fn category(self) -> &'static str {
        match self {
            Kind::Helmet => "helmet",
            Kind::Torso => "torso",
            Kind::Back => "back",
            Kind::Legs => "legs",
            Kind::Gloves => "gloves",
            Kind::Boots => "boots",
        }
    }

    /// The profile slot (helmet 0, mask 1, torso 2, back 3, legs 4, gloves
    /// 5, boots 6).
    fn slot(self) -> u8 {
        match self {
            Kind::Helmet => 0,
            Kind::Torso => 2,
            Kind::Back => 3,
            Kind::Legs => 4,
            Kind::Gloves => 5,
            Kind::Boots => 6,
        }
    }

    /// The config tables of the variants the game draws, each with the
    /// model suffix built for it: the first is the piece as exported.
    fn variants(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Kind::Torso => &[
                ("data_full", ""),
                ("data_short", "_s2"),
                ("data_long", "_s3"),
            ],
            Kind::Legs => &[
                ("data_full", ""),
                ("data_medium", "_s2"),
                ("data_long", "_s3"),
            ],
            _ => &[("data", "")],
        }
    }
}

/// A donor texture set: `<stem>.tga` and `<stem>_nmap.tga` under
/// [`TEXTURES`], drawn by one material named `name`.
pub struct Map {
    pub name: &'static str,
    pub stem: &'static str,
    pub(crate) finish: Finish,
    /// Rows of the diffuse map (v from, v to) STK2 painted black because its
    /// jacket hid them; 0.26g's shorter jackets show them. They are painted
    /// over with the rows below, reflected across the band's lower edge.
    pub black_band: Option<[f64; 2]>,
    /// A waistband painted into a free strip of the map ([`WAISTBAND_STRIP`]),
    /// for trousers whose own top STK2 never textured (the Novice jeans).
    pub waistband: bool,
}

/// One surface of an exported model, drawn with the map `map`.
pub struct Part {
    pub model: &'static str,
    pub surface: &'static str,
    pub map: &'static str,
    /// A jacket with its sleeves pushed up: below them the native forearms
    /// show (the native body's torso variant of each glove class, with its
    /// own skin material), as the stock torsos with bare arms draw them.
    /// Every stock glove's cuff is made for them.
    pub forearms: bool,
}

pub struct Piece {
    pub id: i32,
    pub kind: Kind,
    /// The piece's model and config name in the outfit's namespace.
    pub name: &'static str,
    pub parts: &'static [Part],
    /// The first-person hands (gloves): the exported `_hud` model.
    pub hud: Option<&'static str>,
    /// The stock piece whose config this one copies.
    pub template: &'static str,
    pub icon: [i32; 2],
    /// The trousers or sleeves the piece is worn with (boots and gloves);
    /// `None` keeps the template's.
    pub model_type: Option<&'static str>,
    /// Its own numbers, over the template's.
    pub stats: Stats,
    pub title: Text,
    pub description: Text,
}

/// A piece's numbers (the owner's, 2026-10-31), set over its template's:
/// each inside the stock range of its slot and level in the released game
/// data (stk2-outfits.md of skhid-srp, "Stats").
pub struct Stats {
    /// `parameters.item_level` and `match_making_level`.
    pub level: i32,
    /// `None`: a back piece, which has no hit regions.
    pub protection: Option<Protection>,
    /// kg: `parameters.weight` and the shown weight (prop 15).
    pub weight: f32,
    /// `default_modifications`: stock bonuses, by id in modifications.options.
    pub bonuses: &'static [i32],
    /// The dictionary's `base_cost` (repair is a share of it).
    pub base_cost: i32,
    pub drop_weight: i32,
}

/// Percent, as the shop shows it: armour against bullets, blades and
/// blasts, isolation against fire, shock, toxins and radiation, in every hit
/// region the template has.
pub struct Protection {
    pub armour: u8,
    pub isolation: u8,
    /// The forearms' armour where it is not the body's (a vest over a jacket).
    pub forearms: Option<u8>,
}

const fn worn(armour: u8, isolation: u8) -> Option<Protection> {
    Some(Protection { armour, isolation, forearms: None })
}

pub struct Outfit {
    pub key: &'static str,
    /// Namespace of its models, materials, textures, configs and texts.
    pub ns: &'static str,
    /// `parameters.faction` of its items: 0, no faction (the Independent
    /// traders', as the Novice's knit cap): srp sells gear of no faction
    /// nowhere unless a named exception sells it, so they only drop.
    pub faction: i32,
    pub maps: &'static [Map],
    pub pieces: &'static [Piece],
}

const fn text(
    english: &'static str,
    russian: &'static str,
    ukrainian: &'static str,
    german: &'static str,
    polish: &'static str,
) -> Text {
    Text {
        english,
        russian,
        ukrainian,
        german,
        polish,
    }
}

/// No faction: the Independent traders' (the owner, 2026-10-31).
const NO_FACTION: i32 = 0;

pub const BANDIT: Outfit = Outfit {
    key: "bandit",
    ns: "stk2_bandit_026",
    faction: NO_FACTION,
    maps: &[
        Map {
            name: "torso",
            stem: "bandit/bandit_01/bandit_01_torso",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "attach",
            stem: "bandit/bandit_01/bandit_01_attach",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "legs",
            stem: "bandit/bandit_01/bandit_01_legs",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
    ],
    pieces: &[
        Piece {
            id: 1864,
            kind: Kind::Torso,
            name: "torso",
            parts: &[
                // The bomber's sleeves are pushed up: the native forearms
                // under them. Not the donor hands model's (its forearm stood
                // through every glove's cuff in the game).
                Part {
                    model: "bandit/bandit_01/bandit_01_torso_s1",
                    surface: "bandit_01_torso_s",
                    map: "torso",
                    forearms: true,
                },
                Part {
                    model: "bandit/bandit_01/bandit_01_torso_s1",
                    surface: "bandit_01_attach_s",
                    map: "attach",
                    forearms: false,
                },
            ],
            hud: None,
            template: "gameplay/items/armour/torso/edge_torso_1.options",
            icon: [1, 9],
            model_type: None,
            // decent armour, low isolation (leather); a pocket.
            stats: Stats { level: 6, protection: worn(55, 5), weight: 7.0, bonuses: &[1030], base_cost: 20900, drop_weight: 100 },
            title: text(
                "Leather bomber jacket",
                "Кожаный бомбер",
                "Шкіряний бомбер",
                "Lederbomberjacke",
                "Skórzana kurtka bomberka",
            ),
            description: text(
                "A black leather bomber jacket over a T-shirt, from the cancelled S.T.A.L.K.E.R. 2.",
                "Чёрный кожаный бомбер поверх футболки, из отменённого S.T.A.L.K.E.R. 2.",
                "Чорний шкіряний бомбер поверх футболки, зі скасованого S.T.A.L.K.E.R. 2.",
                "Eine schwarze Lederbomberjacke über einem T-Shirt, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Czarna skórzana kurtka bomberka na koszulce, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1865,
            kind: Kind::Legs,
            name: "legs",
            parts: &[Part {
                model: "bandit/bandit_01/bandit_01_legs_s1",
                surface: "bandit_01_legs_s",
                map: "legs",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/legs/edge_legs_1.options",
            icon: [2, 9],
            model_type: None,
            // light and mobile: the level's least protection and weight.
            stats: Stats { level: 6, protection: worn(50, 10), weight: 1.6, bonuses: &[1163, 1113], base_cost: 10450, drop_weight: 100 },
            title: text(
                "Track pants",
                "Спортивные штаны",
                "Спортивні штани",
                "Trainingshose",
                "Spodnie dresowe",
            ),
            description: text(
                "Striped track pants, from the cancelled S.T.A.L.K.E.R. 2.",
                "Спортивные штаны с лампасами, из отменённого S.T.A.L.K.E.R. 2.",
                "Спортивні штани з лампасами, зі скасованого S.T.A.L.K.E.R. 2.",
                "Trainingshose mit Streifen, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Spodnie dresowe w paski, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1866,
            kind: Kind::Boots,
            name: "boots",
            parts: &[Part {
                model: "bandit/bandit_01/bandit_01_boot",
                surface: "bandit_01_legs_s",
                map: "legs",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/boots/edge_boots_1.options",
            icon: [3, 9],
            model_type: Some("data_full"),
            // light and mobile: the level's least of everything; movement only.
            stats: Stats { level: 6, protection: worn(50, 10), weight: 1.0, bonuses: &[1113], base_cost: 5230, drop_weight: 100 },
            title: text(
                "Sneakers",
                "Кроссовки",
                "Кросівки",
                "Turnschuhe",
                "Buty sportowe",
            ),
            description: text(
                "Worn-out sneakers, from the cancelled S.T.A.L.K.E.R. 2.",
                "Поношенные кроссовки, из отменённого S.T.A.L.K.E.R. 2.",
                "Поношені кросівки, зі скасованого S.T.A.L.K.E.R. 2.",
                "Abgetragene Turnschuhe, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Znoszone buty sportowe, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
    ],
};

pub const NOVICE: Outfit = Outfit {
    key: "novice",
    ns: "stk2_novice_026",
    faction: NO_FACTION,
    maps: &[
        Map {
            name: "torso",
            stem: "neutral/neutral_01_novice/neutral_01_novice_torso",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "attach",
            stem: "neutral/neutral_01_novice/neutral_01_novice_attach",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        // The jeans' waist fades to black from v 0.424 to 0.517 (rows
        // 869-1059 of 2048), where STK2's jacket covered them; their top
        // edge samples row 867, which the raised waist band repeats, so the
        // paint starts at 866.
        Map {
            name: "legs",
            stem: "neutral/neutral_01_novice/neutral_01_novice_legs",
            finish: Finish::CLOTH,
            black_band: Some([866.0 / 2048.0, 1060.0 / 2048.0]),
            waistband: true,
        },
    ],
    pieces: &[
        Piece {
            id: 1867,
            kind: Kind::Helmet,
            name: "hood",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_helmet",
                surface: "neutral_01_attach_s",
                map: "attach",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/helmet/scavenger_helmet_5.options",
            icon: [4, 9],
            model_type: None,
            // light head armour and some isolation; no stock helmet has a bonus.
            stats: Stats { level: 4, protection: worn(20, 20), weight: 1.0, bonuses: &[], base_cost: 8080, drop_weight: 120 },
            title: text(
                "Novice's hood",
                "Капюшон новичка",
                "Каптур новачка",
                "Kapuze des Neulings",
                "Kaptur nowicjusza",
            ),
            description: text(
                "A novice stalker's canvas hood, from the cancelled S.T.A.L.K.E.R. 2.",
                "Брезентовый капюшон сталкера-новичка, из отменённого S.T.A.L.K.E.R. 2.",
                "Брезентовий каптур сталкера-новачка, зі скасованого S.T.A.L.K.E.R. 2.",
                "Segeltuchkapuze eines Neulings-Stalkers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Płócienny kaptur stalkera nowicjusza, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            // The id resource-porter's earlier Novice port reserved for this
            // jacket (`novice_026::JACKET_ID`), never shipped.
            id: 1822,
            kind: Kind::Torso,
            name: "torso",
            parts: &[
                Part {
                    model: "neutral/neutral_01/neutral_01_torso_s1",
                    surface: "neutral_01_torso_s",
                    map: "torso",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_01/neutral_01_torso_s1",
                    surface: "neutral_01_attach_s",
                    map: "attach",
                    forearms: false,
                },
            ],
            hud: None,
            template: "gameplay/items/armour/torso/blackmarket_torso_5.options",
            icon: [19, 6],
            model_type: None,
            // the level's top isolation and a pocket.
            stats: Stats { level: 4, protection: worn(40, 30), weight: 8.0, bonuses: &[1030], base_cost: 12350, drop_weight: 120 },
            title: text(
                "Novice's field jacket",
                "Полевая куртка новичка",
                "Польова куртка новачка",
                "Feldjacke des Neulings",
                "Kurtka polowa nowicjusza",
            ),
            description: text(
                "A novice stalker's field jacket with a canteen and a knife on the belt, from the cancelled S.T.A.L.K.E.R. 2.",
                "Полевая куртка сталкера-новичка с флягой и ножом на ремне, из отменённого S.T.A.L.K.E.R. 2.",
                "Польова куртка сталкера-новачка з флягою та ножем на ремені, зі скасованого S.T.A.L.K.E.R. 2.",
                "Feldjacke eines Neulings-Stalkers mit Feldflasche und Messer am Gürtel, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Kurtka polowa stalkera nowicjusza z manierką i nożem przy pasie, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1868,
            kind: Kind::Torso,
            name: "torso_v2",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_torso_v2_s1",
                surface: "neutral_01_torso_s",
                map: "torso",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/torso/blackmarket_torso_5.options",
            icon: [5, 9],
            model_type: None,
            // its belt pouches an ammo bag, paid for in isolation and weight.
            stats: Stats { level: 4, protection: worn(40, 20), weight: 8.5, bonuses: &[1030, 1050], base_cost: 13650, drop_weight: 120 },
            title: text(
                "Novice's field jacket with pouches",
                "Полевая куртка новичка с подсумками",
                "Польова куртка новачка з підсумками",
                "Feldjacke des Neulings mit Taschen",
                "Kurtka polowa nowicjusza z ładownicami",
            ),
            description: text(
                "A novice stalker's field jacket with two pouches on the belt, from the cancelled S.T.A.L.K.E.R. 2.",
                "Полевая куртка сталкера-новичка с двумя подсумками на ремне, из отменённого S.T.A.L.K.E.R. 2.",
                "Польова куртка сталкера-новачка з двома підсумками на ремені, зі скасованого S.T.A.L.K.E.R. 2.",
                "Feldjacke eines Neulings-Stalkers mit zwei Taschen am Gürtel, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Kurtka polowa stalkera nowicjusza z dwiema ładownicami przy pasie, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1869,
            kind: Kind::Back,
            name: "back",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_back",
                surface: "neutral_01_attach_s",
                map: "attach",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/back/bm_back_1.options",
            icon: [6, 9],
            model_type: None,
            // capacity, as the level-3 and level-5 packs; the price between the level-3 and level-6 packs'.
            stats: Stats { level: 4, protection: None, weight: 4.5, bonuses: &[1030, 1170], base_cost: 7920, drop_weight: 120 },
            title: text(
                "Novice's backpack",
                "Рюкзак новичка",
                "Рюкзак новачка",
                "Rucksack des Neulings",
                "Plecak nowicjusza",
            ),
            description: text(
                "A novice stalker's one-strap canvas backpack, from the cancelled S.T.A.L.K.E.R. 2.",
                "Брезентовый рюкзак сталкера-новичка на одной лямке, из отменённого S.T.A.L.K.E.R. 2.",
                "Брезентовий рюкзак сталкера-новачка на одній лямці, зі скасованого S.T.A.L.K.E.R. 2.",
                "Einriemiger Segeltuchrucksack eines Neulings-Stalkers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Płócienny plecak stalkera nowicjusza na jednym pasku, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1870,
            kind: Kind::Legs,
            name: "legs",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_legs_s1",
                surface: "neutral_01_legs_s",
                map: "legs",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/legs/scavenger_legs_3.options",
            icon: [7, 9],
            model_type: None,
            // middling; pocket size.
            stats: Stats { level: 4, protection: worn(40, 20), weight: 1.4, bonuses: &[1170], base_cost: 6180, drop_weight: 120 },
            title: text(
                "Novice's jeans",
                "Джинсы новичка",
                "Джинси новачка",
                "Jeans des Neulings",
                "Dżinsy nowicjusza",
            ),
            description: text(
                "A novice stalker's worn jeans, from the cancelled S.T.A.L.K.E.R. 2.",
                "Поношенные джинсы сталкера-новичка, из отменённого S.T.A.L.K.E.R. 2.",
                "Поношені джинси сталкера-новачка, зі скасованого S.T.A.L.K.E.R. 2.",
                "Abgetragene Jeans eines Neulings-Stalkers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Znoszone dżinsy stalkera nowicjusza, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1871,
            kind: Kind::Boots,
            name: "boots",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_boot",
                surface: "neutral_01_legs_s",
                map: "legs",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/boots/scavenger_boots_3.options",
            icon: [8, 9],
            // 26 cm, but worn under the jeans as STK2 wore them: the top of
            // the shaft is painted black where the jeans covered it, and
            // tucked trousers would show it.
            model_type: Some("data_full"),
            // middling, sturdier than the Military boots.
            stats: Stats { level: 4, protection: worn(35, 10), weight: 0.9, bonuses: &[1163], base_cost: 3090, drop_weight: 120 },
            title: text(
                "Novice's boots",
                "Ботинки новичка",
                "Черевики новачка",
                "Stiefel des Neulings",
                "Buty nowicjusza",
            ),
            description: text(
                "A novice stalker's leather boots, from the cancelled S.T.A.L.K.E.R. 2.",
                "Кожаные ботинки сталкера-новичка, из отменённого S.T.A.L.K.E.R. 2.",
                "Шкіряні черевики сталкера-новачка, зі скасованого S.T.A.L.K.E.R. 2.",
                "Lederstiefel eines Neulings-Stalkers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Skórzane buty stalkera nowicjusza, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1872,
            kind: Kind::Gloves,
            name: "hand",
            parts: &[Part {
                model: "neutral/neutral_01/neutral_01_hand",
                surface: "neutral_01_torso_s",
                map: "torso",
                forearms: false,
            }],
            hud: Some("neutral/neutral_01/neutral_01_hand_hud"),
            template: "gameplay/items/armour/gloves/eq046_blackmarket_gloves_5.options",
            icon: [9, 9],
            model_type: Some("data_full"),
            // light; bare fingertips search artefacts faster.
            stats: Stats { level: 4, protection: worn(35, 25), weight: 0.35, bonuses: &[1133], base_cost: 2620, drop_weight: 120 },
            title: text(
                "Novice's fingerless gloves",
                "Беспалые перчатки новичка",
                "Безпалі рукавиці новачка",
                "Fingerlose Handschuhe des Neulings",
                "Rękawiczki bez palców nowicjusza",
            ),
            description: text(
                "A novice stalker's fingerless gloves, from the cancelled S.T.A.L.K.E.R. 2.",
                "Беспалые перчатки сталкера-новичка, из отменённого S.T.A.L.K.E.R. 2.",
                "Безпалі рукавиці сталкера-новачка, зі скасованого S.T.A.L.K.E.R. 2.",
                "Fingerlose Handschuhe eines Neulings-Stalkers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Rękawiczki bez palców stalkera nowicjusza, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
    ],
};

pub const LONER: Outfit = Outfit {
    key: "loner",
    ns: "stk2_loner_026",
    faction: NO_FACTION,
    maps: &[
        Map {
            name: "torso",
            stem: "neutral/neutral_01/neutral_torso",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "legs",
            stem: "neutral/neutral_01/neutral_legs",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "bag_01",
            stem: "neutral/neutral_01/neutral_bag_01",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "bag_02",
            stem: "neutral/neutral_01/neutral_bag_02",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "bag_03",
            stem: "neutral/neutral_01/neutral_bag_03",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "bag_04",
            stem: "neutral/neutral_01/neutral_bag_04",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "bag_05",
            stem: "neutral/neutral_01/neutral_bag_05",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "holster",
            stem: "neutral/neutral_01/neutral_legholster",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
        Map {
            name: "nuts",
            stem: "neutral/neutral_01/neutral_nuts",
            finish: Finish::CLOTH,
            black_band: None,
            waistband: false,
        },
    ],
    pieces: &[
        Piece {
            id: 1873,
            kind: Kind::Torso,
            name: "torso",
            parts: &[
                Part {
                    model: "neutral/neutral_03/neutral_03_torso_s1",
                    surface: "neutral_03_torso_s",
                    map: "torso",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_torso_s1",
                    surface: "neutral_bag_02_s",
                    map: "bag_02",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_torso_s1",
                    surface: "neutral_bag_03_s",
                    map: "bag_03",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_torso_s1",
                    surface: "neutral_bag_04_s",
                    map: "bag_04",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_torso_s1",
                    surface: "neutral_bag_05_s",
                    map: "bag_05",
                    forearms: false,
                },
            ],
            hud: None,
            template: "gameplay/items/armour/torso/renesanse_torso_8.options",
            icon: [10, 9],
            model_type: None,
            // the most chest armour of the level-6 torsos; the sleeves the jacket's.
            stats: Stats { level: 6, protection: Some(Protection { armour: 60, isolation: 5, forearms: Some(30) }), weight: 9.5, bonuses: &[1030], base_cost: 23100, drop_weight: 100 },
            title: text(
                "Loner's jacket with vest",
                "Куртка одиночки с разгрузкой",
                "Куртка одинака з розвантаженням",
                "Jacke des Einzelgängers mit Weste",
                "Kurtka samotnika z kamizelką",
            ),
            description: text(
                "A hooded jacket under a load-bearing vest with pouches, from the cancelled S.T.A.L.K.E.R. 2.",
                "Куртка с капюшоном под разгрузочным жилетом с подсумками, из отменённого S.T.A.L.K.E.R. 2.",
                "Куртка з каптуром під розвантажувальним жилетом із підсумками, зі скасованого S.T.A.L.K.E.R. 2.",
                "Kapuzenjacke unter einer Tragweste mit Taschen, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Kurtka z kapturem pod kamizelką taktyczną z ładownicami, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1874,
            kind: Kind::Legs,
            name: "legs",
            parts: &[
                Part {
                    model: "neutral/neutral_03/neutral_03_legs_s1",
                    surface: "neutral_03_legs_s",
                    map: "legs",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_legs_s1",
                    surface: "neutral_bag_01_s",
                    map: "bag_01",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_legs_s1",
                    surface: "neutral_legholster_s",
                    map: "holster",
                    forearms: false,
                },
                Part {
                    model: "neutral/neutral_03/neutral_03_legs_s1",
                    surface: "neutral_nuts_s",
                    map: "nuts",
                    forearms: false,
                },
            ],
            hud: None,
            template: "gameplay/items/armour/legs/renesanse_legs_8.options",
            icon: [11, 9],
            model_type: None,
            // pouch and holster an ammo bag, the nuts bag pocket size.
            stats: Stats { level: 6, protection: worn(55, 15), weight: 1.9, bonuses: &[1050, 1170], base_cost: 11550, drop_weight: 100 },
            title: text(
                "Loner's camo pants",
                "Камуфляжные штаны одиночки",
                "Камуфляжні штани одинака",
                "Tarnhose des Einzelgängers",
                "Spodnie moro samotnika",
            ),
            description: text(
                "Camouflage pants with knee pads, a holster and pouches, from the cancelled S.T.A.L.K.E.R. 2.",
                "Камуфляжные штаны с наколенниками, кобурой и подсумками, из отменённого S.T.A.L.K.E.R. 2.",
                "Камуфляжні штани з наколінниками, кобурою та підсумками, зі скасованого S.T.A.L.K.E.R. 2.",
                "Tarnhose mit Knieschonern, Holster und Taschen, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Spodnie moro z nakolannikami, kaburą i ładownicami, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1875,
            kind: Kind::Boots,
            name: "boots",
            parts: &[Part {
                model: "neutral/neutral_03/neutral_03_boot",
                surface: "neutral_03_legs_s",
                map: "legs",
                forearms: false,
            }],
            hud: None,
            template: "gameplay/items/armour/boots/renesanse_boots_1.options",
            icon: [12, 9],
            model_type: Some("data_full"),
            // middling, the level's medians.
            stats: Stats { level: 6, protection: worn(55, 15), weight: 1.1, bonuses: &[1163], base_cost: 5780, drop_weight: 100 },
            title: text(
                "Loner's boots",
                "Ботинки одиночки",
                "Черевики одинака",
                "Stiefel des Einzelgängers",
                "Buty samotnika",
            ),
            description: text(
                "A loner's laced leather boots, from the cancelled S.T.A.L.K.E.R. 2.",
                "Кожаные ботинки одиночки на шнуровке, из отменённого S.T.A.L.K.E.R. 2.",
                "Шкіряні черевики одинака на шнурівці, зі скасованого S.T.A.L.K.E.R. 2.",
                "Geschnürte Lederstiefel eines Einzelgängers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Sznurowane skórzane buty samotnika, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
        Piece {
            id: 1876,
            kind: Kind::Gloves,
            name: "hand",
            parts: &[Part {
                model: "neutral/neutral_03/neutral_03_hand",
                surface: "neutral_03_torso_s",
                map: "torso",
                forearms: false,
            }],
            hud: Some("neutral/neutral_03/neutral_03_hand_hud"),
            template: "gameplay/items/armour/gloves/scavenger_gloves_5.options",
            icon: [13, 9],
            model_type: Some("data_full"),
            // light, a step above the level-5 gloves.
            stats: Stats { level: 6, protection: worn(50, 15), weight: 0.3, bonuses: &[1143], base_cost: 5230, drop_weight: 100 },
            title: text(
                "Loner's fingerless gloves",
                "Беспалые перчатки одиночки",
                "Безпалі рукавиці одинака",
                "Fingerlose Handschuhe des Einzelgängers",
                "Rękawiczki bez palców samotnika",
            ),
            description: text(
                "A loner's fingerless gloves, from the cancelled S.T.A.L.K.E.R. 2.",
                "Беспалые перчатки одиночки, из отменённого S.T.A.L.K.E.R. 2.",
                "Безпалі рукавиці одинака, зі скасованого S.T.A.L.K.E.R. 2.",
                "Fingerlose Handschuhe eines Einzelgängers, aus dem eingestellten S.T.A.L.K.E.R. 2.",
                "Rękawiczki bez palców samotnika, z anulowanego S.T.A.L.K.E.R. 2.",
            ),
        },
    ],
};

pub const OUTFITS: [&Outfit; 3] = [&BANDIT, &NOVICE, &LONER];

/// The outfit called `key`.
pub fn outfit(key: &str) -> Result<&'static Outfit> {
    OUTFITS
        .into_iter()
        .find(|o| o.key == key)
        .with_context(|| format!("no STK2 outfit {key}"))
}

// --- Fit settings ---

/// A jacket's sleeves over the cuffs of wrist-length gloves (the Novice
/// jacket's port).
const SLEEVES_OVER_CUFFS: LimbFit<'static> = LimbFit {
    axes: &[("LeftForeArm", "LeftHand"), ("RightForeArm", "RightHand")],
    layer: Layer::Outside,
    clearance: 0.004,
    ease: 0.3,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// Sleeves into gauntlets.
const SLEEVES_INTO_GAUNTLETS: LimbFit<'static> = LimbFit {
    axes: &[("LeftForeArm", "LeftHand"), ("RightForeArm", "RightHand")],
    layer: Layer::Inside,
    clearance: 0.003,
    ease: 0.5,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// A glove's cuff inside every wrist-length sleeve: along the arm from the
/// hand, so the sleeve's cuff is its opening.
const CUFFS_INTO_SLEEVES: LimbFit<'static> = LimbFit {
    axes: &[("LeftHand", "LeftForeArm"), ("RightHand", "RightForeArm")],
    layer: Layer::Inside,
    clearance: 0.003,
    ease: 0.5,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// Trousers over the shafts of low boots.
const HEMS_OVER_BOOTS: LimbFit<'static> = LimbFit {
    axes: &[("LeftLeg", "LeftFoot"), ("RightLeg", "RightFoot")],
    layer: Layer::Outside,
    clearance: 0.004,
    ease: 0.3,
    max_radius: 0.15,
    distal_reach: 0.0,
};
/// Trousers tucked into medium and tall boots.
const LEGS_INTO_BOOTS: LimbFit<'static> = LimbFit {
    axes: &[("LeftLeg", "LeftFoot"), ("RightLeg", "RightFoot")],
    layer: Layer::Inside,
    clearance: 0.003,
    ease: 0.5,
    max_radius: 0.15,
    distal_reach: 0.0,
};
/// A low boot's shaft inside every full-length trousers hem: along the leg
/// from the foot, so the hem is the trousers' opening. Below the hem the
/// bound widens fast: the foot is no trousers' business.
const BOOTS_INTO_HEMS: LimbFit<'static> = LimbFit {
    axes: &[("LeftFoot", "LeftLeg"), ("RightFoot", "RightLeg")],
    layer: Layer::Inside,
    clearance: 0.003,
    ease: 1.0,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// A medium or tall boot's shaft over every trousers leg tucked into it.
const BOOTS_OVER_LEGS: LimbFit<'static> = LimbFit {
    axes: &[("LeftLeg", "LeftFoot"), ("RightLeg", "RightFoot")],
    layer: Layer::Outside,
    clearance: 0.003,
    ease: 0.3,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// How far a sleeve or a trousers leg reaches into the accessory whose
/// opening is deepest along its axis (m); beyond, it is cut, as the stock
/// gauntlet and boot variants are.
const TUCK_OVERLAP: f64 = 0.03;
/// Room a collar leaves around the native neck and the masks' hems (the
/// Novice jacket's).
const COLLAR_CLEARANCE: f64 = 0.008;

/// A piece's model: its surfaces, each (surface name = its map's, the
/// converted model, its export properties).
#[derive(Clone)]
struct Model {
    surfaces: Vec<(String, Vec<u8>, BinaryConfig)>,
}

impl Model {
    /// The surfaces as one mesh (vertices in order, indices offset), to fit
    /// as one garment: a pouch moves with the cloth it hangs on.
    fn merged(&self) -> Result<(Vec<u8>, Vec<usize>)> {
        let mut vertices = vec![];
        let mut indices: Vec<u16> = vec![];
        let mut counts = vec![];
        for (_, b, _) in &self.surfaces {
            let n = read_u32(b, 8)?;
            let base = vertices.len() / 48;
            vertices.extend_from_slice(&b[12..12 + n * 48]);
            let at = 12 + n * 48;
            let ni = read_u32(b, at + 8)?;
            for k in 0..ni {
                let v = u16::from_le_bytes(b[at + 12 + k * 2..at + 14 + k * 2].try_into()?);
                indices.push(
                    u16::try_from(base + usize::from(v))
                        .context("merged surfaces exceed 65535 vertices")?,
                );
            }
            counts.push(n);
        }
        let mut out = vec![];
        out.extend_from_slice(&3_u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(4 + vertices.len())?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(vertices.len() / 48)?.to_le_bytes());
        out.extend_from_slice(&vertices);
        out.extend_from_slice(&4_u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(4 + indices.len() * 2)?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(indices.len())?.to_le_bytes());
        for i in indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        Ok((out, counts))
    }

    /// Takes the positions of a fitted [`Self::merged`] mesh back.
    fn unmerge(&mut self, merged: &[u8], counts: &[usize]) {
        let mut base = 0;
        for ((_, b, _), &n) in self.surfaces.iter_mut().zip(counts) {
            for i in 0..n {
                let from = 12 + (base + i) * 48;
                b[12 + i * 48..12 + i * 48 + 12].copy_from_slice(&merged[from..from + 12]);
            }
            base += n;
        }
    }

    /// Fits all surfaces as one garment with `fit`.
    fn fit_merged<T>(&mut self, fit: impl FnOnce(&mut Vec<u8>) -> Result<T>) -> Result<T> {
        let (mut merged, counts) = self.merged()?;
        let r = fit(&mut merged)?;
        self.unmerge(&merged, &counts);
        Ok(r)
    }

    fn surface_bytes(&self) -> Vec<(String, Vec<u8>)> {
        self.surfaces
            .iter()
            .map(|(n, b, _)| (n.clone(), b.clone()))
            .collect()
    }
}

/// One new piece while it is built: its outfit, spec and models by variant
/// suffix.
struct Built {
    outfit: &'static Outfit,
    piece: &'static Piece,
    models: BTreeMap<&'static str, Model>,
    hud: Option<Model>,
}

impl Built {
    fn label(&self) -> String {
        format!("{} {}", self.outfit.key, self.piece.name)
    }

    fn model_path(&self, suffix: &str) -> String {
        format!("ported/{}/{}{suffix}", self.outfit.ns, self.piece.name)
    }

    /// The config tables' class: the model_type a glove or boot is worn as.
    fn class(&self) -> Option<&'static str> {
        self.piece.model_type
    }
}

/// Labelled converted surfaces.
type Surfaces = Vec<(String, Vec<u8>)>;

/// The surfaces a stock (or released) armour config draws for `table`
/// (`data`, `data_full`, ...; `data` when the config has no such table).
fn config_surfaces(
    view: &mut ResourceDb,
    path: &str,
    table: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let config = parse_config(view, path, "equipment")?;
    let data = config
        .root
        .named(table)
        .or_else(|| config.root.named("data"))
        .with_context(|| format!("{path}: no data"))?;
    let mut parts = vec![data];
    if let Some(V::Indexed(extra)) = data.named("additional_models") {
        parts.extend(extra);
    }
    let mut out = vec![];
    for part in parts {
        let (Some(V::String(model)), Some(V::String(surface))) =
            (part.named("base_model"), part.named("part_name"))
        else {
            continue;
        };
        let file = format!("models/{model}.skinned_model/render/{surface}/converted_model");
        if view.is_file(&file) {
            out.push((format!("{path} {table} {surface}"), view.read(&file)?));
        }
    }
    Ok(out)
}

/// A config's `data.model_type` (`data_full` when it has none).
fn model_type(view: &mut ResourceDb, path: &str) -> Result<String> {
    let config = parse_config(view, path, "equipment")?;
    Ok(
        match config
            .root
            .named("data")
            .and_then(|d| d.named("model_type"))
        {
            Some(V::String(t)) => t.clone(),
            _ => "data_full".to_owned(),
        },
    )
}

/// The surfaces of the stock and released pieces of a category, drawn with
/// `table`: by their `data.model_type` with `by_type`, else all under
/// `table`.
fn stock(
    view: &mut ResourceDb,
    category: &str,
    table: &str,
    by_type: bool,
) -> Result<BTreeMap<String, Surfaces>> {
    let mut out: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    for path in crate::novice_026::category_configs(view, category) {
        let class = if by_type {
            model_type(view, &path)?
        } else {
            table.to_owned()
        };
        let surfaces = config_surfaces(view, &path, table)?;
        out.entry(class).or_default().extend(surfaces);
    }
    Ok(out)
}

/// Cuts the triangles of `garment` wholly beyond `TUCK_OVERLAP` past the
/// deepest opening of `accessories` along each side's (from, to) bone axis
/// (`side_axes`, e.g. ForeArm to Hand); returns the garment (`None` when
/// nothing of it is left) and the cut's distance from the first bone.
fn trim_tucked(
    garment: &[u8],
    accessories: &[(String, Vec<u8>)],
    bind_pose: &[u8],
    side_axes: [(&str, &str); 2],
) -> Result<(Option<Vec<u8>>, f64)> {
    let bones = bone_origins(bind_pose)?;
    let bone = |n: &str| {
        bones
            .get(n)
            .copied()
            .with_context(|| format!("no {n} bone"))
    };
    let mut axes = vec![];
    let mut deepest = 0.0_f64;
    for (from, to) in side_axes {
        let (a, b) = (bone(from)?, bone(to)?);
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        let dir = d.map(|v| v / len);
        let along = move |p: [f64; 3]| dot(sub(p, a), dir);
        for (label, model) in accessories {
            let mesh = Shell::parse(label.clone(), model)?.mesh;
            let opening = mesh
                .positions
                .iter()
                .filter(|p| p[0] * a[0] > 0.0)
                .map(|&p| along(p))
                .fold(f64::INFINITY, f64::min);
            if opening.is_finite() {
                deepest = deepest.max(opening);
            }
        }
        axes.push((a[0].signum(), along));
    }
    let cut = deepest + TUCK_OVERLAP;
    let keep = |corners: &[([f64; 3], [f64; 2]); 3]| {
        !axes.iter().any(|(side, along)| {
            corners
                .iter()
                .all(|(p, _)| p[0] * side > 0.0 && along(*p) > cut)
        })
    };
    let mesh = parse_mesh(garment, 48, "garment")?;
    let any = mesh.triangles.iter().any(|t| {
        let corners = t.map(|i| (mesh.positions[i], [0.0, 0.0]));
        keep(&corners)
    });
    Ok((
        if any {
            Some(keep_triangles(garment, keep)?)
        } else {
            None
        },
        cut,
    ))
}

/// The surfaces of `model` cut with [`trim_tucked`]; a surface left with no
/// triangle is dropped.
fn trim_model(
    model: &Model,
    accessories: &[(String, Vec<u8>)],
    bind_pose: &[u8],
    side_axes: [(&str, &str); 2],
) -> Result<(Model, f64)> {
    let mut out = Model { surfaces: vec![] };
    let mut cut = 0.0;
    for (name, bytes, props) in &model.surfaces {
        let (kept, at) = trim_tucked(bytes, accessories, bind_pose, side_axes)?;
        cut = at;
        if let Some(kept) = kept {
            out.surfaces.push((name.clone(), kept, props.clone()));
        }
    }
    ensure!(
        !out.surfaces.is_empty(),
        "nothing of the garment is left out of the accessories"
    );
    Ok((out, cut))
}

/// Reads an exported model's surfaces, turned onto the native rig.
fn read_model(
    source: &Path,
    outfit: &Outfit,
    model: &str,
    parts: &[(&str, &str)],
    bind: &[u8],
    inputs: &mut BTreeMap<String, String>,
) -> Result<Model> {
    let exported = source.join(EXPORTED).join(format!("{model}.skinned_model"));
    let mut donor = ResourceDb::open(&exported).with_context(|| exported.display().to_string())?;
    let bones = crate::duty_026::check_bind(&donor.read("render/bind_pose")?, bind)?;
    let platform = source.join(PLATFORM).join(format!("{model}.skinned_model"));
    let mut surfaces = vec![];
    for (surface, map) in parts {
        ensure!(
            outfit.maps.iter().any(|m| m.name == *map),
            "{}: no map {map}",
            outfit.key
        );
        let input = platform.join(format!("render/{surface}/converted_model"));
        let bytes = fs::read(&input).with_context(|| input.display().to_string())?;
        inputs.insert(
            input.display().to_string(),
            record_file("", "", &bytes).sha256,
        );
        let (mesh, ..) = crate::duty_026::rotate_mesh(&bytes, bones)?;
        let props = crate::duty_026::editor_config(&fs::read(
            platform.join(format!("render/{surface}/export_properties")),
        )?)?;
        ensure!(
            props.root.named("type") == Some(&V::Integer(47)),
            "{model} {surface}: not four-weight skinned"
        );
        ensure!(
            !surfaces
                .iter()
                .any(|(n, _, _): &(String, Vec<u8>, BinaryConfig)| n == map),
            "{model}: two surfaces on map {map}"
        );
        surfaces.push(((*map).to_owned(), mesh, props));
    }
    Ok(Model { surfaces })
}

/// A piece's surfaces from its parts, one per map: parts on one map are
/// joined. With `model`, every part
/// is read from that exported model instead (the gloves' `_hud` hands).
fn read_parts(
    source: &Path,
    outfit: &Outfit,
    parts: &[Part],
    model: Option<&str>,
    bind: &[u8],
    inputs: &mut BTreeMap<String, String>,
) -> Result<Model> {
    let mut out = Model { surfaces: vec![] };
    for part in parts {
        let from = model.unwrap_or(part.model);
        let mut read = read_model(
            source,
            outfit,
            from,
            &[(part.surface, part.map)],
            bind,
            inputs,
        )?;
        let (name, mut bytes, props) = read.surfaces.remove(0);
        if let Some(&(_, clean)) = STRAY_TROUSERS.iter().find(|(m, _)| *m == from) {
            let before = parse_mesh(&bytes, 48, "boots")?.triangles.len();
            bytes = mirror_side(&bytes, clean, bind)?;
            let after = parse_mesh(&bytes, 48, "boots")?.triangles.len();
            println!("{from}: the boot with a trousers leg in it replaced by the other boot mirrored ({before} -> {after} faces)");
        }
        match out.surfaces.iter_mut().find(|(n, _, _)| *n == name) {
            Some((_, joined, _)) => {
                let both = Model {
                    surfaces: vec![
                        (name.clone(), joined.clone(), props.clone()),
                        (name.clone(), bytes, props),
                    ],
                };
                *joined = both.merged()?.0;
            }
            None => out.surfaces.push((name, bytes, props)),
        }
    }
    Ok(out)
}

/// Exported boots with a piece of the trousers in them: the loner's left
/// boot (native -x) carries the bottom of the camouflage pants' leg (a
/// wrapped cuff up to 29 cm, on the pants' part of the map), cut from the
/// legs with the boot; the right boot (+x, the sign given) is the plain low
/// shoe. The bad boot is replaced by the clean one mirrored, so it ends at
/// the same finished collar.
const STRAY_TROUSERS: &[(&str, f64)] = &[("neutral/neutral_03/neutral_03_boot", 1.0)];

/// `bytes` (48-byte vertices) with its faces on the side opposite to
/// `clean` (the sign of x) replaced by the `clean` side's faces mirrored
/// across x = 0: positions and the basis' x flipped, each bone swapped for
/// its Left/Right twin (by the bind pose's names), the winding reversed.
fn mirror_side(bytes: &[u8], clean: f64, bind_pose: &[u8]) -> Result<Vec<u8>> {
    let names: Vec<String> = crate::armor::bind_pose_origins(bind_pose, "bind pose")?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let twin: Vec<u8> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let other = if let Some(r) = n.strip_prefix("Left") {
                format!("Right{r}")
            } else if let Some(r) = n.strip_prefix("Right") {
                format!("Left{r}")
            } else {
                n.clone()
            };
            u8::try_from(names.iter().position(|m| *m == other).unwrap_or(i)).unwrap_or(u8::MAX)
        })
        .collect();
    let mesh = parse_mesh(bytes, 48, "boots")?;
    let centre = |t: &[usize; 3]| t.iter().map(|&i| mesh.positions[i][0]).sum::<f64>() / 3.0;
    let kept: Vec<[usize; 3]> = mesh.triangles.iter().copied().filter(|t| centre(t) * clean > 0.0).collect();
    let used: BTreeSet<usize> = kept.iter().flatten().copied().collect();
    let n = mesh.positions.len();
    let mut vertices: Vec<Vec<u8>> = vec![];
    let mut index = BTreeMap::new();
    for &v in &used {
        let mut vertex = bytes[12 + v * 48..12 + (v + 1) * 48].to_vec();
        let x = -f32::from_le_bytes(vertex[0..4].try_into()?);
        vertex[0..4].copy_from_slice(&x.to_le_bytes());
        for k in 0..4 {
            vertex[12 + k] = *twin.get(usize::from(vertex[12 + k])).context("bone outside the bind pose")?;
        }
        // Packed basis bytes are B, G, R, A (Z, Y, X, auxiliary).
        for offset in [28, 32, 36] {
            vertex[offset + 2] = 255 - vertex[offset + 2];
        }
        index.insert(v, n + vertices.len());
        vertices.push(vertex);
    }
    let mirrored: Vec<[usize; 3]> = kept.iter().map(|t| [index[&t[0]], index[&t[2]], index[&t[1]]]).collect();
    // The clean side and its mirror only: the bad side's faces go.
    let with = append(bytes, &vertices, &mirrored)?;
    keep_triangles(&with, |corners| {
        let x = corners.iter().map(|(p, _)| p[0]).sum::<f64>() / 3.0;
        // Mirrored faces sit on the bad side; tell them by their vertices'
        // place in the buffer is not possible here, so keep every face of
        // the clean side and every face of the bad side that is a mirror.
        x * clean > 0.0 || mirrored_positions(&mesh, &kept, corners)
    })
}

/// Whether `corners` are a mirror image of one of `kept`'s faces.
fn mirrored_positions(mesh: &Mesh, kept: &[[usize; 3]], corners: &[([f64; 3], [f64; 2]); 3]) -> bool {
    let c: [f64; 3] = std::array::from_fn(|k| corners.iter().map(|(p, _)| p[k]).sum::<f64>() / 3.0);
    kept.iter().any(|t| {
        let o: [f64; 3] = std::array::from_fn(|k| t.iter().map(|&i| mesh.positions[i][k]).sum::<f64>() / 3.0);
        (o[0] + c[0]).abs() < 1e-5 && (o[1] - c[1]).abs() < 1e-5 && (o[2] - c[2]).abs() < 1e-5
    })
}

/// The native body's forearms (their shape: the stock gloves' cuffs are made
/// for them), drawn with the native hands' skin: the native torso's own map
/// is its sweater (red, a Christmas pattern), which showed under the
/// bomber's pushed-up sleeves (the owner's test, 2026-10-31).
const NATIVE_ARMS: &str = "forearms";
const NATIVE_ARMS_MATERIAL: &str = "characters/human/base_character/base_character_hand";
const NATIVE_ARMS_TEXTURE: &str = "character/human/base_character/base_character_hand";
/// The plain skin of the native hands' map the forearms are mapped to (u, v
/// from and to): the back of the hand, above the knuckles, no veins or
/// nails. Along the arm v runs down it; round the arm u runs across it and
/// back (mirrored, so it has no seam).
const NATIVE_ARMS_SKIN: [f64; 4] = [0.36, 0.10, 0.62, 0.40];
/// How far above the elbow (the ForeArm bone, along the forearm) the native
/// forearms start (m): inside the pushed-up sleeve.
const NATIVE_ARMS_FROM: f64 = 0.05;

/// Sets a 48-byte vertex's UV: the floats and their sub-texel bytes.
fn set_uv(vertex: &mut [u8], uv: [f64; 2]) {
    for (c, value) in uv.iter().enumerate() {
        let value = *value as f32;
        vertex[40 + c * 4..44 + c * 4].copy_from_slice(&value.to_le_bytes());
        let rebased = f64::from(value) * 1024.0;
        vertex[35 + c * 4] = (255.5 * (rebased - rebased.floor())).floor().clamp(0.0, 255.0) as u8;
    }
}

/// The native body's forearms of the torso variant `suffix` ("" `_s1`,
/// `_s2`, `_s3`: the arms the stock gloves of each class are made for),
/// from [`NATIVE_ARMS_FROM`] to the wrist, as a surface.
fn native_forearms(
    view: &mut ResourceDb,
    suffix: &str,
    bones: &BTreeMap<String, [f64; 3]>,
) -> Result<(String, Vec<u8>, BinaryConfig)> {
    let variant = match suffix {
        "" => "s1",
        "_s2" => "s2",
        "_s3" => "s3",
        other => bail!("no native torso for variant {other}"),
    };
    let root = format!("models/character/human/base/base_character_torso_{variant}.skinned_model/render/base_character_torso_s");
    let body = view.read(&format!("{root}/converted_model"))?;
    let props = BinaryConfig::parse(&view.read(&format!("{root}/export_properties"))?)?;
    let mut axes = vec![];
    for side in ["Left", "Right"] {
        let bone = |n: String| bones.get(&n).copied().with_context(|| format!("no {n} bone"));
        let (a, b) = (bone(format!("{side}ForeArm"))?, bone(format!("{side}Hand"))?);
        let d = sub(b, a);
        let len = dot(d, d).sqrt();
        axes.push((a[0].signum(), a, d.map(|v| v / len), len));
    }
    let mut arms = keep_triangles(&body, |corners| {
        axes.iter().any(|&(side, a, dir, _)| {
            corners.iter().all(|(p, _)| p[0] * side > 0.0 && dot(sub(*p, a), dir) > NATIVE_ARMS_FROM)
        })
    })?;
    let [u0, v0, u1, v1] = NATIVE_ARMS_SKIN;
    let count = read_u32(&arms, 8)? as usize;
    for i in 0..count {
        let o = 12 + i * 48;
        let p: [f64; 3] = std::array::from_fn(|k| f64::from(f32::from_le_bytes(arms[o + k * 4..o + k * 4 + 4].try_into().unwrap())));
        let &(_, a, dir, len) = axes
            .iter()
            .find(|(side, ..)| p[0] * side > 0.0)
            .context("a forearm vertex on neither side")?;
        let r = sub(p, a);
        let along = (dot(r, dir) / len).clamp(0.0, 1.0);
        // Round the arm: from the top (world up, square to the arm) to the
        // bottom, either way.
        let up = {
            let u = sub([0.0, 1.0, 0.0], dir.map(|d| d * dir[1]));
            let n = dot(u, u).sqrt();
            u.map(|c| c / n)
        };
        let across = [dir[1] * up[2] - dir[2] * up[1], dir[2] * up[0] - dir[0] * up[2], dir[0] * up[1] - dir[1] * up[0]];
        let round = dot(r, across).atan2(dot(r, up)).abs() / std::f64::consts::PI;
        set_uv(&mut arms[o..o + 48], [u0 + round * (u1 - u0), v0 + along * (v1 - v0)]);
    }
    let mut props = props;
    props.root.set_path("source.texture_name", V::String(NATIVE_ARMS_TEXTURE.to_owned()))?;
    Ok((NATIVE_ARMS.to_owned(), arms, props))
}

/// The native legs, whose top every stock torso covers (it is drawn under
/// any of them when no trousers are worn).
const BASE_LEGS: &str =
    "models/character/human/base/base_character_legs_s1.skinned_model/render/base_character_legs_s/converted_model";
/// A trousers top lower than this (m) shows under the shortest stock jackets
/// (the Leather sleeveless jacket ends at 0.956); stock trousers end at 0.988
/// to 1.04, the native legs at 1.035 to 1.043.
const WAIST_TOP_MIN: f64 = 0.985;

/// The boundary edges of a converted surface, each as its triangle runs it
/// (corners welded by position).
fn boundary(mesh: &Mesh) -> Vec<(usize, usize)> {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e4).round() as i64);
    // Each position's first vertex stands for it.
    let mut first: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let w: Vec<usize> = (0..mesh.positions.len())
        .map(|i| *first.entry(key(mesh.positions[i])).or_insert(i))
        .collect();
    let mut count: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for t in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (w[t[k]], w[t[(k + 1) % 3]]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut out = vec![];
    for t in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if count[&(w[a].min(w[b]), w[a].max(w[b]))] == 1 {
                out.push((a, b));
            }
        }
    }
    out
}

/// A vertex's bones and weights (48-byte layout: four bone bytes at 12,
/// three weights at 16, the fourth implied), as bone -> weight.
fn skin_of(vertex: &[u8]) -> BTreeMap<u8, f64> {
    let mut out = BTreeMap::new();
    let mut rest = 1.0;
    for k in 0..4 {
        let w = if k < 3 {
            let w = f64::from(f32::from_le_bytes(vertex[16 + k * 4..20 + k * 4].try_into().unwrap()));
            rest -= w;
            w
        } else {
            rest.max(0.0)
        };
        if w > 1e-6 {
            *out.entry(vertex[12 + k]).or_insert(0.0) += w;
        }
    }
    out
}

/// Writes `skin` (its four strongest bones, normalised) into a vertex.
fn set_skin(vertex: &mut [u8], skin: &BTreeMap<u8, f64>) {
    let mut strongest: Vec<(u8, f64)> = skin.iter().map(|(b, w)| (*b, *w)).collect();
    strongest.sort_by(|a, b| b.1.total_cmp(&a.1));
    strongest.truncate(4);
    let total: f64 = strongest.iter().map(|(_, w)| w).sum();
    while strongest.len() < 4 {
        strongest.push((strongest[0].0, 0.0));
    }
    for (k, (bone, w)) in strongest.iter().enumerate() {
        vertex[12 + k] = *bone;
        if k < 3 {
            vertex[16 + k * 4..20 + k * 4].copy_from_slice(&((w / total) as f32).to_le_bytes());
        }
    }
}

fn mix(a: &BTreeMap<u8, f64>, b: &BTreeMap<u8, f64>, t: f64) -> BTreeMap<u8, f64> {
    let mut out = BTreeMap::new();
    for (bone, w) in a {
        *out.entry(*bone).or_insert(0.0) += w * (1.0 - t);
    }
    for (bone, w) in b {
        *out.entry(*bone).or_insert(0.0) += w * t;
    }
    out
}

/// The linear map from a displacement on a triangle's plane to its UV
/// change, applied to `d` (its part along the triangle's normal dropped).
fn uv_change(p: [[f64; 3]; 3], uv: [[f64; 2]; 3], d: [f64; 3]) -> [f64; 2] {
    let (e1, e2) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let (a11, a12, a22) = (dot(e1, e1), dot(e1, e2), dot(e2, e2));
    let det = a11 * a22 - a12 * a12;
    if det.abs() < 1e-14 {
        return [0.0; 2];
    }
    let (b1, b2) = (dot(d, e1), dot(d, e2));
    let s = (a22 * b1 - a12 * b2) / det;
    let t = (a11 * b2 - a12 * b1) / det;
    std::array::from_fn(|k| s * (uv[1][k] - uv[0][k]) + t * (uv[2][k] - uv[0][k]))
}

/// Grows a surface (48-byte vertices) past its boundary edges `edges` (as
/// their triangles run them) in `rows` rows: row k of edge vertex v at
/// `place(v, k)` with the skin `skin(v, k)`, its normal and tangents v's,
/// its UV the surface's own texture reflected across the edge (the UV the
/// displacement from v would have, taken back into the surface), so the
/// cloth runs on without a seam or a patch. Each new quad winds as the
/// edge's triangle (and, `both_sides`, the other way too). Returns the new
/// surface and the triangles added.
fn grow(
    bytes: &[u8],
    edges: &[(usize, usize)],
    rows: usize,
    place: impl Fn(usize, usize) -> [f64; 3],
    skin: impl Fn(usize, usize) -> BTreeMap<u8, f64>,
    both_sides: bool,
) -> Result<(Vec<u8>, usize, BTreeMap<(usize, usize), usize>)> {
    let mesh = parse_mesh(bytes, 48, "surface")?;
    let n = mesh.positions.len();
    let uv_at = |i: usize| -> [f64; 2] {
        [0, 4].map(|o| f64::from(f32::from_le_bytes(bytes[12 + i * 48 + 40 + o..12 + i * 48 + 44 + o].try_into().unwrap())))
    };
    // Each edge vertex's triangle: the one its boundary edge is on.
    let mut triangle_of: BTreeMap<usize, [usize; 3]> = BTreeMap::new();
    for &(a, b) in edges {
        let t = *mesh
            .triangles
            .iter()
            .find(|t| (0..3).any(|k| t[k] == a && t[(k + 1) % 3] == b))
            .context("a boundary edge on no triangle")?;
        triangle_of.entry(a).or_insert(t);
        triangle_of.entry(b).or_insert(t);
    }
    let mut vertices = bytes[12..12 + n * 48].to_vec();
    let mut index: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (&v, t) in &triangle_of {
        let p = t.map(|i| mesh.positions[i]);
        let uv = t.map(uv_at);
        for k in 1..=rows {
            let at = place(v, k);
            let back = uv_change(p, uv, sub(at, mesh.positions[v]));
            let mut vertex = bytes[12 + v * 48..12 + (v + 1) * 48].to_vec();
            for (c, value) in at.iter().enumerate() {
                vertex[c * 4..c * 4 + 4].copy_from_slice(&(*value as f32).to_le_bytes());
            }
            set_skin(&mut vertex, &skin(v, k));
            let own = uv_at(v);
            for c in 0..2 {
                let value = (own[c] - back[c]) as f32;
                vertex[40 + c * 4..44 + c * 4].copy_from_slice(&value.to_le_bytes());
                let rebased = f64::from(value) * 1024.0;
                vertex[35 + c * 4] = (255.5 * (rebased - rebased.floor())).floor().clamp(0.0, 255.0) as u8;
            }
            index.insert((v, k), n + vertices.len() / 48 - n);
            vertices.extend_from_slice(&vertex);
        }
    }
    let row = |v: usize, k: usize| if k == 0 { v } else { index[&(v, k)] };
    let mut triangles = mesh.triangles.clone();
    let mut added = 0;
    for &(a, b) in edges {
        for k in 0..rows {
            let quad = [[row(b, k), row(a, k), row(a, k + 1)], [row(b, k), row(a, k + 1), row(b, k + 1)]];
            for t in quad {
                triangles.push(t);
                added += 1;
                if both_sides {
                    triangles.push([t[0], t[2], t[1]]);
                    added += 1;
                }
            }
        }
    }
    let index_chunk = 12 + n * 48;
    let rest = index_chunk + 8 + read_u32(bytes, index_chunk + 4)?;
    let mut out = vec![];
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + vertices.len())?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(vertices.len() / 48)?.to_le_bytes());
    out.extend_from_slice(&vertices);
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + triangles.len() * 6)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(triangles.len() * 3)?.to_le_bytes());
    for t in &triangles {
        for &i in t {
            out.extend_from_slice(&u16::try_from(i)?.to_le_bytes());
        }
    }
    out.extend_from_slice(bytes.get(rest..).context("truncated surface")?);
    Ok((out, added, index))
}

/// A converted surface (48-byte vertices) with `vertices` and `triangles`
/// added.
fn append(bytes: &[u8], vertices: &[Vec<u8>], triangles: &[[usize; 3]]) -> Result<Vec<u8>> {
    let mesh = parse_mesh(bytes, 48, "surface")?;
    let n = mesh.positions.len();
    let index_chunk = 12 + n * 48;
    let rest = index_chunk + 8 + read_u32(bytes, index_chunk + 4)?;
    let all: Vec<[usize; 3]> = mesh.triangles.iter().copied().chain(triangles.iter().copied()).collect();
    let count = n + vertices.len();
    let mut out = vec![];
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + count * 48)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(count)?.to_le_bytes());
    out.extend_from_slice(&bytes[12..index_chunk]);
    for v in vertices {
        out.extend_from_slice(v);
    }
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + all.len() * 6)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(all.len() * 3)?.to_le_bytes());
    for t in &all {
        for &i in t {
            out.extend_from_slice(&u16::try_from(i)?.to_le_bytes());
        }
    }
    out.extend_from_slice(bytes.get(rest..).context("truncated surface")?);
    Ok(out)
}

/// The height a raised waist reaches (m): the native legs' top ring, which
/// every stock torso covers (it is drawn under any of them when no trousers
/// are worn).
const WAIST_TOP: f64 = 1.04;
/// Where the waistband starts (m): the band below is the trousers' own
/// denim, reflected up; the 4 cm above it the painted waistband.
const WAISTBAND_FROM: f64 = 1.0;
/// The strip of the (cut, 1024-texel) map the waistband is painted on and
/// mapped to: (u, v) from and to; free in the Novice legs map (its
/// top-right, above the jeans, beside the boots).
const WAISTBAND_STRIP: [f64; 4] = [640.0 / 1024.0, 396.0 / 1024.0, 1.0, 430.0 / 1024.0];
/// Plain denim the waistband is cut from (texels of the 1024 map: x, y,
/// width, height): the jeans' calves, the least creased.
const WAISTBAND_DENIM: [u32; 4] = [40, 860, 384, 34];
const WAIST_ROWS: usize = 3;

/// Raises a trousers' waist whose top is under [`WAIST_TOP_MIN`] (the
/// Novice jeans end at 0.937-0.964 where STK2's long jacket covered them;
/// 0.26g's shorter jackets showed the open top, and the background through
/// it): the top edge grows straight up to [`WAIST_TOP`] at the trousers' own
/// width, its denim reflected up from below the edge, its skin blending into
/// the native legs' top ring's. Returns the triangles added and the old top,
/// or `None`.
fn close_waist(model: &mut Model, base_legs: &[u8]) -> Result<Option<(usize, f64)>> {
    let (_, bytes, _) = &model.surfaces[0];
    let mesh = parse_mesh(bytes, 48, "trousers")?;
    let top: Vec<(usize, usize)> = boundary(&mesh).into_iter().filter(|&(a, _)| mesh.positions[a][1] > 0.8).collect();
    ensure!(!top.is_empty(), "trousers without an open top");
    let highest = top.iter().map(|&(a, _)| mesh.positions[a][1]).fold(f64::NEG_INFINITY, f64::max);
    if highest >= WAIST_TOP_MIN {
        return Ok(None);
    }
    ensure!((read_u32(base_legs, 4)? - 4) / read_u32(base_legs, 8)? == 48, "native legs of another vertex layout");
    let base = parse_mesh(base_legs, 48, "native legs")?;
    let ring: Vec<usize> = {
        let mut r: Vec<usize> = boundary(&base).into_iter().map(|(a, _)| a).filter(|&a| base.positions[a][1] > 0.9).collect();
        r.sort_unstable();
        r.dedup();
        r
    };
    ensure!(ring.len() >= 8, "the native legs have no top ring");
    let nearest = |p: [f64; 3]| -> usize {
        *ring
            .iter()
            .min_by(|&&a, &&b| {
                let d = |i: usize| (base.positions[i][0] - p[0]).hypot(base.positions[i][2] - p[2]);
                d(a).total_cmp(&d(b))
            })
            .unwrap()
    };
    let positions = mesh.positions.clone();
    let place = |v: usize, k: usize| {
        let p = positions[v];
        [p[0], p[1] + (WAISTBAND_FROM - p[1]) * k as f64 / WAIST_ROWS as f64, p[2]]
    };
    let skin = |v: usize, k: usize| {
        let own = skin_of(&bytes[12 + v * 48..12 + (v + 1) * 48]);
        let near = nearest(positions[v]);
        let native = skin_of(&base_legs[12 + near * 48..12 + (near + 1) * 48]);
        mix(&own, &native, k as f64 / WAIST_ROWS as f64)
    };
    let (grown, mut added, index) = grow(bytes, &top, WAIST_ROWS, place, skin, false)?;
    // The waistband: its own vertices over the grown top edge (a seam, so
    // the denim below keeps its UVs), up to WAIST_TOP, on the painted strip;
    // its skin the native ring's.
    let row = |v: usize| index[&(v, WAIST_ROWS)];
    let g = parse_mesh(&grown, 48, "trousers")?;
    let n = g.positions.len();
    let mut vertices: Vec<Vec<u8>> = vec![];
    let mut at: BTreeMap<(usize, bool), usize> = BTreeMap::new();
    let [u0, v0, u1, v1] = WAISTBAND_STRIP;
    for &(a, b) in &top {
        for v in [a, b] {
            for upper in [false, true] {
                if at.contains_key(&(v, upper)) {
                    continue;
                }
                let src = row(v);
                let mut vertex = grown[12 + src * 48..12 + (src + 1) * 48].to_vec();
                if upper {
                    vertex[4..8].copy_from_slice(&(WAIST_TOP as f32).to_le_bytes());
                }
                let near = nearest(positions[v]);
                set_skin(&mut vertex, &skin_of(&base_legs[12 + near * 48..12 + (near + 1) * 48]));
                let u = f64::from(f32::from_le_bytes(bytes[12 + v * 48 + 40..12 + v * 48 + 44].try_into()?));
                let uv = [u0 + u.clamp(0.0, 1.0) * (u1 - u0), if upper { v0 } else { v1 }];
                for (c, value) in uv.iter().enumerate() {
                    let value = *value as f32;
                    vertex[40 + c * 4..44 + c * 4].copy_from_slice(&value.to_le_bytes());
                    let rebased = f64::from(value) * 1024.0;
                    vertex[35 + c * 4] = (255.5 * (rebased - rebased.floor())).floor().clamp(0.0, 255.0) as u8;
                }
                at.insert((v, upper), n + vertices.len());
                vertices.push(vertex);
            }
        }
    }
    let triangles: Vec<[usize; 3]> = top
        .iter()
        .flat_map(|&(a, b)| {
            let (la, lb, ua, ub) = (at[&(a, false)], at[&(b, false)], at[&(a, true)], at[&(b, true)]);
            [[lb, la, ua], [lb, ua, ub]]
        })
        .collect();
    added += triangles.len();
    model.surfaces[0].1 = append(&grown, &vertices, &triangles)?;
    Ok(Some((added, highest)))
}

/// Paints the waistband into an uncompressed true-colour TGA (1024 square):
/// the strip [`WAISTBAND_STRIP`] filled with [`WAISTBAND_DENIM`] stretched
/// along it, a shade lighter than the jeans as a folded band catches the
/// light, a stitched seam along each edge (a row of lighter dashes, 3
/// texels in) and the belt line under it (its last 2 rows in shadow); on the
/// normal map the denim's own normals and the seams a little raised.
fn paint_waistband(path: &Path, normal: bool) -> Result<()> {
    let mut b = fs::read(path)?;
    ensure!(b.len() > 18 && b[2] == 2, "{}: not an uncompressed true-colour TGA", path.display());
    let id = usize::from(b[0]);
    let w = usize::from(u16::from_le_bytes([b[12], b[13]]));
    let h = usize::from(u16::from_le_bytes([b[14], b[15]]));
    let bpp = usize::from(b[16]) / 8;
    ensure!(w == 1024 && h == 1024 && bpp >= 3, "{}: {w}x{h}x{bpp}, not the 1024 map", path.display());
    let top_down = b[17] & 0x20 != 0;
    let at = |x: usize, y: usize| 18 + id + ((if top_down { y } else { h - 1 - y }) * w + x) * bpp;
    let [u0, v0, u1, v1] = WAISTBAND_STRIP;
    let (x0, y0, x1, y1) = ((u0 * 1024.0) as usize, (v0 * 1024.0) as usize, (u1 * 1024.0) as usize, (v1 * 1024.0) as usize);
    let [sx, sy, sw, sh] = WAISTBAND_DENIM.map(|v| v as usize);
    let source: Vec<Vec<[u8; 3]>> = (0..sh)
        .map(|j| (0..sw).map(|i| { let a = at(sx + i, sy + j); [b[a], b[a + 1], b[a + 2]] }).collect())
        .collect();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (x - x0) * sw / (x1 - x0);
            let j = (y - y0) * sh / (y1 - y0);
            let mut px = source[j][i];
            let edge = (y - y0 == 3 || y1 - 1 - y == 5) && (x - x0) % 6 < 4;
            let belt_line = y1 - y <= 2;
            if normal {
                if edge {
                    px = [255, 128, 128]; // BGR: a raised dash
                }
            } else if belt_line {
                px = px.map(|c| (u32::from(c) / 2) as u8);
            } else if edge {
                px = px.map(|c| (u32::from(c) * 2).min(255) as u8);
            } else {
                px = px.map(|c| (u32::from(c) * 13 / 10).min(255) as u8);
            }
            let a = at(x, y);
            b[a..a + 3].copy_from_slice(&px);
        }
    }
    fs::write(path, b)?;
    Ok(())
}

/// The stock Scavenger hood (892): how far down and how wide a hood's cowl
/// comes over the neck and shoulders, and its skin there.
const SCAVENGER_HOOD: &str =
    "models/character/human/scavengers/scavengers_03_helmet.skinned_model/render/scavengers_03_helmet_s/converted_model";
/// Below this the Scavenger hood is its cowl (m).
const COWL_TOP: f64 = 1.58;
const COWL_ROWS: usize = 4;
/// Room the grown cowl leaves over the native neck (m).
const COWL_CLEARANCE: f64 = 0.006;

/// Where the cowl ends (m): at the collar line round the neck, a little
/// lower in front (the Scavenger hood's comes to a point there).
const COWL_BOTTOM: f64 = 1.47;
const COWL_FRONT_DROP: f64 = 0.03;
/// Segments of the panel that closes the grown cowl across the throat.
const BRIDGE_SEGMENTS: usize = 8;

/// The Novice hood (48-byte vertices) grown down over the neck and the
/// shoulders, which STK2's collar covered. Its shell's bottom hem (the
/// open edge whose normals face out; the lining's edge stays inside) grows
/// in [`COWL_ROWS`] rows to the Scavenger hood's hem at its angle around the
/// neck (the lowest point of its cowl there), kept [`COWL_CLEARANCE`] out of
/// the native neck; the two flap tips are then joined across the throat, as
/// the Scavenger hood's cowl closes there, with a seam down the middle.
/// Every new vertex takes the hood's own canvas reflected across the edge it
/// grows from (no patch and no break at the join), and its skin blends from
/// the hood's into the Scavenger hood's hem's. Returns the new hood and the
/// triangles added.
fn grow_cowl(hood: &[u8], scavenger: &[u8], head: &[u8]) -> Result<(Vec<u8>, usize)> {
    use crate::mask_fit::farthest_hit;
    ensure!((read_u32(scavenger, 4)? - 4) / read_u32(scavenger, 8)? == 48, "the Scavenger hood has another vertex layout");
    let mesh = parse_mesh(hood, 48, "hood")?;
    let neck = parse_mesh(head, 40, "native head")?;
    let stock = parse_mesh(scavenger, 48, "Scavenger hood")?;
    let all_edges = boundary(&mesh);
    ensure!(!all_edges.is_empty(), "the hood has no open edge");
    let centre_z = all_edges.iter().map(|&(a, _)| mesh.positions[a][2]).sum::<f64>() / all_edges.len() as f64;
    let angle = |p: [f64; 3]| p[0].atan2(p[2] - centre_z);
    let radial = |p: [f64; 3]| {
        let d = [p[0], 0.0, p[2] - centre_z];
        let l = dot(d, d).sqrt().max(1e-9);
        d.map(|c| c / l)
    };
    let normal = |i: usize| -> [f64; 3] {
        let b = &hood[12 + i * 48 + 28..12 + i * 48 + 31];
        [b[2], b[1], b[0]].map(|v| f64::from(v) / 127.5 - 1.0)
    };
    let faces_out = |i: usize| dot(normal(i), radial(mesh.positions[i])) > 0.2;
    let edges: Vec<(usize, usize)> = all_edges.into_iter().filter(|&(a, b)| faces_out(a) && faces_out(b)).collect();
    ensure!(!edges.is_empty(), "the hood's hem has no outward edge");
    // The hem's two ends (by position: the flap tips).
    let key = |p: [f64; 3]| p.map(|v| (v * 1e4).round() as i64);
    let mut uses: BTreeMap<[i64; 3], (usize, usize)> = BTreeMap::new();
    for &(a, b) in &edges {
        for v in [a, b] {
            uses.entry(key(mesh.positions[v])).or_insert((v, 0)).1 += 1;
        }
    }
    let ends: Vec<usize> = uses.values().filter(|(_, n)| *n == 1).map(|(v, _)| *v).collect();
    ensure!(ends.len() == 2, "the hood's outer hem is not one open chain ({} ends)", ends.len());
    // The Scavenger hood's hem: its lowest cowl point in each sector.
    const SECTORS: usize = 24;
    let sector = |p: [f64; 3]| ((angle(p) + std::f64::consts::PI) / std::f64::consts::TAU * SECTORS as f64) as usize % SECTORS;
    let mut hem: Vec<Option<usize>> = vec![None; SECTORS];
    for (i, &p) in stock.positions.iter().enumerate() {
        if p[1] < COWL_TOP {
            let k = sector(p);
            if hem[k].is_none_or(|j| p[1] < stock.positions[j][1]) {
                hem[k] = Some(i);
            }
        }
    }
    let hem_at = |t: f64| -> Result<usize> {
        let p = [t.sin(), 0.0, centre_z + t.cos()];
        let k = sector(p);
        (0..SECTORS)
            .flat_map(|d| [hem[(k + d) % SECTORS], hem[(k + SECTORS - d) % SECTORS]])
            .flatten()
            .next()
            .context("the Scavenger hood has no cowl")
    };
    // Out of the native neck, horizontally from the axis.
    let clear = |mut x: [f64; 3]| {
        let dir = radial(x);
        let origin = [0.0, x[1], centre_z];
        if let Some((t, _, _)) = farthest_hit(origin, dir, &neck.positions, &neck.triangles) {
            if dot(sub(x, origin), dir) < t + COWL_CLEARANCE {
                x[0] = dir[0] * (t + COWL_CLEARANCE);
                x[2] = centre_z + dir[2] * (t + COWL_CLEARANCE);
            }
        }
        x
    };
    let mut targets: BTreeMap<usize, (usize, [f64; 3])> = BTreeMap::new();
    for &(a, b) in &edges {
        for v in [a, b] {
            let p = mesh.positions[v];
            let t = angle(p);
            let h = hem_at(t)?;
            let q = stock.positions[h];
            // Down round the neck to the collar line, hugging it (`clear`
            // keeps it out of the native neck): under every jacket's collar,
            // as the Scavenger hood's cowl is; its hem only lends the skin.
            let _ = q;
            let r = p[0].hypot(p[2] - centre_z);
            let front = t.cos().max(0.0);
            let y = (COWL_BOTTOM - COWL_FRONT_DROP * front).min(p[1] - 0.02);
            targets.entry(v).or_insert((h, [t.sin() * r * 0.85, y, centre_z + t.cos() * r * 0.85]));
        }
    }
    let positions = mesh.positions.clone();
    let place = |v: usize, k: usize| {
        let (p, q) = (positions[v], targets[&v].1);
        let s = k as f64 / COWL_ROWS as f64;
        clear(std::array::from_fn(|c| p[c] + s * (q[c] - p[c])))
    };
    let skin_at = |v: usize, k: usize| {
        let own = skin_of(&hood[12 + v * 48..12 + (v + 1) * 48]);
        let h = targets[&v].0;
        mix(&own, &skin_of(&scavenger[12 + h * 48..12 + (h + 1) * 48]), k as f64 / COWL_ROWS as f64)
    };
    let (grown, mut added, index) = grow(hood, &edges, COWL_ROWS, place, skin_at, false)?;
    // --- The throat: a panel from one flap tip's column to the other's ---
    let row = |v: usize, k: usize| if k == 0 { v } else { index[&(v, k)] };
    let g = parse_mesh(&grown, 48, "grown hood")?;
    let uv_of = |i: usize| -> [f64; 2] {
        [0, 4].map(|o| f64::from(f32::from_le_bytes(grown[12 + i * 48 + 40 + o..12 + i * 48 + 44 + o].try_into().unwrap())))
    };
    let tri_of = |v: usize| -> Result<[usize; 3]> {
        mesh.triangles
            .iter()
            .find(|t| t.contains(&v))
            .copied()
            .context("a flap tip on no triangle")
    };
    let (mut l, mut r) = (ends[0], ends[1]);
    if angle(mesh.positions[l]) > angle(mesh.positions[r]) {
        std::mem::swap(&mut l, &mut r);
    }
    ensure!(angle(mesh.positions[l]) < 0.0 && angle(mesh.positions[r]) > 0.0, "the flap tips are not either side of the throat");
    let mut vertices: Vec<Vec<u8>> = vec![];
    let mut grid: BTreeMap<(usize, usize, bool), usize> = BTreeMap::new();
    let half = BRIDGE_SEGMENTS / 2;
    for k in 0..=COWL_ROWS {
        let (pl, pr) = (g.positions[row(l, k)], g.positions[row(r, k)]);
        let (tl, tr) = (angle(pl), angle(pr));
        let (rl, rr) = (pl[0].hypot(pl[2] - centre_z), pr[0].hypot(pr[2] - centre_z));
        for j in 1..BRIDGE_SEGMENTS {
            let f = j as f64 / BRIDGE_SEGMENTS as f64;
            let (t, rad, y) = (tl + f * (tr - tl), rl + f * (rr - rl), pl[1] + f * (pr[1] - pl[1]));
            let at = clear([t.sin() * rad, y, centre_z + t.cos() * rad]);
            // The middle twice: each half takes its own flap's canvas.
            for side_left in [true, false] {
                if (side_left && j > half) || (!side_left && j < half) {
                    continue;
                }
                let tip = if side_left { l } else { r };
                let src = row(tip, k);
                let tri = tri_of(tip)?;
                let p3 = tri.map(|i| mesh.positions[i]);
                let uv3 = tri.map(|i| {
                    [0, 4].map(|o| f64::from(f32::from_le_bytes(hood[12 + i * 48 + 40 + o..12 + i * 48 + 44 + o].try_into().unwrap())))
                });
                let back = uv_change(p3, uv3, sub(at, mesh.positions[tip]));
                let own = uv_of(tip);
                let mut vertex = grown[12 + src * 48..12 + (src + 1) * 48].to_vec();
                for (c, value) in at.iter().enumerate() {
                    vertex[c * 4..c * 4 + 4].copy_from_slice(&(*value as f32).to_le_bytes());
                }
                let h = hem_at(t)?;
                set_skin(
                    &mut vertex,
                    &mix(
                        &skin_of(&grown[12 + src * 48..12 + (src + 1) * 48]),
                        &skin_of(&scavenger[12 + h * 48..12 + (h + 1) * 48]),
                        k as f64 / COWL_ROWS as f64,
                    ),
                );
                for c in 0..2 {
                    let value = (own[c] - back[c]) as f32;
                    vertex[40 + c * 4..44 + c * 4].copy_from_slice(&value.to_le_bytes());
                    let rebased = f64::from(value) * 1024.0;
                    vertex[35 + c * 4] = (255.5 * (rebased - rebased.floor())).floor().clamp(0.0, 255.0) as u8;
                }
                grid.insert((k, j, side_left), g.positions.len() + vertices.len());
                vertices.push(vertex);
            }
        }
    }
    let at = |k: usize, j: usize, left: bool| -> usize {
        if j == 0 {
            row(l, k)
        } else if j == BRIDGE_SEGMENTS {
            row(r, k)
        } else if j == half {
            grid[&(k, j, left)]
        } else {
            grid[&(k, j, j < half)]
        }
    };
    let mut triangles = vec![];
    for k in 0..COWL_ROWS {
        for j in 0..BRIDGE_SEGMENTS {
            let left = j < half;
            let (a, b, c, d) = (at(k, j, left), at(k, j + 1, left), at(k + 1, j + 1, left), at(k + 1, j, left));
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    // Facing out, as the shell does.
    let pos = |i: usize| if i < g.positions.len() { g.positions[i] } else {
        let v = &vertices[i - g.positions.len()];
        [0, 4, 8].map(|o| f64::from(f32::from_le_bytes(v[o..o + 4].try_into().unwrap())))
    };
    let t0 = triangles[0];
    let n0 = crate::mask_fit::cross(sub(pos(t0[1]), pos(t0[0])), sub(pos(t0[2]), pos(t0[0])));
    if dot(n0, radial(pos(t0[0]))) < 0.0 {
        for t in &mut triangles {
            t.swap(1, 2);
        }
    }
    added += triangles.len();
    Ok((append(&grown, &vertices, &triangles)?, added))
}

/// The lowest a full-length trousers hem should come over low boots (m):
/// the stock `data_full` trousers end 6 to 13 cm above the ground, and the
/// lowest stock low boot is 16.6 cm high.
const FULL_HEM: f64 = 0.12;

/// Lengthens trousers whose hem stands above [`FULL_HEM`] somewhere, for low
/// boots: below the knee (the Leg bone), each point is moved down by the
/// stretch its side and direction around the leg needs to bring the hem
/// there (the hem measured in 24 sectors around each leg, each the lowest
/// of itself and its neighbours). Only heights change. Returns the vertices
/// moved and the largest move (m).
fn lengthen_hems(model: &mut Model, bones: &BTreeMap<String, [f64; 3]>) -> Result<(usize, f64)> {
    const SECTORS: usize = 24;
    let mut legs = vec![];
    for side in ["Left", "Right"] {
        let bone = |n: String| {
            bones
                .get(&n)
                .copied()
                .with_context(|| format!("no {n} bone"))
        };
        legs.push((bone(format!("{side}Leg"))?, bone(format!("{side}Foot"))?));
    }
    let (mut moved, mut most) = (0, 0.0_f64);
    model.fit_merged(|g| {
        let mesh = parse_mesh(g, 48, "trousers")?;
        for &(knee, foot) in &legs {
            let side = knee[0].signum();
            // The leg's axis at a height, and the sector of a point around it.
            let centre = |y: f64| {
                let t = ((knee[1] - y) / (knee[1] - foot[1])).clamp(0.0, 1.5);
                [
                    knee[0] + t * (foot[0] - knee[0]),
                    knee[2] + t * (foot[2] - knee[2]),
                ]
            };
            let sector = |p: [f64; 3]| {
                let c = centre(p[1]);
                let angle = (p[2] - c[1]).atan2(p[0] - c[0]);
                ((angle + std::f64::consts::PI) / std::f64::consts::TAU * SECTORS as f64) as usize
                    % SECTORS
            };
            let on_leg = |p: [f64; 3]| p[0] * side > 0.0 && p[1] < knee[1];
            let mut hem = [f64::INFINITY; SECTORS];
            for &p in mesh.positions.iter().filter(|p| on_leg(**p)) {
                let k = sector(p);
                hem[k] = hem[k].min(p[1]);
            }
            let low: Vec<f64> = (0..SECTORS)
                .map(|k| {
                    hem[(k + SECTORS - 1) % SECTORS]
                        .min(hem[k])
                        .min(hem[(k + 1) % SECTORS])
                })
                .collect();
            for (i, &p) in mesh.positions.iter().enumerate() {
                if !on_leg(p) {
                    continue;
                }
                let h = low[sector(p)];
                if !h.is_finite() || h <= FULL_HEM {
                    continue;
                }
                let y = knee[1] - (knee[1] - p[1]) * (knee[1] - FULL_HEM) / (knee[1] - h);
                let at = 12 + i * 48 + 4;
                g[at..at + 4].copy_from_slice(&(y as f32).to_le_bytes());
                if p[1] - y > 1e-5 {
                    moved += 1;
                    most = most.max(p[1] - y);
                }
            }
        }
        Ok(())
    })?;
    Ok((moved, most))
}

fn print_limb(label: &str, what: &str, n: usize, r: &crate::limb_fit::LimbFitReport) {
    println!(
        "{label}: {what} {n} surfaces: moved {} vertices, up to {:.1} mm",
        r.moved,
        r.max_move * 1000.0
    );
}

/// Everything the three outfits are fitted against.
struct Fitting {
    view: ResourceDb,
    bind: Vec<u8>,
    bones: BTreeMap<String, [f64; 3]>,
    head: Vec<u8>,
    masks: Vec<Shell>,
}

/// Builds the bundle of every outfit in `outfits` under `output/<key>`,
/// fitted against the released game (`db` with `patches`) and the three
/// outfits.
pub fn build(
    source: &Path,
    textures: &Path,
    db: &Path,
    patches: &[PathBuf],
    output: &Path,
    outfits: &[&str],
) -> Result<()> {
    let mut view = crate::novice_026::game_view(db, patches)?;
    let bind = view.read(&format!("{BASE}/render/bind_pose"))?;
    ensure!(
        bind == view.read(HEAD_BIND_POSE)?,
        "the native head and boot use different bind poses"
    );
    let bones = bone_origins(&bind)?;
    let head = view.read(crate::bandit_026::HEAD_MODEL)?;
    let mut masks = vec![];
    for path in crate::novice_026::category_configs(&view, "mask") {
        for (label, model, _) in crate::novice_026::item_surfaces(&mut view, &path)? {
            masks.push(Shell::parse(label, &model)?);
        }
    }
    let mut cx = Fitting {
        view,
        bind,
        bones,
        head,
        masks,
    };
    let mut inputs = BTreeMap::new();

    // --- Read every piece, turned onto the native rig ---
    let mut built: Vec<Built> = vec![];
    for outfit in OUTFITS {
        let ids: BTreeSet<i32> = outfit.pieces.iter().map(|p| p.id).collect();
        ensure!(
            ids.len() == outfit.pieces.len(),
            "{}: an id twice",
            outfit.key
        );
        for piece in outfit.pieces {
            let read = read_parts(
                source,
                outfit,
                piece.parts,
                None,
                &cx.bind,
                &mut inputs,
            )?;
            let hud = piece
                .hud
                .map(|hud| {
                    read_parts(
                        source,
                        outfit,
                        piece.parts,
                        Some(hud),
                        &cx.bind,
                        &mut inputs,
                    )
                })
                .transpose()?;
            if let Some(hud) = &hud {
                ensure!(
                    hud.surfaces
                        .iter()
                        .zip(&read.surfaces)
                        .all(|(a, b)| a.1 == b.1),
                    "{} {}: the first-person hands are not the third-person ones",
                    outfit.key,
                    piece.name
                );
            }
            built.push(Built {
                outfit,
                piece,
                models: BTreeMap::from([("", read)]),
                hud,
            });
        }
    }

    // --- Maps: each cut to the window its surfaces use ---
    let work = output.join("textures-work");
    fs::create_dir_all(&work)?;
    let mut stems: BTreeMap<(&str, &str), PathBuf> = BTreeMap::new();
    for outfit in OUTFITS {
        for map in outfit.maps {
            let stem = textures.join(TEXTURES).join(map.stem);
            let diffuse = stem.with_extension("tga");
            let size = tga_size(&diffuse)?;
            // The window every surface on this map uses, in all pieces.
            let users: Vec<&Vec<u8>> = built
                .iter()
                .filter(|b| std::ptr::eq(b.outfit, outfit))
                .flat_map(|b| {
                    b.models[""]
                        .surfaces
                        .iter()
                        .filter(|(n, _, _)| n == map.name)
                        .map(|(_, m, _)| m)
                })
                .collect();
            ensure!(!users.is_empty(), "{}: map {} unused", outfit.key, map.name);
            let mut window: Option<[u32; 4]> = None;
            for m in &users {
                let w = uv_crop(m, size)?.unwrap_or([0, 0, size[0], size[1]]);
                window = Some(match window {
                    None => w,
                    Some(o) => {
                        // The power-of-two window holding both.
                        let lo = [o[0].min(w[0]), o[1].min(w[1])];
                        let hi = [
                            (o[0] + o[2]).max(w[0] + w[2]),
                            (o[1] + o[3]).max(w[1] + w[3]),
                        ];
                        let mut out = [0; 4];
                        for k in 0..2 {
                            let mut span = (hi[k] - lo[k]).next_power_of_two();
                            loop {
                                let start = lo[k] / span * span;
                                if start + span >= hi[k] || span >= size[k] {
                                    out[k] = if span >= size[k] { 0 } else { start };
                                    out[k + 2] = span.min(size[k]);
                                    break;
                                }
                                span *= 2;
                            }
                        }
                        out
                    }
                });
            }
            let window = window.unwrap();
            let crop = (window[2] < size[0] || window[3] < size[1]).then_some(window);
            if let Some(crop) = crop {
                for b in built.iter_mut().filter(|b| std::ptr::eq(b.outfit, outfit)) {
                    for model in b.models.values_mut().chain(b.hud.iter_mut()) {
                        for (name, bytes, _) in &mut model.surfaces {
                            if name == map.name {
                                remap_uvs(bytes, crop, size)?;
                            }
                        }
                    }
                }
            }
            let dest = work.join(format!("{}_{}", outfit.ns, map.name));
            for suffix in ["", "_nmap"] {
                let from = format!("{}{suffix}.tga", stem.display());
                // The window in this file's texels (a normal map may be
                // larger than its diffuse map: both are powers of two).
                let own = tga_size(Path::new(&from))?;
                let [x, y, w, h] = [0, 1, 2, 3].map(|k| window[k] * own[k % 2] / size[k % 2]);
                let scale = (f64::from(MAX_MAP) / f64::from(w.max(h))).min(1.0);
                let (tw, th) = ((f64::from(w) * scale) as u32, (f64::from(h) * scale) as u32);
                let to = format!("{}{suffix}.tga", dest.display());
                let result = std::process::Command::new("magick")
                    .arg(&from)
                    // The colour as painted: some donor maps carry a cutout in
                    // their alpha (the sneakers in the jeans map), which the
                    // character material does not read.
                    .args(["-alpha", "off"])
                    .args(["-crop", &format!("{w}x{h}+{x}+{y}"), "+repage"])
                    .args(["-filter", "Lanczos", "-resize", &format!("{tw}x{th}!")])
                    .args(["-type", "TrueColor", "-compress", "None"])
                    .arg(&to)
                    .output()
                    .context("starting ImageMagick")?;
                ensure!(
                    result.status.success(),
                    "ImageMagick: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                inputs.insert(from.clone(), record_file("", "", &fs::read(&from)?).sha256);
                println!(
                    "{} map {}{suffix}: {w}x{h} of {own:?} at ({x}, {y}) -> {tw}x{th}",
                    outfit.key, map.name
                );
                if let (Some([v0, v1]), "") = (map.black_band, suffix) {
                    // The band's rows in the cut and resized map.
                    let row = |v: f64| {
                        ((v * f64::from(own[1]) - f64::from(y)) / f64::from(h) * f64::from(th))
                            .round() as u32
                    };
                    let (r0, r1) = (row(v0), row(v1));
                    ensure!(
                        r0 < r1 && r1 + (r1 - r0) <= th,
                        "{}: black band rows {r0}..{r1} outside the map",
                        map.stem
                    );
                    let band = r1 - r0;
                    let result = std::process::Command::new("magick")
                        .arg(&to)
                        .args([
                            "(",
                            "+clone",
                            "-crop",
                            &format!("{tw}x{band}+0+{r1}"),
                            "+repage",
                            "-flip",
                            ")",
                        ])
                        .args(["-geometry", &format!("+0+{r0}"), "-composite"])
                        .args(["-type", "TrueColor", "-compress", "None"])
                        .arg(&to)
                        .output()
                        .context("starting ImageMagick")?;
                    ensure!(
                        result.status.success(),
                        "ImageMagick: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    println!(
                        "{} map {}: rows {r0}..{r1} painted over with the {band} below, reflected",
                        outfit.key, map.name
                    );
                }
                if map.waistband {
                    paint_waistband(Path::new(&to), suffix == "_nmap")?;
                    println!("{} map {}{suffix}: the waistband painted at {:?}", outfit.key, map.name, WAISTBAND_STRIP);
                }
            }
            stems.insert((outfit.key, map.name), dest);
        }
    }

    // --- Fits, accessories first ---
    let bind = cx.bind.clone();
    let stock_torsos = stock(&mut cx.view, "torso", "data_full", false)?
        .remove("data_full")
        .unwrap_or_default();
    let stock_legs: BTreeMap<String, Vec<(String, Vec<u8>)>> =
        ["data_full", "data_medium", "data_long"]
            .into_iter()
            .map(|t| {
                Ok((
                    t.to_owned(),
                    stock(&mut cx.view, "legs", t, false)?
                        .remove(t)
                        .unwrap_or_default(),
                ))
            })
            .collect::<Result<_>>()?;
    let mut gloves = stock(&mut cx.view, "gloves", "data", true)?;
    let mut boots = stock(&mut cx.view, "boots", "data", true)?;
    println!(
        "Fitting against {} stock torsos' surfaces, {} trousers', {} gloves', {} boots', {} masks'",
        stock_torsos.len(),
        stock_legs.values().map(Vec::len).sum::<usize>(),
        gloves.values().map(Vec::len).sum::<usize>(),
        boots.values().map(Vec::len).sum::<usize>(),
        cx.masks.len()
    );
    // Gloves inside every wrist-length sleeve.
    for b in built.iter_mut().filter(|b| b.piece.kind == Kind::Gloves) {
        ensure!(
            b.class() == Some("data_full"),
            "{}: only wrist-length gloves are fitted",
            b.label()
        );
        let label = b.label();
        let model = b.models.get_mut("").unwrap();
        let r = model
            .fit_merged(|g| crate::limb_fit::fit(g, &stock_torsos, &bind, &CUFFS_INTO_SLEEVES))?;
        print_limb(&label, "cuffs into the sleeves of", stock_torsos.len(), &r);
        let fitted = model.clone();
        if let Some(hud) = &mut b.hud {
            *hud = fitted.clone();
        }
        gloves.entry("data_full".to_owned()).or_default().extend(
            fitted
                .surface_bytes()
                .into_iter()
                .map(|(n, m)| (format!("{label} {n}"), m)),
        );
    }
    // Boots: low ones inside every full-length hem, the others over every
    // tucked leg of their class.
    for b in built.iter_mut().filter(|b| b.piece.kind == Kind::Boots) {
        let label = b.label();
        let class = b.class().context("boots without a model_type")?;
        let model = b.models.get_mut("").unwrap();
        let r = match class {
            "data_full" => {
                let legs = &stock_legs["data_full"];
                let r =
                    model.fit_merged(|g| crate::limb_fit::fit(g, legs, &bind, &BOOTS_INTO_HEMS))?;
                print_limb(&label, "shafts into the hems of", legs.len(), &r);
                r
            }
            other => {
                let legs = &stock_legs[other];
                let r =
                    model.fit_merged(|g| crate::limb_fit::fit(g, legs, &bind, &BOOTS_OVER_LEGS))?;
                print_limb(
                    &label,
                    &format!("shafts over the {other} legs of"),
                    legs.len(),
                    &r,
                );
                r
            }
        };
        let _ = r;
        boots.entry(class.to_owned()).or_default().extend(
            model
                .surface_bytes()
                .into_iter()
                .map(|(n, m)| (format!("{label} {n}"), m)),
        );
    }
    // Trousers: over low boots, into the others; cut deep inside them.
    let base_legs = cx.view.read(BASE_LEGS)?;
    let mut all_legs: Vec<(String, Vec<u8>)> = stock_legs.values().flatten().cloned().collect();
    for b in built.iter_mut().filter(|b| b.piece.kind == Kind::Legs) {
        let label = b.label();
        if let Some((added, from)) = close_waist(b.models.get_mut("").unwrap(), &base_legs)? {
            println!(
                "{label}: the waist (its top at {:.0} mm, under the shortest jackets' hems) grown straight up to the native legs' top with {added} triangles",
                from * 1000.0
            );
        }
        let base = b.models[""].clone();
        for &(table, suffix) in Kind::Legs.variants() {
            let class_boots = boots
                .get(table)
                .with_context(|| format!("no {table} boots"))?;
            let mut fitted = base.clone();
            if table == "data_full" {
                let (n, most) = lengthen_hems(&mut fitted, &cx.bones)?;
                println!(
                    "{label}: hems lengthened to {:.0} mm over low boots: {n} vertices, up to {:.1} mm",
                    FULL_HEM * 1000.0,
                    most * 1000.0
                );
                let r = fitted.fit_merged(|g| {
                    crate::limb_fit::fit(g, class_boots, &bind, &HEMS_OVER_BOOTS)
                })?;
                print_limb(&label, "hems over the shafts of", class_boots.len(), &r);
            } else {
                let r = fitted.fit_merged(|g| {
                    crate::limb_fit::fit(g, class_boots, &bind, &LEGS_INTO_BOOTS)
                })?;
                print_limb(
                    &label,
                    &format!("legs into the {table} boots'"),
                    class_boots.len(),
                    &r,
                );
                let (cut, at) = trim_model(
                    &fitted,
                    class_boots,
                    &bind,
                    [("LeftLeg", "LeftFoot"), ("RightLeg", "RightFoot")],
                )?;
                println!("  cut {:.0} mm below the knee", at * 1000.0);
                fitted = cut;
            }
            all_legs.extend(
                fitted
                    .surface_bytes()
                    .into_iter()
                    .map(|(n, m)| (format!("{label}{suffix} {n}"), m)),
            );
            b.models.insert(suffix, fitted);
        }
    }
    // Jackets: the waist over every trousers, the collar over the neck and
    // every mask, the sleeves per glove class.
    let bone = |n: &str| {
        cx.bones
            .get(n)
            .copied()
            .with_context(|| format!("no {n} bone"))
    };
    let axis = [
        [0.0, 0.0, bone("Hip")?[2]],
        bone("Hip")?,
        bone("Spin")?,
        bone("Spin_1")?,
        bone("Chest")?,
        bone("Neck")?,
        bone("Head")?,
    ];
    let leg_meshes: Vec<(String, Mesh)> = all_legs
        .iter()
        .map(|(l, m)| Ok((l.clone(), Shell::parse(l.clone(), m)?.mesh)))
        .collect::<Result<_>>()?;
    let head_mesh = parse_mesh(&cx.head, 40, "native head")?;
    let neck_meshes: Vec<(String, Mesh)> =
        std::iter::once(("native head".to_owned(), head_mesh.clone()))
            .chain(cx.masks.iter().map(|m| (m.label.clone(), m.mesh.clone())))
            .collect();
    for b in built.iter_mut().filter(|b| b.piece.kind == Kind::Torso) {
        let label = b.label();
        let mut base = b.models[""].clone();
        let hem = base
            .surfaces
            .iter()
            .flat_map(|(_, m, _)| {
                parse_mesh(m, 48, "jacket")
                    .map(|m| m.positions)
                    .unwrap_or_default()
            })
            .map(|p| p[1])
            .fold(f64::INFINITY, f64::min);
        let waist = BodyFit {
            axis: &axis,
            y_range: (hem + 0.005, bone("Chest")?[1]),
            max_x: 0.3,
            clearance: 0.004,
            cap: 0.025,
            by_part: true,
            max_forward: 1.0,
        };
        let covers: Vec<Cover<'_>> = leg_meshes
            .iter()
            .map(|(l, m)| Cover {
                label: l.clone(),
                mesh: m,
            })
            .collect();
        let r = base.fit_merged(|g| crate::body_fit::fit(g, &covers, &waist))?;
        println!(
            "{label}: waist over {} trousers surfaces: moved {} vertices, up to {:.1} mm",
            covers.len(),
            r.moved,
            r.max_move * 1000.0
        );
        crate::novice_026::report_covers(&r.covers);
        let collar = BodyFit {
            axis: &axis,
            y_range: (bone("Chest")?[1], bone("Head")?[1]),
            max_x: 0.2,
            clearance: COLLAR_CLEARANCE,
            cap: 0.012,
            by_part: false,
            max_forward: 0.5,
        };
        let covers: Vec<Cover<'_>> = neck_meshes
            .iter()
            .map(|(l, m)| Cover {
                label: l.clone(),
                mesh: m,
            })
            .collect();
        let r = base.fit_merged(|g| crate::body_fit::fit(g, &covers, &collar))?;
        println!(
            "{label}: collar over the native neck and {} mask surfaces: moved {} vertices, up to {:.1} mm",
            cx.masks.len(),
            r.moved,
            r.max_move * 1000.0
        );
        crate::novice_026::report_covers(&r.covers);
        for &(table, suffix) in Kind::Torso.variants() {
            let class_gloves = gloves
                .get(table)
                .with_context(|| format!("no {table} gloves"))?;
            let mut fitted = base.clone();
            let bare_forearms = b.piece.parts.iter().any(|p| p.forearms);
            let native_arms = if bare_forearms { Some(native_forearms(&mut cx.view, suffix, &cx.bones)?) } else { None };
            if table == "data_full" && !bare_forearms {
                let r = fitted.fit_merged(|g| {
                    crate::limb_fit::fit(g, class_gloves, &bind, &SLEEVES_OVER_CUFFS)
                })?;
                print_limb(&label, "sleeves over the cuffs of", class_gloves.len(), &r);
            } else {
                // Bare forearms under pushed-up sleeves go into wrist-length
                // gloves as they are (only cut): skin is no cloth to pull in.
                if table != "data_full" {
                    let r = fitted.fit_merged(|g| {
                        crate::limb_fit::fit(g, class_gloves, &bind, &SLEEVES_INTO_GAUNTLETS)
                    })?;
                    print_limb(
                        &label,
                        &format!("sleeves into the {table} gloves'"),
                        class_gloves.len(),
                        &r,
                    );
                }
                let (cut, at) = trim_model(
                    &fitted,
                    class_gloves,
                    &bind,
                    [("LeftForeArm", "LeftHand"), ("RightForeArm", "RightHand")],
                )?;
                println!("  sleeves end {:.0} mm along the forearm", at * 1000.0);
                fitted = cut;
            }
            if let Some(arms) = native_arms {
                println!("{label}{suffix}: the native forearms ({} vertices)", read_u32(&arms.1, 8)?);
                fitted.surfaces.push(arms);
            }
            b.models.insert(suffix, fitted);
        }
    }
    // The hood over the native head, as the knit cap is.
    for b in built.iter_mut().filter(|b| b.piece.kind == Kind::Helmet) {
        let label = b.label();
        let model = b.models.get_mut("").unwrap();
        ensure!(model.surfaces.len() == 1, "{label}: one surface");
        // The export's hood was made on a head in the native head's place
        // (the export's faces sit on it): it is only pushed out where the
        // native skull comes through it.
        let r = fit_hood(&mut model.surfaces[0].1, &cx.head, &bind)?;
        println!(
            "{label}: {} skull points through it or within {:.0} mm -> {}, pushed {} vertices up to {:.1} mm",
            r.0,
            HOOD_CLEARANCE * 1000.0,
            r.1,
            r.2,
            r.3 * 1000.0
        );
        let stock = cx.view.read(SCAVENGER_HOOD)?;
        let (grown, n) = grow_cowl(&model.surfaces[0].1, &stock, &cx.head)?;
        model.surfaces[0].1 = grown;
        println!("{label}: grown down to the Scavenger hood's hem over the neck and shoulders and closed across the throat ({n} triangles)");
        {
            let full = parse_mesh(&cx.head, 40, "native head")?;
            let centre = crate::hat_fit::head_centre(&full, &bind)?;
            println!("the stock Scavenger hood over the masks:");
            report_mask_clashes(&parse_mesh(&stock, 48, "Scavenger hood")?, centre, &cx.masks, |p| {
                p[1] > COWL_BOTTOM && (p[2] < HOOD_FACE_FROM || p[1] > HOOD_BROW)
            });
        }
        let r = fit_hood_over_masks(&mut model.surfaces[0].1, &cx.head, &bind, &cx.masks)?;
        println!(
            "{label}: over {} mask surfaces: {} mask points through it or within {:.0} mm -> {}, pushed {} vertices up to {:.1} mm",
            cx.masks.len(),
            r.0,
            HOOD_MASK_CLEARANCE * 1000.0,
            r.1,
            r.2,
            r.3 * 1000.0
        );
    }

    // --- The bundles ---
    for key in outfits {
        let outfit = outfit(key)?;
        let pieces: Vec<&Built> = built
            .iter()
            .filter(|b| std::ptr::eq(b.outfit, outfit))
            .collect();
        write_bundle(
            &mut cx,
            outfit,
            &pieces,
            &stems,
            db,
            &inputs,
            &output.join(outfit.key),
        )?;
    }
    Ok(())
}

/// Room the hood leaves over the native skull (as the knit cap's
/// `hat_fit::HEAD_CLEARANCE`) and the farthest it may be pushed.
const HOOD_CLEARANCE: f64 = 0.004;
const HOOD_MAX_PUSH: f64 = 0.015;

/// Pushes the hood (48-byte vertices) out from the head's centre where a
/// vertex of the skull under it ([`head_under_hood`]) comes through it or
/// within [`HOOD_CLEARANCE`] of it, spreading each push to the neighbours;
/// the skull points through it before and after, the vertices pushed and
/// the farthest push (m).
fn fit_hood(hood: &mut [u8], head: &[u8], bind_pose: &[u8]) -> Result<(usize, usize, usize, f64)> {
    let full = parse_mesh(head, 40, "native head")?;
    let centre = crate::hat_fit::head_centre(&full, bind_pose)?;
    let skull = parse_mesh(&head_under_hood(head)?, 40, "native skull")?;
    let points: Vec<[f64; 3]> = skull
        .triangles
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<usize>>()
        .into_iter()
        .map(|i| skull.positions[i])
        .collect();
    push_out(hood, centre, &points, HOOD_CLEARANCE, HOOD_MAX_PUSH, "the native skull")
}

/// Room the hood leaves over a mask under it (m): masks go under the hood
/// (the owner, 2026-10-31), as under the stock helmets that leave the mask
/// free.
const HOOD_MASK_CLEARANCE: f64 = 0.003;
/// The farthest a mask's point is cleared (m). The bulky helmet masks' neck
/// guards, hoses and filters stand 30 to 62 mm out of the hood at the neck
/// and the sides; they stand as far out of the stock Scavenger hood (31 to
/// 57 mm, and the 0.69 balaclavas 23 to 26 mm, which this hood clears), and
/// clearing them would blow the hood up off the head, so they are left.
const HOOD_MASK_REACH: f64 = 0.015;
/// The farthest the hood may be pushed out over the masks (m).
const HOOD_MASK_MAX_PUSH: f64 = 0.02;

/// The hood (48-byte vertices, grown) pushed out over every mask where the
/// mask is under it: its vertices behind the face (as [`head_under_hood`]'s,
/// down to the cowl's hem) that come through the hood or within
/// [`HOOD_MASK_CLEARANCE`] of it. The parts of a mask in front of the face
/// show through the hood's opening and are left as they are.
fn fit_hood_over_masks(
    hood: &mut [u8],
    head: &[u8],
    bind_pose: &[u8],
    masks: &[Shell],
) -> Result<(usize, usize, usize, f64)> {
    let full = parse_mesh(head, 40, "native head")?;
    let centre = crate::hat_fit::head_centre(&full, bind_pose)?;
    let under = |p: &[f64; 3]| p[1] > COWL_BOTTOM && (p[2] < HOOD_FACE_FROM || p[1] > HOOD_BROW);
    let mesh = parse_mesh(hood, 48, "hood")?;
    report_mask_clashes(&mesh, centre, masks, under);
    let reach = |p: &[f64; 3]| {
        let d = sub(*p, centre);
        let r = dot(d, d).sqrt();
        crate::mask_fit::farthest_hit(centre, d.map(|v| v / r), &mesh.positions, &mesh.triangles)
            .is_none_or(|(t, _, _)| r + HOOD_MASK_CLEARANCE - t <= HOOD_MASK_REACH)
    };
    let points: Vec<[f64; 3]> = masks
        .iter()
        .flat_map(|m| m.mesh.positions.iter().copied())
        .filter(|p| under(p) && reach(p))
        .collect();
    push_out(hood, centre, &points, HOOD_MASK_CLEARANCE, HOOD_MASK_MAX_PUSH, "the masks")
}

/// Prints, for each mask, its points under a hood (`under`) that stand out
/// through `hood` (seen from `centre`) and the farthest.
fn report_mask_clashes(hood: &Mesh, centre: [f64; 3], masks: &[Shell], under: impl Fn(&[f64; 3]) -> bool) {
    for m in masks {
        let mut worst = (0.0_f64, 0usize, [0.0; 3]);
        for p in m.mesh.positions.iter().filter(|p| under(p)) {
            let d = sub(*p, centre);
            let r = dot(d, d).sqrt();
            if let Some((t, _, _)) = crate::mask_fit::farthest_hit(centre, d.map(|v| v / r), &hood.positions, &hood.triangles) {
                let pen = r - t;
                if pen > 0.0 {
                    worst.1 += 1;
                    if pen > worst.0 {
                        worst = (pen, worst.1, *p);
                    }
                }
            }
        }
        if worst.1 > 0 {
            println!(
                "  {}: {} points through, up to {:.1} mm (at {:+.3} {:.3} {:+.3})",
                m.label, worst.1, worst.0 * 1000.0, worst.2[0], worst.2[1], worst.2[2]
            );
        }
    }
}

/// Pushes the hood (48-byte vertices) out from `centre` where one of
/// `points` comes through it or within `clearance` of it, spreading each
/// push to the neighbours; the points through it before and after, the
/// vertices pushed and the farthest push (m).
fn push_out(
    hood: &mut [u8],
    centre: [f64; 3],
    points: &[[f64; 3]],
    clearance: f64,
    max_push_allowed: f64,
    what: &str,
) -> Result<(usize, usize, usize, f64)> {
    use crate::mask_fit::farthest_hit;
    let mesh = parse_mesh(hood, 48, "hood")?;
    let (weld, neighbours) = crate::mask_fit::weld_vertices(&mesh);
    let mut positions = mesh.positions.clone();
    let ray = |p: [f64; 3]| {
        let d = sub(p, centre);
        let r = dot(d, d).sqrt();
        (d.map(|v| v / r), r)
    };
    let through = |positions: &[[f64; 3]], margin: f64| {
        let mut out = vec![];
        for &q in points {
            let (dir, r) = ray(q);
            if let Some((t, tri, bary)) = farthest_hit(centre, dir, positions, &mesh.triangles) {
                let pen = r + margin - t;
                if pen > 0.0 {
                    out.push((pen, tri, bary));
                }
            }
        }
        out
    };
    let before = through(&positions, 0.0).len();
    for _ in 0..40 {
        let found = through(&positions, clearance);
        if found.iter().all(|(pen, _, _)| *pen < 0.0002) {
            break;
        }
        let mut need = vec![0.0_f64; neighbours.len()];
        for (pen, tri, bary) in found {
            for (corner, b) in mesh.triangles[tri].iter().zip(bary) {
                let w = &mut need[weld[*corner]];
                *w = w.max(pen * (0.3 + b * 1.4).min(1.0));
            }
        }
        for _ in 0..4 {
            need = (0..neighbours.len())
                .map(|v| {
                    let n = &neighbours[v];
                    let mean = n.iter().map(|&u| need[u]).sum::<f64>() / n.len().max(1) as f64;
                    need[v].max(0.6 * mean)
                })
                .collect();
        }
        for (i, p) in positions.iter_mut().enumerate() {
            let (dir, _) = ray(*p);
            for k in 0..3 {
                p[k] += dir[k] * need[weld[i]];
            }
        }
    }
    let after = through(&positions, 0.0).len();
    ensure!(after == 0, "{what} still come through the hood at {after} points");
    let mut pushed = BTreeSet::new();
    let mut max_push = 0.0_f64;
    for (i, p) in positions.iter().enumerate() {
        let d = sub(*p, mesh.positions[i]);
        let push = dot(d, d).sqrt();
        if push > 1e-4 {
            pushed.insert(weld[i]);
        }
        max_push = max_push.max(push);
    }
    ensure!(
        max_push <= max_push_allowed,
        "the hood had to be pushed {:.1} mm out over {what}",
        max_push * 1000.0
    );
    for (i, p) in positions.iter().enumerate() {
        for (k, v) in p.iter().enumerate() {
            let at = 12 + i * 48 + k * 4;
            hood[at..at + 4].copy_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    Ok((before, after, pushed.len(), max_push))
}

/// The part of the native head (40-byte vertices) under the hood: above the
/// nape under its back hem (m), and behind the cheeks (z, m) or over the
/// brow (m). The neck below is the jacket collar's, and the face shows
/// through the hood's opening.
const HOOD_HEAD_FROM: f64 = 1.60;
const HOOD_FACE_FROM: f64 = 0.04;
const HOOD_BROW: f64 = 1.75;

/// `head` (40-byte vertices) with only its triangles under the hood
/// ([`HOOD_HEAD_FROM`], [`HOOD_FACE_FROM`], [`HOOD_BROW`]); every vertex is
/// kept, so the skin and indices stay valid.
fn head_under_hood(head: &[u8]) -> Result<Vec<u8>> {
    let mesh = parse_mesh(head, 40, "native head")?;
    let under = |p: [f64; 3]| p[1] > HOOD_HEAD_FROM && (p[2] < HOOD_FACE_FROM || p[1] > HOOD_BROW);
    let kept: Vec<&[usize; 3]> = mesh
        .triangles
        .iter()
        .filter(|t| t.iter().all(|&i| under(mesh.positions[i])))
        .collect();
    ensure!(!kept.is_empty(), "nothing of the head is under the hood");
    let index_chunk = 12 + mesh.positions.len() * 40;
    let mut out = head[..index_chunk].to_vec();
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(4 + kept.len() * 6)?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(kept.len() * 3)?.to_le_bytes());
    for t in kept {
        for &i in t {
            out.extend_from_slice(&u16::try_from(i)?.to_le_bytes());
        }
    }
    let rest = index_chunk + 8 + read_u32(head, index_chunk + 4)?;
    out.extend_from_slice(head.get(rest..).context("truncated head")?);
    Ok(out)
}

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.to_owned(),
        value,
    }
}

/// Writes one model folder: the native boot's settings, skeleton and bind
/// pose with this model's surfaces, materials and bounds.
fn write_model(
    cx: &mut Fitting,
    out: &Writer<'_>,
    ns: &str,
    root: &str,
    model: &Model,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut settings = parse_config(&mut cx.view, &format!("{BASE}/settings"), "model template")?;
    let names: Vec<V> = model.surfaces.iter().map(|(n, _, _)| s(n)).collect();
    settings.root.set_path(
        "lod_hierrarchy",
        V::Named(vec![field(
            "LOD0",
            V::Named(vec![field("surfaces", V::Indexed(names))]),
        )]),
    )?;
    settings.root.set_path(
        "material_settings",
        V::Named(
            model
                .surfaces
                .iter()
                .map(|(n, _, _)| {
                    field(
                        n,
                        V::Named(vec![
                            field(
                                "material_name",
                                s(if n == NATIVE_ARMS {
                                    NATIVE_ARMS_MATERIAL.to_owned()
                                } else {
                                    format!("ported/{ns}/{n}")
                                }),
                            ),
                            field("used_in_portals", V::Boolean(false)),
                        ]),
                    )
                })
                .collect(),
        ),
    )?;
    out.config(&format!("{root}/settings"), &settings, files)?;
    out.raw(&format!("{root}/render/bind_pose"), &cx.bind, files)?;
    let skeleton = cx.view.read(&format!("{BASE}/skeleton"))?;
    out.raw(&format!("{root}/skeleton"), &skeleton, files)?;
    let (mut all_min, mut all_max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for (name, bytes, props) in &model.surfaces {
        let (min, max) = converted_bounds(bytes)?;
        for k in 0..3 {
            all_min[k] = all_min[k].min(min[k]);
            all_max[k] = all_max[k].max(max[k]);
        }
        let mut props = props.clone();
        set_bounds(&mut props, min, max)?;
        let vertices = read_u32(bytes, 8)?;
        let indices = read_u32(bytes, 12 + vertices * 48 + 8)?;
        props
            .root
            .set_path("source.vertices", V::Integer(i32::try_from(vertices)?))?;
        props
            .root
            .set_path("source.indices", V::Integer(i32::try_from(indices)?))?;
        if props
            .root
            .named("source")
            .and_then(|s| s.named("primitives"))
            .is_some()
        {
            props
                .root
                .set_path("source.primitives", V::Integer(i32::try_from(indices / 3)?))?;
        }
        out.config(
            &format!("{root}/render/{name}/export_properties"),
            &props,
            files,
        )?;
        out.raw(
            &format!("{root}/render/{name}/converted_model"),
            bytes,
            files,
        )?;
    }
    let mut props = parse_config(
        &mut cx.view,
        &format!("{BASE}/render/export_properties"),
        "render template",
    )?;
    set_bounds(&mut props, all_min, all_max)?;
    props.root.set_path(
        "src_filename",
        s(format!("___surva_stk2_characters_port/{root}")),
    )?;
    out.config(&format!("{root}/render/export_properties"), &props, files)
}

/// A box and, where the properties have one, the sphere around it.
fn set_bounds(c: &mut BinaryConfig, min: [f32; 3], max: [f32; 3]) -> Result<()> {
    c.root.set_path("bounding_box.min", V::Float3(min))?;
    c.root.set_path("bounding_box.max", V::Float3(max))?;
    if c.root.named("bounding_sphere").is_some() {
        let centre: [f32; 3] = std::array::from_fn(|k| (min[k] + max[k]) / 2.0);
        let radius = (0..3)
            .map(|k| (max[k] - centre[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        c.root.set_path("bounding_sphere.pos", V::Float3(centre))?;
        c.root
            .set_path("bounding_sphere.radius", V::Float(radius))?;
    }
    Ok(())
}

/// The model fields of one config table: the variant's first surface, the
/// others as additional models of the same model.
fn set_table(c: &mut BinaryConfig, table: &str, model_path: &str, model: &Model) -> Result<()> {
    let first = &model.surfaces[0].0;
    c.root
        .set_path(&format!("{table}.base_model"), s(model_path))?;
    c.root.set_path(&format!("{table}.part_name"), s(first))?;
    let extra: Vec<V> = model.surfaces[1..]
        .iter()
        .map(|(n, _, _)| {
            V::Named(vec![
                field("part_name", s(n)),
                field("base_model", s(model_path)),
            ])
        })
        .collect();
    c.root
        .set_path(&format!("{table}.additional_models"), V::Indexed(extra))?;
    Ok(())
}

const MODIFICATIONS: &str = "gameplay/items/modifications/modifications.options";

/// Sets `stats` over a copied item config: level, protection in every hit
/// region and the shown armour and isolation (props 6, 8), weight (and prop
/// 15), bonuses, drop weight.
fn set_stats(c: &mut BinaryConfig, stats: &Stats, bonuses: &[i32]) -> Result<()> {
    for key in ["item_level", "match_making_level"] {
        c.root.set_path(&format!("parameters.{key}"), V::Integer(stats.level))?;
    }
    c.root.set_path("parameters.weight", V::Float(stats.weight))?;
    c.root.set_path("parameters.drop_weight", V::Integer(stats.drop_weight))?;
    for id in stats.bonuses {
        ensure!(bonuses.contains(id), "no stock bonus {id}");
    }
    c.root.set_path(
        "default_modifications",
        V::Indexed(stats.bonuses.iter().map(|&id| V::Integer(id)).collect()),
    )?;
    let mut shown = vec![(15, format!("{:.1}", stats.weight))];
    match &stats.protection {
        None => ensure!(
            !matches!(c.root.named("hit_params"), Some(V::Named(regions)) if !regions.is_empty()),
            "a piece with hit regions and no protection"
        ),
        Some(p) => {
            let share = |percent: u8| V::Float(f32::from(percent) / 100.0);
            c.root.set_path("parameters.armor", share(p.armour))?;
            c.root.set_path("parameters.ano_armor", share(p.isolation))?;
            let Some(V::Named(regions)) = c.root.named_mut("hit_params") else {
                bail!("a protecting piece without hit regions")
            };
            for region in regions.iter_mut() {
                let armour = match p.forearms {
                    Some(forearms) if region.name.ends_with("_forearm") => forearms,
                    _ => p.armour,
                };
                let Some(V::Named(kinds)) = region.value.named_mut("hit_types") else {
                    bail!("hit region {} without hit types", region.name)
                };
                for kind in kinds.iter_mut() {
                    let value = match kind.name.as_str() {
                        "ranged" | "slash" | "explosive" => armour,
                        "fire" | "shock" | "toxic" | "radiation" => p.isolation,
                        other => bail!("hit type {other} of no known kind"),
                    };
                    kind.value.set_path("armor", share(value))?;
                }
            }
            shown.push((6, p.armour.to_string()));
            shown.push((8, p.isolation.to_string()));
        }
    }
    let Some(V::Indexed(props)) = c.root.named_mut("ui_desc").and_then(|u| u.named_mut("props_list")) else {
        bail!("no shown properties")
    };
    for (id, value) in shown {
        let prop = props
            .iter_mut()
            .find(|p| p.named("prop_id") == Some(&V::Integer(id)))
            .with_context(|| format!("no shown property {id}"))?;
        prop.set_path("prop_value", s(&value))?;
    }
    Ok(())
}

fn write_bundle(
    cx: &mut Fitting,
    outfit: &Outfit,
    pieces: &[&Built],
    stems: &BTreeMap<(&str, &str), PathBuf>,
    db: &Path,
    inputs: &BTreeMap<String, String>,
    output: &Path,
) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let out = Writer {
        overlay: &overlay,
        tag: "STK2-outfits-026",
    };
    let mut files = vec![];
    let ns = outfit.ns;
    for b in pieces {
        for (suffix, model) in &b.models {
            let root = format!("models/{}.skinned_model", b.model_path(suffix));
            write_model(cx, &out, ns, &root, model, &mut files)?;
        }
        if let Some(hud) = &b.hud {
            let root = format!("models/{}.skinned_model", b.model_path("_hud"));
            write_model(cx, &out, ns, &root, hud, &mut files)?;
        }
    }
    for map in outfit.maps {
        out.cloth_material(
            &mut cx.view,
            ns,
            map.name,
            &stems[&(outfit.key, map.name)],
            map.finish,
            None,
            &mut files,
        )?;
    }

    // --- Items ---
    let mut dictionary = parse_config(&mut cx.view, DICTIONARY, "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected the indexed items dictionary")
    };
    let modifications = parse_config(&mut cx.view, MODIFICATIONS, "modifications")?;
    let bonuses: Vec<i32> = match modifications.root.named("modifications") {
        Some(V::Indexed(rows)) => rows
            .iter()
            .filter_map(|r| match r.named("id") {
                Some(V::Integer(id)) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => bail!("expected the indexed modifications"),
    };
    let mut registration = vec![];
    let mut item_configs = vec![];
    let mut strings = vec![];
    for b in pieces {
        let piece = b.piece;
        let category = piece.kind.category();
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(piece.id))),
            "item id {} is taken",
            piece.id
        );
        let path = format!(
            "gameplay/items/armour/{category}/{ns}_{}.options",
            piece.name
        );
        ensure!(
            !items.iter().any(|v| v.named("cfg_name") == Some(&s(&path))),
            "{path} is taken"
        );
        ensure!(
            piece
                .template
                .starts_with(&format!("gameplay/items/armour/{category}/")),
            "{}: template of another slot",
            piece.name
        );
        let mut row = items
            .iter()
            .find(|v| v.named("cfg_name") == Some(&s(piece.template)))
            .with_context(|| format!("no stock item {}", piece.template))?
            .clone();
        let name = format!("{ns}_{}", piece.name);
        ensure!(
            !items.iter().any(|v| v.named("name") == Some(&s(&name))),
            "item name {name} is taken"
        );
        row.set_path("dict_id", V::Integer(piece.id))?;
        row.set_path("cfg_name", s(&path))?;
        row.set_path("name", s(&name))?;
        row.set_path("is_premium", V::Boolean(false))?;
        let mut c = parse_config(&mut cx.view, piece.template, "equipment")?;
        c.root.set_path("parameters.faction", V::Integer(outfit.faction))?;
        set_stats(&mut c, &piece.stats, &bonuses)
            .with_context(|| format!("{}: its stats", piece.name))?;
        row.set_path("base_cost", V::Integer(piece.stats.base_cost))?;
        for &(table, suffix) in piece.kind.variants() {
            let model = &b.models[suffix];
            let model_path = b.model_path(suffix);
            set_table(&mut c, table, &model_path, model)?;
            if piece.kind.variants().len() > 1 && table == "data_full" {
                set_table(&mut c, "data", &model_path, model)?;
            }
        }
        if let Some(hud) = &b.hud {
            c.root
                .set_path("data.base_model_hud", s(b.model_path("_hud")))?;
            c.root
                .set_path("data.part_name_hud", s(&hud.surfaces[0].0))?;
        }
        if let Some(model_type) = piece.model_type {
            c.root.set_path("data.model_type", s(model_type))?;
        }
        c.root.set_path(
            "data.additional_model_slots",
            // No piece takes another slot: masks go under the hood (the
            // owner, 2026-10-31), as under the stock helmets that leave the
            // mask free.
            V::Indexed(vec![]),
        )?;
        if let Some(V::Named(fields)) = c.root.named_mut("data") {
            fields.retain(|f| f.name != "decals");
        }
        c.root
            .set_path("parameters.camo_available", V::Boolean(false))?;
        c.root
            .set_path("parameters.decal_available", V::Boolean(false))?;
        c.root.set_path(
            "ui_desc.icon",
            V::Indexed(piece.icon.into_iter().map(V::Integer).collect()),
        )?;
        let key = format!("st_{ns}_{}", piece.name);
        c.root
            .set_path("ui_desc.text_descriptions.name", s(format!("{key}_name")))?;
        c.root.set_path(
            "ui_desc.text_descriptions.description",
            s(format!("{key}_desc")),
        )?;
        strings.push((format!("{key}_name"), piece.title));
        strings.push((format!("{key}_desc"), piece.description));
        out.config(&path, &c, &mut files)?;
        registration.push(json!({
            "dict_id": piece.id,
            "slot": piece.kind.slot(),
            "name": piece.title.english,
            "template": piece.template,
            "config": path,
            "category": category,
            "icon": piece.icon,
            // The model the item's icon shows: the piece as exported, with
            // its surfaces and the maps they are drawn with.
            "model": b.model_path(""),
            "surfaces": b.models[""]
                .surfaces
                .iter()
                .map(|(n, _, _)| n.clone())
                .filter(|n| n != NATIVE_ARMS)
                .collect::<Vec<_>>(),
            "variants": b.models.keys().map(|k| b.model_path(k)).collect::<Vec<_>>(),
        }));
        item_configs.push(c.root.clone());
        items.push(row);
    }
    out.config(DICTIONARY, &dictionary, &mut files)?;
    let mut localizations = vec![];
    for language in LANGUAGES {
        let path = format!("localization/{language}/localization");
        let mut loc = parse_config(&mut cx.view, &path, "localization")?;
        for (key, text) in &strings {
            ensure!(
                crate::localization_026::string_of(&loc.root, key)?.is_none(),
                "{key} is taken"
            );
            set_string(&mut loc.root, key, text.get(language)?)?;
        }
        localizations.push((language, path, loc));
    }
    let views: Vec<(&str, &V)> = localizations
        .iter()
        .map(|(l, _, c)| (*l, &c.root))
        .collect();
    for item in &item_configs {
        check_item_texts(item, &views)?;
    }
    for (_, path, loc) in &localizations {
        out.config(path, loc, &mut files)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": format!("stk2-{}-outfit-to-026g", outfit.key),
            "files": files,
            "inputs": inputs,
            "items": registration,
        }))?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(&registration)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!(
        "Built the STK2 {} outfit: {} items, {} resources",
        outfit.key,
        pieces.len(),
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_icons_names_and_texts_are_unique_and_complete() {
        let mut ids = BTreeSet::new();
        let mut icons = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for outfit in OUTFITS {
            for piece in outfit.pieces {
                assert!(ids.insert(piece.id), "id {} twice", piece.id);
                assert!(icons.insert(piece.icon), "icon {:?} twice", piece.icon);
                assert!(
                    keys.insert((outfit.ns, piece.name)),
                    "{} {} twice",
                    outfit.ns,
                    piece.name
                );
                for text in [piece.title, piece.description] {
                    for language in LANGUAGES {
                        assert!(!text.get(language).unwrap().trim().is_empty());
                    }
                }
                assert!(
                    piece
                        .template
                        .contains(&format!("/{}/", piece.kind.category()))
                );
                for part in piece.parts {
                    assert!(
                        outfit.maps.iter().any(|m| m.name == part.map),
                        "{} {}",
                        outfit.key,
                        part.map
                    );
                }
                match piece.kind {
                    Kind::Gloves => {
                        assert!(piece.hud.is_some() && piece.model_type == Some("data_full"))
                    }
                    Kind::Boots => assert!(piece.model_type.is_some()),
                    _ => assert!(piece.hud.is_none() && piece.model_type.is_none()),
                }
            }
        }
        // 1822 is the Novice jacket's own reserved id; the rest follow the
        // ported ids (1823 the Army balaclava, 1824-1863 the 0.69d decals
        // and equipment-046's premium jacket).
        assert!(ids.iter().all(|&id| id == 1822 || id >= 1864), "{ids:?}");
    }

    #[test]
    fn merged_surfaces_come_back_in_place() {
        fn surface(points: &[[f32; 3]]) -> Vec<u8> {
            let mut b = vec![];
            b.extend_from_slice(&3_u32.to_le_bytes());
            b.extend_from_slice(&(4 + 48 * points.len() as u32).to_le_bytes());
            b.extend_from_slice(&(points.len() as u32).to_le_bytes());
            for p in points {
                let mut v = [7_u8; 48];
                for k in 0..3 {
                    v[k * 4..k * 4 + 4].copy_from_slice(&p[k].to_le_bytes());
                }
                b.extend_from_slice(&v);
            }
            b.extend_from_slice(&4_u32.to_le_bytes());
            b.extend_from_slice(&(4 + 6_u32).to_le_bytes());
            b.extend_from_slice(&3_u32.to_le_bytes());
            for i in [0_u16, 1, 2] {
                b.extend_from_slice(&i.to_le_bytes());
            }
            b
        }
        let props = BinaryConfig {
            root: V::Named(vec![]),
        };
        let a = surface(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let b = surface(&[[2.0; 3], [3.0, 0.0, 0.0], [0.0, 3.0, 0.0]]);
        let mut model = Model {
            surfaces: vec![
                ("a".into(), a.clone(), props.clone()),
                ("b".into(), b.clone(), props),
            ],
        };
        let (merged, counts) = model.merged().unwrap();
        let mesh = parse_mesh(&merged, 48, "merged").unwrap();
        assert_eq!(mesh.triangles, [[0, 1, 2], [3, 4, 5]]);
        model
            .fit_merged(|g| {
                // Move the second surface's first vertex.
                g[12 + 3 * 48..12 + 3 * 48 + 4].copy_from_slice(&9.0_f32.to_le_bytes());
                Ok(())
            })
            .unwrap();
        assert_eq!(model.surfaces[0].1, a);
        assert_eq!(
            f32::from_le_bytes(model.surfaces[1].1[12..16].try_into().unwrap()),
            9.0
        );
        // Only positions change; every other vertex byte stays.
        assert_eq!(model.surfaces[1].1[16..], b[16..]);
        assert_eq!(counts, [3, 3]);
    }
}

/// One surface to pose: its `converted_model` and its skinned model's
/// `render/bind_pose`.
pub struct PoseInput {
    pub model: Vec<u8>,
    pub bind_pose: Vec<u8>,
}

/// Copies of `surfaces` with their vertices skinned into a pose of the
/// player's rig: the bind pose (`clip` `None`) or the clip under
/// `animations/single/weapons/` at `at` (0 its start, 1 its end). For
/// looking at fits in the game's poses.
pub fn pose_surfaces(
    db: &Path,
    patches: &[PathBuf],
    surfaces: &[PoseInput],
    clip: Option<&str>,
    at: f32,
) -> Result<Vec<Vec<u8>>> {
    use crate::skeleton_animation::{Clip, Rig};
    let mut view = crate::novice_026::game_view(db, patches)?;
    let bind_pose = view.read(
        "models/character/human/base/base_character_head.skinned_model/render/bind_pose",
    )?;
    let rig = Rig::parse(&bind_pose, &view.read("animations/skeletons/scavengers_01.skeleton")?)?;
    let bind = rig.rest();
    let pose = match clip {
        None => bind.clone(),
        Some(name) => {
            let path = format!("animations/single/weapons/{name}");
            let clip = Clip::parse(
                &view.read(&format!("{path}.b-spline"))?,
                &view.read(&format!("{path}.bones_names"))?,
            )
            .with_context(|| path.clone())?;
            rig.pose(&clip, clip.duration() * at.clamp(0.0, 1.0))
        }
    };
    let names: Vec<String> = rig.bones.iter().map(|b| b.name.clone()).collect();
    surfaces
        .iter()
        .map(|s| {
            let count = read_u32(&s.model, 8)?;
            let stride = (read_u32(&s.model, 4)? - 4) / count;
            let mesh = parse_mesh(&s.model, stride as usize, "surface")?;
            let posed = crate::hat_fit::Posed::new(&mesh, &s.model, &s.bind_pose, &names)?;
            let skins: Vec<crate::mask_fit::Skin> = posed
                .skins
                .iter()
                .map(|sk| sk.iter().map(|&(b, w)| (posed.bones[usize::from(b)], w)).collect())
                .collect();
            let at = crate::mask_fit::skin_positions(&mesh.positions, &skins, &bind, &pose)?;
            let mut out = s.model.clone();
            for (v, p) in at.iter().enumerate() {
                let o = 12 + v * stride as usize;
                for k in 0..3 {
                    out[o + k * 4..o + k * 4 + 4].copy_from_slice(&(p[k] as f32).to_le_bytes());
                }
            }
            Ok(out)
        })
        .collect()
}
