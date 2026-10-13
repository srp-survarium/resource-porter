// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Clone, Debug)]
struct TargetMaterial {
    path: String,
    config: BinaryConfig,
    effects: BTreeMap<String, String>,
    stage_selectors: BTreeMap<String, String>,
    features: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct ConvertedMaterial {
    pub bytes: Vec<u8>,
    pub template_path: String,
    pub donor_texture_bindings: BTreeMap<String, String>,
}

pub struct MaterialCatalog {
    materials: Vec<TargetMaterial>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterialTemplateCandidate {
    pub path: String,
    pub effect_contract: BTreeMap<String, String>,
    pub effect_match: String,
    pub stage_contract_match: bool,
    pub semantic_domain_rank: Option<usize>,
    pub donor_stage_selectors: BTreeMap<String, String>,
    pub target_stage_selectors: BTreeMap<String, String>,
    pub target_features: BTreeMap<String, String>,
    pub feature_mismatches: usize,
    pub shared_path_tokens: usize,
}

impl MaterialCatalog {
    pub fn load(target: &mut ResourceDb) -> Result<Self> {
        let paths: Vec<String> = target
            .paths_with_prefix("material_instances/")
            .filter(|path| path.ends_with(".material") && target.is_file(path))
            .map(ToOwned::to_owned)
            .collect();
        let mut materials = Vec::new();
        for path in paths {
            let bytes = target.read(&path)?;
            let Ok(config) = BinaryConfig::parse(&bytes) else {
                continue;
            };
            let effects = effect_signature(&config.root);
            if effects.is_empty() {
                continue;
            }
            let features = feature_signature(&config.root);
            let stage_selectors = stage_selector_signature(&config.root);
            materials.push(TargetMaterial {
                path,
                config,
                effects,
                stage_selectors,
                features,
            });
        }
        if materials.is_empty() {
            bail!("target database contains no decodable material instances");
        }
        Ok(Self { materials })
    }

    pub fn convert(
        &self,
        path: &str,
        donor_bytes: &[u8],
        explicit_template: Option<&str>,
        donor: &ResourceDb,
        target: &ResourceDb,
    ) -> Result<ConvertedMaterial> {
        let donor_config = BinaryConfig::parse(donor_bytes)
            .with_context(|| format!("parsing donor material {path}"))?;
        let donor_effects = effect_signature(&donor_config.root);
        if donor_effects.is_empty() {
            bail!("donor material {path} has no render-stage effect_id");
        }
        let donor_features = feature_signature(&donor_config.root);
        let donor_stage_selectors = stage_selector_signature(&donor_config.root);

        let template = if let Some(explicit) = explicit_template {
            self.materials
                .iter()
                .find(|candidate| candidate.path == explicit)
                .with_context(|| {
                    format!(
                        "target material template {explicit} for donor material {path} does not exist"
                    )
                })?
        } else {
            let exact = self
                .materials
                .iter()
                .filter(|candidate| {
                    candidate.effects == donor_effects
                        && candidate.stage_selectors == donor_stage_selectors
                        && semantic_domain_rank(path, &candidate.path).is_some()
                })
                .min_by_key(|candidate| {
                    let score = compatibility_score(
                        path,
                        &donor_features,
                        &candidate.path,
                        &candidate.features,
                    );
                    (
                        semantic_domain_rank(path, &candidate.path).unwrap(),
                        score.0,
                        score.1,
                        score.2,
                    )
                });
            let donor_family = compatible_effect_family(&donor_effects);
            exact
                .or_else(|| {
                    donor_family.as_ref().and_then(|family| {
                        self.materials
                            .iter()
                            .filter(|candidate| {
                                effect_ids(&candidate.effects) == *family
                                    && candidate.stage_selectors == donor_stage_selectors
                                    && semantic_domain_rank(path, &candidate.path).is_some()
                            })
                            .min_by_key(|candidate| {
                                let score = compatibility_score(
                                    path,
                                    &donor_features,
                                    &candidate.path,
                                    &candidate.features,
                                );
                                (
                                    semantic_domain_rank(path, &candidate.path).unwrap(),
                                    score.0,
                                    score.1,
                                    score.2,
                                )
                            })
                    })
                })
                .with_context(|| {
                    format!(
                        "no target material has the donor stage/effect contract {:?} for {path}; \
                         declare material_template_overrides.{path}",
                        donor_effects
                    )
                })?
        };

        let mut converted = template.config.clone();
        let mut donor_texture_bindings = BTreeMap::new();
        merge_safe_visual_values(
            &mut converted.root,
            &donor_config.root,
            "",
            "",
            &|texture| texture_exists(texture, donor, target),
            &mut donor_texture_bindings,
        );
        neutralize_template_vertex_blending(&mut converted.root, &donor_config.root);
        Ok(ConvertedMaterial {
            bytes: converted.to_bytes()?,
            template_path: template.path.clone(),
            donor_texture_bindings,
        })
    }

    pub fn rank_templates(
        &self,
        donor_path: &str,
        donor_bytes: &[u8],
    ) -> Result<Vec<MaterialTemplateCandidate>> {
        let donor_config = BinaryConfig::parse(donor_bytes)
            .with_context(|| format!("parsing donor material {donor_path}"))?;
        let donor_effects = effect_signature(&donor_config.root);
        if donor_effects.is_empty() {
            bail!("donor material {donor_path} has no render-stage effect_id");
        }
        let donor_features = feature_signature(&donor_config.root);
        let donor_stage_selectors = stage_selector_signature(&donor_config.root);
        let donor_family = compatible_effect_family(&donor_effects);
        let mut candidates: Vec<_> = self
            .materials
            .iter()
            .filter_map(|candidate| {
                let effect_match = if candidate.effects == donor_effects {
                    "exact"
                } else if donor_family
                    .as_ref()
                    .is_some_and(|family| effect_ids(&candidate.effects) == *family)
                {
                    "compatible_family"
                } else {
                    return None;
                };
                let (feature_mismatches, reverse_shared_tokens, _) = compatibility_score(
                    donor_path,
                    &donor_features,
                    &candidate.path,
                    &candidate.features,
                );
                Some(MaterialTemplateCandidate {
                    path: candidate.path.clone(),
                    effect_contract: candidate.effects.clone(),
                    effect_match: effect_match.to_owned(),
                    stage_contract_match: candidate.stage_selectors == donor_stage_selectors,
                    semantic_domain_rank: semantic_domain_rank(donor_path, &candidate.path),
                    donor_stage_selectors: donor_stage_selectors.clone(),
                    target_stage_selectors: candidate.stage_selectors.clone(),
                    target_features: candidate.features.clone(),
                    feature_mismatches,
                    shared_path_tokens: usize::MAX - reverse_shared_tokens,
                })
            })
            .collect();
        candidates.sort_by(|left, right| {
            (
                left.effect_match != "exact",
                !left.stage_contract_match,
                left.semantic_domain_rank.unwrap_or(usize::MAX),
                left.feature_mismatches,
                usize::MAX - left.shared_path_tokens,
                &left.path,
            )
                .cmp(&(
                    right.effect_match != "exact",
                    !right.stage_contract_match,
                    right.semantic_domain_rank.unwrap_or(usize::MAX),
                    right.feature_mismatches,
                    usize::MAX - right.shared_path_tokens,
                    &right.path,
                ))
        });
        Ok(candidates)
    }
}

fn effect_ids(effects: &BTreeMap<String, String>) -> BTreeSet<String> {
    effects.values().cloned().collect()
}

fn compatible_effect_family(effects: &BTreeMap<String, String>) -> Option<BTreeSet<String>> {
    let mut ids = effect_ids(effects);
    let had_particle_lighting = ids.remove("particle_lighting");
    let had_distortion_panner = ids.remove("distortion_panner");
    if had_distortion_panner {
        ids.insert("distortion_default".into());
    }
    if had_particle_lighting {
        ids.extend([
            "distortion_default".into(),
            "lighting_default".into(),
            "forward_default".into(),
        ]);
    } else if ids.contains("lighting_default") && ids.contains("forward_default") {
        // 0.100b particle materials split the newer combined
        // forward+lighting layout into distortion, lighting, and forward
        // stages. A real target material carrying all three is required so
        // every requested shader permutation already exists in the retail DB.
        ids.insert("distortion_default".into());
    }
    (had_particle_lighting || had_distortion_panner || ids != effect_ids(effects)).then_some(ids)
}

fn material_stages(root: &ConfigValue) -> Option<&[crate::NamedValue]> {
    let ConfigValue::Named(stages) = root.named("material")? else {
        return None;
    };
    Some(stages)
}

fn effect_signature(root: &ConfigValue) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let Some(stages) = material_stages(root) else {
        return result;
    };
    for stage in stages {
        let Some(effect) = stage.value.named("effect") else {
            continue;
        };
        if let Some(ConfigValue::String(effect_id)) = effect.named("effect_id") {
            result.insert(stage.name.clone(), effect_id.clone());
        }
    }
    result
}

fn feature_signature(root: &ConfigValue) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    collect_features(root, "", None, false, &mut result);
    result
}

fn stage_selector_signature(root: &ConfigValue) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    collect_stage_selectors(root, "", None, &mut result);
    result
}

fn collect_stage_selectors(
    value: &ConfigValue,
    path: &str,
    parameter: Option<&str>,
    output: &mut BTreeMap<String, String>,
) {
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                let child_path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}.{}", child.name)
                };
                let child_parameter =
                    if matches!(child.name.as_str(), "value" | "is_model_parameter") {
                        parameter
                    } else {
                        Some(child.name.as_str())
                    };
                collect_stage_selectors(&child.value, &child_path, child_parameter, output);
            }
        }
        ConfigValue::Boolean(value) if is_stage_selector(parameter.unwrap_or("")) => {
            output.insert(path.to_owned(), value.to_string());
        }
        ConfigValue::Integer(value) if is_stage_selector(parameter.unwrap_or("")) => {
            output.insert(path.to_owned(), value.to_string());
        }
        _ => {}
    }
}

fn collect_features(
    value: &ConfigValue,
    path: &str,
    parameter: Option<&str>,
    in_effect: bool,
    output: &mut BTreeMap<String, String>,
) {
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                let child_path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}.{}", child.name)
                };
                let child_parameter =
                    if matches!(child.name.as_str(), "value" | "is_model_parameter") {
                        parameter
                    } else {
                        Some(child.name.as_str())
                    };
                collect_features(
                    &child.value,
                    &child_path,
                    child_parameter,
                    in_effect || child.name == "effect",
                    output,
                );
            }
        }
        ConfigValue::Boolean(value)
            if (in_effect && is_shader_feature(parameter.unwrap_or("")))
                || is_stage_selector(parameter.unwrap_or("")) =>
        {
            output.insert(path.to_owned(), value.to_string());
        }
        ConfigValue::Integer(value)
            if (in_effect && is_shader_feature(parameter.unwrap_or("")))
                || is_stage_selector(parameter.unwrap_or("")) =>
        {
            output.insert(path.to_owned(), value.to_string());
        }
        ConfigValue::String(value) if parameter == Some("effect_id") => {
            output.insert(path.to_owned(), value.clone());
        }
        _ => {}
    }
}

fn is_shader_feature(name: &str) -> bool {
    name.starts_with("use_")
        || name.starts_with("with_")
        || name.starts_with("is_")
        || name.ends_with("_type")
        || matches!(
            name,
            "wind_motion" | "lighting_model" | "decal_type" | "two_sided"
        )
}

fn is_stage_selector(name: &str) -> bool {
    name.starts_with("with_") || matches!(name, "two_sided" | "is_emissive")
}

fn compatibility_score(
    donor_path: &str,
    donor: &BTreeMap<String, String>,
    target_path: &str,
    target: &BTreeMap<String, String>,
) -> (usize, usize, String) {
    let feature_mismatches = donor
        .iter()
        .filter(|(path, value)| target.get(*path) != Some(*value))
        .count()
        + target
            .keys()
            .filter(|path| !donor.contains_key(*path))
            .count();
    let donor_tokens = path_tokens(donor_path);
    let target_tokens = path_tokens(target_path);
    let shared_tokens = donor_tokens.intersection(&target_tokens).count();
    (
        feature_mismatches,
        usize::MAX - shared_tokens,
        target_path.to_owned(),
    )
}

fn path_tokens(path: &str) -> BTreeSet<&str> {
    path.split(['/', '_', '.', '-'])
        .filter(|token| !token.is_empty() && *token != "material")
        .collect()
}

fn semantic_domain_rank(donor_path: &str, target_path: &str) -> Option<usize> {
    let donor_parts: Vec<_> = donor_path
        .trim_start_matches("material_instances/")
        .split('/')
        .collect();
    let target_parts: Vec<_> = target_path
        .trim_start_matches("material_instances/")
        .split('/')
        .collect();
    let donor_domain = *donor_parts.first()?;
    let target_domain = *target_parts.first()?;
    if donor_domain == target_domain {
        if donor_parts.get(1) == target_parts.get(1) {
            return Some(0);
        }
        return Some(1);
    }
    match (donor_domain, target_domain) {
        ("terrain_tiled", "terrain" | "tiled") => Some(2),
        ("generic", "props") => Some(3),
        _ => None,
    }
}

fn merge_safe_visual_values(
    target: &mut ConfigValue,
    donor: &ConfigValue,
    parameter: &str,
    path: &str,
    texture_exists: &impl Fn(&str) -> bool,
    donor_texture_bindings: &mut BTreeMap<String, String>,
) {
    match (target, donor) {
        (ConfigValue::Named(target_values), ConfigValue::Named(donor_values)) => {
            for target_child in target_values {
                let Some(donor_child) = donor_values
                    .iter()
                    .find(|candidate| candidate.name == target_child.name)
                else {
                    continue;
                };
                merge_safe_visual_values(
                    &mut target_child.value,
                    &donor_child.value,
                    if matches!(target_child.name.as_str(), "value" | "is_model_parameter") {
                        parameter
                    } else {
                        &target_child.name
                    },
                    &if path.is_empty() {
                        target_child.name.clone()
                    } else {
                        format!("{path}.{}", target_child.name)
                    },
                    texture_exists,
                    donor_texture_bindings,
                );
            }
        }
        (ConfigValue::Indexed(target_values), ConfigValue::Indexed(donor_values)) => {
            for (index, (target_child, donor_child)) in
                target_values.iter_mut().zip(donor_values).enumerate()
            {
                merge_safe_visual_values(
                    target_child,
                    donor_child,
                    parameter,
                    &if path.is_empty() {
                        index.to_string()
                    } else {
                        format!("{path}.{index}")
                    },
                    texture_exists,
                    donor_texture_bindings,
                );
            }
        }
        (target @ ConfigValue::Float(_), donor @ ConfigValue::Float(_))
        | (target @ ConfigValue::Float2(_), donor @ ConfigValue::Float2(_))
        | (target @ ConfigValue::Float3(_), donor @ ConfigValue::Float3(_))
        | (target @ ConfigValue::Float4(_), donor @ ConfigValue::Float4(_)) => {
            *target = donor.clone();
        }
        (ConfigValue::Float(target), ConfigValue::Integer(donor))
            if !is_shader_feature(parameter) =>
        {
            *target = *donor as f32;
        }
        (target @ ConfigValue::Integer(_), donor @ ConfigValue::Float(_))
            if !is_shader_feature(parameter) =>
        {
            // Numeric material readers accept either scalar type. Preserve the
            // authored fraction instead of keeping the template or truncating it.
            *target = donor.clone();
        }
        (target @ ConfigValue::Integer(_), donor @ ConfigValue::Integer(_))
            if !is_shader_feature(parameter) =>
        {
            *target = donor.clone();
        }
        (ConfigValue::String(target_value), ConfigValue::String(donor_value))
            if is_texture_parameter(parameter)
                && ((!donor_value.is_empty() && texture_exists(donor_value))
                    || (donor_value.is_empty() && target_value.is_empty())) =>
        {
            *target_value = donor_value.clone();
            if !donor_value.is_empty() {
                donor_texture_bindings.insert(path.to_owned(), donor_value.clone());
            }
        }
        _ => {}
    }
}

/// A template whose shader blends a second texture set by vertex colour,
/// converting a donor material that does not, would paint the template's own
/// blend textures (retail floor_linoleum_02_vc's `tiled/floor/floor_trash_02`)
/// over the donor's surface wherever its vertex colour asks for them. The
/// feature flag selects the shader permutation and stays; the blend
/// textures become the material's own, so the blend changes nothing.
fn neutralize_template_vertex_blending(converted: &mut ConfigValue, donor: &ConfigValue) {
    let Some(ConfigValue::Named(stages)) = converted.named_mut("material") else {
        return;
    };
    for stage in stages {
        let donor_effect = donor
            .named("material")
            .and_then(|stages| stages.named(&stage.name))
            .and_then(|stage| stage.named("effect"));
        let Some(effect) = stage.value.named_mut("effect") else {
            continue;
        };
        for (feature, blended, own) in [
            (
                "use_vertex_blended_diffuse",
                "texture_vertex_blended_diffuse",
                "texture_diffuse",
            ),
            (
                "use_vertex_blended_normal",
                "texture_vertex_blended_normal",
                "texture_normal",
            ),
        ] {
            let enabled = |effect: Option<&ConfigValue>| {
                matches!(
                    effect
                        .and_then(|effect| effect.named(feature))
                        .and_then(|parameter| parameter.named("value")),
                    Some(ConfigValue::Boolean(true))
                )
            };
            if !enabled(Some(effect)) || enabled(donor_effect) {
                continue;
            }
            let Some(own_texture) = effect
                .named(own)
                .and_then(|parameter| parameter.named("value"))
                .cloned()
            else {
                continue;
            };
            if let Some(value) = effect
                .named_mut(blended)
                .and_then(|parameter| parameter.named_mut("value"))
            {
                *value = own_texture;
            }
        }
    }
}

fn is_texture_parameter(name: &str) -> bool {
    name.starts_with("texture_") || name.ends_with("_texture")
}

fn texture_exists(texture: &str, donor: &ResourceDb, target: &ResourceDb) -> bool {
    if texture.is_empty() {
        return true;
    }
    let Ok(normalized) = canonical_texture_reference(texture) else {
        return false;
    };
    let path = format!("textures/{normalized}.dds");
    donor.is_file(&path) || target.is_file(&path)
}

pub(crate) fn canonical_texture_reference(texture: &str) -> Result<String> {
    let normalized = texture.replace('\\', "/");
    let normalized = normalized.strip_prefix("resources/").unwrap_or(&normalized);
    let normalized = normalized.strip_prefix("textures/").unwrap_or(normalized);
    let normalized = normalized.strip_suffix(".dds").unwrap_or(normalized);
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("unsafe or empty texture reference {texture:?}");
    }
    Ok(normalized.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, value: ConfigValue) -> crate::NamedValue {
        crate::NamedValue {
            name: name.into(),
            value,
        }
    }

    #[test]
    fn blends_a_donor_without_vertex_blending_only_with_itself() {
        let parameter = |value| ConfigValue::Named(vec![named("value", value)]);
        let string = |text: &str| parameter(ConfigValue::String(text.into()));
        let material = |use_blend: bool, blended: &str| {
            ConfigValue::Named(vec![named(
                "material",
                ConfigValue::Named(vec![named(
                    "g_stage",
                    ConfigValue::Named(vec![named(
                        "effect",
                        ConfigValue::Named(vec![
                            named(
                                "use_vertex_blended_diffuse",
                                parameter(ConfigValue::Boolean(use_blend)),
                            ),
                            named(
                                "use_vertex_blended_normal",
                                parameter(ConfigValue::Boolean(use_blend)),
                            ),
                            named("texture_diffuse", string("terrain_tiled/ground")),
                            named("texture_normal", string("terrain_tiled/ground_nmap")),
                            named("texture_vertex_blended_diffuse", string(blended)),
                            named(
                                "texture_vertex_blended_normal",
                                string("terrain_tiled/sand_nmap"),
                            ),
                        ]),
                    )]),
                )]),
            )])
        };
        let effect_value = |material: &ConfigValue, name: &str| {
            material
                .named("material")
                .and_then(|stages| stages.named("g_stage"))
                .and_then(|stage| stage.named("effect"))
                .and_then(|effect| effect.named(name))
                .and_then(|parameter| parameter.named("value"))
                .cloned()
        };

        let mut converted = material(true, "tiled/floor/floor_trash_02");
        neutralize_template_vertex_blending(&mut converted, &material(false, ""));
        assert_eq!(
            effect_value(&converted, "texture_vertex_blended_diffuse"),
            Some(ConfigValue::String("terrain_tiled/ground".into()))
        );
        assert_eq!(
            effect_value(&converted, "texture_vertex_blended_normal"),
            Some(ConfigValue::String("terrain_tiled/ground_nmap".into()))
        );
        assert_eq!(
            effect_value(&converted, "use_vertex_blended_diffuse"),
            Some(ConfigValue::Boolean(true))
        );

        // A donor that blends keeps its own blend textures.
        let mut converted = material(true, "terrain_tiled/moss");
        neutralize_template_vertex_blending(&mut converted, &material(true, "terrain_tiled/moss"));
        assert_eq!(
            effect_value(&converted, "texture_vertex_blended_diffuse"),
            Some(ConfigValue::String("terrain_tiled/moss".into()))
        );
    }

    #[test]
    fn keeps_template_shader_switches_but_moves_visual_values() {
        let mut target = ConfigValue::Named(vec![
            named("use_nmap", ConfigValue::Boolean(false)),
            named("with_static_meshes", ConfigValue::Boolean(true)),
            named("constant_power", ConfigValue::Float(1.0)),
            named("wind_motion", ConfigValue::Integer(2)),
            named(
                "constant_roughness_min",
                ConfigValue::Named(vec![named("value", ConfigValue::Integer(0))]),
            ),
            named("lighting_model", ConfigValue::Float(2.0)),
            named(
                "constant_transparency",
                ConfigValue::Named(vec![named("value", ConfigValue::Float(0.05))]),
            ),
            named("texture_diffuse", ConfigValue::String("old".into())),
            named("effect_id", ConfigValue::String("forward_default".into())),
        ]);
        let donor = ConfigValue::Named(vec![
            named("use_nmap", ConfigValue::Boolean(true)),
            named("with_static_meshes", ConfigValue::Boolean(false)),
            named("constant_power", ConfigValue::Float(3.0)),
            named("wind_motion", ConfigValue::Float(1.0)),
            named(
                "constant_roughness_min",
                ConfigValue::Named(vec![named("value", ConfigValue::Float(0.3))]),
            ),
            named("lighting_model", ConfigValue::Integer(1)),
            named(
                "constant_transparency",
                ConfigValue::Named(vec![named("value", ConfigValue::Integer(1))]),
            ),
            named("texture_diffuse", ConfigValue::String("new".into())),
            named("effect_id", ConfigValue::String("new_effect".into())),
        ]);
        let mut donor_textures = BTreeMap::new();
        merge_safe_visual_values(
            &mut target,
            &donor,
            "",
            "",
            &|name| name == "new",
            &mut donor_textures,
        );
        let bytes = BinaryConfig { root: target }.to_bytes().unwrap();
        let target = BinaryConfig::parse(&bytes).unwrap().root;
        assert_eq!(target.named("wind_motion"), Some(&ConfigValue::Integer(2)));
        assert_eq!(
            target
                .named("constant_roughness_min")
                .and_then(|value| value.named("value")),
            Some(&ConfigValue::Float(0.3))
        );
        assert_eq!(target.named("use_nmap"), Some(&ConfigValue::Boolean(false)));
        assert_eq!(
            target.named("lighting_model"),
            Some(&ConfigValue::Float(2.0))
        );
        assert_eq!(
            target.named("with_static_meshes"),
            Some(&ConfigValue::Boolean(true))
        );
        assert_eq!(
            target.named("constant_power"),
            Some(&ConfigValue::Float(3.0))
        );
        assert_eq!(
            target
                .named("constant_transparency")
                .and_then(|value| value.named("value")),
            Some(&ConfigValue::Float(1.0))
        );
        assert_eq!(
            target.named("texture_diffuse"),
            Some(&ConfigValue::String("new".into()))
        );
        assert_eq!(
            target.named("effect_id"),
            Some(&ConfigValue::String("forward_default".into()))
        );
        assert_eq!(
            donor_textures,
            BTreeMap::from([("texture_diffuse".into(), "new".into())])
        );
    }

    #[test]
    fn stage_selectors_participate_in_template_matching_outside_effect_blocks() {
        let mut config = ConfigValue::Named(vec![named(
            "material",
            ConfigValue::Named(vec![named(
                "forward",
                ConfigValue::Named(vec![
                    named("with_static_meshes", ConfigValue::Boolean(true)),
                    named(
                        "effect",
                        ConfigValue::Named(vec![named(
                            "effect_id",
                            ConfigValue::String("forward_default".into()),
                        )]),
                    ),
                ]),
            )]),
        )]);
        assert!(
            feature_signature(&config)
                .values()
                .any(|value| value == "true")
        );

        config
            .set_path(
                "material.forward.effect.use_emissive",
                ConfigValue::Boolean(true),
            )
            .unwrap();
        let donor = BinaryConfig {
            root: config.clone(),
        }
        .to_bytes()
        .unwrap();
        let matching = "material_instances/props/light/lamp_lights_light.material";
        let closer = "material_instances/props/light/lamp_greenhouse.material";
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        std::fs::create_dir_all(input.join("material_instances/props/light")).unwrap();
        std::fs::write(input.join(matching), &donor).unwrap();
        config
            .set_path(
                "material.forward.effect.use_emissive",
                ConfigValue::Boolean(false),
            )
            .unwrap();
        std::fs::write(
            input.join(closer),
            BinaryConfig { root: config }.to_bytes().unwrap(),
        )
        .unwrap();
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let mut db = ResourceDb::open(archive).unwrap();
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        let path = "material_instances/props/light/lamp_greenhouse_clone.material";
        let converted = catalog.convert(path, &donor, None, &db, &db).unwrap();
        assert_eq!(converted.template_path, matching);
        assert_eq!(
            catalog.rank_templates(path, &donor).unwrap()[0].path,
            matching
        );
        assert_eq!(
            feature_signature(&BinaryConfig::parse(&converted.bytes).unwrap().root),
            feature_signature(&BinaryConfig::parse(&donor).unwrap().root),
        );
        assert_eq!(
            catalog
                .convert(path, &donor, Some(closer), &db, &db)
                .unwrap()
                .template_path,
            closer,
        );
    }
}
