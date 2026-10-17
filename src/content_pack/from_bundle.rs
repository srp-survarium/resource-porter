// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert a legacy whole-file bundle (`overlay/` + `port-manifest.json`) into
//! a content pack measured against the base it will be composed onto.
//!
//! Legacy bundles carry complete copies of shared files, and a bundle built
//! on top of another port's database also carries that port's dictionary
//! rows, strings and icons. Given the bundle's own build base
//! (`build_base_db`), an entry is kept only when the bundle owns it: it
//! differs from the build base, or the bundle's own files reference it
//! (an item's `cfg_name`, a map's game mode, a string key, an icon cell).
//! Files the bundle relied on from its build base but that the target base
//! lacks (e.g. materials reused from an earlier install) are carried in.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value as Json};

use super::value::{self, from_config, semantic_eq, table_from_config};
use super::{
    AtlasIcon, DICTIONARY, Operation, PackManifest, TARGET_BUILD, atlas, default_id_field,
    localization_path, sha256_hex,
};
use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue};

pub struct FromBundleOptions<'a> {
    pub base_db: &'a Path,
    /// `resources.db.patch_<n>` files over `base_db`, as the game mounts
    /// them: the pack is measured against what players have.
    pub base_patches: &'a [PathBuf],
    pub build_base_db: Option<&'a Path>,
    pub bundle: &'a Path,
    pub output: &'a Path,
    pub id: &'a str,
    pub version: &'a str,
}

#[derive(Default)]
struct Builder {
    whole: Vec<Operation>,
    edits: Vec<Operation>,
    notes: Vec<String>,
}

pub fn from_bundle(options: &FromBundleOptions<'_>) -> Result<PackManifest> {
    crate::weapon::prepare_empty_output(options.output)?;
    let files_dir = options.output.join("files");
    let record: Json = serde_json::from_slice(
        &fs::read(options.bundle.join("port-manifest.json"))
            .context("reading port-manifest.json")?,
    )?;
    let overlay = options.bundle.join("overlay");
    let bundle_files = collect_bundle_files(&overlay, &record)?;
    let mut base = ResourceDb::open_layers(&super::patch::base_layers(
        options.base_db,
        options.base_patches,
    )?)?;
    let mut build_base = options.build_base_db.map(ResourceDb::open).transpose()?;

    // Strings of every config the bundle ships: ownership evidence and
    // dependency roots.
    let mut bundle_strings = BTreeSet::new();
    let mut icon_cells = BTreeSet::new();
    for path in bundle_files.keys() {
        // Shared registries carry other ports' rows; their owned parts are
        // added below from the emitted operations.
        if path == DICTIONARY || path.starts_with("localization/") {
            continue;
        }
        if let Ok(config) = BinaryConfig::parse(&fs::read(safe_join(&overlay, path)?)?) {
            collect_strings(&config.root, &mut bundle_strings);
            if let Some(ConfigValue::Indexed(cell)) = value::value_at(&config.root, "ui_desc.icon")
                && let [ConfigValue::Integer(row), ConfigValue::Integer(col)] = cell.as_slice()
            {
                icon_cells.insert((*row as u32, *col as u32));
            }
        }
    }

    let mut builder = Builder::default();
    let mut localizations = BTreeMap::<String, (Vec<u8>, Vec<u8>)>::new();
    for (path, sha) in &bundle_files {
        let bytes = fs::read(safe_join(&overlay, path)?)?;
        if sha256_hex(&bytes) != *sha {
            bail!("bundle file {path} does not match its manifest hash");
        }
        if !base.contains(path) {
            builder.add_file(&files_dir, path, &bytes)?;
            continue;
        }
        let base_bytes = base.read(path)?;
        if base_bytes == bytes {
            continue;
        }
        // As the bundle's build base has it: not the bundle's change (a base
        // the target's earlier patches since changed).
        if path != DICTIONARY
            && !path.starts_with("localization/")
            && read_optional(build_base.as_mut(), path)?.is_some_and(|previous| previous == bytes)
        {
            builder
                .notes
                .push(format!("{path}: as the build base has it; not carried"));
            continue;
        }
        if path == DICTIONARY {
            let previous = read_optional(build_base.as_mut(), path)?;
            builder.dictionary(
                &base_bytes,
                &bytes,
                previous.as_deref(),
                &bundle_files,
                &files_dir,
            )?;
        } else if let Some(language) = path
            .strip_prefix("localization/")
            .and_then(|rest| rest.strip_suffix("/localization"))
            .filter(|language| !language.contains('/'))
        {
            localizations.insert(language.to_owned(), (base_bytes, bytes));
        } else if path == atlas::DEFAULT_SWF {
            let previous = read_optional(build_base.as_mut(), path)?;
            builder.atlas(
                path,
                &base_bytes,
                &bytes,
                previous.as_deref(),
                &icon_cells,
                options.output,
            )?;
        } else {
            builder.config_or_replace(&files_dir, path, &base_bytes, &bytes)?;
        }
    }
    if !localizations.is_empty() {
        let languages = super::base_languages(&base);
        let mut previous = BTreeMap::new();
        for language in localizations.keys() {
            previous.insert(
                language.clone(),
                read_optional(build_base.as_mut(), &localization_path(language))?,
            );
        }
        builder.localization(
            &localizations,
            &previous,
            &languages,
            &bundle_strings,
            &files_dir,
        )?;
    }
    if let Some(build_base) = build_base.as_mut() {
        let mut roots = bundle_strings;
        for operation in &builder.edits {
            json_strings(&serde_json::to_value(operation)?, &mut roots);
        }
        builder.carry_dependencies(&mut base, build_base, &bundle_files, roots, &files_dir)?;
    }

    // Icons a bundle names directly (`icons/icons.json`: symbol, row, col,
    // source PNG with straight alpha) rather than in a whole movie: one
    // `atlas_icons_add` per atlas.
    let named_icons = options.bundle.join("icons").join("icons.json");
    if named_icons.is_file() {
        let list: Vec<Json> =
            serde_json::from_slice(&fs::read(&named_icons)?).context("reading icons/icons.json")?;
        let mut by_symbol: BTreeMap<String, Vec<AtlasIcon>> = BTreeMap::new();
        for icon in list {
            let symbol = icon["symbol"]
                .as_str()
                .context("icon without a symbol")?
                .to_owned();
            let source = icon["source"].as_str().context("icon without a source")?;
            let row = icon["row"].as_u64().context("icon without a row")? as u32;
            let col = icon["col"].as_u64().context("icon without a col")? as u32;
            let bytes = fs::read(options.bundle.join(source))
                .with_context(|| format!("reading {source}"))?;
            atlas::read_icon_png(&options.bundle.join(source), false)
                .with_context(|| format!("{source} is not a lobby icon"))?;
            let name = format!("icons/{}", source.rsplit('/').next().unwrap());
            write_file(&options.output.join(&name), &bytes)?;
            by_symbol.entry(symbol).or_default().push(AtlasIcon {
                row,
                col,
                source: name,
                premultiplied: false,
                replaces_sha256: None,
            });
        }
        for (symbol, icons) in by_symbol {
            builder.edits.push(Operation::AtlasIconsAdd {
                swf: None,
                symbol: (symbol != atlas::DEFAULT_SYMBOL).then_some(symbol),
                icons,
            });
        }
    }

    let mut server = Map::new();
    let equipment = options.bundle.join("equipment.json");
    if equipment.is_file() {
        server.insert(
            "items".into(),
            serde_json::from_slice(&fs::read(equipment)?)?,
        );
    }
    if let Some(maps) = record.get("maps").and_then(Json::as_array) {
        let ids: Vec<Json> = maps
            .iter()
            .map(|map| {
                map.get("id")
                    .cloned()
                    .map(|id| serde_json::json!({"id": id}))
            })
            .collect::<Option<_>>()
            .context("port-manifest maps entry without id")?;
        server.insert("maps".into(), Json::Array(ids));
    }
    let mut operations = builder.whole;
    operations.extend(builder.edits);
    let manifest = PackManifest {
        format_version: 1,
        id: options.id.to_owned(),
        version: options.version.to_owned(),
        target_build: TARGET_BUILD.into(),
        description: record
            .get("asset_id")
            .and_then(Json::as_str)
            .map(|asset| format!("converted from bundle {asset}")),
        requires: Vec::new(),
        base_db_sha256: record
            .get("target_db_sha256")
            .and_then(Json::as_str)
            .map(str::to_owned),
        operations,
        server: (!server.is_empty()).then_some(Json::Object(server)),
        notes: builder.notes,
    };
    fs::write(
        options.output.join(super::MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn read_optional(db: Option<&mut ResourceDb>, path: &str) -> Result<Option<Vec<u8>>> {
    match db {
        Some(db) if db.is_file(path) => Ok(Some(db.read(path)?)),
        _ => Ok(None),
    }
}

/// Every overlay file with its hash; the manifest list and the tree must agree.
fn collect_bundle_files(overlay: &Path, record: &Json) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![overlay.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(overlay)?
                    .to_str()
                    .context("overlay path is not UTF-8")?
                    .replace('\\', "/");
                files.insert(relative, String::new());
            }
        }
    }
    let listed = record
        .get("files")
        .and_then(Json::as_array)
        .context("port-manifest.json has no files list")?;
    for file in listed {
        let path = file
            .get("path")
            .and_then(Json::as_str)
            .context("file without path")?;
        let sha = file
            .get("sha256")
            .and_then(Json::as_str)
            .context("file without sha256")?;
        match files.get_mut(path) {
            Some(slot) => *slot = sha.to_owned(),
            None => bail!("manifest lists {path}, which the overlay lacks"),
        }
    }
    if let Some((path, _)) = files.iter().find(|(_, sha)| sha.is_empty()) {
        bail!("overlay holds {path}, which the manifest does not list");
    }
    for path in files.keys() {
        super::validate_resource_path(path)?;
    }
    Ok(files)
}

fn collect_strings(value: &ConfigValue, output: &mut BTreeSet<String>) {
    let mut strings = Vec::new();
    value.collect_strings(&mut strings);
    output.extend(strings.into_iter().map(str::to_owned));
}

fn parse_config(bytes: &[u8]) -> Option<BinaryConfig> {
    let config = BinaryConfig::parse(bytes).ok()?;
    // Treat a file as a config only when its parse is stable under
    // re-encoding; arbitrary binaries can masquerade as a tiny config.
    let reparsed = BinaryConfig::parse(&config.to_bytes().ok()?).ok()?;
    semantic_eq(&config.root, &reparsed.root).then_some(config)
}

impl Builder {
    fn add_file(&mut self, files_dir: &Path, path: &str, bytes: &[u8]) -> Result<()> {
        write_file(&safe_join(files_dir, path)?, bytes)?;
        self.whole.push(Operation::AddFile {
            path: path.to_owned(),
            source: None,
        });
        Ok(())
    }

    fn replace_file(
        &mut self,
        files_dir: &Path,
        path: &str,
        base: &[u8],
        bytes: &[u8],
    ) -> Result<()> {
        write_file(&safe_join(files_dir, path)?, bytes)?;
        self.whole.push(Operation::ReplaceFile {
            path: path.to_owned(),
            source: None,
            base_sha256: sha256_hex(base),
        });
        Ok(())
    }

    /// Field-level edits when they reproduce the bundle's config exactly,
    /// else a whole-file replacement.
    fn config_or_replace(
        &mut self,
        files_dir: &Path,
        path: &str,
        base: &[u8],
        bytes: &[u8],
    ) -> Result<()> {
        if let (Some(base_config), Some(target)) = (parse_config(base), parse_config(bytes))
            && let Some(operations) = config_operations(path, &base_config.root, &target.root)?
        {
            self.edits.extend(operations);
            return Ok(());
        }
        self.notes
            .push(format!("{path}: not a field-level change; replaced whole"));
        self.replace_file(files_dir, path, base, bytes)
    }

    fn dictionary(
        &mut self,
        base: &[u8],
        bytes: &[u8],
        previous: Option<&[u8]>,
        bundle_files: &BTreeMap<String, String>,
        files_dir: &Path,
    ) -> Result<()> {
        let (Some(base_config), Some(target)) = (parse_config(base), parse_config(bytes)) else {
            return self.config_or_replace(files_dir, DICTIONARY, base, bytes);
        };
        let previous = previous.and_then(parse_config);
        let ConfigValue::Named(tables) = &target.root else {
            return self.config_or_replace(files_dir, DICTIONARY, base, bytes);
        };
        let mut remainder_base = base_config.root.clone();
        let mut remainder_target = target.root.clone();
        for table in tables {
            let ConfigValue::Indexed(entries) = &table.value else {
                continue;
            };
            let id_field = default_id_field(&table.name);
            if entries.is_empty()
                || !entries
                    .iter()
                    .all(|entry| entry_id(entry, id_field).is_some())
            {
                continue;
            }
            let Some(ConfigValue::Indexed(base_entries)) = base_config.root.named(&table.name)
            else {
                continue;
            };
            let previous_entries =
                previous
                    .as_ref()
                    .and_then(|config| match config.root.named(&table.name) {
                        Some(ConfigValue::Indexed(entries)) => Some(entries.as_slice()),
                        _ => None,
                    });
            let mut added = Vec::new();
            for entry in entries {
                let id = entry_id(entry, id_field).unwrap();
                let base_position = base_entries
                    .iter()
                    .position(|other| entry_id(other, id_field) == Some(id));
                if base_position.is_some_and(|position| semantic_eq(&base_entries[position], entry))
                {
                    continue;
                }
                let owned = match previous_entries {
                    None => true,
                    Some(previous) => {
                        !previous.iter().any(|other| {
                            entry_id(other, id_field) == Some(id) && semantic_eq(other, entry)
                        }) || entry_config(entry)
                            .is_some_and(|path| bundle_files.contains_key(&path))
                    }
                };
                if !owned {
                    self.notes.push(format!(
                        "{}: dropped inherited {id_field} {id} (identical in the build base, not referenced by this bundle)",
                        table.name
                    ));
                    continue;
                }
                match base_position {
                    Some(position) => self.edits.push(Operation::ConfigSet {
                        path: DICTIONARY.into(),
                        values: BTreeMap::from([(
                            format!("{}.{position}", table.name),
                            from_config(entry)?,
                        )]),
                    }),
                    None => added.push(table_from_config(entry)?),
                }
            }
            for base_entry in base_entries {
                let id = entry_id(base_entry, id_field).unwrap_or_default();
                if !entries
                    .iter()
                    .any(|entry| entry_id(entry, id_field) == Some(id))
                {
                    self.notes.push(format!(
                        "{}: base {id_field} {id} removed by the bundle; removal not carried",
                        table.name
                    ));
                }
            }
            if !added.is_empty() {
                self.edits.push(Operation::DictionaryAdd {
                    table: table.name.clone(),
                    id_field: None,
                    entries: added,
                });
            }
            // The table is handled; leave it out of the generic diff.
            for root in [&mut remainder_base, &mut remainder_target] {
                if let ConfigValue::Named(values) = root {
                    values.retain(|value| value.name != table.name);
                }
            }
        }
        match value::diff(&remainder_base, &remainder_target) {
            Some(diff) => {
                if !diff.sets.is_empty() || !diff.appends.is_empty() {
                    self.notes.push(format!(
                        "{DICTIONARY}: non-id tables converted as generic edits"
                    ));
                }
                self.edits.extend(diff_operations(DICTIONARY, diff)?);
                Ok(())
            }
            None => {
                bail!("{DICTIONARY}: the bundle removes non-id dictionary content; convert by hand")
            }
        }
    }

    fn localization(
        &mut self,
        files: &BTreeMap<String, (Vec<u8>, Vec<u8>)>,
        previous: &BTreeMap<String, Option<Vec<u8>>>,
        all_languages: &[String],
        bundle_strings: &BTreeSet<String>,
        files_dir: &Path,
    ) -> Result<()> {
        // key -> language -> text, for strings the pack adds.
        let mut additions = BTreeMap::<String, BTreeMap<String, String>>::new();
        let mut dropped = BTreeMap::<String, Vec<String>>::new();
        for (language, (base, bytes)) in files {
            let path = localization_path(language);
            let (Some(base_config), Some(target)) = (parse_config(base), parse_config(bytes))
            else {
                self.config_or_replace(files_dir, &path, base, bytes)?;
                continue;
            };
            let previous = previous[language].as_deref().and_then(parse_config);
            let strings = |config: &BinaryConfig| match config.root.named("strings") {
                Some(ConfigValue::Named(values)) => Some(values.clone()),
                _ => None,
            };
            let (Some(base_strings), Some(target_strings)) =
                (strings(&base_config), strings(&target))
            else {
                self.config_or_replace(files_dir, &path, base, bytes)?;
                continue;
            };
            let previous_strings = previous.as_ref().and_then(strings);
            let mut changed = BTreeMap::new();
            for entry in &target_strings {
                let ConfigValue::String(text) = &entry.value else {
                    continue;
                };
                let base_entry = base_strings.iter().find(|other| other.name == entry.name);
                if base_entry.is_some_and(|other| other.value == entry.value) {
                    continue;
                }
                let owned = match &previous_strings {
                    None => true,
                    Some(previous) => {
                        !previous
                            .iter()
                            .any(|other| other.name == entry.name && other.value == entry.value)
                            || bundle_strings.contains(&entry.name)
                    }
                };
                if !owned {
                    dropped
                        .entry(entry.name.clone())
                        .or_default()
                        .push(language.clone());
                    continue;
                }
                if base_entry.is_some() && !entry.name.contains('.') {
                    changed.insert(
                        format!("strings.{}", entry.name),
                        Json::String(text.clone()),
                    );
                } else if base_entry.is_some() {
                    bail!(
                        "{path}: changed string {} cannot be addressed by a config path",
                        entry.name
                    );
                } else {
                    additions
                        .entry(entry.name.clone())
                        .or_default()
                        .insert(language.clone(), text.clone());
                }
            }
            if !changed.is_empty() {
                self.edits.push(Operation::ConfigSet {
                    path: path.clone(),
                    values: changed,
                });
            }
            // Anything outside `strings` goes through the generic diff.
            let mut rest_base = base_config.root.clone();
            let mut rest_target = target.root.clone();
            for root in [&mut rest_base, &mut rest_target] {
                if let ConfigValue::Named(values) = root {
                    values.retain(|value| value.name != "strings");
                }
            }
            let diff = value::diff(&rest_base, &rest_target).with_context(|| {
                format!("{path}: unsupported structural change outside strings")
            })?;
            self.edits.extend(diff_operations(&path, diff)?);
        }
        for (key, languages) in dropped {
            self.notes.push(format!(
                "localization: dropped inherited string {key} ({})",
                languages.join(", ")
            ));
        }
        // One operation for strings identical in every language, then one
        // per language for the rest.
        let mut everywhere = BTreeMap::new();
        let mut per_language = BTreeMap::<String, BTreeMap<String, String>>::new();
        for (key, texts) in additions {
            let first = texts.values().next().cloned().unwrap_or_default();
            if texts.len() == all_languages.len()
                && all_languages
                    .iter()
                    .all(|language| texts.get(language) == Some(&first))
            {
                everywhere.insert(key, first);
            } else {
                for (language, text) in texts {
                    per_language
                        .entry(language)
                        .or_default()
                        .insert(key.clone(), text);
                }
            }
        }
        if !everywhere.is_empty() {
            self.edits.push(Operation::LocalizationAdd {
                strings: everywhere,
                languages: None,
            });
        }
        for (language, strings) in per_language {
            self.edits.push(Operation::LocalizationAdd {
                strings,
                languages: Some(vec![language]),
            });
        }
        Ok(())
    }

    fn atlas(
        &mut self,
        path: &str,
        base: &[u8],
        bytes: &[u8],
        previous: Option<&[u8]>,
        referenced: &BTreeSet<(u32, u32)>,
        output: &Path,
    ) -> Result<()> {
        let files_dir = output.join("files");
        let (Ok(base_swf), Ok(target_swf)) = (atlas::Swf::parse(base), atlas::Swf::parse(bytes))
        else {
            self.notes
                .push(format!("{path}: not a parseable SWF; replaced whole"));
            return self.replace_file(&files_dir, path, base, bytes);
        };
        // The one icon bitmap the bundle draws into: equipment by default,
        // or weapons, ammo, ... (the lobby keeps one atlas per item kind).
        let changed: Vec<&str> = atlas::ICON_SYMBOLS
            .iter()
            .copied()
            .filter(
                |symbol| match (base_swf.bitmap(symbol), target_swf.bitmap(symbol)) {
                    (Ok(a), Ok(b)) => a != b,
                    _ => false,
                },
            )
            .collect();
        let symbol = match changed.as_slice() {
            [] => atlas::DEFAULT_SYMBOL,
            [one] => one,
            _ => {
                self.notes.push(format!(
                    "{path}: bundle draws into several icon atlases {changed:?}; replaced whole"
                ));
                return self.replace_file(&files_dir, path, base, bytes);
            }
        };
        let previous_swf = previous.and_then(|bytes| atlas::Swf::parse(bytes).ok());
        let same =
            |a: &atlas::Swf, b: &atlas::Swf| a.same_except_bitmap(b, symbol).unwrap_or(false);
        // A bundle built before the target base's own changes to the movie
        // (an earlier patch's) differs from it beyond the atlas, but not from
        // its build base: its icons are measured against that.
        if !same(&target_swf, &base_swf)
            && !previous_swf
                .as_ref()
                .is_some_and(|previous| same(&target_swf, previous))
        {
            self.notes.push(format!(
                "{path}: changes beyond the icon atlas; replaced whole"
            ));
            return self.replace_file(&files_dir, path, base, bytes);
        }
        let base_bitmap = base_swf.bitmap(symbol)?;
        let target_bitmap = target_swf.bitmap(symbol)?;
        let previous_bitmap = previous_swf.and_then(|swf| swf.bitmap(symbol).ok());
        let mut icons = Vec::new();
        for (row, col) in target_bitmap.occupied_cells()? {
            let pixels = target_bitmap.cell(row, col)?;
            let base_pixels = base_bitmap.cell(row, col)?;
            if pixels == base_pixels {
                continue;
            }
            let owned = match &previous_bitmap {
                None => true,
                Some(previous) => {
                    previous.cell(row, col)? != pixels || referenced.contains(&(row, col))
                }
            };
            let occupied = base_pixels.iter().any(|byte| *byte != 0);
            // A cell an earlier patch drew, which this bundle's own item
            // shows, is redrawn with that icon's hash; any other is the base's.
            let stock = previous_bitmap
                .as_ref()
                .map(|previous| previous.cell(row, col))
                .transpose()?
                .is_none_or(|previous| previous == base_pixels);
            if occupied && (stock || !referenced.contains(&(row, col))) {
                self.notes.push(format!(
                    "{path}: bundle redraws stock atlas cell [{row}, {col}]; replaced whole"
                ));
                return self.replace_file(&files_dir, path, base, bytes);
            }
            if !owned {
                self.notes.push(format!(
                    "{path}: dropped inherited atlas cell [{row}, {col}]"
                ));
                continue;
            }
            let source = format!("icons/r{row:02}c{col:02}.png");
            let destination = output.join(&source);
            fs::create_dir_all(destination.parent().unwrap())?;
            atlas::write_icon_png(&destination, &pixels)?;
            icons.push(AtlasIcon {
                row,
                col,
                source,
                premultiplied: true,
                replaces_sha256: occupied.then(|| sha256_hex(&base_pixels)),
            });
        }
        for (row, col) in base_bitmap.occupied_cells()? {
            if target_bitmap.cell(row, col)?.iter().all(|byte| *byte == 0) {
                self.notes.push(format!(
                    "{path}: bundle clears stock atlas cell [{row}, {col}]; not carried"
                ));
            }
        }
        if !icons.is_empty() {
            self.edits.push(Operation::AtlasIconsAdd {
                swf: None,
                symbol: (symbol != atlas::DEFAULT_SYMBOL).then(|| symbol.to_owned()),
                icons,
            });
        }
        Ok(())
    }

    /// Carry files the bundle's configs reference that its build base had
    /// but the target base lacks.
    fn carry_dependencies(
        &mut self,
        base: &mut ResourceDb,
        build_base: &mut ResourceDb,
        bundle_files: &BTreeMap<String, String>,
        mut strings: BTreeSet<String>,
        files_dir: &Path,
    ) -> Result<()> {
        const ROOTS: [&str; 7] = [
            "",
            "models/",
            "material_instances/",
            "textures/",
            "particles/",
            "sounds/",
            "projects/",
        ];
        let mut visited = BTreeSet::new();
        let mut carried = BTreeSet::<String>::new();
        while let Some(reference) = strings.pop_first() {
            if !visited.insert(reference.clone()) {
                continue;
            }
            let reference = reference.trim_start_matches("resources/");
            if !reference.contains('/') || reference.contains("..") {
                continue;
            }
            for root in ROOTS {
                let exact = format!("{root}{reference}");
                let dotted = format!("{exact}.");
                let candidates: Vec<String> = build_base
                    .paths_with_prefix(&exact)
                    .filter(|path| *path == exact || path.starts_with(&dotted))
                    .filter(|path| build_base.is_file(path))
                    .map(str::to_owned)
                    .collect();
                for path in candidates {
                    if base.contains(&path)
                        || bundle_files.contains_key(&path)
                        || carried.contains(&path)
                    {
                        continue;
                    }
                    let bytes = build_base.read(&path)?;
                    if let Some(config) = parse_config(&bytes) {
                        let mut more = BTreeSet::new();
                        collect_strings(&config.root, &mut more);
                        strings.extend(more.into_iter().filter(|value| !visited.contains(value)));
                    }
                    self.add_file(files_dir, &path, &bytes)?;
                    carried.insert(path);
                }
            }
        }
        if !carried.is_empty() {
            self.notes.push(format!(
                "carried {} referenced file(s) from the build base that the target base lacks: {}",
                carried.len(),
                carried.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(())
    }
}

fn write_file(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn entry_id(entry: &ConfigValue, id_field: &str) -> Option<i32> {
    match entry.named(id_field) {
        Some(ConfigValue::Integer(id)) => Some(*id),
        _ => None,
    }
}

/// The config file a dictionary entry registers.
fn entry_config(entry: &ConfigValue) -> Option<String> {
    match (
        entry.named("cfg_name"),
        entry.named("cfg_mode_name"),
        entry.named("cfg"),
    ) {
        (Some(ConfigValue::String(path)), _, _) => Some(path.clone()),
        (_, Some(ConfigValue::String(mode)), _) => Some(format!("gameplay/game_modes/{mode}")),
        (_, _, Some(ConfigValue::String(path))) => Some(path.clone()),
        _ => None,
    }
}

/// Field-level operations reproducing `target` from `base`, verified by
/// applying them; `None` when the change is structural.
fn config_operations(
    path: &str,
    base: &ConfigValue,
    target: &ConfigValue,
) -> Result<Option<Vec<Operation>>> {
    let Some(diff) = value::diff(base, target) else {
        return Ok(None);
    };
    let operations = diff_operations(path, diff)?;
    let mut applied = base.clone();
    for operation in &operations {
        match operation {
            Operation::ConfigSet { values, .. } => {
                for (key, json) in values {
                    let converted = value::to_config(json, value::value_at(&applied, key))?;
                    applied.set_path(key, converted)?;
                }
            }
            Operation::ConfigAppend {
                array_path, values, ..
            } => {
                let Some(ConfigValue::Indexed(array)) =
                    value::value_at_mut(&mut applied, array_path)
                else {
                    return Ok(None);
                };
                for json in values {
                    let converted = value::to_config(json, array.first())?;
                    array.push(converted);
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(semantic_eq(&applied, target).then_some(operations))
}

fn diff_operations(path: &str, diff: value::ConfigDiff) -> Result<Vec<Operation>> {
    let mut operations = Vec::new();
    if !diff.sets.is_empty() {
        let mut values = BTreeMap::new();
        for (key, value) in &diff.sets {
            values.insert(key.clone(), from_config(value)?);
        }
        operations.push(Operation::ConfigSet {
            path: path.to_owned(),
            values,
        });
    }
    for (array_path, values) in diff.appends {
        operations.push(Operation::ConfigAppend {
            path: path.to_owned(),
            array_path,
            values: values.iter().map(from_config).collect::<Result<_>>()?,
        });
    }
    Ok(operations)
}

fn json_strings(value: &Json, output: &mut BTreeSet<String>) {
    match value {
        Json::String(text) => {
            output.insert(text.clone());
        }
        Json::Array(values) => values.iter().for_each(|value| json_strings(value, output)),
        Json::Object(object) => object
            .values()
            .for_each(|value| json_strings(value, output)),
        _ => {}
    }
}
