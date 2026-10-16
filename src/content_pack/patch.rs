// SPDX-License-Identifier: GPL-3.0-or-later
//! `pack patch`: compose packs into a patch of the players' 0.26g game.
//!
//! The game mounts the stock `resources.db` and then every
//! `resources.db.patch_<n>` beside it in ascending n: a later archive's file
//! replaces an earlier one's, new files are added, and nothing is taken away.
//! A patch is therefore a folder of whole files at their archive paths: every
//! file the composition makes that is new or differs from the base (the stock
//! archive with the earlier patches over it), and nothing else. Packed by
//! skhid-srp's `resources-db` writer, it is `resources.db.patch_<n>`.
//!
//! `--replace` gives a base file the bytes players have now instead of the
//! base's (the server's own menus) before the packs apply: a lobby movie gets
//! the packs' icons drawn onto the live movie, not the stock one.

use std::io::Write;

use anyhow::ensure;
use resources_db::server::{PATCH_PREFIX, patch_number};

use super::*;

pub struct PatchOptions<'a> {
    /// The stock `resources.db`.
    pub base_db: &'a Path,
    /// Earlier patches over it (`resources.db.patch_<n>`, mounted in
    /// ascending n whatever their order here).
    pub base_patches: &'a [PathBuf],
    pub packs: &'a [PathBuf],
    /// `(archive path, file)`: the bytes players have now for a base file.
    pub replace: &'a [(String, PathBuf)],
    /// The patch folder; its record is written beside it, `<out-dir>.json`.
    pub out_dir: &'a Path,
    /// The packed patch, `resources.db.patch_<n>`, n above every base patch.
    pub archive: Option<&'a Path>,
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct PatchFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// `add` (no base file there) or `override`.
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReplacementRecord {
    pub path: String,
    pub file: String,
    pub sha256: String,
}

/// The patch's record, `<out-dir>.json`: what went in and what came out.
#[derive(Debug, Serialize)]
pub struct PatchRecord {
    pub format_version: u32,
    pub target_build: String,
    pub base: DbIdentity,
    pub base_patches: Vec<DbIdentity>,
    pub packs: Vec<InstalledPack>,
    pub replaced: Vec<ReplacementRecord>,
    pub files: Vec<PatchFile>,
    /// Files the composition touched that the base already has as they are.
    pub unchanged: Vec<String>,
    pub archive: Option<DbIdentity>,
    pub added_files: Vec<String>,
    pub replaced_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub notes: Vec<String>,
}

/// The record's path for a patch folder.
pub fn record_path(out_dir: &Path) -> PathBuf {
    let mut name = out_dir.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

/// The base patches in the order the game mounts them, each checked to be
/// named as the game mounts it, once per number.
pub fn mount_order(patches: &[PathBuf]) -> Result<Vec<(u32, PathBuf)>> {
    let mut numbered = BTreeMap::new();
    for patch in patches {
        let name = patch
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let number = patch_number(name).with_context(|| {
            format!(
                "{}: the game mounts only {PATCH_PREFIX}<n>, n from 1 to 255",
                patch.display()
            )
        })?;
        if numbered.insert(number, patch.clone()).is_some() {
            bail!("two base patches are number {number}");
        }
    }
    Ok(numbered.into_iter().collect())
}

/// The stock database and its patches, bottom first, as the game mounts them.
pub fn base_layers(base_db: &Path, patches: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut layers = vec![base_db.to_path_buf()];
    layers.extend(mount_order(patches)?.into_iter().map(|(_, path)| path));
    Ok(layers)
}

pub fn patch(options: &PatchOptions<'_>) -> Result<PatchRecord> {
    let ordered = mount_order(options.base_patches)?;
    if let Some(archive) = options.archive {
        let name = archive
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let number = patch_number(name).with_context(|| {
            format!(
                "--archive {}: the game mounts only {PATCH_PREFIX}<n>, n from 1 to 255",
                archive.display()
            )
        })?;
        if let Some((last, _)) = ordered.last() {
            ensure!(
                number > *last,
                "--archive is patch {number}, mounted before base patch {last}"
            );
        }
    }
    let record_file = record_path(options.out_dir);
    for path in [
        Some(options.out_dir),
        Some(record_file.as_path()),
        options.archive,
    ]
    .into_iter()
    .flatten()
    {
        if path.exists() {
            if !options.force {
                bail!("{} already exists (use --force)", path.display());
            }
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    let packs = load_packs(options.packs)?;
    ensure!(
        !packs.is_empty() || !options.replace.is_empty(),
        "nothing to patch: give a pack or a replacement"
    );
    let base_sha = sha256_file(options.base_db)?;
    let layers = base_layers(options.base_db, options.base_patches)?;
    let mut base = ResourceDb::open_layers(&layers)?;
    let mut plan = Plan::new(&packs, &base_languages(&base));
    let mut replaced = Vec::new();
    for (path, file) in options.replace {
        let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        ensure!(
            !replaced
                .iter()
                .any(|record: &ReplacementRecord| record.path == *path),
            "{path} is replaced twice"
        );
        replaced.push(ReplacementRecord {
            path: path.clone(),
            file: file.display().to_string(),
            sha256: sha256_hex(&bytes),
        });
        plan.replace(path, bytes);
    }
    let resolved = plan.resolve(&mut base, Some(&base_sha))?;

    // Only what differs from the base goes into the patch.
    let mut changed = BTreeMap::<String, Content>::new();
    let mut unchanged = Vec::new();
    let mut files = Vec::new();
    for (path, content) in &resolved.files {
        let bytes = content.bytes()?;
        let kind = if base.is_file(path) {
            if base.read(path)? == bytes {
                unchanged.push(path.clone());
                continue;
            }
            "override"
        } else {
            "add"
        };
        files.push(PatchFile {
            path: path.clone(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            kind,
        });
        changed.insert(path.clone(), Content::Bytes(bytes));
    }
    ensure!(
        !changed.is_empty(),
        "the base already has every file as the packs make it: no patch"
    );
    write_overlay(&mut base, changed.iter(), options.out_dir, false)?;

    let archive = match options.archive {
        Some(archive) => {
            let (bytes, packed) = resources_db::pack::pack(options.out_dir).map_err(|cause| {
                anyhow::anyhow!("packing {}: {cause}", options.out_dir.display())
            })?;
            ensure!(
                packed.len() == changed.len(),
                "the folder holds {} files, the patch {}",
                packed.len(),
                changed.len()
            );
            if let Some(parent) = archive.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut partial = archive.as_os_str().to_owned();
            partial.push(".partial");
            let partial = PathBuf::from(partial);
            {
                let mut file = fs::File::create(&partial)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            fs::rename(&partial, archive)?;

            // The game's view with the patch: every composed file as made,
            // every pack operation in place.
            let mut mounted = layers.clone();
            mounted.push(archive.to_path_buf());
            let mut view = ResourceDb::open_layers(&mounted)?;
            for (path, content) in &resolved.files {
                ensure!(
                    view.read(path)? == content.bytes()?,
                    "{path} does not read back as composed through the patch"
                );
            }
            let operations = verify_in(&mut view, options.packs, &[])?;
            println!("verified_operations={operations}");
            Some(DbIdentity::of(archive, sha256_hex(&bytes))?)
        }
        None => None,
    };

    let record = PatchRecord {
        format_version: 1,
        target_build: TARGET_BUILD.into(),
        base: DbIdentity::of(options.base_db, base_sha)?,
        base_patches: ordered
            .iter()
            .map(|(_, path)| DbIdentity::of(path, sha256_file(path)?))
            .collect::<Result<_>>()?,
        packs: installed_packs(&packs),
        replaced,
        files,
        unchanged,
        archive,
        added_files: kinds_of(&resolved, WholeKind::Add),
        replaced_files: kinds_of(&resolved, WholeKind::Replace),
        edited_files: resolved.edited.iter().cloned().collect(),
        notes: resolved.notes.clone(),
    };
    fs::write(&record_file, serde_json::to_vec_pretty(&record)?)?;
    Ok(record)
}
