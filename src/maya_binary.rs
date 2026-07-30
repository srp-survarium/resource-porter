use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::ResourceDb;

const GROUP_TAGS: [&[u8; 4]; 12] = [
    b"FORM", b"FOR4", b"FOR8", b"CAT ", b"CAT4", b"CAT8", b"LIST", b"LIS4", b"LIS8", b"PROP",
    b"PRO4", b"PRO8",
];

#[derive(Clone, Debug)]
struct Chunk<'a> {
    tag: [u8; 4],
    offset: usize,
    data: &'a [u8],
}

#[derive(Debug)]
pub struct MayaSkin {
    pub influence_names: Vec<String>,
    /// Maya skinCluster bindPreMatrix values, indexed like `influence_names`.
    pub bind_pre_matrices: Vec<[[f64; 4]; 4]>,
    pub weights: Vec<Vec<(usize, f64)>>,
    pub positions: Vec<[f32; 3]>,
}

pub fn inspect_skin(path: &Path, static_model: Option<&Path>) -> Result<MayaSkin> {
    let skin = read_skin(path)?;
    let influence_names = &skin.influence_names;
    let weights = &skin.weights;
    let positions = &skin.positions;

    println!("influences={}", influence_names.len());
    for (index, name) in influence_names.iter().enumerate() {
        println!("influence[{index}]={name}");
    }
    for (index, matrix) in skin.bind_pre_matrices.iter().enumerate() {
        let bind = inverse_affine_f64(*matrix)
            .with_context(|| format!("inverting bind-pre-matrix for {}", influence_names[index]))?;
        let origin = [-bind[3][0] * 0.01, bind[3][1] * 0.01, bind[3][2] * 0.01];
        println!("bind_origin[{}]={origin:?}", influence_names[index]);
    }
    println!("weighted_vertices={}", weights.len());
    let assigned = weights.iter().filter(|weights| !weights.is_empty()).count();
    let max_weights = weights.iter().map(Vec::len).max().unwrap_or(0);
    let min_sum = weights
        .iter()
        .filter(|weights| !weights.is_empty())
        .map(|weights| weights.iter().map(|(_, value)| value).sum::<f64>())
        .fold(f64::INFINITY, f64::min);
    let max_sum = weights
        .iter()
        .filter(|weights| !weights.is_empty())
        .map(|weights| weights.iter().map(|(_, value)| value).sum::<f64>())
        .fold(f64::NEG_INFINITY, f64::max);
    println!("assigned_vertices={assigned}");
    println!("max_weights_per_vertex={max_weights}");
    println!("weight_sum_range={min_sum:.9}..{max_sum:.9}");
    print_bounds("maya_mesh_cm", &positions);
    print_skin_landmarks(&skin);
    print_weight_compression(&skin);
    print_position_ambiguity(&skin);

    if let Some(static_model) = static_model {
        compare_static_model(static_model, &positions)?;
    }

    Ok(skin)
}

fn print_weight_compression(skin: &MayaSkin) {
    let mut discarded = Vec::with_capacity(skin.weights.len());
    for (vertex, source_weights) in skin.weights.iter().enumerate() {
        let mut sorted = source_weights.clone();
        sorted.sort_by(|left, right| right.1.total_cmp(&left.1));
        let discarded_weight: f64 = sorted.iter().skip(4).map(|(_, weight)| weight).sum();
        discarded.push((discarded_weight, vertex, sorted));
    }
    discarded.sort_by(|left, right| right.0.total_cmp(&left.0));

    for threshold in [0.001, 0.01, 0.05, 0.1] {
        let count = discarded
            .iter()
            .filter(|(weight, _, _)| *weight >= threshold)
            .count();
        println!("vertices_discarding_at_least_{threshold:.3}={count}");
    }
    for (rank, (weight, vertex, sorted)) in discarded.iter().take(20).enumerate() {
        let position = skin.positions[*vertex];
        let influences = sorted
            .iter()
            .map(|(influence, weight)| format!("{}:{weight:.6}", skin.influence_names[*influence]))
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "weight_compression_worst[{rank}]=vertex:{vertex} discarded:{weight:.6} position_cm:{position:?} weights:[{influences}]"
        );
    }
}

fn print_skin_landmarks(skin: &MayaSkin) {
    for (influence, name) in skin.influence_names.iter().enumerate() {
        let mut total = 0.0_f64;
        let mut centroid = [0.0_f64; 3];
        for (vertex, weights) in skin.weights.iter().enumerate() {
            let weight: f64 = weights
                .iter()
                .filter(|(index, _)| *index == influence)
                .map(|(_, value)| value)
                .sum();
            total += weight;
            centroid[0] += f64::from(skin.positions[vertex][0]) * 0.01 * weight;
            centroid[1] += f64::from(skin.positions[vertex][1]) * 0.01 * weight;
            centroid[2] -= f64::from(skin.positions[vertex][2]) * 0.01 * weight;
        }
        if total > f64::EPSILON {
            for value in &mut centroid {
                *value /= total;
            }
            println!("influence_landmark[{name}]={centroid:?} total_weight={total:.6}");
        }
    }
}

fn print_position_ambiguity(skin: &MayaSkin) {
    let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (index, position) in skin.positions.iter().enumerate() {
        let transformed = [position[0] * 0.01, position[1] * 0.01, -position[2] * 0.01];
        groups
            .entry(transformed.map(|value| (value * 1_000_000.0).round() as i32))
            .or_default()
            .push(index);
    }
    let duplicate_groups = groups.values().filter(|group| group.len() > 1).count();
    let ambiguous_groups = groups
        .values()
        .filter(|group| {
            group.len() > 1
                && group[1..].iter().any(|index| {
                    !equivalent_weights(&skin.weights[group[0]], &skin.weights[*index])
                })
        })
        .count();
    println!("duplicate_position_groups={duplicate_groups}");
    println!("ambiguous_weight_position_groups={ambiguous_groups}");
}

fn equivalent_weights(left: &[(usize, f64)], right: &[(usize, f64)]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.0 == right.0 && (left.1 - right.1).abs() < 1.0e-8)
}

pub fn read_skin(path: &Path) -> Result<MayaSkin> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let chunks = parse_file(&bytes)?;
    let influence_names = parse_influences(&chunks)?;
    let bind_pre_matrices = parse_bind_pre_matrices(&chunks, influence_names.len())?;
    let weights = parse_weights(&chunks, influence_names.len())?;
    let positions = parse_mesh_positions(&chunks)?;
    if positions.len() != weights.len() {
        bail!(
            "Maya mesh has {} positions but the skin cluster has {} weight rows",
            positions.len(),
            weights.len()
        );
    }
    Ok(MayaSkin {
        influence_names,
        bind_pre_matrices,
        weights,
        positions,
    })
}

impl MayaSkin {
    pub fn match_static_positions(&self, positions: &[[f32; 3]]) -> Result<Vec<usize>> {
        let transformed: Vec<[f32; 3]> = self
            .positions
            .iter()
            .map(|position| [position[0] * 0.01, position[1] * 0.01, -position[2] * 0.01])
            .collect();
        let mut cells: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        for (index, position) in transformed.iter().enumerate() {
            cells
                .entry(floored_position(*position))
                .or_default()
                .push(index);
        }

        let mut result = Vec::with_capacity(positions.len());
        for (vertex, position) in positions.iter().enumerate() {
            let cell = floored_position(*position);
            let mut best = None;
            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        let neighbour = [cell[0] + x, cell[1] + y, cell[2] + z];
                        for &source in cells.get(&neighbour).into_iter().flatten() {
                            let delta = [
                                position[0] - transformed[source][0],
                                position[1] - transformed[source][1],
                                position[2] - transformed[source][2],
                            ];
                            let distance_squared =
                                delta.into_iter().map(|value| value * value).sum::<f32>();
                            if best.is_none_or(|(_, distance)| distance_squared < distance) {
                                best = Some((source, distance_squared));
                            }
                        }
                    }
                }
            }
            let (source, distance_squared) =
                best.with_context(|| format!("static vertex {vertex} has no nearby Maya vertex"))?;
            if distance_squared.sqrt() > 0.00001 {
                bail!(
                    "static vertex {vertex} is {:.9} m from its nearest Maya skin vertex",
                    distance_squared.sqrt()
                );
            }
            result.push(source);
        }
        Ok(result)
    }
}

fn parse_file(bytes: &[u8]) -> Result<Vec<Chunk<'_>>> {
    let mut chunks = Vec::new();
    parse_range(bytes, 0, bytes.len(), 2, &mut Vec::new(), &mut chunks)?;
    Ok(chunks)
}

fn parse_range<'a>(
    bytes: &'a [u8],
    mut offset: usize,
    end: usize,
    alignment: usize,
    group_path: &mut Vec<[u8; 4]>,
    chunks: &mut Vec<Chunk<'a>>,
) -> Result<()> {
    while offset < end {
        if end - offset < 8 {
            if bytes[offset..end].iter().all(|byte| *byte == 0) {
                return Ok(());
            }
            bail!("truncated Maya IFF header at 0x{offset:x}");
        }
        let tag: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
        let size = usize::try_from(u32::from_be_bytes(
            bytes[offset + 4..offset + 8].try_into().unwrap(),
        ))?;
        let payload = offset
            .checked_add(8)
            .context("Maya IFF payload offset overflow")?;
        let payload_end = payload
            .checked_add(size)
            .context("Maya IFF chunk size overflow")?;
        if payload_end > end || payload_end > bytes.len() {
            bail!(
                "Maya IFF chunk {} at 0x{offset:x} ends outside its group",
                tag_text(tag)
            );
        }

        if GROUP_TAGS.contains(&&tag) {
            if size < 4 {
                bail!("Maya IFF group at 0x{offset:x} has no group tag");
            }
            let group_tag: [u8; 4] = bytes[payload..payload + 4].try_into().unwrap();
            let child_alignment = match tag[3] {
                b'4' => 4,
                b'8' => 8,
                _ => 2,
            };
            group_path.push(group_tag);
            parse_range(
                bytes,
                payload + 4,
                payload_end,
                child_alignment,
                group_path,
                chunks,
            )?;
            group_path.pop();
        } else {
            chunks.push(Chunk {
                tag,
                offset: payload,
                data: &bytes[payload..payload_end],
            });
        }

        offset = align_up(payload_end, alignment)?;
        if offset > end {
            bail!("Maya IFF padding at 0x{payload_end:x} exceeds its group");
        }
    }
    Ok(())
}

fn parse_influences(chunks: &[Chunk<'_>]) -> Result<Vec<String>> {
    let mut influences = BTreeMap::new();
    for chunk in chunks.iter().filter(|chunk| chunk.tag == *b"CWFL") {
        let strings = printable_strings(chunk.data);
        let Some((destination_pos, destination)) = strings
            .iter()
            .enumerate()
            .find(|(_, value)| value.starts_with("skinCluster2.ma["))
        else {
            continue;
        };
        let index = destination
            .strip_prefix("skinCluster2.ma[")
            .and_then(|value| value.strip_suffix(']'))
            .context("malformed skinCluster2 matrix connection")?
            .parse::<usize>()?;
        let source = strings[..destination_pos]
            .iter()
            .rev()
            .find_map(|value| value.strip_suffix(".wm"))
            .with_context(|| {
                format!(
                    "skinCluster2 matrix connection {index} at 0x{:x} has no joint source",
                    chunk.offset
                )
            })?;
        if let Some(previous) = influences.insert(index, source.to_owned()) {
            bail!("duplicate skin influence {index}: {previous} and {source}");
        }
    }
    let count = influences
        .last_key_value()
        .map(|(index, _)| index + 1)
        .context("no skinCluster2 matrix connections found")?;
    if influences.len() != count {
        bail!(
            "skin influence indices are sparse: found {} entries through index {}",
            influences.len(),
            count - 1
        );
    }
    Ok((0..count).map(|index| influences[&index].clone()).collect())
}

fn parse_weights(chunks: &[Chunk<'_>], influence_count: usize) -> Result<Vec<Vec<(usize, f64)>>> {
    let mut by_vertex: BTreeMap<usize, Vec<(usize, f64)>> = BTreeMap::new();
    for chunk in chunks.iter().filter(|chunk| chunk.tag == *b"DBLE") {
        let Some(attribute) = first_c_string(chunk.data) else {
            continue;
        };
        let Some(rest) = attribute.strip_prefix("wl[") else {
            continue;
        };
        let Some((vertex, influence)) = rest.split_once("].w[") else {
            continue;
        };
        let vertex = vertex.parse::<usize>()?;
        let influence = influence
            .strip_suffix(']')
            .context("unterminated Maya skin weight attribute")?;
        let (first, last) = if let Some((first, last)) = influence.split_once(':') {
            (first.parse::<usize>()?, last.parse::<usize>()?)
        } else {
            let index = influence.parse::<usize>()?;
            (index, index)
        };
        if last < first || last >= influence_count {
            bail!("invalid influence range {first}:{last} for vertex {vertex}");
        }
        let value_count = last - first + 1;
        let value_bytes = value_count
            .checked_mul(8)
            .context("Maya skin weight byte count overflow")?;
        let values = chunk
            .data
            .get(chunk.data.len().saturating_sub(value_bytes)..)
            .with_context(|| format!("weight chunk at 0x{:x} is too short", chunk.offset))?;
        let vertex_weights = by_vertex.entry(vertex).or_default();
        for (slot, value) in values.chunks_exact(8).enumerate() {
            let value = f64::from_be_bytes(value.try_into().unwrap());
            if !value.is_finite() || value < 0.0 {
                bail!("invalid skin weight {value} for vertex {vertex}");
            }
            vertex_weights.push((first + slot, value));
        }
    }
    let count = by_vertex
        .last_key_value()
        .map(|(index, _)| index + 1)
        .context("no skinCluster2 weights found")?;
    let mut weights = vec![Vec::new(); count];
    for (vertex, mut vertex_weights) in by_vertex {
        vertex_weights.sort_by_key(|(influence, _)| *influence);
        weights[vertex] = vertex_weights;
    }
    Ok(weights)
}

fn parse_bind_pre_matrices(
    chunks: &[Chunk<'_>],
    influence_count: usize,
) -> Result<Vec<[[f64; 4]; 4]>> {
    let mut matrices = BTreeMap::new();
    for chunk in chunks.iter().filter(|chunk| chunk.tag == *b"MATR") {
        let Some(attribute) = first_c_string(chunk.data) else {
            continue;
        };
        let Some(index) = attribute
            .strip_prefix("pm[")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        if index >= influence_count {
            bail!("bind-pre-matrix index {index} exceeds {influence_count} influences");
        }
        let values = chunk
            .data
            .get(chunk.data.len().saturating_sub(16 * 8)..)
            .with_context(|| {
                format!("bind-pre-matrix chunk at 0x{:x} is too short", chunk.offset)
            })?;
        let mut matrix = [[0.0_f64; 4]; 4];
        for (slot, value) in values.chunks_exact(8).enumerate() {
            matrix[slot / 4][slot % 4] = f64::from_be_bytes(value.try_into().unwrap());
        }
        if matrix.iter().flatten().any(|value| !value.is_finite()) {
            bail!("bind-pre-matrix {index} contains non-finite values");
        }
        if matrices.insert(index, matrix).is_some() {
            bail!("duplicate bind-pre-matrix {index}");
        }
    }
    if matrices.len() != influence_count {
        bail!(
            "found {} bind-pre-matrices for {influence_count} influences",
            matrices.len()
        );
    }
    Ok((0..influence_count).map(|index| matrices[&index]).collect())
}

fn parse_mesh_positions(chunks: &[Chunk<'_>]) -> Result<Vec<[f32; 3]>> {
    let chunk = chunks
        .iter()
        .find(|chunk| chunk.tag == *b"MESH" && first_c_string(chunk.data) == Some("o"))
        .context("no Maya mesh payload named o found")?;
    let attribute_end = chunk
        .data
        .iter()
        .position(|byte| *byte == 0)
        .context("Maya mesh attribute is unterminated")?;
    let count_offset = attribute_end + 2;
    let scalar_count = usize::try_from(read_be_u32(chunk.data, count_offset)?)?;
    if scalar_count % 3 != 0 {
        bail!("Maya mesh position scalar count {scalar_count} is not divisible by three");
    }
    let values_offset = count_offset + 4;
    let values_bytes = scalar_count
        .checked_mul(4)
        .context("Maya mesh position byte count overflow")?;
    let values = chunk
        .data
        .get(values_offset..values_offset + values_bytes)
        .context("Maya mesh position payload is truncated")?;
    let mut positions = Vec::with_capacity(scalar_count / 3);
    for xyz in values.chunks_exact(12) {
        positions.push([
            f32::from_be_bytes(xyz[0..4].try_into().unwrap()),
            f32::from_be_bytes(xyz[4..8].try_into().unwrap()),
            f32::from_be_bytes(xyz[8..12].try_into().unwrap()),
        ]);
    }
    if positions.iter().flatten().any(|value| !value.is_finite()) {
        bail!("Maya mesh contains non-finite positions");
    }
    Ok(positions)
}

fn compare_static_model(path: &Path, maya_positions: &[[f32; 3]]) -> Result<()> {
    let mut archive = ResourceDb::open(path)?;
    let surfaces: Vec<String> = archive
        .paths_with_prefix("render/")
        .filter_map(|path| {
            path.strip_prefix("render/")
                .and_then(|path| path.strip_suffix("/vertices"))
                .map(ToOwned::to_owned)
        })
        .collect();
    let mut static_positions = Vec::new();
    let mut surface_ranges = Vec::new();
    for surface in &surfaces {
        let first = static_positions.len();
        let vertices = archive.read(&format!("render/{surface}/vertices"))?;
        let count = usize::try_from(read_le_u32(&vertices, 0)?)?;
        let expected = 4 + count * 56;
        if vertices.len() != expected {
            bail!(
                "static surface {surface} has {} bytes, expected {expected} for {count} vertices",
                vertices.len()
            );
        }
        for index in 0..count {
            let offset = 4 + index * 56;
            static_positions.push([
                read_le_f32(&vertices, offset)?,
                read_le_f32(&vertices, offset + 4)?,
                read_le_f32(&vertices, offset + 8)?,
            ]);
        }
        surface_ranges.push((surface.clone(), first..static_positions.len()));
        println!("static_surface[{surface}]={count}");
    }
    print_bounds("static_model_m", &static_positions);

    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut best = (0_usize, [0, 1, 2], [1.0_f32; 3]);
    for permutation in permutations {
        for sign_bits in 0_u8..8 {
            let signs = [
                if sign_bits & 1 == 0 { 1.0 } else { -1.0 },
                if sign_bits & 2 == 0 { 1.0 } else { -1.0 },
                if sign_bits & 4 == 0 { 1.0 } else { -1.0 },
            ];
            let source: HashMap<[i32; 3], usize> = maya_positions
                .iter()
                .enumerate()
                .map(|(index, position)| {
                    (
                        quantized_position([
                            position[permutation[0]] * signs[0] * 0.01,
                            position[permutation[1]] * signs[1] * 0.01,
                            position[permutation[2]] * signs[2] * 0.01,
                        ]),
                        index,
                    )
                })
                .collect();
            let matches = static_positions
                .iter()
                .filter(|position| source.contains_key(&quantized_position(**position)))
                .count();
            if matches > best.0 {
                best = (matches, permutation, signs);
            }
        }
    }
    println!(
        "static_position_match={}/{} permutation={:?} signs={:?} scale=0.01",
        best.0,
        static_positions.len(),
        best.1,
        best.2
    );
    let transformed: Vec<[f32; 3]> = maya_positions
        .iter()
        .map(|position| {
            [
                position[best.1[0]] * best.2[0] * 0.01,
                position[best.1[1]] * best.2[1] * 0.01,
                position[best.1[2]] * best.2[2] * 0.01,
            ]
        })
        .collect();
    let exact: HashMap<[i32; 3], usize> = transformed
        .iter()
        .enumerate()
        .map(|(index, position)| (quantized_position(*position), index))
        .collect();
    for (surface, range) in surface_ranges {
        let matched = static_positions[range.clone()]
            .iter()
            .filter(|position| exact.contains_key(&quantized_position(**position)))
            .count();
        println!("static_surface_match[{surface}]={matched}/{}", range.len());
    }
    let mut nearest_distances: Vec<f32> = static_positions
        .iter()
        .filter(|position| !exact.contains_key(&quantized_position(**position)))
        .map(|position| {
            transformed
                .iter()
                .map(|source| {
                    let delta = [
                        position[0] - source[0],
                        position[1] - source[1],
                        position[2] - source[2],
                    ];
                    delta
                        .into_iter()
                        .map(|value| value * value)
                        .sum::<f32>()
                        .sqrt()
                })
                .fold(f32::INFINITY, f32::min)
        })
        .collect();
    nearest_distances.sort_by(f32::total_cmp);
    if !nearest_distances.is_empty() {
        println!(
            "unmatched_nearest_distance_median={:.9} p95={:.9} max={:.9}",
            nearest_distances[nearest_distances.len() / 2],
            nearest_distances[nearest_distances.len() * 95 / 100],
            nearest_distances[nearest_distances.len() - 1]
        );
    }
    Ok(())
}

fn print_bounds(label: &str, positions: &[[f32; 3]]) {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(position[axis]);
            maximum[axis] = maximum[axis].max(position[axis]);
        }
    }
    println!("{label}_vertices={}", positions.len());
    println!("{label}_bounds={minimum:?}..{maximum:?}");
}

fn quantized_position(position: [f32; 3]) -> [i32; 3] {
    position.map(|value| (value * 100_000.0).round() as i32)
}

fn floored_position(position: [f32; 3]) -> [i32; 3] {
    position.map(|value| (value * 100_000.0).floor() as i32)
}

fn printable_strings(data: &[u8]) -> Vec<String> {
    data.split(|byte| *byte == 0)
        .filter_map(|part| {
            let start = part.iter().position(|byte| byte.is_ascii_graphic())?;
            let value = &part[start..];
            (value.len() >= 2 && value.iter().all(u8::is_ascii_graphic))
                .then(|| String::from_utf8_lossy(value).into_owned())
        })
        .collect()
}

fn first_c_string(data: &[u8]) -> Option<&str> {
    let end = data.iter().position(|byte| *byte == 0)?;
    std::str::from_utf8(&data[..end]).ok()
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .context("Maya IFF alignment overflow")
}

fn tag_text(tag: [u8; 4]) -> String {
    String::from_utf8_lossy(&tag).into_owned()
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .context("big-endian u32 is truncated")?
            .try_into()
            .unwrap(),
    ))
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("little-endian u32 is truncated")?
            .try_into()
            .unwrap(),
    ))
}

fn read_le_f32(bytes: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("little-endian float is truncated")?
            .try_into()
            .unwrap(),
    ))
}

fn inverse_affine_f64(matrix: [[f64; 4]; 4]) -> Result<[[f64; 4]; 4]> {
    let determinant = matrix[0][0] * (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
        - matrix[0][1] * (matrix[1][0] * matrix[2][2] - matrix[1][2] * matrix[2][0])
        + matrix[0][2] * (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]);
    if determinant.abs() <= 1.0e-12 {
        bail!("affine matrix is singular");
    }
    let reciprocal = determinant.recip();
    let inverse_basis = [
        [
            (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1]) * reciprocal,
            (matrix[0][2] * matrix[2][1] - matrix[0][1] * matrix[2][2]) * reciprocal,
            (matrix[0][1] * matrix[1][2] - matrix[0][2] * matrix[1][1]) * reciprocal,
        ],
        [
            (matrix[1][2] * matrix[2][0] - matrix[1][0] * matrix[2][2]) * reciprocal,
            (matrix[0][0] * matrix[2][2] - matrix[0][2] * matrix[2][0]) * reciprocal,
            (matrix[0][2] * matrix[1][0] - matrix[0][0] * matrix[1][2]) * reciprocal,
        ],
        [
            (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]) * reciprocal,
            (matrix[0][1] * matrix[2][0] - matrix[0][0] * matrix[2][1]) * reciprocal,
            (matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0]) * reciprocal,
        ],
    ];
    let translation = [matrix[3][0], matrix[3][1], matrix[3][2]];
    let inverse_translation = [
        -(translation[0] * inverse_basis[0][0]
            + translation[1] * inverse_basis[1][0]
            + translation[2] * inverse_basis[2][0]),
        -(translation[0] * inverse_basis[0][1]
            + translation[1] * inverse_basis[1][1]
            + translation[2] * inverse_basis[2][1]),
        -(translation[0] * inverse_basis[0][2]
            + translation[1] * inverse_basis[1][2]
            + translation[2] * inverse_basis[2][2]),
    ];
    Ok([
        [
            inverse_basis[0][0],
            inverse_basis[0][1],
            inverse_basis[0][2],
            0.0,
        ],
        [
            inverse_basis[1][0],
            inverse_basis[1][1],
            inverse_basis[1][2],
            0.0,
        ],
        [
            inverse_basis[2][0],
            inverse_basis[2][1],
            inverse_basis[2][2],
            0.0,
        ],
        [
            inverse_translation[0],
            inverse_translation[1],
            inverse_translation[2],
            1.0,
        ],
    ])
}
