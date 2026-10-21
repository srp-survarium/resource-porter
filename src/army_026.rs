// SPDX-License-Identifier: GPL-3.0-or-later
//! The second STK2 balaclava, the one with a single wide eye window, as a
//! mask of the Renaissance Army (faction 3, `st_renaissance_faction`: "The
//! Renaissance Army", "Армия Возрождения").
//!
//! STK2's bandit_01 has three heads: `bandit_01_head_01` wears the
//! two-hole balaclava that ships as the Bandit balaclava (1810: the same
//! surface, 422 vertices and the same indices, as the `banndit_01_atach` of
//! the whole `bandit_01` model it was ported from), `bandit_01_head_02` this
//! one (377 vertices, another part of the same `bandit_01_attach` atlas),
//! and `bandit_01_head_03` is a bare face. Only the balaclava is ported, a
//! mask over the native head as the Bandit balaclava is: never the donor's
//! face (owner, 2026-10-15 and 2026-10-21).
//!
//! The fit is the Bandit balaclava's ([`crate::mask_fit`]): a rigid move of
//! the eyes onto the native eye bones, the radial push-out, the flap tucked
//! onto the native neck, and the cloth over the skull pulled under every
//! helmet worn with a mask. Its one window has no pair of holes to anchor
//! on, so it takes the eye holes of head_01's balaclava, converted the same
//! way: the three heads share one skeleton and bind pose and their faces
//! put the eyes at the same place.
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::outfit_026::{Piece, Writer, add_piece, set_card_prop};
use crate::weapon::parse_config;
use crate::{ArmorPortManifest, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Item id: free in stock and in every released Skhid patch (12-26). 1800-1811
/// and 1821 are released ports; 1812-1820 and 1822 are taken by parked ones.
pub const MASK_ID: i32 = 1823;
/// Its icon: a free cell of the sixth column of the equipment atlas (rows
/// 1-9 the exoskeletons and the Bandit balaclava, 10-17 reserved for the
/// 0.69d masks and the closed Keeper helmet, 18 the Novice cap, 19 the
/// Novice jacket's).
pub const MASK_ICON: [i32; 2] = [20, 6];
/// The Renaissance Army (`factions_dict` 3).
pub const FACTION: i32 = 3;
const PORT: &str = "army_026";
/// The balaclava's surface.
pub const MASK_MODEL: &str =
    "models/ported/army_026/mask.skinned_model/render/mask/converted_model";
/// The donor heads, under the STK2 characters export's root.
const DONOR_DIR: &str = "resources/exported/models/character/human/bandit/bandit_01";
const DONOR: &str = "bandit_01_head_02.skinned_model";
/// Wears the two-hole balaclava, whose eye holes anchor the fit.
const EYE_REFERENCE: &str = "bandit_01_head_01.skinned_model";

/// Stats. Level 6 is the owner's (2026-10-21): the server's rule sells a
/// Renaissance item of level 6 at reputation level 1 (item level - 5). The
/// rest are proposed after the one native level 6 mask, the Fringe Settlers'
/// respirator (edge_resp_1: armour 10, isolation 50, 0.8 kg, 5,230, drop
/// weight 100; the Renaissance Army has none of level 6): its weight, price
/// and drop weight. A respirator's protection is no cloth's, so armour and
/// isolation follow the nearest armour mask, the level 5 Corsair (bm_mask_1:
/// 30 / 5).
pub const LEVEL: i32 = 6;
pub const WEIGHT: f32 = 0.8;
pub const ARMOUR: f32 = 0.3;
pub const ISOLATION: f32 = 0.05;
pub const COST: i32 = 5_230;
pub const DROP_WEIGHT: i32 = 100;
/// Smoothing rounds that undo the fit's folds ([`crate::mask_fit::unfold`]).
const UNFOLD_ROUNDS: usize = 200;

/// The balaclava's name and description in every client language.
pub const TEXTS: [(&str, Text); 2] = [
    (
        "st_army_026_mask_name",
        Text {
            english: "Army balaclava",
            russian: "Армейская балаклава",
            ukrainian: "Армійська балаклава",
            german: "Armee-Sturmhaube",
            polish: "Kominiarka wojskowa",
        },
    ),
    (
        "st_army_026_mask_desc",
        Text {
            english: "A balaclava with one wide eye slit, from the cancelled S.T.A.L.K.E.R. 2.",
            russian: "Балаклава с одной широкой прорезью для глаз из отменённого S.T.A.L.K.E.R. 2.",
            ukrainian: "Балаклава з одним широким вирізом для очей зі скасованого S.T.A.L.K.E.R. 2.",
            german: "Sturmhaube mit einem breiten Augenschlitz aus dem eingestellten S.T.A.L.K.E.R. 2.",
            polish: "Kominiarka z jednym szerokim otworem na oczy z anulowanego S.T.A.L.K.E.R. 2.",
        },
    ),
];

fn manifest() -> Result<ArmorPortManifest> {
    Ok(serde_json::from_str(include_str!(
        "../ports/army-balaclava-stk2-to-026g.json"
    ))?)
}

/// The middle of the eye holes of head_01's balaclava, converted as this
/// one is: where the donor's eyes are in the converted space.
fn eye_reference(source: &Path, db: &Path, output: &Path) -> Result<[f64; 3]> {
    let dir = output.join("eye-reference");
    crate::armor::build_bundle(
        &manifest()?,
        db,
        &source.join(DONOR_DIR).join(EYE_REFERENCE),
        None,
        None,
        &dir,
    )?;
    let reference = fs::read(dir.join("overlay").join(MASK_MODEL))?;
    fs::remove_dir_all(&dir)?;
    crate::mask_fit::mask_eye_holes(&reference)
}

/// `source`: the STK2 characters export (its `resources/exported`);
/// `textures`: STK2's `bandit_01` texture folder (`bandit_01_attach.tga`
/// and `_nmap.tga`); `patches`: the released `resources.db.patch_<n>`,
/// which the shared files (the dictionary, the localizations) and the
/// helmets the cloth goes under are read through.
pub fn build(
    source: &Path,
    textures: &Path,
    db: &Path,
    patches: &[PathBuf],
    output: &Path,
) -> Result<()> {
    let manifest = manifest()?;
    let donor_dir = source.join(DONOR_DIR);
    // The anchor holds only if both heads stand on the same rig.
    let rig = |model: &str| -> Result<(Vec<u8>, Vec<u8>)> {
        let mut archive = ResourceDb::open(&donor_dir.join(model))?;
        Ok((archive.read("render/bind_pose")?, archive.read("skeleton")?))
    };
    ensure!(
        rig(DONOR)? == rig(EYE_REFERENCE)?,
        "{DONOR} and {EYE_REFERENCE} have different skeletons or bind poses"
    );
    let mut record =
        crate::armor::build_bundle(&manifest, db, &donor_dir.join(DONOR), None, None, output)?;
    let holes = eye_reference(source, db, output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Army-026",
    };
    let files = &mut record.files;
    let mut target = crate::novice_026::game_view(db, patches)?;
    let head_root = "models/character/human/base/base_character_head.skinned_model/render";
    let bind_pose = target.read(&format!("{head_root}/bind_pose"))?;
    ensure!(
        bind_pose == target.read(&manifest.model.bind_pose_template)?,
        "native head and converted mask use different bind poses"
    );
    let head = target.read(crate::bandit_026::HEAD_MODEL)?;
    let mut mask = fs::read(overlay.join(MASK_MODEL))?;
    let shells = crate::bandit_026::helmets_worn_over_masks(&mut target, &bind_pose)?;
    let fit =
        crate::mask_fit::fit_to_native_head_under_at(&mut mask, holes, &head, &bind_pose, &shells)?;
    println!(
        "Fitted army balaclava (eyes of head_01's balaclava at {:?}): moved {:?} m, tucked {} vertices onto the neck, pushed {} up to {:.1} mm, pulled {} under helmets up to {:.1} mm",
        holes.map(|v| (v * 1e4).round() / 1e4),
        fit.offset,
        fit.tucked_vertices,
        fit.pushed_vertices,
        fit.max_push * 1000.0,
        fit.pulled_vertices,
        fit.max_pull * 1000.0
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
    // The cloth moves as the skin under it.
    let follow = crate::mask_fit::follow_head_weights(&mut mask, &head)?;
    println!(
        "  skin: {} vertices take the head's (Head, Neck), {} had chest or clavicle weights; farthest from the head {:.1} mm",
        follow.changed,
        follow.off_head_bones,
        follow.max_distance * 1000.0
    );
    // The fit folds the cloth over itself in a few places (the neck tuck):
    // a turned triangle is culled from outside and the skin shows through.
    let (turned, left) = crate::mask_fit::unfold(&mut mask, UNFOLD_ROUNDS)?;
    println!("  unfold: {turned} turned triangles -> {left}");
    ensure!(
        left == 0,
        "{left} triangles of the balaclava stay turned over"
    );
    // Where a pose of the game's clips slides the skin out under the cloth.
    let relief = crate::bandit_026::relieve_poses(
        &mut target,
        &mut mask,
        &head,
        0.0,
        crate::bandit_026::RELIEF_MAX,
    )?;
    println!(
        "  relief over the game's poses: {} rounds, {} vertices moved out up to {:.2} mm",
        relief.rounds,
        relief.moved,
        relief.farthest * 1000.0
    );
    let (mut worst, mut face) = (f64::INFINITY, f64::INFINITY);
    let clips = crate::bandit_026::pose_clearances_at(&mut target, &mask, &head, 0.0, true)?;
    for (clip, c) in &clips {
        worst = worst.min(c.min);
        face = face.min(c.face_min);
        ensure!(
            c.min >= crate::bandit_026::POSED_CLEARANCE,
            "the face shows through the balaclava in {clip}: {:.2} mm at {:?} ({} head vertices through)",
            c.min * 1000.0,
            c.at,
            c.through
        );
    }
    println!(
        "  least gap over the head in the bind pose and {} clips: {:.2} mm (cheeks and jaw {:.2} mm)",
        crate::bandit_026::POSE_CLIPS.len(),
        worst * 1000.0,
        face * 1000.0
    );
    // The balaclava samples about a third of the bandit's attachment atlas:
    // its maps keep only that part.
    let crop = crate::bandit_026::atlas_crop(&mask)?;
    crate::bandit_026::remap_to_crop(&mut mask, &crop)?;
    println!(
        "  atlas: UV {:?}..{:?}, maps resampled to {:?} diffuse and {:?} normal",
        crop.uv_min, crop.uv_max, crop.diffuse, crop.normal
    );
    fs::write(
        output.join("atlas-crop.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"uv_min": crop.uv_min, "uv_max": crop.uv_max}),
        )?,
    )?;
    out.raw(MASK_MODEL, &mask, files)?;
    let props_path = MASK_MODEL.replace("/converted_model", "/export_properties");
    let mut props = crate::BinaryConfig::parse(&fs::read(overlay.join(&props_path))?)?;
    let vertices = crate::mask_fit::read_u32(&mask, 8)?;
    let indices = crate::mask_fit::read_u32(&mask, 12 + vertices * 48 + 8)?;
    for (key, value) in [
        ("source.vertices", vertices),
        ("source.indices", indices),
        ("source.primitives", indices / 3),
    ] {
        props
            .root
            .set_path(key, V::Integer(i32::try_from(value)?))?;
    }
    out.config(&props_path, &props, files)?;
    out.refit_bounds(manifest.model.target_models.iter(), files)?;
    out.cloth_material(
        &mut target,
        PORT,
        "attach",
        &textures.join("bandit_01_attach"),
        crate::outfit_026::Finish::CLOTH,
        Some(&crop),
        files,
    )?;

    // --- The item ---
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected indexed dictionary")
    };
    let mut strings = BTreeMap::new();
    let piece = Piece {
        id: MASK_ID,
        slot: 1,
        category: "mask",
        model: "mask",
        part: "mask",
        name: TEXTS[0].1.english,
        // The Black Market mask is the stock mask drawn over the head:
        // the Renaissance Army's take the head's place (remove_head_model).
        template: "gameplay/items/armour/mask/bm_mask_1.options".to_owned(),
        icon: MASK_ICON,
    };
    let (path, mut c, mut entry) = add_piece(
        &mut target,
        items,
        PORT,
        &piece,
        TEXTS[1].1.english,
        &mut strings,
    )?;
    for key in ["item_level", "match_making_level"] {
        c.root
            .set_path(&format!("parameters.{key}"), V::Integer(LEVEL))?;
    }
    c.root.set_path("parameters.faction", V::Integer(FACTION))?;
    c.root
        .set_path("parameters.drop_weight", V::Integer(DROP_WEIGHT))?;
    c.root.set_path("parameters.weight", V::Float(WEIGHT))?;
    c.root.set_path("parameters.armor", V::Float(ARMOUR))?;
    c.root
        .set_path("parameters.ano_armor", V::Float(ISOLATION))?;
    for (hit, value) in [
        ("ranged", ARMOUR),
        ("slash", ARMOUR),
        ("explosive", ARMOUR),
        ("radiation", ISOLATION),
        ("shock", ISOLATION),
        ("fire", ISOLATION),
        ("toxic", ISOLATION),
    ] {
        c.root.set_path(
            &format!("hit_params.face.hit_types.{hit}.armor"),
            V::Float(value),
        )?;
    }
    // Armour, isolation and weight as the item card shows them.
    for (prop, value) in [
        (6, format!("{}", (ARMOUR * 100.0).round())),
        (8, format!("{}", (ISOLATION * 100.0).round())),
        (15, format!("{WEIGHT:.1}")),
    ] {
        set_card_prop(&mut c, prop, &value)?;
    }
    items
        .iter_mut()
        .find(|v| v.named("dict_id") == Some(&V::Integer(MASK_ID)))
        .context("the balaclava's dictionary entry")?
        .set_path("base_cost", V::Integer(COST))?;
    // The server's rule sells it (faction and level); no override.
    for k in ["name", "template", "config", "slot", "additional_slots"] {
        entry.as_object_mut().unwrap().remove(k);
    }
    out.config(&path, &c, files)?;
    out.config("gameplay/db_static_dictionaries", &dictionary, files)?;
    ensure!(
        strings.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == TEXTS.iter().map(|(key, _)| *key).collect(),
        "the balaclava's string keys changed: {:?}",
        strings.keys()
    );
    let mut localizations = vec![];
    for language in LANGUAGES {
        let path = format!("localization/{language}/localization");
        let mut loc = parse_config(&mut target, &path, "localization")?;
        for (key, text) in &TEXTS {
            set_string(&mut loc.root, key, text.get(language)?)?;
        }
        localizations.push((language, path, loc));
    }
    let views: Vec<(&str, &V)> = localizations
        .iter()
        .map(|(l, _, c)| (*l, &c.root))
        .collect();
    check_item_texts(&c.root, &views)?;
    for (_, path, loc) in &localizations {
        out.config(path, loc, files)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(&[entry])?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!("Built Army balaclava: {} resources", record.files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texts_are_translated_and_name_no_faction_but_the_army() {
        for (key, text) in &TEXTS {
            for other in [text.russian, text.ukrainian, text.german, text.polish] {
                assert_ne!(other, text.english, "{key} is untranslated");
            }
            for language in LANGUAGES {
                let t = text.get(language).unwrap();
                assert!(
                    !t.to_lowercase().contains("bandit") && !t.contains("андит"),
                    "{key}: {t}"
                );
            }
        }
    }

    #[test]
    fn the_manifest_converts_only_head_02s_balaclava() {
        let m = manifest().unwrap();
        let models: Vec<&String> = m.model.target_models.keys().collect();
        assert_eq!(models, ["models/ported/army_026/mask.skinned_model"]);
        let surfaces = &m.model.target_models[models[0]].surfaces;
        assert_eq!(surfaces["mask"].source, "bandit_01_attach_s");
    }

    #[test]
    fn the_card_shows_the_stats() {
        assert_eq!(format!("{}", (ARMOUR * 100.0).round()), "30");
        assert_eq!(format!("{}", (ISOLATION * 100.0).round()), "5");
        assert_eq!(format!("{WEIGHT:.1}"), "0.8");
    }
}
