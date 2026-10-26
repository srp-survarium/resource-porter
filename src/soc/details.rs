// SPDX-License-Identifier: GPL-3.0-or-later
//! Emit a level's detail objects as v0.100b grass (`detail_objects` in a
//! SoC/CS map recipe).
//!
//! One grass model per X-Ray detail model (its own geometry and UVs), one
//! material for all of them (the level's detail texture: Clear Sky and later
//! compilers bake every detail into one `build_details` sheet), and one
//! vegetation layer per model holding the positions X-Ray would have drawn
//! (see [`super::convert::details`]). One material matters: the grass system
//! merges every instance in a 16 m patch into one draw with the material of
//! the patch's first model.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::build::{TexturePlan, load_template, write_config, write_resource};
use super::codec::cform::Cform;
use super::codec::details::{self, Details};
use super::convert::details::{self as convert, GrassMesh, Ground};
use super::manifest::{DetailObjects, SocMapManifest};
use super::source::{self, SocSource};
use crate::archive::ResourceDb;
use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};
use crate::manifest::BuiltFile;

/// What the conversion produced, for the build record.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DetailReport {
    pub density: f32,
    pub texture: String,
    pub source_models: usize,
    /// Instances per generated model (models X-Ray never places are not
    /// emitted).
    pub instances: BTreeMap<String, usize>,
    pub total_instances: usize,
    /// Triangles per instance at LOD 0 / LOD 1, per generated model.
    pub triangles: BTreeMap<String, [usize; 2]>,
}

/// The decoded details and where they stand, before anything is written.
pub(crate) struct DetailPlan {
    details: Details,
    by_model: BTreeMap<u8, Vec<[f32; 3]>>,
    texture_reference: String,
}

pub(crate) fn plan(
    manifest: &SocMapManifest,
    options: &DetailObjects,
    source: &mut SocSource,
    cform: &Cform,
) -> Result<(DetailPlan, TexturePlan)> {
    let level = &manifest.source_level;
    let bytes = source
        .read_level_file(level, "level.details")
        .context("the recipe asks for detail_objects")?;
    let details = details::parse(&bytes)?;
    let placements = convert::place(&details, &Ground::from_cform(cform), options.density)?;
    let mut by_model: BTreeMap<u8, Vec<[f32; 3]>> = BTreeMap::new();
    for p in placements {
        by_model.entry(p.object).or_default().push(p.position);
    }
    let mut textures: Vec<&str> = by_model
        .keys()
        .map(|id| details.objects[usize::from(*id)].texture.as_str())
        .collect();
    textures.sort_unstable();
    textures.dedup();
    let texture = match textures.as_slice() {
        [one] => (*one).to_owned(),
        [] => bail!("level.details of {level} places no detail on the ground"),
        many => bail!(
            "the details of {level} use {} textures ({many:?}); the grass system draws a 16 m \
             patch with one material, so they would need one atlas",
            many.len()
        ),
    };

    let raw = source
        .read_level_texture(level, &texture)?
        .with_context(|| format!("the detail texture {texture} is missing from the source"))?;
    let info = source::parse_dds(&raw, &texture)?;
    let bytes = source::canonicalize_dds(&raw, info, true)
        .with_context(|| format!("canonicalizing {texture}"))?;
    let target_dds = source::target_texture_path(&manifest.namespace, &texture)?;
    let texture_reference = target_dds
        .strip_prefix("textures/")
        .and_then(|path| path.strip_suffix(".dds"))
        .expect("target texture paths are textures/*.dds")
        .to_owned();
    let texture_plan = TexturePlan {
        source_name: texture,
        target_options: format!("textures/{texture_reference}.options"),
        target_dds,
        info,
        bytes,
        is_normal: false,
        srgb: true,
    };
    Ok((
        DetailPlan {
            details,
            by_model,
            texture_reference,
        },
        texture_plan,
    ))
}

fn model_name(manifest: &SocMapManifest, id: u8) -> String {
    format!("{}/details/detail_{id:02}", manifest.namespace)
}

/// Write the material, the models and the `.veg` files; return the client
/// project's vegetation object.
pub(crate) fn write(
    manifest: &SocMapManifest,
    options: &DetailObjects,
    plan: &DetailPlan,
    target: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<(ConfigValue, DetailReport)> {
    // The material: the grass template with the level's sheet in every
    // colour slot and a flat normal map (X-Ray details carry no normals).
    let material_name = format!("{}/details", manifest.namespace);
    let mut material = load_template(target, &options.material_template)?;
    for slot in [
        "texture_diffuse",
        "texture_roughness",
        "texture_diffuse_power",
        "texture_translucency",
    ] {
        material.root.set_path(
            &format!("material.g_stage.effect.{slot}.value"),
            ConfigValue::String(plan.texture_reference.clone()),
        )?;
    }
    for slot in [
        "texture_normal",
        "texture_detail_normal",
        "texture_alphablended_normal",
        "texture_vertex_blended_normal",
    ] {
        material.root.set_path(
            &format!("material.g_stage.effect.{slot}.value"),
            ConfigValue::String(options.flat_normal.clone()),
        )?;
    }
    write_config(
        overlay,
        &format!("material_instances/{material_name}.material"),
        &material,
        "generated:SoC-detail-material",
        files,
    )?;

    let mut report = DetailReport {
        density: options.density,
        texture: plan.texture_reference.clone(),
        source_models: plan.details.objects.len(),
        ..Default::default()
    };
    let template = options.model_template.trim_end_matches('/');
    let (layer_template, mut vegetation) = vegetation_templates(manifest, options, target)?;
    let mut layers = Vec::new();
    for (&id, positions) in &plan.by_model {
        let object = &plan.details.objects[usize::from(id)];
        let name = model_name(manifest, id);
        let lods = convert::grass_lods(object).with_context(|| format!("detail model {id}"))?;
        write_model(
            target,
            overlay,
            template,
            &name,
            &material_name,
            &plan.texture_reference,
            &lods,
            files,
        )?;
        report.triangles.insert(
            name.clone(),
            [lods[0].indices.len() / 3, lods[1].indices.len() / 3],
        );

        let layer_name = format!("detail_{id:02}");
        let (scale, random_scale) = convert::layer_scale(object);
        let mut layer = layer_template.clone();
        let (lt, size) = rectangle(positions);
        layer.set_path("name", ConfigValue::String(layer_name.clone()))?;
        layer.set_path(
            "filename",
            ConfigValue::String(format!("vegetation/{layer_name}.veg")),
        )?;
        layer.set_path("lt_x", ConfigValue::Float(lt[0]))?;
        layer.set_path("lt_z", ConfigValue::Float(lt[1]))?;
        layer.set_path("size_x", ConfigValue::Integer(size[0]))?;
        layer.set_path("size_z", ConfigValue::Integer(size[1]))?;
        layer.set_path("random_scale", ConfigValue::Float(random_scale))?;
        layer.set_path("random_dir", ConfigValue::Boolean(true))?;
        // X-Ray stands every detail upright (`mRotY` only).
        layer.set_path("use_face_normal", ConfigValue::Boolean(false))?;
        layer.set_path(
            "wind_factor",
            ConfigValue::Integer(i32::from(object.waves())),
        )?;
        layer.set_path("visible", ConfigValue::Boolean(true))?;
        layer.set_path(
            "model_def",
            ConfigValue::Indexed(vec![ConfigValue::Named(vec![
                NamedValue {
                    name: "name".into(),
                    value: ConfigValue::String(name.clone()),
                },
                NamedValue {
                    name: "probability".into(),
                    value: ConfigValue::Float(1.0),
                },
                NamedValue {
                    name: "scale".into(),
                    value: ConfigValue::Float(scale),
                },
            ])]),
        )?;
        write_resource(
            overlay,
            &format!(
                "projects/{}/vegetation/{layer_name}.veg",
                manifest.project_name
            ),
            &convert::encode_vegetation(positions),
            "generated:SoC-detail-positions",
            files,
        )?;
        layers.push(layer);
        report.instances.insert(name, positions.len());
        report.total_instances += positions.len();
    }
    vegetation.set_path("position", ConfigValue::Float3([0.0; 3]))?;
    vegetation.set_path("layers", ConfigValue::Indexed(layers))?;
    Ok((vegetation, report))
}

/// The template project's vegetation object and the named layer in it.
fn vegetation_templates(
    manifest: &SocMapManifest,
    options: &DetailObjects,
    target: &mut ResourceDb,
) -> Result<(ConfigValue, ConfigValue)> {
    let project = load_template(target, &manifest.templates.client_project)?;
    let bindings = &manifest.project_bindings;
    let (Some(objects), Some(kind)) = (&bindings.objects_array_path, &bindings.object_type_path)
    else {
        bail!("detail_objects needs project_bindings.objects_array_path and object_type_path");
    };
    let vegetation = match project.root.named(objects) {
        Some(ConfigValue::Indexed(entries)) => entries
            .iter()
            .find(|o| o.named(kind) == Some(&ConfigValue::String("vegetation".into())))
            .cloned(),
        _ => None,
    }
    .with_context(|| {
        format!(
            "{} has no vegetation object",
            manifest.templates.client_project
        )
    })?;
    let layer = match vegetation.named("layers") {
        Some(ConfigValue::Indexed(layers)) => layers
            .iter()
            .find(|l| l.named("name") == Some(&ConfigValue::String(options.layer_template.clone())))
            .cloned(),
        _ => None,
    }
    .with_context(|| {
        format!(
            "the template vegetation has no {} layer",
            options.layer_template
        )
    })?;
    Ok((layer, vegetation))
}

/// The smallest whole-metre rectangle holding every position.
fn rectangle(positions: &[[f32; 3]]) -> ([f32; 2], [i32; 2]) {
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for p in positions {
        min = [min[0].min(p[0]), min[1].min(p[2])];
        max = [max[0].max(p[0]), max[1].max(p[2])];
    }
    let lt = [min[0].floor(), min[1].floor()];
    (
        lt,
        [
            (max[0] - lt[0]).ceil() as i32 + 1,
            (max[1] - lt[1]).ceil() as i32 + 1,
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn write_model(
    target: &mut ResourceDb,
    overlay: &Path,
    template: &str,
    name: &str,
    material: &str,
    texture: &str,
    lods: &[GrassMesh; 2],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    const LODS: [&str; 2] = ["lod_0sg", "lod_1sg"];
    let prefix = format!("models/{name}.model");
    let mut settings = load_template(target, &format!("{template}/settings"))?;
    for lod in LODS {
        settings.root.set_path(
            &format!("material_settings.{lod}.material_name"),
            ConfigValue::String(material.to_owned()),
        )?;
    }
    write_config(
        overlay,
        &format!("{prefix}/settings"),
        &settings,
        "generated:SoC-detail-model-settings",
        files,
    )?;

    let bounds = (
        std::array::from_fn(|k| lods[0].bounds.0[k].min(lods[1].bounds.0[k])),
        std::array::from_fn(|k| lods[0].bounds.1[k].max(lods[1].bounds.1[k])),
    );
    let mut render = load_template(target, &format!("{template}/render/export_properties"))?;
    set_bounds(&mut render, bounds)?;
    write_config(
        overlay,
        &format!("{prefix}/render/export_properties"),
        &render,
        "generated:SoC-detail-model-bounds",
        files,
    )?;
    for (lod, mesh) in LODS.iter().zip(lods) {
        let mut properties = load_template(
            target,
            &format!("{template}/render/{lod}/export_properties"),
        )?;
        for (field, value) in [
            ("source.indices", mesh.indices.len()),
            ("source.primitives", mesh.indices.len() / 3),
            ("source.vertices", mesh.vertices.len()),
        ] {
            properties
                .root
                .set_path(field, ConfigValue::Integer(i32::try_from(value)?))?;
        }
        properties.root.set_path(
            "source.texture_name",
            ConfigValue::String(texture.to_owned()),
        )?;
        set_bounds(&mut properties, mesh.bounds)?;
        write_config(
            overlay,
            &format!("{prefix}/render/{lod}/export_properties"),
            &properties,
            "generated:SoC-detail-surface",
            files,
        )?;
        write_resource(
            overlay,
            &format!("{prefix}/render/{lod}/converted_model"),
            &mesh.encode()?,
            "converted:SoC-detail-model",
            files,
        )?;
    }
    Ok(())
}

fn set_bounds(config: &mut BinaryConfig, bounds: ([f32; 3], [f32; 3])) -> Result<()> {
    config
        .root
        .set_path("bounding_box.min", ConfigValue::Float3(bounds.0))?;
    config
        .root
        .set_path("bounding_box.max", ConfigValue::Float3(bounds.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layer_rectangle_covers_every_position() {
        let (lt, size) = rectangle(&[[-1.5, 0.0, 2.2], [3.1, 0.0, 7.9]]);
        assert_eq!(lt, [-2.0, 2.0]);
        assert_eq!(size, [7, 7]);
    }

    #[test]
    fn an_empty_recipe_entry_takes_xrays_defaults() {
        let options: DetailObjects = serde_json::from_str("{}").unwrap();
        assert_eq!(options.density, 0.3);
        assert_eq!(options.layer_template, "_gra_green");
        assert!(serde_json::from_str::<DetailObjects>(r#"{"densty": 0.5}"#).is_err());
    }
}
