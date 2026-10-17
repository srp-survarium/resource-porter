// SPDX-License-Identifier: GPL-3.0-or-later
//! How an imported material's surface reflects light in 0.26g.
//!
//! 0.26 kept most of 0.21's map materials under the same paths, but re-tuned
//! their finish for its own lighting and re-authored many of their finish
//! maps. 0.21d's tiled metals have `constant_fresnel` 0.7, a polished-metal
//! reflectance, where 0.26g has 0.04 to 0.1 (`tiled/metal/metal_column`,
//! `metal_radio_2`, `metal_03`, `rabica`, `metal_rust_03`), and their gloss
//! maps are about three times brighter (`metal_column_02_spec` averages 0.18
//! in 0.21d and 0.06 in 0.26g, `metal_column_smap` 0.13 and 0.03). Lit by
//! 0.26g's environment probes they reflected their surroundings like
//! polished steel: a glossy sheen over rusty metal.
//!
//! `*_roughness` is gloss in 0.26g (see `outfit_026::Finish`); the G-buffer
//! writes `min + (max - min) * texel` for a finish read from a map.
use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::ConfigValue;

/// The values of a material that say how its surface reflects: gloss,
/// Fresnel reflectance and specular colour.
const FINISH_VALUES: &[&str] = &[
    "constant_fresnel",
    "constant_fresnel_alpha",
    "constant_fresnel_color",
    "constant_fresnel_intensity",
    "constant_fresnel_max",
    "constant_fresnel_min",
    "constant_fresnel_power",
    "constant_roughness",
    "constant_roughness_max",
    "constant_roughness_min",
    "constant_roughness_v_direction",
    "constant_specular_color",
    "constant_specular_color_multiplier",
    "constant_specular_intensity_max",
    "constant_specular_intensity_min",
    "constant_specular_mask",
    "constant_specular_power",
    "constant_specular_power_max",
    "constant_specular_power_min",
    "fresnel_at_0_degree",
    "fresnel_attenuation",
    "max_roughness",
    "min_roughness",
    "min_reflection_angle",
    "reflection_power",
    "specular_color_multiplier",
];

/// Finish maps read instead of a constant: (switch, map, min, max, the
/// constant read without the map).
const FINISH_MAPS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "use_troughness",
        "texture_roughness",
        "constant_roughness_min",
        "constant_roughness_max",
        "constant_roughness",
    ),
    (
        "use_tfresnel",
        "texture_fresnel",
        "constant_fresnel_min",
        "constant_fresnel_max",
        "constant_fresnel",
    ),
];

/// Other finish maps, each behind its own switch.
const FINISH_TEXTURES: &[(&str, &str)] = &[
    ("use_tspecular_intensity", "texture_specular_intensity"),
    ("use_tspecular_power", "texture_specular_power"),
    ("use_tspecular_mask", "texture_specular_mask"),
    ("use_tfresnel_alpha", "texture_fresnel_alpha"),
    (
        "use_fresnel_at_0_degree_texture",
        "texture_fresnel_at_0_degree",
    ),
    // The specular map of a vertex-blended second layer.
    (
        "use_vertex_blended_specular",
        "texture_vertex_blended_specular",
    ),
];

fn value<'a>(effect: &'a ConfigValue, name: &str) -> Option<&'a ConfigValue> {
    effect.named(name)?.named("value")
}

fn set_value(effect: &mut ConfigValue, name: &str, replacement: ConfigValue) {
    if let Some(slot) = effect
        .named_mut(name)
        .and_then(|parameter| parameter.named_mut("value"))
    {
        *slot = replacement;
    }
}

fn switch(effect: &ConfigValue, name: &str) -> Option<bool> {
    match value(effect, name)? {
        ConfigValue::Boolean(on) => Some(*on),
        _ => None,
    }
}

/// Gives a converted material the finish of `native`, the 0.26g material of
/// the same path: its finish values and finish maps, in every stage both
/// have. The donor's colour, normal and other maps stay. Returns the native
/// maps it bound; they must resolve to the native textures, not to a donor
/// texture of the same name.
///
/// The shader switches stay the template's. Where the template reads a
/// finish map that the native material does not (`metal_column_05` on its
/// vertex-colour template), min = max = the native constant. The reverse, a
/// constant where native reads a map, has no faithful value without the
/// map's mean and is refused.
pub fn adopt_native_finish(
    material: &mut ConfigValue,
    native: &ConfigValue,
) -> Result<BTreeSet<String>> {
    adopt_native_finish_with(material, native, &mut |_| None)
}

fn number(value: Option<&ConfigValue>) -> Option<f32> {
    match value? {
        ConfigValue::Float(number) => Some(*number),
        ConfigValue::Integer(number) => Some(*number as f32),
        _ => None,
    }
}

/// [`adopt_native_finish`], and where the template reads a constant that
/// the native material reads from a map (an early donor's constant gloss or
/// Fresnel kept by the converter), the constant becomes the native value at
/// the map's mean: `min + (max - min) * mean(map)`. `map_mean` gives the mean
/// of a native texture's first channel in 0..1, as the shader samples it.
pub fn adopt_native_finish_with(
    material: &mut ConfigValue,
    native: &ConfigValue,
    map_mean: &mut dyn FnMut(&str) -> Option<f32>,
) -> Result<BTreeSet<String>> {
    let mut bound = BTreeSet::new();
    let Some(ConfigValue::Named(stages)) = material.named_mut("material") else {
        return Ok(bound);
    };
    for stage in stages {
        let Some(native_effect) = native
            .named("material")
            .and_then(|stages| stages.named(&stage.name))
            .and_then(|stage| stage.named("effect"))
        else {
            continue;
        };
        let Some(effect) = stage.value.named_mut("effect") else {
            continue;
        };
        for name in FINISH_VALUES {
            if value(effect, name).is_some()
                && let Some(native_value) = value(native_effect, name)
            {
                set_value(effect, name, native_value.clone());
            }
        }
        let mut take_map = |effect: &mut ConfigValue, texture: &str| {
            if value(effect, texture).is_some()
                && let Some(map @ ConfigValue::String(name)) = value(native_effect, texture)
            {
                if !name.is_empty() {
                    bound.insert(name.replace('\\', "/"));
                }
                set_value(effect, texture, map.clone());
            }
        };
        for (name, texture, min, max, constant) in FINISH_MAPS {
            match (switch(effect, name), switch(native_effect, name)) {
                (reads, native_reads) if reads == native_reads => take_map(effect, texture),
                (Some(true), Some(false)) => {
                    let native_constant = value(native_effect, constant)
                        .with_context(|| format!("native {} has no {constant}", stage.name))?
                        .clone();
                    set_value(effect, min, native_constant.clone());
                    set_value(effect, max, native_constant);
                    take_map(effect, texture);
                }
                (Some(false), Some(true))
                    if value(effect, constant).is_some()
                        && let Some(ConfigValue::String(map)) = value(native_effect, texture)
                        && let (Some(low), Some(high)) = (
                            number(value(native_effect, min)),
                            number(value(native_effect, max)),
                        )
                        && let Some(mean) = map_mean(map) =>
                {
                    set_value(
                        effect,
                        constant,
                        ConfigValue::Float(low + (high - low) * mean),
                    );
                }
                (reads, native_reads) => bail!(
                    "{}: the template's {name} is {reads:?} where the native material's is \
                     {native_reads:?}; its finish needs a reviewed value",
                    stage.name
                ),
            }
        }
        for (name, texture) in FINISH_TEXTURES {
            if switch(effect, name) == switch(native_effect, name) {
                take_map(effect, texture);
            }
        }
    }
    Ok(bound)
}

/// A constant finish for a donor material that 0.26g has no counterpart of:
/// gloss and Fresnel reflectance, written as min = max = constant so the
/// template's maps and switches add nothing (as `outfit_026::set_finish`).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConstantFinish {
    pub gloss: f32,
    pub fresnel: f32,
}

pub fn set_constant_finish(material: &mut ConfigValue, finish: ConstantFinish) -> Result<()> {
    let Some(ConfigValue::Named(stages)) = material.named_mut("material") else {
        bail!("not a material");
    };
    let mut set = false;
    for stage in stages {
        let Some(effect) = stage.value.named_mut("effect") else {
            continue;
        };
        for (finish_value, names) in [
            (
                finish.gloss,
                [
                    "constant_roughness",
                    "constant_roughness_min",
                    "constant_roughness_max",
                ],
            ),
            (
                finish.fresnel,
                [
                    "constant_fresnel",
                    "constant_fresnel_min",
                    "constant_fresnel_max",
                ],
            ),
        ] {
            for name in names {
                if value(effect, name).is_some() {
                    set_value(effect, name, ConfigValue::Float(finish_value));
                    set = true;
                }
            }
        }
    }
    ensure!(set, "material has no gloss or Fresnel values");
    Ok(())
}

/// A reviewed exception to a material's finish, kept after the native finish
/// or a constant finish: gloss and/or Fresnel reflectance as min = max =
/// constant, so the template's maps and switches add nothing. `reason` says
/// who chose it and why.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinishException {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gloss: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fresnel: Option<f32>,
    pub reason: String,
}

pub fn apply_finish_exception(
    material: &mut ConfigValue,
    exception: &FinishException,
) -> Result<()> {
    ensure!(
        exception.gloss.is_some() || exception.fresnel.is_some(),
        "a finish exception sets gloss or Fresnel"
    );
    ensure!(
        !exception.reason.trim().is_empty(),
        "a finish exception needs its reason"
    );
    let Some(ConfigValue::Named(stages)) = material.named_mut("material") else {
        bail!("not a material");
    };
    let mut set = false;
    for stage in stages {
        let Some(effect) = stage.value.named_mut("effect") else {
            continue;
        };
        for (finish_value, names) in [
            (
                exception.gloss,
                [
                    "constant_roughness",
                    "constant_roughness_min",
                    "constant_roughness_max",
                ],
            ),
            (
                exception.fresnel,
                [
                    "constant_fresnel",
                    "constant_fresnel_min",
                    "constant_fresnel_max",
                ],
            ),
        ] {
            let Some(finish_value) = finish_value else {
                continue;
            };
            for name in names {
                if value(effect, name).is_some() {
                    set_value(effect, name, ConfigValue::Float(finish_value));
                    set = true;
                }
            }
        }
    }
    ensure!(set, "material has no gloss or Fresnel values");
    Ok(())
}

/// Whether a texture slot is one [`adopt_native_finish`] takes.
pub fn is_finish_slot(slot: &str) -> bool {
    FINISH_MAPS.iter().any(|(_, texture, ..)| *texture == slot)
        || FINISH_TEXTURES.iter().any(|(_, texture)| *texture == slot)
}

/// Every texture a material binds, in any stage.
pub fn material_textures(material: &ConfigValue) -> BTreeSet<String> {
    material_texture_slots(material)
        .into_iter()
        .map(|(_, name)| name)
        .collect()
}

/// Every (slot, texture) a material binds, in any stage.
pub fn material_texture_slots(material: &ConfigValue) -> BTreeSet<(String, String)> {
    fn walk(node: &ConfigValue, parameter: &str, out: &mut BTreeSet<(String, String)>) {
        match node {
            ConfigValue::Named(children) => {
                for child in children {
                    let name = if matches!(child.name.as_str(), "value" | "is_model_parameter") {
                        parameter
                    } else {
                        &child.name
                    };
                    walk(&child.value, name, out);
                }
            }
            ConfigValue::String(text)
                if (parameter.starts_with("texture_") || parameter.ends_with("_texture"))
                    && !text.is_empty() =>
            {
                out.insert((parameter.to_owned(), text.replace('\\', "/")));
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(material, "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    fn named(name: &str, value: ConfigValue) -> NamedValue {
        NamedValue {
            name: name.into(),
            value,
        }
    }
    fn parameter(value: ConfigValue) -> ConfigValue {
        ConfigValue::Named(vec![
            named("value", value),
            named("is_model_parameter", ConfigValue::Boolean(true)),
        ])
    }
    fn material(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(vec![named(
            "material",
            ConfigValue::Named(vec![named(
                "g_stage",
                ConfigValue::Named(vec![named(
                    "effect",
                    ConfigValue::Named(
                        entries
                            .into_iter()
                            .map(|(name, value)| named(name, parameter(value)))
                            .collect(),
                    ),
                )]),
            )]),
        )])
    }
    fn get(material: &ConfigValue, name: &str) -> ConfigValue {
        value(
            material
                .named("material")
                .and_then(|stages| stages.named("g_stage"))
                .and_then(|stage| stage.named("effect"))
                .unwrap(),
            name,
        )
        .unwrap()
        .clone()
    }
    fn s(text: &str) -> ConfigValue {
        ConfigValue::String(text.into())
    }
    use ConfigValue::{Boolean as B, Float as F, Integer as I};

    /// 0.21d `tiled/metal/metal_column_02` against its 0.26g namesake: the
    /// donor's polished-metal Fresnel and bright gloss map give way to the
    /// native finish; its colour and normal maps stay.
    #[test]
    fn a_donor_metal_takes_the_native_finish_and_keeps_its_own_colour() {
        let mut converted = material(vec![
            ("use_troughness", B(true)),
            ("use_tfresnel", B(false)),
            (
                "texture_diffuse",
                s("ported/v021d/tiled/metal/metal_column_02"),
            ),
            (
                "texture_normal",
                s("ported/v021d/tiled/metal/metal_column_02_nmap"),
            ),
            ("texture_roughness", s("tiled/metal/metal_column_02_spec")),
            ("texture_fresnel", s("tiled/metal/metal_column_02_spec")),
            ("constant_roughness", F(0.8)),
            ("constant_roughness_min", I(0)),
            ("constant_roughness_max", I(1)),
            ("constant_fresnel", F(0.7)),
            ("constant_fresnel_min", F(0.04)),
            ("constant_fresnel_max", F(0.4)),
            (
                "constant_specular_color_multiplier",
                ConfigValue::Float3([3.0; 3]),
            ),
        ]);
        let native = material(vec![
            ("use_troughness", B(true)),
            ("use_tfresnel", B(false)),
            ("texture_diffuse", s("tiled/metal/metal_column_02")),
            ("texture_normal", s("tiled/metal/metal_column_02_nmap")),
            ("texture_roughness", s("tiled/metal/metal_column_02_smap")),
            ("texture_fresnel", s("tiled/metal/metal_column_02_fmap")),
            ("constant_roughness", F(0.8)),
            ("constant_roughness_min", I(0)),
            ("constant_roughness_max", I(1)),
            ("constant_fresnel", F(0.02)),
            ("constant_fresnel_min", F(0.04)),
            ("constant_fresnel_max", F(0.2)),
            (
                "constant_specular_color_multiplier",
                ConfigValue::Float3([1.0; 3]),
            ),
        ]);
        let bound = adopt_native_finish(&mut converted, &native).unwrap();
        assert_eq!(get(&converted, "constant_fresnel"), F(0.02));
        assert_eq!(get(&converted, "constant_fresnel_max"), F(0.2));
        assert_eq!(
            get(&converted, "constant_specular_color_multiplier"),
            ConfigValue::Float3([1.0; 3])
        );
        assert_eq!(
            get(&converted, "texture_roughness"),
            s("tiled/metal/metal_column_02_smap")
        );
        assert_eq!(
            get(&converted, "texture_diffuse"),
            s("ported/v021d/tiled/metal/metal_column_02")
        );
        assert_eq!(
            get(&converted, "texture_normal"),
            s("ported/v021d/tiled/metal/metal_column_02_nmap")
        );
        assert_eq!(
            bound,
            BTreeSet::from([
                "tiled/metal/metal_column_02_fmap".into(),
                "tiled/metal/metal_column_02_smap".into()
            ])
        );
        // Applying it again changes nothing.
        let once = converted.clone();
        adopt_native_finish(&mut converted, &native).unwrap();
        assert_eq!(converted, once);
    }

    /// `metal_column_05` keeps its vertex-colour template, which reads a
    /// Fresnel map; the native material reads its constant 0.04.
    #[test]
    fn a_template_map_the_native_material_does_not_read_holds_the_native_constant() {
        let mut converted = material(vec![
            ("use_tfresnel", B(true)),
            ("texture_fresnel", s("tiled/metal/metal_column_05_spec")),
            ("constant_fresnel", F(0.7)),
            ("constant_fresnel_min", F(0.04)),
            ("constant_fresnel_max", F(0.3)),
        ]);
        let native = material(vec![
            ("use_tfresnel", B(false)),
            ("texture_fresnel", s("tiled/metal/metal_column_05_fmap")),
            ("constant_fresnel", F(0.04)),
            ("constant_fresnel_min", I(0)),
            ("constant_fresnel_max", I(1)),
        ]);
        adopt_native_finish(&mut converted, &native).unwrap();
        assert_eq!(get(&converted, "use_tfresnel"), B(true));
        assert_eq!(get(&converted, "constant_fresnel_min"), F(0.04));
        assert_eq!(get(&converted, "constant_fresnel_max"), F(0.04));

        let mut constant = material(vec![
            ("use_tfresnel", B(false)),
            ("constant_fresnel", F(0.7)),
        ]);
        let reads_map = material(vec![
            ("use_tfresnel", B(true)),
            ("constant_fresnel", F(0.04)),
        ]);
        assert!(adopt_native_finish(&mut constant, &reads_map).is_err());
    }

    /// 0.11e `mech_cabinet_02` reads a constant Fresnel 0.4 where 0.26g
    /// reads `0..1 * fmap`: the constant becomes the native value at the
    /// map's mean.
    #[test]
    fn a_template_constant_the_native_material_maps_takes_the_maps_mean() {
        let mut converted = material(vec![
            ("use_tfresnel", B(false)),
            ("texture_fresnel", s("")),
            ("constant_fresnel", F(0.4)),
            ("constant_fresnel_min", F(0.04)),
            ("constant_fresnel_max", I(1)),
        ]);
        let native = material(vec![
            ("use_tfresnel", B(true)),
            ("texture_fresnel", s("props/electric/mech_cabinet2_fmap")),
            ("constant_fresnel", F(0.7)),
            ("constant_fresnel_min", I(0)),
            ("constant_fresnel_max", F(0.5)),
        ]);
        adopt_native_finish_with(&mut converted, &native, &mut |map| {
            (map == "props/electric/mech_cabinet2_fmap").then_some(0.02)
        })
        .unwrap();
        assert_eq!(get(&converted, "use_tfresnel"), B(false));
        assert_eq!(get(&converted, "constant_fresnel"), F(0.01));
        // Without the map's mean there is no faithful value.
        let mut again = material(vec![
            ("use_tfresnel", B(false)),
            ("constant_fresnel", F(0.4)),
        ]);
        assert!(adopt_native_finish(&mut again, &native).is_err());
    }

    /// 0.11e `electrical_cabinet_02` (constant Fresnel 1.0, a mirror under
    /// 0.26g's probes) and 0.26g's (constant 0) both read the constant.
    #[test]
    fn an_early_cabinet_constant_fresnel_becomes_the_native_constant() {
        let mut converted = material(vec![
            ("use_tfresnel", B(false)),
            ("constant_fresnel", I(1)),
            ("constant_fresnel_min", F(0.6)),
            ("constant_fresnel_max", I(1)),
        ]);
        let native = material(vec![
            ("use_tfresnel", B(false)),
            ("constant_fresnel", I(0)),
            ("constant_fresnel_min", I(0)),
            ("constant_fresnel_max", F(0.8)),
        ]);
        adopt_native_finish(&mut converted, &native).unwrap();
        assert_eq!(get(&converted, "use_tfresnel"), B(false));
        assert_eq!(get(&converted, "constant_fresnel"), I(0));
    }

    /// The owner's choice of 2026-10-17: the electrical cabinets keep a
    /// little reflectance (Fresnel 0.05) over the native finish (0, 0.03);
    /// gloss and switches stay native.
    #[test]
    fn a_reviewed_exception_sets_fresnel_and_keeps_the_rest() {
        let mut converted = material(vec![
            ("use_tfresnel", B(false)),
            ("constant_fresnel", I(0)),
            ("constant_fresnel_min", I(0)),
            ("constant_fresnel_max", F(0.8)),
            ("constant_roughness", F(0.5)),
        ]);
        let exception = FinishException {
            gloss: None,
            fresnel: Some(0.05),
            reason: "owner's choice".into(),
        };
        apply_finish_exception(&mut converted, &exception).unwrap();
        for name in [
            "constant_fresnel",
            "constant_fresnel_min",
            "constant_fresnel_max",
        ] {
            assert_eq!(get(&converted, name), F(0.05));
        }
        assert_eq!(get(&converted, "constant_roughness"), F(0.5));
        assert_eq!(get(&converted, "use_tfresnel"), B(false));
        let unexplained = FinishException {
            reason: " ".into(),
            ..exception
        };
        assert!(apply_finish_exception(&mut converted, &unexplained).is_err());
    }

    #[test]
    fn a_constant_finish_overrides_the_maps_range() {
        let mut converted = material(vec![
            ("use_troughness", B(false)),
            ("constant_roughness", F(0.5)),
            ("constant_roughness_min", I(0)),
            ("constant_roughness_max", I(1)),
            ("constant_fresnel", F(0.04)),
            ("constant_fresnel_min", F(0.04)),
            ("constant_fresnel_max", I(1)),
        ]);
        set_constant_finish(
            &mut converted,
            ConstantFinish {
                gloss: 0.29,
                fresnel: 0.1,
            },
        )
        .unwrap();
        for name in [
            "constant_roughness",
            "constant_roughness_min",
            "constant_roughness_max",
        ] {
            assert_eq!(get(&converted, name), F(0.29));
        }
        for name in [
            "constant_fresnel",
            "constant_fresnel_min",
            "constant_fresnel_max",
        ] {
            assert_eq!(get(&converted, name), F(0.1));
        }
    }
}
