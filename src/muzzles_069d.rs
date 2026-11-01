// SPDX-License-Identifier: GPL-3.0-or-later
//! Muzzle devices of 0.69d0 (flash hiders, muzzle brakes) for 0.26g's weapon
//! kit, and the guns that take them (`docs/muzzles-069d-026.md`; skhid-srp's
//! `docs/notes/muzzle-devices.md`; its client patch 90 draws them).
//!
//! A port is a manifest (`ports/muzzle-devices-069d.json`): the devices
//! (0.69d0 `weapon_modules` ids, our dict ids and prices) and the guns (our
//! item configs, each with its 0.69d0 weapon and the built-in device its
//! mesh has, if any). The rules, from the muzzle study of 2026-10-31:
//!
//! * **Our gun meshes are 0.69d0's.** A 0.26g gun mesh is 0.69d0's base mesh
//!   plus the stock modules, vertex for vertex, so 0.69d0's `muzzle_point`
//!   locator is our muzzle: the port adds it to every body of the gun
//!   (`export_properties`), on the bone 0.69d0 names, and the patch draws a
//!   device there.
//! * **The built-in device comes out of the mesh** by the removal rule
//!   ([`modules::cut_builtin`]): our triangles whose corners 0.69d0's base
//!   mesh has none of, inside the placed stock module. The cut part becomes
//!   the gun's stock device (a static model of our own triangles, with the
//!   gun's material and UVs, at the muzzle), drawn when the kit names no
//!   device, so a gun looks as before until a device goes on.
//! * **Checks** (the port fails otherwise): something is cut; the placed
//!   stock module lies on the cut part (its vertices' mean distance to the
//!   cut vertices within the gun's tolerance, 3 mm unless the manifest says
//!   why more); nothing of ours that the base lacks reaches more than 3 mm past
//!   the muzzle near the bore (a second device the cut missed; a barrel tip
//!   ours models a little longer, inside any device, is no device); our
//!   barrel reaches the muzzle (within 3 mm: else 0.69d0's barrel is
//!   another, and a device would float in front of ours). A bare-barrel gun has
//!   nothing past its muzzle either.
//! * **Devices** are 0.69d0's models as they are (authored in the gun
//!   mesh's orientation around the muzzle, +z forward), their materials made
//!   again on the native rifle material ([`weapons_069d::build_material`]),
//!   and an item each (category 22, `parameters.type` `srp_muzl`) whose
//!   modification adds `data.srp_muzzle_model` and `data.srp_muzzle_fire_mm`
//!   (the device's `fire_point`, where the shot's flash goes) and the
//!   device's stats: its two 0.69d0 drop modifications at grade 7 (the
//!   owner's call, 2026-11-01), each kept only where every gun it fits has
//!   that parameter (0.26g lacks the flash hiders' keys). A flash hider also
//!   sets the gun's shot effect to the manifest's (a native small flash).
//! * **Fits** are 0.69d0's own (`module_to_item_links`), by the gun's 0.69d0
//!   weapon (a premium skin follows its base gun: `fits_like`):
//!   `items_compatibility` rows.
//! * **Renders** (`renders/<gun>.png`): each gun's side view with its stock
//!   device, then its muzzle with the stock device and with each device it
//!   takes, to look at before a game does.
//!
//! Suppressors are never ported ([`modules::is_suppressor`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::json;

use crate::weapon::{parse_config, prepare_empty_output};
use crate::weapons_026::modules::{self, Bounds, Mesh};
use crate::weapons_069d::{
    self as w69, DICT, Game, Out, at, bind_pose, build_material, convert, field, int_of,
    list, locator_value, locators, lod0_surfaces, mesh_from_bind, s, string_of,
};
use crate::{ConfigValue as V, ResourceDb};

/// The device items' category: the scopes' (`categories_dict` 22), as the
/// lobby's attachment filter and the kit's server rules have it.
pub const CATEGORY: i32 = 22;
/// The device items' `parameters.type`: the server puts it in a weapon's
/// muzzle slot (logical slot 4).
pub const KIND: &str = "srp_muzl";
/// The grade of a device's drop modifications (0..=10 in 0.69d0).
pub const GRADE: &str = "7";
/// The stock template of a device's dictionary row (a category-22 item).
const ROW_TEMPLATE: &str = "shop_scope_pso_1";
const STATIC_SETTINGS: &str = "models/weapons/attach/leupold_3_7.model/settings";
const DEVICE_CONFIGS: &str = "gameplay/items/attachments/muzzle";
const STOCK_MODELS: &str = "weapons/v069d/muzzle_stock";
const MODIFICATIONS: &str = "gameplay/items/modifications/modifications.options";

/// The port's manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuzzlePort {
    pub format_version: u32,
    pub id: String,
    /// The shot effect a flash hider gives a gun (`particles.shoot`): a
    /// native 0.26g one.
    pub flash_hider_shoot: String,
    pub devices: Vec<DeviceSpec>,
    pub guns: Vec<GunSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    /// 0.69d0's `weapon_modules.<module>`.
    pub module: String,
    pub dict_id: i32,
    pub base_cost: i32,
    /// Its lobby icon: `[row, col]` of `LobbyMenu_AmmoIcons`, 1-based.
    pub icon: [i32; 2],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GunSpec {
    /// Our item config.
    pub config: String,
    /// Its 0.69d0 weapon (`weapons.<donor>`): the base mesh, `muzzle_point`.
    pub donor: String,
    /// The 0.69d0 module our mesh has built in, if any.
    #[serde(default)]
    pub stock: Option<String>,
    /// The 0.69d0 weapon whose fits it takes (a premium skin: its base
    /// gun's), else `donor`.
    #[serde(default)]
    pub fits_like: Option<String>,
    /// The placed stock module's mean distance to the cut, in mm, above the
    /// default 3, with the reason.
    #[serde(default)]
    pub fit_tolerance_mm: Option<f32>,
    #[serde(default)]
    pub note: Option<String>,
}

impl MuzzlePort {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let port: Self = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        ensure!(port.format_version == 1, "format_version 1");
        let mut ids = BTreeSet::new();
        let mut icons = BTreeSet::new();
        for d in &port.devices {
            ensure!(ids.insert(d.dict_id), "dict id {} twice", d.dict_id);
            ensure!(icons.insert(d.icon), "icon {:?} twice", d.icon);
            ensure!(
                !modules::is_suppressor(&d.module),
                "{}: suppressors are not ported (the owner, 2026-11-01)",
                d.module
            );
        }
        let mut configs = BTreeSet::new();
        for g in &port.guns {
            ensure!(configs.insert(&g.config), "{} twice", g.config);
        }
        Ok(port)
    }
}

/// A device kind: a flash hider is what 0.69d0 rolls an unmasking radius
/// on (its drop modifications; 0.69d0's groups and folders mislabel some),
/// else a brake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    FlashHider,
    Brake,
}

impl Kind {
    fn of(stats: &[(Vec<String>, f32, i32)]) -> Self {
        if stats.iter().any(|(path, _, _)| path.last().is_some_and(|k| k == "unmasking_radius")) {
            Self::FlashHider
        } else {
            Self::Brake
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::FlashHider => "flash_hider",
            Self::Brake => "muzzle_brake",
        }
    }
}

/// 0.26g's static vertex: position, normal/tangent/binormal words, half2 UV.
const STATIC_026: usize = 28;

fn f32_le(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The IEEE half nearest `v` (round half to even).
fn to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rest = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let up = rest > mid || (rest == mid && half & 1 == 1);
        return sign | (half + u32::from(up)) as u16;
    }
    let half = ((e as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    let up = rest > 0x1000 || (rest == 0x1000 && half & 1 == 1);
    sign | (half + u32::from(up)) as u16
}

/// A 0.69d0 static `converted_model` (28-byte vertices: position, three
/// packed basis floats, half2 UV; chunks 110/114 its statistics) in 0.26g's
/// static layout (the basis as words: the normal's top byte 127, the
/// others' 255, as 0.26g's static meshes). Positions and UVs as they are.
fn static_069_to_026(b: &[u8]) -> Result<Vec<u8>> {
    let chunks = convert::chunks(b)?;
    for (id, _) in &chunks {
        ensure!(
            matches!(id, 3 | 4 | 110 | 114),
            "unknown 0.69d static chunk {id}"
        );
    }
    let (vertices, stride) = convert::vertex_chunk(&chunks)?;
    ensure!(stride == 28, "not a 0.69d static mesh (stride {stride})");
    let n = vertices.len() / stride;
    let indices = convert::index_chunk(&chunks, n)?;
    let mut out = Vec::with_capacity(n * STATIC_026);
    for v in vertices.chunks_exact(stride) {
        out.extend_from_slice(&v[0..12]);
        for (k, alpha) in [0x7f, 0xff, 0xff].into_iter().enumerate() {
            let word = convert::basis_word(f32_le(v, 12 + 4 * k), alpha)?;
            out.extend(word.to_le_bytes());
        }
        out.extend_from_slice(&v[24..28]);
    }
    static_mesh(&out, &indices)
}

/// A 0.26g static `converted_model` of `vertices` (28 bytes each) and
/// `indices`.
fn static_mesh(vertices: &[u8], indices: &[u16]) -> Result<Vec<u8>> {
    ensure!(vertices.len().is_multiple_of(STATIC_026), "vertex bytes");
    let n = vertices.len() / STATIC_026;
    ensure!(n > 0 && n <= 65_536, "{n} vertices");
    ensure!(
        indices.len().is_multiple_of(3) && indices.iter().all(|&i| usize::from(i) < n),
        "indices"
    );
    let mut out = vec![];
    out.extend(3u32.to_le_bytes());
    out.extend(u32::try_from(4 + vertices.len())?.to_le_bytes());
    out.extend(u32::try_from(n)?.to_le_bytes());
    out.extend_from_slice(vertices);
    out.extend(4u32.to_le_bytes());
    out.extend(u32::try_from(4 + indices.len() * 2)?.to_le_bytes());
    out.extend(u32::try_from(indices.len())?.to_le_bytes());
    for i in indices {
        out.extend(i.to_le_bytes());
    }
    Ok(out)
}

/// Positions of a static 0.26g mesh.
fn static_positions(b: &[u8]) -> Result<Vec<[f32; 3]>> {
    Ok(Mesh::parse(b)?.positions)
}

/// Our skinned surface's triangles `tris` as a static mesh in the device's
/// frame (`origin` the muzzle in mesh space): the used vertices' positions
/// less `origin`, their basis words (top bytes 127, 255, 255), float2 UVs as
/// half2.
fn stock_mesh(ours: &Mesh, tris: &[[u16; 3]], origin: [f32; 3]) -> Result<(Vec<u8>, Vec<[f32; 3]>)> {
    // One bone (36 bytes) or two (40: the second bone and its weight
    // before the words): the words end 8 bytes from the end, the UVs fill it.
    ensure!(
        matches!(ours.stride, 36 | 40),
        "not a 0.26g skinned mesh (stride {})",
        ours.stride
    );
    let words = ours.stride - 20;
    let uv = ours.stride - 8;
    let mut index = BTreeMap::new();
    let mut vertices = vec![];
    let mut local = vec![];
    let mut indices = vec![];
    for t in tris {
        for &i in t {
            let next = u16::try_from(index.len())?;
            let k = *index.entry(i).or_insert_with(|| {
                let v = ours.vertex(usize::from(i));
                let p = [0, 1, 2].map(|c| f32_le(v, 4 * c) - origin[c]);
                for c in p {
                    vertices.extend(c.to_le_bytes());
                }
                for (w, alpha) in [words, words + 4, words + 8].into_iter().zip([0x7fu8, 0xff, 0xff]) {
                    vertices.extend_from_slice(&v[w..w + 3]);
                    vertices.push(alpha);
                }
                for c in [uv, uv + 4] {
                    vertices.extend(to_half(f32_le(v, c)).to_le_bytes());
                }
                local.push(p);
                next
            });
            indices.push(k);
        }
    }
    Ok((static_mesh(&vertices, &indices)?, local))
}

fn bounds_value(b: Bounds) -> V {
    V::Named(vec![
        field("max", V::Float3(b.max)),
        field("min", V::Float3(b.min)),
    ])
}

/// A static model's files: settings, export_properties, each surface's.
fn write_static_model(
    out: &mut Out,
    target: &mut ResourceDb,
    model: &str,
    source: &str,
    surfaces: &[(String, String, Vec<u8>)],
) -> Result<()> {
    let root = format!("models/{model}.model");
    let template = parse_config(target, STATIC_SETTINGS, "native static model settings")?;
    let mut settings = template.root;
    settings.set_path(
        "lod_hierrarchy",
        V::Named(vec![field(
            "LOD0",
            V::Named(vec![field(
                "surfaces",
                V::Indexed(surfaces.iter().map(|(n, _, _)| s(n)).collect()),
            )]),
        )]),
    )?;
    settings.set_path(
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
        settings.set_path(key, V::Boolean(false))?;
    }
    out.config(&format!("{root}/settings"), settings)?;
    let mut all = vec![];
    for (name, _, mesh) in surfaces {
        let p = static_positions(mesh)?;
        let b = Bounds::of(p.iter().copied()).context("an empty surface")?;
        let centre = [0, 1, 2].map(|k| (b.min[k] + b.max[k]) / 2.0);
        let radius = p
            .iter()
            .map(|v| (0..3).map(|k| (v[k] - centre[k]).powi(2)).sum::<f32>().sqrt())
            .fold(0.0, f32::max);
        let parsed = Mesh::parse(mesh)?;
        out.config(
            &format!("{root}/render/{name}/export_properties"),
            V::Named(vec![
                field(
                    "bounding_sphere",
                    V::Named(vec![
                        field("radius", V::Float(radius)),
                        field("center", V::Float3(centre)),
                    ]),
                ),
                field(
                    "source",
                    V::Named(vec![
                        field("indices", V::Integer(i32::try_from(parsed.triangles.len() * 3)?)),
                        field("primitives", V::Integer(i32::try_from(parsed.triangles.len())?)),
                        field("mesh_name", s(format!("|{name}"))),
                        field("texture_name", s("")),
                        field("vertices", V::Integer(i32::try_from(p.len())?)),
                    ]),
                ),
                field("type", V::Integer(0)),
                field("version", V::Integer(0)),
                field("bounding_box", bounds_value(b)),
            ]),
        )?;
        out.raw(&format!("{root}/render/{name}/converted_model"), mesh)?;
        all.extend(p);
    }
    let b = Bounds::of(all).context("no surfaces")?;
    out.config(
        &format!("{root}/render/export_properties"),
        V::Named(vec![
            field("src_filename", s(source)),
            field("type", V::Integer(1)),
            field("version", V::Integer(0)),
            field("bounding_box", bounds_value(b)),
        ]),
    )
}

/// A 0.69d0 module's model path (`weapons/modules/...`), its LOD0 surfaces
/// (name, material) and their meshes as they are.
fn donor_model(donor: &mut ResourceDb, model: &str) -> Result<Vec<(String, String, Vec<u8>)>> {
    let root = format!("models/{model}.model");
    let settings = w69::donor_config(donor, &format!("{root}/settings"))?;
    let mut out = vec![];
    for (surface, material) in lod0_surfaces(&settings.root, &root)? {
        let mesh = donor.read(&format!("{root}/render/{surface}/converted_model"))?;
        out.push((surface, material, mesh));
    }
    Ok(out)
}

/// A static model's locator position by name, from its export_properties.
fn static_locator(donor: &mut ResourceDb, model: &str, name: &str) -> Result<Option<[f32; 3]>> {
    let props = w69::donor_config(donor, &format!("models/{model}.model/render/export_properties"))?;
    Ok(locators(&props.root)?
        .into_iter()
        .find(|l| l.name == name)
        .map(|l| l.position))
}

/// A device's fire point along +z, in mm (its `fire_point`, within 6 mm of
/// the bore: the flash goes on the bore, where the patch puts it).
fn fire_mm(p: [f32; 3], what: &str) -> Result<i32> {
    ensure!(
        p[0].abs() < 0.006 && p[1].abs() < 0.006,
        "{what}: fire_point {p:?} is off the bore"
    );
    Ok((p[2] * 1000.0).round() as i32)
}

/// One 0.69d0 module the port makes a device of.
struct Device {
    spec: DeviceSpec,
    kind: Kind,
    /// Our model (`weapons/v069d/...`, a static model's request path).
    model: String,
    fire_mm: i32,
    /// 0.69d0 weapons it fits.
    links: BTreeSet<String>,
    /// Its two drop modifications at [`GRADE`]: (path, value, type).
    stats: Vec<(Vec<String>, f32, i32)>,
    /// Its mesh (all surfaces), for the renders.
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u16; 3]>,
    name_key: String,
    description_key: String,
    names: BTreeMap<String, String>,
}

impl Device {
    fn item_name(&self) -> String {
        format!("srp_muzzle_{}", self.spec.module)
    }
    fn config(&self) -> String {
        format!("{DEVICE_CONFIGS}/{}.options", self.item_name())
    }
    fn modification(&self) -> i32 {
        self.spec.dict_id * 100 + 1
    }
}

/// The modifiers of 0.69d0's `modifications` of `group` at [`GRADE`].
fn grade_modifiers(game: &Game, group: &str) -> Result<Vec<(Vec<String>, f32, i32)>> {
    for m in list(at(&game.root, "modifications")) {
        let (Some(g), Some(grade)) = (number_string(m.named("group")), number_string(m.named("grade"))) else {
            continue;
        };
        if g != group || grade != GRADE {
            continue;
        }
        let mut out = vec![];
        for modifier in list(m.named("modifiers")) {
            let path = list(modifier.named("path"))
                .iter()
                .map(|p| string_of(Some(p)).map(str::to_owned))
                .collect::<Result<Vec<_>>>()?;
            let value = match modifier.named("value") {
                Some(V::Float(f)) => *f,
                Some(V::Integer(i)) => *i as f32,
                other => bail!("modification {group}: value {other:?}"),
            };
            out.push((path, value, int_of(modifier.named("type"))?));
        }
        return Ok(out);
    }
    bail!("0.69d0 has no modification of group {group} at grade {GRADE}")
}

fn number_string(v: Option<&V>) -> Option<String> {
    match v? {
        V::Integer(i) => Some(i.to_string()),
        V::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// 0.69d0's drop infos of the module with dict id `dict`: its two drop
/// modification groups.
fn drop_groups(game: &Game, dict: &str) -> Result<Vec<String>> {
    for info in list(at(&game.root, "modules_drop_options.modules_additional_infos")) {
        if number_string(info.named("dict_id")).as_deref() == Some(dict) {
            return Ok(["first_drop_modifier", "second_drop_modifier"]
                .iter()
                .filter_map(|k| number_string(info.named(k)))
                .collect());
        }
    }
    Ok(vec![])
}

/// Every module's 0.69d0 weapons (`module_to_item_links`).
fn links(game: &Game) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for l in list(at(&game.root, "module_to_item_links")) {
        let module = string_of(l.named("module"))?.to_owned();
        let item = string_of(l.named("item"))?.to_owned();
        out.entry(module).or_default().insert(item);
    }
    Ok(out)
}

/// 0.69d0's strings of `key` in each language the build has of ours.
fn names(donor: &mut ResourceDb, key: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for lang in ["english", "german", "polish", "russian", "ukrainian"] {
        let loc = w69::donor_config(donor, &format!("localization/{lang}/localization"))?;
        if let Some(V::String(text)) = at(&loc.root, &format!("strings.{key}")) {
            out.insert(lang.to_owned(), text.clone());
        }
    }
    Ok(out)
}

fn build_device(
    spec: &DeviceSpec,
    game: &Game,
    all_links: &BTreeMap<String, BTreeSet<String>>,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
    out: &mut Out,
) -> Result<Device> {
    let module = game.module(&spec.module)?;
    let group = string_of(module.named("group"))?;
    ensure!(
        matches!(
            group,
            "compensator_h" | "compensator_u" | "compensator_v" | "flash_hider" | "firearrestor" | "muzzle"
        ),
        "{}: group {group} is no muzzle device",
        spec.module
    );
    let model = string_of(module.named("model"))?.to_owned();
    ensure!(!modules::is_suppressor(&model), "{model}: a suppressor");
    let ours = w69::ns_name(&model)?;
    let mut surfaces = vec![];
    let mut positions = vec![];
    let mut triangles = vec![];
    for (surface, material, mesh) in donor_model(donor, &model)? {
        build_material(&material, donor, target, out)
            .with_context(|| format!("{}: material {material}", spec.module))?;
        let converted = static_069_to_026(&mesh)
            .with_context(|| format!("{}: surface {surface}", spec.module))?;
        let m = Mesh::parse(&converted)?;
        let base = u16::try_from(positions.len())?;
        positions.extend(m.positions.iter().copied());
        triangles.extend(m.triangles.iter().map(|t| t.map(|i| i + base)));
        surfaces.push((surface, w69::ns_name(&material)?, converted));
    }
    // Two threads' devices may share a model (0.69d0's `_m14x1l` and
    // `_m16x1l` of one hider): written once.
    if !out.has(&format!("models/{ours}.model/settings")) {
        write_static_model(
            out,
            target,
            &ours,
            &format!("converted:0.69d0 models/{model}.model"),
            &surfaces,
        )?;
    }
    let fire = static_locator(donor, &model, "fire_point")?
        .with_context(|| format!("{model} has no fire_point"))?;
    let dict = number_string(at(module, "parameters.dict_id"))
        .with_context(|| format!("{}: no parameters.dict_id", spec.module))?;
    let groups = drop_groups(game, &dict)?;
    ensure!(
        !groups.is_empty(),
        "{}: no drop infos (a gun's default part, not a device players get)",
        spec.module
    );
    let mut stats = vec![];
    for group in groups {
        stats.extend(grade_modifiers(game, &group)?);
    }
    let name_key = string_of(at(module, "ui_desc.name"))?.to_owned();
    Ok(Device {
        spec: spec.clone(),
        kind: Kind::of(&stats),
        model: ours,
        fire_mm: fire_mm(fire, &spec.module)?,
        links: all_links.get(&spec.module).cloned().unwrap_or_default(),
        stats,
        positions,
        triangles,
        description_key: string_of(at(module, "ui_desc.description"))
            .unwrap_or_default()
            .to_owned(),
        names: names(donor, &name_key)?,
        name_key,
    })
}

/// One body of a gun: a skinned model of ours and its LOD0 surfaces.
#[derive(Clone)]
struct Body {
    root: String,
    surfaces: Vec<(String, String)>,
}

fn body(target: &mut ResourceDb, root: &str) -> Result<Body> {
    let settings = parse_config(target, &format!("{root}/settings"), "a skinned model's settings")?;
    Ok(Body {
        root: root.to_owned(),
        surfaces: lod0_surfaces(&settings.root, root)?,
    })
}

/// A surface's mesh, its kept triangles and its cut ones.
type CutSurface = (Mesh, Vec<[u16; 3]>, Vec<[u16; 3]>);
/// Positions and triangles to draw; with a colour.
type Piece = (Vec<[f32; 3]>, Vec<[u16; 3]>);
type Coloured = (Vec<[f32; 3]>, Vec<[u16; 3]>, [u8; 3]);

/// What the removal rule made of one gun's mesh.
struct Cut {
    /// Per surface (by its mesh's bytes' first path): the kept triangles,
    /// the cut ones.
    surfaces: BTreeMap<String, CutSurface>,
    muzzle: [f32; 3],
    report: serde_json::Value,
}

/// A gun's report and output of the port.
struct GunOut {
    name: String,
    dict_id: i32,
    devices: Vec<usize>,
}

fn sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

/// The skinned models of `prefix` by the hashes of their LOD0 surfaces'
/// meshes.
fn index_bodies(target: &mut ResourceDb, prefix: &str) -> Result<Vec<(Body, Vec<String>)>> {
    let roots: Vec<String> = target
        .paths_with_prefix(prefix)
        .filter_map(|p| p.strip_suffix("/settings"))
        .filter(|p| p.ends_with(".skinned_model"))
        .map(str::to_owned)
        .collect();
    let mut out = vec![];
    for root in roots {
        let Ok(b) = body(target, &root) else { continue };
        let mut hashes = vec![];
        let mut ok = true;
        for (surface, _) in &b.surfaces {
            match target.read(&format!("{root}/render/{surface}/converted_model")) {
                Ok(bytes) => hashes.push(sha(&bytes)),
                Err(_) => ok = false,
            }
        }
        if ok {
            out.push((b, hashes));
        }
    }
    Ok(out)
}

/// Positions of every mesh under a 0.69d0 skinned model's render folder.
fn donor_base(donor: &mut ResourceDb, root: &str) -> Result<Vec<[f32; 3]>> {
    let paths: Vec<String> = donor
        .paths_with_prefix(&format!("{root}/render/"))
        .filter(|p| p.ends_with("/converted_model"))
        .map(str::to_owned)
        .collect();
    ensure!(!paths.is_empty(), "{root} has no meshes");
    let mut out = vec![];
    for p in paths {
        out.extend(Mesh::parse(&donor.read(&p)?)?.positions);
    }
    Ok(out)
}

fn key(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c * 1e4).round() as i32)
}

fn cut_gun(
    gun: &GunSpec,
    body: &Body,
    game: &Game,
    donor: &mut ResourceDb,
    target: &mut ResourceDb,
) -> Result<Cut> {
    let w = game.weapon(&gun.donor)?;
    let donor_root = format!("models/{}.skinned_model", string_of(at(w, "object.model"))?);
    let props = w69::donor_config(donor, &format!("{donor_root}/render/export_properties"))?;
    let mp = locators(&props.root)?
        .into_iter()
        .find(|l| l.name == "muzzle_point")
        .with_context(|| format!("{donor_root} has no muzzle_point"))?;
    ensure!(
        mp.rotation.iter().all(|r| r.abs() < 0.01),
        "{}: muzzle_point is turned {:?}",
        gun.donor,
        mp.rotation
    );
    let bind = bind_pose(&donor.read(&format!("{donor_root}/render/bind_pose"))?)?;
    let ours_bind = bind_pose(&target.read(&format!("{}/render/bind_pose", body.root))?)?;
    let t = bind
        .iter()
        .find(|(n, _)| *n == mp.bone)
        .with_context(|| format!("{}: no bone {}", gun.donor, mp.bone))?
        .1;
    let ours_t = ours_bind
        .iter()
        .find(|(n, _)| *n == mp.bone)
        .with_context(|| format!("{}: our rig has no bone {}", body.root, mp.bone))?
        .1;
    // Where our rig binds the bone elsewhere (a break-action's hinge), the
    // locator is written in our bone's frame for the same mesh point.
    let bind_offset_mm = (0..3).map(|k| (t[k] - ours_t[k]).powi(2)).sum::<f32>().sqrt() * 1000.0;
    let p = mesh_from_bind(mp.position);
    let muzzle = [t[0] + p[0], t[1] + p[1], t[2] + p[2]];
    let base = donor_base(donor, &donor_root)?;
    let known: BTreeSet<[i32; 3]> = base.iter().map(|p| key(*p)).collect();
    let placed_stock = match &gun.stock {
        Some(id) => {
            let module = game.module(id)?;
            let model = match at(module, &format!("exceptions.{}.model", gun.donor)) {
                Some(v) => string_of(Some(v))?.to_owned(),
                None => string_of(module.named("model"))?.to_owned(),
            };
            let mut placed = vec![];
            for (_, _, mesh) in donor_model(donor, &model)? {
                let m = Mesh::parse(&mesh)?;
                placed.extend(modules::placed(&m, muzzle));
            }
            Some((model, placed))
        }
        None => None,
    };
    let mut surfaces = BTreeMap::new();
    let mut cut_total = 0;
    let mut extra = 0;
    let (mut extra_z, mut extra_r) = ((f32::MAX, f32::MIN), 0.0f32);
    let mut cut_points = vec![];
    for (surface, _) in &body.surfaces {
        let path = format!("{}/render/{surface}/converted_model", body.root);
        let mesh = Mesh::parse(&target.read(&path)?)?;
        let (kept, cut) = match &placed_stock {
            Some((_, placed)) => {
                let (kept, n) = modules::cut_builtin(&mesh, &base, placed);
                let kept_set: BTreeSet<[u16; 3]> = kept.iter().copied().collect();
                let mut cut: Vec<[u16; 3]> = mesh
                    .triangles
                    .iter()
                    .filter(|t| !kept_set.contains(*t))
                    .copied()
                    .collect();
                ensure!(cut.len() == n, "{path}: duplicate triangles");
                // The seam: a triangle of the device sharing a corner with
                // the base (the removal rule keeps it), inside the device's
                // box and past the muzzle, goes with the device.
                let device_box = Bounds::of(placed.iter().copied())
                    .context("an empty stock module")?
                    .grown(0.003);
                let (seam, kept): (Vec<[u16; 3]>, Vec<[u16; 3]>) = kept.into_iter().partition(|t| {
                    let pts = t.map(|i| mesh.positions[usize::from(i)]);
                    let c = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f32>() / 3.0);
                    pts.iter().any(|p| !known.contains(&key(*p)))
                        && device_box.contains(c)
                        && pts.iter().map(|p| p[2]).fold(f32::MIN, f32::max) > muzzle[2] + 0.001
                });
                cut.extend(seam);
                (kept, cut)
            }
            None => (mesh.triangles.clone(), vec![]),
        };
        cut_total += cut.len();
        for t in &cut {
            cut_points.extend(t.iter().map(|&i| mesh.positions[usize::from(i)]));
        }
        for t in &kept {
            let pts = t.map(|i| mesh.positions[usize::from(i)]);
            if pts.iter().all(|p| known.contains(&key(*p))) {
                continue;
            }
            let c = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f32>() / 3.0);
            let past = pts.iter().map(|p| p[2]).fold(f32::MIN, f32::max) > muzzle[2] + 0.003;
            if (c[0] - muzzle[0]).abs() < 0.03 && (c[1] - muzzle[1]).abs() < 0.03 && past {
                extra += 1;
                for p in pts {
                    extra_z.0 = extra_z.0.min(p[2] - muzzle[2]);
                    extra_z.1 = extra_z.1.max(p[2] - muzzle[2]);
                    extra_r = extra_r.max(((p[0] - muzzle[0]).powi(2) + (p[1] - muzzle[1]).powi(2)).sqrt());
                }
            }
        }
        surfaces.insert(surface.clone(), (mesh, kept, cut));
    }
    ensure!(
        extra == 0,
        "{}: {extra} triangles of ours the 0.69d0 base lacks are left past the muzzle (a device the cut missed; \
         {:.0}..{:.0} mm along the bore from the muzzle, within {:.0} mm of it)",
        gun.config,
        extra_z.0 * 1000.0,
        extra_z.1 * 1000.0,
        extra_r * 1000.0
    );
    // Our barrel's end: the farthest point of what stays near the bore.
    let tip = surfaces
        .values()
        .flat_map(|(mesh, kept, _): &CutSurface| {
            kept.iter().flat_map(move |t| t.iter().map(move |&i| mesh.positions[usize::from(i)]))
        })
        .filter(|p| (p[0] - muzzle[0]).abs() < 0.015 && (p[1] - muzzle[1]).abs() < 0.015)
        .map(|p| p[2])
        .fold(f32::MIN, f32::max);
    let tip_mm = (tip - muzzle[2]) * 1000.0;
    ensure!(
        tip_mm >= -3.0,
        "{}: our barrel ends {:.0} mm short of 0.69d0's muzzle_point (another barrel): a device would float",
        gun.config,
        -tip_mm
    );
    let mut report = json!({
        "barrel_end_mm": (tip_mm * 10.0).round() / 10.0,
        "config": gun.config,
        "body": body.root,
        "donor": gun.donor,
        "muzzle_point": mp.position,
        "bone": mp.bone,
        "muzzle_mesh": muzzle,
        "cut_triangles": cut_total,
        "bind_offset_mm": (bind_offset_mm * 100.0).round() / 100.0,
    });
    if let Some((model, placed)) = &placed_stock {
        ensure!(cut_total > 0, "{}: the stock {model} cuts nothing", gun.config);
        let (mut sum, mut max) = (0.0f32, 0.0f32);
        for p in placed {
            let d = cut_points
                .iter()
                .map(|c| (0..3).map(|k| (p[k] - c[k]).powi(2)).sum::<f32>())
                .fold(f32::MAX, f32::min)
                .sqrt();
            sum += d;
            max = max.max(d);
        }
        let mean_mm = 1000.0 * sum / placed.len() as f32;
        let tolerance = gun.fit_tolerance_mm.unwrap_or(3.0);
        ensure!(
            mean_mm <= tolerance,
            "{}: the stock {model} lies {mean_mm:.2} mm (mean) off the cut, over {tolerance} mm",
            gun.config
        );
        report["stock_module"] = json!(model);
        report["fit_mean_mm"] = json!((mean_mm * 100.0).round() / 100.0);
        report["fit_max_mm"] = json!((max * 100_000.0).round() / 100.0);
    }
    Ok(Cut {
        surfaces,
        muzzle,
        report,
    })
}

/// The port: a bundle (`overlay/`, `port-manifest.json`), its edit list
/// (`muzzle-edits.json`: the guns' stock devices in their configs), the
/// device texts (`texts.json`) and the renders (`renders/`).
#[allow(clippy::too_many_lines)]
pub fn build(port: &MuzzlePort, donor_db: &Path, target_layers: &[PathBuf], output: &Path) -> Result<()> {
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let renders = output.join("renders");
    fs::create_dir_all(&renders)?;
    let mut target = ResourceDb::open_layers(target_layers)?;
    let mut donor = ResourceDb::open(donor_db)?;
    let game = Game::load(&mut donor)?;
    let all_links = links(&game)?;
    let work = output.join("work");
    let mut out = Out {
        overlay: &overlay,
        work: &work,
        files: vec![],
        written: BTreeSet::new(),
    };
    let mut devices = vec![];
    for spec in &port.devices {
        devices.push(
            build_device(spec, &game, &all_links, &mut donor, &mut target, &mut out)
                .with_context(|| format!("device {}", spec.module))?,
        );
    }
    let mut dictionary = parse_config(&mut target, DICT, "0.26g dictionary")?;
    let items = list(dictionary.root.named("items_dict")).to_vec();
    let dict_of = |cfg: &str| -> Result<(i32, String)> {
        let row = items
            .iter()
            .find(|r| r.named("cfg_name") == Some(&s(cfg)))
            .with_context(|| format!("no items_dict row for {cfg}"))?;
        Ok((int_of(row.named("dict_id"))?, string_of(row.named("name"))?.to_owned()))
    };
    let bodies = index_bodies(&mut target, "models/weapons/")?;
    let mut cuts: BTreeMap<String, Cut> = BTreeMap::new();
    // A body another gun's cut made (the same meshes): that cut's key.
    let mut group_of: BTreeMap<String, String> = BTreeMap::new();
    let mut written_bodies = BTreeSet::new();
    let mut guns = vec![];
    let mut edits = vec![];
    let mut reports = vec![];
    let mut gun_configs: BTreeMap<i32, V> = BTreeMap::new();
    for gun in &port.guns {
        let cfg = parse_config(&mut target, &gun.config, "a gun's config")?;
        let model = string_of(at(&cfg.root, "object.model"))?.to_owned();
        let root = format!("models/{model}.skinned_model");
        let main = body(&mut target, &root)?;
        let (dict_id, name) = dict_of(&gun.config)?;
        let group = group_of.get(&root).cloned().unwrap_or_else(|| root.clone());
        if !cuts.contains_key(&group) {
            let cut = cut_gun(gun, &main, &game, &mut donor, &mut target)
                .with_context(|| format!("gun {}", gun.config))?;
            // Every body whose meshes are these (scope variants, skins of
            // the same mesh): the same cut, its own muzzle_point.
            let mut hashes = vec![];
            for (sname, _) in &main.surfaces {
                hashes.push(sha(&target.read(&format!("{root}/render/{sname}/converted_model"))?));
            }
            let mut same: Vec<Body> = bodies
                .iter()
                .filter(|(b, h)| *h == hashes && b.surfaces.iter().map(|x| &x.0).eq(main.surfaces.iter().map(|x| &x.0)))
                .map(|(b, _)| b.clone())
                .collect();
            if !same.iter().any(|b| b.root == root) {
                same.push(main.clone());
            }
            for b in &same {
                ensure!(written_bodies.insert(b.root.clone()), "{} is two guns' body", b.root);
                group_of.insert(b.root.clone(), root.clone());
                for (sname, _) in &b.surfaces {
                    let (mesh, kept, cut_tris) = &cut.surfaces[sname];
                    if !cut_tris.is_empty() {
                        out.raw(
                            &format!("{}/render/{sname}/converted_model", b.root),
                            &mesh.with_triangles(kept),
                        )?;
                    }
                }
                let props_path = format!("{}/render/export_properties", b.root);
                let mut props = parse_config(&mut target, &props_path, "a body's export_properties")?;
                let mut ls = locators(&props.root)?;
                ensure!(
                    !ls.iter().any(|l| l.name == "muzzle_point"),
                    "{props_path} has a muzzle_point already"
                );
                let w = game.weapon(&gun.donor)?;
                let donor_root = format!("models/{}.skinned_model", string_of(at(w, "object.model"))?);
                let dprops = w69::donor_config(&mut donor, &format!("{donor_root}/render/export_properties"))?;
                let mut mp = locators(&dprops.root)?
                    .into_iter()
                    .find(|l| l.name == "muzzle_point")
                    .context("muzzle_point")?;
                // A locator's bone_idx is its bone's place in the bind pose
                // (checked on the body's own locators).
                let order = bind_pose(&target.read(&format!("{}/render/bind_pose", b.root))?)?;
                let idx = |bone: &str| order.iter().position(|(n, _)| n == bone);
                for l in ls.iter().filter(|l| !l.bone.is_empty()) {
                    ensure!(
                        idx(&l.bone).and_then(|i| i32::try_from(i).ok()) == Some(l.bone_idx),
                        "{props_path}: {}'s bone_idx {} is not {}'s place in the bind pose",
                        l.name,
                        l.bone_idx,
                        l.bone
                    );
                }
                mp.bone_idx = i32::try_from(
                    idx(&mp.bone).with_context(|| format!("{props_path}: no bone {}", mp.bone))?,
                )?;
                let body_t = order
                    .iter()
                    .find(|(n, _)| *n == mp.bone)
                    .context("the muzzle bone")?
                    .1;
                mp.position = mesh_from_bind([0, 1, 2].map(|k| cut.muzzle[k] - body_t[k]));
                ls.push(mp);
                props
                    .root
                    .set_path("locators", V::Indexed(ls.iter().map(locator_value).collect()))?;
                out.config(&props_path, props.root)?;
            }
            let mut report = cut.report.clone();
            report["bodies"] = json!(same.iter().map(|b| b.root.clone()).collect::<Vec<_>>());
            reports.push(report);
            cuts.insert(root.clone(), cut);
        }
        let cut = &cuts[&group];
        // The stock device: our cut triangles, each surface's material.
        if gun.stock.is_some() {
            let mut surfaces = vec![];
            let mut front = f32::MIN;
            for (sname, material) in &main.surfaces {
                let (mesh, _, tris) = &cut.surfaces[sname];
                if tris.is_empty() {
                    continue;
                }
                let (bytes, local) =
                    stock_mesh(mesh, tris, cut.muzzle).with_context(|| format!("{}: {sname}", gun.config))?;
                front = local.iter().map(|p| p[2]).fold(front, f32::max);
                surfaces.push((sname.clone(), material.clone(), bytes));
            }
            let stock_model = format!("{STOCK_MODELS}/{name}");
            write_static_model(
                &mut out,
                &mut target,
                &stock_model,
                &format!("converted: the built-in device of {root}"),
                &surfaces,
            )?;
            let module = game.module(gun.stock.as_deref().unwrap())?;
            let model = match at(module, &format!("exceptions.{}.model", gun.donor)) {
                Some(v) => string_of(Some(v))?.to_owned(),
                None => string_of(module.named("model"))?.to_owned(),
            };
            let fire = match static_locator(&mut donor, &model, "fire_point")? {
                Some(p) => fire_mm(p, &model)?,
                None => (front * 1000.0).round() as i32,
            };
            edits.push(json!({"files": [gun.config], "table": "data", "key": "srp_muzzle_stock_model", "value": stock_model}));
            edits.push(json!({"files": [gun.config], "table": "data", "key": "srp_muzzle_stock_fire_mm", "value": fire}));
        }
        let like = gun.fits_like.as_deref().unwrap_or(&gun.donor);
        let fits: Vec<usize> = devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.links.contains(like))
            .map(|(i, _)| i)
            .collect();
        gun_configs.insert(dict_id, cfg.root.clone());
        guns.push(GunOut {
            name,
            dict_id,
            devices: fits,
        });
        let _ = &gun.note;
    }
    // Items: each device's config and dictionary row, its fits' rows.
    let template = items
        .iter()
        .find(|r| r.named("name") == Some(&s(ROW_TEMPLATE)))
        .with_context(|| format!("no {ROW_TEMPLATE} row"))?
        .clone();
    let native_mods = parse_config(&mut target, MODIFICATIONS, "the native modifications")?;
    let prop_of = |path: &[String]| -> Option<i32> {
        list(native_mods.root.named("modifications")).iter().find_map(|m| {
            let mp: Vec<String> = list(at(m, "modifiers.0.path"))
                .iter()
                .filter_map(|p| string_of(Some(p)).ok().map(str::to_owned))
                .collect();
            (mp == path).then(|| int_of(at(m, "ui_desc.props_list.0.prop_id")).ok())?
        })
    };
    let native_ids: BTreeSet<i32> = list(native_mods.root.named("modifications"))
        .iter()
        .chain(list(native_mods.root.named("defects")))
        .filter_map(|m| int_of(m.named("id")).ok())
        .collect();
    let mut new_rows = vec![];
    let mut texts = serde_json::Map::new();
    let mut device_reports = vec![];
    for (i, d) in devices.iter().enumerate() {
        let fitting: Vec<&GunOut> = guns.iter().filter(|g| g.devices.contains(&i)).collect();
        ensure!(!fitting.is_empty(), "{} fits none of the guns", d.spec.module);
        ensure!(!native_ids.contains(&d.modification()), "modification {} is taken", d.modification());
        let mut modifiers = vec![
            json_modifier(&["data"], V::Indexed(vec![s("srp_muzzle_model"), s(&d.model)]), 3),
            json_modifier(&["data"], V::Indexed(vec![s("srp_muzzle_fire_mm"), V::Integer(d.fire_mm)]), 3),
        ];
        let mut props = vec![];
        let mut kept = vec![];
        let mut dropped = vec![];
        for (path, value, kind) in &d.stats {
            let effective = effective_modifiers(path, *value, *kind);
            let every = effective.iter().all(|(p, _, _)| {
                fitting
                    .iter()
                    .all(|g| at(&gun_configs[&g.dict_id], &p.join(".")).is_some())
            });
            if !every {
                dropped.push(path.join("/"));
                continue;
            }
            for (p, v, k) in &effective {
                modifiers.push(json_modifier(
                    &p.iter().map(String::as_str).collect::<Vec<_>>(),
                    V::Float(*v),
                    *k,
                ));
            }
            if let Some(id) = prop_of(path) {
                props.push(V::Named(vec![
                    field("prop_value", s(format!("{value:.2}"))),
                    field("prop_id", V::Integer(id)),
                ]));
            }
            kept.push(json!({"path": path.join("/"), "value": value, "type": kind,
                "as": effective.iter().map(|(p, v, k)| json!({"path": p.join("/"), "value": v, "type": k})).collect::<Vec<_>>()}));
        }
        if d.kind == Kind::FlashHider {
            modifiers.push(json_modifier(&["particles", "shoot"], s(&port.flash_hider_shoot), 0));
        }
        let name_key = format!("st_{}_name", d.item_name());
        let description_key = format!("st_{}_description", d.item_name());
        let cfg = V::Named(vec![
            field(
                "parameters",
                V::Named(vec![
                    field("weight", V::Float(0.0)),
                    field("type", s(KIND)),
                    field("modification", V::Integer(d.modification())),
                ]),
            ),
            field(
                "modifications",
                V::Indexed(vec![V::Named(vec![
                    field("id", V::Integer(d.modification())),
                    field("modifiers", V::Indexed(modifiers)),
                ])]),
            ),
            field(
                "ui_desc",
                V::Named(vec![
                    field(
                        "text_descriptions",
                        V::Named(vec![
                            field("name", s(&name_key)),
                            field("description", s(&description_key)),
                        ]),
                    ),
                    field("icon", V::Indexed(d.spec.icon.iter().map(|&c| V::Integer(c)).collect())),
                    field("props_list", V::Indexed(props)),
                ]),
            ),
        ]);
        out.config(&d.config(), cfg)?;
        let mut row = template.clone();
        row.set_path("cfg_name", s(d.config()))?;
        row.set_path("name", s(d.item_name()))?;
        row.set_path("dict_id", V::Integer(d.spec.dict_id))?;
        row.set_path("base_cost", V::Integer(d.spec.base_cost))?;
        row.set_path("is_premium", V::Boolean(false))?;
        row.set_path("item_category", V::Integer(CATEGORY))?;
        new_rows.push(row);
        texts.insert(
            d.item_name(),
            json!({"name_key": name_key, "description_key": description_key, "donor_key": d.name_key,
                   "donor_description_key": d.description_key, "names": d.names, "kind": d.kind.name()}),
        );
        device_reports.push(json!({
            "module": d.spec.module, "dict_id": d.spec.dict_id, "item": d.item_name(), "kind": d.kind.name(),
            "model": d.model, "fire_mm": d.fire_mm, "base_cost": d.spec.base_cost, "icon": d.spec.icon,
            "stats": kept, "stats_left_out": dropped,
            "guns": fitting.iter().map(|g| g.name.clone()).collect::<Vec<_>>(),
        }));
    }
    for row in &new_rows {
        for key in ["dict_id", "name", "cfg_name"] {
            let value = row.named(key).context("row field")?;
            ensure!(
                !items.iter().any(|r| r.named(key) == Some(value)),
                "{key} {value:?} is taken"
            );
        }
    }
    let Some(V::Indexed(rows)) = dictionary.root.named_mut("items_dict") else {
        bail!("items_dict is not a list")
    };
    rows.extend(new_rows);
    let Some(V::Named(compat)) = dictionary.root.named_mut("items_compatibility") else {
        bail!("items_compatibility is not a table")
    };
    let mut pairs = vec![];
    for g in &guns {
        for &i in &g.devices {
            let name = format!("compatibility_muzzle_{}_{}", g.dict_id, devices[i].spec.dict_id);
            ensure!(!compat.iter().any(|c| c.name == name), "{name} is taken");
            compat.push(field(
                &name,
                V::Named(vec![
                    field("second_item", V::Integer(devices[i].spec.dict_id)),
                    field("first_item", V::Integer(g.dict_id)),
                ]),
            ));
            pairs.push(name);
        }
    }
    out.config(DICT, dictionary.root)?;
    // The renders: each gun's side, then its muzzle with each device.
    for (gun, g) in port.guns.iter().zip(&guns) {
        let cfg = parse_config(&mut target, &gun.config, "a gun's config")?;
        let root = format!("models/{}.skinned_model", string_of(at(&cfg.root, "object.model"))?);
        let cut = &cuts[group_of.get(&root).unwrap_or(&root)];
        let png = render_gun(cut, &devices, &g.devices)?;
        fs::write(renders.join(format!("{}.png", g.name)), png)?;
    }
    if work.exists() {
        fs::remove_dir(&work).context("the encoder left files behind")?;
    }
    let mut files = out.files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "format_version": 1,
            "asset_id": port.id,
            "files": files,
            "donor_db": donor_db,
            "target": target_layers,
            "devices": device_reports,
            "guns": reports,
            "fits": guns.iter().map(|g| json!({"gun": g.name, "dict_id": g.dict_id,
                "devices": g.devices.iter().map(|&i| devices[i].spec.dict_id).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "compatibility": pairs,
        }))?,
    )?;
    fs::write(
        output.join("muzzle-edits.json"),
        serde_json::to_vec_pretty(&json!({ "add_value": edits }))?,
    )?;
    fs::write(output.join("texts.json"), serde_json::to_vec_pretty(&texts)?)?;
    crate::model_validation::validate_overlay_on(&mut target, &overlay)?;
    println!(
        "Built {}: {} devices, {} guns, {} files",
        port.id,
        devices.len(),
        guns.len(),
        files.len()
    );
    Ok(())
}

/// A 0.69d0 stat as the 0.26g keys that take effect on a weapon (as
/// skhid-srp's weapon-rolls-1 does for the stock rolls,
/// docs/notes/weapon-rolls.md): `recoil_modifier` and `aim_time_modifier`
/// are a player's modifiers, which a weapon never registers, so the recoil
/// goes on the six kick bounds the recoil loader reads and the aim time on
/// the weapon's own, by the same fraction (type 2). Any other key as it is.
fn effective_modifiers(path: &[String], value: f32, kind: i32) -> Vec<(Vec<String>, f32, i32)> {
    let owned = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    match (path.iter().map(String::as_str).collect::<Vec<_>>().as_slice(), kind) {
        (["parameters", "recoil_modifier"], 1) => [
            "min_top_recoil",
            "max_top_recoil",
            "min_left_recoil",
            "max_left_recoil",
            "min_right_recoil",
            "max_right_recoil",
        ]
        .iter()
        .map(|k| (owned(&["recoil", k]), value, 2))
        .collect(),
        (["parameters", "aim_time_modifier"], 1) => vec![(owned(&["parameters", "aim_time"]), value, 2)],
        _ => vec![(path.to_vec(), value, kind)],
    }
}

fn json_modifier(path: &[&str], value: V, kind: i32) -> V {
    V::Named(vec![
        field("path", V::Indexed(path.iter().map(|p| s(*p)).collect())),
        field("value", value),
        field("type", V::Integer(kind)),
    ])
}

/// Stacks RGBA images (width, height, pixels) one under another, on a dark
/// ground, each centred.
fn stack(images: &[(usize, usize, Vec<u8>)]) -> (usize, usize, Vec<u8>) {
    let width = images.iter().map(|i| i.0).max().unwrap_or(1);
    let gap = 6;
    let height = images.iter().map(|i| i.1 + gap).sum::<usize>().max(1);
    let mut out = vec![0u8; width * height * 4];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&[24, 24, 28, 255]);
    }
    let mut y0 = 0;
    for (w, h, rgba) in images {
        let x0 = (width - w) / 2;
        for y in 0..*h {
            for x in 0..*w {
                let src = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                if src[3] != 0 {
                    let at = ((y0 + y) * width + x0 + x) * 4;
                    out[at..at + 4].copy_from_slice(src);
                }
            }
        }
        y0 += h + gap;
    }
    (width, height, out)
}

/// A gun's verification render: its whole side with the stock device
/// (orange), then the muzzle (the last 18 cm of barrel) bare, with the
/// stock device and with each device it takes (blue), all at the scale of
/// the muzzle views.
fn render_gun(cut: &Cut, devices: &[Device], fits: &[usize]) -> Result<Vec<u8>> {
    const GUN: [u8; 3] = [170, 170, 165];
    const STOCK: [u8; 3] = [235, 140, 40];
    const DEVICE: [u8; 3] = [80, 150, 235];
    let m = cut.muzzle;
    let mut gun_parts = vec![];
    let mut stock_parts = vec![];
    for (mesh, kept, cut_tris) in cut.surfaces.values() {
        gun_parts.push((mesh.positions.clone(), kept.clone()));
        if !cut_tris.is_empty() {
            stock_parts.push((mesh.positions.clone(), cut_tris.clone()));
        }
    }
    let as_parts = |list: &[Piece], colour| -> Vec<Coloured> {
        list.iter().map(|(p, t)| (p.clone(), t.clone(), colour)).collect()
    };
    let draw = |parts: Vec<Coloured>, width: usize, b: Option<Bounds>| {
        let parts: Vec<modules::Part> = parts
            .iter()
            .filter(|(_, t, _)| !t.is_empty())
            .map(|(p, t, c)| (p.as_slice(), t.as_slice(), *c))
            .collect();
        match b {
            Some(b) => modules::render_side_in(&parts, width, b),
            None => {
                let used = parts.iter().flat_map(|(p, t, _)| {
                    t.iter().flat_map(move |tri| tri.iter().map(move |&i| p[usize::from(i)]))
                });
                modules::render_side_in(&parts, width, Bounds::of(used).context("nothing to render")?)
            }
        }
    };
    let mut images = vec![];
    let mut whole = as_parts(&gun_parts, GUN);
    whole.extend(as_parts(&stock_parts, STOCK));
    images.push(draw(whole, 1400, None)?);
    // The muzzle views share one box: 12 cm of barrel behind the muzzle to
    // the longest device's end, so each is at the same scale and place.
    let mut front = m[2] + 0.02;
    let mut front_of = |p: &[[f32; 3]], t: &[[u16; 3]]| {
        for tri in t {
            for &i in tri {
                front = front.max(p[usize::from(i)][2]);
            }
        }
    };
    for (p, t) in &stock_parts {
        front_of(p, t);
    }
    let placed: Vec<Vec<[f32; 3]>> = fits
        .iter()
        .map(|&i| devices[i].positions.iter().map(|p| [p[0] + m[0], p[1] + m[1], p[2] + m[2]]).collect())
        .collect();
    for (k, &i) in fits.iter().enumerate() {
        front_of(&placed[k], &devices[i].triangles);
    }
    let b = Bounds {
        min: [m[0] - 0.1, m[1] - 0.045, m[2] - 0.12],
        max: [m[0] + 0.1, m[1] + 0.045, front + 0.01],
    };
    let view = |extra: Vec<Coloured>| -> Result<(usize, usize, Vec<u8>)> {
        let mut parts = as_parts(&gun_parts, GUN);
        parts.extend(extra);
        draw(parts, 900, Some(b))
    };
    images.push(view(vec![])?);
    images.push(view(as_parts(&stock_parts, STOCK))?);
    for (k, &i) in fits.iter().enumerate() {
        images.push(view(vec![(placed[k].clone(), devices[i].triangles.clone(), DEVICE)])?);
    }
    let (w, h, rgba) = stack(&images);
    modules::png(w, h, &rgba)
}

#[cfg(test)]
mod tests;
