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
    let fit = crate::mask_fit::fit_to_native_head(
        &mut mask,
        &target.read(&format!(
            "{head_root}/black_market_01_head_s/converted_model"
        ))?,
        &bind_pose,
    )?;
    out.raw(mask_path, &mask, files)?;
    println!(
        "Fitted balaclava: moved {:?} m, tucked {} vertices onto the neck, pushed {} up to {:.1} mm",
        fit.offset,
        fit.tucked_vertices,
        fit.pushed_vertices,
        fit.max_push * 1000.0
    );
    out.refit_bounds(manifest.model.target_models.iter(), files)?;
    // The donor's own face skin is not ported: the balaclava sits on the
    // native head like the stock masks.
    // The balaclava is part of the donor's attachment texture.
    out.cloth_material(
        &mut target,
        "bandit_026",
        "attach",
        &textures.join("bandit_01_attach"),
        0.98,
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
