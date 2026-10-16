// SPDX-License-Identifier: GPL-3.0-or-later
//! Content packs: composable, conflict-checked patches for a v0.26g
//! `resources.db`.
//!
//! A pack is a directory holding `pack.json` and `files/`. Its operations
//! either bring whole files (`add_file`, `replace_file`) or edit shared
//! resources at field/entry granularity (`config_set`, `config_append`,
//! `dictionary_add`, `localization_add`, `atlas_icons_add`). Composing packs
//! parses every shared base file once, applies each pack's edits in order and
//! hands one effective overlay to the non-lossy archive overlay writer, so
//! independent packs no longer clobber each other's copies of shared files.
//! See `docs/content-packs.md`.

pub mod atlas;
pub mod from_bundle;
pub mod patch;
pub mod probes;
pub mod value;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};

use crate::ResourceDb;
use crate::archive::safe_join;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use value::{semantic_eq, to_config, value_at, value_at_mut};

pub const MANIFEST_FILE: &str = "pack.json";
pub const TARGET_BUILD: &str = "0.26g";
pub const DICTIONARY: &str = "gameplay/db_static_dictionaries";
/// Localization languages of the retail v0.26g client; used for checks made
/// without a base database. With a base, its languages are discovered.
pub const DEFAULT_LANGUAGES: [&str; 6] = [
    "english",
    "german",
    "original",
    "polish",
    "russian",
    "ukrainian",
];

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    #[serde(default = "one")]
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub target_build: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Pack ids that must be composed earlier in the same run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Pin the whole base archive (map bundles were built against retail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_db_sha256: Option<String>,
    pub operations: Vec<Operation>,
    /// Server-side content description, passed through untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Json>,
    /// Free-form provenance notes (e.g. what `from-bundle` dropped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// A new resource. `source` defaults to `files/<path>`.
    AddFile {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    /// Replace an existing resource whose current content must hash to
    /// `base_sha256`.
    ReplaceFile {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        base_sha256: String,
    },
    /// Set fields of a binary config by dotted path.
    ConfigSet {
        path: String,
        values: BTreeMap<String, Json>,
    },
    /// Append values to an indexed table of a binary config.
    ConfigAppend {
        path: String,
        array_path: String,
        values: Vec<Json>,
    },
    /// Append entries to an indexed table of `gameplay/db_static_dictionaries`.
    DictionaryAdd {
        table: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id_field: Option<String>,
        entries: Vec<Map<String, Json>>,
    },
    /// Add strings to `localization/<lang>/localization`.
    LocalizationAdd {
        strings: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        languages: Option<Vec<String>>,
    },
    /// Draw icons into empty cells of the lobby equipment atlas.
    AtlasIconsAdd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        swf: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        symbol: Option<String>,
        icons: Vec<AtlasIcon>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtlasIcon {
    pub row: u32,
    pub col: u32,
    pub source: String,
    /// The PNG already holds premultiplied colour (as `from-bundle` writes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub premultiplied: bool,
    /// The cell an earlier patch drew that this icon redraws: the SHA-256 of
    /// its pixels in the base (`atlas::Bitmap::cell`, premultiplied ARGB).
    /// Without it an occupied base cell is a conflict; with it the base cell
    /// must be exactly that one (drift check, as `replace_file`'s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces_sha256: Option<String>,
}

/// Default id field of a dictionary table.
pub fn default_id_field(table: &str) -> &'static str {
    if table == "items_dict" {
        "dict_id"
    } else {
        "id"
    }
}

pub fn localization_path(language: &str) -> String {
    format!("localization/{language}/localization")
}

pub struct Pack {
    pub dir: PathBuf,
    pub manifest: PackManifest,
    pub manifest_sha256: String,
}

impl Pack {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let manifest: PackManifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            dir: dir.to_owned(),
            manifest,
            manifest_sha256: sha256_hex(&bytes),
        })
    }

    pub fn file_source(&self, path: &str, source: &Option<String>) -> Result<PathBuf> {
        match source {
            Some(source) => safe_join(&self.dir, source),
            None => safe_join(&self.dir, &format!("files/{path}")),
        }
    }

    fn id(&self) -> &str {
        &self.manifest.id
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn validate_resource_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("resource path is not a safe relative path: {path:?}");
    }
    if path != path.to_ascii_lowercase() {
        bail!("resource paths are lowercase in the archive: {path:?}");
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
        })
    {
        bail!("pack id must be non-empty [a-z0-9._-]: {id:?}");
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks that need only the pack itself.
pub fn validate_pack(pack: &Pack) -> Vec<String> {
    let mut problems = Vec::new();
    let mut report = |message: String| problems.push(format!("{}: {message}", pack.id()));
    let manifest = &pack.manifest;
    if manifest.format_version != 1 {
        report(format!(
            "unsupported format_version {}",
            manifest.format_version
        ));
    }
    if let Err(error) = validate_id(&manifest.id) {
        report(error.to_string());
    }
    if manifest.version.is_empty() {
        report("version is empty".into());
    }
    if manifest.target_build != TARGET_BUILD {
        report(format!(
            "target_build is {:?}, expected {TARGET_BUILD:?}",
            manifest.target_build
        ));
    }
    if manifest
        .base_db_sha256
        .as_deref()
        .is_some_and(|sha| !is_sha256(sha))
    {
        report("base_db_sha256 is not a lowercase SHA-256".into());
    }
    let mut whole_paths = BTreeSet::new();
    let mut dictionary_ids = BTreeSet::new();
    let mut cells = BTreeSet::new();
    for (index, operation) in manifest.operations.iter().enumerate() {
        let mut fail = |message: String| report(format!("operation {index}: {message}"));
        match operation {
            Operation::AddFile { path, source } | Operation::ReplaceFile { path, source, .. } => {
                if let Err(error) = validate_resource_path(path) {
                    fail(error.to_string());
                    continue;
                }
                if !whole_paths.insert(path.clone()) {
                    fail(format!("{path} is added or replaced twice"));
                }
                match pack.file_source(path, source) {
                    Ok(source) if source.is_file() => {}
                    Ok(source) => fail(format!("source file is missing: {}", source.display())),
                    Err(error) => fail(error.to_string()),
                }
                if let Operation::ReplaceFile { base_sha256, .. } = operation
                    && !is_sha256(base_sha256)
                {
                    fail(format!("base_sha256 of {path} is not a lowercase SHA-256"));
                }
            }
            Operation::ConfigSet { path, values } => {
                if let Err(error) = validate_resource_path(path) {
                    fail(error.to_string());
                }
                if values.is_empty() {
                    fail(format!("config_set on {path} sets nothing"));
                }
                let keys: Vec<&String> = values.keys().collect();
                for (key, json) in values {
                    if let Err(error) = value::split_path(key) {
                        fail(error.to_string());
                    }
                    if let Err(error) = to_config(json, None) {
                        fail(format!("{path} {key}: {error:#}"));
                    }
                }
                for pair in keys.windows(2) {
                    if value::paths_overlap(pair[0], pair[1]) {
                        fail(format!("{path}: {} and {} overlap", pair[0], pair[1]));
                    }
                }
            }
            Operation::ConfigAppend {
                path,
                array_path,
                values,
            } => {
                if let Err(error) = validate_resource_path(path) {
                    fail(error.to_string());
                }
                if let Err(error) = value::split_path(array_path) {
                    fail(error.to_string());
                }
                if values.is_empty() {
                    fail(format!("config_append on {path} appends nothing"));
                }
                for json in values {
                    if let Err(error) = to_config(json, None) {
                        fail(format!("{path} {array_path}: {error:#}"));
                    }
                }
            }
            Operation::DictionaryAdd {
                table,
                id_field,
                entries,
            } => {
                let id_field = id_field.as_deref().unwrap_or(default_id_field(table));
                if entries.is_empty() {
                    fail(format!("dictionary_add to {table} adds nothing"));
                }
                for entry in entries {
                    match entry_id(entry, id_field) {
                        Ok(id) => {
                            if !dictionary_ids.insert((table.clone(), id)) {
                                fail(format!("{table} id {id} is added twice"));
                            }
                        }
                        Err(error) => fail(format!("{table}: {error:#}")),
                    }
                    if let Err(error) = value::table_to_config(entry, None) {
                        fail(format!("{table}: {error:#}"));
                    }
                }
            }
            Operation::LocalizationAdd { strings, languages } => {
                if strings.is_empty() {
                    fail("localization_add adds nothing".into());
                }
                if strings.keys().any(String::is_empty) {
                    fail("localization key is empty".into());
                }
                if let Some(languages) = languages {
                    for language in languages {
                        if let Err(error) = validate_resource_path(&localization_path(language)) {
                            fail(error.to_string());
                        }
                    }
                }
            }
            Operation::AtlasIconsAdd { swf, symbol, icons } => {
                let swf = swf.as_deref().unwrap_or(atlas::DEFAULT_SWF);
                let symbol = symbol.as_deref().unwrap_or(atlas::DEFAULT_SYMBOL);
                if let Err(error) = validate_resource_path(swf) {
                    fail(error.to_string());
                }
                for icon in icons {
                    if !cells.insert((swf.to_owned(), symbol.to_owned(), icon.row, icon.col)) {
                        fail(format!(
                            "atlas cell [{}, {}] is drawn twice",
                            icon.row, icon.col
                        ));
                    }
                    if let Err(error) = atlas::cell_origin(icon.row, icon.col) {
                        fail(error.to_string());
                    }
                    match safe_join(&pack.dir, &icon.source) {
                        Ok(path) => {
                            if let Err(error) = atlas::read_icon_png(&path, icon.premultiplied) {
                                fail(format!("icon {}: {error:#}", icon.source));
                            }
                        }
                        Err(error) => fail(error.to_string()),
                    }
                }
            }
        }
    }
    problems
}

fn entry_id(entry: &Map<String, Json>, id_field: &str) -> Result<i64> {
    entry
        .get(id_field)
        .and_then(|value| match value {
            Json::Number(number) => number.as_i64(),
            Json::Object(tagged) => tagged.get("int").and_then(Json::as_i64),
            _ => None,
        })
        .with_context(|| format!("entry has no integer {id_field}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WholeKind {
    Add,
    Replace,
}

#[derive(Debug)]
struct WholeFile {
    pack: usize,
    kind: WholeKind,
    source: PathBuf,
    base_sha256: Option<String>,
}

#[derive(Debug)]
enum EditKind<'a> {
    Set {
        path: &'a str,
        value: &'a Json,
    },
    Append {
        array_path: &'a str,
        values: &'a [Json],
    },
    DictionaryAdd {
        table: &'a str,
        id_field: &'a str,
        id: i64,
        entry: &'a Map<String, Json>,
    },
    LocalizationAdd {
        key: &'a str,
        text: &'a str,
    },
}

#[derive(Debug)]
struct Edit<'a> {
    pack: usize,
    kind: EditKind<'a>,
}

#[derive(Debug)]
struct IconEdit<'a> {
    pack: usize,
    icon: &'a AtlasIcon,
}

/// The ordered, conflict-checked set of operations across all packs.
pub struct Plan<'a> {
    packs: &'a [Pack],
    whole: BTreeMap<String, WholeFile>,
    edits: BTreeMap<String, Vec<Edit<'a>>>,
    atlas: BTreeMap<(String, String), Vec<IconEdit<'a>>>,
    /// Base files given new bytes from outside the packs ([`Plan::replace`]):
    /// the packs' edits of such a file apply on top of them.
    replacements: BTreeMap<String, Vec<u8>>,
    pub problems: Vec<String>,
}

impl<'a> Plan<'a> {
    /// Build the plan and report every cross-pack conflict. `languages`
    /// resolves `localization_add` without an explicit language list.
    pub fn new(packs: &'a [Pack], languages: &[String]) -> Self {
        let mut plan = Plan {
            packs,
            whole: BTreeMap::new(),
            edits: BTreeMap::new(),
            atlas: BTreeMap::new(),
            replacements: BTreeMap::new(),
            problems: Vec::new(),
        };
        for pack in packs {
            plan.problems.extend(validate_pack(pack));
        }
        plan.check_requirements();
        let mut sets = BTreeMap::<String, Vec<(usize, &'a str, &'a Json)>>::new();
        let mut dictionary_ids = BTreeMap::<(&'a str, i64), usize>::new();
        let mut strings = BTreeMap::<(String, &'a str), (usize, &'a str)>::new();
        let mut cells = BTreeMap::<(String, String, u32, u32), usize>::new();
        for (index, pack) in packs.iter().enumerate() {
            for operation in &pack.manifest.operations {
                plan.add_operation(
                    index,
                    operation,
                    languages,
                    &mut sets,
                    &mut dictionary_ids,
                    &mut strings,
                    &mut cells,
                );
            }
        }
        plan.check_whole_versus_edits();
        plan
    }

    fn pack_id(&self, index: usize) -> &str {
        self.packs[index].id()
    }

    fn conflict(&mut self, message: String) {
        self.problems.push(format!("conflict: {message}"));
    }

    fn check_requirements(&mut self) {
        let mut seen = BTreeMap::new();
        for (index, pack) in self.packs.iter().enumerate() {
            if let Some(previous) = seen.insert(pack.id().to_owned(), index) {
                self.problems.push(format!(
                    "pack id {} is given twice (positions {previous} and {index})",
                    pack.id()
                ));
            }
            for required in &pack.manifest.requires {
                match seen.get(required) {
                    Some(position) if *position < index => {}
                    _ if self.packs.iter().any(|other| other.id() == required) => {
                        self.problems.push(format!(
                            "{} requires {required}, which must be composed before it",
                            pack.id()
                        ));
                    }
                    _ => self
                        .problems
                        .push(format!("{} requires missing pack {required}", pack.id())),
                }
            }
        }
    }

    /// The pack itself plus everything it transitively requires.
    fn trusted_by(&self, index: usize) -> BTreeSet<usize> {
        let mut output = BTreeSet::from([index]);
        let mut pending = vec![index];
        while let Some(current) = pending.pop() {
            for required in &self.packs[current].manifest.requires {
                if let Some(position) = self.packs.iter().position(|pack| pack.id() == required)
                    && output.insert(position)
                {
                    pending.push(position);
                }
            }
        }
        output
    }

    #[allow(clippy::too_many_arguments)]
    fn add_operation(
        &mut self,
        index: usize,
        operation: &'a Operation,
        languages: &[String],
        sets: &mut BTreeMap<String, Vec<(usize, &'a str, &'a Json)>>,
        dictionary_ids: &mut BTreeMap<(&'a str, i64), usize>,
        strings: &mut BTreeMap<(String, &'a str), (usize, &'a str)>,
        cells: &mut BTreeMap<(String, String, u32, u32), usize>,
    ) {
        let pack = &self.packs[index];
        match operation {
            Operation::AddFile { path, source } | Operation::ReplaceFile { path, source, .. } => {
                let (kind, base_sha256) = match operation {
                    Operation::ReplaceFile { base_sha256, .. } => {
                        (WholeKind::Replace, Some(base_sha256.clone()))
                    }
                    _ => (WholeKind::Add, None),
                };
                let Ok(source) = pack.file_source(path, source) else {
                    return;
                };
                if let Some(existing) = self.whole.get(path) {
                    if existing.pack != index {
                        let message = format!(
                            "{} and {} both add or replace {path}",
                            self.pack_id(existing.pack),
                            pack.id()
                        );
                        self.conflict(message);
                    }
                    return;
                }
                self.whole.insert(
                    path.clone(),
                    WholeFile {
                        pack: index,
                        kind,
                        source,
                        base_sha256,
                    },
                );
            }
            Operation::ConfigSet { path, values } => {
                let existing = sets.entry(path.clone()).or_default();
                let mut conflicts = Vec::new();
                for (key, value) in values {
                    for (other, other_key, other_value) in existing.iter() {
                        if *other == index || !value::paths_overlap(key, other_key) {
                            continue;
                        }
                        let same = key == other_key
                            && match (to_config(value, None), to_config(other_value, None)) {
                                (Ok(left), Ok(right)) => semantic_eq(&left, &right),
                                _ => false,
                            };
                        if !same {
                            conflicts.push(format!(
                                "{} and {} set {path} {other_key} / {key} differently",
                                pack.id(),
                                self.packs[*other].id()
                            ));
                        }
                    }
                    existing.push((index, key, value));
                    self.edits.entry(path.clone()).or_default().push(Edit {
                        pack: index,
                        kind: EditKind::Set { path: key, value },
                    });
                }
                for message in conflicts {
                    self.conflict(message);
                }
            }
            Operation::ConfigAppend {
                path,
                array_path,
                values,
            } => {
                self.edits.entry(path.clone()).or_default().push(Edit {
                    pack: index,
                    kind: EditKind::Append { array_path, values },
                });
            }
            Operation::DictionaryAdd {
                table,
                id_field,
                entries,
            } => {
                let id_field = id_field.as_deref().unwrap_or(default_id_field(table));
                for entry in entries {
                    let Ok(id) = entry_id(entry, id_field) else {
                        continue;
                    };
                    if let Some(other) = dictionary_ids.insert((table, id), index)
                        && other != index
                    {
                        let message = format!(
                            "{} and {} both add {table} id {id}",
                            self.pack_id(other),
                            pack.id()
                        );
                        self.conflict(message);
                        continue;
                    }
                    self.edits
                        .entry(DICTIONARY.to_owned())
                        .or_default()
                        .push(Edit {
                            pack: index,
                            kind: EditKind::DictionaryAdd {
                                table,
                                id_field,
                                id,
                                entry,
                            },
                        });
                }
            }
            Operation::LocalizationAdd {
                strings: additions,
                languages: listed,
            } => {
                let languages: Vec<String> = match listed {
                    Some(listed) => listed.clone(),
                    None => languages.to_vec(),
                };
                for language in &languages {
                    let file = localization_path(language);
                    for (key, text) in additions {
                        match strings.get(&(language.clone(), key.as_str())) {
                            Some((other, other_text)) if *other_text != text.as_str() => {
                                let message = format!(
                                    "{} and {} give {language} string {key} different text",
                                    self.pack_id(*other),
                                    pack.id()
                                );
                                self.conflict(message);
                                continue;
                            }
                            Some(_) => continue,
                            None => {}
                        }
                        strings.insert((language.clone(), key), (index, text));
                        self.edits.entry(file.clone()).or_default().push(Edit {
                            pack: index,
                            kind: EditKind::LocalizationAdd { key, text },
                        });
                    }
                }
            }
            Operation::AtlasIconsAdd { swf, symbol, icons } => {
                let swf = swf.as_deref().unwrap_or(atlas::DEFAULT_SWF).to_owned();
                let symbol = symbol
                    .as_deref()
                    .unwrap_or(atlas::DEFAULT_SYMBOL)
                    .to_owned();
                for icon in icons {
                    let key = (swf.clone(), symbol.clone(), icon.row, icon.col);
                    if let Some(other) = cells.insert(key, index)
                        && other != index
                    {
                        let message = format!(
                            "{} and {} both draw atlas cell [{}, {}] of {swf}",
                            self.pack_id(other),
                            pack.id(),
                            icon.row,
                            icon.col
                        );
                        self.conflict(message);
                        continue;
                    }
                    self.atlas
                        .entry((swf.clone(), symbol.clone()))
                        .or_default()
                        .push(IconEdit { pack: index, icon });
                }
            }
        }
    }

    /// A whole-file operation may only be combined with edits of the same
    /// file by its own pack or by packs that require it.
    fn check_whole_versus_edits(&mut self) {
        let mut messages = Vec::new();
        let edited = self
            .edits
            .iter()
            .flat_map(|(path, edits)| edits.iter().map(move |edit| (path.clone(), edit.pack)))
            .chain(self.atlas.iter().flat_map(|((path, _), edits)| {
                edits.iter().map(move |edit| (path.clone(), edit.pack))
            }));
        let mut seen = BTreeSet::new();
        for (path, editor) in edited {
            if !seen.insert((path.clone(), editor)) {
                continue;
            }
            if let Some(whole) = self.whole.get(&path)
                && !self.trusted_by(editor).contains(&whole.pack)
            {
                messages.push(format!(
                    "{} edits {path}, which {} {}s whole; declare `requires` to build on it",
                    self.pack_id(editor),
                    self.pack_id(whole.pack),
                    match whole.kind {
                        WholeKind::Add => "add",
                        WholeKind::Replace => "replace",
                    }
                ));
            }
        }
        for message in messages {
            self.conflict(message);
        }
    }

    /// Resolve the plan against a base archive: run every base-dependent
    /// check and compute the effective content of each touched resource.
    /// Give the base file `path` new bytes before the packs apply: what
    /// players have now in place of the stock file (a menu of the server's
    /// own). Packs may edit it (a lobby movie gets its icons drawn on these
    /// bytes, not the stock ones) but not add or replace it whole.
    pub fn replace(&mut self, path: &str, bytes: Vec<u8>) {
        self.replacements.insert(path.to_owned(), bytes);
    }

    pub fn resolve(mut self, base: &mut ResourceDb, base_sha256: Option<&str>) -> Result<Resolved> {
        let mut problems = std::mem::take(&mut self.problems);
        let mut resolved = Resolved::default();
        for path in self.replacements.keys() {
            if !base.is_file(path) {
                problems.push(format!("replacement {path} is not a file of the base"));
            }
            if let Some(whole) = self.whole.get(path) {
                problems.push(format!(
                    "{} adds or replaces {path} whole, which is also replaced from outside",
                    self.pack_id(whole.pack)
                ));
            }
        }
        for pack in self.packs {
            if let (Some(expected), Some(actual)) = (&pack.manifest.base_db_sha256, base_sha256)
                && expected != actual
            {
                problems.push(format!(
                    "{}: requires base database {expected}, got {actual}",
                    pack.id()
                ));
            }
        }
        for (path, whole) in &self.whole {
            let pack = self.packs[whole.pack].id();
            match whole.kind {
                WholeKind::Add => {
                    if base.contains(path) {
                        problems.push(format!(
                            "{pack}: add_file {path} already exists in the base"
                        ));
                    }
                }
                WholeKind::Replace => {
                    if !base.is_file(path) {
                        problems.push(format!("{pack}: replace_file {path} is not in the base"));
                    } else {
                        let actual = sha256_hex(&base.read(path)?);
                        if Some(&actual) != whole.base_sha256.as_ref() {
                            problems.push(format!(
                                "{pack}: drift: base {path} is {actual}, pack expects {}",
                                whole.base_sha256.as_deref().unwrap_or("?")
                            ));
                        }
                    }
                }
            }
            resolved
                .files
                .insert(path.clone(), Content::Source(whole.source.clone()));
            resolved.kinds.insert(path.clone(), whole.kind);
        }
        for (path, edits) in &self.edits {
            match self.apply_config_edits(base, path, edits) {
                Ok((Some(bytes), notes)) => {
                    resolved.notes.extend(notes);
                    resolved.files.insert(path.clone(), Content::Bytes(bytes));
                    resolved.edited.insert(path.clone());
                }
                Ok((None, notes)) => resolved.notes.extend(notes),
                Err(error) => problems.push(format!("{error:#}")),
            }
        }
        for ((path, symbol), edits) in &self.atlas {
            match self.apply_atlas_edits(base, path, symbol, edits) {
                Ok(Some(bytes)) => {
                    resolved.files.insert(path.clone(), Content::Bytes(bytes));
                    resolved.edited.insert(path.clone());
                }
                Ok(None) => {}
                Err(error) => problems.push(format!("{error:#}")),
            }
        }
        for (path, bytes) in &self.replacements {
            if !resolved.files.contains_key(path) {
                resolved
                    .files
                    .insert(path.clone(), Content::Bytes(bytes.clone()));
            }
        }
        if !problems.is_empty() {
            bail!(
                "{} problem(s):\n  {}",
                problems.len(),
                problems.join("\n  ")
            );
        }
        Ok(resolved)
    }

    fn starting_bytes(&self, base: &mut ResourceDb, path: &str) -> Result<Vec<u8>> {
        match self.whole.get(path) {
            Some(whole) => fs::read(&whole.source)
                .with_context(|| format!("reading {}", whole.source.display())),
            None if self.replacements.contains_key(path) => Ok(self.replacements[path].clone()),
            None if base.is_file(path) => base.read(path),
            None => bail!("edits target {path}, which is neither in the base nor added by a pack"),
        }
    }

    fn apply_config_edits(
        &self,
        base: &mut ResourceDb,
        path: &str,
        edits: &[Edit<'a>],
    ) -> Result<(Option<Vec<u8>>, Vec<String>)> {
        let original = BinaryConfig::parse(&self.starting_bytes(base, path)?)
            .with_context(|| format!("parsing {path} as a binary config"))?;
        let mut config = original.clone();
        let mut notes = Vec::new();
        let mut problems = Vec::new();
        for edit in edits {
            let pack = self.pack_id(edit.pack);
            if let Err(error) = apply_edit(&mut config.root, &edit.kind, &mut notes, pack, path) {
                problems.push(format!("{pack}: {path}: {error:#}"));
            }
        }
        if !problems.is_empty() {
            bail!("{}", problems.join("\n  "));
        }
        if semantic_eq(&original.root, &config.root) {
            return Ok((None, notes));
        }
        Ok((Some(config.to_bytes()?), notes))
    }

    fn apply_atlas_edits(
        &self,
        base: &mut ResourceDb,
        path: &str,
        symbol: &str,
        edits: &[IconEdit<'a>],
    ) -> Result<Option<Vec<u8>>> {
        let mut swf = atlas::Swf::parse(&self.starting_bytes(base, path)?)
            .with_context(|| format!("parsing {path}"))?;
        let original = swf.bitmap(symbol)?;
        let mut bitmap = original.clone();
        let mut problems = Vec::new();
        for edit in edits {
            let pack = &self.packs[edit.pack];
            let icon = edit.icon;
            let pixels =
                atlas::read_icon_png(&safe_join(&pack.dir, &icon.source)?, icon.premultiplied)?;
            let current = original.cell(icon.row, icon.col)?;
            if current == pixels {
                continue;
            }
            let occupied = current.iter().any(|byte| *byte != 0);
            match (&icon.replaces_sha256, occupied) {
                (None, false) => {}
                (None, true) => {
                    problems.push(format!(
                        "{}: atlas cell [{}, {}] of {path} is already occupied in the base",
                        pack.id(),
                        icon.row,
                        icon.col
                    ));
                    continue;
                }
                (Some(wanted), _) if *wanted == sha256_hex(&current) => {}
                (Some(wanted), _) => {
                    problems.push(format!(
                        "{}: atlas cell [{}, {}] of {path} is not the icon it replaces: the base's {} is {}, the pack replaces {wanted}",
                        pack.id(),
                        icon.row,
                        icon.col,
                        if occupied { "icon" } else { "empty cell" },
                        sha256_hex(&current)
                    ));
                    continue;
                }
            }
            if let Err(error) = bitmap.put_cell(icon.row, icon.col, &pixels) {
                problems.push(format!("{}: {path}: {error:#}", pack.id()));
            }
        }
        if !problems.is_empty() {
            bail!("{}", problems.join("\n  "));
        }
        if bitmap == original {
            return Ok(None);
        }
        swf.set_bitmap(symbol, &bitmap)?;
        Ok(Some(swf.to_bytes()?))
    }
}

/// A hint for converting a new dictionary entry: per field, the first entry
/// of the table that has it.
fn entry_hint(table: &[ConfigValue], entry: &Map<String, Json>) -> ConfigValue {
    ConfigValue::Named(
        entry
            .keys()
            .filter_map(|name| {
                table
                    .iter()
                    .find_map(|existing| existing.named(name))
                    .map(|value| NamedValue {
                        name: name.clone(),
                        value: value.clone(),
                    })
            })
            .collect(),
    )
}

fn apply_edit(
    root: &mut ConfigValue,
    edit: &EditKind<'_>,
    notes: &mut Vec<String>,
    pack: &str,
    file: &str,
) -> Result<()> {
    match edit {
        EditKind::Set { path, value } => {
            let converted = to_config(value, value_at(root, path))?;
            root.set_path(path, converted)?;
        }
        EditKind::Append { array_path, values } => {
            let Some(ConfigValue::Indexed(array)) = value_at_mut(root, array_path) else {
                bail!("{array_path} is not an indexed table");
            };
            for json in values.iter() {
                let converted = to_config(json, array.first())?;
                array.push(converted);
            }
        }
        EditKind::DictionaryAdd {
            table,
            id_field,
            id,
            entry,
        } => {
            let Some(ConfigValue::Indexed(values)) = root.named_mut(table) else {
                bail!("dictionary table {table} is missing or not indexed");
            };
            let hint = entry_hint(values, entry);
            let converted = value::table_to_config(entry, Some(&hint))?;
            let wanted = ConfigValue::Integer(i32::try_from(*id)?);
            if let Some(existing) = values
                .iter()
                .find(|value| value.named(id_field) == Some(&wanted))
            {
                if semantic_eq(existing, &converted) {
                    notes.push(format!(
                        "{pack}: {table} id {id} is already present unchanged"
                    ));
                    return Ok(());
                }
                bail!("{table} id {id} is already occupied in the base by a different entry");
            }
            values.push(converted);
        }
        EditKind::LocalizationAdd { key, text } => {
            let Some(ConfigValue::Named(strings)) = root.named_mut("strings") else {
                bail!("localization has no named strings table");
            };
            if let Some(existing) = strings.iter().find(|entry| entry.name == *key) {
                if existing.value == ConfigValue::String((*text).to_owned()) {
                    notes.push(format!("{pack}: {file} {key} is already present unchanged"));
                    return Ok(());
                }
                bail!("string {key} already exists in the base with different text");
            }
            strings.push(NamedValue {
                name: (*key).to_owned(),
                value: ConfigValue::String((*text).to_owned()),
            });
        }
    }
    Ok(())
}

pub enum Content {
    Source(PathBuf),
    Bytes(Vec<u8>),
}

impl Content {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        match self {
            Content::Source(path) => {
                fs::read(path).with_context(|| format!("reading {}", path.display()))
            }
            Content::Bytes(bytes) => Ok(bytes.clone()),
        }
    }
}

/// The effective overlay of a composition.
#[derive(Default)]
pub struct Resolved {
    pub files: BTreeMap<String, Content>,
    pub kinds: BTreeMap<String, WholeKind>,
    pub edited: BTreeSet<String>,
    pub notes: Vec<String>,
}

/// Localization languages present in a base archive.
pub fn base_languages(base: &ResourceDb) -> Vec<String> {
    base.paths_with_prefix("localization/")
        .filter_map(|path| {
            let (language, leaf) = path.strip_prefix("localization/")?.split_once('/')?;
            (leaf == "localization").then(|| language.to_owned())
        })
        .collect()
}

/// A project the packs write must not name an environment probe cubemap
/// the game's files lack (with one missing, 0.26g renders NaNs:
/// [`probes`]).
pub(crate) fn refuse_missing_probes(
    base: &ResourceDb,
    files: &BTreeMap<String, Content>,
) -> Result<()> {
    let missing = probes::missing_in_files(base, files)?;
    if !missing.is_empty() {
        bail!(
            "{} probe cubemap reference(s) do not resolve:\n  {}",
            missing.len(),
            missing
                .iter()
                .map(|m| format!(
                    "{} client_objects[{}] {} {}: {} ({})",
                    m.project,
                    m.index,
                    m.name,
                    m.field,
                    m.texture,
                    m.missing.join(", ")
                ))
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    Ok(())
}

pub fn load_packs(dirs: &[PathBuf]) -> Result<Vec<Pack>> {
    dirs.iter().map(|dir| Pack::load(dir)).collect()
}

/// `pack check`: validate packs, their mutual conflicts and, given a base,
/// every base-dependent precondition.
pub fn check(
    dirs: &[PathBuf],
    base_db: Option<&Path>,
    base_patches: &[PathBuf],
) -> Result<CheckReport> {
    let packs = load_packs(dirs)?;
    let (languages, mut base) = match base_db {
        Some(path) => {
            let base = ResourceDb::open_layers(&patch::base_layers(path, base_patches)?)?;
            (base_languages(&base), Some(base))
        }
        None => (
            DEFAULT_LANGUAGES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            None,
        ),
    };
    let plan = Plan::new(&packs, &languages);
    let report = CheckReport {
        packs: packs.len(),
        operations: packs
            .iter()
            .map(|pack| pack.manifest.operations.len())
            .sum(),
        notes: Vec::new(),
        effective_files: 0,
    };
    match base.as_mut() {
        None => {
            if !plan.problems.is_empty() {
                bail!(
                    "{} problem(s):\n  {}",
                    plan.problems.len(),
                    plan.problems.join("\n  ")
                );
            }
            Ok(report)
        }
        Some(base) => {
            let base_sha = if packs
                .iter()
                .any(|pack| pack.manifest.base_db_sha256.is_some())
            {
                Some(sha256_file(base_db.unwrap())?)
            } else {
                None
            };
            let resolved = plan.resolve(base, base_sha.as_deref())?;
            refuse_missing_probes(base, &resolved.files)?;
            Ok(CheckReport {
                notes: resolved.notes,
                effective_files: resolved.files.len(),
                ..report
            })
        }
    }
}

pub struct CheckReport {
    pub packs: usize,
    pub operations: usize,
    pub effective_files: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DbIdentity {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

impl DbIdentity {
    fn of(path: &Path, sha256: String) -> Result<Self> {
        Ok(Self {
            path: path.display().to_string(),
            size: fs::metadata(path)?.len(),
            sha256,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct InstalledPack {
    pub id: String,
    pub version: String,
    pub manifest_sha256: String,
    pub requires: Vec<String>,
    pub operations: usize,
    pub server: Option<Json>,
}

#[derive(Debug, Serialize)]
pub struct InstallRecord {
    pub format_version: u32,
    pub target_build: String,
    pub base_db: DbIdentity,
    pub output_db: DbIdentity,
    pub packs: Vec<InstalledPack>,
    pub added_files: Vec<String>,
    pub replaced_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub notes: Vec<String>,
}

pub struct ComposeOptions<'a> {
    pub base_db: &'a Path,
    pub packs: &'a [PathBuf],
    pub work_dir: &'a Path,
    pub output_db: &'a Path,
    pub server_export: Option<&'a Path>,
    pub force: bool,
}

pub fn install_record_path(output_db: &Path) -> PathBuf {
    let mut name = output_db.file_name().unwrap_or_default().to_owned();
    name.push(".packs.json");
    output_db.with_file_name(name)
}

/// Write `files` into the folder `dir` at their archive paths (a folder
/// that exists is refused unless `force`, then cleared) and validate it as an
/// overlay of `base` (`validate_effective_overlay`).
pub(crate) fn write_overlay<'a>(
    base: &mut ResourceDb,
    files: impl Iterator<Item = (&'a String, &'a Content)>,
    dir: &Path,
    force: bool,
) -> Result<()> {
    if dir.exists() {
        if !force {
            bail!("{} already exists (use --force)", dir.display());
        }
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    for (path, content) in files {
        let destination = safe_join(dir, path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        match content {
            Content::Source(source) => {
                fs::copy(source, &destination)
                    .with_context(|| format!("copying {}", source.display()))?;
            }
            Content::Bytes(bytes) => fs::write(&destination, bytes)?,
        }
    }
    crate::model_validation::validate_overlay_on(base, dir)
        .context("validating effective model surface/settings contracts")?;
    Ok(())
}

/// `pack compose`: resolve all packs into one overlay and write a whole
/// database with it ([`crate::archive::write_over`]). The players' 0.26g
/// game takes a patch instead (`pack patch`); a whole database is for local
/// runs.
pub fn compose(options: &ComposeOptions<'_>) -> Result<InstallRecord> {
    if options.output_db.exists() && !options.force {
        bail!(
            "output database already exists (use --force): {}",
            options.output_db.display()
        );
    }
    if let Some(export) = options.server_export {
        crate::weapon::prepare_empty_output(export)?;
    }
    let packs = load_packs(options.packs)?;
    if packs.is_empty() {
        bail!("at least one pack is required");
    }
    let base_sha = sha256_file(options.base_db)?;
    let mut base = ResourceDb::open(options.base_db)?;
    let plan = Plan::new(&packs, &base_languages(&base));
    let resolved = plan.resolve(&mut base, Some(&base_sha))?;

    write_overlay(
        &mut base,
        resolved.files.iter(),
        &options.work_dir.join("overlay"),
        options.force,
    )?;
    let files = resolved
        .files
        .iter()
        .map(|(path, content)| {
            Ok((
                path.clone(),
                crate::archive::NewFile::Bytes(content.bytes()?),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    crate::archive::write_over(&mut base, &files, options.output_db)?;
    let output_sha = sha256_file(options.output_db)?;

    let mut output = ResourceDb::open(options.output_db)?;
    for (path, content) in &resolved.files {
        let expected = sha256_hex(&content.bytes()?);
        let actual = sha256_hex(&output.read(path)?);
        if expected != actual {
            bail!("composed resource {path} does not match its effective content");
        }
    }
    let record = InstallRecord {
        format_version: 1,
        target_build: TARGET_BUILD.into(),
        base_db: DbIdentity::of(options.base_db, base_sha)?,
        output_db: DbIdentity::of(options.output_db, output_sha)?,
        packs: installed_packs(&packs),
        added_files: kinds_of(&resolved, WholeKind::Add),
        replaced_files: kinds_of(&resolved, WholeKind::Replace),
        edited_files: resolved.edited.iter().cloned().collect(),
        notes: resolved.notes.clone(),
    };
    fs::write(
        install_record_path(options.output_db),
        serde_json::to_vec_pretty(&record)?,
    )?;
    if let Some(export) = options.server_export {
        export_server(&mut base, &mut output, &resolved, &record, export)?;
    }
    Ok(record)
}

fn installed_packs(packs: &[Pack]) -> Vec<InstalledPack> {
    packs
        .iter()
        .map(|pack| InstalledPack {
            id: pack.manifest.id.clone(),
            version: pack.manifest.version.clone(),
            manifest_sha256: pack.manifest_sha256.clone(),
            requires: pack.manifest.requires.clone(),
            operations: pack.manifest.operations.len(),
            server: pack.manifest.server.clone(),
        })
        .collect()
}

fn kinds_of(resolved: &Resolved, kind: WholeKind) -> Vec<String> {
    resolved
        .kinds
        .iter()
        .filter(|(path, value)| **value == kind && !resolved.edited.contains(*path))
        .map(|(path, _)| path.clone())
        .collect()
}

/// Shared server-relevant resources that are always exported when changed.
const SERVER_CONFIGS: [&str; 2] = [
    "gameplay/items/modifications/modifications.options",
    "gameplay/drops/drops.options",
];

/// The resource paths the srp-026g server needs from a composition.
pub fn server_export_paths(
    base: &mut ResourceDb,
    output: &mut ResourceDb,
    changed: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::from([DICTIONARY.to_owned()]);
    let items = |db: &mut ResourceDb| -> Result<Vec<(i32, String)>> {
        let dictionary = BinaryConfig::parse(&db.read(DICTIONARY)?)?;
        let Some(ConfigValue::Indexed(entries)) = dictionary.root.named("items_dict") else {
            bail!("dictionary has no indexed items_dict");
        };
        Ok(entries
            .iter()
            .filter_map(
                |entry| match (entry.named("dict_id"), entry.named("cfg_name")) {
                    (Some(ConfigValue::Integer(id)), Some(ConfigValue::String(cfg))) => {
                        Some((*id, cfg.clone()))
                    }
                    _ => None,
                },
            )
            .collect())
    };
    let base_ids: BTreeSet<i32> = items(base)?.into_iter().map(|(id, _)| id).collect();
    for (id, cfg) in items(output)? {
        if (!base_ids.contains(&id) || changed.contains(&cfg)) && output.is_file(&cfg) {
            paths.insert(cfg);
        }
    }
    for path in changed {
        if path.starts_with("gameplay/game_modes/") || SERVER_CONFIGS.contains(&path.as_str()) {
            paths.insert(path.clone());
        }
    }
    Ok(paths)
}

/// What the server needs to know about an exported item config without
/// parsing it: the armour slots its model also occupies.
fn exported_item(path: &str, bytes: &[u8]) -> Result<Json> {
    let config = BinaryConfig::parse(bytes).with_context(|| format!("parsing {path}"))?;
    let slots: Vec<Json> = match value_at(&config.root, "data.additional_model_slots") {
        Some(ConfigValue::Indexed(values)) => values
            .iter()
            .filter_map(|value| match value {
                ConfigValue::String(slot) => Some(Json::String(slot.clone())),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(serde_json::json!({"config": path, "additional_model_slots": slots}))
}

fn export_server(
    base: &mut ResourceDb,
    output: &mut ResourceDb,
    resolved: &Resolved,
    record: &InstallRecord,
    export: &Path,
) -> Result<()> {
    let changed: BTreeSet<String> = resolved.files.keys().cloned().collect();
    let mut items = Vec::new();
    for path in server_export_paths(base, output, &changed)? {
        let destination = safe_join(export, &path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = output.read(&path)?;
        if path.starts_with("gameplay/items/") {
            items.push(exported_item(&path, &bytes)?);
        }
        fs::write(&destination, bytes)?;
    }
    let server: Map<String, Json> = record
        .packs
        .iter()
        .map(|pack| (pack.id.clone(), pack.server.clone().unwrap_or(Json::Null)))
        .collect();
    let mut content = serde_json::to_value(record)?;
    content["server"] = Json::Object(server);
    content["items"] = Json::Array(items);
    fs::write(
        export.join("content.json"),
        serde_json::to_vec_pretty(&content)?,
    )?;
    Ok(())
}

/// `pack verify`: confirm a database carries every operation of the packs.
/// `patches` (`resources.db.patch_<n>`) are laid over it as the game mounts
/// them.
pub fn verify(db_path: &Path, patches: &[PathBuf], dirs: &[PathBuf]) -> Result<usize> {
    let layers = patch::base_layers(db_path, patches)?;
    verify_in(&mut ResourceDb::open_layers(&layers)?, dirs, &[])
}

/// [`verify`] on an open database. Operations on `skipped` paths (shipped
/// outside the archive) are not checked.
pub fn verify_in(db: &mut ResourceDb, dirs: &[PathBuf], skipped: &[String]) -> Result<usize> {
    let packs = load_packs(dirs)?;
    let languages = base_languages(db);
    let mut configs = BTreeMap::<String, BinaryConfig>::new();
    let mut problems = Vec::new();
    let mut checked = 0;
    let mut config = |db: &mut ResourceDb, path: &str| -> Result<ConfigValue> {
        if !configs.contains_key(path) {
            configs.insert(path.to_owned(), BinaryConfig::parse(&db.read(path)?)?);
        }
        Ok(configs[path].root.clone())
    };
    for pack in &packs {
        for operation in &pack.manifest.operations {
            checked += 1;
            let result: Result<()> = (|| {
                match operation {
                    Operation::AddFile { path, source }
                    | Operation::ReplaceFile { path, source, .. } => {
                        let expected = sha256_hex(&fs::read(pack.file_source(path, source)?)?);
                        if sha256_hex(&db.read(path)?) != expected {
                            bail!("{path} differs from the pack's file");
                        }
                    }
                    Operation::ConfigSet { path, values } => {
                        let root = config(db, path)?;
                        for (key, json) in values {
                            let actual = value_at(&root, key)
                                .with_context(|| format!("{path} has no {key}"))?;
                            if !semantic_eq(actual, &to_config(json, Some(actual))?) {
                                bail!("{path} {key} differs");
                            }
                        }
                    }
                    Operation::ConfigAppend {
                        path,
                        array_path,
                        values,
                    } => {
                        let root = config(db, path)?;
                        let Some(ConfigValue::Indexed(array)) = value_at(&root, array_path) else {
                            bail!("{path} {array_path} is not an indexed table");
                        };
                        for json in values {
                            let wanted = to_config(json, array.first())?;
                            if !array.iter().any(|value| semantic_eq(value, &wanted)) {
                                bail!("{path} {array_path} lacks an appended value");
                            }
                        }
                    }
                    Operation::DictionaryAdd {
                        table,
                        id_field,
                        entries,
                    } => {
                        let root = config(db, DICTIONARY)?;
                        let id_field = id_field.as_deref().unwrap_or(default_id_field(table));
                        let Some(ConfigValue::Indexed(values)) = root.named(table) else {
                            bail!("dictionary has no indexed {table}");
                        };
                        for entry in entries {
                            let id = entry_id(entry, id_field)?;
                            let wanted =
                                value::table_to_config(entry, Some(&entry_hint(values, entry)))?;
                            let found = values
                                .iter()
                                .find(|value| {
                                    value.named(id_field) == Some(&ConfigValue::Integer(id as i32))
                                })
                                .with_context(|| format!("{table} lacks id {id}"))?;
                            if !semantic_eq(found, &wanted) {
                                bail!("{table} id {id} differs");
                            }
                        }
                    }
                    Operation::LocalizationAdd {
                        strings,
                        languages: listed,
                    } => {
                        for language in listed.as_ref().unwrap_or(&languages) {
                            let root = config(db, &localization_path(language))?;
                            for (key, text) in strings {
                                if root.named("strings").and_then(|table| table.named(key))
                                    != Some(&ConfigValue::String(text.clone()))
                                {
                                    bail!("{language} string {key} differs");
                                }
                            }
                        }
                    }
                    Operation::AtlasIconsAdd { swf, symbol, icons } => {
                        let path = swf.as_deref().unwrap_or(atlas::DEFAULT_SWF);
                        if skipped.iter().any(|skip| skip == path) {
                            return Ok(());
                        }
                        let movie = atlas::Swf::parse(&db.read(path)?)?;
                        let bitmap =
                            movie.bitmap(symbol.as_deref().unwrap_or(atlas::DEFAULT_SYMBOL))?;
                        for icon in icons {
                            let pixels = atlas::read_icon_png(
                                &safe_join(&pack.dir, &icon.source)?,
                                icon.premultiplied,
                            )?;
                            if bitmap.cell(icon.row, icon.col)? != pixels {
                                bail!("atlas cell [{}, {}] differs", icon.row, icon.col);
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                problems.push(format!("{}: {error:#}", pack.id()));
            }
        }
    }
    if !problems.is_empty() {
        bail!(
            "{} problem(s):\n  {}",
            problems.len(),
            problems.join("\n  ")
        );
    }
    Ok(checked)
}

#[cfg(test)]
mod tests;
