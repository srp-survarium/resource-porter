// SPDX-License-Identifier: GPL-3.0-or-later
//! The STK2 exoskeleton sets: Duty (body 1805, helmet 1800) and its Freedom
//! repaint (body 1807, helmet 1808), each a [`Variant`] with its own
//! namespace, items, faction template and protection. The six pre-split Duty
//! models use the native 69-bone rig, rotated 180° around Y. Equipment is
//! additive; native 0.26 item/shader schemas are retained.
use crate::localization_026::{LANGUAGES, Text, check_item_texts, set_string};
use crate::manifest::{BuiltFile, TextureCompression};
use crate::weapon::{parse_config, prepare_empty_output, record_file, write_generated_config};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const BASE: &str = "models/character/human/base/base_character_boot.skinned_model";
const DICT: &str = "gameplay/db_static_dictionaries";
const MATERIAL: &str =
    "material_instances/characters/human/renesanse_tank/renesanse_tank_torso.material";
/// A torso's sleeve variants, which a gloves item's `data.model_type` picks.
const TORSO_TABLES: [&str; 3] = ["data_full", "data_long", "data_short"];
/// The player's hit volumes; their `hit_param`s are every region a hit lands on.
const HIT_TARGETS: &str =
    "models/character/human/base/base_character_full.skinned_model/hit_targets";

/// The regions of a config's `hit_params`.
fn regions(cfg: &V) -> Result<BTreeSet<String>> {
    let Some(V::Named(parts)) = cfg.named("hit_params") else {
        bail!("hit parameters missing");
    };
    Ok(parts.iter().map(|part| part.name.clone()).collect())
}

/// Every region of the player's hit table is protected once across the
/// set's pieces, with every hit type: the helmet brain and face, the gloves
/// the arms, the boots the feet, the body everything else (the native
/// torso's and trousers' regions).
fn check_coverage(hit_targets: &V, pieces: &[&V]) -> Result<()> {
    let Some(V::Indexed(targets)) = hit_targets.named("hit_targets") else {
        bail!("hit targets missing");
    };
    let all: BTreeSet<String> = targets
        .iter()
        .map(|t| match t.named("hit_param") {
            Some(V::String(name)) => Ok(name.clone()),
            _ => bail!("hit target without hit_param"),
        })
        .collect::<Result<_>>()?;
    let mut covered = BTreeSet::new();
    for cfg in pieces {
        for region in regions(cfg)? {
            ensure!(
                covered.insert(region.clone()),
                "two pieces of the set protect {region}"
            );
        }
        let Some(V::Named(parts)) = cfg.named("hit_params") else {
            unreachable!()
        };
        for part in parts {
            let Some(V::Named(types)) = part.value.named("hit_types") else {
                bail!("{}: no hit types", part.name);
            };
            let names: BTreeSet<&str> = types.iter().map(|t| t.name.as_str()).collect();
            ensure!(
                names
                    == BTreeSet::from([
                        "explosive",
                        "fire",
                        "radiation",
                        "ranged",
                        "shock",
                        "slash",
                        "toxic"
                    ]),
                "{} protects only {names:?}",
                part.name
            );
        }
    }
    ensure!(
        covered == all,
        "the set leaves {:?} unprotected and protects unknown {:?}",
        all.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&all).collect::<Vec<_>>()
    );
    Ok(())
}

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}
fn string(s: impl Into<String>) -> V {
    V::String(s.into())
}
fn table(fields: Vec<NamedValue>) -> V {
    V::Named(fields)
}
fn raw(out: &Path, path: &str, bytes: &[u8], files: &mut Vec<BuiltFile>) -> Result<()> {
    let dest = crate::archive::safe_join(out, path)?;
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(dest, bytes)?;
    files.push(record_file(path, "converted:Duty-026", bytes));
    Ok(())
}
fn config(out: &Path, path: &str, root: V, files: &mut Vec<BuiltFile>) -> Result<()> {
    write_generated_config(
        out,
        path,
        &BinaryConfig { root },
        "generated:Duty-026",
        files,
    )
}
fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).context("truncated u32")?.try_into()?,
    ))
}
fn f32_at(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}

/// Match the 0.26 compressor's header, including its colour-space byte in
/// dwPitchOrLinearSize and retaining BOTH PITCH and LINEARSIZE flags.
/// Omitting these produced corrupt colours in the native lobby renderer even
/// though ordinary DDS readers displayed the images correctly.
pub(crate) fn texture_dds(bytes: &[u8], name: &str, srgb: bool) -> Result<Vec<u8>> {
    let mut out = crate::soc::source::canonicalize_dds(
        bytes,
        crate::soc::source::parse_dds(bytes, name)?,
        srgb,
    )?;
    let flags = u32_at(&out, 8)? | 8;
    let size = u32::try_from(out.len() - 128)?;
    out[8..12].copy_from_slice(&flags.to_le_bytes());
    out[20..24].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

/// The supplied editor configs store name before data; database configs store
/// data before name. Only visited nodes are swapped; string/float payloads stay put.
fn editor_config(input: &[u8]) -> Result<BinaryConfig> {
    fn visit(b: &mut [u8], at: usize, seen: &mut BTreeSet<usize>) -> Result<()> {
        ensure!(seen.insert(at), "cyclic editor config");
        let n = b.get(at..at + 24).context("truncated editor config node")?;
        let name = u64::from_le_bytes(n[..8].try_into()?);
        let data = u64::from_le_bytes(n[8..16].try_into()?);
        let kind = u16::from_le_bytes(n[20..22].try_into()?);
        let count = u16::from_le_bytes(n[22..24].try_into()?);
        b[at..at + 8].copy_from_slice(&data.to_le_bytes());
        b[at + 8..at + 16].copy_from_slice(&name.to_le_bytes());
        if kind == 3 || kind == 4 {
            for i in 0..usize::from(count) {
                visit(
                    b,
                    usize::try_from(data)?
                        .checked_add(i * 24)
                        .context("node overflow")?,
                    seen,
                )?;
            }
        }
        Ok(())
    }
    let mut b = input.to_vec();
    visit(&mut b, 0, &mut BTreeSet::new())?;
    BinaryConfig::parse(&b)
}

fn bind_frames(b: &[u8]) -> Result<Vec<(String, [f32; 9])>> {
    let count = u16::from_le_bytes(b.get(..2).context("missing bone count")?.try_into()?);
    let mut at = 2;
    let mut result = vec![];
    for _ in 0..count {
        let len = b
            .get(at..)
            .context("missing bone")?
            .iter()
            .position(|&v| v == 0)
            .context("missing bone terminator")?;
        let name = std::str::from_utf8(&b[at..at + len])?.to_owned();
        at += len + 1;
        let mut frame = [0.; 9];
        for (i, v) in frame.iter_mut().enumerate() {
            *v = f32_at(b, at + i * 4)?;
            ensure!(v.is_finite(), "nonfinite bone");
        }
        at += 36;
        result.push((name, frame));
    }
    ensure!(at == b.len(), "trailing bind data");
    Ok(result)
}
fn basis(f: &[f32; 9]) -> [[f32; 3]; 3] {
    let (xs, xc) = f[3].sin_cos();
    let (ys, yc) = f[4].sin_cos();
    let (zs, zc) = f[5].sin_cos();
    let mut m = [
        [yc * zc, -yc * zs, ys],
        [ys * xs * zc + xc * zs, -xs * ys * zs + xc * zc, -xs * yc],
        [-xc * ys * zc + xs * zs, xc * ys * zs + xs * zc, xc * yc],
    ];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] *= f[6 + r];
        }
    }
    m
}
fn check_bind(source: &[u8], target: &[u8]) -> Result<usize> {
    let s = bind_frames(source)?;
    let t = bind_frames(target)?;
    ensure!(
        s.len() == 69 && s.len() == t.len(),
        "expected the native 69-bone rig"
    );
    for ((sn, sf), (tn, tf)) in s.iter().zip(&t) {
        ensure!(sn == tn, "bone order differs: {sn}/{tn}");
        let sm = basis(sf);
        let tm = basis(tf);
        for c in 0..3 {
            let sign = if c == 1 { 1. } else { -1. };
            ensure!(
                (sf[c] * sign - tf[c]).abs() < 0.0001,
                "bone translation differs: {sn}"
            );
            for r in 0..3 {
                ensure!(
                    (sm[r][c] * sign - tm[r][c]).abs() < 0.0001,
                    "bone rotation differs: {sn}"
                );
            }
        }
    }
    Ok(s.len())
}

/// Preserve all four influences, UVs and index ordering. A Y half-turn has
/// positive determinant, so winding and tangent handedness do not change.
fn rotate_mesh(input: &[u8], bones: usize) -> Result<(Vec<u8>, [f32; 3], [f32; 3], usize, usize)> {
    ensure!(u32_at(input, 0)? == 3, "missing vertex chunk");
    let n = u32_at(input, 8)? as usize;
    let size = u32_at(input, 4)? as usize;
    ensure!(
        n > 0 && size == 4 + n * 48,
        "expected 48-byte four-weight vertices"
    );
    let index_at = 8 + size;
    ensure!(u32_at(input, index_at)? == 4, "missing index chunk");
    let ni = u32_at(input, index_at + 8)? as usize;
    ensure!(
        ni > 0
            && ni % 3 == 0
            && u32_at(input, index_at + 4)? as usize == 4 + ni * 2
            && input.len() == index_at + 12 + ni * 2,
        "invalid index chunk"
    );
    for v in input[index_at + 12..].chunks_exact(2) {
        ensure!(
            (u16::from_le_bytes(v.try_into()?) as usize) < n,
            "index outside vertex array"
        );
    }
    let mut b = input.to_vec();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for i in 0..n {
        let at = 12 + i * 48;
        let mut sum = 0.;
        for k in 0..3 {
            let w = f32_at(input, at + 16 + k * 4)?;
            ensure!(
                w.is_finite() && (-0.0001..=1.0001).contains(&w),
                "invalid weight"
            );
            sum += w;
        }
        ensure!(sum <= 1.0001, "weights exceed one");
        for k in 0..4 {
            ensure!((input[at + 12 + k] as usize) < bones, "invalid bone index");
        }
        for k in 0..3 {
            let p = f32_at(input, at + k * 4)? * if k == 1 { 1. } else { -1. };
            ensure!(p.is_finite(), "nonfinite position");
            min[k] = min[k].min(p);
            max[k] = max[k].max(p);
            b[at + k * 4..at + k * 4 + 4].copy_from_slice(&p.to_le_bytes());
        }
        // Packed basis bytes are B, G, R, A (Z, Y, X, auxiliary).
        for offset in [28, 32, 36] {
            for axis in [0, 2] {
                b[at + offset + axis] = 255 - b[at + offset + axis];
            }
        }
        for offset in [40, 44] {
            ensure!(f32_at(input, at + offset)?.is_finite(), "nonfinite UV");
        }
    }
    Ok((b, min, max, n, ni))
}

/// Separate whole welded components of the backpack from the limb frame.
/// Bounds are in the supplied model's authoring coordinates. No triangle is cut.
fn split_back(input: &[u8], back: bool, wire_anchors: &[usize]) -> Result<Vec<u8>> {
    split_components(input, back, wire_anchors, true)
}

/// Partition whole welded components, preserving all original vertex bytes.
fn split_components(
    input: &[u8],
    selected_half: bool,
    anchors: &[usize],
    back_bounds: bool,
) -> Result<Vec<u8>> {
    rotate_mesh(input, 69)?; // validate layout before indexing
    let n = u32_at(input, 8)? as usize;
    let index_at = 12 + n * 48;
    let indices: Vec<usize> = input[index_at + 12..]
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes(v.try_into().unwrap()) as usize)
        .collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut positions = BTreeMap::new();
    for i in 0..n {
        let p = [
            f32_at(input, 12 + i * 48)?,
            f32_at(input, 16 + i * 48)?,
            f32_at(input, 20 + i * 48)?,
        ];
        let key = p.map(|v| (v * 100_000.).round() as i32);
        if let Some(j) = positions.insert(key, i) {
            let a = root(&mut parent, i);
            let b = root(&mut parent, j);
            parent[a] = b;
        }
    }
    for tri in indices.chunks_exact(3) {
        for &v in &tri[1..] {
            let a = root(&mut parent, tri[0]);
            let b = root(&mut parent, v);
            parent[a] = b;
        }
    }
    let mut selected = BTreeMap::new();
    for i in 0..n {
        let p = [
            f32_at(input, 12 + i * 48)?,
            f32_at(input, 16 + i * 48)?,
            f32_at(input, 20 + i * 48)?,
        ];
        let inside = back_bounds && p[0].abs() < 0.20 && p[1] > 1.03 && p[2] > 0.11;
        *selected.entry(root(&mut parent, i)).or_insert(true) &= inside;
    }
    for &vertex in anchors {
        ensure!(vertex < n, "component anchor outside mesh");
        selected.insert(root(&mut parent, vertex), true);
    }
    let mut vertices = vec![];
    let mut remap = BTreeMap::new();
    let mut out_indices = vec![];
    for tri in indices.chunks_exact(3) {
        if selected[&root(&mut parent, tri[0])] != selected_half {
            continue;
        }
        for &v in tri {
            let next = u16::try_from(remap.len())?;
            let idx = *remap.entry(v).or_insert_with(|| {
                vertices.extend_from_slice(&input[12 + v * 48..12 + (v + 1) * 48]);
                next
            });
            out_indices.extend_from_slice(&idx.to_le_bytes());
        }
    }
    ensure!(!vertices.is_empty(), "empty component split");
    let mut out = vec![];
    for v in [3u32, 4 + vertices.len() as u32, remap.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    out.extend(vertices);
    for v in [
        4u32,
        4 + out_indices.len() as u32,
        (out_indices.len() / 2) as u32,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(out_indices);
    Ok(out)
}

/// One item of the Duty set. Like the native level 15 exoskeleton sets
/// (Zubr UM-4, Pilgrim) the pieces are separate items of one faction and
/// level; 0.26g has no set id or set bonus. Each piece is worn in its
/// category's own slot and, as Zubr's helmet covers the mask, covers the
/// other slots its model replaces (`data.additional_model_slots`).
/// What a set piece is: its model, protection and data follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Torso, frame, back unit and legs.
    Body,
    /// The combined helmet and mask.
    Helmet,
    Gloves,
    Boots,
}

struct SetPiece {
    kind: Kind,
    id: i32,
    /// The dictionary's internal name.
    name: &'static str,
    /// English name, for the registration.
    label: &'static str,
    path: &'static str,
    template: &'static str,
    /// Profile slot: helmet 0, mask 1, torso 2, back 3, legs 4, gloves 5,
    /// boots 6 (the exe's name table at 0xA79870; srp `model_slot`).
    slot: u8,
    covers: &'static [&'static str],
    weight: f32,
    cost: i32,
    icon: [i32; 2],
    modifications: &'static [i32],
    /// Name and description string keys (texts in `Variant::strings`).
    text: (&'static str, &'static str),
}

/// The body suit: torso, frame, back unit and legs. Worn in the torso slot
/// (category 3), as the user expects of body armour; the gloves and boots
/// are pieces of their own (owner, 2026-10-17). It is armour, as its
/// template (`data.type` 4), and carries no oxygen (owner, 2026-10-19): the
/// item factory (0x31F660) makes an item of `data.type` 1 an oxygen tank,
/// which reads only `amount_time_sec` and `influences`, not the armour's
/// `hit_params` or the modifiers of its `default_modifications` (the
/// artefact slot 1040 among them).
const BODY: SetPiece = SetPiece {
    kind: Kind::Body,
    id: 1805,
    name: "duty_exo_suit",
    label: "Duty exoskeleton",
    path: "gameplay/items/armour/torso/duty_exo_026.options",
    template: "gameplay/items/armour/torso/renesanse_torso_6.options",
    slot: 2,
    covers: &["back", "pants"],
    // Owner's balance (2026-10-15): 35 kg and 300 000 for the set, between
    // "Zubr UM-4" (287 500 for five pieces) and "Pilgrim" (310 000 for six).
    // The helmet takes the average of Zubr's (77 500) and Pilgrim's
    // (72 500); the gloves and boots weigh and cost Zubr's; the body is the
    // rest: 35 - 5 - 0.6 - 3 kg, 300 000 - 75 000 - 2 x 26 250.
    weight: 26.4,
    cost: 172_500,
    icon: [1, 6],
    // Zubr's ammo bag and pocket, Pilgrim's artefact slot, and no speed,
    // stamina or carry bonus, which no level 15 equipment has.
    modifications: &[1030, 1050, 1170, 1040],
    text: ("st_duty_026_suit_name", "st_duty_026_suit_desc"),
};

/// The helmet with its mask: the original combined STK2 head, worn alone
/// over the hidden native head, covering the mask slot like Zubr UM-4's.
/// ID, path, name and icon of the v10 split helmet (1800); bundle v10 never
/// shipped to players.
const HELMET: SetPiece = SetPiece {
    kind: Kind::Helmet,
    id: 1800,
    name: "duty_exo_helmet",
    label: "Duty exoskeleton helmet",
    path: "gameplay/items/armour/helmet/duty_exo_026.options",
    template: "gameplay/items/armour/helmet/renesanse_helmet_6.options",
    slot: 0,
    covers: &["mask"],
    weight: 5.0,
    cost: 75_000,
    icon: [3, 6],
    modifications: &[],
    text: ("st_duty_026_helmet_name", "st_duty_026_head_desc"),
};

/// The suit's gloves, a piece of their own as Zubr UM-4's are: the STK2
/// hands, with their first-person model. Weight, price, drops and (no)
/// modifications as Zubr's gloves; protection as the set's.
const GLOVES: SetPiece = SetPiece {
    kind: Kind::Gloves,
    id: 1803,
    name: "duty_exo_gloves",
    label: "Duty gloves",
    path: "gameplay/items/armour/gloves/duty_exo_026.options",
    template: "gameplay/items/armour/gloves/renesanse_gloves_6.options",
    slot: 5,
    covers: &[],
    weight: 0.6,
    cost: 26_250,
    icon: [6, 6],
    modifications: &[],
    text: ("st_duty_026_gloves_name", "st_duty_026_gloves_desc"),
};

/// The suit's boots, a piece of their own as Zubr UM-4's are.
const BOOTS: SetPiece = SetPiece {
    kind: Kind::Boots,
    id: 1804,
    name: "duty_exo_boots",
    label: "Duty boots",
    path: "gameplay/items/armour/boots/duty_exo_026.options",
    template: "gameplay/items/armour/boots/renesanse_boots_6.options",
    slot: 6,
    covers: &[],
    weight: 3.0,
    cost: 26_250,
    icon: [7, 6],
    modifications: &[],
    text: ("st_duty_026_boots_name", "st_duty_026_boots_desc"),
};

/// The Freedom repaint's body: Duty's suit as a Fringe Settlers item (the
/// "Pilgrim" vest's template gives faction 4, affinity 30 and category 3).
/// Owner's stats (2026-10-16; armour provisional): 2 kg lighter than Duty,
/// +3% movement speed (1112, as the level 14 Fringe boots) and -4% stamina
/// use (1161, as 1160 on the Renaissance boots); other utility as Duty's
/// but no pocket capacity (1170; owner, 2026-10-16).
const FREEDOM_BODY: SetPiece = SetPiece {
    id: 1807,
    name: "freedom_exo_suit",
    label: "Freedom exoskeleton",
    path: "gameplay/items/armour/torso/freedom_exo_026.options",
    template: "gameplay/items/armour/torso/edge_torso_6.options",
    // The set's 33 kg and 300 000 less the helmet and Pilgrim's gloves and
    // boots: 33 - 4.5 - 0.4 - 1.9 kg, 300 000 - 75 000 - 2 x 23 750.
    weight: 26.2,
    cost: 177_500,
    icon: [4, 6],
    modifications: &[1030, 1050, 1040, 1112, 1161],
    text: ("st_freedom_026_suit_name", "st_freedom_026_suit_desc"),
    ..BODY
};

/// The Freedom repaint's helmet, on the "Pilgrim" helmet's template (which
/// covers the mask, as Zubr's does).
const FREEDOM_HELMET: SetPiece = SetPiece {
    id: 1808,
    name: "freedom_exo_helmet",
    label: "Freedom exoskeleton helmet",
    path: "gameplay/items/armour/helmet/freedom_exo_026.options",
    template: "gameplay/items/armour/helmet/edge_helmet_6.options",
    weight: 4.5,
    icon: [5, 6],
    text: ("st_freedom_026_helmet_name", "st_freedom_026_head_desc"),
    ..HELMET
};

/// Freedom's gloves, on Pilgrim's.
const FREEDOM_GLOVES: SetPiece = SetPiece {
    id: 1809,
    name: "freedom_exo_gloves",
    label: "Freedom gloves",
    path: "gameplay/items/armour/gloves/freedom_exo_026.options",
    template: "gameplay/items/armour/gloves/edge_gloves_6.options",
    weight: 0.4,
    cost: 23_750,
    icon: [8, 6],
    text: ("st_freedom_026_gloves_name", "st_freedom_026_gloves_desc"),
    ..GLOVES
};

/// Freedom's boots, on Pilgrim's. 1810 is the balaclava's.
const FREEDOM_BOOTS: SetPiece = SetPiece {
    id: 1811,
    name: "freedom_exo_boots",
    label: "Freedom boots",
    path: "gameplay/items/armour/boots/freedom_exo_026.options",
    template: "gameplay/items/armour/boots/edge_boots_6.options",
    weight: 1.9,
    cost: 23_750,
    icon: [9, 6],
    text: ("st_freedom_026_boots_name", "st_freedom_026_boots_desc"),
    ..BOOTS
};

/// One exoskeleton set: its namespace, items and protection. Both sets are
/// built from the same STK2 Duty models; Freedom has its own repainted
/// textures (`scripts/freedom_026`) and no chest disc.
pub struct Variant {
    /// `--variant` name.
    pub key: &'static str,
    /// The models', materials' and textures' namespace.
    ns: &'static str,
    asset_id: &'static str,
    /// Body, helmet, gloves, boots.
    set: [SetPiece; 4],
    /// The dictionary IDs this variant owns and may replace.
    reusable: &'static [i32],
    /// Duty only: its pre-split preview rows (1800..=1806) may be replaced.
    legacy_split: bool,
    /// Every piece's name and description: (string key, text in every
    /// client language).
    strings: [(&'static str, Text); 8],
    /// The brass disc with the red star on the chest (`sign_s`).
    chest_disc: bool,
    /// Ballistic protection (ranged, slash, explosive): body, head.
    ballistic: (f32, f32),
    /// Anomaly protection (fire, shock, radiation, toxic).
    anomaly: f32,
}

pub const DUTY: Variant = Variant {
    key: "duty",
    ns: "ported/duty_026",
    asset_id: "duty-exoskeleton-026g",
    set: [BODY, HELMET, GLOVES, BOOTS],
    reusable: &[1800, 1801, 1802, 1803, 1804, 1805, 1806],
    legacy_split: true,
    // The factions as the localized S.T.A.L.K.E.R. games name them: Долг,
    // Обов'язок, Pflicht, Powinność.
    strings: [
        (
            "st_duty_026_suit_name",
            Text {
                english: "Duty exoskeleton",
                russian: "Экзоскелет \"Долга\"",
                ukrainian: "Екзоскелет \"Обов'язку\"",
                german: "Pflicht-Exoskelett",
                polish: "Egzoszkielet Powinności",
            },
        ),
        (
            "st_duty_026_suit_desc",
            Text {
                english: "An experimental sample of a military exoskeleton. Was never mass-produced due to extraordinary cost and some design flaws. Small batches are still made in underground facilities outside Ukraine. Comes with a built-in artifact container.",
                russian: "Экспериментальный образец военного экзоскелета. В серийное производство так и не пошёл из-за огромной стоимости и ряда конструктивных недостатков. Небольшие партии до сих пор собирают в подпольных мастерских за пределами Украины. Оснащён встроенным контейнером для артефактов.",
                ukrainian: "Експериментальний зразок військового екзоскелета. У серійне виробництво так і не пішов через величезну вартість і низку конструктивних недоліків. Невеликі партії досі збирають у підпільних майстернях за межами України. Оснащений вбудованим контейнером для артефактів.",
                german: "Ein experimentelles Muster eines militärischen Exoskeletts. Wegen der enormen Kosten und einiger Konstruktionsmängel ging es nie in Serie. Kleine Stückzahlen werden noch immer in Untergrundwerkstätten außerhalb der Ukraine gefertigt. Mit eingebautem Artefaktbehälter.",
                polish: "Eksperymentalny egzemplarz wojskowego egzoszkieletu. Nigdy nie trafił do produkcji seryjnej z powodu ogromnych kosztów i wad konstrukcyjnych. Niewielkie partie wciąż powstają w podziemnych warsztatach poza Ukrainą. Wyposażony we wbudowany pojemnik na artefakty.",
            },
        ),
        (
            "st_duty_026_helmet_name",
            Text {
                english: "Duty exoskeleton helmet",
                russian: "Шлем экзоскелета \"Долга\"",
                ukrainian: "Шолом екзоскелета \"Обов'язку\"",
                german: "Pflicht-Exoskeletthelm",
                polish: "Hełm egzoszkieletu Powinności",
            },
        ),
        (
            "st_duty_026_head_desc",
            Text {
                english: "The sealed helmet of the experimental Duty exoskeleton, with a built-in respirator. Covers the whole head and face.",
                russian: "Герметичный шлем экспериментального экзоскелета \"Долга\" со встроенным респиратором. Закрывает всю голову и лицо.",
                ukrainian: "Герметичний шолом експериментального екзоскелета \"Обов'язку\" з вбудованим респіратором. Закриває всю голову та обличчя.",
                german: "Der versiegelte Helm des experimentellen Pflicht-Exoskeletts, mit eingebautem Atemschutz. Bedeckt den ganzen Kopf und das Gesicht.",
                polish: "Hermetyczny hełm eksperymentalnego egzoszkieletu Powinności z wbudowanym respiratorem. Osłania całą głowę i twarz.",
            },
        ),
        (
            "st_duty_026_gloves_name",
            Text {
                english: "Duty gloves",
                russian: "Перчатки \"Долга\"",
                ukrainian: "Рукавиці \"Обов'язку\"",
                german: "Pflicht-Handschuhe",
                polish: "Rękawice Powinności",
            },
        ),
        (
            "st_duty_026_gloves_desc",
            Text {
                english: "Reinforced fingerless gloves that Duty fighters wear with their heavy armour.",
                russian: "Усиленные беспалые перчатки, которые бойцы \"Долга\" носят с тяжёлой бронёй.",
                ukrainian: "Посилені рукавиці без пальців, які бійці \"Обов'язку\" носять із важкою бронею.",
                german: "Verstärkte fingerlose Handschuhe, die Kämpfer der Pflicht zu ihrer schweren Rüstung tragen.",
                polish: "Wzmocnione rękawice bez palców, które bojownicy Powinności noszą do ciężkiego pancerza.",
            },
        ),
        (
            "st_duty_026_boots_name",
            Text {
                english: "Duty boots",
                russian: "Ботинки \"Долга\"",
                ukrainian: "Черевики \"Обов'язку\"",
                german: "Pflicht-Stiefel",
                polish: "Buty Powinności",
            },
        ),
        (
            "st_duty_026_boots_desc",
            Text {
                english: "Heavy armoured boots that Duty fighters wear with their heavy armour.",
                russian: "Тяжёлые бронированные ботинки, которые бойцы \"Долга\" носят с тяжёлой бронёй.",
                ukrainian: "Важкі броньовані черевики, які бійці \"Обов'язку\" носять із важкою бронею.",
                german: "Schwere gepanzerte Stiefel, die Kämpfer der Pflicht zu ihrer schweren Rüstung tragen.",
                polish: "Ciężkie opancerzone buty, które bojownicy Powinności noszą do ciężkiego pancerza.",
            },
        ),
    ],
    chest_disc: true,
    // Between Zubr 0.90/0.75 and Pilgrim 0.85/0.70; anomaly between Zubr
    // 0.5 and Pilgrim 0.9.
    ballistic: (0.87, 0.72),
    anomaly: 0.70,
};

/// Owner's decisions (2026-10-16): the bright Freedom look, no chest disc,
/// Fringe Settlers (faction 4, sold at rank 10 "Magister"), ballistic as
/// Pilgrim's and anomaly 10 points below it (provisional).
pub const FREEDOM: Variant = Variant {
    key: "freedom",
    ns: "ported/freedom_026",
    asset_id: "freedom-exoskeleton-026g",
    set: [FREEDOM_BODY, FREEDOM_HELMET, FREEDOM_GLOVES, FREEDOM_BOOTS],
    reusable: &[1807, 1808, 1809, 1811],
    legacy_split: false,
    // Свобода, Свобода, Freiheit, Swoboda.
    strings: [
        (
            "st_freedom_026_suit_name",
            Text {
                english: "Freedom exoskeleton",
                russian: "Экзоскелет \"Свободы\"",
                ukrainian: "Екзоскелет \"Свободи\"",
                german: "Freiheit-Exoskelett",
                polish: "Egzoszkielet Swobody",
            },
        ),
        (
            "st_freedom_026_suit_desc",
            Text {
                english: "An experimental military exoskeleton repainted in Freedom colours. Never mass-produced due to its cost and design flaws; small batches still leave underground workshops. Comes with a built-in artifact container.",
                russian: "Экспериментальный военный экзоскелет, перекрашенный в цвета \"Свободы\". В серию не пошёл из-за стоимости и конструктивных недостатков; небольшие партии до сих пор выходят из подпольных мастерских. Оснащён встроенным контейнером для артефактов.",
                ukrainian: "Експериментальний військовий екзоскелет, перефарбований у кольори \"Свободи\". У серію не пішов через вартість і конструктивні недоліки; невеликі партії досі виходять із підпільних майстерень. Оснащений вбудованим контейнером для артефактів.",
                german: "Ein experimentelles militärisches Exoskelett in den Farben der Freiheit. Wegen der Kosten und der Konstruktionsmängel ging es nie in Serie; kleine Stückzahlen verlassen noch immer Untergrundwerkstätten. Mit eingebautem Artefaktbehälter.",
                polish: "Eksperymentalny wojskowy egzoszkielet przemalowany w barwy Swobody. Nigdy nie trafił do produkcji seryjnej z powodu kosztów i wad konstrukcyjnych; niewielkie partie wciąż opuszczają podziemne warsztaty. Wyposażony we wbudowany pojemnik na artefakty.",
            },
        ),
        (
            "st_freedom_026_helmet_name",
            Text {
                english: "Freedom exoskeleton helmet",
                russian: "Шлем экзоскелета \"Свободы\"",
                ukrainian: "Шолом екзоскелета \"Свободи\"",
                german: "Freiheit-Exoskeletthelm",
                polish: "Hełm egzoszkieletu Swobody",
            },
        ),
        (
            "st_freedom_026_head_desc",
            Text {
                english: "The sealed helmet of the Freedom exoskeleton, with a built-in respirator. Covers the whole head and face.",
                russian: "Герметичный шлем экзоскелета \"Свободы\" со встроенным респиратором. Закрывает всю голову и лицо.",
                ukrainian: "Герметичний шолом екзоскелета \"Свободи\" з вбудованим респіратором. Закриває всю голову та обличчя.",
                german: "Der versiegelte Helm des Freiheit-Exoskeletts, mit eingebautem Atemschutz. Bedeckt den ganzen Kopf und das Gesicht.",
                polish: "Hermetyczny hełm egzoszkieletu Swobody z wbudowanym respiratorem. Osłania całą głowę i twarz.",
            },
        ),
        (
            "st_freedom_026_gloves_name",
            Text {
                english: "Freedom gloves",
                russian: "Перчатки \"Свободы\"",
                ukrainian: "Рукавиці \"Свободи\"",
                german: "Freiheit-Handschuhe",
                polish: "Rękawice Swobody",
            },
        ),
        (
            "st_freedom_026_gloves_desc",
            Text {
                english: "Reinforced fingerless gloves in Freedom colours, worn with heavy armour.",
                russian: "Усиленные беспалые перчатки в цветах \"Свободы\", их носят с тяжёлой бронёй.",
                ukrainian: "Посилені рукавиці без пальців у кольорах \"Свободи\", їх носять із важкою бронею.",
                german: "Verstärkte fingerlose Handschuhe in den Farben der Freiheit, zu schwerer Rüstung getragen.",
                polish: "Wzmocnione rękawice bez palców w barwach Swobody, noszone do ciężkiego pancerza.",
            },
        ),
        (
            "st_freedom_026_boots_name",
            Text {
                english: "Freedom boots",
                russian: "Ботинки \"Свободы\"",
                ukrainian: "Черевики \"Свободи\"",
                german: "Freiheit-Stiefel",
                polish: "Buty Swobody",
            },
        ),
        (
            "st_freedom_026_boots_desc",
            Text {
                english: "Heavy armoured boots in Freedom colours, worn with heavy armour.",
                russian: "Тяжёлые бронированные ботинки в цветах \"Свободы\", их носят с тяжёлой бронёй.",
                ukrainian: "Важкі броньовані черевики в кольорах \"Свободи\", їх носять із важкою бронею.",
                german: "Schwere gepanzerte Stiefel in den Farben der Freiheit, zu schwerer Rüstung getragen.",
                polish: "Ciężkie opancerzone buty w barwach Swobody, noszone do ciężkiego pancerza.",
            },
        ),
    ],
    chest_disc: false,
    ballistic: (0.85, 0.70),
    anomaly: 0.80,
};

/// The variant a `--variant` name stands for.
pub fn variant(key: &str) -> Result<&'static Variant> {
    [&DUTY, &FREEDOM]
        .into_iter()
        .find(|v| v.key == key)
        .with_context(|| format!("no exoskeleton variant {key}; expected duty or freedom"))
}

/// The profile slot a `data.additional_model_slots` name stands for.
fn profile_slot(name: &str) -> Option<u8> {
    Some(match name {
        "helmet" => 0,
        "mask" => 1,
        "torso" => 2,
        "back" => 3,
        "pants" => 4,
        "gloves" => 5,
        "boots" => 6,
        _ => return None,
    })
}

impl SetPiece {
    fn covered_slots(&self) -> Result<Vec<u8>> {
        self.covers
            .iter()
            .map(|name| profile_slot(name).with_context(|| format!("no profile slot {name}")))
            .collect()
    }
}

/// The native configs the set is made of.
struct Natives {
    /// Zubr UM-4's pieces by category: helmet, torso, legs, gloves, boots.
    protection: Vec<(String, V)>,
    /// Each set piece's template: its dictionary row and config.
    templates: Vec<(V, V)>,
}

/// An earlier row of this variant, which the builder may replace: for Duty a
/// split preview item or the one-item suit worn in the back slot, and a
/// piece of the variant's own set. Never another variant's or mod's row.
fn check_reused_id(variant: &Variant, id: i32, row: &V) -> Result<()> {
    let is = |path: &str, name: &str| {
        row.named("cfg_name") == Some(&string(path)) && row.named("name") == Some(&string(name))
    };
    let split = variant.legacy_split
        && ["helmet", "torso", "legs", "gloves", "boots", "back", "mask"]
            .get(usize::try_from(id - 1800).unwrap_or(usize::MAX))
            .is_some_and(|category| {
                is(
                    &format!("gameplay/items/armour/{category}/duty_exo_026.options"),
                    &format!("duty_exo_{category}"),
                )
            });
    let ours = variant
        .set
        .iter()
        .any(|piece| piece.id == id && is(piece.path, piece.name));
    ensure!(
        variant.reusable.contains(&id) && (split || ours),
        "item ID {id} belongs to other content"
    );
    Ok(())
}

fn remove_field(value: &mut V, key: &str) {
    if let V::Named(fields) = value {
        fields.retain(|field| field.name != key);
    }
}

/// Zubr UM-4's hit regions with the set's armour, split as its pieces split
/// them: the helmet's (brain, face) for the helmet, the gloves' (arms) for
/// the gloves, the boots' (feet) for the boots, and the torso's and
/// trousers' for the body; each region once.
fn split_protection(variant: &Variant, natives: &[(String, V)]) -> Result<BTreeMap<Kind, V>> {
    let mut zones: BTreeMap<Kind, BTreeMap<String, V>> = BTreeMap::new();
    for (category, native) in natives {
        let kind = match category.as_str() {
            "helmet" => Kind::Helmet,
            "gloves" => Kind::Gloves,
            "boots" => Kind::Boots,
            "torso" | "legs" => Kind::Body,
            other => bail!("no set piece takes the native {other}'s regions"),
        };
        let Some(V::Named(parts)) = native.named("hit_params") else {
            bail!("native hit parameters missing");
        };
        for part in parts {
            let mut value = part.value.clone();
            let Some(V::Named(types)) = value.named_mut("hit_types") else {
                bail!("native hit types missing");
            };
            for hit in types {
                let protection = if matches!(hit.name.as_str(), "slash" | "explosive" | "ranged") {
                    // Zubr 0.75/0.90 and Pilgrim 0.70/0.85 (head/body).
                    if kind == Kind::Helmet {
                        variant.ballistic.1
                    } else {
                        variant.ballistic.0
                    }
                } else {
                    // Zubr 0.5, Pilgrim 0.9.
                    variant.anomaly
                };
                hit.value.set_path("armor", V::Float(protection))?;
            }
            ensure!(
                zones.values().all(|other| !other.contains_key(&part.name)),
                "native region {} is protected by two pieces",
                part.name
            );
            zones
                .entry(kind)
                .or_default()
                .insert(part.name.clone(), value);
        }
    }
    let mut out = BTreeMap::new();
    for kind in [Kind::Body, Kind::Helmet, Kind::Gloves, Kind::Boots] {
        let regions = zones.remove(&kind).unwrap_or_default();
        ensure!(
            !regions.is_empty(),
            "native hit regions missing for {kind:?}"
        );
        out.insert(
            kind,
            table(regions.into_iter().map(|(k, v)| field(&k, v)).collect()),
        );
    }
    Ok(out)
}

fn props(values: &[(i32, String)]) -> V {
    V::Indexed(
        values
            .iter()
            .map(|(id, value)| {
                table(vec![
                    field("prop_id", V::Integer(*id)),
                    field("prop_value", string(value.as_str())),
                ])
            })
            .collect(),
    )
}

/// The stamina-regeneration modifier as the stock item configs spell it,
/// and as the exe reads it (its name table at 0x928448).
const STAMINA_REGEN_MISSPELT: &str = "stamina_regenation_speed_modifier";
const STAMINA_REGEN: &str = "stamina_regeneration_speed_modifier";

/// Renames a key of `parameters`, keeping its value. The encoder writes a
/// table in key-CRC order, so the renamed key lands where the game looks.
fn rename_parameter(cfg: &mut V, old: &str, new: &str) -> Result<()> {
    let Some(V::Named(parameters)) = cfg.named_mut("parameters") else {
        return Ok(());
    };
    if !parameters.iter().any(|f| f.name == old) {
        return Ok(());
    }
    ensure!(
        !parameters.iter().any(|f| f.name == new),
        "parameters has both {old} and {new}"
    );
    for f in parameters.iter_mut().filter(|f| f.name == old) {
        f.name = new.into();
    }
    Ok(())
}

/// The dictionary row and config of each piece of the variant's set.
fn set_items<'v>(
    variant: &'v Variant,
    natives: &Natives,
    icons: bool,
) -> Result<Vec<(&'v SetPiece, V, V)>> {
    ensure!(
        natives.templates.len() == variant.set.len(),
        "set templates missing"
    );
    let ns = variant.ns;
    let zones = split_protection(variant, &natives.protection)?;
    let mut out = vec![];
    for (piece, (row, template)) in variant.set.iter().zip(&natives.templates) {
        let body = piece.kind == Kind::Body;
        let mut row = row.clone();
        row.set_path("dict_id", V::Integer(piece.id))?;
        row.set_path("cfg_name", string(piece.path))?;
        row.set_path("name", string(piece.name))?;
        row.set_path("base_cost", V::Integer(piece.cost))?;
        let mut cfg = template.clone();
        // Skhid's weapon balance 1 (revision 21, resources.db.patch_21) spelt
        // the key as the exe reads it in every item config, these pieces'
        // too; the templates come from the stock database.
        rename_parameter(&mut cfg, STAMINA_REGEN_MISSPELT, STAMINA_REGEN)?;
        let armor = if piece.kind == Kind::Helmet {
            variant.ballistic.1
        } else {
            variant.ballistic.0
        };
        let protection = &zones[&piece.kind];
        let mut values = vec![
            ("weight", piece.weight),
            ("armor", armor),
            ("ano_armor", variant.anomaly),
        ];
        if body {
            values.extend([("arms_armor", armor), ("back_armor", armor)]);
        }
        for (key, value) in values {
            cfg.set_path(&format!("parameters.{key}"), V::Float(value))?;
        }
        for key in ["item_level", "match_making_level"] {
            cfg.set_path(&format!("parameters.{key}"), V::Integer(15))?;
        }
        cfg.set_path("parameters.max_carried_weight_modifier", V::Integer(0))?;
        // Drops (owner, 2026-10-17): every piece as the native piece of its
        // tier it is made from (its template's weight, Zubr UM-4's and
        // Pilgrim's 10; the body no longer covers the gloves and boots, so it
        // drops as the native torso). srp rolls a category, an item level,
        // then an item of that category and level by weight: each piece
        // shares its category's level-15 roll.
        let drop_weight = match template
            .named("parameters")
            .and_then(|p| p.named("drop_weight"))
        {
            Some(V::Integer(weight)) if *weight > 0 => *weight,
            other => bail!("native {} drops with weight {other:?}", piece.template),
        };
        cfg.set_path("parameters.drop_weight", V::Integer(drop_weight))?;
        // Imported textures have no customization masks.
        cfg.set_path("parameters.camo_available", V::Boolean(false))?;
        cfg.set_path("parameters.decal_available", V::Boolean(false))?;
        cfg.set_path("hit_params", protection.clone())?;
        if !piece.modifications.is_empty() {
            cfg.set_path(
                "default_modifications",
                V::Indexed(piece.modifications.iter().map(|&m| V::Integer(m)).collect()),
            )?;
        }
        // The template's sleeve variants name native models; the body
        // writes its own below.
        for key in TORSO_TABLES {
            remove_field(&mut cfg, key);
        }
        let covers = V::Indexed(piece.covers.iter().map(|&s| string(s)).collect());
        let model = |name: &str, surface: &str| {
            table(vec![
                field("base_model", string(format!("{ns}/duty_03_{name}"))),
                field("part_name", string(surface)),
            ])
        };
        let shown = [
            (6, format!("{}", (armor * 100.0f32).round())),
            (8, format!("{}", (variant.anomaly * 100.0f32).round())),
            (15, format!("{:.1}", piece.weight)),
        ];
        // Every piece is the armour its template is (`data.type` 4 for the
        // native armour pieces); the body too, so the game applies its
        // `hit_params` and `default_modifications`.
        let native_type = template
            .named("data")
            .and_then(|v| v.named("type"))
            .with_context(|| format!("native {} has no data.type", piece.template))?
            .clone();
        let data = if body {
            let mut data = vec![field("type", native_type)];
            // No `base_model_hud` here: 0.26g's skin cook
            // (`profile_skin_visual_cook::on_configs_loaded`, 0x205336) draws
            // a part's hud model instead of its `base_model` wherever the
            // table has one, which hid the suit behind the hand model. The
            // gloves carry it, as native gloves do.
            let visual = vec![
                field("base_model", string(format!("{ns}/duty_03_torso_s1"))),
                field("part_name", string("torso_s")),
                field(
                    "additional_models",
                    V::Indexed(
                        [
                            ("torso_s1", "exoskeleton_s"),
                            ("torso_s1", "back_s"),
                            ("torso_s1", "sign_s"),
                            ("legs_s1", "legs_s"),
                        ]
                        .into_iter()
                        .filter(|(_, surface)| variant.chest_disc || *surface != "sign_s")
                        .map(|(name, surface)| model(name, surface))
                        .collect(),
                    ),
                ),
            ];
            // The cook reads the torso from the table the gloves' `model_type`
            // names (`data` without gloves). Native torsos have all three
            // sleeve tables; each is the whole suit, whatever gloves are worn.
            for key in TORSO_TABLES {
                cfg.set_path(key, table(visual.clone()))?;
            }
            data.extend(visual);
            data.push(field("additional_model_slots", covers));
            data
        } else {
            let mut data = vec![field("type", native_type)];
            match piece.kind {
                Kind::Helmet => data.extend([
                    field("base_model", string(format!("{ns}/duty_03_head"))),
                    field("part_name", string("helmet_s")),
                    // The combined helmet and mask cover the whole head.
                    field("remove_head_model", V::Boolean(true)),
                ]),
                // As native gloves: the model with its first-person hud
                // pair, and the torso sleeves it is worn with (`data_full`,
                // the long ones the STK2 hands were made for).
                Kind::Gloves => data.extend([
                    field("base_model", string(format!("{ns}/duty_03_hand"))),
                    field("part_name", string("torso_s")),
                    field("base_model_hud", string(format!("{ns}/duty_03_hand_hud"))),
                    field("part_name_hud", string("torso_s")),
                    field("model_type", string("data_full")),
                ]),
                // As native boots: the trousers they are worn with stay the
                // template's (`data_long` for both tiers).
                Kind::Boots => {
                    let model_type = template
                        .named("data")
                        .and_then(|v| v.named("model_type"))
                        .with_context(|| format!("native {} has no model_type", piece.template))?
                        .clone();
                    data.extend([
                        field("base_model", string(format!("{ns}/duty_03_boot"))),
                        field("part_name", string("legs_s")),
                        field("model_type", model_type),
                    ]);
                }
                Kind::Body => unreachable!(),
            }
            data.push(field("additional_model_slots", covers));
            data
        };
        cfg.set_path("data", table(data))?;
        cfg.set_path("ui_desc.props_list", props(&shown))?;
        let (name, description) = piece.text;
        for key in [name, description] {
            ensure!(
                variant.strings.iter().any(|(k, _)| *k == key),
                "{key} has no texts"
            );
        }
        cfg.set_path("ui_desc.text_descriptions.name", string(name))?;
        cfg.set_path("ui_desc.text_descriptions.description", string(description))?;
        if icons {
            cfg.set_path(
                "ui_desc.icon",
                V::Indexed(piece.icon.iter().map(|&v| V::Integer(v)).collect()),
            )?;
        }
        out.push((piece, row, cfg));
    }
    Ok(out)
}

/// Builds the Duty set.
pub fn build(
    source: &Path,
    textures: &Path,
    db: &Path,
    icons_swf: Option<&Path>,
    output: &Path,
) -> Result<()> {
    build_variant(&DUTY, source, textures, db, icons_swf, output)
}

/// Builds one exoskeleton set from the STK2 Duty models and `textures`.
pub fn build_variant(
    variant: &Variant,
    source: &Path,
    textures: &Path,
    db: &Path,
    icons_swf: Option<&Path>,
    output: &Path,
) -> Result<()> {
    let ns = variant.ns;
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut target = ResourceDb::open(db)?;
    let bind = target.read(&format!("{BASE}/render/bind_pose"))?;
    let mut files = vec![];
    let mut provenance = BTreeMap::new();
    let mut torso = vec![
        ("torso_s", "torso"),
        ("exoskeleton_s", "attach"),
        ("back_s", "attach"),
    ];
    // Freedom drops the chest disc surface; it is not just hidden.
    if variant.chest_disc {
        torso.push(("sign_s", "sign"));
    }
    let parts = [
        ("torso_s1", torso),
        ("head", vec![("helmet_s", "helmet")]),
        ("legs_s1", vec![("legs_s", "legs")]),
        ("boot", vec![("legs_s", "legs")]),
        ("hand", vec![("torso_s", "torso")]),
        ("hand_hud", vec![("torso_s", "torso")]),
    ];
    for (part, surfaces) in &parts {
        let name = format!("duty_03_{part}.skinned_model");
        let exported = source
            .join("resources/exported/models/character/human/duty")
            .join(&name);
        let mut donor = ResourceDb::open(&exported)?;
        let bones = check_bind(&donor.read("render/bind_pose")?, &bind)?;
        let platform = source
            .join("resources/platforms/pc_dx11/models/character/human/duty")
            .join(&name);
        let root = format!("models/{ns}/{name}");
        let mut settings =
            parse_config(&mut target, &format!("{BASE}/settings"), "model template")?;
        settings.root.set_path(
            "lod_hierrarchy",
            table(vec![field(
                "LOD0",
                table(vec![field(
                    "surfaces",
                    V::Indexed(surfaces.iter().map(|(s, _)| string(*s)).collect()),
                )]),
            )]),
        )?;
        settings.root.set_path(
            "material_settings",
            table(
                surfaces
                    .iter()
                    .map(|(s, m)| {
                        field(
                            s,
                            table(vec![
                                field("material_name", string(format!("{ns}/{m}"))),
                                field("used_in_portals", V::Boolean(false)),
                            ]),
                        )
                    })
                    .collect(),
            ),
        )?;
        config(
            &overlay,
            &format!("{root}/settings"),
            settings.root,
            &mut files,
        )?;
        raw(
            &overlay,
            &format!("{root}/render/bind_pose"),
            &bind,
            &mut files,
        )?;
        raw(
            &overlay,
            &format!("{root}/skeleton"),
            &target.read(&format!("{BASE}/skeleton"))?,
            &mut files,
        )?;
        let mut all_min = [f32::INFINITY; 3];
        let mut all_max = [f32::NEG_INFINITY; 3];
        for (surface, _) in surfaces {
            let source_surface = if *surface == "back_s" {
                "exoskeleton_s"
            } else {
                surface
            };
            let input = platform.join(format!("render/{source_surface}/converted_model"));
            let source_bytes = fs::read(&input)?;
            provenance.insert(
                input.display().to_string(),
                record_file("", "", &source_bytes).sha256,
            );
            let split;
            let bytes = if *surface == "back_s" || *surface == "exoskeleton_s" {
                // Reviewed flexible hoses/cables, including their end cuffs.
                // Anchor vertices select entire welded components; the hash
                // prevents applying those indices to a different source mesh.
                ensure!(
                    record_file("", "", &source_bytes).sha256
                        == "0c52447877ee36c4d928d2922fc359f60cecbf00913cb082bef0fc17459b8ffe",
                    "Duty attachment mesh changed; review wire anchors"
                );
                split = split_back(
                    &source_bytes,
                    *surface == "back_s",
                    &[
                        7366, 7461, 5388, 5234, 5292, 5450, 212, 35, 67, 3800, 4840, 247, 167,
                        3238, 4278, 854, 2288, 0,
                    ],
                )?;
                &split
            } else {
                &source_bytes
            };
            let (mesh, min, max, n, ni) = rotate_mesh(bytes, bones)?;
            for k in 0..3 {
                all_min[k] = all_min[k].min(min[k]);
                all_max[k] = all_max[k].max(max[k]);
            }
            let mut properties = editor_config(&fs::read(
                platform.join(format!("render/{source_surface}/export_properties")),
            )?)?;
            ensure!(
                properties.root.named("type") == Some(&V::Integer(47)),
                "source is not four-weight skinned"
            );
            properties
                .root
                .set_path("bounding_box.min", V::Float3(min))?;
            properties
                .root
                .set_path("bounding_box.max", V::Float3(max))?;
            // Replace authoring metadata with the actual emitted counts.
            properties
                .root
                .set_path("source.vertices", V::Integer(n.try_into()?))?;
            properties
                .root
                .set_path("source.indices", V::Integer(ni.try_into()?))?;
            config(
                &overlay,
                &format!("{root}/render/{surface}/export_properties"),
                properties.root,
                &mut files,
            )?;
            raw(
                &overlay,
                &format!("{root}/render/{surface}/converted_model"),
                &mesh,
                &mut files,
            )?;
        }
        let mut properties = parse_config(
            &mut target,
            &format!("{BASE}/render/export_properties"),
            "render template",
        )?;
        properties
            .root
            .set_path("bounding_box.min", V::Float3(all_min))?;
        properties
            .root
            .set_path("bounding_box.max", V::Float3(all_max))?;
        properties
            .root
            .set_path("src_filename", string(format!("___surva_char/{name}")))?;
        config(
            &overlay,
            &format!("{root}/render/export_properties"),
            properties.root,
            &mut files,
        )?;
    }
    // The suit and its patch are padded cloth; the helmet and the
    // exoskeleton frame and back unit (`attach`) are painted metal.
    use crate::outfit_026::Finish;
    let texture_sets = [
        (
            "torso",
            "dolg_exo_torso",
            "dolg_exo_torso_nmap",
            Finish::CLOTH,
        ),
        ("legs", "dolg_exo_legs", "dolg_exo_legs_nmap", Finish::CLOTH),
        (
            "helmet",
            "dolg_exo_helmet",
            "dolg_exo_helmet_nmap",
            Finish::ARMOUR,
        ),
        (
            "attach",
            "dold_exo_attach",
            "dolg_exo_attach_nmap",
            Finish::ARMOUR,
        ),
        ("sign", "dol_sign", "dol_sign _nmap", Finish::CLOTH),
    ];
    for (part, diffuse, normal, finish) in texture_sets {
        if part == "sign" && !variant.chest_disc {
            continue;
        }
        let mut material = parse_config(&mut target, MATERIAL, "native equipped armor material")?;
        for (role, source_name, compression, srgb) in [
            ("diffuse", diffuse, TextureCompression::Dxt1, true),
            ("normal", normal, TextureCompression::Dxt5Normal, false),
        ] {
            let input = textures.join(format!("{source_name}.tga"));
            provenance.insert(
                input.display().to_string(),
                record_file("", "", &fs::read(&input)?).sha256,
            );
            let path = format!("textures/{ns}/{part}_{role}");
            let dest = overlay.join(format!("{path}.dds"));
            fs::create_dir_all(dest.parent().unwrap())?;
            crate::armor::convert_texture(&input, &dest, compression)?;
            let dds = fs::read(&dest)?;
            let dds = texture_dds(&dds, &path, srgb)?;
            raw(&overlay, &format!("{path}.dds"), &dds, &mut files)?;
            let mut options = parse_config(
                &mut target,
                "textures/character/human/renesanse_tank/renesanse_tank_torso.options",
                "native texture options",
            )?;
            options
                .root
                .set_path("options.srgb", V::Integer(i32::from(srgb)))?;
            options.root.set_path(
                "options.format",
                string(if role == "normal" {
                    "NORMAL_MAP"
                } else {
                    "DXT1"
                }),
            )?;
            config(
                &overlay,
                &format!("{path}.options"),
                options.root,
                &mut files,
            )?;
            material.root.set_path(
                &format!("material.g_stage.effect.texture_{role}.value"),
                string(format!("{ns}/{part}_{role}")),
            )?;
        }
        // The donor's *_fmap is an old specular-power map, not the modern
        // Fresnel input. Do not interpret it as metallic reflectance. Keep the
        // native shader permutation with a constant stock-like finish.
        crate::outfit_026::set_finish(&mut material, finish)?;
        let material_path = format!("material_instances/{ns}/{part}.material");
        // A revision can reuse an identical installed material. Do not mark it
        // as replaced: that would require auditing every unrelated stock model.
        if !target.is_file(&material_path) || target.read(&material_path)? != material.to_bytes()? {
            config(&overlay, &material_path, material.root, &mut files)?;
        }
    }
    let mut natives = Natives {
        protection: vec![],
        templates: vec![],
    };
    for category in ["helmet", "torso", "legs", "gloves", "boots"] {
        let native = parse_config(
            &mut target,
            &format!("gameplay/items/armour/{category}/renesanse_{category}_6.options"),
            "native protection",
        )?;
        natives.protection.push((category.into(), native.root));
    }
    let mut dictionary = parse_config(&mut target, DICT, "0.26 dictionary")?;
    let V::Indexed(items) = dictionary
        .root
        .named_mut("items_dict")
        .context("items_dict missing")?
    else {
        bail!("expected the 0.26 indexed dictionary");
    };
    for piece in &variant.set {
        let row = items
            .iter()
            .find(|v| v.named("cfg_name") == Some(&string(piece.template)))
            .with_context(|| format!("native {} template missing", piece.template))?
            .clone();
        let config = parse_config(&mut target, piece.template, "native set template")?;
        natives.templates.push((row, config.root));
    }
    // Retire this variant's earlier rows (for Duty the split preview items
    // and the one-item suit in the back slot), never another's or a mod's.
    for &id in variant.reusable {
        if let Some(existing) = items
            .iter()
            .find(|v| v.named("dict_id") == Some(&V::Integer(id)))
        {
            check_reused_id(variant, id, existing)?;
        }
    }
    items.retain(
        |v| !matches!(v.named("dict_id"), Some(V::Integer(id)) if variant.reusable.contains(id)),
    );
    let pieces = set_items(variant, &natives, icons_swf.is_some())?;
    check_coverage(
        &parse_config(&mut target, HIT_TARGETS, "player hit targets")?.root,
        &pieces.iter().map(|(_, _, cfg)| cfg).collect::<Vec<_>>(),
    )?;
    for (piece, row, _) in &pieces {
        ensure!(
            !items
                .iter()
                .any(|v| v.named("cfg_name") == Some(&string(piece.path))),
            "{} already belongs to another item",
            piece.path
        );
        items.push(row.clone());
    }
    let item_configs: Vec<V> = pieces.iter().map(|(_, _, cfg)| cfg.clone()).collect();
    let mut registration = vec![];
    for (piece, row, cfg) in pieces {
        config(&overlay, piece.path, cfg, &mut files)?;
        registration.push(json!({
            "dict_id": piece.id,
            "slot": piece.slot,
            "additional_slots": piece.covered_slots()?,
            "name": piece.label,
            "template": piece.template,
            "weight": piece.weight,
            "item_level": 15,
            "match_making_level": 15,
            "default_modifications": piece.modifications,
            "dictionary_entry": format!("{row:?}"),
            "config": piece.path,
        }));
    }
    if let Some(path) = icons_swf {
        let bytes = fs::read(path)?;
        provenance.insert(
            path.display().to_string(),
            record_file("", "", &bytes).sha256,
        );
        raw(&overlay, "flash_movies/lobby_menu.swf", &bytes, &mut files)?;
    }
    config(&overlay, DICT, dictionary.root, &mut files)?;
    // Every language the client ships, each in its own words.
    let mut localizations = vec![];
    for lang in LANGUAGES {
        let path = format!("localization/{lang}/localization");
        let mut loc = parse_config(&mut target, &path, "localization")?;
        for (key, text) in &variant.strings {
            set_string(&mut loc.root, key, text.get(lang)?)?;
        }
        localizations.push((lang, loc.root));
    }
    let views: Vec<(&str, &V)> = localizations.iter().map(|(l, r)| (*l, r)).collect();
    for cfg in &item_configs {
        check_item_texts(cfg, &views)?;
    }
    for (lang, root) in localizations {
        config(
            &overlay,
            &format!("localization/{lang}/localization"),
            root,
            &mut files,
        )?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(
            &json!({"format_version":1,"asset_id":variant.asset_id,"files":files,"inputs":provenance,"items":registration,"base_db":db,"limitations":["Custom experimental balance; native Renaissance tier requirements","Approximate matte material response; source diffuse and normal textures"]}),
        )?,
    )?;
    fs::write(
        output.join("equipment.json"),
        serde_json::to_vec_pretty(&registration)?,
    )?;
    crate::model_validation::validate_effective_overlay(db, &overlay)?;
    println!(
        "Built the {} set, body {}, helmet {}, gloves {}, boots {}: {} resources",
        variant.key,
        variant.set[0].id,
        variant.set[1].id,
        variant.set[2].id,
        variant.set[3].id,
        files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A native armour config with its regions, as the 0.26g files are.
    fn native(kind: &str, regions: &[&str], data: V) -> V {
        let hit = |names: &[&str]| {
            table(
                names
                    .iter()
                    .map(|n| field(n, table(vec![field("armor", V::Float(0.5))])))
                    .collect(),
            )
        };
        table(vec![
            field(
                "parameters",
                table(vec![
                    field("type", string(kind)),
                    field("weight", V::Integer(20)),
                    field("drop_weight", V::Integer(10)),
                ]),
            ),
            field(
                "hit_params",
                table(
                    regions
                        .iter()
                        .map(|r| {
                            field(
                                r,
                                table(vec![field("hit_types", hit(&["ranged", "toxic"]))]),
                            )
                        })
                        .collect(),
                ),
            ),
            field(
                "ui_desc",
                table(vec![field(
                    "text_descriptions",
                    table(vec![field("name", string("native"))]),
                )]),
            ),
            field("data", data),
            field(
                "data_full",
                table(vec![field("base_model", string("zubr"))]),
            ),
        ])
    }

    fn natives() -> Natives {
        let model = |type_: i32| {
            table(vec![
                field("type", V::Integer(type_)),
                field("base_model", string("character/human/renesanse/zubr")),
                field(
                    "additional_models",
                    V::Indexed(vec![table(vec![field("base_model", string("balaclava"))])]),
                ),
            ])
        };
        let row = |category: i32| {
            table(vec![
                field("cfg_name", string("native")),
                field("name", string("native")),
                field("item_category", V::Integer(category)),
                field("dict_id", V::Integer(316)),
                field("base_cost", V::Integer(105000)),
            ])
        };
        let zubr = |category: &str, regions: &[&str]| {
            (category.to_owned(), native("arm", regions, model(4)))
        };
        Natives {
            protection: vec![
                zubr("helmet", &["brain", "face"]),
                zubr("torso", &["chest", "stomach", "left_arm"]),
                zubr("legs", &["left_leg", "right_leg"]),
                zubr("gloves", &["left_hand"]),
                zubr("boots", &["left_foot"]),
            ],
            templates: vec![
                (row(3), native("arm_tors", &["chest"], model(4))),
                (row(1), native("arm_hlmt", &["brain"], model(4))),
                (row(6), native("arm_hand", &["left_hand"], model(4))),
                (
                    row(7),
                    native("arm_boot", &["left_foot"], {
                        let mut data = model(4);
                        data.set_path("model_type", string("data_long")).unwrap();
                        data
                    }),
                ),
            ],
        }
    }

    fn data_of<'a>(cfg: &'a V, key: &str) -> Option<&'a V> {
        cfg.named("data").and_then(|data| data.named(key))
    }

    fn zones(cfg: &V) -> Vec<String> {
        let Some(V::Named(zones)) = cfg.named("hit_params") else {
            panic!("no hit_params")
        };
        zones.iter().map(|zone| zone.name.clone()).collect()
    }

    /// The ids of a config's lobby card lines (`ui_desc.props_list`).
    fn prop_ids(cfg: &V) -> Vec<i32> {
        let Some(V::Indexed(props)) = cfg.named("ui_desc").and_then(|u| u.named("props_list"))
        else {
            panic!("no props_list")
        };
        props
            .iter()
            .map(|prop| match prop.named("prop_id") {
                Some(V::Integer(id)) => *id,
                other => panic!("prop_id {other:?}"),
            })
            .collect()
    }

    /// The body is armour, as its torso template, and carries no oxygen
    /// (owner, 2026-10-19): an item of `data.type` 1 is an oxygen tank, and
    /// the game then applies neither its `hit_params` nor its modifiers.
    fn assert_body_is_armour_without_oxygen(cfg: &V) {
        assert_eq!(data_of(cfg, "type"), Some(&V::Integer(4)));
        assert_eq!(data_of(cfg, "amount_time_sec"), None);
        assert_eq!(data_of(cfg, "influences"), None);
        assert_eq!(
            cfg.named("parameters")
                .and_then(|p| p.named("oxygen_amount")),
            None
        );
        // Armour, isolation and weight; no "Oxygen capacity" line (11).
        assert_eq!(prop_ids(cfg), [6, 8, 15]);
        assert!(cfg.named("hit_params").is_some());
    }

    #[test]
    fn duty_body_is_torso_armour_without_oxygen() {
        let set = set_items(&DUTY, &natives(), true).unwrap();
        let (piece, row, cfg) = &set[0];
        assert_eq!(piece.id, 1805);
        // The user's report: the suit was a backpack (category 4, slot 3).
        assert_eq!(row.named("item_category"), Some(&V::Integer(3)));
        assert_eq!(piece.slot, 2);
        assert_eq!(row.named("cfg_name"), Some(&string(BODY.path)));
        assert!(BODY.path.starts_with("gameplay/items/armour/torso/"));
        assert_eq!(
            cfg.named("parameters").and_then(|p| p.named("type")),
            Some(&string("arm_tors"))
        );
        // The back unit and the legs are linked cells; the gloves and boots
        // are pieces of their own (owner, 2026-10-17), and the helmet and
        // mask stay free for the set's helmet.
        assert_eq!(piece.covered_slots().unwrap(), [3, 4]);
        assert_eq!(
            data_of(cfg, "additional_model_slots"),
            Some(&V::Indexed(["back", "pants"].map(string).to_vec()))
        );
        assert_body_is_armour_without_oxygen(cfg);
        // The native head shows without the helmet; no head model is worn.
        assert_eq!(data_of(cfg, "remove_head_model"), None);
        let Some(V::Indexed(models)) = data_of(cfg, "additional_models") else {
            panic!("no additional models")
        };
        // Frame, back unit, chest disc, legs: no head, hands or boots.
        assert_eq!(models.len(), 4);
        for part in ["duty_03_head", "duty_03_hand", "duty_03_boot"] {
            assert!(!format!("{models:?}").contains(part), "{part}");
        }
        // The suit itself is drawn: a hud model in the torso's table would
        // take the place of its `base_model` (the user's invisible jacket).
        assert_eq!(data_of(cfg, "base_model_hud"), None);
        assert_eq!(data_of(cfg, "part_name_hud"), None);
        assert_eq!(
            data_of(cfg, "base_model"),
            Some(&string("ported/duty_026/duty_03_torso_s1"))
        );
        assert!(models.iter().all(|m| m.named("base_model_hud").is_none()));
        // Every sleeve table a gloves' `model_type` may pick is the suit,
        // with its frame; none names a native model.
        for key in TORSO_TABLES {
            let sleeve = cfg.named(key).unwrap_or_else(|| panic!("no {key}"));
            assert_eq!(sleeve.named("base_model"), data_of(cfg, "base_model"));
            assert_eq!(
                sleeve.named("additional_models"),
                data_of(cfg, "additional_models")
            );
            assert_eq!(sleeve.named("base_model_hud"), None);
        }
        assert!(!format!("{cfg:?}").contains("zubr"));
        assert_eq!(
            cfg.named("default_modifications"),
            Some(&V::Indexed(
                [1030, 1050, 1170, 1040].map(V::Integer).to_vec()
            ))
        );
        assert_eq!(
            cfg.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(1), V::Integer(6)]))
        );
    }

    #[test]
    fn duty_helmet_covers_the_mask_like_zubrs() {
        let set = set_items(&DUTY, &natives(), true).unwrap();
        let (piece, row, cfg) = &set[1];
        assert_eq!((piece.id, piece.slot), (1800, 0));
        assert_eq!(row.named("item_category"), Some(&V::Integer(1)));
        assert_eq!(row.named("name"), Some(&string("duty_exo_helmet")));
        assert_eq!(piece.covered_slots().unwrap(), [1]);
        assert_eq!(data_of(cfg, "type"), Some(&V::Integer(4)));
        assert_eq!(data_of(cfg, "remove_head_model"), Some(&V::Boolean(true)));
        assert_eq!(
            data_of(cfg, "base_model"),
            Some(&string("ported/duty_026/duty_03_head"))
        );
        // Zubr's balaclava head would show inside the Duty head.
        assert_eq!(data_of(cfg, "additional_models"), None);
        assert_eq!(
            cfg.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(3), V::Integer(6)]))
        );
    }

    #[test]
    fn every_piece_spells_stamina_regeneration_as_the_exe_reads_it() {
        // The stock templates' misspelt key, as weapon balance 1 found it.
        let mut natives = natives();
        for (_, template) in &mut natives.templates {
            template
                .set_path(
                    &format!("parameters.{STAMINA_REGEN_MISSPELT}"),
                    V::Integer(7),
                )
                .unwrap();
        }
        for variant in [&DUTY, &FREEDOM] {
            for (piece, _, cfg) in set_items(variant, &natives, true).unwrap() {
                let parameters = cfg.named("parameters").unwrap();
                assert_eq!(
                    parameters.named(STAMINA_REGEN),
                    Some(&V::Integer(7)),
                    "{}",
                    piece.name
                );
                assert_eq!(parameters.named(STAMINA_REGEN_MISSPELT), None);
            }
        }
        // Without the misspelt key nothing is added.
        let set = set_items(&DUTY, &self::natives(), true).unwrap();
        assert!(set.iter().all(|(_, _, cfg)| {
            cfg.named("parameters")
                .unwrap()
                .named(STAMINA_REGEN)
                .is_none()
        }));
    }

    #[test]
    fn the_gloves_and_boots_are_pieces_of_their_own_as_zubrs() {
        for variant in [&DUTY, &FREEDOM] {
            let ns = variant.ns;
            let set = set_items(variant, &natives(), true).unwrap();
            let (gloves, boots) = (&set[2], &set[3]);
            assert_eq!(gloves.0.kind, Kind::Gloves);
            assert_eq!(boots.0.kind, Kind::Boots);
            // Their own slots and categories, covering nothing.
            assert_eq!(gloves.0.slot, 5);
            assert_eq!(boots.0.slot, 6);
            assert_eq!(gloves.1.named("item_category"), Some(&V::Integer(6)));
            assert_eq!(boots.1.named("item_category"), Some(&V::Integer(7)));
            for (piece, _, cfg) in [gloves, boots] {
                assert!(piece.covered_slots().unwrap().is_empty());
                assert_eq!(
                    data_of(cfg, "additional_model_slots"),
                    Some(&V::Indexed(vec![]))
                );
                assert_eq!(data_of(cfg, "type"), Some(&V::Integer(4)));
                // As native gloves and boots: no modifications, drop weight 10.
                assert!(cfg.named("default_modifications").is_none());
                assert_eq!(
                    cfg.named("parameters").unwrap().named("drop_weight"),
                    Some(&V::Integer(10))
                );
                assert!(!format!("{cfg:?}").contains("zubr"));
            }
            // The gloves give the first-person hands, as native gloves do,
            // and the sleeves they are worn with.
            assert_eq!(
                data_of(&gloves.2, "base_model"),
                Some(&string(format!("{ns}/duty_03_hand")))
            );
            assert_eq!(
                data_of(&gloves.2, "base_model_hud"),
                Some(&string(format!("{ns}/duty_03_hand_hud")))
            );
            assert_eq!(
                data_of(&gloves.2, "part_name_hud"),
                Some(&string("torso_s"))
            );
            assert_eq!(data_of(&gloves.2, "model_type"), Some(&string("data_full")));
            assert_eq!(
                data_of(&boots.2, "base_model"),
                Some(&string(format!("{ns}/duty_03_boot")))
            );
            assert_eq!(data_of(&boots.2, "model_type"), Some(&string("data_long")));
        }
        let duty = set_items(&DUTY, &natives(), true).unwrap();
        assert_eq!((duty[2].0.id, duty[3].0.id), (1803, 1804));
        assert_eq!(
            duty[2].2.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(6), V::Integer(6)]))
        );
        let freedom = set_items(&FREEDOM, &natives(), true).unwrap();
        assert_eq!((freedom[2].0.id, freedom[3].0.id), (1809, 1811));
        assert_eq!(
            freedom[2].0.template,
            "gameplay/items/armour/gloves/edge_gloves_6.options"
        );
        assert_eq!(
            freedom[3].0.template,
            "gameplay/items/armour/boots/edge_boots_6.options"
        );
    }

    #[test]
    fn duty_set_splits_the_suit_without_overlap() {
        let set = set_items(&DUTY, &natives(), false).unwrap();
        let (body, head) = (&set[0], &set[1]);
        // Worn together: no piece covers another's slot, and the four fill
        // the seven armour slots.
        let mut all: Vec<u8> = set
            .iter()
            .flat_map(|(piece, _, _)| [vec![piece.slot], piece.covered_slots().unwrap()].concat())
            .collect();
        all.sort_unstable();
        assert_eq!(all, [0, 1, 2, 3, 4, 5, 6]);
        // Each region is protected by one piece, as the native pieces split
        // them.
        assert_eq!(zones(&head.2), ["brain", "face"]);
        assert_eq!(
            zones(&body.2),
            ["chest", "left_arm", "left_leg", "right_leg", "stomach"]
        );
        assert_eq!(zones(&set[2].2), ["left_hand"]);
        assert_eq!(zones(&set[3].2), ["left_foot"]);
        // The set keeps the owner's totals: 35 kg and 300 000.
        let weight: f32 = set.iter().map(|(piece, _, _)| piece.weight).sum();
        assert!((weight - 35.0).abs() < 1e-4, "{weight}");
        let cost = |row: &V| match row.named("base_cost") {
            Some(V::Integer(cost)) => *cost,
            _ => panic!("no cost"),
        };
        assert_eq!(
            set.iter().map(|(_, row, _)| cost(row)).sum::<i32>(),
            300_000
        );
        // The gloves and boots weigh and cost Zubr UM-4's.
        assert_eq!((set[2].0.weight, cost(&set[2].1)), (0.6, 26_250));
        assert_eq!((set[3].0.weight, cost(&set[3].1)), (3.0, 26_250));
        // Every piece drops as the native piece it is made from (the
        // fixture's 10, Zubr UM-4's and Pilgrim's), the body as the torso.
        let drop = |cfg: &V| {
            cfg.named("parameters")
                .unwrap()
                .named("drop_weight")
                .cloned()
        };
        for (_, _, cfg) in &set {
            assert_eq!(drop(cfg), Some(V::Integer(10)));
        }
        for (_, _, cfg) in &set {
            let p = cfg.named("parameters").unwrap();
            assert_eq!(p.named("item_level"), Some(&V::Integer(15)));
        }
        // No icon swf: the template's own icon stays.
        assert!(body.2.named("ui_desc").unwrap().named("icon").is_none());
    }

    #[test]
    fn only_earlier_duty_rows_are_replaced() {
        let row = |path: &str, name: &str| {
            table(vec![
                field("cfg_name", string(path)),
                field("name", string(name)),
            ])
        };
        let back = row(
            "gameplay/items/armour/back/duty_exo_026.options",
            "duty_exo_back",
        );
        // The released one-item suit, and this set's own rows, are replaced.
        check_reused_id(&DUTY, 1805, &back).unwrap();
        check_reused_id(&DUTY, 1805, &row(BODY.path, BODY.name)).unwrap();
        check_reused_id(&DUTY, 1800, &row(HELMET.path, HELMET.name)).unwrap();
        // Another mod's item, or a Duty row under a foreign ID, is not.
        assert!(
            check_reused_id(&DUTY, 1805, &row("gameplay/items/other.options", "other")).is_err()
        );
        assert!(check_reused_id(&DUTY, 1803, &row(BODY.path, BODY.name)).is_err());
        assert!(check_reused_id(&DUTY, 1804, &back).is_err());
    }

    #[test]
    fn freedom_is_its_own_fringe_set_without_the_chest_disc() {
        let set = set_items(&FREEDOM, &natives(), true).unwrap();
        let (body, head) = (&set[0], &set[1]);
        assert_eq!((body.0.id, body.0.slot), (1807, 2));
        assert_eq!((head.0.id, head.0.slot), (1808, 0));
        assert_eq!(
            body.0.template,
            "gameplay/items/armour/torso/edge_torso_6.options"
        );
        assert_eq!(
            head.0.template,
            "gameplay/items/armour/helmet/edge_helmet_6.options"
        );
        assert_eq!(body.0.covered_slots().unwrap(), [3, 4]);
        assert_eq!(head.0.covered_slots().unwrap(), [1]);
        // The owner's decision: no chest disc, dropped rather than hidden.
        let Some(V::Indexed(models)) = data_of(&body.2, "additional_models") else {
            panic!("no additional models")
        };
        assert_eq!(models.len(), 3);
        let text = format!("{models:?}");
        assert!(!text.contains("sign_s"));
        assert!(text.contains("ported/freedom_026/duty_03_torso_s1"));
        assert!(!text.contains("duty_026"));
        assert_eq!(
            data_of(&head.2, "base_model"),
            Some(&string("ported/freedom_026/duty_03_head"))
        );
        // Armour, as Duty's: no oxygen, the artefact slot (1040) kept.
        assert_body_is_armour_without_oxygen(&body.2);
        // Stats: 33 kg and 300 000 for the set, the gloves and boots as
        // Pilgrim's; +3% speed and -4% stamina, and no pocket capacity (1170,
        // which Duty keeps).
        let weight: f32 = set.iter().map(|(piece, _, _)| piece.weight).sum();
        assert!((weight - 33.0).abs() < 1e-4, "{weight}");
        let cost = |row: &V| match row.named("base_cost") {
            Some(V::Integer(cost)) => *cost,
            _ => panic!("no cost"),
        };
        assert_eq!(
            set.iter().map(|(_, row, _)| cost(row)).collect::<Vec<_>>(),
            [177_500, 75_000, 23_750, 23_750]
        );
        assert_eq!((set[2].0.weight, set[3].0.weight), (0.4, 1.9));
        assert_eq!(
            body.2.named("default_modifications"),
            Some(&V::Indexed(
                [1030, 1050, 1040, 1112, 1161].map(V::Integer).to_vec()
            ))
        );
        assert!(head.2.named("default_modifications").is_none());
        let param = |cfg: &V, key: &str| cfg.named("parameters").unwrap().named(key).cloned();
        assert_eq!(param(&body.2, "armor"), Some(V::Float(0.85)));
        assert_eq!(param(&head.2, "armor"), Some(V::Float(0.70)));
        assert_eq!(param(&body.2, "ano_armor"), Some(V::Float(0.80)));
        assert_eq!(param(&head.2, "ano_armor"), Some(V::Float(0.80)));
        assert_eq!(param(&body.2, "drop_weight"), Some(V::Integer(10)));
        assert_eq!(param(&head.2, "drop_weight"), Some(V::Integer(10)));
        assert_eq!(param(&body.2, "item_level"), Some(V::Integer(15)));
        assert_eq!(
            body.2
                .named("ui_desc")
                .and_then(|u| u.named("text_descriptions"))
                .and_then(|t| t.named("name")),
            Some(&string("st_freedom_026_suit_name"))
        );
        assert_eq!(
            body.2.named("ui_desc").and_then(|u| u.named("icon")),
            Some(&V::Indexed(vec![V::Integer(4), V::Integer(6)]))
        );
    }

    #[test]
    fn each_variant_replaces_only_its_own_rows() {
        let row = |path: &str, name: &str| {
            table(vec![
                field("cfg_name", string(path)),
                field("name", string(name)),
            ])
        };
        check_reused_id(&FREEDOM, 1807, &row(FREEDOM_BODY.path, FREEDOM_BODY.name)).unwrap();
        check_reused_id(
            &FREEDOM,
            1808,
            &row(FREEDOM_HELMET.path, FREEDOM_HELMET.name),
        )
        .unwrap();
        // Freedom never takes Duty's rows, nor Duty Freedom's.
        assert!(check_reused_id(&FREEDOM, 1805, &row(BODY.path, BODY.name)).is_err());
        assert!(check_reused_id(&FREEDOM, 1800, &row(HELMET.path, HELMET.name)).is_err());
        assert!(check_reused_id(&DUTY, 1807, &row(FREEDOM_BODY.path, FREEDOM_BODY.name)).is_err());
        // Each set's gloves and boots are its own.
        check_reused_id(&DUTY, 1803, &row(GLOVES.path, GLOVES.name)).unwrap();
        check_reused_id(&DUTY, 1804, &row(BOOTS.path, BOOTS.name)).unwrap();
        check_reused_id(
            &FREEDOM,
            1809,
            &row(FREEDOM_GLOVES.path, FREEDOM_GLOVES.name),
        )
        .unwrap();
        check_reused_id(&FREEDOM, 1811, &row(FREEDOM_BOOTS.path, FREEDOM_BOOTS.name)).unwrap();
        assert!(check_reused_id(&FREEDOM, 1803, &row(GLOVES.path, GLOVES.name)).is_err());
        // Duty's legacy split rows are Duty's only.
        let back = row(
            "gameplay/items/armour/back/duty_exo_026.options",
            "duty_exo_back",
        );
        assert!(check_reused_id(&FREEDOM, 1805, &back).is_err());
        // Disjoint ID ranges, and clear of the balaclava (1810).
        assert!(!DUTY.reusable.contains(&1807) && !FREEDOM.reusable.contains(&1806));
        assert!(!FREEDOM.reusable.contains(&1810));
        assert_eq!(variant("freedom").unwrap().ns, "ported/freedom_026");
        assert!(variant("svoboda").is_err());
    }
    /// The player's twelve hit regions (`base_character_full` hit_targets)
    /// and the native level 15 pieces that protect them.
    const HEAD_REGIONS: [&str; 2] = ["brain", "face"];
    const NATIVE_REGIONS: [(&str, &[&str]); 5] = [
        ("helmet", &HEAD_REGIONS),
        ("torso", &["back", "front", "left_forearm", "right_forearm"]),
        ("legs", &["left_hip", "right_hip"]),
        ("gloves", &["left_arm", "right_arm"]),
        ("boots", &["left_foot", "right_foot"]),
    ];

    fn real_natives() -> Natives {
        let mut natives = natives();
        let hit_types = [
            "explosive",
            "fire",
            "radiation",
            "ranged",
            "shock",
            "slash",
            "toxic",
        ];
        natives.protection = NATIVE_REGIONS
            .iter()
            .map(|(category, regions)| {
                let mut cfg = native("arm", regions, table(vec![]));
                let parts = regions
                    .iter()
                    .map(|r| {
                        let types = hit_types
                            .iter()
                            .map(|t| {
                                field(
                                    t,
                                    table(vec![
                                        field("armor", V::Float(0.5)),
                                        field("absorption", V::Float(0.1)),
                                    ]),
                                )
                            })
                            .collect();
                        field(r, table(vec![field("hit_types", table(types))]))
                    })
                    .collect();
                cfg.set_path("hit_params", table(parts)).unwrap();
                ((*category).to_owned(), cfg)
            })
            .collect();
        natives
    }

    fn hit_targets(regions: &[&str]) -> V {
        table(vec![field(
            "hit_targets",
            V::Indexed(
                regions
                    .iter()
                    .map(|r| table(vec![field("hit_param", string(*r))]))
                    .collect(),
            ),
        )])
    }

    fn all_regions() -> Vec<&'static str> {
        NATIVE_REGIONS
            .iter()
            .flat_map(|(_, r)| r.iter().copied())
            .collect()
    }

    #[test]
    fn each_piece_protects_its_native_pieces_regions_at_the_sets_armour() {
        // The set covers all twelve regions once: the body the native
        // torso's and trousers', the gloves the arms, the boots the feet.
        for variant in [&DUTY, &FREEDOM] {
            let set = set_items(variant, &real_natives(), false).unwrap();
            let cfgs: Vec<&V> = set.iter().map(|(_, _, cfg)| cfg).collect();
            check_coverage(&hit_targets(&all_regions()), &cfgs).unwrap();
            let expect = |categories: &[&str]| {
                let mut regions: Vec<&str> = NATIVE_REGIONS
                    .iter()
                    .filter(|(c, _)| categories.contains(c))
                    .flat_map(|(_, r)| r.iter().copied())
                    .collect();
                regions.sort_unstable();
                regions
            };
            assert_eq!(
                zones(cfgs[0]),
                expect(&["torso", "legs"]),
                "{}",
                variant.key
            );
            assert_eq!(zones(cfgs[1]), HEAD_REGIONS, "{}", variant.key);
            assert_eq!(zones(cfgs[2]), ["left_arm", "right_arm"], "{}", variant.key);
            assert_eq!(
                zones(cfgs[3]),
                ["left_foot", "right_foot"],
                "{}",
                variant.key
            );
            for (index, cfg) in cfgs.iter().enumerate() {
                let ballistic = if index == 1 {
                    variant.ballistic.1
                } else {
                    variant.ballistic.0
                };
                let Some(V::Named(parts)) = cfg.named("hit_params") else {
                    unreachable!()
                };
                for part in parts {
                    let Some(V::Named(types)) = part.value.named("hit_types") else {
                        panic!("{}", part.name)
                    };
                    for hit in types {
                        let wanted =
                            if ["ranged", "slash", "explosive"].contains(&hit.name.as_str()) {
                                ballistic
                            } else {
                                variant.anomaly
                            };
                        assert_eq!(
                            hit.value.named("armor"),
                            Some(&V::Float(wanted)),
                            "{} {} {}",
                            variant.key,
                            part.name,
                            hit.name
                        );
                        // The native piece's other values stay.
                        assert_eq!(hit.value.named("absorption"), Some(&V::Float(0.1)));
                    }
                }
                let parameters = cfg.named("parameters").unwrap();
                assert_eq!(parameters.named("armor"), Some(&V::Float(ballistic)));
                assert_eq!(
                    parameters.named("ano_armor"),
                    Some(&V::Float(variant.anomaly))
                );
            }
        }
    }

    #[test]
    fn a_region_left_unprotected_or_protected_twice_stops_the_build() {
        let set = set_items(&DUTY, &real_natives(), false).unwrap();
        let cfgs: Vec<&V> = set.iter().map(|(_, _, cfg)| cfg).collect();
        // A hit region no piece protects (say, a future neck target).
        let mut more = all_regions();
        more.push("neck");
        assert!(check_coverage(&hit_targets(&more), &cfgs).is_err());
        // The set without its gloves leaves the arms unprotected.
        let without = [cfgs[0], cfgs[1], cfgs[3]];
        assert!(check_coverage(&hit_targets(&all_regions()), &without).is_err());
        // The helmet protecting a region the body also does.
        let mut both = cfgs[1].clone();
        let front = cfgs[0]
            .named("hit_params")
            .unwrap()
            .named("front")
            .unwrap()
            .clone();
        let Some(V::Named(parts)) = both.named_mut("hit_params") else {
            unreachable!()
        };
        parts.push(field("front", front));
        let twice = [cfgs[0], &both, cfgs[2], cfgs[3]];
        assert!(check_coverage(&hit_targets(&all_regions()), &twice).is_err());
        // A region without every hit type.
        let mut thin = cfgs[0].clone();
        thin.set_path("hit_params.front.hit_types", table(vec![]))
            .unwrap();
        let thin_set = [&thin, cfgs[1], cfgs[2], cfgs[3]];
        assert!(check_coverage(&hit_targets(&all_regions()), &thin_set).is_err());
    }

    /// The client's localizations with only a strings table, each given the
    /// texts in its own language.
    fn localizations(texts: &[(&str, Text)]) -> Vec<(&'static str, V)> {
        LANGUAGES
            .iter()
            .map(|&language| {
                let mut root = table(vec![field("strings", table(vec![]))]);
                for (key, text) in texts {
                    set_string(&mut root, key, text.get(language).unwrap()).unwrap();
                }
                (language, root)
            })
            .collect()
    }

    #[test]
    fn every_set_piece_is_named_and_described_in_every_client_language() {
        for variant in [&DUTY, &FREEDOM] {
            let roots = localizations(&variant.strings);
            let views: Vec<(&str, &V)> = roots.iter().map(|(l, r)| (*l, r)).collect();
            for (piece, _, cfg) in set_items(variant, &natives(), true).unwrap() {
                check_item_texts(&cfg, &views).unwrap_or_else(|e| panic!("{}: {e:#}", piece.id));
            }
            for (key, text) in &variant.strings {
                // Russian and Ukrainian in Cyrillic, every language its own words.
                for cyrillic in [text.russian, text.ukrainian] {
                    assert!(
                        cyrillic
                            .chars()
                            .any(|c| ('\u{400}'..='\u{4ff}').contains(&c)),
                        "{key}"
                    );
                }
                for other in [text.russian, text.ukrainian, text.german, text.polish] {
                    assert_ne!(other, text.english, "{key} is untranslated");
                }
            }
        }
        // A key no language has stops the build.
        let roots = localizations(&DUTY.strings[..3]);
        let views: Vec<(&str, &V)> = roots.iter().map(|(l, r)| (*l, r)).collect();
        let set = set_items(&DUTY, &natives(), true).unwrap();
        assert!(check_item_texts(&set[1].2, &views).is_err());
    }

    #[test]
    fn each_piece_drops_as_its_native_template_which_must_drop() {
        let mut natives = natives();
        natives.templates[1]
            .1
            .set_path("parameters.drop_weight", V::Integer(0))
            .unwrap();
        assert!(set_items(&DUTY, &natives, false).is_err());
        // The body takes its torso template's weight whatever it is: no
        // halving any more (owner, 2026-10-17).
        let mut torso = super::tests::natives();
        torso.templates[0]
            .1
            .set_path("parameters.drop_weight", V::Integer(7))
            .unwrap();
        let set = set_items(&DUTY, &torso, false).unwrap();
        assert_eq!(
            set[0].2.named("parameters").unwrap().named("drop_weight"),
            Some(&V::Integer(7))
        );
    }

    #[test]
    fn backpack_split_keeps_whole_components_and_vertex_attributes() {
        // Two rear triangles, one joined across a UV seam to a frame triangle.
        let positions = [
            [0., 1.2, 0.2],
            [0.01, 1.2, 0.2],
            [0., 1.21, 0.2],
            [0.1, 1.4, 0.3],
            [0.11, 1.4, 0.3],
            [0.1, 1.41, 0.3],
            [0., 1.2, 0.2],
            [0.4, 1.2, 0.2],
            [0.4, 1.3, 0.2],
        ];
        let mut b = vec![];
        for v in [3u32, 4 + 9 * 48, 9] {
            b.extend(v.to_le_bytes());
        }
        for (i, p) in positions.iter().enumerate() {
            let mut v = [0u8; 48];
            for (axis, &x) in p.iter().enumerate() {
                v[axis * 4..axis * 4 + 4].copy_from_slice(&f32::to_le_bytes(x));
            }
            v[16..20].copy_from_slice(&1f32.to_le_bytes());
            v[40..44].copy_from_slice(&(i as f32 / 10.).to_le_bytes());
            b.extend(v);
        }
        for v in [4u32, 22, 9] {
            b.extend(v.to_le_bytes());
        }
        for v in 0..9u16 {
            b.extend(v.to_le_bytes());
        }
        let back = split_back(&b, true, &[]).unwrap();
        let frame = split_back(&b, false, &[]).unwrap();
        assert_eq!(u32_at(&back, 8).unwrap(), 3);
        assert_eq!(u32_at(&frame, 8).unwrap(), 6);
        assert_eq!(&back[12..156], &b[12 + 3 * 48..12 + 6 * 48]);
        assert_eq!(&frame[12..156], &b[12..156]);
        assert_eq!(&frame[156..300], &b[12 + 6 * 48..12 + 9 * 48]);
        // An anchor on the duplicate vertex also selects the triangle across
        // the UV seam. Future anchored partitions use this same contract.
        assert_eq!(split_components(&b, true, &[6], false).unwrap(), frame);
        assert_eq!(split_components(&b, false, &[6], false).unwrap(), back);
        assert!(split_components(&b, true, &[9], false).is_err());
        rotate_mesh(&back, 69).unwrap();
        rotate_mesh(&frame, 69).unwrap();
    }
    #[test]
    fn texture_header_matches_026_without_changing_compressed_pixels() {
        let mut source = vec![0u8; 128 + 40];
        source[..4].copy_from_slice(b"DDS ");
        for (at, value) in [
            (4, 124u32),
            (8, 0xa1007),
            (12, 8),
            (16, 8),
            (20, 32),
            (28, 2),
            (76, 32),
            (80, 4),
            (108, 0x401008),
        ] {
            source[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        source[84..88].copy_from_slice(b"DXT1");
        source[128..].fill(0x5a);
        let out = texture_dds(&source, "fixture", true).unwrap();
        assert_eq!(u32_at(&out, 8).unwrap(), 0xa100f);
        assert_eq!(u32_at(&out, 20).unwrap(), 41);
        assert_eq!(&out[128..168], &source[128..]);
        assert_eq!(out[168], 1);
    }
    #[test]
    fn editor_order_is_converted_without_swapping_payloads() {
        let cfg = BinaryConfig {
            root: table(vec![
                field("value", string("kept")),
                field("v", V::Float3([1., 2., 3.])),
            ]),
        };
        let mut b = cfg.to_bytes().unwrap();
        // Root and its two scalar children are the only nodes.
        for at in [0, 24, 48] {
            for i in 0..8 {
                b.swap(at + i, at + i + 8);
            }
        }
        let decoded = editor_config(&b).unwrap();
        assert_eq!(decoded.root.named("value"), cfg.root.named("value"));
        assert_eq!(decoded.root.named("v"), cfg.root.named("v"));
        assert!(editor_config(&b[..25]).is_err());
    }
    #[test]
    fn mesh_rotation_keeps_weights_uv_and_winding() {
        let mut b = vec![0u8; 12 + 3 * 48 + 12 + 6];
        for (at, v) in [(0, 3u32), (4, 148), (8, 3), (156, 4), (160, 10), (164, 3)] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        for i in 0..3 {
            let at = 12 + i * 48;
            for (o, v) in [(0, 1f32), (4, 2.), (8, 3.), (16, 1.), (40, 0.2), (44, 0.7)] {
                b[at + o..at + o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        for i in 0..3 {
            b[168 + i * 2..170 + i * 2].copy_from_slice(&(i as u16).to_le_bytes());
        }
        let (out, min, max, _, _) = rotate_mesh(&b, 69).unwrap();
        assert_eq!(min, [-1., 2., -3.]);
        assert_eq!(max, min);
        assert_eq!(&out[24..40], &b[24..40]);
        assert_eq!(&out[52..60], &b[52..60]);
        assert_eq!(&out[156..], &b[156..]);
        let (twice, _, _, _, _) = rotate_mesh(&out, 69).unwrap();
        assert_eq!(twice, b);
        b[24] = 69;
        assert!(rotate_mesh(&b, 69).is_err());
    }
}
