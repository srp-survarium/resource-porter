//! Read an installed F.E.A.R. archive set the way the engine mounts it.
//!
//! An install root carries `Default.archcfg` — an ordered list of `.Arch00`
//! archives — and optionally the expansion directories `FEARXP` and `FEARXP2`,
//! each with an archcfg of its own. Patches ship as *new* archives appended to
//! the list (`FEAR_1.Arch00` .. `FEAR_8.Arch00`), so an entry later in the
//! mount order must shadow an earlier one or no patch would ever apply; the
//! expansions re-ship newer copies of the base multiplayer worlds the same
//! way. This mounts base, then XP, then XP2, later archives winning.
//!
//! Lookups are case-insensitive with backslash separators, because the
//! archives themselves disagree on case — see [`arch::normalize`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::codec::arch::{self, Arch00, ArchEntry};

/// The expansion directories mounted over the base game, in mount order.
const EXPANSION_DIRS: [&str; 2] = ["FEARXP", "FEARXP2"];
const ARCHCFG: &str = "Default.archcfg";

/// One mounted install tree.
#[derive(Debug)]
pub struct LilithSource {
    root: PathBuf,
    archives: Vec<Arch00>,
    /// Normalized path -> (archive, entry), later mounts having overwritten
    /// earlier ones.
    index: BTreeMap<String, (usize, usize)>,
    /// Every component-aligned proper suffix of every stored path.
    ///
    /// Archives nest their content under mount-prefix chains that resource
    /// references omit -- one level (`Materials\Materials\Asylum\Brick.Mat00`
    /// for the reference `Materials\Asylum\Brick.Mat00`), or several
    /// (`Materials\Materials\City\Materials\City\Buildings\BuildB.Mat00` for
    /// `Materials\City\Buildings\BuildB.Mat00`). A reference is always a full
    /// resource path, so matching it against stored-path suffixes recovers
    /// the entry regardless of how deep the packer nested it. Some references
    /// match stored paths exactly (`Tex\specular_black.dds` is stored so),
    /// which is why this is a fallback rather than the index.
    suffixes: BTreeMap<String, (usize, usize)>,
}

impl LilithSource {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let mut archive_paths = archcfg_archives(root)
            .with_context(|| format!("mounting the base game at {}", root.display()))?;
        for dir in EXPANSION_DIRS {
            let expansion = root.join(dir);
            if !expansion.is_dir() {
                continue;
            }
            archive_paths.extend(
                archcfg_archives(&expansion)
                    .with_context(|| format!("mounting the expansion at {}", expansion.display()))?,
            );
        }

        let mut archives = Vec::with_capacity(archive_paths.len());
        let mut index = BTreeMap::new();
        let mut suffixes = BTreeMap::new();
        let mut mounted = std::collections::BTreeSet::new();
        for path in archive_paths {
            // An expansion's archcfg re-lists the base archives it builds on;
            // the file is already mounted, and its entries would not change.
            let canonical = fs::canonicalize(&path)
                .with_context(|| format!("resolving {}", path.display()))?;
            if !mounted.insert(canonical) {
                continue;
            }
            let archive = Arch00::open(&path)?;
            let archive_index = archives.len();
            for (entry_index, entry) in archive.entries().iter().enumerate() {
                let normalized = arch::normalize(&entry.path);
                let mut rest = normalized.as_str();
                while let Some((_, suffix)) = rest.split_once('\\') {
                    // Single-component suffixes are bare file names, which no
                    // reference uses and many entries share.
                    if !suffix.contains('\\') {
                        break;
                    }
                    suffixes.insert(suffix.to_owned(), (archive_index, entry_index));
                    rest = suffix;
                }
                index.insert(normalized, (archive_index, entry_index));
            }
            archives.push(archive);
        }
        if archives.is_empty() {
            bail!("{}: no archives mounted", root.display());
        }
        Ok(Self {
            root: root.to_path_buf(),
            archives,
            index,
            suffixes,
        })
    }

    /// The winning location for a resource reference: the stored path when an
    /// archive carries it directly, the mount-prefix-stripped view otherwise.
    fn resolve(&self, path: &str) -> Option<(usize, usize)> {
        let normalized = arch::normalize(path);
        self.index
            .get(&normalized)
            .or_else(|| self.suffixes.get(&normalized))
            .copied()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The mounted archives, in mount order.
    pub fn archives(&self) -> &[Arch00] {
        &self.archives
    }

    pub fn contains(&self, path: &str) -> bool {
        self.resolve(path).is_some()
    }

    /// The winning entry for a path, if any archive carries it.
    pub fn entry(&self, path: &str) -> Option<&ArchEntry> {
        let (archive, entry) = self.resolve(path)?;
        Some(&self.archives[archive].entries()[entry])
    }

    /// Which mounted archive a path resolves to, for provenance reporting.
    pub fn winning_archive(&self, path: &str) -> Option<&Path> {
        let (archive, _) = self.resolve(path)?;
        Some(self.archives[archive].path())
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let (archive, entry) = self
            .resolve(path)
            .with_context(|| format!("no mounted archive carries {path}"))?;
        self.archives[archive].read(entry)
    }

    /// Every distinct normalized path, with its winning entry.
    pub fn paths(&self) -> impl Iterator<Item = (&str, &ArchEntry)> {
        self.index.iter().map(|(path, (archive, entry))| {
            (path.as_str(), &self.archives[*archive].entries()[*entry])
        })
    }

    /// The multiplayer worlds this install ships, as normalized paths.
    ///
    /// Two directories, not a filename convention: the base game and both
    /// expansions put deathmatch worlds under `ReleaseMultiplayer`, and
    /// Perseus Mandate adds its own five under `MultiplayerXP2`. Singleplayer
    /// worlds live elsewhere (`Release`, `MultiplayerXP` does not exist).
    pub fn multiplayer_worlds(&self) -> Vec<String> {
        self.index
            .keys()
            .filter(|path| {
                path.ends_with(".world00p")
                    && (path.starts_with("worlds\\worlds\\releasemultiplayer\\")
                        || path.starts_with("worlds\\worlds\\multiplayerxp2\\"))
            })
            .cloned()
            .collect()
    }
}

/// The archives one archcfg mounts, resolved to on-disk paths, in order.
///
/// An expansion's config reaches back into the base install with `../`
/// entries, and listed names do not always match the filesystem case —
/// XP2's config says `FEARM_XP.Arch00` for a file shipped as
/// `FEARM_XP.arch00` — so every path component resolves against a
/// case-insensitive directory listing. A listed archive that resolves to
/// nothing is an error: a silently skipped archive would drop every resource
/// it carries.
fn archcfg_archives(dir: &Path) -> Result<Vec<PathBuf>> {
    let config_path = dir.join(ARCHCFG);
    let config = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let mut archives = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if !line.to_ascii_lowercase().ends_with(".arch00") {
            // The trailing `Game` line in XP2's config names a loose-file
            // directory, which retail installs do not use.
            continue;
        }
        let path = resolve_case_insensitive(dir, line)
            .with_context(|| format!("{} lists {line}", config_path.display()))?;
        archives.push(path);
    }
    if archives.is_empty() {
        bail!("{} lists no archives", config_path.display());
    }
    Ok(archives)
}

/// Resolve a config-relative path against the filesystem, ignoring case.
fn resolve_case_insensitive(dir: &Path, relative: &str) -> Result<PathBuf> {
    let mut current = dir.to_path_buf();
    for component in relative.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                if !current.pop() {
                    bail!("{relative} climbs above the filesystem root");
                }
            }
            name => {
                let mut on_disk = BTreeMap::new();
                for entry in fs::read_dir(&current)
                    .with_context(|| format!("listing {}", current.display()))?
                {
                    let file_name = entry?.file_name();
                    let Some(file_name) = file_name.to_str() else {
                        continue;
                    };
                    on_disk.insert(file_name.to_ascii_lowercase(), file_name.to_owned());
                }
                let real = on_disk
                    .get(&name.to_ascii_lowercase())
                    .with_context(|| format!("{name} is not in {}", current.display()))?;
                current.push(real);
            }
        }
    }
    Ok(current)
}
