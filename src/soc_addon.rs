//! Optional vegetation, lighting and objective-item layer for an X-Ray map.
//!
//! This intentionally emits an ordinary overlay bundle. The converted map is
//! its base, not its output, so removing the addon restores the map exactly.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::archive::safe_join;
use crate::manifest::BuiltFile;
use crate::soc::convert::spawn::{Candidate, STANDING_CLEARANCE, standable};
use crate::soc::source::{SocSource, canonicalize_dds, parse_dds};
use crate::weapon::{prepare_empty_output, record_file};
use crate::{BinaryConfig, ConfigValue, ResourceDb};

const CLIENT_PROJECT: &str = "projects/cs_training_camp/client_project";
const SERVER_PROJECT: &str = "projects/cs_training_camp/server_project";
const POST_PROCESS: &str = "material_instances/ported/cs/training_camp/post_process.material";
const FLOWER_TEXTURE: &str = "ported/cs/training_camp/addons/wildflower_atlas";
const FLOWER_MATERIAL: &str = "ported/cs/training_camp/addons/wildflowers";
const FLOWER_MODEL: &str = "ported/cs/training_camp/addons/wildflower";
const SOURCE_FLOWER_MODEL: &str = "models/level/03/flora/small_grass_01.model/";
const VICTORY_ITEM_MODEL: &str = "item/quest/electrobox";
const VICTORY_ITEM_CONTAINER_MODEL: &str = "item/quest/container";

const GRASS_TERRAIN_SHADERS: &[&str] = &[
    "levels\\mp_darkvalley_asfalt",
    "levels\\mp_darkvalley_grass",
    "levels\\mp_darkvalley_sand",
];

// The barbed-wire render surfaces (source shaders 72 and 73) bound the
// Training Camp enclosure at x=-9.2..65.3, z=-30.1..37.8.
const FENCE_MIN: [f32; 2] = [-9.2, -30.1];
const FENCE_MAX: [f32; 2] = [65.3, 37.8];
// Keep an inward margin so upright grass cards do not poke through the fence.
const FENCED_ARENA_MIN: [f32; 2] = [-8.2, -29.1];
const FENCED_ARENA_MAX: [f32; 2] = [64.3, 36.8];
// The eroded southern channel continues well beyond the fence. Keep the
// meadow continuous down to the end of its authored terrain instead of
// cutting it off at the fence's rectangular bounding box.
const SOUTH_RAVINE_MIN: [f32; 2] = [-8.5, -67.0];
const SOUTH_RAVINE_MAX: [f32; 2] = [53.0, FENCED_ARENA_MIN[1]];
const GRASS_BATCH_TILE: f32 = 8.0;
const GRASS_BATCH_PREFIX: &str = "ported/cs/training_camp/addons/grass_batch";
const GRASS_MODEL_TEMPLATE: &str = "models/level/03/flora/gra_green_01.model/";
const CONIFER_GRID: f32 = 12.0;
const CONIFER_CLUSTER_TREES: usize = 27;
const CONIFER_CLUSTER_RADIUS: f32 = 13.0;
const MAX_CONIFER_CLUSTER_RELIEF: f32 = 2.5;
const BASE_CONIFER_CLUSTERS: usize = 600;
const OUTSKIRT_FOREST_DISTANCE: f32 = 100.0;
const CONIFER_BATCH_PREFIX: &str = "ported/cs/training_camp/addons/conifer_batch";
const CONIFER_MODEL_TEMPLATE: &str = "models/flora/elka_green_01.model/";

const STATIC_VERTEX_DECLARATION: [u8; 48] = [
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x10, 0x00, 0x04, 0x00, 0x06, 0x00, 0x00, 0x00, 0x14, 0x00, 0x04, 0x00, 0x07, 0x00,
    0x00, 0x00, 0x18, 0x00, 0x01, 0x00, 0x05, 0x00, 0xff, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00,
];

const GRASS_MODELS: &[&str] = &[
    "level/03/flora/gra_green_01",
    "level/03/flora/gra_green_02",
    "level/03/flora/gra_green_03",
    "level/03/flora/gra_most_green_01",
    "level/03/flora/gra_most_green_02",
    "level/03/flora/small_grass_02",
    "level/03/flora/small_grass_03",
    "level/03/flora/small_grass_05",
];

const CONIFER_MODELS: &[&str] = &[
    "flora/elka_green_01",
    "flora/elka_green_02",
    "flora/elka_green_03",
    "flora/elka_green_04",
    "flora/elka_green_05",
];

#[derive(Debug, Serialize)]
pub struct SocAddonRecord {
    format_version: u32,
    asset_id: String,
    pub grass_instances: usize,
    pub grass_batches: usize,
    pub conifer_instances: usize,
    pub conifer_batches: usize,
    pub flower_instances: usize,
    flower_island_center: [f32; 2],
    flower_island_radius: f32,
    pub victory_item_spawners: Vec<[f32; 3]>,
    pub victory_item_containers: Vec<[f32; 3]>,
    skylight_multiplier: f32,
    skylight_power: f32,
    files: Vec<BuiltFile>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_bundle(
    source_root: &Path,
    level: &str,
    base_db: &Path,
    flower_png: &Path,
    output: &Path,
    grass_limit: usize,
    conifer_limit: usize,
    flower_center: [f32; 2],
    flower_radius: f32,
    flower_limit: usize,
    victory_item_count: usize,
    skylight_multiplier: f32,
    skylight_power: f32,
) -> Result<SocAddonRecord> {
    if grass_limit == 0 || conifer_limit == 0 || flower_limit == 0 || victory_item_count == 0 {
        bail!("grass, conifer, flower and victory-item counts must be positive");
    }
    if flower_radius <= 0.0 {
        bail!("flower island radius must be positive");
    }
    prepare_empty_output(output)?;
    let overlay = output.join("overlay");
    fs::create_dir_all(&overlay)?;

    let mut source = SocSource::open(source_root)?;
    let level_data = crate::soc::build::decode_level(&mut source, level)?;
    let victory_item_containers = authored_starting_bases(&mut source, level)?;
    let victory_item_spawners =
        authored_victory_item_spawners(&mut source, level, victory_item_count)?;
    let grass_positions = terrain_grass_positions(&level_data, grass_limit)?;
    let conifer_positions =
        terrain_conifer_positions(&level_data, conifer_limit.div_ceil(CONIFER_CLUSTER_TREES))?;

    let radius_squared = flower_radius * flower_radius;
    let flower_candidates = standable(&level_data.cform, 1.5, usize::MAX);
    let flower_positions: Vec<[f32; 3]> = flower_candidates
        .iter()
        .filter(|candidate| {
            let dx = candidate.position[0] - flower_center[0];
            let dz = candidate.position[2] - flower_center[1];
            dx * dx + dz * dz <= radius_squared
        })
        .take(flower_limit)
        .map(vegetation_position)
        .collect();
    if flower_positions.len() < flower_limit / 4 {
        bail!(
            "flower island at [{:.1}, {:.1}] yielded only {} placements; choose another centre",
            flower_center[0],
            flower_center[1],
            flower_positions.len()
        );
    }

    let mut db = ResourceDb::open(base_db)?;
    let mut files = Vec::new();
    let grass_batches = write_grass_batches(&mut db, &overlay, &grass_positions, &mut files)?;
    let conifer_batches = write_conifer_batches(&mut db, &overlay, &conifer_positions, &mut files)?;
    write_client_project(
        &mut db,
        &overlay,
        &grass_batches,
        grass_positions.len(),
        &conifer_batches,
        conifer_positions.len(),
        &flower_positions,
        &victory_item_spawners,
        &victory_item_containers,
        &mut files,
    )?;
    write_server_project(
        &mut db,
        &overlay,
        &victory_item_spawners,
        &victory_item_containers,
        &mut files,
    )?;
    write_post_process(
        &mut db,
        &overlay,
        skylight_multiplier,
        skylight_power,
        &mut files,
    )?;
    write_flower_texture(&mut db, flower_png, &overlay, &mut files)?;
    write_flower_material(&mut db, &overlay, &mut files)?;
    copy_flower_model(&mut db, &overlay, &mut files)?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    let record = SocAddonRecord {
        format_version: 1,
        asset_id: "cs-training-camp-meadow-addon".into(),
        grass_instances: grass_positions.len(),
        grass_batches: grass_batches.len(),
        conifer_instances: conifer_positions.len() * CONIFER_CLUSTER_TREES,
        conifer_batches: conifer_batches.len(),
        flower_instances: flower_positions.len(),
        flower_island_center: flower_center,
        flower_island_radius: flower_radius,
        victory_item_spawners,
        victory_item_containers,
        skylight_multiplier,
        skylight_power,
        files,
    };
    fs::write(
        output.join("port-manifest.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    Ok(record)
}

fn terrain_grass_positions(
    level: &crate::soc::build::DecodedLevel,
    limit: usize,
) -> Result<Vec<[f32; 3]>> {
    let (surfaces, _) =
        crate::soc::convert::surface::merge_visuals(&level.visuals, &level.buffers)?;
    let mut positions = Vec::new();
    for surface in surfaces {
        let shader = &level.shader_table.get(surface.shader_id)?.shader;
        if !GRASS_TERRAIN_SHADERS.contains(&shader.as_str()) {
            continue;
        }
        for (triangle_index, triangle) in surface.indices.chunks_exact(3).enumerate() {
            let a = surface.vertices[usize::from(triangle[0])].position;
            let b = surface.vertices[usize::from(triangle[1])].position;
            let c = surface.vertices[usize::from(triangle[2])].position;
            let edge0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let edge1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                edge0[1] * edge1[2] - edge0[2] * edge1[1],
                edge0[2] * edge1[0] - edge0[0] * edge1[2],
                edge0[0] * edge1[1] - edge0[1] * edge1[0],
            ];
            let double_area =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if !double_area.is_normal() || normal[1] / double_area < 0.45 {
                continue;
            }

            // Generate more points than the project can ultimately hold, then
            // evenly limit them below. This gives the enclosed terrain an
            // approximately 0.35-square-metre sampling grid without patterns
            // following the compiler's triangle edges.
            let samples = ((double_area * 0.5 / 0.35).ceil() as usize).clamp(1, 64);
            let triangle_seed = u32::from(surface.shader_id).wrapping_mul(0x9e37_79b9)
                ^ (triangle_index as u32).wrapping_mul(0x85eb_ca6b);
            for sample in 0..samples {
                let sample = sample as u32;
                let mut u = hash_unit(triangle_seed ^ sample.wrapping_mul(0xc2b2_ae35));
                let mut v =
                    hash_unit(triangle_seed.rotate_left(13) ^ sample.wrapping_mul(0x27d4_eb2d));
                if u + v > 1.0 {
                    u = 1.0 - u;
                    v = 1.0 - v;
                }
                let weights = [1.0 - u - v, u, v];
                let position = [
                    a[0] * weights[0] + b[0] * weights[1] + c[0] * weights[2],
                    a[1] * weights[0] + b[1] * weights[1] + c[1] * weights[2] + 0.03,
                    a[2] * weights[0] + b[2] * weights[1] + c[2] * weights[2],
                ];
                if inside_grass_region(position) {
                    positions.push(position);
                }
            }
        }
    }
    if positions.is_empty() {
        bail!("the level has no upward-facing triangles in the terrain shader mask");
    }
    if positions.len() <= limit {
        return Ok(positions);
    }

    // Preserve coverage across the entire material mask when a caller asks
    // for fewer instances; truncation would cover only the first shader/area.
    let count = positions.len();
    Ok((0..limit)
        .map(|index| positions[index * count / limit])
        .collect())
}

fn inside_grass_region(position: [f32; 3]) -> bool {
    let inside = |minimum: [f32; 2], maximum: [f32; 2]| {
        position[0] >= minimum[0]
            && position[0] <= maximum[0]
            && position[2] >= minimum[1]
            && position[2] <= maximum[1]
    };
    inside(FENCED_ARENA_MIN, FENCED_ARENA_MAX) || inside(SOUTH_RAVINE_MIN, SOUTH_RAVINE_MAX)
}

fn hash_unit(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value & 0x00ff_ffff) as f32 / 0x0100_0000 as f32
}

fn terrain_conifer_positions(
    level: &crate::soc::build::DecodedLevel,
    limit: usize,
) -> Result<Vec<[f32; 3]>> {
    let (surfaces, _) =
        crate::soc::convert::surface::merge_visuals(&level.visuals, &level.buffers)?;
    // Retain the triangle centre nearest each placement-grid centre. The
    // final farthest-point pass removes the grid's visible regularity while
    // preserving broad coverage of the non-playable terrain.
    #[derive(Clone, Copy)]
    struct TerrainCell {
        position: [f32; 3],
        score: f32,
        minimum_height: f32,
        maximum_height: f32,
    }

    let mut cells: BTreeMap<(i32, i32), TerrainCell> = BTreeMap::new();
    for surface in surfaces {
        let shader = &level.shader_table.get(surface.shader_id)?.shader;
        if !GRASS_TERRAIN_SHADERS.contains(&shader.as_str()) {
            continue;
        }
        for triangle in surface.indices.chunks_exact(3) {
            let a = surface.vertices[usize::from(triangle[0])].position;
            let b = surface.vertices[usize::from(triangle[1])].position;
            let c = surface.vertices[usize::from(triangle[2])].position;
            let edge0 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let edge1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                edge0[1] * edge1[2] - edge0[2] * edge1[1],
                edge0[2] * edge1[0] - edge0[0] * edge1[2],
                edge0[0] * edge1[1] - edge0[1] * edge1[0],
            ];
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if !length.is_normal() || normal[1] / length < 0.65 {
                continue;
            }
            let position = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0 + 0.05,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            // Cluster instances can rotate and scale; reserve their complete
            // crown radius in addition to the requested ten-metre clear belt.
            if distance_from_fence(position) < 10.0 + CONIFER_CLUSTER_RADIUS {
                continue;
            }
            let cell = (
                (position[0] / CONIFER_GRID).floor() as i32,
                (position[2] / CONIFER_GRID).floor() as i32,
            );
            let centre = [
                (cell.0 as f32 + 0.5) * CONIFER_GRID,
                (cell.1 as f32 + 0.5) * CONIFER_GRID,
            ];
            let score = (position[0] - centre[0]).powi(2) + (position[2] - centre[1]).powi(2);
            if let Some(existing) = cells.get_mut(&cell) {
                existing.minimum_height = existing.minimum_height.min(position[1]);
                existing.maximum_height = existing.maximum_height.max(position[1]);
                if score < existing.score {
                    existing.position = position;
                    existing.score = score;
                }
            } else {
                cells.insert(
                    cell,
                    TerrainCell {
                        position,
                        score,
                        minimum_height: position[1],
                        maximum_height: position[1],
                    },
                );
            }
        }
    }
    let mut candidates = Vec::new();
    for (&cell_key, cell) in &cells {
        let mut minimum_height = cell.minimum_height;
        let mut maximum_height = cell.maximum_height;
        // A cluster is larger than one placement cell. Include all adjacent
        // cells when measuring relief so clusters straddling the ravine lip do
        // not leave their downhill trees suspended in the air.
        for offset_x in -1..=1 {
            for offset_z in -1..=1 {
                let neighbour_key = (cell_key.0 + offset_x, cell_key.1 + offset_z);
                if let Some(neighbour) = cells.get(&neighbour_key) {
                    minimum_height = minimum_height.min(neighbour.minimum_height);
                    maximum_height = maximum_height.max(neighbour.maximum_height);
                }
            }
        }
        let mut position = cell.position;
        // Preserve the original forest placement and height on ordinary
        // terrain. Only a cluster touching a sharp ravine lip needs its shared
        // transform lowered; the trunks then sink harmlessly instead of the
        // downhill edge floating in the air.
        if maximum_height - minimum_height > MAX_CONIFER_CLUSTER_RELIEF {
            position[1] = minimum_height + 0.05;
        }
        candidates.push(Candidate {
            position,
            area: 0.0,
            headroom: f32::INFINITY,
        });
    }
    if candidates.is_empty() {
        bail!("the map has no terrain at least ten metres outside the fence");
    }
    let wanted = limit.min(candidates.len());
    // Preserve the established 600-cluster forest byte-for-byte in X/Z, then
    // fill every still-empty valid cell in the distant skyline ring. Those
    // unused outer cells are what leave terrain showing beside the perimeter
    // towers; adding them here avoids thickening the playable camp boundary.
    let mut selected = farthest_positions(&candidates, wanted.min(BASE_CONIFER_CLUSTERS))?;
    if selected.len() < wanted {
        for candidate in &candidates {
            if distance_from_fence(candidate.position) >= OUTSKIRT_FOREST_DISTANCE
                && !selected.contains(&candidate.position)
            {
                selected.push(candidate.position);
                if selected.len() == wanted {
                    break;
                }
            }
        }
    }
    // Keep custom limits useful even after all outer-ring cells are occupied.
    if selected.len() < wanted {
        for position in farthest_positions(&candidates, wanted)? {
            if !selected.contains(&position) {
                selected.push(position);
                if selected.len() == wanted {
                    break;
                }
            }
        }
    }
    Ok(selected)
}

fn distance_from_fence(position: [f32; 3]) -> f32 {
    let dx = if position[0] < FENCE_MIN[0] {
        FENCE_MIN[0] - position[0]
    } else if position[0] > FENCE_MAX[0] {
        position[0] - FENCE_MAX[0]
    } else {
        0.0
    };
    let dz = if position[2] < FENCE_MIN[1] {
        FENCE_MIN[1] - position[2]
    } else if position[2] > FENCE_MAX[1] {
        position[2] - FENCE_MAX[1]
    } else {
        0.0
    };
    (dx * dx + dz * dz).sqrt()
}

fn authored_victory_item_spawners(
    source: &mut SocSource,
    level: &str,
    count: usize,
) -> Result<Vec<[f32; 3]>> {
    let game = source.read_level_file(level, "level.game")?;
    let candidates: Vec<Candidate> = crate::soc::codec::spawn::actor_respawns(&game)?
        .into_iter()
        // Neutral deathmatch starts are hand-authored, standable positions on
        // the separate playable islands. They are safer than points inferred
        // from the map-wide collision mesh, which includes inaccessible land.
        .filter(|respawn| respawn.team == 0 && respawn.game_type & 0x0002 != 0)
        .map(|respawn| Candidate {
            position: respawn.position,
            area: 0.0,
            headroom: f32::INFINITY,
        })
        .collect();
    farthest_positions(&candidates, count).with_context(|| {
        format!(
            "selecting {count} victory-item spawners from {} authored island points",
            candidates.len()
        )
    })
}

fn authored_starting_bases(source: &mut SocSource, level: &str) -> Result<Vec<[f32; 3]>> {
    let game = source.read_level_file(level, "level.game")?;
    let respawns = crate::soc::codec::spawn::actor_respawns(&game)?;
    let mut teams: BTreeMap<u8, Vec<[f32; 3]>> = BTreeMap::new();
    for respawn in respawns {
        // 0x0004 is Clear Sky's team-deathmatch bit. Team zero entries are
        // neutral artefact/DM locations rather than a starting base.
        if respawn.team != 0 && respawn.game_type & 0x0004 != 0 {
            teams
                .entry(respawn.team)
                .or_default()
                .push(respawn.position);
        }
    }
    if teams.is_empty() {
        bail!("{level} has no authored team starting bases in level.game");
    }
    Ok(teams
        .into_values()
        .map(|points| {
            let count = points.len() as f32;
            let sum = points.into_iter().fold([0.0; 3], |mut sum, point| {
                for axis in 0..3 {
                    sum[axis] += point[axis];
                }
                sum
            });
            [sum[0] / count, sum[1] / count, sum[2] / count]
        })
        .collect())
}

fn vegetation_position(candidate: &Candidate) -> [f32; 3] {
    [
        candidate.position[0],
        candidate.position[1] - STANDING_CLEARANCE + 0.02,
        candidate.position[2],
    ]
}

fn farthest_positions(candidates: &[Candidate], count: usize) -> Result<Vec<[f32; 3]>> {
    let Some(first) = candidates.first() else {
        bail!("the map has no standable victory-item position");
    };
    let mut selected = vec![first.position];
    while selected.len() < count {
        let next = candidates
            .iter()
            .filter(|candidate| !selected.contains(&candidate.position))
            .max_by(|left, right| {
                minimum_distance_squared(left.position, &selected)
                    .total_cmp(&minimum_distance_squared(right.position, &selected))
            })
            .context("not enough distinct standable victory-item positions")?;
        selected.push(next.position);
    }
    Ok(selected)
}

fn minimum_distance_squared(position: [f32; 3], selected: &[[f32; 3]]) -> f32 {
    selected
        .iter()
        .map(|other| {
            let dx = position[0] - other[0];
            let dz = position[2] - other[2];
            dx * dx + dz * dz
        })
        .fold(f32::INFINITY, f32::min)
}

#[derive(Clone, Debug, PartialEq)]
struct StaticMesh {
    vertices: Vec<[u8; 32]>,
    indices: Vec<u16>,
}

impl StaticMesh {
    fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }
}

struct GrassBatch {
    model: String,
    position: [f32; 3],
}

struct ConiferBatch {
    model: String,
    position: [f32; 3],
}

fn write_grass_batches(
    db: &mut ResourceDb,
    overlay: &Path,
    positions: &[[f32; 3]],
    files: &mut Vec<BuiltFile>,
) -> Result<Vec<GrassBatch>> {
    let mut high_meshes = Vec::with_capacity(GRASS_MODELS.len());
    let mut low_meshes = Vec::with_capacity(GRASS_MODELS.len());
    for model in GRASS_MODELS {
        high_meshes.push(parse_static_mesh(&db.read(&format!(
            "models/{model}.model/render/lod_0sg/converted_model"
        ))?)?);
        low_meshes.push(parse_static_mesh(&db.read(&format!(
            "models/{model}.model/render/lod_1sg/converted_model"
        ))?)?);
    }

    let mut tiles: BTreeMap<(i32, i32), Vec<(usize, [f32; 3])>> = BTreeMap::new();
    for (index, position) in positions.iter().copied().enumerate() {
        let tile = (
            (position[0] / GRASS_BATCH_TILE).floor() as i32,
            (position[2] / GRASS_BATCH_TILE).floor() as i32,
        );
        tiles.entry(tile).or_default().push((index, position));
    }

    let mut batches = Vec::with_capacity(tiles.len());
    for (batch_index, (tile, instances)) in tiles.into_iter().enumerate() {
        let origin = [
            tile.0 as f32 * GRASS_BATCH_TILE,
            0.0,
            tile.1 as f32 * GRASS_BATCH_TILE,
        ];
        let mut high = StaticMesh::empty();
        let mut low = StaticMesh::empty();
        for (seed, position) in instances {
            let model = seed % GRASS_MODELS.len();
            let translation = [
                position[0] - origin[0],
                position[1],
                position[2] - origin[2],
            ];
            let scale = grass_scale(seed);
            append_static_mesh(&mut high, &high_meshes[model], translation, scale)?;
            append_static_mesh(&mut low, &low_meshes[model], translation, scale)?;
        }
        let model = format!("{GRASS_BATCH_PREFIX}_{batch_index:03}");
        write_grass_batch_model(db, overlay, &model, &high, &low, files)?;
        batches.push(GrassBatch {
            model,
            position: origin,
        });
    }
    Ok(batches)
}

fn write_conifer_batches(
    db: &mut ResourceDb,
    overlay: &Path,
    positions: &[[f32; 3]],
    files: &mut Vec<BuiltFile>,
) -> Result<Vec<ConiferBatch>> {
    const LODS: [&str; 3] = ["elka_0_sg", "elka_1_sg", "elka_2_sg"];
    const OFFSETS: [[f32; 2]; CONIFER_CLUSTER_TREES] = [
        [-6.0, -6.0],
        [-3.0, -6.0],
        [0.0, -6.0],
        [3.0, -6.0],
        [6.0, -6.0],
        [-6.0, -3.0],
        [-3.0, -3.0],
        [0.0, -3.0],
        [3.0, -3.0],
        [6.0, -3.0],
        [-6.0, 0.0],
        [-3.0, 0.0],
        [0.0, 0.0],
        [3.0, 0.0],
        [6.0, 0.0],
        [-6.0, 3.0],
        [-3.0, 3.0],
        [0.0, 3.0],
        [3.0, 3.0],
        [6.0, 3.0],
        [-6.0, 6.0],
        [-3.0, 6.0],
        [0.0, 6.0],
        [3.0, 6.0],
        [6.0, 6.0],
        [-7.5, 0.0],
        [7.5, 0.0],
    ];
    let mut source_meshes: [Vec<StaticMesh>; 3] = std::array::from_fn(|_| Vec::new());
    for model in CONIFER_MODELS {
        for (lod_index, lod) in LODS.iter().enumerate() {
            source_meshes[lod_index].push(parse_static_mesh(&db.read(&format!(
                "models/{model}.model/render/{lod}/converted_model"
            ))?)?);
        }
    }

    // Five reusable cluster meshes provide variation without copying every
    // tree's geometry into the archive. Their instances can still rotate and
    // scale independently in the client project.
    let mut cluster_models = Vec::with_capacity(CONIFER_MODELS.len());
    for cluster_index in 0..CONIFER_MODELS.len() {
        let mut meshes: [StaticMesh; 3] = std::array::from_fn(|_| StaticMesh::empty());
        for (tree_index, offset) in OFFSETS.iter().enumerate() {
            let model = (cluster_index + tree_index) % CONIFER_MODELS.len();
            let translation = [offset[0], -0.45, offset[1]];
            let scale = 0.82
                + hash_unit(
                    (cluster_index as u32).wrapping_mul(0x9e37_79b9)
                        ^ (tree_index as u32).wrapping_mul(0x85eb_ca6b),
                ) * 0.28;
            for lod_index in 0..LODS.len() {
                append_static_mesh(
                    &mut meshes[lod_index],
                    &source_meshes[lod_index][model],
                    translation,
                    scale,
                )?;
            }
        }
        let model = format!("{CONIFER_BATCH_PREFIX}_{cluster_index:03}");
        write_conifer_batch_model(db, overlay, &model, &meshes, files)?;
        cluster_models.push(model);
    }

    let mut batches = Vec::with_capacity(positions.len());
    for (index, position) in positions.iter().copied().enumerate() {
        batches.push(ConiferBatch {
            model: cluster_models[index % cluster_models.len()].clone(),
            position,
        });
    }
    Ok(batches)
}

fn grass_scale(seed: usize) -> f32 {
    let variation = 0.85 + ((seed.wrapping_mul(4051) & 0xff) as f32 / 255.0) * 0.3;
    1.35 * variation
}

fn parse_static_mesh(bytes: &[u8]) -> Result<StaticMesh> {
    if read_le_u32(bytes, 0)? != 3 {
        bail!("grass converted_model does not start with vertex chunk 3");
    }
    let vertex_chunk_size = usize::try_from(read_le_u32(bytes, 4)?)?;
    if bytes.get(8..56) != Some(STATIC_VERTEX_DECLARATION.as_slice()) {
        bail!("grass converted_model has an unexpected vertex declaration");
    }
    let vertex_count = usize::try_from(read_le_u32(bytes, 56)?)?;
    let vertex_start = 60;
    let vertex_end = vertex_start + vertex_count * 32;
    let index_chunk = 8 + vertex_chunk_size;
    if vertex_end != index_chunk || read_le_u32(bytes, index_chunk)? != 4 {
        bail!("grass converted_model has an invalid vertex or index chunk");
    }
    let index_count = usize::try_from(read_le_u32(bytes, index_chunk + 8)?)?;
    let index_start = index_chunk + 12;
    let index_end = index_start + index_count * 2;
    if index_end != bytes.len() {
        bail!("grass converted_model is truncated or has trailing data");
    }
    let mut vertices = Vec::with_capacity(vertex_count);
    for vertex in bytes[vertex_start..vertex_end].chunks_exact(32) {
        vertices.push(vertex.try_into().expect("32-byte vertex"));
    }
    let indices = bytes[index_start..index_end]
        .chunks_exact(2)
        .map(|index| u16::from_le_bytes(index.try_into().expect("two-byte index")))
        .collect();
    Ok(StaticMesh { vertices, indices })
}

fn append_static_mesh(
    target: &mut StaticMesh,
    source: &StaticMesh,
    translation: [f32; 3],
    scale: f32,
) -> Result<()> {
    let base = u16::try_from(target.vertices.len())?;
    if target.vertices.len() + source.vertices.len() > u16::MAX as usize {
        bail!("a grass batch exceeds the target model's u16 vertex space");
    }
    for source_vertex in &source.vertices {
        let mut vertex = *source_vertex;
        for axis in 0..3 {
            let offset = axis * 4;
            let value = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            vertex[offset..offset + 4]
                .copy_from_slice(&(value * scale + translation[axis]).to_le_bytes());
        }
        target.vertices.push(vertex);
    }
    target.indices.extend(
        source
            .indices
            .iter()
            .map(|index| {
                index
                    .checked_add(base)
                    .context("grass batch index overflow")
            })
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(())
}

fn encode_static_mesh(mesh: &StaticMesh) -> Result<Vec<u8>> {
    let vertex_chunk_size = STATIC_VERTEX_DECLARATION.len() + 4 + mesh.vertices.len() * 32;
    let index_chunk_size = 4 + mesh.indices.len() * 2;
    let mut output = Vec::with_capacity(16 + vertex_chunk_size + index_chunk_size);
    output.extend_from_slice(&3_u32.to_le_bytes());
    output.extend_from_slice(&u32::try_from(vertex_chunk_size)?.to_le_bytes());
    output.extend_from_slice(&STATIC_VERTEX_DECLARATION);
    output.extend_from_slice(&u32::try_from(mesh.vertices.len())?.to_le_bytes());
    for vertex in &mesh.vertices {
        output.extend_from_slice(vertex);
    }
    output.extend_from_slice(&4_u32.to_le_bytes());
    output.extend_from_slice(&u32::try_from(index_chunk_size)?.to_le_bytes());
    output.extend_from_slice(&u32::try_from(mesh.indices.len())?.to_le_bytes());
    for index in &mesh.indices {
        output.extend_from_slice(&index.to_le_bytes());
    }
    Ok(output)
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .context("converted_model u32 is outside the payload")?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn mesh_bounds(mesh: &StaticMesh) -> Result<([f32; 3], [f32; 3])> {
    if mesh.vertices.is_empty() {
        bail!("grass batch contains no vertices");
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            let offset = axis * 4;
            let value = f32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap());
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    Ok((min, max))
}

fn write_grass_batch_model(
    db: &mut ResourceDb,
    overlay: &Path,
    model: &str,
    high: &StaticMesh,
    low: &StaticMesh,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let high_bounds = mesh_bounds(high)?;
    let low_bounds = mesh_bounds(low)?;
    let model_bounds = (
        std::array::from_fn(|axis| high_bounds.0[axis].min(low_bounds.0[axis])),
        std::array::from_fn(|axis| high_bounds.1[axis].max(low_bounds.1[axis])),
    );
    let target_prefix = format!("models/{model}.model/");

    let mut settings = BinaryConfig::parse(&db.read(&format!("{GRASS_MODEL_TEMPLATE}settings"))?)?;
    for lod in ["lod_0sg", "lod_1sg"] {
        settings.root.set_path(
            &format!("material_settings.{lod}.used_in_portals"),
            ConfigValue::Boolean(false),
        )?;
    }
    write_config_resource(
        overlay,
        &format!("{target_prefix}settings"),
        &settings,
        "addon:batched-grass-settings",
        files,
    )?;

    let mut render =
        BinaryConfig::parse(&db.read(&format!("{GRASS_MODEL_TEMPLATE}render/export_properties"))?)?;
    set_bounds(&mut render, model_bounds)?;
    write_config_resource(
        overlay,
        &format!("{target_prefix}render/export_properties"),
        &render,
        "addon:batched-grass-render-bounds",
        files,
    )?;

    for (lod, mesh, bounds) in [("lod_0sg", high, high_bounds), ("lod_1sg", low, low_bounds)] {
        let mut properties = BinaryConfig::parse(&db.read(&format!(
            "{GRASS_MODEL_TEMPLATE}render/{lod}/export_properties"
        ))?)?;
        properties.root.set_path(
            "source.indices",
            ConfigValue::Integer(i32::try_from(mesh.indices.len())?),
        )?;
        properties.root.set_path(
            "source.primitives",
            ConfigValue::Integer(i32::try_from(mesh.indices.len() / 3)?),
        )?;
        properties.root.set_path(
            "source.vertices",
            ConfigValue::Integer(i32::try_from(mesh.vertices.len())?),
        )?;
        set_surface_bounds(&mut properties, bounds)?;
        write_config_resource(
            overlay,
            &format!("{target_prefix}render/{lod}/export_properties"),
            &properties,
            "addon:batched-grass-surface-bounds",
            files,
        )?;
        write_resource(
            overlay,
            &format!("{target_prefix}render/{lod}/converted_model"),
            &encode_static_mesh(mesh)?,
            "addon:batched-grass-geometry",
            files,
        )?;
    }
    Ok(())
}

fn write_conifer_batch_model(
    db: &mut ResourceDb,
    overlay: &Path,
    model: &str,
    meshes: &[StaticMesh; 3],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    const LODS: [&str; 3] = ["elka_0_sg", "elka_1_sg", "elka_2_sg"];
    let bounds: Vec<([f32; 3], [f32; 3])> =
        meshes.iter().map(mesh_bounds).collect::<Result<_>>()?;
    let model_bounds = bounds.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |mut combined, bounds| {
            for axis in 0..3 {
                combined.0[axis] = combined.0[axis].min(bounds.0[axis]);
                combined.1[axis] = combined.1[axis].max(bounds.1[axis]);
            }
            combined
        },
    );
    let target_prefix = format!("models/{model}.model/");

    let mut settings =
        BinaryConfig::parse(&db.read(&format!("{CONIFER_MODEL_TEMPLATE}settings"))?)?;
    for lod in LODS {
        settings.root.set_path(
            &format!("material_settings.{lod}.used_in_portals"),
            ConfigValue::Boolean(false),
        )?;
    }
    write_config_resource(
        overlay,
        &format!("{target_prefix}settings"),
        &settings,
        "addon:batched-conifer-settings",
        files,
    )?;

    let mut render = BinaryConfig::parse(
        &db.read(&format!("{CONIFER_MODEL_TEMPLATE}render/export_properties"))?,
    )?;
    set_bounds(&mut render, model_bounds)?;
    write_config_resource(
        overlay,
        &format!("{target_prefix}render/export_properties"),
        &render,
        "addon:batched-conifer-render-bounds",
        files,
    )?;

    for (index, lod) in LODS.iter().enumerate() {
        let mesh = &meshes[index];
        let mut properties = BinaryConfig::parse(&db.read(&format!(
            "{CONIFER_MODEL_TEMPLATE}render/{lod}/export_properties"
        ))?)?;
        properties.root.set_path(
            "source.indices",
            ConfigValue::Integer(i32::try_from(mesh.indices.len())?),
        )?;
        properties.root.set_path(
            "source.primitives",
            ConfigValue::Integer(i32::try_from(mesh.indices.len() / 3)?),
        )?;
        properties.root.set_path(
            "source.vertices",
            ConfigValue::Integer(i32::try_from(mesh.vertices.len())?),
        )?;
        set_surface_bounds(&mut properties, bounds[index])?;
        write_config_resource(
            overlay,
            &format!("{target_prefix}render/{lod}/export_properties"),
            &properties,
            "addon:batched-conifer-surface-bounds",
            files,
        )?;
        write_resource(
            overlay,
            &format!("{target_prefix}render/{lod}/converted_model"),
            &encode_static_mesh(mesh)?,
            "addon:batched-conifer-geometry",
            files,
        )?;
    }
    Ok(())
}

fn set_bounds(config: &mut BinaryConfig, bounds: ([f32; 3], [f32; 3])) -> Result<()> {
    config
        .root
        .set_path("bounding_box.min", ConfigValue::Float3(bounds.0))?;
    config
        .root
        .set_path("bounding_box.max", ConfigValue::Float3(bounds.1))?;
    Ok(())
}

fn set_surface_bounds(config: &mut BinaryConfig, bounds: ([f32; 3], [f32; 3])) -> Result<()> {
    set_bounds(config, bounds)?;
    // Grass LOD surfaces carry only a box; adding a sphere they never had is a
    // patch the config cannot take.
    if config.root.named("bounding_sphere").is_none() {
        return Ok(());
    }
    let center = std::array::from_fn(|axis| (bounds.0[axis] + bounds.1[axis]) * 0.5);
    let radius = (0..3)
        .map(|axis| (bounds.1[axis] - center[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    config
        .root
        .set_path("bounding_sphere.center", ConfigValue::Float3(center))?;
    config
        .root
        .set_path("bounding_sphere.radius", ConfigValue::Float(radius))?;
    Ok(())
}

fn write_client_project(
    db: &mut ResourceDb,
    overlay: &Path,
    grass_batches: &[GrassBatch],
    grass_count: usize,
    conifer_batches: &[ConiferBatch],
    conifer_count: usize,
    flowers: &[[f32; 3]],
    victory_items: &[[f32; 3]],
    victory_item_containers: &[[f32; 3]],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut project = BinaryConfig::parse(&db.read(CLIENT_PROJECT)?)?;
    let ConfigValue::Indexed(existing) = project
        .root
        .named("render_visuals")
        .context("client project has no render_visuals")?
    else {
        bail!("client render_visuals is not indexed");
    };
    let prototype = existing
        .first()
        .context("client project has no visual prototype")?
        .clone();
    let mut visuals = existing.clone();
    for batch in grass_batches {
        visuals.push(fixed_visual_instance(
            &prototype,
            &batch.model,
            batch.position,
            0.0,
            1.0,
        )?);
    }
    for (index, batch) in conifer_batches.iter().enumerate() {
        visuals.push(visual_instance(
            &prototype,
            &batch.model,
            batch.position,
            index + grass_count,
            1.0,
        )?);
    }
    for (index, position) in flowers.iter().enumerate() {
        visuals.push(visual_instance(
            &prototype,
            FLOWER_MODEL,
            *position,
            index + grass_count + conifer_count,
            1.8,
        )?);
    }
    for (index, position) in victory_items.iter().enumerate() {
        let mut position = *position;
        position[1] += 0.05;
        visuals.push(visual_instance(
            &prototype,
            VICTORY_ITEM_MODEL,
            position,
            index + grass_count + conifer_count + flowers.len(),
            1.25,
        )?);
    }
    for (index, position) in victory_item_containers.iter().enumerate() {
        let mut position = *position;
        position[1] += 0.02;
        visuals.push(fixed_visual_instance(
            &prototype,
            VICTORY_ITEM_CONTAINER_MODEL,
            position,
            if index == 0 {
                0.0
            } else {
                std::f32::consts::PI
            },
            1.15,
        )?);
    }
    project
        .root
        .set_path("render_visuals", ConfigValue::Indexed(visuals))?;
    write_config_resource(
        overlay,
        CLIENT_PROJECT,
        &project,
        "addon:meadow-client-project",
        files,
    )
}

fn fixed_visual_instance(
    prototype: &ConfigValue,
    model: &str,
    position: [f32; 3],
    yaw: f32,
    scale: f32,
) -> Result<ConfigValue> {
    let mut visual = prototype.clone();
    visual.set_path("position", ConfigValue::Float3(position))?;
    visual.set_path("rotation", ConfigValue::Float3([0.0, yaw, 0.0]))?;
    visual.set_path("scale", ConfigValue::Float3([scale, scale, scale]))?;
    visual.set_path("lib_name", ConfigValue::String(model.into()))?;
    Ok(visual)
}

fn visual_instance(
    prototype: &ConfigValue,
    model: &str,
    position: [f32; 3],
    seed: usize,
    base_scale: f32,
) -> Result<ConfigValue> {
    let mut visual = prototype.clone();
    let turn =
        ((seed.wrapping_mul(2_654_435_761) & 0xffff) as f32 / 65_535.0) * std::f32::consts::TAU;
    let variation = 0.85 + ((seed.wrapping_mul(4051) & 0xff) as f32 / 255.0) * 0.3;
    let scale = base_scale * variation;
    visual.set_path("position", ConfigValue::Float3(position))?;
    visual.set_path("rotation", ConfigValue::Float3([0.0, turn, 0.0]))?;
    visual.set_path("scale", ConfigValue::Float3([scale, scale, scale]))?;
    visual.set_path("lib_name", ConfigValue::String(model.into()))?;
    Ok(visual)
}

fn write_server_project(
    db: &mut ResourceDb,
    overlay: &Path,
    positions: &[[f32; 3]],
    container_positions: &[[f32; 3]],
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut project = BinaryConfig::parse(&db.read(SERVER_PROJECT)?)?;
    let objects = project
        .root
        .named_mut("objects")
        .context("server project has no objects")?;
    let ConfigValue::Indexed(existing) = objects
        .named("victory_item_spawners")
        .context("server project has no victory_item_spawners")?
    else {
        bail!("victory_item_spawners is not indexed");
    };
    let prototype = existing
        .first()
        .context("server project has no victory-item-spawner prototype")?;
    let mut generated = Vec::with_capacity(positions.len());
    for (index, position) in positions.iter().enumerate() {
        let mut spawner = prototype.clone();
        spawner.set_path("position", ConfigValue::Float3(*position))?;
        if spawner.named("point_id").is_some() {
            spawner.set_path("point_id", ConfigValue::Integer(i32::try_from(index)?))?;
        }
        if spawner.named("id").is_some() {
            spawner.set_path("id", ConfigValue::Integer(i32::try_from(index)?))?;
        }
        clear_collision_references(&mut spawner);
        generated.push(spawner);
    }
    objects.set_path("victory_item_spawners", ConfigValue::Indexed(generated))?;
    place_victory_item_containers(objects, container_positions)?;
    write_config_resource(
        overlay,
        SERVER_PROJECT,
        &project,
        "addon:victory-item-spawners-and-base-containers",
        files,
    )
}

fn place_victory_item_containers(objects: &mut ConfigValue, positions: &[[f32; 3]]) -> Result<()> {
    let ConfigValue::Indexed(existing) = objects
        .named("victory_items_containers")
        .context("server project has no victory_items_containers")?
    else {
        bail!("victory_items_containers is not indexed");
    };
    let mut containers = existing.clone();
    for container in &mut containers {
        let team = match container.named("team") {
            Some(ConfigValue::Integer(team)) => usize::try_from(*team)?,
            _ => bail!("victory-item container has no integer team"),
        };
        let position = positions.get(team).with_context(|| {
            format!(
                "server template has a team-{team} container but the source has only {} bases",
                positions.len()
            )
        })?;
        container.set_path("position", ConfigValue::Float3(*position))?;
    }
    objects.set_path("victory_items_containers", ConfigValue::Indexed(containers))?;

    // The container refers to a separately authored trigger volume. Move that
    // volume too, or deposits would register at the donor map's coordinates.
    let ConfigValue::Indexed(existing_collisions) = objects
        .named("collision_geometry")
        .context("server project has no collision_geometry")?
    else {
        bail!("collision_geometry is not indexed");
    };
    let mut collisions = existing_collisions.clone();
    let mut moved = 0;
    for collision in &mut collisions {
        let Some(ConfigValue::String(full_name)) = collision.named("full_name") else {
            continue;
        };
        let team = if full_name.contains("/team1/team_1_vic_collision") {
            Some(0)
        } else if full_name.contains("/team2/team_2_vic_collision") {
            Some(1)
        } else {
            None
        };
        if let Some(team) = team {
            let position = positions
                .get(team)
                .context("victory-item-container trigger has no matching source base")?;
            collision.set_path("position", ConfigValue::Float3(*position))?;
            moved += 1;
        }
    }
    if moved != positions.len() {
        bail!(
            "moved {moved} victory-item-container triggers for {} starting bases",
            positions.len()
        );
    }
    objects.set_path("collision_geometry", ConfigValue::Indexed(collisions))?;
    Ok(())
}

fn clear_collision_references(value: &mut ConfigValue) {
    match value {
        ConfigValue::Named(values) => {
            for child in values {
                if child.name == "collision_geometries" {
                    child.value = ConfigValue::Indexed(Vec::new());
                } else {
                    clear_collision_references(&mut child.value);
                }
            }
        }
        ConfigValue::Indexed(values) => {
            for child in values {
                clear_collision_references(child);
            }
        }
        _ => {}
    }
}

fn write_post_process(
    db: &mut ResourceDb,
    overlay: &Path,
    multiplier: f32,
    power: f32,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut material = BinaryConfig::parse(&db.read(POST_PROCESS)?)?;
    material.root.set_path(
        "material.post_process.skylight_multiplier.value",
        ConfigValue::Float(multiplier),
    )?;
    material.root.set_path(
        "material.post_process.skylight_power.value",
        ConfigValue::Float(power),
    )?;
    write_config_resource(overlay, POST_PROCESS, &material, "addon:shadow-fill", files)
}

fn write_flower_texture(
    db: &mut ResourceDb,
    png: &Path,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    if !png.is_file() {
        bail!("flower atlas does not exist: {}", png.display());
    }
    let dds_path = format!("textures/{FLOWER_TEXTURE}.dds");
    let output = safe_join(overlay, &dds_path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let status = Command::new("magick")
        .arg(png)
        .args(["-resize", "1024x1024!", "-define", "dds:compression=dxt5"])
        .arg(&output)
        .status()
        .context("starting ImageMagick for the flower atlas")?;
    if !status.success() {
        bail!("ImageMagick failed to convert the flower atlas: {status}");
    }
    let generic = fs::read(&output)?;
    let info = parse_dds(&generic, "generated flower atlas")?;
    let dds = canonicalize_dds(&generic, info, true)?;
    fs::write(&output, &dds)?;
    files.push(record_file(&dds_path, "imagegen:wildflower-atlas", &dds));

    let mut options = BinaryConfig::parse(&db.read("textures/flora/grass_l03_alt.options")?)?;
    options
        .root
        .set_path("options.format", ConfigValue::String("DXT5".into()))?;
    options.root.set_path(
        "options.has_mip_levels",
        ConfigValue::Integer(i32::from(info.mip_levels > 1)),
    )?;
    options
        .root
        .set_path("options.srgb", ConfigValue::Integer(1))?;
    write_config_resource(
        overlay,
        &format!("textures/{FLOWER_TEXTURE}.options"),
        &options,
        "addon:wildflower-texture-options",
        files,
    )
}

fn write_flower_material(
    db: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let mut material =
        BinaryConfig::parse(&db.read("material_instances/flora/grass_l03_alt.material")?)?;
    for path in [
        "material.g_stage.effect.texture_diffuse.value",
        "material.g_stage.effect.texture_roughness.value",
        "material.g_stage.effect.texture_diffuse_power.value",
    ] {
        material
            .root
            .set_path(path, ConfigValue::String(FLOWER_TEXTURE.into()))?;
    }
    write_config_resource(
        overlay,
        &format!("material_instances/{FLOWER_MATERIAL}.material"),
        &material,
        "addon:wildflower-material",
        files,
    )
}

fn copy_flower_model(
    db: &mut ResourceDb,
    overlay: &Path,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let paths: Vec<String> = db
        .paths_with_prefix(SOURCE_FLOWER_MODEL)
        .filter(|path| db.is_file(path))
        .map(ToOwned::to_owned)
        .collect();
    for source_path in paths {
        let suffix = source_path.strip_prefix(SOURCE_FLOWER_MODEL).unwrap();
        let target_path = format!("models/{FLOWER_MODEL}.model/{suffix}");
        let mut bytes = db.read(&source_path)?;
        if suffix == "settings" {
            let mut settings = BinaryConfig::parse(&bytes)?;
            for lod in ["lod_0sg", "lod_1sg"] {
                settings.root.set_path(
                    &format!("material_settings.{lod}.material_name"),
                    ConfigValue::String(FLOWER_MATERIAL.into()),
                )?;
            }
            bytes = settings.to_bytes()?;
        }
        write_resource(
            overlay,
            &target_path,
            &bytes,
            &format!("addon:flower-model-clone:{source_path}"),
            files,
        )?;
    }
    Ok(())
}

fn write_config_resource(
    overlay: &Path,
    path: &str,
    config: &BinaryConfig,
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    write_resource(overlay, path, &config.to_bytes()?, source, files)
}

fn write_resource(
    overlay: &Path,
    path: &str,
    bytes: &[u8],
    source: &str,
    files: &mut Vec<BuiltFile>,
) -> Result<()> {
    let output = safe_join(overlay, path)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes)?;
    files.push(record_file(path, source, bytes));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farthest_selection_spreads_points() {
        let candidate = |x| Candidate {
            position: [x, 0.5, 0.0],
            area: 1.0,
            headroom: f32::INFINITY,
        };
        let selected =
            farthest_positions(&[candidate(0.0), candidate(1.0), candidate(10.0)], 2).unwrap();
        assert_eq!(selected, vec![[0.0, 0.5, 0.0], [10.0, 0.5, 0.0]]);
    }

    #[test]
    fn static_grass_mesh_roundtrips_and_batches() {
        let vertex = |position: [f32; 3]| {
            let mut bytes = [0_u8; 32];
            for (axis, value) in position.into_iter().enumerate() {
                bytes[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            bytes
        };
        let source = StaticMesh {
            vertices: vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            indices: vec![0, 1, 2],
        };
        assert_eq!(
            parse_static_mesh(&encode_static_mesh(&source).unwrap()).unwrap(),
            source
        );

        let mut batch = StaticMesh::empty();
        append_static_mesh(&mut batch, &source, [5.0, 2.0, -3.0], 2.0).unwrap();
        append_static_mesh(&mut batch, &source, [9.0, 0.0, 4.0], 1.0).unwrap();
        assert_eq!(batch.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(
            mesh_bounds(&batch).unwrap(),
            ([5.0, 0.0, -3.0], [10.0, 4.0, 4.0])
        );
    }

    #[test]
    fn fence_distance_is_zero_inside_and_euclidean_at_corners() {
        assert_eq!(distance_from_fence([0.0, 0.0, 0.0]), 0.0);
        assert!((distance_from_fence([FENCE_MAX[0] + 10.0, 0.0, 0.0]) - 10.0).abs() < 0.001);
        let diagonal = 10.0 / 2.0_f32.sqrt();
        let distance = distance_from_fence([FENCE_MAX[0] + diagonal, 0.0, FENCE_MAX[1] + diagonal]);
        assert!((distance - 10.0).abs() < 0.001);
    }

    #[test]
    fn grass_region_includes_the_southern_ravine_but_not_the_outer_world() {
        assert!(inside_grass_region([20.0, 0.0, 0.0]));
        assert!(inside_grass_region([20.0, -2.0, -60.0]));
        assert!(!inside_grass_region([80.0, 0.0, -60.0]));
        assert!(!inside_grass_region([20.0, 0.0, -80.0]));
    }
}
