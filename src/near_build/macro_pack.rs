// SPDX-License-Identifier: GPL-3.0-or-later
//! Shader macros a client patch makes 0.26g set, as a content pack: the
//! families' masks read them, players' programs move to their keys with the
//! new macros 0, and the donors' programs come for the keys players lack.
//!
//! A program's key has a character for every macro of the key order, `_`
//! where `shaders/masks` says the family does not read it. A macro 0.26g
//! never set was left out of a family's mask; once a patch sets it (skhid's
//! patch 169: CONFIG_USE_OLTA for a DXT1 transparency map,
//! CONFIG_USE_FUZZINESS for a distortion of the emissive map alone), every
//! key of the family has a character more, so:
//!
//! 1. the masks gain the macros (`config_append` to `shaders/masks`);
//! 2. each program of players' family is added again under its key with
//!    the new macros 0 (the old files stay, read by no key);
//! 3. every program of the donors' families players then lack is added at
//!    its 0.26g key where 0.26g can feed it (`programs::Known::refuses`,
//!    with the constants the patch sets), a family the manifest names.
//!
//! Two donors may be given (both Londons': 0.44a5 and 0.32a4); a key both
//! have is one program when its code is the same (the include records before
//! the code keep each source file's time); where the code differs the first
//! donor's is supplied and the report lists it (`donors_differ`).

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;

use super::programs::{KeyOrder, Keys, Known, code, translate};
use crate::{
    ResourceDb,
    content_pack::{Operation, PackManifest, TARGET_BUILD},
};

const ROOT: &str = "shaders/sm_4_0/";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub pack: super::Pack,
    pub stock_sha256: String,
    /// The client patch that sets the macros (for the notes).
    pub client_patch: u32,
    /// 0.26g's key order, and each donor's (beside the manifest), in the
    /// order of the `--donor` lists.
    pub target_key_order: std::path::PathBuf,
    pub donor_key_orders: Vec<std::path::PathBuf>,
    /// The macros each family's mask gains.
    pub new_macros: BTreeMap<String, Vec<String>>,
    /// The macros the patch sets in each family's keys (the new ones and
    /// those 0.26g's masks list but never set on): the report counts the
    /// supplied programs with one on, and each such key must cover every
    /// runtime choice.
    pub patch_macros: BTreeMap<String, Vec<String>>,
    /// The families whose donors' programs are supplied.
    pub families: Vec<String>,
    /// Macros a draw or a player's setting chooses, not the material (the
    /// quality settings, the light, the mesh's vertex input, the level of
    /// detail): a key with a new macro on must be there for each of their
    /// values the key with it off has.
    pub runtime_macros: Vec<String>,
    /// Constants and textures a family's donor programs read that the
    /// donor's effect never sets for them either (with why): 0 in both
    /// builds' draws, so they do not refuse a program.
    #[serde(default)]
    pub unset: BTreeMap<String, Unset>,
    /// Constants the patch sets for a family, with their bytes: a donor
    /// program may read them that far.
    #[serde(default)]
    pub filled: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unset {
    pub constants: Vec<String>,
    pub textures: Vec<String>,
    pub why: String,
}

/// What the pack holds, by family.
#[derive(Debug, Default, serde::Serialize)]
pub struct FamilyReport {
    /// Players' programs added again under their wider keys.
    pub players_rekeyed: usize,
    /// Players' programs whose key is not the mask's (stale files of an
    /// older mask), left as they are.
    pub players_stale: usize,
    /// The donors' programs players lack, and how many are supplied.
    pub donor_lacking: usize,
    pub donor_supplied: usize,
    /// Of the supplied, those with a new macro not 0 (the patch's own).
    pub supplied_with_new_macros: usize,
    pub not_supplied: BTreeMap<String, usize>,
    /// Keys both donors have.
    pub keys_in_both_donors: usize,
    /// Of those, the ones whose code differs: the first donor's is supplied.
    pub donors_differ: Vec<String>,
    /// Groups of keys with a new macro on that lack a runtime choice (the
    /// build stops on any).
    pub incomplete: Vec<String>,
}

fn layers(list: &Path) -> Result<Vec<std::path::PathBuf>> {
    let text = fs::read_to_string(list).with_context(|| format!("reading {}", list.display()))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(Into::into).collect())
}

fn programs(db: &ResourceDb, family: &str) -> Vec<String> {
    let prefix = format!("{ROOT}{family}/");
    db.paths_with_prefix(&prefix).filter(|p| db.is_file(p)).map(|p| p[prefix.len()..].to_owned()).collect()
}

pub fn build(manifest_path: &Path, donors: &[std::path::PathBuf], shipped: &Path, output: &Path) -> Result<()> {
    let m: Manifest = serde_json::from_slice(&fs::read(manifest_path)?).with_context(|| format!("reading {}", manifest_path.display()))?;
    ensure!(!output.exists(), "output must be new");
    ensure!(!donors.is_empty() && donors.len() == m.donor_key_orders.len(), "one --donor list for each of the manifest's donor key orders");
    let beside = manifest_path.parent().unwrap_or(Path::new("."));
    let order = |file: &Path| -> Result<KeyOrder> { KeyOrder::parse(&fs::read_to_string(beside.join(file)).with_context(|| format!("reading {}", file.display()))?) };

    let shipped_layers = layers(shipped)?;
    crate::survarium_026::check_hash(&shipped_layers[0], &m.stock_sha256)?;
    let mut shipped = ResourceDb::open_layers(&shipped_layers)?;
    let old = Keys::read(order(&m.target_key_order)?, &mut shipped)?;
    let mut new = Keys::read(order(&m.target_key_order)?, &mut shipped)?;
    for (family, macros) in &m.new_macros {
        new = new.with_macros(family, macros)?;
    }
    let mut donor_dbs = Vec::new();
    let mut donor_keys = Vec::new();
    for (list, key_order) in donors.iter().zip(&m.donor_key_orders) {
        let mut db = ResourceDb::open_layers(&layers(list)?)?;
        donor_keys.push(Keys::read(order(key_order)?, &mut db)?);
        donor_dbs.push(db);
    }

    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut report: BTreeMap<String, FamilyReport> = BTreeMap::new();
    let all: BTreeSet<&String> = m.new_macros.keys().chain(&m.families).collect();
    for family in all {
        let r = report.entry(family.clone()).or_default();
        // Players' programs under the wider keys.
        let theirs = programs(&shipped, family);
        let mut players_keys = BTreeSet::new();
        for key in &theirs {
            let wide = match translate(key, family, &old, &new) {
                Ok(wide) => wide,
                Err(_) => {
                    r.players_stale += 1;
                    continue;
                }
            };
            players_keys.insert(wide.clone());
            if wide != *key {
                let path = format!("{ROOT}{family}/{wide}");
                ensure!(!shipped.is_file(&path), "{path}: players have it already");
                out.insert(path, shipped.read(&format!("{ROOT}{family}/{key}"))?);
                r.players_rekeyed += 1;
            }
        }
        if !m.families.contains(family) {
            continue;
        }
        // The donors' programs players lack, at 0.26g's keys.
        let mut known = Known::read(&mut shipped, family)?;
        for (name, size) in m.filled.get(family).into_iter().flatten() {
            known.fill(name, *size);
        }
        if let Some(unset) = m.unset.get(family) {
            for name in &unset.constants {
                known.fill(name, u32::MAX);
            }
            for name in &unset.textures {
                known.allow_unbound(name);
            }
        }
        let new_macros: Vec<usize> = m.patch_macros.get(family).into_iter().flatten().filter_map(|name| new.order.place(name)).collect();
        // The values players' keys give each macro: a macro 0.26g's family reads
        // and a donor's does not (0.44a5's particle lighting has no quality
        // variants) takes each of them, the donor's one program serving all.
        let mut players_values: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for key in &players_keys {
            for (at, v) in new.values(family, key)?.into_iter().enumerate() {
                players_values.entry(at).or_default().insert(v.to_owned());
            }
        }
        let mut found: BTreeMap<String, (String, Vec<u8>)> = BTreeMap::new();
        for (index, (db, keys)) in donor_dbs.iter_mut().zip(&donor_keys).enumerate() {
            let target_only: Vec<usize> =
                new.order.names().iter().enumerate().filter(|(_, name)| new.reads(family, name) && !keys.reads(family, name)).map(|(at, _)| at).collect();
            for key in programs(db, family) {
                // A key with a macro 0.26g does not read on is no key 0.26g computes.
                let Ok(target) = translate(&key, family, keys, &new) else { continue };
                // The round trip names this program: no other key of the
                // donor's falls on the same 0.26g key.
                if translate(&target, family, &new, keys).ok().as_deref() != Some(key.as_str()) {
                    *r.not_supplied.entry("another donor program has the same 0.26g key".into()).or_default() += 1;
                    continue;
                }
                let donor_path = format!("{ROOT}{family}/{key}");
                let blob = db.read(&donor_path)?;
                let mut targets = vec![new.values(family, &target)?.into_iter().map(str::to_owned).collect::<Vec<String>>()];
                for at in &target_only {
                    let choices = players_values.get(at).cloned().unwrap_or_else(|| ["0".to_owned()].into());
                    targets = targets
                        .into_iter()
                        .flat_map(|t| choices.iter().map(move |c| { let mut t = t.clone(); t[*at] = c.clone(); t }))
                        .collect();
                }
                for target in targets.into_iter().map(|values| values.concat()) {
                match found.get(&target) {
                    Some((first, bytes)) => {
                        r.keys_in_both_donors += 1;
                        // The same code is one program (the include records keep
                        // each source file's time); other code is the first donor's,
                        // listed.
                        let (ours, theirs) = (code(bytes)?, code(&blob)?);
                        if ours != theirs {
                            let chunks: Vec<String> =
                                ours.keys().chain(theirs.keys()).filter(|t| ours.get(*t) != theirs.get(*t)).map(|t| String::from_utf8_lossy(t).into_owned()).collect::<BTreeSet<_>>().into_iter().collect();
                            r.donors_differ.push(format!("{target}: {first} kept; donor {} {donor_path} differs in {}", index + 1, chunks.join(", ")));
                        }
                    }
                    None => {
                        found.insert(target, (format!("donor {} {donor_path}", index + 1), blob.clone()));
                    }
                }
                }
            }
        }
        for (target, (from, blob)) in found {
            if players_keys.contains(&target) {
                continue;
            }
            r.donor_lacking += 1;
            if let Some(why) = known.refuses(family, &from, &blob)? {
                let why = why.split_once(" reads what ").map_or(why.clone(), |(_, w)| format!("reads what {w}"));
                *r.not_supplied.entry(why).or_default() += 1;
                continue;
            }
            let values = new.values(family, &target)?;
            if new_macros.iter().any(|at| values[*at] != "0") {
                r.supplied_with_new_macros += 1;
            }
            r.donor_supplied += 1;
            out.insert(format!("{ROOT}{family}/{target}"), blob);
        }
        // A key with a new macro on is there for every choice a draw or a
        // player's setting makes (`runtime_macros`) that the same key with the
        // new macros off has, or a material that turns the macro on finds no
        // program at some quality.
        if !new_macros.is_empty() {
            let runtime: Vec<usize> = m.runtime_macros.iter().filter_map(|name| new.order.place(name)).collect();
            let prefix = format!("{ROOT}{family}/");
            let mut have: BTreeSet<String> = players_keys.clone();
            have.extend(out.keys().filter_map(|p| p.strip_prefix(&prefix)).map(str::to_owned));
            let split = |key: &str| -> Result<(Vec<String>, Vec<String>, bool)> {
                let values = new.values(family, key)?;
                let mut material = Vec::new();
                let mut choice = Vec::new();
                for (at, v) in values.iter().enumerate() {
                    if runtime.contains(&at) {
                        choice.push((*v).to_owned());
                    } else if new_macros.contains(&at) {
                        material.push("0".to_owned());
                    } else {
                        material.push((*v).to_owned());
                    }
                }
                Ok((material, choice, new_macros.iter().any(|at| values[*at] != "0")))
            };
            let mut choices: BTreeMap<Vec<String>, BTreeSet<Vec<String>>> = BTreeMap::new();
            let mut on: BTreeMap<(Vec<String>, Vec<String>), BTreeSet<Vec<String>>> = BTreeMap::new();
            for key in &have {
                let (material, choice, with_new) = split(key)?;
                if with_new {
                    let values = new.values(family, key)?;
                    let new_values: Vec<String> = new_macros.iter().map(|at| values[*at].to_owned()).collect();
                    on.entry((material, new_values)).or_default().insert(choice);
                } else {
                    choices.entry(material).or_default().insert(choice);
                }
            }
            for ((material, new_values), got) in &on {
                let wanted = choices.get(material).cloned().unwrap_or_default();
                let missing: Vec<&Vec<String>> = wanted.difference(got).collect();
                if !missing.is_empty() {
                    r.incomplete.push(format!(
                        "new macros {new_values:?} of material values {}: {} of {} runtime choices missing, e.g. {:?}",
                        material.join(""),
                        missing.len(),
                        wanted.len(),
                        missing.first()
                    ));
                }
            }
        }
    }

    fs::create_dir_all(output)?;
    fs::write(output.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    let incomplete: Vec<&String> = report.values().flat_map(|r| &r.incomplete).collect();
    ensure!(
        incomplete.is_empty(),
        "programs with a patch's macro on lack runtime choices the same keys with it off have (a material would find no program at some quality; report.json says which and why the donors' were not supplied): {incomplete:#?}"
    );
    let mut operations = Vec::new();
    for (family, macros) in &m.new_macros {
        let (name, stage) = family.rsplit_once('.').with_context(|| format!("{family}: not <name>.<stage>"))?;
        operations.push(Operation::ConfigAppend {
            path: "shaders/masks".into(),
            array_path: format!("{name}.{stage}"),
            values: macros.iter().map(|m| json!(m)).collect(),
        });
    }
    for (path, bytes) in &out {
        let at = crate::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, bytes)?;
        operations.push(Operation::AddFile { path: path.clone(), source: None });
    }
    let notes = vec![format!(
        "Built by build-shader-macro-pack: the masks of {} gain the macros client patch {} sets; players' programs under their wider keys; the donors' programs players lack where 0.26g feeds them.",
        m.new_macros.keys().cloned().collect::<Vec<_>>().join(", "),
        m.client_patch
    )];
    let pack = PackManifest {
        format_version: 1,
        id: m.pack.id.clone(),
        version: m.pack.version.clone(),
        target_build: TARGET_BUILD.into(),
        description: Some(m.pack.description.clone()),
        requires: vec![],
        base_db_sha256: Some(m.stock_sha256.clone()),
        operations,
        server: None,
        notes,
    };
    fs::create_dir_all(output)?;
    fs::write(output.join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    for (family, r) in &report {
        println!(
            "{family}: {} of players' rekeyed ({} stale left); donors' lacking {}, supplied {} ({} with a new macro on), {} keys in both donors ({} with other code); not supplied {:?}",
            r.players_rekeyed, r.players_stale, r.donor_lacking, r.donor_supplied, r.supplied_with_new_macros, r.keys_in_both_donors, r.donors_differ.len(), r.not_supplied
        );
    }
    Ok(())
}
