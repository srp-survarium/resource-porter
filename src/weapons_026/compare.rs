// SPDX-License-Identifier: GPL-3.0-or-later
//! A bundle against files players already have, file by file: identical,
//! equivalent (a clip whose only differences are the words the engine
//! overwrites on load), or its differences, each explained by a row of a
//! known-differences table (`ports/*.known.tsv`) or reported unexplained.
//!
//! Made for patch 19's 0.29a3 weapons (`ports/weapons-029a3-patch19.json`):
//! the tool's port of the same guns from the same donor must give patch 19's
//! files, or say why not and which side is right.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::clips::engine_words_cleared;
use super::donor::is_config;
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};

/// A known difference: which files and keys, which side is right, why.
#[derive(Clone, Debug)]
pub struct Known {
    pub path: String,
    pub key: String,
    pub right: Right,
    pub why: String,
}

/// Which side of a known difference is right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Right {
    /// The tool's output.
    Ours,
    /// The reference (the tool should change, or the port's manifest).
    Theirs,
    /// Either: a choice, both work.
    Either,
}

impl Right {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "ours" => Self::Ours,
            "theirs" => Self::Theirs,
            "either" => Self::Either,
            other => bail!("{other}: ours, theirs or either"),
        })
    }
    fn label(self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
            Self::Either => "either",
        }
    }
}

/// `path_glob TAB key_glob TAB ours|theirs|either TAB why`; `#` comments.
pub fn load_known(path: &Path) -> Result<Vec<Known>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|(n, l)| {
            let cols: Vec<&str> = l.split('\t').collect();
            let [p, k, r, why] = cols.as_slice() else {
                bail!("{}:{}: four columns", path.display(), n + 1);
            };
            Ok(Known {
                path: (*p).into(),
                key: (*k).into(),
                right: Right::parse(r)?,
                why: (*why).into(),
            })
        })
        .collect()
}

/// `*` matches any run of characters.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut at = 0;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !text.starts_with(part) {
                return false;
            }
            at = part.len();
        } else if i == parts.len() - 1 {
            return text.len() >= at + part.len() && text[at..].ends_with(part);
        } else if let Some(found) = text[at..].find(part) {
            at += found + part.len();
        } else {
            return false;
        }
    }
    true
}

/// A config's leaves: `a.b[2].c` and the value, as `dump-db-config --leaves`.
pub fn leaves(v: &V) -> BTreeMap<String, String> {
    fn walk(v: &V, key: &str, out: &mut BTreeMap<String, String>) {
        match v {
            V::Named(fields) => {
                for f in fields {
                    let k = if key.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{key}.{}", f.name)
                    };
                    walk(&f.value, &k, out);
                }
            }
            V::Indexed(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, &format!("{key}[{i}]"), out);
                }
            }
            V::Boolean(b) => {
                out.insert(key.into(), format!("bool:{b}"));
            }
            V::Integer(i) => {
                out.insert(key.into(), format!("i32:{i}"));
            }
            V::Float(f) => {
                out.insert(key.into(), format!("f32:{f}"));
            }
            V::String(s) => {
                out.insert(key.into(), format!("string:{s:?}"));
            }
            other => {
                out.insert(key.into(), format!("{other:?}"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(v, "", &mut out);
    out
}

/// One file's comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Identical,
    /// Plays the same: what differs is overwritten on load.
    Equivalent(&'static str),
    /// Differs; see the rows.
    Differs,
    OnlyOurs,
    OnlyTheirs,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub path: String,
    pub status: Status,
    /// The key (a config leaf) or `-` (the whole file).
    pub key: String,
    pub ours: String,
    pub theirs: String,
    /// The known difference that explains it.
    pub known: Option<Known>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub rows: Vec<Row>,
}

impl Report {
    pub fn unexplained(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| {
            matches!(
                r.status,
                Status::Differs | Status::OnlyOurs | Status::OnlyTheirs
            ) && r.known.is_none()
        })
    }

    /// Per status and file kind, a count; then every unexplained row.
    pub fn summary(&self) -> String {
        let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut files: BTreeSet<(&str, String)> = BTreeSet::new();
        for r in &self.rows {
            let status = match &r.status {
                Status::Identical => "identical".to_owned(),
                Status::Equivalent(_) => "equivalent".to_owned(),
                Status::Differs if r.known.is_some() => {
                    format!(
                        "differs, explained ({})",
                        r.known.as_ref().unwrap().right.label()
                    )
                }
                Status::Differs => "differs, UNEXPLAINED".to_owned(),
                Status::OnlyOurs | Status::OnlyTheirs if r.known.is_some() => {
                    format!("{:?}, explained", r.status)
                }
                other => format!("{other:?}, UNEXPLAINED"),
            };
            if files.insert((r.path.as_str(), status.clone())) {
                *counts.entry((kind(&r.path), status)).or_default() += 1;
            }
        }
        let mut out = String::from("files by kind and result:\n");
        for ((k, s), n) in &counts {
            let _ = writeln!(out, "  {n:5}  {k:18} {s}");
        }
        let mut explained: BTreeMap<String, (usize, &Known)> = BTreeMap::new();
        for r in &self.rows {
            if let Some(k) = &r.known {
                explained
                    .entry(format!("{}\t{}", k.path, k.key))
                    .or_insert((0, k))
                    .0 += 1;
            }
        }
        let _ = writeln!(out, "explained differences (rows, right side, why):");
        for (_, (n, k)) in explained {
            let _ = writeln!(
                out,
                "  {n:5}  {}  {} {}: {}",
                k.right.label(),
                k.path,
                k.key,
                k.why
            );
        }
        let un: Vec<&Row> = self.unexplained().collect();
        let _ = writeln!(out, "unexplained: {}", un.len());
        for r in un {
            let _ = writeln!(
                out,
                "  {} {} ours={} theirs={}",
                r.path, r.key, r.ours, r.theirs
            );
        }
        out
    }

    pub fn tsv(&self) -> String {
        let mut out = String::from("path\tstatus\tkey\tours\ttheirs\tright\twhy\n");
        for r in &self.rows {
            let (right, why) = match (&r.known, &r.status) {
                (Some(k), _) => (k.right.label(), k.why.as_str()),
                (None, Status::Equivalent(why)) => ("either", *why),
                _ => ("", ""),
            };
            let _ = writeln!(
                out,
                "{}\t{:?}\t{}\t{}\t{}\t{right}\t{why}",
                r.path, r.status, r.key, r.ours, r.theirs
            );
        }
        out
    }
}

fn kind(path: &str) -> String {
    if path.ends_with(".b-spline") {
        return "clip".into();
    }
    for (needle, k) in [
        ("/converted_model", "mesh"),
        (".material", "material"),
        (".dds", "texture"),
        (".ogg", "sound"),
        (".rms", "sound"),
        (".raw_options", "sound options"),
        (".composite_sound", "composite sound"),
        (".sound_collection", "sound collection"),
        (".bones_names", "clip bones"),
        ("gameplay/items/", "item config"),
        ("localization/", "localization"),
        ("gameplay/db_static", "dictionary"),
    ] {
        if path.contains(needle) {
            return k.into();
        }
    }
    "other".into()
}

/// The dict ids of the rows whose configs the bundle ships.
fn dictionary_ids(dict: &V, ours: &BTreeMap<String, std::path::PathBuf>) -> BTreeSet<i32> {
    let Some(V::Indexed(items)) = dict.named("items_dict") else {
        return BTreeSet::new();
    };
    items
        .iter()
        .filter_map(|row| {
            let cfg = match row.named("cfg_name") {
                Some(V::String(c)) => c,
                _ => return None,
            };
            match row.named("dict_id") {
                Some(V::Integer(id)) if ours.contains_key(cfg) => Some(*id),
                _ => None,
            }
        })
        .collect()
}

/// A dictionary's rows and compatibility pairs of `ids`, by id (the
/// rows' and pairs' places differ from patch to patch).
fn dictionary_view(dict: &V, ids: &BTreeSet<i32>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(V::Indexed(items)) = dict.named("items_dict") {
        for row in items {
            if let Some(V::Integer(id)) = row.named("dict_id")
                && ids.contains(id)
            {
                for (k, v) in leaves(row) {
                    out.insert(format!("items_dict[id {id}].{k}"), v);
                }
            }
        }
    }
    if let Some(V::Named(pairs)) = dict.named("items_compatibility") {
        for p in pairs {
            if let (Some(V::Integer(a)), Some(V::Integer(b))) =
                (p.value.named("first_item"), p.value.named("second_item"))
                && ids.contains(a)
            {
                out.insert(format!("items_compatibility[{a} with {b}]"), "pair".into());
            }
        }
    }
    out
}

/// `i32:3` and `f32:3`.
fn same_number(a: &str, b: &str) -> bool {
    let num = |v: &str| -> Option<f64> {
        let (kind, x) = v.split_once(':')?;
        matches!(kind, "i32" | "f32").then(|| x.parse().ok())?
    };
    matches!((num(a), num(b)), (Some(x), Some(y)) if (x - y).abs() < 1e-6)
}

fn explain<'k>(known: &'k [Known], path: &str, key: &str) -> Option<&'k Known> {
    known
        .iter()
        .find(|k| glob(&k.path, path) && glob(&k.key, key))
}

/// Compares the bundle's `overlay/` with `reference`. Files the reference
/// has under the bundle's own model, clip and sound folders (each file's
/// first four path components) that the bundle lacks are rows too.
pub fn compare(overlay: &Path, reference: &mut ResourceDb, known: &[Known]) -> Result<Report> {
    let ours = crate::archive::folder_files(overlay)?;
    let mut report = Report::default();
    let mut roots = BTreeSet::new();
    for (path, file) in &ours {
        let mine = std::fs::read(file)?;
        if path.starts_with("models/")
            || path.starts_with("animations/")
            || path.starts_with("sounds/")
            || path.starts_with("textures/")
        {
            let root: Vec<&str> = path.split('/').take(4).collect();
            if root.len() == 4 && !root[3].contains('.') {
                roots.insert(root.join("/") + "/");
            }
        }
        let mut row = |status: Status, key: &str, a: String, b: String| {
            let known = match status {
                Status::Identical | Status::Equivalent(_) => None,
                _ => explain(known, path, key).cloned().or_else(|| {
                    same_number(&a, &b).then(|| Known {
                        path: "*".into(),
                        key: "*".into(),
                        right: Right::Either,
                        why: "the same number, an integer on one side and a float on the other (the engine reads either: live configs have both)".into(),
                    })
                }),
            };
            report.rows.push(Row {
                path: path.clone(),
                status,
                key: key.into(),
                ours: a,
                theirs: b,
                known,
            });
        };
        if !reference.contains(path) {
            row(
                Status::OnlyOurs,
                "-",
                format!("{} bytes", mine.len()),
                "-".into(),
            );
            continue;
        }
        let theirs = reference.read(path)?;
        if theirs == mine {
            row(Status::Identical, "-", String::new(), String::new());
            continue;
        }
        if path.ends_with(".b-spline")
            && engine_words_cleared(&mine).ok() == engine_words_cleared(&theirs).ok()
            && engine_words_cleared(&mine).is_ok()
        {
            row(
                Status::Equivalent(
                    "the clip differs only in the slots the engine overwrites on load",
                ),
                "-",
                String::new(),
                String::new(),
            );
            continue;
        }
        if is_config(path)
            && let (Ok(a), Ok(b)) = (BinaryConfig::parse(&mine), BinaryConfig::parse(&theirs))
        {
            let (a, b) = if path == "gameplay/db_static_dictionaries" {
                let ids = dictionary_ids(&a.root, &ours);
                (
                    dictionary_view(&a.root, &ids),
                    dictionary_view(&b.root, &ids),
                )
            } else {
                (leaves(&a.root), leaves(&b.root))
            };
            if a == b {
                row(
                    Status::Equivalent("the same values, encoded otherwise"),
                    "-",
                    String::new(),
                    String::new(),
                );
                continue;
            }
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let (x, y) = (a.get(k), b.get(k));
                if x != y {
                    row(
                        Status::Differs,
                        k,
                        x.cloned().unwrap_or_else(|| "-".into()),
                        y.cloned().unwrap_or_else(|| "-".into()),
                    );
                }
            }
            continue;
        }
        row(
            Status::Differs,
            "-",
            format!("{} bytes", mine.len()),
            format!("{} bytes", theirs.len()),
        );
    }
    let theirs_only: Vec<String> = roots
        .iter()
        .flat_map(|root| {
            reference
                .paths_with_prefix(root)
                .filter(|p| reference.is_file(p) && !ours.contains_key(*p))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    for path in theirs_only {
        let known = explain(known, &path, "-").cloned();
        report.rows.push(Row {
            path,
            status: Status::OnlyTheirs,
            key: "-".into(),
            ours: "-".into(),
            theirs: "file".into(),
            known,
        });
    }
    Ok(report)
}
