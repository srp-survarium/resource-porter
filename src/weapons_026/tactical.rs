// SPDX-License-Identifier: GPL-3.0-or-later
//! A tactical reload for live 0.26g guns that reload with one clip
//! (`vostok-port port-tactical-reload`), the way the live vz. 58 and the
//! other 0.29a3 ports already reload: data and clips only.
//!
//! 0.26g's two-clip reload (state type 314, `pistol_weapon_core_reload_state`)
//! plays its tactical clip with rounds left and its empty clip without
//! (`get_weapon_lexeme` 0x30FE90, slot `2 x empty + 4 x crouch`), both at one
//! speed: the tactical clip's length over `reload_time` (the ctor 0x30FC50).
//! The stock AKs, SMGs and machine guns reload with the one-clip state (303).
//! A port (`ports/tactical-reload-*.json`, one donor build each) names live
//! configs and their donor items; for each the bundle holds the live config
//! with
//!
//! * state type 314 and the donor's two-clip reload: its tactical and empty
//!   clips (hands `[tactical, empty] x 4`, item `[tactical, empty]`), the pair
//!   one build made, so the empty reload lasts `reload_time` x (empty clip /
//!   tactical clip) as in that build; `reload_time` as it is;
//! * the donor's reload sound lists (its clips' sound events index them);
//!
//! and the clips (converted by the donor: 0.29a3's and 0.69d0's 12-byte event
//! headers to 0.26g's 16) and sounds 0.26g lacks. A donor clip at a path
//! 0.26g has with other bytes goes to `<path>_<donor>` (`_029a3`, `_069d0`),
//! so the live clip of that name stays for anything else that plays it.
//!
//! Checked, or the gun is refused: the hands clips drive 0.26g's player rig
//! bone for bone, the item clips the bones the gun's live reload drives, and
//! at each clip's start and end every part of the gun sits in the hands where
//! 0.26g has it at rest (the hands' `Weapon` bone, then the gun's model-space
//! frames; the gun's idle) or where its own reload has it, within
//! [`REST_TRANSLATION`] and [`REST_ROTATION`]: 0.69d0 moved some guns in the
//! hands (the static placement 0.26g's clips keep in the gun's `offset` bone
//! is its hands' `Weapon` bone's), and their reload would slide the gun at
//! its start and end. A part a port lists in `accept` (a lever, a cartridge)
//! is reported, not refused.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::json;

use super::build::Carry;
use super::donor::{ArchiveDonor, Donor, DonorBuild};
use super::donor_069d::{Donor069, NATIVE_HANDS_CLIP, Spec069, bind_pose, clip_bones};
use super::{clips, get, string_of};
use crate::skeleton_animation::{Clip, Frame, Rig, euler_to_basis, multiply, transform};
use crate::weapon::{parse_config, prepare_empty_output};
use crate::{ConfigValue as V, ResourceDb};

/// 0.26g's one-clip reload.
pub const ONE_CLIP_RELOAD: i32 = 303;
/// 0.26g's two-clip reload (`pistol_weapon_core_reload_state`).
pub const TWO_CLIP_RELOAD: i32 = 314;
/// How far a gun part may start or end the tactical clip from where 0.26g
/// has it: metres.
pub const REST_TRANSLATION: f32 = 0.005;
/// The same for its turn: radians, the angle of the rotation between them.
pub const REST_ROTATION: f32 = 0.02;

/// `ports/tactical-reload-*.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TacticalPort {
    pub format_version: u32,
    pub id: String,
    pub donor_build: DonorBuild,
    pub guns: Vec<TacticalGun>,
    /// Live one-clip guns this port leaves as they are, each with why (a
    /// record; not read).
    #[serde(default)]
    pub not_ported: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TacticalGun {
    /// The live item config.
    pub config: String,
    /// Its donor item: a config path (0.29a3) or a `weapons.<item>` key of
    /// `gameplay/game.options` (0.69d0).
    pub donor_item: String,
    /// Gun parts (bones) whose place at the tactical clip's start or end may
    /// differ from 0.26g's, each with why; reported.
    #[serde(default)]
    pub accept: BTreeMap<String, String>,
}

impl TacticalPort {
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
        let mut seen = BTreeSet::new();
        for g in &port.guns {
            ensure!(seen.insert(&g.config), "{} listed twice", g.config);
        }
        Ok(port)
    }
}

/// One converted gun, for the report.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Converted {
    pub config: String,
    pub donor_item: String,
    /// The hands and item clips the config names: tactical, empty.
    pub clips: [[String; 2]; 2],
    /// Whether the reload lists the donor's sounds (else the live ones,
    /// the same).
    pub donor_sounds: bool,
    /// The largest start/end distance of a part (not accepted) from 0.26g's:
    /// metres, radians.
    pub rest: [f32; 2],
    pub notes: Vec<String>,
}

pub(super) fn strings(v: Option<&V>) -> Result<Vec<String>> {
    match v {
        Some(V::Indexed(items)) => items
            .iter()
            .map(|i| match i {
                V::String(s) => Ok(s.clone()),
                other => bail!("expected a clip name, found {other:?}"),
            })
            .collect(),
        other => bail!("expected a list of clips, found {other:?}"),
    }
}

pub(super) fn bspline(name: &str) -> String {
    format!("{}.b-spline", name.trim_start_matches("resources/"))
}

pub(super) fn bones_names(name: &str) -> String {
    format!("{}.bones_names", name.trim_start_matches("resources/"))
}

/// A reload state's sound list of one view, as (kind, name) pairs.
pub(super) fn sound_list(state: &V, view: &str) -> Result<Vec<(i32, String)>> {
    match get(state, &format!("sounds.{view}")) {
        Some(V::Indexed(list)) => list
            .iter()
            .map(|pair| match pair {
                V::Indexed(p) => match p.as_slice() {
                    [V::Integer(k), V::String(n)] => Ok((*k, n.clone())),
                    other => bail!("a sound entry {other:?}"),
                },
                other => bail!("a sound entry {other:?}"),
            })
            .collect(),
        // No sounds: an empty table (some stock states write one) or none.
        None | Some(V::Named(_)) => Ok(vec![]),
        other => bail!("sounds.{view}: {other:?}"),
    }
}

fn sound_value(list: &[(i32, String)]) -> V {
    V::Indexed(
        list.iter()
            .map(|(k, n)| V::Indexed(vec![V::Integer(*k), V::String(n.clone())]))
            .collect(),
    )
}

/// How far from its parent a part counts as put away (a dropped magazine
/// sent off): metres.
const PUT_AWAY: f32 = 1.0;

/// The gun's parts in the hands' frame: the hands clip's `Weapon` bone
/// (where the gun hangs) at `hands_time`, then the item clip's model-space
/// frames at `item_time`.
pub(super) fn placement(
    rig: &Rig,
    (hands, hands_time): (&Clip, f32),
    (item, item_time): (&Clip, f32),
) -> Vec<Frame> {
    let (wb, wt) = match hands.bones.iter().position(|b| b == "Weapon") {
        Some(w) => {
            let v: [f32; 6] =
                std::array::from_fn(|c| hands.tracks[w].channels[c].evaluate(hands_time));
            (euler_to_basis([v[3], v[4], v[5]]), [v[0], v[1], v[2]])
        }
        None => (euler_to_basis([0.0; 3]), [0.0; 3]),
    };
    rig.pose(item, item_time)
        .into_iter()
        .map(|(basis, t)| {
            let moved = transform(t, &wb);
            (multiply(&basis, &wb), [0, 1, 2].map(|k| moved[k] + wt[k]))
        })
        .collect()
}

/// The bone two clips' poses turn most apart, at `at` and `theirs_at`, and
/// the angle (radians).
fn largest_turn(ours: &Clip, at: f32, theirs: &Clip, theirs_at: f32) -> (String, f32) {
    let mut worst = (String::new(), 0.0f32);
    for (bone, track) in ours.bones.iter().zip(&ours.tracks) {
        let Some(j) = theirs.bones.iter().position(|b| b == bone) else {
            continue;
        };
        let angles = |t: &[crate::skeleton_animation::Channel], time: f32| {
            euler_to_basis([
                t[3].evaluate(time),
                t[4].evaluate(time),
                t[5].evaluate(time),
            ])
        };
        let (a, b) = (
            angles(&track.channels, at),
            angles(&theirs.tracks[j].channels, theirs_at),
        );
        let trace: f32 = (0..3)
            .map(|r| (0..3).map(|k| a[k][r] * b[k][r]).sum::<f32>())
            .sum();
        let turn = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos();
        if turn > worst.1 {
            worst = (bone.clone(), turn);
        }
    }
    worst
}

/// Whether `frames` put part `i` away (over [`PUT_AWAY`] from its parent).
fn is_put_away(rig: &Rig, frames: &[Frame], i: usize) -> bool {
    let p = rig.bones[i]
        .parent
        .map_or(rig.bones[i].rest_translation, |p| frames[p].1);
    (0..3)
        .map(|k| (frames[i].1[k] - p[k]).powi(2))
        .sum::<f32>()
        .sqrt()
        > PUT_AWAY
}

/// Each part's distance (metres) and turn (radians) in `ours` from
/// `theirs`, by name. The hands' contact points (`*_hand_cont`, the IK
/// targets) are left out; so is a part `theirs` puts away; one only `ours`
/// puts away is infinitely far.
pub(super) fn part_distances(
    rig: &Rig,
    ours: &[Frame],
    theirs: &[Frame],
) -> Vec<(String, f32, f32)> {
    let away = |frames: &[Frame], i: usize| is_put_away(rig, frames, i);
    rig.bones
        .iter()
        .enumerate()
        .filter(|(i, bone)| !bone.name.ends_with("_hand_cont") && !away(theirs, *i))
        .map(|(i, bone)| {
            let (a, b) = (&ours[i], &theirs[i]);
            let dt = if away(ours, i) {
                f32::INFINITY
            } else {
                (0..3)
                    .map(|k| (a.1[k] - b.1[k]).powi(2))
                    .sum::<f32>()
                    .sqrt()
            };
            let trace: f32 = (0..3)
                .map(|r| (0..3).map(|k| a.0[k][r] * b.0[k][r]).sum::<f32>())
                .sum();
            (
                bone.name.clone(),
                dt,
                ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos(),
            )
        })
        .collect()
}

/// The bundle of a reload port from `donor` (0.29a3: its `.split` or
/// `resources.db`; 0.69d0: the archive of its files the reloads name, with
/// `gameplay/game.options`): the live configs with a two-clip reload, the
/// clips and sounds 0.26g lacks, `port-manifest.json` (for `pack
/// from-bundle`) and `tactical-reload.json` (each gun's clips and checks).
pub fn build(
    port: &TacticalPort,
    donor: &Path,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<Vec<Converted>> {
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

fn build_with<D: Donor>(
    port: &TacticalPort,
    donor: &mut D,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<Vec<Converted>> {
    ensure!(
        donor.build() == port.donor_build,
        "the donor is {:?}, the port's {:?}",
        donor.build(),
        port.donor_build
    );
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let rules = donor.rules();
    let tag = match port.donor_build {
        DonorBuild::V029a3 => "_029a3",
        DonorBuild::V031a2 => "_031a2",
        DonorBuild::V069d0 => "_069d0",
    };
    let mut target = ResourceDb::open_layers(target_layers)?;
    let hands = clip_bones(&target.read(NATIVE_HANDS_CLIP)?)?;
    let mut carry = Carry::new(donor, &mut target, &overlay, format!("converted:{tag}"));
    // Each clip path written and its bytes: a clip two guns share must come
    // out the same.
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut converted = vec![];
    let mut refused = vec![];
    for gun in &port.guns {
        let what = || format!("{} ({})", gun.config, gun.donor_item);
        let live = parse_config(carry.target(), &gun.config, "live weapon")?.root;
        let theirs = carry.donor().item_config(&gun.donor_item)?;
        let state = get(&live, "states.reload").with_context(what)?.clone();
        ensure!(
            get(&state, "state_type") == Some(&V::Integer(ONE_CLIP_RELOAD)),
            "{}: its reload is not the one-clip one ({ONE_CLIP_RELOAD})",
            what()
        );
        let donor_state = get(&theirs, "states.reload").with_context(what)?.clone();
        let kind = match get(&donor_state, "state_type") {
            Some(V::Integer(k)) => rules.state_type(*k)?,
            other => bail!("{}: donor reload state type {other:?}", what()),
        };
        ensure!(
            kind == TWO_CLIP_RELOAD,
            "{}: the donor's reload is no two-clip one (0.26g's {kind})",
            what()
        );
        let donor_hands = strings(get(&donor_state, "user_animations"))?;
        let donor_item = strings(get(&donor_state, "animations"))?;
        ensure!(
            donor_hands.len() == 8
                && donor_item.len() == 2
                && donor_hands.iter().step_by(2).all(|c| *c == donor_hands[0]),
            "{}: the donor's reload is not [tactical, empty] x 4 and [tactical, empty]",
            what()
        );
        let live_hands = strings(get(&state, "user_animations"))?;
        let live_item = strings(get(&state, "animations"))?;
        ensure!(
            live_hands.len() == 4 && live_item.len() == 1,
            "{}: the live reload has {} hands and {} item clips, not 4 and 1",
            what(),
            live_hands.len(),
            live_item.len()
        );

        // The clips: 0.26g's where it has them alike, else the donor's.
        let mut ship = |name: &String, carry: &mut Carry<'_, D>| -> Result<String> {
            let path = bspline(name);
            let bytes = carry.donor().read(&path)?;
            let names = carry.donor().read(&bones_names(name))?;
            if carry.target().contains(&path)
                && clip_bones(&carry.target().read(&bones_names(name))?)? == clip_bones(&names)?
                && clips::engine_words_cleared(&carry.target().read(&path)?)?
                    == clips::engine_words_cleared(&bytes)?
            {
                return Ok(name.clone());
            }
            let at = if carry.target().contains(&path) {
                format!("{name}{tag}")
            } else {
                name.clone()
            };
            if !written.contains_key(&at) {
                ensure!(
                    !carry.target().contains(&bspline(&at)),
                    "{at}: the target has it"
                );
                carry.raw(&bspline(&at), &bytes)?;
                carry.raw(&bones_names(&at), &names)?;
                written.insert(at.clone(), bytes);
            }
            Ok(at)
        };
        let mut pair = |list: &[String], carry: &mut Carry<'_, D>| -> Result<[String; 2]> {
            Ok([ship(&list[0], carry)?, ship(&list[1], carry)?])
        };
        let hands_pair = pair(&donor_hands, &mut carry)?;
        let item_pair = pair(&donor_item, &mut carry)?;

        // The checks against the live rig and rest.
        let read_clip = |name: &str, carry: &mut Carry<'_, D>| -> Result<Clip> {
            let (b, n) = if written.contains_key(name) {
                (
                    fs::read(crate::archive::safe_join(&overlay, &bspline(name))?)?,
                    fs::read(crate::archive::safe_join(&overlay, &bones_names(name))?)?,
                )
            } else {
                (
                    carry.target().read(&bspline(name))?,
                    carry.target().read(&bones_names(name))?,
                )
            };
            Clip::parse(&b, &n).with_context(|| name.to_owned())
        };
        let mut notes = vec![];
        let live_h = read_clip(&live_hands[0], &mut carry)?;
        let live_i = read_clip(&live_item[0], &mut carry)?;
        let live_idle = read_clip(
            &strings(get(&live, "states.idle.animations"))?[0],
            &mut carry,
        )?;
        let live_set: BTreeSet<String> = live_i.bones.iter().cloned().collect();
        let model = string_of(get(&live, "object.model")).context("object.model")?;
        let mut model_bones: BTreeSet<String> = bind_pose(
            &carry
                .target()
                .read(&format!("models/{model}.skinned_model/render/bind_pose"))?,
        )?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
        model_bones.insert("object_movement".into());
        let mut pairs = vec![];
        for k in 0..2 {
            let (h, i) = (
                read_clip(&hands_pair[k], &mut carry)?,
                read_clip(&item_pair[k], &mut carry)?,
            );
            ensure!(
                h.bones == hands,
                "{}: {} drives other bones than 0.26g's player rig",
                what(),
                hands_pair[k]
            );
            let set: BTreeSet<String> = i.bones.iter().cloned().collect();
            ensure!(
                set == live_set,
                "{}: {} drives {set:?}, the live reload {live_set:?}",
                what(),
                item_pair[k]
            );
            if k == 0 && set != model_bones {
                notes.push(format!(
                    "{} drives {set:?}, the model has {model_bones:?}: as the live reload",
                    item_pair[k]
                ));
            }
            for (end, ours, theirs) in [
                ("start", 0.0, 0.0),
                ("end", h.duration(), live_h.duration()),
            ] {
                let (bone, turn) = largest_turn(&h, ours, &live_h, theirs);
                if turn > REST_ROTATION {
                    notes.push(format!(
                        "{} {end}: the hands' {bone} turned {turn:.2} rad from 0.26g's reload",
                        hands_pair[k]
                    ));
                }
            }
            pairs.push((h, i, item_pair[k].clone()));
        }
        let skeleton = string_of(get(&live, "object.skeleton")).context("object.skeleton")?;
        let rig = Rig::parse(
            &carry
                .target()
                .read(&format!("models/{model}.skinned_model/render/bind_pose"))?,
            &carry
                .target()
                .read(&format!("animations/skeletons/{skeleton}.skeleton"))?,
        );
        let rig = match rig {
            Ok(r) => r,
            Err(e) => {
                refused.push(format!("{}: the gun's rig: {e}", what()));
                continue;
            }
        };
        let mut rest = [0.0f32; 2];
        for ((h, i, item_name), (end, at)) in pairs
            .iter()
            .flat_map(|p| [(p, ("start", 0.0)), (p, ("end", 1.0))])
        {
            let ours = placement(&rig, (h, at * h.duration()), (i, at * i.duration()));
            let hands_at = at * live_h.duration();
            let idle = placement(&rig, (&live_h, hands_at), (&live_idle, 0.0));
            let reload = placement(&rig, (&live_h, hands_at), (&live_i, at * live_i.duration()));
            let as_reload: BTreeMap<String, (f32, f32)> = part_distances(&rig, &ours, &reload)
                .into_iter()
                .map(|(b, dt, dr)| (b, (dt, dr)))
                .collect();
            let fits = |dt: f32, dr: f32| dt <= REST_TRANSLATION && dr <= REST_ROTATION;
            for (bone, dt, dr) in part_distances(&rig, &ours, &idle) {
                // Put away where 0.26g's own reload puts it away (the old
                // magazine of a two-magazine gun).
                let j = rig.index(&bone).expect("a rig bone");
                if dt.is_infinite() && is_put_away(&rig, &reload, j) {
                    notes.push(format!(
                        "{} {end}: {bone} put away, as 0.26g's reload puts it",
                        item_name
                    ));
                    continue;
                }
                let (dt, dr) = match as_reload.get(&bone) {
                    Some((t, r)) if fits(*t, *r) && !fits(dt, dr) => (*t, *r),
                    _ => (dt, dr),
                };
                if fits(dt, dr) {
                    rest = [rest[0].max(dt), rest[1].max(dr)];
                    continue;
                }
                match gun.accept.get(&bone) {
                    Some(why) => notes.push(format!(
                        "{} {end}: {bone} {dt:.4} m / {dr:.4} rad from 0.26g's, accepted: {why}",
                        item_name
                    )),
                    None => refused.push(format!(
                        "{}: at the {end} of {}, {bone} is {dt:.4} m / {dr:.4} rad from where 0.26g has it",
                        what(),
                        item_name
                    )),
                }
            }
        }
        for bone in gun.accept.keys() {
            ensure!(
                rig.index(bone).is_some(),
                "{}: accept names {bone}, which the gun has not",
                what()
            );
        }

        // The config.
        let mut cfg = live.clone();
        let names = |v: Vec<String>| V::Indexed(v.into_iter().map(V::String).collect());
        cfg.set_path("states.reload.state_type", V::Integer(TWO_CLIP_RELOAD))?;
        cfg.set_path(
            "states.reload.user_animations",
            names(live_hands.iter().flat_map(|_| hands_pair.clone()).collect()),
        )?;
        cfg.set_path("states.reload.animations", names(item_pair.to_vec()))?;
        let mut donor_sounds = false;
        for view in ["first_view", "third_view"] {
            let (ours, theirs) = (sound_list(&state, view)?, sound_list(&donor_state, view)?);
            if ours == theirs {
                continue;
            }
            donor_sounds = true;
            for (k, n) in &theirs {
                carry.sound(*k, n).with_context(what)?;
            }
            cfg.set_path(
                &format!("states.reload.sounds.{view}"),
                sound_value(&theirs),
            )?;
        }
        carry.config(&gun.config, cfg)?;
        converted.push(Converted {
            config: gun.config.clone(),
            donor_item: gun.donor_item.clone(),
            clips: [hands_pair, item_pair],
            donor_sounds,
            rest,
            notes,
        });
    }
    ensure!(refused.is_empty(), "{}", refused.join("\n"));
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
    fs::write(
        output.join("tactical-reload.json"),
        serde_json::to_vec_pretty(&json!({
            "guns": converted,
            "not_ported": port.not_ported,
        }))?,
    )?;
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton_animation::{BoneTracks, RigBone};

    fn clip(bones: &[(&str, [f32; 3], [f32; 3])]) -> Clip {
        Clip {
            bones: bones.iter().map(|(n, _, _)| (*n).to_owned()).collect(),
            tracks: bones
                .iter()
                .map(|(_, t, r)| BoneTracks::rest(*t, *r, 10.0))
                .collect(),
            animation_type: 0,
        }
    }

    /// 0.26g keeps a gun's place in the hands in its `offset` bone, under a
    /// root turned half a turn about y; 0.69d0 in the hands' `Weapon` bone.
    /// The same numbers there put the gun elsewhere; the turned ones match.
    #[test]
    fn a_gun_placed_by_the_weapon_bone_is_compared_through_the_root_turn() {
        let pi = std::f32::consts::PI;
        let rig = Rig {
            bones: vec![
                RigBone {
                    name: "wpn".into(),
                    parent: None,
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0, pi, 0.0],
                },
                RigBone {
                    name: "offset".into(),
                    parent: Some(0),
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0, pi, 0.0],
                },
                RigBone {
                    name: "magazin".into(),
                    parent: Some(1),
                    rest_translation: [0.0, -0.1, 0.0],
                    rest_rotation: [0.0, pi, 0.0],
                },
            ],
        };
        let d = [0.0, -0.006, -0.066];
        let ours_item = |o: [f32; 3]| {
            clip(&[
                ("wpn", [0.0; 3], [0.0, -pi, 0.0]),
                ("offset", o, [0.0; 3]),
                ("magazin", [0.0, -0.1, 0.0], [0.0; 3]),
            ])
        };
        let hands = |w: [f32; 3]| clip(&[("Weapon", w, [0.0; 3])]);
        let live = placement(&rig, (&hands([0.0; 3]), 0.0), (&ours_item(d), 0.0));
        // 0.69d0's way with the same numbers: 13 cm along z.
        let moved = placement(&rig, (&hands(d), 0.0), (&ours_item([0.0; 3]), 0.0));
        // The parts below the root (the root itself is the frame the
        // offset moves the gun in).
        let parts = |d: Vec<(String, f32, f32)>| -> Vec<(String, f32, f32)> {
            d.into_iter().filter(|(b, _, _)| b != "wpn").collect()
        };
        let far = parts(part_distances(&rig, &moved, &live));
        assert!(
            far.iter().all(|(_, dt, _)| (dt - 0.132).abs() < 1e-3),
            "{far:?}"
        );
        // The turned numbers in the Weapon bone: where 0.26g has it.
        let turned = placement(
            &rig,
            (&hands([-d[0], d[1], -d[2]]), 0.0),
            (&ours_item([0.0; 3]), 0.0),
        );
        let near = parts(part_distances(&rig, &turned, &live));
        assert!(
            near.iter().all(|(_, dt, dr)| *dt < 1e-4 && *dr < 1e-3),
            "{near:?}"
        );
    }

    #[test]
    fn a_part_put_away_where_0_26g_shows_it_is_infinitely_far() {
        let rig = Rig {
            bones: vec![
                RigBone {
                    name: "wpn".into(),
                    parent: None,
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0; 3],
                },
                RigBone {
                    name: "magazin".into(),
                    parent: Some(0),
                    rest_translation: [0.0, -0.1, 0.0],
                    rest_rotation: [0.0; 3],
                },
                RigBone {
                    name: "left_hand_cont".into(),
                    parent: Some(0),
                    rest_translation: [0.0; 3],
                    rest_rotation: [0.0; 3],
                },
            ],
        };
        let hands = clip(&[("Weapon", [0.0; 3], [0.0; 3])]);
        let item = |m: [f32; 3], c: [f32; 3]| {
            clip(&[
                ("wpn", [0.0; 3], [0.0; 3]),
                ("magazin", m, [0.0; 3]),
                ("left_hand_cont", c, [0.0; 3]),
            ])
        };
        let shown = placement(
            &rig,
            (&hands, 0.0),
            (&item([0.0, -0.1, 0.0], [0.0; 3]), 0.0),
        );
        let away = placement(
            &rig,
            (&hands, 0.0),
            (&item([0.0, -50.0, 0.0], [0.3, 0.0, 0.0]), 0.0),
        );
        let d = part_distances(&rig, &away, &shown);
        assert!(
            d.iter()
                .any(|(b, dt, _)| b == "magazin" && dt.is_infinite())
        );
        // The contact point (an IK target) is not a part.
        assert!(d.iter().all(|(b, _, _)| b != "left_hand_cont"));
        // Put away in both: not compared.
        assert!(
            part_distances(&rig, &away, &away)
                .iter()
                .all(|(b, _, _)| b != "magazin")
        );
    }
}
