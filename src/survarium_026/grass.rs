// SPDX-License-Identifier: GPL-3.0-or-later
//! The grass 0.26 can load. A map's grass is its project's `.veg` layers;
//! the client cooks them all at once on its resource thread
//! (`grass_cook::on_layers_loaded` 0x12A820), with 16 bytes of stack an
//! instance of the whole vegetation (`_chkstk` from 0x12AC7E), on a 2 MiB
//! thread (Skhid's exe patch 128, `THREAD_STACKS` 0xE37BA). About 130,000
//! instances overflow it whatever the graphics settings: Experimental Pool's
//! 125,844 loaded with 68 KiB to spare, Dark Valley's 142,346 and Fire
//! Station's 147,551 crashed the client (`STACK_OVERFLOW`). Every map the
//! importer writes holds at most `MAX_INSTANCES`: more are thinned evenly.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};

/// Most grass instances a map gets: a margin under the ~130,000 at which the
/// cook overflows its thread.
pub const MAX_INSTANCES: usize = 120_000;

const POSITIONS: u32 = 0x58;
const NORMALS: u32 = 0x59;

/// A `.veg` file's chunks in file order: (id, payload).
fn chunks(bytes: &[u8]) -> Result<Vec<(u32, &[u8])>> {
    let mut out: Vec<(u32, &[u8])> = Vec::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        ensure!(remaining.len() >= 8, "truncated vegetation chunk header");
        let id = u32::from_le_bytes(remaining[..4].try_into().unwrap());
        let size = u32::from_le_bytes(remaining[4..8].try_into().unwrap()) as usize;
        ensure!(size <= remaining.len() - 8, "truncated vegetation chunk");
        ensure!(out.iter().all(|(seen, _)| *seen != id), "duplicate vegetation chunk");
        out.push((id, &remaining[8..8 + size]));
        remaining = &remaining[8 + size..];
    }
    Ok(out)
}

/// The instances of a `.veg` file (its positions, 12 bytes each).
pub fn instances(bytes: &[u8]) -> Result<usize> {
    let chunks = chunks(bytes)?;
    let get = |id| chunks.iter().find(|(c, _)| *c == id).map(|(_, p)| *p);
    let positions = get(POSITIONS).context("missing vegetation positions")?;
    let normals = get(NORMALS).context("missing vegetation normals")?;
    ensure!(
        positions.len() % 12 == 0 && positions.len() == normals.len(),
        "vegetation position/normal count mismatch"
    );
    for values in [positions, normals] {
        ensure!(
            values.chunks_exact(4).all(|v| f32::from_le_bytes(v.try_into().unwrap()).is_finite()),
            "nonfinite vegetation coordinates"
        );
    }
    Ok(positions.len() / 12)
}

/// Where each `.veg` file stands in its map's grass: the map is the project
/// folder its `vegetation/` folder is in, its layers in path order.
#[derive(Debug, Default)]
pub struct Plan {
    /// path -> (instances before it in its map, the map's instances)
    layers: BTreeMap<String, (usize, usize)>,
}

impl Plan {
    /// `files`: every `.veg` path the maps import with its instances.
    pub fn new<'a>(files: impl IntoIterator<Item = (&'a str, usize)>) -> Result<Self> {
        let mut maps: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
        for (path, count) in files {
            let map = path
                .rsplit_once("/vegetation/")
                .map(|(map, _)| map)
                .with_context(|| format!("{path}: a vegetation layer outside a vegetation folder"))?;
            maps.entry(map).or_default().push((path, count));
        }
        let mut layers = BTreeMap::new();
        for mut files in maps.into_values() {
            files.sort();
            let total = files.iter().map(|(_, n)| n).sum();
            let mut before = 0;
            for (path, count) in files {
                layers.insert(path.to_owned(), (before, total));
                before += count;
            }
        }
        Ok(Self { layers })
    }

    /// The instances of the map a layer is in, and how many it keeps.
    pub fn map_of(&self, path: &str) -> Option<(usize, usize)> {
        self.layers.get(path).map(|&(_, total)| (total, total.min(MAX_INSTANCES)))
    }

    /// The layer as the map writes it: thinned when its map has more than
    /// `MAX_INSTANCES`, as it is otherwise.
    pub fn layer(&self, path: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let &(before, total) = self.layers.get(path).with_context(|| format!("{path}: not a planned layer"))?;
        if total <= MAX_INSTANCES {
            return Ok(bytes.to_vec());
        }
        thin(bytes, before, total, MAX_INSTANCES)
    }
}

/// Instance `g` of a map's `n` (its layers one after another) stays when
/// `g * max / n` steps: exactly `max` stay, thinned the same everywhere and
/// in every layer.
fn keeps(g: usize, n: usize, max: usize) -> bool {
    let (g, n, max) = (g as u64, n as u64, max as u64);
    (g + 1) * max / n != g * max / n
}

/// A layer of a map of `n` instances thinned to the map's `max`; `before`:
/// the map's instances in the layers before it.
fn thin(bytes: &[u8], before: usize, n: usize, max: usize) -> Result<Vec<u8>> {
    let count = instances(bytes)?;
    let mut out = Vec::with_capacity(bytes.len());
    for (id, payload) in chunks(bytes)? {
        let payload: Vec<u8> = if id == POSITIONS || id == NORMALS {
            payload
                .chunks_exact(12)
                .enumerate()
                .filter(|&(i, _)| keeps(before + i, n, max))
                .flat_map(|(_, v)| v.iter().copied())
                .collect()
        } else {
            payload.to_vec()
        };
        out.extend(id.to_le_bytes());
        out.extend(u32::try_from(payload.len())?.to_le_bytes());
        out.extend(payload);
    }
    ensure!(instances(&out)? <= count, "thinning added vegetation instances");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn veg(xs: impl IntoIterator<Item = f32>) -> Vec<u8> {
        let xs: Vec<f32> = xs.into_iter().collect();
        let mut out = Vec::new();
        for (id, normal) in [(POSITIONS, false), (NORMALS, true)] {
            out.extend(id.to_le_bytes());
            out.extend(u32::try_from(xs.len() * 12).unwrap().to_le_bytes());
            for &x in &xs {
                for c in if normal { [0.0, 1.0, 0.0] } else { [x, 0.0, 0.0] } {
                    out.extend(c.to_le_bytes());
                }
            }
        }
        out
    }

    fn xs(bytes: &[u8]) -> Vec<f32> {
        let positions = chunks(bytes).unwrap()[0].1.to_vec();
        positions.chunks_exact(12).map(|v| f32::from_le_bytes(v[..4].try_into().unwrap())).collect()
    }

    #[test]
    fn vegetation_keeps_authored_positions_and_rejects_truncation() {
        let mut bytes = Vec::new();
        for (id, values) in [
            (0x4du32, vec![]),
            (0x58, vec![12f32, 2., -9.]),
            (0x59, vec![0., 1., 0.]),
        ] {
            bytes.extend(id.to_le_bytes());
            bytes.extend(((values.len() * 4) as u32).to_le_bytes());
            for v in values {
                bytes.extend(v.to_le_bytes());
            }
        }
        assert_eq!(instances(&bytes).unwrap(), 1);
        assert!(instances(&bytes[..bytes.len() - 1]).is_err());
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(instances(&bytes).is_err());
    }

    #[test]
    fn a_map_under_the_cap_keeps_every_layer_byte_for_byte() {
        let a = veg((0..100).map(|i| i as f32));
        let plan = Plan::new([("projects/p/day_4/vegetation/a.veg", 100)]).unwrap();
        assert_eq!(plan.layer("projects/p/day_4/vegetation/a.veg", &a).unwrap(), a);
        assert_eq!(plan.map_of("projects/p/day_4/vegetation/a.veg"), Some((100, 100)));
    }

    #[test]
    fn a_map_over_the_cap_keeps_exactly_the_cap_evenly_across_its_layers() {
        let (n_a, n_b) = (100_000, 42_346);
        let a = veg((0..n_a).map(|i| i as f32));
        let b = veg((n_a..n_a + n_b).map(|i| i as f32));
        let plan = Plan::new([
            ("projects/p/vegetation/b.veg", n_b),
            ("projects/p/vegetation/a.veg", n_a),
            ("projects/q/vegetation/a.veg", 10),
        ])
        .unwrap();
        let a = xs(&plan.layer("projects/p/vegetation/a.veg", &a).unwrap());
        let b = xs(&plan.layer("projects/p/vegetation/b.veg", &b).unwrap());
        assert_eq!(a.len() + b.len(), MAX_INSTANCES);
        // Each layer thinned by the map's share; nothing dropped two in a row.
        assert!((a.len() as f64 / n_a as f64 - b.len() as f64 / n_b as f64).abs() < 1e-3);
        let all: Vec<f32> = a.into_iter().chain(b).collect();
        assert!(all.windows(2).all(|w| w[1] - w[0] <= 2.0));
        assert_eq!(plan.map_of("projects/q/vegetation/a.veg"), Some((10, 10)));
        assert_eq!(plan.map_of("projects/p/vegetation/a.veg"), Some((n_a + n_b, MAX_INSTANCES)));
    }
}
