use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::Result;
use serde::Serialize;

use crate::{BinaryConfig, ResourceDb};

#[derive(Clone, Debug, Default, Serialize)]
pub struct DependencyPlan {
    pub roots: Vec<String>,
    pub donor_files: BTreeSet<String>,
    pub target_files: BTreeSet<String>,
    pub unresolved_references: BTreeSet<String>,
    /// Context-qualified edges resolved relative to a project resource.
    /// Keys are `<referrer> -> <serialized reference>` and values are the
    /// concrete archive paths selected by nearest-ancestor lookup.
    pub resolved_relative_references: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SourcePolicy {
    DonorRequired,
    TargetPreferred,
    TargetOnly,
}

pub fn scan(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    roots: impl IntoIterator<Item = String>,
) -> Result<DependencyPlan> {
    scan_with_forced_donor(donor, target, roots, std::iter::empty())
}

pub fn scan_with_forced_donor(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    roots: impl IntoIterator<Item = String>,
    forced_donor_paths: impl IntoIterator<Item = String>,
) -> Result<DependencyPlan> {
    scan_with_forced_donor_and_overrides(donor, target, roots, forced_donor_paths, &BTreeMap::new())
}

pub fn scan_with_forced_donor_and_overrides(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    roots: impl IntoIterator<Item = String>,
    forced_donor_paths: impl IntoIterator<Item = String>,
    content_overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<DependencyPlan> {
    let roots: Vec<String> = roots.into_iter().collect();
    let forced_donor_paths: BTreeSet<String> =
        roots.iter().cloned().chain(forced_donor_paths).collect();
    let mut plan = DependencyPlan {
        roots: roots.clone(),
        ..DependencyPlan::default()
    };
    let mut queue: VecDeque<(String, SourcePolicy)> = roots
        .into_iter()
        .map(|path| (path, SourcePolicy::DonorRequired))
        .collect();
    let mut visited = BTreeSet::<(String, SourcePolicy)>::new();

    while let Some((path, policy)) = queue.pop_front() {
        if !visited.insert((path.clone(), policy)) {
            continue;
        }

        let selected = match policy {
            SourcePolicy::DonorRequired => {
                select_path(donor, &path).map(|selection| (selection, true))
            }
            SourcePolicy::TargetPreferred => select_path(target, &path)
                .map(|selection| (selection, false))
                .or_else(|| select_path(donor, &path).map(|selection| (selection, true))),
            SourcePolicy::TargetOnly => {
                select_path(target, &path).map(|selection| (selection, false))
            }
        };
        let Some((selection, from_donor)) = selected else {
            plan.unresolved_references.insert(path);
            continue;
        };

        if let PathSelection::Directory(children) = selection {
            queue.extend(children.into_iter().map(|child| {
                let child_policy = dependency_policy(&child, from_donor, &forced_donor_paths);
                (child, child_policy)
            }));
            continue;
        }

        let bytes = if let Some(bytes) = content_overrides.get(&path) {
            bytes.clone()
        } else if from_donor {
            donor.read(&path)?
        } else {
            target.read(&path)?
        };
        if from_donor {
            plan.donor_files.insert(path.clone());
        } else {
            plan.target_files.insert(path.clone());
        }

        let Ok(config) = BinaryConfig::parse(&bytes) else {
            continue;
        };
        let mut strings = Vec::new();
        config.root.collect_strings(&mut strings);
        for reference in strings {
            let (candidates, relative) = resolve_candidates_from(reference, &path, donor, target);
            if let Some(resolved) = relative {
                plan.resolved_relative_references
                    .insert(format!("{path} -> {reference}"), resolved);
            }
            for candidate in candidates {
                let candidate_policy =
                    dependency_policy(&candidate, from_donor, &forced_donor_paths);
                queue.push_back((candidate, candidate_policy));
            }
        }
    }
    Ok(plan)
}

fn dependency_policy(
    path: &str,
    referrer_from_donor: bool,
    forced_donor_paths: &BTreeSet<String>,
) -> SourcePolicy {
    if is_forced_donor(path, forced_donor_paths) {
        SourcePolicy::DonorRequired
    } else if referrer_from_donor {
        SourcePolicy::TargetPreferred
    } else {
        SourcePolicy::TargetOnly
    }
}

fn is_forced_donor(path: &str, forced_donor_paths: &BTreeSet<String>) -> bool {
    forced_donor_paths.iter().any(|forced| {
        let forced = forced.trim_end_matches('/');
        path == forced
            || path
                .strip_prefix(forced)
                .is_some_and(|tail| tail.starts_with('/'))
    })
}

enum PathSelection {
    File,
    Directory(Vec<String>),
}

fn select_path(db: &ResourceDb, path: &str) -> Option<PathSelection> {
    if db.is_file(path) {
        return Some(PathSelection::File);
    }
    let prefix = format!("{}/", path.trim_end_matches('/'));
    let children: Vec<String> = db
        .paths_with_prefix(&prefix)
        .filter(|child| db.is_file(child))
        .map(ToOwned::to_owned)
        .collect();
    (!children.is_empty()).then_some(PathSelection::Directory(children))
}

pub(crate) fn resolve_candidates(
    reference: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
) -> BTreeSet<String> {
    resolve_candidates_from(reference, "", donor, target).0
}

fn resolve_candidates_from(
    reference: &str,
    referring_path: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
) -> (BTreeSet<String>, Option<String>) {
    if let Some(resolved) =
        resolve_project_relative_reference(reference, referring_path, donor, target)
    {
        return (BTreeSet::from([resolved.clone()]), Some(resolved));
    }
    (resolve_global_candidates(reference, donor, target), None)
}

fn resolve_project_relative_reference(
    reference: &str,
    referring_path: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
) -> Option<String> {
    let normalized = reference.replace('\\', "/");
    if referring_path.is_empty()
        || !referring_path.starts_with("projects/")
        || normalized.starts_with("resources/")
        || normalized.starts_with("projects/")
        || normalized.starts_with('/')
        || normalized.contains(' ')
        || !normalized.contains('/')
    {
        return None;
    }
    let normalized = normalized
        .split('#')
        .next()
        .unwrap_or_default()
        .trim_matches('/');
    if normalized.is_empty()
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return None;
    }

    let mut directory = referring_path.rsplit_once('/')?.0;
    loop {
        let candidate = format!("{directory}/{normalized}");
        if target.contains(&candidate) || donor.contains(&candidate) {
            return Some(candidate);
        }
        let Some(parent) = directory.rsplit_once('/').map(|(parent, _)| parent) else {
            break;
        };
        // Never escape projects/<project-name>; a bare project-relative
        // reference must not capture an unrelated global donor resource.
        if parent.matches('/').count() < 1 || !parent.starts_with("projects/") {
            break;
        }
        directory = parent;
    }
    None
}

fn resolve_global_candidates(
    reference: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
) -> BTreeSet<String> {
    let normalized_reference = reference.replace('\\', "/");
    let is_explicit_resource_path = normalized_reference.starts_with("resources/");
    let normalized = normalized_reference
        .trim_start_matches("resources/")
        .split('#')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if normalized.is_empty() || normalized.contains(' ') {
        return BTreeSet::new();
    }
    let mut probes = vec![normalized.clone()];
    let mut required_companions = Vec::new();
    if !normalized.contains('.') {
        let texture_base = if normalized.starts_with("textures/") {
            normalized.clone()
        } else {
            format!("textures/{normalized}")
        };
        let sound_base = if normalized.starts_with("sounds/") {
            normalized.clone()
        } else {
            format!("sounds/{normalized}")
        };
        let single_sound_base = if normalized.starts_with("sounds/single/") {
            normalized.clone()
        } else {
            format!("sounds/single/{}", normalized.trim_start_matches("sounds/"))
        };
        let sound_collection = if normalized.starts_with("sounds/collections/") {
            format!("{normalized}.sound_collection")
        } else {
            format!(
                "sounds/collections/{}.sound_collection",
                normalized.trim_start_matches("sounds/")
            )
        };
        let animation_base = if normalized.starts_with("animations/") {
            normalized.clone()
        } else {
            format!("animations/{normalized}")
        };
        probes.extend([
            format!("{normalized}.dds"),
            format!("{normalized}.options"),
            format!("{normalized}.high.ogg"),
            format!("{normalized}.material"),
            format!("{normalized}.sound_collection"),
            format!("{normalized}.single_sound_options"),
            format!("{normalized}.clip"),
            format!("{normalized}.b-spline"),
            format!("{normalized}.bones_names"),
            format!("models/{normalized}.model/"),
            format!("models/{normalized}.skinned_model/"),
            format!("animations/skeletons/{normalized}.skeleton"),
            format!("material_instances/{normalized}.material"),
            format!("particles/{normalized}.particle"),
            format!("particles/{normalized}.binary_particle"),
            format!("{texture_base}.dds"),
            format!("{texture_base}.options"),
            format!("{sound_base}.high.ogg"),
            format!("{sound_base}.single_sound_options"),
            format!("{single_sound_base}.high.ogg"),
            format!("{single_sound_base}.single_sound_options"),
            sound_collection,
            format!("{animation_base}.b-spline"),
            format!("{animation_base}.bones_names"),
        ]);
        if donor.is_file(&format!("{texture_base}.dds"))
            || donor.is_file(&format!("{texture_base}.options"))
            || target.is_file(&format!("{texture_base}.dds"))
            || target.is_file(&format!("{texture_base}.options"))
        {
            required_companions.extend([
                format!("{texture_base}.dds"),
                format!("{texture_base}.options"),
            ]);
        }
        if donor.is_file(&format!("{single_sound_base}.high.ogg"))
            || donor.is_file(&format!("{single_sound_base}.single_sound_options"))
            || target.is_file(&format!("{single_sound_base}.high.ogg"))
            || target.is_file(&format!("{single_sound_base}.single_sound_options"))
        {
            required_companions.extend([
                format!("{single_sound_base}.high.ogg"),
                format!("{single_sound_base}.single_sound_options"),
            ]);
        }
        if animation_base.starts_with("animations/single/")
            && (donor.is_file(&format!("{animation_base}.b-spline"))
                || donor.is_file(&format!("{animation_base}.bones_names"))
                || target.is_file(&format!("{animation_base}.b-spline"))
                || target.is_file(&format!("{animation_base}.bones_names")))
        {
            required_companions.extend([
                format!("{animation_base}.b-spline"),
                format!("{animation_base}.bones_names"),
            ]);
        }
    }

    let mut resolved = BTreeSet::new();
    let mut target_path_exists = false;
    for probe in probes {
        let target_matches = matching_files(target, &probe);
        if target.contains(&probe) {
            // Some old FATs expose collision payloads as opaque folder-like
            // nodes with no enumerable children. Their exact path still owns
            // the dependency; importing a donor child beneath such a node is
            // both unnecessary and impossible for an additive archive.
            target_path_exists = true;
            resolved.extend(target_matches);
        } else if target_matches.is_empty() {
            resolved.extend(matching_files(donor, &probe));
        } else {
            target_path_exists = true;
            resolved.extend(target_matches);
        }
    }
    resolved.extend(required_companions);
    if resolved.is_empty()
        && !target_path_exists
        && (is_explicit_resource_path
            || [
                "models/",
                "textures/",
                "sounds/",
                "animations/",
                "particles/",
                "material_instances/",
                "projects/",
                "gameplay/",
            ]
            .iter()
            .any(|prefix| normalized.starts_with(prefix)))
    {
        resolved.insert(normalized);
    }
    resolved
}

fn matching_files(db: &ResourceDb, probe: &str) -> BTreeSet<String> {
    let mut matches = BTreeSet::new();
    if db.is_file(probe) {
        matches.insert(probe.to_owned());
    }
    let prefix = format!("{}/", probe.trim_end_matches('/'));
    matches.extend(
        db.paths_with_prefix(&prefix)
            .filter(|path| db.is_file(path))
            .map(ToOwned::to_owned),
    );
    matches
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use resources_db::pack;
    use tempfile::TempDir;

    use super::*;
    use crate::binary_config::{ConfigValue, NamedValue};

    fn write_file(root: &Path, path: &str, bytes: &[u8]) {
        let output = root.join(path);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(output, bytes).unwrap();
    }

    fn pack_fixture(root: &Path, output: &Path) {
        pack::assemble_to_file(&pack::build_src_tree(root).unwrap(), output).unwrap();
    }

    #[test]
    fn normalizes_resource_root() {
        let value = "resources/animations/example".trim_start_matches("resources/");
        assert_eq!(value, "animations/example");
    }

    #[test]
    fn keeps_roots_from_donor_but_prefers_complete_target_dependency_subtrees() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");

        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "model".into(),
                value: ConfigValue::String("props/shared".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &donor_root,
            "models/props/shared.model/settings",
            b"donor settings",
        );
        write_file(
            &donor_root,
            "models/props/shared.model/render/new_surface/converted_model",
            b"donor-only surface",
        );
        write_file(
            &target_root,
            "models/props/shared.model/settings",
            b"target settings",
        );
        write_file(
            &target_root,
            "models/props/shared.model/render/stock_surface/converted_model",
            b"target surface",
        );

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("projects/imported/client_project")
        );
        assert!(
            plan.target_files
                .contains("models/props/shared.model/settings")
        );
        assert!(
            plan.target_files
                .contains("models/props/shared.model/render/stock_surface/converted_model")
        );
        assert!(
            !plan
                .donor_files
                .contains("models/props/shared.model/render/new_surface/converted_model")
        );
    }

    #[test]
    fn imports_the_complete_donor_subtree_when_target_has_no_dependency() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");

        let project = BinaryConfig {
            root: ConfigValue::Named(vec![
                NamedValue {
                    name: "model".into(),
                    value: ConfigValue::String("props/donor_only".into()),
                },
                NamedValue {
                    name: "texture".into(),
                    value: ConfigValue::String("textures/donor_only".into()),
                },
            ]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &donor_root,
            "models/props/donor_only.model/settings",
            b"donor settings",
        );
        write_file(
            &donor_root,
            "models/props/donor_only.model/render/surface/converted_model",
            b"donor surface",
        );
        write_file(&donor_root, "textures/donor_only.dds", b"donor texture");
        write_file(
            &donor_root,
            "textures/donor_only.options",
            b"donor texture options",
        );
        fs::create_dir_all(&target_root).unwrap();

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("models/props/donor_only.model/settings")
        );
        assert!(
            plan.donor_files
                .contains("models/props/donor_only.model/render/surface/converted_model")
        );
        assert!(plan.donor_files.contains("textures/donor_only.dds"));
        assert!(plan.donor_files.contains("textures/donor_only.options"));
        assert!(plan.target_files.is_empty());
    }

    #[test]
    fn explicit_donor_dependency_overrides_a_complete_target_subtree() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");

        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "model".into(),
                value: ConfigValue::String("props/shared".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &donor_root,
            "models/props/shared.model/settings",
            b"donor settings",
        );
        write_file(
            &target_root,
            "models/props/shared.model/settings",
            b"target settings",
        );

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan_with_forced_donor(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
            ["models/props/shared.model".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("models/props/shared.model/settings")
        );
        assert!(
            !plan
                .target_files
                .contains("models/props/shared.model/settings")
        );
    }

    #[test]
    fn target_owned_dependencies_never_fall_back_to_donor() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");

        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "post_effect".into(),
                value: ConfigValue::String("post_process/legacy_day".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        let target_material = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "disabled_texture".into(),
                value: ConfigValue::String("editor/moon".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &donor_root,
            "material_instances/editor/moon.material",
            b"incompatible donor material",
        );
        write_file(
            &target_root,
            "material_instances/post_process/legacy_day.material",
            &target_material,
        );

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.target_files
                .contains("material_instances/post_process/legacy_day.material")
        );
        assert!(
            !plan
                .donor_files
                .contains("material_instances/editor/moon.material")
        );
        assert!(
            plan.unresolved_references
                .contains("material_instances/editor/moon.material")
        );
    }

    #[test]
    fn extensionless_texture_reference_resolves_metadata_and_payload() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");
        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "texture".into(),
                value: ConfigValue::String("flora/grass_l01".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(&donor_root, "textures/flora/grass_l01.options", b"options");
        write_file(
            &donor_root,
            "textures/flora/grass_l01.dds",
            b"texture payload",
        );
        fs::create_dir_all(&target_root).unwrap();

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("textures/flora/grass_l01.options")
        );
        assert!(plan.donor_files.contains("textures/flora/grass_l01.dds"));
    }

    #[test]
    fn logical_sound_name_resolves_target_single_sound_payload() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");
        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "sound_name".into(),
                value: ConfigValue::String("scapes/creatures/creatures_line".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &target_root,
            "sounds/single/scapes/creatures/creatures_line.high.ogg",
            b"target sound",
        );
        write_file(
            &target_root,
            "sounds/single/scapes/creatures/creatures_line.single_sound_options",
            b"target options",
        );

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.target_files
                .contains("sounds/single/scapes/creatures/creatures_line.high.ogg")
        );
        assert!(
            plan.target_files
                .contains("sounds/single/scapes/creatures/creatures_line.single_sound_options")
        );
        assert!(plan.unresolved_references.is_empty());
    }

    #[test]
    fn instance_qualified_resource_resolves_its_backing_directory() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");
        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "collision".into(),
                value: ConfigValue::String(
                    "resources/models/level/terrain.model/collision#[2.0][1.0][2.0]".into(),
                ),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        write_file(
            &donor_root,
            "models/level/terrain.model/collision/vertices",
            b"vertices",
        );
        fs::create_dir_all(&target_root).unwrap();

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("models/level/terrain.model/collision/vertices")
        );
        assert!(plan.unresolved_references.is_empty());
    }

    #[test]
    fn missing_path_like_resource_is_reported() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");
        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "model".into(),
                value: ConfigValue::String("resources/models/level/missing.model/render".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(&donor_root, "projects/imported/client_project", &project);
        fs::create_dir_all(&target_root).unwrap();

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/imported/client_project".into()],
        )
        .unwrap();

        assert_eq!(
            plan.unresolved_references,
            BTreeSet::from(["models/level/missing.model/render".into()])
        );
    }

    #[test]
    fn resolves_layered_project_reference_from_nearest_existing_ancestor() {
        let temp = TempDir::new().unwrap();
        let donor_root = temp.path().join("donor");
        let target_root = temp.path().join("target");
        let project = BinaryConfig {
            root: ConfigValue::Named(vec![NamedValue {
                name: "vegetation".into(),
                value: ConfigValue::String("vegetation/a_high.veg".into()),
            }]),
        }
        .to_bytes()
        .unwrap();
        write_file(
            &donor_root,
            "projects/level_03/day/client_project",
            &project,
        );
        write_file(
            &donor_root,
            "projects/level_03/vegetation/a_high.veg",
            b"project-local vegetation",
        );
        write_file(
            &donor_root,
            "vegetation/a_high.veg",
            b"unrelated global vegetation",
        );
        fs::create_dir_all(&target_root).unwrap();

        let donor_db = temp.path().join("donor.db");
        let target_db = temp.path().join("target.db");
        pack_fixture(&donor_root, &donor_db);
        pack_fixture(&target_root, &target_db);
        let mut donor = ResourceDb::open(donor_db).unwrap();
        let mut target = ResourceDb::open(target_db).unwrap();

        let plan = scan(
            &mut donor,
            &mut target,
            ["projects/level_03/day/client_project".into()],
        )
        .unwrap();

        assert!(
            plan.donor_files
                .contains("projects/level_03/vegetation/a_high.veg")
        );
        assert!(!plan.donor_files.contains("vegetation/a_high.veg"));
        assert_eq!(
            plan.resolved_relative_references
                .get("projects/level_03/day/client_project -> vegetation/a_high.veg"),
            Some(&"projects/level_03/vegetation/a_high.veg".to_owned())
        );
    }
}
