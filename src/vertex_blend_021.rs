// SPDX-License-Identifier: GPL-3.0-or-later
//! Preserve original unmasked vertex blending in 0.21 imports only.
//!
//! Original diffuse/normal: lerp(base, blended, 1-v).
//! Native base weight: saturate(pow(abs(v), falloff*(1-mask.r)^2)*strength).
//! For authored falloff 1, black mask and new-engine strength 1 restore v.
//! Other channels and permutations remain unchanged. See the accompanying
//! compatibility note for shader paths and the native constant binding proof.
use crate::ConfigValue;
use anyhow::{Result, ensure};

pub const MASK: &str = "ported/v021d/compat/linear_vertex_blend_mask";
pub const MASK_OPTIONS: &str = "textures/tiled/vertex_blend_masks/tiled_mask_01.options";
pub const STRENGTH: &str = "material.g_stage.effect.constant_vertex_blended_strength.value";
pub const MASK_SLOT: &str = "material.g_stage.effect.texture_vertex_blended_tiled_mask.value";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Unchanged,
    UnsupportedUntiledMask,
    UnsupportedFalloff,
    Linear,
}

fn effect(root: &ConfigValue) -> Option<&ConfigValue> {
    root.named("material")?.named("g_stage")?.named("effect")
}
fn value<'a>(effect: &'a ConfigValue, key: &str) -> Option<&'a ConfigValue> {
    effect.named(key)?.named("value")
}
fn one(value: Option<&ConfigValue>) -> bool {
    matches!(value, Some(ConfigValue::Integer(1)))
        || matches!(value, Some(ConfigValue::Float(v)) if *v == 1.0)
}

/// Call only at the 0.21 donor boundary, never for native 0.26 materials.
/// Classification does not alter dependency bindings before resolution.
pub fn classify(donor: &ConfigValue, converted: &ConfigValue) -> Result<Compatibility> {
    let (Some(d), Some(n)) = (effect(donor), effect(converted)) else {
        return Ok(Compatibility::Unchanged);
    };
    if n.named("effect_id") != Some(&ConfigValue::String("g_stage_default".into()))
        || value(n, "vertex_blend_technique") != Some(&ConfigValue::Integer(1))
        || value(d, "use_vertex_blended_mask") != Some(&ConfigValue::Boolean(false))
        || !["use_vertex_blended_diffuse", "use_vertex_blended_normal"]
            .iter()
            .any(|k| value(d, k) == Some(&ConfigValue::Boolean(true)))
    {
        return Ok(Compatibility::Unchanged);
    }
    if value(n, "use_tiled_vertex_blended_mask") != Some(&ConfigValue::Boolean(true)) {
        return Ok(Compatibility::UnsupportedUntiledMask);
    }
    if !one(value(n, "constant_vertex_blended_falloff"))
        || value(d, "constant_vertex_blended_falloff")
            != value(n, "constant_vertex_blended_falloff")
    {
        return Ok(Compatibility::UnsupportedFalloff);
    }
    ensure!(
        value(d, "constant_vertex_blended_strength").is_none()
            && value(d, "texture_vertex_blended_tiled_mask").is_none(),
        "compatibility fields unexpectedly authored by donor"
    );
    ensure!(
        value(n, "constant_vertex_blended_strength").is_some()
            && value(n, "texture_vertex_blended_tiled_mask").is_some(),
        "missing native vertex blend parameters"
    );
    Ok(Compatibility::Linear)
}

impl Compatibility {
    /// Apply after dependency resolution: MASK is generated, not donor artwork.
    pub fn apply(self, root: &mut ConfigValue) -> Result<()> {
        if self == Self::Linear {
            root.set_path(STRENGTH, ConfigValue::Integer(1))?;
            root.set_path(MASK_SLOT, ConfigValue::String(MASK.into()))?;
        }
        Ok(())
    }
}

/// Linear DXT1 black, including Vostok's trailing colour-space byte.
/// Every sample and filtered mip value is zero; this is an ABI adapter.
pub fn black_mask() -> Vec<u8> {
    let mut data = vec![0u8; 137];
    data[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124u32),
        (8, 0x81007),
        (12, 4),
        (16, 4),
        (20, 8),
        (28, 1),
        (76, 32),
        (80, 4),
        (108, 0x1000),
    ] {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    data[84..88].copy_from_slice(b"DXT1");
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;
    fn named(values: Vec<(&str, ConfigValue)>) -> ConfigValue {
        ConfigValue::Named(
            values
                .into_iter()
                .map(|(name, value)| NamedValue {
                    name: name.into(),
                    value,
                })
                .collect(),
        )
    }
    fn material(native: bool) -> ConfigValue {
        let mut fields = vec![
            ("effect_id", ConfigValue::String("g_stage_default".into())),
            (
                "constant_vertex_blended_falloff",
                named(vec![("value", ConfigValue::Integer(1))]),
            ),
            (
                "use_vertex_blended_diffuse",
                named(vec![("value", ConfigValue::Boolean(true))]),
            ),
            (
                "use_vertex_blended_normal",
                named(vec![("value", ConfigValue::Boolean(true))]),
            ),
            (
                "use_vertex_blended_mask",
                named(vec![("value", ConfigValue::Boolean(false))]),
            ),
        ];
        if native {
            fields.extend([
                (
                    "vertex_blend_technique",
                    named(vec![("value", ConfigValue::Integer(1))]),
                ),
                (
                    "use_tiled_vertex_blended_mask",
                    named(vec![("value", ConfigValue::Boolean(true))]),
                ),
                (
                    "constant_vertex_blended_strength",
                    named(vec![("value", ConfigValue::Integer(2))]),
                ),
                (
                    "texture_vertex_blended_tiled_mask",
                    named(vec![(
                        "value",
                        ConfigValue::String("tiled/vertex_blend_masks/tiled_mask_01".into()),
                    )]),
                ),
            ]);
        }
        named(vec![(
            "material",
            named(vec![("g_stage", named(vec![("effect", named(fields))]))]),
        )])
    }
    #[test]
    fn adapter_changes_only_new_engine_fields_and_is_idempotent() {
        let donor = material(false);
        let mut native = material(true);
        let original = native.clone();
        let rule = classify(&donor, &native).unwrap();
        assert_eq!(rule, Compatibility::Linear);
        assert_eq!(native, original); // dependency discovery still sees native texture
        rule.apply(&mut native).unwrap();
        let once = native.clone();
        classify(&donor, &native)
            .unwrap()
            .apply(&mut native)
            .unwrap();
        assert_eq!(native, once);
        native.set_path(STRENGTH, ConfigValue::Integer(2)).unwrap();
        native
            .set_path(
                MASK_SLOT,
                ConfigValue::String("tiled/vertex_blend_masks/tiled_mask_01".into()),
            )
            .unwrap();
        assert_eq!(native, original);
        assert!(classify(&original, &original).is_err()); // native donor has no original contract
    }
    #[test]
    fn unsupported_original_contracts_are_not_retuned() {
        let donor = material(false);
        let native = material(true);
        for (path, value, expected) in [
            (
                "material.g_stage.effect.use_tiled_vertex_blended_mask.value",
                ConfigValue::Boolean(false),
                Compatibility::UnsupportedUntiledMask,
            ),
            (
                "material.g_stage.effect.constant_vertex_blended_falloff.value",
                ConfigValue::Integer(2),
                Compatibility::UnsupportedFalloff,
            ),
            (
                "material.g_stage.effect.vertex_blend_technique.value",
                ConfigValue::Integer(2),
                Compatibility::Unchanged,
            ),
        ] {
            let mut n = native.clone();
            n.set_path(path, value).unwrap();
            let before = n.clone();
            let rule = classify(&donor, &n).unwrap();
            assert_eq!(rule, expected);
            rule.apply(&mut n).unwrap();
            assert_eq!(n, before);
        }
        let mut masked = donor.clone();
        masked
            .set_path(
                "material.g_stage.effect.use_vertex_blended_mask.value",
                ConfigValue::Boolean(true),
            )
            .unwrap();
        assert_eq!(
            classify(&masked, &native).unwrap(),
            Compatibility::Unchanged
        );
    }
    #[test]
    fn native_weight_matches_original_for_every_vertex_byte() {
        for byte in 0..=255 {
            let v = byte as f32 / 255.0;
            let native = (v.abs().powf(1.0 * (1.0_f32 - 0.0).powi(2)) * 1.0).clamp(0.0, 1.0);
            assert_eq!(native, v);
            for (base, blended) in [(0.2, 0.9), (-1.0, 1.0)] {
                let original = base + (blended - base) * (1.0 - v);
                let converted = blended + (base - blended) * native;
                assert!((original - converted).abs() < 0.000001);
            }
        }
    }
}
