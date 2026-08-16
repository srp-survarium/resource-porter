//! Access to Shadow of Chernobyl source data.
//!
//! Two backends, one interface:
//!
//! * a retail installation, read straight from its `gamedata.db*` archives;
//! * an already-unpacked `gamedata` directory.
//!
//! The archive backend is the normal one. The directory backend exists because
//! mods and partial extractions are common, and because being able to point the
//! converter at a tree you can inspect by hand is worth keeping.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

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
    /// Open a retail installation directory holding `gamedata.db*`.
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
        let packed = fs::read_dir(root)
            .with_context(|| format!("listing {}", root.display()))?
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_ascii_lowercase().starts_with("gamedata.db"))
            });
        if packed {
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
    pub fn format(&self) -> Result<&'static str> {
        Ok(match &self.four_cc {
            b"DXT1" => "DXT1",
            b"DXT3" => "DXT3",
            b"DXT5" => "DXT5",
            other => bail!(
                "unsupported source DDS format {:?}",
                String::from_utf8_lossy(other)
            ),
        })
    }

    /// v0.100b stops a block-compressed chain at the last complete 4x4 block.
    /// A source chain that continues to 2x2 and 1x1 is the documented cause of
    /// the engine substituting its visible `missing texture` fallback.
    pub fn target_mip_levels(&self) -> u32 {
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
