//! Shared original atmosphere, lighting and tone arithmetic, using the existing
//! fingerprinted shader adapters. No geometry, objects or local light edits.
//! lighting-021-pack MANIFEST DONOR_DB STOCK_DB OUT [LIVE_PATCH ...]
use std::{collections::BTreeSet, fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::survarium_026::check_hash;
use vostok_resource_porter::weather_fog_021 as shaders;
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    version: String,
    donor_sha256: String,
    target_sha256: String,
    profiles: Vec<Profile>,
    /// Scenes drawn with the native tone curve at their own build's exposure key
    /// (London: 0.31a2's 6.0), one marker per weather.
    #[serde(default)]
    key_profiles: Vec<KeyProfileSpec>,
    /// Scenes whose exposure follows the view as 0.69d's does (`ExposureProfile`): each
    /// 0.69d map's day, with the range and key resource-porter's 0.69d map builder prints
    /// for it.
    #[serde(default)]
    exposure_profiles: Vec<ExposureProfileSpec>,
    /// Scenes whose rain darkens what it does not reach by their own factor (London's rain:
    /// 0.31a2's rain_shadow_diffuse_multiplier 0.7 for 0.26g's fixed 0.85).
    #[serde(default)]
    shelter_profiles: Vec<ShelterProfileSpec>,
    /// Scenes whose sun 0.69d's cloud shadows dim (`CloudShadowProfile`): each 0.69d map's
    /// day with `use_cloud_shadows`, with the values resource-porter's 0.69d map builder
    /// prints for it.
    #[serde(default)]
    cloud_shadow_profiles: Vec<CloudShadowProfileSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudShadowProfileSpec {
    name: String,
    marker: f32,
    /// The direction sunlight travels.
    light: [f32; 3],
    cloud_altitude_km: f32,
    cloud_shadow_opacity: f32,
    cloud_shadow_opacity_power: f32,
    clouds_rounds_per_minute: f32,
    sky_clouds_rotation_degrees: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShelterProfileSpec {
    name: String,
    marker: f32,
    rain_shadow_diffuse_multiplier: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyProfileSpec {
    name: String,
    key: f32,
    /// Each weather's `mie_fog_base_height` marker as its projects carry it.
    markers: Vec<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExposureProfileSpec {
    name: String,
    /// The scene's `mie_fog_base_height` marker as its projects carry it.
    marker: f32,
    floor: f32,
    ceiling: f32,
    key: f32,
    /// 0.69d's sky over 0.26g's under the port's sun (`shaders::SkyScale`), as the 0.69d map
    /// builder prints it; 1 when absent.
    #[serde(default = "one")]
    sky: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    project: String,
    marker: f32,
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() >= 4,
        "MANIFEST DONOR_DB STOCK_DB OUT [LIVE_PATCH ...]"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(&args[0])?)?;
    check_hash(&args[1], &manifest.donor_sha256)?;
    check_hash(&args[2], &manifest.target_sha256)?;
    ensure!(!args[3].exists(), "output must be new");
    let mut donor = ResourceDb::open(&args[1])?;
    let mut layers = vec![args[2].clone()];
    layers.extend_from_slice(&args[4..]);
    let mut native = ResourceDb::open_layers(&layers)?;
    let mut profiles = Vec::new();
    let mut scattering = Vec::new();
    let mut markers = BTreeSet::new();
    let mut rain_markers = Vec::new();
    for spec in &manifest.profiles {
        ensure!(
            spec.marker.is_finite() && spec.marker < -32021.0 && spec.marker.fract() == 0.0,
            "invalid profile marker"
        );
        ensure!(
            markers.insert(spec.marker as i32),
            "duplicate profile marker"
        );
        if spec.project.contains("/rain/") {
            rain_markers.push(spec.marker);
        }
        let project = BinaryConfig::parse(&donor.read(&spec.project)?)?;
        let Some(ConfigValue::Indexed(objects)) = project.root.named("client_objects") else {
            anyhow::bail!("{} has no client objects", spec.project);
        };
        let mut environments = objects.iter().filter(|o| {
            o.named("game_object_type") == Some(&ConfigValue::String("environment".into()))
        });
        let environment = environments.next().context("missing environment")?;
        ensure!(environments.next().is_none(), "ambiguous environment");
        let properties = environment
            .named("properties")
            .context("missing properties")?;
        let Some(ConfigValue::Float4(rgb)) = properties.named("rayleigh_fog_color") else {
            anyhow::bail!("missing original Rayleigh colour");
        };
        let color = [rgb[0].powf(2.2), rgb[1].powf(2.2), rgb[2].powf(2.2)];
        ensure!(
            color.iter().all(|c| c.is_finite() && *c >= 0.0),
            "invalid fog colour"
        );
        profiles.push(shaders::Profile {
            marker: spec.marker,
            color,
        });
        let rayleigh_sun = match properties.named("atmosphere_rayleighSun_multiplier") {
            Some(ConfigValue::Float(v)) => *v,
            Some(ConfigValue::Integer(v)) => *v as f32,
            _ => anyhow::bail!("missing original Rayleigh sunlight multiplier"),
        };
        ensure!(
            rayleigh_sun.is_finite() && rayleigh_sun >= 0.0,
            "invalid Rayleigh gain"
        );
        scattering.push(shaders::ScatteringProfile {
            marker: spec.marker,
            rayleigh_sun,
        });
        println!(
            "{}: marker {}, original linear Rayleigh RGB {:?}",
            spec.project, spec.marker, color
        );
    }
    ensure!(!profiles.is_empty(), "no original profiles");
    let markers: Vec<f32> = profiles.iter().map(|p| p.marker).collect();
    let keys: Vec<shaders::KeyProfile> = manifest
        .key_profiles
        .iter()
        .flat_map(|p| {
            println!("{}: exposure key {}, markers {:?}", p.name, p.key, p.markers);
            p.markers.iter().map(|&marker| shaders::KeyProfile { marker, key: p.key })
        })
        .collect();
    let exposures: Vec<shaders::ExposureProfile> = manifest
        .exposure_profiles
        .iter()
        .map(|p| {
            println!("{}: marker {}, exposure follows the mean from {} to {}, key {}", p.name, p.marker, p.floor, p.ceiling, p.key);
            shaders::ExposureProfile { marker: p.marker, floor: p.floor, ceiling: p.ceiling, key: p.key }
        })
        .collect();
    let native_fog = native.read(shaders::PATH)?;
    let fog = shaders::build(&donor.read(shaders::PATH)?, &native_fog, &profiles)?;
    let mut converted = vec![(shaders::PATH.to_owned(), native_fog, fog)];
    let source_tone = donor.read(shaders::TONEMAP_SOURCE)?;
    let paths: Vec<String> = native
        .paths_with_prefix(shaders::TONEMAP_PREFIX)
        .map(str::to_owned)
        .collect();
    let tone_start = converted.len();
    for path in paths {
        let before = native.read(&path)?;
        if let Some(after) = shaders::build_tonemap_profiles(&source_tone, &before, &markers, &keys, &exposures)? {
            converted.push((path, before, after));
        }
    }
    // Four programs take every profile; with an exposure profile, the six of post-process
    // quality 0 and 1 take it too. The two depth-of-field programs stay native.
    ensure!(
        converted.len() - tone_start == if exposures.is_empty() { 4 } else { 10 },
        "unexpected tone-map variant count: {}",
        converted.len() - tone_start
    );
    // An exposure profile's scene is metered as 0.69d meters, by the weighted geometric mean
    // of the frame: the three programs of the luminance chain (one variant each).
    if !exposures.is_empty() {
        for (prefix, build) in [
            (
                shaders::LOG_METER_PREFIX,
                shaders::build_log_meter as fn(&[u8], &[shaders::ExposureProfile]) -> Result<Vec<u8>>,
            ),
            (shaders::SUM_METER_PREFIX, shaders::build_sum_meter),
            (shaders::MEAN_METER_PREFIX, shaders::build_mean_meter),
        ] {
            let paths: Vec<String> = native
                .paths_with_prefix(prefix)
                .filter(|p| native.is_file(p))
                .map(str::to_owned)
                .collect();
            ensure!(paths.len() == 1, "unexpected variant count for {prefix}");
            let before = native.read(&paths[0])?;
            let after = build(&before, &exposures).with_context(|| paths[0].clone())?;
            converted.push((paths[0].clone(), before, after));
        }
    }
    // A cloud shadow profile's scene takes 0.69d's cloud shadows on its sun: the 16
    // sun_shadow variants that read sky_shadows_texture.
    let clouds: Vec<shaders::CloudShadowProfile> = manifest
        .cloud_shadow_profiles
        .iter()
        .map(|p| {
            println!("{}: marker {}, cloud shadows {:?} km {} opacity {} power {} rpm {} rotation {}", p.name, p.marker, p.light, p.cloud_altitude_km, p.cloud_shadow_opacity, p.cloud_shadow_opacity_power, p.clouds_rounds_per_minute, p.sky_clouds_rotation_degrees);
            shaders::CloudShadowProfile {
                marker: p.marker,
                light: p.light,
                altitude_km: p.cloud_altitude_km,
                opacity: p.cloud_shadow_opacity,
                power: p.cloud_shadow_opacity_power,
                rate: p.clouds_rounds_per_minute / 60.0,
                phase: p.sky_clouds_rotation_degrees / 360.0,
            }
        })
        .collect();
    if !clouds.is_empty() {
        for (variant, _) in shaders::SKY_SHADOW_VARIANTS {
            let path = format!("{}{variant}", shaders::SUN_SHADOW_PREFIX);
            let before = native.read(&path)?;
            let after = shaders::build_cloud_shadow(&before, variant, &clouds).with_context(|| path.clone())?;
            converted.push((path, before, after));
        }
    }
    let sky = native.read(shaders::SKY_PATH)?;
    let sky_scales: Vec<shaders::SkyScale> = manifest
        .exposure_profiles
        .iter()
        .map(|p| shaders::SkyScale { marker: p.marker, sky: p.sky })
        .collect();
    converted.push((
        shaders::SKY_PATH.into(),
        sky.clone(),
        shaders::scale_sky(
            &shaders::build_sky(&donor.read(shaders::SKY_PATH)?, &sky, &markers)?,
            &sky_scales,
        )?,
    ));
    let before = native.read(shaders::SCATTERING_PATH)?;
    let after = shaders::build_scattering(
        &donor.read(shaders::SCATTERING_PATH)?,
        &before,
        &sky,
        &scattering,
    )?;
    converted.push((shaders::SCATTERING_PATH.into(), before, after));
    let before = native.read(shaders::AO_PATH)?;
    let after =
        shaders::build_ambient_occlusion(&donor.read(shaders::AO_SOURCE)?, &before, &markers)?;
    converted.push((shaders::AO_PATH.into(), before, after));
    for quality in 0..4 {
        let path = shaders::god_rays_path(quality);
        let before = native.read(&path)?;
        let after =
            shaders::build_god_rays_falloff(&donor.read(&path)?, &before, quality, &markers)?;
        converted.push((path, before, after));
    }
    let paths: Vec<String> = native
        .paths_with_prefix(shaders::CLOUD_PREFIX)
        .filter(|p| native.is_file(p))
        .map(str::to_owned)
        .collect();
    ensure!(paths.len() == 8, "unexpected cloud variant count");
    for path in paths {
        let variant = path
            .strip_prefix(shaders::CLOUD_PREFIX)
            .context("cloud prefix")?;
        ensure!(
            variant.len() == 67 && matches!(variant.as_bytes()[31], b'0' | b'1'),
            "cloud key changed"
        );
        let lit = variant.as_bytes()[31] == b'1';
        let before = native.read(&path)?;
        let after = shaders::build_cloud(
            &donor.read(&shaders::cloud_source_path(lit))?,
            &before,
            &markers,
            lit,
        )?;
        converted.push((path, before, after));
    }
    let source = donor.read(shaders::COMPOSITION_SOURCE)?;
    let sun = donor.read(shaders::SUN_COMPOSITION_SOURCE)?;
    let template = native.read(shaders::COMPOSITION_TEMPLATE)?;
    let paths: Vec<String> = native
        .paths_with_prefix(shaders::COMPOSITION_PREFIX)
        .filter(|p| native.is_file(p))
        .map(str::to_owned)
        .collect();
    ensure!(paths.len() == 64, "unexpected composition variant count");
    let start = converted.len();
    for path in paths {
        let before = native.read(&path)?;
        if let Some(after) =
            shaders::build_composition(&source, &sun, &before, &template, &rain_markers, &markers)?
        {
            converted.push((path, before, after));
        }
    }
    ensure!(
        converted.len() - start == 4,
        "unexpected compatible sun variant count"
    );
    let shelters: Vec<shaders::ShelterProfile> = manifest
        .shelter_profiles
        .iter()
        .map(|p| {
            println!("{}: rain shelter {} on marker {}", p.name, p.rain_shadow_diffuse_multiplier, p.marker);
            shaders::ShelterProfile { marker: p.marker, factor: p.rain_shadow_diffuse_multiplier }
        })
        .collect();
    if !shelters.is_empty() {
        for (path, _, after) in &mut converted[start..] {
            *after = shaders::build_shelter(after, &shelters).with_context(|| path.clone())?;
        }
    }
    for (prefix, source_path, template_path, expected, compatible, build) in [
        (
            shaders::PARTICLE_PREFIX,
            shaders::PARTICLE_SOURCE,
            shaders::PARTICLE_TEMPLATE,
            480,
            24,
            shaders::build_particle
                as fn(&[u8], &[u8], &[u8], &[shaders::Profile]) -> Result<Option<Vec<u8>>>,
        ),
        (
            shaders::WATER_PREFIX,
            shaders::WATER_SOURCE,
            shaders::WATER_TEMPLATE,
            120,
            8,
            shaders::build_water,
        ),
    ] {
        let source = donor.read(source_path)?;
        let template = native.read(template_path)?;
        let paths: Vec<String> = native
            .paths_with_prefix(prefix)
            .filter(|p| native.is_file(p))
            .map(str::to_owned)
            .collect();
        ensure!(
            paths.len() == expected,
            "unexpected variant count for {prefix}"
        );
        let start = converted.len();
        for path in paths {
            let before = native.read(&path)?;
            if let Some(after) = build(&source, &before, &template, &profiles)? {
                converted.push((path, before, after));
            }
        }
        ensure!(
            converted.len() - start == compatible,
            "unexpected compatible variant count for {prefix}"
        );
        if prefix == shaders::PARTICLE_PREFIX {
            // The family's other fogged programs: the same fog found by what it
            // computes, in whatever registers; the 24 above keep their bytes.
            use shaders::forward_glass::VertexFog;
            let pinned = [
                native.read(VertexFog::NATIVE[0])?,
                native.read(VertexFog::NATIVE[1])?,
            ];
            let mut fog = VertexFog::new(&donor.read(VertexFog::ORIGINAL)?, [&pinned[0], &pinned[1]])?;
            let done: std::collections::BTreeSet<String> =
                converted[start..].iter().map(|(path, _, _)| path.clone()).collect();
            let paths: Vec<String> = native
                .paths_with_prefix(prefix)
                .filter(|p| native.is_file(p) && !done.contains(*p))
                .map(str::to_owned)
                .collect();
            // Both adaptations of the 24 agree byte for byte.
            for (path, before, after) in &converted[start..] {
                ensure!(
                    fog.build(before, &profiles)?.as_ref() == Some(after),
                    "the two particle fog adaptations differ for {path}"
                );
            }
            let more = converted.len();
            for path in paths {
                let before = native.read(&path)?;
                if let Some(after) = fog.build(&before, &profiles)? {
                    converted.push((path, before, after));
                }
            }
            ensure!(
                converted.len() - more == 40,
                "unexpected count of other fogged particle programs: {}",
                converted.len() - more
            );
        }
    }
    let pairs = shaders::local_light_pairs()?;
    ensure!(pairs.len() == 32, "unexpected local-light variant count");
    for (source_path, path) in pairs {
        let before = native.read(path)?;
        let after = shaders::build_local_light(path, &donor.read(source_path)?, &before, &markers)?;
        converted.push((path.into(), before, after));
    }
    let mut operations = Vec::new();
    for (path, before, after) in converted {
        let file = args[3].join("files").join(&path);
        fs::create_dir_all(file.parent().context("file without parent")?)?;
        fs::write(file, &after)?;
        println!("{path}: {} -> {}", sha256_hex(&before), sha256_hex(&after));
        operations.push(Operation::ReplaceFile {
            path,
            source: None,
            base_sha256: sha256_hex(&before),
        });
    }
    let pack = PackManifest {
        format_version: 1, id: manifest.id, version: manifest.version,
        target_build: TARGET_BUILD.into(),
        description: Some("Original 0.21 atmosphere, direct-sun lighting, quality-1 AO, ray falloff and tone curve for all six 0.21 maps in day, rain and evening; unchanged native shader fallback for unmarked scenes".into()),
        requires: vec!["maps-021d-school-fort".into(), "maps-021d-rudnya".into()],
        base_db_sha256: Some(manifest.target_sha256), operations, server: None,
        notes: vec!["Built by examples/lighting-021-pack.rs from fingerprinted retail programs. Projects opt in with their explicit mie_fog_base_height marker; authored fog inputs are preserved by the content edit lists.".into()],
    };
    fs::write(args[3].join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    Ok(())
}
