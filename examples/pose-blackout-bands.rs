// SPDX-License-Identifier: GPL-3.0-or-later
//! A torso and its Blackout bands skinned into one pose of a clip, for
//! offline renders: copies of their `converted_model`s with the posed
//! positions and normals, at the same paths under `--output`.
//!
//! ```text
//! cargo run --release --example pose-blackout-bands -- --target-db RETAIL/resources.db \
//!     [--patch RETAIL/resources.db.patch_N ...] --overlay BUNDLE/overlay \
//!     --torso character/human/scavengers/scavengers_01_torso_s1 \
//!     --clip ak/player/stand/3rd_view/death/on_site_back --at 1.0 --output posed
//! ```
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{fs, path::PathBuf};
use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::blackout_bands::{COLOURS, band_model};
use vostok_resource_porter::skeleton_animation::{Clip, Frame, Rig, transform, transpose};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    target_db: PathBuf,
    #[arg(long = "patch")]
    patches: Vec<PathBuf>,
    /// The bands' bundle overlay.
    #[arg(long)]
    overlay: PathBuf,
    /// The torso part's `base_model`.
    #[arg(long)]
    torso: String,
    /// A clip under `animations/single/weapons/`.
    #[arg(long)]
    clip: String,
    /// Additive clips (a weapon's `hands_only` hold, a `_look`) layered on
    /// `--clip` in order, as the game mixes a third-person pose.
    #[arg(long = "add")]
    adds: Vec<String>,
    /// Where in each `--add` clip, 0 to 1 (default: `--at`).
    #[arg(long = "add-at")]
    add_at: Vec<f32>,
    /// Where in the clip, 0 (start) to 1 (end).
    #[arg(long, default_value_t = 1.0)]
    at: f32,
    #[arg(long)]
    output: PathBuf,
}

const BIND_POSE: &str =
    "models/character/human/scavengers/scavengers_01_torso_s1.skinned_model/render/bind_pose";

fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `model` with every vertex skinned from `bind` into `pose`.
fn pose_surface(model: &[u8], bind: &[Frame], pose: &[Frame]) -> Result<Vec<u8>> {
    let mut out = model.to_vec();
    let count = u32_at(model, 8);
    ensure!(count > 0, "empty surface");
    let stride = (u32_at(model, 4) - 4) / count;
    let weights = match stride {
        48 => 3,
        44 => 2,
        40 => 1,
        36 => 0,
        _ => anyhow::bail!("{stride}-byte vertices"),
    };
    for i in 0..count {
        let v = 12 + i * stride;
        let p = [f32_at(model, v), f32_at(model, v + 4), f32_at(model, v + 8)];
        let bones = &model[v + 12..v + 16];
        let mut w = [0.0_f32; 4];
        for (k, x) in w.iter_mut().enumerate().take(weights) {
            *x = f32_at(model, v + 16 + k * 4);
        }
        w[weights] = 1.0 - w[..weights].iter().sum::<f32>();
        let n_at = v + stride - 20;
        let packed = u32_at(model, n_at) as u32;
        let n = [(packed >> 16) & 255, (packed >> 8) & 255, packed & 255]
            .map(|c| c as f32 / 127.5 - 1.0);
        let (mut posed, mut normal) = ([0.0_f32; 3], [0.0_f32; 3]);
        for (k, &b) in bones.iter().enumerate().take(weights + 1) {
            if w[k] == 0.0 {
                continue;
            }
            let (bind_basis, bind_at) = bind.get(usize::from(b)).context("bone outside the rig")?;
            let (pose_basis, pose_at) = &pose[usize::from(b)];
            let local = transform([0, 1, 2].map(|c| p[c] - bind_at[c]), &transpose(bind_basis));
            let moved = transform(local, pose_basis);
            let turned = transform(transform(n, &transpose(bind_basis)), pose_basis);
            for c in 0..3 {
                posed[c] += w[k] * (moved[c] + pose_at[c]);
                normal[c] += w[k] * turned[c];
            }
        }
        for (c, x) in posed.iter().enumerate() {
            out[v + c * 4..v + c * 4 + 4].copy_from_slice(&x.to_le_bytes());
        }
        let len = normal.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        let byte = |x: f32| (((x / len + 1.0) * 127.5).floor() as i32).clamp(0, 255) as u32;
        let repacked = (packed & 0xff00_0000)
            | (byte(normal[0]) << 16)
            | (byte(normal[1]) << 8)
            | byte(normal[2]);
        out[n_at..n_at + 4].copy_from_slice(&repacked.to_le_bytes());
    }
    Ok(out)
}

fn main() -> Result<()> {
    let a = Args::parse();
    let mut layers = vec![a.target_db.clone()];
    layers.extend(a.patches.iter().cloned());
    let mut view = ResourceDb::open_layers(&layers)?;
    let bind_pose = view.read(BIND_POSE)?;
    let rig = Rig::parse(
        &bind_pose,
        &view.read("animations/skeletons/scavengers_01.skeleton")?,
    )?;
    let bind = rig.rest();
    let path = format!("animations/single/weapons/{}", a.clip);
    let clip = Clip::parse(
        &view.read(&format!("{path}.b-spline"))?,
        &view.read(&format!("{path}.bones_names"))?,
    )?;
    let mut adds = vec![];
    for (k, name) in a.adds.iter().enumerate() {
        let path = format!("animations/single/weapons/{name}");
        let add = Clip::parse(
            &view.read(&format!("{path}.b-spline"))?,
            &view.read(&format!("{path}.bones_names"))?,
        )?;
        ensure!(add.animation_type == 1, "{name} is not an additive clip");
        let at = a.add_at.get(k).copied().unwrap_or(a.at).clamp(0.0, 1.0);
        let time = add.duration() * at;
        adds.push((add, time));
    }
    let layers: Vec<(&Clip, f32)> = adds.iter().map(|(c, t)| (c, *t)).collect();
    let pose = rig.pose_layered((&clip, clip.duration() * a.at.clamp(0.0, 1.0)), &layers);
    // How the twist bone turns against its arm: a twist about the arm's
    // axis keeps the axis; a swing turns it.
    for side in ["Left", "Right"] {
        let i = |n: &str| rig.index(&format!("{side}{n}")).context("bone");
        let (arm, roll, fore) = (i("Arm")?, i("Arm_Roll")?, i("ForeArm")?);
        let d = {
            let (a0, f0) = (bind[arm].1, bind[fore].1);
            let v = [0, 1, 2].map(|k| f0[k] - a0[k]);
            let l = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.map(|x| x / l)
        };
        let through = |b: usize| transform(transform(d, &transpose(&bind[b].0)), &pose[b].0);
        let (da, dr) = (through(arm), through(roll));
        let cos = (0..3).map(|k| da[k] * dr[k]).sum::<f32>().clamp(-1.0, 1.0);
        println!(
            "{side}: Arm_Roll swings {:.1} deg off the arm's axis",
            cos.acos().to_degrees()
        );
    }
    let root = format!("models/{}.skinned_model/render/", a.torso);
    let surfaces: Vec<String> = view
        .paths_with_prefix(&root)
        .filter(|p| p.ends_with("/converted_model"))
        .map(str::to_owned)
        .collect();
    let mut written = 0;
    for path in surfaces {
        let posed = pose_surface(&view.read(&path)?, &bind, &pose)?;
        let dest = a.output.join(&path);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, posed)?;
        written += 1;
    }
    for colour in &COLOURS {
        let path = format!(
            "models/{}.skinned_model/render/band/converted_model",
            band_model(&a.torso, colour.name)
        );
        let posed = pose_surface(&fs::read(a.overlay.join(&path))?, &bind, &pose)?;
        let dest = a.output.join(&path);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, posed)?;
        written += 1;
    }
    println!("{written} surfaces posed at {} of {}", a.at, a.clip);
    Ok(())
}
