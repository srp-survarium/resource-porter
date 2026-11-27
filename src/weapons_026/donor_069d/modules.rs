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
use weapon_modules::{Affine, AttachEye, DictId, EyeAnchor, Loadout, Placement, Role, SlotId, Table, Wanted};

use super::catalog::{AttachDef, Catalog, ModuleDef};
use super::plan::{self, Base, GunPlan, Kind, Site};
use super::render::{self, Shape};
use super::{
    BindFrame, Body, Composed, Donor069, Gun069, Locator, Rig, Spec069, body_path, db_config, encode as encode_config,
    fit, list, locators, lod0_surfaces, socket, string_of,
};
use crate::skeleton_animation::{Clip, Rig as Skeleton};
use crate::weapon::parse_config;
use crate::weapons_026::modules::{Bounds, Mesh};
use crate::weapons_026::{field, get, s};
use crate::{ConfigValue as V, ResourceDb};

pub mod legacy;
pub use legacy::{LegacyReport, Lost, Released, read_released};

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
/// A grip's hand against 0.69d0's, metres: the hand is the IK's target, and the clamp grips on the
/// VSK-94's suppressor stand 2.1 to 2.6 mm off 0.69d0's by the crate's rule (the grip's place, not
/// its turn), which no picture shows.
pub const HAND_LIMIT: f64 = 0.003;

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
pub(super) struct Frame {
    pub(super) r: [[f64; 3]; 3],
    pub(super) t: [f64; 3],
}

const HALF_TURN: Frame = Frame::HALF_TURN;

impl Frame {
    const IDENTITY: Frame = Frame {
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };
    pub(super) const HALF_TURN: Frame = Frame {
        r: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
        t: [0.0; 3],
    };

    /// The inverse of a rigid frame (a rotation and a translation).
    pub(super) fn inverse(&self) -> Frame {
        let r: [[f64; 3]; 3] = std::array::from_fn(|row| std::array::from_fn(|c| self.r[c][row]));
        let back = Frame { r, t: [0.0; 3] };
        Frame {
            r,
            t: back.turn(self.t).map(|c| -c),
        }
    }

    /// How far apart two frames put a point 10 cm along each axis, or their origins:
    /// the largest distance, metres.
    pub(super) fn apart(&self, other: &Frame) -> f64 {
        spread(&PROBE, self, other)
    }

    /// `create_rotation(angles) x create_translation(position)`: a
    /// locator's frame in its bone's, a bone's bind frame.
    pub(super) fn of(angles: [f32; 3], position: [f32; 3]) -> Frame {
        Frame {
            r: socket::create_rotation(angles),
            t: position.map(f64::from),
        }
    }

    pub(super) fn translation(t: [f32; 3]) -> Frame {
        Frame {
            t: t.map(f64::from),
            ..Frame::IDENTITY
        }
    }

    fn turn(&self, v: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|c| (0..3).map(|r| v[r] * self.r[r][c]).sum())
    }

    pub(super) fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let p = self.turn(v);
        std::array::from_fn(|k| p[k] + self.t[k])
    }

    /// `self x next`: through `self`, then through `next`.
    pub(super) fn then(&self, next: &Frame) -> Frame {
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
    look: Option<Look>,
    laid: Vec<Laid>,
    /// Under the gun's default optic (`Gun069::optic`): the slots whose sights are folded and
    /// the slots whose part makes way. Empty for a gun without one.
    folded: BTreeSet<String>,
    taken: BTreeSet<String>,
}

/// One check: a part of a gun with something fitted, ours against 0.69d0's.
#[derive(Clone, Debug, Serialize)]
pub struct Checked {
    pub gun: String,
    /// The fitted module, or `-` for a refusal of the fit itself.
    pub fit: String,
    /// What was compared: a part's model, `eye` (`eye line`: an eye at the gun's own eye plane,
    /// its line of sight), `fire_point`, `hand`, `optic`.
    pub what: String,
    /// The largest distance between ours and 0.69d0's, mm.
    pub mm: f64,
    pub pass: bool,
    /// Why it fails, when it is not the distance.
    pub note: String,
}

/// How a remodel's whole body differs in shape from the live model it
/// replaces, once laid on it: what each has that the other lacks (vertices
/// farther than [`LOOK_LIMIT`] from the other body).
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Look {
    pub live_only: fit::Apart,
    pub remodel_only: fit::Apart,
}

/// A vertex this far (metres) from the other body is a part the other lacks.
pub const LOOK_LIMIT: f32 = 0.005;

/// A suppressor or a grip 0.69d0 links to a gun, laid on the live gun's mesh.
#[derive(Clone, Debug, Serialize)]
pub struct Laid {
    pub module: String,
    pub slot: String,
    /// The median distance from its vertices to the live mesh, mm.
    pub gap_mm: f32,
    /// The live gun has it: baked as the gun's own part (the first in its slot that lies on
    /// the mesh).
    pub built_in: bool,
}

impl Laid {
    /// It lies on the live mesh: the live gun has this part, in this paint or another.
    fn on_mesh(&self) -> bool {
        self.gap_mm <= BUILT_IN_GAP_MM
    }
}

/// A part lying this close (mm, median) on the live mesh is the live gun's own.
pub const BUILT_IN_GAP_MM: f32 = 1.5;

#[derive(Serialize)]
pub struct GunReport {
    pub model: String,
    pub bare: String,
    pub fits_like: String,
    pub laid: Vec<Laid>,
    pub fit: Option<fit::Fit>,
    pub look: Option<Look>,
    /// The bones the live rig binds otherwise than 0.69d0, each with the seat its locators took.
    pub seats: Vec<super::Seat>,
    pub plan: GunPlan,
}

#[derive(Serialize)]
pub struct Report {
    pub guns: BTreeMap<String, GunReport>,
    /// By live item: its dict id and its 0.69d0 item.
    pub items: BTreeMap<String, (u16, String)>,
    /// Modules 0.69d0 sells that have no item of ours (no row in the ids).
    pub without_item: BTreeSet<String>,
    /// Module surfaces left out (`<model>:<surface>`): a lens or a reticle with no material
    /// of ours.
    pub left_out: BTreeSet<String>,
    /// Slots 0.69d0's rules bring at a locator the module's model lacks.
    pub missing_locators: BTreeSet<String>,
    /// Suppressors by their shape, each with the suppressor whose material it draws
    /// (`Catalog::find_suppressor_shapes`): no fit, as every suppressor.
    pub suppressor_shapes: BTreeMap<String, String>,
    pub modules: usize,
    pub table_bytes: usize,
    pub checks: Vec<Checked>,
    /// By 0.69d0 module: each optic a gun takes, with its one pair of models (what its scope
    /// item's config names).
    pub optics: BTreeMap<String, OpticReport>,
    /// Where each scope's reticle is seen on each gun it goes on.
    pub centres: Vec<Centre>,
    /// The released scope pairs 0.69d0 does not give, as they stand in the table.
    pub legacy: Vec<LegacyReport>,
    /// The released scope pairs the table does not have.
    pub lost: Vec<Lost>,
    pub files: usize,
}

fn optic_report(row: &ModuleSource, module: &ModuleDef) -> Result<OpticReport> {
    Ok(OpticReport {
        dict: row.dict,
        idle_model: row.model.clone(),
        aimed_model: row.aimed.clone().unwrap_or_else(|| row.model.clone()),
        reticle: Reticle::of(module)?.map(|r| format!("{r:?}").to_lowercase()),
        group: module.group.clone(),
    })
}

/// An optic of the table: what its scope item's config holds of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpticReport {
    /// Its scope item's dict id.
    pub dict: u16,
    /// `data/idle_model` and `data/aimed_model`: its origin the frame of the slot it goes in.
    pub idle_model: String,
    pub aimed_model: String,
    /// The reticle put into a collimator's aimed model (`dot`, `ring`); none for a magnified
    /// scope, whose reticle is its aimed model's own.
    pub reticle: Option<String>,
    /// 0.69d0's group (`scope_collimator`, `scope_4x`, ...).
    pub group: String,
}

/// Where a scope's reticle is seen on a gun, aimed and at rest: by the
/// records the game is given (the scope's frame and the eye, as
/// `attachments::EYE` anchors it) through the game's own two formulas
/// (`calculate_locator`, `calculate_aim_alignment`), on the gun's bones
/// through its aimed idle clip.
#[derive(Clone, Debug, Serialize)]
pub struct Centre {
    pub gun: String,
    pub fit: String,
    /// The surface of the aimed model taken as the reticle.
    pub surface: String,
    /// The angle between the eye's line and the reticle's middle, mrad, at the clip's start,
    /// middle and end. The screen's centre is 0.
    pub off_mrad: [f64; 3],
    /// The reticle's middle along the eye's own k at the clip's start, metres, signed (which sign is in
    /// front of the camera is the game's pictures' to say).
    pub ahead: f64,
    /// The optic's own origin along the eye's k at the clip's start, metres, signed (the eye relief).
    pub eye_behind_optic: f64,
}

/// A magnified scope's reticle in its aimed model (0.69d0's own).
const SCOPE_RETICLE: &str = "scope_aimgrid_s";

impl Report {
    pub fn failures(&self) -> impl Iterator<Item = &Checked> {
        self.checks.iter().filter(|c| !c.pass)
    }
}

/// A module's model at its new path: the text the table and an item's config
/// hold. `weapons/modules/<x>` goes to `weapons/modules_069/<x>`; a module
/// 0.69d0 keeps beside its gun (`weapons/assault/famas/famas_muzzlebrake`)
/// to `weapons/modules_069/<its path under weapons/>`.
fn module_path(model: &str) -> Result<String> {
    let rest = model
        .strip_prefix("weapons/")
        .with_context(|| format!("{model}: a module's model outside weapons/"))?;
    Ok(format!("{MODULES_ROOT}/{}", rest.strip_prefix("modules/").unwrap_or(rest)))
}

/// A collimator: 0.69d0 draws its reticle as a model of its own in the optic's `reticle` slot,
/// which is the optic's `aim` locator (`possible_reticles`, the first until a player picks
/// another). So does the client (skhid-srp's patch `reticle`, `docs/notes/reticle-choice.md`): a
/// pattern model of [`RETICLE_PATTERNS`] at the optic's `aim` place in its aimed model's frame,
/// in the colour and brightness the player chose. Its aimed model has no reticle of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reticle {
    /// A red dot first (0.69d0's `ncstar_4_dot_*`).
    Dot,
    /// The EOTech's ring and dot first (`eotech_552_o_reticle`).
    Ring,
}

impl Reticle {
    /// The one 0.69d0 gives a collimator first; None for a scope without
    /// reticles of its own (a magnified scope: its reticle is in its aimed
    /// model's glass).
    fn of(module: &ModuleDef) -> Result<Option<Reticle>> {
        let Some(first) = module.reticles.first() else {
            return Ok(None);
        };
        let name = first.rsplit('/').next().unwrap_or(first);
        if name == "eotech_552_o_reticle" {
            Ok(Some(Reticle::Ring))
        } else if name.contains("_dot_") {
            Ok(Some(Reticle::Dot))
        } else {
            bail!("{}: its first reticle {first} is neither a dot nor the EOTech's ring", module.id)
        }
    }
}

/// The reticle patterns, each a model `weapons/reticles_069/<name>` made of one of 0.69d0's
/// reticle models (a quad whose texture's alpha draws the shape), at 0.69d0's size. 0.69d0's dot
/// has four sizes, four models (`dot_04` to `dot_01`); here the dot is `dot_04` and a size is
/// the client's scale on any pattern (0.69d0's dot sizes as ratios to `dot_04`: 1, 0.80, 0.60,
/// 0.40).
const RETICLE_PATTERNS: [(&str, &str); 10] = [
    ("dot", "ncstar_4_collimator/reticles/ncstar_4_dot_04_reticle"),
    ("eotech_ring", "eotech_552_collimator/reticles/eotech_552_o_reticle"),
    ("eotech_xr308", "eotech_552_collimator/reticles/eotech_552_xr308_reticle"),
    ("ncstar_cross", "ncstar_4_collimator/reticles/ncstar_4_cross_reticle"),
    ("ncstar_bullseye", "ncstar_4_collimator/reticles/ncstar_4_bullseye_reticle"),
    ("ncstar_starburst", "ncstar_4_collimator/reticles/ncstar_4_starburst_reticle"),
    ("cobra_cross", "ekp8cobra_collimator/reticles/ekp8cobra_cross_reticle"),
    ("cobra_tick", "ekp8cobra_collimator/reticles/ekp8cobra_tick_reticle"),
    ("cobra_tick_dot", "ekp8cobra_collimator/reticles/ekp8cobra_tick_with_dot_reticle"),
    ("dppro_triangle", "leupold_dppro_collimator/reticles/leupold_dppro_triangle_reticle"),
];
/// Where 0.69d0 keeps them, and the folder they go to.
const RETICLES_069: &str = "weapons/modules/collimators";
const RETICLES_ROOT: &str = "weapons/reticles_069";
/// A pattern model's one surface.
const PATTERN_SURFACE: &str = "reticle_s";
/// The patterns' one material, and its atlas (0.69d0's `collimator_reticles`).
const PATTERN_MATERIAL: &str = "weapons/reticles_069/reticle";
const PATTERN_ATLAS: &str = "weapons/reticles_069/collimator_reticles";
const ATLAS_069: &str = "weapons/modules/collimators/collimator_reticles/collimator_reticles";
/// The players' collimator emitter (patch 21), whose switches the material starts from.
const EMITTER_TEMPLATE: &str = "material_instances/weapons/loc2/collimator_glowing_dot_red.material";
/// 0.69d0's reticle's one surface.
const RETICLE_SURFACE_069: &str = "collimator_reticles_s";
/// The surface the players' collimators' aimed models had their dot in: none of ours has one now.
const RETICLE_SURFACE: &str = "collimator_glowing_dot_s";
/// What a collimator's centre is measured at, in the record: the optic's `aim` place.
const RETICLE_PLACE: &str = "aim";

/// 0.69d0's lens surfaces: the first-person one of an aimed model (`scope_lense_1p_s`, which
/// no collimator of the players' has in its aimed model) and the outside one of an idle model
/// (`scope_lense_3p_s`).
const LENS: &str = "scope_lense";
/// An idle optic's outside lens: both glasses of the tube, front and rear, in one surface.
const OUTSIDE_LENS: &str = "scope_lense_3p";
/// The material the outside lens is drawn with: 0.26g's own PSO-1 lens, which blends.
const GLASS_TEMPLATE: &str = "material_instances/weapons/pso_1_lens.material";
/// The glass's opacity in its map (of 255) and its colour (sRGB), a light cool tint.
const GLASS_OPACITY: u8 = 40;
const GLASS_TINT: [u8; 3] = [90, 110, 120];
/// Both stages' opacity is the map's times this: 88 of 255 in the glass.
const GLASS_ALPHA_SCALE: f32 = 2.2;
/// The lit stage's diffuse times the map's colour: a dark blue-violet coating, about (22, 24, 54).
const GLASS_DIFFUSE: [f32; 4] = [0.25, 0.22, 0.45, 1.0];
/// The lit stage's specular times this (the PSO-1's 0.1): sharp highlights that show at that opacity.
const GLASS_SPECULAR: f32 = 0.5;

fn slot_id(name: &str) -> Result<SlotId> {
    SlotId::from_name(name)
        .with_context(|| format!("slot {name} is none of the table's (skhid-srp's scripts/weapon-modules/slot_ids.py)"))
}

/// The gun whose links an item takes when its own and its base item's ([`Catalog::linked`])
/// fit nothing: the item of the longest proper prefix of its name (cut at `_`) that 0.69d0
/// links modules to. In 0.69d0 that is the Neostead 2, which is its own base item and has no
/// link: the pair's links are the `neostead`'s, whose base item it is.
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

/// The modules 0.69d0 links an item: its own and its base item's, else those of [`base_of`].
fn links_of<'a>(catalog: &'a Catalog, item: &str) -> Vec<&'a str> {
    let links = catalog.linked(item);
    match base_of(catalog, item) {
        Some(base) if links.is_empty() => catalog.linked(base),
        _ => links,
    }
}

impl Donor069 {
    /// A module's static model (`weapons/modules/<x>`) written at its new
    /// path, once; its materials as the guns'.
    fn module_model(&mut self, model: &str, target: &mut ResourceDb, work: &Path) -> Result<String> {
        self.module_model_at(model, module_path(model)?, false, target, work)
    }

    /// A collimator's aimed model, at the aimed model's new path (`<model>_aim` where 0.69d0
    /// aims through the optic's one model): its first-person lens left out, no reticle in it
    /// (the client draws the player's pattern, [`Reticle`]). The patterns are made with the
    /// first one ([`Self::reticle_patterns`]).
    fn collimator_aimed(&mut self, model: &str, aimed: Option<&str>, target: &mut ResourceDb, work: &Path) -> Result<String> {
        let path = match aimed {
            Some(aimed) if aimed != model => module_path(aimed)?,
            _ => format!("{}_aim", module_path(model)?),
        };
        self.reticle_patterns(target)?;
        self.module_model_at(aimed.unwrap_or(model), path, true, target, work)
    }

    /// [`RETICLE_PATTERNS`]' models, their material and its atlas, made once.
    ///
    /// - The atlas: 0.69d0's as it is (DXT5, linear), its alpha the shapes, its colour grey in
    ///   them and black elsewhere.
    /// - The material: the players' emitter's forward stage (forward_default) with the atlas as
    ///   its emissive and its transparency map, added (`blend_mode` 2), both sides, no fade by
    ///   the view's angle, and no light of its own: `constant_emissive_multiplier` 0. The client
    ///   appends this material to the pattern's model and sets the appended copy's emission
    ///   (`solid_color_specular`: the colour times the brightness) each time the player's choice
    ///   changes; the model's own surface adds black.
    /// - A model: 0.69d0's quad as it is, in its x-y plane, its sight line its z: the client
    ///   draws it at the optic's `aim` place in the eye's axes, as 0.69d0 draws it in its
    ///   `reticle` slot before its eye. Its origin is the aiming point as 0.69d0's (a dot's or a
    ///   ring's middle, a tick's or a chevron's tip).
    fn reticle_patterns(&mut self, target: &mut ResourceDb) -> Result<()> {
        let material_path = format!("material_instances/{PATTERN_MATERIAL}.material");
        if self.files.contains_key(&material_path) {
            return Ok(());
        }
        for path in [material_path.clone(), format!("textures/{PATTERN_ATLAS}.dds")] {
            ensure!(!target.contains(&path), "{path}: the target has it; a reticle's path must be new");
        }
        let atlas = self.db.read(&format!("textures/{ATLAS_069}.dds"))?;
        self.put(format!("textures/{PATTERN_ATLAS}.dds"), crate::duty_026::texture_dds(&atlas, PATTERN_ATLAS, false)?)?;
        let options = self.db.read(&format!("textures/{ATLAS_069}.options"))?;
        self.put(format!("textures/{PATTERN_ATLAS}.options"), options)?;
        let mut material = parse_config(target, EMITTER_TEMPLATE, "the players' collimator emitter")?.root;
        for (key, value) in [
            ("forward.two_sided", V::Boolean(true)),
            ("forward.effect.use_angle_depended_attenuation", V::Boolean(false)),
            ("forward.effect.use_temissive", V::Boolean(true)),
            ("forward.effect.texture_emissive", s(PATTERN_ATLAS)),
            ("forward.effect.use_ttransparency", V::Boolean(true)),
            ("forward.effect.texture_transparency", s(PATTERN_ATLAS)),
            ("forward.effect.blend_mode", V::Integer(2)),
            ("forward.effect.constant_transparency", V::Float(1.0)),
            ("forward.effect.constant_emissive", V::Float4([1.0, 1.0, 1.0, 1.0])),
            ("forward.effect.constant_emissive_multiplier", V::Float(0.0)),
        ] {
            let key = format!("material.{key}.value");
            ensure!(get(&material, &key).is_some(), "{EMITTER_TEMPLATE}: no {key}");
            material.set_path(&key, value)?;
        }
        self.put(material_path, encode_config(material)?)?;
        for (name, from) in RETICLE_PATTERNS {
            let root = format!("models/{RETICLES_ROOT}/{name}.model");
            ensure!(!target.contains(&format!("{root}/settings")), "{root}: the target has it");
            let source = format!("models/{RETICLES_069}/{from}.model");
            let settings = db_config(&mut self.db, &format!("{source}/settings"))?;
            let surfaces = lod0_surfaces(&settings, &source)?;
            ensure!(
                surfaces.len() == 1 && surfaces[0].0 == RETICLE_SURFACE_069,
                "{source}: not one {RETICLE_SURFACE_069} surface"
            );
            let bytes = self.db.read(&format!("{source}/render/{RETICLE_SURFACE_069}/converted_model"))?;
            let mesh = Mesh::parse(&super::convert::static_069_to_026(&bytes)?)?;
            ensure!(mesh.positions.iter().all(|p| p[2].abs() < 1e-6), "{source}: not a quad in its x-y plane");
            let surfaces = [(PATTERN_SURFACE.to_owned(), PATTERN_MATERIAL.to_owned(), mesh.mapped(|p| p))];
            self.put_static(&root, &format!("Survarium 0.69d0: {source}"), &surfaces, target)?;
        }
        Ok(())
    }
    /// `model` written at `path`. `collimator`: an aimed collimator, its first-person lens left
    /// out.
    fn module_model_at(
        &mut self,
        model: &str,
        path: String,
        collimator: bool,
        target: &mut ResourceDb,
        work: &Path,
    ) -> Result<String> {
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
            if collimator && surface.starts_with(LENS) {
                continue;
            }
            let bytes = self.db.read(&format!("{source}/render/{surface}/converted_model"))?;
            let mesh = super::convert::static_069_to_026(&bytes).with_context(|| format!("{source}: {surface}"))?;
            // A material this port makes is a weapon's: an opaque stage whose maps are the
            // archive's. A lens, a reticle or a dot is none (no opaque stage, or a map of
            // another kind of thing): the players' own material of its name (patch 19's 0.51c5
            // modules: `weapons/loc2/<name>`) where they have one, else the surface is left
            // out and recorded (an optic's glass is its scope item's: the scopes' lane's).
            // A lens is never remade: 0.69d0's outside lens is an opaque stage with all its maps,
            // and on the AK-12's material it kept that template's Fresnel map, which drew the
            // AK-12's parts in the glass. The outside lens is see-through glass of our own
            // (outside_glass), the first-person one the players' own.
            if surface.starts_with(OUTSIDE_LENS) {
                let glass = self.outside_glass(target, work)?;
                surfaces.push((surface, glass, mesh));
                continue;
            }
            let donor_material = db_config(&mut self.db, &format!("material_instances/{material}.material"))?;
            let makes = !surface.starts_with(LENS)
                && get(&donor_material, "material.g_stage.effect").is_some_and(|effect| {
                    ["t_albedo", "t_normal", "t_smoothness", "t_metalness"].iter().all(|role| {
                        // (A map switched off and not in the archive is none: compose_material.)
                        let off = get(effect, &format!("use_{role}.value")) == Some(&V::Boolean(false));
                        match string_of(get(effect, &format!("{role}.value"))) {
                            Ok(texture) if !texture.is_empty() => {
                                self.db.contains(&format!("textures/{texture}.dds"))
                                    || (off && *role != "t_albedo")
                            }
                            _ => true,
                        }
                    })
                });
            let material = if makes {
                self.compose_material(&material, target, work)
                    .with_context(|| format!("material {material}"))?
            } else {
                let name = material.rsplit('/').next().unwrap_or(&material);
                let ours = format!("weapons/loc2/{name}");
                if !target.contains(&format!("material_instances/{ours}.material")) {
                    self.left_out.insert(format!("{model}:{surface}"));
                    continue;
                }
                ours
            };
            surfaces.push((surface, material, mesh));
        }
        ensure!(!surfaces.is_empty(), "{source}: no surface with a material 0.26g can draw");
        self.put_static(&root, &format!("Survarium 0.69d0: {source}"), &surfaces, target)?;
        Ok(path)
    }

    /// The idle optics' outside lens, see-through: the stock PSO-1's lens material (a forward
    /// lit stage whose highlights stay, and a blended one whose opacity is its transparency
    /// map's alpha) on patch 19's lens maps. Its texture is that lens's own (alpha 0 in the
    /// glass, 255 on the rim, the 0.69d0 lens's UVs) with GLASS_OPACITY in the glass and
    /// GLASS_TINT there under the rim's colour. Made once; returns the material's name.
    fn outside_glass(&mut self, target: &mut ResourceDb, work: &Path) -> Result<String> {
        const MATERIAL: &str = "weapons/modules_069/scope_lense_3p_glass";
        const TEXTURE: &str = "weapons/modules_069/scope_lense_3p_glass";
        const MAPS: &str = "weapons/attach/scope/scope_lense/scope_lense_1p";
        let path = format!("material_instances/{MATERIAL}.material");
        if self.files.contains_key(&path) {
            return Ok(MATERIAL.to_owned());
        }
        let (w, h, lens) = super::convert::decode_dds(&target.read(&format!("textures/{MAPS}.dds"))?)?;
        let px: Vec<[u8; 4]> = lens
            .iter()
            .map(|p| {
                let rim = u32::from(p[3]);
                let mix = |glass: u8, edge: u8| {
                    ((u32::from(glass) * (255 - rim) + u32::from(edge) * rim) / 255) as u8
                };
                [
                    mix(GLASS_TINT[0], p[0]),
                    mix(GLASS_TINT[1], p[1]),
                    mix(GLASS_TINT[2], p[2]),
                    mix(GLASS_OPACITY, 255),
                ]
            })
            .collect();
        fs::create_dir_all(work)?;
        let tga = work.join("scope_lense_3p_glass.tga");
        let dds = work.join("scope_lense_3p_glass.dds");
        fs::write(&tga, super::convert::tga_rgba(w, h, &px)?)?;
        crate::armor::convert_texture(&tga, &dds, crate::manifest::TextureCompression::Dxt5)?;
        let bytes = crate::duty_026::texture_dds(&fs::read(&dds)?, TEXTURE, true)?;
        fs::remove_file(&tga)?;
        fs::remove_file(&dds)?;
        ensure!(!target.contains(&format!("textures/{TEXTURE}.dds")), "{TEXTURE}: the target has it");
        self.put(format!("textures/{TEXTURE}.dds"), bytes)?;
        self.put(
            format!("textures/{TEXTURE}.options"),
            target.read("textures/weapons/attach/sight_11mm/pso_1/pso_1.options")?,
        )?;
        let mut material = parse_config(target, GLASS_TEMPLATE, "the stock PSO-1's lens")?.root;
        for (key, texture) in [
            ("lighting.effect.texture_diffuse", TEXTURE.to_owned()),
            ("lighting.effect.texture_normal", format!("{MAPS}_nmap")),
            ("lighting.effect.texture_roughness", format!("{MAPS}_smap")),
            ("forward.effect.texture_transparency", TEXTURE.to_owned()),
            ("lighting.effect.texture_transparency", TEXTURE.to_owned()),
        ] {
            material.set_path(&format!("material.{key}.value"), s(&texture))?;
        }
        // The lit stage blends by its own alpha, `constant_transparency` alone unless it reads the
        // map too: the PSO-1's 1 drew the lit glass over everything behind it (the owner's milky
        // disc, 2026-11-23). With the map (stock `weapons/pasted_glass`'s switches) the glass is
        // GLASS_ALPHA_SCALE x the map's alpha in both stages, nearly no diffuse, and the light's
        // highlights scaled up so they still show at that opacity.
        for (key, value) in [
            ("lighting.effect.use_ttransparency", V::Boolean(true)),
            ("lighting.effect.constant_transparency", V::Float(GLASS_ALPHA_SCALE)),
            ("forward.effect.constant_transparency", V::Float(GLASS_ALPHA_SCALE)),
            ("lighting.effect.constant_diffuse", V::Float4(GLASS_DIFFUSE)),
            ("lighting.effect.constant_specular_color_multiplier", V::Float3([GLASS_SPECULAR; 3])),
        ] {
            material.set_path(&format!("material.{key}.value"), value)?;
        }
        self.put(path, encode_config(material)?)?;
        Ok(MATERIAL.to_owned())
    }

    /// A static model of `surfaces` (each its name, its material and its
    /// 0.26g mesh) at `root`, on the native static model's settings.
    fn put_static(
        &mut self,
        root: &str,
        source: &str,
        surfaces: &[(String, String, Vec<u8>)],
        target: &mut ResourceDb,
    ) -> Result<()> {
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
        for (name, _, mesh) in surfaces {
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
            field("src_filename", s(source)),
            field("type", V::Integer(1)),
            field("version", V::Integer(0)),
            field("bounding_box", box_value(bounds(&all))),
        ]);
        self.put(format!("{root}/render/export_properties"), encode_config(props)?)?;
        Ok(())
    }

    /// A module model's locators, by name (0.69d0's, in its model's space).
    fn module_locators(&mut self, model: &str) -> Result<BTreeMap<String, Frame>> {
        let props = db_config(&mut self.db, &format!("models/{model}.model/render/export_properties"))?;
        Ok(locators(&props)?
            .into_iter()
            .map(|l| (l.name.clone(), locator_frame(&l)))
            .collect())
    }

    /// A 0.69d0 module model's LOD0 meshes, as its files have them.
    pub(super) fn donor_meshes(&mut self, model: &str) -> Result<Vec<Mesh>> {
        let source = format!("models/{model}.model");
        let settings = db_config(&mut self.db, &format!("{source}/settings"))?;
        // Of a model the bundle has, the surfaces the bundle has (a lens may be left out).
        let built = module_path(model).ok().map(|p| format!("models/{p}.model"));
        let built = built.filter(|root| self.files.contains_key(&format!("{root}/settings")));
        let mut out = vec![];
        for (surface, _) in lod0_surfaces(&settings, &source)? {
            if built
                .as_ref()
                .is_some_and(|root| !self.files.contains_key(&format!("{root}/render/{surface}/converted_model")))
            {
                continue;
            }
            let bytes = self.db.read(&format!("{source}/render/{surface}/converted_model"))?;
            out.push(Mesh::parse(&super::convert::static_069_to_026(&bytes)?)?);
        }
        Ok(out)
    }

    /// A model of the bundle (`models/<path><kind>`): its LOD0 meshes, what
    /// the game would draw.
    fn built_meshes(&self, path: &str, kind: &str) -> Result<Vec<Mesh>> {
        let root = format!("models/{path}{kind}");
        let settings = crate::BinaryConfig::parse(
            self.files
                .get(&format!("{root}/settings"))
                .with_context(|| format!("{root} is not in the bundle"))?,
        )?
        .root;
        let mut out = vec![];
        for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
            let surface = string_of(Some(surface))?;
            out.push(Mesh::parse(&self.files[&format!("{root}/render/{surface}/converted_model")])?);
        }
        Ok(out)
    }

    /// 0.69d0's own base model of a gun: its LOD0 surfaces (mesh space, its
    /// own bind pose), no module on it.
    fn donor_body(&mut self, item: &str) -> Result<Vec<(Vec<[f32; 3]>, Vec<[u16; 3]>)>> {
        let root = format!(
            "models/{}.skinned_model",
            string_of(get(self.weapon(item)?, "object.model"))?
        );
        let bones = super::bind_pose(&self.db.read(&format!("{root}/render/bind_pose"))?)?.len();
        let settings = db_config(&mut self.db, &format!("{root}/settings"))?;
        let mut out = vec![];
        for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
            let surface = string_of(Some(surface))?;
            let bytes = self.db.read(&format!("{root}/render/{surface}/converted_model"))?;
            let mesh = super::convert::skinned_069_to_026(&bytes, bones)?;
            let points = (0..mesh.vertices.len()).map(|i| mesh.position(i)).collect();
            let triangles = mesh.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
            out.push((points, triangles));
        }
        Ok(out)
    }
}

/// A live model's LOD0 vertices (mesh space, bind pose).
pub(super) fn live_points(target: &mut ResourceDb, model: &str) -> Result<Vec<[f32; 3]>> {
    let settings = parse_config(target, &format!("{model}/settings"), "live model settings")?.root;
    let mut out = vec![];
    for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
        let surface = string_of(Some(surface))?;
        let bytes = target.read(&format!("{model}/render/{surface}/converted_model"))?;
        out.extend(Mesh::parse(&bytes)?.positions);
    }
    Ok(out)
}

/// The suppressors and grips 0.69d0 links to a gun that go in a free slot of its own, each
/// laid on the live gun's mesh: one that lies on it is the live gun's own (the first such in
/// its slot is taken; another there is the same part in another paint, the SR-3M premium's
/// two foregrips). A gun 0.26g draws with a suppressor or a foregrip of its own keeps it so
/// in both bodies, with its live hand locators, and what hangs on it has its slot.
fn built_in(
    donor: &mut Donor069,
    catalog: &Catalog,
    item: &str,
    gun: &Gun069,
    measured: &Composed,
    live: &[[f32; 3]],
) -> Result<Vec<Laid>> {
    let own = gun.own(catalog.gun(item)?);
    let standing = plan::plan(catalog, item, &own, &[], &|_| false)?;
    let shift = Frame::translation(gun.offset);
    let mut out: Vec<Laid> = vec![];
    for id in links_of(catalog, item) {
        let module = catalog.module(id)?;
        let candidate = module.is_suppressor() || plan::kind(module) == Some(Kind::Grip);
        if !candidate || own.iter().any(|(m, _)| m == id) {
            continue;
        }
        let Some(model) = module.models_on(item).model else {
            continue;
        };
        for attach in module.attaches_on(item) {
            let Some(site) = standing.slots.get(&attach.slot) else {
                continue;
            };
            if site.base != Base::Gun || standing.parts.iter().any(|p| p.slot == attach.slot) {
                continue;
            }
            let Some(l) = measured.donor_locators.iter().find(|l| l.name == site.locator) else {
                continue;
            };
            let Some((_, bone)) = measured.donor_bind.iter().find(|(n, _)| *n == l.bone) else {
                continue;
            };
            let frame = HALF_TURN.then(&locator_frame(l)).then(&bind_frame(bone)).then(&shift);
            let points: Vec<[f32; 3]> = donor
                .donor_meshes(&model)?
                .iter()
                .flat_map(|m| m.positions.iter().map(|p| frame.apply(p.map(f64::from)).map(|c| c as f32)))
                .collect();
            let mut laid = Laid {
                module: id.to_owned(),
                slot: attach.slot.clone(),
                gap_mm: fit::median_gap_mm(&points, live),
                built_in: false,
            };
            laid.built_in = laid.on_mesh() && !out.iter().any(|l| l.built_in && l.slot == laid.slot);
            out.push(laid);
            break;
        }
    }
    Ok(out)
}

/// A skinned model's LOD0 surfaces as shapes, each in the mean colour of its
/// diffuse texture: from `files` (a bundle's) where they have a path, else
/// from the target.
fn look_shapes(target: &mut ResourceDb, files: Option<&BTreeMap<String, Vec<u8>>>, root: &str) -> Result<Vec<Shape>> {
    let mut read = |path: &str| -> Result<Vec<u8>> {
        match files.and_then(|f| f.get(path)) {
            Some(bytes) => Ok(bytes.clone()),
            None => target.read(path),
        }
    };
    let settings = crate::BinaryConfig::parse(&read(&format!("{root}/settings"))?)?.root;
    let mut out = vec![];
    for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
        let surface = string_of(Some(surface))?;
        let mesh = Mesh::parse(&read(&format!("{root}/render/{surface}/converted_model"))?)?;
        let material = string_of(get(&settings, &format!("material_settings.{surface}.material_name")))?;
        let material = crate::BinaryConfig::parse(&read(&format!("material_instances/{material}.material"))?)?.root;
        // A surface without a readable diffuse texture is drawn grey.
        let colour = string_of(get(&material, "material.g_stage.effect.texture_diffuse.value"))
            .ok()
            .and_then(|texture| read(&format!("textures/{texture}.dds")).ok())
            .and_then(|dds| super::convert::decode_dds(&dds).ok())
            .map_or(render::BODY, |(_, _, pixels)| {
                let step = (pixels.len() / 4096).max(1);
                let (mut sum, mut n) = ([0u64; 3], 0u64);
                for p in pixels.iter().step_by(step) {
                    for k in 0..3 {
                        sum[k] += u64::from(p[k]);
                    }
                    n += 1;
                }
                // Lifted towards grey, so the shading shows on a black gun.
                sum.map(|c| (64 + (c / n.max(1)) * 3 / 4) as u8)
            });
        out.push(Shape {
            points: mesh.positions,
            triangles: mesh.triangles,
            colour,
        });
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
    ids: &'a BTreeMap<String, u16>,
    /// Slots a module's rules bring at a locator its model lacks
    /// (`<module>: <slot> at <locator>`): 0.69d0 cannot place anything there either.
    missing: std::cell::RefCell<BTreeSet<String>>,
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
            let attach = self.attach(&module.id, rules, &locator, role)?;
            // An optic in a slot where it moves no eye is no optic there (the table refuses it).
            if role == Role::Fitted(weapon_modules::Kind::Scope) && attach.eye.is_none() {
                continue;
            }
            attaches.push(attach);
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
        let aimed = match (role, Reticle::of(module)?) {
            (Role::Fitted(weapon_modules::Kind::Scope), Some(_)) => {
                // One eye an optic: the client draws the reticle at it, in its aimed model's frame.
                let mut eyes = module.attaches_on(item).iter().flat_map(|a| &a.overrides).filter(|p| p.id == "aim");
                let eye = eyes.next().with_context(|| format!("{}: a collimator without an eye", module.id))?;
                ensure!(
                    eyes.all(|other| other.locator == eye.locator),
                    "{}: its slots give it more than one eye",
                    module.id
                );
                Some(donor.collimator_aimed(&model, models.aimed.as_deref(), target, work)?)
            }
            (Role::Fitted(weapon_modules::Kind::Scope), None) => path(&models.aimed, donor, target)?,
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

    fn attach(
        &self,
        module: &str,
        rules: &AttachDef,
        locator: &dyn Fn(&str) -> Result<Frame>,
        role: Role,
    ) -> Result<AttachSource> {
        // A module's own locator `L` is the table's `half turn x L`.
        let own = |name: &str| -> Result<Affine> { Ok(HALF_TURN.then(&locator(name)?).affine()) };
        let mut out = AttachSource::new(slot_id(&rules.slot)?);
        for add in &rules.adds {
            match own(&add.locator) {
                Ok(at) => out.adds.push((slot_id(&add.id)?, at)),
                Err(_) => {
                    let note = format!("{module}: {} at {}", add.id, add.locator);
                    self.missing.borrow_mut().insert(note);
                }
            }
        }
        for point in &rules.overrides {
            match point.id.as_str() {
                "aim" => {
                    let at = own(&point.locator)?;
                    out.eye = Some(match point.plane.as_deref() {
                        None => AttachEye::Own(at),
                        Some("body_aim") => AttachEye::InGunPlane(at),
                        Some(other) => bail!("{module}: aim keeps the plane of {other}, a slot the table has no rule for"),
                    });
                }
                "fire_point" => out.fire_point = Some(own(&point.locator)?),
                // The charging handle and the second aim points: 0.26g has one zoom and one eye.
                _ => {}
            }
        }
        // The gun's own points stand until a fit moves them: a remodel keeps the live model's
        // `aim` and `barrel_point` (its sights and flash are where they are today, whichever
        // body is drawn), so an own part moves neither, and only an optic moves the eye (a
        // rail mount's is where the bare rail would aim).
        if !matches!(role, Role::Fitted(weapon_modules::Kind::Scope)) {
            out.eye = None;
        }
        if role == Role::Stock {
            out.fire_point = None;
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
        Ok(out)
    }
}

/// The gun's bones' matrices (the model's space) through its aimed idle
/// clip, its config's `states/aimed_idle`, else `states/idle`'s: at the
/// clip's start (the table's pose), its middle and its end.
fn aimed_poses(target: &mut ResourceDb, config: &V, bind: &[u8]) -> Result<[Vec<Frame>; 3]> {
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
    let skeleton_bytes = target.read(&skeleton)?;
    let rig = Skeleton::parse(bind, &skeleton_bytes)
        .or_else(|_| Skeleton::parse_by_root(bind, &skeleton_bytes))
        .with_context(|| skeleton.clone())?;
    let clip = Clip::parse(
        &target.read(&format!("{clip}.b-spline"))?,
        &target.read(&format!("{clip}.bones_names"))?,
    )
    .with_context(|| clip.to_owned())?;
    let length = clip.duration();
    Ok([0.0, length / 2.0, length].map(|time| {
        rig.pose(&clip, time)
            .into_iter()
            .map(|(basis, at)| Frame {
                r: basis.map(|row| row.map(f64::from)),
                t: at.map(f64::from),
            })
            .collect()
    }))
}

/// A gun while it is aimed and at rest: its bones through the aimed idle clip.
struct Aimed {
    poses: [Vec<Frame>; 3],
}

/// Builds the bundle. `only`: the 0.69d0 items to build (all when empty).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn port_modules(
    id: &str,
    spec: &Spec069,
    items: &[LiveItem],
    ids: &BTreeMap<String, u16>,
    only: &[String],
    released: &[Released],
    renders: Option<&Path>,
    donor_db: &Path,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<Report> {
    ensure!(!output.exists(), "{} is already there", output.display());
    let work = output.with_extension("work");
    let mut donor = Donor069::empty(donor_db, spec)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut catalog = Catalog::read(&donor.game)?;
    catalog.find_suppressor_shapes(&mut |model| {
        let source = format!("models/{model}.model");
        if !donor.db.contains(&format!("{source}/settings")) {
            return Ok(None);
        }
        let settings = db_config(&mut donor.db, &format!("{source}/settings"))?;
        Ok(Some(lod0_surfaces(&settings, &source)?.into_iter().map(|(_, material)| material).collect()))
    })?;
    let catalog = catalog;
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
    let mut legacy: Vec<LegacyReport> = vec![];
    let mut lost: Vec<Lost> = vec![];
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
        let mut gun = gun.clone();
        let (mut measured, mut look, mut laid) = (None, None, vec![]);
        if let Rig::Live(live) = &gun.rig {
            gun.offset = [0.0; 3];
            let ours = donor
                .compose_model(item, &gun, &Body::Measure, &mut target, &work)
                .with_context(|| format!("{item}: measuring"))?;
            let theirs = live_points(&mut target, live)?;
            let fit = fit::fit(&theirs, &ours.points);
            gun.offset = fit.shift;
            laid = built_in(&mut donor, &catalog, item, &gun, &ours, &theirs).with_context(|| format!("{item}: built-in parts"))?;
            let mut points = ours.points;
            for found in laid.iter().filter(|l| l.built_in) {
                // (A premium's bundled extra the sidecar left to the kit is the gun's own after all.)
                gun.separable.retain(|m| *m != found.module);
                gun.fixed.push((found.module.clone(), found.slot.clone()));
            }
            if !gun.fixed.is_empty() {
                gun.offset = [0.0; 3];
                points = donor.compose_model(item, &gun, &Body::Measure, &mut target, &work)?.points;
                gun.offset = fit.shift;
            }
            let laid: Vec<[f32; 3]> = points.iter().map(|p| std::array::from_fn(|k| p[k] + fit.shift[k])).collect();
            look = Some(Look {
                live_only: fit::apart(&theirs, &laid, LOOK_LIMIT),
                remodel_only: fit::apart(&laid, &theirs, LOOK_LIMIT),
            });
            measured = Some(fit);
        }
        let own = gun.own(catalog.gun(item)?);
        // A part the live gun has built in has no item on it, in any paint.
        let offered = |m: &ModuleDef| ids.contains_key(&m.id) && !laid.iter().any(|l| l.module == m.id && l.on_mesh());
        let mut fits_like = item.clone();
        let mut plan = plan::plan(&catalog, item, &own, &catalog.linked(item), &offered)?;
        if let Some(base) = &catalog.gun(item)?.base {
            // A skin whose own links add no fit and no carrier takes its base item's fits as
            // they are (the table's `fits_of`).
            let of_base = |module: &String| catalog.links.get(base).is_some_and(|links| links.contains(module));
            let carried = plan.carriers.values().flatten().map(|link| &link.module);
            if !plan.fits.is_empty() && plan.fits.iter().map(|fit| &fit.module).chain(carried).all(of_base) {
                fits_like = base.clone();
            }
        }
        if plan.fits.is_empty()
            && let Some(base) = base_of(&catalog, item)
        {
            fits_like = base.to_owned();
            plan = plan::plan(&catalog, item, &own, &catalog.linked(base), &offered)?;
        }
        // The released pairs its own links do not give and its base gun's do (legacy.rs, 1).
        if let Some(base) = base_of(&catalog, item).filter(|base| *base != fits_like) {
            let lives: Vec<&str> = items.iter().filter(|i| i.donor_item == *item).map(|i| i.name.as_str()).collect();
            let like = plan::plan(&catalog, item, &own, &catalog.linked(base), &offered)?;
            for pair in released.iter().filter(|r| lives.contains(&r.weapon.as_str())) {
                let Some((module, _)) = ids.iter().find(|(_, id)| **id == pair.scope) else {
                    continue;
                };
                if plan.fits.iter().any(|f| f.module == *module) {
                    continue;
                }
                let Some(fit) = like.fits.iter().find(|f| f.module == *module && f.kind == Kind::Scope) else {
                    continue;
                };
                // Its carriers come with it, unless the item has others for that slot.
                if !fit.chain.is_empty() {
                    match (plan.carriers.get(&fit.slot), like.carriers.get(&fit.slot)) {
                        (Some(have), Some(theirs)) if have == theirs => {}
                        (None, Some(theirs)) => {
                            plan.carriers.insert(fit.slot.clone(), theirs.clone());
                        }
                        _ => continue,
                    }
                }
                plan.fits.push(fit.clone());
                legacy.push(LegacyReport {
                    gun: pair.weapon.clone(),
                    module: module.clone(),
                    how: format!("base gun {base}"),
                    slot: fit.slot.clone(),
                    found: String::new(),
                    residual_mm: 0.0,
                    moved: [0.0; 3],
                    rails: vec![],
                    rail_slot: String::new(),
                    optic_mm: 0.0,
                });
            }
        }
        // The released pairs no link gives whose optic names a slot the gun itself has free (the
        // PSO-1 on the Saiga-12's dovetail): in that slot by 0.69d0's rules (legacy.rs, 1b).
        let lives: Vec<&str> = items.iter().filter(|i| i.donor_item == *item).map(|i| i.name.as_str()).collect();
        for pair in released.iter().filter(|r| lives.contains(&r.weapon.as_str())) {
            let Some((module, _)) = ids.iter().find(|(_, id)| **id == pair.scope) else {
                continue;
            };
            if plan.fits.iter().any(|f| f.module == *module) {
                continue;
            }
            let def = catalog.module(module)?;
            if plan::kind(def) != Some(Kind::Scope) {
                continue;
            }
            let free = def
                .attaches_on(item)
                .iter()
                .map(|a| &a.slot)
                .find(|slot| plan.slots.contains_key(*slot) && !plan.parts.iter().any(|p| p.slot == **slot))
                .cloned();
            let Some(slot) = free else {
                continue;
            };
            plan.fits.push(plan::Fit {
                module: module.clone(),
                kind: Kind::Scope,
                slot: slot.clone(),
                chain: vec![],
            });
            legacy.push(LegacyReport {
                gun: pair.weapon.clone(),
                module: module.clone(),
                how: format!("own slot {slot}"),
                slot,
                found: String::new(),
                residual_mm: 0.0,
                moved: [0.0; 3],
                rails: vec![],
                rail_slot: String::new(),
                optic_mm: 0.0,
            });
        }
        let (folded, taken) = if gun.optic { donor.optic_slots()? } else { Default::default() };
        for module in links_of(&catalog, item) {
            let module = catalog.module(module)?;
            let sold = plan::kind(module).is_some_and(|k| k != Kind::Scope) && catalog.has_stats(module);
            if sold && !ids.contains_key(&module.id) {
                without_item.insert(module.id.clone());
            }
        }
        let composed = donor
            .compose_model(item, &gun, &Body::Whole, &mut target, &work)
            .with_context(|| format!("{item}: whole body"))?;
        let leave_out: BTreeSet<(String, String)> = plan
            .stock()
            .map(|(_, p)| (p.module.clone(), p.site.locator.clone()))
            .collect();
        for (module, locator) in &leave_out {
            // A gun drawn under its default optic has no sight in the slot the optic takes.
            ensure!(
                gun.optic || donor.modules[item].iter().any(|m| m.baked && m.id == *module && m.locator == *locator),
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
            look,
            laid,
            folded,
            taken,
        });
        if let (Some(dir), Rig::Live(live)) = (renders, &spec.guns[item].rig) {
            // Today's gun and its remodel, each surface in its diffuse texture's mean colour.
            let left = look_shapes(&mut target, None, live)?;
            let right = look_shapes(&mut target, Some(&donor.files), &format!("models/{path}.skinned_model"))?;
            let dest = dir.join(item);
            fs::create_dir_all(&dest)?;
            fs::write(dest.join("_look.png"), render::pair(&left, &right, 640)?)?;
        }
    }

    // The table's rows.
    let maker = ModuleMaker {
        ids,
        missing: Default::default(),
    };
    let mut rows = Rows::default();
    let mut source = Source::default();
    let mut report_items = BTreeMap::new();
    // By gun: its table index, and each fit's and own part's module row.
    let mut gun_at: BTreeMap<String, usize> = BTreeMap::new();
    let mut aimed: BTreeMap<String, Aimed> = BTreeMap::new();
    let mut optics: BTreeMap<String, OpticReport> = BTreeMap::new();
    // By gun of the table: its legacy rows; and each placed one, to check through the table.
    let mut gun_legacy: Vec<BTreeSet<usize>> = vec![];
    let mut placed_legacy: Vec<(usize, u16, u16, Placement)> = vec![];
    for live in items {
        let Some(b) = built.iter().find(|b| b.item == live.donor_item) else {
            continue;
        };
        let (dict, config_path) = dict_row(&live.name)?;
        let config = parse_config(&mut target, &config_path, "weapon config")?.root;
        let item = &b.item;
        let model_path = b.gun.model_path.as_deref().unwrap_or_default();
        let ship_bind_bytes = donor.files[&format!("models/{model_path}.skinned_model/render/bind_pose")].clone();
        let poses = aimed_poses(&mut target, &config, &ship_bind_bytes).with_context(|| format!("{}: aimed pose", live.name))?;
        let pose = &poses[0];
        let find = |name: &str| -> Result<&Locator> {
            b.composed
                .slots
                .iter()
                .find(|l| l.name == name)
                .with_context(|| format!("{item}: 0.69d0's model has no locator {name}"))
        };
        // The game's own eye: the shipped model's `aim` (the live model's on a remodel).
        let aim = b
            .composed
            .locators
            .iter()
            .find(|l| l.name == "aim")
            .with_context(|| format!("{item}: its model has no aim"))?;
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
            aim_bone: u16::try_from(aim.bone_idx)?,
            // the plane an open sight's eye keeps (0.69d0's `body_aim`): the game's own eye
            body_aim: (u16::try_from(aim.bone_idx)?, locator_frame(aim).affine()),
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
            // Under the gun's default optic, which the game draws itself in both bodies, a
            // part in a slot an optic takes is not there, and a folded sight's shape is its own.
            if b.taken.contains(&part.slot) {
                continue;
            }
            let module = catalog.module(&part.module)?;
            let mut row = maker
                .source(&mut donor, &mut target, &work, module, Role::Stock, item, Some(&[part.slot.as_str()]))
                .with_context(|| format!("{item}: own part {}", part.module))?;
            if b.folded.contains(&part.slot)
                && let Some(folded) = row.alt.take()
            {
                row.model = folded;
            }
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
                if fit.kind == Kind::Scope {
                    let optic = optic_report(&row, module)?;
                    if let Some(before) = optics.insert(fit.module.clone(), optic.clone()) {
                        // One pair of models an optic: its scope item's config names them once.
                        ensure!(before == optic, "{}: other models on {item} than on a gun before: {before:?}, {optic:?}", fit.module);
                    }
                }
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
        // The released pairs the table still lacks: where their variants put them (legacy.rs, 2).
        // (A base gun's legacy rows are placed on that gun's own record: an item that takes its
        // fits places its own, or does not take those.)
        let fitted: Vec<usize> = match gun.fits_of {
            Some(base) => source.guns[base].fits.iter().filter(|row| !gun_legacy[base].contains(row)).copied().collect(),
            None => gun.fits.clone(),
        };
        let have: BTreeSet<u16> = fitted.iter().map(|&row| rows.modules[row].dict).collect();
        let named: Vec<weapon_modules::SlotId> =
            fitted.iter().flat_map(|&row| rows.modules[row].attaches.iter().map(|a| a.slot)).collect();
        let mut own_legacy = BTreeSet::new();
        for pair in released.iter().filter(|r| r.weapon == live.name && !have.contains(&r.scope)) {
            let placed = (|| -> Result<legacy::Placed> {
                let (module, _) = ids
                    .iter()
                    .find(|(_, id)| **id == pair.scope)
                    .with_context(|| format!("no module of the ids has the dict id {}", pair.scope))?;
                let module = catalog.module(module)?;
                ensure!(plan::kind(module) == Some(Kind::Scope), "{} is no optic of 0.69d0's", module.id);
                legacy::place(pair, module, b, &live.name, &config, &mut donor, &mut target, &work, &maker, &mut rows, &mut gun, &named)
            })();
            match placed {
                Ok(placed) => {
                    // Its slot (or its rail's seat) hangs on `scope_point`'s bone: the table keeps
                    // the aimed idle pose of every bone a slot hangs on.
                    let bone = placed.at.bone.0;
                    if !gun.pose.iter().any(|(b, _)| *b == bone) {
                        let frame = pose.get(usize::from(bone)).with_context(|| format!("{item}: bone {bone} has no pose"))?;
                        gun.pose.push((bone, frame.affine()));
                        gun.pose.sort_by_key(|(b, _)| *b);
                    }
                    let module = catalog.module(&placed.report.module)?;
                    let optic = optic_report(&rows.modules[placed.row], module)?;
                    if let Some(before) = optics.insert(module.id.clone(), optic.clone()) {
                        ensure!(before == optic, "{}: other models on {item} than on a gun before: {before:?}, {optic:?}", module.id);
                    }
                    match gun.fits_of {
                        Some(base) if source.guns[base].fits.contains(&placed.row) => {}
                        Some(base) => {
                            gun.fits = source.guns[base].fits.clone();
                            gun.fits_of = None;
                            gun.fits.push(placed.row);
                        }
                        None => gun.fits.push(placed.row),
                    }
                    own_legacy.insert(placed.row);
                    placed_legacy.push((legacy.len(), dict, pair.scope, placed.at));
                    legacy.push(placed.report);
                }
                Err(why) => lost.push(Lost {
                    gun: live.name.clone(),
                    scope: pair.scope,
                    why: format!("{why:#}"),
                }),
            }
        }
        if let Some(base) = gun.fits_of {
            // Its base gun's legacy rows it has no slot for are not its fits.
            let foreign: Vec<usize> = gun_legacy[base].difference(&own_legacy).copied().collect();
            if !foreign.is_empty() {
                gun.fits = source.guns[base].fits.iter().filter(|row| !foreign.contains(row)).copied().collect();
                gun.fits_of = None;
            }
        }
        gun_legacy.push(own_legacy);
        aimed.insert(live.name.clone(), Aimed { poses: poses.clone() });
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
    let mut centres = vec![];
    for live in items {
        let Some(b) = built.iter().find(|b| b.item == live.donor_item) else {
            continue;
        };
        let dict = report_items[&live.name].0;
        check(&mut donor, &catalog, &parsed, b, &live.name, dict, ids, renders, &aimed[&live.name], &mut checks, &mut centres)
            .with_context(|| format!("checking {}", live.name))?;
    }
    // The placed legacy rows through the written table: the optic where its variant has it.
    for (at, dict, scope, want) in &placed_legacy {
        let live_item = legacy[*at].gun.clone();
        let wanted = Wanted {
            scope: DictId::new(*scope),
            ..Wanted::default()
        };
        let gun = parsed.gun(DictId::new(*dict).context("a dict id of 0")?).context("a legacy row's gun")?;
        match weapon_modules::resolve(gun, wanted) {
            Ok(Loadout::Bare(bare)) => {
                let Some(optic) = bare.optic else {
                    bail!("{live_item}: resolve gives the legacy scope {scope} no optic");
                };
                ensure!(optic.frame.bone == want.bone, "{live_item}: the legacy scope {scope} on another bone");
                let apart = spread(&PROBE, &Frame::from_affine(&optic.frame.offset), &Frame::from_affine(&want.offset));
                legacy[*at].optic_mm = (apart * 1e6).round() / 1e3;
                ensure!(
                    bare.parts().count() >= usize::from(!legacy[*at].rails.is_empty()),
                    "{live_item}: the legacy scope {scope} without its rail"
                );
                if let Some(eye) = bare.eye {
                    let module = legacy[*at].module.clone();
                    push_centre(&donor, &live_item, &module, &optic, &eye, &aimed[&live_item], &mut centres)?;
                }
            }
            other => bail!("{live_item}: the legacy scope {scope} does not resolve: {:?}", other.err()),
        }
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
                    look: b.look,
                    seats: b.composed.seats,
                    laid: b.laid,
                    plan: b.plan,
                };
                (b.item, report)
            })
            .collect(),
        items: report_items,
        without_item,
        left_out: donor.left_out.clone(),
        missing_locators: maker.missing.borrow().clone(),
        suppressor_shapes: catalog
            .modules
            .values()
            .filter_map(|m| Some((m.id.clone(), m.suppressor_shape.clone()?)))
            .collect(),
        modules: source.modules.len(),
        table_bytes: table.len(),
        checks,
        optics,
        centres,
        legacy,
        lost,
        files: files.len(),
    };
    fs::write(output.join("modules-069d.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

/// `frame` moved along its own line of sight (its k) to where that line
/// passes nearest `point`.
fn nearest_on_line(frame: &Frame, point: &[f64; 3]) -> Frame {
    let k = frame.r[2];
    let along = (0..3).map(|i| (point[i] - frame.t[i]) * k[i]).sum::<f64>() / (0..3).map(|i| k[i] * k[i]).sum::<f64>();
    Frame { t: [0, 1, 2].map(|i| frame.t[i] + along * k[i]), ..*frame }
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
    renders: Option<&Path>,
    aimed: &Aimed,
    out: &mut Vec<Checked>,
    centres: &mut Vec<Centre>,
) -> Result<()> {
    let item = &b.item;
    let unshift = Frame::translation(b.gun.offset.map(|c| -c));
    let gun = table
        .gun(DictId::new(dict).context("a dict id of 0")?)
        .with_context(|| format!("the table has no gun {dict}"))?;
    // 0.69d0's frame of a locator of the gun: half turn x locator x its bone's bind frame.
    // (On a bone the live rig binds otherwise and whose own parts lie on the live mesh at the
    // live seat, `Seat`, 0.69d0's locator stands on the live bind frame, with no shift.)
    let donor_gun = |name: &str| -> Result<Frame> {
        let l = b
            .composed
            .donor_locators
            .iter()
            .find(|l| l.name == name)
            .with_context(|| format!("0.69d0's {item} has no locator {name}"))?;
        let ours = b.composed.slots.iter().find(|x| x.name == name);
        let live_seat = ours.and_then(|x| b.composed.seats.iter().find(|s| s.bone == x.bone && s.live));
        if let (Some(ours), Some(_)) = (ours, live_seat) {
            let (_, bone) = b
                .composed
                .ship_bind
                .iter()
                .find(|(n, _)| *n == ours.bone)
                .with_context(|| format!("{name} names an unknown bone {}", ours.bone))?;
            return Ok(HALF_TURN.then(&locator_frame(l)).then(&bind_frame(bone)).then(&unshift));
        }
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
                pass: note.is_empty() && metres <= if what == "hand" { HAND_LIMIT } else { LIMIT },
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
            if displaced.contains(&at) || b.taken.contains(&part.slot) {
                continue;
            }
            let models = catalog.module(&part.module)?.models_on(item);
            let is_folded = folded.contains(part.slot.as_str()) || b.folded.contains(&part.slot);
            let model = match (&models.alt, is_folded) {
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
            let flat = |meshes: Vec<Mesh>| -> Vec<[f32; 3]> { meshes.into_iter().flat_map(|m| m.positions).collect() };
            let (built, theirs_points) = (flat(donor.built_meshes(&path, ".model")?), flat(donor.donor_meshes(model)?));
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
        if let Some(dir) = renders {
            let placed = |meshes: Vec<Mesh>, frame: &Frame, colour: [u8; 3]| -> Vec<Shape> {
                meshes
                    .into_iter()
                    .map(|m| Shape {
                        points: m.positions.iter().map(|p| frame.apply(p.map(f64::from)).map(|c| c as f32)).collect(),
                        triangles: m.triangles,
                        colour,
                    })
                    .collect()
            };
            // 0.69d0: its base model, its own parts that stay, the carriers, the fit.
            let mut left: Vec<Shape> = donor
                .donor_body(item)?
                .into_iter()
                .map(|(points, triangles)| Shape {
                    points: points.iter().map(|p| shift.apply(p.map(f64::from)).map(|c| c as f32)).collect(),
                    triangles,
                    colour: render::BODY,
                })
                .collect();
            for part in b.plan.parts.iter().filter(|p| !p.stock) {
                let model = catalog.module(&part.module)?.models_on(item).model.context("a part without a model")?;
                let frame = site_frame(b, catalog, donor, &part.site, &donor_gun)?.then(&shift);
                left.extend(placed(donor.donor_meshes(&model)?, &frame, render::BODY));
            }
            let chain_paths: BTreeSet<String> = fit
                .chain
                .iter()
                .filter_map(|l| catalog.module(&l.module).ok()?.models_on(item).model)
                .filter_map(|m| module_path(&m).ok())
                .collect();
            let fit_model = module.models_on(item).model.context("a fit without a model")?;
            for (path, (model, frame)) in &expected {
                let colour = if chain_paths.contains(path) {
                    render::CARRIER
                } else if *model == fit_model {
                    render::FITTED
                } else {
                    render::OWN
                };
                left.extend(placed(donor.donor_meshes(model)?, &frame.then(&shift), colour));
            }
            if fit.kind == Kind::Scope {
                left.extend(placed(donor.donor_meshes(&fit_model)?, &fit_frame.then(&shift), render::FITTED));
            }
            // Ours: the bare body of the bundle and what resolve places on it.
            let bare_path = body_path(b.gun.model_path.as_deref().unwrap_or_default(), &Body::Bare(&BTreeSet::new()));
            let mut right = placed(donor.built_meshes(&bare_path, ".skinned_model")?, &Frame::IDENTITY, render::BODY);
            for part in bare.parts() {
                let colour = match part.module.role() {
                    Role::Stock => render::OWN,
                    Role::Carrier => render::CARRIER,
                    Role::Fitted(_) => render::FITTED,
                };
                right.extend(placed(donor.built_meshes(part.model().to_str()?, ".model")?, &ours(&part.at)?, colour));
            }
            if let Some(optic) = bare.optic {
                let path = optic.module.model().to_str()?;
                right.extend(placed(donor.built_meshes(path, ".model")?, &ours(&optic.frame)?, render::FITTED));
            }
            let dest = dir.join(live_item);
            fs::create_dir_all(&dest)?;
            fs::write(dest.join(format!("{}.png", fit.module)), render::pair(&left, &right, 640)?)?;
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
                        let ours_eye = ours(&eye.placed(EyeAnchor::Optic))?;
                        // An open sight's eye keeps the gun's own eye plane (table version 3):
                        // ours the live gun's, 0.69d0's its own body's, so the two eyes agree on
                        // the line of sight, not on the place along it.
                        let in_plane = rules
                            .and_then(|rules| rules.overrides.iter().find(|o| o.id == "aim"))
                            .is_some_and(|aim| aim.plane.is_some());
                        let theirs = if in_plane { nearest_on_line(&theirs, &ours_eye.t) } else { theirs };
                        row(if in_plane { "eye line" } else { "eye" }, spread(&PROBE, &ours_eye, &theirs), "");
                        if !push_centre(donor, live_item, &fit.module, &optic, &eye, aimed, centres)? {
                            row("reticle", 0.0, "its aimed model has no reticle surface");
                        }
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
                    // As a gun rig's locator on the grip: half turn x L x half turn, at the grip's
                    // place in the gun's axes, not turned with the grip (the crate's Hand::new: the
                    // PKP's own grip, a quarter turn on its gun, holds the hand 139 mm off otherwise).
                    let at = Frame { r: Frame::IDENTITY.r, t: fit_frame.t };
                    let theirs = HALF_TURN.then(l).then(&HALF_TURN).then(&at).then(&shift);
                    row("hand", spread(&PROBE, &ours(&hand.wrist)?, &theirs), "");
                }
                _ => row("hand", 0.0, "no hand on one side"),
            }
        }
    }
    Ok(())
}

/// The reticle of `optic`'s aimed model (a collimator's [`RETICLE_SURFACE`],
/// a scope's [`SCOPE_RETICLE`]) as it is seen on `gun`, to `centres`; false
/// when the aimed model has neither surface.
fn push_centre(
    donor: &Donor069,
    gun: &str,
    fit: &str,
    optic: &weapon_modules::Optic,
    eye: &weapon_modules::Eye,
    aimed: &Aimed,
    centres: &mut Vec<Centre>,
) -> Result<bool> {
    let aimed_model = optic.module.aimed().unwrap_or(optic.module.model()).to_str()?;
    // A collimator's dot; else a magnified scope's lens, whose middle is its optical axis (the
    // reticle's own box is not: the PSO-1's chevron and range scale stand off its middle).
    let render = format!("models/{aimed_model}.model/render/");
    let lens = donor
        .files
        .keys()
        .filter_map(|key| key.strip_prefix(render.as_str())?.strip_suffix("/converted_model"))
        .filter(|surface| surface.starts_with(LENS))
        .min();
    let surfaces = [Some(RETICLE_SURFACE), lens, Some(SCOPE_RETICLE)].into_iter().flatten().find_map(|surface| {
        let mesh = donor.files.get(&format!("{render}{surface}/converted_model"))?;
        let bounds = Bounds::of(Mesh::parse(mesh).ok()?.positions)?;
        Some((surface.to_owned(), std::array::from_fn(|k| f64::from(bounds.min[k] + bounds.max[k]) / 2.0)))
    });
    // A collimator's aimed model has no reticle (the client draws the player's pattern at the
    // optic's `aim` place, whose origin is the pattern's aiming point).
    let reticle = surfaces.or_else(|| {
        let eye = optic.module.attaches().find_map(|attach| attach.eye())?;
        let (AttachEye::Own(at) | AttachEye::InGunPlane(at)) = eye;
        Some((RETICLE_PLACE.to_owned(), at.t.map(f64::from)))
    });
    let Some((surface, middle)) = reticle else {
        return Ok(false);
    };
    centres.push(centre(gun, fit, &surface, middle, &optic.frame, eye, &aimed.poses)?);
    Ok(true)
}

/// Where the reticle whose middle is `middle` (the optic's model space) is
/// seen on a gun ([`Centre`]). The eye's record is the one the client
/// writes (`attachments::EYE`, [`EyeAnchor::GunIdle`]).
fn centre(
    gun: &str,
    fit: &str,
    surface: &str,
    middle: [f64; 3],
    optic: &Placement,
    eye: &weapon_modules::Eye,
    poses: &[Vec<Frame>; 3],
) -> Result<Centre> {
    let eye = eye.placed(EyeAnchor::GunIdle);
    let mut seen = [[0.0; 3]; 3];
    let mut origin = [0.0; 3];
    for (nth, pose) in poses.iter().enumerate() {
        let on = |at: &Placement| -> Result<Frame> {
            let bone = pose
                .get(usize::from(at.bone.0))
                .with_context(|| format!("{gun}: no pose of bone {}", at.bone.0))?;
            Ok(HALF_TURN.then(&Frame::from_affine(&at.offset)).then(bone))
        };
        // calculate_aim_alignment: the eye's frame is aim_offset x half turn x offset x bone, and
        // a scope item's modification sets the gun's aim_offset to 0 (the eye is the table's alone).
        let camera = on(&eye)?.inverse();
        let frame = on(optic)?;
        seen[nth] = camera.apply(frame.apply(middle));
        if nth == 0 {
            origin = camera.apply(frame.apply([0.0; 3]));
        }
    }
    let off = |p: [f64; 3]| (p[0].hypot(p[1]) / p[2].abs()).atan() * 1e3;
    let round = |v: f64| (v * 1e4).round() / 1e4;
    Ok(Centre {
        gun: gun.to_owned(),
        fit: fit.to_owned(),
        surface: surface.to_owned(),
        off_mrad: seen.map(|p| round(off(p))),
        ahead: round(seen[0][2]),
        eye_behind_optic: round(origin[2]),
    })
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
