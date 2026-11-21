// SPDX-License-Identifier: GPL-3.0-or-later
//! A donor's models, materials and textures against the files players have.
//!
//! The two builds share most of their files by name. A donor file players do
//! not have is added at its own path. One they have in the same form is
//! theirs: the same bytes, or for a texture the same image (`image`), for a
//! render surface the same vertices and triangles, for a material or a
//! model's settings the same values once names are resolved. One they have in
//! another form is imported under the map's namespace, and whatever names it
//! (a material its texture, a model its material, the project its model)
//! names the import.
//!
//! A material keeps the stage and effect fields 0.26g's own materials have
//! for its effect. Its switches select shader programs: a set of switches no
//! material players have uses is not known to have its programs, and the
//! game waits forever for a missing one. Such a material takes the largest
//! set players' materials use that adds no switch (`Degraded`), unless
//! programs for its own set are supplied.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use super::image;
use crate::{
    ResourceDb,
    binary_config::{BinaryConfig, ConfigValue, NamedValue},
};

/// A stage's switches: `use_*` fields that are on, and `<name>=<n>` for its
/// techniques, modes and wind motion.
type Switches = BTreeSet<String>;

/// What 0.26g's materials are made of, and which switch sets players'
/// materials use.
#[derive(Default)]
pub struct Effects {
    /// Stage-level fields beside `effect`, by stage.
    stage_fields: BTreeMap<String, BTreeSet<String>>,
    /// Effect fields by stage and effect id.
    effect_fields: BTreeMap<(String, String), BTreeSet<String>>,
    /// The switch sets in use by stage, effect id and what the stage is
    /// compiled for (`flags`).
    known: BTreeMap<(String, String, String), BTreeSet<Switches>>,
    /// Effect fields skhid's client patches read, by stage and effect id
    /// (`learn_client_fields`). Their switches select programs the content
    /// that ships with the patch supplies, so they are kept and the rest of
    /// a material's set is judged without them.
    client_fields: BTreeSet<(String, String, String)>,
    /// Stages and effects whose programs a required pack supplies in full
    /// from the donor (forward-effects: every donor program of their
    /// families players lack, for every runtime choice): a material keeps
    /// its donor switch set there.
    supplied: BTreeSet<(String, String)>,
}

fn value_of(field: &ConfigValue) -> Option<&ConfigValue> {
    field.named("value")
}

fn stages(root: &ConfigValue) -> impl Iterator<Item = &NamedValue> {
    let fields: &[NamedValue] = match root.named("material") {
        Some(ConfigValue::Named(fields)) => fields,
        _ => &[],
    };
    fields.iter().filter(|f| f.value.named("effect").is_some())
}

fn effect_id(stage: &ConfigValue) -> String {
    match stage.named("effect").and_then(|e| e.named("effect_id")) {
        Some(ConfigValue::String(id)) => id.clone(),
        _ => String::new(),
    }
}

/// A field whose number selects programs: a technique, a mode, the wind's
/// motion (a vertex shader for each).
fn is_selector(name: &str) -> bool {
    name.ends_with("_technique") || name.ends_with("_mode") || name == "wind_motion"
}

fn switches(stage: &ConfigValue) -> Switches {
    let mut on = Switches::new();
    if let Some(ConfigValue::Named(effect)) = stage.named("effect") {
        for field in effect {
            match value_of(&field.value) {
                Some(ConfigValue::Boolean(true)) if field.name.starts_with("use_") => {
                    on.insert(field.name.clone());
                }
                Some(ConfigValue::Integer(i)) if is_selector(&field.name) => {
                    on.insert(format!("{}={i}", field.name));
                }
                _ => {}
            }
        }
    }
    on
}

/// What a stage is compiled for: its `with_*` flags that are on (static
/// meshes, vertex-colour meshes, skeletal meshes, vegetation, particles),
/// each a set of programs of its own. A stage with none is compiled once.
fn flags(stage: &ConfigValue) -> Vec<String> {
    let ConfigValue::Named(fields) = stage else { return vec![] };
    let on: Vec<String> = fields
        .iter()
        .filter(|f| f.name.starts_with("with_") && matches!(value_of(&f.value), Some(ConfigValue::Boolean(true))))
        .map(|f| f.name.clone())
        .collect();
    if on.is_empty() { vec![String::new()] } else { on }
}

impl Effects {
    /// The fields of the stock game's materials: what its executable reads.
    pub fn learn_fields(&mut self, material: &ConfigValue) {
        for stage in stages(material) {
            let ConfigValue::Named(fields) = &stage.value else { continue };
            self.stage_fields.entry(stage.name.clone()).or_default().extend(fields.iter().filter(|f| f.name != "effect").map(|f| f.name.clone()));
            if let Some(ConfigValue::Named(effect)) = stage.value.named("effect") {
                self.effect_fields.entry((stage.name.clone(), effect_id(&stage.value))).or_default().extend(effect.iter().map(|f| f.name.clone()));
            }
        }
    }

    /// The fields skhid's client patches make 0.26g read (`stage`,
    /// `effect`, `field` a row, tab-separated, then the patch and why;
    /// `#` comments): a material keeps them.
    pub fn learn_client_fields(&mut self, table: &str) -> Result<()> {
        for line in table.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
            let mut cells = line.split('\t');
            let (Some(stage), Some(effect), Some(field)) = (cells.next(), cells.next(), cells.next()) else {
                bail!("client fields: `{line}` is not stage, effect, field");
            };
            if stage == "stage" {
                continue;
            }
            self.effect_fields.entry((stage.to_owned(), effect.to_owned())).or_default().insert(field.to_owned());
            self.client_fields.insert((stage.to_owned(), effect.to_owned(), field.to_owned()));
        }
        Ok(())
    }

    /// The stage and effect whose programs a required pack supplies in full.
    pub fn learn_supplied(&mut self, stage: &str, effect: &str) {
        self.supplied.insert((stage.to_owned(), effect.to_owned()));
    }

    fn is_client_field(&self, stage: &str, effect: &str, switch: &str) -> bool {
        self.client_fields.contains(&(stage.to_owned(), effect.to_owned(), switch.to_owned()))
    }

    /// The switch sets of a material players have: its programs are shipped.
    /// The client fields are learnt first: their switches are not part of a
    /// set.
    pub fn learn_programs(&mut self, material: &ConfigValue) {
        for stage in stages(material) {
            let id = effect_id(&stage.value);
            let on: Switches = switches(&stage.value).into_iter().filter(|s| !self.is_client_field(&stage.name, &id, s)).collect();
            for flag in flags(&stage.value) {
                self.known.entry((stage.name.clone(), effect_id(&stage.value), flag)).or_default().insert(on.clone());
            }
        }
    }

    fn has_programs(&self, stage: &str, effect: &str, flags: &[String], on: &Switches) -> bool {
        flags.iter().all(|flag| self.known.get(&(stage.to_owned(), effect.to_owned(), flag.clone())).is_some_and(|sets| sets.contains(on)))
    }

    /// The nearest switch set with programs for every flag: no switch added,
    /// the fewest dropped or changed, the first in order among equals.
    fn nearest(&self, stage: &str, effect: &str, flags: &[String], on: &Switches) -> Option<Switches> {
        let name = |s: &str| s.split('=').next().unwrap_or(s).to_owned();
        let wanted: BTreeSet<String> = on.iter().map(|s| name(s)).collect();
        let first = self.known.get(&(stage.to_owned(), effect.to_owned(), flags.first()?.clone()))?;
        first
            .iter()
            .filter(|set| set.iter().all(|s| wanted.contains(&name(s))))
            .filter(|set| self.has_programs(stage, effect, flags, set))
            .min_by_key(|set| (on.difference(set).count(), (*set).clone()))
            .cloned()
    }
}

/// A material whose switch set had no programs, and the set it took.
#[derive(Debug, serde::Serialize)]
pub struct Degraded {
    pub material: String,
    pub stage: String,
    pub dropped: Vec<String>,
    pub set: Vec<String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct KindCount {
    /// Players have it in the same form.
    pub theirs: usize,
    /// Players do not have it: added at its own path.
    pub added: usize,
    /// Players have another form: imported under the namespace.
    pub imported: usize,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct FileReport {
    pub textures: KindCount,
    /// Of the textures that are players': the same image in another encoding.
    pub textures_same_image: usize,
    pub materials: KindCount,
    pub models: KindCount,
    /// Why each imported model, material or texture is not players' own.
    pub imported: BTreeMap<String, String>,
    pub degraded: Vec<Degraded>,
    /// Material switch sets with no programs and no smaller set that has them.
    pub without_programs: Vec<String>,
    /// Materials that kept a switch set players' materials do not use
    /// (`UnknownSwitches::Keep`), with the set.
    pub kept_unknown_switches: BTreeMap<String, Vec<String>>,
    /// Donor effect and stage fields 0.26g's materials do not have, by name.
    pub material_fields_dropped: BTreeMap<String, usize>,
    /// Switches that were on among those: what 0.26g cannot draw.
    pub switches_not_in_026g: BTreeMap<String, Vec<String>>,
    /// Stages whose effect 0.26g has no material of: left out of the material.
    pub effects_not_in_026g: BTreeMap<String, Vec<String>>,
    /// Chunks of render surfaces 0.26g's surfaces do not have, by id.
    pub surface_chunks_dropped: BTreeMap<u32, usize>,
    /// Names the donor refers to and neither build has.
    pub missing: BTreeSet<String>,
}

/// What a material does when players' materials do not use its switch set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnknownSwitches {
    /// It takes the nearest set that has programs.
    Nearest,
    /// It keeps its set: the build is for finding the programs the game
    /// then asks for (it stops with an assert for each one it lacks).
    Keep,
}

pub struct Importer<'a> {
    pub donor: &'a mut ResourceDb,
    pub shipped: &'a mut ResourceDb,
    pub effects: &'a Effects,
    pub unknown_switches: UnknownSwitches,
    /// The folder imports go under (`ported/v044a`).
    pub namespace: &'a str,
    /// The files of the pack, by path.
    pub out: BTreeMap<String, Vec<u8>>,
    pub report: FileReport,
    textures: BTreeMap<String, String>,
    materials: BTreeMap<String, String>,
    models: BTreeMap<(String, &'static str), String>,
}

/// A static surface's chunks: (id, payload).
fn chunks(bytes: &[u8]) -> Option<Vec<(u32, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let id = u32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        out.push((id, bytes.get(at + 8..at + 8 + size)?));
        at += 8 + size;
    }
    (at == bytes.len()).then_some(out)
}

const VERTICES: u32 = 3;
const INDICES: u32 = 4;

/// The triangles of an index chunk (a count, then 16-bit indices), each
/// turned to start at its smallest index, sorted: later builds write the
/// same triangles in another order.
fn triangles(chunk: &[u8]) -> Option<Vec<[u16; 3]>> {
    let count = u32::from_le_bytes(chunk.get(..4)?.try_into().ok()?) as usize;
    let indices = chunk.get(4..4 + count * 2)?;
    if !count.is_multiple_of(3) || chunk.len() != 4 + count * 2 {
        return None;
    }
    let mut out: Vec<[u16; 3]> = indices
        .chunks_exact(6)
        .map(|t| {
            let t: [u16; 3] = std::array::from_fn(|i| u16::from_le_bytes([t[2 * i], t[2 * i + 1]]));
            let first = (0..3).min_by_key(|&i| t[i]).unwrap();
            [t[first], t[(first + 1) % 3], t[(first + 2) % 3]]
        })
        .collect();
    out.sort_unstable();
    Some(out)
}

/// Whether two static surfaces draw the same mesh: the same vertex chunk and
/// the same triangles.
pub fn same_surface(a: &[u8], b: &[u8]) -> bool {
    let (Some(a), Some(b)) = (chunks(a), chunks(b)) else { return false };
    let part = |c: &[(u32, &[u8])], id: u32| c.iter().find(|c| c.0 == id).map(|c| c.1.to_vec());
    let (Some(va), Some(vb), Some(ia), Some(ib)) = (part(&a, VERTICES), part(&b, VERTICES), part(&a, INDICES), part(&b, INDICES)) else {
        return false;
    };
    va == vb && (ia == ib || matches!((triangles(&ia), triangles(&ib)), (Some(x), Some(y)) if x == y))
}

/// A static surface as 0.26g's are: the vertex and index chunks alone.
fn surface_026g(bytes: &[u8], dropped: &mut BTreeMap<u32, usize>) -> Option<Vec<u8>> {
    let parts = chunks(bytes)?;
    if !parts.iter().any(|c| c.0 == VERTICES) || !parts.iter().any(|c| c.0 == INDICES) {
        return None;
    }
    let mut out = Vec::new();
    for (id, body) in parts {
        if id == VERTICES || id == INDICES {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
        } else {
            *dropped.entry(id).or_default() += 1;
        }
    }
    Some(out)
}

/// Whether two configs hold the same values, whatever the order of fields.
pub fn same_config(a: &ConfigValue, b: &ConfigValue) -> bool {
    match (a, b) {
        (ConfigValue::Named(x), ConfigValue::Named(y)) => {
            x.len() == y.len() && x.iter().all(|f| y.iter().any(|g| g.name == f.name && same_config(&f.value, &g.value)))
        }
        (ConfigValue::Indexed(x), ConfigValue::Indexed(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_config(p, q)),
        (ConfigValue::Integer(i), ConfigValue::Float(f)) | (ConfigValue::Float(f), ConfigValue::Integer(i)) => *i as f32 == *f,
        (x, y) => x == y,
    }
}

fn parse(bytes: &[u8], path: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(bytes).with_context(|| format!("parsing {path}"))?.root)
}

fn encode(root: ConfigValue, path: &str) -> Result<Vec<u8>> {
    BinaryConfig { root }.to_bytes().with_context(|| format!("writing {path}"))
}

impl<'a> Importer<'a> {
    pub fn new(donor: &'a mut ResourceDb, shipped: &'a mut ResourceDb, effects: &'a Effects, namespace: &'a str) -> Self {
        Self {
            donor,
            shipped,
            effects,
            unknown_switches: UnknownSwitches::Nearest,
            namespace,
            out: BTreeMap::new(),
            report: FileReport::default(),
            textures: BTreeMap::new(),
            materials: BTreeMap::new(),
            models: BTreeMap::new(),
        }
    }

    fn own(&self, name: &str) -> String {
        format!("{}/{name}", self.namespace)
    }

    /// The name to draw the donor's texture `name` by.
    pub fn texture(&mut self, name: &str) -> Result<String> {
        if let Some(known) = self.textures.get(name) {
            return Ok(known.clone());
        }
        let dds = format!("textures/{name}.dds");
        let options = format!("textures/{name}.options");
        let used = if !self.donor.is_file(&dds) {
            if !self.shipped.is_file(&dds) {
                self.report.missing.insert(dds);
            }
            name.to_owned()
        } else {
            let own = self.donor.read(&dds)?;
            let (to, why) = if !self.shipped.is_file(&dds) {
                self.report.textures.added += 1;
                (name.to_owned(), None)
            } else {
                let theirs = self.shipped.read(&dds)?;
                if theirs == own {
                    self.report.textures.theirs += 1;
                    self.textures.insert(name.to_owned(), name.to_owned());
                    return Ok(name.to_owned());
                }
                if image::same_image(&own, &theirs)? {
                    self.report.textures.theirs += 1;
                    self.report.textures_same_image += 1;
                    self.textures.insert(name.to_owned(), name.to_owned());
                    return Ok(name.to_owned());
                }
                self.report.textures.imported += 1;
                (self.own(name), Some("another image than players' file of the name"))
            };
            if let Some(why) = why {
                self.report.imported.insert(format!("textures/{to}.dds"), why.to_owned());
            }
            self.out.insert(format!("textures/{to}.dds"), own);
            if self.donor.is_file(&options) {
                self.out.insert(format!("textures/{to}.options"), self.donor.read(&options)?);
            }
            to
        };
        self.textures.insert(name.to_owned(), used.clone());
        Ok(used)
    }

    /// The donor's material with the fields 0.26g's materials have for its
    /// stages and effects, its textures by the names to draw them by, and a
    /// switch set that has programs.
    fn material_026g(&mut self, name: &str, root: &ConfigValue) -> Result<ConfigValue> {
        let ConfigValue::Named(top) = root else { bail!("{name}: a material is not a table") };
        let mut out_top = Vec::new();
        for field in top {
            if field.name != "material" {
                out_top.push(field.clone());
                continue;
            }
            let ConfigValue::Named(parts) = &field.value else { bail!("{name}: material is not a table") };
            let mut dropped_stages = Vec::new();
            let mut out_parts = Vec::new();
            for part in parts {
                let Some(ConfigValue::Named(effect)) = part.value.named("effect") else {
                    out_parts.push(part.clone());
                    continue;
                };
                let stage = part.name.as_str();
                let id = effect_id(&part.value);
                let Some(known) = self.effects.effect_fields.get(&(stage.to_owned(), id.clone())) else {
                    self.report.effects_not_in_026g.entry(format!("{stage}: {id}")).or_default().push(name.to_owned());
                    dropped_stages.push(stage.to_owned());
                    continue;
                };
                let mut out_effect = Vec::new();
                for f in effect {
                    if !known.contains(&f.name) {
                        *self.report.material_fields_dropped.entry(format!("{stage}.{}", f.name)).or_default() += 1;
                        if f.name.starts_with("use_") && matches!(value_of(&f.value), Some(ConfigValue::Boolean(true))) {
                            self.report.switches_not_in_026g.entry(format!("{stage}.{}", f.name)).or_default().push(name.to_owned());
                        }
                        continue;
                    }
                    let mut f = f.clone();
                    // A number a client patch reads is a float there (an editor's
                    // whole number is an int): written as 0.26g's materials keep it.
                    if self.effects.is_client_field(stage, &id, &f.name)
                        && let Some(v) = f.value.named_mut("value")
                        && let ConfigValue::Integer(i) = *v
                    {
                        *v = ConfigValue::Float(i as f32);
                    }
                    if f.name.starts_with("texture")
                        && let Some(ConfigValue::String(texture)) = value_of(&f.value)
                        && !texture.is_empty()
                    {
                        let used = self.texture(&texture.replace('\\', "/"))?;
                        if let Some(v) = f.value.named_mut("value") {
                            *v = ConfigValue::String(used);
                        }
                    }
                    out_effect.push(f);
                }
                let stage_known = self.effects.stage_fields.get(stage);
                let ConfigValue::Named(stage_fields) = &part.value else { unreachable!() };
                let mut out_stage = Vec::new();
                for f in stage_fields {
                    if f.name == "effect" {
                        out_stage.push(NamedValue { name: "effect".into(), value: ConfigValue::Named(std::mem::take(&mut out_effect)) });
                    } else if stage_known.is_some_and(|k| k.contains(&f.name)) {
                        out_stage.push(f.clone());
                    } else {
                        *self.report.material_fields_dropped.entry(format!("{stage}.{}", f.name)).or_default() += 1;
                    }
                }
                let mut out_stage = ConfigValue::Named(out_stage);
                self.programs(name, stage, &id, &mut out_stage);
                out_parts.push(NamedValue { name: stage.to_owned(), value: out_stage });
            }
            // A stage left out is not listed among the material's stages.
            for part in &mut out_parts {
                if part.name == "common"
                    && let Some(ConfigValue::Named(listed)) = part.value.named_mut("stages")
                {
                    listed.retain(|s| !dropped_stages.contains(&s.name));
                }
            }
            out_top.push(NamedValue { name: "material".into(), value: ConfigValue::Named(out_parts) });
        }
        Ok(ConfigValue::Named(out_top))
    }

    /// Gives the stage a switch set that has programs.
    fn programs(&mut self, material: &str, stage: &str, effect: &str, value: &mut ConfigValue) {
        let on: Switches = switches(value).into_iter().filter(|s| !self.effects.is_client_field(stage, effect, s)).collect();
        let stage_flags = flags(value);
        if self.effects.supplied.contains(&(stage.to_owned(), effect.to_owned())) || self.effects.has_programs(stage, effect, &stage_flags, &on) {
            return;
        }
        if self.unknown_switches == UnknownSwitches::Keep {
            self.report.kept_unknown_switches.insert(format!("{material}: {stage}"), on.into_iter().collect());
            return;
        }
        let Some(set) = self.effects.nearest(stage, effect, &stage_flags, &on) else {
            self.report.without_programs.push(format!("{material}: {stage} {effect} [{}] for {}", on.iter().cloned().collect::<Vec<_>>().join(","), stage_flags.join(",")));
            return;
        };
        let Some(ConfigValue::Named(fields)) = value.named_mut("effect") else { return };
        for f in fields.iter_mut() {
            let Some(v) = f.value.named_mut("value") else { continue };
            match v {
                ConfigValue::Boolean(b) if f.name.starts_with("use_") && !self.effects.is_client_field(stage, effect, &f.name) => {
                    *b = set.contains(&f.name)
                }
                ConfigValue::Integer(i) if is_selector(&f.name) => {
                    if let Some(n) = set.iter().find_map(|s| s.strip_prefix(&format!("{}=", f.name)).and_then(|n| n.parse().ok())) {
                        *i = n;
                    }
                }
                _ => {}
            }
        }
        self.report.degraded.push(Degraded {
            material: material.to_owned(),
            stage: stage.to_owned(),
            dropped: on.difference(&set).cloned().collect(),
            set: set.into_iter().collect(),
        });
    }

    /// The name to draw the donor's material `name` by.
    pub fn material(&mut self, name: &str) -> Result<String> {
        if let Some(known) = self.materials.get(name) {
            return Ok(known.clone());
        }
        let path = format!("material_instances/{name}.material");
        // A name is settled before its textures are walked: nothing loops.
        self.materials.insert(name.to_owned(), name.to_owned());
        if !self.donor.is_file(&path) {
            if !self.shipped.is_file(&path) {
                self.report.missing.insert(path);
            }
            return Ok(name.to_owned());
        }
        let own = parse(&self.donor.read(&path)?, &path)?;
        let made = self.material_026g(name, &own)?;
        let used = if !self.shipped.is_file(&path) {
            self.report.materials.added += 1;
            name.to_owned()
        } else if same_config(&made, &parse(&self.shipped.read(&path)?, &path)?) {
            // Players' own material: it draws as it does today. A switch set
            // it lost stays reported, for the programs that would restore it.
            self.report.materials.theirs += 1;
            return Ok(name.to_owned());
        } else {
            self.report.materials.imported += 1;
            let own = self.own(name);
            self.report.imported.insert(format!("material_instances/{own}.material"), "other values than players' material of the name".into());
            own
        };
        let to = format!("material_instances/{used}.material");
        self.out.insert(to.clone(), encode(made, &to)?);
        self.materials.insert(name.to_owned(), used.clone());
        Ok(used)
    }

    /// A model's settings with its materials by the names to draw them by.
    fn settings(&mut self, root: &ConfigValue) -> Result<ConfigValue> {
        let ConfigValue::Named(fields) = root else { bail!("a model's settings are not a table") };
        let mut out = Vec::new();
        for field in fields {
            let mut field = field.clone();
            if field.name.starts_with("material_settings")
                && let ConfigValue::Named(surfaces) = &mut field.value
            {
                for surface in surfaces {
                    if let Some(ConfigValue::String(material)) = surface.value.named_mut("material_name")
                        && !material.is_empty()
                    {
                        *material = self.material(&material.replace('\\', "/"))?;
                    }
                }
            }
            // An unassigned game material is id 65535 under either name.
            if field.name == "game_material_settings"
                && let ConfigValue::Named(shapes) = &mut field.value
            {
                for shape in shapes {
                    if shape.value.named("game_material_id") == Some(&ConfigValue::Integer(65535))
                        && let Some(name) = shape.value.named_mut("game_material_name")
                    {
                        *name = ConfigValue::String(String::new());
                    }
                }
            }
            out.push(field);
        }
        Ok(ConfigValue::Named(out))
    }

    /// The name to place the donor's model `name` by (`.model` or
    /// `.skinned_model`).
    pub fn model(&mut self, name: &str, kind: &'static str) -> Result<String> {
        if let Some(known) = self.models.get(&(name.to_owned(), kind)) {
            return Ok(known.clone());
        }
        let prefix = format!("models/{name}.{kind}/");
        // 0.26g's executable has no reader for a model's `hq_collision`
        // (later builds' finer hit mesh): it is not carried.
        let paths: Vec<String> = self
            .donor
            .paths_with_prefix(&prefix)
            .filter(|p| self.donor.is_file(p) && !p[prefix.len()..].starts_with("hq_collision/"))
            .map(str::to_owned)
            .collect();
        if paths.is_empty() {
            if !self.shipped.paths_with_prefix(&prefix).any(|p| self.shipped.is_file(p)) {
                self.report.missing.insert(prefix);
            }
            self.models.insert((name.to_owned(), kind), name.to_owned());
            return Ok(name.to_owned());
        }
        let theirs: BTreeSet<String> = self
            .shipped
            .paths_with_prefix(&prefix)
            .filter(|p| self.shipped.is_file(p) && !p[prefix.len()..].starts_with("hq_collision/"))
            .map(str::to_owned)
            .collect();
        let mut made = BTreeMap::new();
        let mut why = None;
        if !theirs.is_empty() && theirs != paths.iter().cloned().collect() {
            why = Some("other files than players' model of the name".to_owned());
        }
        for path in &paths {
            let own = self.donor.read(path)?;
            let part = &path[prefix.len()..];
            let bytes = if part == "settings" {
                let settings = self.settings(&parse(&own, path)?)?;
                if why.is_none() && theirs.contains(path) && !same_config(&settings, &self.settings_of_theirs(path)?) {
                    why = Some("other settings or materials than players' model of the name".to_owned());
                }
                encode(settings, path)?
            } else if kind == "model" && part.ends_with("/converted_model") {
                if why.is_none() && theirs.contains(path) {
                    let have = self.shipped.read(path)?;
                    if have != own && !same_surface(&own, &have) {
                        why = Some(format!("another mesh than players' {part}"));
                    }
                }
                surface_026g(&own, &mut self.report.surface_chunks_dropped).unwrap_or(own)
            } else {
                if why.is_none() && theirs.contains(path) && self.shipped.read(path)? != own {
                    why = Some(format!("another {part} than players'"));
                }
                own
            };
            made.insert(part.to_owned(), bytes);
        }
        let used = if theirs.is_empty() {
            self.report.models.added += 1;
            name.to_owned()
        } else if let Some(why) = why {
            self.report.models.imported += 1;
            let own = self.own(name);
            self.report.imported.insert(format!("models/{own}.{kind}"), why);
            own
        } else {
            self.report.models.theirs += 1;
            self.models.insert((name.to_owned(), kind), name.to_owned());
            return Ok(name.to_owned());
        };
        for (part, bytes) in made {
            self.out.insert(format!("models/{used}.{kind}/{part}"), bytes);
        }
        self.models.insert((name.to_owned(), kind), used.clone());
        Ok(used)
    }

    /// Players' settings of a model, with the unassigned game material
    /// written as the donor's are compared.
    fn settings_of_theirs(&mut self, path: &str) -> Result<ConfigValue> {
        let mut root = parse(&self.shipped.read(path)?, path)?;
        if let Some(ConfigValue::Named(shapes)) = root.named_mut("game_material_settings") {
            for shape in shapes {
                if shape.value.named("game_material_id") == Some(&ConfigValue::Integer(65535))
                    && let Some(name) = shape.value.named_mut("game_material_name")
                {
                    *name = ConfigValue::String(String::new());
                }
            }
        }
        Ok(root)
    }

    /// A collision's `resources/models/<name>.model/<part>[#scale]` by the
    /// model to place.
    pub fn collision(&mut self, reference: &str) -> Result<String> {
        let Some(rest) = reference.strip_prefix("resources/models/") else { return Ok(reference.to_owned()) };
        let Some((name, tail)) = rest.split_once(".model/") else { return Ok(reference.to_owned()) };
        Ok(format!("resources/models/{}.model/{tail}", self.model(name, "model")?))
    }

    /// A render visual's `model[set]` by the model to place.
    pub fn visual(&mut self, reference: &str) -> Result<String> {
        match reference.split_once('[') {
            Some((name, set)) => Ok(format!("{}[{set}", self.model(name, "model")?)),
            None => self.model(reference, "model"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(indices: &[u16], extra: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut chunk = |id: u32, body: &[u8]| {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(body);
        };
        chunk(VERTICES, &[1, 0, 0, 0, 9, 9, 9, 9]);
        let mut body = (indices.len() as u32).to_le_bytes().to_vec();
        body.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
        chunk(INDICES, &body);
        if extra {
            chunk(110, &[0; 4]);
        }
        out
    }

    #[test]
    fn the_same_triangles_in_another_order_are_the_same_surface() {
        let a = surface(&[0, 1, 2, 2, 1, 3], false);
        assert!(same_surface(&a, &surface(&[1, 3, 2, 1, 2, 0], true)));
        // A turned triangle faces the other way.
        assert!(!same_surface(&a, &surface(&[0, 2, 1, 2, 1, 3], false)));
        let mut dropped = BTreeMap::new();
        assert_eq!(surface_026g(&surface(&[0, 1, 2], true), &mut dropped).unwrap(), surface(&[0, 1, 2], false));
        assert_eq!(dropped.get(&110), Some(&1));
    }

    fn switch(name: &str, on: bool) -> NamedValue {
        NamedValue { name: name.into(), value: ConfigValue::Named(vec![NamedValue { name: "value".into(), value: ConfigValue::Boolean(on) }]) }
    }
    fn material(on: &[&str]) -> ConfigValue {
        let mut effect = vec![NamedValue { name: "effect_id".into(), value: ConfigValue::String("g_stage_default".into()) }];
        effect.extend(["use_tdiffuse", "use_nmap", "use_parallax", "use_tiled_vertex_blended_mask"].iter().map(|s| switch(s, on.contains(s))));
        let stage = ConfigValue::Named(vec![NamedValue { name: "effect".into(), value: ConfigValue::Named(effect) }, switch("with_static_meshes", true)]);
        let material = ConfigValue::Named(vec![NamedValue { name: "g_stage".into(), value: stage }]);
        ConfigValue::Named(vec![NamedValue { name: "material".into(), value: material }])
    }

    #[test]
    fn a_switch_set_without_programs_takes_the_nearest_that_adds_none() {
        let mut effects = Effects::default();
        for known in [&["use_tdiffuse"][..], &["use_tdiffuse", "use_nmap"], &["use_tdiffuse", "use_nmap", "use_parallax"]] {
            effects.learn_fields(&material(known));
            effects.learn_programs(&material(known));
        }
        let flags = vec!["with_static_meshes".to_owned()];
        let wanted: Switches = ["use_tdiffuse", "use_nmap", "use_tiled_vertex_blended_mask"].iter().map(|s| (*s).to_owned()).collect();
        assert!(!effects.has_programs("g_stage", "g_stage_default", &flags, &wanted));
        let nearest = effects.nearest("g_stage", "g_stage_default", &flags, &wanted).unwrap();
        // Not the set with parallax: that would add a switch.
        assert_eq!(nearest.into_iter().collect::<Vec<_>>(), ["use_nmap", "use_tdiffuse"]);
    }

    #[test]
    fn a_client_switch_is_kept_and_the_rest_of_the_set_judged_without_it() {
        let mut effects = Effects::default();
        effects.learn_fields(&material(&["use_tdiffuse"]));
        effects.learn_client_fields("stage\teffect\tfield\tpatch\twhat\ng_stage\tg_stage_default\tuse_parallax\t169\ta test\n").unwrap();
        effects.learn_programs(&material(&["use_tdiffuse", "use_parallax"]));
        let flags = vec!["with_static_meshes".to_owned()];
        let plain: Switches = ["use_tdiffuse"].iter().map(|s| (*s).to_owned()).collect();
        // Players' set with the client switch is known without it.
        assert!(effects.has_programs("g_stage", "g_stage_default", &flags, &plain));
        assert!(effects.is_client_field("g_stage", "g_stage_default", "use_parallax"));
        assert!(effects.effect_fields[&("g_stage".to_owned(), "g_stage_default".to_owned())].contains("use_parallax"));
    }

    #[test]
    fn configs_are_the_same_whatever_their_order_and_number_type() {
        let a = ConfigValue::Named(vec![switch("x", true), NamedValue { name: "n".into(), value: ConfigValue::Integer(2) }]);
        let b = ConfigValue::Named(vec![NamedValue { name: "n".into(), value: ConfigValue::Float(2.0) }, switch("x", true)]);
        assert!(same_config(&a, &b));
        assert!(!same_config(&a, &ConfigValue::Named(vec![switch("x", false), NamedValue { name: "n".into(), value: ConfigValue::Integer(2) }])));
    }
}
