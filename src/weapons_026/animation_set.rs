// SPDX-License-Identifier: GPL-3.0-or-later
//! A live 0.26g gun's animations from a later build (`vostok-port
//! port-animation-set`, `ports/animations-*.json`): the same states, the same
//! rig, newer clips.
//!
//! For each listed state the live config takes the donor's hands and item
//! clips, shipped under a renamed weapon folder (`weapons/<from>/...` at
//! `weapons/<to>/...`, so the live clips stay for anything else that plays
//! them), and each listed model of the gun (its scoped variants too) takes
//! the donor model's hand points of the listed names (`<finger>_idle`: where
//! the hands' IK puts the fingers), position, rotation and bone (its index
//! that model's own).
//!
//! Checked, or refused: the donor's state lists as many clips as the live
//! one, and where its clips carry sound events, the same sound lists (the
//! events index them; the live lists stay), or none live (the state takes
//! the donor's sounds); every
//! hands clip drives 0.26g's player rig bone for bone and every item clip the
//! bones of the live clip it replaces; at the idle's start the gun's parts sit in the hands where the live
//! idle has them (the placement [`super::tactical`] checks).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::json;

use super::build::Carry;
use super::donor::{ArchiveDonor, Donor, DonorBuild};
use super::donor_069d::{Donor069, NATIVE_HANDS_CLIP, Spec069, bind_pose, clip_bones};
use super::tactical::{
    REST_ROTATION, REST_TRANSLATION, bones_names, bspline, part_distances, placement, sound_list,
    strings,
};
use super::{clips, get, string_of};
use crate::skeleton_animation::{Clip, Rig};
use crate::weapon::{parse_config, prepare_empty_output};
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};

/// `ports/animations-*.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationPort {
    pub format_version: u32,
    pub id: String,
    pub donor_build: DonorBuild,
    pub guns: Vec<AnimatedGun>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimatedGun {
    /// The live item config.
    pub config: String,
    /// Its donor item (a config path, or 0.69d0's `weapons.<item>`).
    pub donor_item: String,
    /// The states whose clips are the donor's.
    pub states: Vec<String>,
    /// The weapon clip folder and its new name (`weapons/<from>/` ->
    /// `weapons/<to>/`).
    pub folder: [String; 2],
    /// Model roots (`models/....skinned_model`) whose
    /// `render/export_properties` take the donor model's locators below.
    #[serde(default)]
    pub models: Vec<String>,
    /// Locator names taken from the donor model.
    #[serde(default)]
    pub locators: Vec<String>,
    /// Gun parts whose place in the new idle may differ from the live
    /// idle's, each with why; reported.
    #[serde(default)]
    pub accept: BTreeMap<String, String>,
}

impl AnimationPort {
    pub fn load(path: &Path) -> Result<Self> {
        let port: Self = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        ensure!(
            port.format_version == 1,
            "{}: format_version 1",
            path.display()
        );
        Ok(port)
    }
}

/// The bundle of an animation port: the configs, clips and model hand
/// points, `port-manifest.json` (for `pack from-bundle`) and
/// `animation-set.json` (what each gun took).
pub fn build(
    port: &AnimationPort,
    donor: &Path,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<serde_json::Value> {
    prepare_empty_output(output)?;
    match port.donor_build {
        DonorBuild::V069d0 => {
            let spec = Spec069 {
                format_version: 1,
                guns: BTreeMap::new(),
            };
            let mut d = Donor069::open(donor, &spec, target_layers, &output.join("work"))?;
            build_with(port, &mut d, target_layers, output)
        }
        build => {
            let mut d = ArchiveDonor::open(donor, build)?;
            build_with(port, &mut d, target_layers, output)
        }
    }
}

fn locator_list(cfg: &V) -> &[V] {
    match cfg.named("locators") {
        Some(V::Indexed(l)) => l,
        _ => &[],
    }
}

fn locator_name(l: &V) -> Option<&str> {
    string_of(l.named("name"))
}

fn build_with<D: Donor>(
    port: &AnimationPort,
    donor: &mut D,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<serde_json::Value> {
    ensure!(
        donor.build() == port.donor_build,
        "the donor is {:?}, the port's {:?}",
        donor.build(),
        port.donor_build
    );
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let hands = clip_bones(&target.read(NATIVE_HANDS_CLIP)?)?;
    let mut carry = Carry::new(donor, &mut target, &overlay, "converted:animations".into());
    let mut report = vec![];
    for gun in &port.guns {
        let what = || format!("{} ({})", gun.config, gun.donor_item);
        let live = parse_config(carry.target(), &gun.config, "live weapon")?.root;
        let theirs = carry.donor().item_config(&gun.donor_item)?;
        let [from, to] = &gun.folder;
        let (from, to) = (format!("/weapons/{from}/"), format!("/weapons/{to}/"));
        let renamed = |name: &str| -> Result<String> {
            ensure!(name.contains(&from), "{name} is not under {from}");
            Ok(name.replacen(&from, &to, 1))
        };
        let read_live = |name: &str, carry: &mut Carry<'_, D>| -> Result<Clip> {
            Clip::parse(
                &carry.target().read(&bspline(name))?,
                &carry.target().read(&bones_names(name))?,
            )
            .with_context(|| name.to_owned())
        };
        let mut cfg = live.clone();
        let mut shipped: BTreeSet<String> = BTreeSet::new();
        let mut new_idle: Option<(Clip, Clip)> = None;
        for state in &gun.states {
            let ours = get(&live, &format!("states.{state}"))
                .with_context(what)?
                .clone();
            let donor_state = get(&theirs, &format!("states.{state}"))
                .with_context(|| format!("{}: the donor has no {state}", what()))?
                .clone();
            // The donor's clips' sound events index its lists: those must be
            // the live ones.
            let mut sounding = false;
            for key in ["user_animations", "animations"] {
                for name in strings(get(&donor_state, key))? {
                    let bytes = carry.donor().read(&bspline(&name))?;
                    sounding |= clips::event_names(&bytes)?
                        .iter()
                        .any(|n| n == "sound_events");
                }
            }
            if sounding {
                for view in ["first_view", "third_view"] {
                    let (live_list, donor_list) =
                        (sound_list(&ours, view)?, sound_list(&donor_state, view)?);
                    if live_list == donor_list {
                        continue;
                    }
                    // A state the live gun plays silent takes the donor's
                    // sounds; one with other sounds is refused.
                    ensure!(
                        live_list.is_empty(),
                        "{}: {state}'s {view} sounds are not the live ones",
                        what()
                    );
                    for (k, n) in &donor_list {
                        carry.sound(*k, n).with_context(what)?;
                    }
                    let path = format!("states.{state}.sounds");
                    let mut sounds = get(&donor_state, "sounds").cloned().context("sounds")?;
                    if let Some(V::Named(_)) = get(&live, &path) {
                        // keep the live table's other keys
                        let mut merged = get(&live, &path).cloned().expect("named");
                        if let V::Named(fields) = &mut sounds {
                            for f in fields.iter() {
                                merged.set_path(&f.name, f.value.clone())?;
                            }
                        }
                        sounds = merged;
                    }
                    cfg.set_path(&path, sounds)?;
                    break;
                }
            }
            let mut idle_pair = (None, None);
            for (key, drives_hands) in [("user_animations", true), ("animations", false)] {
                let live_list = strings(get(&ours, key))?;
                let donor_list = strings(get(&donor_state, key))?;
                ensure!(
                    live_list.len() == donor_list.len(),
                    "{}: {state}.{key} lists {} clips live, {} in the donor",
                    what(),
                    live_list.len(),
                    donor_list.len()
                );
                let mut names = vec![];
                for (ours_name, name) in live_list.iter().zip(&donor_list) {
                    let at = renamed(name)?;
                    let bytes = carry.donor().read(&bspline(name))?;
                    let bones = carry.donor().read(&bones_names(name))?;
                    let clip = Clip::parse(&bytes, &bones).with_context(|| name.clone())?;
                    if drives_hands {
                        ensure!(
                            clip.bones == hands,
                            "{}: {name} drives other bones than 0.26g's player rig",
                            what()
                        );
                    } else {
                        let live_clip = read_live(ours_name, &mut carry)?;
                        let a: BTreeSet<&String> = clip.bones.iter().collect();
                        let b: BTreeSet<&String> = live_clip.bones.iter().collect();
                        ensure!(a == b, "{}: {name} drives {a:?}, {ours_name} {b:?}", what());
                    }
                    if shipped.insert(at.clone()) {
                        ensure!(
                            !carry.target().contains(&bspline(&at)),
                            "{at}: the target has it"
                        );
                        carry.raw(&bspline(&at), &bytes)?;
                        carry.raw(&bones_names(&at), &bones)?;
                    }
                    if state == "idle" {
                        if drives_hands {
                            idle_pair.0.get_or_insert(clip);
                        } else {
                            idle_pair.1.get_or_insert(clip);
                        }
                    }
                    names.push(V::String(at));
                }
                cfg.set_path(&format!("states.{state}.{key}"), V::Indexed(names))?;
            }
            if let (Some(h), Some(i)) = idle_pair {
                new_idle = Some((h, i));
            }
        }

        // The gun's rig and its place in the hands.
        let model = string_of(get(&live, "object.model")).context("object.model")?;
        let skeleton = string_of(get(&live, "object.skeleton")).context("object.skeleton")?;
        let live_bind = carry
            .target()
            .read(&format!("models/{model}.skinned_model/render/bind_pose"))?;
        let rig = Rig::parse(
            &live_bind,
            &carry
                .target()
                .read(&format!("animations/skeletons/{skeleton}.skeleton"))?,
        )?;
        let donor_model = string_of(get(&theirs, "object.model")).context("donor object.model")?;
        let mut notes = vec![];
        if let Some((h, i)) = &new_idle {
            let lh = strings(get(&live, "states.idle.user_animations"))?;
            let li = strings(get(&live, "states.idle.animations"))?;
            let (live_h, live_i) = (
                read_live(&lh[0], &mut carry)?,
                read_live(&li[0], &mut carry)?,
            );
            let ours = placement(&rig, (h, 0.0), (i, 0.0));
            let was = placement(&rig, (&live_h, 0.0), (&live_i, 0.0));
            let mut far = vec![];
            for (bone, dt, dr) in part_distances(&rig, &ours, &was) {
                if dt <= REST_TRANSLATION && dr <= REST_ROTATION {
                    continue;
                }
                match gun.accept.get(&bone) {
                    Some(why) => notes.push(format!(
                        "the new idle holds {bone} {dt:.4} m / {dr:.4} rad from the live one, accepted: {why}"
                    )),
                    None => far.push(format!("{bone} {dt:.4} m / {dr:.4} rad")),
                }
            }
            ensure!(
                far.is_empty(),
                "{}: the new idle holds parts elsewhere than the live one: {}",
                what(),
                far.join(", ")
            );
        }

        // The hand points.
        if !gun.locators.is_empty() {
            let donor_props = carry.donor().config(&format!(
                "models/{donor_model}.skinned_model/render/export_properties"
            ))?;
            let wanted: Vec<&V> = gun
                .locators
                .iter()
                .map(|n| {
                    locator_list(&donor_props)
                        .iter()
                        .find(|l| locator_name(l) == Some(n))
                        .with_context(|| format!("{}: the donor model has no {n}", what()))
                })
                .collect::<Result<_>>()?;
            for root in &gun.models {
                let path = format!("{root}/render/export_properties");
                let bones: Vec<String> =
                    bind_pose(&carry.target().read(&format!("{root}/render/bind_pose"))?)?
                        .into_iter()
                        .map(|(n, _)| n)
                        .collect();
                let mut props = parse_config(carry.target(), &path, "live model")?.root;
                let Some(V::Indexed(list)) = props.named_mut("locators") else {
                    bail!("{path}: no locators");
                };
                for w in &wanted {
                    let name = locator_name(w).expect("named");
                    let slot = list
                        .iter_mut()
                        .find(|l| locator_name(l) == Some(name))
                        .with_context(|| format!("{path} has no {name}"))?;
                    for key in ["position", "rotation", "bone_name"] {
                        let v = w
                            .named(key)
                            .with_context(|| format!("the donor's {name} has no {key}"))?;
                        slot.set_path(key, v.clone())?;
                    }
                    // The bone's index in this model's own bind pose.
                    let bone = string_of(w.named("bone_name")).context("bone_name")?;
                    let idx = bones
                        .iter()
                        .position(|b| b == bone)
                        .with_context(|| format!("{root} has no bone {bone}"))?;
                    slot.set_path("bone_idx", V::Integer(i32::try_from(idx)?))?;
                }
                let bytes = BinaryConfig { root: props }.to_bytes()?;
                carry.raw(&path, &bytes)?;
                notes.push(format!("{path}: {} hand points the donor's", wanted.len()));
            }
        }
        carry.config(&gun.config, cfg)?;
        report.push(json!({
            "config": gun.config,
            "donor_item": gun.donor_item,
            "states": gun.states,
            "clips": shipped,
            "notes": notes,
        }));
    }
    let (files, reused, warnings) = carry.finish();
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": port.id,
            "donor_build": port.donor_build,
            "files": files,
            "target": target_layers,
            "items": port.guns.iter().map(|g| g.config.clone()).collect::<Vec<_>>(),
            "reused_from_target": reused,
            "notes": {"files": warnings},
        }))?,
    )?;
    let report = json!({ "guns": report });
    fs::write(
        output.join("animation-set.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
