//! Inventory and decode reports for one streamed Dirty Bomb map graph.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::SCALE;
use super::package::{ExportEntry, Package};
use super::scene;
use super::source::DirtyBombSource;
use super::static_mesh;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub path: String,
    pub disk_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledMap {
    pub name: String,
    pub package_count: usize,
    pub disk_bytes: u64,
    pub packages: Vec<InstalledPackage>,
}

/// Filesystem-only inventory. This is intentionally cheap; `probe_map` is the
/// command that treats successful UE3 decoding as evidence.
pub fn inventory(source: &DirtyBombSource) -> Result<Vec<InstalledMap>> {
    let mut maps = Vec::new();
    for name in source.maps()? {
        let mut packages = Vec::new();
        let mut disk_bytes = 0u64;
        for path in source.map_packages(&name)? {
            let bytes = fs::metadata(&path)
                .with_context(|| format!("stating {}", path.display()))?
                .len();
            disk_bytes = disk_bytes
                .checked_add(bytes)
                .context("map disk byte count overflow")?;
            packages.push(InstalledPackage {
                path: source.relative_path(&path),
                disk_bytes: bytes,
            });
        }
        maps.push(InstalledMap {
            name,
            package_count: packages.len(),
            disk_bytes,
            packages,
        });
    }
    Ok(maps)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub count: usize,
    pub min_uu: [f32; 3],
    pub max_uu: [f32; 3],
    /// `[UE.X, UE.Z, -UE.Y] * SCALE`: centimetres/Z-up to metres/Y-up,
    /// preserving handedness so placed geometry and lettering are not
    /// mirrored.
    pub min_vostok_m: [f32; 3],
    pub max_vostok_m: [f32; 3],
}

impl Bounds {
    fn one(point: [f32; 3]) -> Self {
        Self {
            count: 1,
            min_uu: point,
            max_uu: point,
            min_vostok_m: ue_to_vostok(point),
            max_vostok_m: ue_to_vostok(point),
        }
    }

    fn include(&mut self, point: [f32; 3]) {
        self.count += 1;
        for axis in 0..3 {
            self.min_uu[axis] = self.min_uu[axis].min(point[axis]);
            self.max_uu[axis] = self.max_uu[axis].max(point[axis]);
        }
        // The axis rotation negates UE Y, so converting the two endpoints can
        // exchange the target Z minimum and maximum.
        let converted = [ue_to_vostok(self.min_uu), ue_to_vostok(self.max_uu)];
        self.min_vostok_m = std::array::from_fn(|axis| converted[0][axis].min(converted[1][axis]));
        self.max_vostok_m = std::array::from_fn(|axis| converted[0][axis].max(converted[1][axis]));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub package: String,
    pub object: String,
    pub location_uu: [f32; 3],
    pub location_vostok_m: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageProbe {
    pub path: String,
    pub disk_bytes: usize,
    pub logical_bytes: usize,
    pub version: u16,
    pub licensee_version: u16,
    pub engine_version: i32,
    pub cooked_content_version: i32,
    pub compression_flags: u32,
    pub compression_chunks: usize,
    pub names: usize,
    pub imports: usize,
    pub exports: usize,
    pub static_meshes_decoded: usize,
    pub static_mesh_lod0_vertices: usize,
    pub static_mesh_lod0_triangles: usize,
    pub static_mesh_lod0_sections: usize,
    pub static_mesh_errors: BTreeMap<String, usize>,
    pub static_mesh_placements: usize,
    pub visible_static_mesh_placements: usize,
    pub hidden_static_mesh_placements: usize,
    pub collidable_static_mesh_placements: usize,
    pub local_static_mesh_placements: usize,
    pub imported_static_mesh_placements: usize,
    pub static_mesh_placement_classes: BTreeMap<String, usize>,
    pub properties_decoded: usize,
    pub property_errors: BTreeMap<String, usize>,
}

/// Stable evidence used to decide and later verify a map port.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapProbe {
    pub map: String,
    pub packages: Vec<PackageProbe>,
    pub package_count: usize,
    pub disk_bytes: usize,
    pub logical_bytes: usize,
    pub names: usize,
    pub imports: usize,
    pub exports: usize,
    pub classes: BTreeMap<String, usize>,
    pub class_serial_bytes: BTreeMap<String, usize>,
    pub external_packages: BTreeSet<String>,
    pub declared_streaming_levels: BTreeSet<String>,
    pub discovered_levels: BTreeSet<String>,
    pub missing_streaming_levels: BTreeSet<String>,
    pub undeclared_sublevels: BTreeSet<String>,
    pub location_bounds: BTreeMap<String, Bounds>,
    pub player_starts: Vec<SpawnPoint>,
    pub static_mesh_references: BTreeSet<String>,
    pub unresolved_static_mesh_references: usize,
    pub static_meshes_decoded: usize,
    pub static_mesh_lod0_vertices: usize,
    pub static_mesh_lod0_triangles: usize,
    pub static_mesh_lod0_sections: usize,
    pub static_mesh_errors: BTreeMap<String, usize>,
    pub static_mesh_placements: usize,
    pub visible_static_mesh_placements: usize,
    pub hidden_static_mesh_placements: usize,
    pub collidable_static_mesh_placements: usize,
    pub local_static_mesh_placements: usize,
    pub imported_static_mesh_placements: usize,
    pub static_mesh_placement_classes: BTreeMap<String, usize>,
    pub properties_decoded: usize,
    pub property_errors: BTreeMap<String, usize>,
    pub tagged_properties: BTreeMap<String, BTreeMap<String, usize>>,
}

pub fn probe_map(source: &DirtyBombSource, requested: &str) -> Result<MapProbe> {
    let map = source.resolve_map(requested)?;
    let paths = source.map_packages(&map)?;
    let discovered_levels: BTreeSet<String> = paths
        .iter()
        .filter_map(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .collect();
    let mut report = MapProbe {
        map: map.clone(),
        packages: Vec::new(),
        package_count: paths.len(),
        disk_bytes: 0,
        logical_bytes: 0,
        names: 0,
        imports: 0,
        exports: 0,
        classes: BTreeMap::new(),
        class_serial_bytes: BTreeMap::new(),
        external_packages: BTreeSet::new(),
        declared_streaming_levels: BTreeSet::new(),
        discovered_levels,
        missing_streaming_levels: BTreeSet::new(),
        undeclared_sublevels: BTreeSet::new(),
        location_bounds: BTreeMap::new(),
        player_starts: Vec::new(),
        static_mesh_references: BTreeSet::new(),
        unresolved_static_mesh_references: 0,
        static_meshes_decoded: 0,
        static_mesh_lod0_vertices: 0,
        static_mesh_lod0_triangles: 0,
        static_mesh_lod0_sections: 0,
        static_mesh_errors: BTreeMap::new(),
        static_mesh_placements: 0,
        visible_static_mesh_placements: 0,
        hidden_static_mesh_placements: 0,
        collidable_static_mesh_placements: 0,
        local_static_mesh_placements: 0,
        imported_static_mesh_placements: 0,
        static_mesh_placement_classes: BTreeMap::new(),
        properties_decoded: 0,
        property_errors: BTreeMap::new(),
        tagged_properties: BTreeMap::new(),
    };

    for path in paths {
        let relative = source.relative_path(&path);
        let package = Package::read(&path)
            .with_context(|| format!("decoding Dirty Bomb package {relative}"))?;
        probe_package(&package, &relative, &mut report)?;
    }

    let root_lower = map.to_ascii_lowercase();
    let declared_lower: BTreeSet<String> = report
        .declared_streaming_levels
        .iter()
        .map(|level| leaf_package_name(level).to_ascii_lowercase())
        .collect();
    let discovered_lower: BTreeMap<String, String> = report
        .discovered_levels
        .iter()
        .map(|level| (level.to_ascii_lowercase(), level.clone()))
        .collect();
    for level in &report.declared_streaming_levels {
        let leaf = leaf_package_name(level);
        if !discovered_lower.contains_key(&leaf.to_ascii_lowercase()) {
            report.missing_streaming_levels.insert(level.clone());
        }
    }
    for level in &report.discovered_levels {
        let lower = level.to_ascii_lowercase();
        if lower != root_lower && !declared_lower.contains(&lower) {
            report.undeclared_sublevels.insert(level.clone());
        }
    }
    report.player_starts.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then_with(|| left.object.cmp(&right.object))
    });
    Ok(report)
}

fn probe_package(package: &Package, path: &str, report: &mut MapProbe) -> Result<()> {
    let mut package_probe = PackageProbe {
        path: path.to_owned(),
        disk_bytes: package.disk_size,
        logical_bytes: package.logical_size,
        version: package.summary.version,
        licensee_version: package.summary.licensee_version,
        engine_version: package.summary.engine_version,
        cooked_content_version: package.summary.cooked_content_version,
        compression_flags: package.summary.compression_flags,
        compression_chunks: package.summary.compressed_chunks.len(),
        names: package.names.len(),
        imports: package.imports.len(),
        exports: package.exports.len(),
        static_meshes_decoded: 0,
        static_mesh_lod0_vertices: 0,
        static_mesh_lod0_triangles: 0,
        static_mesh_lod0_sections: 0,
        static_mesh_errors: BTreeMap::new(),
        static_mesh_placements: 0,
        visible_static_mesh_placements: 0,
        hidden_static_mesh_placements: 0,
        collidable_static_mesh_placements: 0,
        local_static_mesh_placements: 0,
        imported_static_mesh_placements: 0,
        static_mesh_placement_classes: BTreeMap::new(),
        properties_decoded: 0,
        property_errors: BTreeMap::new(),
    };
    report.disk_bytes = report
        .disk_bytes
        .checked_add(package.disk_size)
        .context("map disk byte count overflow")?;
    report.logical_bytes = report
        .logical_bytes
        .checked_add(package.logical_size)
        .context("map logical byte count overflow")?;
    report.names += package.names.len();
    report.imports += package.imports.len();
    report.exports += package.exports.len();
    report.external_packages.extend(package.external_packages());

    for placement in scene::placements(package)? {
        package_probe.static_mesh_placements += 1;
        report.static_mesh_placements += 1;
        if placement.visible {
            package_probe.visible_static_mesh_placements += 1;
            report.visible_static_mesh_placements += 1;
        } else {
            package_probe.hidden_static_mesh_placements += 1;
            report.hidden_static_mesh_placements += 1;
        }
        if placement.collision_enabled {
            package_probe.collidable_static_mesh_placements += 1;
            report.collidable_static_mesh_placements += 1;
        }
        if placement.local_mesh {
            package_probe.local_static_mesh_placements += 1;
            report.local_static_mesh_placements += 1;
        } else {
            package_probe.imported_static_mesh_placements += 1;
            report.imported_static_mesh_placements += 1;
        }
        *package_probe
            .static_mesh_placement_classes
            .entry(placement.actor_class.clone())
            .or_default() += 1;
        *report
            .static_mesh_placement_classes
            .entry(placement.actor_class)
            .or_default() += 1;
    }

    for export in &package.exports {
        *report.classes.entry(export.class_name.clone()).or_default() += 1;
        *report
            .class_serial_bytes
            .entry(export.class_name.clone())
            .or_default() += export.serial_size;
        if export.class_name == "StaticMesh" {
            match static_mesh::parse(package, export) {
                Ok(mesh) => {
                    let lod = &mesh.lods[0];
                    let triangles: usize =
                        lod.elements.iter().map(|element| element.triangles).sum();
                    package_probe.static_meshes_decoded += 1;
                    package_probe.static_mesh_lod0_vertices += lod.vertices.len();
                    package_probe.static_mesh_lod0_triangles += triangles;
                    package_probe.static_mesh_lod0_sections += lod.elements.len();
                    report.static_meshes_decoded += 1;
                    report.static_mesh_lod0_vertices += lod.vertices.len();
                    report.static_mesh_lod0_triangles += triangles;
                    report.static_mesh_lod0_sections += lod.elements.len();
                }
                Err(error) => {
                    let message = error.to_string();
                    *package_probe
                        .static_mesh_errors
                        .entry(message.clone())
                        .or_default() += 1;
                    *report.static_mesh_errors.entry(message).or_default() += 1;
                }
            }
        }
        if !has_useful_tagged_properties(export) {
            continue;
        }
        let properties = match package.properties(export) {
            Ok(properties) => properties,
            Err(error) => {
                let message = error.to_string();
                *package_probe
                    .property_errors
                    .entry(message.clone())
                    .or_default() += 1;
                *report.property_errors.entry(message).or_default() += 1;
                continue;
            }
        };
        package_probe.properties_decoded += 1;
        report.properties_decoded += 1;

        let mut location = None;
        for property in &properties {
            *report
                .tagged_properties
                .entry(export.class_name.clone())
                .or_default()
                .entry(property.name.clone())
                .or_default() += 1;
            if matches!(property.name.as_str(), "Location" | "Translation") {
                location = package.property_vector(property).filter(|point| {
                    point.iter().all(|coordinate| coordinate.is_finite())
                        && point.iter().all(|coordinate| coordinate.abs() < 1.0e9)
                });
            }
            if export.class_name == "StaticMeshComponent" && property.name == "StaticMesh" {
                if let Some(index) = package.property_object(property)? {
                    if index == 0 {
                        report.unresolved_static_mesh_references += 1;
                    } else {
                        match package.object_path(index) {
                            Ok(path) => {
                                report.static_mesh_references.insert(path);
                            }
                            Err(_) => report.unresolved_static_mesh_references += 1,
                        }
                    }
                }
            }
            if export.class_name.starts_with("LevelStreaming")
                && matches!(
                    property.name.as_str(),
                    "PackageName" | "PackageNameToLoad" | "PackageName_DEPRECATED"
                )
            {
                if let Some(name) = package.property_name(property)? {
                    if name != "None" && !name.is_empty() {
                        report.declared_streaming_levels.insert(name);
                    }
                } else if let Some(name) = package.property_string(property)? {
                    if !name.is_empty() {
                        report.declared_streaming_levels.insert(name);
                    }
                }
            }
        }
        if let Some(location) = location {
            report
                .location_bounds
                .entry(export.class_name.clone())
                .and_modify(|bounds| bounds.include(location))
                .or_insert_with(|| Bounds::one(location));
            if export.class_name == "PlayerStart" {
                report.player_starts.push(SpawnPoint {
                    package: path.to_owned(),
                    object: export.object_name.clone(),
                    location_uu: location,
                    location_vostok_m: ue_to_vostok(location),
                });
            }
        }
    }
    report.packages.push(package_probe);
    Ok(())
}

fn has_useful_tagged_properties(export: &ExportEntry) -> bool {
    export.serial_size >= 12
        && (export.class_name.ends_with("Actor")
            || export.class_name.ends_with("Volume")
            || export.class_name.ends_with("Light")
            || matches!(
                export.class_name.as_str(),
                "PlayerStart"
                    | "Emitter"
                    | "AutoLadder"
                    | "SplineLoftActor"
                    | "SDStitchingProxy"
                    | "StaticMeshComponent"
                    | "SplineComponent"
                    | "SplineMeshComponent"
                    | "Material"
                    | "MaterialInstanceConstant"
                    | "Texture2D"
                    | "LevelStreamingAlwaysLoaded"
                    | "LevelStreamingKismet"
            ))
}

fn leaf_package_name(name: &str) -> &str {
    name.rsplit(['.', '/', '\\']).next().unwrap_or(name)
}

pub fn ue_to_vostok(point: [f32; 3]) -> [f32; 3] {
    [point[0] * SCALE, point[2] * SCALE, -point[1] * SCALE]
}

pub fn write_json(report: &MapProbe, output: &Path) -> Result<()> {
    fs::write(output, serde_json::to_vec_pretty(report)?)
        .with_context(|| format!("writing {}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ue3_units_and_up_axis() {
        assert_eq!(ue_to_vostok([100.0, 200.0, 300.0]), [2.0, 6.0, -4.0]);
    }

    #[test]
    fn extracts_a_streaming_package_leaf() {
        assert_eq!(
            leaf_package_name("Maps/EXE_Test.EXE_Test_Art"),
            "EXE_Test_Art"
        );
    }
}
