// SPDX-License-Identifier: GPL-3.0-or-later
//! Print a weapon bone's model-space frame at rest and at a clip's start:
//! `probe-weapon-pose <db> <model root> <skeleton> <clip (no extension)> <069|026> <bone>..`.
use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::skeleton_animation::{Clip, Rig, basis_to_euler};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0])?;
    let bind = db.read(&format!("{}/render/bind_pose", a[1]))?;
    let skeleton = db.read(&a[2])?;
    let rig = Rig::parse(&bind, &skeleton)?;
    let mut spline = db.read(&format!("{}.b-spline", a[3]))?;
    if a[4] == "069" {
        spline = vostok_resource_porter::weapons_069d::convert::bspline_069_to_026(&spline)?;
    }
    let clip = Clip::parse(&spline, &db.read(&format!("{}.bones_names", a[3]))?)?;
    let rest = rig.rest();
    let pose = rig.pose(&clip, 0.0);
    for bone in &a[5..] {
        let i = rig
            .index(bone)
            .ok_or_else(|| anyhow::anyhow!("no {bone}"))?;
        println!(
            "{bone}: rest t={:?} r={:?} | idle t={:?} r={:?}",
            rest[i].1,
            basis_to_euler(&rest[i].0),
            pose[i].1,
            basis_to_euler(&pose[i].0)
        );
    }
    Ok(())
}
