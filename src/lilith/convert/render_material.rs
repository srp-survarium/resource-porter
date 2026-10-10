// SPDX-License-Identifier: GPL-3.0-or-later
//! Classify Jupiter EX shaders into the target's render families.
//!
//! Where the SoC classifier had to reason from blender classes and sorting
//! priorities, F.E.A.R. states its intent in the material's `.fx` path.
//! The whole game uses 27 shaders across every multiplayer world (see
//! `lilith-audit-shaders`), so the table below *is* the classification --
//! an unlisted shader is a parse-time error, not a guess.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::soc::convert::render_material::RenderFamily;

/// Why a shader converts the way it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderClass {
    Family(RenderFamily),
    /// Not converted, with the reviewed reason.
    Dropped(&'static str),
}

/// The reviewed classification of every shader the multiplayer worlds use.
///
/// Notes on the judgement calls:
///
/// * every `solid\specular*` variant lands on `opaque` -- the target material
///   templates supply their own specular behaviour, and the port binds the
///   source's diffuse and normal maps on top;
/// * `translucent\additive*` and `solid\emissive` are the light glows, screens
///   and holograms: the same additive-on-black idiom as X-Ray's lightplanes,
///   so they take the `emissive` family and its forward additive blend;
/// * `translucent\multiply` is blood and grime laid on surfaces -- a decal.
///   Multiply blending darkens where v0.100b's decal alpha does not, an
///   accepted approximation;
/// * `murky_water_low` is the sewer water, routed through the target's own
///   water family like the SoC ports' rivers;
/// * the sky shaders are dropped because the cloned template project keeps
///   its own sky object -- converted sky geometry would draw a box around
///   the map from inside the template's sky;
/// * `shadowvolume.fxi` is stencil-shadow helper geometry, invisible in the
///   source and meaningless in a renderer that does its own shadows.
pub fn default_shader_classes() -> BTreeMap<&'static str, ShaderClass> {
    use RenderFamily::*;
    use ShaderClass::*;
    [
        ("shaders\\rigid\\solid\\specular.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\specular_bump.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\specular_detail.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\specular_env.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\diffuse.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\cloth.fx", Family(Opaque)),
        ("shaders\\rigid\\solid\\twotonepaint.fx", Family(Opaque)),
        (
            "shaders\\rigid\\prototype\\specular_mirror.fx",
            Family(Opaque),
        ),
        (
            "shaders\\rigid\\solid\\specular_alphatest.fx",
            Family(AlphaTested),
        ),
        (
            "shaders\\rigid\\solid\\specular_alphatest_bump.fx",
            Family(AlphaTested),
        ),
        ("shaders\\rigid\\solid\\glass.fx", Family(Translucent)),
        ("shaders\\rigid\\solid\\glass_env.fx", Family(Translucent)),
        (
            "shaders\\rigid\\solid\\glass_env_flat.fx",
            Family(Translucent),
        ),
        ("shaders\\rigid\\solid\\glass_flat.fx", Family(Translucent)),
        (
            "shaders\\rigid\\translucent\\translucent.fx",
            Family(Translucent),
        ),
        ("shaders\\rigid\\translucent\\cloud.fx", Family(Translucent)),
        (
            "shaders\\rigid\\translucent\\effect\\refract.fx",
            Family(Translucent),
        ),
        (
            "shaders\\rigid\\translucent\\effect\\refract_blur.fx",
            Family(Translucent),
        ),
        (
            "shaders\\rigid\\translucent\\effect\\blur.fx",
            Family(Translucent),
        ),
        (
            "shaders\\rigid\\prototype\\murky_water_low.fx",
            Family(Water),
        ),
        ("shaders\\rigid\\solid\\emissive.fx", Family(Emissive)),
        ("shaders\\rigid\\translucent\\additive.fx", Family(Emissive)),
        (
            "shaders\\rigid\\translucent\\additive_falloff.fx",
            Family(Emissive),
        ),
        ("shaders\\rigid\\translucent\\multiply.fx", Family(Decal)),
        (
            "shaders\\rigid\\solid\\sky.fx",
            Dropped("the template project's own sky replaces source sky geometry"),
        ),
        (
            "shaders\\rigid\\solid\\skybox.fx",
            Dropped("the template project's own sky replaces source sky geometry"),
        ),
        (
            "shaders\\rigid\\shadowvolume.fxi",
            Dropped("stencil-shadow helper geometry, invisible in the source"),
        ),
    ]
    .into_iter()
    .collect()
}

/// Classify one shader path, manifest overrides winning.
///
/// Returns `None` for a reviewed drop. An unknown shader fails: this table
/// covers every shader in retail, so a miss means new content, and new
/// content deserves a review rather than a default.
pub fn classify(
    shader: &str,
    overrides: &BTreeMap<String, RenderFamily>,
) -> Result<Option<RenderFamily>> {
    let normalized = shader.to_ascii_lowercase();
    if let Some(family) = overrides
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&normalized))
        .map(|(_, family)| *family)
    {
        return Ok(Some(family));
    }
    match default_shader_classes().get(normalized.as_str()) {
        Some(ShaderClass::Family(family)) => Ok(Some(*family)),
        Some(ShaderClass::Dropped(_)) => Ok(None),
        None => bail!(
            "shader {shader} has no reviewed v0.100b family; add a \
             render_shader_families entry after reviewing it"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_case_insensitively() {
        let overrides = BTreeMap::new();
        assert_eq!(
            classify("Shaders\\Rigid\\Solid\\Specular.fx", &overrides).unwrap(),
            Some(RenderFamily::Opaque)
        );
        assert_eq!(
            classify("shaders\\rigid\\solid\\sky.fx", &overrides).unwrap(),
            None
        );
    }

    #[test]
    fn unknown_shader_fails() {
        let error = classify("shaders\\rigid\\solid\\mystery.fx", &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("mystery"), "{error}");
    }

    #[test]
    fn override_wins() {
        let overrides: BTreeMap<String, RenderFamily> = [(
            "shaders\\rigid\\solid\\sky.fx".to_owned(),
            RenderFamily::Opaque,
        )]
        .into_iter()
        .collect();
        assert_eq!(
            classify("shaders\\rigid\\solid\\sky.fx", &overrides).unwrap(),
            Some(RenderFamily::Opaque)
        );
    }
}
