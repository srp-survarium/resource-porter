// SPDX-License-Identifier: GPL-3.0-or-later
//! One whole `resources.db` of an archive and its patches as the 0.26g game
//! mounts them (a later archive's file wins), for tools that read a single
//! archive (a local server's game data): `flatten-layers <out> <db> [patch..]`.
fn main() -> anyhow::Result<()> {
    let a: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    anyhow::ensure!(a.len() >= 2, "usage: flatten-layers <out> <db> [patch..]");
    let mut db = vostok_resource_porter::ResourceDb::open_layers(&a[1..])?;
    let stats = vostok_resource_porter::archive::write_over(&mut db, &Default::default(), &a[0])?;
    println!("{stats:?}");
    Ok(())
}
