// SPDX-License-Identifier: GPL-3.0-or-later
//! STK2 Novice stalker (`neutral_01_novice`): his knit cap and his field
//! jacket, retargeted to the native v0.26g player as a helmet and a torso.
//!
//! The donor's head surface is his face and the cap in one; only the cap is
//! kept and worn on the native head ([`crate::hat_fit`]), as the Bandit
//! balaclava is. The jacket is the donor's torso surface without his hands
//! (the gloves slot draws those), pushed out over every stock trousers'
//! waist and over the native neck and the masks' hems at the collar
//! ([`crate::body_fit`]), in one variant per glove class with the sleeves
//! over the short cuffs and inside the gauntlets ([`crate::limb_fit`]).
use crate::body_fit::{BodyFit, Cover};
use crate::limb_fit::{Layer, LimbFit};
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::manifest::{BuiltFile, TargetModelSpec};
use crate::mask_fit::{Mesh, Shell, parse_mesh};
use crate::outfit_026::{Piece, Writer, add_piece, keep_triangles, s, set_card_prop};
use crate::weapon::parse_config;
use crate::{ArmorPortManifest, BinaryConfig, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const ROOT: &str = "models/ported/novice_026";
const HAT: &str = "models/ported/novice_026/hat.skinned_model/render/hat/converted_model";
const TORSO: &str = "models/ported/novice_026/torso.skinned_model/render/torso/converted_model";
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";
const PLAYER_SKELETON: &str = "animations/skeletons/scavengers_01.skeleton";

/// The cap's part of the head texture: the knit band at the top, above the
/// eyes' and the face's UVs (v from 0.37 down).
const CAP_UV_MAX_V: f64 = 0.375;
/// The donor's hands with their fingerless gloves: the upper right of the
/// torso texture, on the arms. The jacket and its sleeves keep the rest.
fn is_glove(corners: &[([f64; 3], [f64; 2]); 3]) -> bool {
    let (mut u, mut v, mut x) = (0.0, 0.0, 0.0);
    for (p, uv) in corners {
        u += uv[0] / 3.0;
        v += uv[1] / 3.0;
        x += p[0].abs() / 3.0;
    }
    u > 0.75 && v < 0.32 && x > 0.3
}

/// Item ids, free in stock and in every released Skhid patch (12-22);
/// 1800-1820 are taken or reserved by other ports.
pub const CAP_ID: i32 = 1821;
pub const JACKET_ID: i32 = 1822;
/// Free cells of the equipment atlas over patch 22's lobby movie: rows 1-9
/// of column 6 are the exoskeletons' and the balaclava's, 10-17 are
/// reserved for the 0.69d masks and the closed Keeper helmet.
pub const CAP_ICON: [i32; 2] = [18, 6];
pub const JACKET_ICON: [i32; 2] = [19, 6];

pub const TEXTS: [(&str, Text); 4] = [
    (
        "st_novice_026_helmet_name",
        Text {
            english: "Novice's knit cap",
            russian: "Вязаная шапка новичка",
            ukrainian: "В'язана шапка новачка",
            german: "Strickmütze des Neulings",
            polish: "Czapka nowicjusza",
        },
    ),
    (
        "st_novice_026_helmet_desc",
        Text {
            english: "A novice stalker's knit cap from the cancelled S.T.A.L.K.E.R. 2.",
            russian: "Вязаная шапка сталкера-новичка из отменённого S.T.A.L.K.E.R. 2.",
            ukrainian: "В'язана шапка сталкера-новачка зі скасованого S.T.A.L.K.E.R. 2.",
            german: "Strickmütze eines Neulings-Stalkers aus dem eingestellten S.T.A.L.K.E.R. 2.",
            polish: "Dziergana czapka stalkera nowicjusza z anulowanego S.T.A.L.K.E.R. 2.",
        },
    ),
    (
        "st_novice_026_torso_name",
        Text {
            english: "Novice's field jacket",
            russian: "Полевая куртка новичка",
            ukrainian: "Польова куртка новачка",
            german: "Feldjacke des Neulings",
            polish: "Kurtka polowa nowicjusza",
        },
    ),
    (
        "st_novice_026_torso_desc",
        Text {
            english: "A novice stalker's field jacket over a sweater, from the cancelled S.T.A.L.K.E.R. 2.",
            russian: "Полевая куртка сталкера-новичка поверх свитера, из отменённого S.T.A.L.K.E.R. 2.",
            ukrainian: "Польова куртка сталкера-новачка поверх светра, зі скасованого S.T.A.L.K.E.R. 2.",
            german: "Feldjacke eines Neulings-Stalkers über einem Pullover, aus dem eingestellten S.T.A.L.K.E.R. 2.",
            polish: "Kurtka polowa stalkera nowicjusza na swetrze, z anulowanego S.T.A.L.K.E.R. 2.",
        },
    ),
];

/// Room the collar leaves around the native neck and the masks' hems in
/// the bind pose: the neck turns with the head inside a collar that follows
/// the chest, and the nape comes back into it when the head tilts back.
/// (Lowering the donor's high collar instead, by 3 to 5 cm, left its
/// narrow rim on the wider native nape: more of the neck came through it
/// in the clips, not less.)
const COLLAR_CLEARANCE: f64 = 0.008;
/// One item's numbers. Armour and isolation are fractions; the card shows
/// them in percent.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub level: i32,
    pub armor: f32,
    pub isolation: f32,
    pub weight: f32,
    pub drop_weight: i32,
    pub cost: i32,
}

/// The owner's numbers (2026-10-19): level 2, in no faction ("they
/// shouldn't belong to any group"); drops between level 1 (150) and level 3
/// (130). The cap as proposed; the jacket with 25 armour and one more
/// pocket capacity (below).
pub const CAP_STATS: Stats = Stats {
    level: 2,
    armor: 0.10,
    isolation: 0.10,
    weight: 0.3,
    drop_weight: 140,
    cost: 2000,
};
pub const JACKET_STATS: Stats = Stats {
    level: 2,
    armor: 0.25,
    isolation: 0.10,
    weight: 4.5,
    drop_weight: 140,
    cost: 3300,
};
/// No faction: `parameters.faction` 0, as the configs of the gear no
/// faction sells (the PP-19-01 Vityaz, the MP-27M Lancaster).
const NO_FACTION: i32 = 0;
/// Pocket capacity +1 (`st_modify_quick_slot_size`: `pocket_slot_size`
/// +1), the default modification native torsos carry for it (the
/// Reinforced padded jacket, the OZK-18KB and Zubr UM-4 vests).
const POCKET_CAPACITY_PLUS_ONE: i32 = 1170;

/// Clips the cap is checked through (as the balaclava is).
pub use crate::bandit_026::POSE_CLIPS;
const POSE_SAMPLES: usize = 24;
/// The least gap the cap keeps over the head and over every mask in every
/// sampled pose (m).
pub const POSED_CLEARANCE: f64 = 0.0005;

/// Sleeves lie over the cuffs of wrist-length gloves.
const OVER_CUFFS: LimbFit<'static> = LimbFit {
    axes: &[("LeftForeArm", "LeftHand"), ("RightForeArm", "RightHand")],
    layer: Layer::Outside,
    clearance: 0.004,
    ease: 0.3,
    max_radius: 0.12,
    distal_reach: 0.0,
};
/// Sleeves go into gauntlets.
const INTO_GAUNTLETS: LimbFit<'static> = LimbFit {
    axes: &[("LeftForeArm", "LeftHand"), ("RightForeArm", "RightHand")],
    layer: Layer::Inside,
    clearance: 0.003,
    ease: 0.5,
    max_radius: 0.12,
    distal_reach: 0.0,
};

/// How far the sleeve reaches into the gauntlet whose opening is deepest
/// along the forearm (m).
const GAUNTLET_OVERLAP: f64 = 0.03;

/// The jacket without the sleeve triangles wholly beyond
/// [`GAUNTLET_OVERLAP`] past the deepest opening of `gloves` along each
/// forearm (ForeArm to Hand); and that distance from the ForeArm bone (m).
fn trim_into_gauntlets(
    jacket: &[u8],
    gloves: &[(String, Vec<u8>)],
    bind_pose: &[u8],
) -> Result<(Vec<u8>, f64)> {
    let bones = crate::mask_fit::bone_origins(bind_pose)?;
    let bone = |n: &str| bones.get(n).copied().with_context(|| format!("no {n} bone"));
    let mut axes = vec![];
    let mut deepest = 0.0_f64;
    for side in ["Left", "Right"] {
        let (a, b) = (bone(&format!("{side}ForeArm"))?, bone(&format!("{side}Hand"))?);
        let d = crate::mask_fit::sub(b, a);
        let len = crate::mask_fit::dot(d, d).sqrt();
        let dir = d.map(|v| v / len);
        let along = move |p: [f64; 3]| crate::mask_fit::dot(crate::mask_fit::sub(p, a), dir);
        for (_, model) in gloves {
            let mesh = Shell::parse("glove", model)?.mesh;
            let opening = mesh
                .positions
                .iter()
                .filter(|p| p[0] * a[0] > 0.0)
                .map(|&p| along(p))
                .fold(f64::INFINITY, f64::min);
            ensure!(opening.is_finite(), "a glove without a {side} hand");
            deepest = deepest.max(opening);
        }
        axes.push((a[0].signum(), along));
    }
    let cut = deepest + GAUNTLET_OVERLAP;
    let kept = keep_triangles(jacket, |corners| {
        !axes.iter().any(|(side, along)| {
            corners
                .iter()
                .all(|(p, _)| p[0] * side > 0.0 && along(*p) > cut)
        })
    })?;
    Ok((kept, cut))
}

/// The game's view of the resources: the stock archive with the released
/// patches over it.
fn game_view(db: &Path, patches: &[PathBuf]) -> Result<ResourceDb> {
    let mut layers = vec![db.to_path_buf()];
    let mut sorted = patches.to_vec();
    sorted.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('_').next())
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0)
    });
    layers.extend(sorted);
    ResourceDb::open_layers(&layers)
}

/// Every surface of an item config's model and its additional models:
/// (label, converted model, the model's bind pose).
fn item_surfaces(view: &mut ResourceDb, path: &str) -> Result<Vec<(String, Vec<u8>, Vec<u8>)>> {
    let config = parse_config(view, path, "equipment")?;
    let data = config.root.named("data").context("item without data")?;
    let mut parts = vec![data];
    if let Some(V::Indexed(extra)) = data.named("additional_models") {
        parts.extend(extra);
    }
    let mut out = vec![];
    for part in parts {
        let (Some(V::String(model)), Some(V::String(surface))) =
            (part.named("base_model"), part.named("part_name"))
        else {
            bail!("{path}: model part without base_model/part_name");
        };
        let root = format!("models/{model}.skinned_model/render");
        out.push((
            format!("{path} {surface}"),
            view.read(&format!("{root}/{surface}/converted_model"))?,
            view.read(&format!("{root}/bind_pose"))?,
        ));
    }
    Ok(out)
}

/// The item configs of an armour category in the game's view, ported ones
/// included unless they are this port's own.
fn category_configs(view: &ResourceDb, category: &str) -> Vec<String> {
    let prefix = format!("gameplay/items/armour/{category}/");
    view.paths_with_prefix(&prefix)
        .filter(|p| p.ends_with(".options") && !p.contains("novice_026"))
        .map(str::to_owned)
        .collect()
}

/// The surfaces of every model a torso item is drawn with, by class: legs
/// (every variant), gloves by `model_type`. Stock and released items alike.
fn worn_with(
    view: &mut ResourceDb,
    category: &str,
    keys: &[&str],
) -> Result<BTreeMap<String, Vec<(String, Vec<u8>)>>> {
    let mut by_class: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    for path in category_configs(view, category) {
        let c = parse_config(view, &path, "equipment")?;
        let class = match c.root.named("data").and_then(|d| d.named("model_type")) {
            Some(V::String(t)) => t.clone(),
            _ => "data_full".to_owned(),
        };
        let mut models = BTreeSet::new();
        for key in keys {
            if let Some(V::String(m)) = c.root.named(key).and_then(|d| d.named("base_model")) {
                models.insert(m.clone());
            }
        }
        for model in models {
            let root = format!("models/{model}.skinned_model/render/");
            let surfaces: Vec<String> = view
                .paths_with_prefix(&root)
                .filter(|p| p.ends_with("/converted_model"))
                .map(str::to_owned)
                .collect();
            for surface in surfaces {
                let bytes = view.read(&surface)?;
                by_class
                    .entry(class.clone())
                    .or_default()
                    .push((format!("{path} {surface}"), bytes));
            }
        }
    }
    Ok(by_class)
}

/// The surface's properties carry its counts.
fn set_counts(out: &Writer<'_>, path: &str, model: &[u8], files: &mut Vec<BuiltFile>) -> Result<()> {
    let props_path = path.replace("/converted_model", "/export_properties");
    let mut props = BinaryConfig::parse(&fs::read(out.overlay.join(&props_path))?)?;
    let vertices = crate::mask_fit::read_u32(model, 8)?;
    let indices = crate::mask_fit::read_u32(model, 12 + vertices * 48 + 8)?;
    for (key, value) in [
        ("source.vertices", vertices),
        ("source.indices", indices),
        ("source.primitives", indices / 3),
    ] {
        props
            .root
            .set_path(key, V::Integer(i32::try_from(value)?))?;
    }
    out.config(&props_path, &props, files)
}

/// Writes `<part><suffix>.skinned_model`: the converted model's files with
/// `geometry`.
fn variant(
    out: &Writer<'_>,
    manifest: &ArmorPortManifest,
    part: &str,
    suffix: &str,
    geometry: &[u8],
    models: &mut BTreeMap<String, TargetModelSpec>,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let from = format!("{ROOT}/{part}.skinned_model");
    let to = format!("{ROOT}/{part}{suffix}.skinned_model");
    if !suffix.is_empty() {
        let sources: Vec<String> = files
            .iter()
            .filter_map(|f| f.path.strip_prefix(&format!("{from}/")).map(str::to_owned))
            .collect();
        for rest in sources {
            let bytes = fs::read(out.overlay.join(format!("{from}/{rest}")))?;
            out.raw(&format!("{to}/{rest}"), &bytes, files)?;
        }
    }
    let surface = format!("{to}/render/{part}/converted_model");
    out.raw(&surface, geometry, files)?;
    set_counts(out, &surface, geometry, files)?;
    models.insert(to, manifest.model.target_models[&from].clone());
    Ok(())
}

fn set_stats(c: &mut BinaryConfig, stats: &Stats, zones: &[&str]) -> Result<()> {
    for key in ["item_level", "match_making_level"] {
        c.root
            .set_path(&format!("parameters.{key}"), V::Integer(stats.level))?;
    }
    c.root
        .set_path("parameters.faction", V::Integer(NO_FACTION))?;
    c.root
        .set_path("parameters.weight", V::Float(stats.weight))?;
    c.root
        .set_path("parameters.armor", V::Float(stats.armor))?;
    c.root
        .set_path("parameters.ano_armor", V::Float(stats.isolation))?;
    c.root
        .set_path("parameters.drop_weight", V::Integer(stats.drop_weight))?;
    for zone in zones {
        for (hit, value) in [
            ("ranged", stats.armor),
            ("slash", stats.armor),
            ("explosive", stats.armor),
            ("radiation", stats.isolation),
            ("shock", stats.isolation),
            ("fire", stats.isolation),
            ("toxic", stats.isolation),
        ] {
            c.root.set_path(
                &format!("hit_params.{zone}.hit_types.{hit}.armor"),
                V::Float(value),
            )?;
        }
    }
    let percent = |f: f32| format!("{}", (f * 100.0).round() as i32);
    for (prop, value) in [
        (6, percent(stats.armor)),
        (8, percent(stats.isolation)),
        (15, format!("{:.1}", stats.weight)),
    ] {
        set_card_prop(c, prop, &value)?;
    }
    Ok(())
}

/// The hit zones a config protects.
fn zones(c: &BinaryConfig) -> Vec<String> {
    match c.root.named("hit_params") {
        Some(V::Named(z)) => z.iter().map(|n| n.name.clone()).collect(),
        _ => vec![],
    }
}

pub fn build(source: &Path, db: &Path, patches: &[PathBuf], output: &Path) -> Result<()> {
    let manifest: ArmorPortManifest =
        serde_json::from_str(include_str!("../ports/novice-stk2-to-026g.json"))?;
    let donor = source.join(
        "resources/exported/models/character/human/neutral/neutral_01_novice/neutral_01_novice.skinned_model",
    );
    let textures = source.join("resources/sources/textures/character/human/neutral/neutral_01_novice");
    let mut record = crate::armor::build_bundle(&manifest, db, &donor, None, None, output)?;
    let overlay = output.join("overlay");
    let out = Writer {
        overlay: &overlay,
        tag: "Novice-026",
    };
    let files = &mut record.files;
    // Everything the items edit or are fitted to is read as players have
    // it: the stock archive with the released patches over it. Shared files
    // (the dictionary, the localizations) are then the released ones plus
    // this port's entries, and a pack made over the patches holds only those.
    let mut target = game_view(db, patches)?;
    let bind_pose = target.read(HEAD_BIND_POSE)?;
    ensure!(
        bind_pose == target.read(&manifest.model.bind_pose_template)?,
        "native head and converted models use different bind poses"
    );
    let head_bytes = target.read(crate::bandit_026::HEAD_MODEL)?;
    let head = parse_mesh(&head_bytes, 40, "native head")?;
    let bones = crate::mask_fit::bone_origins(&bind_pose)?;
    let bone = |n: &str| bones.get(n).copied().with_context(|| format!("no {n} bone"));

    // --- The cap ---
    let donor_head = fs::read(overlay.join(HAT))?;
    let whole = parse_mesh(&donor_head, 48, "donor head")?;
    let uv = |i: usize| -> [f64; 2] {
        let at = 12 + i * 48 + 40;
        [0, 4].map(|o| f64::from(f32::from_le_bytes(donor_head[at + o..at + o + 4].try_into().unwrap())))
    };
    // Its parts: the cap, the face skin and the two eyes are separate pieces.
    let parts = mesh_parts(&whole);
    let count = parts.iter().max().map_or(0, |m| m + 1);
    let (mut v_min, mut v_max) = (vec![f64::INFINITY; count], vec![f64::NEG_INFINITY; count]);
    let mut size = vec![0_usize; count];
    for (i, &p) in parts.iter().enumerate() {
        v_min[p] = v_min[p].min(uv(i)[1]);
        v_max[p] = v_max[p].max(uv(i)[1]);
        size[p] += 1;
    }
    let cap_parts: BTreeSet<usize> = (0..count).filter(|&p| v_max[p] <= CAP_UV_MAX_V).collect();
    ensure!(cap_parts.len() == 1, "expected one cap piece in the head, found {}", cap_parts.len());
    let face_part = (0..count)
        .filter(|p| !cap_parts.contains(p))
        .max_by_key(|&p| size[p])
        .context("no face in the donor head")?;
    let eyes: Vec<usize> = (0..count)
        .filter(|p| *p != face_part && !cap_parts.contains(p))
        .collect();
    ensure!(eyes.len() == 2, "expected two eyes in the donor head, found {}", eyes.len());
    let face: Vec<[f64; 3]> = (0..whole.positions.len())
        .filter(|&i| parts[i] == face_part)
        .map(|i| whole.positions[i])
        .collect();
    let eye_points: Vec<f64> = (0..whole.positions.len())
        .filter(|&i| eyes.contains(&parts[i]))
        .map(|i| whole.positions[i][1])
        .collect();
    let donor_eyes_y = eye_points.iter().sum::<f64>() / eye_points.len() as f64;
    let native_eyes_y = (bone("LeftEye")?[1] + bone("RightEye")?[1]) / 2.0;
    let cap_part = *cap_parts.iter().next().unwrap();
    let in_cap = |c: &[([f64; 3], [f64; 2]); 3]| c.iter().all(|(_, uv)| uv[1] <= CAP_UV_MAX_V);
    let mut cap = keep_triangles(&donor_head, in_cap)?;
    ensure!(
        crate::mask_fit::read_u32(&cap, 8)? == size[cap_part],
        "the cap's UV band holds more than its own piece"
    );
    // Every mask the cap is worn over, in the game's view (stock and
    // released ports).
    let mut masks = vec![];
    let mut mask_models = vec![];
    for path in category_configs(&target, "mask") {
        for (label, model, model_bind) in item_surfaces(&mut target, &path)? {
            masks.push(Shell::parse(label.clone(), &model)?);
            mask_models.push((label, model, model_bind));
        }
    }
    ensure!(!masks.is_empty(), "no mask in the game's view");
    let fit = crate::hat_fit::fit_hat(
        &mut cap,
        &face,
        [0.0, native_eyes_y - donor_eyes_y, 0.0],
        &head_bytes,
        &bind_pose,
        &masks,
    )?;
    println!(
        "Cap: moved {:?} m with the donor face (its median distance to the native head {:.1} mm), pushed {} vertices up to {:.1} mm",
        fit.offset.map(|v| (v * 1e4).round() / 1e4),
        fit.face_median * 1000.0,
        fit.pushed_vertices,
        fit.max_push * 1000.0
    );
    for s in &fit.shells {
        println!(
            "  {}: {} vertices outside (farthest {:.1} mm) -> {} ({:.1} mm)",
            s.label,
            s.outside_before,
            s.depth_before * 1000.0,
            s.outside_after,
            s.depth_after * 1000.0
        );
        ensure!(s.outside_after == 0, "{} shows through the cap", s.label);
    }
    let follow = crate::mask_fit::follow_head_weights(&mut cap, &head_bytes)?;
    println!(
        "  skin: {} vertices take the head's, farthest from it {:.1} mm",
        follow.changed,
        follow.max_distance * 1000.0
    );
    check_cap_poses(&mut target, &cap, &head_bytes, &bind_pose, &mask_models)?;
    // Same quality, less size: the cap samples only the knit band of the
    // head texture, so its maps are cut to the power-of-two window its UVs
    // use, texel for texel.
    let head_texture = textures.join("neutral_01_novice_head");
    let size = crate::outfit_026::tga_size(&head_texture.with_extension("tga"))?;
    let cap_crop = crate::outfit_026::uv_crop(&cap, size)?;
    if let Some(crop) = cap_crop {
        crate::outfit_026::remap_uvs(&mut cap, crop, size)?;
        println!("  cap maps cut to {crop:?} of {size:?}");
    }
    out.raw(HAT, &cap, files)?;
    set_counts(&out, HAT, &cap, files)?;

    // --- The jacket ---
    let donor_torso = fs::read(overlay.join(TORSO))?;
    let jacket = keep_triangles(&donor_torso, |c| !is_glove(c))?;
    let mut jacket = jacket;
    // Its trunk over every trousers' waist.
    let legs = worn_with(&mut target, "legs", &["data"])?;
    let leg_meshes: Vec<(String, Mesh)> = legs
        .into_values()
        .flatten()
        .map(|(l, b)| Ok((l.clone(), Shell::parse(l, &b)?.mesh)))
        .collect::<Result<_>>()?;
    let hem = parse_mesh(&jacket, 48, "jacket")?
        .positions
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min);
    let axis = [
        [0.0, 0.0, bone("Hip")?[2]],
        bone("Hip")?,
        bone("Spin")?,
        bone("Spin_1")?,
        bone("Chest")?,
        bone("Neck")?,
        bone("Head")?,
    ];
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
    let r = crate::body_fit::fit(&mut jacket, &covers, &waist)?;
    println!(
        "Jacket over {} trousers surfaces: moved {} vertices, up to {:.1} mm",
        covers.len(),
        r.moved,
        r.max_move * 1000.0
    );
    report_covers(&r.covers);
    // The collar over the native neck and the masks' hems.
    let neck_meshes: Vec<(String, Mesh)> = std::iter::once(("native head".to_owned(), head.clone()))
        .chain(masks.iter().map(|m| (m.label.clone(), m.mesh.clone())))
        .collect();
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
    let r = crate::body_fit::fit(&mut jacket, &covers, &collar)?;
    println!(
        "Collar over the native neck and {} mask surfaces: moved {} vertices, up to {:.1} mm",
        masks.len(),
        r.moved,
        r.max_move * 1000.0
    );
    report_covers(&r.covers);
    // One variant per glove class.
    let gloves = worn_with(&mut target, "gloves", &["data"])?;
    let mut models = BTreeMap::new();
    models.insert(
        format!("{ROOT}/hat.skinned_model"),
        manifest.model.target_models[&format!("{ROOT}/hat.skinned_model")].clone(),
    );
    for (class, suffix, spec) in [
        ("data_full", "", &OVER_CUFFS),
        ("data_short", "_short", &INTO_GAUNTLETS),
        ("data_long", "_long", &INTO_GAUNTLETS),
    ] {
        let mut fitted = jacket.clone();
        let class_gloves = gloves
            .get(class)
            .with_context(|| format!("no {class} gloves"))?;
        let r = crate::limb_fit::fit(&mut fitted, class_gloves, &bind_pose, spec)?;
        println!(
            "Sleeves {} {} {class} gloves: moved {} vertices, up to {:.1} mm",
            if spec.layer == Layer::Outside { "over" } else { "into" },
            class_gloves.len(),
            r.moved,
            r.max_move * 1000.0
        );
        if spec.layer == Layer::Inside {
            // As stock gauntlet variants do, the sleeve ends inside the
            // gauntlet: what is deep in it can only show through it.
            let (kept, cut_at) = trim_into_gauntlets(&fitted, class_gloves, &bind_pose)?;
            println!(
                "  sleeves end {:.0} mm inside the deepest-opening gauntlet ({:.0} mm along the forearm)",
                GAUNTLET_OVERLAP * 1000.0,
                cut_at * 1000.0
            );
            fitted = kept;
        }
        variant(&out, &manifest, "torso", suffix, &fitted, &mut models, files)?;
    }
    out.refit_bounds(&models, files)?;

    // --- Materials: matte cloth, as stock knit and canvas ---
    // The jacket's UVs use its whole texture (only the hands' block, inside
    // it, is left), so its maps stay whole.
    for (part, stem, crop) in [
        ("hat", "neutral_01_novice_head", cap_crop),
        ("torso", "neutral_01_novice_torso", None),
    ] {
        out.cloth_material_cropped(
            &mut target,
            "novice_026",
            part,
            &textures.join(stem),
            crop,
            crate::outfit_026::Finish::CLOTH,
            files,
        )?;
    }

    // --- Items ---
    let mut dictionary =
        parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?;
    let Some(V::Indexed(items)) = dictionary.root.named_mut("items_dict") else {
        bail!("expected indexed dictionary")
    };
    let mut registration = vec![];
    let mut strings = BTreeMap::new();
    let mut item_configs = vec![];
    for (id, slot, category, model, template, icon, stats, name) in [
        (
            CAP_ID,
            1,
            "helmet",
            "hat",
            "gameplay/items/armour/helmet/scavenger_helmet_5.options",
            CAP_ICON,
            CAP_STATS,
            TEXTS[0].1.english,
        ),
        (
            JACKET_ID,
            2,
            "torso",
            "torso",
            "gameplay/items/armour/torso/scavenger_torso_1.options",
            JACKET_ICON,
            JACKET_STATS,
            TEXTS[2].1.english,
        ),
    ] {
        let piece = Piece {
            id,
            slot,
            category,
            model,
            part: model,
            name,
            template: template.to_owned(),
            icon,
        };
        let description = TEXTS
            .iter()
            .find(|(k, _)| *k == format!("st_novice_026_{category}_desc"))
            .context("description")?
            .1
            .english;
        let (path, mut c, mut entry) =
            add_piece(&mut target, items, "novice_026", &piece, description, &mut strings)?;
        if category == "torso" {
            for (key, suffix) in [("data_short", "_short"), ("data_long", "_long")] {
                ensure!(c.root.named(key).is_some(), "torso template has no {key}");
                c.root.set_path(
                    &format!("{key}.base_model"),
                    s(format!("ported/novice_026/torso{suffix}")),
                )?;
            }
        }
        let zones = zones(&c);
        ensure!(!zones.is_empty(), "{template} protects no hit zone");
        let zones: Vec<&str> = zones.iter().map(String::as_str).collect();
        set_stats(&mut c, &stats, &zones)?;
        if category == "torso" {
            // The template's own default modification (the Quilted jacket's
            // ammo bag, 1050) stays; the pocket capacity is added to it.
            let mut mods = match c.root.named("default_modifications") {
                Some(V::Indexed(m)) => m.clone(),
                _ => vec![],
            };
            ensure!(
                !mods.contains(&V::Integer(POCKET_CAPACITY_PLUS_ONE)),
                "the torso template already has pocket capacity"
            );
            mods.push(V::Integer(POCKET_CAPACITY_PLUS_ONE));
            c.root.set_path("default_modifications", V::Indexed(mods))?;
        }
        items
            .iter_mut()
            .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
            .context("the new dictionary entry")?
            .set_path("base_cost", V::Integer(stats.cost))?;
        // The server's rule sells it (faction and level); no override.
        for k in ["name", "template", "config", "slot", "additional_slots"] {
            entry.as_object_mut().unwrap().remove(k);
        }
        out.config(&path, &c, files)?;
        item_configs.push(c.root.clone());
        registration.push(entry);
    }
    out.config("gameplay/db_static_dictionaries", &dictionary, files)?;
    ensure!(
        strings.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == TEXTS.iter().map(|(key, _)| *key).collect(),
        "the items' string keys changed: {:?}",
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
    println!("Built Novice cap and jacket: {} resources", record.files.len());
    Ok(())
}

/// The native neck under a jacket's collar through the cap's clips: for
/// each clip, the most neck points outside the collar (by more than 1 mm) in
/// one pose and how far. `jacket` is a converted torso of the player's rig.
/// Prints and returns the worst; it is a comparison, not a gate (stock
/// collars are checked the same way for the reference).
pub fn collar_poses(db: &Path, patches: &[PathBuf], jackets: &[(String, Vec<u8>, Vec<u8>)]) -> Result<()> {
    use crate::skeleton_animation::{Clip, Rig};
    let mut target = game_view(db, patches)?;
    let bind_pose = target.read(HEAD_BIND_POSE)?;
    let rig = Rig::parse(&bind_pose, &target.read(PLAYER_SKELETON)?)?;
    let bind = rig.rest();
    let names: Vec<String> = rig.bones.iter().map(|b| b.name.clone()).collect();
    let head_bytes = target.read(crate::bandit_026::HEAD_MODEL)?;
    let head = parse_mesh(&head_bytes, 40, "native head")?;
    let head_posed = crate::hat_fit::Posed::new(&head, &head_bytes, &bind_pose, &names)?;
    let bones = crate::mask_fit::bone_origins(&bind_pose)?;
    // The neck below the jaw, and the nape behind the Head bone up to it;
    // not the jaw, whose corners stand over the collar.
    let (neck_bone, head_bone) = (bones["Neck"], bones["Head"]);
    let in_neck = |p: [f64; 3]| {
        p[1] < neck_bone[1] + 0.05 || (p[2] < head_bone[2] && p[1] < head_bone[1])
    };
    // The neck below the jaw, not the throat in front (the chin hangs over
    // the collar there).
    let neck: Vec<usize> = (0..head.positions.len())
        .filter(|&i| in_neck(head.positions[i]))
        .collect();
    let mut poses = vec![("bind pose".to_owned(), bind.clone())];
    for name in POSE_CLIPS {
        let path = format!("animations/single/weapons/{name}");
        let clip = Clip::parse(
            &target.read(&format!("{path}.b-spline"))?,
            &target.read(&format!("{path}.bones_names"))?,
        )?;
        let end = clip.duration();
        for k in 0..POSE_SAMPLES {
            poses.push(((*name).to_owned(), rig.pose(&clip, end * k as f32 / (POSE_SAMPLES - 1) as f32)));
        }
    }
    let axis_bones = ["Hip", "Spin", "Spin_1", "Chest", "Neck", "Head"].map(|n| rig.index(n).unwrap());
    let skin = |p: &crate::hat_fit::Posed<'_>, pose: &[crate::mask_fit::Frame]| -> Result<Vec<[f64; 3]>> {
        let skins: Vec<crate::mask_fit::Skin> = p
            .skins
            .iter()
            .map(|s| s.iter().map(|&(b, w)| (p.bones[usize::from(b)], w)).collect())
            .collect();
        crate::mask_fit::skin_positions(&p.mesh.positions, &skins, &bind, pose)
    };
    for (label, model, model_bind) in jackets {
        let mesh = parse_mesh(model, 48, "jacket")?;
        let posed = crate::hat_fit::Posed::new(&mesh, model, model_bind, &names)?;
        let mut per_clip: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        for (clip, pose) in &poses {
            let mut axis: Vec<[f64; 3]> = axis_bones
                .iter()
                .map(|&b| pose[b].1.map(f64::from))
                .collect();
            axis.sort_by(|a, b| a[1].total_cmp(&b[1]));
            let spec = BodyFit {
                axis: &axis,
                y_range: (axis[3][1] - 0.05, axis[5][1]),
                max_x: 0.2,
                clearance: 0.0,
                cap: 1.0,
                by_part: false,
                max_forward: 0.5,
            };
            let garment = skin(&posed, pose)?;
            let head_now = skin(&head_posed, pose)?;
            let points: Vec<[f64; 3]> = neck.iter().map(|&i| head_now[i]).collect();
            let (n, depth) = crate::body_fit::outside(&spec, &garment, &mesh.triangles, &points, 0.001);
            let e = per_clip.entry(clip.clone()).or_default();
            if n > e.0 {
                *e = (n, e.1.max(depth));
            } else {
                e.1 = e.1.max(if n > 0 { depth } else { 0.0 });
            }
        }
        let total: usize = per_clip.values().map(|v| v.0).sum();
        let worst = per_clip
            .iter()
            .max_by_key(|(_, v)| v.0)
            .map(|(c, v)| format!("{c}: {} points, {:.1} mm", v.0, v.1 * 1000.0))
            .unwrap_or_default();
        println!(
            "{label}: neck points out of the collar, summed over the clips' worst poses: {total}; worst {worst}"
        );
    }
    Ok(())
}

fn report_covers(covers: &[crate::body_fit::CoverReport]) {
    for c in covers {
        if c.outside_before == 0 && c.left_points == 0 {
            continue;
        }
        println!(
            "  {}: {} points outside -> {} (farthest {:.1} mm){}",
            c.label,
            c.outside_before,
            c.outside_after,
            c.depth_after * 1000.0,
            if c.left_points > 0 {
                format!(
                    "; left {} points of parts standing out up to {:.0} mm",
                    c.left_points,
                    c.left_depth * 1000.0
                )
            } else {
                String::new()
            }
        );
    }
}

/// The connected piece of each vertex (seam copies welded).
fn mesh_parts(mesh: &Mesh) -> Vec<usize> {
    let (weld, neighbours) = crate::mask_fit::weld_vertices(mesh);
    let mut part = vec![usize::MAX; neighbours.len()];
    let mut next = 0;
    for start in 0..neighbours.len() {
        if part[start] != usize::MAX {
            continue;
        }
        part[start] = next;
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            for &u in &neighbours[v] {
                if part[u] == usize::MAX {
                    part[u] = next;
                    stack.push(u);
                }
            }
        }
        next += 1;
    }
    weld.iter().map(|&w| part[w]).collect()
}

/// The cap over the head and over every mask, skinned through the lobby's
/// and the match's clips.
fn check_cap_poses(
    db: &mut ResourceDb,
    cap: &[u8],
    head: &[u8],
    bind_pose: &[u8],
    masks: &[(String, Vec<u8>, Vec<u8>)],
) -> Result<()> {
    use crate::skeleton_animation::{Clip, Rig};
    let rig = Rig::parse(bind_pose, &db.read(PLAYER_SKELETON)?)?;
    let bind = rig.rest();
    let names: Vec<String> = rig.bones.iter().map(|b| b.name.clone()).collect();
    let head_bone = rig.index("Head").context("no Head bone")?;
    let cap_mesh = parse_mesh(cap, 48, "cap")?;
    let head_mesh = parse_mesh(head, 40, "native head")?;
    let centre = crate::hat_fit::head_centre(&head_mesh, bind_pose)?;
    let outer = crate::hat_fit::Posed::new(&cap_mesh, cap, bind_pose, &names)?;
    let inner_meshes: Vec<(String, Mesh, &[u8], &[u8])> =
        std::iter::once(("native head".to_owned(), head_mesh.clone(), head, bind_pose))
            .chain(masks.iter().map(|(l, m, b)| {
                Ok((l.clone(), Shell::parse(l.clone(), m)?.mesh, m.as_slice(), b.as_slice()))
            }).collect::<Result<Vec<_>>>()?)
            .collect();
    let inners: Vec<(String, crate::hat_fit::Posed<'_>)> = inner_meshes
        .iter()
        .map(|(l, mesh, model, model_bind)| {
            Ok((l.clone(), crate::hat_fit::Posed::new(mesh, model, model_bind, &names)?))
        })
        .collect::<Result<_>>()?;
    let mut worst: BTreeMap<String, (f64, String)> = BTreeMap::new();
    let mut poses = vec![("bind pose".to_owned(), bind.clone())];
    for name in POSE_CLIPS {
        let path = format!("animations/single/weapons/{name}");
        let clip = Clip::parse(
            &db.read(&format!("{path}.b-spline"))?,
            &db.read(&format!("{path}.bones_names"))?,
        )
        .with_context(|| path.clone())?;
        let end = clip.duration();
        for k in 0..POSE_SAMPLES {
            let time = end * k as f32 / (POSE_SAMPLES - 1) as f32;
            poses.push(((*name).to_owned(), rig.pose(&clip, time)));
        }
    }
    for (clip, pose) in &poses {
        for (label, inner) in &inners {
            let (min, through) =
                crate::hat_fit::posed_gap(&outer, inner, centre, head_bone, &bind, pose)?;
            ensure!(
                through == 0 && min >= POSED_CLEARANCE,
                "{label} shows through the cap in {clip}: {:.2} mm, {through} vertices",
                min * 1000.0
            );
            let e = worst
                .entry(label.clone())
                .or_insert((f64::INFINITY, String::new()));
            if min < e.0 {
                *e = (min, clip.clone());
            }
        }
    }
    println!(
        "  posed: {} clips x {POSE_SAMPLES} poses and the bind pose",
        POSE_CLIPS.len()
    );
    for (label, (min, clip)) in &worst {
        println!("    {label}: least gap {:.2} mm ({clip})", min * 1000.0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigValue, NamedValue};

    #[test]
    fn the_items_are_named_and_described_in_every_client_language() {
        let named = |name: &str, value: ConfigValue| NamedValue {
            name: name.into(),
            value,
        };
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
        for category in ["helmet", "torso"] {
            let item = ConfigValue::Named(vec![named(
                "ui_desc",
                ConfigValue::Named(vec![named(
                    "text_descriptions",
                    ConfigValue::Named(vec![
                        named(
                            "name",
                            ConfigValue::String(format!("st_novice_026_{category}_name")),
                        ),
                        named(
                            "description",
                            ConfigValue::String(format!("st_novice_026_{category}_desc")),
                        ),
                    ]),
                )]),
            )]);
            check_item_texts(&item, &views).unwrap();
        }
        for (key, text) in &TEXTS {
            for other in [text.russian, text.ukrainian, text.german, text.polish] {
                assert_ne!(other, text.english, "{key} is untranslated");
            }
        }
    }

    #[test]
    fn the_glove_region_is_the_hands_only() {
        let at = |x: f64, u: f64, v: f64| ([x, 1.0, 0.0], [u, v]);
        assert!(is_glove(&[at(0.5, 0.8, 0.1), at(0.51, 0.81, 0.1), at(0.5, 0.8, 0.11)]));
        // The same texels on the chest are an attachment of the jacket.
        assert!(!is_glove(&[at(0.1, 0.8, 0.1), at(0.11, 0.81, 0.1), at(0.1, 0.8, 0.11)]));
        // The sleeve's own cuff.
        assert!(!is_glove(&[at(0.5, 0.5, 0.6), at(0.51, 0.51, 0.6), at(0.5, 0.5, 0.61)]));
    }
}
