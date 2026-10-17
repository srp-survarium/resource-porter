// SPDX-License-Identifier: GPL-3.0-or-later
//! Environment probes whose cubemaps are not in the game's files.
//!
//! A project's `environment_probe` objects name their cubemap twice: `texture`
//! and, per project variant, `textures.<variant>` (`day_2`, `prerain_3`, ...).
//! The engine loads `textures/<name>.dds` and its `_diffuse` companion; with
//! either missing, 0.26g renders NaNs where the probe lights (native Cologne,
//! `level_08/prerain_3`: a black screen when shooting and flickering
//! squares). [`missing_in_files`] lists such references in the projects a
//! patch writes; [`missing_in_db`] in every project of an archive.

use std::collections::BTreeMap;

use anyhow::{Context, Result};

use super::Content;
use crate::ResourceDb;
use crate::binary_config::{BinaryConfig, ConfigValue};

/// One probe cubemap reference that does not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingProbe {
    /// `projects/<name>/client_project`.
    pub project: String,
    /// The probe's index in `client_objects`, and its `full_name`.
    pub index: usize,
    pub name: String,
    /// The field: `texture` or `textures.<variant>`.
    pub field: String,
    /// The texture name, and which of its two files are missing.
    pub texture: String,
    pub missing: Vec<String>,
}

/// Whether a path is a project's client project.
pub fn is_project(path: &str) -> bool {
    path.starts_with("projects/") && path.ends_with("/client_project")
}

/// The variant a project is loaded as: its folder's last name (`day_2`).
fn variant(project: &str) -> Option<&str> {
    project.strip_suffix("/client_project")?.rsplit('/').next()
}

/// Every probe cubemap the project loads: `(index, full_name, field,
/// texture)`. The `textures` entries of other variants are other projects'
/// and are not loaded by this one.
pub fn probe_references(project: &str, root: &ConfigValue) -> Vec<(usize, String, String, String)> {
    let mut out = Vec::new();
    let Some(ConfigValue::Indexed(objects)) = root.named("client_objects") else {
        return out;
    };
    let string = |value: Option<&ConfigValue>| match value {
        Some(ConfigValue::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    for (index, object) in objects.iter().enumerate() {
        if string(object.named("game_object_type")).as_deref() != Some("environment_probe") {
            continue;
        }
        let name = string(object.named("full_name")).unwrap_or_default();
        if let Some(texture) = string(object.named("texture")) {
            out.push((index, name.clone(), "texture".to_owned(), texture));
        }
        if let Some(variant) = variant(project)
            && let Some(texture) = string(object.named("textures").and_then(|t| t.named(variant)))
        {
            out.push((index, name.clone(), format!("textures.{variant}"), texture));
        }
    }
    out
}

fn missing_of(
    project: &str,
    root: &ConfigValue,
    exists: &mut dyn FnMut(&str) -> bool,
) -> Vec<MissingProbe> {
    let mut out = Vec::new();
    for (index, name, field, texture) in probe_references(project, root) {
        let missing: Vec<String> = [
            format!("textures/{texture}.dds"),
            format!("textures/{texture}_diffuse.dds"),
        ]
        .into_iter()
        .filter(|path| !exists(path))
        .collect();
        if !missing.is_empty() {
            out.push(MissingProbe {
                project: project.to_owned(),
                index,
                name,
                field,
                texture,
                missing,
            });
        }
    }
    out
}

/// The unresolved probe cubemaps of the projects among `files` (a patch's
/// resolved files over `base`): a texture resolves in `files` or in `base`.
pub fn missing_in_files(
    base: &ResourceDb,
    files: &BTreeMap<String, Content>,
) -> Result<Vec<MissingProbe>> {
    let mut out = Vec::new();
    for (path, content) in files.iter().filter(|(path, _)| is_project(path)) {
        let root = BinaryConfig::parse(&content.bytes()?)
            .with_context(|| format!("decoding {path}"))?
            .root;
        out.extend(missing_of(path, &root, &mut |texture| {
            files.contains_key(texture) || base.is_file(texture)
        }));
    }
    Ok(out)
}

/// The unresolved probe cubemaps of every project of `db`.
pub fn missing_in_db(db: &mut ResourceDb) -> Result<Vec<MissingProbe>> {
    let projects: Vec<String> = db
        .paths()
        .filter(|path| is_project(path))
        .map(str::to_owned)
        .collect();
    let mut out = Vec::new();
    for project in projects {
        let root = BinaryConfig::parse(&db.read(&project)?)
            .with_context(|| format!("decoding {project}"))?
            .root;
        out.extend(missing_of(&project, &root, &mut |texture| db.is_file(texture)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_config::NamedValue;

    fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            fields
                .into_iter()
                .map(|(name, value)| NamedValue {
                    name: name.into(),
                    value,
                })
                .collect(),
        )
    }

    fn probe(name: &str, texture: &str, variants: Vec<(&str, &str)>) -> ConfigValue {
        named(vec![
            ("game_object_type", ConfigValue::String("environment_probe".into())),
            ("full_name", ConfigValue::String(name.into())),
            ("texture", ConfigValue::String(texture.into())),
            (
                "textures",
                named(
                    variants
                        .into_iter()
                        .map(|(k, v)| (k, ConfigValue::String(v.into())))
                        .collect(),
                ),
            ),
        ])
    }

    #[test]
    fn a_probe_of_its_variant_without_its_cubemap_is_found() {
        // Cologne prerain_3 as retail ships it: probe (19) names a prerain_3
        // cubemap that is not there; the prerain_2 one of the same probe is.
        let project = "projects/level_08/prerain_3/client_project";
        let root = named(vec![(
            "client_objects",
            ConfigValue::Indexed(vec![
                named(vec![("game_object_type", ConfigValue::String("light".into()))]),
                probe(
                    "probes/indoor(19)",
                    "probes/p3_gone",
                    vec![("prerain_2", "probes/p2_there"), ("prerain_3", "probes/p3_gone")],
                ),
                probe(
                    "probes/fine",
                    "probes/p3_there",
                    vec![("prerain_3", "probes/p3_there"), ("prerain_4", "probes/p4_gone")],
                ),
            ]),
        )]);
        let have = [
            "textures/probes/p2_there.dds",
            "textures/probes/p2_there_diffuse.dds",
            "textures/probes/p3_there.dds",
            "textures/probes/p3_there_diffuse.dds",
            "textures/probes/p3_gone.dds",
        ];
        let found = missing_of(project, &root, &mut |path| have.contains(&path));
        // Only the probe's own variant counts: prerain_4 is another project's.
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|m| m.index == 1 && m.texture == "probes/p3_gone"));
        assert_eq!(
            found.iter().map(|m| m.field.as_str()).collect::<Vec<_>>(),
            ["texture", "textures.prerain_3"]
        );
        // The .dds is there; its diffuse companion is not.
        assert_eq!(found[0].missing, ["textures/probes/p3_gone_diffuse.dds"]);
        // Pointed at the existing prerain_2 cubemap, nothing is missing.
        let mut fixed = root.clone();
        for field in ["client_objects.1.texture", "client_objects.1.textures.prerain_3"] {
            *super::super::value::value_at_mut(&mut fixed, field).unwrap() =
                ConfigValue::String("probes/p2_there".into());
        }
        assert!(missing_of(project, &fixed, &mut |path| have.contains(&path)).is_empty());
    }
}
