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
    ensure!(
        w <= CELL_WIDTH && h <= CELL_HEIGHT,
        "icon is {w}x{h}, larger than a cell"
    );
    let (x0, y0) = ((CELL_WIDTH - w) / 2, (CELL_HEIGHT - h) / 2);
    let mut cell = vec![0u8; CELL_WIDTH * CELL_HEIGHT * 4];
    for y in 0..h {
        for x in 0..w {
            let p = &buf[(y * w + x) * channels..(y * w + x + 1) * channels];
            let at = ((y0 + y) * CELL_WIDTH + x0 + x) * 4;
            cell[at..at + 3].copy_from_slice(&p[..3]);
            cell[at + 3] = if channels == 4 { p[3] } else { 255 };
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

/// A kill log image as the hud takes it: a PNG, as the donor has it (the
/// hud tints it per side; `make_weapon_icons.py` checks its size).
pub fn kill_feed_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width <= 256 && info.height <= 64,
        "kill log image is {}x{}",
        info.width,
        info.height
    );
    Ok(bytes.to_vec())
}
