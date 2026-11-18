//! Scoped original forward fog, built from pinned retail programs.
//! forward-fog-021-pack PROFILES DONOR_DB STOCK_DB PATCHES_TSV PATCH_CACHE NEW_OUTPUT
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::weather_fog_021::{
    Profile,
    forward_glass::{self, Kind},
};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    id: String,
    version: String,
    donor_sha256: String,
    target_sha256: String,
    profiles: Vec<Scene>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scene {
    project: String,
    marker: f32,
}

struct NativeLayer {
    path: PathBuf,
    sha256: String,
    db: ResourceDb,
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 6,
        "PROFILES DONOR_DB STOCK_DB PATCHES_TSV PATCH_CACHE NEW_OUTPUT"
    );
    let manifest: Profiles = serde_json::from_slice(&fs::read(&args[0])?)?;
    ensure!(
        manifest.id == "three-maps-021-lighting" && manifest.version == "6",
        "unsupported profile source"
    );
    vostok_resource_porter::survarium_026::check_hash(&args[1], &manifest.donor_sha256)?;
    vostok_resource_porter::survarium_026::check_hash(&args[2], &manifest.target_sha256)?;
    ensure!(!args[5].exists(), "output must be new");
    let mut donor = ResourceDb::open(&args[1])?;
    let mut layers = vec![NativeLayer {
        path: args[2].clone(),
        sha256: manifest.target_sha256.clone(),
        db: ResourceDb::open(&args[2])?,
    }];
    let stack_source = fs::read(&args[3])?;
    for line in std::str::from_utf8(&stack_source)?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields: Vec<_> = line.splitn(4, '\t').collect();
        ensure!(fields.len() == 4, "invalid published patch row");
        let revision = fields[0]
            .strip_prefix("resources.db.patch_")
            .context("published patch name")?;
        ensure!(
            !revision.is_empty() && revision.bytes().all(|b| b.is_ascii_digit()),
            "invalid patch name"
        );
        let path = args[4].join(fields[0]);
        ensure!(
            fs::metadata(&path)?.len() == fields[2].parse::<u64>()?,
            "published archive size changed"
        );
        layers.push(NativeLayer {
            sha256: fields[1].into(),
            db: ResourceDb::open(&path)?,
            path,
        });
    }
    let mut verified = BTreeSet::from([0_usize]);
    let mut profiles = Vec::new();
    let mut markers = BTreeSet::new();
    for scene in manifest.profiles {
        ensure!(
            scene.marker.is_finite()
                && scene.marker < -32021.0
                && scene.marker.fract() == 0.0
                && markers.insert(scene.marker.to_bits()),
            "invalid scene marker"
        );
        let root = BinaryConfig::parse(&donor.read(&scene.project)?)?.root;
        let Some(ConfigValue::Indexed(objects)) = root.named("client_objects") else {
            anyhow::bail!("missing objects")
        };
        let mut environments = objects.iter().filter(|o| {
            o.named("game_object_type") == Some(&ConfigValue::String("environment".into()))
        });
        let environment = environments.next().context("missing environment")?;
        ensure!(environments.next().is_none(), "ambiguous environment");
        let Some(ConfigValue::Float4(rgb)) = environment
            .named("properties")
            .and_then(|p| p.named("rayleigh_fog_color"))
        else {
            anyhow::bail!("missing original Rayleigh color")
        };
        profiles.push(Profile {
            marker: scene.marker,
            color: [rgb[0].powf(2.2), rgb[1].powf(2.2), rgb[2].powf(2.2)],
        });
    }
    ensure!(
        !profiles.is_empty() && profiles.len() % 3 == 0,
        "expected a day, rain and evening profile a map"
    );
    let scenes = profiles.len();
    let mut operations = Vec::new();
    for kind in [Kind::Base, Kind::Probe, Kind::Direct] {
        let (source_path, native_path) = kind.paths();
        let source = donor.read(source_path)?;
        let template = layers[0].db.read(native_path)?;
        ensure!(
            forward_glass::build(kind, &source, &template, &template, &profiles)?.is_some(),
            "the observed forward program must be adapted"
        );
        let prefix = native_path
            .rsplit_once('/')
            .context("shader family")?
            .0
            .to_owned()
            + "/";
        let mut paths = BTreeMap::new();
        for (index, layer) in layers.iter().enumerate() {
            for path in layer
                .db
                .paths_with_prefix(&prefix)
                .filter(|p| layer.db.is_file(p))
            {
                paths.insert(path.to_owned(), index);
            }
        }
        let mut count = 0;
        for (path, layer_index) in paths {
            let before = layers[layer_index].db.read(&path)?;
            let Some(after) = forward_glass::build(kind, &source, &template, &before, &profiles)?
            else {
                continue;
            };
            if verified.insert(layer_index) {
                vostok_resource_porter::survarium_026::check_hash(
                    &layers[layer_index].path,
                    &layers[layer_index].sha256,
                )?;
            }
            let output = args[5].join("files").join(&path);
            fs::create_dir_all(output.parent().context("shader parent")?)?;
            fs::write(output, &after)?;
            operations.push(Operation::ReplaceFile {
                path,
                source: None,
                base_sha256: sha256_hex(&before),
            });
            count += 1;
        }
        ensure!(count > 0, "no matching forward program");
        println!("{prefix}: {count} exact matching fog blocks, all {scenes} scene guards");
    }
    let sources: Vec<_> = verified.iter().map(|&i| serde_json::json!({"archive": layers[i].path.file_name().unwrap().to_string_lossy(), "sha256": layers[i].sha256})).collect();
    fs::write(
        args[5].join("source-archives.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"published_manifest_sha256": sha256_hex(&stack_source), "used_archives": sources}),
        )?,
    )?;
    let pack = PackManifest {
        format_version: 1, id: "v021d-forward-fog".into(), version: "3".into(), target_build: TARGET_BUILD.into(),
        description: Some(format!("Original fog in exact matching forward base, probe and direct-lighting programs, scoped to {scenes} original 0.21 day/rain/evening scene markers")),
        requires: vec!["three-maps-021-lighting".into()], base_db_sha256: Some(manifest.target_sha256), operations, server: None,
        notes: vec!["Built by examples/forward-fog-021-pack.rs using the original lighting profile source and pinned original/native shader blocks. Retains native lighting, transparency, bindings and complete unmarked native fog fallback. Uses the same original Rayleigh RGB upload, far-distance schema conversion and observed infinite height multiplier as the established day fullscreen/particle/water adapters. The complete effective stock/published shader layers are enumerated, including published aliases; every source archive contributing a shader is hash-verified and recorded in source-archives.json. Nonmatching forward permutations are not claimed adapted.".into()],
    };
    fs::write(args[5].join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    Ok(())
}
