// SPDX-License-Identifier: GPL-3.0-or-later
//! Donor models that must not resolve to the target's copy of the same path.
//!
//! A later Survarium build edits a level's own architecture together with its
//! terrain: 0.20f's level_03 cuts a trench into `terrain_all` and fills it
//! with steps that live in its own `level/03/blocks`, while v0.100b ships an
//! older `level/03/blocks` at the same path. Shared paths resolve to the
//! target, so the trench came out as a hole onto the black underground plane.
//! Such models are emitted from the donor under a per-map root instead, and
//! the client project's references follow them. Retail resources stay
//! untouched, and two donor builds of one level no longer share a path.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use crate::ResourceDb;
use crate::binary_config::{BinaryConfig, ConfigValue};

/// The resource directory a model reference such as `level/03/blocks` names.
pub fn model_directory(model: &str) -> String {
    format!("models/{model}.model")
}

/// Where a donor file of a relocated model is emitted, if it belongs to one.
pub fn relocated_path(path: &str, models: &BTreeSet<String>, root: &str) -> Option<String> {
    models.iter().find_map(|model| {
        let tail = path.strip_prefix(&format!("{}/", model_directory(model)))?;
        Some(format!(
            "{}/{tail}",
            model_directory(&format!("{root}/{model}"))
        ))
    })
}

/// Point every reference to a relocated model at its new root: the bare
/// render reference (`level/03/blocks`, also used by vegetation model
/// definitions) and the collision form
/// (`resources/models/level/03/blocks.model/collision#[scale]`).
pub fn relocate_references(
    value: &mut ConfigValue,
    models: &BTreeSet<String>,
    root: &str,
    applied: &mut BTreeMap<String, usize>,
) {
    match value {
        ConfigValue::String(text) => {
            for model in models {
                let relocated = format!("{root}/{model}");
                let replacement = if text == model {
                    Some(relocated.clone())
                } else {
                    ["resources/", ""].iter().find_map(|prefix| {
                        let old = format!("{prefix}{}/", model_directory(model));
                        text.strip_prefix(&old)
                            .map(|tail| format!("{prefix}{}/{tail}", model_directory(&relocated)))
                    })
                };
                if let Some(replacement) = replacement {
                    *applied.entry(model.clone()).or_default() += 1;
                    *text = replacement;
                    return;
                }
            }
        }
        ConfigValue::Named(values) => {
            for child in values {
                relocate_references(&mut child.value, models, root, applied);
            }
        }
        ConfigValue::Indexed(values) => {
            for child in values {
                relocate_references(child, models, root, applied);
            }
        }
        _ => {}
    }
}

/// Every model a client project references, as `level/03/blocks`.
pub fn project_models(project: &ConfigValue) -> BTreeSet<String> {
    let mut strings = Vec::new();
    project.collect_strings(&mut strings);
    strings
        .into_iter()
        .filter_map(|text| {
            let text = text.split('#').next()?.trim();
            if let Some(rest) = text.strip_prefix("resources/models/") {
                return Some(rest.split(".model/").next()?.to_owned());
            }
            (text.contains('/') && !text.contains('.') && !text.contains(' '))
                .then(|| text.to_owned())
        })
        .collect()
}

/// Why a donor model under `prefix` should be relocated, or `None` when the
/// target's copy has the same render surfaces, geometry and collision.
pub fn relocation_reason(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    model: &str,
) -> Result<Option<String>> {
    let directory = format!("{}/", model_directory(model));
    let donor_files: BTreeSet<String> = donor
        .paths_with_prefix(&directory)
        .filter(|path| donor.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    if donor_files.is_empty() {
        return Ok(None);
    }
    let target_files: BTreeSet<String> = target
        .paths_with_prefix(&directory)
        .filter(|path| target.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    if target_files.is_empty() {
        return Ok(Some("donor-only".into()));
    }
    let surfaces = |files: &BTreeSet<String>| -> BTreeSet<String> {
        files
            .iter()
            .filter_map(|path| path.strip_suffix("/converted_model"))
            .map(ToOwned::to_owned)
            .collect()
    };
    let (donor_surfaces, target_surfaces) = (surfaces(&donor_files), surfaces(&target_files));
    if donor_surfaces != target_surfaces {
        return Ok(Some("render surfaces differ".into()));
    }
    for surface in &donor_surfaces {
        let path = format!("{surface}/converted_model");
        let a = surface_geometry(&donor.read(&path)?)
            .with_context(|| format!("decoding donor {path}"))?;
        let b = surface_geometry(&target.read(&path)?)
            .with_context(|| format!("decoding target {path}"))?;
        if !same_geometry(&a, &b) {
            return Ok(Some(format!("geometry of {path} differs")));
        }
    }
    for collision in ["collision", "walker_collision"] {
        let path = format!("{directory}{collision}/vertices");
        match (donor.is_file(&path), target.is_file(&path)) {
            (false, false) => {}
            (true, true) => {
                if donor.read(&path)? != target.read(&path)? {
                    return Ok(Some(format!("{collision} differs")));
                }
            }
            _ => return Ok(Some(format!("{collision} exists in only one build"))),
        }
    }
    Ok(None)
}

/// Vertex positions and index count of a static surface, in either the
/// target's declared layout or a later donor's declaration-less one.
fn surface_geometry(bytes: &[u8]) -> Result<(Vec<[f32; 3]>, u32)> {
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .context("static surface is truncated")?
                .try_into()?,
        ))
    };
    if read_u32(0)? != 3 {
        bail!("static surface does not start with vertex chunk 3");
    }
    let chunk = read_u32(4)? as usize;
    let declaration = crate::map_port::explicit_vertex_declaration_size(bytes).unwrap_or(0);
    let count = read_u32(8 + declaration)? as usize;
    if count == 0 {
        bail!("static surface has no vertices");
    }
    let payload = chunk
        .checked_sub(declaration + 4)
        .context("static surface vertex chunk is truncated")?;
    let stride = payload / count;
    let start = 12 + declaration;
    let mut positions = Vec::with_capacity(count);
    for vertex in 0..count {
        let offset = start + vertex * stride;
        let mut position = [0.0; 3];
        for (axis, value) in position.iter_mut().enumerate() {
            *value = f32::from_le_bytes(
                bytes
                    .get(offset + axis * 4..offset + axis * 4 + 4)
                    .context("static surface vertex is truncated")?
                    .try_into()?,
            );
        }
        positions.push(position);
    }
    Ok((positions, read_u32(8 + chunk + 8)?))
}

fn same_geometry(a: &(Vec<[f32; 3]>, u32), b: &(Vec<[f32; 3]>, u32)) -> bool {
    a.1 == b.1
        && a.0.len() == b.0.len()
        && a.0.iter().zip(&b.0).all(|(left, right)| {
            left.iter()
                .zip(right)
                .all(|(left, right)| (left - right).abs() <= 0.01)
        })
}

/// The models a donor client project references under `prefix` that would
/// otherwise resolve to a different target copy, or to a path another donor
/// build of the same level also emits.
pub fn list_relocations(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    client_project: &str,
    prefix: &str,
) -> Result<BTreeMap<String, String>> {
    let project = BinaryConfig::parse(
        &donor
            .read(client_project)
            .with_context(|| format!("reading donor {client_project}"))?,
    )
    .with_context(|| format!("parsing donor {client_project}"))?;
    let mut relocations = BTreeMap::new();
    for model in project_models(&project.root) {
        if !model.starts_with(prefix) {
            continue;
        }
        if let Some(reason) = relocation_reason(donor, target, &model)? {
            relocations.insert(model, reason);
        }
    }
    Ok(relocations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_config::NamedValue;

    #[test]
    fn relocates_render_collision_and_vegetation_references() {
        let string = |text: &str| ConfigValue::String(text.into());
        let mut project = ConfigValue::Named(vec![
            NamedValue {
                name: "render_visuals".into(),
                value: ConfigValue::Indexed(vec![
                    string("level/03/blocks"),
                    string("level/03/blocks_old"),
                ]),
            },
            NamedValue {
                name: "collision_objects".into(),
                value: ConfigValue::Indexed(vec![
                    string("resources/models/level/03/blocks.model/collision#[0.77][0.77][0.77]"),
                    string("resources/models/level/03/blocks.model/walker_collision"),
                    string("resources/models/props/box/woodbox.model/collision"),
                ]),
            },
        ]);
        let models = BTreeSet::from(["level/03/blocks".to_owned()]);
        let mut applied = BTreeMap::new();
        relocate_references(&mut project, &models, "ported/survarium/x", &mut applied);

        let ConfigValue::Indexed(visuals) = project.named("render_visuals").unwrap() else {
            panic!("render_visuals must stay indexed");
        };
        assert_eq!(visuals[0], string("ported/survarium/x/level/03/blocks"));
        assert_eq!(visuals[1], string("level/03/blocks_old"));
        let ConfigValue::Indexed(collision) = project.named("collision_objects").unwrap() else {
            panic!("collision_objects must stay indexed");
        };
        assert_eq!(
            collision[0],
            string(
                "resources/models/ported/survarium/x/level/03/blocks.model/collision#[0.77][0.77][0.77]"
            )
        );
        assert_eq!(
            collision[1],
            string("resources/models/ported/survarium/x/level/03/blocks.model/walker_collision")
        );
        assert_eq!(
            collision[2],
            string("resources/models/props/box/woodbox.model/collision")
        );
        assert_eq!(applied.get("level/03/blocks"), Some(&3));

        assert_eq!(
            relocated_path(
                "models/level/03/blocks.model/render/stupeni_s/converted_model",
                &models,
                "ported/survarium/x"
            ),
            Some(
                "models/ported/survarium/x/level/03/blocks.model/render/stupeni_s/converted_model"
                    .into()
            )
        );
        assert_eq!(
            relocated_path(
                "models/level/03/blocks_old.model/settings",
                &models,
                "ported/survarium/x"
            ),
            None
        );
    }

    #[test]
    fn compares_declared_and_declarationless_surfaces_by_geometry() {
        // One triangle in the donor's 28-byte layout and in the target's
        // declared 32-byte layout: same positions, different encoding.
        let positions = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let mut donor = Vec::new();
        donor.extend(3u32.to_le_bytes());
        donor.extend((4 + 3 * 28u32).to_le_bytes());
        donor.extend(3u32.to_le_bytes());
        for position in positions {
            for value in position {
                donor.extend(value.to_le_bytes());
            }
            donor.extend([0u8; 16]);
        }
        donor.extend(4u32.to_le_bytes());
        donor.extend(10u32.to_le_bytes());
        donor.extend(3u32.to_le_bytes());
        donor.extend([0u8, 0, 1, 0, 2, 0]);
        let upgraded = match crate::map_port::upgrade_stripped_static_model(&donor).unwrap() {
            crate::map_port::StaticUpgrade::Converted(bytes) => bytes,
            crate::map_port::StaticUpgrade::AlreadyCompatible => {
                panic!("donor layout must convert")
            }
        };
        let a = surface_geometry(&donor).unwrap();
        let b = surface_geometry(&upgraded).unwrap();
        assert_eq!(a.0, positions.to_vec());
        assert!(same_geometry(&a, &b));
        let mut moved = b.clone();
        moved.0[1][0] += 0.5;
        assert!(!same_geometry(&a, &moved));
    }
}
