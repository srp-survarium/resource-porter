// SPDX-License-Identifier: GPL-3.0-or-later
//! The Fringe Settlers' "Keeper" helmet (`edge_helmet_5`) with its visor
//! closed: a second item beside the native one, which keeps its raised visor
//! (owner's request, 2026-10-17). `docs/keeper-closed-026.md`.
//!
//! No build of the game has a closed pose: the model is the same from 0.26g
//! to 0.28d (0.31a2 and 0.46c2 recompress it, and 0.31a2 gives
//! `edge_helmet_5` another model). The visor is its own UV islands of the
//! one surface, hinged on the two teardrop tabs of the shell's side band.
//! Closed, the visor and the tabs turn down about the tabs' screws and the
//! plate slides down and forward until its grille is on the native head's
//! eyes (the arms lengthen); the tabs' old place is filled with the hinge
//! disc's dark metal, a flap closes the gap between the plate and the brow,
//! and the helmet has its own diffuse texture without the darkness the raised
//! visor baked into the parts it covered (`scripts/keeper-closed-026-texture.py`).
use crate::content_pack::{AtlasIcon, Operation, PackManifest, patch::base_layers, value};
use crate::{BinaryConfig, ConfigValue as V, NamedValue, ResourceDb};
use anyhow::{Context, Result, bail, ensure};
use encoding_rs::WINDOWS_1251;
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const PACK_ID: &str = "keeper-closed";
pub const PACK_VERSION: &str = "2";
/// Free in the stock dictionary and every Skhid patch and pack (1800-1811
/// are the STK2 ports, 1815/1816 are planned for the STK2 stalker suit).
pub const DICT_ID: i32 = 1820;
/// `ui_desc.icon`: the sixth atlas column's first rows are the ports'.
pub const ICON: (u32, u32) = (14, 6);
pub const ITEM: &str = "gameplay/items/armour/helmet/edge_helmet_5_closed.options";
pub const MODEL: &str = "models/character/human/edge/edge_assault_05_helmet_closed.skinned_model";
/// How far the visor turns down from the native (raised) pose, about the
/// tabs' screws.
pub const CLOSED_DEGREES: f32 = 82.0;
/// How far the plate then slides forward (m): 8.8 mm clear of the native
/// player head (`black_market_01_head_s`) at its closest. It slides down by
/// as much as puts the grille's centre on the eyes' height.
pub const SLIDE_FORWARD: f32 = 0.035;
/// The slide takes the plate whole from this far from the screws, and the
/// arms not at all to this near: between, they lengthen (m).
const SLIDE_FULL: f32 = 0.08;
const SLIDE_NONE: f32 = 0.045;
/// The grille's centre on the texture (on the plate, island 22).
const GRILLE_UV: [f32; 2] = [0.095, 0.5];
/// The native player head's bind pose, for its eyes.
const HEAD_BIND_POSE: &str =
    "models/character/human/base/base_character_head.skinned_model/render/bind_pose";
pub const TEXTURE: &str = "textures/character/human/edge_assault_05/edge_assault_05_helmet_closed";
const NATIVE_TEXTURE: &str = "textures/character/human/edge_assault_05/edge_assault_05_helmet";
pub const MATERIAL: &str =
    "material_instances/characters/human/edge_assault_05/edge_assault_05_helmet_closed.material";
const NATIVE_MATERIAL: &str =
    "material_instances/characters/human/edge_assault_05/edge_assault_05_helmet.material";

const NATIVE_ITEM: &str = "gameplay/items/armour/helmet/edge_helmet_5.options";
const NATIVE_ITEM_SHA256: &str = "c91595a42cb40a16d6b2968bb4b1a637ec6f4d72fd7e1fe15f095ca1c7bdb655";
const NATIVE_MODEL: &str = "models/character/human/edge/edge_assault_05_helmet.skinned_model";
const SURFACE: &str = "edge_assault_05_helmet_s";
const NATIVE_SURFACE_SHA256: &str =
    "4d3d5a775e02d4cf1d360d1fe497372bdb2ed5a5eaf00f47468c105a2a98517c";
const NAME_KEY: &str = "st_edge_helmet_5_closed_name";
const NATIVE_NAME_KEY: &str = "st_edge_helmet_5_name";
/// Every file of the native model, copied as it is but the surface's mesh
/// and its export properties.
const MODEL_FILES: [&str; 6] = [
    "settings",
    "skeleton",
    "render/bind_pose",
    "render/export_properties",
    "render/edge_assault_05_helmet_s/export_properties",
    "render/edge_assault_05_helmet_s/converted_model",
];

/// Position, bone (the head, 44), normal, tangent, binormal, UV.
const STRIDE: usize = 36;
const NATIVE_VERTICES: usize = 870;
const NATIVE_INDICES: usize = 3246;
/// The visor: the UV islands wholly above the brow and before the hinge
/// (the plate, its rim, underside and arms). Island 12, the wedge under the
/// raised plate, stays with the shell.
const VISOR_MIN_Y: f32 = 1.735;
const VISOR_VERTICES: usize = 130;
const VISOR_TRIANGLES: usize = 138;
/// The teardrop tabs the visor's arms end in, on the right (+x) and left
/// sides, vertex for mirrored vertex: the outline (the first
/// [`TAB_OUTLINE`]) and the inside; the screw is [`SCREWS`]. They share their
/// outline with the shell's hinge disc, so the turned tabs are a copy; the
/// native tabs' triangles become the fill of their old place.
const RIGHT_TAB: [u16; 18] = [
    155, 153, 146, 148, 150, 152, 158, 160, 161, 173, 154, 168, 165, 163, 164, 166, 167, 169,
];
const LEFT_TAB: [u16; 18] = [
    481, 480, 473, 474, 476, 478, 484, 487, 485, 498, 479, 495, 490, 492, 491, 493, 494, 496,
];
const SCREWS: [u16; 2] = [163, 492];
const TAB_OUTLINE: usize = 10;
const TAB_TRIANGLES: usize = 46;
/// The turned copy of a tab sits this far outside the native one.
const TAB_OFFSET: f32 = 0.001;
/// The fill of a tab's old place: flat at this |x| inside its outline, drawn
/// from the hinge disc's dark ring (this far from the screw on the texture),
/// shrunk to this much of the tab's own texture layout.
const FILL_X: f32 = 0.111;
const FILL_DARK: [f32; 2] = [-0.02, 0.05];
const FILL_SCALE: f32 = 0.15;
/// The flap from the closed plate's top to the brow: columns at these x,
/// their front (just inside the plate's top) and back (under the brow's
/// lower edge) as (y, z) for |x| up to 0.075, 0.095 and beyond.
const FLAP_X: [f32; 9] = [-0.103, -0.09, -0.06, -0.03, 0.0, 0.03, 0.06, 0.09, 0.103];
const FLAP_FRONT: [[f32; 2]; 3] = [[1.7195, 0.1525], [1.7255, 0.1305], [1.7295, 0.1205]];
const FLAP_BACK: [[f32; 2]; 3] = [[1.7485, 0.107], [1.745, 0.092], [1.7445, 0.088]];
/// The flap's texels: the dome's lit middle (u0, v0, u1, v1), v0 at the back.
const FLAP_UV: [f32; 4] = [0.40, 0.73, 0.57, 0.83];

/// One text in the client's six localizations (`original` is the Russian).
struct Text {
    english: &'static str,
    russian: &'static str,
    ukrainian: &'static str,
    german: &'static str,
    polish: &'static str,
}

/// The native name, then the closed one.
const NATIVE_NAME: Text = Text {
    english: "\"Keeper\" helmet",
    russian: "Шлем \"Хранитель\"",
    ukrainian: "Шолом \"Хранитель\"",
    german: "Helm \"Hüter\"",
    polish: "Hełm \"Strażnik\"",
};
const NAME: Text = Text {
    english: "\"Keeper\" helmet (closed)",
    russian: "Шлем \"Хранитель\" (закрытый)",
    ukrainian: "Шолом \"Хранитель\" (закритий)",
    german: "Helm \"Hüter\" (geschlossen)",
    polish: "Hełm \"Strażnik\" (zamknięty)",
};

impl Text {
    fn get(&self, language: &str) -> Result<&'static str> {
        Ok(match language {
            "english" => self.english,
            "original" | "russian" => self.russian,
            "ukrainian" => self.ukrainian,
            "german" => self.german,
            "polish" => self.polish,
            _ => bail!("no text for the {language} localization"),
        })
    }
}

/// The client's localization folders, the Russian two together.
const LANGUAGE_GROUPS: [&[&str]; 5] = [
    &["english"],
    &["original", "russian"],
    &["ukrainian"],
    &["german"],
    &["polish"],
];

/// A text as the localization files hold it: their strings are UTF-8 bytes,
/// which a binary config reads as Windows-1251.
fn game_string(text: &str) -> String {
    WINDOWS_1251
        .decode_without_bom_handling(text.as_bytes())
        .0
        .into_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn u32_at(b: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .context("truncated converted model")?
            .try_into()?,
    ) as usize)
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Turns `(y, z)` by `angle` about `pivot` (about +x: +y towards +z, down
/// over the face for the raised visor).
fn turn(y: f32, z: f32, pivot: [f32; 2], angle: f32) -> (f32, f32) {
    let (s, c) = angle.sin_cos();
    let (dy, dz) = (y - pivot[0], z - pivot[1]);
    (pivot[0] + dy * c - dz * s, pivot[1] + dy * s + dz * c)
}

/// Turns a packed basis vector (bytes B, G, R, A = z, y, x, auxiliary).
fn turn_basis(packed: &mut [u8], angle: f32) {
    let unpack = |b: u8| f32::from(b) / 127.5 - 1.0;
    let pack = |v: f32| ((v + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
    let (y, z) = turn(unpack(packed[1]), unpack(packed[0]), [0.0, 0.0], angle);
    packed[1] = pack(y);
    packed[0] = pack(z);
}

/// Turns one vertex: position, normal, tangent and binormal.
fn turn_vertex(vertex: &mut [u8], pivot: [f32; 2], angle: f32) {
    let (y, z) = turn(f32_at(vertex, 4), f32_at(vertex, 8), pivot, angle);
    vertex[4..8].copy_from_slice(&y.to_le_bytes());
    vertex[8..12].copy_from_slice(&z.to_le_bytes());
    for at in [16, 20, 24] {
        turn_basis(&mut vertex[at..at + 4], angle);
    }
}

fn set_f32(vertex: &mut [u8], at: usize, value: f32) {
    vertex[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Packs a unit vector as the basis bytes (B, G, R = z, y, x), keeping A.
fn pack_basis(packed: &mut [u8], v: [f32; 3]) {
    let pack = |c: f32| ((c + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
    packed[0] = pack(v[2]);
    packed[1] = pack(v[1]);
    packed[2] = pack(v[0]);
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / l)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What [`close_visor`] did.
#[derive(Debug)]
pub struct Closed {
    pub mesh: Vec<u8>,
    pub vertices: usize,
    pub indices: usize,
    pub visor_vertices: usize,
    pub tab_triangles: usize,
    pub pivot: [f32; 2],
    /// The plate's slide (y, z) after the turn.
    pub slide: [f32; 2],
    /// The grille's centre, closed.
    pub grille: [f32; 3],
}

/// The native surface with the visor closed: turned [`CLOSED_DEGREES`] about
/// the tabs' screws, the plate slid [`SLIDE_FORWARD`] forward and down until
/// the grille's centre is at `eye_y`.
pub fn close_visor(native: &[u8], eye_y: f32) -> Result<Closed> {
    ensure!(u32_at(native, 0)? == 3, "no vertex chunk");
    let n = u32_at(native, 8)?;
    ensure!(
        n == NATIVE_VERTICES && u32_at(native, 4)? == 4 + n * STRIDE,
        "not the native helmet's {NATIVE_VERTICES} {STRIDE}-byte vertices"
    );
    let index_at = 12 + n * STRIDE;
    ensure!(u32_at(native, index_at)? == 4, "no index chunk");
    let ni = u32_at(native, index_at + 8)?;
    ensure!(
        ni == NATIVE_INDICES
            && u32_at(native, index_at + 4)? == 4 + ni * 2
            && native.len() == index_at + 12 + ni * 2,
        "not the native helmet's index chunk"
    );
    let vertex = |i: usize| &native[12 + i * STRIDE..12 + (i + 1) * STRIDE];
    let position = |i: usize| {
        let v = vertex(i);
        [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]
    };
    let uv = |i: usize| {
        let v = vertex(i);
        [f32_at(v, 28), f32_at(v, 32)]
    };
    let triangles: Vec<[usize; 3]> = native[index_at + 12..]
        .chunks_exact(6)
        .map(|t| [0, 1, 2].map(|k| usize::from(u16::from_le_bytes([t[k * 2], t[k * 2 + 1]]))))
        .collect();
    ensure!(
        triangles.iter().flatten().all(|&v| v < n),
        "index outside the vertices"
    );

    // UV islands: vertices joined by triangles (not by position).
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for t in &triangles {
        let a = root(&mut parent, t[0]);
        for &v in &t[1..] {
            let b = root(&mut parent, v);
            parent[b] = a;
        }
    }
    let island: Vec<usize> = (0..n).map(|i| root(&mut parent, i)).collect();
    let mut outside: BTreeSet<usize> = BTreeSet::new();
    for i in 0..n {
        let [_, y, z] = position(i);
        if y <= VISOR_MIN_Y || z <= 0.0 {
            outside.insert(island[i]);
        }
    }
    let visor: Vec<bool> = (0..n).map(|i| !outside.contains(&island[i])).collect();
    let visor_vertices = visor.iter().filter(|&&v| v).count();
    let visor_triangles = triangles.iter().filter(|t| visor[t[0]]).count();
    ensure!(
        visor_vertices == VISOR_VERTICES && visor_triangles == VISOR_TRIANGLES,
        "visor islands: {visor_vertices} vertices, {visor_triangles} triangles"
    );

    // The tabs mirror each other; the screws are their pivots.
    for (&r, &l) in RIGHT_TAB.iter().zip(&LEFT_TAB) {
        let (pr, pl) = (position(r.into()), position(l.into()));
        ensure!(
            (pr[0] + pl[0]).abs() < 1e-5
                && (pr[1] - pl[1]).abs() < 1e-5
                && (pr[2] - pl[2]).abs() < 1e-5
                && pr[0] > 0.1,
            "tab vertices {r} and {l} are not mirrored"
        );
    }
    ensure!(
        RIGHT_TAB[13] == SCREWS[0] && LEFT_TAB[13] == SCREWS[1],
        "the screws are not the tabs' 14th vertices"
    );
    let [screw, _] = SCREWS.map(|s| position(s.into()));
    let pivot = [screw[1], screw[2]];
    let tab: BTreeSet<usize> = RIGHT_TAB
        .iter()
        .chain(&LEFT_TAB)
        .map(|&v| usize::from(v))
        .collect();
    ensure!(
        tab.iter().all(|&v| !visor[v]),
        "a tab vertex is in the visor islands"
    );
    let is_tab_triangle = |t: &[usize; 3]| t.iter().all(|v| tab.contains(v));
    let tab_triangles = triangles.iter().filter(|t| is_tab_triangle(t)).count();
    ensure!(
        tab_triangles == TAB_TRIANGLES,
        "{tab_triangles} tab triangles"
    );

    // The grille's centre, open: on the plate, where its texture is.
    let grille_open = triangles
        .iter()
        .filter(|t| visor[t[0]])
        .find_map(|t| {
            let [a, b, c] = t.map(uv);
            let (e1, e2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
            let det = e1[0] * e2[1] - e1[1] * e2[0];
            if det.abs() < 1e-12 {
                return None;
            }
            let q = [GRILLE_UV[0] - a[0], GRILLE_UV[1] - a[1]];
            let s = (q[0] * e2[1] - q[1] * e2[0]) / det;
            let r = (e1[0] * q[1] - e1[1] * q[0]) / det;
            (s >= 0.0 && r >= 0.0 && s + r <= 1.0).then(|| {
                let [pa, pb, pc] = t.map(position);
                [0, 1, 2].map(|k| pa[k] + s * (pb[k] - pa[k]) + r * (pc[k] - pa[k]))
            })
        })
        .context("no visor triangle has the grille's centre")?;
    let angle = CLOSED_DEGREES.to_radians();
    let turned_grille = turn(grille_open[1], grille_open[2], pivot, angle);
    let slide = [eye_y - turned_grille.0, SLIDE_FORWARD];
    let weight = |p: [f32; 3]| {
        let r = ((p[1] - pivot[0]).powi(2) + (p[2] - pivot[1]).powi(2)).sqrt();
        smoothstep((r - SLIDE_NONE) / (SLIDE_FULL - SLIDE_NONE))
    };
    let grille = [
        grille_open[0],
        turned_grille.0 + weight(grille_open) * slide[0],
        turned_grille.1 + weight(grille_open) * slide[1],
    ];

    let mut vertices: Vec<Vec<u8>> = (0..n).map(|i| vertex(i).to_vec()).collect();
    for (i, v) in vertices.iter_mut().enumerate() {
        if visor[i] {
            let w = weight(position(i));
            turn_vertex(v, pivot, angle);
            let (y, z) = (f32_at(v, 4), f32_at(v, 8));
            set_f32(v, 4, y + w * slide[0]);
            set_f32(v, 8, z + w * slide[1]);
        }
    }
    // The tabs' old place: their triangles on copies of their vertices,
    // flat inside the outline and in the disc's dark metal.
    let mut fill = BTreeMap::new();
    for side in [&RIGHT_TAB, &LEFT_TAB] {
        let screw_uv = uv(side[13].into());
        for (k, &v) in side.iter().enumerate() {
            let v = usize::from(v);
            let mut bytes = vertex(v).to_vec();
            if k >= TAB_OUTLINE {
                let x = f32_at(&bytes, 0);
                set_f32(&mut bytes, 0, FILL_X * x.signum());
            }
            let own = uv(v);
            for c in 0..2 {
                set_f32(
                    &mut bytes,
                    28 + c * 4,
                    screw_uv[c] + FILL_DARK[c] + FILL_SCALE * (own[c] - screw_uv[c]),
                );
            }
            fill.insert(v, vertices.len());
            vertices.push(bytes);
        }
    }
    // The tabs, turned with the visor on the screws.
    let mut copy = BTreeMap::new();
    for &v in &tab {
        let mut bytes = vertex(v).to_vec();
        let x = f32_at(&bytes, 0);
        set_f32(&mut bytes, 0, x + TAB_OFFSET * x.signum());
        turn_vertex(&mut bytes, pivot, angle);
        copy.insert(v, vertices.len());
        vertices.push(bytes);
    }
    // The flap, a strip of quads, wound as the native triangles (counter-
    // clockwise about the normal), normal up and forward.
    let template = vertex(0).to_vec();
    let flap_start = vertices.len();
    let row = |x: f32, rows: &[[f32; 2]; 3]| {
        let a = x.abs();
        let r = if a > 0.095 {
            2
        } else if a > 0.075 {
            1
        } else {
            0
        };
        [x, rows[r][0], rows[r][1]]
    };
    let columns = FLAP_X.len();
    for (front, v_at) in [(true, FLAP_UV[3]), (false, FLAP_UV[1])] {
        for (i, &x) in FLAP_X.iter().enumerate() {
            let p = row(x, if front { &FLAP_FRONT } else { &FLAP_BACK });
            let (f, b) = (row(x, &FLAP_FRONT), row(x, &FLAP_BACK));
            let along = unit([0.0, f[1] - b[1], f[2] - b[2]]);
            let tangent = [1.0, 0.0, 0.0];
            let normal = unit(cross(along, tangent));
            let mut bytes = template.clone();
            for k in 0..3 {
                set_f32(&mut bytes, k * 4, p[k]);
            }
            pack_basis(&mut bytes[16..20], normal);
            pack_basis(&mut bytes[20..24], tangent);
            pack_basis(&mut bytes[24..28], along);
            let u = FLAP_UV[0] + (FLAP_UV[2] - FLAP_UV[0]) * i as f32 / (columns - 1) as f32;
            set_f32(&mut bytes, 28, u);
            set_f32(&mut bytes, 32, v_at);
            vertices.push(bytes);
        }
    }
    let mut flap = Vec::new();
    for i in 0..columns - 1 {
        let (f0, f1) = (flap_start + i, flap_start + i + 1);
        let (b0, b1) = (f0 + columns, f1 + columns);
        flap.push([f0, f1, b0]);
        flap.push([f1, b1, b0]);
    }

    let all: Vec<[usize; 3]> = triangles
        .iter()
        .map(|t| {
            if is_tab_triangle(t) {
                t.map(|v| fill[&v])
            } else {
                *t
            }
        })
        .chain(
            triangles
                .iter()
                .filter(|t| is_tab_triangle(t))
                .map(|t| t.map(|v| copy[&v])),
        )
        .chain(flap)
        .collect();
    let count = vertices.len();
    ensure!(count <= usize::from(u16::MAX), "too many vertices");
    let mut mesh = Vec::with_capacity(24 + count * STRIDE + all.len() * 6);
    mesh.extend_from_slice(&3u32.to_le_bytes());
    mesh.extend_from_slice(&u32::try_from(4 + count * STRIDE)?.to_le_bytes());
    mesh.extend_from_slice(&u32::try_from(count)?.to_le_bytes());
    for v in &vertices {
        mesh.extend_from_slice(v);
    }
    mesh.extend_from_slice(&4u32.to_le_bytes());
    mesh.extend_from_slice(&u32::try_from(4 + all.len() * 6)?.to_le_bytes());
    mesh.extend_from_slice(&u32::try_from(all.len() * 3)?.to_le_bytes());
    for t in &all {
        for &v in t {
            mesh.extend_from_slice(&u16::try_from(v)?.to_le_bytes());
        }
    }
    Ok(Closed {
        mesh,
        vertices: count,
        indices: all.len() * 3,
        visor_vertices,
        tab_triangles,
        pivot,
        slide,
        grille,
    })
}

/// The height of the native player head's eyes (the two eye bones).
fn eye_height(bind_pose: &[u8]) -> Result<f32> {
    let bones = crate::armor::bind_pose_origins(bind_pose, "native head bind pose")?;
    let eye = |name: &str| {
        bones
            .iter()
            .find(|(bone, _)| bone == name)
            .map(|(_, at)| at[1])
            .with_context(|| format!("no {name} bone"))
    };
    Ok((eye("LeftEye")? + eye("RightEye")?) / 2.0)
}

/// The closed helmet's item config: the native one with its own name, icon
/// and model, and, as the native helmets that close over the face
/// (`edge_helmet_6`, `blackmarket_helmet_5`, `renesanse_helmet_4/6`), the
/// mask slot taken and the face protected as the head.
fn item_config(native: &[u8]) -> Result<BinaryConfig> {
    let mut config = BinaryConfig::parse(native)?;
    let root = &mut config.root;
    root.set_path("ui_desc.text_descriptions.name", V::String(NAME_KEY.into()))?;
    root.set_path(
        "ui_desc.icon",
        V::Indexed(vec![V::Integer(ICON.0 as i32), V::Integer(ICON.1 as i32)]),
    )?;
    root.set_path(
        "data.base_model",
        V::String(
            MODEL
                .trim_start_matches("models/")
                .trim_end_matches(".skinned_model")
                .into(),
        ),
    )?;
    ensure!(
        root.named("data").and_then(|d| d.named("part_name")) == Some(&V::String(SURFACE.into())),
        "the native item names another surface"
    );
    ensure!(
        root.named("data")
            .and_then(|d| d.named("additional_model_slots"))
            .is_none(),
        "the native item already covers a slot"
    );
    root.set_path(
        "data.additional_model_slots",
        V::Indexed(vec![V::String("mask".into())]),
    )?;
    let Some(V::Named(regions)) = root.named_mut("hit_params") else {
        bail!("the native item has no hit_params");
    };
    ensure!(
        regions.len() == 1 && regions[0].name == "brain",
        "the native item protects more than the head"
    );
    let face = NamedValue {
        name: "face".into(),
        value: regions[0].value.clone(),
    };
    regions.insert(0, face);
    Ok(config)
}

fn add_file(files: &Path, path: &str, bytes: &[u8], operations: &mut Vec<Operation>) -> Result<()> {
    let target = crate::archive::safe_join(&files.join("files"), path)?;
    fs::create_dir_all(target.parent().context("no parent")?)?;
    fs::write(&target, bytes)?;
    operations.push(Operation::AddFile {
        path: path.into(),
        source: None,
    });
    Ok(())
}

/// Writes the pack to `output` (`pack.json`, `files/`). `diffuse` is the
/// helmet's own diffuse texture (an image ImageMagick reads), made by
/// `scripts/keeper-closed-026-texture.py`; the icon, `icons/r14c06.png`, is
/// rendered from the pack's model by `scripts/render-keeper-closed-026-icon.py`.
pub fn build(
    base_db: &Path,
    base_patches: &[PathBuf],
    diffuse: &Path,
    output: &Path,
) -> Result<()> {
    ensure!(
        !output.join("pack.json").exists(),
        "{} already holds a pack",
        output.display()
    );
    let mut db = ResourceDb::open_layers(&base_layers(base_db, base_patches)?)?;
    ensure!(
        !db.contains(ITEM) && !db.contains(&format!("{MODEL}/settings")),
        "the base already has the closed helmet"
    );
    let native_item = db.read(NATIVE_ITEM)?;
    ensure!(
        sha256_hex(&native_item) == NATIVE_ITEM_SHA256,
        "{NATIVE_ITEM} is not the stock config"
    );
    let mut operations = Vec::new();
    let eye_y = eye_height(&db.read(HEAD_BIND_POSE)?)?;

    // The texture and the material that names it.
    let work = output.join("work");
    fs::create_dir_all(&work)?;
    let dds = work.join("diffuse.dds");
    crate::armor::convert_texture(diffuse, &dds, crate::manifest::TextureCompression::Dxt1)?;
    let texture = crate::duty_026::texture_dds(&fs::read(&dds)?, TEXTURE, true)?;
    fs::remove_dir_all(&work)?;
    add_file(output, &format!("{TEXTURE}.dds"), &texture, &mut operations)?;
    add_file(
        output,
        &format!("{TEXTURE}.options"),
        &db.read(&format!("{NATIVE_TEXTURE}.options"))?,
        &mut operations,
    )?;
    let mut material = BinaryConfig::parse(&db.read(NATIVE_MATERIAL)?)?;
    let diffuse_key = "material.g_stage.effect.texture_diffuse.value";
    ensure!(
        value::value_at(&material.root, diffuse_key)
            == Some(&V::String(
                NATIVE_TEXTURE.trim_start_matches("textures/").into()
            )),
        "{NATIVE_MATERIAL} draws another diffuse texture"
    );
    material.root.set_path(
        diffuse_key,
        V::String(TEXTURE.trim_start_matches("textures/").into()),
    )?;
    add_file(output, MATERIAL, &material.to_bytes()?, &mut operations)?;

    // The model: the native files, the closed mesh and its counts, and the
    // settings naming the helmet's own material.
    for file in MODEL_FILES {
        let bytes = db.read(&format!("{NATIVE_MODEL}/{file}"))?;
        let path = format!("{MODEL}/{file}");
        let bytes = match file {
            "render/edge_assault_05_helmet_s/converted_model" => {
                ensure!(
                    sha256_hex(&bytes) == NATIVE_SURFACE_SHA256,
                    "{NATIVE_MODEL}: not the stock surface"
                );
                let closed = close_visor(&bytes, eye_y)?;
                println!(
                    "visor: {} vertices turned {CLOSED_DEGREES} degrees about y {:.4}, z {:.4}, the plate slid y {:+.4}, z {:+.4}: grille centre at y {:.4} (eyes {eye_y:.4}), z {:.4}; {} tab triangles copied and filled; {} vertices, {} indices",
                    closed.visor_vertices,
                    closed.pivot[0],
                    closed.pivot[1],
                    closed.slide[0],
                    closed.slide[1],
                    closed.grille[1],
                    closed.grille[2],
                    closed.tab_triangles,
                    closed.vertices,
                    closed.indices
                );
                let properties = format!("{MODEL}/render/{SURFACE}/export_properties");
                let mut config = BinaryConfig::parse(&db.read(&format!(
                    "{NATIVE_MODEL}/render/{SURFACE}/export_properties"
                ))?)?;
                config
                    .root
                    .set_path("source.vertices", V::Integer(closed.vertices as i32))?;
                config
                    .root
                    .set_path("source.indices", V::Integer(closed.indices as i32))?;
                config
                    .root
                    .set_path("source.primitives", V::Integer((closed.indices / 3) as i32))?;
                add_file(output, &properties, &config.to_bytes()?, &mut operations)?;
                closed.mesh
            }
            "render/edge_assault_05_helmet_s/export_properties" => continue,
            "settings" => {
                let mut settings = BinaryConfig::parse(&bytes)?;
                let key = format!("material_settings.{SURFACE}.material_name");
                let name = |path: &str| {
                    path.trim_start_matches("material_instances/")
                        .trim_end_matches(".material")
                        .to_owned()
                };
                ensure!(
                    value::value_at(&settings.root, &key)
                        == Some(&V::String(name(NATIVE_MATERIAL))),
                    "{NATIVE_MODEL}: the surface has another material"
                );
                settings.root.set_path(&key, V::String(name(MATERIAL)))?;
                settings.to_bytes()?
            }
            _ => bytes,
        };
        add_file(output, &path, &bytes, &mut operations)?;
    }
    operations.sort_by_key(|op| match op {
        Operation::AddFile { path, .. } => path.clone(),
        _ => String::new(),
    });

    add_file(
        output,
        ITEM,
        &item_config(&native_item)?.to_bytes()?,
        &mut operations,
    )?;

    // The dictionary row: the native one's, with the new id and config.
    let dictionary = BinaryConfig::parse(&db.read("gameplay/db_static_dictionaries")?)?;
    let Some(V::Indexed(items)) = dictionary.root.named("items_dict") else {
        bail!("no items_dict");
    };
    ensure!(
        !items
            .iter()
            .any(|row| row.named("dict_id") == Some(&V::Integer(DICT_ID))),
        "dictionary id {DICT_ID} is taken in the base"
    );
    let native_row = items
        .iter()
        .find(|row| row.named("cfg_name") == Some(&V::String(NATIVE_ITEM.into())))
        .context("no dictionary row for the native helmet")?;
    let mut row: Map<String, Json> = value::table_from_config(native_row)?;
    row.insert("cfg_name".into(), json!(ITEM));
    row.insert("name".into(), json!("edge_helmet_5_closed"));
    row.insert("dict_id".into(), json!(DICT_ID));
    operations.push(Operation::DictionaryAdd {
        table: "items_dict".into(),
        id_field: None,
        entries: vec![row],
    });

    // The name, in each language beside the native one's.
    for group in LANGUAGE_GROUPS {
        for language in group {
            let path = format!("localization/{language}/localization");
            let localization = BinaryConfig::parse(&db.read(&path)?)?;
            let native = localization
                .root
                .named("strings")
                .and_then(|s| s.named(NATIVE_NAME_KEY));
            ensure!(
                native == Some(&V::String(game_string(NATIVE_NAME.get(language)?))),
                "{path}: {NATIVE_NAME_KEY} is not the native name ({native:?})"
            );
        }
        operations.push(Operation::LocalizationAdd {
            strings: BTreeMap::from([(NAME_KEY.into(), game_string(NAME.get(group[0])?))]),
            languages: Some(group.iter().map(|l| l.to_string()).collect()),
        });
    }

    operations.push(Operation::AtlasIconsAdd {
        swf: None,
        symbol: None,
        icons: vec![AtlasIcon {
            row: ICON.0,
            col: ICON.1,
            source: format!("icons/r{:02}c{:02}.png", ICON.0, ICON.1),
            premultiplied: false,
        }],
    });

    let manifest = PackManifest {
        format_version: 1,
        id: PACK_ID.into(),
        version: PACK_VERSION.into(),
        target_build: "0.26g".into(),
        description: Some(
            "The Fringe Settlers' \"Keeper\" helmet with its visor closed, beside the native one"
                .into(),
        ),
        requires: vec![],
        base_db_sha256: None,
        operations,
        server: Some(json!({"items": [{
            "dict_id": DICT_ID,
            "config": ITEM,
            "name": NAME.english,
            "template": NATIVE_ITEM,
        }]})),
        notes: vec![
            format!("{MODEL}: {NATIVE_MODEL} with the visor turned down {CLOSED_DEGREES} degrees about its hinge screws and the plate slid until its grille is on the eyes; the tabs' old place filled, a flap to the brow"),
            format!("{TEXTURE}: the native diffuse without the darkness the raised visor baked into what it covered"),
            format!("{ITEM}: {NATIVE_ITEM} with its own name, icon and model; the mask slot taken and the face protected as the head"),
            "price, seller and drop weight: the native helmet's (item level 14, the Fringe Settlers at rank 9)".into(),
        ],
    };
    fs::write(
        output.join("pack.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    println!("wrote {}", output.join("pack.json").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_a_vertex_turns_its_position_and_basis() {
        let mut v = [0u8; STRIDE];
        v[4..8].copy_from_slice(&1.8f32.to_le_bytes());
        v[8..12].copy_from_slice(&0.1f32.to_le_bytes());
        // Normal +z, tangent +y, binormal +x (B, G, R, A).
        v[16..20].copy_from_slice(&[255, 128, 128, 127]);
        v[20..24].copy_from_slice(&[128, 255, 128, 3]);
        v[24..28].copy_from_slice(&[128, 128, 255, 9]);
        let pivot = [1.7, 0.0];
        let mut turned = v;
        turn_vertex(&mut turned, pivot, 90f32.to_radians());
        // +y over the pivot goes to +z.
        assert!((f32_at(&turned, 4) - 1.6).abs() < 1e-5);
        assert!((f32_at(&turned, 8) - 0.1).abs() < 1e-5);
        // A zero component is 127 or 128 (0 lies between two bytes).
        let near =
            |got: &[u8], want: [u8; 4]| got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 1);
        // The normal +z turns to -y, the tangent +y to +z, the binormal stays.
        assert!(
            near(&turned[16..20], [128, 0, 128, 127]),
            "{:?}",
            &turned[16..20]
        );
        assert!(
            near(&turned[20..24], [255, 128, 128, 3]),
            "{:?}",
            &turned[20..24]
        );
        assert!(
            near(&turned[24..28], [128, 128, 255, 9]),
            "{:?}",
            &turned[24..28]
        );
        turn_vertex(&mut turned, pivot, (-90f32).to_radians());
        assert!((f32_at(&turned, 4) - 1.8).abs() < 1e-5);
        assert!((f32_at(&turned, 8) - 0.1).abs() < 1e-5);
    }

    #[test]
    fn closed_names_extend_the_native_ones_in_every_language() {
        for language in [
            "english",
            "original",
            "russian",
            "ukrainian",
            "german",
            "polish",
        ] {
            let native = NATIVE_NAME.get(language).unwrap();
            let closed = NAME.get(language).unwrap();
            assert!(
                closed.starts_with(native) && closed.ends_with(')'),
                "{language}"
            );
            // The files hold UTF-8; every byte reads back from Windows-1251.
            let game = game_string(closed);
            let (bytes, _, bad) = WINDOWS_1251.encode(&game);
            assert!(!bad);
            assert_eq!(&*bytes, closed.as_bytes());
        }
    }
}
