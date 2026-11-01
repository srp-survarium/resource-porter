// SPDX-License-Identifier: GPL-3.0-or-later
//! A port's bundle: every file its guns name, in 0.26g's layouts, over the
//! target (the stock database and the patches players have).
//!
//! A file the target has is not shipped (it is reused); a file only the
//! donor has is carried, by kind:
//!
//! * clips, meshes, textures, sounds: as the donor serves them (its layout
//!   conversions, [`super::donor`]);
//! * materials: on the native material of the same base material, the
//!   effect keys 0.26g's shaders do not take left out ([`conform_material`]);
//! * sound options (`raw_options`): on a native one's keys (0.31 adds
//!   `pitch`);
//! * firing sounds (`composite_sound`): the later builds' reverb layer,
//!   `weapons/environments/weapons_<view>_view*` (a composite of per-location
//!   reverbs, which 0.26g's maps have no zones for), becomes the outdoor
//!   collection of the same kind, as patch 19's ([`environment_layer`]);
//! * any other config: as it is, and what it names is carried in turn.
//!
//! The item configs are [`super::config::weapon_config`]'s, on templates
//! read from the stock database (the patches' configs carry edit lists'
//! keys, which the edit lists add again).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::json;

use super::config::weapon_config;
use super::donor::{Donor, is_config};
use super::icons::{cell_png, kill_feed_name, kill_feed_png};
use super::manifest::{Weapon, WeaponPort};
use super::{field, get, s, string_of};
use crate::manifest::BuiltFile;
use crate::weapon::{parse_config, prepare_empty_output, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};

const DICT: &str = "gameplay/db_static_dictionaries";

/// A native material of each base material the donors use.
const MATERIAL_TEMPLATES: [(&str, &str); 2] = [
    ("complex", "material_instances/weapons/ak_12.material"),
    (
        "forward_default",
        "material_instances/weapons/1p_69_giperon_grid.material",
    ),
];
const RAW_OPTIONS_TEMPLATE: &str =
    "sounds/single/weapons/ak_12/layers/ak_12_shutter_01.raw_options";

/// The later builds' reverb composites and the outdoor collection each
/// becomes (patch 19's choice for its 0.29a3 weapons; the four marked
/// "by kind" no patch-19 weapon names, and take the collection of their
/// kind).
const ENVIRONMENT_LAYERS: [(&str, &str); 9] = [
    ("weapons_1st_view", "rudnya_outdoor"),
    ("weapons_3rd_view", "rudnya_outdoor"),
    ("weapons_1st_view_smg", "rudnya_outdoor_smg"),
    ("weapons_3rd_view_smg", "rudnya_outdoor_smg"),
    ("weapons_1st_view_suppressed", "rudnya_outdoor_suppressed"),
    ("weapons_1st_view_one_shot_auto", "rudnya_outdoor_one_shot"),
    (
        "weapons_3rd_view_one_shot",
        "rudnya_outdoor_one_shot_distant",
    ),
    // by kind
    ("weapons_1st_view_one_shot", "rudnya_outdoor_one_shot"),
    (
        "weapons_3rd_view_one_shot_auto",
        "rudnya_outdoor_one_shot_distant",
    ),
];

/// The outdoor collection a reverb composite becomes.
pub fn environment_layer(composite: &str) -> Option<String> {
    let name = composite.strip_prefix("weapons/environments/")?;
    ENVIRONMENT_LAYERS
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| format!("weapons/environments/level_01/automatic/outdoor/{to}"))
}

/// What a build made, besides the bundle's files.
#[derive(Debug, Default)]
pub struct BuildReport {
    pub files: usize,
    pub reused: usize,
    /// Per item config, what the build left out or kept native.
    pub notes: BTreeMap<String, Vec<String>>,
}

/// Carries files from a donor into a bundle's overlay (the rules above);
/// what [`build`] and the reload port ([`super::tactical_069d`]) share.
pub(crate) struct Carry<'a, D: Donor> {
    donor: &'a mut D,
    target: &'a mut ResourceDb,
    overlay: &'a Path,
    files: Vec<BuiltFile>,
    written: BTreeSet<String>,
    reused: BTreeSet<String>,
    source: String,
    file_values: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    warnings: Vec<String>,
    material_templates: BTreeMap<String, V>,
}

impl<'a, D: Donor> Carry<'a, D> {
    pub(crate) fn new(
        donor: &'a mut D,
        target: &'a mut ResourceDb,
        overlay: &'a Path,
        source: String,
    ) -> Self {
        Self {
            donor,
            target,
            overlay,
            files: vec![],
            written: BTreeSet::new(),
            reused: BTreeSet::new(),
            source,
            file_values: BTreeMap::new(),
            warnings: vec![],
            material_templates: BTreeMap::new(),
        }
    }

    /// The files written, the target's files reused, the warnings.
    pub(crate) fn finish(self) -> (Vec<BuiltFile>, BTreeSet<String>, Vec<String>) {
        let mut files = self.files;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files, self.reused, self.warnings)
    }

    pub(crate) fn donor(&mut self) -> &mut D {
        self.donor
    }

    pub(crate) fn target(&mut self) -> &mut ResourceDb {
        self.target
    }

    /// A native material of base material `base`: the listed one, or the
    /// first native weapon material of that base.
    fn material_template(&mut self, base: &str) -> Result<V> {
        if let Some(t) = self.material_templates.get(base) {
            return Ok(t.clone());
        }
        let listed = MATERIAL_TEMPLATES
            .iter()
            .find(|(b, _)| *b == base)
            .map(|(_, p)| vec![(*p).to_owned()]);
        let candidates = match listed {
            Some(l) => l,
            None => self
                .target
                .paths_with_prefix("material_instances/weapons/")
                .filter(|p| p.ends_with(".material"))
                .map(str::to_owned)
                .collect(),
        };
        for c in candidates {
            let t = parse_config(self.target, &c, "native material")?.root;
            if string_of(get(&t, "material.instance.base_material")) == Some(base) {
                self.material_templates.insert(base.into(), t.clone());
                return Ok(t);
            }
        }
        bail!("the target has no weapon material of base {base}")
    }

    pub(crate) fn raw(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        let dest = crate::archive::safe_join(self.overlay, path)?;
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
        self.files.push(record_file(path, &self.source, bytes));
        Ok(())
    }

    pub(crate) fn config(&mut self, path: &str, root: V) -> Result<()> {
        ensure!(self.written.insert(path.into()), "{path} written twice");
        write_generated_config(
            self.overlay,
            path,
            &BinaryConfig { root },
            &self.source,
            &mut self.files,
        )
    }

    /// Ships `path` from the donor, in 0.26g's layout, unless the target has
    /// it, and everything it names.
    pub(crate) fn file(&mut self, path: &str) -> Result<()> {
        if self.written.contains(path) || self.reused.contains(path) {
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
        if !is_config(path) {
            let bytes = self.donor.read(path)?;
            return self.raw(path, &bytes);
        }
        let bytes = self.donor.read(path)?;
        let root = BinaryConfig::parse(&bytes)
            .with_context(|| format!("parsing donor {path}"))?
            .root;
        let mut root = if path.ends_with(".material") {
            let base = string_of(get(&root, "material.instance.base_material"))
                .context("material without a base")?
                .to_owned();
            let template = self
                .material_template(&base)
                .with_context(|| path.to_owned())?;
            conform_material(&root, &template)?
        } else if path.ends_with(".raw_options") {
            let template = parse_config(self.target, RAW_OPTIONS_TEMPLATE, "native raw_options")?;
            known_keys_only(&root, &template.root)
        } else if path.ends_with(".composite_sound") {
            environment_layers(root)?
        } else {
            root
        };
        if let Some(values) = self.file_values.get(path) {
            for (key, value) in values {
                super::config::set_typed(&mut root, key, value)
                    .with_context(|| format!("{path}: {key}"))?;
            }
        }
        self.follow(path, &root)?;
        if BinaryConfig::parse(&bytes)?.root == root {
            return self.raw(path, &bytes);
        }
        self.config(path, root)
    }

    /// Every file under `prefix` (a model folder), unless the target has
    /// one of them.
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
        let paths = self.donor.paths_with_prefix(&format!("{prefix}/"));
        ensure!(!paths.is_empty(), "the donor has no {prefix}");
        for p in paths {
            self.file(&p)?;
        }
        Ok(())
    }

    pub(crate) fn clip(&mut self, name: &str) -> Result<()> {
        let name = name.trim_start_matches("resources/");
        self.file(&format!("{name}.b-spline"))?;
        self.file(&format!("{name}.bones_names"))
    }

    pub(crate) fn sound(&mut self, kind: i32, name: &str) -> Result<()> {
        match kind {
            256 => {
                for ext in ["high.ogg", "high.rms", "raw_options"] {
                    let path = format!("sounds/single/{name}.{ext}");
                    // A sound without options plays with the defaults.
                    if ext == "raw_options"
                        && !self.target.contains(&path)
                        && !self.donor.contains(&path)
                    {
                        continue;
                    }
                    self.file(&path)?;
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
                    let options = format!("textures/{t}.options");
                    if self.target.contains(&options) || self.donor.contains(&options) {
                        self.file(&options)?;
                    }
                } else if !t.starts_with("editor/") {
                    // A dangling name draws nothing; the donor shipped it so.
                    self.warnings
                        .push(format!("{path} names {t}, which neither archive has"));
                }
            }
        } else if path.ends_with("/settings") {
            if let Some(V::Named(materials)) = root.named("material_settings") {
                let names: Vec<String> = materials
                    .iter()
                    .filter_map(|m| string_of(m.value.named("material_name")).map(str::to_owned))
                    .filter(|n| !n.is_empty())
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
    /// The clips and sounds `cfg` names. A clip the template names that no
    /// archive has (the stock pistols' `on_site/death_fwd`) stays named, as
    /// in the stock config, and is noted.
    fn states(&mut self, cfg: &V, template: &V, notes: &mut Vec<String>) -> Result<()> {
        let mut strings = vec![];
        cfg.collect_strings(&mut strings);
        let mut theirs = vec![];
        template.collect_strings(&mut theirs);
        let clips: Vec<String> = strings
            .into_iter()
            .filter(|c| c.starts_with("resources/animations/"))
            .map(str::to_owned)
            .collect();
        for c in clips {
            let path = format!("{}.b-spline", c.trim_start_matches("resources/"));
            if theirs.contains(&c.as_str())
                && !self.target.contains(&path)
                && !self.donor.contains(&path)
            {
                notes.push(format!("{c}: the template's, which no archive has"));
                continue;
            }
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

/// A firing composite's reverb layers as outdoor collections.
fn environment_layers(mut root: V) -> Result<V> {
    let Some(V::Indexed(items)) = root
        .named_mut("composite_sound")
        .and_then(|c| c.named_mut("sound_items"))
    else {
        return Ok(root);
    };
    for item in items.iter_mut() {
        let Some(name) = string_of(item.named("filename")).map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("weapons/environments/weapons_") {
            continue;
        }
        let to = environment_layer(&name)
            .with_context(|| format!("no outdoor collection for the reverb layer {name}"))?;
        item.set_path("filename", s(to))?;
        item.set_path("resource_type", V::Integer(258))?;
    }
    Ok(root)
}

/// `donor` without the keys `template`'s tables lack (0.31's `pitch`,
/// 0.29's `add_rms`: 0.26g's exe reads neither), the rest as it is.
pub fn known_keys_only(donor: &V, template: &V) -> V {
    match (donor, template) {
        (V::Named(d), V::Named(t)) => V::Named(
            d.iter()
                .filter_map(|f| {
                    let tf = t.iter().find(|x| x.name == f.name)?;
                    Some(field(&f.name, known_keys_only(&f.value, &tf.value)))
                })
                .collect(),
        ),
        (d, _) => d.clone(),
    }
}

/// A later build's material on 0.26g's effect keys (the extra ones left
/// out), for each stage it has; the native template has the same base
/// material and stages.
pub fn conform_material(donor: &V, template: &V) -> Result<V> {
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

/// The port's rows in `items_dict` (each its template's row with the gun's
/// config, name, id and price) and one `items_compatibility` row a gun and
/// round, `compatibility_<gun>_<round>` (srp and the game take any name).
fn dictionary(dict: &mut V, port: &WeaponPort) -> Result<Vec<String>> {
    let V::Indexed(items) = dict.named_mut("items_dict").context("items_dict")? else {
        bail!("items_dict is not indexed");
    };
    for w in &port.weapons {
        let mut row = items
            .iter()
            .find(|v| string_of(v.named("cfg_name")) == Some(w.template.as_str()))
            .with_context(|| format!("no dictionary row for the template {}", w.template))?
            .clone();
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(w.dict_id))
                    || string_of(v.named("cfg_name")) == Some(w.config.as_str())
                    || string_of(v.named("name")) == Some(w.name.as_str())),
            "item {} ({}) is taken",
            w.dict_id,
            w.name
        );
        row.set_path("cfg_name", s(&w.config))?;
        row.set_path("name", s(&w.name))?;
        row.set_path("dict_id", V::Integer(w.dict_id))?;
        row.set_path("base_cost", V::Integer(w.base_cost))?;
        if row.named("is_premium").is_some() {
            row.set_path("is_premium", V::Boolean(w.premium))?;
        }
        items.push(row);
    }
    for a in &port.ammo_items {
        let mut row = items
            .iter()
            .find(|v| string_of(v.named("cfg_name")) == Some(a.template.as_str()))
            .with_context(|| format!("no dictionary row for the template {}", a.template))?
            .clone();
        ensure!(
            !items
                .iter()
                .any(|v| v.named("dict_id") == Some(&V::Integer(a.dict_id))
                    || string_of(v.named("cfg_name")) == Some(a.config.as_str())
                    || string_of(v.named("name")) == Some(a.name.as_str())),
            "item {} ({}) is taken",
            a.dict_id,
            a.name
        );
        row.set_path("cfg_name", s(&a.config))?;
        row.set_path("name", s(&a.name))?;
        row.set_path("dict_id", V::Integer(a.dict_id))?;
        row.set_path("base_cost", V::Integer(a.base_cost))?;
        if let Some(category) = a.category {
            row.set_path("item_category", V::Integer(category))?;
        }
        items.push(row);
    }
    let ids: BTreeSet<i32> = items
        .iter()
        .filter_map(|v| match v.named("dict_id") {
            Some(V::Integer(i)) => Some(*i),
            _ => None,
        })
        .collect();
    let V::Named(compat) = dict
        .named_mut("items_compatibility")
        .context("items_compatibility")?
    else {
        bail!("items_compatibility is not a table");
    };
    // Each row named by its pair, not the next free number: packs made
    // apart (each weapon batch its own) never take the same name.
    let mut added = vec![];
    for w in &port.weapons {
        for ammo in &w.ammo {
            ensure!(
                ids.contains(ammo) || port.external_ammo.contains(ammo),
                "{}: no ammunition item {ammo}",
                w.name
            );
            let name = format!("compatibility_{}_{ammo}", w.dict_id);
            ensure!(!compat.iter().any(|c| c.name == name), "{name} is taken");
            compat.push(field(
                &name,
                V::Named(vec![
                    field("second_item", V::Integer(*ammo)),
                    field("first_item", V::Integer(w.dict_id)),
                ]),
            ));
            added.push(name);
        }
    }
    Ok(added)
}

/// The text keys an item config names.
fn text_keys(cfg: &V) -> Vec<String> {
    [
        "ui_desc.text_descriptions.name",
        "ui_desc.text_descriptions.description",
    ]
    .iter()
    .filter_map(|k| string_of(get(cfg, k)).map(str::to_owned))
    .collect()
}

/// The languages a donor may have dictionaries for.
const DONOR_LANGUAGES: [&str; 6] = [
    "english",
    "russian",
    "ukrainian",
    "german",
    "polish",
    "original",
];

/// Builds `port`'s bundle at `output` from `donor` over `target_layers`
/// (the stock database first, then the patches players have):
/// `overlay/`, `port-manifest.json`, `icons/icons.json` and the lobby
/// cells, `kill-feed/` (the hud's images), `texts.tsv` (the keys the
/// configs name and the donor's texts: skhid-srp's localization tables
/// carry the texts).
pub fn build<D: Donor>(
    port: &WeaponPort,
    donor: &mut D,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<BuildReport> {
    port.check()?;
    ensure!(
        donor.build() == port.donor_build,
        "the donor is {:?}, the port's {:?}",
        donor.build(),
        port.donor_build
    );
    ensure!(
        port.modules.is_empty(),
        "modules are ported by `port-modules` (modules.rs)"
    );
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut stock = ResourceDb::open(&target_layers[0])?;
    let file_values = port
        .weapons
        .iter()
        .flat_map(|w| w.file_values.clone())
        .collect();
    let mut carry = Carry {
        donor,
        target: &mut target,
        overlay: &overlay,
        files: vec![],
        written: BTreeSet::new(),
        reused: BTreeSet::new(),
        source: format!(
            "converted:{}",
            serde_json::to_value(port.donor_build)?
                .as_str()
                .unwrap_or("?")
        ),
        file_values,
        warnings: vec![],
        material_templates: BTreeMap::new(),
    };
    let mut report = BuildReport::default();
    let natives: Vec<V> = stock
        .paths_with_prefix("gameplay/items/weapons/wpn_")
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
        .map(|p| Ok(parse_config(&mut stock, &p, "native weapon")?.root))
        .collect::<Result<_>>()?;
    let schema = super::config::schema(&natives);
    let mut native_refs = super::resources::native_references(&natives);
    let mut configs: Vec<(&Weapon, V)> = vec![];
    for w in &port.weapons {
        ensure!(!carry.target.contains(&w.config), "{} exists", w.config);
        let donor_cfg = carry.donor.item_config(&w.donor_item)?;
        let template = parse_config(&mut stock, &w.template, "native template")?.root;
        let target: &ResourceDb = carry.target;
        let foreign = |clip: &str| -> bool {
            let path = clip.trim_start_matches("resources/");
            let set: Vec<&str> = path.splitn(5, '/').take(4).collect();
            set.len() == 4
                && target
                    .paths_with_prefix(&format!("{}/", set.join("/")))
                    .next()
                    .is_some()
                && !target.contains(&format!("{path}.b-spline"))
        };
        let (cfg, notes) = weapon_config(
            carry.donor.rules(),
            w,
            &donor_cfg,
            &template,
            &schema,
            &foreign,
        )
        .with_context(|| w.name.clone())?;
        let mut notes = notes;
        let model = string_of(get(&cfg, "object.model"))
            .context("object.model")?
            .to_owned();
        carry.folder(&format!("models/{model}.skinned_model"))?;
        let skeleton = string_of(get(&cfg, "object.skeleton"))
            .context("object.skeleton")?
            .to_owned();
        carry.file(&format!("animations/skeletons/{skeleton}.skeleton"))?;
        carry
            .states(&cfg, &template, &mut notes)
            .with_context(|| w.name.clone())?;
        report.notes.insert(w.config.clone(), notes);
        configs.push((w, cfg));
    }
    let mut ammo_configs: Vec<(String, V)> = vec![];
    for a in &port.ammo_items {
        ensure!(!carry.target.contains(&a.config), "{} exists", a.config);
        let mut cfg = parse_config(carry.target, &a.template, "native round")?.root;
        native_refs.extend(super::resources::native_references([&cfg]));
        for (path, value) in &a.set {
            super::config::set_typed(&mut cfg, path, value).with_context(|| a.name.clone())?;
        }
        if let Some(icon) = &a.lobby_icon {
            cfg.set_path(
                "ui_desc.icon",
                V::Indexed(vec![
                    V::Integer(icon.row as i32),
                    V::Integer(icon.col as i32),
                ]),
            )?;
        }
        ammo_configs.push((a.config.clone(), cfg));
    }
    // Every resource a config names exists, in the target or the bundle.
    for (w, cfg) in &configs {
        let (target, written) = (&*carry.target, &carry.written);
        let exists = |file: &str| target.contains(file) || written.contains(file);
        super::resources::check_resources(&w.config, cfg, &exists, &native_refs)?;
    }
    let mut texts = BTreeSet::new();
    for (w, cfg) in &configs {
        texts.extend(text_keys(cfg));
        carry.config(&w.config, cfg.clone())?;
    }
    for (path, cfg) in &ammo_configs {
        let (target, written) = (&*carry.target, &carry.written);
        let exists = |file: &str| target.contains(file) || written.contains(file);
        super::resources::check_resources(path, cfg, &exists, &native_refs)?;
        texts.extend(text_keys(cfg));
        if let Some(key) = string_of(get(cfg, "ui_desc.ammo_indicator_name")) {
            texts.insert(key.to_owned());
        }
        carry.config(path, cfg.clone())?;
    }
    let mut dict = parse_config(carry.target, DICT, "0.26g dictionary")?;
    let compat = dictionary(&mut dict.root, port)?;
    carry.config(DICT, dict.root)?;

    // Texts: the keys, 0.26g's text where it has one, the donor's.
    let mut tsv = String::from("key\tlanguage\tlive\tdonor\n");
    for lang in DONOR_LANGUAGES {
        let path = format!("localization/{lang}/localization");
        let live = parse_config(carry.target, &path, "localization")
            .ok()
            .map(|c| c.root);
        let theirs = if carry.donor.contains(&path) {
            Some(carry.donor.config(&path)?)
        } else {
            None
        };
        for key in &texts {
            let find = |root: &Option<V>| -> Result<String> {
                Ok(match root {
                    Some(r) => crate::localization_026::string_of(r, key)?.unwrap_or_default(),
                    None => String::new(),
                })
            };
            let (l, d) = (find(&live)?, find(&theirs)?);
            tsv.push_str(&format!(
                "{key}\t{lang}\t{}\t{}\n",
                l.replace(['\t', '\n'], " "),
                d.replace(['\t', '\n'], " ")
            ));
        }
    }
    fs::write(output.join("texts.tsv"), tsv)?;

    // Icons.
    let icons_dir = output.join("icons");
    fs::create_dir_all(&icons_dir)?;
    let mut icons = vec![];
    for w in &port.weapons {
        if let Some(icon) = &w.lobby_icon {
            let symbol = "LobbyMenu_WeaponIcons";
            let name = format!("{symbol}-r{:02}c{:02}.png", icon.row, icon.col);
            fs::write(
                icons_dir.join(&name),
                cell_png(&carry.donor.read(&icon.donor_png)?)
                    .with_context(|| icon.donor_png.clone())?,
            )?;
            icons.push(json!({"symbol": symbol, "row": icon.row, "col": icon.col, "source": format!("icons/{name}"), "donor": icon.donor_png}));
        }
        if let Some(png) = &w.kill_feed.donor_png {
            let dest = output.join(kill_feed_name(w.kill_feed.icon));
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::write(
                dest,
                kill_feed_png(&carry.donor.read(png)?).with_context(|| png.clone())?,
            )?;
        }
    }
    for a in &port.ammo_items {
        if let Some(icon) = &a.lobby_icon {
            let symbol = "LobbyMenu_AmmoIcons";
            let name = format!("{symbol}-r{:02}c{:02}.png", icon.row, icon.col);
            let image = match &icon.file {
                Some(file) => {
                    fs::read(port.dir.join(file)).with_context(|| format!("reading {file}"))?
                }
                None => carry.donor.read(&icon.donor_png)?,
            };
            fs::write(
                icons_dir.join(&name),
                cell_png(&image).with_context(|| icon.donor_png.clone())?,
            )?;
            icons.push(json!({"symbol": symbol, "row": icon.row, "col": icon.col, "source": format!("icons/{name}"), "donor": icon.donor_png}));
        }
    }
    fs::write(
        icons_dir.join("icons.json"),
        serde_json::to_vec_pretty(&icons)?,
    )?;
    let Carry {
        mut files,
        reused,
        warnings,
        ..
    } = carry;
    report.notes.insert("files".into(), warnings);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": port.id,
            "donor_build": port.donor_build,
            "files": files,
            "target": target_layers,
            "items": configs.iter().map(|(w, _)| w.config.clone()).chain(ammo_configs.iter().map(|(p, _)| p.clone())).collect::<Vec<_>>(),
            "compatibility": compat,
            "icons": icons,
            "reused_from_target": reused,
            "notes": report.notes,
        }))?,
    )?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    report.files = files.len();
    report.reused = reused.len();
    Ok(report)
}
