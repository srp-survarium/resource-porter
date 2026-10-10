//! Static collision a project names must exist in the database it runs on.
//!
//! The client cooks every `collision_objects[].lib_name` through the class-32
//! collision cook. A name with nothing behind it fails that cook, and
//! `static_collision::insert` then builds a rigid body from the NULL shape and
//! faults (`physics::create_static_rigid_body`, read of 0x108). Nothing
//! upstream catches it: the dependency scan resolves the model directory, and
//! `compose` keeps the first bundle's copy of a shared model namespace whole,
//! so a later map's `collision` folder is suppressed when an earlier map only
//! needed the model's render surfaces.
//!
//! This pass runs over a composed bundle against its base database: every
//! collision reference in every project is checked against the effective view,
//! and objects whose collision cannot cook are dropped from the project, with
//! a per-project report.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::archive::safe_join;
use crate::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Debug, Default, Serialize)]
pub struct CollisionReport {
    /// Project path -> collision objects checked.
    pub checked: BTreeMap<String, usize>,
    /// Project path -> dropped references, as named in the project.
    pub dropped: BTreeMap<String, Vec<String>>,
    /// Model settings -> surface game materials whose id is not the
    /// database dictionary's id for that name (audit only).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub game_material_mismatches: BTreeMap<String, Vec<String>>,
}

/// The resource folder a `lib_name` names: `resources/` and any `#[...]`
/// instance suffix removed.
fn collision_folder(lib_name: &str) -> String {
    let name = lib_name.split('#').next().unwrap_or(lib_name);
    name.strip_prefix("resources/").unwrap_or(name).to_owned()
}

/// Whether the cook will accept a collision folder: exported primitives, a
/// triangle mesh, or a node at the folder path itself. Retail's level_03 names
/// three empty folders (`soviet_comp_01.model/collision`,
/// `antenna_tree.model/walker_collision`) and the client logs nothing for
/// them; only a name with no node at all fails the cook and crashes.
fn cookable(folder: &str, exists: &dyn Fn(&str) -> bool) -> bool {
    exists(folder)
        || exists(&format!("{folder}/exported_primitives"))
        || ["vertices", "indices", "face_data"]
            .iter()
            .all(|part| exists(&format!("{folder}/{part}")))
}

/// Any node -- file, folder or link -- at a path.
fn present(db: &ResourceDb, path: &str) -> bool {
    db.entries().get(path).is_some_and(|entry| {
        matches!(
            entry.kind,
            resources_db::NodeKind::File { .. }
                | resources_db::NodeKind::Folder
                | resources_db::NodeKind::HardLink
                | resources_db::NodeKind::SoftLink
        )
    })
}

/// Drop every `collision_objects` entry whose folder cannot cook.
fn prune(value: &mut ConfigValue, exists: &dyn Fn(&str) -> bool, checked: &mut usize, dropped: &mut Vec<String>) {
    match value {
        ConfigValue::Named(fields) => {
            for field in fields {
                if field.name == "collision_objects"
                    && let ConfigValue::Indexed(objects) = &mut field.value
                {
                    objects.retain(|object| {
                        let Some(ConfigValue::String(lib_name)) = object.named("lib_name") else {
                            return true;
                        };
                        *checked += 1;
                        if cookable(&collision_folder(lib_name), exists) {
                            true
                        } else {
                            dropped.push(lib_name.clone());
                            false
                        }
                    });
                } else {
                    prune(&mut field.value, exists, checked, dropped);
                }
            }
        }
        ConfigValue::Indexed(values) => {
            for value in values {
                prune(value, exists, checked, dropped);
            }
        }
        _ => {}
    }
}

/// Check and repair every project in `bundle` against `base_db`, in place.
pub fn prune_uncookable_collision(base_db: &Path, bundle: &Path) -> Result<CollisionReport> {
    let manifest_path = bundle.join("port-manifest.json");
    let mut manifest: Value = serde_json::from_reader(
        fs::File::open(&manifest_path)
            .with_context(|| format!("opening {}", manifest_path.display()))?,
    )?;
    let base = ResourceDb::open(base_db)?;
    let files = manifest["files"]
        .as_array_mut()
        .context("bundle manifest has no files array")?;
    let bundle_paths: BTreeSet<String> = files
        .iter()
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect();
    let bundle_folders: BTreeSet<String> = bundle_paths
        .iter()
        .flat_map(|path| {
            path.match_indices('/')
                .map(|(index, _)| path[..index].to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let exists = |path: &str| {
        bundle_paths.contains(path) || bundle_folders.contains(path) || present(&base, path)
    };

    let mut report = CollisionReport::default();
    for file in files.iter_mut() {
        let Some(path) = file["path"].as_str().map(str::to_owned) else {
            continue;
        };
        if !(path.starts_with("projects/")
            && (path.ends_with("/client_project") || path.ends_with("/server_project")))
        {
            continue;
        }
        let disk = safe_join(&bundle.join("overlay"), &path)?;
        let mut project = BinaryConfig::parse(&fs::read(&disk)?)
            .with_context(|| format!("parsing {path}"))?;
        let (mut checked, mut dropped) = (0, Vec::new());
        prune(&mut project.root, &exists, &mut checked, &mut dropped);
        report.checked.insert(path.clone(), checked);
        if dropped.is_empty() {
            continue;
        }
        let bytes = project.to_bytes()?;
        fs::write(&disk, &bytes)?;
        file["size"] = Value::from(bytes.len() as u64);
        file["sha256"] = Value::from(
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
        if let Some(source) = file["source"].as_str() {
            file["source"] = Value::from(format!("{source}+pruned-uncookable-collision"));
        }
        report.dropped.insert(path, dropped);
    }
    serde_json::to_writer_pretty(fs::File::create(&manifest_path)?, &manifest)?;
    serde_json::to_writer_pretty(
        fs::File::create(bundle.join("collision-report.json"))?,
        &report,
    )?;
    Ok(report)
}

/// Report, without changing anything, every project collision object in a
/// packed database whose collision cannot cook.
pub fn audit_database(db: &Path) -> Result<CollisionReport> {
    let mut db = ResourceDb::open(db)?;
    let projects: Vec<String> = db
        .entries()
        .keys()
        .filter(|path| {
            path.starts_with("projects/")
                && (path.ends_with("/client_project") || path.ends_with("/server_project"))
        })
        .cloned()
        .collect();
    let mut report = CollisionReport::default();
    for path in projects {
        let mut project =
            BinaryConfig::parse(&db.read(&path)?).with_context(|| format!("parsing {path}"))?;
        let exists = |candidate: &str| present(&db, candidate);
        let (mut checked, mut dropped) = (0, Vec::new());
        prune(&mut project.root, &exists, &mut checked, &mut dropped);
        report.checked.insert(path.clone(), checked);
        if !dropped.is_empty() {
            report.dropped.insert(path, dropped);
        }
    }

    // Every model's surfaces must bind a game material the dictionary has,
    // under the name it has: an id past the table's end selects another
    // material's decals and sounds.
    let dictionary = crate::target_native_map::load_game_materials(&mut db)?;
    let settings: Vec<String> = db
        .entries()
        .keys()
        .filter(|path| path.ends_with(".model/settings") || path.ends_with(".skinned_model/settings"))
        .cloned()
        .collect();
    for path in settings {
        let Ok(config) = BinaryConfig::parse(&db.read(&path)?) else {
            continue;
        };
        let Some(ConfigValue::Named(entries)) = config.root.named("game_material_settings") else {
            continue;
        };
        let mut wrong = Vec::new();
        for entry in entries {
            let id = match entry.value.named("game_material_id") {
                Some(ConfigValue::Integer(id)) => *id,
                _ => continue,
            };
            let name = match entry.value.named("game_material_name") {
                Some(ConfigValue::String(name)) => name.clone(),
                _ => String::new(),
            };
            if id == 65_535 {
                continue;
            }
            let expected = dictionary.get(&name).copied();
            if expected != Some(id) || !dictionary.values().any(|known| *known == id) {
                wrong.push(format!("{}={id}:{name}", entry.name));
            }
        }
        if !wrong.is_empty() {
            report.game_material_mismatches.insert(path, wrong);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    fn object(lib_name: &str) -> ConfigValue {
        ConfigValue::Named(vec![NamedValue {
            name: "lib_name".into(),
            value: ConfigValue::String(lib_name.into()),
        }])
    }

    #[test]
    fn drops_only_collision_that_cannot_cook() {
        let present: BTreeSet<&str> = [
            "models/a.model/collision/exported_primitives",
            "models/b.model/collision/vertices",
            "models/b.model/collision/indices",
            "models/b.model/collision/face_data",
            "models/c.model/collision/vertices",
        ]
        .into();
        let exists = |path: &str| present.contains(path);
        let mut root = ConfigValue::Named(vec![NamedValue {
            name: "collision_objects".into(),
            value: ConfigValue::Indexed(vec![
                object("resources/models/a.model/collision"),
                object("resources/models/b.model/collision#[1,1,1]"),
                object("resources/models/c.model/collision"),
                object("resources/models/d.model/walker_collision"),
            ]),
        }]);
        let (mut checked, mut dropped) = (0, Vec::new());
        prune(&mut root, &exists, &mut checked, &mut dropped);
        assert_eq!(checked, 4);
        assert_eq!(
            dropped,
            vec![
                "resources/models/c.model/collision".to_owned(),
                "resources/models/d.model/walker_collision".to_owned()
            ]
        );
    }
}
