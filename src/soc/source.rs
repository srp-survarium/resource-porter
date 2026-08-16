//! Access to an unpacked Shadow of Chernobyl `gamedata` tree.
//!
//! Retail ships `gamedata.db0`..`gamedata.dbd`, whose file tables are both
//! LZ-compressed and obfuscated with a key table belonging to the game. This
//! converter deliberately does not reimplement that: unpack once with an
//! external tool and point `--source-root` at the resulting `gamedata`
//! directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// An unpacked `gamedata` tree.
#[derive(Clone, Debug)]
pub struct SocSource {
    root: PathBuf,
    /// Lower-cased relative path -> real path, so the converter is insensitive
    /// to how the tree was unpacked on a case-sensitive filesystem.
    index: BTreeMap<String, PathBuf>,
}

impl SocSource {
    /// Index the tree below `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if !root.is_dir() {
            bail!("{} is not an unpacked gamedata directory", root.display());
        }
        let mut index = BTreeMap::new();
        walk(&root, &root, &mut index)?;
        if index.is_empty() {
            bail!("{} contains no files", root.display());
        }
        Ok(Self { root, index })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a `gamedata`-relative path, accepting either separator.
    pub fn find(&self, relative: &str) -> Option<&Path> {
        self.index.get(&normalize(relative)).map(PathBuf::as_path)
    }

    /// Read a `gamedata`-relative file.
    pub fn read(&self, relative: &str) -> Result<Vec<u8>> {
        let path = self
            .find(relative)
            .with_context(|| format!("{relative} is missing from {}", self.root.display()))?;
        fs::read(path).with_context(|| format!("reading {}", path.display()))
    }

    /// Read one of a compiled level's three containers.
    pub fn read_level_file(&self, level: &str, file: &str) -> Result<Vec<u8>> {
        self.read(&format!("levels/{level}/{file}"))
    }

    /// Read the physical-material library.
    pub fn read_game_materials(&self) -> Result<Vec<u8>> {
        self.read("config/gamemtl.xr")
    }

    /// Resolve a shader-table texture name to its source DDS.
    pub fn texture(&self, name: &str) -> Option<&Path> {
        self.find(&format!("textures/{}.dds", normalize(name)))
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
        let source = SocSource::open(root.path()).unwrap();
        assert!(source.texture("PROP\\prop_bochka").is_some());
        assert!(source.texture("prop/missing").is_none());
    }
}
