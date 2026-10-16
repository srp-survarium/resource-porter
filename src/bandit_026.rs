// SPDX-License-Identifier: GPL-3.0-or-later
//! STK2 Bandit balaclava retargeted to the native v0.26g player skeleton.
//!
//! Only the balaclava ships (owner's decision, 2026-10-15); the jacket, jeans,
//! hands and shoes stay in the port manifest but are not built.
use crate::outfit_026::{Piece, Writer, add_piece, set_card_prop};
use crate::weapon::parse_config;
use crate::{ArmorPortManifest, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

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
    use crate::skeleton_animation::{Clip, Rig};
    let bind_pose = db.read(HEAD_BIND_POSE)?;
    let rig = Rig::parse(&bind_pose, &db.read(PLAYER_SKELETON)?)?;
    let bind = rig.rest();
    let mut out = vec![(
        "bind pose".to_owned(),
        crate::mask_fit::posed_clearance(mask, head, &bind_pose, &bind, &bind)?,
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
            let c = crate::mask_fit::posed_clearance(mask, head, &bind_pose, &bind, &pose)?;
            if worst.is_none_or(|w| c.min < w.min) {
                worst = Some(c);
            }
        }
        out.push(((*name).to_owned(), worst.context("no poses sampled")?));
    }
    Ok(out)
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
        files,
    )?;
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected indexed dictionary")
    };
    let mut registration = vec![];
    let mut strings = BTreeMap::new();
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
        registration.push(registration_entry);
    }
    out.config("gameplay/db_static_dictionaries", &dictionary, files)?;
    out.localize(&mut target, &strings, files)?;
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
