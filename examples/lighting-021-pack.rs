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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyProfileSpec {
    name: String,
    key: f32,
    /// Each weather's `mie_fog_base_height` as its projects carry it.
    markers: Vec<f32>,
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
    let native_fog = native.read(shaders::PATH)?;
    let fog = shaders::build(&donor.read(shaders::PATH)?, &native_fog, &profiles)?;
    let mut converted = vec![(shaders::PATH.to_owned(), native_fog, fog)];
    let source_tone = donor.read(shaders::TONEMAP_SOURCE)?;
    let paths: Vec<String> = native
        .paths_with_prefix(shaders::TONEMAP_PREFIX)
        .map(str::to_owned)
        .collect();
    for path in paths {
        let before = native.read(&path)?;
        if let Some(after) = shaders::build_tonemap_profiles(&source_tone, &before, &markers, &keys)? {
            converted.push((path, before, after));
        }
    }
    ensure!(converted.len() == 5, "unexpected compatible tone-map count");
    let sky = native.read(shaders::SKY_PATH)?;
    converted.push((
        shaders::SKY_PATH.into(),
        sky.clone(),
        shaders::build_sky(&donor.read(shaders::SKY_PATH)?, &sky, &markers)?,
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
