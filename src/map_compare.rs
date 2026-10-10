// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::dependency;
use crate::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ShapeStats {
    pub kinds: BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub indexed_lengths: BTreeSet<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectStructure {
    pub path: String,
    pub sha256: String,
    pub byte_size: usize,
    pub shapes: BTreeMap<String, ShapeStats>,
    pub sections: BTreeMap<String, SectionSummary>,
    pub resource_like_strings: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SectionSummary {
    pub kind: String,
    pub item_count: usize,
    pub string_values: BTreeMap<String, BTreeMap<String, usize>>,
    pub float3_ranges: BTreeMap<String, Float3Range>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Float3Range {
    pub count: usize,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectDifference {
    pub path: String,
    pub classification: String,
    pub target: Option<ShapeStats>,
    pub donor: Option<ShapeStats>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceNode {
    pub path: String,
    pub family: String,
    pub byte_size: usize,
    pub sha256: String,
    pub decoded_config: bool,
    pub schema_sha256: Option<String>,
    pub references: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ResourceGraph {
    pub roots: Vec<String>,
    pub nodes: BTreeMap<String, ResourceNode>,
    pub unresolved_references: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceDifference {
    pub path: String,
    pub family: String,
    pub classification: String,
    pub target_sha256: Option<String>,
    pub donor_sha256: Option<String>,
    pub target_schema_sha256: Option<String>,
    pub donor_schema_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FamilyDifference {
    pub family: String,
    pub classification: String,
    pub target_members: BTreeSet<String>,
    pub donor_members: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ComparisonCounts {
    pub project_shape_differences: usize,
    pub target_graph_nodes: usize,
    pub donor_graph_nodes: usize,
    pub identical_resources: usize,
    pub changed_same_schema_resources: usize,
    pub changed_schema_resources: usize,
    pub changed_opaque_resources: usize,
    pub target_only_resources: usize,
    pub donor_only_resources: usize,
    pub changed_resource_families: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MapStructureComparison {
    pub format_version: u32,
    pub target_project: ProjectStructure,
    pub donor_project: ProjectStructure,
    pub project_differences: Vec<ProjectDifference>,
    pub target_graph: ResourceGraph,
    pub donor_graph: ResourceGraph,
    pub resource_differences: Vec<ResourceDifference>,
    pub family_differences: Vec<FamilyDifference>,
    pub counts: ComparisonCounts,
}

pub fn compare(
    target_db_path: &Path,
    target_project_path: &str,
    donor_db_path: &Path,
    donor_project_path: &str,
) -> Result<MapStructureComparison> {
    let mut target_db = ResourceDb::open(target_db_path)?;
    let mut donor_db = ResourceDb::open(donor_db_path)?;

    let target_bytes = target_db
        .read(target_project_path)
        .with_context(|| format!("reading target project {target_project_path}"))?;
    let donor_bytes = donor_db
        .read(donor_project_path)
        .with_context(|| format!("reading donor project {donor_project_path}"))?;
    let target_project = inspect_project(target_project_path, &target_bytes)?;
    let donor_project = inspect_project(donor_project_path, &donor_bytes)?;
    let project_differences = compare_project_shapes(&target_project, &donor_project);

    let target_graph = build_resource_graph(&mut target_db, [target_project_path.to_owned()])?;
    let donor_graph = build_resource_graph(&mut donor_db, [donor_project_path.to_owned()])?;
    let resource_differences = compare_resources(&target_graph, &donor_graph);
    let family_differences = compare_families(&target_graph, &donor_graph);

    let mut counts = ComparisonCounts {
        project_shape_differences: project_differences.len(),
        target_graph_nodes: target_graph.nodes.len(),
        donor_graph_nodes: donor_graph.nodes.len(),
        changed_resource_families: family_differences.len(),
        ..ComparisonCounts::default()
    };
    for difference in &resource_differences {
        match difference.classification.as_str() {
            "identical" => counts.identical_resources += 1,
            "changed_same_schema" => counts.changed_same_schema_resources += 1,
            "changed_schema" => counts.changed_schema_resources += 1,
            "changed_opaque" => counts.changed_opaque_resources += 1,
            "target_only" => counts.target_only_resources += 1,
            "donor_only" => counts.donor_only_resources += 1,
            _ => {}
        }
    }

    Ok(MapStructureComparison {
        format_version: 1,
        target_project,
        donor_project,
        project_differences,
        target_graph,
        donor_graph,
        resource_differences,
        family_differences,
        counts,
    })
}

pub fn write_report(report: &MapStructureComparison, output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    serde_json::to_writer_pretty(
        File::create(output).with_context(|| format!("creating {}", output.display()))?,
        report,
    )?;
    Ok(())
}

fn inspect_project(path: &str, bytes: &[u8]) -> Result<ProjectStructure> {
    let config = BinaryConfig::parse(bytes).with_context(|| format!("parsing project {path}"))?;
    let mut shapes = BTreeMap::new();
    collect_shapes(&config.root, "$", &mut shapes);
    let mut strings = Vec::new();
    config.root.collect_strings(&mut strings);
    let sections = inspect_sections(&config.root);
    let mut resource_like_strings = BTreeMap::new();
    for text in strings {
        if looks_resource_like(text) {
            *resource_like_strings.entry(text.to_owned()).or_default() += 1;
        }
    }
    Ok(ProjectStructure {
        path: path.to_owned(),
        sha256: sha256(bytes),
        byte_size: bytes.len(),
        shapes,
        sections,
        resource_like_strings,
    })
}

fn inspect_sections(root: &ConfigValue) -> BTreeMap<String, SectionSummary> {
    let ConfigValue::Named(fields) = root else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|field| {
            let mut summary = SectionSummary {
                kind: value_kind(&field.value).to_owned(),
                item_count: match &field.value {
                    ConfigValue::Indexed(values) => values.len(),
                    ConfigValue::Named(values) => values.len(),
                    _ => 1,
                },
                ..SectionSummary::default()
            };
            collect_section_values(
                &field.value,
                "$",
                &mut summary.string_values,
                &mut summary.float3_ranges,
            );
            (field.name.clone(), summary)
        })
        .collect()
}

fn collect_section_values(
    value: &ConfigValue,
    path: &str,
    strings: &mut BTreeMap<String, BTreeMap<String, usize>>,
    float3_ranges: &mut BTreeMap<String, Float3Range>,
) {
    match value {
        ConfigValue::String(text) => {
            *strings
                .entry(path.to_owned())
                .or_default()
                .entry(text.clone())
                .or_default() += 1;
        }
        ConfigValue::Float3(vector) if vector.iter().all(|value| value.is_finite()) => {
            let range = float3_ranges
                .entry(path.to_owned())
                .or_insert_with(|| Float3Range {
                    count: 0,
                    min: *vector,
                    max: *vector,
                });
            range.count += 1;
            for index in 0..3 {
                range.min[index] = range.min[index].min(vector[index]);
                range.max[index] = range.max[index].max(vector[index]);
            }
        }
        ConfigValue::Named(values) => {
            for child in values {
                collect_section_values(
                    &child.value,
                    &format!("{path}.{}", child.name),
                    strings,
                    float3_ranges,
                );
            }
        }
        ConfigValue::Indexed(values) => {
            for child in values {
                collect_section_values(child, &format!("{path}[]"), strings, float3_ranges);
            }
        }
        _ => {}
    }
}

fn collect_shapes(value: &ConfigValue, path: &str, output: &mut BTreeMap<String, ShapeStats>) {
    let kind = value_kind(value);
    let stats = output.entry(path.to_owned()).or_default();
    *stats.kinds.entry(kind.to_owned()).or_default() += 1;
    if let ConfigValue::Indexed(values) = value {
        stats.indexed_lengths.insert(values.len());
    }
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                collect_shapes(&child.value, &format!("{path}.{}", child.name), output);
            }
        }
        ConfigValue::Indexed(values) => {
            for child in values {
                collect_shapes(child, &format!("{path}[]"), output);
            }
        }
        _ => {}
    }
}

fn compare_project_shapes(
    target: &ProjectStructure,
    donor: &ProjectStructure,
) -> Vec<ProjectDifference> {
    let paths: BTreeSet<_> = target
        .shapes
        .keys()
        .chain(donor.shapes.keys())
        .cloned()
        .collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let left = target.shapes.get(&path);
            let right = donor.shapes.get(&path);
            if left == right {
                return None;
            }
            let classification = match (left, right) {
                (None, Some(_)) => "donor_only_shape",
                (Some(_), None) => "target_only_shape",
                (Some(left), Some(right))
                    if left.kinds.keys().collect::<BTreeSet<_>>()
                        != right.kinds.keys().collect::<BTreeSet<_>>() =>
                {
                    "type_or_shape_changed"
                }
                _ => "cardinality_changed",
            };
            Some(ProjectDifference {
                path,
                classification: classification.to_owned(),
                target: left.cloned(),
                donor: right.cloned(),
            })
        })
        .collect()
}

fn build_resource_graph(
    db: &mut ResourceDb,
    roots: impl IntoIterator<Item = String>,
) -> Result<ResourceGraph> {
    let roots: Vec<String> = roots.into_iter().collect();
    let mut graph = ResourceGraph {
        roots: roots.clone(),
        ..ResourceGraph::default()
    };
    let mut queue: VecDeque<String> = roots.into();
    let mut visited = BTreeSet::new();

    while let Some(path) = queue.pop_front() {
        if !visited.insert(path.clone()) {
            continue;
        }
        if !db.is_file(&path) {
            let prefix = format!("{}/", path.trim_end_matches('/'));
            let children: Vec<_> = db
                .paths_with_prefix(&prefix)
                .filter(|child| db.is_file(child))
                .map(ToOwned::to_owned)
                .collect();
            if children.is_empty() {
                graph.unresolved_references.insert(path);
            } else {
                queue.extend(children);
            }
            continue;
        }

        let bytes = db.read(&path)?;
        let parsed = BinaryConfig::parse(&bytes).ok();
        let mut references = BTreeSet::new();
        if let Some(config) = &parsed {
            let mut strings = Vec::new();
            config.root.collect_strings(&mut strings);
            for reference in strings {
                let candidates = dependency::resolve_candidates(reference, db, db);
                if candidates.is_empty() {
                    if looks_resource_like(reference) {
                        graph.unresolved_references.insert(reference.to_owned());
                    }
                } else {
                    references.extend(candidates);
                }
            }
        }
        queue.extend(references.iter().cloned());
        let schema_sha256 = parsed
            .as_ref()
            .map(|config| normalized_schema_sha256(&config.root));
        graph.nodes.insert(
            path.clone(),
            ResourceNode {
                family: resource_family(&path),
                path,
                byte_size: bytes.len(),
                sha256: sha256(&bytes),
                decoded_config: parsed.is_some(),
                schema_sha256,
                references,
            },
        );
    }
    Ok(graph)
}

fn compare_resources(target: &ResourceGraph, donor: &ResourceGraph) -> Vec<ResourceDifference> {
    let paths: BTreeSet<_> = target
        .nodes
        .keys()
        .chain(donor.nodes.keys())
        .cloned()
        .collect();
    paths
        .into_iter()
        .map(|path| {
            let left = target.nodes.get(&path);
            let right = donor.nodes.get(&path);
            let classification = match (left, right) {
                (Some(left), Some(right)) if left.sha256 == right.sha256 => "identical",
                (Some(left), Some(right))
                    if left.decoded_config
                        && right.decoded_config
                        && left.schema_sha256 == right.schema_sha256 =>
                {
                    "changed_same_schema"
                }
                (Some(left), Some(right)) if left.decoded_config && right.decoded_config => {
                    "changed_schema"
                }
                (Some(_), Some(_)) => "changed_opaque",
                (Some(_), None) => "target_only",
                (None, Some(_)) => "donor_only",
                (None, None) => unreachable!(),
            };
            ResourceDifference {
                family: left
                    .map(|node| node.family.clone())
                    .or_else(|| right.map(|node| node.family.clone()))
                    .unwrap(),
                path,
                classification: classification.to_owned(),
                target_sha256: left.map(|node| node.sha256.clone()),
                donor_sha256: right.map(|node| node.sha256.clone()),
                target_schema_sha256: left.and_then(|node| node.schema_sha256.clone()),
                donor_schema_sha256: right.and_then(|node| node.schema_sha256.clone()),
            }
        })
        .collect()
}

fn compare_families(target: &ResourceGraph, donor: &ResourceGraph) -> Vec<FamilyDifference> {
    let target_families = family_members(target);
    let donor_families = family_members(donor);
    let families: BTreeSet<_> = target_families
        .keys()
        .chain(donor_families.keys())
        .cloned()
        .collect();
    families
        .into_iter()
        .filter_map(|family| {
            let target_members = target_families.get(&family).cloned().unwrap_or_default();
            let donor_members = donor_families.get(&family).cloned().unwrap_or_default();
            if target_members == donor_members {
                return None;
            }
            let classification = match (target_members.is_empty(), donor_members.is_empty()) {
                (true, false) => "donor_only_family",
                (false, true) => "target_only_family",
                _ => "family_layout_changed",
            };
            Some(FamilyDifference {
                family,
                classification: classification.to_owned(),
                target_members,
                donor_members,
            })
        })
        .collect()
}

fn family_members(graph: &ResourceGraph) -> BTreeMap<String, BTreeSet<String>> {
    let mut output = BTreeMap::<String, BTreeSet<String>>::new();
    for node in graph.nodes.values() {
        output
            .entry(node.family.clone())
            .or_default()
            .insert(node.path.clone());
    }
    output
}

fn resource_family(path: &str) -> String {
    for marker in [".skinned_model", ".model"] {
        if let Some(index) = path.find(marker) {
            return path[..index + marker.len()].to_owned();
        }
    }
    if let Some(base) = path.strip_suffix(".options") {
        return base.to_owned();
    }
    if let Some(base) = path.strip_suffix(".dds") {
        return base.to_owned();
    }
    path.to_owned()
}

fn value_kind(value: &ConfigValue) -> &'static str {
    match value {
        ConfigValue::Boolean(_) => "boolean",
        ConfigValue::Integer(_) => "integer",
        ConfigValue::Float(_) => "float",
        ConfigValue::String(_) => "string",
        ConfigValue::Float2(_) => "float2",
        ConfigValue::Float3(_) => "float3",
        ConfigValue::Float4(_) => "float4",
        ConfigValue::Named(_) => "named",
        ConfigValue::Indexed(_) => "indexed",
    }
}

fn normalized_schema_sha256(root: &ConfigValue) -> String {
    let mut shapes = BTreeMap::new();
    collect_shapes(root, "$", &mut shapes);
    let normalized: BTreeMap<_, BTreeSet<_>> = shapes
        .into_iter()
        .map(|(path, stats)| (path, stats.kinds.into_keys().collect()))
        .collect();
    sha256(&serde_json::to_vec(&normalized).expect("shape serialization cannot fail"))
}

fn looks_resource_like(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    normalized.starts_with("resources/")
        || [
            "models/",
            "textures/",
            "sounds/",
            "animations/",
            "particles/",
            "material_instances/",
            "projects/",
            "gameplay/",
            "level/",
            "flora/",
            "vegetation/",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
