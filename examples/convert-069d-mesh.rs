// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert one 0.69d skinned `converted_model` to 0.26g's layout, for
//! inspection: `convert-069d-mesh <in> <bones> <out>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mesh = vostok_resource_porter::weapons_026::convert::skinned_069_to_026(&std::fs::read(&a[0])?, a[1].parse()?)?;
    std::fs::write(&a[2], mesh.to_bytes()?)?;
    Ok(())
}
