// SPDX-License-Identifier: GPL-3.0-or-later
//! What a map of a build near 0.26g needs from its donor, against what players
//! have: the file closure of its projects, each file absent, the same or
//! different in the target; the material switch combinations no target
//! material has (their shader programs may not be shipped); the static render
//! geometry each project loads.
//!
//!     near-build-survey DONOR.list TARGET.list OUT.json PROJECT...
//!
//! A `.list` names the archives of a game, one a line, bottom first.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb, dependency};

fn layers(list: &str) -> Result<Vec<PathBuf>> {
    Ok(std::fs::read_to_string(list)
        .with_context(|| format!("reading {list}"))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect())
}

/// A static surface's chunks: (id, payload).
fn chunks(bytes: &[u8]) -> Option<Vec<(u32, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let id = u32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..at + 8 + size)?;
        out.push((id, body));
        at += 8 + size;
    }
    (at == bytes.len()).then_some(out)
}

fn geometry(bytes: &[u8]) -> usize {
    let u = |o: usize| {
        bytes
            .get(o..o + 4)
            .map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()) as usize)
    };
    if u(0) != 3 {
        return 0;
    }
    (u(4) - 4) + u(8 + u(4) + 8) * 2
}

fn kind_of(path: &str) -> &'static str {
    if path.starts_with("textures/environment_probes/") {
        "probe cubemap"
    } else if path.starts_with("textures/") {
        "texture"
    } else if path.ends_with("/converted_model") {
        "render surface"
    } else if path.contains(".model/collision")
        || path.contains(".model/walker_collision")
        || path.contains("_collision/")
    {
        "collision"
    } else if path.starts_with("models/") {
        "model config"
    } else if path.starts_with("material_instances/") {
        "material"
    } else if path.starts_with("projects/") {
        "project"
    } else if path.starts_with("particles/") {
        "particle"
    } else if path.starts_with("sounds/") {
        "sound"
    } else if path.starts_with("animations/") {
        "animation"
    } else {
        "other"
    }
}

/// (effect id, switches on, selector values) of each stage of a material.
fn stages(root: &ConfigValue, known: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(ConfigValue::Named(material)) = root.named("material") else {
        return out;
    };
    for stage in material {
        let Some(ConfigValue::Named(effect)) = stage.value.named("effect") else {
            continue;
        };
        let mut id = String::new();
        let mut on = BTreeSet::new();
        for field in effect {
            if field.name == "effect_id" {
                if let ConfigValue::String(s) = &field.value {
                    id = s.clone();
                }
                continue;
            }
            let value = field.value.named("value");
            if !known.is_empty() && !known.contains(&field.name) {
                continue;
            }
            if field.name.starts_with("use_") {
                if matches!(value, Some(ConfigValue::Boolean(true))) {
                    on.insert(field.name.clone());
                }
            } else if (field.name.ends_with("_technique") || field.name.ends_with("_mode"))
                && let Some(ConfigValue::Integer(i)) = value
            {
                on.insert(format!("{}={i}", field.name));
            }
        }
        out.insert(
            stage.name.clone(),
            format!("{id}\t{}", on.into_iter().collect::<Vec<_>>().join(",")),
        );
    }
    out
}

fn field_names(root: &ConfigValue, prefix: &str, out: &mut BTreeSet<String>) {
    match root {
        ConfigValue::Named(fields) => {
            for f in fields {
                let p = format!("{prefix}.{}", f.name);
                field_names(&f.value, &p, out);
            }
        }
        ConfigValue::Indexed(items) => {
            for i in items {
                field_names(i, &format!("{prefix}[]"), out);
            }
        }
        _ => {
            out.insert(prefix.to_owned());
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 4,
        "usage: near-build-survey DONOR.list TARGET.list OUT.json PROJECT..."
    );
    let donor_layers = layers(&args[0])?;
    let mut donor = ResourceDb::open_layers(&donor_layers)?;
    let mut donor_again = ResourceDb::open_layers(&donor_layers)?;
    let mut target = ResourceDb::open_layers(&layers(&args[1])?)?;
    let projects = &args[3..];
    // The effect fields the stock game's own materials have (KNOWN_FROM: its
    // archive list): a switch none of them has is one its executable does
    // not read, and selects nothing there.
    let mut known: BTreeSet<String> = BTreeSet::new();
    if let Ok(list) = std::env::var("KNOWN_FROM") {
        let mut stock = ResourceDb::open_layers(&layers(&list)?)?;
        let paths: Vec<String> = stock
            .paths_with_prefix("material_instances/")
            .filter(|p| p.ends_with(".material"))
            .map(str::to_owned)
            .collect();
        for path in paths {
            let Ok(config) = BinaryConfig::parse(&stock.read(&path)?) else { continue };
            let Some(ConfigValue::Named(material)) = config.root.named("material") else { continue };
            for stage in material {
                if let Some(ConfigValue::Named(effect)) = stage.value.named("effect") {
                    known.extend(effect.iter().map(|f| f.name.clone()));
                }
            }
        }
    }
    let mut unknown_on: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // The closure, resolved in the donor alone.
    let plan = dependency::scan(&mut donor, &mut donor_again, projects.iter().cloned())?;
    let closure: BTreeSet<String> = plan
        .donor_files
        .iter()
        .chain(plan.target_files.iter())
        .cloned()
        .collect();

    #[derive(Default, serde::Serialize)]
    struct Class {
        files: usize,
        bytes: u64,
    }
    let mut classes: BTreeMap<String, Class> = BTreeMap::new();
    let mut different: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut absent: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut unparsed = Vec::new();
    let mut surface_chunks: BTreeMap<String, usize> = BTreeMap::new();
    let mut material_fields_donor = BTreeSet::new();
    let mut donor_combos: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in &closure {
        let bytes = donor.read(path)?;
        let kind = kind_of(path);
        let state = if !target.is_file(path) {
            absent.entry(kind).or_default().push(path.clone());
            "absent"
        } else {
            let have = target.read(path)?;
            if have == bytes {
                "same"
            } else if kind == "render surface"
                && let (Some(a), Some(b)) = (chunks(&bytes), chunks(&have))
                && a.iter().filter(|c| c.0 == 3 || c.0 == 4).collect::<Vec<_>>()
                    == b.iter().filter(|c| c.0 == 3 || c.0 == 4).collect::<Vec<_>>()
            {
                "same geometry, other chunks"
            } else {
                different.entry(kind).or_default().push(path.clone());
                "different"
            }
        };
        let class = classes.entry(format!("{kind}\t{state}")).or_default();
        class.files += 1;
        class.bytes += bytes.len() as u64;
        if kind == "render surface" {
            let ids = chunks(&bytes).map_or("unreadable".to_owned(), |c| {
                c.iter().map(|c| c.0.to_string()).collect::<Vec<_>>().join(",")
            });
            *surface_chunks.entry(ids).or_default() += 1;
        }
        if kind == "material" {
            match BinaryConfig::parse(&bytes) {
                Ok(config) => {
                    field_names(&config.root, "", &mut material_fields_donor);
                    if let Some(ConfigValue::Named(material)) = config.root.named("material") {
                        for stage in material {
                            let Some(ConfigValue::Named(effect)) = stage.value.named("effect") else { continue };
                            for field in effect {
                                if !known.is_empty()
                                    && !known.contains(&field.name)
                                    && field.name.starts_with("use_")
                                    && matches!(field.value.named("value"), Some(ConfigValue::Boolean(true)))
                                {
                                    unknown_on
                                        .entry(format!("{}.{}", stage.name, field.name))
                                        .or_default()
                                        .push(path.clone());
                                }
                            }
                        }
                    }
                    for (stage, combo) in stages(&config.root, &known) {
                        donor_combos
                            .entry(format!("{stage}\t{combo}"))
                            .or_default()
                            .push(path.clone());
                    }
                }
                Err(e) => unparsed.push(format!("{path}: {e}")),
            }
        }
    }

    // Every switch combination a target material has.
    let mut target_combos = BTreeSet::new();
    let mut material_fields_target = BTreeSet::new();
    let materials: Vec<String> = target
        .paths_with_prefix("material_instances/")
        .filter(|p| p.ends_with(".material"))
        .map(str::to_owned)
        .collect();
    for path in &materials {
        if let Ok(config) = BinaryConfig::parse(&target.read(path)?) {
            field_names(&config.root, "", &mut material_fields_target);
            for (stage, combo) in stages(&config.root, &known) {
                target_combos.insert(format!("{stage}\t{combo}"));
            }
        }
    }
    let new_combos: BTreeMap<&String, &Vec<String>> = donor_combos
        .iter()
        .filter(|(combo, _)| !target_combos.contains(*combo))
        .collect();

    // Geometry per project: every distinct render model's surfaces.
    let mut geometry_of = BTreeMap::new();
    let mut objects_of = BTreeMap::new();
    for project in projects {
        let root = BinaryConfig::parse(&donor.read(project)?)
            .with_context(|| format!("parsing {project}"))?
            .root;
        let mut models = BTreeSet::new();
        if let Some(ConfigValue::Indexed(visuals)) = root.named("render_visuals") {
            for v in visuals {
                if let Some(ConfigValue::String(s)) =
                    v.named("model_name").or_else(|| v.named("lib_name"))
                {
                    models.insert(s.replace('\\', "/"));
                }
            }
        }
        let mut total = 0usize;
        let mut absent_bytes = 0usize;
        for m in &models {
            let prefix = format!("models/{m}.model/render/");
            let paths: Vec<String> = donor
                .paths_with_prefix(&prefix)
                .filter(|p| p.ends_with("/converted_model"))
                .map(str::to_owned)
                .collect();
            for p in paths {
                let g = geometry(&donor.read(&p)?);
                total += g;
                if !target.is_file(&p) {
                    absent_bytes += g;
                }
            }
        }
        geometry_of.insert(project.clone(), (models.len(), total, absent_bytes));
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        if let Some(ConfigValue::Indexed(objects)) = root.named("client_objects") {
            for o in objects {
                if let Some(ConfigValue::String(s)) = o.named("lib_name") {
                    *kinds.entry(s.clone()).or_default() += 1;
                }
            }
        }
        objects_of.insert(project.clone(), kinds);
    }

    let digest = |paths: &BTreeSet<String>| {
        let mut h = Sha256::new();
        for p in paths {
            h.update(p.as_bytes());
            h.update([0]);
        }
        format!("{:x}", h.finalize())
    };
    let report = serde_json::json!({
        "projects": projects,
        "closure_files": closure.len(),
        "closure_paths_sha256": digest(&closure),
        "unresolved": plan.unresolved_references,
        "classes": classes,
        "absent": absent,
        "different": different,
        "surface_chunk_sets": surface_chunks,
        "unparsed": unparsed,
        "material_fields_only_donor": material_fields_donor.difference(&material_fields_target).collect::<Vec<_>>(),
        "target_materials": materials.len(),
        "target_combos": target_combos.len(),
        "donor_combos": donor_combos.len(),
        "new_combos": new_combos,
        "switches_on_the_target_does_not_know": unknown_on,
        "geometry": geometry_of,
        "client_objects": objects_of,
    });
    std::fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    println!("closure {} files; unresolved {}", closure.len(), plan.unresolved_references.len());
    for (class, c) in &classes {
        println!("{class}\t{}\t{:.1} MiB", c.files, c.bytes as f64 / 1048576.0);
    }
    println!(
        "material combos: donor {} / target {} / donor-only {}",
        donor_combos.len(),
        target_combos.len(),
        new_combos.len()
    );
    for (project, (models, total, absent)) in &geometry_of {
        println!(
            "{project}: {models} models, geometry {:.1} MiB ({:.1} MiB in files the target lacks)",
            *total as f64 / 1048576.0,
            *absent as f64 / 1048576.0
        );
    }
    Ok(())
}
