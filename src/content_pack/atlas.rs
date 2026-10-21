// SPDX-License-Identifier: GPL-3.0-or-later
//! Lobby icon atlases inside `flash_movies/lobby_menu.swf`.
//!
//! Each atlas is one bitmap exported under a symbol (`LobbyMenu_EquipmentIcons`,
//! `LobbyMenu_DecalsIcons`, ...), which `LobbyMenu.parseSpriteSheets` cuts into
//! cells of its own size ([`grid`]). Items address a cell through
//! `ui_desc.icon = [row, col]` (1-based): the cell's top left is
//! `((col - 1) * step_x, (row - 1) * step_y)`. The equipment atlas's cells are
//! 171x83 pixels on a 171x91 grid. Adding a column widens the bitmap without
//! moving any existing cell, so stock icon indices stay valid.
//!
//! Most atlases are `DefineBitsLossless2` bitmaps (format 5, premultiplied
//! ARGB). The big camouflage atlas is a `DefineBitsJPEG2`: it is read as the
//! decoder gives it (opaque) and written back as a `DefineBitsLossless2` under
//! the same character id, which the movie's `BitmapAsset` takes alike. Its
//! empty cells are solid red, not transparent, so an icon drawn there names
//! the cell it replaces (`replaces_sha256`).

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

pub const DEFAULT_SWF: &str = "flash_movies/lobby_menu.swf";
pub const DEFAULT_SYMBOL: &str = "LobbyMenu_EquipmentIcons";
pub const CELL_WIDTH: usize = 171;
pub const CELL_HEIGHT: usize = 83;
pub const ROW_PITCH: usize = 91;

const TAG_DEFINE_BITS_JPEG2: u16 = 21;
const TAG_DEFINE_BITS_LOSSLESS2: u16 = 36;
const TAG_SYMBOL_CLASS: u16 = 76;

/// An atlas's cells: their size and the step from one cell to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    pub step_x: usize,
    pub step_y: usize,
}

impl Grid {
    const fn square(size: usize) -> Self {
        Self {
            width: size,
            height: size,
            step_x: size,
            step_y: size,
        }
    }

    /// Bytes of one cell's premultiplied ARGB pixels.
    pub fn cell_len(&self) -> usize {
        self.width * self.height * 4
    }
}

/// The equipment, weapon, ammo and voucher atlases' cells: 171x83 with 8
/// pixels between rows (`IconContainer(.., 171, 83, .., 0, 0, 8, false)`).
pub const EQUIPMENT_GRID: Grid = Grid {
    width: CELL_WIDTH,
    height: CELL_HEIGHT,
    step_x: CELL_WIDTH,
    step_y: ROW_PITCH,
};

/// The cells of the atlas exported as `symbol`, as `LobbyMenu.parseSpriteSheets`
/// (0.26g) cuts it. An atlas it does not list is refused, never guessed.
pub fn grid(symbol: &str) -> Result<Grid> {
    Ok(match symbol {
        "LobbyMenu_EquipmentIcons"
        | "LobbyMenu_WeaponIcons"
        | "LobbyMenu_AmmoIcons"
        | "LobbyMenu_VoucherIcons" => EQUIPMENT_GRID,
        // `IconContainer("decals", 82, 82, ..)`: the decals' icons.
        "LobbyMenu_DecalsIcons" => Grid::square(82),
        // `IconContainer("camo", 72, 72, ..)`: the stash's and the shop's.
        "LobbyMenu_CamoIcons" => Grid::square(72),
        // `IconContainer("camo_big", 140, 140, ..)`: the upgrade window's slots.
        "LobbyMenu_CamoIconsBig" => Grid::square(140),
        other => bail!("{other:?} is not an icon atlas whose cells are known"),
    })
}

struct Tag {
    kind: u16,
    /// The tag's original header and payload, reused verbatim when unchanged.
    raw: Vec<u8>,
    payload_offset: usize,
}

impl Tag {
    fn payload(&self) -> &[u8] {
        &self.raw[self.payload_offset..]
    }
}

pub struct Swf {
    compressed: bool,
    version: u8,
    /// Movie header after the 8-byte file header (frame rect, rate, count).
    header: Vec<u8>,
    tags: Vec<Tag>,
}

/// A decoded ARGB (premultiplied) bitmap.
#[derive(Clone, PartialEq)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub argb: Vec<u8>,
    /// The atlas's cells ([`grid`] of its symbol).
    pub grid: Grid,
}

impl Swf {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() >= 8, "SWF is shorter than its header");
        let compressed = match &bytes[..3] {
            b"CWS" => true,
            b"FWS" => false,
            _ => bail!("only zlib (CWS) or uncompressed (FWS) SWF files are supported"),
        };
        let version = bytes[3];
        let body = if compressed {
            let mut body = Vec::new();
            ZlibDecoder::new(&bytes[8..])
                .read_to_end(&mut body)
                .context("decompressing SWF body")?;
            body
        } else {
            bytes[8..].to_vec()
        };
        ensure!(!body.is_empty(), "SWF body is empty");
        let rect_bits = 5 + 4 * usize::from(body[0] >> 3);
        let header_len = rect_bits.div_ceil(8) + 4;
        ensure!(body.len() >= header_len, "SWF movie header is truncated");
        let mut tags = Vec::new();
        let mut at = header_len;
        while at < body.len() {
            ensure!(at + 2 <= body.len(), "SWF tag header is truncated");
            let code = u16::from_le_bytes([body[at], body[at + 1]]);
            let mut header = 2;
            let mut size = usize::from(code & 0x3f);
            if size == 0x3f {
                ensure!(at + 6 <= body.len(), "SWF long tag header is truncated");
                size = u32::from_le_bytes(body[at + 2..at + 6].try_into().unwrap()) as usize;
                header = 6;
            }
            let end = at + header + size;
            ensure!(end <= body.len(), "SWF tag runs past the movie");
            tags.push(Tag {
                kind: code >> 6,
                raw: body[at..end].to_vec(),
                payload_offset: header,
            });
            at = end;
        }
        Ok(Self {
            compressed,
            version,
            header: body[..header_len].to_vec(),
            tags,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut body = self.header.clone();
        for tag in &self.tags {
            body.extend_from_slice(&tag.raw);
        }
        let length = u32::try_from(8 + body.len()).context("SWF exceeds 4 GiB")?;
        let mut output = Vec::with_capacity(body.len() / 2);
        output.extend_from_slice(if self.compressed { b"CWS" } else { b"FWS" });
        output.push(self.version);
        output.extend_from_slice(&length.to_le_bytes());
        if self.compressed {
            let mut encoder = ZlibEncoder::new(output, Compression::best());
            encoder.write_all(&body)?;
            output = encoder.finish()?;
        } else {
            output.extend_from_slice(&body);
        }
        Ok(output)
    }

    fn symbol_id(&self, symbol: &str) -> Result<u16> {
        for tag in self.tags.iter().filter(|tag| tag.kind == TAG_SYMBOL_CLASS) {
            let payload = tag.payload();
            ensure!(payload.len() >= 2, "SymbolClass tag is truncated");
            let count = u16::from_le_bytes([payload[0], payload[1]]);
            let mut at = 2;
            for _ in 0..count {
                ensure!(at + 2 <= payload.len(), "SymbolClass entry is truncated");
                let id = u16::from_le_bytes([payload[at], payload[at + 1]]);
                at += 2;
                let end = payload[at..]
                    .iter()
                    .position(|byte| *byte == 0)
                    .context("SymbolClass name is unterminated")?
                    + at;
                if &payload[at..end] == symbol.as_bytes() {
                    return Ok(id);
                }
                at = end + 1;
            }
        }
        bail!("SWF exports no symbol {symbol:?}")
    }

    fn bitmap_tag(&self, symbol: &str) -> Result<(usize, u16)> {
        let id = self.symbol_id(symbol)?;
        let index = self
            .tags
            .iter()
            .position(|tag| {
                matches!(tag.kind, TAG_DEFINE_BITS_LOSSLESS2 | TAG_DEFINE_BITS_JPEG2)
                    && tag.payload().len() >= 7
                    && u16::from_le_bytes([tag.payload()[0], tag.payload()[1]]) == id
            })
            .with_context(|| {
                format!("symbol {symbol:?} is not a DefineBitsLossless2 or DefineBitsJPEG2 bitmap")
            })?;
        Ok((index, id))
    }

    pub fn bitmap(&self, symbol: &str) -> Result<Bitmap> {
        let grid = grid(symbol)?;
        let (index, _) = self.bitmap_tag(symbol)?;
        let payload = self.tags[index].payload();
        if self.tags[index].kind == TAG_DEFINE_BITS_JPEG2 {
            return decode_jpeg(&payload[2..], grid)
                .with_context(|| format!("decoding the JPEG of {symbol:?}"));
        }
        ensure!(
            payload[2] == 5,
            "atlas bitmap is not 32-bit ARGB (format {})",
            payload[2]
        );
        let width = usize::from(u16::from_le_bytes([payload[3], payload[4]]));
        let height = usize::from(u16::from_le_bytes([payload[5], payload[6]]));
        let mut argb = Vec::with_capacity(width * height * 4);
        ZlibDecoder::new(&payload[7..])
            .read_to_end(&mut argb)
            .context("decompressing atlas bitmap")?;
        ensure!(
            argb.len() == width * height * 4,
            "atlas bitmap holds {} bytes, expected {}",
            argb.len(),
            width * height * 4
        );
        Ok(Bitmap {
            width,
            height,
            argb,
            grid,
        })
    }

    /// Writes `bitmap` as the symbol's `DefineBitsLossless2` (a JPEG atlas
    /// becomes lossless under the same character id).
    pub fn set_bitmap(&mut self, symbol: &str, bitmap: &Bitmap) -> Result<()> {
        let (index, id) = self.bitmap_tag(symbol)?;
        let mut payload = Vec::new();
        payload.extend_from_slice(&id.to_le_bytes());
        payload.push(5);
        payload.extend_from_slice(&u16::try_from(bitmap.width)?.to_le_bytes());
        payload.extend_from_slice(&u16::try_from(bitmap.height)?.to_le_bytes());
        let mut encoder = ZlibEncoder::new(payload, Compression::best());
        encoder.write_all(&bitmap.argb)?;
        let payload = encoder.finish()?;
        let mut raw = Vec::with_capacity(payload.len() + 6);
        raw.extend_from_slice(&((TAG_DEFINE_BITS_LOSSLESS2 << 6) | 0x3f).to_le_bytes());
        raw.extend_from_slice(&u32::try_from(payload.len())?.to_le_bytes());
        raw.extend_from_slice(&payload);
        self.tags[index] = Tag {
            kind: TAG_DEFINE_BITS_LOSSLESS2,
            raw,
            payload_offset: 6,
        };
        Ok(())
    }

    /// True when both movies are identical apart from the named bitmap.
    pub fn same_except_bitmap(&self, other: &Swf, symbol: &str) -> Result<bool> {
        if self.version != other.version
            || self.header != other.header
            || self.tags.len() != other.tags.len()
        {
            return Ok(false);
        }
        let (left, _) = self.bitmap_tag(symbol)?;
        let (right, _) = other.bitmap_tag(symbol)?;
        Ok(left == right
            && self
                .tags
                .iter()
                .zip(&other.tags)
                .enumerate()
                .all(|(index, (a, b))| {
                    index == left || (a.kind == b.kind && a.payload() == b.payload())
                }))
    }
}

/// A `DefineBitsJPEG2` image (its JPEG stream, after the character id) as an
/// opaque premultiplied ARGB bitmap. An SWF writer's leading `FF D9 FF D8`
/// (an empty image before the real one) is skipped.
fn decode_jpeg(stream: &[u8], grid: Grid) -> Result<Bitmap> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let stream = stream
        .strip_prefix(&[0xff, 0xd9, 0xff, 0xd8][..])
        .unwrap_or(stream);
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(stream, options);
    let rgb = decoder
        .decode()
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let info = decoder.info().context("the JPEG has no header")?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    ensure!(
        rgb.len() == width * height * 3,
        "the JPEG decoded to {} bytes, expected {width}x{height} RGB",
        rgb.len()
    );
    let argb = rgb
        .chunks_exact(3)
        .flat_map(|pixel| [255, pixel[0], pixel[1], pixel[2]])
        .collect();
    Ok(Bitmap {
        width,
        height,
        argb,
        grid,
    })
}

/// Top-left pixel of a 1-based `[row, col]` cell of `grid`.
pub fn cell_origin(grid: Grid, row: u32, col: u32) -> Result<(usize, usize)> {
    ensure!(
        row >= 1 && col >= 1,
        "atlas cells are 1-based: [{row}, {col}]"
    );
    Ok((
        (col as usize - 1) * grid.step_x,
        (row as usize - 1) * grid.step_y,
    ))
}

impl Bitmap {
    /// The cell's ARGB pixels; area beyond the bitmap reads as empty.
    pub fn cell(&self, row: u32, col: u32) -> Result<Vec<u8>> {
        let grid = self.grid;
        let (x, y) = cell_origin(grid, row, col)?;
        let mut output = vec![0; grid.cell_len()];
        for line in 0..grid.height {
            if y + line >= self.height {
                break;
            }
            for column in 0..grid.width {
                if x + column >= self.width {
                    break;
                }
                let source = ((y + line) * self.width + x + column) * 4;
                let target = (line * grid.width + column) * 4;
                output[target..target + 4].copy_from_slice(&self.argb[source..source + 4]);
            }
        }
        Ok(output)
    }

    /// Write a cell, widening the bitmap to whole extra columns if needed.
    pub fn put_cell(&mut self, row: u32, col: u32, argb: &[u8]) -> Result<()> {
        let grid = self.grid;
        ensure!(
            argb.len() == grid.cell_len(),
            "icon is not {}x{} ARGB",
            grid.width,
            grid.height
        );
        let (x, y) = cell_origin(grid, row, col)?;
        ensure!(
            y + grid.height <= self.height,
            "atlas row {row} does not fit the {}-pixel-high bitmap",
            self.height
        );
        if x + grid.width > self.width {
            self.widen(x + grid.width);
        }
        for line in 0..grid.height {
            let target = ((y + line) * self.width + x) * 4;
            let source = line * grid.width * 4;
            self.argb[target..target + grid.width * 4]
                .copy_from_slice(&argb[source..source + grid.width * 4]);
        }
        Ok(())
    }

    fn widen(&mut self, width: usize) {
        let mut argb = vec![0; width * self.height * 4];
        for line in 0..self.height {
            let source = line * self.width * 4;
            argb[line * width * 4..line * width * 4 + self.width * 4]
                .copy_from_slice(&self.argb[source..source + self.width * 4]);
        }
        self.width = width;
        self.argb = argb;
    }

    /// Every occupied cell, in row-major order.
    pub fn occupied_cells(&self) -> Result<Vec<(u32, u32)>> {
        let grid = self.grid;
        let mut cells = Vec::new();
        let columns = self.width.div_ceil(grid.step_x) as u32;
        let rows = (self.height / grid.step_y
            + usize::from(self.height % grid.step_y >= grid.height)) as u32;
        for row in 1..=rows {
            for col in 1..=columns {
                if self.cell(row, col)?.iter().any(|byte| *byte != 0) {
                    cells.push((row, col));
                }
            }
        }
        Ok(cells)
    }
}

/// Read a PNG of one `grid` cell as premultiplied ARGB.
pub fn read_icon_png(path: &Path, premultiplied: bool, grid: Grid) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::fs::File::open(path)?);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer)?;
    ensure!(
        frame.width as usize == grid.width && frame.height as usize == grid.height,
        "icon {} is {}x{}, expected {}x{}",
        path.display(),
        frame.width,
        frame.height,
        grid.width,
        grid.height
    );
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => bail!("icon {} must be RGB or RGBA, not {other:?}", path.display()),
    };
    let mut argb = Vec::with_capacity(grid.cell_len());
    for pixel in buffer[..frame.buffer_size()].chunks_exact(channels) {
        let alpha = if channels == 4 { pixel[3] } else { 255 };
        argb.push(alpha);
        for channel in &pixel[..3] {
            argb.push(if premultiplied {
                *channel
            } else {
                ((u16::from(*channel) * u16::from(alpha) + 127) / 255) as u8
            });
        }
    }
    Ok(argb)
}

/// Write premultiplied ARGB cell pixels as an RGBA PNG holding the
/// premultiplied channels unchanged (read back with `premultiplied: true`).
pub fn write_icon_png(path: &Path, argb: &[u8], grid: Grid) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, grid.width as u32, grid.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let rgba: Vec<u8> = argb
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[1], pixel[2], pixel[3], pixel[0]])
        .collect();
    writer.write_image_data(&rgba)?;
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny CWS movie with a SymbolClass export and a lossless bitmap.
    pub fn synthetic_swf(width: u16, height: u16) -> Vec<u8> {
        synthetic_swf_of(&[(DEFAULT_SYMBOL, width, height)])
    }

    /// A movie of one empty bitmap per `(symbol, width, height)`, as
    /// lobby_menu.swf holds its equipment, weapon and ammo icon atlases.
    pub fn synthetic_swf_of(atlases: &[(&str, u16, u16)]) -> Vec<u8> {
        let mut body = vec![0x08, 0x00, 0x00, 0x18, 0x01, 0x00];
        let mut symbols = (atlases.len() as u16).to_le_bytes().to_vec();
        for (index, (name, width, height)) in atlases.iter().enumerate() {
            let id = 7 + index as u16;
            let mut bitmap = id.to_le_bytes().to_vec();
            bitmap.push(5);
            bitmap.extend_from_slice(&width.to_le_bytes());
            bitmap.extend_from_slice(&height.to_le_bytes());
            let mut encoder = ZlibEncoder::new(bitmap, Compression::best());
            encoder
                .write_all(&vec![0; usize::from(*width) * usize::from(*height) * 4])
                .unwrap();
            let bitmap = encoder.finish().unwrap();
            body.extend_from_slice(&((36_u16 << 6) | 0x3f).to_le_bytes());
            body.extend_from_slice(&(bitmap.len() as u32).to_le_bytes());
            body.extend_from_slice(&bitmap);
            symbols.extend_from_slice(&id.to_le_bytes());
            symbols.extend_from_slice(name.as_bytes());
            symbols.push(0);
        }
        if symbols.len() < 0x3f {
            body.extend_from_slice(&((76_u16 << 6) | symbols.len() as u16).to_le_bytes());
        } else {
            body.extend_from_slice(&((76_u16 << 6) | 0x3f).to_le_bytes());
            body.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
        }
        body.extend_from_slice(&symbols);
        body.extend_from_slice(&[0, 0]);
        let mut swf = Swf {
            compressed: false,
            version: 10,
            header: Vec::new(),
            tags: Vec::new(),
        };
        let mut raw = b"FWS".to_vec();
        raw.push(10);
        raw.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        raw.extend_from_slice(&body);
        let parsed = Swf::parse(&raw).unwrap();
        swf.header = parsed.header;
        swf.tags = parsed.tags;
        swf.compressed = true;
        swf.to_bytes().unwrap()
    }

    #[test]
    fn adds_a_column_without_moving_existing_pixels() {
        let bytes = synthetic_swf(171, 182);
        let mut swf = Swf::parse(&bytes).unwrap();
        let mut bitmap = swf.bitmap(DEFAULT_SYMBOL).unwrap();
        bitmap.argb[0] = 255;
        let icon = vec![9; CELL_WIDTH * CELL_HEIGHT * 4];
        bitmap.put_cell(2, 2, &icon).unwrap();
        assert_eq!(bitmap.width, 342);
        assert_eq!(bitmap.argb[0], 255);
        swf.set_bitmap(DEFAULT_SYMBOL, &bitmap).unwrap();
        let reread = Swf::parse(&swf.to_bytes().unwrap()).unwrap();
        let decoded = reread.bitmap(DEFAULT_SYMBOL).unwrap();
        assert_eq!(decoded.cell(2, 2).unwrap(), icon);
        assert_eq!(decoded.occupied_cells().unwrap(), vec![(1, 1), (2, 2)]);
        assert!(
            reread
                .same_except_bitmap(&Swf::parse(&bytes).unwrap(), DEFAULT_SYMBOL)
                .unwrap()
        );
    }

    #[test]
    fn decal_cells_are_82_pixels_square() {
        let symbol = "LobbyMenu_DecalsIcons";
        let mut swf = Swf::parse(&synthetic_swf_of(&[(symbol, 164, 164)])).unwrap();
        let mut bitmap = swf.bitmap(symbol).unwrap();
        assert_eq!(bitmap.grid, grid(symbol).unwrap());
        assert_eq!(cell_origin(bitmap.grid, 2, 2).unwrap(), (82, 82));
        let icon = vec![7; 82 * 82 * 4];
        assert!(bitmap.put_cell(1, 1, &vec![7; CELL_WIDTH * CELL_HEIGHT * 4]).is_err());
        bitmap.put_cell(2, 2, &icon).unwrap();
        assert_eq!(bitmap.width, 164);
        swf.set_bitmap(symbol, &bitmap).unwrap();
        let decoded = Swf::parse(&swf.to_bytes().unwrap())
            .unwrap()
            .bitmap(symbol)
            .unwrap();
        assert_eq!(decoded.cell(2, 2).unwrap(), icon);
        assert_eq!(decoded.occupied_cells().unwrap(), vec![(2, 2)]);
    }

    #[test]
    fn an_unknown_atlas_is_refused() {
        assert!(grid("LobbyMenu_FactionIcons").is_err());
        let swf = Swf::parse(&synthetic_swf_of(&[("LobbyMenu_FactionIcons", 82, 82)])).unwrap();
        assert!(swf.bitmap("LobbyMenu_FactionIcons").is_err());
    }

    /// A movie whose `LobbyMenu_CamoIconsBig` is a DefineBitsJPEG2 of two
    /// 140x140 cells: blue, then the atlas's empty red.
    fn jpeg_swf() -> Vec<u8> {
        let jpeg = include_bytes!("testdata/camo_big_2x1.jpg");
        let mut body = vec![0x08, 0x00, 0x00, 0x18, 0x01, 0x00];
        let mut payload = 9_u16.to_le_bytes().to_vec();
        payload.extend_from_slice(jpeg);
        body.extend_from_slice(&((TAG_DEFINE_BITS_JPEG2 << 6) | 0x3f).to_le_bytes());
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&payload);
        let mut symbols = 1_u16.to_le_bytes().to_vec();
        symbols.extend_from_slice(&9_u16.to_le_bytes());
        symbols.extend_from_slice(b"LobbyMenu_CamoIconsBig\0");
        body.extend_from_slice(&((TAG_SYMBOL_CLASS << 6) | symbols.len() as u16).to_le_bytes());
        body.extend_from_slice(&symbols);
        body.extend_from_slice(&[0, 0]);
        let mut raw = b"FWS".to_vec();
        raw.push(10);
        raw.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        raw.extend_from_slice(&body);
        raw
    }

    #[test]
    fn a_jpeg_atlas_is_written_back_lossless() {
        let symbol = "LobbyMenu_CamoIconsBig";
        let mut swf = Swf::parse(&jpeg_swf()).unwrap();
        let mut bitmap = swf.bitmap(symbol).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (280, 140));
        let blue = bitmap.cell(1, 1).unwrap();
        let red = bitmap.cell(1, 2).unwrap();
        for (cell, (r, b)) in [(&blue, (0, 200)), (&red, (254, 0))] {
            for pixel in cell.chunks_exact(4) {
                assert_eq!(pixel[0], 255);
                assert!(pixel[1].abs_diff(r) <= 3 && pixel[3].abs_diff(b) <= 3);
            }
        }
        // The empty red cell is "occupied": a pack names it by its hash.
        assert_eq!(bitmap.occupied_cells().unwrap(), vec![(1, 1), (1, 2)]);
        let icon = vec![255; 140 * 140 * 4];
        bitmap.put_cell(1, 2, &icon).unwrap();
        swf.set_bitmap(symbol, &bitmap).unwrap();
        let reread = Swf::parse(&swf.to_bytes().unwrap()).unwrap();
        let (index, id) = reread.bitmap_tag(symbol).unwrap();
        assert_eq!((reread.tags[index].kind, id), (TAG_DEFINE_BITS_LOSSLESS2, 9));
        let decoded = reread.bitmap(symbol).unwrap();
        assert_eq!(decoded.cell(1, 1).unwrap(), blue);
        assert_eq!(decoded.cell(1, 2).unwrap(), icon);
    }
}
