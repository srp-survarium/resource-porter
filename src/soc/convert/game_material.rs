// SPDX-License-Identifier: GPL-3.0-or-later
//! Translate X-Ray physical materials into v0.100b ones.
//!
//! Numeric physical-material identifiers are build-local in both engines, so
//! the port is a *name* translation with a target lookup at the end:
//!
//! ```text
//! cform face material id
//!   -> gamemtl.xr name          ("materials\concrete")
//!   -> reviewed target name     ("concrete")
//!   -> v0.100b dictionary id    (resolved from game_materials/game.materials)
//! ```
//!
//! Skipping the middle step and reusing the source identifier would bind
//! unrelated behaviour: SoC id 3 is concrete, and v0.100b id 3 is something
//! else entirely.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::soc::codec::gamemtl::GameMaterial;
use serde::{Deserialize, Serialize};

/// The face-dictionary entry name the collision family uses for source
/// material `id`. Keeping the source identifier in the name makes an incorrect
/// binding visible in the emitted resource rather than only in the manifest.
pub fn face_dictionary_name(id: u16) -> String {
    format!("soc_mtl_{id:04}")
}

/// The reviewed default correspondence between Shadow of Chernobyl physical
/// materials and v0.100b ones.
///
/// Choices worth stating explicitly, because they are lossy:
///
/// * SoC separates `wood`, `wooden_board` and `tree_trunk`; v0.100b's closest
///   single analogue is `wood_silent`.
/// * SoC separates `metal`, `metal_plate`, `metal_pipe` and `tin` by thickness
///   and resonance; all four collapse onto `metall_thick`.
/// * `stucco` behaves as `concrete`, and `shifer` (asbestos roofing sheet) as
///   `stone`, because neither has a target entry.
/// * `fake` is X-Ray's invisible blocking volume. It keeps `default` so that it
///   still blocks movement; it is *not* dropped.
pub fn default_correspondence() -> BTreeMap<String, String> {
    [
        ("default", "default"),
        ("materials\\asphalt", "asphalt"),
        ("materials\\bricks", "brick"),
        ("materials\\bush", "foliage"),
        ("materials\\cloth", "cloth"),
        ("materials\\concrete", "concrete"),
        // X-Ray's instant-death surfaces. v0.100b has no injurious physical
        // material, so the damage is lost and only the footing survives. A
        // converted map's radioactive pools and crushers become ordinary
        // ground.
        ("materials\\death", "default"),
        ("materials\\earth_death", "ground"),
        ("materials\\dirt", "mud"),
        ("materials\\earth", "ground"),
        ("materials\\fake", "default"),
        ("materials\\grass", "grass"),
        ("materials\\sand", "ground"),
        ("materials\\water", "water"),
        ("materials\\flooring_tile", "stone"),
        ("materials\\glass", "glass"),
        ("materials\\gravel", "gravel"),
        ("materials\\metal", "metall_thick"),
        ("materials\\metal_pipe", "metall_thick"),
        ("materials\\metal_plate", "metall_thick"),
        ("materials\\setka_rabica", "rabitz"),
        ("materials\\shifer", "stone"),
        ("materials\\stucco", "concrete"),
        ("materials\\tin", "metall_thick"),
        ("materials\\tree_trunk", "wood_silent"),
        ("materials\\wood", "wood_silent"),
        ("materials\\wooden_board", "wood_silent"),
    ]
    .into_iter()
    .map(|(source, target)| (source.to_owned(), target.to_owned()))
    .collect()
}

/// One resolved collision material binding, recorded in the port manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionMaterialBinding {
    pub source_id: u16,
    pub source_name: String,
    pub face_name: String,
    pub target_id: i32,
    pub target_name: String,
    pub triangle_count: usize,
}

/// Resolve every source material the level's collision actually uses.
///
/// `library` names the source ids, `correspondence` is the reviewed
/// source-to-target name table, and `target_materials` is the v0.100b
/// dictionary read from `game_materials/game.materials`.
pub fn resolve_bindings(
    used: &BTreeMap<u16, usize>,
    library: &BTreeMap<u16, GameMaterial>,
    correspondence: &BTreeMap<String, String>,
    target_materials: &BTreeMap<String, i32>,
) -> Result<Vec<CollisionMaterialBinding>> {
    let mut bindings = Vec::with_capacity(used.len());
    for (source_id, triangle_count) in used {
        let source_name = library
            .get(source_id)
            .with_context(|| {
                format!(
                    "collision uses game material id {source_id}, which gamemtl.xr does not define"
                )
            })?
            .name
            .clone();
        let target_name = correspondence
            .get(&source_name)
            .with_context(|| {
                format!(
                    "no reviewed v0.100b physical material for {source_name:?}; add it to \
                     collision_material_overrides"
                )
            })?
            .clone();
        let target_id = *target_materials
            .get(&target_name)
            .with_context(|| format!("v0.100b has no physical material named {target_name:?}"))?;
        // The target settings writer rejects the same sentinels; catching them
        // here names the source material that produced one.
        if target_id < 0 || target_id == 65_535 {
            bail!(
                "physical material {target_name:?} resolved to the invalid id {target_id} \
                 (from source {source_name:?})"
            );
        }
        bindings.push(CollisionMaterialBinding {
            source_id: *source_id,
            source_name,
            face_name: face_dictionary_name(*source_id),
            target_id,
            target_name,
            triangle_count: *triangle_count,
        });
    }
    Ok(bindings)
}

/// Reject overrides that no longer apply, so a stale review cannot survive a
/// source change unnoticed.
pub fn validate_override_usage(
    overrides: &BTreeMap<String, String>,
    library: &BTreeMap<u16, GameMaterial>,
    used: &BTreeMap<u16, usize>,
) -> Result<()> {
    let used_names: std::collections::BTreeSet<&String> = used
        .keys()
        .filter_map(|id| library.get(id).map(|material| &material.name))
        .collect();
    let stale: Vec<&String> = overrides
        .keys()
        .filter(|name| !used_names.contains(name))
        .collect();
    if !stale.is_empty() {
        bail!("collision material overrides no longer apply to any used material: {stale:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::codec::gamemtl;

    fn library_of(entries: &[(u16, &str)]) -> BTreeMap<u16, GameMaterial> {
        entries
            .iter()
            .map(|(id, name)| {
                (
                    *id,
                    GameMaterial {
                        name: (*name).to_owned(),
                        flags: 0,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn names_the_face_dictionary_after_the_source_identifier() {
        assert_eq!(face_dictionary_name(3), "soc_mtl_0003");
        assert_eq!(face_dictionary_name(1024), "soc_mtl_1024");
    }

    #[test]
    fn resolves_used_materials_through_reviewed_names() {
        let library = gamemtl::parse_library(&gamemtl::fixtures::library()).unwrap();
        let used = BTreeMap::from([(3_u16, 62_749_usize), (39, 579_315)]);
        let target = BTreeMap::from([("concrete".to_owned(), 19), ("wood_silent".to_owned(), 13)]);
        let bindings =
            resolve_bindings(&used, &library, &default_correspondence(), &target).unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].face_name, "soc_mtl_0003");
        assert_eq!(bindings[0].target_name, "concrete");
        assert_eq!(bindings[0].target_id, 19);
        assert_eq!(bindings[1].target_name, "wood_silent");
        assert_eq!(bindings[1].triangle_count, 579_315);
    }

    /// Four distinct source metals share one target entry; the port must keep
    /// them as separate dictionary entries so the loss stays visible.
    #[test]
    fn distinct_source_metals_keep_distinct_dictionary_entries() {
        let correspondence = default_correspondence();
        for name in [
            "materials\\metal",
            "materials\\metal_plate",
            "materials\\metal_pipe",
            "materials\\tin",
        ] {
            assert_eq!(correspondence[name], "metall_thick");
        }
        assert_ne!(face_dictionary_name(5), face_dictionary_name(13));
    }

    #[test]
    fn fails_on_a_source_material_without_a_reviewed_target() {
        let library = library_of(&[(5, "materials\\lava")]);
        let used = BTreeMap::from([(5_u16, 1_usize)]);
        let error = resolve_bindings(&used, &library, &default_correspondence(), &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no reviewed v0.100b physical material"),
            "{error}"
        );
    }

    #[test]
    fn fails_when_the_target_dictionary_lacks_the_reviewed_name() {
        let library = library_of(&[(3, "materials\\concrete")]);
        let used = BTreeMap::from([(3_u16, 1_usize)]);
        let error = resolve_bindings(&used, &library, &default_correspondence(), &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("no physical material named"), "{error}");
    }

    #[test]
    fn rejects_a_stale_override() {
        let library = library_of(&[(3, "materials\\concrete")]);
        let used = BTreeMap::from([(3_u16, 1_usize)]);
        let overrides = BTreeMap::from([("materials\\lava".to_owned(), "stone".to_owned())]);
        assert!(validate_override_usage(&overrides, &library, &used).is_err());
        assert!(
            validate_override_usage(
                &BTreeMap::from([("materials\\concrete".to_owned(), "stone".to_owned())]),
                &library,
                &used
            )
            .is_ok()
        );
    }
}
