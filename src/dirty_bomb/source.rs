//! Locate cooked Dirty Bomb maps without depending on a Steam library layout.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// One installed Dirty Bomb cooked-data tree.
#[derive(Clone, Debug)]
pub struct DirtyBombSource {
    install_root: PathBuf,
    cooked_pc: PathBuf,
    maps_root: PathBuf,
}

impl DirtyBombSource {
    /// Open either the game install root or its `ShooterGame/CookedPC` folder.
    pub fn open(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("resolving Dirty Bomb source {}", root.display()))?;
        let cooked_pc = if root.join("ShooterGame/CookedPC").is_dir() {
            root.join("ShooterGame/CookedPC")
        } else if root.join("Maps").is_dir() {
            root.clone()
        } else {
            bail!(
                "{} is neither a Dirty Bomb install root nor a CookedPC directory",
                root.display()
            );
        };
        let maps_root = cooked_pc.join("Maps");
        if !maps_root.is_dir() {
            bail!(
                "Dirty Bomb maps directory is missing: {}",
                maps_root.display()
            );
        }
        Ok(Self {
            install_root: root,
            cooked_pc,
            maps_root,
        })
    }

    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    pub fn cooked_pc(&self) -> &Path {
        &self.cooked_pc
    }

    /// Stable list of map directories containing at least one `.umap`.
    pub fn maps(&self) -> Result<Vec<String>> {
        let mut maps = Vec::new();
        for entry in fs::read_dir(&self.maps_root)
            .with_context(|| format!("listing {}", self.maps_root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.umaps_in(&entry.path())?.is_empty() {
                continue;
            }
            maps.push(name);
        }
        maps.sort_by_key(|name| name.to_ascii_lowercase());
        Ok(maps)
    }

    /// Resolve a map directory case-insensitively. `Overground` is accepted
    /// as shorthand for `EXE_Overground`.
    pub fn resolve_map(&self, requested: &str) -> Result<String> {
        let maps = self.maps()?;
        let mut matches: Vec<String> = maps
            .iter()
            .filter(|name| name.eq_ignore_ascii_case(requested))
            .cloned()
            .collect();
        if matches.is_empty() && !requested.to_ascii_lowercase().starts_with("exe_") {
            let shorthand = format!("EXE_{requested}");
            matches = maps
                .into_iter()
                .filter(|name| name.eq_ignore_ascii_case(&shorthand))
                .collect();
        }
        match matches.as_slice() {
            [only] => Ok(only.clone()),
            [] => bail!(
                "no cooked Dirty Bomb map is named {requested}; dirty-bomb-inventory lists maps"
            ),
            many => bail!("{requested} is ambiguous: {}", many.join(", ")),
        }
    }

    /// Every package belonging to one streamed map graph, in stable order.
    pub fn map_packages(&self, requested: &str) -> Result<Vec<PathBuf>> {
        let name = self.resolve_map(requested)?;
        let packages = self.umaps_in(&self.maps_root.join(&name))?;
        if packages.is_empty() {
            bail!("{name} has no cooked .umap packages");
        }
        Ok(packages)
    }

    /// Render a path relative to `CookedPC` for deterministic reports.
    pub fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.cooked_pc)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn umaps_in(&self, directory: &Path) -> Result<Vec<PathBuf>> {
        let mut packages = Vec::new();
        for entry in
            fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("umap"))
            {
                packages.push(path);
            }
        }
        packages.sort_by_key(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase()
        });
        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_unrelated_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let error = DirtyBombSource::open(temporary.path())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("neither a Dirty Bomb install root"),
            "{error}"
        );
    }
}
