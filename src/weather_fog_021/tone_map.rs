// SPDX-License-Identifier: GPL-3.0-or-later
//! Original tone curve inside the pinned native postprocess shader.
use super::{assemble, bytes, chunks, instructions, pinned_code, u32_at, words};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const TONEMAP_PREFIX: &str = "shaders/sm_4_0/complex_post_process_blend.ps/";
pub const TONEMAP_SOURCE: &str = "shaders/sm_4_0/complex_post_process_blend.ps/__2_____________________00_________________________________________";
const SOURCE_HASH: &str = "5f41f44601a18b83ea3128482380ed0f63d9442b00f712487f7b75762c647aee";
const TARGET_HASHES: [&str; 2] = [
    "df08c4c6d3771c50e047c63058cfdd47a731565d09594953633508817fcfe1c6",
    "53a7ec7a0281328e5a298d83e9463fb06945af517ebd1391f04ea0ac3dc4847d",
];

/// The family's program of post-process quality 0 and 1 (six paths): the same tone block
/// and declarations, no vignette. The 0.21 adaptation and the key profiles were made for
/// the two programs above and leave this one native; an exposure profile cannot (a scene
/// of one is lit for its own exposure range: under the native block it draws ten times
/// too bright), so this program takes the exposure profiles alone.
const EXPOSURE_ONLY_HASHES: [&str; 1] = ["469fb8c0864690159b1c8e01b36671e6e4f34f9388af3d71fabf2239f19b1633"];

/// The native tone curve's exposure key (`div r, l(4.5), r` in every native program).
const NATIVE_KEY: f32 = 4.5;
/// A key profile's marker: a whole height above any camera of a map.
pub(super) const MIN_KEY_MARKER: f32 = 1000.0;

/// A scene drawn with the native tone curve at its own build's exposure key, selected by
/// its own marker: London (0.31a2's curve is 0.26g's with key 6.0 where 0.26g has 4.5,
/// every program of the family, the only difference besides the constant-buffer rows).
/// Its marker is the scene's `mie_fog_base_height`, set to a value no map has and above
/// every height a camera reaches there: every native shader reads that field only as
/// `min(1, exp((base - eye_y) * falloff))`, which is 1 for an eye below the base, as it is
/// with the authored base (London: 60, 100, 50; its cameras stay below 50 m). The 0.21
/// markers are below -32000; a key profile's marker must be at least 1000.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyProfile {
    pub marker: f32,
    pub key: f32,
}

/// A scene whose exposure follows the view as its own build's does, selected by its own
/// marker (as a key profile's): a 0.69d map. 0.69d exposes a view by its mean luminance
/// between its environment's minimum and maximum exposure values and holds the exposure
/// outside them; 0.26g's curve follows the adapted mean `L` between 0.05 and 2 only, and
/// by `(key / L)^0.2` everywhere. The profile is the native tone block with the mean
/// clamped to `floor..ceiling` before both uses (the two instructions that clamp and
/// divide change places), no `+ 1` beside `10 L` (it stops the curve following a dark
/// view) and its own exposure key:
///
/// ```text
/// Lc = clamp(L, floor, ceiling)
/// c  = (c * (k(L) / Lc)^0.2)^1.2            k(L) = 1.03 - 0.1 / (2 + log10(L + 1))
/// c  = 1 - 2^(-c * key / (10 * Lc))
/// ```
///
/// (`exp` and `log` are sm_4_0's, `2^x` and `log2`: the key is for that base.)
///
/// so the exposure is constant above the ceiling and below the floor and follows the
/// mean between them (as `Lc^-1.24`), as 0.69d's does. A floor equal to the ceiling is a
/// fixed exposure (0.69d's lobby: EV 9.5 at both ends). What the curve then shows is made
/// 0.69d's by the scene's own colour grading texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureProfile {
    pub marker: f32,
    pub floor: f32,
    pub ceiling: f32,
    pub key: f32,
    /// 0.69d's vignette, `k` = 10 x the environment's `vignette_intensity`; 0 draws none.
    /// 0.69d (`tone_mapping.ps`, `cb4[14].w`) scales the scene's light before its curve by
    /// `1 / (1 + (k (u - 0.5))^2 + (k (v - 0.5))^2)^2`, `u, v` the screen's 0..1.
    pub vignette: f32,
}

/// Only the selected maps use original tone mapping. Native bloom, texture
/// bindings, grading, gamma, vignette and unmarked tone mapping are preserved.
pub fn build_tonemap(source: &[u8], target: &[u8], markers: &[f32]) -> Result<Option<Vec<u8>>> {
    build_tonemap_profiles(source, target, markers, &[], &[])
}

/// `build_tonemap` with key and exposure profiles: after the original branches, one more
/// branch per key profile runs the native tone block with its key and one per exposure
/// profile the native block with its exposure range; then the unchanged native block.
pub fn build_tonemap_profiles(
    source: &[u8],
    target: &[u8],
    markers: &[f32],
    keys: &[KeyProfile],
    exposures: &[ExposureProfile],
) -> Result<Option<Vec<u8>>> {
    ensure!(!markers.is_empty(), "no original tone-map markers");
    let mut seen = BTreeSet::new();
    for marker in markers {
        ensure!(
            marker.is_finite() && *marker < -32000.0 && seen.insert(marker.to_bits()),
            "invalid or duplicate tone-map marker"
        );
    }
    for k in keys {
        ensure!(
            k.marker.is_finite()
                && k.marker >= MIN_KEY_MARKER
                && k.marker.fract() == 0.0
                && seen.insert(k.marker.to_bits())
                && k.key.is_finite()
                && k.key > 0.0,
            "invalid or duplicate key profile"
        );
    }
    for e in exposures {
        ensure!(
            e.marker.is_finite()
                && e.marker >= MIN_KEY_MARKER
                && e.marker.fract() == 0.0
                && seen.insert(e.marker.to_bits())
                && [e.floor, e.ceiling, e.key].iter().all(|v| v.is_finite() && *v > 0.0)
                && e.floor <= e.ceiling,
            "invalid or duplicate exposure profile"
        );
    }
    let target_hash = format!("{:x}", Sha256::digest(target));
    let exposure_only = EXPOSURE_ONLY_HASHES.contains(&target_hash.as_str());
    if exposure_only && exposures.is_empty() || !exposure_only && !TARGET_HASHES.contains(&target_hash.as_str()) {
        return Ok(None);
    }
    let (markers, keys): (&[f32], &[KeyProfile]) = if exposure_only { (&[], &[]) } else { (markers, keys) };
    let (_, original) = pinned_code(source, SOURCE_HASH)?;
    let (offset, native) = pinned_code(target, &target_hash)?;
    let original_chunks = chunks(original)?;
    let mut entries = chunks(native)?;
    let si = entries
        .iter()
        .position(|(t, _)| t == b"SHDR")
        .context("missing tone-map program")?;
    let source_ops = instructions(&words(
        &original_chunks
            .iter()
            .find(|(t, _)| t == b"SHDR")
            .context("missing original tone-map program")?
            .1,
    )?)?;
    let native_words = words(&entries[si].1)?;
    let native_ops = instructions(&native_words)?;
    let nt = native_ops
        .iter()
        .position(|i| i[0] & 2047 == 104)
        .context("missing tone-map temporaries")?;
    ensure!(
        native_ops[nt] == [0x02000068, 2] && nt < 18,
        "tone-map temporary layout changed"
    );
    ensure!(
        source_ops.len() > 47
            && native_ops.len() > 41
            && source_ops[23][0] & 2047 == 69
            && native_ops[18][0] & 2047 == 69
            && source_ops[46] == native_ops[40]
            && source_ops[47] == native_ops[41],
        "tone-map boundaries changed"
    );
    let mut prefix = native_ops[..18].to_vec();
    prefix[nt][1] = 3;
    prefix.insert(nt, vec![0x04000059, 0x00208e46, 1, 19]); // static_globals b1
    let mut updated = native_words[..2].to_vec();
    updated.extend(prefix.iter().flatten());
    for marker in markers {
        updated.extend([
            0x08000018,
            0x00100012,
            2,
            0x0020803a,
            1,
            18,
            0x00004001,
            marker.to_bits(),
            0x0304001f,
            0x0010000a,
            2,
        ]);
        updated.extend(source_ops[23..47].iter().flatten());
        updated.push(0x01000012);
    }
    for k in keys {
        updated.extend([
            0x08000018,
            0x00100012,
            2,
            0x0020803a,
            1,
            18,
            0x00004001,
            k.marker.to_bits(),
            0x0304001f,
            0x0010000a,
            2,
        ]);
        updated.extend(with_key(&native_ops[18..41], k.key)?.iter().flatten());
        updated.push(0x01000012);
    }
    for e in exposures {
        updated.extend([
            0x08000018,
            0x00100012,
            2,
            0x0020803a,
            1,
            18,
            0x00004001,
            e.marker.to_bits(),
            0x0304001f,
            0x0010000a,
            2,
        ]);
        updated.extend(with_exposure(&native_ops[18..41], e)?.iter().flatten());
        updated.push(0x01000012);
    }
    updated.extend(native_ops[18..41].iter().flatten());
    updated.extend(std::iter::repeat_n(0x01000015, markers.len() + keys.len() + exposures.len()));
    updated.extend(native_ops[41..].iter().flatten());
    updated[1] = updated.len() as u32;
    instructions(&updated)?;
    entries[si].1 = bytes(&updated);
    let r = entries
        .iter_mut()
        .find(|(t, _)| t == b"RDEF")
        .context("missing tone-map reflection")?;
    r.1 = add_marker_binding(&r.1)?;
    let mut output = target[..offset].to_vec();
    output.extend(assemble(native, &entries)?);
    Ok(Some(output))
}

/// The native tone block with its one `l(4.5)` immediate (a one-component immediate operand
/// of a `div`) set to `key`.
fn with_key(block: &[Vec<u32>], key: f32) -> Result<Vec<Vec<u32>>> {
    let mut out = block.to_vec();
    let mut found = 0;
    for op in &mut out {
        if op[0] & 2047 != 14 {
            continue;
        }
        for i in 1..op.len() - 1 {
            if op[i] == 0x00004001 && op[i + 1] == NATIVE_KEY.to_bits() {
                op[i + 1] = key.to_bits();
                found += 1;
            }
        }
    }
    ensure!(found == 1, "{found} native exposure keys in the tone block, expected one");
    Ok(out)
}

/// The native tone block as an exposure profile draws it (`ExposureProfile`). The five
/// instructions it touches are pinned token for token:
///
/// ```text
/// [1]  max r1.xy, r1.xxxx, l(0.001, 0.05, 0, 0)   0.05 -> floor
/// [8]  div r0.w, r0.w, r1.x                        after [9]: divides by the clamped mean
/// [9]  min r1.x, r1.y, l(2.0)                      2.0 -> ceiling
/// [10] mad r1.x, r1.x, l(10.0), l(1.000001)        1.000001 -> 0
/// [11] div r1.x, l(4.5), r1.x                      4.5 -> key
/// ```
fn with_exposure(block: &[Vec<u32>], e: &ExposureProfile) -> Result<Vec<Vec<u32>>> {
    const MAX: [u32; 10] = [0x0a000034, 0x00100032, 1, 0x00100006, 1, 0x00004002, 0x3a83126f, 0x3d4ccccd, 0, 0];
    const DIV: [u32; 7] = [0x0700000e, 0x00100082, 0, 0x0010003a, 0, 0x0010000a, 1];
    const MIN: [u32; 7] = [0x07000033, 0x00100012, 1, 0x0010001a, 1, 0x00004001, 0x40000000];
    const MAD: [u32; 9] = [0x09000032, 0x00100012, 1, 0x0010000a, 1, 0x00004001, 0x41200000, 0x00004001, 0x3f800008];
    const KEY: [u32; 7] = [0x0700000e, 0x00100012, 1, 0x00004001, 0x40900000, 0x0010000a, 1];
    ensure!(
        block.len() == 23 && block[1] == MAX && block[8] == DIV && block[9] == MIN && block[10] == MAD && block[11] == KEY,
        "the native tone block's exposure instructions changed"
    );
    ensure!(e.vignette.is_finite() && e.vignette >= 0.0, "invalid vignette {}", e.vignette);
    let mut out = block.to_vec();
    out[1][7] = e.floor.to_bits();
    out[9][6] = e.ceiling.to_bits();
    out[10][8] = 0f32.to_bits();
    out[11][4] = e.key.to_bits();
    out.swap(8, 9);
    if e.vignette > 0.0 {
        // Before the block (whose first instruction overwrites r1), on the light in r0:
        //   add r1.zw, v1.xxxy, l(0, 0, -0.5, -0.5)
        //   mul r1.zw, r1.zzzw, l(k)
        //   dp2 r1.z, r1.zwzz, r1.zwzz
        //   add r1.z, r1.z, l(1.0)
        //   div r1.z, l(1.0), r1.z
        //   mul r1.z, r1.z, r1.z
        //   mul r0.xyz, r0.xyzx, r1.zzzz
        let half = (-0.5f32).to_bits();
        let one = 1f32.to_bits();
        let vignette = vec![
            vec![0x0a000000, 0x001000c2, 1, 0x00101406, 1, 0x00004002, 0, 0, half, half],
            vec![0x07000038, 0x001000c2, 1, 0x00100ea6, 1, 0x00004001, e.vignette.to_bits()],
            vec![0x0700000f, 0x00100042, 1, 0x00100ae6, 1, 0x00100ae6, 1],
            vec![0x07000000, 0x00100042, 1, 0x0010002a, 1, 0x00004001, one],
            vec![0x0700000e, 0x00100042, 1, 0x00004001, one, 0x0010002a, 1],
            vec![0x07000038, 0x00100042, 1, 0x0010002a, 1, 0x0010002a, 1],
            vec![0x07000038, 0x00100072, 0, 0x00100246, 0, 0x00100aa6, 1],
        ];
        out.splice(0..0, vignette);
    }
    Ok(out)
}

fn name(data: &[u8], at: usize) -> Result<&str> {
    let tail = data.get(at..).context("missing reflection name")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated reflection name")?;
    Ok(std::str::from_utf8(&tail[..end])?)
}

#[derive(Default)]
struct Reflection {
    bytes: Vec<u8>,
    names: BTreeMap<String, u32>,
    types: BTreeMap<[u8; 16], u32>,
}
impl Reflection {
    fn put(&mut self, at: usize, values: &[u32]) {
        for (i, value) in values.iter().enumerate() {
            self.bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    fn name(&mut self, value: &str) -> u32 {
        if let Some(&at) = self.names.get(value) {
            return at;
        }
        let at = self.bytes.len() as u32;
        self.bytes.extend(value.as_bytes());
        self.bytes.push(0);
        self.names.insert(value.to_owned(), at);
        at
    }
    fn align(&mut self) {
        self.bytes.resize((self.bytes.len() + 3) & !3, 0);
    }
    fn ty(&mut self, value: [u8; 16]) -> u32 {
        if let Some(&at) = self.types.get(&value) {
            return at;
        }
        self.align();
        let at = self.bytes.len() as u32;
        self.bytes.extend(value);
        self.types.insert(value, at);
        at
    }
}

/// The tone program's reflection (nine bindings, the 22 `$Globals` of 304 bytes at b0)
/// with the marker's buffer at b1.
fn add_marker_binding(old: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        u32_at(old, 0)? == 1 && u32_at(old, 8)? == 9,
        "unexpected tone-map reflection tables"
    );
    let cb = u32_at(old, 4)? as usize;
    ensure!(
        name(old, u32_at(old, cb)? as usize)? == "$Globals"
            && u32_at(old, cb + 4)? == 22
            && u32_at(old, cb + 12)? == 304
            && u32_at(old, cb + 16)? == 0
            && u32_at(old, cb + 20)? == 0,
        "unexpected tone-map globals layout"
    );
    let (out, slot) = add_marker_buffer(old)?;
    ensure!(slot == 1, "the tone-map marker buffer is not at b1");
    Ok(out)
}

/// A program's reflection with the marker's constant buffer after the buffers it has:
/// `static_globals` (304 bytes) with the one variable `atmosphere_parameters3` at offset
/// 288, and its binding. Returns the buffer's slot too: the game binds a pixel program's
/// constant buffers by their order in the reflection (`constants_handler<1>::apply`,
/// 0x3D5A30 of the 0.26g exe: `PSSetConstantBuffers(i, 1, &table.buffers[i])`), and binds
/// each used variable by its name (`shader_constant_table::parse_constant_buffer`), so the
/// slot is the count of buffers the program had: b1 beside `$Globals`, b0 for a program
/// with no constant buffer.
///
/// The retail reflection implementation treats binding names before the
/// buffer table as their own string pool. Keep them there, not after variables.
pub(super) fn add_marker_buffer(old: &[u8]) -> Result<(Vec<u8>, u32)> {
    const MARKER_BUFFER: &str = "static_globals";
    let buffer_count = u32_at(old, 0)? as usize;
    let binding_count = u32_at(old, 8)? as usize;
    // A program has 14 constant-buffer slots.
    ensure!(buffer_count < 14 && binding_count < 128, "unexpected reflection tables");
    let cb = u32_at(old, 4)? as usize;
    let mut buffers = Vec::new();
    for b in 0..buffer_count {
        let at = cb + 24 * b;
        let header = words(old.get(at..at + 24).context("truncated buffer")?)?;
        let mut variables = Vec::new();
        for i in 0..header[1] as usize {
            let at = header[2] as usize + 24 * i;
            let v = words(old.get(at..at + 24).context("truncated variable")?)?;
            let ty: [u8; 16] = old
                .get(v[4] as usize..v[4] as usize + 16)
                .context("truncated type")?
                .try_into()?;
            ensure!(
                ty[10..] == [0; 6] && v[5] == 0,
                "unsupported structured/defaulted constant"
            );
            variables.push((name(old, v[0] as usize)?.to_owned(), v[1..4].to_vec(), ty));
        }
        buffers.push((name(old, header[0] as usize)?.to_owned(), header[3..6].to_vec(), variables));
    }
    let mut bindings = Vec::new();
    for i in 0..binding_count {
        let at = u32_at(old, 12)? as usize + 32 * i;
        let values = words(old.get(at..at + 32).context("truncated binding")?)?;
        let label = name(old, values[0] as usize)?.to_owned();
        // A constant buffer's slot is its place in the buffer table (the game's rule).
        ensure!(
            values[1] != 0 || buffers.iter().position(|b| b.0 == label) == Some(values[5] as usize),
            "a constant buffer is not bound at its place in the reflection"
        );
        bindings.push((label, values[1..].to_vec()));
    }
    ensure!(
        bindings.iter().all(|b| b.0 != MARKER_BUFFER),
        "the program already binds the marker's buffer"
    );
    let slot = buffer_count as u32;
    bindings.push((MARKER_BUFFER.into(), vec![0, 0, 0, 0, slot, 1, 0]));
    let resources = 28;
    let mut out = Reflection {
        bytes: vec![0; resources + bindings.len() * 32],
        ..Default::default()
    };
    out.bytes[..28].copy_from_slice(old.get(..28).context("truncated reflection")?);
    for (i, (name, values)) in bindings.iter().enumerate() {
        let label = out.name(name);
        out.put(resources + 32 * i, &[label]);
        out.put(resources + 32 * i + 4, values);
    }
    out.align();
    let table = out.bytes.len();
    let mut variables_at = table + 24 * (buffers.len() + 1);
    out.bytes.resize(
        variables_at + 24 * (buffers.iter().map(|b| b.2.len()).sum::<usize>() + 1),
        0,
    );
    let mut rows = Vec::new();
    for (i, (name, fields, variables)) in buffers.iter().enumerate() {
        let label = out.name(name);
        out.put(
            table + 24 * i,
            &[label, variables.len() as u32, variables_at as u32, fields[0], fields[1], fields[2]],
        );
        rows.push(variables_at);
        variables_at += 24 * variables.len();
    }
    let statics = out.name(MARKER_BUFFER);
    out.put(table + 24 * buffers.len(), &[statics, 1, variables_at as u32, 304, 0, 0]);
    for ((_, _, variables), at) in buffers.iter().zip(rows) {
        for (i, (name, fields, ty)) in variables.iter().enumerate() {
            let label = out.name(name);
            let ty = out.ty(*ty);
            out.put(at + 24 * i, &[label, fields[0], fields[1], fields[2], ty, 0]);
        }
    }
    let label = out.name("atmosphere_parameters3");
    let ty = out.ty([1, 0, 3, 0, 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.put(variables_at, &[label, 288, 16, 2, ty, 0]);
    let creator = out.name(name(old, u32_at(old, 24)? as usize)?);
    out.put(
        0,
        &[
            buffers.len() as u32 + 1,
            table as u32,
            bindings.len() as u32,
            resources as u32,
            u32_at(old, 16)?,
            u32_at(old, 20)?,
            creator,
        ],
    );
    Ok((out.bytes, slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_tone_map_markers_fail_before_shader_selection() {
        for markers in [
            vec![],
            vec![0.],
            vec![f32::NAN],
            vec![f32::NEG_INFINITY],
            vec![-32198., -32198.],
        ] {
            assert!(build_tonemap(&[], &[], &markers).is_err());
        }
        assert!(build_tonemap(&[], &[], &[-32198.]).unwrap().is_none());
        assert!(add_marker_binding(&[]).is_err());
    }

    #[test]
    fn key_profiles_are_checked_and_set_the_one_native_key() {
        for keys in [
            vec![KeyProfile { marker: 60.0, key: 6.0 }],
            vec![KeyProfile { marker: -32198.0, key: 6.0 }],
            vec![KeyProfile { marker: 32909.5, key: 6.0 }],
            vec![KeyProfile { marker: 32909.0, key: f32::NAN }],
            vec![KeyProfile { marker: 32909.0, key: 6.0 }; 2],
        ] {
            assert!(build_tonemap_profiles(&[], &[], &[-32198.], &keys, &[]).is_err());
        }
        let ok = [KeyProfile { marker: 32909.0, key: 6.0 }];
        assert!(build_tonemap_profiles(&[], &[], &[-32198.], &ok, &[]).unwrap().is_none());
        // An exposure profile: a marker of its own, a floor below its ceiling.
        let exposure = ExposureProfile { marker: 33001.0, floor: 0.001, ceiling: 0.1, key: 1.0, vignette: 0.0 };
        assert!(build_tonemap_profiles(&[], &[], &[-32198.], &ok, &[exposure]).unwrap().is_none());
        for bad in [
            ExposureProfile { marker: 32909.0, ..exposure },
            ExposureProfile { marker: 60.0, ..exposure },
            ExposureProfile { floor: 0.2, ..exposure },
            ExposureProfile { key: 0.0, ..exposure },
        ] {
            assert!(build_tonemap_profiles(&[], &[], &[-32198.], &ok, &[bad]).is_err());
        }
        let div = vec![0x0800000e, 0x00100012, 1, 0x00004001, NATIVE_KEY.to_bits(), 0x0010000a, 1];
        let block = vec![vec![0x01000012], div.clone()];
        let out = with_key(&block, 6.0).unwrap();
        assert_eq!(out[0], block[0]);
        assert_eq!(out[1][4], 6.0f32.to_bits());
        assert_eq!(out[1][..4], div[..4]);
        assert!(with_key(&[vec![0x01000012]], 6.0).is_err());
        assert!(with_key(&[div.clone(), div], 6.0).is_err());
    }

    #[test]
    #[ignore = "requires WEATHER_SHADER_AUDIT with pinned retail blobs and diagnostics31/32"]
    fn retail_tone_map_matches_guarded_diagnostic_and_preserves_fallback() {
        let root = std::path::PathBuf::from(std::env::var("WEATHER_SHADER_AUDIT").unwrap());
        let source =
            std::fs::read(root.join("original-1b4161539682733eee7ac7cb4c1d4d2a.blob")).unwrap();
        let expected = std::fs::read(root.join("trial31-guarded-tonemap.dxbc")).unwrap();
        let expected_two_maps = std::fs::read(root.join("trial32-guarded-tonemap.dxbc")).unwrap();
        for i in 0..4 {
            let target = std::fs::read(root.join(format!("trial25-native-{i}.blob"))).unwrap();
            let (offset, native) =
                pinned_code(&target, &format!("{:x}", Sha256::digest(&target))).unwrap();
            let result = build_tonemap(&source, &target, &[-32198.])
                .unwrap()
                .unwrap();
            assert_eq!(&result[..offset], &target[..offset]);
            assert_eq!(&result[offset..], expected);
            let both = build_tonemap(&source, &target, &[-32189., -32198.])
                .unwrap()
                .unwrap();
            assert_eq!(&both[offset..], expected_two_maps);
            let original_chunks = chunks(native).unwrap();
            let new_chunks = chunks(&result[offset..]).unwrap();
            for (tag, data) in &original_chunks {
                if tag != b"SHDR" && tag != b"RDEF" {
                    assert_eq!(data, &new_chunks.iter().find(|(t, _)| t == tag).unwrap().1);
                }
            }
            let r = &new_chunks.iter().find(|(t, _)| t == b"RDEF").unwrap().1;
            let buffers = u32_at(r, 4).unwrap() as usize;
            let table = u32_at(r, 12).unwrap() as usize;
            for j in 0..10 {
                let label = u32_at(r, table + 32 * j).unwrap() as usize;
                assert!((table + 320..buffers).contains(&label));
                assert!(!name(r, label).unwrap().is_empty());
            }
            let old_r = &original_chunks
                .iter()
                .find(|(t, _)| t == b"RDEF")
                .unwrap()
                .1;
            let old_table = u32_at(old_r, 12).unwrap() as usize;
            for j in 0..9 {
                let before = old_table + 32 * j;
                let after = table + 32 * j;
                assert_eq!(&old_r[before + 4..before + 32], &r[after + 4..after + 32]);
                assert_eq!(
                    name(old_r, u32_at(old_r, before).unwrap() as usize).unwrap(),
                    name(r, u32_at(r, after).unwrap() as usize).unwrap()
                );
            }
            let old_cb = u32_at(old_r, 4).unwrap() as usize;
            let old_vars = u32_at(old_r, old_cb + 8).unwrap() as usize;
            let new_vars = u32_at(r, buffers + 8).unwrap() as usize;
            for j in 0..22 {
                let before = old_vars + 24 * j;
                let after = new_vars + 24 * j;
                assert_eq!(&old_r[before + 4..before + 16], &r[after + 4..after + 16]);
                assert_eq!(
                    name(old_r, u32_at(old_r, before).unwrap() as usize).unwrap(),
                    name(r, u32_at(r, after).unwrap() as usize).unwrap()
                );
                let old_type = u32_at(old_r, before + 16).unwrap() as usize;
                let new_type = u32_at(r, after + 16).unwrap() as usize;
                assert_eq!(&old_r[old_type..old_type + 16], &r[new_type..new_type + 16]);
            }
            let old_ops = instructions(
                &words(
                    &original_chunks
                        .iter()
                        .find(|(t, _)| t == b"SHDR")
                        .unwrap()
                        .1,
                )
                .unwrap(),
            )
            .unwrap();
            for markers in [vec![-32198.], vec![-32198., -32189.]] {
                let output = build_tonemap(&source, &target, &markers).unwrap().unwrap();
                let c = chunks(&output[offset..]).unwrap();
                let ops =
                    instructions(&words(&c.iter().find(|(t, _)| t == b"SHDR").unwrap().1).unwrap())
                        .unwrap();
                let native_start = ops.iter().rposition(|op| op == &vec![0x01000012]).unwrap() + 1;
                assert_eq!(ops[native_start..native_start + 23], old_ops[18..41]);
                assert_eq!(ops[native_start + 23 + markers.len()..], old_ops[41..]);
                assert_eq!(ops.iter().filter(|op|op[0]&2047==24 && op.last()==Some(&(-32198f32).to_bits())).count(),1);
                assert_eq!(
                    super::super::checksum(&output[offset..]).unwrap(),
                    output[offset + 4..offset + 20]
                );
            }
            let mut damaged = target;
            damaged[0] ^= 1;
            assert!(
                build_tonemap(&source, &damaged, &[-32198.])
                    .unwrap()
                    .is_none()
            );
        }
        let target = std::fs::read(root.join("trial25-native-0.blob")).unwrap();
        let mut damaged = source;
        damaged[0] ^= 1;
        assert!(build_tonemap(&damaged, &target, &[-32198.]).is_err());
    }
    /// The native tone block (instructions 18..41 of both converted programs).
    fn native_block() -> Vec<Vec<u32>> {
        let text = "0c000045 001000f2 00000001 00004002 3f000000 3f000000 00000000 00000000 00107e46 00000002 00106000 00000001
0a000034 00100032 00000001 00100006 00000001 00004002 3a83126f 3d4ccccd 00000000 00000000
07000000 00100082 00000000 0010000a 00000001 00004001 3f800000
0500002f 00100082 00000000 0010003a 00000000
09000032 00100082 00000000 0010003a 00000000 00004001 3e9a209b 00004001 40000000
07000038 00100082 00000000 0010003a 00000000 00004001 41a00000
0700000e 00100082 00000000 00004001 40000000 0010003a 00000000
08000000 00100082 00000000 8010003a 00000041 00000000 00004001 3f83d70a
0700000e 00100082 00000000 0010003a 00000000 0010000a 00000001
07000033 00100012 00000001 0010001a 00000001 00004001 40000000
09000032 00100012 00000001 0010000a 00000001 00004001 41200000 00004001 3f800008
0700000e 00100012 00000001 00004001 40900000 0010000a 00000001
07000034 00100082 00000000 0010003a 00000000 00004001 38d1b717
0500002f 00100082 00000000 0010003a 00000000
07000038 00100082 00000000 0010003a 00000000 00004001 3e4ccccd
05000019 00100082 00000000 0010003a 00000000
07000038 00100072 00000000 00100246 00000000 00100ff6 00000000
0600002f 00100072 00000000 80100246 00000081 00000000
0a000038 00100072 00000000 00100246 00000000 00004002 3f99999a 3f99999a 3f99999a 00000000
05000019 00100072 00000000 00100246 00000000
08000038 00100072 00000000 00100246 00000000 80100006 00000041 00000001
05000019 00100072 00000000 00100246 00000000
0b000000 00100072 00000000 80100246 00000041 00000000 00004002 3f800000 3f800000 3f800000 00000000";
        text.lines().map(|l| l.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect()).collect()
    }

    /// The block's arithmetic on one channel: `lum` the adapted mean, `c` the colour.
    fn run(block: &[Vec<u32>], lum: f32, c: f32) -> f32 {
        let f = |w: u32| f32::from_bits(w);
        // Read back the constants and the order of the two instructions the profile moves.
        let (l_min, floor) = (f(block[1][6]), f(block[1][7]));
        let divide_first = block[8][0] & 2047 == 14;
        let ceiling = f(block[if divide_first { 9 } else { 8 }][6]);
        let mad = &block[10];
        let key = f(block[11][4]);
        let (l, lb) = (lum.max(l_min), lum.max(floor));
        let k = 1.03 - 2.0 / ((l + 1.0).log2() * 0.301_03 + 2.0) / 20.0;
        let clamped = lb.min(ceiling);
        let gain = (k / if divide_first { l } else { clamped }).max(0.0001).powf(0.2);
        let a = key / (clamped * f(mad[6]) + f(mad[8]));
        // The block's powers and its curve are sm_4_0 `log` and `exp`: log2 and 2^x.
        let opcode = |i: usize| block[i][0] & 2047;
        assert_eq!([opcode(3), opcode(13), opcode(15), opcode(17), opcode(19), opcode(21)], [0x2f, 0x2f, 0x19, 0x2f, 0x19, 0x19]);
        1.0 - (-(c * gain).powf(1.2) * a).exp2()
    }

    #[test]
    fn an_exposure_profile_follows_the_mean_between_its_floor_and_ceiling() {
        let native = native_block();
        let profile = ExposureProfile { marker: 33001.0, floor: 0.001151, ceiling: 0.104167, key: 0.3367, vignette: 0.0 };
        let block = with_exposure(&native, &profile).unwrap();
        // The same instructions but two that changed places; four constants changed.
        assert_eq!(block.len(), native.len());
        assert_eq!((&block[8], &block[9]), (&native[9][..5].iter().cloned().chain([0x00004001, profile.ceiling.to_bits()]).collect::<Vec<_>>(), &native[8]));
        let changed = (0..23).filter(|i| ![8, 9].contains(i) && block[*i] != native[*i]).collect::<Vec<_>>();
        assert_eq!(changed, [1, 10, 11]);
        // Native: a grey at the mean is shown alike from 0.1 up, and falls with the mean below.
        let shown = |b: &[Vec<u32>], lum: f32| run(b, lum, lum);
        assert!((shown(&native, 1.0) / shown(&native, 0.3) - 1.0).abs() < 0.3);
        assert!(shown(&native, 0.005) < 0.1 * shown(&native, 0.3));
        // The profile: alike between floor and ceiling, constant exposure outside them.
        let (hi, lo) = (shown(&block, 0.104167), shown(&block, 0.001151));
        assert!((hi / lo - 1.0).abs() < 0.25, "{hi} {lo}");
        assert!((shown(&block, 0.01) / hi - 1.0).abs() < 0.15);
        let exposure = |lum: f32| (-(1.0 - run(&block, lum, 0.01)).log2()).powf(1.0 / 1.2);
        // (k(L) moves the exposure by 0.2 % from the ceiling to a mean of 2.)
        assert!((exposure(2.0) / exposure(0.104167) - 1.0).abs() < 3e-3 && (exposure(0.5) / exposure(0.2) - 1.0).abs() < 3e-3);
        assert!((exposure(0.0001) / exposure(0.001151) - 1.0).abs() < 3e-3);
        // The light 0.69d's curve shows as white (6.9 at the ceiling) stays below the curve's top.
        assert!((run(&block, 0.104167, 6.9) - 0.98).abs() < 0.005);
        // Another native block is refused.
        let mut other = native.clone();
        other[9][6] = 3f32.to_bits();
        assert!(with_exposure(&other, &profile).is_err());
    }
}
