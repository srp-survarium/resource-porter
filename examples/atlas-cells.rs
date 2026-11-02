// SPDX-License-Identifier: GPL-3.0-or-later
//! Print a lobby icon atlas's size and its occupied cells:
//! `atlas-cells <lobby_menu.swf> <symbol> [<out.png>]` (1-based `row,col`;
//! with a PNG path, the whole atlas is written there too, straight alpha).
use vostok_resource_porter::content_pack::atlas::Swf;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (swf, symbol, png) = match a.as_slice() {
        [swf, symbol] => (swf, symbol, None),
        [swf, symbol, png] => (swf, symbol, Some(png)),
        _ => anyhow::bail!("usage: atlas-cells <lobby_menu.swf> <symbol> [<out.png>]"),
    };
    let bitmap = Swf::parse(&std::fs::read(swf)?)?.bitmap(symbol)?;
    let g = bitmap.grid;
    println!(
        "{symbol}: {}x{} px, {} rows x {} cols",
        bitmap.width,
        bitmap.height,
        bitmap.height.div_ceil(g.step_y),
        bitmap.width / g.step_x
    );
    for (row, col) in bitmap.occupied_cells()? {
        println!("{row},{col}");
    }
    if let Some(path) = png {
        let mut rgba = Vec::with_capacity(bitmap.argb.len());
        for px in bitmap.argb.chunks_exact(4) {
            let (a, r, g, b) = (px[0], px[1], px[2], px[3]);
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                }
            };
            rgba.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, bitmap.width as u32, bitmap.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header()?.write_image_data(&rgba)?;
    }
    Ok(())
}
