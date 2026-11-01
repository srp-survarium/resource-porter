// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d0 as a [`Donor`] of the shared core (`docs/weapons-069d-026.md`).
//!
//! 0.69d0 keeps its item configs in `gameplay/game.options`
//! (`weapons.<item>`, `weapon_modules.<id>`), writes 16-byte configs and
//! 12-byte clip event headers, draws weapons as a rig of modules placed at
//! locators, and shades them with PBR maps. This donor answers the core in
//! 0.26g's layouts:
//!
//! * configs re-encoded in 24-byte records; clips with 16-byte event headers
//!   (the core's [`clips::events_12_to_16`]);
//! * each gun's model ([`Gun069`], the port's donor sidecar
//!   `ports/weapons-*.donor.json`) composed at 0.69d0's own model path: its
//!   skinned surfaces widened to 0.26g's 36-byte vertices and its
//!   **structural** modules baked in (receiver parts, stocks, pistol grips,
//!   magazines, bolts, the bare barrel). Muzzle devices, foregrips, scopes and
//!   suppressors are never baked: [`Donor069::attach_points`] records where
//!   each gun takes them. On a live rig ([`Rig::Live`]) the model takes the
//!   live model's bind pose, skeleton file and hand locators, so the live
//!   clips play on it;
//! * each material the models draw on the native AK-12 material with the
//!   four maps fitted from PBR ([`convert`]), and those maps;
//! * an item config without what 0.26g cannot take: suppressed sounds and
//!   0.69d0-only clips (aimed shots, the pump's empty-chamber start).
//!
//! What a port decides (template, balance, which states stay the
//! template's) is the core manifest's, as for any donor.

pub mod convert;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::clips;
use super::donor::{Donor, DonorBuild, is_config};
use super::{field, get, s};
use crate::weapon::parse_config;
use crate::{BinaryConfig, ConfigValue as V, ResourceDb};
use convert::Skinned;

const MATERIAL_TEMPLATE: &str = "material_instances/weapons/ak_12.material";
const SETTINGS_TEMPLATE: &str = "models/weapons/ak_12/ak_12.skinned_model/settings";
/// A native weapon's hands clip: the 0.26g player rig every hands clip drives.
pub const NATIVE_HANDS_CLIP: &str =
    "animations/single/weapons/ak_12/player/stand/3rd_view/on_site/hands_only/reload.bones_names";

/// 0.69d weapon rigs keep their meshes rotated half a turn about Y from the
/// frame their bind pose and locators are written in (x and z negated).
fn mesh_from_bind(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], -p[2]]
}

/// Whose skeleton and clips a gun uses.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Rig {
    /// A live weapon's (`models/....skinned_model`): the donor rig is that
    /// skeleton bone for bone (checked); the model takes its bind pose,
    /// skeleton file and hand locators.
    Live(String),
    /// The donor's own skeleton and clips.
    Donor,
}

/// What 0.69d0 needs to know of a gun beyond the core manifest: its rig and
/// the modules left out.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gun069 {
    pub rig: Rig,
    /// Bundled or default modules (`weapon_modules.<id>`) that are **not**
    /// baked: muzzle devices (the weapon kit's), and any foregrip, scope or
    /// suppressor. Every other one is structural and baked.
    #[serde(default)]
    pub separable: Vec<String>,
    /// Baked modules turned about the vertical axis (degrees) before they
    /// are placed: meshes 0.69d0 authored facing another way than the gun.
    #[serde(default)]
    pub module_turns: BTreeMap<String, f32>,
}

/// A new round (an ammunition item no 0.26g weapon fires yet): a live
/// round's config and dictionary row with its own values.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Round069 {
    pub name: String,
    pub dict_id: i32,
    pub config: String,
    /// The live round (`items_dict` name) its config and row start from.
    pub template: String,
    pub base_cost: i32,
    /// Config values by dotted path, typed as the template has them.
    pub set: BTreeMap<String, serde_json::Value>,
    /// Its cell of `LobbyMenu_AmmoIcons`, and the live round whose icon it
    /// takes (0.69d0 draws its 23 mm round with its 12/70 buckshot's).
    pub icon: [u32; 2],
    pub icon_like: String,
}

/// The donor sidecar of a 0.69d0 port (`ports/weapons-*.donor.json`).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec069 {
    pub format_version: u32,
    /// By the core manifest's `donor_item` (0.69d0's item name).
    pub guns: BTreeMap<String, Gun069>,
    #[serde(default)]
    pub rounds: Vec<Round069>,
}

impl Spec069 {
    pub fn load(path: &Path) -> Result<Self> {
        let spec: Self = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        ensure!(spec.format_version == 1, "{}: format_version 1", path.display());
        Ok(spec)
    }
}

/// A point the weapon kit may attach to: a donor locator (its bone and its
/// bind-frame position from that bone) and the donor module left out there.
#[derive(Clone, Debug, Serialize)]
pub struct AttachPoint {
    pub locator: String,
    pub bone: String,
    pub position: [f32; 3],
    pub module: Option<String>,
}

/// One of the donor's bundled or default modules, resolved to its model and
/// the rig locator it sits at.
#[derive(Clone, Debug, Serialize)]
pub struct PlacedModule {
    pub id: String,
    pub model: String,
    pub locator: String,
    pub baked: bool,
}

/// 0.69d0's files for the core: the archive's, re-encoded, and the
/// composed models, materials and textures.
pub struct Donor069 {
    db: ResourceDb,
    game: V,
    files: BTreeMap<String, Vec<u8>>,
    attach_points: BTreeMap<String, Vec<AttachPoint>>,
    modules: BTreeMap<String, Vec<PlacedModule>>,
}

fn string_of(v: Option<&V>) -> Result<&str> {
    match v {
        Some(V::String(s)) => Ok(s),
        other => bail!("expected a string, found {other:?}"),
    }
}
fn int_of(v: Option<&V>) -> Result<i32> {
    match v {
        Some(V::Integer(i)) => Ok(*i),
        other => bail!("expected an integer, found {other:?}"),
    }
}
fn float3(v: Option<&V>) -> Result<[f32; 3]> {
    match v {
        Some(V::Float3(f)) => Ok(*f),
        other => bail!("expected a float3, found {other:?}"),
    }
}
fn list(v: Option<&V>) -> &[V] {
    match v {
        Some(V::Indexed(items)) => items,
        _ => &[],
    }
}

fn db_config(db: &mut ResourceDb, path: &str) -> Result<V> {
    let bytes = db.read(path).with_context(|| format!("reading donor {path}"))?;
    Ok(BinaryConfig::parse(&bytes)
        .with_context(|| format!("parsing donor {path}"))?
        .root)
}

fn encode(root: V) -> Result<Vec<u8>> {
    BinaryConfig { root }.to_bytes()
}

/// A bind pose: per bone its name and translation.
pub fn bind_pose(b: &[u8]) -> Result<Vec<(String, [f32; 3])>> {
    ensure!(b.len() >= 2, "bind pose shorter than its count");
    let n = usize::from(u16::from_le_bytes([b[0], b[1]]));
    let mut at = 2;
    let mut out = vec![];
    for _ in 0..n {
        let end = b[at..]
            .iter()
            .position(|c| *c == 0)
            .context("unterminated bone name")?;
        let name = String::from_utf8(b[at..at + end].to_vec())?;
        at += end + 1;
        let f: Vec<f32> = b
            .get(at..at + 36)
            .context("truncated bind frame")?
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        at += 36;
        ensure!(
            f[6..9].iter().all(|s| (s - 1.0).abs() < 1e-4),
            "bone {name} is scaled"
        );
        out.push((name, [f[0], f[1], f[2]]));
    }
    ensure!(at == b.len(), "bind pose has trailing bytes");
    Ok(out)
}

/// `bones_names` of a clip.
pub fn clip_bones(bytes: &[u8]) -> Result<Vec<String>> {
    let config = BinaryConfig::parse(bytes)?;
    list(config.root.named("bones_names"))
        .iter()
        .map(|n| match n {
            V::String(s) => Ok(s.clone()),
            _ => bail!("bones_names holds a non-string"),
        })
        .collect()
}

/// Muzzle devices, suppressors, foregrips and optics: never baked.
pub fn is_attachment(id: &str, model: &str) -> bool {
    const KINDS: [&str; 7] = [
        "muzzlebrake",
        "flashhider",
        "silencer",
        "suppressor",
        "foregrip",
        "scope",
        "collimator",
    ];
    KINDS
        .iter()
        .any(|k| id.contains(k) || model.contains(&format!("/{k}")))
}

/// Clips of 0.69d0-only states, left out of an item config: the aimed shots
/// (`_calim`) and the pump's start of a reload into an empty chamber
/// (`reload_finalround`; 0.26g's pumps start every reload alike).
pub fn drops_clip(reference: &str) -> bool {
    reference.ends_with("_calim") || reference.ends_with("/reload_finalround")
}

/// An item config without what 0.26g cannot take: suppressed sounds (no
/// suppressors, the owner) and clips of 0.69d0-only states.
pub fn without_069d_only(v: &V) -> V {
    match v {
        V::Named(fields) => V::Named(
            fields
                .iter()
                .map(|f| field(&f.name, without_069d_only(&f.value)))
                .collect(),
        ),
        V::Indexed(items) => V::Indexed(
            items
                .iter()
                .filter(|item| match item {
                    V::String(s) => !drops_clip(s),
                    V::Indexed(pair) => !matches!(
                        pair.as_slice(),
                        [V::Integer(_), V::String(name)] if name.contains("silencer")
                    ),
                    _ => true,
                })
                .map(without_069d_only)
                .collect(),
        ),
        other => other.clone(),
    }
}

#[derive(Clone)]
struct Locator {
    name: String,
    bone: String,
    bone_idx: i32,
    position: [f32; 3],
    rotation: [f32; 3],
}

fn locators(properties: &V) -> Result<Vec<Locator>> {
    list(properties.named("locators"))
        .iter()
        .map(|l| {
            Ok(Locator {
                name: string_of(l.named("name"))?.into(),
                bone: string_of(l.named("bone_name"))?.into(),
                bone_idx: int_of(l.named("bone_idx"))?,
                position: float3(l.named("position"))?,
                rotation: float3(l.named("rotation"))?,
            })
        })
        .collect()
}

fn locator_value(l: &Locator) -> V {
    // Native field order (the AK-12's).
    V::Named(vec![
        field("rotation", V::Float3(l.rotation)),
        field("position", V::Float3(l.position)),
        field("name", s(&l.name)),
        field("bone_idx", V::Integer(l.bone_idx)),
        field("bone_name", s(&l.bone)),
    ])
}

/// Where a module's origin lands in mesh space: the socket bone's bind
/// translation plus the locator's position, both in the bind frame.
fn socket(loc: &Locator, bind: &[(String, [f32; 3])]) -> Result<[f32; 3]> {
    let (_, t) = bind
        .iter()
        .find(|(n, _)| *n == loc.bone)
        .with_context(|| format!("{} names an unknown bone {}", loc.name, loc.bone))?;
    let p = mesh_from_bind(loc.position);
    Ok([t[0] + p[0], t[1] + p[1], t[2] + p[2]])
}

/// A socket's turn in mesh space. Only a turn about the bore's crosswise
/// axis (x) is met (the PPSh's drum well, -3.7 degrees); in mesh space, half
/// a turn about y from the bind frame, it is the opposite angle about x.
fn socket_rotation(loc: &Locator) -> Result<[[f32; 3]; 3]> {
    let [x, y, z] = loc.rotation;
    ensure!(
        y.abs() < 1e-5 && z.abs() < 1e-5,
        "{} turns about y or z ({:?}); only turns about x are baked",
        loc.name,
        loc.rotation
    );
    let (s, c) = (-x).sin_cos();
    Ok([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
}

fn turn_about_y(a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

/// `a` after `b`: the matrix applying `b` first.
fn multiply3(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// Locator names of the 0.26g contract that are made, not copied.
const CONTRACT: [&str; 4] = ["aim", "barrel_point", "scope_point", "handguard_point"];
/// Donor locators a kit attachment can use.
const ATTACH_LOCATORS: [&str; 6] = [
    "muzzle_point",
    "fire_point",
    "scope_picatinnyrail_point",
    "picatinny_scope_point",
    "grip_picatinnyrail_point",
    "tactical_picatinnyrail_point",
];

fn is_hand_locator(name: &str) -> bool {
    ["_idle", "_lobby", "_reload", "_reload_pivot"]
        .iter()
        .any(|k| name.ends_with(k))
}

/// A static model's LOD0 surfaces and their materials.
fn lod0_surfaces(settings: &V, path: &str) -> Result<Vec<(String, String)>> {
    let surfaces = list(get(settings, "lod_hierrarchy.LOD0.surfaces"));
    ensure!(!surfaces.is_empty(), "{path} has no LOD0 surface");
    surfaces
        .iter()
        .map(|surface| {
            let surface = string_of(Some(surface))?.to_owned();
            let material = string_of(get(
                settings,
                &format!("material_settings.{surface}.material_name"),
            ))?
            .to_owned();
            Ok((surface, material))
        })
        .collect()
}

/// The donor map roles and the 0.26g ones they feed.
const MAP_ROLES: [(&str, &str, Option<convert::ToneCurve>); 4] = [
    ("t_albedo", "texture_diffuse", Some(convert::ALBEDO)),
    ("t_normal", "texture_normal", None),
    (
        "t_smoothness",
        "texture_roughness",
        Some(convert::SMOOTHNESS_TO_GLOSS),
    ),
    (
        "t_metalness",
        "texture_fresnel",
        Some(convert::METALNESS_TO_FRESNEL),
    ),
];

/// The 0.26g diffuse of a 0.69d albedo and metalness pair (see
/// [`convert::diffuse_from_albedo`]), DXT1 with mips like a native weapon's.
fn diffuse_dds(albedo: &[u8], metalness: &[u8], name: &str, work: &Path) -> Result<Vec<u8>> {
    let (w, h, a) = convert::decode_dxt1(albedo)?;
    let (mw, mh, m) = convert::decode_dxt1(metalness)?;
    let px: Vec<[u8; 3]> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let metal = m[(y * mh / h) * mw + x * mw / w][0];
            convert::diffuse_from_albedo(a[i], metal)
        })
        .collect();
    fs::create_dir_all(work)?;
    let stem = name.replace('/', "_");
    let tga = work.join(format!("{stem}.tga"));
    let dds = work.join(format!("{stem}.dds"));
    fs::write(&tga, convert::tga(w, h, &px)?)?;
    crate::armor::convert_texture(&tga, &dds, crate::manifest::TextureCompression::Dxt1)?;
    let out = crate::duty_026::texture_dds(&fs::read(&dds)?, name, true)?;
    fs::remove_file(&tga)?;
    fs::remove_file(&dds)?;
    Ok(out)
}

impl Donor069 {
    /// The donor archive (0.69d0's files the guns draw, with
    /// `gameplay/game.options`), composed for `spec`'s guns over `target`
    /// (the stock database and the patches players have; the live rigs and
    /// the native templates). `work` holds the texture encoder's scratch.
    pub fn open(db: &Path, spec: &Spec069, target_layers: &[PathBuf], work: &Path) -> Result<Self> {
        let mut db = ResourceDb::open(db)?;
        let game = db_config(&mut db, "gameplay/game.options")?;
        let mut target = ResourceDb::open_layers(target_layers)?;
        let mut donor = Self {
            db,
            game,
            files: BTreeMap::new(),
            attach_points: BTreeMap::new(),
            modules: BTreeMap::new(),
        };
        for (item, gun) in &spec.guns {
            let materials = donor
                .compose_model(item, gun, &mut target)
                .with_context(|| format!("{item}: model"))?;
            for material in materials {
                donor
                    .compose_material(&material, &mut target, work)
                    .with_context(|| format!("{item}: material {material}"))?;
            }
        }
        if work.exists() {
            fs::remove_dir(work).context("the texture encoder left files behind")?;
        }
        for path in donor.files.keys() {
            ensure!(
                !target.contains(path),
                "{path}: the target has it; 0.69d0's paths must be new"
            );
        }
        Ok(donor)
    }

    /// Each gun's attach points for the weapon kit, by 0.69d0 item.
    pub fn attach_points(&self) -> &BTreeMap<String, Vec<AttachPoint>> {
        &self.attach_points
    }

    /// Each gun's bundled and default modules, baked or left out.
    pub fn modules(&self) -> &BTreeMap<String, Vec<PlacedModule>> {
        &self.modules
    }

    fn weapon(&self, item: &str) -> Result<&V> {
        get(&self.game, &format!("weapons.{item}"))
            .with_context(|| format!("0.69d0 has no weapons.{item}"))
    }

    fn put(&mut self, path: String, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            self.files.insert(path.clone(), bytes).is_none(),
            "{path} made twice"
        );
        Ok(())
    }

    /// The gun's modules in their attach order (bundled, then default), each
    /// at the locator its slot names after the slot overrides of the baked
    /// modules before it.
    fn placed_modules(&self, item: &str, gun: &Gun069) -> Result<Vec<PlacedModule>> {
        let w = self.weapon(item)?;
        let mut slots: BTreeMap<String, String> = BTreeMap::new();
        for slot in list(w.named("slots")) {
            slots.insert(
                string_of(slot.named("id"))?.into(),
                string_of(slot.named("locator"))?.into(),
            );
        }
        let mut ids = vec![];
        for kind in ["bundled_modules", "default_modules"] {
            for m in list(w.named(kind)) {
                ids.push(string_of(m.named("module"))?.to_owned());
            }
        }
        for id in &gun.separable {
            ensure!(ids.contains(id), "{item}: {id} is none of its modules");
        }
        let mut out = vec![];
        for id in ids {
            let module = get(&self.game, &format!("weapon_modules.{id}"))
                .with_context(|| format!("0.69d0 has no weapon_modules.{id}"))?;
            let model = match get(module, &format!("exceptions.{item}.model")) {
                Some(v) => string_of(Some(v))?.to_owned(),
                None => string_of(module.named("model"))?.to_owned(),
            };
            let attach = list(module.named("slots_to_attach_to"));
            ensure!(attach.len() == 1, "{id} attaches to {} slots", attach.len());
            let slot = string_of(attach[0].named("slot"))?.to_owned();
            let locator = slots
                .get(&slot)
                .with_context(|| format!("{item}: {id}'s slot {slot} has no locator"))?
                .clone();
            let baked = !gun.separable.contains(&id);
            ensure!(
                !(baked && is_attachment(&id, &model)),
                "{item}: {id} ({model}) is an attachment, not structural; list it as separable"
            );
            if baked {
                for key in ["slots_to_override", "slots_to_add"] {
                    for o in list(get(module, &format!("attach_params.{key}"))) {
                        slots.insert(
                            string_of(o.named("id"))?.into(),
                            string_of(o.named("locator"))?.into(),
                        );
                    }
                }
            }
            out.push(PlacedModule {
                id,
                model,
                locator,
                baked,
            });
        }
        Ok(out)
    }

    /// The gun's model at 0.69d0's model path; returns the materials it draws.
    #[allow(clippy::too_many_lines)]
    fn compose_model(
        &mut self,
        item: &str,
        gun: &Gun069,
        target: &mut ResourceDb,
    ) -> Result<BTreeSet<String>> {
        let w = self.weapon(item)?.clone();
        let root = format!("models/{}.skinned_model", string_of(get(&w, "object.model"))?);
        let bind_bytes = self.db.read(&format!("{root}/render/bind_pose"))?;
        let bind = bind_pose(&bind_bytes)?;
        let properties = db_config(&mut self.db, &format!("{root}/render/export_properties"))?;
        let all = locators(&properties)?;
        let find = |name: &str| {
            all.iter()
                .find(|l| l.name == name)
                .with_context(|| format!("{item} has no {name} locator"))
        };
        // The bind pose shipped: the live model's for a live rig (its clips
        // are written for it), else the donor's.
        let (ship_bind, live_props, skeleton_file) = match &gun.rig {
            Rig::Live(model) => {
                let live = target.read(&format!("{model}/render/bind_pose"))?;
                let names: Vec<String> = bind_pose(&live)?.into_iter().map(|(n, _)| n).collect();
                let ours: Vec<String> = bind.iter().map(|(n, _)| n.clone()).collect();
                ensure!(
                    names == ours,
                    "{item}: the donor rig's bones {ours:?} are not {model}'s {names:?}"
                );
                let props =
                    parse_config(target, &format!("{model}/render/export_properties"), "live rig")?
                        .root;
                let skeleton = parse_config(target, &format!("{model}/skeleton"), "live skeleton")?.root;
                (live, Some(props), skeleton)
            }
            Rig::Donor => (
                bind_bytes.clone(),
                None,
                db_config(&mut self.db, &format!("{root}/skeleton"))?,
            ),
        };
        let ship_bones = bind_pose(&ship_bind)?;
        let settings = db_config(&mut self.db, &format!("{root}/settings"))?;
        let mut surfaces: Vec<(String, Skinned, String)> = vec![];
        let mut attach: Vec<AttachPoint> = vec![];
        for surface in list(get(&settings, "lod_hierrarchy.LOD0.surfaces")) {
            let surface = string_of(Some(surface))?;
            let material = string_of(get(
                &settings,
                &format!("material_settings.{surface}.material_name"),
            ))?;
            let bytes = self
                .db
                .read(&format!("{root}/render/{surface}/converted_model"))?;
            let mesh = convert::skinned_069_to_026(&bytes, bind.len())?;
            surfaces.push((surface.into(), mesh, material.into()));
        }
        let modules = self.placed_modules(item, gun)?;
        for module in &modules {
            let loc = find(&module.locator)?;
            if !module.baked {
                attach.push(AttachPoint {
                    locator: loc.name.clone(),
                    bone: loc.bone.clone(),
                    position: loc.position,
                    module: Some(module.id.clone()),
                });
                continue;
            }
            let path = format!("models/{}.model", module.model);
            let msettings = db_config(&mut self.db, &format!("{path}/settings"))?;
            // In the game a bone's vertices are drawn at clip * bind^-1 of
            // the shipped rig; 0.69d0 drew the module at clip * locator. So
            // the module sits at the shipped bind frame of its bone (the live
            // rig's on a live rig: the AKMSU's magazine bone rests 15 cm off
            // in 0.69d0's bind pose, where the AK clips never take it).
            let origin = socket(loc, &ship_bones)?;
            let mut rotation = socket_rotation(loc)?;
            if let Some(degrees) = gun.module_turns.get(&module.id) {
                rotation = multiply3(&rotation, &turn_about_y(degrees.to_radians()));
            }
            let bone = bind
                .iter()
                .position(|(n, _)| *n == loc.bone)
                .context("socket bone")? as u32;
            for (surface, material) in lod0_surfaces(&msettings, &path)? {
                let bytes = self
                    .db
                    .read(&format!("{path}/render/{surface}/converted_model"))?;
                let mesh = convert::static_069_to_skinned(&bytes, rotation, origin, bone)?;
                let module_name = module.model.rsplit('/').next().unwrap();
                let name = format!("{module_name}_{surface}");
                ensure!(
                    !surfaces.iter().any(|(n, _, _)| *n == name),
                    "{item}: two surfaces {name}"
                );
                surfaces.push((name, mesh, material));
            }
        }
        for name in ATTACH_LOCATORS {
            if let Ok(loc) = find(name)
                && !attach.iter().any(|a| a.locator == name)
            {
                attach.push(AttachPoint {
                    locator: loc.name.clone(),
                    bone: loc.bone.clone(),
                    position: loc.position,
                    module: None,
                });
            }
        }

        // Locators: 0.26g's contract made from the donor's geometry (the
        // bare barrel's end is `muzzle_point`, where 0.69d0 mounts its
        // devices; a gun with no device slot, the PPSh's compensator jacket,
        // fires from its `fire_point`), and the hand locators (the live
        // model's on a live rig).
        let muzzle = find("muzzle_point").or_else(|_| find("fire_point"))?;
        let aim = find("aim")?;
        let scope = ["scope_picatinnyrail_point", "picatinny_scope_point"]
            .iter()
            .find_map(|n| find(n).ok())
            .unwrap_or(aim);
        let handguard = ["handguard_point", "forend_point", "LeftHand_idle"]
            .iter()
            .find_map(|n| find(n).ok())
            .context("no handguard locator")?;
        let mut locs = vec![
            Locator {
                name: "aim".into(),
                ..aim.clone()
            },
            Locator {
                name: "barrel_point".into(),
                rotation: [0.0; 3],
                ..muzzle.clone()
            },
            Locator {
                name: "scope_point".into(),
                rotation: [0.0; 3],
                ..scope.clone()
            },
            Locator {
                name: "handguard_point".into(),
                rotation: [0.0; 3],
                ..handguard.clone()
            },
        ];
        match &live_props {
            Some(live) => locs.extend(
                locators(live)?
                    .into_iter()
                    .filter(|l| !CONTRACT.contains(&l.name.as_str())),
            ),
            None => locs.extend(all.iter().filter(|l| is_hand_locator(&l.name)).cloned()),
        }
        ensure!(
            locs.iter().filter(|l| l.name.ends_with("_idle")).count() == 32,
            "{item}: not 32 idle hand locators"
        );

        // Files.
        let mut native = parse_config(target, SETTINGS_TEMPLATE, "native rifle settings")?.root;
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
                    .map(|(n, _, m)| {
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
        self.put(format!("{root}/settings"), encode(native)?)?;
        self.put(format!("{root}/skeleton"), encode(skeleton_file)?)?;
        self.put(format!("{root}/render/bind_pose"), ship_bind)?;
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (name, mesh, _) in &surfaces {
            let (a, b) = mesh.bounds();
            // Boxes are written in the bind frame, as the rig's (and native) ones.
            let (a, b) = (mesh_from_bind(a), mesh_from_bind(b));
            let (a, b) = (
                [a[0].min(b[0]), a[1], a[2].min(b[2])],
                [a[0].max(b[0]), b[1], a[2].max(b[2])],
            );
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
            let props = V::Named(vec![
                field(
                    "source",
                    V::Named(vec![
                        field("indices", V::Integer(i32::try_from(mesh.indices.len())?)),
                        field(
                            "primitives",
                            V::Integer(i32::try_from(mesh.indices.len() / 3)?),
                        ),
                        field("mesh_name", s(format!("|{item}|0.69d0|{name}"))),
                        field("texture_name", s("")),
                        field("vertices", V::Integer(i32::try_from(mesh.vertices.len())?)),
                    ]),
                ),
                field("type", V::Integer(44)),
                field("version", V::Integer(0)),
                field(
                    "bounding_box",
                    V::Named(vec![field("max", V::Float3(b)), field("min", V::Float3(a))]),
                ),
            ]);
            self.put(format!("{root}/render/{name}/export_properties"), encode(props)?)?;
            self.put(format!("{root}/render/{name}/converted_model"), mesh.to_bytes()?)?;
        }
        let props = V::Named(vec![
            field("src_filename", s(format!("Survarium 0.69d0: {root}"))),
            field("type", V::Integer(40)),
            field("version", V::Integer(0)),
            field("locators", V::Indexed(locs.iter().map(locator_value).collect())),
            field(
                "bounding_box",
                V::Named(vec![field("max", V::Float3(hi)), field("min", V::Float3(lo))]),
            ),
        ]);
        self.put(format!("{root}/render/export_properties"), encode(props)?)?;
        self.attach_points.insert(item.into(), attach);
        self.modules.insert(item.into(), modules);
        Ok(surfaces.into_iter().map(|(_, _, m)| m).collect())
    }

    /// A donor material on the native AK-12's, its four PBR maps fitted to
    /// 0.26g's diffuse, normal, gloss and Fresnel (at the donor's paths). The
    /// detail normal stays the template's (`det/weap_nmap`, the one 0.69d0's
    /// weapons name too; the cartridges name another).
    fn compose_material(&mut self, name: &str, target: &mut ResourceDb, work: &Path) -> Result<()> {
        let path = format!("material_instances/{name}.material");
        if self.files.contains_key(&path) {
            return Ok(());
        }
        let source = db_config(&mut self.db, &path)?;
        let effect = get(&source, "material.g_stage.effect")
            .with_context(|| format!("{path} has no g_stage effect"))?
            .clone();
        let mut material = parse_config(target, MATERIAL_TEMPLATE, "native rifle material")?.root;
        let mut names = BTreeMap::new();
        for (role, native, _) in MAP_ROLES {
            let texture = string_of(get(&effect, &format!("{role}.value")))
                .with_context(|| format!("{path}: {role}"))?
                .to_owned();
            ensure!(!texture.is_empty(), "{path} binds no {role}");
            material.set_path(&format!("material.g_stage.effect.{native}.value"), s(&texture))?;
            names.insert(role, texture);
        }
        for (role, _, curve) in MAP_ROLES {
            let texture = &names[role];
            let dds = format!("textures/{texture}.dds");
            if self.files.contains_key(&dds) {
                continue;
            }
            let bytes = self.db.read(&dds)?;
            let bytes = if role == "t_albedo" {
                let metalness = self.db.read(&format!("textures/{}.dds", names["t_metalness"]))?;
                diffuse_dds(&bytes, &metalness, &dds, work)?
            } else {
                match curve {
                    Some(curve) => convert::remap_dds(&bytes, curve)?,
                    None => bytes,
                }
            };
            self.put(dds, bytes)?;
            let options = db_config(&mut self.db, &format!("textures/{texture}.options"))?;
            self.put(format!("textures/{texture}.options"), encode(options)?)?;
        }
        self.put(path, encode(material)?)
    }
}

impl Donor for Donor069 {
    fn build(&self) -> DonorBuild {
        DonorBuild::V069d0
    }
    fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.db.contains(path)
    }
    fn is_file(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.db.is_file(path)
    }
    fn paths_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut out: BTreeSet<String> = self
            .files
            .keys()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect();
        // A composed model replaces the donor's folder whole.
        if out.is_empty() {
            out.extend(
                self.db
                    .paths_with_prefix(prefix)
                    .filter(|p| self.db.is_file(p))
                    .map(str::to_owned),
            );
        }
        out.into_iter().collect()
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        if let Some(bytes) = self.files.get(path) {
            return Ok(bytes.clone());
        }
        let bytes = self
            .db
            .read(path)
            .with_context(|| format!("reading donor {path}"))?;
        if path.ends_with(".b-spline") {
            return clips::events_12_to_16(&bytes).with_context(|| format!("converting {path}"));
        }
        ensure!(
            !path.ends_with("/converted_model") && !path.ends_with("/bind_pose"),
            "{path}: 0.69d0's meshes are served only as composed models"
        );
        if is_config(path) && BinaryConfig::is_compact(&bytes) {
            return encode(
                BinaryConfig::parse(&bytes)
                    .with_context(|| format!("parsing donor {path}"))?
                    .root,
            );
        }
        Ok(bytes)
    }
    fn item_config(&mut self, item: &str) -> Result<V> {
        Ok(without_069d_only(self.weapon(item)?))
    }
    fn dictionary(&mut self) -> Result<V> {
        bail!("0.69d0 keeps its dictionary on its server")
    }
}

// ---------------------------------------------------------------------------
// A 0.69d0 port: the core's bundle, and what only 0.69d0 needs
// ---------------------------------------------------------------------------

/// Builds a 0.69d0 port: the core's bundle ([`super::build`]) from
/// [`Donor069`], then the new rounds (`spec.rounds`: config, dictionary row),
/// `donor-069d.json` (each gun's modules, baked or left out, and its attach
/// points for the weapon kit), `equipment.json`, `icons-069d.json` (what
/// `scripts/render-weapons-069d-icons.py` renders), and the check that every
/// carried clip drives its rig's bones.
pub fn build_069d(
    port: &super::manifest::WeaponPort,
    spec: &Spec069,
    donor_db: &Path,
    target_layers: &[PathBuf],
    output: &Path,
) -> Result<super::BuildReport> {
    ensure!(
        port.donor_build == DonorBuild::V069d0,
        "{} is not a 0.69d0 port",
        port.id
    );
    for w in &port.weapons {
        ensure!(
            spec.guns.contains_key(&w.donor_item),
            "{}: the donor sidecar has no {}",
            w.name,
            w.donor_item
        );
    }
    let work = output.with_extension("work");
    let mut donor = Donor069::open(donor_db, spec, target_layers, &work)?;
    let report = super::build(port, &mut donor, target_layers, output)?;
    let overlay = output.join("overlay");
    let mut target = ResourceDb::open_layers(target_layers)?;
    let manifest_path = output.join("port-manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;

    // The new rounds.
    let dict_path = "gameplay/db_static_dictionaries";
    let mut dict = BinaryConfig::parse(&fs::read(crate::archive::safe_join(&overlay, dict_path)?)?)?.root;
    let mut rounds = vec![];
    for round in &spec.rounds {
        let Some(V::Indexed(items)) = dict.named("items_dict") else {
            bail!("items_dict is not indexed");
        };
        let template_row = items
            .iter()
            .find(|v| v.named("name") == Some(&s(&round.template)))
            .with_context(|| format!("no {} row", round.template))?
            .clone();
        ensure!(
            !items.iter().any(|v| v.named("dict_id") == Some(&V::Integer(round.dict_id))
                || v.named("name") == Some(&s(&round.name))
                || v.named("cfg_name") == Some(&s(&round.config))),
            "round {} ({}) is taken",
            round.dict_id,
            round.name
        );
        let template_cfg = string_of(template_row.named("cfg_name"))?.to_owned();
        let mut cfg = parse_config(&mut target, &template_cfg, "live round")?.root;
        for (key, value) in &round.set {
            super::config::set_typed(&mut cfg, key, value).with_context(|| format!("{}: {key}", round.name))?;
        }
        cfg.set_path(
            "ui_desc.icon",
            V::Indexed(round.icon.iter().map(|v| V::Integer(*v as i32)).collect()),
        )?;
        let icon_like = parse_config(
            &mut target,
            &format!(
                "{}",
                string_of(
                    items
                        .iter()
                        .find(|v| v.named("name") == Some(&s(&round.icon_like)))
                        .with_context(|| format!("no {} row", round.icon_like))?
                        .named("cfg_name")
                )?
            ),
            "live round",
        )?
        .root;
        let like: Vec<i32> = list(get(&icon_like, "ui_desc.icon"))
            .iter()
            .map(|v| int_of(Some(v)))
            .collect::<Result<_>>()?;
        let mut row = template_row;
        row.set_path("cfg_name", s(&round.config))?;
        row.set_path("name", s(&round.name))?;
        row.set_path("dict_id", V::Integer(round.dict_id))?;
        row.set_path("base_cost", V::Integer(round.base_cost))?;
        let Some(V::Indexed(items)) = dict.named_mut("items_dict") else {
            unreachable!()
        };
        items.push(row);
        write_file(&overlay, &mut manifest, &round.config, &encode(cfg)?)?;
        rounds.push(serde_json::json!({
            "name": round.name, "dict_id": round.dict_id,
            "icon": round.icon, "icon_like": like,
        }));
    }
    if !spec.rounds.is_empty() {
        write_file(&overlay, &mut manifest, dict_path, &encode(dict)?)?;
    }
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    // The record and the icons' list.
    let mut guns = serde_json::Map::new();
    let mut icons = vec![];
    for w in &port.weapons {
        guns.insert(
            w.name.clone(),
            serde_json::json!({
                "donor_item": w.donor_item,
                "modules": donor.modules().get(&w.donor_item),
                "attach_points": donor.attach_points().get(&w.donor_item),
            }),
        );
        let cfg = BinaryConfig::parse(&fs::read(crate::archive::safe_join(&overlay, &w.config)?)?)?.root;
        let model = string_of(get(&cfg, "object.model"))?;
        let icon: Vec<i32> = list(get(&cfg, "ui_desc.icon"))
            .iter()
            .map(|v| int_of(Some(v)))
            .collect::<Result<_>>()?;
        icons.push(serde_json::json!({
            "name": w.name,
            "model_root": format!("models/{model}.skinned_model"),
            "icon": icon,
            "template": w.template,
        }));
        check_clips(&cfg, &overlay, &mut target).with_context(|| w.name.clone())?;
    }
    fs::write(
        output.join("donor-069d.json"),
        serde_json::to_vec_pretty(&guns)?,
    )?;
    fs::write(
        output.join("icons-069d.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"weapons": icons, "rounds": rounds}))?,
    )?;
    let ids: Vec<serde_json::Value> = port
        .weapons
        .iter()
        .map(|w| w.dict_id)
        .chain(spec.rounds.iter().map(|r| r.dict_id))
        .map(|id| serde_json::json!({"dict_id": id}))
        .collect();
    fs::write(output.join("equipment.json"), serde_json::to_vec_pretty(&ids)?)?;
    Ok(report)
}

/// Writes `path` into the overlay and its row of the port manifest's files.
fn write_file(overlay: &Path, manifest: &mut serde_json::Value, path: &str, bytes: &[u8]) -> Result<()> {
    let dest = crate::archive::safe_join(overlay, path)?;
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(&dest, bytes)?;
    let row = serde_json::to_value(crate::weapon::record_file(path, "converted:0.69d0", bytes))?;
    let files = manifest
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
        .context("port-manifest.json has no files")?;
    files.retain(|f| f.get("path").and_then(serde_json::Value::as_str) != Some(path));
    files.push(row);
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(())
}

/// Every clip an item config names that the bundle carries drives its rig's
/// bones: a hands clip 0.26g's player rig, an item clip the gun's model.
fn check_clips(cfg: &V, overlay: &Path, target: &mut ResourceDb) -> Result<()> {
    let read = |path: &str, target: &mut ResourceDb| -> Result<Vec<u8>> {
        let p = crate::archive::safe_join(overlay, path)?;
        if p.is_file() {
            Ok(fs::read(p)?)
        } else {
            target.read(path)
        }
    };
    let hands = clip_bones(&target.read(NATIVE_HANDS_CLIP)?)?;
    let model = string_of(get(cfg, "object.model"))?;
    let bind = bind_pose(&read(&format!("models/{model}.skinned_model/render/bind_pose"), target)?)?;
    let mut item: BTreeSet<String> = bind.into_iter().map(|(n, _)| n).collect();
    item.insert("object_movement".into());
    let mut strings = vec![];
    cfg.collect_strings(&mut strings);
    for clip in strings.into_iter().filter(|c| c.starts_with("resources/animations/")) {
        let path = clip.trim_start_matches("resources/");
        if !crate::archive::safe_join(overlay, &format!("{path}.b-spline"))?.is_file() {
            continue;
        }
        let bones = clip_bones(&read(&format!("{path}.bones_names"), target)?)?;
        if path.contains("/player/") {
            ensure!(bones == hands, "{path} drives other bones than 0.26g's player rig");
        } else {
            let set: BTreeSet<String> = bones.iter().cloned().collect();
            ensure!(set == item, "{path} drives {bones:?}, not the gun's {item:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
