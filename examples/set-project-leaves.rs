//! Reviewed numeric edits of a built content pack's projects, in place: named
//! client objects' existing leaves set to typed values, each checked against
//! the value it replaces, so a pack version can be derived from the one
//! before with exactly those changes (each one is printed for the record).
//!
//!     cargo run --release --example set-project-leaves -- PACK_DIR PLAN.json
//!
//! PLAN.json: `{"projects": {"<project path under files/>": [{"object": "<full_name>",
//! "index": N, "set": {"<leaf path>": {"was": OLD, "i32" | "f32" | "bool": NEW}}}]}}`.
//! The object is found by its `full_name` in `client_objects` and must sit at
//! `index` (the plan names both, so neither drifts); a leaf path is dotted,
//! array elements by number (`artefacts.0.count`). The leaf must exist and
//! hold `was` (an integer or a float, compared by value); it becomes NEW of the
//! named type, which may differ from the old one (native 0.26 stores some
//! chances as i32 1 where 0.21 stored f32). Every edit must change something.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use vostok_resource_porter::{BinaryConfig, ConfigValue};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    projects: BTreeMap<String, Vec<Edit>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    object: String,
    index: usize,
    set: BTreeMap<String, Leaf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Leaf {
    was: serde_json::Value,
    #[serde(default)]
    i32: Option<i32>,
    #[serde(default)]
    f32: Option<f32>,
    #[serde(default)]
    bool: Option<bool>,
}

fn leaf_mut<'a>(mut value: &'a mut ConfigValue, path: &str) -> Result<&'a mut ConfigValue> {
    for part in path.split('.') {
        value = match value {
            ConfigValue::Named(_) => value.named_mut(part).with_context(|| format!("no {part} in {path}"))?,
            ConfigValue::Indexed(items) => {
                let i: usize = part.parse().with_context(|| format!("{part} of {path} is no index"))?;
                items.get_mut(i).with_context(|| format!("no element {i} in {path}"))?
            }
            _ => bail!("{path}: {part} is below a leaf"),
        };
    }
    Ok(value)
}

fn holds(value: &ConfigValue, was: &serde_json::Value) -> bool {
    match (value, was) {
        (ConfigValue::Integer(v), serde_json::Value::Number(n)) => n.as_f64() == Some(f64::from(*v)),
        (ConfigValue::Float(v), serde_json::Value::Number(n)) => n.as_f64().is_some_and(|n| (n as f32) == *v),
        (ConfigValue::Boolean(v), serde_json::Value::Bool(b)) => v == b,
        _ => false,
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(args.len() == 2, "usage: PACK_DIR PLAN.json");
    let files = Path::new(&args[0]).join("files");
    let plan: Plan = serde_json::from_reader(std::fs::File::open(&args[1])?)?;
    for (path, edits) in &plan.projects {
        let file = files.join(path);
        let mut project = BinaryConfig::parse(&std::fs::read(&file).with_context(|| file.display().to_string())?)?;
        let Some(ConfigValue::Indexed(objects)) = project.root.named_mut("client_objects") else {
            bail!("{path}: no client_objects");
        };
        for edit in edits {
            let found: Vec<usize> = objects
                .iter()
                .enumerate()
                .filter(|(_, o)| matches!(o.named("full_name"), Some(ConfigValue::String(n)) if *n == edit.object))
                .map(|(i, _)| i)
                .collect();
            ensure!(found == [edit.index], "{path}: {} is at {found:?}, not [{}]", edit.object, edit.index);
            let object = &mut objects[edit.index];
            for (leaf, value) in &edit.set {
                let new = match (value.i32, value.f32, value.bool) {
                    (Some(v), None, None) => ConfigValue::Integer(v),
                    (None, Some(v), None) => ConfigValue::Float(v),
                    (None, None, Some(v)) => ConfigValue::Boolean(v),
                    _ => bail!("{path}: {}.{leaf}: name exactly one of i32, f32, bool", edit.object),
                };
                let slot = leaf_mut(object, leaf).with_context(|| format!("{path}: {}", edit.object))?;
                ensure!(holds(slot, &value.was), "{path}: {}.{leaf} is {slot:?}, not {}", edit.object, value.was);
                ensure!(*slot != new, "{path}: {}.{leaf} is already {new:?}", edit.object);
                println!("{path}: client_objects[{}] {}.{leaf}: {slot:?} -> {new:?}", edit.index, edit.object);
                *slot = new;
            }
        }
        std::fs::write(&file, project.to_bytes()?)?;
    }
    Ok(())
}
