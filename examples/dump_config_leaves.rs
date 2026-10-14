//! Bulk-decode binary configs under one or more prefixes as JSON lines.
//! Usage: dump_config_leaves DB PREFIX... > out.jsonl
//! Each line: {"path": ..., "leaves": {"a.b[0].c": ["kind", value]}} or
//! {"path": ..., "error": ...} for payloads that are not binary configs.
use std::env;

use serde_json::{Map, Value, json};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

fn leaves(value: &ConfigValue, path: &str, out: &mut Map<String, Value>) {
    let child = |name: &str| {
        if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}.{name}")
        }
    };
    match value {
        ConfigValue::Named(values) => {
            if values.is_empty() {
                out.insert(path.to_owned(), json!(["named", 0]));
            }
            for entry in values {
                leaves(&entry.value, &child(&entry.name), out);
            }
        }
        ConfigValue::Indexed(values) => {
            if values.is_empty() {
                out.insert(path.to_owned(), json!(["indexed", 0]));
            }
            for (index, entry) in values.iter().enumerate() {
                leaves(entry, &format!("{path}[{index}]"), out);
            }
        }
        ConfigValue::Boolean(v) => {
            out.insert(path.to_owned(), json!(["bool", v]));
        }
        ConfigValue::Integer(v) => {
            out.insert(path.to_owned(), json!(["i32", v]));
        }
        ConfigValue::Float(v) => {
            out.insert(path.to_owned(), json!(["f32", v]));
        }
        ConfigValue::String(v) => {
            out.insert(path.to_owned(), json!(["string", v]));
        }
        ConfigValue::Float2(v) => {
            out.insert(path.to_owned(), json!(["float2", v]));
        }
        ConfigValue::Float3(v) => {
            out.insert(path.to_owned(), json!(["float3", v]));
        }
        ConfigValue::Float4(v) => {
            out.insert(path.to_owned(), json!(["float4", v]));
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let db = args.next().expect("usage: dump_config_leaves DB PREFIX...");
    let prefixes: Vec<String> = args.collect();
    let mut archive = ResourceDb::open(db)?;
    let paths: Vec<String> = prefixes
        .iter()
        .flat_map(|prefix| {
            archive
                .paths_with_prefix(prefix)
                .filter(|p| archive.is_file(p))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    for path in paths {
        let bytes = archive.read(&path)?;
        let line = match BinaryConfig::parse(&bytes) {
            Ok(config) => {
                let mut out = Map::new();
                leaves(&config.root, "", &mut out);
                json!({"path": path, "leaves": out})
            }
            Err(error) => json!({"path": path, "error": error.to_string(), "size": bytes.len()}),
        };
        println!("{line}");
    }
    Ok(())
}
