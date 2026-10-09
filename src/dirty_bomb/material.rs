//! Resolve cooked UE3 materials and extract their diffuse `Texture2D` data.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::soc::build::TexturePlan;
use crate::soc::convert::render_material::{MaterialPlan, RenderFamily};
use crate::soc::manifest::SocMapManifest;
use crate::soc::source::{self, DdsInfo};

use super::package::{ExportEntry, Package, Property};
use super::source::DirtyBombSource;

const BULKDATA_STORE_IN_SEPARATE_FILE: u32 = 1 << 0;
const BULKDATA_SERIALIZE_COMPRESSED_ZLIB: u32 = 1 << 1;
const BULKDATA_SERIALIZE_COMPRESSED_LZO: u32 = 1 << 4;
const BULKDATA_UNUSED: u32 = 1 << 5;
const BULKDATA_SERIALIZE_COMPRESSED_LZX: u32 = 1 << 7;
/// Keep the first test bundle comfortably below the memory used by retaining
/// hundreds of 2K/4K source textures alongside the fully expanded map mesh.
const MAX_TEXTURE_DIMENSION: u32 = 1_024;
/// Preserve enough texels for the most densely tiled source pylon layer. A
/// 1024-pixel atlas made the authored 9x repeat only ~114 pixels wide and was
/// visibly coarse in game; 4096 keeps the effective layer density close to
/// the cooked source while remaining within UE3/Vostok-era texture limits.
const LAYERED_COMPOSITE_DIMENSION: u32 = 4_096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayeredDiffusePlan {
    pub source_diffuses: [String; 3],
    pub source_mask: String,
    pub tiles: [i32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MaterialAssetReport {
    pub used_materials: usize,
    pub resolved_materials: usize,
    pub diffuse_bound_materials: usize,
    pub emitted_textures: usize,
    pub material_failures: BTreeMap<String, String>,
    /// Dangling cooked material imports and the reviewed exported materials
    /// used in their place.
    pub material_aliases_used: BTreeMap<String, String>,
    /// Manifest-reviewed replacements for an ambiguous automatically selected
    /// diffuse or opacity parameter. Retained separately from the effective
    /// bindings so every manual decision remains searchable in the report.
    pub material_diffuse_overrides_used: BTreeMap<String, String>,
    pub material_opacity_overrides_used: BTreeMap<String, String>,
    pub material_family_overrides_used: BTreeMap<String, RenderFamily>,
    pub texture_failures: BTreeMap<String, String>,
    /// Fully resolved texture parameters after applying the UE3 instance
    /// parent chain. Retained in the build record so layered-material mistakes
    /// can be audited without re-decoding the cooked package graph.
    pub material_textures: BTreeMap<String, BTreeMap<String, String>>,
    /// Material which most recently authored each resolved texture parameter.
    /// This distinguishes a deliberate instance override from an unrelated
    /// default inherited through a broad UE3 master material.
    pub material_texture_sources: BTreeMap<String, BTreeMap<String, String>>,
    /// Fully resolved scalar parameters, primarily retained for reconstructing
    /// tiled/layered UE3 materials without guessing their repeat counts.
    pub material_scalars: BTreeMap<String, BTreeMap<String, f32>>,
    pub material_scalar_sources: BTreeMap<String, BTreeMap<String, String>>,
    pub material_vectors: BTreeMap<String, BTreeMap<String, [f32; 4]>>,
    pub material_vector_sources: BTreeMap<String, BTreeMap<String, String>>,
    pub material_constant_colours: BTreeMap<String, [u8; 4]>,
    pub material_diffuse: BTreeMap<String, String>,
    pub material_diffuse_parameters: BTreeMap<String, String>,
    /// Texture actually bound after opacity composition.
    pub material_bound_diffuse: BTreeMap<String, String>,
    pub material_opacity: BTreeMap<String, String>,
    pub material_opacity_parameters: BTreeMap<String, String>,
    pub material_blend_modes: BTreeMap<String, u8>,
    pub material_families: BTreeMap<String, RenderFamily>,
    /// UE3 channel-pack materials baked into an ordinary high-resolution
    /// diffuse atlas. The plan records the source contract for auditability.
    pub layered_diffuse_materials: BTreeMap<String, LayeredDiffusePlan>,
    /// Opacity defaults inherited from a different material than the selected
    /// diffuse override. These are intentionally not composited: the foliage
    /// master, for example, leaves its dead-tree mask on unrelated grass
    /// children whose own DXT5 atlases already contain alpha.
    pub skipped_inherited_opacity: BTreeMap<String, String>,
    pub composited_opacity_materials: usize,
    pub texture_parameter_names: BTreeMap<String, usize>,
}

pub(crate) struct PlannedMaterialAssets {
    pub(crate) materials: Vec<MaterialPlan>,
    pub(crate) textures: Vec<TexturePlan>,
    pub(crate) report: MaterialAssetReport,
}

#[derive(Clone, Copy, Debug)]
struct ObjectLocation {
    package: usize,
    index: i32,
}

#[derive(Clone, Debug, Default)]
struct ResolvedMaterial {
    textures: BTreeMap<String, String>,
    texture_sources: BTreeMap<String, String>,
    scalars: BTreeMap<String, f32>,
    scalar_sources: BTreeMap<String, String>,
    vectors: BTreeMap<String, [f32; 4]>,
    vector_sources: BTreeMap<String, String>,
    blend_mode: u8,
}

struct PackageGraph {
    packages: Vec<Package>,
    objects: HashMap<String, Vec<ObjectLocation>>,
}

impl PackageGraph {
    fn read(source: &DirtyBombSource, map: &str) -> Result<Self> {
        let mut packages = Vec::new();
        for path in source.map_packages(map)? {
            let label = source.relative_path(&path);
            packages.push(
                Package::read(&path)
                    .with_context(|| format!("decoding material package {label}"))?,
            );
        }
        let mut objects: HashMap<String, Vec<ObjectLocation>> = HashMap::new();
        for (package_index, package) in packages.iter().enumerate() {
            for export_index in 1..=package.exports.len() {
                let index = i32::try_from(export_index)?;
                let path = package.object_path(index)?;
                objects.entry(path).or_default().push(ObjectLocation {
                    package: package_index,
                    index,
                });
            }
        }
        Ok(Self { packages, objects })
    }

    fn locate(&self, path: &str) -> Option<ObjectLocation> {
        self.objects.get(path).and_then(|locations| {
            locations
                .iter()
                .copied()
                .find(|location| {
                    matches!(
                        self.export(*location).class_name.as_str(),
                        "Material"
                            | "MaterialInstanceConstant"
                            | "MaterialInstanceTimeVarying"
                            | "Texture2D"
                            | "DecalMaterial"
                    )
                })
                .or_else(|| locations.first().copied())
        })
    }

    fn export(&self, location: ObjectLocation) -> &ExportEntry {
        &self.packages[location.package].exports[location.index as usize - 1]
    }

    fn resolve_reference(
        &self,
        owner: ObjectLocation,
        index: i32,
    ) -> Result<Option<ObjectLocation>> {
        if index == 0 {
            return Ok(None);
        }
        if index > 0 {
            return Ok(Some(ObjectLocation {
                package: owner.package,
                index,
            }));
        }
        let path = self.packages[owner.package].object_path(index)?;
        Ok(self.locate(&path))
    }

    fn reference_path(&self, owner: ObjectLocation, index: i32) -> Result<Option<String>> {
        if index == 0 {
            return Ok(None);
        }
        Ok(Some(self.packages[owner.package].object_path(index)?))
    }
}

pub(crate) fn plan_assets(
    source: &DirtyBombSource,
    manifest: &SocMapManifest,
    used_materials: &BTreeMap<String, u16>,
    surface_counts: &BTreeMap<u16, usize>,
) -> Result<PlannedMaterialAssets> {
    let graph = PackageGraph::read(source, &manifest.source_level)?;
    let mut report = MaterialAssetReport {
        used_materials: used_materials.len(),
        ..MaterialAssetReport::default()
    };
    let mut cache = HashMap::new();
    let mut plans = Vec::with_capacity(used_materials.len());
    let mut wanted_textures = BTreeSet::new();
    let mut opacity_composites = BTreeMap::new();
    let mut layered_composites = BTreeMap::new();
    let mut constant_colours = BTreeMap::new();

    for (material_path, shader_id) in used_materials {
        let effective_material_path = material_alias(material_path, &manifest.material_aliases);
        if effective_material_path != material_path {
            report
                .material_aliases_used
                .insert(material_path.clone(), effective_material_path.to_owned());
        }
        let resolved = graph.locate(effective_material_path).map(|location| {
            resolve_material(
                &graph,
                location,
                &mut cache,
                &mut BTreeSet::new(),
                &mut report,
            )
        });
        let resolved = match resolved {
            Some(Ok(resolved)) => {
                report.resolved_materials += 1;
                Some(resolved)
            }
            Some(Err(error)) => {
                report
                    .material_failures
                    .insert(material_path.clone(), error.to_string());
                None
            }
            None => {
                report.material_failures.insert(
                    material_path.clone(),
                    if effective_material_path == material_path {
                        "material is imported but not exported by this streamed map graph".to_owned()
                    } else {
                        format!(
                            "material alias {effective_material_path} is not exported by this streamed map graph"
                        )
                    },
                );
                None
            }
        };
        if let Some(material) = &resolved {
            report
                .material_textures
                .insert(material_path.clone(), material.textures.clone());
            report
                .material_texture_sources
                .insert(material_path.clone(), material.texture_sources.clone());
            report
                .material_scalars
                .insert(material_path.clone(), material.scalars.clone());
            report
                .material_scalar_sources
                .insert(material_path.clone(), material.scalar_sources.clone());
            report
                .material_vectors
                .insert(material_path.clone(), material.vectors.clone());
            report
                .material_vector_sources
                .insert(material_path.clone(), material.vector_sources.clone());
            report
                .material_blend_modes
                .insert(material_path.clone(), material.blend_mode);
        }
        let diffuse_candidates = resolved
            .as_ref()
            .map(|material| ranked_diffuses(&material.textures))
            .unwrap_or_default();
        let diffuse_override =
            reviewed_override(material_path, &manifest.material_diffuse_overrides);
        let opacity_override =
            reviewed_override(material_path, &manifest.material_opacity_overrides);
        if let Some(texture) = diffuse_override {
            report
                .material_diffuse_overrides_used
                .insert(material_path.clone(), texture.to_owned());
        }
        if let Some(texture) = opacity_override {
            report
                .material_opacity_overrides_used
                .insert(material_path.clone(), texture.to_owned());
        }
        let selected_diffuse = diffuse_override
            .map(|texture| ("manifest_override", texture))
            .or_else(|| diffuse_candidates.first().copied());
        let selected_opacity = opacity_override
            .map(|texture| ("manifest_override", texture))
            .or_else(|| {
                resolved
                    .as_ref()
                    .and_then(|material| select_opacity(&material.textures))
            });
        let mut source_diffuse = selected_diffuse.map(|(_, texture)| texture.to_owned());
        if source_diffuse.is_none()
            && let Some(colour) = resolved
                .as_ref()
                .and_then(|material| select_constant_colour(material_path, &material.vectors))
                .or_else(|| named_material_colour(material_path))
        {
            let name = format!("{material_path}__constant_colour");
            constant_colours.insert(name.clone(), colour);
            report
                .material_constant_colours
                .insert(material_path.clone(), colour);
            source_diffuse = Some(name);
        }
        let opacity = selected_opacity.map(|(_, texture)| texture.to_owned());
        let family_override =
            reviewed_family_override(material_path, &manifest.render_shader_families);
        if let Some(family) = family_override {
            report
                .material_family_overrides_used
                .insert(material_path.clone(), family);
        }
        let classified_family = family_override.unwrap_or_else(|| {
            classify_family(
                material_path,
                resolved.as_ref().map_or(0, |material| material.blend_mode),
                source_diffuse.as_deref(),
                opacity.as_deref(),
            )
        });
        let family = if resolved.is_none()
            || source_diffuse.is_none() && classified_family != RenderFamily::Water
        {
            RenderFamily::Unknown
        } else {
            classified_family
        };
        report
            .material_families
            .insert(material_path.clone(), family);
        let layered_diffuse = resolved
            .as_ref()
            .filter(|_| family == RenderFamily::Opaque)
            .and_then(select_layered_diffuse);
        let layered_composite = layered_diffuse.as_ref().map(|layered| {
            for texture in &layered.source_diffuses {
                wanted_textures.insert(texture.clone());
            }
            wanted_textures.insert(layered.source_mask.clone());
            let composite = layered_composite_name(material_path);
            layered_composites.insert(composite.clone(), layered.clone());
            report
                .layered_diffuse_materials
                .insert(material_path.clone(), layered.clone());
            composite
        });
        let opacity_matches_diffuse_source = diffuse_override.is_some()
            || opacity_override.is_some()
            || resolved.as_ref().is_some_and(|material| {
                let diffuse_source = selected_diffuse
                    .and_then(|(parameter, _)| material.texture_sources.get(parameter));
                let opacity_source = selected_opacity
                    .and_then(|(parameter, _)| material.texture_sources.get(parameter));
                diffuse_source.is_some()
                    && (diffuse_source == opacity_source
                        || opacity_source.is_some_and(|source| source == material_path))
            });
        let diffuse = if let Some(composite) = layered_composite {
            Some(composite)
        } else {
            match (&source_diffuse, &opacity) {
                (Some(diffuse), Some(opacity))
                    if matches!(
                        family,
                        RenderFamily::Foliage
                            | RenderFamily::AlphaTested
                            | RenderFamily::Translucent
                            | RenderFamily::Decal
                    ) && opacity_matches_diffuse_source =>
                {
                    let composite = opacity_composite_name(diffuse, opacity);
                    opacity_composites
                        .insert(composite.clone(), (diffuse.clone(), opacity.clone()));
                    wanted_textures.insert(diffuse.clone());
                    wanted_textures.insert(opacity.clone());
                    Some(composite)
                }
                (Some(diffuse), Some(opacity))
                    if matches!(
                        family,
                        RenderFamily::Foliage
                            | RenderFamily::AlphaTested
                            | RenderFamily::Translucent
                            | RenderFamily::Decal
                    ) =>
                {
                    report.skipped_inherited_opacity.insert(
                    material_path.clone(),
                    format!(
                        "selected diffuse {diffuse} and opacity {opacity} were authored by different material levels"
                    ),
                );
                    wanted_textures.insert(diffuse.clone());
                    Some(diffuse.clone())
                }
                (Some(diffuse), _) => {
                    wanted_textures.insert(diffuse.clone());
                    Some(diffuse.clone())
                }
                (None, _) => None,
            }
        };
        if let Some((parameter, texture)) = selected_diffuse {
            report
                .material_diffuse
                .insert(material_path.clone(), texture.to_owned());
            report
                .material_diffuse_parameters
                .insert(material_path.clone(), parameter.to_owned());
        }
        if let Some(texture) = &opacity {
            report
                .material_opacity
                .insert(material_path.clone(), texture.clone());
            report.material_opacity_parameters.insert(
                material_path.clone(),
                selected_opacity
                    .expect("a selected opacity has a parameter")
                    .0
                    .to_owned(),
            );
        }
        plans.push(MaterialPlan {
            shader_id: *shader_id,
            name: format!("shader_{shader_id:04}"),
            source_shader: material_path.clone(),
            family,
            source_diffuse: diffuse,
            source_normal: None,
            dropped_lightmaps: Vec::new(),
            surface_count: surface_counts.get(shader_id).copied().unwrap_or(0),
            terrain: None,
        });
    }
    plans.sort_by_key(|plan| plan.shader_id);

    let mut decoded = BTreeMap::new();
    for texture_path in wanted_textures {
        if constant_colours.contains_key(&texture_path) {
            continue;
        }
        let effective_path = manifest
            .texture_aliases
            .get(&texture_path)
            .unwrap_or(&texture_path);
        let Some(location) = graph.locate(effective_path) else {
            if let Some(rgba) =
                known_flat_colour(effective_path).or_else(|| known_flat_colour(&texture_path))
            {
                decoded.insert(
                    texture_path.clone(),
                    make_constant_texture_plan(&texture_path, rgba, manifest)?,
                );
                continue;
            }
            report.texture_failures.insert(
                texture_path.clone(),
                "texture is imported but not exported by this streamed map graph".to_owned(),
            );
            continue;
        };
        match decode_texture(&graph, source, location, effective_path, manifest) {
            Ok(mut texture) => {
                if effective_path != &texture_path {
                    retarget_texture(&mut texture, &texture_path, manifest)?;
                }
                decoded.insert(texture_path, texture);
            }
            Err(error) => {
                report
                    .texture_failures
                    .insert(texture_path.clone(), error.to_string());
            }
        }
    }
    for (composite, (diffuse, opacity)) in &opacity_composites {
        let Some(diffuse_texture) = decoded.get(diffuse) else {
            continue;
        };
        let Some(opacity_texture) = decoded.get(opacity) else {
            continue;
        };
        match composite_opacity_texture(diffuse_texture, opacity_texture, composite, manifest) {
            Ok(texture) => {
                decoded.insert(composite.clone(), texture);
            }
            Err(error) => {
                report
                    .texture_failures
                    .insert(composite.clone(), error.to_string());
            }
        }
    }
    for (composite, layered) in &layered_composites {
        match composite_layered_diffuse(&decoded, layered, composite, manifest) {
            Ok(texture) => {
                decoded.insert(composite.clone(), texture);
            }
            Err(error) => {
                report
                    .texture_failures
                    .insert(composite.clone(), error.to_string());
            }
        }
    }
    for (name, rgba) in &constant_colours {
        decoded.insert(
            name.clone(),
            make_constant_texture_plan(name, *rgba, manifest)?,
        );
    }
    for plan in &mut plans {
        if plan
            .source_diffuse
            .as_ref()
            .is_some_and(|texture| !decoded.contains_key(texture))
        {
            let unavailable = plan.source_diffuse.take().expect("checked as present");
            plan.family = RenderFamily::Unknown;
            report
                .material_families
                .insert(plan.source_shader.clone(), RenderFamily::Unknown);
            report.material_failures.entry(plan.source_shader.clone()).or_insert_with(|| {
                format!(
                    "selected diffuse {unavailable} could not be emitted; rendered as UNKNOWN instead of choosing another texture"
                )
            });
        }
    }
    let mut referenced = BTreeSet::new();
    for plan in &plans {
        if let Some(diffuse) = &plan.source_diffuse {
            referenced.insert(diffuse.as_str());
        }
    }
    let textures: Vec<TexturePlan> = decoded
        .into_iter()
        .filter_map(|(name, texture)| referenced.contains(name.as_str()).then_some(texture))
        .collect();
    report.emitted_textures = textures.len();
    report.diffuse_bound_materials = plans
        .iter()
        .filter(|plan| plan.source_diffuse.is_some())
        .count();
    report.composited_opacity_materials = plans
        .iter()
        .filter(|plan| {
            plan.source_diffuse
                .as_ref()
                .is_some_and(|texture| opacity_composites.contains_key(texture))
        })
        .count();
    report.material_bound_diffuse = plans
        .iter()
        .filter_map(|plan| {
            plan.source_diffuse
                .as_ref()
                .map(|texture| (plan.source_shader.clone(), texture.clone()))
        })
        .collect();
    Ok(PlannedMaterialAssets {
        materials: plans,
        textures,
        report,
    })
}

fn material_alias<'a>(material_path: &'a str, aliases: &'a BTreeMap<String, String>) -> &'a str {
    aliases
        .iter()
        .find_map(|(source, replacement)| {
            source
                .eq_ignore_ascii_case(material_path)
                .then_some(replacement.as_str())
        })
        .unwrap_or(material_path)
}

fn reviewed_override<'a>(
    material_path: &str,
    overrides: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    overrides.iter().find_map(|(source, replacement)| {
        source
            .eq_ignore_ascii_case(material_path)
            .then_some(replacement.as_str())
    })
}

fn reviewed_family_override(
    material_path: &str,
    overrides: &BTreeMap<String, RenderFamily>,
) -> Option<RenderFamily> {
    overrides.iter().find_map(|(source, family)| {
        source
            .eq_ignore_ascii_case(material_path)
            .then_some(*family)
    })
}

fn resolve_material(
    graph: &PackageGraph,
    location: ObjectLocation,
    cache: &mut HashMap<String, ResolvedMaterial>,
    stack: &mut BTreeSet<String>,
    report: &mut MaterialAssetReport,
) -> Result<ResolvedMaterial> {
    let package = &graph.packages[location.package];
    let export = graph.export(location);
    let path = package.object_path(location.index)?;
    if let Some(cached) = cache.get(&path) {
        return Ok(cached.clone());
    }
    if !stack.insert(path.clone()) {
        bail!("material parent graph contains a cycle at {path}");
    }
    let properties = package.properties(export)?;
    let mut resolved = match export.class_name.as_str() {
        class if class.starts_with("MaterialInstance") => {
            let parent_index = object_property(package, &properties, "Parent")?
                .with_context(|| format!("{path} has no Parent"))?;
            match graph.resolve_reference(location, parent_index)? {
                Some(parent) => resolve_material(graph, parent, cache, stack, report)?,
                None => ResolvedMaterial::default(),
            }
        }
        class if class.ends_with("Material") => ResolvedMaterial {
            blend_mode: byte_property(package, &properties, "BlendMode")?.unwrap_or(0),
            ..ResolvedMaterial::default()
        },
        class => bail!("{path} is {class}, not a supported material"),
    };

    if export.class_name.ends_with("Material") {
        if let Some(expressions) = object_array_property(package, &properties, "Expressions")? {
            for expression_index in expressions {
                let Some(expression_location) =
                    graph.resolve_reference(location, expression_index)?
                else {
                    continue;
                };
                let expression = graph.export(expression_location);
                let expression_package = &graph.packages[expression_location.package];
                let expression_properties = expression_package.properties(expression)?;
                if expression.class_name.contains("TextureSample") {
                    let Some(texture_index) =
                        object_property(expression_package, &expression_properties, "Texture")?
                    else {
                        continue;
                    };
                    let Some(texture_path) =
                        graph.reference_path(expression_location, texture_index)?
                    else {
                        continue;
                    };
                    let parameter =
                        name_property(expression_package, &expression_properties, "ParameterName")?
                            .filter(|name| name != "None")
                            .unwrap_or_else(|| expression.object_name.clone());
                    resolved
                        .textures
                        .insert(parameter.to_ascii_lowercase(), texture_path);
                    resolved
                        .texture_sources
                        .insert(parameter.to_ascii_lowercase(), path.clone());
                } else if expression.class_name.contains("VectorParameter") {
                    let Some(parameter) =
                        name_property(expression_package, &expression_properties, "ParameterName")?
                            .filter(|name| name != "None")
                    else {
                        continue;
                    };
                    let Some(value) = find_property(&expression_properties, "DefaultValue")
                        .and_then(|property| expression_package.property_linear_color(property))
                    else {
                        continue;
                    };
                    resolved
                        .vectors
                        .insert(parameter.to_ascii_lowercase(), value);
                    resolved
                        .vector_sources
                        .insert(parameter.to_ascii_lowercase(), path.clone());
                }
            }
        }
    } else if export.class_name == "MaterialInstanceConstant" {
        if let Some(property) = find_property(&properties, "TextureParameterValues") {
            for (parameter, texture_index) in package
                .property_texture_parameters(property)?
                .unwrap_or_default()
            {
                *report
                    .texture_parameter_names
                    .entry(parameter.clone())
                    .or_default() += 1;
                if let Some(texture_path) = graph.reference_path(location, texture_index)? {
                    resolved
                        .textures
                        .insert(parameter.to_ascii_lowercase(), texture_path);
                    resolved
                        .texture_sources
                        .insert(parameter.to_ascii_lowercase(), path.clone());
                }
            }
        }
        if let Some(property) = find_property(&properties, "ScalarParameterValues") {
            for (parameter, value) in package
                .property_scalar_parameters(property)?
                .unwrap_or_default()
            {
                resolved
                    .scalars
                    .insert(parameter.to_ascii_lowercase(), value);
                resolved
                    .scalar_sources
                    .insert(parameter.to_ascii_lowercase(), path.clone());
            }
        }
        if let Some(property) = find_property(&properties, "VectorParameterValues") {
            for (parameter, value) in package
                .property_vector_parameters(property)?
                .unwrap_or_default()
            {
                resolved
                    .vectors
                    .insert(parameter.to_ascii_lowercase(), value);
                resolved
                    .vector_sources
                    .insert(parameter.to_ascii_lowercase(), path.clone());
            }
        }
    }
    stack.remove(&path);
    cache.insert(path, resolved.clone());
    Ok(resolved)
}

#[cfg(test)]
fn select_diffuse(textures: &BTreeMap<String, String>) -> Option<(&str, &str)> {
    ranked_diffuses(textures).first().copied()
}

fn ranked_diffuses(textures: &BTreeMap<String, String>) -> Vec<(&str, &str)> {
    let mut candidates = textures
        .iter()
        .filter_map(|(parameter, texture)| {
            let score = diffuse_score(parameter, texture);
            (score >= 0).then_some((score, parameter.as_str(), texture.as_str()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.2.cmp(right.2))
            .then_with(|| left.1.cmp(right.1))
    });
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter_map(|(_, parameter, texture)| seen.insert(texture).then_some((parameter, texture)))
        .collect()
}

fn select_opacity(textures: &BTreeMap<String, String>) -> Option<(&str, &str)> {
    textures
        .iter()
        .find(|(parameter, _)| parameter.contains("opacity"))
        .map(|(parameter, texture)| (parameter.as_str(), texture.as_str()))
}

fn select_layered_diffuse(material: &ResolvedMaterial) -> Option<LayeredDiffusePlan> {
    let first = material.textures.get("tex1_diffuse")?.clone();
    let second = material.textures.get("tex2_diffuse")?.clone();
    let third = material.textures.get("tex3_diffuse")?.clone();
    let mask = material.textures.get("tex_materialmask")?.clone();
    let mut distinct = BTreeSet::new();
    distinct.extend([first.as_str(), second.as_str(), third.as_str()]);
    if distinct.len() < 2 {
        return None;
    }
    let tile = |layer: usize| {
        material
            .scalars
            .get(&format!("scalar{layer}_uv_tiling"))
            .copied()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value.round() as i32)
            .unwrap_or(1)
            .clamp(1, 2_000)
    };
    Some(LayeredDiffusePlan {
        source_diffuses: [first, second, third],
        source_mask: mask,
        tiles: [tile(1), tile(2), tile(3)],
    })
}

fn layered_composite_name(material: &str) -> String {
    format!("{material}__baked_layered_diffuse")
}

fn known_flat_colour(path: &str) -> Option<[u8; 4]> {
    let leaf = path.rsplit('.').next()?.to_ascii_lowercase();
    let value = match leaf.as_str() {
        "t_flat_white_d" => 255,
        "t_flat_lightgray_d" | "t_flat_lightgrey_d" => 192,
        "t_flat_gray_d" | "t_flat_grey_d" => 128,
        "t_flat_darkgray_d" | "t_flat_darkgrey_d" => 64,
        "t_flat_vdarkgray_d" | "t_flat_vdarkgrey_d" => 32,
        "t_flat_black_d" => 0,
        _ => return None,
    };
    Some([value, value, value, 255])
}

fn select_constant_colour(
    material_path: &str,
    vectors: &BTreeMap<String, [f32; 4]>,
) -> Option<[u8; 4]> {
    let emissive_material = ["emiss", "glow", "light"]
        .iter()
        .any(|token| material_path.to_ascii_lowercase().contains(token));
    vectors
        .iter()
        .filter_map(|(parameter, value)| {
            if !value.iter().all(|channel| channel.is_finite()) {
                return None;
            }
            let parameter = parameter.to_ascii_lowercase();
            let mut score = 0;
            for token in ["basecolor", "base_color", "diffuse", "colour", "color"] {
                if parameter.contains(token) {
                    score += 100;
                }
            }
            for token in ["tint", "paint"] {
                if parameter.contains(token) {
                    score += 50;
                }
            }
            if emissive_material && parameter.contains("emiss") {
                score += 100;
            }
            if ["spec", "reflect", "fresnel", "rough", "gloss"]
                .iter()
                .any(|token| parameter.contains(token))
            {
                score -= 500;
            }
            (score > 0).then_some((score, parameter, linear_colour_to_rgba(*value)))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, _, colour)| colour)
}

fn named_material_colour(material_path: &str) -> Option<[u8; 4]> {
    let lower = material_path.to_ascii_lowercase();
    let gray = |value| Some([value, value, value, 255]);
    if lower.contains("blackunlit") {
        gray(0)
    } else if lower.contains("v_dark_gray")
        || lower.contains("v_dark_grey")
        || lower.contains("dgrey")
    {
        gray(32)
    } else if lower.contains("darker_gray")
        || lower.contains("darker_grey")
        || lower.contains("dark_gray")
        || lower.contains("dark_grey")
    {
        gray(64)
    } else if lower.contains("light_gray") || lower.contains("light_grey") {
        gray(192)
    } else if lower.contains("bgmat-grey")
        || lower.contains("mi_gray")
        || lower.contains("mi_grey")
        || lower.ends_with("defaultmaterial")
    {
        gray(128)
    } else if lower == "dirtybomb.none" {
        gray(128)
    } else if lower.contains("redlight") {
        Some([255, 24, 16, 255])
    } else if lower.contains("glow_yellow") {
        Some([255, 224, 32, 255])
    } else {
        None
    }
}

fn linear_colour_to_rgba(value: [f32; 4]) -> [u8; 4] {
    let encode = |linear: f32| {
        let linear = linear.clamp(0.0, 1.0);
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    };
    [
        encode(value[0]),
        encode(value[1]),
        encode(value[2]),
        (value[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn make_constant_texture_plan(
    name: &str,
    rgba: [u8; 4],
    manifest: &SocMapManifest,
) -> Result<TexturePlan> {
    const SIDE: u32 = 4;
    let pixels = vec![rgba; (SIDE * SIDE) as usize];
    let mips = vec![TextureMip {
        width: SIDE,
        height: SIDE,
        bytes: encode_bc1(&pixels, SIDE, SIDE)?,
    }];
    let info = DdsInfo {
        width: SIDE,
        height: SIDE,
        mip_levels: 1,
        four_cc: *b"DXT1",
    };
    let dds = make_dds(info, &mips)?;
    let bytes = if manifest.allow_untrimmed_mip_chains {
        dds
    } else {
        source::canonicalize_dds(&dds, info, true)?
    };
    let target_dds = source::target_texture_path(&manifest.namespace, name)?;
    let target_options = target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    Ok(TexturePlan {
        source_name: name.to_owned(),
        target_dds,
        target_options,
        info,
        bytes,
        is_normal: false,
        srgb: true,
    })
}

fn retarget_texture(
    texture: &mut TexturePlan,
    declared_name: &str,
    manifest: &SocMapManifest,
) -> Result<()> {
    texture.source_name = declared_name.to_owned();
    texture.target_dds = source::target_texture_path(&manifest.namespace, declared_name)?;
    texture.target_options = texture
        .target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    Ok(())
}

fn opacity_composite_name(diffuse: &str, opacity: &str) -> String {
    let opacity_leaf = opacity.rsplit('.').next().unwrap_or(opacity);
    format!("{diffuse}__opacity__{opacity_leaf}")
}

fn diffuse_score(parameter: &str, texture: &str) -> i32 {
    let parameter = parameter.to_ascii_lowercase();
    let texture_leaf = texture
        .rsplit('.')
        .next()
        .unwrap_or(texture)
        .to_ascii_lowercase();
    let parameter_is_colour = [
        "diffuse",
        "albedo",
        "basecolor",
        "base_color",
        "colour",
        "color",
        "base texture",
        "basetexture",
        "gradientmap",
    ]
    .iter()
    .any(|token| parameter.contains(token));
    let combined_colour_opacity = parameter.contains("colouropacity")
        || parameter.contains("coloropacity")
        || parameter.contains("colour_opacity")
        || parameter.contains("color_opacity");
    if [
        "normal",
        "nrm",
        "norm",
        "spec",
        "rough",
        "gloss",
        "mask",
        "height",
        "flow",
        "cube",
        "reflection",
        "emissive",
    ]
    .iter()
    .any(|token| parameter.contains(token))
        || parameter.contains("opacity") && !combined_colour_opacity
    {
        return -1;
    }
    // Generic MaterialExpressionTextureSample names carry no role. In that
    // case the texture leaf is the only safe way to reject a normal/specular
    // input. Never inspect the full object path: package names such as
    // `CDA_StructureDetails_01` contain semantic words while holding valid
    // `*_D` colour maps.
    let non_colour_leaf = if parameter == "gradientmap" {
        0
    } else {
        texture_role_penalty(&texture_leaf)
    };
    if !parameter_is_colour
        && parameter.starts_with("materialexpressiontexturesample")
        && non_colour_leaf < 0
    {
        return -1;
    }
    let mut score = 1;

    // Prefer the authored base role over optional dirt, moss, paint and fade
    // layers. The previous lexical tie-break selected `diffuse_03` (moss) for
    // a Camden concrete floor and `moss_diffuse` over `bricks_layer`, painting
    // an entire ground or brick section green.
    score += match parameter.as_str() {
        "diffusemap" | "diffuse" | "dif" | "base dif" | "base_diffuse" | "base texture"
        | "basetexture" | "basecolor" | "base_color" | "colour" | "color" => 500,
        "bricks_layer" | "diffuse_01" | "tex1_diffuse" | "layer0 dif" | "gradientmap" => 400,
        value if value.starts_with("diffuse_02") || value.starts_with("layer1 dif") => 200,
        value if value.starts_with("diffuse_03") || value.starts_with("layer2 dif") => 150,
        value if value.starts_with("diffuse_04") => 100,
        value if value.contains("moss") || value.contains("paint") || value.contains("dirt") => {
            -100
        }
        _ => 0,
    };
    for token in [
        "diffuse",
        "albedo",
        "basecolor",
        "base_color",
        "colour",
        "color",
        "base texture",
        "basetexture",
    ] {
        if parameter.contains(token) {
            score += 100;
        }
    }
    if texture_leaf.ends_with("_d") {
        score += 50;
    }
    score += non_colour_leaf;
    score
}

/// Penalize conventional non-colour suffixes without turning them into an
/// unconditional rejection. Some authored instances deliberately bind a
/// monochrome `_O`/`_M` atlas through `DiffuseMap` and tint it in the material
/// graph; the explicit role must still beat an unrelated inherited fallback.
fn texture_role_penalty(texture_leaf: &str) -> i32 {
    let suffix = texture_leaf.rsplit('_').next().unwrap_or(texture_leaf);
    match suffix {
        "n" | "nrm" | "normal" | "s" | "spec" | "specular" | "r" | "rough" | "roughness" | "g"
        | "gloss" | "h" | "height" | "reg" => -600,
        "ao" => -500,
        "m" | "mask" | "o" | "opacity" => -250,
        _ => 0,
    }
}

fn classify_family(
    path: &str,
    blend_mode: u8,
    diffuse: Option<&str>,
    opacity: Option<&str>,
) -> RenderFamily {
    let path_lower = path.to_ascii_lowercase();
    let lower = format!("{} {}", path, diffuse.unwrap_or_default()).to_ascii_lowercase();
    let material_name = path_lower.rsplit('.').next().unwrap_or(&path_lower);
    if ["vertexcolour", "vertexcolor", "vertexpaint"]
        .iter()
        .any(|token| path_lower.contains(token))
    {
        return RenderFamily::VertexLit;
    }
    if path_lower.starts_with("envwater_fx.")
        || ["m_water", "mi_water", "m_river", "mi_river"]
            .iter()
            .any(|prefix| {
                material_name == *prefix || material_name.starts_with(&format!("{prefix}_"))
            })
    {
        return RenderFamily::Water;
    }
    if matches!(blend_mode, 3 | 5) || lower.contains("emiss") || lower.contains("glow") {
        return RenderFamily::Emissive;
    }
    let glass = ["glass", "window"]
        .iter()
        .any(|token| lower.contains(token));
    if glass && (matches!(blend_mode, 2 | 4 | 7 | 8) || opacity.is_some()) {
        return RenderFamily::Translucent;
    }
    // UE3 renders these thin meshes as surface overlays. In the target they
    // need the decal-family geometric lift; leaving them in the ordinary
    // opaque/alpha-tested family makes their millimetre separation collapse
    // in the depth buffer at distance.
    let surface_overlay = ["decal", "graffit", "stain", "wallmark"]
        .iter()
        .any(|token| path_lower.contains(token))
        || path_lower.contains("dirt_mound") && opacity.is_some();
    if surface_overlay {
        return RenderFamily::Decal;
    }
    if matches!(blend_mode, 2 | 4 | 7 | 8) {
        return RenderFamily::Translucent;
    }
    // The dead-tree trunk and branch instances share a colour atlas, but only
    // the branch instance carries an Opacitymap. Do not infer foliage from the
    // diffuse name for an explicitly named trunk.
    if (path_lower.contains("trunk")
        || path_lower.contains("tree_01_frame")
        || path_lower.contains("tree01_frame"))
        && opacity.is_none()
    {
        return RenderFamily::Opaque;
    }
    // A few Camden terrain materials name their ground colour atlas `Grass`.
    // That describes the painted ground, not card foliage, and must not select
    // the two-sided alpha-tested vegetation template.
    if path_lower.contains("terrain") && opacity.is_none() {
        return RenderFamily::Opaque;
    }
    // Dirty Bomb's distant tree atlases are not consistently marked masked.
    // Classify from both the material and selected diffuse path so the target
    // keeps alpha testing and the two-sided foliage contract instead of
    // drawing the whole quad as a bulky opaque card.
    // `Canopy` is also Dirty Bomb's name for Camden's market roofs and their
    // columns (`LDN_Canopy_01`). Treating that architectural package as tree
    // canopy put whole market sections through the animated leaf shader. Real
    // tree canopies still carry `tree`, `birch`, `leaf`, or `foliage` in the
    // material/diffuse path, so `canopy` is deliberately not evidence by
    // itself.
    let vegetation = [
        "grass", "leaf", "leaves", "ivy", "foliage", "bramble", "branch", "birch",
    ]
    .iter()
    .any(|token| lower.contains(token))
        || contains_tree_asset_token(&lower) && lower.contains("canopy");
    if vegetation {
        return RenderFamily::Foliage;
    }
    // A substantial set of Dirty Bomb instances bind purpose-built `_O`,
    // `_M`, or `_T` maps while their cooked BlendMode remains the opaque
    // default. The bindings occur on the leaf instance itself and identify
    // real grids, cables, fan covers, signage and soft mound edges. Treat the
    // explicit opacity contract as stronger evidence than the missing enum.
    if opacity.is_some() || matches!(blend_mode, 1 | 6) {
        return RenderFamily::AlphaTested;
    }
    RenderFamily::Opaque
}

/// Match `tree`, `tree03`, and similar asset-name components without also
/// matching the middle of `street`. The caller additionally requires canopy
/// evidence: package names such as `GEN_CDA_Wrapped_Trees` contain rigid tape
/// and cages that must not inherit a foliage material merely from `tree`.
fn contains_tree_asset_token(value: &str) -> bool {
    value
        .match_indices("tree")
        .any(|(offset, _)| offset == 0 || !value.as_bytes()[offset - 1].is_ascii_alphanumeric())
}

fn find_property<'a>(properties: &'a [Property<'a>], name: &str) -> Option<&'a Property<'a>> {
    properties
        .iter()
        .rev()
        .find(|property| property.name == name)
}

fn object_property(
    package: &Package,
    properties: &[Property<'_>],
    name: &str,
) -> Result<Option<i32>> {
    find_property(properties, name)
        .map(|property| package.property_object(property))
        .transpose()
        .map(Option::flatten)
}

fn object_array_property(
    package: &Package,
    properties: &[Property<'_>],
    name: &str,
) -> Result<Option<Vec<i32>>> {
    find_property(properties, name)
        .map(|property| package.property_object_array(property))
        .transpose()
        .map(Option::flatten)
}

fn name_property(
    package: &Package,
    properties: &[Property<'_>],
    name: &str,
) -> Result<Option<String>> {
    find_property(properties, name)
        .map(|property| package.property_name(property))
        .transpose()
        .map(Option::flatten)
}

fn byte_property(package: &Package, properties: &[Property<'_>], name: &str) -> Result<Option<u8>> {
    let Some(property) = find_property(properties, name) else {
        return Ok(None);
    };
    if let Some(value) = package.property_byte(property) {
        return Ok(Some(value));
    }
    let Some(value) = package.property_enum_value(property)? else {
        return Ok(None);
    };
    let discriminant = match value.as_str() {
        "BLEND_Opaque" => 0,
        "BLEND_Masked" => 1,
        "BLEND_Translucent" => 2,
        "BLEND_Additive" => 3,
        "BLEND_Modulate" => 4,
        "BLEND_ModulateAndAdd" => 5,
        "BLEND_SoftMasked" => 6,
        "BLEND_AlphaComposite" => 7,
        "BLEND_DitheredTranslucent" => 8,
        "PF_A8R8G8B8" => 2,
        "PF_DXT1" => 5,
        "PF_DXT3" => 6,
        "PF_DXT5" => 7,
        _ => bail!("unsupported {name} enum value {value}"),
    };
    Ok(Some(discriminant))
}

#[derive(Clone, Debug)]
struct TextureMip {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct BulkHeader {
    flags: u32,
    element_count: usize,
    disk_size: usize,
    file_offset: u64,
}

fn decode_texture(
    graph: &PackageGraph,
    source: &DirtyBombSource,
    location: ObjectLocation,
    path: &str,
    manifest: &SocMapManifest,
) -> Result<TexturePlan> {
    let package = &graph.packages[location.package];
    let export = graph.export(location);
    if !matches!(export.class_name.as_str(), "Texture2D" | "TextureFlipBook") {
        bail!(
            "{path} is {}, not Texture2D/TextureFlipBook",
            export.class_name
        );
    }
    let tagged = package.tagged_properties(export)?;
    let properties = &tagged.values;
    let width = int_property(package, properties, "SizeX")?;
    let height = int_property(package, properties, "SizeY")?;
    let format = byte_property(package, properties, "Format")?
        .with_context(|| format!("{path} has no byte Format"))?;
    let srgb = find_property(properties, "SRGB")
        .and_then(|property| property.bool_value)
        .unwrap_or(true);
    let cache_name =
        name_property(package, properties, "TextureFileCacheName")?.filter(|name| name != "None");
    let mut cursor = NativeCursor::new(tagged.native_data, tagged.native_offset);
    cursor.skip_bulk_payload(None, source, "SourceArt")?;
    let mip_count = cursor.count("texture mip count")?;
    if mip_count == 0 {
        bail!("{path} has no texture mips");
    }
    let mut mips: Vec<TextureMip> = Vec::with_capacity(mip_count);
    for mip_index in 0..mip_count {
        let bytes =
            cursor.bulk_payload(cache_name.as_deref(), source, &format!("mip {mip_index}"))?;
        let mip_width = cursor.positive_u32("mip width")?;
        let mip_height = cursor.positive_u32("mip height")?;
        if bytes.is_empty() {
            // Cooked textures can discard the largest mips and retain a
            // smaller resident chain in the TFC. Dirty Bomb marks each
            // discarded header UNUSED with count=0 and size/offset=-1.
            continue;
        }
        if mip_width > MAX_TEXTURE_DIMENSION || mip_height > MAX_TEXTURE_DIMENSION {
            continue;
        }
        if let Some(previous) = mips.last() {
            if mip_width != (previous.width / 2).max(1)
                || mip_height != (previous.height / 2).max(1)
            {
                // UE3 packs the final sub-block levels into a mip tail. The
                // v0.100b target drops everything below one complete 4x4
                // block, so the valid prefix is the exact data we need.
                break;
            }
        }
        mips.push(TextureMip {
            width: mip_width,
            height: mip_height,
            bytes,
        });
        if mip_width <= 4 && mip_height <= 4 {
            break;
        }
    }
    if mips.is_empty() {
        bail!("{path} has no resident texture mip payload");
    }
    if mips[0].width > width || mips[0].height > height {
        bail!(
            "{path} property size {width}x{height} is smaller than selected mip {}x{}",
            mips[0].width,
            mips[0].height
        );
    }
    let four_cc = match format {
        2 => [0; 4],
        5 => *b"DXT1",
        6 => *b"DXT3",
        7 => *b"DXT5",
        other => bail!("{path} uses unsupported UE3 pixel format {other}"),
    };
    validate_mips(path, &mips, four_cc)?;
    let info = DdsInfo {
        width: mips[0].width,
        height: mips[0].height,
        mip_levels: u32::try_from(mips.len())?,
        four_cc,
    };
    let dds = make_dds(info, &mips)?;
    let bytes = if manifest.allow_untrimmed_mip_chains {
        dds
    } else {
        source::canonicalize_dds(&dds, info, srgb)?
    };
    let target_dds = source::target_texture_path(&manifest.namespace, path)?;
    let target_options = target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    Ok(TexturePlan {
        source_name: path.to_owned(),
        target_dds,
        target_options,
        info,
        bytes,
        is_normal: false,
        srgb,
    })
}

fn int_property(package: &Package, properties: &[Property<'_>], name: &str) -> Result<u32> {
    let value = find_property(properties, name)
        .and_then(|property| package.property_i32(property))
        .with_context(|| format!("texture has no integer {name}"))?;
    u32::try_from(value).with_context(|| format!("texture {name} is negative ({value})"))
}

fn validate_mips(path: &str, mips: &[TextureMip], four_cc: [u8; 4]) -> Result<()> {
    let block_bytes = match &four_cc {
        b"DXT1" => 8,
        b"DXT3" | b"DXT5" => 16,
        [0, 0, 0, 0] => 4,
        _ => unreachable!(),
    };
    for (index, mip) in mips.iter().enumerate() {
        let units = if four_cc == [0; 4] {
            mip.width as usize * mip.height as usize
        } else {
            (mip.width as usize).div_ceil(4) * (mip.height as usize).div_ceil(4)
        };
        let expected = units
            .checked_mul(block_bytes)
            .context("texture mip size overflow")?;
        if mip.bytes.len() != expected {
            bail!(
                "{path} mip {index} is {} bytes for {}x{}, expected {expected}",
                mip.bytes.len(),
                mip.width,
                mip.height
            );
        }
    }
    Ok(())
}

fn make_dds(info: DdsInfo, mips: &[TextureMip]) -> Result<Vec<u8>> {
    const DDSD_CAPS: u32 = 0x1;
    const DDSD_HEIGHT: u32 = 0x2;
    const DDSD_WIDTH: u32 = 0x4;
    const DDSD_PITCH: u32 = 0x8;
    const DDSD_PIXELFORMAT: u32 = 0x1000;
    const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
    const DDSD_LINEARSIZE: u32 = 0x8_0000;
    let payload_size: usize = mips.iter().map(|mip| mip.bytes.len()).sum();
    let compressed = info.four_cc != [0; 4];
    let mut dds = Vec::with_capacity(128 + payload_size);
    dds.extend_from_slice(b"DDS ");
    dds.extend_from_slice(&124_u32.to_le_bytes());
    dds.extend_from_slice(
        &(DDSD_CAPS
            | DDSD_HEIGHT
            | DDSD_WIDTH
            | DDSD_PIXELFORMAT
            | DDSD_MIPMAPCOUNT
            | if compressed {
                DDSD_LINEARSIZE
            } else {
                DDSD_PITCH
            })
        .to_le_bytes(),
    );
    dds.extend_from_slice(&info.height.to_le_bytes());
    dds.extend_from_slice(&info.width.to_le_bytes());
    let pitch = if compressed {
        u32::try_from(mips[0].bytes.len())?
    } else {
        info.width * 4
    };
    dds.extend_from_slice(&pitch.to_le_bytes());
    dds.extend_from_slice(&0_u32.to_le_bytes());
    dds.extend_from_slice(&info.mip_levels.to_le_bytes());
    dds.extend_from_slice(&[0; 44]);
    dds.extend_from_slice(&32_u32.to_le_bytes());
    if compressed {
        dds.extend_from_slice(&0x4_u32.to_le_bytes());
        dds.extend_from_slice(&info.four_cc);
        dds.extend_from_slice(&[0; 20]);
    } else {
        dds.extend_from_slice(&0x41_u32.to_le_bytes());
        dds.extend_from_slice(&[0; 4]);
        dds.extend_from_slice(&32_u32.to_le_bytes());
        dds.extend_from_slice(&0x00ff_0000_u32.to_le_bytes());
        dds.extend_from_slice(&0x0000_ff00_u32.to_le_bytes());
        dds.extend_from_slice(&0x0000_00ff_u32.to_le_bytes());
        dds.extend_from_slice(&0xff00_0000_u32.to_le_bytes());
    }
    let caps: u32 = if info.mip_levels > 1 {
        0x401008
    } else {
        0x1000
    };
    dds.extend_from_slice(&caps.to_le_bytes());
    dds.extend_from_slice(&[0; 16]);
    for mip in mips {
        dds.extend_from_slice(&mip.bytes);
    }
    Ok(dds)
}

/// Bake Dirty Bomb's three tiled colour layers into the untiled material-mask
/// UV space. This deliberately produces an ordinary opaque diffuse texture:
/// Vostok's terrain material needs a terrain-specific lighting contract and
/// renders an otherwise valid static mesh black.
fn composite_layered_diffuse(
    decoded: &BTreeMap<String, TexturePlan>,
    layered: &LayeredDiffusePlan,
    name: &str,
    manifest: &SocMapManifest,
) -> Result<TexturePlan> {
    let mask = decoded
        .get(&layered.source_mask)
        .with_context(|| format!("{name} has no decoded mask {}", layered.source_mask))?;
    let mask = decode_dds_base(mask)?;
    let layers = layered.source_diffuses[..3]
        .iter()
        .map(|path| {
            decoded
                .get(path)
                .with_context(|| format!("{name} has no decoded layer {path}"))
                .and_then(decode_dds_base)
        })
        .collect::<Result<Vec<_>>>()?;

    let side = LAYERED_COMPOSITE_DIMENSION;
    let pixel_count = (side as usize)
        .checked_mul(side as usize)
        .context("layered composite pixel count overflow")?;
    let mut pixels = Vec::with_capacity(pixel_count);
    for y in 0..side {
        let v = (y as f32 + 0.5) / side as f32;
        for x in 0..side {
            let u = (x as f32 + 0.5) / side as f32;
            let weights = sample_wrapped(&mask, u, v);
            let mut sum = u32::from(weights[0]) + u32::from(weights[1]) + u32::from(weights[2]);
            let mut weights = [weights[0], weights[1], weights[2]];
            if sum == 0 {
                // The UE3 graph leaves black mask texels on its base layer.
                weights[0] = 255;
                sum = 255;
            }
            let colours = [
                sample_wrapped(
                    &layers[0],
                    u * layered.tiles[0] as f32,
                    v * layered.tiles[0] as f32,
                ),
                sample_wrapped(
                    &layers[1],
                    u * layered.tiles[1] as f32,
                    v * layered.tiles[1] as f32,
                ),
                sample_wrapped(
                    &layers[2],
                    u * layered.tiles[2] as f32,
                    v * layered.tiles[2] as f32,
                ),
            ];
            let blend = |channel: usize| {
                let weighted = (0..3)
                    .map(|layer| u32::from(colours[layer][channel]) * u32::from(weights[layer]))
                    .sum::<u32>();
                ((weighted + sum / 2) / sum) as u8
            };
            pixels.push([blend(0), blend(1), blend(2), 255]);
        }
    }

    let mut mips = Vec::new();
    let mut mip_pixels = pixels;
    let mut width = side;
    let mut height = side;
    loop {
        mips.push(TextureMip {
            width,
            height,
            bytes: encode_bc1(&mip_pixels, width, height)?,
        });
        if width <= 4 || height <= 4 {
            break;
        }
        let (next, next_width, next_height) = downsample_rgba(&mip_pixels, width, height)?;
        mip_pixels = next;
        width = next_width;
        height = next_height;
    }
    let info = DdsInfo {
        width: side,
        height: side,
        mip_levels: u32::try_from(mips.len())?,
        four_cc: *b"DXT1",
    };
    let dds = make_dds(info, &mips)?;
    let bytes = if manifest.allow_untrimmed_mip_chains {
        dds
    } else {
        source::canonicalize_dds(&dds, info, true)?
    };
    let target_dds = source::target_texture_path(&manifest.namespace, name)?;
    let target_options = target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    Ok(TexturePlan {
        source_name: name.to_owned(),
        target_dds,
        target_options,
        info,
        bytes,
        is_normal: false,
        srgb: true,
    })
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<[u8; 4]>,
}

fn decode_dds_base(texture: &TexturePlan) -> Result<RgbaImage> {
    let info = source::parse_dds(&texture.bytes, &texture.source_name)?;
    let (payload, width, height) = dds_mip_payload(texture, info, 0)?;
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .context("decoded DDS pixel count overflow")?;
    let mut pixels = vec![[0_u8; 4]; pixel_count];
    if info.four_cc == [0; 4] {
        for (pixel, bgra) in pixels.iter_mut().zip(payload.chunks_exact(4)) {
            *pixel = [bgra[2], bgra[1], bgra[0], bgra[3]];
        }
        return Ok(RgbaImage {
            width,
            height,
            pixels,
        });
    }

    let block_bytes = match &info.four_cc {
        b"DXT1" => 8,
        b"DXT3" | b"DXT5" => 16,
        other => bail!(
            "{} has unsupported layered DDS format {:?}",
            texture.source_name,
            String::from_utf8_lossy(other)
        ),
    };
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_index = (block_y as usize * blocks_x as usize + block_x as usize)
                .checked_mul(block_bytes)
                .context("DDS block offset overflow")?;
            let block = payload
                .get(block_index..block_index + block_bytes)
                .context("DDS colour block is truncated")?;
            let colour_offset = usize::from(info.four_cc != *b"DXT1") * 8;
            let colour: &[u8; 8] = block[colour_offset..colour_offset + 8]
                .try_into()
                .expect("an eight-byte colour block");
            let colour_0 = u16::from_le_bytes(colour[0..2].try_into().unwrap());
            let colour_1 = u16::from_le_bytes(colour[2..4].try_into().unwrap());
            let mut palette = bc1_palette(colour_0, colour_1);
            if info.four_cc != *b"DXT1" && colour_0 <= colour_1 {
                for channel in 0..3 {
                    palette[2][channel] = ((2 * u16::from(palette[0][channel])
                        + u16::from(palette[1][channel]))
                        / 3) as u8;
                    palette[3][channel] = ((u16::from(palette[0][channel])
                        + 2 * u16::from(palette[1][channel]))
                        / 3) as u8;
                }
            }
            let selectors = u32::from_le_bytes(colour[4..8].try_into().unwrap());
            let alpha_palette = (info.four_cc == *b"DXT5").then(|| bc3_alpha_palette(block));
            let alpha_selectors = (info.four_cc == *b"DXT5").then(|| {
                u64::from_le_bytes([
                    block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
                ])
            });
            let dxt3_alpha = (info.four_cc == *b"DXT3")
                .then(|| u64::from_le_bytes(block[0..8].try_into().unwrap()));
            for local_y in 0..4_u32 {
                for local_x in 0..4_u32 {
                    let x = block_x * 4 + local_x;
                    let y = block_y * 4 + local_y;
                    if x >= width || y >= height {
                        continue;
                    }
                    let local = (local_y * 4 + local_x) as usize;
                    let selector = ((selectors >> (local * 2)) & 3) as usize;
                    let transparent =
                        info.four_cc == *b"DXT1" && colour_0 <= colour_1 && selector == 3;
                    let alpha = if transparent {
                        0
                    } else if let (Some(palette), Some(selectors)) =
                        (alpha_palette.as_ref(), alpha_selectors)
                    {
                        palette[((selectors >> (local * 3)) & 7) as usize]
                    } else if let Some(alpha) = dxt3_alpha {
                        (((alpha >> (local * 4)) & 15) as u8) * 17
                    } else {
                        255
                    };
                    let rgb = palette[selector];
                    pixels[y as usize * width as usize + x as usize] =
                        [rgb[0], rgb[1], rgb[2], alpha];
                }
            }
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

fn bc3_alpha_palette(block: &[u8]) -> [u8; 8] {
    let a0 = block[0];
    let a1 = block[1];
    let mut palette = [0_u8; 8];
    palette[0] = a0;
    palette[1] = a1;
    if a0 > a1 {
        for index in 1..=6 {
            palette[index + 1] =
                (((7 - index) as u16 * u16::from(a0) + index as u16 * u16::from(a1)) / 7) as u8;
        }
    } else {
        for index in 1..=4 {
            palette[index + 1] =
                (((5 - index) as u16 * u16::from(a0) + index as u16 * u16::from(a1)) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    palette
}

fn sample_wrapped(image: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let wrap = |value: f32, extent: u32| {
        (value.rem_euclid(1.0) * extent as f32).floor() as usize % extent as usize
    };
    image.pixels[wrap(v, image.height) * image.width as usize + wrap(u, image.width)]
}

fn downsample_rgba(
    pixels: &[[u8; 4]],
    width: u32,
    height: u32,
) -> Result<(Vec<[u8; 4]>, u32, u32)> {
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut next = Vec::with_capacity(next_width as usize * next_height as usize);
    for y in 0..next_height {
        for x in 0..next_width {
            let mut sum = [0_u32; 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let source_x = (x * 2 + dx).min(width - 1);
                    let source_y = (y * 2 + dy).min(height - 1);
                    let pixel = pixels
                        .get(source_y as usize * width as usize + source_x as usize)
                        .context("downsample source pixel is outside the image")?;
                    for channel in 0..4 {
                        sum[channel] += u32::from(pixel[channel]);
                    }
                }
            }
            next.push([
                ((sum[0] + 2) / 4) as u8,
                ((sum[1] + 2) / 4) as u8,
                ((sum[2] + 2) / 4) as u8,
                ((sum[3] + 2) / 4) as u8,
            ]);
        }
    }
    Ok((next, next_width, next_height))
}

fn encode_bc1(pixels: &[[u8; 4]], width: u32, height: u32) -> Result<Vec<u8>> {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let mut output = Vec::with_capacity(blocks_x as usize * blocks_y as usize * 8);
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut block = [[0_u8; 3]; 16];
            let mut low = [255_u8; 3];
            let mut high = [0_u8; 3];
            for local_y in 0..4_u32 {
                for local_x in 0..4_u32 {
                    let x = (block_x * 4 + local_x).min(width - 1);
                    let y = (block_y * 4 + local_y).min(height - 1);
                    let source = pixels
                        .get(y as usize * width as usize + x as usize)
                        .context("BC1 source pixel is outside the image")?;
                    let index = (local_y * 4 + local_x) as usize;
                    block[index] = [source[0], source[1], source[2]];
                    for channel in 0..3 {
                        low[channel] = low[channel].min(source[channel]);
                        high[channel] = high[channel].max(source[channel]);
                    }
                }
            }
            let mut colour_0 = rgb_to_565(high);
            let mut colour_1 = rgb_to_565(low);
            if colour_0 < colour_1 {
                std::mem::swap(&mut colour_0, &mut colour_1);
            }
            output.extend_from_slice(&colour_0.to_le_bytes());
            output.extend_from_slice(&colour_1.to_le_bytes());
            if colour_0 == colour_1 {
                output.extend_from_slice(&0_u32.to_le_bytes());
                continue;
            }
            let palette = bc1_palette(colour_0, colour_1);
            let mut selectors = 0_u32;
            for (index, pixel) in block.iter().enumerate() {
                let selector = palette
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, candidate)| {
                        (0..3)
                            .map(|channel| {
                                let delta =
                                    i32::from(pixel[channel]) - i32::from(candidate[channel]);
                                delta * delta
                            })
                            .sum::<i32>()
                    })
                    .map(|(selector, _)| selector)
                    .unwrap_or(0);
                selectors |= (selector as u32) << (index * 2);
            }
            output.extend_from_slice(&selectors.to_le_bytes());
        }
    }
    Ok(output)
}

fn rgb_to_565([red, green, blue]: [u8; 3]) -> u16 {
    let quantise = |value: u8, bits: u32| -> u16 {
        let max = (1_u16 << bits) - 1;
        ((u32::from(value) * u32::from(max) + 127) / 255) as u16
    };
    (quantise(red, 5) << 11) | (quantise(green, 6) << 5) | quantise(blue, 5)
}

/// Merge an explicit UE3 opacity texture into a colour atlas's alpha channel.
///
/// Vostok's alpha-tested gbuffer path clips `texture_diffuse_color.a`; its
/// material schema has no independent opacity sampler. Dirty Bomb's foliage
/// instances often bind colour and opacity atlases separately. DXT1 masks use
/// colour luminance; DXT3/DXT5 masks carry the authored cutout in alpha.
fn composite_opacity_texture(
    diffuse: &TexturePlan,
    opacity: &TexturePlan,
    name: &str,
    manifest: &SocMapManifest,
) -> Result<TexturePlan> {
    let diffuse_info = source::parse_dds(&diffuse.bytes, &diffuse.source_name)?;
    let opacity_info = source::parse_dds(&opacity.bytes, &opacity.source_name)?;
    if !matches!(&diffuse_info.four_cc, b"DXT1" | b"DXT5")
        || !matches!(&opacity_info.four_cc, b"DXT1" | b"DXT3" | b"DXT5")
    {
        bail!(
            "{name} opacity composition requires a DXT1/DXT5 diffuse and DXT1/3/5 mask, got {:?} and {:?}",
            String::from_utf8_lossy(&diffuse_info.four_cc),
            String::from_utf8_lossy(&opacity_info.four_cc)
        );
    }

    let mut diffuse_offset = 128_usize;
    let mut width = diffuse_info.width;
    let mut height = diffuse_info.height;
    let mut mips = Vec::with_capacity(diffuse_info.mip_levels as usize);
    for mip_index in 0..diffuse_info.mip_levels {
        let blocks = (width as usize)
            .div_ceil(4)
            .checked_mul((height as usize).div_ceil(4))
            .context("opacity composite block count overflow")?;
        let diffuse_block_bytes = if diffuse_info.four_cc == *b"DXT1" {
            8
        } else {
            16
        };
        let mip_bytes = blocks
            .checked_mul(diffuse_block_bytes)
            .context("opacity composite mip size overflow")?;
        let diffuse_end = diffuse_offset
            .checked_add(mip_bytes)
            .context("diffuse mip offset overflow")?;
        let diffuse_payload = diffuse
            .bytes
            .get(diffuse_offset..diffuse_end)
            .with_context(|| format!("{} mip {mip_index} is truncated", diffuse.source_name))?;
        let opacity_mip = mip_index.min(opacity_info.mip_levels.saturating_sub(1));
        let (opacity_payload, opacity_width, opacity_height) =
            dds_mip_payload(opacity, opacity_info, opacity_mip)?;
        let mut bytes = Vec::with_capacity(blocks * 16);
        let block_width = (width as usize).div_ceil(4);
        for block_index in 0..blocks {
            let block_x = block_index % block_width;
            let block_y = block_index / block_width;
            let mut alpha = [0_u8; 16];
            for pixel_y in 0..4 {
                for pixel_x in 0..4 {
                    let x = (block_x * 4 + pixel_x).min(width.saturating_sub(1) as usize);
                    let y = (block_y * 4 + pixel_y).min(height.saturating_sub(1) as usize);
                    let mask_x = (x * opacity_width as usize / width as usize)
                        .min(opacity_width.saturating_sub(1) as usize);
                    let mask_y = (y * opacity_height as usize / height as usize)
                        .min(opacity_height.saturating_sub(1) as usize);
                    alpha[pixel_y * 4 + pixel_x] = opacity_at(
                        opacity_payload,
                        opacity_info.four_cc,
                        opacity_width,
                        mask_x,
                        mask_y,
                    )?;
                }
            }
            bytes.extend_from_slice(&dxt5_alpha_values(&alpha));
            let colour_start = block_index * diffuse_block_bytes
                + usize::from(diffuse_info.four_cc == *b"DXT5") * 8;
            bytes.extend_from_slice(
                diffuse_payload
                    .get(colour_start..colour_start + 8)
                    .with_context(|| {
                        format!(
                            "{} mip {mip_index} colour block is truncated",
                            diffuse.source_name
                        )
                    })?,
            );
        }
        mips.push(TextureMip {
            width,
            height,
            bytes,
        });
        diffuse_offset = diffuse_end;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }

    let info = DdsInfo {
        four_cc: *b"DXT5",
        ..diffuse_info
    };
    let dds = make_dds(info, &mips)?;
    let bytes = if manifest.allow_untrimmed_mip_chains {
        dds
    } else {
        source::canonicalize_dds(&dds, info, diffuse.srgb)?
    };
    let target_dds = source::target_texture_path(&manifest.namespace, name)?;
    let target_options = target_dds
        .strip_suffix(".dds")
        .expect("target texture paths end in .dds")
        .to_owned()
        + ".options";
    Ok(TexturePlan {
        source_name: name.to_owned(),
        target_dds,
        target_options,
        info,
        bytes,
        is_normal: false,
        srgb: diffuse.srgb,
    })
}

fn dds_mip_payload(
    texture: &TexturePlan,
    info: DdsInfo,
    wanted_mip: u32,
) -> Result<(&[u8], u32, u32)> {
    let (block_bytes, compressed) = match &info.four_cc {
        b"DXT1" => (8, true),
        b"DXT3" | b"DXT5" => (16, true),
        [0, 0, 0, 0] => (4, false),
        other => bail!(
            "{} has unsupported block format {:?}",
            texture.source_name,
            String::from_utf8_lossy(other)
        ),
    };
    let mut offset = 128_usize;
    let mut width = info.width;
    let mut height = info.height;
    for mip in 0..info.mip_levels {
        let units = if compressed {
            (width as usize).div_ceil(4) * (height as usize).div_ceil(4)
        } else {
            width as usize * height as usize
        };
        let bytes = units
            .checked_mul(block_bytes)
            .context("DDS mip size overflow")?;
        let end = offset
            .checked_add(bytes)
            .context("DDS mip offset overflow")?;
        if mip == wanted_mip {
            return Ok((
                texture
                    .bytes
                    .get(offset..end)
                    .with_context(|| format!("{} mip {mip} is truncated", texture.source_name))?,
                width,
                height,
            ));
        }
        offset = end;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    bail!("{} has no declared mip {wanted_mip}", texture.source_name)
}

fn bc1_luminance_at(payload: &[u8], width: u32, x: usize, y: usize) -> Result<u8> {
    let block_width = (width as usize).div_ceil(4);
    let block_index = (y / 4)
        .checked_mul(block_width)
        .and_then(|row| row.checked_add(x / 4))
        .context("BC1 block index overflow")?;
    let start = block_index.checked_mul(8).context("BC1 offset overflow")?;
    let block: &[u8; 8] = payload
        .get(start..start + 8)
        .context("BC1 mask block is truncated")?
        .try_into()
        .expect("an eight-byte slice");
    let colour_0 = u16::from_le_bytes(block[0..2].try_into().unwrap());
    let colour_1 = u16::from_le_bytes(block[2..4].try_into().unwrap());
    let palette = bc1_palette(colour_0, colour_1);
    let selectors = u32::from_le_bytes(block[4..8].try_into().unwrap());
    let pixel = (y % 4) * 4 + x % 4;
    let selector = ((selectors >> (pixel * 2)) & 3) as usize;
    if colour_0 <= colour_1 && selector == 3 {
        return Ok(0);
    }
    let [red, green, blue] = palette[selector];
    Ok(((u16::from(red) + 2 * u16::from(green) + u16::from(blue)) / 4) as u8)
}

fn opacity_at(payload: &[u8], four_cc: [u8; 4], width: u32, x: usize, y: usize) -> Result<u8> {
    if four_cc == *b"DXT1" {
        return bc1_luminance_at(payload, width, x, y);
    }
    let block_width = (width as usize).div_ceil(4);
    let block_index = (y / 4)
        .checked_mul(block_width)
        .and_then(|row| row.checked_add(x / 4))
        .context("opacity block index overflow")?;
    let start = block_index
        .checked_mul(16)
        .context("opacity block offset overflow")?;
    let block = payload
        .get(start..start + 16)
        .context("opacity block is truncated")?;
    let pixel = (y % 4) * 4 + x % 4;
    match &four_cc {
        b"DXT3" => {
            let values = u64::from_le_bytes(block[..8].try_into().unwrap());
            Ok((((values >> (pixel * 4)) & 15) as u8) * 17)
        }
        b"DXT5" => {
            let palette = bc3_alpha_palette(block);
            let selectors = u64::from_le_bytes([
                block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
            ]);
            Ok(palette[((selectors >> (pixel * 3)) & 7) as usize])
        }
        other => bail!(
            "unsupported opacity format {:?}",
            String::from_utf8_lossy(other)
        ),
    }
}

#[cfg(test)]
fn dxt5_alpha_block(mask: &[u8; 8]) -> [u8; 8] {
    let colour_0 = u16::from_le_bytes(mask[0..2].try_into().unwrap());
    let colour_1 = u16::from_le_bytes(mask[2..4].try_into().unwrap());
    let palette = bc1_palette(colour_0, colour_1);
    let selectors = u32::from_le_bytes(mask[4..8].try_into().unwrap());
    let mut alpha = [0_u8; 16];
    for (pixel, value) in alpha.iter_mut().enumerate() {
        let selector = ((selectors >> (pixel * 2)) & 3) as usize;
        *value = if colour_0 <= colour_1 && selector == 3 {
            0
        } else {
            let [red, green, blue] = palette[selector];
            ((u16::from(red) + 2 * u16::from(green) + u16::from(blue)) / 4) as u8
        };
    }
    dxt5_alpha_values(&alpha)
}

fn bc1_palette(colour_0: u16, colour_1: u16) -> [[u8; 3]; 4] {
    let endpoints = [rgb565(colour_0), rgb565(colour_1)];
    let mut palette = [[0_u8; 3]; 4];
    palette[0] = endpoints[0];
    palette[1] = endpoints[1];
    if colour_0 > colour_1 {
        for channel in 0..3 {
            palette[2][channel] =
                ((2 * u16::from(palette[0][channel]) + u16::from(palette[1][channel])) / 3) as u8;
            palette[3][channel] =
                ((u16::from(palette[0][channel]) + 2 * u16::from(palette[1][channel])) / 3) as u8;
        }
    } else {
        for channel in 0..3 {
            palette[2][channel] =
                ((u16::from(palette[0][channel]) + u16::from(palette[1][channel])) / 2) as u8;
        }
    }

    palette
}

fn dxt5_alpha_values(alpha: &[u8; 16]) -> [u8; 8] {
    const ALPHAS: [u8; 8] = [255, 0, 219, 182, 146, 109, 73, 36];
    let mut packed = 0_u64;
    for (pixel, alpha) in alpha.iter().enumerate() {
        let encoded = ALPHAS
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| alpha.abs_diff(**candidate))
            .map(|(index, _)| index as u64)
            .unwrap();
        packed |= encoded << (pixel * 3);
    }
    let packed = packed.to_le_bytes();
    [
        255, 0, packed[0], packed[1], packed[2], packed[3], packed[4], packed[5],
    ]
}

fn rgb565(value: u16) -> [u8; 3] {
    let red = ((value >> 11) & 0x1f) as u8;
    let green = ((value >> 5) & 0x3f) as u8;
    let blue = (value & 0x1f) as u8;
    [
        (u16::from(red) * 255 / 31) as u8,
        (u16::from(green) * 255 / 63) as u8,
        (u16::from(blue) * 255 / 31) as u8,
    ]
}

struct NativeCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    absolute_start: usize,
}

impl<'a> NativeCursor<'a> {
    fn new(bytes: &'a [u8], absolute_start: usize) -> Self {
        Self {
            bytes,
            offset: 0,
            absolute_start,
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .context("texture cursor overflow")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .context("texture native payload is truncated")?;
        self.offset = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn count(&mut self, field: &str) -> Result<usize> {
        let value = self.i32()?;
        if !(0..=1_000_000).contains(&value) {
            bail!("invalid {field} {value}");
        }
        Ok(value as usize)
    }

    fn positive_u32(&mut self, field: &str) -> Result<u32> {
        let value = self.i32()?;
        if value <= 0 {
            bail!("invalid {field} {value}");
        }
        Ok(value as u32)
    }

    fn bulk_header(&mut self) -> Result<BulkHeader> {
        let flags = self.u32()?;
        let element_count = self.i32()?;
        let disk_size = self.i32()?;
        let file_offset = self.i32()?;
        if element_count < 0 {
            bail!("negative texture bulk-data header");
        }
        let empty = flags & BULKDATA_UNUSED != 0 || element_count == 0;
        if !empty && (disk_size < 0 || file_offset < 0) {
            bail!("negative texture bulk-data header");
        }
        Ok(BulkHeader {
            flags,
            element_count: element_count as usize,
            disk_size: if empty { 0 } else { disk_size as usize },
            file_offset: if empty { 0 } else { file_offset as u64 },
        })
    }

    fn skip_bulk_payload(
        &mut self,
        cache_name: Option<&str>,
        source: &DirtyBombSource,
        label: &str,
    ) -> Result<()> {
        let header = self.bulk_header()?;
        if header.flags & BULKDATA_UNUSED != 0 || header.element_count == 0 {
            return Ok(());
        }
        if header.flags & BULKDATA_STORE_IN_SEPARATE_FILE != 0 {
            return Ok(());
        }
        let expected_offset = self
            .absolute_start
            .checked_add(self.offset)
            .context("inline texture offset overflow")?;
        if header.file_offset != expected_offset as u64 {
            bail!(
                "{label} inline offset is {:#x}, expected {expected_offset:#x}",
                header.file_offset
            );
        }
        self.take(header.disk_size)?;
        let _ = (cache_name, source);
        Ok(())
    }

    fn bulk_payload(
        &mut self,
        cache_name: Option<&str>,
        source: &DirtyBombSource,
        label: &str,
    ) -> Result<Vec<u8>> {
        let header = self.bulk_header()?;
        if header.flags & BULKDATA_UNUSED != 0 || header.element_count == 0 {
            return Ok(Vec::new());
        }
        let disk = if header.flags & BULKDATA_STORE_IN_SEPARATE_FILE != 0 {
            let cache = cache_name
                .with_context(|| format!("{label} is external but TextureFileCacheName is None"))?;
            let path = source.cooked_pc().join(format!("{cache}.tfc"));
            let mut file = File::open(&path)
                .with_context(|| format!("opening texture cache {}", path.display()))?;
            file.seek(SeekFrom::Start(header.file_offset))?;
            let mut bytes = vec![0; header.disk_size];
            file.read_exact(&mut bytes)?;
            bytes
        } else {
            let expected_offset = self
                .absolute_start
                .checked_add(self.offset)
                .context("inline texture offset overflow")?;
            if header.file_offset != expected_offset as u64 {
                bail!(
                    "{label} inline offset is {:#x}, expected {expected_offset:#x}",
                    header.file_offset
                );
            }
            self.take(header.disk_size)?.to_vec()
        };
        decompress_bulk(header, disk, label)
    }
}

fn decompress_bulk(header: BulkHeader, disk: Vec<u8>, label: &str) -> Result<Vec<u8>> {
    let compression = header.flags
        & (BULKDATA_SERIALIZE_COMPRESSED_ZLIB
            | BULKDATA_SERIALIZE_COMPRESSED_LZO
            | BULKDATA_SERIALIZE_COMPRESSED_LZX);
    if compression == 0 {
        if disk.len() != header.element_count {
            bail!(
                "{label} has {} uncompressed bytes, expected {}",
                disk.len(),
                header.element_count
            );
        }
        return Ok(disk);
    }
    if compression == BULKDATA_SERIALIZE_COMPRESSED_LZO {
        return decompress_chunked_lzo(&disk, header.element_count, label);
    }
    bail!("{label} uses unsupported bulk compression flags {compression:#x}")
}

fn decompress_chunked_lzo(disk: &[u8], expected: usize, label: &str) -> Result<Vec<u8>> {
    const PACKAGE_TAG: u32 = 0x9e2a_83c1;
    if disk.len() < 16 {
        bail!("{label} compressed bulk payload has no chunk summary");
    }
    let read = |offset: usize| u32::from_le_bytes(disk[offset..offset + 4].try_into().unwrap());
    if read(0) != PACKAGE_TAG {
        bail!("{label} compressed bulk payload has invalid package tag");
    }
    let block_size = usize::try_from(read(4))?;
    let total_compressed = usize::try_from(read(8))?;
    let total_uncompressed = usize::try_from(read(12))?;
    if block_size == 0 || total_uncompressed != expected {
        bail!(
            "{label} compressed bulk summary is {total_compressed}/{total_uncompressed} with block {block_size}, expected {expected}"
        );
    }
    let block_count = total_uncompressed.div_ceil(block_size);
    let table_end = 16_usize
        .checked_add(block_count.checked_mul(8).context("bulk table overflow")?)
        .context("bulk table overflow")?;
    if table_end > disk.len() {
        bail!("{label} compressed bulk block table is truncated");
    }
    let mut descriptors = Vec::with_capacity(block_count);
    let mut table_offset = 16;
    let mut described_compressed = 0;
    let mut described_uncompressed = 0;
    for _ in 0..block_count {
        let compressed = usize::try_from(read(table_offset))?;
        let uncompressed = usize::try_from(read(table_offset + 4))?;
        table_offset += 8;
        described_compressed += compressed;
        described_uncompressed += uncompressed;
        descriptors.push((compressed, uncompressed));
    }
    if described_compressed != total_compressed
        || described_uncompressed != total_uncompressed
        || table_end + total_compressed != disk.len()
    {
        bail!("{label} compressed bulk chunk sizes do not match their summary");
    }
    let mut input_offset = table_end;
    let mut output = vec![0; total_uncompressed];
    let mut output_offset = 0;
    for (block_index, (compressed, uncompressed)) in descriptors.into_iter().enumerate() {
        let input = &disk[input_offset..input_offset + compressed];
        let target = &mut output[output_offset..output_offset + uncompressed];
        let written = lzo::decompress_into(input, target)
            .with_context(|| format!("{label} LZO block {block_index}"))?;
        if written != uncompressed {
            bail!("{label} LZO block {block_index} wrote {written}, expected {uncompressed}");
        }
        input_offset += compressed;
        output_offset += uncompressed;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_normal_maps_as_diffuse_candidates() {
        assert!(diffuse_score("normalmap", "package.texture_n") < 0);
        assert!(diffuse_score("diffuse", "package.texture_d") > 0);
        assert!(
            diffuse_score(
                "materialexpressiontexturesample_0",
                "CDA_StructureDetails_01.Textures.T_CDA_Lightpost_02_D",
            ) > 0
        );
        assert!(
            diffuse_score(
                "materialexpressiontexturesample_0",
                "CDA_StructureDetails_01.Textures.T_CDA_Lightpost_02_N",
            ) < 0
        );
    }

    #[test]
    fn prefers_authored_base_over_moss_and_dirt_layers() {
        let concrete = BTreeMap::from([
            ("diffuse_01".to_owned(), "wall_d".to_owned()),
            ("diffuse_02".to_owned(), "dirt_d".to_owned()),
            ("diffuse_03".to_owned(), "moss_d".to_owned()),
        ]);
        assert_eq!(
            select_diffuse(&concrete).map(|(_, texture)| texture),
            Some("wall_d")
        );

        let bricks = BTreeMap::from([
            ("bricks_layer".to_owned(), "brick_d".to_owned()),
            ("moss_diffuse".to_owned(), "moss_d".to_owned()),
        ]);
        assert_eq!(
            select_diffuse(&bricks).map(|(_, texture)| texture),
            Some("brick_d")
        );

        let decal = BTreeMap::from([
            ("customcolourmap".to_owned(), "generic_blast_d".to_owned()),
            ("diffusemap".to_owned(), "authored_stain_d".to_owned()),
        ]);
        assert_eq!(
            select_diffuse(&decal).map(|(_, texture)| texture),
            Some("authored_stain_d")
        );

        let authored_mask = BTreeMap::from([
            ("customcolourmap".to_owned(), "generic_blast_d".to_owned()),
            ("diffusemap".to_owned(), "authored_decal_o".to_owned()),
        ]);
        assert_eq!(
            select_diffuse(&authored_mask).map(|(_, texture)| texture),
            Some("authored_decal_o")
        );

        let ao_with_colour_detail = BTreeMap::from([
            ("diffuse_detail".to_owned(), "rust_d".to_owned()),
            ("diffusemap".to_owned(), "wrought_iron_ao".to_owned()),
        ]);
        assert_eq!(
            select_diffuse(&ao_with_colour_detail).map(|(_, texture)| texture),
            Some("rust_d")
        );
    }

    #[test]
    fn material_aliases_are_case_insensitive() {
        let aliases = BTreeMap::from([(
            "LDN_LedSign_01.Materials.MI_LDN_LedSign".to_owned(),
            "GEN_Tileable_Materials_03.Materials.MI_GEN_Brushed_Metal_01_INST".to_owned(),
        )]);
        assert_eq!(
            material_alias("ldn_ledsign_01.materials.mi_ldn_ledsign", &aliases),
            "GEN_Tileable_Materials_03.Materials.MI_GEN_Brushed_Metal_01_INST"
        );
        assert_eq!(material_alias("unaliased", &aliases), "unaliased");
    }

    #[test]
    fn reviewed_material_texture_overrides_are_case_insensitive() {
        let overrides = BTreeMap::from([(
            "GEN_Decals_01.Materials_Decals.M_Stain_02".to_owned(),
            "GEN_Decals_01.Textures.T_decal_stain_02_d".to_owned(),
        )]);
        assert_eq!(
            reviewed_override("gen_decals_01.materials_decals.m_stain_02", &overrides),
            Some("GEN_Decals_01.Textures.T_decal_stain_02_d")
        );
        assert_eq!(reviewed_override("unconfigured", &overrides), None);
    }

    #[test]
    fn reviewed_material_family_overrides_are_case_insensitive() {
        let overrides = BTreeMap::from([(
            "Vista_Castle_Matte_Painting.Materials.MI_River_Bed".to_owned(),
            RenderFamily::Opaque,
        )]);
        assert_eq!(
            reviewed_family_override(
                "vista_castle_matte_painting.materials.mi_river_bed",
                &overrides
            ),
            Some(RenderFamily::Opaque)
        );
        assert_eq!(reviewed_family_override("unconfigured", &overrides), None);
    }

    #[test]
    fn classifies_ue3_blend_modes() {
        assert_eq!(classify_family("wall", 0, None, None), RenderFamily::Opaque);
        assert_eq!(classify_family("ivy", 1, None, None), RenderFamily::Foliage);
        assert_eq!(
            classify_family(
                "LDN_Canopy_01.Materials.MI_Internal_Panels",
                0,
                Some("LDN_Canopy_01.Textures.T_LDN_BM_InternalPanels_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "GEN_Prop_Foliage_01.Materials.MI_Tree_Birch_Canopy_01_Unlit",
                0,
                Some("GEN_Prop_Foliage_01.Textures.T_tree_01_leaves_01_D"),
                None,
            ),
            RenderFamily::Foliage
        );
        assert_eq!(
            classify_family(
                "LDN_Prop_Street_01.Materials.MI_StreetLight_03",
                0,
                Some("LDN_Prop_Street_01.Textures.T_LDN_Prop_Street_01_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "GEN_Prop_Tree_01.Materials.MI_Tree03_Canopy_02",
                0,
                Some("GEN_Prop_Tree_01.Textures.T_Maple_Canopy_02_D"),
                None,
            ),
            RenderFamily::Foliage
        );
        assert_eq!(
            classify_family(
                "GEN_Prop_Foliage_01.Materials.MI_tree_01_frame_01",
                0,
                Some("GEN_Prop_Foliage_01.Textures.T_tree_01_frame_01_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "GEN_CDA_Wrapped_Trees.MI_TreeWrapping_Tape",
                0,
                Some("GEN_CDA_Wrapped_Trees.T_treeWrapping-tape_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "GEN_CDA_Wrapped_Trees.M_GEN_CDA_Tree_Wrapping_Cage",
                0,
                Some("GEN_CDA_Wrapped_Trees.T_treeWrapping_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family("glass", 2, None, None),
            RenderFamily::Translucent
        );
        assert_eq!(
            classify_family("industrial_grid", 0, Some("grid_d"), Some("grid_o")),
            RenderFamily::AlphaTested
        );
        assert_eq!(
            classify_family(
                "GEN_Dirt_Mounds.MI_GEN_Dirt_Mound_Tube_light",
                1,
                Some("GEN_Dirt_Mounds.T_GEN_dirt_mounds_D"),
                Some("GEN_Dirt_Mounds.T_GEN_dirt_mounds_O"),
            ),
            RenderFamily::Decal
        );
        assert_eq!(
            classify_family("van_glass", 0, Some("glass_d"), Some("glass_o")),
            RenderFamily::Translucent
        );
        assert_eq!(
            classify_family("lamp_glow", 3, None, None),
            RenderFamily::Emissive
        );
        assert_eq!(
            classify_family("river_water", 2, None, None),
            RenderFamily::Translucent
        );
        assert_eq!(
            classify_family(
                "EnvWater_FX.Materials.MI_River_Overground",
                2,
                Some("EnvWater_FX.Textures.T_Camden_River_FX2_D"),
                None,
            ),
            RenderFamily::Water
        );
        assert_eq!(
            classify_family(
                "LDN_Camden_Various_01.Materials.MIC_Camden_Water_Tower_Metal",
                0,
                Some("LDN_Camden_Various_01.Textures.T_Camden_WaterTower_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family("GEN_Industrial_Set_01.MI_GEN_waterpump", 0, None, None),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "LDN_Camden_Terrain_01.Materials.MI_LDN_Camden_Terrain_02_INST",
                0,
                Some("LDN_Camden_Terrain_01.Textures.T_Terrain_Canal_01_Grass_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "MI_Tube_Vista_Deadtree_Trunk",
                0,
                Some("T_Tube_Vista_DeadTree_Branch_D"),
                None,
            ),
            RenderFamily::Opaque
        );
        assert_eq!(
            classify_family(
                "MI_Tube_Vista_Deadtree_Branches",
                0,
                Some("T_Tube_Vista_DeadTree_Branch_D"),
                Some("T_Tube_Vista_DeadTree_Branch_O"),
            ),
            RenderFamily::Foliage
        );
    }

    #[test]
    fn converts_a_white_dxt1_mask_to_opaque_dxt5_alpha() {
        let white_mask = [0xff, 0xff, 0, 0, 0, 0, 0, 0];
        assert_eq!(dxt5_alpha_block(&white_mask), [255, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            dxt5_alpha_values(&[0; 16]),
            [255, 0, 73, 146, 36, 73, 146, 36]
        );
    }

    #[test]
    fn accepts_discarded_mip_bulk_header_sentinels() {
        let bytes = [
            0x21, 0, 0, 0, // separate-file + unused
            0, 0, 0, 0, // element count
            0xff, 0xff, 0xff, 0xff, // disk size sentinel
            0xff, 0xff, 0xff, 0xff, // file offset sentinel
        ];
        let mut cursor = NativeCursor::new(&bytes, 0);
        let header = cursor.bulk_header().unwrap();
        assert_eq!(header.flags, 0x21);
        assert_eq!(header.element_count, 0);
        assert_eq!(header.disk_size, 0);
        assert_eq!(header.file_offset, 0);
    }
}
