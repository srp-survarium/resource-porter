use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use resources_db::{Archive, FileEntry, NodeKind};

pub struct ResourceDb {
    archive: Archive,
    entries: BTreeMap<String, FileEntry>,
}

impl ResourceDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let archive = Archive::open(path)
            .with_context(|| format!("opening resource database {}", path.display()))?;
        let mut entries = BTreeMap::new();
        for entry in archive.list() {
            let key = String::from_utf8(entry.path.clone())
                .with_context(|| "resource path is not valid UTF-8")?;
            // The engine resolves the last occurrence of a duplicate path.
            entries.insert(key, entry);
        }
        Ok(Self { archive, entries })
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
        self.archive
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

    #[test]
    fn rejects_path_traversal() {
        assert!(safe_join(Path::new("/tmp/out"), "../bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "/bad").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "models/good").is_ok());
    }
}
