// SPDX-License-Identifier: GPL-3.0-or-later
//! `port-modules`: 0.69d0's module system for 0.26g's guns
//! (`docs/modules-069d-026.md`; skhid-srp's `docs/notes/attachments.md`).
//!
//! From 0.69d0's `game.options` and models, for the guns of a remodel
//! sidecar:
//!
//! * each gun's two bodies: `whole` (every own part in the mesh, drawn when
//!   nothing is fitted) and `bare` (without the own parts a fit can
//!   displace, [`super::plan`]), on the live rig, laid on the live mesh by a
//!   measured shift ([`super::fit`]);
//! * every module a body leaves out, a fit brings or a player fits, as a
//!   static model of its own (0.69d0's vertices as they are: a part's origin
//!   is its slot's frame), its materials made as the guns';
//! * the modules table (`weapon_modules::encode`: guns, slots, own parts,
//!   carriers, fits, eyes, fire points, hand locators, shot effects);
//! * a check of every gun and fit against 0.69d0 itself ([`check`]).
//!
//! **Locators.** The game evaluates a locator record as `half turn x offset
//! x bone` (`weapon::calculate_locator`), and 0.69d0 draws a module at a
//! gun's locator the same way ([`super::socket`]): a gun's slot is its
//! locator as it is. A module's own locators are written in its model's
//! space, plainly: the forend's rails lie on its mesh at their positions,
//! the left one at -x. The table's rule for a part on a part is `locator x
//! half turn x parent`, so such a locator `L` goes into the table as `half
//! turn x L`. A grip's hand locators are written as a gun rig's (the left
//! hand at +x, as on the gun's own half-turned bones), so they go in as `L x
//! half turn`: the wrist's record is then `L x grip's offset`, what the
//! gun's own hand locator would be there.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use weapon_modules::encode::{AttachSource, GunSource, ModuleSource, Source, encode};
use weapon_modules::{Affine, DictId, EyeAnchor, Loadout, Placement, Role, SlotId, Table, Wanted};

use super::catalog::{AttachDef, Catalog, ModuleDef};
use super::plan::{self, Base, GunPlan, Kind, Site};
use super::{
    BindFrame, Body, Composed, Donor069, Gun069, Locator, Rig, Spec069, body_path, db_config, encode as encode_config,
    fit, list, locators, lod0_surfaces, socket, string_of,
};
use crate::skeleton_animation::{Clip, Rig as Skeleton};
use crate::weapon::parse_config;
use crate::weapons_026::modules::Mesh;
use crate::weapons_026::{field, get, s};
use crate::{ConfigValue as V, ResourceDb};

/// The table's file in the game data (`resources/gameplay/weapon_modules.srpm`
/// as the game asks for it).
pub const TABLE_PATH: &str = "gameplay/weapon_modules.srpm";
/// Where the modules' static models go: 0.69d0's `weapons/modules/<x>` at
/// `weapons/modules_069/<x>`, new to the players' files.
const MODULES_ROOT: &str = "weapons/modules_069";
/// A native static model's settings (a stock scope's), the modules' template.
const STATIC_SETTINGS: &str = "models/weapons/attach/leupold_3_7.model/settings";
/// The check's limit, metres.
pub const LIMIT: f64 = 0.0005;

/// A live weapon item of the sidecar: `items.tsv`'s row.
#[derive(Clone, Debug)]
pub struct LiveItem {
    /// Its name in the dictionary.
    pub name: String,
    /// Its 0.69d0 item (the sidecar's key).
    pub donor_item: String,
}

/// `items.tsv`: a live item, its config, its live model, its 0.69d0 item and
/// its remodel a line (`#` comments).
pub fn read_items(path: &Path) -> Result<Vec<LiveItem>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            ensure!(cols.len() == 5, "{}: a row of {} columns: {l}", path.display(), cols.len());
            Ok(LiveItem {
                name: cols[0].into(),
                donor_item: cols[3].into(),
            })
        })
        .collect()
}

/// The fitted modules' items: a 0.69d0 module id and our dict id a line
/// (tab-separated, `#` comments; more columns are the item's own). Append
/// only: an id is never given to another module.
pub fn read_ids(path: &Path) -> Result<BTreeMap<String, u16>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for l in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut cols = l.split('\t');
        let (module, id) = (cols.next().context("a module")?, cols.next().context("a dict id")?);
        let id: u16 = id.parse().with_context(|| format!("{module}: dict id {id}"))?;
        ensure!(id != 0 && seen.insert(id), "{}: dict id {id} twice or zero", path.display());
        ensure!(out.insert(module.to_owned(), id).is_none(), "{}: {module} twice", path.display());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Frames, in f64: the engine's row vectors (`v x M`).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    r: [[f64; 3]; 3],
    t: [f64; 3],
}

const HALF_TURN: Frame = Frame {
    r: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
    t: [0.0; 3],
};

impl Frame {
    const IDENTITY: Frame = Frame {
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };

    /// `create_rotation(angles) x create_translation(position)`: a
    /// locator's frame in its bone's, a bone's bind frame.
    fn of(angles: [f32; 3], position: [f32; 3]) -> Frame {
        Frame {
            r: socket::create_rotation(angles),
            t: position.map(f64::from),
        }
    }

    fn translation(t: [f32; 3]) -> Frame {
        Frame {
            t: t.map(f64::from),
            ..Frame::IDENTITY
        }
    }

    fn turn(&self, v: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|c| (0..3).map(|r| v[r] * self.r[r][c]).sum())
    }

    fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let p = self.turn(v);
        std::array::from_fn(|k| p[k] + self.t[k])
    }

    /// `self x next`: through `self`, then through `next`.
    fn then(&self, next: &Frame) -> Frame {
        Frame {
            r: std::array::from_fn(|row| next.turn(self.r[row])),
            t: next.apply(self.t),
        }
    }

    fn affine(&self) -> Affine {
        let row = |r: [f64; 3]| r.map(|c| c as f32);
        Affine {
            x: row(self.r[0]),
            y: row(self.r[1]),
            z: row(self.r[2]),
            t: row(self.t),
        }
    }

    fn from_affine(a: &Affine) -> Frame {
        let row = |r: [f32; 3]| r.map(f64::from);
        Frame {
            r: [row(a.x), row(a.y), row(a.z)],
            t: row(a.t),
        }
    }
}

fn bind_frame(b: &BindFrame) -> Frame {
    Frame::of(b.rotation, b.translation)
}

fn locator_frame(l: &Locator) -> Frame {
    Frame::of(l.rotation, l.position)
}

// ---------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------

/// A gun as built: its plan, what composing it left, and where it lies.
struct Built {
    item: String,
    gun: Gun069,
    plan: GunPlan,
    /// The 0.69d0 item whose links it takes.
    fits_like: String,
    composed: Composed,
    fit: Option<fit::Fit>,
}

/// One check: a part of a gun with something fitted, ours against 0.69d0's.
#[derive(Clone, Debug, Serialize)]
pub struct Checked {
    pub gun: String,
    /// The fitted module, or `-` for a refusal of the fit itself.
    pub fit: String,
    /// What was compared: a part's model, `eye`, `fire_point`, `hand`, `optic`.
    pub what: String,
    /// The largest distance between ours and 0.69d0's, mm.
    pub mm: f64,
    pub pass: bool,
    /// Why it fails, when it is not the distance.
    pub note: String,
}

#[derive(Serialize)]
pub struct GunReport {
    pub model: String,
    pub bare: String,
    pub fits_like: String,
    pub fit: Option<fit::Fit>,
    pub plan: GunPlan,
}

#[derive(Serialize)]
pub struct Report {
    pub guns: BTreeMap<String, GunReport>,
    /// By live item: its dict id and its 0.69d0 item.
    pub items: BTreeMap<String, (u16, String)>,
    /// Modules 0.69d0 sells that have no item of ours (no row in the ids).
    pub without_item: BTreeSet<String>,
    pub modules: usize,
    pub table_bytes: usize,
    pub checks: Vec<Checked>,
    pub files: usize,
}

impl Report {
    pub fn failures(&self) -> impl Iterator<Item = &Checked> {
        self.checks.iter().filter(|c| !c.pass)
    }
}

/// A module's model at its new path (`weapons/modules/<x>` to
/// `weapons/modules_069/<x>`): the text the table and an item's config hold.
fn module_path(model: &str) -> Result<String> {
    let rest = model
        .strip_prefix("weapons/modules/")
        .with_context(|| format!("{model}: a module's model outside weapons/modules/"))?;
    Ok(format!("{MODULES_ROOT}/{rest}"))
}

fn slot_id(name: &str) -> Result<SlotId> {
    SlotId::from_name(name)
        .with_context(|| format!("slot {name} is none of the table's (skhid-srp's scripts/weapon-modules/slot_ids.py)"))
}

/// The base gun of an item whose own links fit nothing: the item of the
/// longest proper prefix of its name (cut at `_`) that 0.69d0 links modules
/// to (`ak_74n_premium`: `ak_74n`).
fn base_of<'a>(catalog: &'a Catalog, item: &str) -> Option<&'a str> {
    let mut cut = item;
    while let Some((head, _)) = cut.rsplit_once('_') {
        if let Some((name, _)) = catalog.links.get_key_value(head) {
            return Some(name);
        }
        cut = head;
    }
    None
}

impl Donor069 {
    /// A module's static model (`weapons/modules/<x>`) written at its new
    /// path, once; its materials as the guns'.
    fn module_model(&mut self, model: &str, target: &mut ResourceDb, work: &Path) -> Result<String> {
        let path = module_path(model)?;
        let root = format!("models/{path}.model");
        if self.files.contains_key(&format!("{root}/settings")) {
            return Ok(path);
        }
        ensure!(
            !target.contains(&format!("{root}/settings")),
            "{root}: the target has it; a module's model path must be new"
        );
        let source = format!("models/{model}.model");
        let settings = db_config(&mut self.db, &format!("{source}/settings"))?;
        let mut surfaces = vec![];
        for (surface, material) in lod0_surfaces(&settings, &source)? {
            let bytes = self.db.read(&format!("{source}/render/{surface}/converted_model"))?;
            let mesh = super::convert::static_069_to_026(&bytes).with_context(|| format!("{source}: {surface}"))?;
            let material = self
                .compose_material(&material, target, work)
                .with_context(|| format!("material {material}"))?;
            surfaces.push((surface, material, mesh));
        }
        let mut native = parse_config(target, STATIC_SETTINGS, "native static model settings")?.root;
        native.set_path(
            "lod_hierrarchy",
            V::Named(vec![field(
                "LOD0",
                V::Named(vec![field(
                    "surfaces",
                    V::Indexed(surfaces.iter().map(|(n, _, _)| s(n)).collect()),
                )]),
            )]),
        )?;
        native.set_path(
            "material_settings",
            V::Named(
                surfaces
                    .iter()
                    .map(|(n, m, _)| {
                        field(
                            n,
                            V::Named(vec![
                                field("material_name", s(m)),
                                field("used_in_portals", V::Boolean(false)),
                            ]),
                        )
                    })
                    .collect(),
            ),
        )?;
        for key in ["collision_enabled", "walker_collision_enabled", "collision_use_raytrace_for_walker"] {
            if native.named(key).is_some() {
                native.set_path(key, V::Boolean(false))?;
            }
        }
        self.put(format!("{root}/settings"), encode_config(native)?)?;
        let bounds = |points: &[[f32; 3]]| -> ([f32; 3], [f32; 3]) {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for p in points {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (lo, hi)
        };
        let box_value = |(lo, hi): ([f32; 3], [f32; 3])| {
            V::Named(vec![field("max", V::Float3(hi)), field("min", V::Float3(lo))])
        };
        let mut all = vec![];
        for (name, _, mesh) in &surfaces {
            let parsed = Mesh::parse(mesh)?;
            let (lo, hi) = bounds(&parsed.positions);
            let centre: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0);
            let radius = parsed
                .positions
                .iter()
                .map(|v| (0..3).map(|k| (v[k] - centre[k]).powi(2)).sum::<f32>().sqrt())
                .fold(0.0, f32::max);
            let props = V::Named(vec![
                field(
                    "bounding_sphere",
                    V::Named(vec![field("radius", V::Float(radius)), field("center", V::Float3(centre))]),
                ),
                field(
                    "source",
                    V::Named(vec![
                        field("indices", V::Integer(i32::try_from(parsed.triangles.len() * 3)?)),
                        field("primitives", V::Integer(i32::try_from(parsed.triangles.len())?)),
                        field("mesh_name", s(format!("|{name}"))),
                        field("texture_name", s("")),
                        field("vertices", V::Integer(i32::try_from(parsed.positions.len())?)),
                    ]),
                ),
                field("type", V::Integer(0)),
                field("version", V::Integer(0)),
                field("bounding_box", box_value((lo, hi))),
            ]);
            self.put(format!("{root}/render/{name}/export_properties"), encode_config(props)?)?;
            self.put(format!("{root}/render/{name}/converted_model"), mesh.clone())?;
            all.extend(parsed.positions);
        }
        let props = V::Named(vec![
            field("src_filename", s(format!("Survarium 0.69d0: {source}"))),
            field("type", V::Integer(1)),
            field("version", V::Integer(0)),
            field("bounding_box", box_value(bounds(&all))),
        ]);
        self.put(format!("{root}/render/export_properties"), encode_config(props)?)?;
        Ok(path)
    }

    /// A module model's locators, by name (0.69d0's, in its model's space).
    fn module_locators(&mut self, model: &str) -> Result<BTreeMap<String, Frame>> {
        let props = db_config(&mut self.db, &format!("models/{model}.model/render/export_properties"))?;
        Ok(locators(&props)?
            .into_iter()
            .map(|l| (l.name.clone(), locator_frame(&l)))
            .collect())
    }

    /// A model's LOD0 vertices: a module's of 0.69d0 as its files have them.
    fn donor_points(&mut self, model: &str) -> Result<Vec<[f32; 3]>> {
        let source = format!("models/{model}.model");
        let settings = db_config(&mut self.db, &format!("{source}/settings"))?;
        let mut out = vec![];
        for (surface, _) in lod0_surfaces(&settings, &source)? {
            let bytes = self.db.read(&format!("{source}/render/{surface}/converted_model"))?;
            out.extend(super::convert::static_positions_069(&bytes)?);
        }
        Ok(out)
    }

    /// The same model's vertices as the bundle has them (what the game
    /// would draw).
    fn built_points(&self, path: &str) -> Result<Vec<[f32; 3]>> {
        let root = format!("models/{path}.model");
        let settings = crate::BinaryConfig::parse(
            self.files
                .get(&format!("{root}/settings"))
                .with_context(|| format!("{root} is not in the bundle"))?,
        )?
        .root;
        let mut out = vec![];
        for (surface, _) in lod0_surfaces(&settings, &root)? {
            let bytes = &self.files[&format!("{root}/render/{surface}/converted_model")];
            out.extend(Mesh::parse(bytes)?.positions);
        }
        Ok(out)
    }
}

/// A live model's LOD0 vertices (mesh space, bind pose).
fn live_points(target: &mut ResourceDb, model: &str) -> Result<Vec<[f32; 3]>> {
    let settings = parse_config(target, &format!("{model}/settings"), "live model settings")?.root;
    let mut out = vec![];
    for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
        let surface = string_of(Some(surface))?;
        let bytes = target.read(&format!("{model}/render/{surface}/converted_model"))?;
        out.extend(Mesh::parse(&bytes)?.positions);
    }
    Ok(out)
}

/// The rows of the modules table being made: a module is a row a role and
/// a set of rules (a gun's own part's are its slot's alone; an exception's
/// models are a row of their own).
#[derive(Default)]
struct Rows {
    modules: Vec<ModuleSource>,
    keys: BTreeMap<String, usize>,
}

impl Rows {
    fn row(&mut self, module: ModuleSource) -> usize {
        let key = format!("{module:?}");
        if let Some(at) = self.keys.get(&key) {
            return *at;
        }
        self.modules.push(module);
        self.keys.insert(key, self.modules.len() - 1);
        self.modules.len() - 1
    }
}

/// What a module needs of the donor to become a row: its models' locators.
struct ModuleMaker<'a> {
    catalog: &'a Catalog,
    ids: &'a BTreeMap<String, u16>,
}

impl ModuleMaker<'_> {
    /// `module`'s row as `role` on `item`, with its rules in `slots` (every
    /// slot it goes in there when None).
    fn source(
        &self,
        donor: &mut Donor069,
        target: &mut ResourceDb,
        work: &Path,
        module: &ModuleDef,
        role: Role,
        item: &str,
        slots: Option<&[&str]>,
    ) -> Result<ModuleSource> {
        let models = module.models_on(item);
        let model = models.model.with_context(|| format!("{} has no model", module.id))?;
        let own = donor.module_locators(&model)?;
        let locator = |name: &str| -> Result<Frame> {
            own.get(name)
                .copied()
                .with_context(|| format!("{}: its model {model} has no locator {name}", module.id))
        };
        let mut attaches = vec![];
        for rules in module.attaches_on(item) {
            if slots.is_some_and(|only| !only.contains(&rules.slot.as_str())) {
                continue;
            }
            attaches.push(self.attach(rules, &locator, role)?);
        }
        ensure!(!attaches.is_empty(), "{}: no slot of {slots:?} on {item}", module.id);
        let hand = match role {
            Role::Fitted(weapon_modules::Kind::Grip) => {
                let mut hand = [Affine::IDENTITY; 16];
                for (out, name) in hand.iter_mut().zip(&module.finger_locators) {
                    // Written as a gun rig's: `L x half turn` (the module's header).
                    *out = locator(name)?.then(&HALF_TURN).affine();
                }
                Some(hand)
            }
            _ => None,
        };
        let path = |m: &Option<String>, donor: &mut Donor069, target: &mut ResourceDb| -> Result<Option<String>> {
            match m {
                Some(m) if *m != model => Ok(Some(donor.module_model(m, target, work)?)),
                _ => Ok(None),
            }
        };
        let alt = path(&models.alt, donor, target)?;
        let aimed = match role {
            Role::Fitted(weapon_modules::Kind::Scope) => path(&models.aimed, donor, target)?,
            _ => None,
        };
        Ok(ModuleSource {
            dict: self.ids.get(&module.id).copied().unwrap_or(u16::try_from(module.dict_id)?),
            role,
            model: donor.module_model(&model, target, work)?,
            alt,
            aimed,
            attaches,
            hand,
        })
    }

    fn attach(&self, rules: &AttachDef, locator: &dyn Fn(&str) -> Result<Frame>, role: Role) -> Result<AttachSource> {
        // A module's own locator `L` is the table's `half turn x L`.
        let own = |name: &str| -> Result<Affine> { Ok(HALF_TURN.then(&locator(name)?).affine()) };
        let mut out = AttachSource::new(slot_id(&rules.slot)?);
        for add in &rules.adds {
            out.adds.push((slot_id(&add.id)?, own(&add.locator)?));
        }
        for point in &rules.overrides {
            match point.id.as_str() {
                "aim" => out.eye = Some(own(&point.locator)?),
                "fire_point" => out.fire_point = Some(own(&point.locator)?),
                // The charging handle and the second aim points: 0.26g has one zoom and one eye.
                _ => {}
            }
        }
        // Only an optic's eye is the gun's while it is on (a rail mount's is where the bare
        // rail would aim; 0.26g aims through the optic on it or over the gun's own sights).
        if !matches!(role, Role::Fitted(weapon_modules::Kind::Scope) | Role::Stock) {
            out.eye = None;
        }
        for (slot, param) in &rules.alters {
            if *param == 1 {
                out.alters.push(slot_id(slot)?);
            }
        }
        for slot in &rules.closes {
            out.closes.push(slot_id(slot)?);
        }
        out.shot_effect = rules.shoot.clone();
        let _ = self.catalog;
        Ok(out)
    }
}

/// The gun's bones' matrices in its aimed idle pose (the model's space):
/// its config's `states/aimed_idle` clip, else `states/idle`'s, at its start.
fn aimed_pose(target: &mut ResourceDb, config: &V, bind: &[u8]) -> Result<Vec<Frame>> {
    let clip = ["states.aimed_idle.animations.0", "states.idle.animations.0"]
        .iter()
        .find_map(|key| get(config, key))
        .context("the config has no idle clip")?;
    let clip = string_of(Some(clip))?;
    let clip = clip.strip_prefix("resources/").unwrap_or(clip);
    let skeleton = format!(
        "animations/skeletons/{}.skeleton",
        string_of(get(config, "object.skeleton"))?
    );
    let rig = Skeleton::parse(bind, &target.read(&skeleton)?).with_context(|| skeleton.clone())?;
    let clip = Clip::parse(
        &target.read(&format!("{clip}.b-spline"))?,
        &target.read(&format!("{clip}.bones_names"))?,
    )
    .with_context(|| clip.to_owned())?;
    Ok(rig
        .pose(&clip, 0.0)
        .into_iter()
        .map(|(basis, at)| Frame {
            r: basis.map(|row| row.map(f64::from)),
            t: at.map(f64::from),
        })
        .collect())
}

/// Builds the bundle. `only`: the 0.69d0 items to build (all when empty).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn port_modules(
    id: &str,
    spec: &Spec069,
    items: &[LiveItem],
    ids: &BTreeMap<String, u16>,
    only: &[String],
    donor_db: &Path,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<Report> {
    ensure!(!output.exists(), "{} is already there", output.display());
    let work = output.with_extension("work");
    let mut donor = Donor069::empty(donor_db, spec)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let catalog = Catalog::read(&donor.game)?;
    let offered = |m: &ModuleDef| ids.contains_key(&m.id);
    let dictionary = parse_config(&mut target, "gameplay/db_static_dictionaries", "dictionary")?.root;
    let dict_row = |name: &str| -> Result<(u16, String)> {
        let row = list(dictionary.named("items_dict"))
            .iter()
            .find(|r| r.named("name") == Some(&V::String(name.into())))
            .with_context(|| format!("the dictionary has no item {name}"))?;
        let Some(V::Integer(id)) = row.named("dict_id") else {
            bail!("{name}: no dict id")
        };
        Ok((u16::try_from(*id)?, string_of(row.named("cfg_name"))?.to_owned()))
    };

    // Each gun: its plan, where it lies, its two bodies.
    let mut built: Vec<Built> = vec![];
    let mut without_item = BTreeSet::new();
    for (item, gun) in &spec.guns {
        if !only.is_empty() && !only.contains(item) {
            continue;
        }
        let path = gun
            .model_path
            .as_ref()
            .with_context(|| format!("{item}: a gun of the modules names its model_path"))?;
        ensure!(
            gun.optic_model_path.is_none(),
            "{item}: optic_model_path: a folded body is the bare one with its sights as parts"
        );
        let own = gun.own(catalog.gun(item)?);
        let mut fits_like = item.clone();
        let mut plan = plan::plan(&catalog, item, &own, item, &offered)?;
        if plan.fits.is_empty()
            && let Some(base) = base_of(&catalog, item)
        {
            fits_like = base.to_owned();
            plan = plan::plan(&catalog, item, &own, base, &offered)?;
        }
        for module in catalog.links.get(&fits_like).into_iter().flatten() {
            let module = catalog.module(module)?;
            let sold = plan::kind(module).is_some_and(|k| k != Kind::Scope) && catalog.has_stats(module);
            if sold && !offered(module) {
                without_item.insert(module.id.clone());
            }
        }
        let mut gun = gun.clone();
        let mut measured = None;
        if let Rig::Live(live) = &gun.rig {
            gun.offset = [0.0; 3];
            let ours = donor
                .compose_model(item, &gun, &Body::Measure, &mut target, &work)
                .with_context(|| format!("{item}: measuring"))?;
            let fit = fit::fit(&live_points(&mut target, live)?, &ours.points);
            gun.offset = fit.shift;
            measured = Some(fit);
        }
        let composed = donor
            .compose_model(item, &gun, &Body::Whole, &mut target, &work)
            .with_context(|| format!("{item}: whole body"))?;
        let leave_out: BTreeSet<(String, String)> = plan
            .stock()
            .map(|(_, p)| (p.module.clone(), p.site.locator.clone()))
            .collect();
        for (module, locator) in &leave_out {
            ensure!(
                donor.modules[item].iter().any(|m| m.baked && m.id == *module && m.locator == *locator),
                "{item}: its own part {module} at {locator} is not in its whole body"
            );
        }
        donor
            .compose_model(item, &gun, &Body::Bare(&leave_out), &mut target, &work)
            .with_context(|| format!("{item}: bare body"))?;
        let _ = path;
        built.push(Built {
            item: item.clone(),
            gun,
            plan,
            fits_like,
            composed,
            fit: measured,
        });
    }

    // The table's rows.
    let maker = ModuleMaker {
        catalog: &catalog,
        ids,
    };
    let mut rows = Rows::default();
    let mut source = Source::default();
    let mut report_items = BTreeMap::new();
    // By gun: its table index, and each fit's and own part's module row.
    let mut gun_at: BTreeMap<String, usize> = BTreeMap::new();
    for live in items {
        let Some(b) = built.iter().find(|b| b.item == live.donor_item) else {
            continue;
        };
        let (dict, config_path) = dict_row(&live.name)?;
        let config = parse_config(&mut target, &config_path, "weapon config")?.root;
        let item = &b.item;
        let model_path = b.gun.model_path.as_deref().unwrap_or_default();
        let ship_bind_bytes = donor.files[&format!("models/{model_path}.skinned_model/render/bind_pose")].clone();
        let pose = aimed_pose(&mut target, &config, &ship_bind_bytes).with_context(|| format!("{}: aimed pose", live.name))?;
        let find = |name: &str| -> Result<&Locator> {
            b.composed
                .locators
                .iter()
                .find(|l| l.name == name)
                .with_context(|| format!("{item}: its model has no locator {name}"))
        };
        // Where a slot's frame is: a locator of the gun, or of an own part standing on one.
        fn seat(
            b: &Built,
            catalog: &Catalog,
            donor: &mut Donor069,
            site: &Site,
            find: &dyn Fn(&str) -> Result<Locator>,
        ) -> Result<Placement> {
            match site.base {
                Base::Gun => {
                    let l = find(&site.locator)?;
                    Ok(Placement {
                        bone: weapon_modules::BoneIndex(u16::try_from(l.bone_idx)?),
                        offset: locator_frame(&l).affine(),
                    })
                }
                Base::Part(on) => {
                    let part = &b.plan.parts[on];
                    let parent = seat(b, catalog, donor, &part.site, find)?;
                    let model = catalog
                        .module(&part.module)?
                        .models_on(&b.item)
                        .model
                        .context("a part without a model")?;
                    let own = donor.module_locators(&model)?;
                    let l = own
                        .get(&site.locator)
                        .with_context(|| format!("{}: its model has no locator {}", part.module, site.locator))?;
                    Ok(parent.child(&HALF_TURN.then(l).affine()))
                }
            }
        }
        let find_owned = |name: &str| find(name).cloned();
        let mut gun = GunSource {
            dict,
            bones: u16::try_from(b.composed.ship_bind.len())?,
            aim_bone: u16::try_from(find("aim")?.bone_idx)?,
            fits_of: None,
            bare: body_path(model_path, &Body::Bare(&BTreeSet::new())),
            ..GunSource::default()
        };
        // The gun's slots: its own and those of the own parts that stay in the bare body.
        let mut bones = BTreeSet::from([gun.aim_bone]);
        for (name, site) in &b.plan.slots {
            if matches!(site.base, Base::Part(on) if b.plan.parts[on].stock) {
                continue;
            }
            // A slot the table has no id for is none a fit of ours goes in.
            let Some(slot) = SlotId::from_name(name) else {
                continue;
            };
            let at = seat(b, &catalog, &mut donor, site, &find_owned).with_context(|| format!("{item}: slot {name}"))?;
            bones.insert(at.bone.0);
            gun.slots.push((slot, at.bone.0, at.offset));
        }
        for bone in bones {
            let frame = pose
                .get(usize::from(bone))
                .with_context(|| format!("{item}: bone {bone} has no pose"))?;
            gun.pose.push((bone, frame.affine()));
        }
        // Its own parts a fit can displace.
        for (_, part) in b.plan.stock() {
            let module = catalog.module(&part.module)?;
            let row = maker
                .source(&mut donor, &mut target, &work, module, Role::Stock, item, Some(&[part.slot.as_str()]))
                .with_context(|| format!("{item}: own part {}", part.module))?;
            gun.stock.push((slot_id(&part.slot)?, rows.row(row)));
        }
        // A premium item takes its base gun's fits: the base's record, when it is a gun of ours.
        let base = items
            .iter()
            .find(|i| i.donor_item == b.fits_like && b.fits_like != *item)
            .and_then(|i| gun_at.get(&i.name).copied());
        if b.fits_like != *item && base.is_some() {
            gun.fits_of = base;
        } else {
            for fit in &b.plan.fits {
                let module = catalog.module(&fit.module)?;
                let role = Role::Fitted(match fit.kind {
                    Kind::Scope => weapon_modules::Kind::Scope,
                    Kind::Muzzle => weapon_modules::Kind::Muzzle,
                    Kind::Grip => weapon_modules::Kind::Grip,
                });
                let row = maker
                    .source(&mut donor, &mut target, &work, module, role, item, None)
                    .with_context(|| format!("{item}: fit {}", fit.module))?;
                gun.fits.push(rows.row(row));
            }
        }
        for (brings, links) in &b.plan.carriers {
            let mut chain = vec![];
            for link in links {
                let module = catalog.module(&link.module)?;
                let row = maker
                    .source(&mut donor, &mut target, &work, module, Role::Carrier, item, None)
                    .with_context(|| format!("{item}: carrier {}", link.module))?;
                chain.push((rows.row(row), slot_id(&link.slot)?));
            }
            gun.carriers.push((slot_id(brings)?, chain));
        }
        gun_at.insert(live.name.clone(), source.guns.len());
        report_items.insert(live.name.clone(), (dict, item.clone()));
        source.guns.push(gun);
    }
    source.modules = rows.modules;
    let table = encode(&source).map_err(|e| anyhow::anyhow!("the modules table: {e:?}"))?;
    donor.put(TABLE_PATH.into(), table.clone())?;

    // The check against 0.69d0.
    let parsed = Table::parse(&table).map_err(|e| anyhow::anyhow!("the written table: {e:?}"))?;
    let mut checks = vec![];
    for live in items {
        let Some(b) = built.iter().find(|b| b.item == live.donor_item) else {
            continue;
        };
        let dict = report_items[&live.name].0;
        check(&mut donor, &catalog, &parsed, b, &live.name, dict, ids, &mut checks)
            .with_context(|| format!("checking {}", live.name))?;
    }
    if work.exists() {
        fs::remove_dir(&work).context("the texture encoder left files behind")?;
    }

    // Files.
    let overlay = output.join("overlay");
    let mut files = vec![];
    for (path, bytes) in &donor.files {
        if target.contains(path) {
            ensure!(target.read(path)? == *bytes, "the target has {path}, otherwise");
        }
        let dest = crate::archive::safe_join(&overlay, path)?;
        fs::create_dir_all(dest.parent().context("a file without a folder")?)?;
        fs::write(&dest, bytes)?;
        files.push(crate::weapon::record_file(path, "0.69d0", bytes));
    }
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format_version": 1,
            "asset_id": id,
            "donor_build": crate::weapons_026::donor::DonorBuild::V069d0,
            "files": files,
            "target": target_layers,
            "items": [],
            "compatibility": [],
            "icons": [],
            "reused_from_target": [],
            "notes": {},
        }))?,
    )?;
    let report = Report {
        guns: built
            .into_iter()
            .map(|b| {
                let model = b.gun.model_path.clone().unwrap_or_default();
                let report = GunReport {
                    bare: body_path(&model, &Body::Bare(&BTreeSet::new())),
                    model,
                    fits_like: b.fits_like,
                    fit: b.fit,
                    plan: b.plan,
                };
                (b.item, report)
            })
            .collect(),
        items: report_items,
        without_item,
        modules: source.modules.len(),
        table_bytes: table.len(),
        checks,
        files: files.len(),
    };
    fs::write(output.join("modules-069d.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

/// The largest distance between `points` through `ours` and through
/// `theirs`, metres.
fn spread(points: &[[f32; 3]], ours: &Frame, theirs: &Frame) -> f64 {
    points
        .iter()
        .map(|p| {
            let p = p.map(f64::from);
            let (a, b) = (ours.apply(p), theirs.apply(p));
            (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
        })
        .fold(0.0, f64::max)
}

/// A frame's probe: its origin and a point 10 cm along each axis.
const PROBE: [[f32; 3]; 4] = [[0.0; 3], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0], [0.0, 0.0, 0.1]];

/// Every fit of one gun, alone: what [`weapon_modules::resolve`] and the
/// bundle's models would draw against 0.69d0's own (its base model in its
/// own bind pose, each module at 0.69d0's locators by the engine's
/// arithmetic, [`socket`]; a module on a module at that one's locator as
/// it is). Both in mesh space, ours less the gun's measured shift.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn check(
    donor: &mut Donor069,
    catalog: &Catalog,
    table: &Table,
    b: &Built,
    live_item: &str,
    dict: u16,
    ids: &BTreeMap<String, u16>,
    out: &mut Vec<Checked>,
) -> Result<()> {
    let item = &b.item;
    let gun = table
        .gun(DictId::new(dict).context("a dict id of 0")?)
        .with_context(|| format!("the table has no gun {dict}"))?;
    // 0.69d0's frame of a locator of the gun: half turn x locator x its bone's bind frame.
    let donor_gun = |name: &str| -> Result<Frame> {
        let l = b
            .composed
            .donor_locators
            .iter()
            .find(|l| l.name == name)
            .with_context(|| format!("0.69d0's {item} has no locator {name}"))?;
        let (_, bone) = b
            .composed
            .donor_bind
            .iter()
            .find(|(n, _)| *n == l.bone)
            .with_context(|| format!("{name} names an unknown bone {}", l.bone))?;
        Ok(HALF_TURN.then(&locator_frame(l)).then(&bind_frame(bone)))
    };
    fn site_frame(
        b: &Built,
        catalog: &Catalog,
        donor: &mut Donor069,
        site: &Site,
        donor_gun: &dyn Fn(&str) -> Result<Frame>,
    ) -> Result<Frame> {
        match site.base {
            Base::Gun => donor_gun(&site.locator),
            Base::Part(on) => {
                let part = &b.plan.parts[on];
                let parent = site_frame(b, catalog, donor, &part.site, donor_gun)?;
                let model = catalog
                    .module(&part.module)?
                    .models_on(&b.item)
                    .model
                    .context("a part without a model")?;
                let own = donor.module_locators(&model)?;
                let l = own
                    .get(&site.locator)
                    .with_context(|| format!("{}: no locator {}", part.module, site.locator))?;
                Ok(l.then(&parent))
            }
        }
    }
    let shift = Frame::translation(b.gun.offset);
    let ship: Vec<Frame> = b.composed.ship_bind.iter().map(|(_, f)| bind_frame(f)).collect();
    // Ours: the game's own formula for a record, in the shipped bind pose.
    let ours = |at: &Placement| -> Result<Frame> {
        let bone = ship
            .get(usize::from(at.bone.0))
            .with_context(|| format!("a placement on bone {} of {}", at.bone.0, ship.len()))?;
        Ok(HALF_TURN.then(&Frame::from_affine(&at.offset)).then(bone))
    };
    let fits = if b.plan.fits.is_empty() || gun.fits().next().is_none() {
        return Ok(());
    } else {
        &b.plan.fits
    };
    for fit in fits {
        let module = catalog.module(&fit.module)?;
        let mut row = |what: &str, metres: f64, note: &str| {
            out.push(Checked {
                gun: live_item.to_owned(),
                fit: fit.module.clone(),
                what: what.to_owned(),
                mm: (metres * 1e6).round() / 1e3,
                pass: note.is_empty() && metres <= LIMIT,
                note: note.to_owned(),
            });
        };
        let wanted_id = ids.get(&fit.module).copied().and_then(DictId::new).context("a fit without an id")?;
        let wanted = match fit.kind {
            Kind::Scope => Wanted {
                scope: Some(wanted_id),
                ..Wanted::default()
            },
            Kind::Muzzle => Wanted {
                muzzle: Some(wanted_id),
                ..Wanted::default()
            },
            Kind::Grip => Wanted {
                grip: Some(wanted_id),
                ..Wanted::default()
            },
        };
        let bare = match weapon_modules::resolve(gun, wanted) {
            Ok(Loadout::Bare(bare)) => bare,
            Ok(Loadout::Whole) => {
                row("-", 0.0, "resolve draws the whole body");
                continue;
            }
            Err(refusal) => {
                row("-", 0.0, &format!("resolve refuses it: {refusal:?}"));
                continue;
            }
        };
        // 0.69d0's side: each model that should be drawn, at its frame.
        let mut expected: BTreeMap<String, (String, Frame)> = BTreeMap::new();
        let mut frames: BTreeMap<String, Frame> = BTreeMap::new();
        // The carriers, then the fit, each on the gun or on the one before.
        let mut brought: BTreeMap<String, Frame> = BTreeMap::new();
        let mut displaced: BTreeSet<usize> = BTreeSet::new();
        let mut goes: Vec<(&ModuleDef, &str)> = fit
            .chain
            .iter()
            .map(|l| Ok((catalog.module(&l.module)?, l.slot.as_str())))
            .collect::<Result<_>>()?;
        goes.push((module, &fit.slot));
        let mut folded: BTreeSet<&str> = BTreeSet::new();
        let mut fit_frame = Frame::IDENTITY;
        for (m, slot) in &goes {
            let frame = match brought.get(*slot) {
                Some(frame) => *frame,
                None => {
                    let site = b
                        .plan
                        .slots
                        .get(*slot)
                        .with_context(|| format!("{item} has no slot {slot}"))?;
                    site_frame(b, catalog, donor, site, &donor_gun)?
                }
            };
            let models = m.models_on(item);
            let model = models.model.clone().context("a module without a model")?;
            let own = donor.module_locators(&model)?;
            if let Some(rules) = m.attach_on(item, slot) {
                for add in &rules.adds {
                    if let Some(l) = own.get(&add.locator) {
                        brought.insert(add.id.clone(), l.then(&frame));
                    }
                }
                for closed in &rules.closes {
                    displaced.extend(b.plan.parts.iter().position(|p| p.stock && p.slot == *closed));
                }
                folded.extend(rules.alters.iter().filter(|(_, p)| *p == 1).map(|(s, _)| s.as_str()));
            }
            displaced.extend(b.plan.parts.iter().position(|p| p.stock && p.slot == *slot));
            frames.insert(m.id.clone(), frame);
            fit_frame = frame;
            // The optic itself is the game's scope object's: its frame is checked below.
            if !(fit.kind == Kind::Scope && std::ptr::eq(*m, module)) {
                expected.insert(module_path(&model)?, (model, frame));
            }
        }
        // The gun's own parts that stay, whatever stood on a displaced one gone with it.
        loop {
            let more: Vec<usize> = b
                .plan
                .parts
                .iter()
                .enumerate()
                .filter(|(at, p)| p.stock && !displaced.contains(at) && matches!(p.site.base, Base::Part(on) if displaced.contains(&on)))
                .map(|(at, _)| at)
                .collect();
            if more.is_empty() {
                break;
            }
            displaced.extend(more);
        }
        for (at, part) in b.plan.stock() {
            if displaced.contains(&at) {
                continue;
            }
            let models = catalog.module(&part.module)?.models_on(item);
            let model = match (&models.alt, folded.contains(part.slot.as_str())) {
                (Some(alt), true) => alt.clone(),
                _ => models.model.clone().context("a part without a model")?,
            };
            let frame = site_frame(b, catalog, donor, &part.site, &donor_gun)?;
            expected.insert(module_path(&model)?, (model, frame));
        }
        // Ours against them.
        let mut drawn = BTreeSet::new();
        for part in bare.parts() {
            let path = part.model().to_str()?.to_owned();
            drawn.insert(path.clone());
            let Some((model, theirs)) = expected.get(&path) else {
                row(&path, 0.0, "0.69d0 draws no such part here");
                continue;
            };
            let (built, theirs_points) = (donor.built_points(&path)?, donor.donor_points(model)?);
            if built.len() != theirs_points.len() {
                row(&path, 0.0, "the bundle's model has other vertices than 0.69d0's");
                continue;
            }
            let (ours, theirs) = (ours(&part.at)?, theirs.then(&shift));
            let worst = built
                .iter()
                .zip(&theirs_points)
                .map(|(p, q)| {
                    let (a, b) = (ours.apply(p.map(f64::from)), theirs.apply(q.map(f64::from)));
                    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
                })
                .fold(0.0, f64::max);
            row(&path, worst, "");
        }
        for (path, _) in expected.iter().filter(|(path, _)| !drawn.contains(*path)) {
            row(path, 0.0, "0.69d0 draws it, resolve does not");
        }
        let own = donor.module_locators(&module.models_on(item).model.context("a fit without a model")?)?;
        let rules = module.attach_on(item, &fit.slot);
        let point = |id: &str| -> Option<Frame> {
            let locator = &rules?.overrides.iter().find(|o| o.id == id)?.locator;
            Some(own.get(locator)?.then(&fit_frame).then(&shift))
        };
        match (fit.kind, bare.optic) {
            (Kind::Scope, Some(optic)) => {
                row("optic", spread(&PROBE, &ours(&optic.frame)?, &fit_frame.then(&shift)), "");
                match (bare.eye, point("aim")) {
                    (Some(eye), Some(theirs)) => {
                        row("eye", spread(&PROBE, &ours(&eye.placed(EyeAnchor::Optic))?, &theirs), "");
                    }
                    _ => row("eye", 0.0, "no eye on one side"),
                }
            }
            (Kind::Scope, None) => row("optic", 0.0, "resolve gives no optic"),
            _ => {}
        }
        if fit.kind == Kind::Muzzle {
            match (bare.fire_point, point("fire_point")) {
                (Some(at), Some(theirs)) => row("fire_point", spread(&PROBE, &ours(&at)?, &theirs), ""),
                (None, None) => {}
                _ => row("fire_point", 0.0, "a fire point on one side only"),
            }
        }
        if fit.kind == Kind::Grip {
            let wrist = module.finger_locators.last().and_then(|name| own.get(name));
            match (bare.left_hand, wrist) {
                (Some(hand), Some(l)) => {
                    // As a gun rig's locator on the grip: half turn x L x half turn x grip's frame.
                    let theirs = HALF_TURN.then(l).then(&HALF_TURN).then(&fit_frame).then(&shift);
                    row("hand", spread(&PROBE, &ours(&hand.wrist)?, &theirs), "");
                }
                _ => row("hand", 0.0, "no hand on one side"),
            }
        }
    }
    Ok(())
}

/// `remodel-069d.json`'s guns need no `offset`: [`port_modules`] measures it.
pub fn refuse_typed_offsets(spec: &Spec069) -> Result<()> {
    for (item, gun) in &spec.guns {
        ensure!(
            gun.offset == [0.0; 3],
            "{item}: a typed offset {:?}; the shift is measured (fit.rs)",
            gun.offset
        );
    }
    Ok(())
}
