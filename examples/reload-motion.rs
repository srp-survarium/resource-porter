// SPDX-License-Identifier: GPL-3.0-or-later
//! How fast a gun's reload moves the hands, the gun and its parts as 0.26g
//! plays it.
//!
//! The engine plays a reload state's hands clips at one speed, the first
//! clip's length over `parameters/reload_time` (0.26g's 0x30D750, 0x30FC50):
//! clip `i` lasts `reload_time * frames_i / frames_0`. Per reload hands clip of
//! a weapon config (the first two distinct; or `--clips`), this prints its
//! frames, the seconds it lasts (or `--seconds`), its frames a 1/30 s, and how
//! fast each hand and the gun (`Weapon`) move in the chest's frame on the
//! config's first-person stance (its `user_animations` set's `stand_hud[0]`):
//! the median, the 90th percentile and the fastest, metres a second over 1/30 s
//! steps of game time. `--profile` adds the hands' speed a step.
//!
//! `--pace` prints instead, a JSON line per clip, what skhid-srp's
//! `reload_pace` (content-edit `reload-timings-1.json`) takes: the hands clip
//! and its item clip (the gun's clip of the same slot), their sha256, the
//! frames, and per source frame the fastest of the two hands, the gun and the
//! gun's parts in its own frame (a part jumping over 3 m/s, put away, left
//! out), in cm/s at 30 source frames a second.
//!
//!     cargo run --release --example reload-motion -- --db <stock> [--db <patch>].. \
//!         [--files <dir>].. --config gameplay/items/weapons/wpn_<gun>.options \
//!         [--seconds <s>[,<s>]] [--clips <hands>[=<item>],..] [--profile | --pace]

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use vostok_resource_porter::binary_config::{BinaryConfig, ConfigValue};
use vostok_resource_porter::skeleton_animation::{Clip, Rig};
use vostok_resource_porter::weapons_026::hands::{Skeleton, Stance, pose};

/// What to print.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Table,
    Profile,
    Pace,
}

/// A part moving faster than this between two steps was put away (m/s).
const JUMP: f32 = 3.0;

fn main() -> Result<()> {
    let mut dbs: Vec<PathBuf> = vec![];
    let mut files: Vec<PathBuf> = vec![];
    let mut config = None;
    let mut seconds: Option<Vec<f32>> = None;
    let mut clips: Option<Vec<String>> = None;
    let mut output = Output::Table;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => dbs.push(args.next().context("--db")?.into()),
            "--files" => files.push(args.next().context("--files")?.into()),
            "--config" => config = Some(args.next().context("--config")?),
            "--seconds" => {
                seconds = Some(
                    args.next()
                        .context("--seconds")?
                        .split(',')
                        .map(str::parse::<f32>)
                        .collect::<std::result::Result<_, _>>()?,
                )
            }
            "--clips" => {
                clips = Some(
                    args.next()
                        .context("--clips")?
                        .split(',')
                        .map(str::to_owned)
                        .collect(),
                )
            }
            "--profile" => output = Output::Profile,
            "--pace" => output = Output::Pace,
            _ => bail!("unknown argument {a}"),
        }
    }
    let config = config.context("--config")?;
    let mut db = vostok_resource_porter::ResourceDb::open_layers(&dbs)?;
    let mut read = |path: &str| -> Result<Vec<u8>> {
        for dir in &files {
            let file = dir.join(path);
            if file.is_file() {
                return Ok(std::fs::read(file)?);
            }
        }
        db.read(path).with_context(|| path.to_owned())
    };
    let cfg = BinaryConfig::parse(&read(&config)?)?.root;
    let at = |path: &str| -> Option<&ConfigValue> {
        let mut v = &cfg;
        for p in path.split('.') {
            v = v.named(p)?;
        }
        Some(v)
    };
    let string = |v: Option<&ConfigValue>| -> Option<String> {
        match v? {
            ConfigValue::String(s) => Some(s.trim_start_matches("resources/").to_owned()),
            _ => None,
        }
    };
    let strings = |path: &str| -> Vec<String> {
        match at(path) {
            Some(ConfigValue::Indexed(list)) => {
                list.iter().filter_map(|c| string(Some(c))).collect()
            }
            _ => vec![],
        }
    };
    // The hands clips of the first two slots (tactical, empty; or the one)
    // and each one's item clip.
    let (names, items): (Vec<String>, Vec<String>) = match clips {
        Some(c) => c
            .iter()
            .map(|n| match n.split_once('=') {
                Some((h, i)) => (h.to_owned(), i.to_owned()),
                None => (n.clone(), String::new()),
            })
            .unzip(),
        None => {
            let (hands, item) = (
                strings("states.reload.user_animations"),
                strings("states.reload.animations"),
            );
            let mut names: Vec<String> = vec![];
            for h in hands {
                if names.len() < 2 && !names.contains(&h) {
                    names.push(h);
                }
            }
            let items = (0..names.len())
                .map(|i| item.get(i).or(item.first()).cloned().unwrap_or_default())
                .collect();
            (names, items)
        }
    };
    ensure_some(&names)?;
    let reload = match at("parameters.reload_time") {
        Some(ConfigValue::Float(f)) => *f,
        Some(ConfigValue::Integer(i)) => *i as f32,
        _ => bail!("no reload_time"),
    };
    let skeleton = Skeleton::parse(&read("animations/skeletons/scavengers_01.skeleton")?)?;
    let set = BinaryConfig::parse(&read(
        &string(at("user_animations")).context("no user_animations")?,
    )?)?;
    let body = match set.root.named("stand_hud") {
        Some(ConfigValue::Indexed(l)) => match l.first() {
            Some(ConfigValue::String(s)) => s.trim_start_matches("resources/").to_owned(),
            _ => bail!("stand_hud[0] is not a clip"),
        },
        _ => bail!("the set has no stand_hud"),
    };
    let stance = Stance::of(&skeleton, &read_clip(&mut read, &body)?)?;
    let bones = ["LeftHand", "RightHand", "Weapon", "Chest"]
        .map(|b| skeleton.index(b))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let rig = Rig::parse(
        &read(&format!(
            "models/{}.skinned_model/render/bind_pose",
            string(at("object.model")).context("no object.model")?
        ))?,
        &read(&format!(
            "animations/skeletons/{}.skeleton",
            string(at("object.skeleton")).context("no object.skeleton")?
        ))?,
    )?;
    let hands: Vec<Clip> = names
        .iter()
        .map(|n| read_clip(&mut read, n))
        .collect::<Result<_>>()?;
    let item_clips: Vec<Option<Clip>> = items
        .iter()
        .map(|n| (!n.is_empty()).then(|| read_clip(&mut read, n)).transpose())
        .collect::<Result<_>>()?;
    let first = hands[0].duration();
    if output != Output::Pace {
        println!(
            "clip\tframes\tseconds\tframes/30\tleft p50/p90/max m/s\tright p50/p90/max\tgun p50/p90/max"
        );
    }
    for (i, (name, clip)) in names.iter().zip(&hands).enumerate() {
        let frames = clip.duration();
        let secs = match (output, &seconds) {
            (Output::Pace, _) => frames / 30.0,
            (_, Some(s)) => s[i.min(s.len() - 1)],
            (_, None) => reload * frames / first,
        };
        // Steps of 1/30 s of game time.
        let steps = (secs * 30.0).round().max(1.0) as usize;
        let dt = secs / steps as f32;
        let mut track: Vec<[[f32; 3]; 3]> = vec![];
        let mut parts: Vec<Vec<[f32; 3]>> = vec![];
        for s in 0..=steps {
            let t = frames * s as f32 / steps as f32;
            if let Some(item) = &item_clips[i] {
                let p = rig.pose(item, (t * item.duration() / frames).min(item.duration()));
                parts.push(
                    p.iter()
                        .zip(&rig.bones)
                        .filter(|(_, b)| !b.name.ends_with("_hand_cont"))
                        .map(|(f, _)| f.1)
                        .collect(),
                );
            }
            let p = pose(&skeleton, &stance, clip, t)?;
            let chest = &p[bones[3]];
            // A bone in the chest's frame.
            let rel = |b: usize| {
                let d = [0, 1, 2].map(|k| p[b].1[k] - chest.1[k]);
                let r = &chest.0;
                [0, 1, 2].map(|k| d[0] * r[k][0] + d[1] * r[k][1] + d[2] * r[k][2])
            };
            track.push([rel(bones[0]), rel(bones[1]), rel(bones[2])]);
        }
        let distance =
            |a: &[f32; 3], b: &[f32; 3]| (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f32>().sqrt();
        let speeds = |j: usize| -> Vec<f32> {
            track
                .windows(2)
                .map(|w| distance(&w[0][j], &w[1][j]) / dt)
                .collect()
        };
        let (left, right, gun) = (speeds(0), speeds(1), speeds(2));
        if output == Output::Pace {
            let part: Vec<f32> = parts
                .windows(2)
                .map(|w| {
                    w[0].iter()
                        .zip(&w[1])
                        .map(|(a, b)| distance(a, b) / dt)
                        .filter(|v| *v < JUMP)
                        .fold(0.0, f32::max)
                })
                .collect();
            let speed: Vec<String> = (0..left.len())
                .map(|s| {
                    let m = left[s]
                        .max(right[s])
                        .max(gun[s])
                        .max(part.get(s).copied().unwrap_or(0.0));
                    format!("{}", (m * 100.0).round() as i64)
                })
                .collect();
            let item = &items[i];
            let mut sha = |p: &str| -> Result<String> {
                Ok(format!(
                    "{:x}",
                    Sha256::digest(read(&format!("{p}.b-spline"))?)
                ))
            };
            println!(
                r#"{{"hands": "{}", "item": "{}", "hands_sha256": "{}", "item_sha256": "{}", "frames": {}, "speed": [{}]}}"#,
                name,
                item,
                sha(name)?,
                sha(item)?,
                frames,
                speed.join(", ")
            );
            continue;
        }
        let stats = |mut v: Vec<f32>| {
            v.sort_by(f32::total_cmp);
            let q = |f: f32| v[((v.len() - 1) as f32 * f).round() as usize];
            format!("{:.2}/{:.2}/{:.2}", q(0.5), q(0.9), q(1.0))
        };
        println!(
            "{}\t{frames:.0}\t{secs:.3}\t{:.3}\t{}\t{}\t{}",
            name.rsplit('/').next().unwrap_or(name),
            frames / 30.0 / secs,
            stats(left.clone()),
            stats(right.clone()),
            stats(gun)
        );
        if output == Output::Profile {
            for (s, (a, b)) in left.iter().zip(&right).enumerate() {
                println!(
                    "  {:.3}\t{:.1}\t{a:.2}\t{b:.2}",
                    s as f32 * dt,
                    frames * s as f32 / steps as f32
                );
            }
        }
    }
    Ok(())
}

fn ensure_some(names: &[String]) -> Result<()> {
    if names.is_empty() {
        bail!("the config's reload names no hands clips");
    }
    Ok(())
}

fn read_clip(read: &mut dyn FnMut(&str) -> Result<Vec<u8>>, name: &str) -> Result<Clip> {
    Clip::parse(
        &read(&format!("{name}.b-spline"))?,
        &read(&format!("{name}.bones_names"))?,
    )
    .with_context(|| name.to_owned())
}
