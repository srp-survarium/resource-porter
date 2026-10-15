// SPDX-License-Identifier: GPL-3.0-or-later
//! STK2 Bandit geometry retargeted to the native v0.26g player skeleton.
use crate::manifest::{BuiltFile, TextureCompression};
use crate::weapon::{add_localization_strings, parse_config, record_file, write_generated_config};
use crate::{ArmorPortManifest, BinaryConfig, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}
fn config(out: &Path, path: &str, c: &BinaryConfig, files: &mut Vec<BuiltFile>) -> Result<()> {
    files.retain(|f| f.path != path);
    write_generated_config(out, path, c, "generated:Bandit-026", files)
}
fn raw(out: &Path, path: &str, b: &[u8], files: &mut Vec<BuiltFile>) -> Result<()> {
    let p = crate::archive::safe_join(out, path)?;
    fs::create_dir_all(p.parent().unwrap())?;
    fs::write(p, b)?;
    files.retain(|f| f.path != path);
    files.push(record_file(path, "converted:Bandit-026", b));
    Ok(())
}

pub fn build(source: &Path, db: &Path, output: &Path) -> Result<()> {
    let manifest: ArmorPortManifest =
        serde_json::from_str(include_str!("../ports/bandit-stk2-to-026g.json"))?;
    let donor = source
        .join("resources/exported/models/character/human/bandit/bandit_01/bandit_01.skinned_model");
    let textures = source.join("resources/sources/textures/character/human/bandit/bandit_01");
    let mut record = crate::armor::build_bundle(&manifest, db, &donor, None, None, output)?;
    let mut hud = manifest.clone();
    hud.model.bind_pose_template="models/character/human/black_market/black_market_01_hand_hud.skinned_model/render/bind_pose".into();
    hud.model.target_models = BTreeMap::from([(
        "models/ported/bandit_026/gloves_hud.skinned_model".into(),
        manifest.model.target_models["models/ported/bandit_026/gloves.skinned_model"].clone(),
    )]);
    let hud_dir = output.join("hud-conversion");
    let hud_record = crate::armor::build_bundle(&hud, db, &donor, None, None, &hud_dir)?;
    for f in hud_record.files {
        let dest = output.join("overlay").join(&f.path);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(hud_dir.join("overlay").join(&f.path), dest)?;
        record.files.push(f);
    }
    let overlay = output.join("overlay");
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
        &target.read(&format!("{head_root}/black_market_01_head_s/converted_model"))?,
        &bind_pose,
    )?;
    raw(&overlay, mask_path, &mask, files)?;
    println!(
        "Fitted balaclava: moved {:?} m, tucked {} vertices onto the neck, pushed {} up to {:.1} mm",
        fit.offset,
        fit.tucked_vertices,
        fit.pushed_vertices,
        fit.max_push * 1000.0
    );
    // Bounds must describe the retargeted vertices, not the borrowed boot.
    for (root, model) in manifest
        .model
        .target_models
        .iter()
        .chain(hud.model.target_models.iter())
    {
        let mut all_min = [f32::INFINITY; 3];
        let mut all_max = [f32::NEG_INFINITY; 3];
        for surf in model.surfaces.keys() {
            let b = fs::read(overlay.join(format!("{root}/render/{surf}/converted_model")))?;
            let n = u32::from_le_bytes(b[8..12].try_into()?) as usize;
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for i in 0..n {
                for k in 0..3 {
                    let at = 12 + i * 48 + k * 4;
                    let v = f32::from_le_bytes(b[at..at + 4].try_into()?);
                    ensure!(v.is_finite(), "nonfinite converted position");
                    min[k] = min[k].min(v);
                    max[k] = max[k].max(v);
                }
            }
            let p = format!("{root}/render/{surf}/export_properties");
            let mut c = BinaryConfig::parse(&fs::read(overlay.join(&p))?)?;
            set_bounds(&mut c, min, max)?;
            config(&overlay, &p, &c, files)?;
            for k in 0..3 {
                all_min[k] = all_min[k].min(min[k]);
                all_max[k] = all_max[k].max(max[k]);
            }
        }
        let p = format!("{root}/render/export_properties");
        let mut c = BinaryConfig::parse(&fs::read(overlay.join(&p))?)?;
        set_bounds(&mut c, all_min, all_max)?;
        config(&overlay, &p, &c, files)?;
    }
    // The donor's own face skin is not ported: the balaclava sits on the
    // native head like the stock masks.
    for part in ["attach", "torso", "legs"] {
        let mut mat = parse_config(
            &mut target,
            "material_instances/characters/human/renesanse_tank/renesanse_tank_torso.material",
            "material",
        )?;
        for (role, suffix, compression, srgb) in [
            ("diffuse", "", TextureCompression::Dxt1, true),
            ("normal", "_nmap", TextureCompression::Dxt5Normal, false),
        ] {
            let p = format!("textures/ported/bandit_026/{part}_{role}");
            let dest = overlay.join(format!("{p}.dds"));
            fs::create_dir_all(dest.parent().unwrap())?;
            crate::armor::convert_texture(
                &textures.join(format!("bandit_01_{part}{suffix}.tga")),
                &dest,
                compression,
            )?;
            let bytes = crate::duty_026::texture_dds(&fs::read(&dest)?, &p, srgb)?;
            raw(&overlay, &format!("{p}.dds"), &bytes, files)?;
            let mut opts = parse_config(
                &mut target,
                "textures/character/human/renesanse_tank/renesanse_tank_torso.options",
                "texture options",
            )?;
            opts.root
                .set_path("options.srgb", V::Integer(i32::from(srgb)))?;
            opts.root.set_path(
                "options.format",
                s(if srgb { "DXT1" } else { "NORMAL_MAP" }),
            )?;
            config(&overlay, &format!("{p}.options"), &opts, files)?;
            mat.root.set_path(
                &format!("material.g_stage.effect.texture_{role}.value"),
                s(format!("ported/bandit_026/{part}_{role}")),
            )?;
        }
        let roughness = if part == "torso" {
            0.85
        } else {
            0.98
        };
        for (key, value) in [
            ("constant_fresnel_min", 0.008),
            ("constant_fresnel_max", 0.008),
            ("constant_roughness_min", roughness),
            ("constant_roughness_max", roughness),
        ] {
            mat.root.set_path(
                &format!("material.g_stage.effect.{key}.value"),
                V::Float(value),
            )?;
        }
        mat.root.set_path(
            "material.g_stage.effect.constant_specular_color_multiplier.value",
            V::Float3([0.025; 3]),
        )?;
        config(
            &overlay,
            &format!("material_instances/ported/bandit_026/{part}.material"),
            &mat,
            files,
        )?;
    }
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected indexed dictionary")
    };
    let mut registration = vec![];
    let mut strings = BTreeMap::new();
    for (id, slot, category, model, part, name) in [
        (1810, 1, "mask", "mask", "mask", "Bandit balaclava"),
        (1811, 2, "torso", "torso", "torso", "Bandit leather jacket"),
        (1812, 4, "legs", "legs", "legs", "Bandit jeans"),
        (1813, 5, "gloves", "gloves", "gloves", "Bandit bare hands"),
        (1814, 6, "boots", "boots", "boots", "Bandit shoes"),
    ] {
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(id))),
            "item ID {id} occupied"
        );
        let template = if category == "mask" {
            "gameplay/items/armour/mask/bm_mask_1.options".to_owned()
        } else {
            format!("gameplay/items/armour/{category}/blackmarket_{category}_1.options")
        };
        let path = format!("gameplay/items/armour/{category}/bandit_026.options");
        let mut item = items
            .iter()
            .find(|v| v.named("cfg_name") == Some(&s(&template)))
            .context("native item missing")?
            .clone();
        item.set_path("dict_id", V::Integer(id))?;
        item.set_path("cfg_name", s(&path))?;
        item.set_path("name", s(format!("bandit_026_{category}")))?;
        items.push(item);
        let mut c = parse_config(&mut target, &template, "equipment")?;
        let model_path = format!("ported/bandit_026/{model}");
        for key in ["data", "data_full", "data_short", "data_long"] {
            if key == "data" || c.root.named(key).is_some() {
                c.root
                    .set_path(&format!("{key}.base_model"), s(&model_path))?;
                c.root.set_path(&format!("{key}.part_name"), s(part))?;
            }
        }
        c.root
            .set_path("parameters.camo_available", V::Boolean(false))?;
        c.root
            .set_path("parameters.decal_available", V::Boolean(false))?;
        c.root
            .set_path("data.additional_models", V::Indexed(vec![]))?;
        c.root
            .set_path("data.additional_model_slots", V::Indexed(vec![]))?;
        if slot == 5 {
            c.root
                .set_path("data.base_model_hud", s("ported/bandit_026/gloves_hud"))?;
            c.root.set_path("data.part_name_hud", s("gloves"))?;
        }
        let mut registration_entry = json!({"dict_id":id,"slot":slot,"additional_slots":Vec::<u8>::new(),"name":name,"template":template,"config":path});
        if category == "mask" {
            // Owner's spec (2026-10-15): a tier 3 Black Market mask sold from
            // rank 2 (Fighter), 0.5 kg, armour 10, isolation 5.
            for key in ["item_level", "match_making_level"] {
                c.root
                    .set_path(&format!("parameters.{key}"), V::Integer(3))?;
            }
            c.root.set_path("parameters.weight", V::Float(0.5))?;
            c.root.set_path("parameters.armor", V::Float(0.1))?;
            c.root.set_path("parameters.ano_armor", V::Float(0.05))?;
            for (hit, value) in [
                ("ranged", 0.1),
                ("slash", 0.1),
                ("explosive", 0.1),
                ("radiation", 0.05),
                ("shock", 0.05),
                ("fire", 0.05),
                ("toxic", 0.05),
            ] {
                c.root.set_path(
                    &format!("hit_params.face.hit_types.{hit}.armor"),
                    V::Float(value),
                )?;
            }
            // Armour, isolation and weight as the item card shows them.
            for (prop, value) in [(6, "10"), (8, "5"), (15, "0.5")] {
                set_card_prop(&mut c, prop, value)?;
            }
            let extra = json!({"weight":0.5,"item_level":3,"match_making_level":3,"sellers":[[2,2]]});
            for (k, v) in extra.as_object().unwrap() {
                registration_entry[k] = v.clone();
            }
        }
        let key = format!("st_bandit_026_{category}");
        c.root.set_path(
            "ui_desc.icon",
            V::Indexed(vec![V::Integer(id - 1810 + 2), V::Integer(6)]),
        )?;
        c.root
            .set_path("ui_desc.text_descriptions.name", s(format!("{key}_name")))?;
        c.root.set_path(
            "ui_desc.text_descriptions.description",
            s(format!("{key}_desc")),
        )?;
        strings.insert(format!("{key}_name"), name.to_owned());
        strings.insert(format!("{key}_desc"),"Bandit outfit from the cancelled S.T.A.L.K.E.R. 2. Developer conversion preview; provisional Black Market equipment stats.".into());
        config(&overlay, &path, &c, files)?;
        registration.push(registration_entry);
    }
    config(
        &overlay,
        "gameplay/db_static_dictionaries",
        &dictionary,
        files,
    )?;
    for lang in [
        "english",
        "original",
        "russian",
        "polish",
        "german",
        "ukrainian",
    ] {
        let p = format!("localization/{lang}/localization");
        let mut c = parse_config(&mut target, &p, "localization")?;
        add_localization_strings(&mut c.root, &strings)?;
        config(&overlay, &p, &c, files)?;
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
    println!(
        "Built Bandit preview: {} resources, 5 items",
        record.files.len()
    );
    Ok(())
}
fn set_card_prop(c: &mut BinaryConfig, prop_id: i32, value: &str) -> Result<()> {
    let Some(V::Indexed(props)) = c
        .root
        .named_mut("ui_desc")
        .and_then(|ui| ui.named_mut("props_list"))
    else {
        bail!("item card has no props_list")
    };
    props
        .iter_mut()
        .find(|p| p.named("prop_id") == Some(&V::Integer(prop_id)))
        .with_context(|| format!("item card has no prop {prop_id}"))?
        .set_path("prop_value", s(value))
}
fn set_bounds(c: &mut BinaryConfig, min: [f32; 3], max: [f32; 3]) -> Result<()> {
    c.root.set_path("bounding_box.min", V::Float3(min))?;
    c.root.set_path("bounding_box.max", V::Float3(max))?;
    let center = std::array::from_fn(|k| (min[k] + max[k]) / 2.0);
    let radius = (0..3)
        .map(|k| (max[k] - center[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    if c.root.named("bounding_sphere").is_some() {
        c.root.set_path("bounding_sphere.pos", V::Float3(center))?;
        c.root
            .set_path("bounding_sphere.radius", V::Float(radius))?;
    }
    Ok(())
}
