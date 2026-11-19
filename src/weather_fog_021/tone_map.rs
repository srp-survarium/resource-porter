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

/// The native tone curve's exposure key (`div r, l(4.5), r` in every native program).
const NATIVE_KEY: f32 = 4.5;
/// A key profile's marker: a whole height above any camera of a map.
const MIN_KEY_MARKER: f32 = 1000.0;

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

/// Only the selected maps use original tone mapping. Native bloom, texture
/// bindings, grading, gamma, vignette and unmarked tone mapping are preserved.
pub fn build_tonemap(source: &[u8], target: &[u8], markers: &[f32]) -> Result<Option<Vec<u8>>> {
    build_tonemap_profiles(source, target, markers, &[])
}

/// `build_tonemap` with key profiles: after the original branches, one more branch per
/// key profile runs the native tone block with its key; then the unchanged native block.
pub fn build_tonemap_profiles(
    source: &[u8],
    target: &[u8],
    markers: &[f32],
    keys: &[KeyProfile],
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
    let target_hash = format!("{:x}", Sha256::digest(target));
    if !TARGET_HASHES.contains(&target_hash.as_str()) {
        return Ok(None);
    }
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
    updated.extend(native_ops[18..41].iter().flatten());
    updated.extend(std::iter::repeat_n(0x01000015, markers.len() + keys.len()));
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

/// The retail reflection implementation treats binding names before the
/// buffer table as their own string pool. Keep them there, not after variables.
fn add_marker_binding(old: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        u32_at(old, 0)? == 1 && u32_at(old, 8)? == 9,
        "unexpected tone-map reflection tables"
    );
    let mut bindings = Vec::new();
    for i in 0..9 {
        let at = u32_at(old, 12)? as usize + 32 * i;
        let values = words(old.get(at..at + 32).context("truncated binding")?)?;
        bindings.push((
            name(old, values[0] as usize)?.to_owned(),
            values[1..].to_vec(),
        ));
    }
    bindings.push(("static_globals".into(), vec![0, 0, 0, 0, 1, 1, 0]));
    let cb = u32_at(old, 4)? as usize;
    ensure!(
        name(old, u32_at(old, cb)? as usize)? == "$Globals"
            && u32_at(old, cb + 4)? == 22
            && u32_at(old, cb + 12)? == 304
            && u32_at(old, cb + 16)? == 0
            && u32_at(old, cb + 20)? == 0,
        "unexpected tone-map globals layout"
    );
    let mut variables = Vec::new();
    for i in 0..22 {
        let at = u32_at(old, cb + 8)? as usize + 24 * i;
        let v = words(old.get(at..at + 24).context("truncated variable")?)?;
        let ty: [u8; 16] = old
            .get(v[4] as usize..v[4] as usize + 16)
            .context("truncated type")?
            .try_into()?;
        ensure!(
            ty[10..] == [0; 6] && v[5] == 0,
            "unsupported structured/defaulted tone-map constant"
        );
        variables.push((name(old, v[0] as usize)?.to_owned(), v[1..4].to_vec(), ty));
    }
    let resources = 28;
    let mut out = Reflection {
        bytes: vec![0; resources + bindings.len() * 32],
        ..Default::default()
    };
    out.bytes[..28].copy_from_slice(&old[..28]);
    for (i, (name, values)) in bindings.iter().enumerate() {
        let label = out.name(name);
        out.put(resources + 32 * i, &[label]);
        out.put(resources + 32 * i + 4, values);
    }
    out.align();
    let buffers = out.bytes.len();
    let vars0 = buffers + 48;
    let vars1 = vars0 + 24 * variables.len();
    out.bytes.resize(vars1 + 24, 0);
    let globals = out.name("$Globals");
    let statics = out.name("static_globals");
    out.put(
        buffers,
        &[globals, variables.len() as u32, vars0 as u32, 304, 0, 0],
    );
    out.put(buffers + 24, &[statics, 1, vars1 as u32, 304, 0, 0]);
    for (i, (name, fields, ty)) in variables.iter().enumerate() {
        let label = out.name(name);
        let ty = out.ty(*ty);
        out.put(
            vars0 + 24 * i,
            &[label, fields[0], fields[1], fields[2], ty, 0],
        );
    }
    let label = out.name("atmosphere_parameters3");
    let ty = out.ty([1, 0, 3, 0, 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.put(vars1, &[label, 288, 16, 2, ty, 0]);
    let creator = out.name(name(old, u32_at(old, 24)? as usize)?);
    out.put(
        0,
        &[
            2,
            buffers as u32,
            bindings.len() as u32,
            resources as u32,
            u32_at(old, 16)?,
            u32_at(old, 20)?,
            creator,
        ],
    );
    Ok(out.bytes)
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
            assert!(build_tonemap_profiles(&[], &[], &[-32198.], &keys).is_err());
        }
        let ok = [KeyProfile { marker: 32909.0, key: 6.0 }];
        assert!(build_tonemap_profiles(&[], &[], &[-32198.], &ok).unwrap().is_none());
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
}
