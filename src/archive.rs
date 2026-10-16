// SPDX-License-Identifier: GPL-3.0-or-later
//! `resources.db` archives through skhid-srp's `resources-db` crate: one
//! archive, or the stock archive with its `resources.db.patch_<n>` files laid
//! over it as the 0.26g game mounts them (a later archive's file wins), read
//! by path; and a whole archive written again with some files changed
//! ([`write_over`]), for builds that mount no patches.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use resources_db::overlay::Overlay;
use resources_db::write::{Stored, write_to};
use resources_db::{Archive, Entry};

/// One archive, or several laid over one another.
pub struct ResourceDb {
    layers: Overlay<BufReader<File>>,
    /// The archives, bottom first.
    names: Vec<PathBuf>,
    /// Every file seen and every empty folder, by path.
    paths: BTreeSet<String>,
    empty_folders: BTreeSet<String>,
}

fn error(cause: resources_db::Error) -> anyhow::Error {
    anyhow::anyhow!("{cause}")
}

impl ResourceDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_layers(&[path.as_ref().to_path_buf()])
    }

    /// `paths` over one another, the first at the bottom: for each path the
    /// file of the last archive that has one.
    pub fn open_layers(paths: &[PathBuf]) -> Result<Self> {
        if paths.is_empty() {
            bail!("no archive to open");
        }
        let mut archives = Vec::with_capacity(paths.len());
        let mut empty_folders = BTreeSet::new();
        for path in paths {
            let file = File::open(path)
                .with_context(|| format!("opening resource database {}", path.display()))?;
            let archive = Archive::new(BufReader::with_capacity(1 << 20, file))
                .map_err(error)
                .with_context(|| format!("reading resource database {}", path.display()))?;
            for folder in archive.empty_folders() {
                empty_folders.insert(
                    String::from_utf8(folder.to_vec())
                        .context("resource folder path is not valid UTF-8")?,
                );
            }
            archives.push(archive);
        }
        let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        let layers = Overlay::new(archives, &names).map_err(error)?;
        // A folder one archive has empty may hold another's files.
        empty_folders.retain(|folder| {
            let prefix = format!("{folder}/");
            layers.files_under(&prefix).next().is_none()
        });
        let mut paths_seen: BTreeSet<String> = layers
            .files_under("")
            .map(|(path, _, _)| path.to_owned())
            .collect();
        paths_seen.extend(empty_folders.iter().cloned());
        Ok(Self {
            layers,
            names: paths.to_vec(),
            paths: paths_seen,
            empty_folders,
        })
    }

    /// The archives, bottom first.
    pub fn archives(&self) -> &[PathBuf] {
        &self.names
    }

    /// Every file and empty folder, in path order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(String::as_str)
    }

    /// How many files are seen.
    pub fn file_count(&self) -> usize {
        self.layers.len()
    }

    /// The file seen at `path`: the index of its archive and its entry there.
    pub fn entry(&self, path: &str) -> Option<(usize, Entry)> {
        self.layers
            .entry(path)
            .map(|(index, entry)| (index, *entry))
    }

    /// A file or an empty folder at `path`.
    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    pub fn is_file(&self, path: &str) -> bool {
        self.layers.entry(path).is_some()
    }

    pub fn is_empty_folder(&self, path: &str) -> bool {
        self.empty_folders.contains(path)
    }

    pub fn empty_folders(&self) -> impl Iterator<Item = &str> {
        self.empty_folders.iter().map(String::as_str)
    }

    /// Files and empty folders whose path starts with `prefix`.
    pub fn paths_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.paths
            .range(prefix.to_owned()..)
            .map(String::as_str)
            .take_while(move |path| path.starts_with(prefix))
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        if self.empty_folders.contains(path) {
            bail!("resource is not a regular file: {path}");
        }
        self.layers
            .read(path)
            .map_err(error)
            .with_context(|| format!("reading resource {path}"))?
            .with_context(|| format!("resource does not exist: {path}"))
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
            .layers
            .files_under(prefix)
            .map(|(path, _, _)| path.to_owned())
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
        for folder in &self.empty_folders {
            fs::create_dir_all(safe_join(root, folder)?)
                .with_context(|| format!("creating resource directory {folder}"))?;
        }
        let paths: Vec<String> = self
            .layers
            .files_under("")
            .map(|(path, _, _)| path.to_owned())
            .collect();
        for path in &paths {
            self.extract_file(path, root)?;
        }
        Ok(paths.len())
    }
}

/// A file [`write_over`] puts into the archive: bytes, or a file on disk
/// read when its turn comes.
pub enum NewFile {
    Bytes(Vec<u8>),
    Disk(PathBuf),
}

impl NewFile {
    fn bytes(&self) -> Result<Vec<u8>> {
        match self {
            NewFile::Bytes(bytes) => Ok(bytes.clone()),
            NewFile::Disk(path) => {
                fs::read(path).with_context(|| format!("reading {}", path.display()))
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct WriteStats {
    pub unchanged_files: usize,
    pub replaced_files: usize,
    pub added_files: usize,
    pub size: u64,
}

/// A whole archive: every file of `base` (as its layers lay them) and its
/// empty folders, with `files` added or put in place of the base's. For a
/// build that mounts no patches (v0.100b); 0.26g takes patches instead
/// (`pack patch`). Files that share their bytes in the base (hard links)
/// share one copy of them in the output; the output has no hard-link nodes.
/// Written to `<output>.partial`, then renamed.
pub fn write_over(
    base: &mut ResourceDb,
    files: &BTreeMap<String, NewFile>,
    output: &Path,
) -> Result<WriteStats> {
    let mut stats = WriteStats::default();
    let mut table = BTreeMap::new();
    for (path, _, entry) in base.layers.files_under("") {
        if files.contains_key(path) {
            continue;
        }
        let (layer, _) = base.layers.entry(path).expect("a file seen");
        table.insert(
            path.to_owned(),
            Stored {
                size: entry.size,
                hash: entry.hash,
                share: Some(share_key(layer, entry.offset)),
            },
        );
        stats.unchanged_files += 1;
    }
    for (path, file) in files {
        validate_virtual_path(path)?;
        if base.is_file(path) {
            stats.replaced_files += 1;
        } else {
            stats.added_files += 1;
        }
        let bytes = file.bytes()?;
        table.insert(
            path.clone(),
            Stored {
                size: u32::try_from(bytes.len()).with_context(|| format!("{path}: over 4 GB"))?,
                hash: resources_db::crc32(&bytes),
                share: None,
            },
        );
    }
    let empty: BTreeSet<String> = base
        .empty_folders
        .iter()
        .filter(|folder| {
            let prefix = format!("{folder}/");
            !files.keys().any(|path| path.starts_with(&prefix))
        })
        .cloned()
        .collect();

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut partial = output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let mut out = BufWriter::with_capacity(1 << 20, File::create(&partial)?);
    let mut failure = None;
    let written = write_to(&mut out, &table, &empty, |path| {
        let bytes = match files.get(path) {
            Some(file) => file.bytes(),
            None => base.read(path),
        };
        bytes.map_err(|cause| {
            let message = format!("{cause:#}");
            failure = Some(cause);
            resources_db::Error::Content(message)
        })
    });
    match (written, failure) {
        (Ok(size), _) => stats.size = size,
        (Err(_), Some(cause)) => return Err(cause),
        (Err(cause), None) => return Err(error(cause)),
    }
    out.into_inner()
        .map_err(|cause| cause.into_error())?
        .sync_all()?;
    fs::rename(&partial, output)?;
    Ok(stats)
}

/// A key the bytes of a file of layer `layer` at `offset` share with every
/// other file there.
fn share_key(layer: usize, offset: u64) -> u64 {
    ((layer as u64) << 48) | offset
}

/// Pack `files` into an archive of their own (a patch), read it back, and
/// write it to `output` (never over an existing file).
pub fn write_patch(files: &BTreeMap<String, Vec<u8>>, output: &Path) -> Result<Vec<u8>> {
    let bytes = resources_db::write::write(files).map_err(error)?;
    resources_db::pack::check(&bytes, files).map_err(error)?;
    if output.exists() {
        bail!("{} exists; it is never overwritten", output.display());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut partial = output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    {
        let mut file = File::create(&partial)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(&partial, output)?;
    Ok(bytes)
}

/// Every file under `root` by its archive path (`/` between names).
pub fn folder_files(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    Ok(folder_tree(root)?.0)
}

/// [`folder_files`], and the empty folders under `root`.
fn folder_tree(root: &Path) -> Result<(BTreeMap<String, PathBuf>, BTreeSet<String>)> {
    fn walk(
        root: &Path,
        current: &Path,
        files: &mut BTreeMap<String, PathBuf>,
        empty: &mut BTreeSet<String>,
    ) -> Result<()> {
        let relative = |path: &Path| -> Result<String> {
            Ok(path
                .strip_prefix(root)?
                .to_str()
                .with_context(|| format!("{}: not a UTF-8 name", path.display()))?
                .replace('\\', "/"))
        };
        let mut any = false;
        for entry in fs::read_dir(current)
            .with_context(|| format!("reading directory {}", current.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            any = true;
            if kind.is_dir() {
                walk(root, &path, files, empty)?;
            } else if kind.is_file() {
                files.insert(relative(&path)?, path);
            } else {
                bail!("not a plain file or folder: {}", path.display());
            }
        }
        if !any && current != root {
            empty.insert(relative(current)?);
        }
        Ok(())
    }
    let (mut files, mut empty) = (BTreeMap::new(), BTreeSet::new());
    walk(root, root, &mut files, &mut empty)?;
    Ok((files, empty))
}

/// An archive of the folder `root`: its files at their paths and its empty
/// folders (fixtures and whole test databases; a patch is
/// `resources_db::pack::pack`, which refuses empty folders).
pub fn write_folder(root: &Path, output: &Path) -> Result<()> {
    let (files, empty) = folder_tree(root)?;
    let mut table = BTreeMap::new();
    for (path, file) in &files {
        let bytes = fs::read(file)?;
        table.insert(
            path.clone(),
            Stored {
                size: u32::try_from(bytes.len())?,
                hash: resources_db::crc32(&bytes),
                share: None,
            },
        );
    }
    let mut out = BufWriter::new(File::create(output)?);
    write_to(&mut out, &table, &empty, |path| {
        fs::read(&files[path]).map_err(resources_db::Error::Io)
    })
    .map_err(error)?;
    out.flush()?;
    Ok(())
}

fn validate_virtual_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path
            .split('/')
            .any(|name| name.is_empty() || name.contains('\0'))
    {
        bail!("not an archive path: {path:?}");
    }
    Ok(())
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

    fn archive(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let files: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
            .collect();
        let path = dir.join(name);
        write_patch(&files, &path).unwrap();
        path
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(safe_join(Path::new("/tmp/out"), "../bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "/bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "models/good").is_ok());
    }

    #[test]
    fn layers_read_the_last_archive_that_has_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let stock = archive(
            temp.path(),
            "resources.db",
            &[("a", b"stock"), ("b/c", b"c")],
        );
        let patch = archive(
            temp.path(),
            "resources.db.patch_1",
            &[("a", b"new"), ("b/d", b"d")],
        );
        let mut db = ResourceDb::open_layers(&[stock, patch]).unwrap();
        assert_eq!(db.read("a").unwrap(), b"new");
        assert_eq!(db.read("b/c").unwrap(), b"c");
        assert_eq!(db.paths().collect::<Vec<_>>(), ["a", "b/c", "b/d"]);
        assert_eq!(db.entry("a").unwrap().0, 1);
        assert!(db.read("b").is_err());
    }

    #[test]
    fn write_over_keeps_the_base_and_changes_what_it_is_given() {
        let temp = tempfile::tempdir().unwrap();
        let stock = archive(
            temp.path(),
            "base.db",
            &[("a", b"stock"), ("b/c", &[5; 3000])],
        );
        let mut base = ResourceDb::open(&stock).unwrap();
        let files = BTreeMap::from([
            ("a".to_owned(), NewFile::Bytes(b"changed".to_vec())),
            ("n/new".to_owned(), NewFile::Bytes(b"added".to_vec())),
        ]);
        let output = temp.path().join("out.db");
        let stats = write_over(&mut base, &files, &output).unwrap();
        assert_eq!(
            (
                stats.unchanged_files,
                stats.replaced_files,
                stats.added_files
            ),
            (1, 1, 1)
        );
        let mut written = ResourceDb::open(&output).unwrap();
        assert_eq!(written.read("a").unwrap(), b"changed");
        assert_eq!(written.read("b/c").unwrap(), vec![5; 3000]);
        assert_eq!(written.read("n/new").unwrap(), b"added");
        assert_eq!(stats.size, fs::metadata(&output).unwrap().len());
    }
}
