// SPDX-License-Identifier: GPL-3.0-or-later
//! Each surface's bounds of a 0.26g skinned model, from a db or a folder:
//! `probe-mesh-bounds <db or folder> <model root>`.
use std::path::Path;

use vostok_resource_porter::ResourceDb;
use vostok_resource_porter::weapons_026::donor_069d::convert::Skinned;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = &a[1];
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    if Path::new(&a[0]).is_dir() {
        let dir = Path::new(&a[0]).join(root).join("render");
        for e in std::fs::read_dir(&dir)? {
            let p = e?.path().join("converted_model");
            if p.exists() {
                files.push((
                    p.parent()
                        .unwrap()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into(),
                    std::fs::read(p)?,
                ));
            }
        }
    } else {
        let mut db = ResourceDb::open(&a[0])?;
        let prefix = format!("{root}/render/");
        let paths: Vec<String> = db.paths_with_prefix(&prefix).map(str::to_owned).collect();
        for path in paths {
            if let Some(s) = path
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix("/converted_model"))
            {
                let s = s.to_owned();
                files.push((s, db.read(&path)?));
            }
        }
    }
    files.sort();
    for (name, b) in files {
        let m = Skinned::parse_026(&b)?;
        let (lo, hi) = m.bounds();
        println!("{name}: {} v lo {lo:.4?} hi {hi:.4?}", m.vertices.len());
        // Sight probes: vertices above `y` within a z band (a front post, a rear notch).
        if a.len() > 2 {
            let f: Vec<f32> = a[2..].iter().map(|s| s.parse().unwrap()).collect();
            for band in f.chunks(3) {
                let (z0, z1, y0) = (band[0], band[1], band[2]);
                let pts: Vec<[f32; 3]> = (0..m.vertices.len())
                    .map(|i| m.position(i))
                    .filter(|p| p[2] >= z0 && p[2] <= z1 && p[1] >= y0)
                    .collect();
                if pts.is_empty() {
                    continue;
                }
                let (mut xl, mut xh, mut yh) =
                    (f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
                for p in &pts {
                    xl = xl.min(p[0]);
                    xh = xh.max(p[0]);
                    yh = yh.max(p[1]);
                }
                println!(
                    "  z {z0}..{z1} y>={y0}: {} v, x {xl:.4}..{xh:.4} (mid {:.4}), top y {yh:.4}",
                    pts.len(),
                    (xl + xh) / 2.0
                );
            }
        }
    }
    Ok(())
}
