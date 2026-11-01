// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
fn halves_round_to_nearest() {
    assert_eq!(to_half(0.0), 0);
    assert_eq!(to_half(1.0), 0x3c00);
    assert_eq!(to_half(-2.0), 0xc000);
    assert_eq!(to_half(0.5), 0x3800);
    assert_eq!(to_half(65504.0), 0x7bff);
    assert_eq!(to_half(1e6), 0x7c00);
    // 1 + 2^-11 is halfway: even (down); 1 + 3 * 2^-11 rounds up.
    assert_eq!(to_half(1.0 + 2f32.powi(-11)), 0x3c00);
    assert_eq!(to_half(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
    // A subnormal half.
    assert_eq!(to_half(2f32.powi(-24)), 1);
}

fn mesh_069(vertices: &[([f32; 3], [f32; 3], [u16; 2])], indices: &[u16]) -> Vec<u8> {
    let mut v = vec![];
    for (p, basis, uv) in vertices {
        for c in p.iter().chain(basis) {
            v.extend(c.to_le_bytes());
        }
        for c in uv {
            v.extend(c.to_le_bytes());
        }
    }
    let mut out = vec![];
    out.extend(3u32.to_le_bytes());
    out.extend((4 + v.len() as u32).to_le_bytes());
    out.extend((vertices.len() as u32).to_le_bytes());
    out.extend(v);
    out.extend(4u32.to_le_bytes());
    out.extend((4 + 2 * indices.len() as u32).to_le_bytes());
    out.extend((indices.len() as u32).to_le_bytes());
    for i in indices {
        out.extend(i.to_le_bytes());
    }
    // 0.69d's statistics chunk, which 0.26g lacks.
    out.extend(110u32.to_le_bytes());
    out.extend(4u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out
}

#[test]
fn a_069_static_mesh_becomes_026s_layout() {
    // The basis float of x=0x00, y=0x7f, z=0x80: (0x007f80) / 2^24.
    let normal = 0x007f80 as f32 / 16_777_216.0;
    let src = mesh_069(
        &[
            ([0.0, 0.0, 0.0], [normal, normal, normal], [0x3c00, 0]),
            ([0.0, 0.0, 0.1], [normal, normal, normal], [0, 0x3c00]),
            ([0.0, 0.1, 0.1], [normal, normal, normal], [0x3800, 0x3800]),
        ],
        &[0, 1, 2],
    );
    let out = static_069_to_026(&src).unwrap();
    let m = Mesh::parse(&out).unwrap();
    assert_eq!(m.stride, STATIC_026);
    assert_eq!(m.positions[1], [0.0, 0.0, 0.1]);
    assert_eq!(m.triangles, vec![[0, 1, 2]]);
    let v = m.vertex(0);
    assert_eq!(&v[12..16], &[0x80, 0x7f, 0x00, 0x7f]);
    assert_eq!(&v[16..20], &[0x80, 0x7f, 0x00, 0xff]);
    assert_eq!(&v[20..24], &[0x80, 0x7f, 0x00, 0xff]);
    assert_eq!(&v[24..28], &[0x00, 0x3c, 0x00, 0x00]);
    // Only chunks 3 and 4 remain.
    assert_eq!(out.len(), 8 + 4 + 3 * 28 + 8 + 4 + 6);
}

fn skinned(vertices: &[([f32; 3], [u8; 12], [f32; 2])], indices: &[u16]) -> Vec<u8> {
    let mut v = vec![];
    for (p, words, uv) in vertices {
        for c in p {
            v.extend(c.to_le_bytes());
        }
        v.extend(1u32.to_le_bytes());
        v.extend_from_slice(words);
        for c in uv {
            v.extend(c.to_le_bytes());
        }
    }
    static_like(&v, vertices.len(), indices)
}

fn static_like(v: &[u8], n: usize, indices: &[u16]) -> Vec<u8> {
    let mut out = vec![];
    out.extend(3u32.to_le_bytes());
    out.extend((4 + v.len() as u32).to_le_bytes());
    out.extend((n as u32).to_le_bytes());
    out.extend_from_slice(v);
    out.extend(4u32.to_le_bytes());
    out.extend((4 + 2 * indices.len() as u32).to_le_bytes());
    out.extend((indices.len() as u32).to_le_bytes());
    for i in indices {
        out.extend(i.to_le_bytes());
    }
    out
}

#[test]
fn the_stock_device_is_our_cut_triangles_at_the_muzzle() {
    let words = [1, 2, 3, 0x11, 4, 5, 6, 0x22, 7, 8, 9, 0x33];
    let gun = Mesh::parse(&skinned(
        &[
            ([0.0, 0.05, 0.60], words, [0.25, 0.5]),
            ([0.0, 0.05, 0.65], words, [0.5, 0.5]),
            ([0.0, 0.06, 0.65], words, [0.5, 1.0]),
            ([0.0, 0.05, 0.10], words, [0.0, 0.0]),
        ],
        &[0, 1, 2, 3, 0, 1],
    ))
    .unwrap();
    let (bytes, local) = stock_mesh(&gun, &[[0, 1, 2]], [0.0, 0.05, 0.60]).unwrap();
    let m = Mesh::parse(&bytes).unwrap();
    assert_eq!(m.stride, STATIC_026);
    assert_eq!(m.positions.len(), 3);
    assert_eq!(m.triangles, vec![[0, 1, 2]]);
    assert!((local[1][2] - 0.05).abs() < 1e-6);
    let v = m.vertex(0);
    // The words' xyz kept, their top bytes the static meshes' (127, 255, 255).
    assert_eq!(&v[12..24], &[1, 2, 3, 0x7f, 4, 5, 6, 0xff, 7, 8, 9, 0xff]);
    assert_eq!(&v[24..28], &[0x00, 0x34, 0x00, 0x38]);
}

#[test]
fn a_fire_point_off_the_bore_is_refused() {
    assert_eq!(fire_mm([0.0, 0.0, 0.0669], "x").unwrap(), 67);
    assert!(fire_mm([0.01, 0.0, 0.05], "x").is_err());
    assert_eq!(fire_mm([0.0, 0.0044, 0.0464], "x").unwrap(), 46);
}

#[test]
fn a_brakes_stats_go_where_a_weapon_reads_them() {
    let p = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<String>>();
    let recoil = effective_modifiers(&p(&["parameters", "recoil_modifier"]), -0.04, 1);
    assert_eq!(recoil.len(), 6);
    assert!(recoil.iter().all(|(path, v, k)| path[0] == "recoil" && *v == -0.04 && *k == 2));
    assert_eq!(
        effective_modifiers(&p(&["parameters", "aim_time_modifier"]), 0.03, 1),
        vec![(p(&["parameters", "aim_time"]), 0.03, 2)]
    );
    assert_eq!(
        effective_modifiers(&p(&["parameters", "unmasking_radius"]), -0.4, 2),
        vec![(p(&["parameters", "unmasking_radius"]), -0.4, 2)]
    );
    assert_eq!(Kind::of(&[(p(&["parameters", "unmasking_radius"]), -0.4, 2)]), Kind::FlashHider);
    assert_eq!(Kind::of(&recoil), Kind::Brake);
}
