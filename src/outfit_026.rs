// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared steps for STK2 outfit pieces retargeted to the native v0.26g player.
use crate::manifest::{BuiltFile, TargetModelSpec, TextureCompression};
use crate::weapon::{add_localization_strings, parse_config, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub(crate) fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}

/// How a ported surface answers light, in 0.26g's terms.
///
/// Despite its name, a material's `constant_roughness` (and the
/// `texture_roughness` map stock artists fill with `*_gloss` textures, e.g.
/// `scavenger_02_mask_gloss`) is gloss: the G-buffer stores one minus it as
/// its smoothness channel, and the lighting reads that channel as roughness.
/// It blurs the probe reflection and fades it as the channel rises
/// (`environment_probe_lighting.ps`). At 0.98 the STK2 ports were mirrors. On
/// high and ultra lighting quality the probe's Cook-Torrance specular then
/// took the sharpest probe mip at full Fresnel, a sheen with glints sliding
/// over the normal map. Medium and low use a weaker approximation and hid it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Finish {
    pub gloss: f32,
    pub fresnel: f32,
}

impl Finish {
    /// Knit, canvas, leather straps. Medians of stock cloth maps (gloss /
    /// fresnel): Hunter's hood .06/.03, Paratrooper's balaclava head .11/.01,
    /// Assault mask hood .09/.44, Corsair mask .22/.12.
    pub(crate) const CLOTH: Self = Self {
        gloss: 0.1,
        fresnel: 0.03,
    };
    /// Painted armour plates and frames. Medians of stock exo helmets: Zubr
    /// UM-4 .09/.13, Molot UM-2 .09/.12, Pilgrim .13/.06.
    pub(crate) const ARMOUR: Self = Self {
        gloss: 0.12,
        fresnel: 0.12,
    };
}

/// Gives a material made from a stock template a constant `finish`. The
/// template's own gloss and Fresnel maps stay bound so its shader permutation
/// is kept, but min = max: the G-buffer writes min + (max - min) * texel, so
/// they add nothing. The specular colour is stock (1), as on every stock
/// character material.
pub(crate) fn set_finish(mat: &mut BinaryConfig, finish: Finish) -> Result<()> {
    for (key, value) in [
        ("constant_fresnel_min", finish.fresnel),
        ("constant_fresnel_max", finish.fresnel),
        ("constant_roughness_min", finish.gloss),
        ("constant_roughness_max", finish.gloss),
    ] {
        mat.root.set_path(
            &format!("material.g_stage.effect.{key}.value"),
            V::Float(value),
        )?;
    }
    mat.root.set_path(
        "material.g_stage.effect.constant_specular_color_multiplier.value",
        V::Float3([1.0; 3]),
    )
}

/// Writes generated and converted files with the port's provenance tag.
pub(crate) struct Writer<'a> {
    pub overlay: &'a Path,
    pub tag: &'a str,
}

impl Writer<'_> {
    pub(crate) fn config(
        &self,
        path: &str,
        c: &BinaryConfig,
        files: &mut Vec<BuiltFile>,
    ) -> Result<()> {
        files.retain(|f| f.path != path);
        write_generated_config(
            self.overlay,
            path,
            c,
            &format!("generated:{}", self.tag),
            files,
        )
    }

    pub(crate) fn raw(&self, path: &str, b: &[u8], files: &mut Vec<BuiltFile>) -> Result<()> {
        let p = crate::archive::safe_join(self.overlay, path)?;
        fs::create_dir_all(p.parent().unwrap())?;
        fs::write(p, b)?;
        files.retain(|f| f.path != path);
        files.push(record_file(path, &format!("converted:{}", self.tag), b));
        Ok(())
    }

    /// Bounds must describe the retargeted vertices, not the borrowed boot.
    pub(crate) fn refit_bounds<'m>(
        &self,
        models: impl IntoIterator<Item = (&'m String, &'m TargetModelSpec)>,
        files: &mut Vec<BuiltFile>,
    ) -> Result<()> {
        for (root, model) in models {
            let mut all_min = [f32::INFINITY; 3];
            let mut all_max = [f32::NEG_INFINITY; 3];
            for surf in model.surfaces.keys() {
                let b = fs::read(
                    self.overlay
                        .join(format!("{root}/render/{surf}/converted_model")),
                )?;
                let (min, max) = converted_bounds(&b)?;
                let p = format!("{root}/render/{surf}/export_properties");
                let mut c = BinaryConfig::parse(&fs::read(self.overlay.join(&p))?)?;
                set_bounds(&mut c, min, max)?;
                self.config(&p, &c, files)?;
                for k in 0..3 {
                    all_min[k] = all_min[k].min(min[k]);
                    all_max[k] = all_max[k].max(max[k]);
                }
            }
            let p = format!("{root}/render/export_properties");
            let mut c = BinaryConfig::parse(&fs::read(self.overlay.join(&p))?)?;
            set_bounds(&mut c, all_min, all_max)?;
            self.config(&p, &c, files)?;
        }
        Ok(())
    }

    /// A material on the native tank-torso template, with the donor's
    /// diffuse and normal maps, `<texture_stem><suffix>.tga`, and `finish`.
    pub(crate) fn cloth_material(
        &self,
        target: &mut ResourceDb,
        port: &str,
        part: &str,
        texture_stem: &Path,
        finish: Finish,
        files: &mut Vec<BuiltFile>,
    ) -> Result<()> {
        let mut mat = parse_config(
            target,
            "material_instances/characters/human/renesanse_tank/renesanse_tank_torso.material",
            "material",
        )?;
        for (role, suffix, compression, srgb) in [
            ("diffuse", "", TextureCompression::Dxt1, true),
            ("normal", "_nmap", TextureCompression::Dxt5Normal, false),
        ] {
            let p = format!("textures/ported/{port}/{part}_{role}");
            let dest = self.overlay.join(format!("{p}.dds"));
            fs::create_dir_all(dest.parent().unwrap())?;
            let mut source = texture_stem.as_os_str().to_owned();
            source.push(format!("{suffix}.tga"));
            crate::armor::convert_texture(Path::new(&source), &dest, compression)?;
            let bytes = crate::duty_026::texture_dds(&fs::read(&dest)?, &p, srgb)?;
            self.raw(&format!("{p}.dds"), &bytes, files)?;
            let mut opts = parse_config(
                target,
                "textures/character/human/renesanse_tank/renesanse_tank_torso.options",
                "texture options",
            )?;
            opts.root
                .set_path("options.srgb", V::Integer(i32::from(srgb)))?;
            opts.root.set_path(
                "options.format",
                s(if srgb { "DXT1" } else { "NORMAL_MAP" }),
            )?;
            self.config(&format!("{p}.options"), &opts, files)?;
            mat.root.set_path(
                &format!("material.g_stage.effect.texture_{role}.value"),
                s(format!("ported/{port}/{part}_{role}")),
            )?;
        }
        set_finish(&mut mat, finish)?;
        self.config(
            &format!("material_instances/ported/{port}/{part}.material"),
            &mat,
            files,
        )
    }

    pub(crate) fn localize(
        &self,
        target: &mut ResourceDb,
        strings: &BTreeMap<String, String>,
        files: &mut Vec<BuiltFile>,
    ) -> Result<()> {
        for lang in [
            "english",
            "original",
            "russian",
            "polish",
            "german",
            "ukrainian",
        ] {
            let p = format!("localization/{lang}/localization");
            let mut c = parse_config(target, &p, "localization")?;
            add_localization_strings(&mut c.root, strings)?;
            self.config(&p, &c, files)?;
        }
        Ok(())
    }
}

/// One wearable item built from a native equipment template.
pub(crate) struct Piece<'a> {
    pub id: i32,
    pub slot: u8,
    pub category: &'a str,
    pub model: &'a str,
    pub part: &'a str,
    pub name: &'a str,
    pub template: String,
    pub icon: [i32; 2],
}

/// Adds the dictionary entry and returns the item config (not yet written)
/// with the server registration entry.
pub(crate) fn add_piece(
    target: &mut ResourceDb,
    items: &mut Vec<V>,
    port: &str,
    piece: &Piece<'_>,
    description: &str,
    strings: &mut BTreeMap<String, String>,
) -> Result<(String, BinaryConfig, Value)> {
    let Piece {
        id,
        slot,
        category,
        model,
        part,
        name,
        ref template,
        icon,
    } = *piece;
    ensure!(
        !items
            .iter()
            .any(|v| v.named("dict_id") == Some(&V::Integer(id))),
        "item ID {id} occupied"
    );
    let path = format!("gameplay/items/armour/{category}/{port}.options");
    let mut item = items
        .iter()
        .find(|v| v.named("cfg_name") == Some(&s(template)))
        .context("native item missing")?
        .clone();
    item.set_path("dict_id", V::Integer(id))?;
    item.set_path("cfg_name", s(&path))?;
    item.set_path("name", s(format!("{port}_{category}")))?;
    items.push(item);
    let mut c = parse_config(target, template, "equipment")?;
    let model_path = format!("ported/{port}/{model}");
    for key in [
        "data",
        "data_full",
        "data_short",
        "data_medium",
        "data_long",
    ] {
        if key == "data" || c.root.named(key).is_some() {
            c.root
                .set_path(&format!("{key}.base_model"), s(&model_path))?;
            c.root.set_path(&format!("{key}.part_name"), s(part))?;
        }
    }
    c.root
        .set_path("parameters.camo_available", V::Boolean(false))?;
    c.root
        .set_path("parameters.decal_available", V::Boolean(false))?;
    c.root
        .set_path("data.additional_models", V::Indexed(vec![]))?;
    c.root
        .set_path("data.additional_model_slots", V::Indexed(vec![]))?;
    let key = format!("st_{port}_{category}");
    c.root.set_path(
        "ui_desc.icon",
        V::Indexed(icon.into_iter().map(V::Integer).collect()),
    )?;
    c.root
        .set_path("ui_desc.text_descriptions.name", s(format!("{key}_name")))?;
    c.root.set_path(
        "ui_desc.text_descriptions.description",
        s(format!("{key}_desc")),
    )?;
    strings.insert(format!("{key}_name"), name.to_owned());
    strings.insert(format!("{key}_desc"), description.to_owned());
    let registration = json!({"dict_id":id,"slot":slot,"additional_slots":Vec::<u8>::new(),"name":name,"template":template,"config":path});
    Ok((path, c, registration))
}

pub(crate) fn set_card_prop(c: &mut BinaryConfig, prop_id: i32, value: &str) -> Result<()> {
    let Some(V::Indexed(props)) = c
        .root
        .named_mut("ui_desc")
        .and_then(|ui| ui.named_mut("props_list"))
    else {
        bail!("item card has no props_list")
    };
    props
        .iter_mut()
        .find(|p| p.named("prop_id") == Some(&V::Integer(prop_id)))
        .with_context(|| format!("item card has no prop {prop_id}"))?
        .set_path("prop_value", s(value))
}

/// Position bounds of a converted skinned surface.
pub(crate) fn converted_bounds(b: &[u8]) -> Result<([f32; 3], [f32; 3])> {
    let n = u32::from_le_bytes(b[8..12].try_into()?) as usize;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for i in 0..n {
        for k in 0..3 {
            let at = 12 + i * 48 + k * 4;
            let v = f32::from_le_bytes(b[at..at + 4].try_into()?);
            ensure!(v.is_finite(), "nonfinite converted position");
            min[k] = min[k].min(v);
            max[k] = max[k].max(v);
        }
    }
    Ok((min, max))
}

fn set_bounds(c: &mut BinaryConfig, min: [f32; 3], max: [f32; 3]) -> Result<()> {
    c.root.set_path("bounding_box.min", V::Float3(min))?;
    c.root.set_path("bounding_box.max", V::Float3(max))?;
    let center = std::array::from_fn(|k| (min[k] + max[k]) / 2.0);
    let radius = (0..3)
        .map(|k| (max[k] - center[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    if c.root.named("bounding_sphere").is_some() {
        c.root.set_path("bounding_sphere.pos", V::Float3(center))?;
        c.root
            .set_path("bounding_sphere.radius", V::Float(radius))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    fn named(name: &str, value: V) -> NamedValue {
        NamedValue {
            name: name.into(),
            value,
        }
    }

    /// A template material as stock ships it: textured gloss and Fresnel over
    /// the full 0..1 range, white specular.
    fn template() -> BinaryConfig {
        let field = |name: &str, value: V| named(name, V::Named(vec![named("value", value)]));
        BinaryConfig {
            root: V::Named(vec![named(
                "material",
                V::Named(vec![named(
                    "g_stage",
                    V::Named(vec![named(
                        "effect",
                        V::Named(vec![
                            field("constant_fresnel_min", V::Integer(0)),
                            field("constant_fresnel_max", V::Integer(1)),
                            field("constant_roughness_min", V::Integer(0)),
                            field("constant_roughness_max", V::Integer(1)),
                            field("constant_specular_color_multiplier", V::Float3([1.0; 3])),
                        ]),
                    )]),
                )]),
            )]),
        }
    }

    fn effect(c: &BinaryConfig, key: &str) -> V {
        let V::Named(fields) = c
            .root
            .named("material")
            .and_then(|m| m.named("g_stage"))
            .and_then(|g| g.named("effect"))
            .and_then(|e| e.named(key))
            .unwrap()
        else {
            panic!("{key}")
        };
        fields[0].value.clone()
    }

    #[test]
    fn a_finish_is_constant_and_as_dull_as_stock_cloth() {
        for finish in [Finish::CLOTH, Finish::ARMOUR] {
            let mut c = template();
            set_finish(&mut c, finish).unwrap();
            // min = max: the template's maps add nothing.
            assert_eq!(effect(&c, "constant_roughness_min"), V::Float(finish.gloss));
            assert_eq!(effect(&c, "constant_roughness_max"), V::Float(finish.gloss));
            assert_eq!(effect(&c, "constant_fresnel_min"), V::Float(finish.fresnel));
            assert_eq!(effect(&c, "constant_fresnel_max"), V::Float(finish.fresnel));
            assert_eq!(
                effect(&c, "constant_specular_color_multiplier"),
                V::Float3([1.0; 3])
            );
            // "Roughness" is gloss: stock cloth and armour maps sit near 0.1.
            // The 0.98 the ports used made mirrors on high lighting quality.
            assert!(finish.gloss <= 0.15, "{finish:?} is glossier than stock");
        }
    }
}
