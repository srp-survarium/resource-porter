// SPDX-License-Identifier: GPL-3.0-or-later
//! Decals on ported 0.26g armour, as the native pieces take them.
//!
//! What 0.26g does (its stock `resources.db` and exe; skhid-srp's
//! `docs/notes/client_patch.md` 3.44 and `items.md`):
//!
//! - Every stock vest (24 of 24) and pair of trousers (23 of 23) has
//!   `parameters.decal_available` true and an empty `data.decals`; no
//!   helmet, mask, backpack, glove or boot config has either key.
//! - `decal_available` gives the item its two decal slots in the lobby (and
//!   on the server). A decal's modification appends its texture and place
//!   to `data.decals` of the item wearing it; a config without that table
//!   drops it, and the game bakes whatever the table holds.
//! - There are no fixed places: the player clicks where the decal goes (a
//!   box projected along the surface's normal, its size and turn the
//!   player's too). One never placed is on the chest.
//! - The bake paints the textures of the piece's first surface
//!   (`part_name`), in that surface's own UV space. The surfaces of
//!   `additional_models` (pouches, fur, a frame) take none.
//!
//! - The game keeps each baked texture in a file named after the decals
//!   (their textures and places) and the model's file name, without its
//!   folder: `4_4205_7-8172320283889761536torso_s3d.dds` is the diffuse of
//!   a model `.../torso_s3`. The files are kept from game to game. Stock
//!   models all have file names of their own; two ported models of one file
//!   name in two folders share their bakes, and the second one seen wears
//!   the first one's textures (seen in a game, 2026-11-20: a jacket
//!   `stk2_novice_026/torso` in the leather of `stk2_bandit_026/torso`).
//!
//! So a ported piece takes decals by its slot ([`takes_decals`], [`set`]),
//! on its first surface, and that surface has to be one a bake can paint
//! ([`check_surface`]): the garment itself, inside the map, each texel
//! painted from one place. Its models have file names no other model has
//! ([`check_model_names`]).
//!
//! Camouflage is another matter: its bake needs a mask beside the diffuse
//! map (`<diffuse>_aoac`), which no port here makes. It stays off.

use crate::mask_fit::{parse_mesh, read_u32};
use crate::{ConfigValue as V, ResourceDb};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

/// Whether the native pieces of an armour category
/// (`gameplay/items/armour/<category>/`) take decals.
pub fn takes_decals(category: &str) -> bool {
    matches!(category, "torso" | "legs")
}

/// Gives an item config of `category` the decal keys its native peers
/// have: the flag and the empty table for a vest or trousers, neither
/// for anything else.
pub fn set(config: &mut V, category: &str) -> Result<()> {
    let takes = takes_decals(category);
    config.set_path("parameters.decal_available", V::Boolean(takes))?;
    if takes {
        ensure!(
            matches!(config.named("data"), Some(V::Named(_))),
            "a {category} config without a data table cannot hold decals"
        );
        config.set_path("data.decals", V::Named(vec![]))?;
    } else if let Some(V::Named(fields)) = config.named_mut("data") {
        fields.retain(|field| field.name != "decals");
    }
    Ok(())
}

/// A model's file name: the last part of its path (`ported/x/torso_s2` is
/// `torso_s2`), which is all of the model the game puts into the names of
/// its baked textures.
pub fn model_file_name(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

/// Every skinned model of a game (`models/<path>.skinned_model/`): its
/// file name and the paths that have it.
pub fn game_model_names(game: &ResourceDb) -> BTreeMap<String, BTreeSet<String>> {
    let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in game.paths_with_prefix("models/") {
        if let Some((model, _)) = file["models/".len()..].split_once(".skinned_model/") {
            names
                .entry(model_file_name(model).to_owned())
                .or_default()
                .insert(model.to_owned());
        }
    }
    names
}

/// Checks that each of `models` (the models of pieces that take decals:
/// their game paths, without `models/` and `.skinned_model`) has a file
/// name of its own: none of the others has it, and no other model of the
/// game ([`game_model_names`]).
pub fn check_model_names(
    models: &[String],
    game: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for model in models {
        let name = model_file_name(model);
        if let Some(other) = seen.insert(name, model)
            && other != model
        {
            anyhow::bail!(
                "{model} and {other} take decals and share the file name {name}: the game would give both the same baked textures"
            );
        }
        if let Some(other) = game
            .get(name)
            .and_then(|paths| paths.iter().find(|path| *path != model))
        {
            anyhow::bail!(
                "{model} takes decals and the game's {other} has its file name {name}: the game would give both the same baked textures"
            );
        }
    }
    Ok(())
}

/// The share of a decal surface's used texels that two places of it may
/// both paint. A decal put on such a texel shows at both places. Most
/// stock vests and trousers share none; six share 10 to 30 % (mirrored
/// halves). A port whose map is cut for it stays near none.
pub const MAX_SHARED_TEXELS: f64 = 0.05;
/// Two places of a surface are apart when the points painting one texel
/// are farther than this, in metres.
const APART: f64 = 0.03;
/// The grid the surface's map is sampled on.
const GRID: usize = 512;

/// What a decal bake finds on a piece's first surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecalSurface {
    /// Square metres of the surface.
    pub area: f64,
    /// The share of its used texels painted from two places apart.
    pub shared_texels: f64,
}

/// Checks that `surfaces` (a piece's converted models, `part_name`'s
/// first) are fit for decals: the first is the garment (no other surface
/// is larger), its UVs are inside the map, and at most
/// [`MAX_SHARED_TEXELS`] of its texels are painted from two places.
pub fn check_surface(label: &str, surfaces: &[&[u8]]) -> Result<DecalSurface> {
    let (first, rest) = surfaces
        .split_first()
        .with_context(|| format!("{label}: no surface"))?;
    let surface = Surface::read(first, label)?;
    let area = surface.area();
    for (i, other) in rest.iter().enumerate() {
        let other = Surface::read(other, label)?.area();
        ensure!(
            other <= area,
            "{label}: decals go on the first surface ({area:.2} m2), but surface {} is larger ({other:.2} m2)",
            i + 1
        );
    }
    for uv in &surface.uvs {
        ensure!(
            uv.iter().all(|c| (-1e-3..=1.0 + 1e-3).contains(c)),
            "{label}: decal surface UV {uv:?} is outside its map"
        );
    }
    let shared_texels = surface.shared_texels();
    ensure!(
        shared_texels <= MAX_SHARED_TEXELS,
        "{label}: {:.1} % of the decal surface's texels are painted from two places (at most {:.0} %)",
        shared_texels * 100.0,
        MAX_SHARED_TEXELS * 100.0
    );
    Ok(DecalSurface {
        area,
        shared_texels,
    })
}

struct Surface {
    positions: Vec<[f64; 3]>,
    uvs: Vec<[f64; 2]>,
    triangles: Vec<[usize; 3]>,
}

impl Surface {
    /// A converted model of any skinned vertex layout: the UV pair is the
    /// vertex's last eight bytes.
    fn read(model: &[u8], label: &str) -> Result<Self> {
        let count = read_u32(model, 8)?;
        ensure!(count > 0, "{label}: a surface without vertices");
        let stride = (read_u32(model, 4)? - 4) / count;
        let mesh = parse_mesh(model, stride, label)?;
        let uvs = (0..count)
            .map(|i| {
                let at = 12 + i * stride + stride - 8;
                let c = |o: usize| -> Result<f64> {
                    let bytes = model.get(at + o..at + o + 4).context("truncated vertex")?;
                    Ok(f64::from(f32::from_le_bytes(bytes.try_into()?)))
                };
                Ok([c(0)?, c(4)?])
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            positions: mesh.positions,
            uvs,
            triangles: mesh.triangles,
        })
    }

    fn area(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| self.positions[v]);
                let (u, v) = (sub3(b, a), sub3(c, a));
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                0.5 * cross.iter().map(|x| x * x).sum::<f64>().sqrt()
            })
            .sum()
    }

    /// Each texel of the map keeps the box of the points that paint it;
    /// a box wider than [`APART`] is a texel two places share.
    fn shared_texels(&self) -> f64 {
        let mut boxes: Vec<Option<([f64; 3], [f64; 3])>> = vec![None; GRID * GRID];
        let grid = GRID as f64;
        for t in &self.triangles {
            let p = t.map(|v| self.positions[v]);
            let uv = t.map(|v| [self.uvs[v][0] * grid, self.uvs[v][1] * grid]);
            let det = (uv[1][1] - uv[2][1]) * (uv[0][0] - uv[2][0])
                + (uv[2][0] - uv[1][0]) * (uv[0][1] - uv[2][1]);
            if det.abs() < 1e-12 {
                continue;
            }
            let range = |axis: usize| {
                let lo = uv.iter().map(|c| c[axis]).fold(f64::INFINITY, f64::min);
                let hi = uv.iter().map(|c| c[axis]).fold(f64::NEG_INFINITY, f64::max);
                (lo.floor().max(0.0) as usize)..(hi.ceil().min(grid) as usize)
            };
            for y in range(1) {
                for x in range(0) {
                    let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                    let a = ((uv[1][1] - uv[2][1]) * (cx - uv[2][0])
                        + (uv[2][0] - uv[1][0]) * (cy - uv[2][1]))
                        / det;
                    let b = ((uv[2][1] - uv[0][1]) * (cx - uv[2][0])
                        + (uv[0][0] - uv[2][0]) * (cy - uv[2][1]))
                        / det;
                    let c = 1.0 - a - b;
                    if a < 0.0 || b < 0.0 || c < 0.0 {
                        continue;
                    }
                    let point = [0, 1, 2].map(|k| a * p[0][k] + b * p[1][k] + c * p[2][k]);
                    let texel = &mut boxes[y * GRID + x];
                    *texel = Some(match *texel {
                        None => (point, point),
                        Some((lo, hi)) => (
                            [0, 1, 2].map(|k| lo[k].min(point[k])),
                            [0, 1, 2].map(|k| hi[k].max(point[k])),
                        ),
                    });
                }
            }
        }
        let used = boxes.iter().flatten().count();
        let shared = boxes
            .iter()
            .flatten()
            .filter(|(lo, hi)| sub3(*hi, *lo).iter().map(|x| x * x).sum::<f64>().sqrt() > APART)
            .count();
        if used == 0 {
            0.0
        } else {
            shared as f64 / used as f64
        }
    }
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NamedValue;

    fn field(name: &str, value: V) -> NamedValue {
        NamedValue {
            name: name.into(),
            value,
        }
    }

    /// An item config as a port leaves it before the decal keys are set.
    fn config(decals: bool) -> V {
        let mut data = vec![field("part_name", V::String("torso".into()))];
        if decals {
            data.push(field("decals", V::Named(vec![])));
        }
        V::Named(vec![
            field(
                "parameters",
                V::Named(vec![field("decal_available", V::Boolean(!decals))]),
            ),
            field("data", V::Named(data)),
        ])
    }

    fn keys(config: &V) -> (Option<&V>, Option<&V>) {
        (
            config
                .named("parameters")
                .and_then(|p| p.named("decal_available")),
            config.named("data").and_then(|d| d.named("decals")),
        )
    }

    #[test]
    fn vests_and_trousers_take_decals_as_the_native_ones() {
        for category in ["torso", "legs"] {
            for had_table in [false, true] {
                let mut c = config(had_table);
                set(&mut c, category).unwrap();
                assert_eq!(
                    keys(&c),
                    (Some(&V::Boolean(true)), Some(&V::Named(vec![]))),
                    "{category}"
                );
            }
        }
    }

    #[test]
    fn no_other_slot_takes_decals() {
        for category in ["helmet", "mask", "back", "gloves", "boots"] {
            for had_table in [false, true] {
                let mut c = config(had_table);
                set(&mut c, category).unwrap();
                assert_eq!(keys(&c), (Some(&V::Boolean(false)), None), "{category}");
            }
        }
    }

    #[test]
    fn a_vest_without_a_data_table_is_refused() {
        let mut c = V::Named(vec![field("parameters", V::Named(vec![]))]);
        assert!(set(&mut c, "torso").is_err());
    }

    #[test]
    fn decal_models_need_file_names_of_their_own() {
        let game: BTreeMap<String, BTreeSet<String>> = [(
            "duty_03_torso_s1".to_owned(),
            BTreeSet::from(["ported/duty_026/duty_03_torso_s1".to_owned()]),
        )]
        .into();
        let models = |paths: &[&str]| paths.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        // Named after their outfits, and the game's own model built again.
        check_model_names(
            &models(&[
                "ported/stk2_bandit_026/stk2_bandit_026_torso",
                "ported/stk2_novice_026/stk2_novice_026_torso",
                "ported/duty_026/duty_03_torso_s1",
            ]),
            &game,
        )
        .unwrap();
        // Two outfits' jackets both called `torso`.
        let error = check_model_names(
            &models(&[
                "ported/stk2_bandit_026/torso",
                "ported/stk2_novice_026/torso",
            ]),
            &game,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("share the file name torso"), "{error}");
        // A repaint's model under the name of the suit it repaints.
        let error = check_model_names(&models(&["ported/freedom_026/duty_03_torso_s1"]), &game)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("ported/duty_026/duty_03_torso_s1"),
            "{error}"
        );
    }

    #[test]
    fn a_models_file_name_is_its_paths_last_part() {
        assert_eq!(
            model_file_name("ported/stk2_novice_026/torso_v2_s3"),
            "torso_v2_s3"
        );
        assert_eq!(model_file_name("torso"), "torso");
    }

    /// A converted model of 48-byte vertices: `quads` of (corner, two
    /// edges, the UV corner and size), two triangles each.
    fn model(quads: &[([f32; 3], [f32; 3], [f32; 3], [f32; 2], [f32; 2])]) -> Vec<u8> {
        let mut vertices = vec![];
        let mut indices: Vec<u16> = vec![];
        for (at, u, v, uv, size) in quads {
            let base = (vertices.len() / 48) as u16;
            for (s, t) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                let mut vertex = [0_u8; 48];
                for k in 0..3 {
                    let p: f32 = at[k] + s * u[k] + t * v[k];
                    vertex[k * 4..k * 4 + 4].copy_from_slice(&p.to_le_bytes());
                }
                vertex[40..44].copy_from_slice(&(uv[0] + s * size[0]).to_le_bytes());
                vertex[44..48].copy_from_slice(&(uv[1] + t * size[1]).to_le_bytes());
                vertices.extend(vertex);
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let count = (vertices.len() / 48) as u32;
        let mut out = vec![];
        for word in [3, 4 + vertices.len() as u32, count] {
            out.extend(word.to_le_bytes());
        }
        out.extend(vertices);
        for word in [4, 4 + indices.len() as u32 * 2, indices.len() as u32] {
            out.extend(word.to_le_bytes());
        }
        out.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
        out
    }

    const X: [f32; 3] = [0.5, 0.0, 0.0];
    const Y: [f32; 3] = [0.0, 0.5, 0.0];

    #[test]
    fn a_surface_with_its_own_texels_is_fit() {
        // A front and a back panel, each on its own half of the map.
        let garment = model(&[
            ([0.0, 1.0, 0.1], X, Y, [0.0, 0.0], [0.5, 1.0]),
            ([0.0, 1.0, -0.1], X, Y, [0.5, 0.0], [0.5, 1.0]),
        ]);
        let pouch = model(&[(
            [0.0, 1.0, 0.15],
            [0.1, 0.0, 0.0],
            [0.0, 0.1, 0.0],
            [0.0, 0.0],
            [1.0, 1.0],
        )]);
        let found = check_surface("vest", &[&garment, &pouch]).unwrap();
        assert!((found.area - 0.5).abs() < 1e-6, "{found:?}");
        assert_eq!(found.shared_texels, 0.0);
    }

    #[test]
    fn mirrored_halves_on_one_part_of_the_map_are_refused() {
        // Both sleeves drawn from the same texels: a decal on one shows on both.
        let garment = model(&[
            ([0.3, 1.0, 0.0], X, Y, [0.0, 0.0], [1.0, 1.0]),
            ([-0.8, 1.0, 0.0], X, Y, [0.0, 0.0], [1.0, 1.0]),
        ]);
        let error = check_surface("jacket", &[&garment])
            .unwrap_err()
            .to_string();
        assert!(error.contains("painted from two places"), "{error}");
    }

    #[test]
    fn an_attachment_first_is_refused() {
        let garment = model(&[([0.0, 1.0, 0.1], X, Y, [0.0, 0.0], [1.0, 1.0])]);
        let pouch = model(&[(
            [0.0, 1.0, 0.15],
            [0.1, 0.0, 0.0],
            [0.0, 0.1, 0.0],
            [0.0, 0.0],
            [1.0, 1.0],
        )]);
        let error = check_surface("vest", &[&pouch, &garment])
            .unwrap_err()
            .to_string();
        assert!(error.contains("is larger"), "{error}");
    }

    #[test]
    fn uvs_outside_the_map_are_refused() {
        let garment = model(&[([0.0, 1.0, 0.1], X, Y, [0.5, 0.0], [1.0, 1.0])]);
        let error = check_surface("vest", &[&garment]).unwrap_err().to_string();
        assert!(error.contains("outside its map"), "{error}");
    }
}
