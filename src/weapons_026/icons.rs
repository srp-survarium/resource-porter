// SPDX-License-Identifier: GPL-3.0-or-later
//! A gun's icons: its lobby cell (`LobbyMenu_WeaponIcons`, drawn by the
//! pack's `atlas_icons_add`) and its kill feed image (the hud movie's
//! `CombatLogWeapon<icon>` bitmap, which skhid-srp's
//! `scripts/kill-feed-icons` adds to the movie).

use anyhow::{Result, bail, ensure};

use crate::content_pack::atlas::{CELL_HEIGHT, CELL_WIDTH};

/// A donor icon PNG centred in a lobby cell (RGBA PNG of the cell's size).
pub fn cell_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    ensure!(info.bit_depth == png::BitDepth::Eight, "icon is not 8-bit");
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => bail!("icon colour type {other:?} after expansion"),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    // Straight RGBA, then the opaque rows' bounds.
    let rgba: Vec<[u8; 4]> = (0..w * h)
        .map(|i| {
            let p = &buf[i * channels..(i + 1) * channels];
            [p[0], p[1], p[2], if channels == 4 { p[3] } else { 255 }]
        })
        .collect();
    let row_empty = |y: usize| (0..w).all(|x| rgba[y * w + x][3] == 0);
    let top = (0..h).find(|&y| !row_empty(y)).unwrap_or(0);
    let bottom = (0..h).rev().find(|&y| !row_empty(y)).map_or(h, |y| y + 1);
    let art_h = bottom - top;
    // 0.31a2's icons are a cell and its 8-pixel gap (171x91); art taller
    // than a cell is scaled down to it (area average), its aspect kept.
    let scale = (CELL_HEIGHT as f32 / art_h as f32)
        .min(CELL_WIDTH as f32 / w as f32)
        .min(1.0);
    let (nw, nh) = (
        ((w as f32 * scale).round() as usize).max(1),
        ((art_h as f32 * scale).round() as usize).max(1),
    );
    ensure!(nw <= CELL_WIDTH && nh <= CELL_HEIGHT, "icon is {w}x{h}");
    let (x0, y0) = ((CELL_WIDTH - nw) / 2, (CELL_HEIGHT - nh) / 2);
    let mut cell = vec![0u8; CELL_WIDTH * CELL_HEIGHT * 4];
    for y in 0..nh {
        for x in 0..nw {
            // The source area of this pixel, premultiplied average.
            let (sx0, sx1) = (x as f32 / scale, (x + 1) as f32 / scale);
            let (sy0, sy1) = (y as f32 / scale, (y + 1) as f32 / scale);
            let mut acc = [0f32; 4];
            let mut area = 0f32;
            let mut sy = sy0.floor() as usize;
            while (sy as f32) < sy1 && sy < art_h {
                let wy = (sy1.min(sy as f32 + 1.0) - sy0.max(sy as f32)).max(0.0);
                let mut sx = sx0.floor() as usize;
                while (sx as f32) < sx1 && sx < w {
                    let wx = (sx1.min(sx as f32 + 1.0) - sx0.max(sx as f32)).max(0.0);
                    let p = rgba[(top + sy) * w + sx];
                    let a = f32::from(p[3]) / 255.0;
                    let k = wx * wy;
                    for c in 0..3 {
                        acc[c] += f32::from(p[c]) * a * k;
                    }
                    acc[3] += a * k;
                    area += k;
                    sx += 1;
                }
                sy += 1;
            }
            let at = ((y0 + y) * CELL_WIDTH + x0 + x) * 4;
            if area > 0.0 && acc[3] > 0.0 {
                for c in 0..3 {
                    cell[at + c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                cell[at + 3] = (acc[3] / area * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, CELL_WIDTH as u32, CELL_HEIGHT as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&cell)?;
    }
    Ok(out)
}

/// The kill feed image's file in a bundle.
pub fn kill_feed_name(icon: i32) -> String {
    format!("kill-feed/CombatLogWeapon{icon}.png")
}

/// A kill log image as the hud takes it: an 8-bit RGBA PNG, not interlaced
/// (the hud build's bitmaps; 0.31a2's are palette images, interlaced), its
/// pixels the donor's (the hud tints it per side).
pub fn kill_feed_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "kill log image is not 8-bit"
    );
    let (w, h) = (info.width as usize, info.height as usize);
    ensure!(w <= 256 && h <= 64, "kill log image is {w}x{h}");
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2]
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => bail!("kill log image colour type {other:?} after expansion"),
    };
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba)?;
    }
    Ok(out)
}

/// `png` (an RGBA cell) with the rectangles `[x0, y0, x1, y1)` made
/// transparent (cut to the image).
pub fn erase_rects(png_bytes: &[u8], rects: &[[u32; 4]]) -> Result<Vec<u8>> {
    if rects.is_empty() {
        return Ok(png_bytes.to_vec());
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "a cell is 8-bit RGBA"
    );
    let (w, h) = (info.width, info.height);
    for &[x0, y0, x1, y1] in rects {
        ensure!(x0 < x1 && y0 < y1, "an empty rectangle {x0},{y0},{x1},{y1}");
        for y in y0.min(h)..y1.min(h) {
            for x in x0.min(w)..x1.min(w) {
                let at = ((y * w + x) * 4) as usize;
                buf[at..at + 4].fill(0);
            }
        }
    }
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(&buf[..(w * h * 4) as usize])?;
    }
    Ok(out)
}
