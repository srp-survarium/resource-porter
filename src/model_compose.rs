// SPDX-License-Identifier: GPL-3.0-or-later
//! One static model made of the surfaces of several, each moved by its own
//! translation: an optic placed on a weapon's mount with the mount's rail,
//! or a separate reticle model baked into an optic's aimed model as a
//! surface of its own.
//!
//! A static model directory (`<name>.model`) holds `settings` (the surfaces'
//! materials, `lod_hierrarchy.LOD0.surfaces`), `render/export_properties`
//! (the model's bounding box and locators) and, for each surface,
//! `render/<surface>/converted_model` and `render/<surface>/export_properties`.
//! A `converted_model` is a vertex block, `u32 3`, `u32 4 + 28 n`, `u32 n`,
//! then `n` 28-byte vertices whose first 12 bytes are the float3 position,
//! followed by the index block, which stays as it is. Only translations are
//! applied: normals, tangents and uvs keep their bytes.
//!
//! The composed settings are the canonical static-model settings the first
//! part has (its flags), with one `material_settings` entry per surface
//! (`material_name`, renamed through the given map, and `used_in_portals`
//! false), `game_material_settings.default` and the surfaces in LOD0 in the
//! order the parts give them. The root `export_properties` is the first
//! part's, its locators moved by the first part's translation, its bounding
//! box the union of every surface's.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::binary_config::{BinaryConfig, ConfigValue, NamedValue};

/// One model's contribution: its directory, a translation, and the surfaces
/// taken (all of LOD0 when empty).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub model: PathBuf,
    pub translation: [f32; 3],
    pub surfaces: Vec<String>,
}

impl Part {
    /// `DIR[=TX,TY,TZ][:SURFACE,SURFACE...]`.
    pub fn parse(text: &str) -> Result<Self> {
        let (rest, surfaces) = match text.rsplit_once(':') {
            Some((rest, list)) if !list.contains('/') && !list.contains('=') => (
                rest,
                list.split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            _ => (text, Vec::new()),
        };
        let (model, translation) = match rest.rsplit_once('=') {
            Some((model, numbers)) => {
                let values: Vec<f32> = numbers
                    .split(',')
                    .map(|n| n.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("{text}: the translation is not three numbers"))?;
                ensure!(
                    values.len() == 3,
                    "{text}: the translation is not three numbers"
                );
                (model, [values[0], values[1], values[2]])
            }
            None => (rest, [0.0; 3]),
        };
        Ok(Self {
            model: PathBuf::from(model),
            translation,
            surfaces,
        })
    }
}

const VERTEX_SIZE: usize = 28;

/// `data` (a converted_model) with every vertex position moved by `t`.
pub fn translate_mesh(data: &[u8], t: [f32; 3]) -> Result<Vec<u8>> {
    ensure!(
        data.len() >= 12,
        "a converted_model shorter than its header"
    );
    let word = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let (kind, size, count) = (word(0), word(4), word(8));
    ensure!(
        kind == 3 && size == 4 + VERTEX_SIZE * count && data.len() >= 12 + VERTEX_SIZE * count,
        "not a static mesh of {VERTEX_SIZE}-byte vertices (block {kind}, {size} bytes, {count} vertices)"
    );
    let mut out = data.to_vec();
    for i in 0..count {
        let at = 12 + VERTEX_SIZE * i;
        for (axis, delta) in t.iter().enumerate() {
            let place = at + 4 * axis;
            let value = f32::from_le_bytes(out[place..place + 4].try_into().unwrap()) + delta;
            out[place..place + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

fn read_config(path: &Path) -> Result<ConfigValue> {
    let bytes = fs::read(path).with_context(|| format!("{}", path.display()))?;
    Ok(BinaryConfig::parse(&bytes)
        .with_context(|| format!("{}", path.display()))?
        .root)
}

fn write_config(path: &Path, root: ConfigValue) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, BinaryConfig { root }.to_bytes()?)?;
    Ok(())
}

fn named(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
    ConfigValue::Named(
        entries
            .into_iter()
            .map(|(name, value)| NamedValue {
                name: name.to_owned(),
                value,
            })
            .collect(),
    )
}

fn lod0_surfaces(settings: &ConfigValue) -> Result<Vec<String>> {
    let Some(ConfigValue::Indexed(list)) = settings
        .named("lod_hierrarchy")
        .and_then(|lod| lod.named("LOD0"))
        .and_then(|lod| lod.named("surfaces"))
    else {
        bail!("settings without lod_hierrarchy.LOD0.surfaces");
    };
    list.iter()
        .map(|s| match s {
            ConfigValue::String(name) => Ok(name.clone()),
            _ => bail!("a LOD0 surface that is not a name"),
        })
        .collect()
}

fn material_of(settings: &ConfigValue, surface: &str) -> Result<String> {
    match settings
        .named("material_settings")
        .and_then(|m| m.named(surface))
        .and_then(|m| m.named("material_name"))
    {
        Some(ConfigValue::String(name)) => Ok(name.clone()),
        _ => bail!("settings name no material for surface {surface}"),
    }
}

fn moved(value: &ConfigValue, t: [f32; 3]) -> ConfigValue {
    match value {
        ConfigValue::Float3([x, y, z]) => ConfigValue::Float3([x + t[0], y + t[1], z + t[2]]),
        other => other.clone(),
    }
}

/// A surface's or a model's export_properties with its bounding box (and
/// sphere centre, locator positions) moved by `t`.
fn moved_properties(value: &ConfigValue, t: [f32; 3]) -> ConfigValue {
    let mut out = value.clone();
    for path in [
        "bounding_box.min",
        "bounding_box.max",
        "bounding_sphere.center",
    ] {
        let (table, key) = path.split_once('.').unwrap();
        if let Some(v) = out.named_mut(table).and_then(|table| table.named_mut(key)) {
            *v = moved(v, t);
        }
    }
    if let Some(ConfigValue::Indexed(locators)) = out.named_mut("locators") {
        for locator in locators {
            if let Some(position) = locator.named_mut("position") {
                *position = moved(position, t);
            }
        }
    }
    out
}

fn bounds(properties: &ConfigValue) -> Option<([f32; 3], [f32; 3])> {
    let read = |key: &str| match properties.named("bounding_box").and_then(|b| b.named(key)) {
        Some(ConfigValue::Float3(v)) => Some(*v),
        _ => None,
    };
    Some((read("min")?, read("max")?))
}

/// Compose `parts` into the model directory `out` (a new one), each
/// material name renamed through `materials` (from, to).
pub fn compose(parts: &[Part], materials: &BTreeMap<String, String>, out: &Path) -> Result<()> {
    ensure!(!parts.is_empty(), "no part to compose");
    ensure!(!out.exists(), "{}: already there", out.display());
    let first_settings = read_config(&parts[0].model.join("settings"))?;
    let mut surfaces: Vec<(String, String)> = Vec::new();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for part in parts {
        let settings = read_config(&part.model.join("settings"))?;
        let available = lod0_surfaces(&settings)?;
        let taken = if part.surfaces.is_empty() {
            available.clone()
        } else {
            for s in &part.surfaces {
                ensure!(
                    available.contains(s),
                    "{}: no surface {s}",
                    part.model.display()
                );
            }
            part.surfaces.clone()
        };
        for surface in taken {
            ensure!(
                surfaces.iter().all(|(s, _)| *s != surface),
                "two parts give the surface {surface}"
            );
            let source = part.model.join("render").join(&surface);
            let target = out.join("render").join(&surface);
            fs::create_dir_all(&target)?;
            let mesh = fs::read(source.join("converted_model"))
                .with_context(|| format!("{}/converted_model", source.display()))?;
            fs::write(
                target.join("converted_model"),
                translate_mesh(&mesh, part.translation)?,
            )?;
            let properties = moved_properties(
                &read_config(&source.join("export_properties"))?,
                part.translation,
            );
            if let Some((a, b)) = bounds(&properties) {
                for i in 0..3 {
                    lo[i] = lo[i].min(a[i]);
                    hi[i] = hi[i].max(b[i]);
                }
            }
            write_config(&target.join("export_properties"), properties)?;
            let material = material_of(&settings, &surface)?;
            let material = materials.get(&material).cloned().unwrap_or(material);
            surfaces.push((surface, material));
        }
    }
    // the settings: the first part's flags, the composed surfaces
    let mut entries: Vec<NamedValue> = Vec::new();
    if let ConfigValue::Named(values) = &first_settings {
        for value in values {
            if !matches!(
                value.name.as_str(),
                "material_settings" | "game_material_settings" | "lod_hierrarchy"
            ) {
                entries.push(value.clone());
            }
        }
    }
    entries.push(NamedValue {
        name: "material_settings".to_owned(),
        value: ConfigValue::Named(
            surfaces
                .iter()
                .map(|(surface, material)| NamedValue {
                    name: surface.clone(),
                    value: named(vec![
                        ("material_name", ConfigValue::String(material.clone())),
                        ("used_in_portals", ConfigValue::Boolean(false)),
                    ]),
                })
                .collect(),
        ),
    });
    entries.push(NamedValue {
        name: "game_material_settings".to_owned(),
        value: named(vec![(
            "default",
            named(vec![
                ("game_material_id", ConfigValue::Integer(65535)),
                (
                    "game_material_name",
                    ConfigValue::String("not assigned or deleted".to_owned()),
                ),
            ]),
        )]),
    });
    entries.push(NamedValue {
        name: "lod_hierrarchy".to_owned(),
        value: named(vec![(
            "LOD0",
            named(vec![(
                "surfaces",
                ConfigValue::Indexed(
                    surfaces
                        .iter()
                        .map(|(s, _)| ConfigValue::String(s.clone()))
                        .collect(),
                ),
            )]),
        )]),
    });
    write_config(&out.join("settings"), ConfigValue::Named(entries))?;
    // the root export_properties: the first part's, moved, the union box
    let mut root = moved_properties(
        &read_config(&parts[0].model.join("render/export_properties"))?,
        parts[0].translation,
    );
    if lo[0].is_finite()
        && let Some(b) = root.named_mut("bounding_box")
    {
        if let Some(v) = b.named_mut("min") {
            *v = ConfigValue::Float3(lo);
        }
        if let Some(v) = b.named_mut("max") {
            *v = ConfigValue::Float3(hi);
        }
    }
    if let Some(v) = root.named_mut("src_filename") {
        *v = ConfigValue::String("composed by vostok-port compose-model".to_owned());
    }
    write_config(&out.join("render/export_properties"), root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(points: &[[f32; 3]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(3u32.to_le_bytes());
        out.extend((4 + VERTEX_SIZE * points.len()).to_le_bytes()[..4].iter());
        out.extend((points.len() as u32).to_le_bytes());
        for p in points {
            for v in p {
                out.extend(v.to_le_bytes());
            }
            out.extend([7u8; 16]);
        }
        out.extend(4u32.to_le_bytes());
        out.extend(10u32.to_le_bytes());
        out.extend(3u32.to_le_bytes());
        out.extend([0u8, 0, 1, 0, 2, 0]);
        out
    }

    fn model(dir: &Path, surfaces: &[(&str, &str, &[[f32; 3]])]) {
        for (surface, _, points) in surfaces {
            let at = dir.join("render").join(surface);
            fs::create_dir_all(&at).unwrap();
            fs::write(at.join("converted_model"), mesh(points)).unwrap();
            write_config(
                &at.join("export_properties"),
                named(vec![(
                    "bounding_box",
                    named(vec![
                        ("min", ConfigValue::Float3(points[0])),
                        ("max", ConfigValue::Float3(points[points.len() - 1])),
                    ]),
                )]),
            )
            .unwrap();
        }
        write_config(
            &dir.join("settings"),
            named(vec![
                ("calculate_ao", ConfigValue::Boolean(false)),
                (
                    "material_settings",
                    ConfigValue::Named(
                        surfaces
                            .iter()
                            .map(|(s, m, _)| NamedValue {
                                name: (*s).to_owned(),
                                value: named(vec![(
                                    "material_name",
                                    ConfigValue::String((*m).to_owned()),
                                )]),
                            })
                            .collect(),
                    ),
                ),
                (
                    "lod_hierrarchy",
                    named(vec![(
                        "LOD0",
                        named(vec![(
                            "surfaces",
                            ConfigValue::Indexed(
                                surfaces
                                    .iter()
                                    .map(|(s, _, _)| ConfigValue::String((*s).to_owned()))
                                    .collect(),
                            ),
                        )]),
                    )]),
                ),
            ]),
        )
        .unwrap();
        write_config(
            &dir.join("render/export_properties"),
            named(vec![
                (
                    "bounding_box",
                    named(vec![
                        ("min", ConfigValue::Float3([0.0; 3])),
                        ("max", ConfigValue::Float3([1.0; 3])),
                    ]),
                ),
                (
                    "locators",
                    ConfigValue::Indexed(vec![named(vec![
                        ("name", ConfigValue::String("aim".to_owned())),
                        ("position", ConfigValue::Float3([0.0, 0.5, 0.0])),
                    ])]),
                ),
                ("src_filename", ConfigValue::String("x".to_owned())),
            ]),
        )
        .unwrap();
    }

    fn positions(path: &Path) -> Vec<[f32; 3]> {
        let data = fs::read(path).unwrap();
        let n = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        (0..n)
            .map(|i| {
                let at = 12 + VERTEX_SIZE * i;
                let f = |k: usize| {
                    f32::from_le_bytes(data[at + 4 * k..at + 4 * k + 4].try_into().unwrap())
                };
                [f(0), f(1), f(2)]
            })
            .collect()
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn parts_parse_with_and_without_translation_and_surfaces() {
        let p = Part::parse("a/b.model=0.5,-1,2:x_s,y_s").unwrap();
        assert_eq!(p.model, PathBuf::from("a/b.model"));
        assert_eq!(p.translation, [0.5, -1.0, 2.0]);
        assert_eq!(p.surfaces, ["x_s", "y_s"]);
        let p = Part::parse("/abs/c.model").unwrap();
        assert_eq!((p.translation, p.surfaces.len()), ([0.0; 3], 0));
        assert!(Part::parse("c.model=1,2").is_err());
    }

    #[test]
    fn a_reticle_is_baked_into_an_aimed_model_as_its_own_surface() {
        let dir = std::env::temp_dir().join(format!("compose-model-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (optic, reticle, rail, out) = (
            dir.join("optic.model"),
            dir.join("dot.model"),
            dir.join("rail.model"),
            dir.join("out.model"),
        );
        model(
            &optic,
            &[
                (
                    "body_s",
                    "modules/body",
                    &[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
                ),
                (
                    "lens_s",
                    "modules/lens",
                    &[[0.2, 0.2, 0.2], [0.3, 0.3, 0.3]],
                ),
            ],
        );
        model(
            &reticle,
            &[(
                "dot_s",
                "loc/dot_red",
                &[[-0.001, -0.001, 0.0], [0.001, 0.001, 0.0]],
            )],
        );
        model(
            &rail,
            &[
                ("rail_s", "loc/rail", &[[0.0, -0.1, 0.0], [1.0, 0.0, 1.0]]),
                ("other_s", "loc/x", &[[0.0; 3], [1.0; 3]]),
            ],
        );
        let parts = vec![
            Part::parse(&format!("{}=0,0.1,-1", optic.display())).unwrap(),
            Part::parse(&format!("{}=0,0.6,-0.4", reticle.display())).unwrap(),
            Part::parse(&format!("{}:rail_s", rail.display())).unwrap(),
        ];
        let renames = BTreeMap::from([("modules/lens".to_owned(), "loc/lens".to_owned())]);
        compose(&parts, &renames, &out).unwrap();
        // every vertex moved by its part's translation, the other bytes kept
        assert!(near(
            positions(&out.join("render/body_s/converted_model"))[1],
            [1.0, 1.1, 0.0]
        ));
        assert!(near(
            positions(&out.join("render/dot_s/converted_model"))[0],
            [-0.001, 0.599, -0.4]
        ));
        assert!(near(
            positions(&out.join("render/rail_s/converted_model"))[0],
            [0.0, -0.1, 0.0]
        ));
        let body = fs::read(out.join("render/body_s/converted_model")).unwrap();
        assert_eq!(&body[12 + 12..12 + 28], &[7u8; 16]);
        assert_eq!(&body[body.len() - 6..], &[0u8, 0, 1, 0, 2, 0]);
        assert!(!out.join("render/other_s").exists());
        // the settings: the surfaces in order, the materials renamed
        let settings = read_config(&out.join("settings")).unwrap();
        assert_eq!(
            lod0_surfaces(&settings).unwrap(),
            ["body_s", "lens_s", "dot_s", "rail_s"]
        );
        assert_eq!(material_of(&settings, "lens_s").unwrap(), "loc/lens");
        assert_eq!(material_of(&settings, "dot_s").unwrap(), "loc/dot_red");
        assert_eq!(
            settings.named("calculate_ao"),
            Some(&ConfigValue::Boolean(false))
        );
        // the root box spans every surface; the first part's locator moved
        let root = read_config(&out.join("render/export_properties")).unwrap();
        let (lo, hi) = bounds(&root).unwrap();
        assert!(near(lo, [-0.001, -0.1, -1.0]) && near(hi, [1.0, 1.1, 1.0]));
        let ConfigValue::Indexed(locators) = root.named("locators").unwrap() else {
            panic!()
        };
        let Some(ConfigValue::Float3(aim)) = locators[0].named("position") else {
            panic!()
        };
        assert!(near(*aim, [0.0, 0.6, -1.0]));
        // a second compose to the same place, or a surface given twice, is refused
        assert!(compose(&parts, &renames, &out).is_err());
        let twice = vec![parts[0].clone(), parts[0].clone()];
        assert!(compose(&twice, &renames, &dir.join("twice.model")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_mesh_of_another_layout_is_refused() {
        let mut bad = mesh(&[[0.0; 3]]);
        bad[4] = 99;
        assert!(translate_mesh(&bad, [1.0; 3]).is_err());
    }
}
