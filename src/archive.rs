// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use resources_db::{Archive, FileEntry, NodeKind};
use sha2::{Digest, Sha256};

/// A resource database: one `resources.db`, or the ordered volumes of a
/// split build (0.69d ships `resources.00.db` .. `resources.75.db`).
///
/// A split set is named by a `.split` text file listing its volumes in mount
/// order, one path per line (relative paths resolve against the file). Later
/// volumes override earlier ones, and an erased node in a later volume removes
/// the path. A listed volume may be only its FAT head (header and node
/// buffer): it still indexes and fingerprints, and reading a payload from it
/// fails with a request to extract the full volume.
pub struct ResourceDb {
    archives: Vec<Archive>,
    volumes: Vec<PathBuf>,
    entries: BTreeMap<String, FileEntry>,
    owner: BTreeMap<String, usize>,
}

pub const SPLIT_EXTENSION: &str = "split";

/// Volume paths of a `.split` file, in mount order.
pub fn split_volumes(path: &Path) -> Result<Vec<PathBuf>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading split database list {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let volumes: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| base.join(line))
        .collect();
    ensure!(!volumes.is_empty(), "split database list names no volumes");
    Ok(volumes)
}

/// Fingerprint of a split set: SHA-256 over every volume's FAT head (header
/// and node buffer) in order. The heads pin each payload's position, size and
/// stored hash, and do not depend on where the volumes are kept.
pub fn split_fingerprint(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    for volume in split_volumes(path)? {
        let bytes = fat_head(&volume)?;
        hash.update(Sha256::digest(&bytes));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn fat_head(volume: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut file = fs::File::open(volume)
        .with_context(|| format!("opening database volume {}", volume.display()))?;
    let mut header = [0_u8; 24];
    file.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    let mut bytes = header.to_vec();
    bytes.resize(24 + size, 0);
    file.read_exact(&mut bytes[24..])
        .with_context(|| format!("reading FAT of {}", volume.display()))?;
    Ok(bytes)
}

impl ResourceDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.extension().is_some_and(|e| e == SPLIT_EXTENSION) {
            return Self::open_split(&split_volumes(path)?);
        }
        Self::open_split(&[path.to_path_buf()])
    }

    pub fn open_split(volumes: &[PathBuf]) -> Result<Self> {
        let mut archives = Vec::new();
        let mut entries = BTreeMap::new();
        let mut owner = BTreeMap::new();
        for (index, path) in volumes.iter().enumerate() {
            let archive = Archive::open(path)
                .with_context(|| format!("opening resource database {}", path.display()))?;
            for entry in archive.list() {
                let key = String::from_utf8(entry.path.clone())
                    .with_context(|| "resource path is not valid UTF-8")?;
                if matches!(entry.kind, NodeKind::Erased) && volumes.len() > 1 {
                    // A patch volume removes the path from earlier volumes.
                    entries.remove(&key);
                    owner.remove(&key);
                    continue;
                }
                // The engine resolves the last occurrence of a duplicate path.
                entries.insert(key.clone(), entry);
                owner.insert(key, index);
            }
            archives.push(archive);
        }
        Ok(Self {
            archives,
            volumes: volumes.to_vec(),
            entries,
            owner,
        })
    }

    /// Index of the volume that supplies `path` (always 0 for one database).
    pub fn volume_of(&self, path: &str) -> Option<usize> {
        self.owner.get(path).copied()
    }

    pub fn volumes(&self) -> &[PathBuf] {
        &self.volumes
    }
    pub fn entries(&self) -> &BTreeMap<String, FileEntry> {
        &self.entries
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    pub fn is_file(&self, path: &str) -> bool {
        self.entries
            .get(path)
            .is_some_and(|entry| matches!(entry.kind, NodeKind::File { .. }))
    }

    pub fn paths_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .range(prefix.to_owned()..)
            .map(|(path, _)| path.as_str())
            .take_while(move |path| path.starts_with(prefix))
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(path)
            .with_context(|| format!("resource does not exist: {path}"))?
            .clone();
        if !matches!(entry.kind, NodeKind::File { .. }) {
            bail!("resource is not a regular file: {path}");
        }
        let volume = self.owner[path];
        let end = entry.pos_in_db + u64::from(entry.size_in_db);
        if entry.inline_buffer_offset.is_none() && self.volumes.len() > 1 {
            let length = fs::metadata(&self.volumes[volume])?.len();
            ensure!(
                end <= length,
                "{path} lives in {}, which is only a FAT head here; extract that volume",
                self.volumes[volume].display()
            );
        }
        self.archives[volume]
            .read_file(&entry)
            .with_context(|| format!("reading resource {path}"))
    }

    pub fn extract_file(&mut self, path: &str, root: &Path) -> Result<PathBuf> {
        let output = safe_join(root, path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, self.read(path)?)
            .with_context(|| format!("writing {}", output.display()))?;
        Ok(output)
    }

    pub fn extract_prefix(&mut self, prefix: &str, root: &Path) -> Result<Vec<PathBuf>> {
        let paths: Vec<String> = self
            .paths_with_prefix(prefix)
            .filter(|path| {
                self.entries
                    .get(*path)
                    .is_some_and(|entry| matches!(entry.kind, NodeKind::File { .. }))
            })
            .map(ToOwned::to_owned)
            .collect();
        if paths.is_empty() {
            bail!("resource prefix matched no files: {prefix}");
        }
        paths
            .iter()
            .map(|path| self.extract_file(path, root))
            .collect()
    }

    pub fn extract_all(&mut self, root: &Path) -> Result<usize> {
        let paths: Vec<String> = self.entries.keys().cloned().collect();
        let mut count = 0;
        for path in paths {
            let Some(entry) = self.entries.get(&path) else {
                continue;
            };
            match entry.kind {
                NodeKind::Folder => {
                    fs::create_dir_all(safe_join(root, &path)?)
                        .with_context(|| format!("creating resource directory {path}"))?;
                }
                NodeKind::File { .. } => {
                    self.extract_file(&path, root)?;
                    count += 1;
                }
                _ => {}
            }
        }
        Ok(count)
    }
}

pub fn safe_join(root: &Path, virtual_path: &str) -> Result<PathBuf> {
    let path = Path::new(virtual_path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe virtual resource path: {virtual_path}");
    }
    Ok(root.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let tree = dir.join(format!("{name}-tree"));
        for (path, bytes) in files {
            let file = tree.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        }
        let db = dir.join(format!("{name}.db"));
        let source = resources_db::pack::build_src_tree(&tree).unwrap();
        resources_db::pack::assemble_to_file(&source, &db).unwrap();
        db
    }

    #[test]
    fn later_split_volumes_override_earlier_ones_and_heads_only_index() {
        let dir = tempfile::tempdir().unwrap();
        let first = volume(dir.path(), "a", &[("models/x/settings", b"old"), ("textures/t.dds", b"tex")]);
        let second = volume(dir.path(), "b", &[("models/x/settings", b"new"), ("projects/p/client_project", b"proj")]);
        fs::write(dir.path().join("set.split"), "# mount order\na.db\nb.db\n").unwrap();
        let split = dir.path().join("set.split");
        let mut db = ResourceDb::open(&split).unwrap();
        assert_eq!(db.read("models/x/settings").unwrap(), b"new");
        assert_eq!(db.volume_of("models/x/settings"), Some(1));
        assert_eq!(db.read("textures/t.dds").unwrap(), b"tex");
        assert_eq!(db.volume_of("textures/t.dds"), Some(0));
        let fingerprint = split_fingerprint(&split).unwrap();
        // A volume cut down to its FAT head still indexes and fingerprints
        // the same, and reading a payload from it asks for the full volume.
        let bytes = fs::read(&first).unwrap();
        let head = 24 + u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        fs::write(&first, &bytes[..head]).unwrap();
        assert_eq!(split_fingerprint(&split).unwrap(), fingerprint);
        let mut db = ResourceDb::open(&split).unwrap();
        assert!(db.contains("textures/t.dds"));
        let error = db.read("textures/t.dds").unwrap_err().to_string();
        assert!(error.contains("only a FAT head"), "{error}");
        assert_eq!(db.read("models/x/settings").unwrap(), b"new");
        let _ = second;
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(safe_join(Path::new("/tmp/out"), "../bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "/bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "models/good").is_ok());
    }
}
