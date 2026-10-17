// SPDX-License-Identifier: GPL-3.0-or-later
//! A three-bone rifle: a body, a stripper clip and one cartridge, with an
//! `idle` that parks the clip 1.4 m under the barrel (as the retail Mosin's
//! does) and a `reload_start` that starts with it stowed in the stock.
use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::binary_config::NamedValue;
use crate::skeleton_animation::{BoneTracks, Channel};

const HELD: [&str; 2] = ["clip", "patron"];
const CLIPS: &str = "animations/single/weapons/gun/item";
const MODEL: &str = "models/weapons/gun/gun.skinned_model";
/// Where `reload_start` stows the clip at t=0: inside the stock.
const STOWED: [f32; 3] = [0.0, -0.02, -0.2];
/// Where `idle` parks it, as retail's Mosin does.
const PARKED: [f32; 3] = [-0.165, -1.275, -0.525];

fn held() -> Vec<String> {
    HELD.map(str::to_owned).to_vec()
}

fn named(name: &str, value: ConfigValue) -> NamedValue {
    NamedValue {
        name: name.to_owned(),
        value,
    }
}

fn config(root: ConfigValue) -> Vec<u8> {
    BinaryConfig { root }.to_bytes().unwrap()
}

/// `object_movement > body > clip > patron`, each with a mask leaf.
fn skeleton() -> Vec<u8> {
    let mask = || named("mask", ConfigValue::Integer(1));
    let patron = ConfigValue::Named(vec![mask()]);
    let clip = ConfigValue::Named(vec![named("patron", patron), mask()]);
    let body = ConfigValue::Named(vec![named("clip", clip), mask()]);
    let root = ConfigValue::Named(vec![named("body", body), mask()]);
    config(ConfigValue::Named(vec![named("object_movement", root)]))
}

fn bind_pose() -> Vec<u8> {
    let bones = [
        ("body", [0.0, 0.0, 0.0]),
        ("clip", [0.2, 0.2, 0.1]),
        ("patron", [0.2, 0.21, 0.1]),
    ];
    let mut bytes = (bones.len() as u16).to_le_bytes().to_vec();
    for (name, at) in bones {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        for value in at.into_iter().chain([0.0; 3]).chain([1.0; 3]) {
            bytes.extend_from_slice(&f32::to_le_bytes(value));
        }
    }
    bytes
}

/// Rigid 36-byte vertices: a box per bone, and its triangles' indices.
fn converted_model() -> Vec<u8> {
    let boxes = [
        (0_u32, [-0.03, -0.06, -0.3], [0.03, 0.1, 0.7]),
        (1, [0.19, 0.19, 0.09], [0.21, 0.21, 0.11]),
        (2, [0.195, 0.205, 0.095], [0.205, 0.215, 0.105]),
    ];
    let mut vertices = Vec::new();
    for (bone, low, high) in boxes {
        for corner in 0..8 {
            let pick = |k: usize| {
                if corner >> k & 1 == 0 {
                    low[k]
                } else {
                    high[k]
                }
            };
            let mut vertex = Vec::new();
            for k in 0..3 {
                vertex.extend_from_slice(&f32::to_le_bytes(pick(k)));
            }
            vertex.extend_from_slice(&bone.to_le_bytes());
            vertex.resize(RIGID_VERTEX_BYTES, 0);
            vertices.extend(vertex);
        }
    }
    let count = 24_u32;
    let indices: Vec<u16> = (0..24).collect();
    let mut bytes = Vec::new();
    for value in [3, 4 + count * 36, count] {
        bytes.extend_from_slice(&u32::to_le_bytes(value));
    }
    bytes.extend(vertices);
    for value in [4, 4 + indices.len() as u32 * 2, indices.len() as u32] {
        bytes.extend_from_slice(&u32::to_le_bytes(value));
    }
    for index in indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

fn settings() -> Vec<u8> {
    let surfaces = ConfigValue::Indexed(vec![ConfigValue::String("gun_s".to_owned())]);
    let lod0 = ConfigValue::Named(vec![named("surfaces", surfaces)]);
    let lods = ConfigValue::Named(vec![named("LOD0", lod0)]);
    config(ConfigValue::Named(vec![named("lod_hierrarchy", lods)]))
}

fn model_skeleton() -> Vec<u8> {
    config(ConfigValue::Named(vec![named(
        "skeleton",
        ConfigValue::String("resources/animations/skeletons/weapons/gun/gun".to_owned()),
    )]))
}

fn rest(translation: [f32; 3], end: f32) -> BoneTracks {
    BoneTracks::rest(translation, [0.0; 3], end)
}

/// The bones, local to their parents: body still, the clip at `clip`,
/// the cartridge 1 cm above it.
fn clip(clip: BoneTracks, end: f32) -> Clip {
    Clip {
        bones: ["object_movement", "body", "clip", "patron"]
            .map(str::to_owned)
            .to_vec(),
        tracks: vec![
            rest([0.0; 3], end),
            rest([0.0; 3], end),
            clip,
            rest([0.0, 0.01, 0.0], end),
        ],
        animation_type: 0,
    }
}

fn idle() -> Clip {
    clip(rest(PARKED, 16.0), 16.0)
}

/// Starts stowed, then lifts the clip out along x.
fn reload_start() -> Clip {
    let times: Vec<f32> = (0..=10).map(|k| k as f32 * 5.0).collect();
    let mut tracks = rest(STOWED, 50.0);
    tracks.channels[0] =
        Channel::fit(&times, &times.iter().map(|t| t * 0.004).collect::<Vec<_>>()).unwrap();
    clip(tracks, 50.0)
}

fn bytes(clip: &Clip) -> (Vec<u8>, Vec<u8>) {
    (clip.to_bspline().unwrap(), clip.to_bones_names().unwrap())
}

fn rig() -> Rig {
    Rig::parse(&bind_pose(), &skeleton()).unwrap()
}

#[test]
fn holding_keeps_the_size_and_changes_only_the_held_bones() {
    let (before, names) = bytes(&idle());
    let pose = reference_pose(&reload_start(), &held(), 0.0).unwrap();
    let after = hold_pose(&before, &names, &pose).unwrap();
    assert_eq!(after.len(), before.len());

    // Every changed byte is a control point of a held bone's curve.
    let offsets = channel_offsets(&before, 4).unwrap();
    let mut held_points = Vec::new();
    for bone in [2, 3] {
        for (at, _) in offsets[bone] {
            for point in 0..4 {
                let slot = at + 4 + point * 8 + 4;
                held_points.extend(slot..slot + 4);
            }
        }
    }
    let changed: Vec<usize> = (0..before.len())
        .filter(|&i| before[i] != after[i])
        .collect();
    assert!(!changed.is_empty());
    assert!(changed.iter().all(|i| held_points.contains(i)));

    let (old, new) = (
        Clip::parse(&before, &names).unwrap(),
        Clip::parse(&after, &names).unwrap(),
    );
    assert_eq!(old.bones, new.bones);
    for bone in [0, 1] {
        assert_eq!(old.tracks[bone].channels, new.tracks[bone].channels);
    }
}

#[test]
fn the_held_bones_take_the_reference_start_for_the_whole_clip() {
    let (before, names) = bytes(&idle());
    let reference = reload_start();
    let pose = reference_pose(&reference, &held(), 0.0).unwrap();
    for k in 0..3 {
        assert!(
            (pose["clip"][k] - STOWED[k]).abs() < 1e-5,
            "{:?}",
            pose["clip"]
        );
    }
    let after = Clip::parse(&hold_pose(&before, &names, &pose).unwrap(), &names).unwrap();
    for (track, bone) in [(2, "clip"), (3, "patron")] {
        for k in 0..=16 {
            let time = k as f32;
            for channel in 0..CHANNELS {
                let held = after.tracks[track].channels[channel].evaluate(time);
                let start = reference.tracks[track].channels[channel].evaluate(0.0);
                assert!((held - start).abs() < 1e-6, "{bone} {channel} at {time}");
                assert!((held - pose[bone][channel]).abs() < 1e-6);
            }
        }
    }
    // The reference itself moves on: the start, not a later frame, was taken.
    assert!((reference.tracks[2].channels[0].evaluate(50.0) - 0.2).abs() < 1e-3);
}

#[test]
fn refuses_to_refit_a_moving_curve() {
    let (moving, names) = bytes(&reload_start());
    let pose = reference_pose(&idle(), &held(), 0.0).unwrap();
    let error = hold_pose(&moving, &names, &pose).unwrap_err().to_string();
    assert!(
        error.contains("clip channel 0 is not a constant curve"),
        "{error}"
    );
}

#[test]
fn a_constant_an_ulp_apart_is_still_held() {
    // As retail writes them: the first control point a float step off.
    let mut parked = idle();
    for channel in &mut parked.tracks[2].channels {
        channel.points[0] = f32::from_bits(channel.points[0].to_bits() + 1);
    }
    let (before, names) = bytes(&parked);
    let pose = reference_pose(&reload_start(), &held(), 0.0).unwrap();
    assert_eq!(
        hold_pose(&before, &names, &pose).unwrap().len(),
        before.len()
    );
}

#[test]
fn refuses_a_bone_the_clip_lacks() {
    let (before, names) = bytes(&idle());
    let mut pose = reference_pose(&reload_start(), &held(), 0.0).unwrap();
    pose.insert("bayonet".to_owned(), [0.0; CHANNELS]);
    let error = hold_pose(&before, &names, &pose).unwrap_err().to_string();
    assert!(error.contains("does not drive bayonet"), "{error}");
    assert!(reference_pose(&idle(), &["bayonet".to_owned()], 0.0).is_err());
}

#[test]
fn the_held_clip_ends_inside_the_weapon() {
    let rig = rig();
    let vertices = rigid_vertices(&converted_model()).unwrap();
    assert_eq!(vertices.len(), 24);
    let parked = farthest_outside(&rig, &vertices, &idle(), &held(), "body").unwrap();
    assert!(parked > 1.0, "parked clip {parked} m out");

    let (before, names) = bytes(&idle());
    let pose = reference_pose(&reload_start(), &held(), 0.0).unwrap();
    let stowed = Clip::parse(&hold_pose(&before, &names, &pose).unwrap(), &names).unwrap();
    let gap = farthest_outside(&rig, &vertices, &stowed, &held(), "body").unwrap();
    assert!(gap <= BOUNDS_TOLERANCE, "stowed clip {gap} m out");
}

#[test]
fn only_rigid_weapon_vertices_are_checked() {
    let mut model = converted_model();
    model[4..8].copy_from_slice(&(4 + 24 * 40_u32).to_le_bytes());
    model[8..12].copy_from_slice(&24_u32.to_le_bytes());
    assert!(rigid_vertices(&model).is_err());
}

struct Fixture {
    temp: TempDir,
    base: std::path::PathBuf,
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let tree = temp.path().join("tree");
        write(
            &tree,
            "animations/skeletons/weapons/gun/gun.skeleton",
            &skeleton(),
        );
        write(&tree, &format!("{MODEL}/skeleton"), &model_skeleton());
        write(&tree, &format!("{MODEL}/settings"), &settings());
        write(&tree, &format!("{MODEL}/render/bind_pose"), &bind_pose());
        write(
            &tree,
            &format!("{MODEL}/render/gun_s/converted_model"),
            &converted_model(),
        );
        for (name, clip) in [
            ("idle", idle()),
            ("shoot", idle()),
            ("reload_start", reload_start()),
        ] {
            let (bspline, names) = bytes(&clip);
            write(&tree, &format!("{CLIPS}/{name}.b-spline"), &bspline);
            write(&tree, &format!("{CLIPS}/{name}.bones_names"), &names);
        }
        let base = temp.path().join("base.db");
        crate::archive::write_folder(&tree, &base).unwrap();
        Self { temp, base }
    }

    fn build(&self, reference: &str, pack: &str) -> Result<Vec<ClipReport>> {
        let clips = ["idle", "shoot"].map(|c| format!("{CLIPS}/{c}"));
        let bounds = BoundsCheck {
            model: MODEL.to_owned(),
            inside_bone: "body".to_owned(),
        };
        build(&ClipPoseOptions {
            base_db: &self.base,
            base_patches: &[],
            clips: &clips,
            bones: &held(),
            reference: &format!("{CLIPS}/{reference}"),
            time: 0.0,
            bounds: Some(&bounds),
            pack_dir: &self.temp.path().join(pack),
            id: "gun-clip-fix",
            version: "1",
            force: false,
        })
    }
}

#[test]
fn builds_a_pack_of_two_replaced_clips_that_checks_against_its_base() {
    let fixture = Fixture::new();
    let reports = fixture.build("reload_start", "pack").unwrap();
    let mut base = ResourceDb::open(&fixture.base).unwrap();
    assert_eq!(reports.len(), 2);
    for report in &reports {
        let stock = base.read(&report.path).unwrap();
        assert_eq!(report.bytes, stock.len());
        assert_eq!(report.base_sha256, sha256_hex(&stock));
        assert!(report.outside_before.unwrap() > 1.0);
        assert!(report.outside_after.unwrap() <= BOUNDS_TOLERANCE);
        let file = fixture.temp.path().join("pack/files").join(&report.path);
        assert_eq!(sha256_hex(&fs::read(file).unwrap()), report.sha256);
    }
    let pack_dir = fixture.temp.path().join("pack");
    let pack = crate::content_pack::Pack::load(&pack_dir).unwrap();
    assert_eq!(pack.manifest.id, "gun-clip-fix");
    assert!(pack.manifest.operations.iter().all(|op| matches!(
        op,
        Operation::ReplaceFile { path, base_sha256, .. }
            if reports.iter().any(|r| &r.path == path && &r.base_sha256 == base_sha256)
    )));
    crate::content_pack::check(&[pack_dir], Some(&fixture.base), &[]).unwrap();
}

#[test]
fn a_pose_that_hides_nothing_fails_the_build() {
    let fixture = Fixture::new();
    let error = fixture.build("idle", "pack").unwrap_err().to_string();
    assert!(error.contains("outside body's box"), "{error}");
}

/// The retail Mosin: `VOSTOK_026G_DB=…/resources.db cargo test -- --ignored`.
#[test]
#[ignore = "needs the retail v0.26g resources.db in VOSTOK_026G_DB"]
fn the_retail_mosin_clip_is_stowed_in_idle_and_shoot() {
    let db = std::path::PathBuf::from(std::env::var("VOSTOK_026G_DB").unwrap());
    let temp = TempDir::new().unwrap();
    let base = "animations/single/weapons/mosin_nagant/item/stand/3rd_view";
    let clips = ["idle", "shoot"].map(|c| format!("{base}/{c}"));
    let bones: Vec<String> = [
        "bn_oboima",
        "bn_patron01",
        "bn_patron02",
        "bn_patron03",
        "bn_patron04",
        "bn_patron05",
    ]
    .map(str::to_owned)
    .to_vec();
    let bounds = BoundsCheck {
        model: "models/weapons/mosin_nagant/mosin_nagant.skinned_model".to_owned(),
        inside_bone: "offset".to_owned(),
    };
    let reports = build(&ClipPoseOptions {
        base_db: &db,
        base_patches: &[],
        clips: &clips,
        bones: &bones,
        reference: &format!("{base}/reload_start"),
        time: 0.0,
        bounds: Some(&bounds),
        pack_dir: &temp.path().join("pack"),
        id: "mosin-clip-fix",
        version: "1",
        force: false,
    })
    .unwrap();
    assert_eq!(
        reports.iter().map(|r| r.bytes).collect::<Vec<_>>(),
        [6556, 8700]
    );
    for report in reports {
        assert!(report.outside_before.unwrap() > 1.2, "{report:?}");
        assert!(
            report.outside_after.unwrap() <= BOUNDS_TOLERANCE,
            "{report:?}"
        );
    }
}
