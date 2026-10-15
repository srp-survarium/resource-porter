// SPDX-License-Identifier: GPL-3.0-or-later
//! JSON <-> binary-config value mapping, path access and field-level diffs.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Number, Value as Json};

use crate::binary_config::{ConfigValue, NamedValue};

/// Convert a manifest JSON value to a config value.
///
/// Plain JSON maps by shape: bool, integer literal (`1`), float literal
/// (`1.0`), string, array. An integer literal written where the existing
/// field is a float becomes a float, and a 2-4 number array written where the
/// existing field is a float vector becomes that vector. Tagged objects force
/// a type: `{"int": 1}`, `{"float": 1}`, `{"bool": true}`, `{"string": ""}`,
/// `{"float2"|"float3"|"float4": [..]}`, `{"array": [..]}`, and
/// `{"table": {"name": value}}` for a named table.
pub fn to_config(value: &Json, hint: Option<&ConfigValue>) -> Result<ConfigValue> {
    Ok(match value {
        Json::Bool(value) => ConfigValue::Boolean(*value),
        Json::Number(number) => number_value(number, hint)?,
        Json::String(value) => ConfigValue::String(value.clone()),
        Json::Array(values) => {
            if let Some(vector) = vector_from_hint(values, hint)? {
                return Ok(vector);
            }
            array_value(values, hint)?
        }
        Json::Object(object) => tagged_value(object, hint)?,
        Json::Null => bail!("config values cannot be null"),
    })
}

/// Convert a table of JSON fields (an entry object) to a named config table.
pub fn table_to_config(
    object: &Map<String, Json>,
    hint: Option<&ConfigValue>,
) -> Result<ConfigValue> {
    let mut values = Vec::with_capacity(object.len());
    for (name, value) in object {
        let child_hint = hint.and_then(|hint| hint.named(name));
        values.push(NamedValue {
            name: name.clone(),
            value: to_config(value, child_hint).with_context(|| format!("field {name}"))?,
        });
    }
    Ok(ConfigValue::Named(values))
}

fn number_value(number: &Number, hint: Option<&ConfigValue>) -> Result<ConfigValue> {
    if let Some(integer) = number.as_i64() {
        if matches!(hint, Some(ConfigValue::Float(_))) {
            return Ok(ConfigValue::Float(integer as f32));
        }
        return Ok(ConfigValue::Integer(
            i32::try_from(integer).context("integer does not fit i32")?,
        ));
    }
    if number.is_u64() {
        bail!("integer does not fit i32: {number}");
    }
    Ok(ConfigValue::Float(
        number
            .as_f64()
            .context("number cannot be represented as f64")? as f32,
    ))
}

fn vector_from_hint(values: &[Json], hint: Option<&ConfigValue>) -> Result<Option<ConfigValue>> {
    let expected = match hint {
        Some(ConfigValue::Float2(_)) => 2,
        Some(ConfigValue::Float3(_)) => 3,
        Some(ConfigValue::Float4(_)) => 4,
        _ => return Ok(None),
    };
    if values.len() != expected || !values.iter().all(Json::is_number) {
        return Ok(None);
    }
    Ok(Some(vector(values, expected)?))
}

fn vector(values: &[Json], width: usize) -> Result<ConfigValue> {
    if values.len() != width {
        bail!("float{width} needs {width} numbers, got {}", values.len());
    }
    let floats: Vec<f32> = values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|value| value as f32)
                .context("float vector component is not a number")
        })
        .collect::<Result<_>>()?;
    Ok(match width {
        2 => ConfigValue::Float2([floats[0], floats[1]]),
        3 => ConfigValue::Float3([floats[0], floats[1], floats[2]]),
        _ => ConfigValue::Float4([floats[0], floats[1], floats[2], floats[3]]),
    })
}

fn array_value(values: &[Json], hint: Option<&ConfigValue>) -> Result<ConfigValue> {
    let hints: &[ConfigValue] = match hint {
        Some(ConfigValue::Indexed(values)) => values,
        _ => &[],
    };
    let mut output = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let child_hint = hints.get(index).or_else(|| hints.first());
        output.push(to_config(value, child_hint).with_context(|| format!("array index {index}"))?);
    }
    Ok(ConfigValue::Indexed(output))
}

fn tagged_value(object: &Map<String, Json>, hint: Option<&ConfigValue>) -> Result<ConfigValue> {
    let mut entries = object.iter();
    let (Some((tag, value)), None) = (entries.next(), entries.next()) else {
        bail!(
            "config objects must be a single type tag (int, float, bool, string, float2, \
             float3, float4, array, table); named tables are written as {{\"table\": {{..}}}}"
        );
    };
    Ok(match (tag.as_str(), value) {
        ("int", Json::Number(number)) => ConfigValue::Integer(
            number
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .context("int tag needs an i32 integer")?,
        ),
        ("float", Json::Number(number)) => {
            ConfigValue::Float(number.as_f64().context("float tag needs a number")? as f32)
        }
        ("bool", Json::Bool(value)) => ConfigValue::Boolean(*value),
        ("string", Json::String(value)) => ConfigValue::String(value.clone()),
        ("float2", Json::Array(values)) => vector(values, 2)?,
        ("float3", Json::Array(values)) => vector(values, 3)?,
        ("float4", Json::Array(values)) => vector(values, 4)?,
        ("array", Json::Array(values)) => array_value(values, hint)?,
        ("table", Json::Object(fields)) => table_to_config(fields, hint)?,
        _ => bail!("unsupported or mistyped config value tag {tag:?}"),
    })
}

/// Canonical JSON for a config value; [`to_config`] reads it back exactly
/// without a hint.
pub fn from_config(value: &ConfigValue) -> Result<Json> {
    Ok(match value {
        ConfigValue::Boolean(value) => Json::Bool(*value),
        ConfigValue::Integer(value) => Json::from(*value),
        ConfigValue::Float(value) => float_json(*value)?,
        ConfigValue::String(value) => Json::String(value.clone()),
        ConfigValue::Float2(values) => tagged("float2", floats_json(values)?),
        ConfigValue::Float3(values) => tagged("float3", floats_json(values)?),
        ConfigValue::Float4(values) => tagged("float4", floats_json(values)?),
        ConfigValue::Indexed(values) => {
            Json::Array(values.iter().map(from_config).collect::<Result<_>>()?)
        }
        ConfigValue::Named(_) => tagged("table", Json::Object(table_from_config(value)?)),
    })
}

/// Fields of a named table as a JSON object (an entry object).
pub fn table_from_config(value: &ConfigValue) -> Result<Map<String, Json>> {
    let ConfigValue::Named(values) = value else {
        bail!("value is not a named table");
    };
    let mut object = Map::new();
    for entry in values {
        if object
            .insert(entry.name.clone(), from_config(&entry.value)?)
            .is_some()
        {
            bail!("named table repeats key {:?}", entry.name);
        }
    }
    Ok(object)
}

fn tagged(tag: &str, value: Json) -> Json {
    let mut object = Map::new();
    object.insert(tag.to_owned(), value);
    Json::Object(object)
}

fn floats_json(values: &[f32]) -> Result<Json> {
    Ok(Json::Array(
        values
            .iter()
            .map(|value| float_json(*value))
            .collect::<Result<_>>()?,
    ))
}

/// The shortest decimal that reads back to the same f32, always written with
/// a fraction so it is not mistaken for an integer.
fn float_json(value: f32) -> Result<Json> {
    let shortest: f64 = value.to_string().parse()?;
    Number::from_f64(shortest)
        .map(Json::Number)
        .with_context(|| format!("non-finite float {value} cannot be written to a pack"))
}

/// Order-insensitive equality: named tables are encoded in key-CRC order, so
/// their in-memory order carries no meaning.
pub fn semantic_eq(left: &ConfigValue, right: &ConfigValue) -> bool {
    match (left, right) {
        (ConfigValue::Named(left), ConfigValue::Named(right)) => {
            left.len() == right.len()
                && left.iter().all(|entry| {
                    right
                        .iter()
                        .find(|other| other.name == entry.name)
                        .is_some_and(|other| semantic_eq(&entry.value, &other.value))
                })
        }
        (ConfigValue::Indexed(left), ConfigValue::Indexed(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| semantic_eq(left, right))
        }
        (ConfigValue::Float(left), ConfigValue::Float(right)) => left.to_bits() == right.to_bits(),
        _ => left == right,
    }
}

pub fn split_path(path: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.iter().any(|part| part.is_empty()) {
        bail!("config path has an empty component: {path:?}");
    }
    Ok(parts)
}

pub fn value_at<'a>(root: &'a ConfigValue, path: &str) -> Option<&'a ConfigValue> {
    let mut current = root;
    for part in path.split('.') {
        current = child(current, part)?;
    }
    Some(current)
}

pub fn value_at_mut<'a>(root: &'a mut ConfigValue, path: &str) -> Option<&'a mut ConfigValue> {
    let mut current = root;
    for part in path.split('.') {
        current = match current {
            ConfigValue::Named(values) => values
                .iter_mut()
                .find(|entry| entry.name == part)
                .map(|entry| &mut entry.value)?,
            ConfigValue::Indexed(values) => values.get_mut(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn child<'a>(value: &'a ConfigValue, part: &str) -> Option<&'a ConfigValue> {
    match value {
        ConfigValue::Named(values) => values
            .iter()
            .find(|entry| entry.name == part)
            .map(|entry| &entry.value),
        ConfigValue::Indexed(values) => values.get(part.parse::<usize>().ok()?),
        _ => None,
    }
}

/// True when one dotted path is the other or lies inside it.
pub fn paths_overlap(left: &str, right: &str) -> bool {
    let inside = |outer: &str, inner: &str| {
        inner == outer || (inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'.')
    };
    inside(left, right) || inside(right, left)
}

/// Field-level edits that turn `base` into `target`.
#[derive(Debug, Default)]
pub struct ConfigDiff {
    pub sets: Vec<(String, ConfigValue)>,
    pub appends: Vec<(String, Vec<ConfigValue>)>,
}

/// Diff two configs. Returns `None` when the change is not field-level
/// (a named key was removed, a key repeats, or the root changed type).
pub fn diff(base: &ConfigValue, target: &ConfigValue) -> Option<ConfigDiff> {
    let mut output = ConfigDiff::default();
    match (base, target) {
        (ConfigValue::Named(_), ConfigValue::Named(_)) => {
            diff_into(base, target, "", &mut output)?;
            Some(output)
        }
        _ => None,
    }
}

fn join(prefix: &str, part: &str) -> String {
    if prefix.is_empty() {
        part.to_owned()
    } else {
        format!("{prefix}.{part}")
    }
}

fn diff_into(
    base: &ConfigValue,
    target: &ConfigValue,
    path: &str,
    out: &mut ConfigDiff,
) -> Option<()> {
    if semantic_eq(base, target) {
        return Some(());
    }
    match (base, target) {
        (ConfigValue::Named(base_values), ConfigValue::Named(target_values)) => {
            if has_duplicate_names(base_values) || has_duplicate_names(target_values) {
                return None;
            }
            if base_values
                .iter()
                .any(|entry| !target_values.iter().any(|other| other.name == entry.name))
            {
                // Removing a key cannot be expressed as a field set.
                return None;
            }
            if base_values.iter().any(|entry| entry.name.contains('.')) {
                return None;
            }
            for entry in target_values {
                if entry.name.contains('.') {
                    return None;
                }
                let child_path = join(path, &entry.name);
                match base_values.iter().find(|other| other.name == entry.name) {
                    Some(base_entry) => {
                        diff_into(&base_entry.value, &entry.value, &child_path, out)?
                    }
                    None => out.sets.push((child_path, entry.value.clone())),
                }
            }
            Some(())
        }
        (ConfigValue::Indexed(base_values), ConfigValue::Indexed(target_values))
            if target_values.len() >= base_values.len() && !path.is_empty() =>
        {
            for (index, (base_value, target_value)) in
                base_values.iter().zip(target_values).enumerate()
            {
                diff_into(
                    base_value,
                    target_value,
                    &join(path, &index.to_string()),
                    out,
                )?;
            }
            if target_values.len() > base_values.len() {
                out.appends
                    .push((path.to_owned(), target_values[base_values.len()..].to_vec()));
            }
            Some(())
        }
        _ if path.is_empty() => None,
        _ => {
            out.sets.push((path.to_owned(), target.clone()));
            Some(())
        }
    }
}

fn has_duplicate_names(values: &[NamedValue]) -> bool {
    let mut names: Vec<&str> = values.iter().map(|entry| entry.name.as_str()).collect();
    names.sort_unstable();
    names.windows(2).any(|pair| pair[0] == pair[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_round_trips_every_type() {
        let value = ConfigValue::Named(vec![
            NamedValue {
                name: "b".into(),
                value: ConfigValue::Boolean(true),
            },
            NamedValue {
                name: "i".into(),
                value: ConfigValue::Integer(-3),
            },
            NamedValue {
                name: "f".into(),
                value: ConfigValue::Float(27.0),
            },
            NamedValue {
                name: "g".into(),
                value: ConfigValue::Float(-0.05),
            },
            NamedValue {
                name: "s".into(),
                value: ConfigValue::String("Долг".into()),
            },
            NamedValue {
                name: "v".into(),
                value: ConfigValue::Float3([1.0, 0.5, -2.0]),
            },
            NamedValue {
                name: "a".into(),
                value: ConfigValue::Indexed(vec![ConfigValue::Integer(1), ConfigValue::Float(2.5)]),
            },
        ]);
        let json = from_config(&value).unwrap();
        let text = serde_json::to_string(&json).unwrap();
        let reread: Json = serde_json::from_str(&text).unwrap();
        assert!(semantic_eq(&to_config(&reread, None).unwrap(), &value));
    }

    #[test]
    fn integer_literals_follow_the_existing_float_type() {
        let hint = ConfigValue::Float(1.5);
        assert_eq!(
            to_config(&json!(80), Some(&hint)).unwrap(),
            ConfigValue::Float(80.0)
        );
        assert_eq!(
            to_config(&json!(80), None).unwrap(),
            ConfigValue::Integer(80)
        );
        let vector = ConfigValue::Float3([0.0; 3]);
        assert_eq!(
            to_config(&json!([1, 2, 3]), Some(&vector)).unwrap(),
            ConfigValue::Float3([1.0, 2.0, 3.0])
        );
        assert!(to_config(&json!({"a": 1, "b": 2}), None).is_err());
    }

    #[test]
    fn diff_reports_sets_and_appends_but_not_removals() {
        let base = to_config(
            &json!({"table": {"x": 1, "list": [1, 2], "n": {"table": {"k": "a"}}}}),
            None,
        )
        .unwrap();
        let target = to_config(
            &json!({"table": {"x": 2, "list": [1, 2, 3], "n": {"table": {"k": "a", "new": true}}}}),
            None,
        )
        .unwrap();
        let output = diff(&base, &target).unwrap();
        assert_eq!(output.sets.len(), 2);
        assert_eq!(
            output.appends,
            vec![("list".to_owned(), vec![ConfigValue::Integer(3)])]
        );
        let removed = to_config(&json!({"table": {"x": 1}}), None).unwrap();
        assert!(diff(&base, &removed).is_none());
    }

    #[test]
    fn overlapping_paths_are_detected() {
        assert!(paths_overlap("a.b", "a.b.c"));
        assert!(paths_overlap("a.b", "a.b"));
        assert!(!paths_overlap("a.b", "a.bc"));
    }
}
