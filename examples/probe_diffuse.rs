//! The `_diffuse` cube of an environment probe cube on disk: probe_diffuse PROBE.dds OUT.dds [GAIN]
//!
//! For probes rendered outside the porter (Chemical Plant v0.11's evening and rain, baked in the game).
//! Same builder as the early-map ports (`probe_irradiance::diffuse_cube_scaled`, gain 1 by default).
use std::env;
use vostok_resource_porter::probe_irradiance::diffuse_cube_scaled;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    anyhow::ensure!(matches!(args.len(), 2 | 3), "usage: probe_diffuse PROBE.dds OUT.dds [GAIN]");
    let gain = args.get(2).map(|g| g.parse::<f32>()).transpose()?.unwrap_or(1.0);
    std::fs::write(&args[1], diffuse_cube_scaled(&std::fs::read(&args[0])?, gain)?)?;
    Ok(())
}
