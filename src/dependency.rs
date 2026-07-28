use std::collections::{BTreeSet, VecDeque};

use anyhow::Result;
use serde::Serialize;

use crate::{BinaryConfig, ResourceDb};

#[derive(Clone, Debug, Default, Serialize)]
pub struct DependencyPlan {
    pub roots: Vec<String>,
    pub donor_files: BTreeSet<String>,
    pub target_files: BTreeSet<String>,
    pub unresolved_references: BTreeSet<String>,
}

pub fn scan(
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    roots: impl IntoIterator<Item = String>,
) -> Result<DependencyPlan> {
    let roots: Vec<String> = roots.into_iter().collect();
    let mut plan = DependencyPlan {
        roots: roots.clone(),
        ..DependencyPlan::default()
    };
    let mut queue: VecDeque<String> = roots.into();
    let mut visited = BTreeSet::new();

    while let Some(path) = queue.pop_front() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let (bytes, from_donor) = if donor.contains(&path) {
            (donor.read(&path)?, true)
        } else if target.contains(&path) {
            (target.read(&path)?, false)
        } else {
            plan.unresolved_references.insert(path);
            continue;
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
            for candidate in resolve_candidates(reference, donor, target) {
                queue.push_back(candidate);
            }
        }
    }
    Ok(plan)
}

fn resolve_candidates(
    reference: &str,
    donor: &ResourceDb,
    target: &ResourceDb,
) -> BTreeSet<String> {
    let normalized = reference
        .replace('\\', "/")
        .trim_start_matches("resources/")
        .to_owned();
    if normalized.is_empty() || normalized.contains(' ') {
        return BTreeSet::new();
    }
    let mut probes = vec![normalized.clone()];
    if !normalized.contains('.') {
        probes.extend([
            format!("{normalized}.options"),
            format!("{normalized}.material"),
            format!("{normalized}.sound_collection"),
            format!("{normalized}.single_sound_options"),
            format!("{normalized}.clip"),
            format!("models/{normalized}.model/"),
            format!("models/{normalized}.skinned_model/"),
            format!("animations/skeletons/{normalized}.skeleton"),
            format!("material_instances/{normalized}.material"),
            format!("particles/{normalized}.binary_particle"),
        ]);
    }

    let mut resolved = BTreeSet::new();
    for probe in probes {
        if donor.contains(&probe) || target.contains(&probe) {
            resolved.insert(probe.clone());
        }
        for path in donor.paths_with_prefix(&probe) {
            resolved.insert(path.to_owned());
        }
        for path in target.paths_with_prefix(&probe) {
            resolved.insert(path.to_owned());
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes_resource_root() {
        let value = "resources/animations/example".trim_start_matches("resources/");
        assert_eq!(value, "animations/example");
    }
}
