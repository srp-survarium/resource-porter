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
    /// Source texture-versus-constant switches restored, emulated or left
    /// unresolved because no shipped native permutation supports them.
    pub switch_notes: Vec<String>,
}

pub struct MaterialCatalog {
    materials: Vec<TargetMaterial>,
    /// Shipped `gbuffer_pass.ps` keys indexed by material-switch projection,
    /// then by the remaining material digits, holding each geometry variant;
    /// `None` when the archive does not follow the verified key layout.
    gbuffer_projections: Option<ShippedVariants>,
    /// SHA-256 of the sorted shipped shader key paths (`shaders/sm_4_0/...`),
    /// to which runtime evidence is pinned.
    shader_catalog_sha256: String,
    /// Switch restorations proven by runtime loads: material -> switch -> level.
    runtime_proven: BTreeMap<String, BTreeMap<String, i64>>,
    /// Texture name bound where a material names a texture that exists in
    /// neither archive (see `bind_missing_textures`); `None` keeps them.
    null_texture: Option<String>,
}


/// `gbuffer_pass.ps` permutation key digits for material switches, as
/// positions in the 67-character key (68-character keys carry a two-digit
/// geometry value at 22 and shift later digits by one). Derived from
/// `shaders/masks` membership across all shipped shaders and checked on load:
/// every catalog g-stage material must project onto a shipped key, while
/// swapping any two of these assignments breaks between 8 and 1,616.
const GBUFFER_KEY_SWITCHES: [(usize, &str); 15] = [
    (5, "use_tdiffuse"),
    (6, "use_nmap"),
    (7, "use_detail_nmap"),
    (8, "use_alpha_test"),
    (10, "use_parallax"),
    (11, "use_ttranslucency"),
    (12, "use_tspecular_intensity"),
    (13, "use_tfresnel"),
    (14, "use_troughness"),
    (16, "use_tditail_diffuse"),
    (28, "use_alphablended_normal"),
    (35, "use_tiled_vertex_blended_mask"),
    (44, "use_uv_scrolling"),
    (46, "use_fuzziness"),
    (53, "wind_motion"),
];
/// Shading-quality digit; both shipped qualities must contain a permutation.
const GBUFFER_KEY_QUALITY: usize = 1;
const GBUFFER_PREFIX: &str = "shaders/sm_4_0/gbuffer_pass.ps/";

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
        let gbuffer_projections = gbuffer_projections(target, &materials);
        let shader_catalog_sha256 = shader_catalog_sha256(target);
        Ok(Self {
            materials,
            gbuffer_projections,
            shader_catalog_sha256,
            runtime_proven: BTreeMap::new(),
            null_texture: None,
        })
    }

    /// Bind `name` (a texture the caller emits: all channels zero) wherever
    /// a converted material names a texture that exists in neither archive.
    pub fn set_null_texture(&mut self, name: Option<String>) {
        self.null_texture = name;
    }

    /// Hash of the shipped shader catalogue that runtime evidence must match.
    pub fn shader_catalog_sha256(&self) -> &str {
        &self.shader_catalog_sha256
    }

    /// Accept switch restorations proven by runtime loads where the static
    /// variant proof is too conservative. Each entry must name the shader
    /// catalogue it was observed with; evidence for another catalogue is an
    /// error, since the observed loads would no longer prove anything.
    pub fn set_runtime_proven_switches<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (&'a str, &'a str, &'a BTreeMap<String, i64>)>,
    ) -> Result<()> {
        self.runtime_proven.clear();
        for (material, catalog, switches) in entries {
            if catalog != self.shader_catalog_sha256 {
                bail!(
                    "{material}: runtime switch evidence was observed with shader catalogue {catalog}, not {}",
                    self.shader_catalog_sha256
                );
            }
            if switches.is_empty() || switches.keys().any(|name| !GBUFFER_KEY_SWITCHES.iter().any(|(_, n)| n == name)) {
                bail!("{material}: runtime evidence must name decoded permutation switches");
            }
            self.runtime_proven.insert(material.to_owned(), switches.clone());
        }
        Ok(())
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
            let compatible = |candidate: &&TargetMaterial| {
                candidate.effects == donor_effects
                    && candidate.stage_selectors == donor_stage_selectors
            };
            let closest = self
                .materials
                .iter()
                .filter(compatible)
                .filter(|candidate| semantic_domain_rank(path, &candidate.path).is_some())
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
            let exact = closest;
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
        neutralize_template_water_foam(&mut converted.root, &donor_config.root);
        let switch_notes = restore_source_switches(
            &mut converted.root,
            &donor_config.root,
            self.gbuffer_projections.as_ref(),
            self.runtime_proven.get(path),
        )?;
        restore_factory_lamp_emission(
            path,
            &template.path,
            &donor_config.root,
            &mut converted.root,
            target,
        )?;
        let mut switch_notes = switch_notes;
        if let Some(null) = &self.null_texture {
            switch_notes.extend(bind_missing_textures(
                &mut converted.root,
                &|texture| texture_exists(texture, donor, target),
                null,
            ));
        }
        Ok(ConvertedMaterial {
            bytes: converted.to_bytes()?,
            template_path: template.path.clone(),
            donor_texture_bindings,
            switch_notes,
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

fn restore_factory_lamp_emission(
    path: &str,
    template: &str,
    donor: &ConfigValue,
    converted: &mut ConfigValue,
    target: &ResourceDb,
) -> Result<()> {
    // This source bulb uses a non-emitting native template. The separate
    // emission pass was verified in 0.26 with these two switches restored;
    // copying arbitrary source features can request absent permutations.
    if path != "material_instances/props/light/lamp_greenhouse_clone.material"
        || template != "material_instances/props/light/lamp_greenhouse.material"
    {
        return Ok(());
    }
    let flags = ["use_emissive", "use_emissive_map"];
    let source_effect = donor
        .named("material")
        .and_then(|v| v.named("g_stage"))
        .and_then(|v| v.named("effect"));
    if !flags.iter().all(|flag| {
        source_effect
            .and_then(|v| v.named(flag))
            .and_then(|v| v.named("value"))
            == Some(&ConfigValue::Boolean(true))
    }) {
        return Ok(());
    }
    // Both quality variants of the observed textured-emission shader must
    // exist. Other target builds keep their existing template contract.
    if !["0", "1"].iter().all(|quality| target.is_file(&format!(
        "shaders/sm_4_0/gbuffer_emissive_pass.ps/________0______2_0____1________0_0__________0______0_{quality}_____________"
    ))) {
        return Ok(());
    }
    for flag in flags {
        converted.set_path(
            &format!("material.g_stage.effect.{flag}.value"),
            ConfigValue::Boolean(true),
        )?;
    }
    Ok(())
}
fn effect_value<'a>(root: &'a ConfigValue, stage: &str, name: &str) -> Option<&'a ConfigValue> {
    root.named("material")?
        .named(stage)?
        .named("effect")?
        .named(name)?
        .named("value")
}

fn path_value<'a>(root: &'a ConfigValue, path: &str) -> Option<&'a ConfigValue> {
    path.split('.').try_fold(root, |value, name| value.named(name))
}

#[cfg(test)]
fn effect_switch(root: &ConfigValue, name: &str) -> Option<bool> {
    match effect_value(root, "g_stage", name)? {
        ConfigValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

/// A permutation switch as its key digit value (boolean or small integer).
fn switch_level(root: &ConfigValue, stage: &str, name: &str) -> Option<i64> {
    match effect_value(root, stage, name)? {
        ConfigValue::Boolean(value) => Some(i64::from(*value)),
        ConfigValue::Integer(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn effect_number(root: &ConfigValue, name: &str) -> Option<f32> {
    match effect_value(root, "g_stage", name)? {
        ConfigValue::Float(value) => Some(*value),
        ConfigValue::Integer(value) => Some(*value as f32),
        _ => None,
    }
}
fn key_position(length: usize, position: usize) -> usize {
    if length == 68 && position > 22 { position + 1 } else { position }
}

fn gbuffer_projection(
    root: &ConfigValue,
    overrides: &BTreeMap<&str, i64>,
    quality: char,
) -> Option<String> {
    switch_level(root, "g_stage", "use_tdiffuse")?;
    let mut key = String::new();
    key.push(quality);
    for (_, name) in GBUFFER_KEY_SWITCHES {
        let value = overrides
            .get(name)
            .copied()
            .or_else(|| switch_level(root, "g_stage", name))
            .unwrap_or(0);
        key.push(char::from_digit(u32::try_from(value).ok()?, 10)?);
    }
    Some(key)
}

/// Every shipped shader key of every family (`shaders/sm_4_0/<family>/<key>`).
/// Key characters are positions in one global configuration order shared
/// by all families (checked by the catalog consistency test); `_` marks a
/// configuration the family does not use.
#[derive(Default)]
struct ShippedVariants {
    keys: BTreeMap<String, BTreeSet<Vec<u8>>>,
}

const SHADER_PREFIX: &str = "shaders/sm_4_0/";

fn gbuffer_projections(target: &ResourceDb, materials: &[TargetMaterial]) -> Option<ShippedVariants> {
    let mut shipped = ShippedVariants::default();
    for path in target.paths_with_prefix(SHADER_PREFIX) {
        let Some((family, key)) = path.strip_prefix(SHADER_PREFIX).and_then(|rest| rest.split_once('/')) else {
            continue;
        };
        if matches!(key.len(), 67 | 68) {
            shipped.keys.entry(family.to_owned()).or_default().insert(key.as_bytes().to_vec());
        }
    }
    let gbuffer: BTreeSet<String> = shipped
        .keys
        .get(GBUFFER_PREFIX.trim_start_matches(SHADER_PREFIX).trim_end_matches('/'))?
        .iter()
        .filter_map(|key| {
            let mut projection = String::from(key[GBUFFER_KEY_QUALITY] as char);
            projection.extend(GBUFFER_KEY_SWITCHES.iter().map(|(p, _)| key[key_position(key.len(), *p)] as char));
            projection.chars().all(|c| c.is_ascii_digit()).then_some(projection)
        })
        .collect();
    // Refuse the layout unless every shipped native g-stage material agrees.
    let consistent = !gbuffer.is_empty()
        && materials.iter().all(|material| {
            ['0', '3'].iter().all(|quality| {
                gbuffer_projection(&material.config.root, &BTreeMap::new(), *quality)
                    .is_none_or(|key| gbuffer.contains(&key))
            })
        });
    consistent.then_some(shipped)
}

fn shader_catalog_sha256(target: &ResourceDb) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for path in target.paths_with_prefix(SHADER_PREFIX) {
        hash.update(path.as_bytes());
        hash.update(b"\n");
    }
    format!("{:x}", hash.finalize())
}

/// Whether changing material switches can request an unshipped shader.
///
/// The unconverted material is known to load: every key it requests ships.
/// Any shipped key, of any family, that agrees with the material on every
/// decoded switch digit the family uses could be one of those requests.
/// For each such key the same key with the changed digits must also ship.
/// Geometry, LOD, instancing, vertex-input and undecoded material digits are
/// carried over unchanged, so no assumption about how the material is drawn
/// (static mesh, vegetation instance, skinned, decal...) is needed. This is
/// conservative: a switch is refused if any compatible variant lacks it.
fn variants_ship(shipped: &ShippedVariants, converted: &ConfigValue, overrides: &BTreeMap<&str, i64>) -> bool {
    let current: Vec<(usize, u8, Option<u8>)> = GBUFFER_KEY_SWITCHES
        .iter()
        .map(|(position, name)| {
            let digit = |value: i64| u8::try_from(value).ok().filter(|v| *v <= 9).map(|v| b'0' + v);
            let before = digit(switch_level(converted, "g_stage", name).unwrap_or(0));
            let after = overrides.get(name).copied().and_then(digit);
            (*position, before.unwrap_or(b'?'), after)
        })
        .collect();
    if current.iter().any(|(_, before, after)| *before == b'?' || matches!(after, Some(b'?'))) {
        return false;
    }
    shipped.keys.values().all(|keys| {
        keys.iter().all(|key| {
            let mut changed = key.clone();
            for (position, before, after) in &current {
                let at = key_position(key.len(), *position);
                match key[at] {
                    b'_' => {}
                    digit if digit != *before => return true, // another material's variant
                    _ => {
                        if let Some(after) = after {
                            changed[at] = *after;
                        }
                    }
                }
            }
            changed == *key || keys.contains(&changed)
        })
    })
}

/// Switches that are render state or g-buffer constants rather than shader
/// permutations (none appears in `shaders/masks`). The source value is used;
/// a switch the source does not have is off, as it did not exist there.
const DATA_SWITCHES: [(&str, bool); 2] = [("two_sided", true), ("is_anisotropic_material", true)];
/// Data fields copied only where the source authors them.
const AUTHORED_DATA: [&str; 1] = ["fresnel_material_type"];

/// Texture slots and the switch that makes a shader sample them.
const GATED_TEXTURES: [(&str, &str); 17] = [
    ("texture_bump", "use_parallax"),
    ("texture_vertex_blended_normal", "use_vertex_blended_normal"),
    ("texture_vertex_blended_diffuse", "use_vertex_blended_diffuse"),
    ("texture_vertex_blended_specular", "use_vertex_blended_specular"),
    ("texture_roughness", "use_troughness"),
    ("texture_fresnel", "use_tfresnel"),
    ("texture_specular_intensity", "use_tspecular_intensity"),
    ("texture_specular_power", "use_tspecular_power"),
    ("texture_normal", "use_nmap"),
    ("texture_detail_normal", "use_detail_nmap"),
    ("texture_ditail_diffuse", "use_tditail_diffuse"),
    ("texture_alphablended_normal", "use_alphablended_normal"),
    ("texture_alphablended_diffuse", "use_alphablended_diffuse"),
    ("texture_vertex_blended_tiled_mask", "use_tiled_vertex_blended_mask"),
    ("texture_emissive", "use_emissive_map"),
    ("texture_translucency", "use_ttranslucency"),
    ("normal_waves_texture", "use_normal_waves"),
];

fn stage_names(root: &ConfigValue) -> Vec<String> {
    material_stages(root)
        .map(|stages| stages.iter().map(|stage| stage.name.clone()).collect())
        .unwrap_or_default()
}

/// Restore the donor's switches. Permutation switches of the g-stage are
/// restored as the largest subset with a shipped `gbuffer_pass` permutation;
/// a constant roughness left textured is emulated by a zero-width range.
/// Data switches follow the source directly. Template textures in slots the
/// source left empty are cleared when their switch is off. Anything else
/// keeps the template value and is reported.
fn restore_source_switches(
    converted: &mut ConfigValue,
    donor: &ConfigValue,
    shipped: Option<&ShippedVariants>,
    proven: Option<&BTreeMap<String, i64>>,
) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let mut wanted = BTreeMap::new();
    for (_, name) in GBUFFER_KEY_SWITCHES {
        let Some(native) = switch_level(converted, "g_stage", name) else {
            continue;
        };
        let source = switch_level(donor, "g_stage", name).unwrap_or(0);
        if source != native {
            wanted.insert(name, source);
        }
    }
    // A subset of switches observed loading at runtime (same material, same
    // shader catalogue) is accepted where the static proof is conservative.
    let observed = |overrides: &BTreeMap<&str, i64>| {
        proven.is_some_and(|proven| overrides.iter().all(|(name, level)| proven.get(*name) == Some(level)))
    };
    let supported = |overrides: &BTreeMap<&str, i64>| {
        shipped.is_some_and(|shipped| variants_ship(shipped, converted, overrides)) || observed(overrides)
    };
    // Restore the largest subset of donor switches with a shipped
    // permutation (subsets are bounded: at most a few switches differ).
    let names: Vec<&str> = wanted.keys().copied().collect();
    let mut applied = BTreeMap::new();
    if names.len() <= 12 {
        for mask in (1..1usize << names.len()).rev() {
            let subset: BTreeMap<&str, i64> = names
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, name)| (*name, wanted[name]))
                .collect();
            if subset.len() > applied.len() && supported(&subset) {
                applied = subset;
            }
        }
    }
    let proof = if applied.is_empty() || shipped.is_some_and(|shipped| variants_ship(shipped, converted, &applied)) {
        "shipped permutation"
    } else {
        "runtime-proven load"
    };
    for (name, value) in &applied {
        let current = effect_value(converted, "g_stage", name).cloned();
        let replacement = match current {
            Some(ConfigValue::Integer(_)) => ConfigValue::Integer(*value as i32),
            _ => ConfigValue::Boolean(*value != 0),
        };
        converted.set_path(&format!("material.g_stage.effect.{name}.value"), replacement)?;
        notes.push(format!("{name}={value} restored from source ({proof})"));
    }
    for (name, value) in &wanted {
        if applied.contains_key(name) {
            continue;
        }
        let emulated = *name == "use_troughness"
            && *value == 0
            && effect_number(converted, "constant_roughness_min").is_some()
            && effect_number(converted, "constant_roughness_max").is_some();
        match effect_number(converted, "constant_roughness").filter(|_| emulated) {
            Some(constant) => {
                for field in ["constant_roughness_min", "constant_roughness_max"] {
                    converted.set_path(
                        &format!("material.g_stage.effect.{field}.value"),
                        ConfigValue::Float(constant),
                    )?;
                }
                notes.push(format!(
                    "use_troughness=false emulated by zero-width textured range at constant {constant}"
                ));
            }
            None => notes.push(format!(
                "{name}={value} unresolved: no shipped gbuffer permutation; template value kept"
            )),
        }
    }
    for stage in stage_names(converted) {
        for (name, absent_is_off) in DATA_SWITCHES {
            // These live either in the effect block or on the stage itself.
            for prefix in [format!("material.{stage}.effect"), format!("material.{stage}")] {
                let path = format!("{prefix}.{name}.value");
                let Some(ConfigValue::Boolean(native)) = path_value(converted, &path).cloned() else {
                    continue;
                };
                let source = match path_value(donor, &path) {
                    Some(ConfigValue::Boolean(value)) => *value,
                    None if absent_is_off => false,
                    _ => continue,
                };
                if source != native {
                    converted.set_path(&path, ConfigValue::Boolean(source))?;
                    notes.push(format!("{path}={source} taken from source (render state)"));
                }
            }
        }
        for name in AUTHORED_DATA {
            if let (Some(native), Some(source)) = (
                effect_value(converted, &stage, name).cloned(),
                effect_value(donor, &stage, name).cloned(),
            ) {
                if native != source && std::mem::discriminant(&native) == std::mem::discriminant(&source) {
                    converted.set_path(&format!("material.{stage}.effect.{name}.value"), source)?;
                    notes.push(format!("{stage}.{name} taken from source"));
                }
            }
        }
        for (texture, gate) in GATED_TEXTURES {
            let Some(ConfigValue::String(native)) = effect_value(converted, &stage, texture).cloned() else {
                continue;
            };
            let Some(ConfigValue::String(source)) = effect_value(donor, &stage, texture) else {
                continue;
            };
            if !source.is_empty() || native.is_empty() {
                continue;
            }
            if switch_level(converted, &stage, gate).unwrap_or(0) == 0 {
                converted.set_path(
                    &format!("material.{stage}.effect.{texture}.value"),
                    ConfigValue::String(String::new()),
                )?;
                notes.push(format!("{stage}.{texture} cleared: empty in source and {gate} is off"));
            } else {
                notes.push(format!(
                    "{stage}.{texture}={native} kept from template: empty in source but {gate} is on"
                ));
            }
        }
    }
    // Shadow casting is a g-stage property outside the effect block.
    if let (Some(ConfigValue::Boolean(native)), Some(ConfigValue::Boolean(source))) = (
        converted
            .named("material")
            .and_then(|m| m.named("g_stage"))
            .and_then(|s| s.named("is_cast_shadow"))
            .and_then(|v| v.named("value"))
            .cloned(),
        donor
            .named("material")
            .and_then(|m| m.named("g_stage"))
            .and_then(|s| s.named("is_cast_shadow"))
            .and_then(|v| v.named("value")),
    ) {
        // Enabling shadows would request shadow-pass shaders this material
        // never loaded; only removing a template-added caster is provably safe.
        if native && !*source {
            converted.set_path(
                "material.g_stage.is_cast_shadow.value",
                ConfigValue::Boolean(false),
            )?;
            notes.push("g_stage.is_cast_shadow=false taken from source".to_owned());
        } else if !native && *source {
            notes.push("g_stage.is_cast_shadow=true unresolved: enabling shadows needs unproven shadow-pass shaders".to_owned());
        }
    }
    Ok(notes)
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
        let donor_subdomain = donor_parts.get(1).filter(|_| donor_parts.len() > 2);
        let target_subdomain = target_parts.get(1).filter(|_| target_parts.len() > 2);
        if donor_subdomain == target_subdomain {
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

// Early simple water has no foam layer. Newer templates add one at full
// intensity; retaining that default paints bright foam over the old puddles.
fn neutralize_template_water_foam(converted: &mut ConfigValue, donor: &ConfigValue) {
    fn water_effect(root: &ConfigValue) -> Option<&ConfigValue> {
        root.named("material")
            .and_then(|v| v.named("forward"))
            .and_then(|v| v.named("effect"))
    }
    let Some(source) = water_effect(donor) else {
        return;
    };
    let Some(target) = water_effect(converted) else {
        return;
    };
    let is_water = |effect: &ConfigValue| matches!(effect.named("effect_id"), Some(ConfigValue::String(id)) if id == "forward_default_simpe_water");
    if is_water(source) && is_water(target) && source.named("foam_intensity").is_none() {
        if let Some(value) = converted
            .named_mut("material")
            .and_then(|v| v.named_mut("forward"))
            .and_then(|v| v.named_mut("effect"))
            .and_then(|v| v.named_mut("foam_intensity"))
            .and_then(|v| v.named_mut("value"))
        {
            *value = ConfigValue::Integer(0);
        }
    }
}

/// The original renderer leaves a texture that fails to load without a
/// resource (0.10b `resource_manager::on_texture_loaded` logs "Texture %s was
/// not found!" and returns), so its shaders sample a null view: zero in every
/// channel. 0.26 binds a "missing texture" lettering placeholder instead,
/// which as a normal map draws text and grazing-angle moire rings on floors
/// (retail and source `tiled/floor/asphalt` name a nonexistent
/// `asphalt_nmap` with `use_nmap` on). Such references are rebound to an
/// all-zero texture, reproducing what the original shaders read.
fn bind_missing_textures(converted: &mut ConfigValue, exists: &dyn Fn(&str) -> bool, null: &str) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(ConfigValue::Named(stages)) = converted.named_mut("material") else {
        return notes;
    };
    for stage in stages {
        let Some(ConfigValue::Named(entries)) = stage.value.named_mut("effect") else {
            continue;
        };
        for entry in entries {
            if !is_texture_parameter(&entry.name) {
                continue;
            }
            let value = match &mut entry.value {
                ConfigValue::String(value) => Some(value),
                other => match other.named_mut("value") {
                    Some(ConfigValue::String(value)) => Some(value),
                    _ => None,
                },
            };
            if let Some(value) = value {
                if !value.is_empty() && !exists(value) {
                    notes.push(format!(
                        "{}.{}: missing texture {value:?} bound to the all-zero texture (original null view)",
                        stage.name, entry.name
                    ));
                    *value = null.to_owned();
                }
            }
        }
    }
    notes
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
    fn water_templates_do_not_add_foam_absent_from_the_source() {
        let material = |effect: &str, foam: Option<f32>| {
            let mut values = vec![named("effect_id", ConfigValue::String(effect.into()))];
            if let Some(foam) = foam {
                values.push(named(
                    "foam_intensity",
                    ConfigValue::Named(vec![named("value", ConfigValue::Float(foam))]),
                ));
            }
            ConfigValue::Named(vec![named(
                "material",
                ConfigValue::Named(vec![named(
                    "forward",
                    ConfigValue::Named(vec![named("effect", ConfigValue::Named(values))]),
                )]),
            )])
        };
        let water = "forward_default_simpe_water";
        let mut converted = material(water, Some(1.0));
        neutralize_template_water_foam(&mut converted, &material(water, None));
        let mut expected = material(water, Some(1.0));
        expected
            .set_path(
                "material.forward.effect.foam_intensity.value",
                ConfigValue::Integer(0),
            )
            .unwrap();
        assert_eq!(converted, expected);

        // Authored foam, unrelated effects and older targets remain untouched.
        for (target, donor) in [
            (material(water, Some(0.3)), material(water, Some(0.3))),
            (
                material("forward_default", Some(1.0)),
                material("forward_default", None),
            ),
            (
                material(water, Some(1.0)),
                material("forward_default", None),
            ),
            (material(water, None), material(water, None)),
        ] {
            let mut actual = target.clone();
            neutralize_template_water_foam(&mut actual, &donor);
            assert_eq!(actual, target);
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

    fn gloss_material(troughness: bool, tspecular: bool, tfresnel: bool) -> Vec<u8> {
        let parameter = |value| ConfigValue::Named(vec![named("value", value)]);
        let switch = |name: &str, value| named(name, parameter(ConfigValue::Boolean(value)));
        let number = |name: &str, value| named(name, parameter(ConfigValue::Float(value)));
        let effect = ConfigValue::Named(vec![
            named("effect_id", ConfigValue::String("default".into())),
            switch("use_tdiffuse", true),
            switch("use_nmap", true),
            switch("use_alpha_test", false),
            switch("use_troughness", troughness),
            switch("use_tspecular_intensity", tspecular),
            switch("use_tfresnel", tfresnel),
            number("constant_roughness", 0.5),
            number("constant_roughness_min", 0.1),
            number("constant_roughness_max", 0.2),
        ]);
        BinaryConfig {
            root: ConfigValue::Named(vec![named(
                "material",
                ConfigValue::Named(vec![named(
                    "g_stage",
                    ConfigValue::Named(vec![named("effect", effect)]),
                )]),
            )]),
        }
        .to_bytes()
        .unwrap()
    }

    fn gbuffer_key(quality: char, troughness: bool, tspecular: bool, tfresnel: bool) -> String {
        let mut key: Vec<char> = "_0___0000_00000_00____0_____0__0_00000____00000____000__________0__"
            .chars()
            .collect();
        key[GBUFFER_KEY_QUALITY] = quality;
        let digit = |value: bool| if value { '1' } else { '0' };
        key[5] = '1';
        key[6] = '1';
        key[12] = digit(tspecular);
        key[13] = digit(tfresnel);
        key[14] = digit(troughness);
        key.into_iter().collect()
    }

    fn gloss_catalog(
        templates: &[(&str, Vec<u8>)],
        shipped: &[(bool, bool, bool)],
    ) -> (tempfile::TempDir, ResourceDb) {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        for (path, bytes) in templates {
            let file = input.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let shaders = input.join(GBUFFER_PREFIX.trim_end_matches('/'));
        std::fs::create_dir_all(&shaders).unwrap();
        for (troughness, tspecular, tfresnel) in shipped {
            for quality in ['0', '3'] {
                std::fs::write(
                    shaders.join(gbuffer_key(quality, *troughness, *tspecular, *tfresnel)),
                    b"DXBC",
                )
                .unwrap();
            }
        }
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let db = ResourceDb::open(archive).unwrap();
        (temp, db)
    }

    #[test]
    fn restores_source_gloss_switches_from_shipped_permutations() {
        let effect = |bytes: &[u8], name: &str| {
            let root = BinaryConfig::parse(bytes).unwrap().root;
            (effect_switch(&root, name), effect_number(&root, name))
        };
        // Remains-like donor: constant roughness .5 with an intensity mask.
        let donor = gloss_material(false, true, false);
        let template = "material_instances/characters/human/remains/rag.material";
        let (_temp, mut db) = gloss_catalog(
            &[(template, gloss_material(true, false, false))],
            &[(true, false, false), (true, true, false)],
        );
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        assert!(catalog.gbuffer_projections.is_some());
        let path = "material_instances/characters/human/remains/skeleton.material";
        let converted = catalog.convert(path, &donor, None, &db, &db).unwrap();
        assert_eq!(converted.template_path, template);
        // The mask permutation ships only with textured roughness, so the
        // donor constant becomes an exact zero-width range.
        assert_eq!(effect(&converted.bytes, "use_tspecular_intensity").0, Some(true));
        assert_eq!(effect(&converted.bytes, "use_troughness").0, Some(true));
        assert_eq!(effect(&converted.bytes, "constant_roughness_min").1, Some(0.5));
        assert_eq!(effect(&converted.bytes, "constant_roughness_max").1, Some(0.5));
        assert_eq!(converted.switch_notes.len(), 2, "{:?}", converted.switch_notes);

        // Without any shipped mask permutation the template value stays and
        // the loss is reported instead of requesting an absent shader.
        let (_temp, mut db) = gloss_catalog(
            &[(template, gloss_material(true, false, false))],
            &[(true, false, false)],
        );
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        let converted = catalog.convert(path, &donor, None, &db, &db).unwrap();
        assert_eq!(effect(&converted.bytes, "use_tspecular_intensity").0, Some(false));
        assert!(converted.switch_notes.iter().any(|note| note.contains("unresolved")));
        assert_eq!(effect(&converted.bytes, "constant_roughness_max").1, Some(0.5));

        // The closest template stays; its lost intensity mask is restored
        // because that variant ships for the same geometry.
        let metal = gloss_material(true, true, true);
        let closer = "material_instances/tiled/metal/metal_column.material";
        let (_temp, mut db) = gloss_catalog(
            &[(closer, gloss_material(true, false, true))],
            &[(true, false, true), (true, true, true)],
        );
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        let converted = catalog.convert(closer, &metal, None, &db, &db).unwrap();
        assert_eq!(converted.template_path, closer);
        assert_eq!(effect(&converted.bytes, "use_tspecular_intensity").0, Some(true));
    }

    #[test]
    fn refuses_a_switch_whose_vertex_format_variant_does_not_ship() {
        // Regression for the 2026-10-15 Chemical crash: a wind-animated tree
        // material (translucency, wind 2) drawn with vertex input type 1. Its
        // intensity-mask switch exists in other variants, but not for input
        // type 1, so restoring it requested an unshipped gbuffer_pass key.
        let parameter = |value| ConfigValue::Named(vec![named("value", value)]);
        let leaves = |tspecular: bool| {
            BinaryConfig {
                root: ConfigValue::Named(vec![named(
                    "material",
                    ConfigValue::Named(vec![named(
                        "g_stage",
                        ConfigValue::Named(vec![named(
                            "effect",
                            ConfigValue::Named(vec![
                                named("effect_id", ConfigValue::String("default".into())),
                                named("use_tdiffuse", parameter(ConfigValue::Boolean(true))),
                                named("use_nmap", parameter(ConfigValue::Boolean(true))),
                                named("use_alpha_test", parameter(ConfigValue::Boolean(true))),
                                named("use_ttranslucency", parameter(ConfigValue::Boolean(true))),
                                named("use_tspecular_intensity", parameter(ConfigValue::Boolean(tspecular))),
                                named("use_troughness", parameter(ConfigValue::Boolean(true))),
                                named("wind_motion", parameter(ConfigValue::Integer(2))),
                            ]),
                        )]),
                    )]),
                )]),
            }
            .to_bytes()
            .unwrap()
        };
        let key = |quality: char, tspecular: char, input: char| {
            let mut key: Vec<char> =
                "_0___1101_01001_00____0_____0__0_00000____00000____002__________0__".chars().collect();
            key[GBUFFER_KEY_QUALITY] = quality;
            key[12] = tspecular;
            key[22] = input;
            key.into_iter().collect::<String>()
        };
        let template = "material_instances/flora/poplar_leaves_01.material";
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let file = input.join(template);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, leaves(false)).unwrap();
        let shaders = input.join(GBUFFER_PREFIX.trim_end_matches('/'));
        std::fs::create_dir_all(&shaders).unwrap();
        for quality in ['0', '3'] {
            for name in [key(quality, '0', '1'), key(quality, '1', '0'), key(quality, '0', '0')] {
                std::fs::write(shaders.join(name), b"DXBC").unwrap();
            }
        }
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let mut db = ResourceDb::open(archive).unwrap();
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        let converted = catalog
            .convert(template, &leaves(true), Some(template), &db, &db)
            .unwrap();
        let root = BinaryConfig::parse(&converted.bytes).unwrap().root;
        assert_eq!(effect_switch(&root, "use_tspecular_intensity"), Some(false));
        assert!(converted.switch_notes.iter().any(|note| note.contains("unresolved")));
    }

    #[test]
    fn restores_every_source_switch_and_clears_unused_template_textures() {
        let parameter = |value| ConfigValue::Named(vec![named("value", value)]);
        let material = |alpha: bool, two_sided: Option<bool>, roughness_texture: &str, shadow: bool| {
            let mut effect = vec![
                named("effect_id", ConfigValue::String("default".into())),
                named("use_tdiffuse", parameter(ConfigValue::Boolean(true))),
                named("use_nmap", parameter(ConfigValue::Boolean(true))),
                named("use_alpha_test", parameter(ConfigValue::Boolean(alpha))),
                named("use_troughness", parameter(ConfigValue::Boolean(false))),
                named("texture_roughness", parameter(ConfigValue::String(roughness_texture.into()))),
            ];
            if let Some(value) = two_sided {
                effect.push(named("two_sided", parameter(ConfigValue::Boolean(value))));
            }
            BinaryConfig {
                root: ConfigValue::Named(vec![named(
                    "material",
                    ConfigValue::Named(vec![named(
                        "g_stage",
                        ConfigValue::Named(vec![
                            named("is_cast_shadow", parameter(ConfigValue::Boolean(shadow))),
                            named("effect", ConfigValue::Named(effect)),
                        ]),
                    )]),
                )]),
            }
            .to_bytes()
            .unwrap()
        };
        // Template: alpha test on, two-sided, a successor roughness map, no
        // shadow. Source: opaque, single-sided (absent), empty roughness slot.
        let template = "material_instances/props/bag/sandbags.material";
        let donor = material(false, None, "", true);
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let file = input.join(template);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, material(true, Some(true), "props/bag/sandbags_smap", false)).unwrap();
        let shaders = input.join(GBUFFER_PREFIX.trim_end_matches('/'));
        std::fs::create_dir_all(&shaders).unwrap();
        for alpha in ['0', '1'] {
            for quality in ['0', '3'] {
                let mut key: Vec<char> = gbuffer_key(quality, false, false, false).chars().collect();
                key[8] = alpha;
                std::fs::write(shaders.join(key.into_iter().collect::<String>()), b"DXBC").unwrap();
            }
        }
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let mut db = ResourceDb::open(archive).unwrap();
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        // Same-named native successors are explicit templates (0.20e path).
        let converted = catalog
            .convert(template, &donor, Some(template), &db, &db)
            .unwrap();
        let root = BinaryConfig::parse(&converted.bytes).unwrap().root;
        assert_eq!(effect_switch(&root, "use_alpha_test"), Some(false));
        assert_eq!(effect_switch(&root, "two_sided"), Some(false));
        assert_eq!(
            effect_value(&root, "g_stage", "texture_roughness"),
            Some(&ConfigValue::String(String::new()))
        );
        // Enabling a shadow caster would request unproven shadow-pass shaders.
        assert_eq!(
            path_value(&root, "material.g_stage.is_cast_shadow.value"),
            Some(&ConfigValue::Boolean(false))
        );
        assert!(converted.switch_notes.iter().any(|note| note.contains("is_cast_shadow=true unresolved")));
        assert_eq!(converted.switch_notes.len(), 4, "{:?}", converted.switch_notes);
    }

    #[test]
    fn keeps_vegetation_vertex_input_variants_when_restoring_a_switch() {
        // Regression for the second 2026-10-15 crash: grass_l04 is drawn by
        // vegetation instances (vertex input 11, a 68-character key). Its
        // constant roughness had been restored by flipping use_troughness,
        // whose variant ships only for mesh inputs 0/1. The zero-width range
        // must be used instead.
        let donor = gloss_material(false, false, false);
        let template = "material_instances/flora/grass_l04.material";
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let file = input.join(template);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, gloss_material(true, false, false)).unwrap();
        let shaders = input.join(GBUFFER_PREFIX.trim_end_matches('/'));
        std::fs::create_dir_all(&shaders).unwrap();
        for quality in ['0', '3'] {
            for troughness in [false, true] {
                std::fs::write(shaders.join(gbuffer_key(quality, troughness, false, false)), b"DXBC").unwrap();
            }
            // Vegetation instance variant: two-digit vertex input at 22..24.
            let mut key: Vec<char> = gbuffer_key(quality, true, false, false).chars().collect();
            key.splice(22..23, ['1', '1']);
            std::fs::write(shaders.join(key.into_iter().collect::<String>()), b"DXBC").unwrap();
        }
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let mut db = ResourceDb::open(archive).unwrap();
        let catalog = MaterialCatalog::load(&mut db).unwrap();
        let converted = catalog.convert(template, &donor, Some(template), &db, &db).unwrap();
        let root = BinaryConfig::parse(&converted.bytes).unwrap().root;
        assert_eq!(effect_switch(&root, "use_troughness"), Some(true));
        assert_eq!(effect_number(&root, "constant_roughness_min"), Some(0.5));
        assert_eq!(effect_number(&root, "constant_roughness_max"), Some(0.5));
    }

    #[test]
    fn missing_textures_are_bound_to_the_null_texture() {
        // Retail and source asphalt name a nonexistent asphalt_nmap with
        // use_nmap on; 0.26 would draw its lettered placeholder.
        let value = |text: &str| ConfigValue::Named(vec![named("value", ConfigValue::String(text.into()))]);
        let mut root = ConfigValue::Named(vec![named(
            "material",
            ConfigValue::Named(vec![named(
                "g_stage",
                ConfigValue::Named(vec![named(
                    "effect",
                    ConfigValue::Named(vec![
                        named("texture_normal", value("tiled/floor/asphalt_nmap")),
                        named("texture_diffuse", value("tiled/floor/asphalt")),
                        named("texture_bump", value("")),
                        named("effect_id", ConfigValue::String("gbuffer_pass".into())),
                    ]),
                )]),
            )]),
        )]);
        let notes = bind_missing_textures(&mut root, &|t| t == "tiled/floor/asphalt", "ported/v011e/srp/null");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(effect_value(&root, "g_stage", "texture_normal"), Some(&ConfigValue::String("ported/v011e/srp/null".into())));
        assert_eq!(effect_value(&root, "g_stage", "texture_diffuse"), Some(&ConfigValue::String("tiled/floor/asphalt".into())));
        assert_eq!(effect_value(&root, "g_stage", "texture_bump"), Some(&ConfigValue::String(String::new())));
    }

    #[test]
    fn runtime_evidence_restores_a_switch_the_static_proof_refuses() {
        // The skeleton/rag remains repair (use_tspecular_intensity) loaded
        // in user sessions, but some shipped variant of another vertex input
        // lacks the switch, so the static proof refuses it. Evidence pinned
        // to the shader catalogue restores exactly the observed switches;
        // evidence for another catalogue is rejected.
        let donor = gloss_material(false, false, false);
        let template = "material_instances/flora/grass_l04.material";
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let file = input.join(template);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, gloss_material(true, false, false)).unwrap();
        let shaders = input.join(GBUFFER_PREFIX.trim_end_matches('/'));
        std::fs::create_dir_all(&shaders).unwrap();
        for quality in ['0', '3'] {
            for troughness in [false, true] {
                std::fs::write(shaders.join(gbuffer_key(quality, troughness, false, false)), b"DXBC").unwrap();
            }
            let mut key: Vec<char> = gbuffer_key(quality, true, false, false).chars().collect();
            key.splice(22..23, ['1', '1']);
            std::fs::write(shaders.join(key.into_iter().collect::<String>()), b"DXBC").unwrap();
        }
        let archive = temp.path().join("materials.db");
        resources_db::pack::assemble_to_file(
            &resources_db::pack::build_src_tree(&input).unwrap(),
            &archive,
        )
        .unwrap();
        let mut db = ResourceDb::open(archive).unwrap();
        let mut catalog = MaterialCatalog::load(&mut db).unwrap();
        let proven = BTreeMap::from([("use_troughness".to_owned(), 0)]);
        assert!(catalog.set_runtime_proven_switches([(template, "stale", &proven)]).is_err());
        let hash = catalog.shader_catalog_sha256().to_owned();
        catalog.set_runtime_proven_switches([(template, hash.as_str(), &proven)]).unwrap();
        let converted = catalog.convert(template, &donor, Some(template), &db, &db).unwrap();
        let root = BinaryConfig::parse(&converted.bytes).unwrap().root;
        assert_eq!(effect_switch(&root, "use_troughness"), Some(false));
        assert!(converted.switch_notes.iter().any(|note| note.contains("use_troughness=0 restored from source (runtime-proven load)")));
        // Other materials keep the static proof.
        let other = "material_instances/flora/grass_l05.material";
        assert!(catalog.runtime_proven.get(other).is_none());
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
        let different_features = BinaryConfig { root: config }.to_bytes().unwrap();
        std::fs::write(input.join(closer), &different_features).unwrap();
        // A filename directly under flora is not a semantic subdirectory.
        std::fs::create_dir_all(input.join("material_instances/flora")).unwrap();
        let flat_closer = "material_instances/flora/big_oak_tile.material";
        let flat_matching = "material_instances/flora/big_oak_tile_rust.material";
        std::fs::write(input.join(flat_closer), &different_features).unwrap();
        std::fs::write(input.join(flat_matching), &donor).unwrap();
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
            catalog
                .convert(flat_closer, &donor, None, &db, &db)
                .unwrap()
                .template_path,
            flat_matching,
        );
        assert_eq!(
            catalog.rank_templates(flat_closer, &donor).unwrap()[0].path,
            flat_matching
        );
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
