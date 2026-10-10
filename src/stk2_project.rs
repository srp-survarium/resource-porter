// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectObject {
    pub id: String,
    pub kind: String,
    pub library_name: Option<String>,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    /// Flat, top-level editor properties retained for target-schema
    /// projection. Nested editor-only tables are deliberately not included.
    pub properties: BTreeMap<String, ProjectProperty>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectProperty {
    Boolean(bool),
    Integer(i32),
    Float(f32),
    String(String),
    Float3([f32; 3]),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProjectParseReport {
    pub total_objects: usize,
    pub object_kinds: BTreeMap<String, usize>,
    pub incomplete_objects: Vec<String>,
    pub composite_instances: usize,
    pub composite_children: usize,
    pub composite_child_kinds: BTreeMap<String, usize>,
    pub omitted_composite_local_entries: BTreeMap<String, usize>,
    pub resolved_composite_children: usize,
    pub unresolved_composite_children: Vec<String>,
}

#[derive(Clone, Debug)]
struct CompositeChild {
    kind: String,
    library_name: Option<String>,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 3]>,
    scale: [f32; 3],
}

type CompositeLibrary = BTreeMap<(String, String), CompositeChild>;

pub fn parse_project(path: &Path) -> Result<(Vec<ProjectObject>, ProjectParseReport)> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading STK2 source project {}", path.display()))?;
    parse_project_text(&text, None)
        .with_context(|| format!("parsing STK2 source project {}", path.display()))
}

pub fn parse_project_with_composites(
    path: &Path,
    composite_library_path: &Path,
) -> Result<(Vec<ProjectObject>, ProjectParseReport)> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading STK2 source project {}", path.display()))?;
    let library_text = fs::read_to_string(composite_library_path).with_context(|| {
        format!(
            "reading STK2 composite library {}",
            composite_library_path.display()
        )
    })?;
    let library = parse_composite_library(&library_text).with_context(|| {
        format!(
            "parsing STK2 composite library {}",
            composite_library_path.display()
        )
    })?;
    parse_project_text(&text, Some(&library))
        .with_context(|| format!("parsing STK2 source project {}", path.display()))
}

fn parse_project_text(
    text: &str,
    composite_library: Option<&CompositeLibrary>,
) -> Result<(Vec<ProjectObject>, ProjectParseReport)> {
    let lines: Vec<&str> = text.lines().collect();
    let objects_start = lines
        .iter()
        .position(|line| line.trim_end() == "objects = {")
        .context("source project has no top-level objects table")?;
    let mut objects = Vec::new();
    let mut report = ProjectParseReport::default();
    let mut index = objects_start + 1;

    while index < lines.len() {
        let line = lines[index];
        if line == "} -- objects" {
            break;
        }
        let Some(id) = table_entry_key(line, 1) else {
            index += 1;
            continue;
        };
        let end = find_entry_end(&lines, index + 1, 1)
            .with_context(|| format!("top-level object {id:?} has no closing line"))?;
        let block = &lines[index + 1..end];
        parse_project_object(&id, block, composite_library, &mut objects, &mut report)?;
        index = end + 1;
    }
    Ok((objects, report))
}

fn parse_project_object(
    id: &str,
    block: &[&str],
    composite_library: Option<&CompositeLibrary>,
    objects: &mut Vec<ProjectObject>,
    report: &mut ProjectParseReport,
) -> Result<()> {
    report.total_objects += 1;
    let partial = parse_fields(block, 2)?;
    let kind = partial.kind.unwrap_or_else(|| "<missing>".into());
    *report.object_kinds.entry(kind.clone()).or_default() += 1;

    if kind != "composite" {
        match (partial.position, partial.rotation, partial.scale) {
            (Some(position), Some(rotation), Some(scale)) => objects.push(ProjectObject {
                id: id.to_owned(),
                kind,
                library_name: partial.library_name,
                position,
                rotation,
                scale,
                properties: partial.properties,
            }),
            _ => report.incomplete_objects.push(id.to_owned()),
        }
        return Ok(());
    }

    report.composite_instances += 1;
    let Some(library) = composite_library else {
        report
            .unresolved_composite_children
            .push(format!("{id}: composite library was not supplied"));
        return Ok(());
    };
    let children_start = block
        .iter()
        .position(|line| *line == "\t\tobjects = {")
        .context("composite object has no objects table")?;
    let mut index = children_start + 1;
    while index < block.len() {
        if block[index] == "\t\t}, -- objects" {
            break;
        }
        let Some(child_name) = table_entry_key(block[index], 3) else {
            index += 1;
            continue;
        };
        let end = find_entry_end(block, index + 1, 3)
            .with_context(|| format!("composite child {id}/{child_name} has no closing line"))?;
        let child_block = &block[index + 1..end];
        let child_fields = parse_fields(child_block, 4)?;
        let reference = child_block
            .iter()
            .find_map(|line| parse_composite_reference(line.trim()));
        if reference.is_none() {
            *report
                .omitted_composite_local_entries
                .entry(child_name.clone())
                .or_default() += 1;
            index = end + 1;
            continue;
        }
        report.composite_children += 1;
        let resolved = reference
            .as_ref()
            .and_then(|key| library.get(key))
            .or_else(|| {
                partial
                    .library_guid
                    .as_ref()
                    .and_then(|guid| library.get(&(guid.clone(), child_name.clone())))
            });
        let Some(resolved) = resolved else {
            report.unresolved_composite_children.push(format!(
                "{id}/{child_name}: unresolved composite supertable"
            ));
            index = end + 1;
            continue;
        };
        report.resolved_composite_children += 1;
        *report
            .composite_child_kinds
            .entry(resolved.kind.clone())
            .or_default() += 1;
        if resolved.kind != "solid_visual" {
            index = end + 1;
            continue;
        }
        let position = child_fields.position.or(resolved.position);
        let rotation = child_fields.rotation.or(resolved.rotation);
        let (Some(position), Some(rotation)) = (position, rotation) else {
            report
                .unresolved_composite_children
                .push(format!("{id}/{child_name}: missing saved world transform"));
            index = end + 1;
            continue;
        };
        objects.push(ProjectObject {
            id: format!("{id}/{child_name}"),
            kind: resolved.kind.clone(),
            library_name: resolved.library_name.clone(),
            position,
            rotation,
            scale: child_fields.scale.unwrap_or(resolved.scale),
            properties: child_fields.properties,
        });
        index = end + 1;
    }
    Ok(())
}

fn parse_composite_library(text: &str) -> Result<CompositeLibrary> {
    let lines: Vec<&str> = text.lines().collect();
    let library_start = lines
        .iter()
        .position(|line| line.trim_end() == "library = {")
        .context("composite library has no library table")?;
    let mut library = CompositeLibrary::new();
    let mut index = library_start + 1;
    while index < lines.len() {
        if lines[index] == "} -- library" {
            break;
        }
        let Some(guid) = table_entry_key(lines[index], 1) else {
            index += 1;
            continue;
        };
        let guid_end = find_entry_end(&lines, index + 1, 1)
            .with_context(|| format!("composite library entry {guid:?} has no closing line"))?;
        let block = &lines[index + 1..guid_end];
        let Some(children_start) = block.iter().position(|line| *line == "\t\tobjects = {") else {
            index = guid_end + 1;
            continue;
        };
        let mut child_index = children_start + 1;
        while child_index < block.len() {
            if block[child_index] == "\t\t}, -- objects" {
                break;
            }
            let Some(child_name) = table_entry_key(block[child_index], 3) else {
                child_index += 1;
                continue;
            };
            let child_end = find_entry_end(block, child_index + 1, 3).with_context(|| {
                format!("composite library child {guid}/{child_name} has no closing line")
            })?;
            let fields = parse_fields(&block[child_index + 1..child_end], 4)?;
            if let (Some(kind), Some(scale)) = (fields.kind, fields.scale) {
                let key = (guid.clone(), child_name.clone());
                if library
                    .insert(
                        key.clone(),
                        CompositeChild {
                            kind,
                            library_name: fields.library_name,
                            position: fields.position,
                            rotation: fields.rotation,
                            scale,
                        },
                    )
                    .is_some()
                {
                    bail!("duplicate composite library child {}/{}", key.0, key.1);
                }
            }
            child_index = child_end + 1;
        }
        index = guid_end + 1;
    }
    if library.is_empty() {
        bail!("composite library contains no complete child objects");
    }
    Ok(library)
}

fn find_entry_end(lines: &[&str], start: usize, indent: usize) -> Option<usize> {
    let prefix = "\t".repeat(indent);
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| {
            (line.starts_with(&format!("{prefix}}}")) && !line.starts_with(&format!("{prefix}\t")))
                .then_some(index)
        })
}

fn table_entry_key(line: &str, indent: usize) -> Option<String> {
    let line = line.strip_prefix(&"\t".repeat(indent))?;
    if line.starts_with('\t') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("[\"") {
        return Some(rest.split_once("\"] = ")?.0.to_owned());
    }
    let (name, value) = line.split_once(" = ")?;
    (value == "{" || value == "create_property_table ( {").then(|| name.to_owned())
}

fn parse_composite_reference(line: &str) -> Option<(String, String)> {
    let library = line.split_once("library[\\\"")?.1;
    let (guid, rest) = library.split_once("\\\"]")?;
    let object = rest.split_once(".objects")?.1;
    let child = if let Some(object) = object.strip_prefix("[\\\"") {
        object.split_once("\\\"]")?.0
    } else {
        object
            .strip_prefix('.')?
            .strip_suffix("\",")
            .or_else(|| object.strip_prefix('.')?.strip_suffix('"'))?
    };
    Some((guid.to_owned(), child.to_owned()))
}

fn parse_fields(lines: &[&str], indent: usize) -> Result<PartialObject> {
    let prefix = "\t".repeat(indent);
    let mut partial = PartialObject::default();
    for (line_index, line) in lines.iter().enumerate() {
        let Some(field) = line.strip_prefix(&prefix) else {
            continue;
        };
        if field.starts_with('\t') {
            continue;
        }
        if let Some(value) = quoted_assignment(field, "game_object_type") {
            partial.kind = Some(value.to_owned());
        } else if let Some(value) = quoted_assignment(field, "lib_name") {
            partial.library_name = Some(value.to_owned());
        } else if let Some(value) = quoted_assignment(field, "lib_guid") {
            partial.library_guid = Some(value.to_owned());
        } else if let Some(value) = float3_assignment(field, "position")? {
            partial.position = Some(value);
        } else if let Some(value) = float3_assignment(field, "rotation")? {
            partial.rotation = Some(value);
        } else if let Some(value) = float3_assignment(field, "scale")? {
            partial.scale = Some(value);
        } else if field.starts_with("position = float3(")
            || field.starts_with("rotation = float3(")
            || field.starts_with("scale = float3(")
        {
            bail!(
                "malformed float3 in block at relative line {}",
                line_index + 1
            );
        }
        if let Some((name, value)) = scalar_assignment(field)? {
            if partial.properties.insert(name.clone(), value).is_some() {
                bail!("duplicate scalar property {name:?} in project object");
            }
        }
    }
    Ok(partial)
}

#[derive(Default)]
struct PartialObject {
    kind: Option<String>,
    library_name: Option<String>,
    library_guid: Option<String>,
    position: Option<[f32; 3]>,
    rotation: Option<[f32; 3]>,
    scale: Option<[f32; 3]>,
    properties: BTreeMap<String, ProjectProperty>,
}

fn quoted_assignment<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.strip_prefix(&format!("{name} = \""))?
        .strip_suffix("\",")
}

fn float3_assignment(line: &str, name: &str) -> Result<Option<[f32; 3]>> {
    let Some(value) = line.strip_prefix(&format!("{name} = float3(")) else {
        return Ok(None);
    };
    let value = value
        .trim()
        .strip_suffix("),")
        .context("float3 assignment has no closing parenthesis")?
        .trim();
    let components = value
        .split(',')
        .map(|component| {
            component
                .trim()
                .parse::<f32>()
                .with_context(|| format!("invalid float3 component {component:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if components.len() != 3 || !components.iter().all(|value| value.is_finite()) {
        bail!("float3 assignment must contain three finite values");
    }
    Ok(Some([components[0], components[1], components[2]]))
}

fn scalar_assignment(line: &str) -> Result<Option<(String, ProjectProperty)>> {
    let Some((name, raw_value)) = line.split_once(" = ") else {
        return Ok(None);
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Ok(None);
    }
    let raw_value = raw_value.strip_suffix(',').unwrap_or(raw_value).trim();
    if raw_value == "{" || raw_value.starts_with("create_property_table") {
        return Ok(None);
    }
    let value = if let Some(value) = raw_value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        ProjectProperty::String(value.to_owned())
    } else if raw_value == "true" {
        ProjectProperty::Boolean(true)
    } else if raw_value == "false" {
        ProjectProperty::Boolean(false)
    } else if raw_value.starts_with("float3(") {
        ProjectProperty::Float3(
            float3_assignment(line, name)?
                .with_context(|| format!("malformed float3 property {name:?}"))?,
        )
    } else if !raw_value.contains(['.', 'e', 'E']) {
        let Ok(value) = raw_value.parse::<i32>() else {
            return Ok(None);
        };
        ProjectProperty::Integer(value)
    } else {
        let Ok(value) = raw_value.parse::<f32>() else {
            return Ok(None);
        };
        if !value.is_finite() {
            bail!("project property {name:?} is not finite");
        }
        ProjectProperty::Float(value)
    };
    Ok(Some((name.to_owned(), value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_top_level_object_fields() {
        let source = r#"camera = {}

objects = {
	["one"] = {
		game_object_type = "solid_visual",
		lib_name = "props/box",
		position = float3( 1, 2, 3 ),
		properties = {
			position = float3( 90, 90, 90 ),
		},
		rotation = float3( 0, 0.5, 0 ),
		scale = float3( 1, 1, 1 ),
	}, -- one

	["two"] = {
		game_object_type = "light",
		position = float3( 4, 5, 6 ),
		rotation = float3( 0, 0, 0 ),
		scale = float3( 1, 1, 1 ),
	}, -- two
} -- objects
"#;
        let (objects, report) = parse_project_text(source, None).unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].library_name.as_deref(), Some("props/box"));
        assert_eq!(objects[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(
            objects[1].properties["position"],
            ProjectProperty::Float3([4.0, 5.0, 6.0])
        );
        assert_eq!(
            objects[1].properties["game_object_type"],
            ProjectProperty::String("light".into())
        );
        assert_eq!(report.object_kinds["solid_visual"], 1);
        assert_eq!(report.object_kinds["light"], 1);
    }

    #[test]
    fn resolves_composite_child_inherited_fields() {
        let source = r#"objects = {
	["parent"] = {
		game_object_type = "composite",
		lib_guid = "guid",
		objects = {
			["child"] = create_property_table ( {
				__super_tables = {
					"modules['library/composite/composite.library'].library[\"guid\"].objects[\"child\"]",
				}, -- __super_tables
				position = float3( 10, 20, 30 ),
				rotation = float3( 1, 2, 3 ),
			}, this ), -- child
		}, -- objects
		position = float3( 99, 99, 99 ),
		rotation = float3( 0, 0, 0 ),
		scale = float3( 1, 1, 1 ),
	}, -- parent
} -- objects
"#;
        let mut library = CompositeLibrary::new();
        library.insert(
            ("guid".into(), "child".into()),
            CompositeChild {
                kind: "solid_visual".into(),
                library_name: Some("props/box".into()),
                position: Some([100.0, 100.0, 100.0]),
                rotation: Some([0.0, 0.0, 0.0]),
                scale: [2.0, 2.0, 2.0],
            },
        );
        let (objects, report) = parse_project_text(source, Some(&library)).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, "parent/child");
        assert_eq!(objects[0].position, [10.0, 20.0, 30.0]);
        assert_eq!(objects[0].scale, [2.0, 2.0, 2.0]);
        assert_eq!(report.composite_children, 1);
        assert_eq!(report.resolved_composite_children, 1);
        assert_eq!(report.composite_child_kinds["solid_visual"], 1);
    }

    #[test]
    fn parses_both_composite_reference_key_syntaxes() {
        assert_eq!(
            parse_composite_reference(
                r#""modules['library/composite/composite.library'].library[\"guid\"].objects[\"quoted child\"]","#
            ),
            Some(("guid".into(), "quoted child".into()))
        );
        assert_eq!(
            parse_composite_reference(
                r#""modules['library/composite/composite.library'].library[\"guid\"].objects.simple_child","#
            ),
            Some(("guid".into(), "simple_child".into()))
        );
    }
}
