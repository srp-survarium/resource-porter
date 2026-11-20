// SPDX-License-Identifier: GPL-3.0-or-later
//! The check's pictures: a gun with something fitted as 0.69d0 places it
//! (left) and as the table and the bundle's models would draw it (right),
//! from two fixed cameras (the side, then from above), in one frame.

use anyhow::Result;

use crate::weapons_026::modules::{Bounds, png, render_side};

/// A mesh in the gun's mesh space, and its colour.
pub struct Shape {
    pub points: Vec<[f32; 3]>,
    pub triangles: Vec<[u16; 3]>,
    pub colour: [u8; 3],
}

/// The gun's body, its own parts, a carrier, the fitted module.
pub const BODY: [u8; 3] = [150, 150, 150];
pub const OWN: [u8; 3] = [190, 190, 170];
pub const CARRIER: [u8; 3] = [90, 150, 220];
pub const FITTED: [u8; 3] = [240, 150, 50];

/// From above: the viewer at +y, the gun's length across.
fn from_above(p: [f32; 3]) -> [f32; 3] {
    [p[1], -p[0], p[2]]
}

fn view(shapes: &[Shape], frame: &[[f32; 3]], width: usize, turn: fn([f32; 3]) -> [f32; 3]) -> Result<(usize, usize, Vec<u8>)> {
    let turned: Vec<Vec<[f32; 3]>> = shapes.iter().map(|s| s.points.iter().map(|p| turn(*p)).collect()).collect();
    let corners: Vec<[f32; 3]> = frame.iter().map(|p| turn(*p)).collect();
    let mut parts: Vec<(&[[f32; 3]], &[[u16; 3]], [u8; 3])> = shapes
        .iter()
        .zip(&turned)
        .map(|(s, points)| (points.as_slice(), s.triangles.as_slice(), s.colour))
        .collect();
    // The frame's corners take part in the bounds only: both sides get one scale.
    parts.push((&corners, &[], [0; 3]));
    render_side(&parts, width)
}

/// `left` beside `right`, the side views over the views from above, as a PNG.
pub fn pair(left: &[Shape], right: &[Shape], width: usize) -> Result<Vec<u8>> {
    let all = left.iter().chain(right).flat_map(|s| s.points.iter().copied());
    let Some(b) = Bounds::of(all) else {
        anyhow::bail!("nothing to render");
    };
    let frame = [b.min, b.max];
    let same = |p: [f32; 3]| p;
    let cells = [
        view(left, &frame, width, same)?,
        view(right, &frame, width, same)?,
        view(left, &frame, width, from_above)?,
        view(right, &frame, width, from_above)?,
    ];
    let gap = 8;
    let (top, bottom) = (cells[0].1.max(cells[1].1), cells[2].1.max(cells[3].1));
    let (w, h) = (2 * width + gap, top + gap + bottom);
    let mut out = vec![0u8; w * h * 4];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&[24, 24, 28, 255]);
    }
    for (k, (cw, ch, rgba)) in cells.iter().enumerate() {
        let (x0, y0) = ((k % 2) * (width + gap), if k < 2 { 0 } else { top + gap });
        for y in 0..*ch {
            for x in 0..*cw {
                let from = (y * cw + x) * 4;
                if rgba[from + 3] != 0 {
                    let to = ((y0 + y) * w + x0 + x) * 4;
                    out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
                }
            }
        }
    }
    png(w, h, &out)
}
