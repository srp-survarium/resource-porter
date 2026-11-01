// SPDX-License-Identifier: GPL-3.0-or-later
//! Rifles of Survarium 0.31a2 (September 2015) for v0.26g: the H&K 416A5
//! (763) with its 5.56x45 round (841), and the MSBS Radon (926) with its
//! EOTech 552 sight (819). See `docs/weapons-031-026.md`.
//!
//! 0.31a2 keeps 0.26g's formats: 24-byte configs, 36-byte skinned vertices,
//! the same clips, skeletons, materials, textures and sounds. So the art is
//! copied, with three exceptions found by comparing files both builds ship:
//! meshes carry a 4-byte statistics chunk (110) after 0.26g's two, materials
//! name a few more effect keys (alpha cut-off, parallax), and `raw_options`
//! a `pitch`. Those are dropped, so every file is in a native 0.26g layout.
//!
//! The item configs are not copied: 0.31 renumbered the weapon state types
//! (reload 313, grenade 315, melee 320 against 0.26g's 303, 316, 321) and
//! the item data types (+1: ammo 11, scopes 6), renamed two parameters and
//! added others. Each config is a native 0.26g one of its kind with the
//! donor's own states (clips and sounds), fire modes, recoil, model,
//! particles, texts and lobby poses put in, and the balance below.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::json;

use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::manifest::BuiltFile;
use crate::weapon::{parse_config, prepare_empty_output, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};

const DICT: &str = "gameplay/db_static_dictionaries";
const SOURCE: &str = "converted:0.31a2";

/// A native config of each kind; its states' types and the parts not taken
/// from the donor stay native.
const RIFLE_TEMPLATE: &str = "gameplay/items/weapons/wpn_ak_12.options";
const AMMO_TEMPLATE: &str = "gameplay/items/weapons/ammo/ammo_5.45x39_7n6m.options";
const SCOPE_TEMPLATE: &str = "gameplay/items/scopes/scope_pso1.options";
/// A native material of each base material the donors use.
const MATERIAL_TEMPLATES: [(&str, &str); 2] = [
    ("complex", "material_instances/weapons/ak_12.material"),
    // The Giperon's reticle: the EOTech's glass and dot.
    (
        "forward_default",
        "material_instances/weapons/1p_69_giperon_grid.material",
    ),
];
const RAW_OPTIONS_TEMPLATE: &str =
    "sounds/single/weapons/ak_12/layers/ak_12_shutter_01.raw_options";

/// Provisional balance (owner to confirm).
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub faction: i32,
    pub level: i32,
    pub drop_weight: i32,
    pub base_cost: i32,
    pub rounds_per_minute: i32,
    pub bullet_damage: f32,
    pub bullet_pierce: f32,
    pub stopping_power: f32,
    pub weight: f32,
    pub reload_time: f32,
    pub aim_time: f32,
    pub aim_zoom_factor: f32,
    pub base_dispersion: f32,
    pub magazine: i32,
    pub min_damage_distance: i32,
    pub max_damage_distance: i32,
}

/// One firing layer: a sound collection and its start offset (ms).
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub collection: &'static str,
    pub start_offset: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Rifle {
    pub key: &'static str,
    pub dict_id: i32,
    /// The donor's config (also this item's).
    pub config: &'static str,
    pub stats: Stats,
    /// The ammunition it takes (`items_compatibility`; 0.31's `compatible_ammo`).
    pub ammo: &'static [i32],
    /// The first-person firing sound, rebuilt where the donor's nests 0.31's
    /// per-location reverbs (`None`: the donor's names a native sound):
    /// name, layers, master layer.
    pub fire_composite: Option<(&'static str, &'static [Layer], i32)>,
    /// Cell [row, col] of the lobby's weapon atlas, and the donor's icon.
    pub icon: [i32; 2],
    pub icon_png: &'static str,
    /// A native kill-feed icon.
    pub combat_log_icon: i32,
    /// Texts the donor lacks or gets wrong, by key, in every language.
    pub texts: &'static [(&'static str, Text)],
}

#[derive(Clone, Copy, Debug)]
pub struct Ammo {
    pub dict_id: i32,
    pub config: &'static str,
    pub level: i32,
    pub drop_weight: i32,
    /// For a clip (an ammo's `base_cost` is a clip's).
    pub base_cost: i32,
    pub icon: [i32; 2],
    pub icon_png: &'static str,
    pub texts: &'static [(&'static str, Text)],
}

#[derive(Clone, Copy, Debug)]
pub struct Scope {
    pub dict_id: i32,
    pub config: &'static str,
}

/// One port: the items that ship together.
#[derive(Clone, Copy, Debug)]
pub struct Port {
    pub key: &'static str,
    pub rifles: &'static [Rifle],
    pub ammo: &'static [Ammo],
    pub scopes: &'static [Scope],
}

/// 5.56x45 (0.31: no seller, level 0). A level-9 round, as the 9x19 7N31 of
/// the PP-2000 (the Fringe Settlers' level-9 automatic): level 9, the drop
/// weight of every 0.26g ammo (160), 70 a clip. Ballistics are the basic
/// 5.45x39 7N6M's, the template, as 0.31's 5.56 were its basic round's.
pub const AMMO_556: Ammo = Ammo {
    dict_id: 841,
    config: "gameplay/items/weapons/ammo/ammo_5.56x45.options",
    level: 9,
    drop_weight: 160,
    base_cost: 70,
    icon: [16, 3],
    icon_png: "flash_movies/textures/ui_icons/items/ammo/ammo_1_4.png",
    // 0.31 named it ("5.56x45") but wrote no description.
    texts: &[(
        "st_ammo_556x45_description",
        Text {
            english: "NATO intermediate cartridge with a lead-core bullet. The light bullet's high muzzle velocity gives a flat trajectory.",
            russian: "Промежуточный патрон НАТО с пулей со свинцовым сердечником. Высокая начальная скорость лёгкой пули обеспечивает настильную траекторию.",
            ukrainian: "Проміжний набій НАТО з кулею зі свинцевим осердям. Висока початкова швидкість легкої кулі забезпечує настильну траєкторію.",
            german: "NATO-Mittelpatrone mit Bleikerngeschoss. Die hohe Mündungsgeschwindigkeit des leichten Geschosses sorgt für eine flache Flugbahn.",
            polish: "Nabój pośredni NATO z pociskiem o rdzeniu ołowianym. Duża prędkość wylotowa lekkiego pocisku zapewnia płaski tor lotu.",
        },
    )],
};

/// The H&K 416A5, at the Fringe Settlers' level 9 (the owner's choice,
/// 2026-10-17), balanced against their level-9 weapons and the level-10
/// rifles (0.31: Black Market level 9, 0.32 damage where its AK-12 has 0.35).
pub const HK416: Rifle = Rifle {
    key: "hk416",
    dict_id: 763,
    config: "gameplay/items/weapons/assault/hk416.options",
    stats: Stats {
        faction: 4,
        level: 9,
        drop_weight: 70,
        base_cost: 45_000,
        rounds_per_minute: 850,
        bullet_damage: 0.4,
        bullet_pierce: 0.3,
        stopping_power: 0.15,
        weight: 3.5,
        reload_time: 3.0,
        aim_time: 0.3,
        aim_zoom_factor: 1.2,
        base_dispersion: 0.2,
        magazine: 30,
        min_damage_distance: 50,
        max_damage_distance: 20,
    },
    ammo: &[841],
    // 0.31's own layers (shutter at 60 ms, the report), then the AK-12's
    // first-person tails in place of `weapons/environments/weapons_1st_view`.
    fire_composite: Some((
        "weapons/hk416",
        &[
            Layer {
                collection: "weapons/hk416/hk416_shutter",
                start_offset: 60,
            },
            Layer {
                collection: "weapons/hk416/hk416_loud",
                start_offset: 0,
            },
            Layer {
                collection: "weapons/akm_n/akm_n_close_forest",
                start_offset: 0,
            },
            Layer {
                collection: "weapons/akm_n/akm_n_main_layer",
                start_offset: 0,
            },
        ],
        1,
    )),
    icon: [7, 3],
    icon_png: "flash_movies/textures/ui_icons/items/weapons/weapon_4_12.png",
    // The AK-74M's: the nearest class (a level-10 assault rifle of a
    // small-calibre round); 0.26g draws no AR-15.
    combat_log_icon: 31,
    // 0.31's name has a Cyrillic "А" ("416А5"); there is no description.
    texts: &[
        (
            "st_hk416_name",
            Text {
                english: "H&K 416A5",
                russian: "H&K 416A5",
                ukrainian: "H&K 416A5",
                german: "H&K 416A5",
                polish: "H&K 416A5",
            },
        ),
        (
            "st_hk416_description",
            Text {
                english: "German assault rifle on the AR-15 pattern with a short-stroke gas piston, chambered for 5.56x45 NATO. Supports single shot and automatic fire modes.",
                russian: "Немецкая штурмовая винтовка по схеме AR-15 с газовым поршнем с коротким ходом под патрон 5.56×45 НАТО. Поддерживает одиночный и автоматический режим стрельбы.",
                ukrainian: "Німецька штурмова гвинтівка за схемою AR-15 з газовим поршнем із коротким ходом під набій 5.56×45 НАТО. Підтримує одиночний і автоматичний режим стрільби.",
                german: "Deutsches Sturmgewehr nach AR-15-Muster mit Kurzhub-Gaskolben, Kaliber 5,56x45 NATO. Einzel- und Dauerfeuer.",
                polish: "Niemiecki karabin szturmowy w układzie AR-15 z tłokiem gazowym o krótkim skoku, na amunicję 5,56x45 NATO. Ogień pojedynczy i ciągły.",
            },
        ),
    ],
};

/// The MSBS Radon (to follow the H&K; provisional). 0.31: Scavengers level
/// 8, 750 rpm, 0.32 damage, auto, single and bursts of three, with the
/// EOTech 552 fixed on.
pub const RADON: Rifle = Rifle {
    key: "radon",
    dict_id: 926,
    config: "gameplay/items/weapons/assault/msbs_carbine.options",
    stats: Stats {
        faction: 4,
        level: 9,
        drop_weight: 70,
        base_cost: 43_000,
        rounds_per_minute: 750,
        bullet_damage: 0.42,
        bullet_pierce: 0.3,
        stopping_power: 0.15,
        weight: 3.7,
        reload_time: 3.0,
        aim_time: 0.3,
        aim_zoom_factor: 1.2,
        base_dispersion: 0.2,
        magazine: 30,
        min_damage_distance: 50,
        max_damage_distance: 20,
    },
    ammo: &[841],
    fire_composite: None,
    icon: [8, 3],
    icon_png: "flash_movies/textures/ui_icons/items/weapons/weapon_4_22.png",
    combat_log_icon: 31,
    texts: &[(
        "st_msbs_carbine_description",
        Text {
            english: "Polish modular assault rifle, adopted by the Polish army in 2017 as the Grot, chambered for 5.56x45 NATO. Supports single shot, three-round burst and automatic fire modes.",
            russian: "Польская модульная штурмовая винтовка под патрон 5.56×45 НАТО, принятая на вооружение польской армии в 2017 году под названием Grot. Поддерживает одиночный режим, очередь по три выстрела и автоматический режим стрельбы.",
            ukrainian: "Польська модульна штурмова гвинтівка під набій 5.56×45 НАТО, прийнята на озброєння польської армії 2017 року під назвою Grot. Підтримує одиночний режим, черги по три постріли й автоматичний режим стрільби.",
            german: "Polnisches modulares Sturmgewehr im Kaliber 5,56x45 NATO, 2017 als Grot bei der polnischen Armee eingeführt. Einzelfeuer, Dreischussfeuerstöße und Dauerfeuer.",
            polish: "Polski modułowy karabin szturmowy na amunicję 5,56x45 NATO, przyjęty na uzbrojenie Wojska Polskiego w 2017 roku jako Grot. Ogień pojedynczy, seriami po trzy strzały i ciągły.",
        },
    )],
};

pub const EOTECH: Scope = Scope {
    dict_id: 819,
    config: "gameplay/items/scopes/eotech_552.options",
};

pub const PORTS: [Port; 2] = [
    // The H&K's 0.31 model has its rear sight folded and no front sight: it
    // aims through the EOTech (its aim locator is at the sight's window), as
    // 0.31 fitted it. The Radon, after it, takes the same sight.
    Port {
        key: "hk416",
        rifles: &[HK416],
        ammo: &[AMMO_556],
        scopes: &[EOTECH],
    },
    Port {
        key: "radon",
        rifles: &[RADON],
        ammo: &[],
        scopes: &[],
    },
];

pub fn port(key: &str) -> Result<&'static Port> {
    PORTS
        .iter()
        .find(|p| p.key == key)
        .with_context(|| format!("no port {key}; {:?}", PORTS.map(|p| p.key)))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}
fn s(v: impl Into<String>) -> V {
    V::String(v.into())
}
fn string_of(v: Option<&V>) -> Option<&str> {
    match v {
        Some(V::String(s)) => Some(s),
        _ => None,
    }
}
pub(crate) fn get<'v>(root: &'v V, path: &str) -> Option<&'v V> {
    path.split('.').try_fold(root, |v, part| match v {
        V::Named(_) => v.named(part),
        V::Indexed(items) => items.get(part.parse::<usize>().ok()?),
        _ => None,
    })
}

/// `donor` on `template`'s keys: of each named table the template's keys,
/// with the donor's values where it has them (recursively) and the
/// template's otherwise. An indexed table is the donor's whole; an integer
/// where the template has a float becomes a float.
pub fn conform(donor: &V, template: &V) -> V {
    match (donor, template) {
        (V::Named(d), V::Named(t)) => V::Named(
            t.iter()
                .map(|tf| match d.iter().find(|df| df.name == tf.name) {
                    Some(df) => field(&tf.name, conform(&df.value, &tf.value)),
                    None => tf.clone(),
                })
                .collect(),
        ),
        (V::Integer(i), V::Float(_)) => V::Float(*i as f32),
        (d, _) => d.clone(),
    }
}

/// Removes chunk 110 (0.31's mesh statistics) after 0.26g's chunks 3 and 4.
pub fn strip_mesh_statistics(b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(b.len());
    let mut at = 0;
    let mut ids = vec![];
    while at < b.len() {
        let head = b.get(at..at + 8).context("truncated chunk header")?;
        let id = u32::from_le_bytes(head[..4].try_into()?);
        let size = u32::from_le_bytes(head[4..].try_into()?) as usize;
        let end = at + 8 + size;
        ensure!(end <= b.len(), "truncated chunk {id}");
        match id {
            3 | 4 => out.extend_from_slice(&b[at..end]),
            110 => ensure!(size == 4, "chunk 110 of {size} bytes"),
            other => bail!("unknown mesh chunk {other}"),
        }
        ids.push(id);
        at = end;
    }
    ensure!(ids.starts_with(&[3, 4]), "mesh chunks {ids:?}");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Carrying files
// ---------------------------------------------------------------------------

struct Carry<'a> {
    donor: &'a mut ResourceDb,
    target: &'a mut ResourceDb,
    overlay: &'a Path,
    files: Vec<BuiltFile>,
    written: BTreeSet<String>,
    /// Paths the target has (not shipped), for the record.
    reused: BTreeSet<String>,
}

impl Carry<'_> {
    fn raw(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        let dest = crate::archive::safe_join(self.overlay, path)?;
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
        self.files.push(record_file(path, SOURCE, bytes));
        Ok(())
    }
    fn config(&mut self, path: &str, root: V) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        write_generated_config(
            self.overlay,
            path,
            &BinaryConfig { root },
            SOURCE,
            &mut self.files,
        )
    }
    fn donor_config(&mut self, path: &str) -> Result<BinaryConfig> {
        let bytes = self
            .donor
            .read(path)
            .with_context(|| format!("reading donor {path}"))?;
        ensure!(
            !BinaryConfig::is_compact(&bytes),
            "{path} is not a 0.26g-layout config"
        );
        BinaryConfig::parse(&bytes).with_context(|| format!("parsing donor {path}"))
    }

    /// Ships `path` from the donor, in 0.26g's layout, unless the target has
    /// it, and everything it names.
    fn file(&mut self, path: &str) -> Result<()> {
        if self.written.contains(path) {
            return Ok(());
        }
        if self.target.contains(path) {
            self.reused.insert(path.into());
            return Ok(());
        }
        ensure!(
            self.donor.contains(path),
            "{path} is in neither the target nor the donor"
        );
        if path.ends_with("/converted_model") {
            let bytes = strip_mesh_statistics(&self.donor.read(path)?)?;
            return self.raw(path, &bytes);
        }
        if [".dds", ".ogg", ".rms", ".b-spline", "/bind_pose"]
            .iter()
            .any(|e| path.ends_with(e))
        {
            let bytes = self.donor.read(path)?;
            return self.raw(path, &bytes);
        }
        let config = self.donor_config(path)?;
        let root = if path.ends_with(".material") {
            let base = string_of(get(&config.root, "material.instance.base_material"))
                .context("material without a base")?;
            let (_, template) = MATERIAL_TEMPLATES
                .iter()
                .find(|(b, _)| *b == base)
                .with_context(|| format!("{path}: no native template for base material {base}"))?;
            let template = parse_config(self.target, template, "native material")?;
            conform_material(&config.root, &template.root)?
        } else if path.ends_with(".raw_options") {
            let template = parse_config(self.target, RAW_OPTIONS_TEMPLATE, "native raw_options")?;
            conform(&config.root, &template.root)
        } else {
            config.root
        };
        self.follow(path, &root)?;
        self.config(path, root)
    }

    /// Every file under `prefix` (a model folder), unless the target has it.
    fn folder(&mut self, prefix: &str) -> Result<()> {
        if self
            .target
            .paths_with_prefix(&format!("{prefix}/"))
            .next()
            .is_some()
        {
            self.reused.insert(prefix.into());
            return Ok(());
        }
        let paths: Vec<String> = self
            .donor
            .paths_with_prefix(&format!("{prefix}/"))
            .filter(|p| self.donor.is_file(p))
            .map(str::to_owned)
            .collect();
        ensure!(!paths.is_empty(), "the donor has no {prefix}");
        for p in paths {
            self.file(&p)?;
        }
        Ok(())
    }

    fn clip(&mut self, name: &str) -> Result<()> {
        let name = name.trim_start_matches("resources/");
        self.file(&format!("{name}.b-spline"))?;
        self.file(&format!("{name}.bones_names"))
    }

    fn sound(&mut self, kind: i32, name: &str) -> Result<()> {
        match kind {
            256 => {
                for ext in ["high.ogg", "high.rms", "raw_options"] {
                    self.file(&format!("sounds/single/{name}.{ext}"))?;
                }
                Ok(())
            }
            257 => self.file(&format!("sounds/composite/{name}.composite_sound")),
            258 => self.file(&format!("sounds/collections/{name}.sound_collection")),
            other => bail!("unknown sound resource type {other} for {name}"),
        }
    }

    /// The resources a carried config names.
    fn follow(&mut self, path: &str, root: &V) -> Result<()> {
        if path.ends_with(".material") {
            let mut strings = vec![];
            root.collect_strings(&mut strings);
            let textures: Vec<String> = strings
                .into_iter()
                .filter(|t| t.contains('/'))
                .map(str::to_owned)
                .collect();
            for t in textures {
                let dds = format!("textures/{t}.dds");
                if self.target.contains(&dds) || self.donor.contains(&dds) {
                    self.file(&dds)?;
                    self.file(&format!("textures/{t}.options"))?;
                } else {
                    ensure!(
                        t.starts_with("editor/"),
                        "{path} names {t}, which neither archive has"
                    );
                }
            }
        } else if path.ends_with("/settings") {
            if let Some(V::Named(materials)) = root.named("material_settings") {
                let names: Vec<String> = materials
                    .iter()
                    .filter_map(|m| string_of(m.value.named("material_name")).map(str::to_owned))
                    .collect();
                for name in names {
                    self.file(&format!("material_instances/{name}.material"))?;
                }
            }
        } else if path.ends_with("/skeleton") {
            if let Some(skeleton) = string_of(root.named("skeleton")) {
                let name = skeleton.trim_start_matches("resources/").to_owned();
                self.file(&format!("{name}.skeleton"))?;
            }
        } else if path.ends_with(".composite_sound") || path.ends_with(".sound_collection") {
            let items =
                get(root, "composite_sound.sound_items").or(get(root, "collection.sound_items"));
            if let Some(V::Indexed(items)) = items {
                for item in items.clone() {
                    let kind = match item.named("resource_type") {
                        Some(V::Integer(k)) => *k,
                        None => 256,
                        other => bail!("{path}: resource type {other:?}"),
                    };
                    let name =
                        string_of(item.named("filename")).context("sound item without a file")?;
                    self.sound(kind, name)?;
                }
            }
        }
        Ok(())
    }

    /// The clips and sounds an item config names.
    fn states(&mut self, cfg: &V) -> Result<()> {
        let mut strings = vec![];
        cfg.collect_strings(&mut strings);
        let clips: Vec<String> = strings
            .into_iter()
            .filter(|c| c.starts_with("resources/animations/"))
            .map(str::to_owned)
            .collect();
        for c in clips {
            self.clip(&c)?;
        }
        let mut lists = vec![];
        if let Some(states) = cfg.named("states") {
            collect_sound_lists(states, &mut lists);
        }
        for (kind, name) in lists {
            self.sound(kind, &name)?;
        }
        Ok(())
    }
}

fn collect_sound_lists(v: &V, out: &mut Vec<(i32, String)>) {
    if let V::Named(fields) = v {
        for f in fields {
            if f.name == "sounds" {
                for view in ["first_view", "third_view"] {
                    if let Some(V::Indexed(list)) = f.value.named(view) {
                        for pair in list {
                            if let V::Indexed(p) = pair
                                && let [V::Integer(k), V::String(n)] = p.as_slice()
                            {
                                out.push((*k, n.clone()));
                            }
                        }
                    }
                }
            } else {
                collect_sound_lists(&f.value, out);
            }
        }
    }
}

/// A 0.31 material on 0.26g's effect keys (0.31's extra ones dropped), for
/// each stage it has (`g_stage`, `forward`); the native template has the
/// same base material and stages.
pub(crate) fn conform_material(donor: &V, template: &V) -> Result<V> {
    let mut out = donor.clone();
    let mut stages = 0;
    for stage in ["g_stage", "forward", "lighting"] {
        let Some(d) = get(donor, &format!("material.{stage}.effect")) else {
            continue;
        };
        stages += 1;
        let (V::Named(dk), Some(V::Named(tk))) =
            (d, get(template, &format!("material.{stage}.effect")))
        else {
            bail!("the native template has no {stage} effect");
        };
        // Every texture the donor binds must survive.
        for f in dk {
            if f.name.starts_with("texture_")
                && string_of(get(&f.value, "value")).is_some_and(|v| !v.is_empty())
            {
                ensure!(
                    tk.iter().any(|k| k.name == f.name),
                    "0.26g has no {stage} {}",
                    f.name
                );
            }
        }
        let effect = V::Named(
            dk.iter()
                .filter(|f| tk.iter().any(|k| k.name == f.name))
                .cloned()
                .collect(),
        );
        out.set_path(&format!("material.{stage}.effect"), effect)?;
    }
    ensure!(stages > 0, "material without an effect stage");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Item configs
// ---------------------------------------------------------------------------

/// 0.31 renamed these 0.26g parameters (0.26g's own spelling included).
pub const RENAMED: [(&str, &str); 2] = [
    ("health_modifier", "pain_health_modifier"),
    (
        "stamina_regeneration_speed_modifier",
        "stamina_regenation_speed_modifier",
    ),
];

/// The donor's states that replace the template's; each keeps the
/// template's `state_type` (0.31 renumbered them). Melee and grenade stay
/// the template's: the donor's play `weapons/base/*`, a set 0.26g lacks.
pub const DONOR_STATES: [&str; 5] = ["show", "hide", "idle", "fire", "reload"];

pub(crate) fn card_props(st: &Stats) -> [(i32, String); 8] {
    let fmt = |v: f32| {
        let t = format!("{v:.2}");
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    // The 0.26g rifles' card: damage, pierce and stopping power x 100, aim
    // time, dispersion, distance, rate of fire, weight.
    [
        (1, format!("{}", (st.bullet_damage * 100.0).round() as i32)),
        (3, format!("{}", (st.bullet_pierce * 100.0).round() as i32)),
        (133, fmt(st.aim_time)),
        (
            132,
            format!("{}", (st.stopping_power * 100.0).round() as i32),
        ),
        (2, fmt(st.base_dispersion)),
        (24, format!("{}", st.min_damage_distance)),
        (4, format!("{}", st.rounds_per_minute)),
        (15, fmt(st.weight)),
    ]
}

/// Whether `v` names a clip of 0.31's shared `weapons/base/*` set, which
/// 0.26g lacks; such parts stay the template's (0.26g's `weapons/ak/*`).
fn plays_base(v: &V) -> bool {
    let mut strings = vec![];
    v.collect_strings(&mut strings);
    strings
        .iter()
        .any(|s| s.contains("animations/single/weapons/base/"))
}

pub(crate) fn rifle_config(rifle: &Rifle, donor: &V, template: &V) -> Result<V> {
    let mut donor = donor.clone();
    if let Some(V::Named(params)) = donor.named_mut("parameters") {
        for p in params.iter_mut() {
            if let Some((_, new)) = RENAMED.iter().find(|(old, _)| *old == p.name) {
                p.name = (*new).into();
            }
        }
    }
    let mut cfg = template.clone();
    let params = conform(
        donor.named("parameters").context("no parameters")?,
        template
            .named("parameters")
            .context("template parameters")?,
    );
    cfg.set_path("parameters", params)?;
    for state in DONOR_STATES {
        let theirs = get(&donor, &format!("states.{state}"))
            .with_context(|| format!("the donor has no {state} state"))?;
        ensure!(
            !plays_base(theirs),
            "the donor's {state} state plays weapons/base clips"
        );
        let ours = get(template, &format!("states.{state}")).context("template state")?;
        let mut merged = conform(theirs, ours);
        merged.set_path(
            "state_type",
            ours.named("state_type").context("state_type")?.clone(),
        )?;
        cfg.set_path(&format!("states.{state}"), merged)?;
    }
    for path in [
        "recoil",
        "recoil_animations",
        "object",
        "particles",
        "dispersion",
        "breath_vibration",
        "user_animations",
    ] {
        if let Some(v) = donor.named(path).filter(|v| !plays_base(v)) {
            let v = match template.named(path) {
                Some(t) => conform(v, t),
                None => v.clone(),
            };
            cfg.set_path(path, v)?;
        }
    }
    for path in ["preview", "preview_item"] {
        if let Some(v) =
            get(&donor, &format!("user_animations_in_place.{path}")).filter(|v| !plays_base(v))
        {
            cfg.set_path(&format!("user_animations_in_place.{path}"), v.clone())?;
        }
    }
    if let Some(addons) = donor.named("addons") {
        cfg.set_path("addons", addons.clone())?;
    }
    // The kill-feed icon stays a native one: 0.26g's HUD has 52 (0..=51);
    // 0.31's numbers go further (the H&K's is 63).
    cfg.set_path("ui_desc.combat_log_icon", V::Integer(rifle.combat_log_icon))?;
    for path in ["text_descriptions"] {
        let v = get(&donor, &format!("ui_desc.{path}"))
            .with_context(|| format!("ui_desc.{path}"))?
            .clone();
        cfg.set_path(&format!("ui_desc.{path}"), v)?;
    }
    let st = &rifle.stats;
    for (key, value) in [
        ("faction", V::Integer(st.faction)),
        ("item_level", V::Integer(st.level)),
        ("match_making_level", V::Integer(st.level)),
        ("drop_weight", V::Integer(st.drop_weight)),
        ("rounds_per_minute", V::Integer(st.rounds_per_minute)),
        ("bullet_damage", V::Float(st.bullet_damage)),
        ("bullet_pierce", V::Float(st.bullet_pierce)),
        ("stopping_power", V::Float(st.stopping_power)),
        ("weight", V::Float(st.weight)),
        ("reload_time", V::Float(st.reload_time)),
        ("aim_time", V::Float(st.aim_time)),
        ("aim_zoom_factor", V::Float(st.aim_zoom_factor)),
        ("base_dispersion", V::Float(st.base_dispersion)),
        ("magazine_capacity", V::Integer(st.magazine)),
        ("min_damage_distance", V::Integer(st.min_damage_distance)),
        ("max_damage_distance", V::Integer(st.max_damage_distance)),
    ] {
        ensure!(
            get(&cfg, &format!("parameters.{key}")).is_some(),
            "no parameters.{key}"
        );
        cfg.set_path(&format!("parameters.{key}"), value)?;
    }
    cfg.set_path("dispersion.base_dispersion", V::Float(st.base_dispersion))?;
    if let Some((name, _, _)) = rifle.fire_composite {
        cfg.set_path(
            "states.fire.sounds.first_view",
            V::Indexed(vec![V::Indexed(vec![V::Integer(257), s(name)])]),
        )?;
    }
    cfg.set_path(
        "ui_desc.icon",
        V::Indexed(rifle.icon.iter().map(|v| V::Integer(*v)).collect()),
    )?;
    cfg.set_path(
        "ui_desc.props_list",
        V::Indexed(
            card_props(st)
                .into_iter()
                .map(|(id, value)| {
                    V::Named(vec![
                        field("prop_value", s(value)),
                        field("prop_id", V::Integer(id)),
                    ])
                })
                .collect(),
        ),
    )?;
    Ok(cfg)
}

pub(crate) fn ammo_config(ammo: &Ammo, donor: &V, template: &V) -> Result<V> {
    let mut cfg = template.clone();
    for (key, value) in [
        ("item_level", V::Integer(ammo.level)),
        ("match_making_level", V::Integer(ammo.level)),
        ("drop_weight", V::Integer(ammo.drop_weight)),
        (
            "clip_size",
            get(donor, "parameters.clip_size")
                .context("clip_size")?
                .clone(),
        ),
        (
            "clip_weight",
            get(donor, "parameters.clip_weight")
                .context("clip_weight")?
                .clone(),
        ),
    ] {
        ensure!(
            get(&cfg, &format!("parameters.{key}")).is_some(),
            "no parameters.{key}"
        );
        cfg.set_path(&format!("parameters.{key}"), value)?;
    }
    for path in ["ammo_indicator_name", "text_descriptions"] {
        let v = get(donor, &format!("ui_desc.{path}"))
            .with_context(|| format!("ui_desc.{path}"))?
            .clone();
        cfg.set_path(&format!("ui_desc.{path}"), v)?;
    }
    cfg.set_path(
        "ui_desc.icon",
        V::Indexed(ammo.icon.iter().map(|v| V::Integer(*v)).collect()),
    )?;
    Ok(cfg)
}

pub(crate) fn scope_config(donor: &V, template: &V) -> Result<V> {
    let mut cfg = conform(donor, template);
    // 0.31's item data types are 0.26g's + 1 (scopes 6, 0.26g's 5).
    cfg.set_path(
        "data.type",
        get(template, "data.type").context("data.type")?.clone(),
    )?;
    // A holographic sight: no magnification (0.31 names no fov factor).
    if get(donor, "data.fov_factor").is_none() {
        cfg.set_path("data.fov_factor", V::Float(1.0))?;
    }
    // The weapon stays drawn when aiming through it: no nearer near plane
    // (the PSO-1 template's 0.6 is for a sight that hides the weapon).
    if get(donor, "data.near_plane_factor").is_none() {
        cfg.set_path("data.near_plane_factor", V::Float(1.0))?;
    }
    Ok(cfg)
}

pub(crate) fn composite(layers: &[Layer], master: i32) -> V {
    V::Named(vec![field(
        "composite_sound",
        V::Named(vec![
            field("use_master_layer_index", V::Boolean(true)),
            field("master_layer_index", V::Integer(master)),
            field(
                "sound_items",
                V::Indexed(
                    layers
                        .iter()
                        .map(|l| {
                            V::Named(vec![
                                field("filename", s(l.collection)),
                                field("enabled", V::Boolean(true)),
                                field("max_distance", V::Integer(2000)),
                                field("resource_type", V::Integer(258)),
                                field("start_offset", V::Integer(l.start_offset)),
                                field("start_offset_max", V::Integer(l.start_offset)),
                                field("min_distance", V::Integer(0)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    )])
}

// ---------------------------------------------------------------------------
// Dictionary, icons
// ---------------------------------------------------------------------------

pub(crate) fn dictionary(dict: &mut V, donor_dict: &V, port: &Port) -> Result<Vec<String>> {
    let V::Indexed(items) = dict.named_mut("items_dict").context("items_dict")? else {
        bail!("items_dict is not indexed");
    };
    let V::Indexed(donor_items) = donor_dict.named("items_dict").context("donor items_dict")?
    else {
        bail!("the donor's items_dict is not indexed");
    };
    let native = |cfg: &str| -> Result<V> {
        Ok(items
            .iter()
            .find(|v| string_of(v.named("cfg_name")) == Some(cfg))
            .with_context(|| format!("no native row for {cfg}"))?
            .clone())
    };
    let rows: Vec<(i32, &str, V, i32)> = port
        .rifles
        .iter()
        .map(|r| {
            Ok((
                r.dict_id,
                r.config,
                native(RIFLE_TEMPLATE)?,
                r.stats.base_cost,
            ))
        })
        .chain(
            port.ammo
                .iter()
                .map(|a| Ok((a.dict_id, a.config, native(AMMO_TEMPLATE)?, a.base_cost))),
        )
        .chain(
            port.scopes
                .iter()
                .map(|sc| Ok((sc.dict_id, sc.config, native(SCOPE_TEMPLATE)?, 0))),
        )
        .collect::<Result<_>>()?;
    for (id, cfg, mut row, cost) in rows {
        let theirs = donor_items
            .iter()
            .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
            .with_context(|| format!("the donor has no item {id}"))?;
        ensure!(
            string_of(theirs.named("cfg_name")) == Some(cfg),
            "donor item {id} is not {cfg}"
        );
        let name = string_of(theirs.named("name"))
            .context("donor row without a name")?
            .to_owned();
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(id))
                    || string_of(v.named("cfg_name")) == Some(cfg)
                    || string_of(v.named("name")) == Some(name.as_str())),
            "item {id} ({name}) is taken"
        );
        ensure!(
            theirs.named("item_category") == row.named("item_category"),
            "{name}: another category than its template's"
        );
        row.set_path("cfg_name", s(cfg))?;
        row.set_path("name", s(&name))?;
        row.set_path("dict_id", V::Integer(id))?;
        row.set_path("base_cost", V::Integer(cost))?;
        items.push(row);
    }
    let V::Named(compat) = dict
        .named_mut("items_compatibility")
        .context("items_compatibility")?
    else {
        bail!("items_compatibility is not a table");
    };
    let mut next = compat
        .iter()
        .filter_map(|c| c.name.strip_prefix("compatibility_")?.parse::<u32>().ok())
        .max()
        .context("no compatibility rows")?
        + 1;
    let mut added = vec![];
    for rifle in port.rifles {
        for ammo in rifle.ammo {
            let name = format!("compatibility_{next}");
            next += 1;
            compat.push(field(
                &name,
                V::Named(vec![
                    field("second_item", V::Integer(*ammo)),
                    field("first_item", V::Integer(rifle.dict_id)),
                ]),
            ));
            added.push(name);
        }
    }
    Ok(added)
}

/// A donor icon PNG centred in a 171x83 lobby cell (RGBA PNG).
pub(crate) fn cell_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palette and grey icons as RGB(A).
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    ensure!(info.bit_depth == png::BitDepth::Eight, "icon is not 8-bit");
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => bail!("icon colour type {other:?} after expansion"),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    ensure!(w <= 171 && h <= 83, "icon is {w}x{h}, larger than a cell");
    let (x0, y0) = ((171 - w) / 2, (83 - h) / 2);
    let mut cell = vec![0u8; 171 * 83 * 4];
    for y in 0..h {
        for x in 0..w {
            let p = &buf[(y * w + x) * channels..(y * w + x + 1) * channels];
            let at = ((y0 + y) * 171 + x0 + x) * 4;
            cell[at..at + 3].copy_from_slice(&p[..3]);
            cell[at + 3] = if channels == 4 { p[3] } else { 255 };
        }
    }
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, 171, 83);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&cell)?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Builds `port`'s bundle (`overlay/`, `port-manifest.json`,
/// `equipment.json`, and `icons/` with `icons.json` for the pack's
/// `atlas_icons_add`) over `target_layers` (stock and the players' patches)
/// from `donor_db`, an archive of the 0.31a2 files with its dictionary,
/// localizations and icons.
pub fn build(port: &Port, donor_db: &Path, target_layers: &[PathBuf], output: &Path) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut donor = ResourceDb::open(donor_db)?;
    let mut carry = Carry {
        donor: &mut donor,
        target: &mut target,
        overlay: &overlay,
        files: vec![],
        written: BTreeSet::new(),
        reused: BTreeSet::new(),
    };
    let mut configs = vec![];
    for rifle in port.rifles {
        ensure!(
            !carry.target.contains(rifle.config),
            "{} exists",
            rifle.config
        );
        let donor_cfg = carry.donor_config(rifle.config)?.root;
        let template = parse_config(carry.target, RIFLE_TEMPLATE, "native rifle")?.root;
        let cfg = rifle_config(rifle, &donor_cfg, &template)?;
        let model = string_of(get(&cfg, "object.model"))
            .context("object.model")?
            .to_owned();
        carry.folder(&format!("models/{model}.skinned_model"))?;
        let skeleton = string_of(get(&cfg, "object.skeleton"))
            .context("object.skeleton")?
            .to_owned();
        carry.file(&format!("animations/skeletons/{skeleton}.skeleton"))?;
        if let Some((name, layers, master)) = rifle.fire_composite {
            for l in layers {
                carry.sound(258, l.collection)?;
            }
            let path = format!("sounds/composite/{name}.composite_sound");
            ensure!(!carry.target.contains(&path), "{path} exists");
            carry.config(&path, composite(layers, master))?;
        }
        carry.states(&cfg)?;
        if let Some(V::Integer(scope)) = get(&cfg, "addons.rifle_scope_dict_id") {
            let shipped = port.scopes.iter().any(|sc| sc.dict_id == *scope)
                || PORTS
                    .iter()
                    .flat_map(|p| p.scopes)
                    .any(|sc| sc.dict_id == *scope && carry.target.contains(sc.config));
            ensure!(
                shipped,
                "{}: scope {scope} is neither in the port nor in the target",
                rifle.key
            );
        }
        configs.push((rifle.config.to_owned(), cfg));
    }
    for ammo in port.ammo {
        let donor_cfg = carry.donor_config(ammo.config)?.root;
        let template = parse_config(carry.target, AMMO_TEMPLATE, "native ammo")?.root;
        configs.push((
            ammo.config.to_owned(),
            ammo_config(ammo, &donor_cfg, &template)?,
        ));
    }
    for scope in port.scopes {
        let donor_cfg = carry.donor_config(scope.config)?.root;
        let template = parse_config(carry.target, SCOPE_TEMPLATE, "native scope")?.root;
        let cfg = scope_config(&donor_cfg, &template)?;
        for key in ["data.idle_model", "data.aimed_model"] {
            let m = string_of(get(&cfg, key)).context(key)?.to_owned();
            carry.folder(&format!("models/{m}.model"))?;
        }
        configs.push((scope.config.to_owned(), cfg));
    }
    let mut texts = BTreeSet::new();
    let scope_configs: Vec<&str> = port.scopes.iter().map(|sc| sc.config).collect();
    for (path, cfg) in &configs {
        if scope_configs.contains(&path.as_str()) {
            // Scopes are no inventory items; 0.26g's own name no texts
            // (st_leupold_*), and the EOTech's are those.
            ensure!(!carry.target.contains(path), "{path} exists");
            carry.config(path, cfg.clone())?;
            continue;
        }
        for key in [
            "ui_desc.text_descriptions.name",
            "ui_desc.text_descriptions.description",
            "ui_desc.ammo_indicator_name",
        ] {
            if let Some(t) = string_of(get(cfg, key)) {
                texts.insert(t.to_owned());
            }
        }
        ensure!(!carry.target.contains(path), "{path} exists");
        carry.config(path, cfg.clone())?;
    }
    let donor_dict = carry.donor_config(DICT)?.root;
    let mut dict = parse_config(carry.target, DICT, "0.26g dictionary")?;
    let compat = dictionary(&mut dict.root, &donor_dict, port)?;
    carry.config(DICT, dict.root)?;
    // Texts: the donor's own in every language; a key 0.26g has keeps its text.
    let mut localizations = vec![];
    let mut carried_texts = BTreeMap::new();
    for lang in LANGUAGES {
        let path = format!("localization/{lang}/localization");
        let mut loc = parse_config(carry.target, &path, "localization")?;
        let theirs = carry.donor_config(&path)?.root;
        let own: Vec<(&str, Text)> = port
            .rifles
            .iter()
            .flat_map(|r| r.texts.iter())
            .chain(port.ammo.iter().flat_map(|a| a.texts.iter()))
            .copied()
            .collect();
        for key in &texts {
            if crate::localization_026::string_of(&loc.root, key)?.is_some() {
                ensure!(!own.iter().any(|(k, _)| k == key), "{key} is 0.26g's own");
                continue;
            }
            let text = match own.iter().find(|(k, _)| k == key) {
                Some((_, text)) => text.get(lang)?.to_owned(),
                None => crate::localization_026::string_of(&theirs, key)?
                    .with_context(|| format!("the donor's {lang} localization has no {key}"))?,
            };
            set_string(&mut loc.root, key, &text)?;
            carried_texts.insert(format!("{lang}:{key}"), text);
        }
        localizations.push((lang, loc.root));
    }
    let views: Vec<(&str, &V)> = localizations.iter().map(|(l, r)| (*l, r)).collect();
    for (path, cfg) in &configs {
        if !scope_configs.contains(&path.as_str()) {
            check_item_texts(cfg, &views)?;
        }
    }
    for (lang, root) in localizations {
        carry.config(&format!("localization/{lang}/localization"), root)?;
    }
    let icons_dir = output.join("icons");
    fs::create_dir_all(&icons_dir)?;
    let mut icons = vec![];
    let cells = port
        .rifles
        .iter()
        .map(|r| ("LobbyMenu_WeaponIcons", r.icon, r.icon_png))
        .chain(
            port.ammo
                .iter()
                .map(|a| ("LobbyMenu_AmmoIcons", a.icon, a.icon_png)),
        );
    for (symbol, cell, png) in cells {
        let name = format!("{symbol}-r{:02}c{:02}.png", cell[0], cell[1]);
        fs::write(icons_dir.join(&name), cell_png(&carry.donor.read(png)?)?)?;
        icons.push(json!({"symbol": symbol, "row": cell[0], "col": cell[1], "source": format!("icons/{name}"), "donor": png}));
    }
    fs::write(
        icons_dir.join("icons.json"),
        serde_json::to_vec_pretty(&icons)?,
    )?;
    let Carry {
        mut files, reused, ..
    } = carry;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": format!("weapons-{}", port.key),
            "files": files,
            "donor_db": donor_db,
            "target": target_layers,
            "items": configs.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            "compatibility": compat,
            "icons": icons,
            "reused_from_target": reused,
            "texts": carried_texts,
        }))?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(
            &port
                .rifles
                .iter()
                .map(|r| json!({"dict_id": r.dict_id}))
                .collect::<Vec<_>>(),
        )?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {}: {} files, {} reused from the target",
        port.key,
        files.len(),
        reused.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests;
