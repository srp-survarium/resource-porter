// SPDX-License-Identifier: GPL-3.0-or-later
//! Access to retail X-Ray 1.x source data.
//!
//! Two backends, one interface:
//!
//! * a retail installation, read straight from its packed archives;
//! * an already-unpacked `gamedata` directory.
//!
//! The archive backend is the normal one. The directory backend exists because
//! mods and partial extractions are common, and because being able to point the
//! converter at a tree you can inspect by hand is worth keeping.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use super::codec::archive::SocArchiveSet;

/// A source of Shadow of Chernobyl resources.
pub enum SocSource {
    /// A retail installation's packed archives.
    Archives {
        root: PathBuf,
        set: Box<SocArchiveSet>,
    },
    /// An unpacked `gamedata` tree.
    Directory {
        root: PathBuf,
        /// Normalised relative path -> real path.
        index: BTreeMap<String, PathBuf>,
    },
}

impl SocSource {
    /// Open a retail X-Ray installation directory.
    pub fn open_installation(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let set = SocArchiveSet::open_installation(&root)?;
        Ok(Self::Archives {
            root,
            set: Box::new(set),
        })
    }

    /// Open an unpacked `gamedata` tree.
    pub fn open_directory(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if !root.is_dir() {
            bail!("{} is not an unpacked gamedata directory", root.display());
        }
        let mut index = BTreeMap::new();
        walk(&root, &root, &mut index)?;
        if index.is_empty() {
            bail!("{} contains no files", root.display());
        }
        Ok(Self::Directory { root, index })
    }

    /// Open whichever backend `root` is: an installation if it holds packed
    /// archives, an unpacked tree otherwise.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let packed_at_root = fs::read_dir(root)
            .with_context(|| format!("listing {}", root.display()))?
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_ascii_lowercase().starts_with("gamedata.db"))
            });
        let split_packed = ["resources", "levels", "localization", "mp", "patches"]
            .into_iter()
            .map(|directory| root.join(directory))
            .filter(|directory| directory.is_dir())
            .any(|directory| {
                fs::read_dir(directory).is_ok_and(|entries| {
                    entries.filter_map(Result::ok).any(|entry| {
                        entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| name.to_ascii_lowercase().contains(".db"))
                    })
                })
            });
        if packed_at_root || split_packed {
            Self::open_installation(root)
        } else {
            Self::open_directory(root)
        }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::Archives { root, .. } | Self::Directory { root, .. } => root,
        }
    }

    pub fn contains(&self, relative: &str) -> bool {
        match self {
            Self::Archives { set, .. } => set.contains(relative),
            Self::Directory { index, .. } => index.contains_key(&normalize(relative)),
        }
    }

    /// Read a `gamedata`-relative file.
    pub fn read(&mut self, relative: &str) -> Result<Vec<u8>> {
        match self {
            Self::Archives { set, .. } => set.read(relative),
            Self::Directory { root, index } => {
                let path = index
                    .get(&normalize(relative))
                    .with_context(|| format!("{relative} is missing from {}", root.display()))?;
                fs::read(path).with_context(|| format!("reading {}", path.display()))
            }
        }
    }

    /// Read one of a compiled level's containers.
    pub fn read_level_file(&mut self, level: &str, file: &str) -> Result<Vec<u8>> {
        self.read(&format!("levels/{level}/{file}"))
            .with_context(|| format!("reading {file} of level {level}"))
    }

    /// Read the physical-material library.
    ///
    /// Retail keeps it at the `gamedata` root; the SDK and some mods keep it
    /// under `config/`.
    pub fn read_game_materials(&mut self) -> Result<Vec<u8>> {
        for candidate in ["gamemtl.xr", "config/gamemtl.xr"] {
            if self.contains(candidate) {
                return self.read(candidate);
            }
        }
        bail!(
            "neither gamemtl.xr nor config/gamemtl.xr is present in {}",
            self.root().display()
        )
    }

    /// Read the shader library a level's shader table indexes into.
    pub fn read_shaders(&mut self) -> Result<Vec<u8>> {
        for candidate in ["shaders.xr", "config/shaders.xr"] {
            if self.contains(candidate) {
                return self.read(candidate);
            }
        }
        bail!("shaders.xr is not present in {}", self.root().display())
    }

    /// Read a texture named the way the level shader table names it.
    pub fn read_texture(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = format!("textures/{}.dds", normalize(name));
        if !self.contains(&path) {
            return Ok(None);
        }
        self.read(&path).map(Some)
    }

    /// Read a texture using X-Ray's global texture root, then the current
    /// level's local root.
    ///
    /// Clear Sky terrain sheets are referenced as
    /// `terrain\\terrain_<level>` but stored below
    /// `levels/<level>/terrain/`; Shadow of Chernobyl keeps the textures used
    /// by its converted maps in the global root. Keeping both lookups here
    /// preserves the source reference as the emitted target name.
    pub fn read_level_texture(&mut self, level: &str, name: &str) -> Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.read_texture(name)? {
            return Ok(Some(bytes));
        }
        let path = format!("levels/{}/{}.dds", normalize(level), normalize(name));
        if !self.contains(&path) {
            return Ok(None);
        }
        self.read(&path).map(Some)
    }

    /// Every level directory the source contains.
    pub fn levels(&self) -> Vec<String> {
        let names: Vec<String> = match self {
            Self::Archives { set, .. } => set.names().map(ToOwned::to_owned).collect(),
            Self::Directory { index, .. } => index.keys().cloned().collect(),
        };
        let mut levels: Vec<String> = names
            .iter()
            .filter_map(|name| name.strip_prefix("levels/"))
            .filter_map(|name| name.split('/').next())
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        levels.sort();
        levels.dedup();
        levels
    }
}

fn walk(root: &Path, directory: &Path, index: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("reading directory {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, index)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("walked paths stay below the root")
            .to_str()
            .with_context(|| format!("{} is not a UTF-8 path", path.display()))?;
        index.insert(normalize(relative), path);
    }
    Ok(())
}

/// X-Ray writes resource names with backslashes and treats them case
/// insensitively.
pub fn normalize(name: &str) -> String {
    name.replace('\\', "/").to_ascii_lowercase()
}

/// The target resource path for a source texture, keeping the source directory
/// layout below the port namespace.
pub fn target_texture_path(namespace: &str, source_name: &str) -> Result<String> {
    let normalized = normalize(source_name);
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("..")
        || normalized.ends_with('/')
    {
        bail!("unsafe source texture name: {source_name:?}");
    }
    Ok(format!("textures/{namespace}/{normalized}.dds"))
}

/// Enough of a DDS header to generate and validate target `.options`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub four_cc: [u8; 4],
}

impl DdsInfo {
    /// Whether the payload is block-compressed rather than raw pixels.
    pub fn is_compressed(&self) -> bool {
        self.four_cc != [0; 4]
    }

    /// The value the target's `.options` uses for this payload.
    ///
    /// `NORMAL_MAP` is a format in its own right in v0.100b, not a DXT5 with a
    /// convention attached, so a converted normal map has to declare it.
    pub fn target_format(&self, is_normal: bool) -> Result<&'static str> {
        if is_normal {
            return Ok("NORMAL_MAP");
        }
        Ok(match &self.four_cc {
            b"DXT1" => "DXT1",
            b"DXT3" | b"DXT5" => "DXT5",
            // X-Ray keeps a few detail textures as raw 32-bit pixels, and
            // v0.100b has a format for exactly that.
            [0, 0, 0, 0] => "UNCOMPRESSED",
            other => bail!(
                "unsupported source DDS format {:?}",
                String::from_utf8_lossy(other)
            ),
        })
    }

    pub fn format(&self) -> Result<&'static str> {
        self.target_format(false)
    }

    /// v0.100b stops a block-compressed chain at the last complete 4x4 block.
    /// A source chain that continues to 2x2 and 1x1 is the documented cause of
    /// the engine substituting its visible `missing texture` fallback.
    pub fn target_mip_levels(&self) -> u32 {
        // The 4x4 floor is a block-compression constraint; raw pixels have none.
        if !self.is_compressed() {
            return self.mip_levels;
        }
        let mut width = self.width;
        let mut height = self.height;
        let mut levels = 1;
        while width > 4 && height > 4 {
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            levels += 1;
        }
        levels
    }

    pub fn has_trimmed_mip_chain(&self) -> bool {
        self.mip_levels <= self.target_mip_levels()
    }
}

/// Bytes per 4x4 block for a block-compressed format.
fn block_bytes(four_cc: &[u8; 4]) -> Result<usize> {
    Ok(match four_cc {
        b"DXT1" => 8,
        b"DXT3" | b"DXT5" => 16,
        // Raw 32-bit pixels: one "block" is one pixel.
        [0, 0, 0, 0] => 4,
        other => bail!(
            "unsupported block-compressed format {:?}",
            String::from_utf8_lossy(other)
        ),
    })
}

/// Payload size of the first `levels` mips.
fn chain_bytes(info: DdsInfo, levels: u32) -> Result<usize> {
    let block = block_bytes(&info.four_cc)?;
    let compressed = info.is_compressed();
    let (mut width, mut height) = (info.width, info.height);
    let mut total = 0_usize;
    for _ in 0..levels {
        let blocks = if compressed {
            (width as usize).div_ceil(4) * (height as usize).div_ceil(4)
        } else {
            width as usize * height as usize
        };
        total = total
            .checked_add(blocks * block)
            .context("DDS chain size overflow")?;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    Ok(total)
}

/// Rewrite a source DDS into the exact container v0.100b expects.
///
/// Three differences from a generic DDS, each of which alone makes the target's
/// texture cooker substitute its visible `missing texture` fallback:
///
/// * the mip chain stops at the last complete 4x4 block, not at 1x1;
/// * `dwPitchOrLinearSize` holds every byte after the header -- the retained
///   payload plus the colour-space byte below -- with `DDSD_PITCH` and
///   `DDSD_LINEARSIZE` set, exactly as in every retail v0.100b and 0.26 DDS
///   (flags 0xA100F). One byte short, 0.26's streamer reads the packed mip
///   tail one byte early and distant mips render as colour noise;
/// * one trailing byte follows the payload, carrying the colour space — 1 for
///   sRGB colour, 0 for linear data such as a normal map.
///
/// The reserved header block is cleared: a producer tag left there is enough on
/// its own to fail the cooker.
/// How a source normal map lays out its tangent-space axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalPacking {
    /// X-Ray `_bump`: `deffer_impl` reads `tex.wzyx`, so X is alpha, Y blue
    /// and Z green. Its Y also points the other way: against the +V binormal
    /// the converted vertices carry (as native v0.100b and 0.26 vertices do),
    /// every X-Ray bump's Y runs opposite to every native normal map's, so the
    /// relief along V is lit upside down. Horizontal grooves (stair risers,
    /// plank joints) then read as ridges whose shading slides against the
    /// geometry as the camera moves.
    XRayBump,
    /// Plain RGB: X red, Y green, Z blue (F.E.A.R. `_n`).
    Rgb,
}

/// Re-encode a normal map in the layout v0.100b samples.
///
/// Build 802's shaders read a `NORMAL_MAP` texture as DXT5nm: X from alpha
/// and Y from green (`terrain_gbuffer_pass`: `tex.wy * 2 - 1`), with Z
/// rebuilt; red and blue are never read. Copied unchanged, an X-Ray bump puts
/// its Z in green and an RGB map puts nothing but its height in alpha, and
/// either way the surface is lit as if steeply tilted -- black contours at
/// grazing angles. The pixels are swizzled and recompressed with ImageMagick.
pub fn repack_normal_dds(bytes: &[u8], packing: NormalPacking, label: &str) -> Result<Vec<u8>> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let info = parse_dds(bytes, label)?;
    let swap = match packing {
        NormalPacking::XRayBump => "green<=>blue",
        NormalPacking::Rgb => "red<=>alpha",
    };
    // Beside the work, not in TMPDIR: a nix-shell TMPDIR is not always visible
    // to the `magick` process.
    let scratch = std::env::current_dir()?.join(".vostok-port-normal-repack");
    fs::create_dir_all(&scratch)?;
    let stem = scratch.join(format!(
        "vostok-port-normal-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let input = stem.with_extension("in.dds");
    let output = stem.with_extension("out.dds");
    fs::write(&input, bytes)?;
    let result = std::process::Command::new("magick")
        .arg(&input)
        .args(["-channel-fx", swap, "-define", "dds:compression=dxt5"])
        .arg("-define")
        .arg(format!("dds:mipmaps={}", info.mip_levels.saturating_sub(1)))
        .arg(&output)
        .output()
        .context("starting ImageMagick to repack a normal map")?;
    let _ = fs::remove_file(&input);
    if !result.status.success() {
        let _ = fs::remove_file(&output);
        bail!(
            "ImageMagick could not repack {label}: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    let mut repacked = fs::read(&output)?;
    let _ = fs::remove_file(&output);
    if packing == NormalPacking::XRayBump {
        negate_dxt5_green(&mut repacked).with_context(|| format!("flipping the Y of {label}"))?;
    }
    Ok(repacked)
}

/// Negate the green channel (DXT5nm Y) of every block of a DXT5 chain,
/// losslessly: 6-bit green g becomes 63 - g, whose 8-bit expansion is exactly
/// 255 minus that of g, and the colour palette interpolates linearly between
/// the endpoints. A block whose endpoint order flips is swapped and its
/// indices remapped, so it keeps its order.
pub fn negate_dxt5_green(bytes: &mut [u8]) -> Result<()> {
    let info = parse_dds(bytes, "normal map")?;
    ensure!(&info.four_cc == b"DXT5", "expected a DXT5 normal map, found {:?}", info.four_cc);
    let end = 128 + chain_bytes(info, info.mip_levels.max(1))?;
    ensure!(bytes.len() >= end, "DDS is shorter than its mip chain");
    for block in bytes[128..end].chunks_exact_mut(16) {
        let colour = &mut block[8..16];
        let flip = |c: u16| (c & !0x07e0) | ((63 - ((c >> 5) & 63)) << 5);
        let (c0, c1) = (u16::from_le_bytes([colour[0], colour[1]]), u16::from_le_bytes([colour[2], colour[3]]));
        let (mut n0, mut n1) = (flip(c0), flip(c1));
        let mut indices = u32::from_le_bytes(colour[4..8].try_into().unwrap());
        if (c0 > c1) != (n0 > n1) && n0 != n1 {
            std::mem::swap(&mut n0, &mut n1);
            let mut remapped = 0_u32;
            for texel in 0..16 {
                let index = (indices >> (2 * texel)) & 3;
                remapped |= [1, 0, 3, 2][index as usize] << (2 * texel);
            }
            indices = remapped;
        }
        colour[0..2].copy_from_slice(&n0.to_le_bytes());
        colour[2..4].copy_from_slice(&n1.to_le_bytes());
        colour[4..8].copy_from_slice(&indices.to_le_bytes());
    }
    Ok(())
}

pub fn canonicalize_dds(bytes: &[u8], info: DdsInfo, srgb: bool) -> Result<Vec<u8>> {
    // Trim, never invent: some source textures — the per-level terrain sheets
    // in particular — ship a single level, and asking for the full chain would
    // read past the end of the file.
    let levels = info.target_mip_levels().min(info.mip_levels);
    let payload = chain_bytes(info, levels)?;
    let end = 128 + payload;
    if bytes.len() < end {
        bail!(
            "DDS is {} bytes but its first {levels} mip levels need {end}",
            bytes.len()
        );
    }

    let mut output = Vec::with_capacity(end + 1);
    output.extend_from_slice(&bytes[..end]);

    const DDSD_CAPS: u32 = 0x1;
    const DDSD_HEIGHT: u32 = 0x2;
    const DDSD_WIDTH: u32 = 0x4;
    const DDSD_PITCH: u32 = 0x8;
    const DDSD_PIXELFORMAT: u32 = 0x1000;
    const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
    const DDSD_LINEARSIZE: u32 = 0x8_0000;

    let write = |bytes: &mut [u8], offset: usize, value: u32| {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    write(
        &mut output,
        8,
        DDSD_CAPS
            | DDSD_HEIGHT
            | DDSD_WIDTH
            | DDSD_PITCH
            | DDSD_PIXELFORMAT
            | DDSD_MIPMAPCOUNT
            | DDSD_LINEARSIZE,
    );
    write(&mut output, 20, u32::try_from(payload + 1)?);
    write(&mut output, 28, levels);
    output[32..76].fill(0);

    output.push(u8::from(srgb));
    Ok(output)
}

/// Parse the fixed part of a DDS header.
pub fn parse_dds(bytes: &[u8], label: &str) -> Result<DdsInfo> {
    if bytes.len() < 128 {
        bail!("{label}: DDS is shorter than one header");
    }
    if &bytes[0..4] != b"DDS " {
        bail!("{label}: missing DDS magic");
    }
    let read = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if read(4) != 124 {
        bail!("{label}: DDS header size is {}, expected 124", read(4));
    }
    let mip_levels = read(28).max(1);
    let mut four_cc = [0_u8; 4];
    four_cc.copy_from_slice(&bytes[84..88]);
    let info = DdsInfo {
        height: read(12),
        width: read(16),
        mip_levels,
        four_cc,
    };
    if info.width == 0 || info.height == 0 {
        bail!("{label}: DDS declares a zero dimension");
    }
    info.format()
        .with_context(|| format!("{label}: unsupported texture format"))?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dds(width: u32, height: u32, mips: u32, four_cc: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mips.to_le_bytes());
        bytes[84..88].copy_from_slice(four_cc);
        bytes
    }

    #[test]
    fn negating_dxt5_green_flips_y_exactly_and_keeps_the_rest() {
        let expand = |g: u16| ((g << 2) | (g >> 4)) as i32;
        let decode = |block: &[u8], texel: usize| {
            let c0 = u16::from_le_bytes([block[8], block[9]]);
            let c1 = u16::from_le_bytes([block[10], block[11]]);
            let (g0, g1) = (expand((c0 >> 5) & 63), expand((c1 >> 5) & 63));
            let (r0, r1) = (expand5((c0 >> 11) & 31), expand5((c1 >> 11) & 31));
            let index = (u32::from_le_bytes(block[12..16].try_into().unwrap()) >> (2 * texel)) & 3;
            let mix = |a: i32, b: i32| match index {
                0 => a * 3,
                1 => b * 3,
                2 => 2 * a + b,
                _ => a + 2 * b,
            };
            (mix(r0, r1), mix(g0, g1))
        };
        fn expand5(r: u16) -> i32 {
            ((r << 3) | (r >> 2)) as i32
        }
        // Two blocks: one whose endpoint order survives the flip (red decides)
        // and one whose order flips (equal red, green decides).
        let mut texture = dds(8, 4, 1, b"DXT5");
        let alpha = [200_u8, 40, 0x24, 0x49, 0x92, 0x24, 0x49, 0x92];
        for (c0, c1) in [
            ((20_u16 << 11) | (50 << 5) | 4, (3_u16 << 11) | (10 << 5) | 30),
            ((5_u16 << 11) | (50 << 5) | 4, (5_u16 << 11) | (10 << 5) | 30),
        ] {
            texture.extend(alpha);
            texture.extend(c0.to_le_bytes());
            texture.extend(c1.to_le_bytes());
            texture.extend(0xe4e4_e4e4_u32.to_le_bytes());
        }
        texture.push(0);
        let before = texture.clone();
        negate_dxt5_green(&mut texture).unwrap();
        assert_eq!(texture[..136], before[..136], "header and X untouched");
        assert_eq!(texture[144..152], before[144..152], "X untouched");
        assert_eq!(texture.last(), Some(&0));
        for block in [128, 144] {
            let (b, a) = (&before[block..block + 16], &texture[block..block + 16]);
            assert!(u16::from_le_bytes([a[8], a[9]]) > u16::from_le_bytes([a[10], a[11]]), "order kept");
            for texel in 0..16 {
                let ((red_before, green_before), (red_after, green_after)) =
                    (decode(b, texel), decode(a, texel));
                assert_eq!(red_after, red_before, "block {block} texel {texel}");
                assert_eq!(green_after, 3 * 255 - green_before, "block {block} texel {texel}");
            }
        }
        negate_dxt5_green(&mut texture).unwrap();
        assert_eq!(texture, before, "an involution");
        let mut dxt1 = dds(4, 4, 1, b"DXT1");
        dxt1.extend([0; 8]);
        assert!(negate_dxt5_green(&mut dxt1).is_err());
    }

    #[test]
    fn normalizes_x_ray_resource_names() {
        assert_eq!(normalize("PROP\\Prop_Bochka"), "prop/prop_bochka");
    }

    #[test]
    fn keeps_the_source_directory_below_the_port_namespace() {
        assert_eq!(
            target_texture_path("ported/soc/cordon", "prop\\prop_bochka").unwrap(),
            "textures/ported/soc/cordon/prop/prop_bochka.dds"
        );
    }

    #[test]
    fn rejects_an_escaping_texture_name() {
        assert!(target_texture_path("ported/soc/cordon", "..\\secret").is_err());
        assert!(target_texture_path("ported/soc/cordon", "").is_err());
    }

    #[test]
    fn reads_dimensions_and_format() {
        let info = parse_dds(&dds(1024, 512, 9, b"DXT1"), "prop.dds").unwrap();
        assert_eq!(info.width, 1024);
        assert_eq!(info.height, 512);
        assert_eq!(info.format().unwrap(), "DXT1");
    }

    #[test]
    fn computes_the_target_block_compressed_chain_length() {
        // 1024x1024 stops at 4x4: 1024,512,256,128,64,32,16,8,4 -> 9 levels.
        let square = parse_dds(&dds(1024, 1024, 11, b"DXT5"), "a.dds").unwrap();
        assert_eq!(square.target_mip_levels(), 9);
        assert!(!square.has_trimmed_mip_chain());

        // 2048x512 stops when the short side reaches 4: 8 levels.
        let wide = parse_dds(&dds(2048, 512, 8, b"DXT1"), "b.dds").unwrap();
        assert_eq!(wide.target_mip_levels(), 8);
        assert!(wide.has_trimmed_mip_chain());
    }

    #[test]
    fn rejects_an_unsupported_format() {
        let error = parse_dds(&dds(64, 64, 1, b"ATI2"), "c.dds")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported"), "{error}");
    }

    #[test]
    fn indexes_an_unpacked_tree_case_insensitively() {
        let root = tempfile::tempdir().unwrap();
        let textures = root.path().join("textures/prop");
        fs::create_dir_all(&textures).unwrap();
        fs::write(textures.join("prop_bochka.dds"), b"payload").unwrap();
        let mut source = SocSource::open(root.path()).unwrap();
        assert_eq!(
            source.read_texture("PROP\\prop_bochka").unwrap().as_deref(),
            Some(b"payload".as_slice())
        );
        assert!(source.read_texture("prop/missing").unwrap().is_none());
    }

    #[test]
    fn resolves_clear_sky_level_local_textures() {
        let root = tempfile::tempdir().unwrap();
        let terrain = root.path().join("levels/mp_training_camp/terrain");
        fs::create_dir_all(&terrain).unwrap();
        fs::write(terrain.join("terrain_mp_training_camp.dds"), b"terrain").unwrap();
        let mut source = SocSource::open(root.path()).unwrap();
        assert_eq!(
            source
                .read_level_texture("MP_TRAINING_CAMP", "terrain\\terrain_mp_training_camp")
                .unwrap()
                .as_deref(),
            Some(b"terrain".as_slice())
        );
    }

    #[test]
    fn lists_level_directories() {
        let root = tempfile::tempdir().unwrap();
        for level in ["l01_escape", "testers_mp_atp"] {
            let directory = root.path().join("levels").join(level);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("level"), b"x").unwrap();
        }
        let source = SocSource::open(root.path()).unwrap();
        assert_eq!(source.levels(), vec!["l01_escape", "testers_mp_atp"]);
    }
}
