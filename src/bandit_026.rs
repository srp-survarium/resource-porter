// SPDX-License-Identifier: GPL-3.0-or-later
//! STK2 Bandit balaclava retargeted to the native v0.26g player skeleton.
//!
//! Only the balaclava ships (owner's decision, 2026-10-15); the jacket, jeans,
//! hands and shoes stay in the port manifest but are not built.
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::outfit_026::{Piece, Writer, add_piece, set_card_prop};
use crate::weapon::parse_config;
use crate::{ArmorPortManifest, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// The surfaces of every stock helmet that can be worn together with a mask:
/// those whose item does not also take the mask slot.
pub(crate) fn helmets_worn_over_masks(
    target: &mut ResourceDb,
    bind_pose: &[u8],
) -> Result<Vec<crate::mask_fit::Shell>> {
    let prefix = "gameplay/items/armour/helmet/";
    let configs: Vec<String> = target
        .paths_with_prefix(prefix)
        .filter(|p| p.ends_with(".options"))
        .map(str::to_owned)
        .collect();
    let mut shells = vec![];
    for path in configs {
        let config = parse_config(target, &path, "helmet")?;
        let data = config.root.named("data").context("helmet without data")?;
        let takes_mask = matches!(
            data.named("additional_model_slots"),
            Some(V::Indexed(slots)) if slots.iter().any(|s| matches!(s, V::String(s) if s == "mask"))
        );
        if takes_mask {
            continue;
        }
        let mut parts = vec![data];
        if let Some(V::Indexed(extra)) = data.named("additional_models") {
            parts.extend(extra);
        }
        for part in parts {
            let (Some(V::String(model)), Some(V::String(surface))) =
                (part.named("base_model"), part.named("part_name"))
            else {
                bail!("{path}: model part without base_model/part_name");
            };
            let root = format!("models/{model}.skinned_model/render");
            // Helmets bind fewer bones than the head; the ones they share
            // must sit where the head's do, so both meshes share one space.
            let own = crate::mask_fit::bone_origins(&target.read(&format!("{root}/bind_pose"))?)?;
            let head = crate::mask_fit::bone_origins(bind_pose)?;
            for (bone, at) in &own {
                if let Some(native) = head.get(bone) {
                    let d = crate::mask_fit::sub(*at, *native);
                    ensure!(
                        crate::mask_fit::dot(d, d).sqrt() < 1e-4,
                        "{path}: {model} binds {bone} elsewhere than the native head"
                    );
                }
            }
            ensure!(own.contains_key("Head"), "{path}: {model} has no Head bone");
            let label = format!(
                "{} {surface}",
                path.trim_start_matches(prefix).trim_end_matches(".options")
            );
            let bytes = target.read(&format!("{root}/{surface}/converted_model"))?;
            shells.push(crate::mask_fit::Shell::parse(label, &bytes)?);
        }
    }
    ensure!(!shells.is_empty(), "no stock helmet is worn over a mask");
    Ok(shells)
}

/// The balaclava's name and description in every client language.
pub const TEXTS: [(&str, Text); 2] = [
    (
        "st_bandit_026_mask_name",
        Text {
            english: "Bandit balaclava",
            russian: "Бандитская балаклава",
            ukrainian: "Бандитська балаклава",
            german: "Banditen-Sturmhaube",
            polish: "Kominiarka bandyty",
        },
    ),
    (
        "st_bandit_026_mask_desc",
        Text {
            english: "Bandit balaclava from the cancelled S.T.A.L.K.E.R. 2.",
            russian: "Бандитская балаклава из отменённого S.T.A.L.K.E.R. 2.",
            ukrainian: "Бандитська балаклава зі скасованого S.T.A.L.K.E.R. 2.",
            german: "Banditen-Sturmhaube aus dem eingestellten S.T.A.L.K.E.R. 2.",
            polish: "Kominiarka bandyty z anulowanego S.T.A.L.K.E.R. 2.",
        },
    ),
];

/// The native head the balaclava is worn on.
pub const HEAD_MODEL: &str = "models/character/human/base/base_character_head.skinned_model/render/black_market_01_head_s/converted_model";
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";
/// The player's rig, which every clip below drives.
const PLAYER_SKELETON: &str = "animations/skeletons/scavengers_01.skeleton";
/// The balaclava's surface.
pub const MASK_MODEL: &str =
    "models/ported/bandit_026/mask.skinned_model/render/mask/converted_model";

/// Clips the cloth is checked through: the lobby's idles (the user saw the
/// face through the left cheek there, holding a pistol) and the match's
/// standing, aiming, running, sprinting and crouching, with a pistol and a
/// rifle. Each name is under `animations/single/weapons/`.
pub const POSE_CLIPS: &[&str] = &[
    "tt33/player/stand/3rd_view/on_site/idle_wpn_loby",
    "tt33/player/stand/3rd_view/on_site/idle_wpn_loby_1",
    "tt33/player/stand/3rd_view/on_site/idle_wpn_loby_2",
    "tt33/player/stand/3rd_view/on_site/idle_wpn_loby_4",
    "ak/player/stand/3rd_view/on_site/idle_lobby",
    "ak/player/stand/3rd_view/on_site/idle_wpn_lobby_5",
    "ak/player/stand/3rd_view/on_site/idle_wpn_loby_1",
    "ak/player/stand/3rd_view/on_site/idle_wpn_loby_6",
    "rem870/player/stand/3rd_view/on_site/idle_wpn_loby_6",
    "ak/player/stand/3rd_view/on_site/on_site_idle",
    "ak/player/stand/3rd_view/on_site/aimed_on_site_idle",
    "ak/player/stand/3rd_view/run/forward",
    "ak/player/stand/3rd_view/run/aimed_forward_left",
    "ak/player/stand/3rd_view/run/backward_right",
    "ak/player/stand/3rd_view/sprint/sprint",
    "tt33/player/stand/3rd_view/sprint/sprint_left",
    "ak/player/crouch/3rd_view/on_site_idle",
    "ak/player/crouch/3rd_view/aimed_forward",
    "ak/player/crouch/3rd_view/forward_right",
];
/// Poses sampled across each clip.
const POSE_SAMPLES: usize = 24;
/// The least gap the cloth keeps over the head in every sampled pose (m):
/// nothing shows through, with a margin. The fit keeps 0.5 mm in the bind
/// pose where a helmet presses the cloth down; where the Head and Neck
/// weights blend, behind the jaw, a pose takes up to 0.2 mm of that.
pub const POSED_CLEARANCE: f64 = 0.0002;

/// The worst pose of each of [`POSE_CLIPS`]: the native head and `mask`
/// skinned through it.
pub fn pose_clearances(
    db: &mut ResourceDb,
    mask: &[u8],
    head: &[u8],
) -> Result<Vec<(String, crate::mask_fit::PosedClearance)>> {
    pose_clearances_rim(db, mask, head, 0.0)
}

/// As [`pose_clearances`], not measuring where the head shows through an
/// opening (`mask_fit::posed_clearance_rim`).
pub fn pose_clearances_rim(
    db: &mut ResourceDb,
    mask: &[u8],
    head: &[u8],
    rim_zone: f64,
) -> Result<Vec<(String, crate::mask_fit::PosedClearance)>> {
    use crate::skeleton_animation::{Clip, Rig};
    let bind_pose = db.read(HEAD_BIND_POSE)?;
    let rig = Rig::parse(&bind_pose, &db.read(PLAYER_SKELETON)?)?;
    let bind = rig.rest();
    let mut out = vec![(
        "bind pose".to_owned(),
        crate::mask_fit::posed_clearance_rim(mask, head, &bind_pose, &bind, &bind, rim_zone)?,
    )];
    for name in POSE_CLIPS {
        let path = format!("animations/single/weapons/{name}");
        let clip = Clip::parse(
            &db.read(&format!("{path}.b-spline"))?,
            &db.read(&format!("{path}.bones_names"))?,
        )
        .with_context(|| path.clone())?;
        ensure!(
            clip.animation_type == 0,
            "{path}: animation type {}; only whole-body clips are checked",
            clip.animation_type
        );
        let end = clip.duration();
        let mut worst: Option<crate::mask_fit::PosedClearance> = None;
        for k in 0..POSE_SAMPLES {
            let time = end * k as f32 / (POSE_SAMPLES - 1) as f32;
            let pose = rig.pose(&clip, time);
            let c = crate::mask_fit::posed_clearance_rim(
                mask, head, &bind_pose, &bind, &pose, rim_zone,
            )?;
            if worst.is_none_or(|w| c.min < w.min) {
                worst = Some(c);
            }
        }
        out.push(((*name).to_owned(), worst.context("no poses sampled")?));
    }
    Ok(out)
}

/// Texels of the 1024² diffuse kept around the balaclava's UVs, so the
/// smaller mips still see the cloth's surroundings as before.
const ATLAS_MARGIN: f64 = 16.0;
/// Sizes of the cropped maps (width, height). The island is about 790 x 560
/// texels of the 1024² diffuse: its width keeps every texel (the knit's ribs
/// run across it), its height loses about a tenth. The donor's 1:2
/// diffuse:normal ratio stays.
const ATLAS_DIFFUSE: [u32; 2] = [1024, 512];
const ATLAS_NORMAL: [u32; 2] = [2048, 1024];
const VERTEX: usize = 48;

fn vertex_uvs(mask: &[u8]) -> Result<impl Iterator<Item = (usize, [f32; 2])> + '_> {
    let count = crate::mask_fit::read_u32(mask, 8)?;
    ensure!(mask.len() >= 12 + count * VERTEX, "truncated mask vertices");
    let f = move |at: usize| f32::from_le_bytes(mask[at..at + 4].try_into().unwrap());
    Ok((0..count).map(move |i| {
        let at = 12 + i * VERTEX;
        (at, [f(at + 40), f(at + 44)])
    }))
}

/// The rectangle of the atlas the mask's UVs cover, widened by
/// `ATLAS_MARGIN` and put on the 1/1024 grid.
pub(crate) fn atlas_crop(mask: &[u8]) -> Result<crate::outfit_026::AtlasCrop> {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (_, uv) in vertex_uvs(mask)? {
        for k in 0..2 {
            lo[k] = lo[k].min(f64::from(uv[k]));
            hi[k] = hi[k].max(f64::from(uv[k]));
        }
    }
    ensure!(
        lo.iter().chain(&hi).all(|c| (0.0..=1.0).contains(c)),
        "the mask's UVs leave the atlas: {lo:?}..{hi:?}"
    );
    let grid = |c: f64, up: bool| {
        let t = c * 1024.0 + if up { ATLAS_MARGIN } else { -ATLAS_MARGIN };
        (if up { t.ceil() } else { t.floor() }).clamp(0.0, 1024.0) / 1024.0
    };
    Ok(crate::outfit_026::AtlasCrop {
        uv_min: [grid(lo[0], false), grid(lo[1], false)],
        uv_max: [grid(hi[0], true), grid(hi[1], true)],
        diffuse: ATLAS_DIFFUSE,
        normal: ATLAS_NORMAL,
    })
}

/// Moves every UV into the cropped maps: the float pair and the sub-texel
/// fraction kept in the tangent's and binormal's top bytes.
pub(crate) fn remap_to_crop(mask: &mut [u8], crop: &crate::outfit_026::AtlasCrop) -> Result<()> {
    let uvs: Vec<_> = vertex_uvs(mask)?.collect();
    for (at, uv) in uvs {
        for k in 0..2 {
            let scaled = ((f64::from(uv[k]) - crop.uv_min[k]) / (crop.uv_max[k] - crop.uv_min[k])) as f32;
            ensure!((0.0..=1.0).contains(&scaled), "UV outside the crop at vertex byte {at}");
            mask[at + 40 + 4 * k..at + 44 + 4 * k].copy_from_slice(&scaled.to_le_bytes());
            mask[at + 35 + 4 * k] = crate::armor::quantized_uv_fraction(scaled);
        }
    }
    Ok(())
}

pub fn build(source: &Path, db: &Path, output: &Path) -> Result<()> {
    let mut manifest: ArmorPortManifest =
        serde_json::from_str(include_str!("../ports/bandit-stk2-to-026g.json"))?;
    manifest
        .model
        .target_models
        .retain(|path, _| path == "models/ported/bandit_026/mask.skinned_model");
    let donor = source
        .join("resources/exported/models/character/human/bandit/bandit_01/bandit_01.skinned_model");
    let textures = source.join("resources/sources/textures/character/human/bandit/bandit_01");
    let mut record = crate::armor::build_bundle(&manifest, db, &donor, None, None, output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Bandit-026",
    };
    let files = &mut record.files;
    let mut target = ResourceDb::open(db)?;
    // The donor head sits higher and further forward on its skeleton than the
    // native one, so move the balaclava onto the native eyes and wrap it.
    let head_root = "models/character/human/base/base_character_head.skinned_model/render";
    let bind_pose = target.read(&format!("{head_root}/bind_pose"))?;
    ensure!(
        bind_pose == target.read(&manifest.model.bind_pose_template)?,
        "native head and converted mask use different bind poses"
    );
    let mask_path = "models/ported/bandit_026/mask.skinned_model/render/mask/converted_model";
    let mut mask = fs::read(overlay.join(mask_path))?;
    // Stock masks have no per-helmet variant or hide flag; a helmet that
    // cannot be worn over a mask takes the mask slot itself
    // (`additional_model_slots`). Every other helmet must cover the cloth.
    let shells = helmets_worn_over_masks(&mut target, &bind_pose)?;
    let fit = crate::mask_fit::fit_to_native_head_under(
        &mut mask,
        &target.read(HEAD_MODEL)?,
        &bind_pose,
        &shells,
    )?;
    // The cloth moves as the skin under it: the donor's weights followed
    // the chest too, and the face showed through the cheek as the head turned.
    let head = target.read(HEAD_MODEL)?;
    let follow = crate::mask_fit::follow_head_weights(&mut mask, &head)?;
    println!(
        "Balaclava skin: {} vertices take the head's (Head, Neck), {} had chest or clavicle weights; farthest from the head {:.1} mm",
        follow.changed,
        follow.off_head_bones,
        follow.max_distance * 1000.0
    );
    let (mut worst, mut face) = (f64::INFINITY, f64::INFINITY);
    for (clip, c) in pose_clearances(&mut target, &mask, &head)? {
        worst = worst.min(c.min);
        face = face.min(c.face_min);
        ensure!(
            c.min >= POSED_CLEARANCE,
            "the face shows through the balaclava in {clip}: {:.2} mm at {:?} ({} head vertices through)",
            c.min * 1000.0,
            c.at,
            c.through
        );
    }
    println!(
        "  least gap over the head in {} clips x {POSE_SAMPLES} poses: {:.2} mm (cheeks and jaw {:.2} mm)",
        POSE_CLIPS.len(),
        worst * 1000.0,
        face * 1000.0
    );
    // The balaclava samples about a quarter of the donor's attachment atlas;
    // the rest is other bandit gear. Its maps keep only that part.
    let crop = atlas_crop(&mask)?;
    remap_to_crop(&mut mask, &crop)?;
    println!(
        "Balaclava atlas: UV {:?}..{:?}, maps resampled to {:?} diffuse and {:?} normal",
        crop.uv_min, crop.uv_max, crop.diffuse, crop.normal
    );
    out.raw(mask_path, &mask, files)?;
    // The fit refines the cloth over the skull; the surface's properties
    // carry its counts.
    let props_path = mask_path.replace("/converted_model", "/export_properties");
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
    println!(
        "Fitted balaclava: moved {:?} m, tucked {} vertices onto the neck, pushed {} up to {:.1} mm, pulled {} under helmets up to {:.1} mm",
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
    out.refit_bounds(manifest.model.target_models.iter(), files)?;
    // The donor's own face skin is not ported: the balaclava sits on the
    // native head like the stock masks.
    // The balaclava is part of the donor's attachment texture.
    out.cloth_material(
        &mut target,
        "bandit_026",
        "attach",
        &textures.join("bandit_01_attach"),
        crate::outfit_026::Finish::CLOTH,
        Some(&crop),
        files,
    )?;
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected indexed dictionary")
    };
    let mut registration = vec![];
    let mut strings = BTreeMap::new();
    let mut item_configs = vec![];
    for (id, slot, category, model, part, name) in
        [(1810, 1, "mask", "mask", "mask", "Bandit balaclava")]
    {
        let template = if category == "mask" {
            "gameplay/items/armour/mask/bm_mask_1.options".to_owned()
        } else {
            format!("gameplay/items/armour/{category}/blackmarket_{category}_1.options")
        };
        let piece = Piece {
            id,
            slot,
            category,
            model,
            part,
            name,
            template,
            icon: [id - 1810 + 2, 6],
        };
        let (path, mut c, mut registration_entry) = add_piece(
            &mut target,
            items,
            "bandit_026",
            &piece,
            "Bandit balaclava from the cancelled S.T.A.L.K.E.R. 2.",
            &mut strings,
        )?;
        if category == "mask" {
            // Owner's spec (2026-10-16): a tier 8 Black Market mask, sold at
            // the faction's last rank by the server's rule (rank = item level,
            // at most the top rank), 1.0 kg, armour 30, isolation 15: between
            // the Smaragd gas mask (edge_resp_3: level 8, 20/65, 1.0 kg) and
            // the Assault mask (renesanse_mask_2: level 9, 40/5, 1.5 kg).
            for key in ["item_level", "match_making_level"] {
                c.root
                    .set_path(&format!("parameters.{key}"), V::Integer(8))?;
            }
            c.root.set_path("parameters.weight", V::Float(1.0))?;
            c.root.set_path("parameters.armor", V::Float(0.3))?;
            c.root.set_path("parameters.ano_armor", V::Float(0.15))?;
            for (hit, value) in [
                ("ranged", 0.3),
                ("slash", 0.3),
                ("explosive", 0.3),
                ("radiation", 0.15),
                ("shock", 0.15),
                ("fire", 0.15),
                ("toxic", 0.15),
            ] {
                c.root.set_path(
                    &format!("hit_params.face.hit_types.{hit}.armor"),
                    V::Float(value),
                )?;
            }
            // Armour, isolation and weight as the item card shows them.
            for (prop, value) in [(6, "30"), (8, "15"), (15, "1.0")] {
                set_card_prop(&mut c, prop, value)?;
            }
            let extra =
                json!({"weight":1.0,"item_level":8,"match_making_level":8,"sellers":[[2,5]]});
            for (k, v) in extra.as_object().unwrap() {
                registration_entry[k] = v.clone();
            }
            // Its own price: the template's (bm_mask_1) is 4460.
            items
                .iter_mut()
                .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
                .context("the balaclava's dictionary entry")?
                .set_path("base_cost", V::Integer(10_000))?;
        }
        out.config(&path, &c, files)?;
        item_configs.push(c.root.clone());
        registration.push(registration_entry);
    }
    out.config("gameplay/db_static_dictionaries", &dictionary, files)?;
    // Every language the client ships, each in its own words.
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
    for item in &item_configs {
        check_item_texts(item, &views)?;
    }
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
        serde_json::to_vec_pretty(&registration)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!("Built Bandit balaclava: {} resources", record.files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigValue, NamedValue};

    fn mask_with_uvs(uvs: &[[f32; 2]]) -> Vec<u8> {
        let mut mask = vec![3, 0, 0, 0, 0, 0, 0, 0];
        mask.extend((uvs.len() as u32).to_le_bytes());
        for uv in uvs {
            let mut v = [0_u8; 48];
            v[40..44].copy_from_slice(&uv[0].to_le_bytes());
            v[44..48].copy_from_slice(&uv[1].to_le_bytes());
            mask.extend(v);
        }
        mask
    }

    #[test]
    fn the_atlas_crop_keeps_the_island_and_its_margin() {
        let mut mask = mask_with_uvs(&[[0.25, 0.5], [0.75, 0.875], [0.5, 0.6]]);
        let crop = atlas_crop(&mask).unwrap();
        assert_eq!(crop.uv_min, [240.0 / 1024.0, 496.0 / 1024.0]);
        assert_eq!(crop.uv_max, [784.0 / 1024.0, 912.0 / 1024.0]);
        remap_to_crop(&mut mask, &crop).unwrap();
        let uvs: Vec<_> = vertex_uvs(&mask).unwrap().map(|(_, uv)| uv).collect();
        // The same atlas point before and after: old = min + new * size.
        for (old, new) in [[0.25_f32, 0.5], [0.75, 0.875], [0.5, 0.6]].iter().zip(&uvs) {
            for k in 0..2 {
                let back = crop.uv_min[k] + f64::from(new[k]) * (crop.uv_max[k] - crop.uv_min[k]);
                assert!((back - f64::from(old[k])).abs() < 1e-6);
            }
        }
        // The fraction bytes follow the new UVs.
        assert_eq!(mask[12 + 35], crate::armor::quantized_uv_fraction(uvs[0][0]));
        assert_eq!(mask[12 + 39], crate::armor::quantized_uv_fraction(uvs[0][1]));
    }

    #[test]
    fn the_balaclava_is_named_and_described_in_every_client_language() {
        let named = |name: &str, value: ConfigValue| NamedValue {
            name: name.into(),
            value,
        };
        // As add_piece names them for the mask.
        let item = ConfigValue::Named(vec![named(
            "ui_desc",
            ConfigValue::Named(vec![named(
                "text_descriptions",
                ConfigValue::Named(vec![
                    named(
                        "name",
                        ConfigValue::String("st_bandit_026_mask_name".into()),
                    ),
                    named(
                        "description",
                        ConfigValue::String("st_bandit_026_mask_desc".into()),
                    ),
                ]),
            )]),
        )]);
        let roots: Vec<(&str, ConfigValue)> = LANGUAGES
            .iter()
            .map(|&language| {
                let mut root =
                    ConfigValue::Named(vec![named("strings", ConfigValue::Named(vec![]))]);
                for (key, text) in &TEXTS {
                    set_string(&mut root, key, text.get(language).unwrap()).unwrap();
                }
                (language, root)
            })
            .collect();
        let views: Vec<(&str, &ConfigValue)> = roots.iter().map(|(l, r)| (*l, r)).collect();
        check_item_texts(&item, &views).unwrap();
        let russian = crate::localization_026::string_of(&roots[2].1, "st_bandit_026_mask_name")
            .unwrap()
            .unwrap();
        assert_eq!(russian, "Бандитская балаклава");
        for (key, text) in &TEXTS {
            for other in [text.russian, text.ukrainian, text.german, text.polish] {
                assert_ne!(other, text.english, "{key} is untranslated");
            }
        }
    }
}
