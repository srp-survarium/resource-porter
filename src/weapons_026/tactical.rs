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
//!
//! A gun the donor holds by other bones' numbers, its hands the same, takes
//! [`Placement::LiveRest`]: the hands' `Weapon` held as the live reload holds
//! it and the gun's `offset` curve rebased to start where the live reload's
//! does ([`Rebase`]); the gun's own motion stays the donor's. A live two-clip reload that plays one clip for both (the
//! SVU-A of weapons-046) is converted as a one-clip one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::json;

use super::build::Carry;
use super::donor::{ArchiveDonor, Donor, DonorBuild};
use super::donor_069d::{Donor069, NATIVE_HANDS_CLIP, Spec069, clip_bones};
use super::{clips, get, string_of};
use crate::skeleton_animation::{
    Channel, Clip, Frame, Rig, basis_to_euler, euler_to_basis, multiply, transform, transpose,
};
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
    /// Where the gun hangs in the hands through the donor's reload.
    #[serde(default)]
    pub placement: Placement,
}

/// Where a ported reload hangs the gun in the hands.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// The donor's clips as they are: its hands' `Weapon` bone and its gun's
    /// `offset` bone place the gun as 0.26g's do.
    #[default]
    Donor,
    /// The donor keeps the gun's place in the hands in other bones' numbers
    /// than 0.26g, its hands as ours: 0.69d0 moved the SR-2M's, SR-3M's and
    /// UMP-45's from the gun's `offset` bone to the hands' `Weapon` bone, and
    /// the AEK-919K's offset to nothing. The hands' `Weapon` is held as the
    /// live reload holds it and the gun's `offset` curve is rebased to start
    /// where the live reload's does ([`Rebase`]): the gun's root, and what
    /// hangs on it (the SR-2M's left-hand `_reload` points), stays where
    /// 0.26g has it; the gun's own motion is the donor's.
    LiveRest,
}

/// The live reload a port replaces, by its clips.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LiveReload {
    /// 0.26g's one-clip reload (303): one hands clip in its four slots, one
    /// item clip.
    OneClip { hands: String, item: String },
    /// The two-clip reload (314) with one clip in both slots: weapons-046's
    /// SVU-A (0.46c2 reloads it with one).
    SameClipTwice { hands: String, item: String },
}

impl LiveReload {
    fn parse(state: &V) -> Result<Self> {
        let hands = strings(get(state, "user_animations"))?;
        let item = strings(get(state, "animations"))?;
        let one = |list: &[String], count: usize| {
            (list.len() == count && list.iter().all(|c| *c == list[0])).then(|| list[0].clone())
        };
        let clips = |count: usize| match (one(&hands, 4 * count), one(&item, count)) {
            (Some(hands), Some(item)) => Ok((hands, item)),
            _ => bail!(
                "its reload's {} hands and {} item clips are not one of each in every slot",
                hands.len(),
                item.len()
            ),
        };
        match get(state, "state_type") {
            Some(V::Integer(ONE_CLIP_RELOAD)) => {
                let (hands, item) = clips(1)?;
                Ok(Self::OneClip { hands, item })
            }
            Some(V::Integer(TWO_CLIP_RELOAD)) => {
                let (hands, item) = clips(2)?;
                Ok(Self::SameClipTwice { hands, item })
            }
            other => bail!(
                "its reload state {other:?} is neither the one-clip ({ONE_CLIP_RELOAD}) nor the \
                 two-clip one ({TWO_CLIP_RELOAD})"
            ),
        }
    }

    fn clips(&self) -> (&str, &str) {
        match self {
            Self::OneClip { hands, item } | Self::SameClipTwice { hands, item } => (hands, item),
        }
    }
}

/// How far a held bone's channels may move through a clip (metres, radians).
const HELD: f32 = 1e-5;

/// Bone `bone`'s translation and rotation in `clip`, which it holds from
/// start to end (sampled every quarter frame), or an error.
fn holds(clip: &Clip, bone: &str) -> Result<[f32; 6]> {
    let i = clip
        .bones
        .iter()
        .position(|b| b == bone)
        .with_context(|| format!("the clip has no {bone} bone"))?;
    let channels = &clip.tracks[i].channels;
    let start: [f32; 6] = std::array::from_fn(|c| channels[c].evaluate(0.0));
    let frames = (clip.duration() * 4.0).ceil() as usize;
    for step in 0..=frames {
        let t = step as f32 / 4.0;
        for (c, value) in start.iter().enumerate() {
            let now = channels[c].evaluate(t);
            ensure!(
                (now - value).abs() <= HELD,
                "{bone} moves in the clip (channel {c}: {value} at its start, {now} at frame {t})"
            );
        }
    }
    Ok(start)
}

fn frame_of(v: [f32; 6]) -> Frame {
    (euler_to_basis([v[3], v[4], v[5]]), [v[0], v[1], v[2]])
}

/// `a`, then `b` (row vectors: `a`'s frame inside `b`'s).
fn then(a: &Frame, b: &Frame) -> Frame {
    let moved = transform(a.1, &b.0);
    (multiply(&a.0, &b.0), [0, 1, 2].map(|k| moved[k] + b.1[k]))
}

fn inverse(f: &Frame) -> Frame {
    let basis = transpose(&f.0);
    let moved = transform(f.1, &basis);
    (basis, moved.map(|v| -v))
}

/// How far apart two frames are: metres, and radians (from the bases'
/// difference, `|A - B| = 2 sqrt 2 sin(angle / 2)`, which resolves small
/// turns an f32 `acos` of the trace does not).
fn apart(a: &Frame, b: &Frame) -> (f32, f32) {
    let distance = (0..3).map(|k| (a.1[k] - b.1[k]).powi(2)).sum::<f32>().sqrt();
    let norm = (0..3)
        .flat_map(|r| (0..3).map(move |c| (r, c)))
        .map(|(r, c)| (a.0[r][c] - b.0[r][c]).powi(2))
        .sum::<f32>()
        .sqrt();
    (distance, 2.0 * (norm / (2.0 * 2f32.sqrt())).min(1.0).asin())
}

/// Whether both of `(distance, turn)` are within `limit`'s.
fn within((distance, turn): (f32, f32), limit: (f32, f32)) -> bool {
    distance <= limit.0 && turn <= limit.1
}

/// How far a refitted `offset` curve may stray from the frames it was fitted
/// to, between its samples (every quarter frame): metres, radians.
const REFIT_TRANSLATION: f32 = 1e-4;
const REFIT_ROTATION: f32 = 1e-3;

/// [`Placement::LiveRest`]: the hands' `Weapon` held as the live reload
/// holds it, and the gun's `offset` curve `O(t) * Q` ([`then`]'s order) with
/// `Q = O(0)^-1 * O_live(0)`, `O` the donor's (local frames, under the gun's
/// root, which the donor's and the live clips hold alike). The offset starts
/// where the live reload's does and moves as the donor's; nothing else of the
/// clips changes. 0.69d0's SR-2M, SR-3M, AEK-919K and UMP-45 empty reloads
/// are their live reloads so re-expressed: rebased, each reproduces the live
/// `offset` curve within 2 mm and 0.012 rad.
struct Rebase {
    /// The live hands' `Weapon`, and the donor's it replaces.
    weapon: [f32; 6],
    donor_weapon: [f32; 6],
    q: Frame,
}

impl Rebase {
    fn new(
        rig: &Rig,
        (live_hands, live_item): (&Clip, &Clip),
        donor_hands: &[Clip; 2],
        donor_item: &[Clip; 2],
    ) -> Result<Self> {
        let weapon = holds(live_hands, "Weapon").context("the live reload's hands")?;
        let [donor_weapon, empty] = [0, 1].map(|k| holds(&donor_hands[k], "Weapon"));
        let (donor_weapon, empty) = (
            donor_weapon.context("the donor's tactical hands")?,
            empty.context("the donor's empty hands")?,
        );
        ensure!(
            within(apart(&frame_of(donor_weapon), &frame_of(empty)), (HELD, 1e-3)),
            "the donor's tactical and empty hands hold Weapon at {donor_weapon:?} and {empty:?}"
        );
        let offset = rig.index("offset").context("the gun has no offset bone")?;
        let root = rig.bones[offset]
            .parent
            .map(|p| rig.bones[p].name.clone())
            .context("the gun's offset bone is its root")?;
        let at_start = |clip: &Clip, bone: &str| -> Result<Frame> {
            let i = clip
                .bones
                .iter()
                .position(|b| b == bone)
                .with_context(|| format!("an item clip without {bone}"))?;
            Ok(frame_of(std::array::from_fn(|c| {
                clip.tracks[i].channels[c].evaluate(0.0)
            })))
        };
        let live_root = at_start(live_item, &root)?;
        for clip in donor_item {
            holds(clip, &root).context("the donor's item clip")?;
            ensure!(
                within(apart(&at_start(clip, &root)?, &live_root), (HELD, 1e-3)),
                "the donor's item clip holds the gun's root {root} otherwise than the live one"
            );
        }
        let [tactical, empty] = [0, 1].map(|k| at_start(&donor_item[k], "offset"));
        let (tactical, empty) = (tactical?, empty?);
        ensure!(
            within(apart(&tactical, &empty), (1e-4, 1e-3)),
            "the donor's tactical and empty item clips start the offset bone {:?} apart",
            apart(&tactical, &empty)
        );
        Ok(Self {
            weapon,
            donor_weapon,
            q: then(&inverse(&tactical), &at_start(live_item, "offset")?),
        })
    }

    /// A donor hands clip with `Weapon` held at the live hands'.
    fn hands(&self, bytes: &[u8], clip: &Clip) -> Result<Vec<u8>> {
        let w = clip
            .bones
            .iter()
            .position(|b| b == "Weapon")
            .context("a hands clip without the Weapon bone")?;
        let channels = &clip.tracks[w].channels;
        let curves = std::array::from_fn(|c| {
            Channel::constant(self.weapon[c], *channels[c].knots.last().expect("knots"))
        });
        self.check(bytes, clip, w, clips::replace_bone_curves(bytes, w, &curves)?, |_| {
            frame_of(self.weapon)
        })
    }

    /// A donor item clip with its `offset` curve rebased: sampled every
    /// frame (and at its end), fitted again, then checked every quarter frame.
    fn item(&self, bytes: &[u8], clip: &Clip) -> Result<Vec<u8>> {
        let o = clip
            .bones
            .iter()
            .position(|b| b == "offset")
            .context("an item clip without the offset bone")?;
        let channels = &clip.tracks[o].channels;
        let rebased = |t: f32| {
            then(
                &frame_of(std::array::from_fn(|c| channels[c].evaluate(t))),
                &self.q,
            )
        };
        let start = channels[..6]
            .iter()
            .map(|c| c.knots[0])
            .fold(f32::INFINITY, f32::min);
        let end = clip.duration();
        let mut times: Vec<f32> = (0..)
            .map(|k| start + k as f32)
            .take_while(|t| *t < end)
            .collect();
        times.push(end);
        let mut values: [Vec<f32>; 6] = Default::default();
        for t in &times {
            let (basis, at) = rebased(*t);
            let mut angles = basis_to_euler(&basis);
            // Continuous through the clip: no jump of a full turn.
            if let Some(prev) = values[3].last().map(|_| [3, 4, 5].map(|c| *values[c].last().unwrap())) {
                for (a, p) in angles.iter_mut().zip(prev) {
                    *a -= std::f32::consts::TAU * ((*a - p) / std::f32::consts::TAU).round();
                }
            }
            for (c, v) in at.iter().chain(&angles).enumerate() {
                values[c].push(*v);
            }
        }
        let curves: [Channel; 6] = (0..6)
            .map(|c| Channel::fit(&times, &values[c]))
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("six curves");
        self.check(bytes, clip, o, clips::replace_bone_curves(bytes, o, &curves)?, rebased)
    }

    /// `out` (`bytes` with bone `bone`'s curves replaced) checked: every other
    /// curve as it was, and the bone within [`REFIT_TRANSLATION`] and
    /// [`REFIT_ROTATION`] of `wanted` every quarter frame.
    fn check(
        &self,
        bytes: &[u8],
        clip: &Clip,
        bone: usize,
        out: Vec<u8>,
        wanted: impl Fn(f32) -> Frame,
    ) -> Result<Vec<u8>> {
        let names = clip.to_bones_names()?;
        let new = Clip::parse(&out, &names)?;
        ensure!(new.bones == clip.bones, "the rebased clip drives other bones");
        for (i, (a, b)) in new.tracks.iter().zip(&clip.tracks).enumerate() {
            let same = |c: usize| a.channels[c] == b.channels[c];
            ensure!(
                (0..9).all(|c| (i == bone && c < 6) || same(c)),
                "the rebase changed the curves of {}",
                clip.bones[i]
            );
        }
        ensure!(
            clips::event_names(&out)? == clips::event_names(bytes)?,
            "the rebase lost an event channel"
        );
        let channels = &new.tracks[bone].channels;
        for step in 0..=(clip.duration() * 4.0).ceil() as usize {
            let t = (step as f32 / 4.0).min(clip.duration());
            let got = frame_of(std::array::from_fn(|c| channels[c].evaluate(t)));
            let (dt, dr) = apart(&got, &wanted(t));
            ensure!(
                dt <= REFIT_TRANSLATION && dr <= REFIT_ROTATION,
                "the rebased {} strays {dt} m / {dr} rad at frame {t}",
                clip.bones[bone]
            );
        }
        Ok(out)
    }
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
        let live_reload = LiveReload::parse(&state).with_context(what)?;
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
        let (live_hands, live_item) = live_reload.clips();
        let mut notes = vec![];
        if let LiveReload::SameClipTwice { .. } = live_reload {
            notes.push(format!(
                "the live reload is the two-clip one ({TWO_CLIP_RELOAD}) with one clip for both"
            ));
        }

        // The live clips and the gun's rig, which the donor's are checked
        // against (and placed by).
        let read_target = |name: &str, carry: &mut Carry<'_, D>| -> Result<Clip> {
            Clip::parse(
                &carry.target().read(&bspline(name))?,
                &carry.target().read(&bones_names(name))?,
            )
            .with_context(|| name.to_owned())
        };
        let live_h = read_target(live_hands, &mut carry)?;
        let live_i = read_target(live_item, &mut carry)?;
        let live_idle = read_target(
            &strings(get(&live, "states.idle.animations"))?[0],
            &mut carry,
        )?;
        let model = string_of(get(&live, "object.model")).context("object.model")?;
        let skeleton = string_of(get(&live, "object.skeleton")).context("object.skeleton")?;
        let rig = Rig::parse_root_as_skeleton(
            &carry
                .target()
                .read(&format!("models/{model}.skinned_model/render/bind_pose"))?,
            &carry
                .target()
                .read(&format!("animations/skeletons/{skeleton}.skeleton"))?,
        );
        let rig = match rig {
            Ok((r, renamed)) => {
                if let Some((ours, theirs)) = renamed {
                    notes.push(format!(
                        "the model's root {ours} is the skeleton's {theirs}, which the clips drive"
                    ));
                }
                r
            }
            Err(e) => {
                refused.push(format!("{}: the gun's rig: {e}", what()));
                continue;
            }
        };

        // The donor's clips; with `placement: live_rest` the hands' `Weapon`
        // held where the live reload's `offset` puts the gun.
        let read_donor = |name: &String, carry: &mut Carry<'_, D>| -> Result<(Vec<u8>, Vec<u8>)> {
            Ok((
                carry.donor().read(&bspline(name))?,
                carry.donor().read(&bones_names(name))?,
            ))
        };
        let mut hands_files = [
            read_donor(&donor_hands[0], &mut carry)?,
            read_donor(&donor_hands[1], &mut carry)?,
        ];
        let mut item_files = [
            read_donor(&donor_item[0], &mut carry)?,
            read_donor(&donor_item[1], &mut carry)?,
        ];
        if gun.placement == Placement::LiveRest {
            let parse = |(b, n): &(Vec<u8>, Vec<u8>)| Clip::parse(b, n);
            let hands_clips = [parse(&hands_files[0])?, parse(&hands_files[1])?];
            let item_clips = [parse(&item_files[0])?, parse(&item_files[1])?];
            let rebase = Rebase::new(&rig, (&live_h, &live_i), &hands_clips, &item_clips)
                .with_context(what)?;
            for (files, clip) in hands_files.iter_mut().zip(&hands_clips) {
                files.0 = rebase.hands(&files.0, clip).with_context(what)?;
            }
            for (files, clip) in item_files.iter_mut().zip(&item_clips) {
                files.0 = rebase.item(&files.0, clip).with_context(what)?;
            }
            notes.push(format!(
                "the hands' Weapon held at {:?} (the live reload's), not the donor's {:?}; the \
                 gun's offset curve rebased by {:?}: it starts where the live reload's does",
                rebase.weapon, rebase.donor_weapon, rebase.q
            ));
        }

        // The clips: 0.26g's where it has them alike, else the donor's.
        let mut ship = |name: &String,
                        (bytes, names): &(Vec<u8>, Vec<u8>),
                        carry: &mut Carry<'_, D>|
         -> Result<String> {
            let path = bspline(name);
            if carry.target().contains(&path)
                && clip_bones(&carry.target().read(&bones_names(name))?)? == clip_bones(names)?
                && clips::engine_words_cleared(&carry.target().read(&path)?)?
                    == clips::engine_words_cleared(bytes)?
            {
                return Ok(name.clone());
            }
            let at = if carry.target().contains(&path) {
                format!("{name}{tag}")
            } else {
                name.clone()
            };
            match written.get(&at) {
                Some(shipped) => ensure!(
                    shipped == bytes,
                    "{at}: two guns make it otherwise (their placements differ)"
                ),
                None => {
                    ensure!(
                        !carry.target().contains(&bspline(&at)),
                        "{at}: the target has it"
                    );
                    carry.raw(&bspline(&at), bytes)?;
                    carry.raw(&bones_names(&at), names)?;
                    written.insert(at.clone(), bytes.clone());
                }
            }
            Ok(at)
        };
        let hands_pair = [
            ship(&donor_hands[0], &hands_files[0], &mut carry)?,
            ship(&donor_hands[1], &hands_files[1], &mut carry)?,
        ];
        let item_pair = [
            ship(&donor_item[0], &item_files[0], &mut carry)?,
            ship(&donor_item[1], &item_files[1], &mut carry)?,
        ];

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
        let live_set: BTreeSet<String> = live_i.bones.iter().cloned().collect();
        let mut model_bones: BTreeSet<String> = rig.bones.iter().map(|b| b.name.clone()).collect();
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
            names((0..4).flat_map(|_| hands_pair.clone()).collect()),
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
