// SPDX-License-Identifier: GPL-3.0-or-later
//! DDS header convention under a prefix: retail 0.26 (and v0.100b) textures
//! have flags 0xA100F and dwPitchOrLinearSize = every byte after the
//! 128-byte header (the mip payload plus one trailing colour-space byte),
//! and that layout is header + sum of mips + 1. Counts what matches and lists
//! what does not. Exit status 1 on any mismatch.
//!
//!     dds_headers DB PREFIX [SHOW]
use vostok_resource_porter::ResourceDb;
fn mip_bytes(w: u32, h: u32, mips: u32, block: u32) -> usize {
    (0..mips.max(1)).map(|i| {
        let (w, h) = ((w >> i).max(1), (h >> i).max(1));
        (w.div_ceil(4) * h.div_ceil(4) * block) as usize
    }).sum()
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut db = ResourceDb::open(&a[0]).unwrap();
    let show: usize = a.get(2).map(|s| s.parse().unwrap()).unwrap_or(20);
    let paths: Vec<String> = db.paths_with_prefix(&a[1]).filter(|p| p.ends_with(".dds")).map(str::to_owned).collect();
    let (mut ok, mut bad) = (0, 0);
    for p in &paths {
        let b = db.read(p).unwrap();
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let (flags, h, w, pitch, mips, fourcc, caps2) = (u(8), u(12), u(16), u(20), u(28), &b[84..88], u(112));
        let block = match fourcc { b"DXT1" => 8, b"DXT5" | b"DXT3" => 16, _ => 0 };
        let faces = if caps2 & 0xfe00 != 0 { 6 } else { 1 };
        let expected = if block > 0 { Some(mip_bytes(w, h, mips, block) * faces + 1) } else { None };
        let after = b.len() - 128;
        let good = flags == 0xA100F && pitch as usize == after && expected.is_none_or(|e| e == after);
        if good { ok += 1 } else {
            bad += 1;
            if bad <= show { println!("{p}: flags {flags:#x} pitch {pitch} after-header {after} expected {expected:?} {}x{} mips {mips} {}", w, h, String::from_utf8_lossy(fourcc)); }
        }
    }
    println!("{} textures under {}: {ok} native header, {bad} not", paths.len(), a[1]);
    if bad > 0 { std::process::exit(1) }
}
