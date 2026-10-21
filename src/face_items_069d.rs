// SPDX-License-Identifier: GPL-3.0-or-later
//! Seven 0.69d mask-slot items as new v0.26g masks, each on the native head.
//!
//! The four face-wear pieces of 0.69d (category 57, which 0.26g lacks: they
//! become masks with protection) and three armour masks:
//!
//! | 0.26g | 0.69d | item |
//! |---|---|---|
//! | 1812 | 2092 `edge_balaclava` | Keffiyeh (Fringe Settlers) |
//! | 1813 | 2094 `scavenger_balaclava` | 3-Hole Balaclava (Scavengers) |
//! | 1814 | 2093 `renesanse_balaclava` | Balaclava (Renaissance) |
//! | 1815 | 2095 `bm_balaclava` | Buff (Black Market) |
//! | 1816 | 948 `bm_mask_9` | Protective mask (Black Market) |
//! | 1817 | 2062 `snow_20_mask_7` | Snow-20 Thermal vision |
//! | 1818 | 1913 `league_shadow_mask_premium` | Delta 2 mask (premium) |
//!
//! 0.69d's head-worn models bind the native player rig in its bind pose
//! ([`crate::v069d_skinned`]), so each surface is converted in place and
//! only fitted: pushed off the native head, its neck tucked into the stock
//! collars and the part over the skull pulled in under every helmet worn with
//! a mask ([`crate::mask_fit::fit_same_rig_under`]). The cloth then takes the
//! native head's skin and is checked through the game's clips, as the Bandit
//! balaclava was. 0.69d draws the face-wear over per-face copies of its own
//! heads (`remove_head_model`) and the two closed hoods without a head; none
//! of those heads is ported: each item is worn over the native head.
//!
//! Work in progress (parked 2026-10-17): 1812..=1815 fit and pass the pose
//! check; 1816 misses it at one nape vertex (-0.09 mm), 1817 and 1818 were
//! not reached. No icons or pack yet.
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::outfit_026::{Finish, Piece, Writer, add_piece, s, set_card_prop, set_finish};
use crate::weapon::parse_config;
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// The port's folder under `models/ported/`, `material_instances/ported/`
/// and `textures/ported/`.
pub const PORT: &str = "face_069d";

/// What a 0.69d surface is drawn with in 0.26g.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// The native cloth template (`renesanse_tank_torso`), as the Bandit
    /// balaclava.
    Cloth,
    /// The same, with the painted-armour finish: hoods with metal goggles
    /// and plates.
    Armour,
    /// The native Bulwark mask's template (`edge_assault_05_mask`), which
    /// lights an emissive map: the monocular's diode.
    Emissive(&'static str),
}

/// One ported item.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub id: i32,
    /// The 0.69d dictionary id and name, for provenance.
    pub donor_id: i32,
    pub donor_name: &'static str,
    /// The item's own name under `PORT` (model, material, config).
    pub name: &'static str,
    /// The 0.69d model (under `models/`, without `.skinned_model`) and its
    /// LOD0 surface.
    pub model: &'static str,
    pub surface: &'static str,
    /// The 0.69d albedo texture (under `textures/`); its normal map is
    /// `<albedo>_nmap`.
    pub albedo: &'static str,
    pub look: Look,
    pub faction: i32,
    pub item_level: i32,
    /// Protection against bullets, blades and blasts (`armor`), and against
    /// radiation, shock, fire and toxins (`ano_armor`).
    pub armor: f32,
    pub isolation: f32,
    pub weight: f32,
    pub drop_weight: i32,
    /// Silver, or gold for a premium item.
    pub base_cost: i32,
    pub is_premium: bool,
    /// Whether it takes the helmet slot too (`additional_model_slots`): a
    /// rigid part over the skull (goggles, a monocular, a night vision mount)
    /// cannot be pulled in under a helmet as cloth is, so no helmet is worn
    /// with it and it is fitted over the bare head only.
    pub covers_helmet: bool,
    /// Atlas cell, `[row, col]`.
    pub icon: [i32; 2],
    pub name_text: Text,
    pub description: Text,
}

/// The seven items. Protection follows the native masks on 0.26g's scale:
/// the face-wear below the level-3 "Amber" respirator's 35 in all, the
/// armour masks beside the level-5 "Corsair" (30 / 5, 1.0 kg). Proposed,
/// not yet approved by the owner.
pub const SPECS: [Spec; 7] = [
    Spec {
        id: 1812,
        donor_id: 2092,
        donor_name: "edge_balaclava",
        name: "keffiyeh",
        model: "character/human/balaclavas/edge_balaclavas/edge_balaclava_head_00",
        surface: "edge_balaclava_s",
        albedo: "character/human/balaclavas/edge_balaclava/edge_balaclava",
        look: Look::Cloth,
        faction: 4,
        item_level: 3,
        armor: 0.15,
        isolation: 0.10,
        weight: 0.3,
        drop_weight: 130,
        base_cost: 1300,
        is_premium: false,
        covers_helmet: false,
        icon: [10, 6],
        name_text: Text {
            english: "Keffiyeh",
            russian: "Куфия",
            ukrainian: "Куфія",
            german: "Kufiya",
            polish: "Kefija",
        },
        description: Text {
            english: "A cotton keffiyeh wound over the head and face. It keeps out dust and wind and softens a glancing blow a little.",
            russian: "Хлопковая куфия, намотанная на голову и лицо. Защищает от пыли и ветра и немного смягчает скользящий удар.",
            ukrainian: "Бавовняна куфія, намотана на голову й обличчя. Захищає від пилу та вітру й трохи пом'якшує ковзний удар.",
            german: "Eine Kufiya aus Baumwolle, um Kopf und Gesicht gewickelt. Sie hält Staub und Wind ab und dämpft einen Streifschlag ein wenig.",
            polish: "Bawełniana kefija owinięta wokół głowy i twarzy. Chroni przed pyłem i wiatrem i nieco łagodzi uderzenie po stycznej.",
        },
    },
    Spec {
        id: 1813,
        donor_id: 2094,
        donor_name: "scavenger_balaclava",
        name: "balaclava_3_hole",
        model: "character/human/balaclavas/scavenger_balaclavas/scavenger_balaclava_head_00",
        surface: "scavenger_balaclava_s",
        albedo: "character/human/balaclavas/scavenger_balaclava/scavenger_balaclava",
        look: Look::Cloth,
        faction: 1,
        item_level: 3,
        armor: 0.12,
        isolation: 0.12,
        weight: 0.3,
        drop_weight: 130,
        base_cost: 1300,
        is_premium: false,
        covers_helmet: false,
        icon: [11, 6],
        name_text: Text {
            english: "3-Hole Balaclava",
            russian: "Трёхдырочная балаклава",
            ukrainian: "Трьохдіркова балаклава",
            german: "3-Loch-Sturmhaube",
            polish: "Kominiarka z 3 otworami",
        },
        description: Text {
            english: "A knitted balaclava with holes for the eyes and the mouth. Warm, and it hides the face.",
            russian: "Вязаная балаклава с прорезями для глаз и рта. Тёплая и скрывает лицо.",
            ukrainian: "В'язана балаклава з прорізами для очей і рота. Тепла й приховує обличчя.",
            german: "Eine gestrickte Sturmhaube mit Löchern für Augen und Mund. Sie wärmt und verbirgt das Gesicht.",
            polish: "Dziana kominiarka z otworami na oczy i usta. Grzeje i zakrywa twarz.",
        },
    },
    Spec {
        id: 1814,
        donor_id: 2093,
        donor_name: "renesanse_balaclava",
        name: "balaclava",
        model: "character/human/balaclavas/army_balaclavas/army_balaclava_head_00",
        surface: "army_balaclava_s",
        albedo: "character/human/balaclavas/army_balaclava/army_balaclava",
        look: Look::Cloth,
        faction: 3,
        item_level: 3,
        armor: 0.18,
        isolation: 0.06,
        weight: 0.3,
        drop_weight: 130,
        base_cost: 1300,
        is_premium: false,
        covers_helmet: false,
        icon: [12, 6],
        name_text: Text {
            english: "Balaclava",
            russian: "Балаклава",
            ukrainian: "Балаклава",
            german: "Sturmhaube",
            polish: "Kominiarka",
        },
        description: Text {
            english: "A close-fitting army balaclava with one opening for the eyes, made to be worn under a helmet.",
            russian: "Облегающая армейская балаклава с одной прорезью для глаз, сделанная для ношения под шлемом.",
            ukrainian: "Облягаюча армійська балаклава з одним прорізом для очей, зроблена для носіння під шоломом.",
            german: "Eine eng anliegende Armee-Sturmhaube mit einer Öffnung für die Augen, gemacht zum Tragen unter dem Helm.",
            polish: "Przylegająca wojskowa kominiarka z jednym otworem na oczy, do noszenia pod hełmem.",
        },
    },
    Spec {
        id: 1815,
        donor_id: 2095,
        donor_name: "bm_balaclava",
        name: "buff",
        model: "character/human/balaclavas/market_balaclavas/market_balaclava_head_00",
        surface: "market_balaclava_s",
        albedo: "character/human/balaclavas/market_balaclava/market_balaclava",
        look: Look::Cloth,
        faction: 2,
        item_level: 3,
        armor: 0.20,
        isolation: 0.05,
        weight: 0.2,
        drop_weight: 130,
        base_cost: 1300,
        is_premium: false,
        covers_helmet: false,
        icon: [13, 6],
        name_text: Text {
            english: "Buff",
            russian: "Бафф",
            ukrainian: "Баф",
            german: "Schlauchtuch",
            polish: "Buff",
        },
        description: Text {
            english: "A tube scarf with a printed skull, pulled up over the mouth and nose. It hides the face and keeps the dust out.",
            russian: "Шарф-труба с принтом черепа, натянутый на рот и нос. Скрывает лицо и не пускает пыль.",
            ukrainian: "Шарф-труба з принтом черепа, натягнутий на рот і ніс. Приховує обличчя й не пускає пил.",
            german: "Ein Schlauchtuch mit aufgedrucktem Totenkopf, über Mund und Nase gezogen. Es verbirgt das Gesicht und hält den Staub ab.",
            polish: "Komin z nadrukiem czaszki naciągnięty na usta i nos. Zakrywa twarz i nie wpuszcza pyłu.",
        },
    },
    Spec {
        id: 1816,
        donor_id: 948,
        donor_name: "bm_mask_9",
        name: "protective_mask",
        model: "character/human/black_market/black_market_raider_09_mask",
        surface: "black_market_raider_09_mask_s",
        albedo: "character/human/black_market_raider_09/black_market_raider_09_mask",
        look: Look::Armour,
        faction: 2,
        item_level: 5,
        armor: 0.35,
        isolation: 0.10,
        weight: 0.8,
        drop_weight: 110,
        base_cost: 5200,
        is_premium: false,
        covers_helmet: true,
        icon: [15, 6],
        name_text: Text {
            english: "Protective mask",
            russian: "Защитная маска",
            ukrainian: "Захисна маска",
            german: "Schutzmaske",
            polish: "Maska ochronna",
        },
        description: Text {
            english: "A canvas hood with welding goggles and a filtered mouthpiece. It covers the whole head and stops splinters better than it stops fumes.",
            russian: "Брезентовый капюшон со сварочными очками и фильтром у рта. Закрывает всю голову; от осколков защищает лучше, чем от испарений.",
            ukrainian: "Брезентовий каптур зі зварювальними окулярами й фільтром біля рота. Закриває всю голову; від уламків захищає краще, ніж від випарів.",
            german: "Eine Haube aus Segeltuch mit Schweißerbrille und Filtermundstück. Sie bedeckt den ganzen Kopf und hält Splitter besser ab als Dämpfe.",
            polish: "Brezentowy kaptur z goglami spawalniczymi i filtrem przy ustach. Zakrywa całą głowę; lepiej chroni przed odłamkami niż przed oparami.",
        },
    },
    Spec {
        id: 1817,
        donor_id: 2062,
        donor_name: "snow_20_mask_7",
        name: "snow_20_thermal",
        model: "character/human/newyears_2020/renesanse_sniper_07_snow_mask",
        surface: "renesanse_sniper_07_mask_s",
        albedo: "character/human/newyears_2020/renesanse_sniper_07_snow_mask",
        look: Look::Emissive("character/human/newyears_2020/renesanse_sniper_07_snow_mask_emis"),
        faction: 3,
        item_level: 4,
        armor: 0.25,
        isolation: 0.05,
        weight: 0.6,
        drop_weight: 120,
        base_cost: 3300,
        is_premium: false,
        covers_helmet: false,
        icon: [16, 6],
        name_text: Text {
            english: "Snow-20 Thermal vision",
            russian: "Тепловизор \"Снег-20\"",
            ukrainian: "Тепловізор \"Сніг-20\"",
            german: "Schnee-20 Infrarotsicht",
            polish: "Termowizor \"Śnieg-20\"",
        },
        description: Text {
            english: "A winter hood with a thermal imaging monocular, from the Snow-20 kit.",
            russian: "Зимний капюшон с монокуляром-тепловизором из комплекта \"Снег-20\".",
            ukrainian: "Зимовий каптур з монокуляром-тепловізором із комплекту \"Сніг-20\".",
            german: "Eine Winterhaube mit Wärmebild-Monokular aus der Ausrüstung Schnee-20.",
            polish: "Zimowy kaptur z monokularem termowizyjnym z zestawu \"Śnieg-20\".",
        },
    },
    Spec {
        id: 1818,
        donor_id: 1913,
        donor_name: "league_shadow_mask_premium",
        name: "delta_2",
        model: "character/human/special_items/league_shadow/league_shadow_mask_prem",
        surface: "league_shadow_mask_s",
        albedo: "character/human/special_items/league_shadow/league_shadow_mask",
        look: Look::Armour,
        faction: 0,
        item_level: 5,
        armor: 0.35,
        isolation: 0.05,
        weight: 0.6,
        drop_weight: 0,
        base_cost: 500,
        is_premium: true,
        covers_helmet: true,
        icon: [17, 6],
        name_text: Text {
            english: "Delta 2 mask",
            russian: "Маска \"Дельта 2\"",
            ukrainian: "Маска \"Дельта 2\"",
            german: "Delta II Maske",
            polish: "Maska \"Delta 2\"",
        },
        description: Text {
            english: "A strap harness with night vision goggles and a face guard, the mask of the Delta 2 set.",
            russian: "Ременная подвесная система с прибором ночного видения и щитком для лица, маска комплекта \"Дельта 2\".",
            ukrainian: "Ремінна підвісна система з приладом нічного бачення та щитком для обличчя, маска комплекту \"Дельта 2\".",
            german: "Ein Gurtgeschirr mit Nachtsichtbrille und Gesichtsschutz, die Maske des Sets Delta 2.",
            polish: "Uprząż z paskami, noktowizorem i osłoną twarzy, maska zestawu \"Delta 2\".",
        },
    },
];

/// The native mask whose config, model files and dictionary row each item
/// starts from.
const ITEM_TEMPLATE: &str = "gameplay/items/armour/mask/bm_mask_1.options";
const MODEL_TEMPLATE: &str = "models/character/human/scavengers/scavengers_02_mask.skinned_model";
const SURFACE_TEMPLATE: &str = "scavengers_02_attach_s";
const CLOTH_MATERIAL: &str =
    "material_instances/characters/human/renesanse_tank/renesanse_tank_torso.material";
const EMISSIVE_MATERIAL: &str =
    "material_instances/characters/human/edge_assault_05/edge_assault_05_mask.material";
const TEXTURE_OPTIONS: [(&str, &str); 3] = [
    (
        "diffuse",
        "textures/character/human/renesanse_tank/renesanse_tank_torso.options",
    ),
    (
        "normal",
        "textures/character/human/renesanse_tank/renesanse_tank_torso_nmap.options",
    ),
    (
        "emissive",
        "textures/character/human/edge_assault_05/edge_assault_05_mask_emap.options",
    ),
];
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";

/// Native string keys of an item.
pub fn text_keys(spec: &Spec) -> [String; 2] {
    let key = format!("st_{PORT}_{}", spec.name);
    [format!("{key}_name"), format!("{key}_desc")]
}

fn model_root(spec: &Spec) -> String {
    format!("models/ported/{PORT}/{}.skinned_model", spec.name)
}

/// The surface of an item, as 0.26g reads it.
pub fn surface_path(spec: &Spec) -> String {
    format!("{}/render/{}/converted_model", model_root(spec), spec.name)
}

fn set(c: &mut BinaryConfig, path: &str, value: V) -> Result<()> {
    c.root.set_path(path, value)
}

/// Converts a 0.69d DDS (DXT1 albedo or emission, DXT5 normal map; the
/// same encodings 0.26g uses) to the native header and adds its options.
fn texture(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    source: &Path,
    donor: &str,
    dest: &str,
    role: &str,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let bytes = fs::read(source.join(format!("textures/{donor}.dds")))
        .with_context(|| format!("0.69d texture {donor}"))?;
    let srgb = role == "diffuse";
    let dds = crate::duty_026::texture_dds(&bytes, donor, srgb)?;
    let fourcc = dds.get(84..88).context("short DDS")?;
    ensure!(
        fourcc == if role == "normal" { b"DXT5" } else { b"DXT1" },
        "{donor}: {} where 0.26g has {}",
        String::from_utf8_lossy(fourcc),
        if role == "normal" { "DXT5" } else { "DXT1" }
    );
    out.raw(&format!("{dest}.dds"), &dds, files)?;
    let template = TEXTURE_OPTIONS
        .iter()
        .find(|(r, _)| *r == role)
        .map(|(_, p)| *p)
        .context("texture role")?;
    let opts = parse_config(target, template, "texture options")?;
    out.config(&format!("{dest}.options"), &opts, files)
}

/// The item's material: the native template of its look with the donor's
/// albedo and normal map (and emission) and a matte finish.
fn material(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    source: &Path,
    spec: &Spec,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<String> {
    let (template, finish) = match spec.look {
        Look::Cloth => (CLOTH_MATERIAL, Finish::CLOTH),
        Look::Armour => (CLOTH_MATERIAL, Finish::ARMOUR),
        Look::Emissive(_) => (EMISSIVE_MATERIAL, Finish::ARMOUR),
    };
    let mut mat = parse_config(target, template, "material")?;
    let mut maps = vec![
        ("diffuse", spec.albedo.to_owned()),
        ("normal", format!("{}_nmap", spec.albedo)),
    ];
    if let Look::Emissive(emission) = spec.look {
        maps.push(("emissive", emission.to_owned()));
    }
    for (role, donor) in maps {
        let name = format!("ported/{PORT}/{}_{role}", spec.name);
        texture(
            out,
            target,
            source,
            &donor,
            &format!("textures/{name}"),
            role,
            files,
        )?;
        set(
            &mut mat,
            &format!("material.g_stage.effect.texture_{role}.value"),
            s(name),
        )?;
    }
    set_finish(&mut mat, finish)?;
    let name = format!("ported/{PORT}/{}", spec.name);
    out.config(&format!("material_instances/{name}.material"), &mat, files)?;
    Ok(name)
}

/// Writes the model folder of a fitted surface: the native head's bind pose,
/// and the settings, skeleton and properties of a native mask.
fn model_files(
    out: &Writer<'_>,
    target: &mut ResourceDb,
    spec: &Spec,
    surface: &[u8],
    material: &str,
    files: &mut Vec<crate::manifest::BuiltFile>,
) -> Result<()> {
    let root = model_root(spec);
    let bind_pose = target.read(HEAD_BIND_POSE)?;
    // The template mask's own pose differs from the head's only in Root and
    // Item, as 0.69d's do.
    same_rig(
        &target.read(&format!("{MODEL_TEMPLATE}/render/bind_pose"))?,
        &bind_pose,
        MODEL_TEMPLATE,
    )?;
    out.raw(&format!("{root}/render/bind_pose"), &bind_pose, files)?;
    let skeleton = target.read(&format!("{MODEL_TEMPLATE}/skeleton"))?;
    out.raw(&format!("{root}/skeleton"), &skeleton, files)?;
    let mut settings = parse_config(target, &format!("{MODEL_TEMPLATE}/settings"), "settings")?;
    let Some(V::Named(materials)) = settings.root.named_mut("material_settings") else {
        bail!("mask settings without material_settings");
    };
    ensure!(
        materials.len() == 1 && materials[0].name == SURFACE_TEMPLATE,
        "the template mask has other surfaces"
    );
    materials[0].name = spec.name.to_owned();
    set(
        &mut settings,
        &format!("material_settings.{}.material_name", spec.name),
        s(material),
    )?;
    set(
        &mut settings,
        "lod_hierrarchy.LOD0.surfaces",
        V::Indexed(vec![s(spec.name)]),
    )?;
    out.config(&format!("{root}/settings"), &settings, files)?;

    let (min, max) = crate::outfit_026::converted_bounds(surface)?;
    let vertices = crate::mask_fit::read_u32(surface, 8)?;
    let indices = crate::mask_fit::read_u32(surface, 12 + vertices * 48 + 8)?;
    out.raw(&surface_path(spec), surface, files)?;
    let mut props = parse_config(
        target,
        &format!("{MODEL_TEMPLATE}/render/{SURFACE_TEMPLATE}/export_properties"),
        "surface properties",
    )?;
    for (key, value) in [
        ("source.vertices", vertices),
        ("source.indices", indices),
        ("source.primitives", indices / 3),
    ] {
        set(&mut props, key, V::Integer(i32::try_from(value)?))?;
    }
    // 43 + bones a vertex: 44 for the template's one, 47 for these four.
    set(&mut props, "type", V::Integer(47))?;
    set(
        &mut props,
        "source.mesh_name",
        s(format!("|{}|{}", spec.donor_name, spec.surface)),
    )?;
    set(&mut props, "source.texture_name", s(material))?;
    set(&mut props, "bounding_box.min", V::Float3(min))?;
    set(&mut props, "bounding_box.max", V::Float3(max))?;
    out.config(
        &format!("{root}/render/{}/export_properties", spec.name),
        &props,
        files,
    )?;
    let mut render = parse_config(
        target,
        &format!("{MODEL_TEMPLATE}/render/export_properties"),
        "render properties",
    )?;
    set(
        &mut render,
        "src_filename",
        s(format!("0.69d/{}", spec.model)),
    )?;
    set(&mut render, "bounding_box.min", V::Float3(min))?;
    set(&mut render, "bounding_box.max", V::Float3(max))?;
    out.config(&format!("{root}/render/export_properties"), &render, files)
}

/// Converts and fits one surface; prints what the fit did. Returns the
/// fitted 48-byte surface.
/// With `debug`, the converted surface before and after the fit is kept
/// there as `<name>-converted` and `<name>-fitted`.
pub fn fit_surface(
    target: &mut ResourceDb,
    source: &Path,
    spec: &Spec,
    debug: Option<&Path>,
) -> Result<Vec<u8>> {
    let path = source.join(format!(
        "models/{}.skinned_model/render/{}/converted_model",
        spec.model, spec.surface
    ));
    let donor = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed = crate::v069d_skinned::parse_069d(&donor, spec.surface)?;
    // The rig is the player's: the donor's bind pose is the native one.
    let donor_pose = fs::read(source.join(format!(
        "models/{}.skinned_model/render/bind_pose",
        spec.model
    )))?;
    let bind_pose = target.read(HEAD_BIND_POSE)?;
    same_rig(&donor_pose, &bind_pose, spec.model)?;
    let mut mask = crate::v069d_skinned::to_026_four_bone(&parsed)?;
    if let Some(dir) = debug {
        fs::create_dir_all(dir)?;
        fs::write(dir.join(format!("{}-converted", spec.name)), &mask)?;
    }
    let head = target.read(crate::bandit_026::HEAD_MODEL)?;
    let shells = if spec.covers_helmet {
        vec![]
    } else {
        crate::bandit_026::helmets_worn_over_masks(target, &bind_pose)?
    };
    let fit = crate::mask_fit::fit_same_rig_under(&mut mask, &head, &bind_pose, &shells)?;
    let follow = crate::mask_fit::follow_head_weights_along_rays(&mut mask, &head, &bind_pose)?;
    // Where a pose of the game's clips slides the skin out under the cloth.
    let relief =
        crate::bandit_026::relieve_poses(target, &mut mask, &head, crate::mask_fit::RIM_ZONE)?;
    if let Some(dir) = debug {
        fs::write(dir.join(format!("{}-fitted", spec.name)), &mask)?;
    }
    println!(
        "{} ({}): {} vertices; tucked {} onto the neck, pushed {} up to {:.1} mm, pulled {} under helmets up to {:.1} mm; skin from the head for {} ({} had chest or clavicle bones)",
        spec.id,
        spec.donor_name,
        parsed.vertices.len(),
        fit.tucked_vertices,
        fit.pushed_vertices,
        fit.max_push * 1000.0,
        fit.pulled_vertices,
        fit.max_pull * 1000.0,
        follow.changed,
        follow.off_head_bones,
    );
    for s in &fit.shells {
        println!(
            "  {}: {} vertices outside (deepest {:.1} mm) -> {} ({:.1} mm)",
            s.label,
            s.outside_before,
            s.depth_before * 1000.0,
            s.outside_after,
            s.depth_after * 1000.0
        );
    }
    println!(
        "  relief over the game's poses: {} rounds, {} vertices moved out up to {:.2} mm",
        relief.rounds,
        relief.moved,
        relief.farthest * 1000.0
    );
    let (mut worst, mut face) = (f64::INFINITY, f64::INFINITY);
    for (clip, c) in
        crate::bandit_026::pose_clearances_rim(target, &mask, &head, crate::mask_fit::RIM_ZONE)?
    {
        worst = worst.min(c.min);
        face = face.min(c.face_min);
        ensure!(
            c.min >= crate::bandit_026::POSED_CLEARANCE,
            "{}: the head shows through in {clip}: {:.2} mm at {:?} ({} head vertices through)",
            spec.donor_name,
            c.min * 1000.0,
            c.at,
            c.through
        );
    }
    println!(
        "  least gap over the head in {} clips: {:.2} mm (cheeks and jaw {:.2} mm)",
        crate::bandit_026::POSE_CLIPS.len(),
        worst * 1000.0,
        face * 1000.0
    );
    Ok(mask)
}

/// Whether a donor bind pose is the native one: the same bones in the same
/// order at the same frames, but for Root and Item, which nothing is
/// skinned to (0.69d moved both).
pub fn same_rig(donor: &[u8], native: &[u8], label: &str) -> Result<()> {
    let d = crate::armor::bind_pose_frames(donor, label)?;
    let n = crate::armor::bind_pose_frames(native, "native head")?;
    ensure!(
        d.len() == n.len(),
        "{label}: {} bones, the native rig {}",
        d.len(),
        n.len()
    );
    for (i, ((dn, df), (nn, nf))) in d.iter().zip(&n).enumerate() {
        ensure!(dn == nn, "{label}: bone {i} is {dn}, natively {nn}");
        if dn == "Root" || dn == "Item" {
            continue;
        }
        let worst = df
            .iter()
            .zip(nf)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        ensure!(
            worst < 1e-4,
            "{label}: {dn} binds {worst} away from the native rig"
        );
    }
    Ok(())
}

/// The item config, from the native "Corsair" mask (`bm_mask_1`): this
/// item's model, protection, weight, level and faction.
fn configure(c: &mut BinaryConfig, spec: &Spec) -> Result<()> {
    for key in ["item_level", "match_making_level"] {
        set(c, &format!("parameters.{key}"), V::Integer(spec.item_level))?;
    }
    set(c, "parameters.faction", V::Integer(spec.faction))?;
    set(c, "parameters.weight", V::Float(spec.weight))?;
    set(c, "parameters.armor", V::Float(spec.armor))?;
    set(c, "parameters.ano_armor", V::Float(spec.isolation))?;
    set(c, "parameters.drop_weight", V::Integer(spec.drop_weight))?;
    for (hit, value) in [
        ("ranged", spec.armor),
        ("slash", spec.armor),
        ("explosive", spec.armor),
        ("radiation", spec.isolation),
        ("shock", spec.isolation),
        ("fire", spec.isolation),
        ("toxic", spec.isolation),
    ] {
        set(
            c,
            &format!("hit_params.face.hit_types.{hit}.armor"),
            V::Float(value),
        )?;
    }
    // Armour, isolation and weight as the item card shows them.
    for (prop, value) in [
        (6, format!("{:.0}", spec.armor * 100.0)),
        (8, format!("{:.0}", spec.isolation * 100.0)),
        (15, format!("{:.1}", spec.weight)),
    ] {
        set_card_prop(c, prop, &value)?;
    }
    Ok(())
}

/// `patches`: the released `resources.db.patch_<n>`, which the shared files
/// (the dictionary, the localizations) and the helmets the cloth goes under
/// are read through, as players have them.
pub fn build(
    source: &Path,
    db: &Path,
    patches: &[std::path::PathBuf],
    output: &Path,
) -> Result<()> {
    crate::weapon::prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Face-069d",
    };
    let mut files = vec![];
    let mut target = crate::novice_026::game_view(db, patches)?;
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let mut registration = vec![];
    let mut item_configs = vec![];
    let mut strings = BTreeMap::new();
    for spec in &SPECS {
        let mask = fit_surface(&mut target, source, spec, Some(&output.join("debug")))?;
        let material = material(&out, &mut target, source, spec, &mut files)?;
        model_files(&out, &mut target, spec, &mask, &material, &mut files)?;

        let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
            bail!("expected indexed dictionary")
        };
        let piece = Piece {
            id: spec.id,
            slot: 1,
            category: "mask",
            model: spec.name,
            part: spec.name,
            name: spec.name_text.english,
            template: ITEM_TEMPLATE.to_owned(),
            icon: spec.icon,
        };
        let (_, mut c, mut entry) = add_piece(
            &mut target,
            items,
            PORT,
            &piece,
            spec.description.english,
            &mut strings,
        )?;
        // add_piece names an item by its port and category; there are seven.
        let path = format!("gameplay/items/armour/mask/{PORT}_{}.options", spec.name);
        let [name_key, desc_key] = text_keys(spec);
        set(
            &mut c,
            "data.base_model",
            s(format!("ported/{PORT}/{}", spec.name)),
        )?;
        set(&mut c, "ui_desc.text_descriptions.name", s(&name_key))?;
        set(
            &mut c,
            "ui_desc.text_descriptions.description",
            s(&desc_key),
        )?;
        strings.retain(|k, _| !k.starts_with(&format!("st_{PORT}_mask")));
        strings.insert(name_key, spec.name_text.english.to_owned());
        strings.insert(desc_key, spec.description.english.to_owned());
        configure(&mut c, spec)?;
        if spec.covers_helmet {
            set(
                &mut c,
                "data.additional_model_slots",
                V::Indexed(vec![s("helmet")]),
            )?;
        }
        let row = items
            .iter_mut()
            .find(|v| v.named("dict_id") == Some(&V::Integer(spec.id)))
            .context("the new dictionary row")?;
        row.set_path("cfg_name", s(&path))?;
        row.set_path("name", s(format!("{PORT}_{}", spec.name)))?;
        row.set_path("base_cost", V::Integer(spec.base_cost))?;
        row.set_path("is_premium", V::Boolean(spec.is_premium))?;
        // The server reads the slots from the config (additional_model_slots).
        for k in ["slot", "additional_slots", "template"] {
            entry.as_object_mut().unwrap().remove(k);
        }
        entry["config"] = json!(path);
        entry["donor"] =
            json!({"build": "0.69d", "dict_id": spec.donor_id, "name": spec.donor_name});
        for (k, v) in [
            ("item_level", json!(spec.item_level)),
            ("faction", json!(spec.faction)),
            ("weight", json!(spec.weight)),
            ("base_cost", json!(spec.base_cost)),
            ("is_premium", json!(spec.is_premium)),
        ] {
            entry[k] = v;
        }
        out.config(&path, &c, &mut files)?;
        item_configs.push(c.root.clone());
        registration.push(entry);
    }
    out.config("gameplay/db_static_dictionaries", &dictionary, &mut files)?;

    let texts: Vec<(String, Text)> = SPECS
        .iter()
        .flat_map(|spec| {
            let [n, d] = text_keys(spec);
            [(n, spec.name_text), (d, spec.description)]
        })
        .collect();
    ensure!(
        strings.keys().cloned().collect::<BTreeSet<_>>()
            == texts.iter().map(|(k, _)| k.clone()).collect(),
        "the items' string keys changed: {:?}",
        strings.keys()
    );
    let mut localizations = vec![];
    for language in LANGUAGES {
        let path = format!("localization/{language}/localization");
        let mut loc = parse_config(&mut target, &path, "localization")?;
        for (key, text) in &texts {
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
    let record = json!({
        "format_version": 1,
        "asset_id": "face-items-069d-to-026g",
        "donor_build": "0.69d",
        "target_build": "0.26g",
        "armor_dict_ids": [],
        "files": files,
    });
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(&registration)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!(
        "Built {} 0.69d masks: {} resources",
        SPECS.len(),
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    #[test]
    fn ids_names_and_cells_are_distinct_and_clear_of_the_other_ports() {
        let ids: BTreeSet<i32> = SPECS.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), SPECS.len());
        for id in &ids {
            // Duty, Freedom and the Bandit balaclava (1800..=1811), the closed
            // Keeper helmet (1820), the 0.69d ids themselves.
            assert!((1812..=1819).contains(id), "{id}");
        }
        let names: BTreeSet<&str> = SPECS.iter().map(|s| s.name).collect();
        assert_eq!(names.len(), SPECS.len());
        let cells: BTreeSet<[i32; 2]> = SPECS.iter().map(|s| s.icon).collect();
        assert_eq!(cells.len(), SPECS.len());
        // Column 6, rows 1..=9, holds the Duty, Freedom and Bandit icons; the
        // Keeper helmet takes [14, 6].
        for [row, col] in cells {
            assert_eq!(col, 6);
            assert!(row >= 10 && row != 14 && row <= 22, "{row}");
        }
    }

    #[test]
    fn protection_follows_the_native_masks() {
        for s in &SPECS {
            // Every native mask: armour 5..50, isolation 5..70, 0.5..2 kg.
            assert!((0.05..=0.5).contains(&s.armor), "{}", s.name);
            assert!((0.05..=0.7).contains(&s.isolation), "{}", s.name);
            assert!((0.2..=2.0).contains(&s.weight), "{}", s.name);
            // Below the "Corsair" mask's 30 below level 5, cloth below the
            // level-3 "Amber" respirator's 35 in all.
            if s.item_level < 5 {
                assert!(s.armor <= 0.3, "{}", s.name);
            }
            if matches!(s.look, Look::Cloth) {
                assert!(s.armor + s.isolation <= 0.35, "{}", s.name);
            }
            // Premium: gold, not dropped, no seller of its own.
            if s.is_premium {
                assert_eq!((s.drop_weight, s.faction), (0, 0), "{}", s.name);
            } else {
                assert!(
                    s.drop_weight > 0 && (1..=4).contains(&s.faction),
                    "{}",
                    s.name
                );
            }
        }
    }

    #[test]
    fn every_item_is_named_and_described_in_every_client_language() {
        let named = |name: &str, value: V| NamedValue {
            name: name.into(),
            value,
        };
        for spec in &SPECS {
            let [n, d] = text_keys(spec);
            let item = V::Named(vec![named(
                "ui_desc",
                V::Named(vec![named(
                    "text_descriptions",
                    V::Named(vec![
                        named("name", V::String(n.clone())),
                        named("description", V::String(d.clone())),
                    ]),
                )]),
            )]);
            let roots: Vec<(&str, V)> = LANGUAGES
                .iter()
                .map(|&language| {
                    let mut root = V::Named(vec![named("strings", V::Named(vec![]))]);
                    set_string(&mut root, &n, spec.name_text.get(language).unwrap()).unwrap();
                    set_string(&mut root, &d, spec.description.get(language).unwrap()).unwrap();
                    (language, root)
                })
                .collect();
            let views: Vec<(&str, &V)> = roots.iter().map(|(l, r)| (*l, r)).collect();
            check_item_texts(&item, &views).unwrap();
            let t = spec.description;
            for other in [t.russian, t.ukrainian, t.german, t.polish] {
                assert_ne!(other, t.english, "{} is untranslated", spec.name);
            }
        }
    }

    #[test]
    fn a_bind_pose_off_the_native_rig_is_refused() {
        // Two bones: Root moves freely, Head may not.
        let pose = |head_y: f32, root_x: f32| {
            let mut b = 2u16.to_le_bytes().to_vec();
            for (name, t) in [("Root", [root_x, 0.0, 0.0]), ("Head", [0.0, head_y, 0.0])] {
                b.extend_from_slice(name.as_bytes());
                b.push(0);
                for v in t.into_iter().chain([0.0; 3]).chain([1.0; 3]) {
                    b.extend_from_slice(&f32::to_le_bytes(v));
                }
            }
            b
        };
        same_rig(&pose(1.6, 0.2), &pose(1.6, 0.0), "test").unwrap();
        assert!(same_rig(&pose(1.65, 0.0), &pose(1.6, 0.0), "test").is_err());
    }
}
