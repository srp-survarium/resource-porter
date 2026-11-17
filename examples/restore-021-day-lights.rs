//! Restore donor lighting through source edit lists, preserving later gameplay edits.
//! DONOR_DB STOCK_DB COMPOSED_LAYERS_JSON OUTPUT_DIR
use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use vostok_resource_porter::content_pack::{Operation, PackManifest, TARGET_BUILD, sha256_hex};
use vostok_resource_porter::survarium_026::{check_hash, original_lighting_objects};
use vostok_resource_porter::{BinaryConfig, ConfigValue, ResourceDb};

fn array(value: &ConfigValue) -> Result<&[ConfigValue]> {
    let ConfigValue::Indexed(values) = value else {
        anyhow::bail!("expected indexed table")
    };
    Ok(values)
}

fn text<'a>(value: &'a ConfigValue, key: &str) -> Result<&'a str> {
    let Some(ConfigValue::String(value)) = value.named(key) else {
        anyhow::bail!("missing string {key}")
    };
    Ok(value)
}

fn lighting(value: &ConfigValue) -> bool {
    matches!(
        text(value, "game_object_type"),
        Ok("light" | "ambient_light" | "environment_probe")
    )
}

// content_edit's vectors=true representation: preserve scalar types and reject
// indexed numeric tuples that would otherwise be mistaken for vectors.
fn edit_value(value: &ConfigValue) -> Result<Value> {
    Ok(match value {
        ConfigValue::Boolean(v) => json!(v),
        ConfigValue::Integer(v) => json!(v),
        ConfigValue::Float(v) => {
            ensure!(v.is_finite(), "nonfinite float");
            json!(v)
        }
        ConfigValue::String(v) => json!(v),
        ConfigValue::Float2(v) => json!(v),
        ConfigValue::Float3(v) => json!(v),
        ConfigValue::Float4(v) => json!(v),
        ConfigValue::Named(fields) => {
            let mut result = serde_json::Map::new();
            for field in fields {
                ensure!(
                    result
                        .insert(field.name.clone(), edit_value(&field.value)?)
                        .is_none(),
                    "duplicate field"
                );
            }
            Value::Object(result)
        }
        ConfigValue::Indexed(values) => {
            ensure!(
                !(2..=4).contains(&values.len())
                    || !values
                        .iter()
                        .all(|v| matches!(v, ConfigValue::Float(_) | ConfigValue::Integer(_))),
                "ambiguous indexed numeric tuple"
            );
            Value::Array(values.iter().map(edit_value).collect::<Result<_>>()?)
        }
    })
}

fn rewrite(value: &mut ConfigValue, names: &BTreeMap<String, String>) {
    match value {
        ConfigValue::String(v) => {
            if let Some(new) = names.get(v) {
                *v = new.clone();
            }
        }
        ConfigValue::Named(fields) => {
            for field in fields {
                rewrite(&mut field.value, names);
            }
        }
        ConfigValue::Indexed(values) => {
            for value in values {
                rewrite(value, names);
            }
        }
        _ => (),
    }
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() == 4,
        "DONOR_DB STOCK_DB COMPOSED_LAYERS_JSON OUTPUT_DIR"
    );
    check_hash(
        &args[0],
        "e28cc2878e0e2f04ab9ab0a1861034622af487a491bf230ea24f44e9af9715a4",
    )?;
    let stock_hash = "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c";
    check_hash(&args[1], stock_hash)?;
    ensure!(!args[3].exists(), "output must be new");
    let mut donor = ResourceDb::open(&args[0])?;
    let mut stock = ResourceDb::open(&args[1])?;
    let layers: Vec<PathBuf> = serde_json::from_slice(&fs::read(&args[2])?)?;
    ensure!(
        layers.first() == Some(&args[1]),
        "composed layers must start with pinned stock"
    );
    let mut current = ResourceDb::open_layers(&layers)?;
    let mut resources = BTreeMap::new();
    let mut remove = Vec::new();
    let mut append = Vec::new();
    let mut environments = Vec::new();
    let mut lamps = Vec::new();
    for level in ["level_03", "level_06"] {
        let source =
            BinaryConfig::parse(&donor.read(&format!("projects/{level}/day/client_project"))?)?;
        let native =
            BinaryConfig::parse(&stock.read(&format!("projects/{level}/day_2/client_project"))?)?;
        let mut restored = original_lighting_objects(&source.root, &native.root)?;
        let mut strings = Vec::new();
        for object in array(&restored)? {
            if text(object, "game_object_type")? == "environment_probe" {
                let textures = object.named("textures").context("probe textures")?;
                for key in ["day", "day_2", "day_4", "day_8"] {
                    ensure!(
                        donor.is_file(&format!("textures/{}.dds", text(textures, key)?)),
                        "active day probe has no donor cubemap"
                    );
                }
                textures.collect_strings(&mut strings);
            }
        }
        let mut names = BTreeMap::new();
        for name in strings
            .into_iter()
            // Retail's unused default placeholder has no texture resource.
            // All active day bindings above must use real authored cubemaps.
            .filter(|name| {
                name.starts_with("environment_probes/")
                    && *name != "environment_probes/environment_probe"
            })
        {
            // Retail also carries unused default aliases whose textures are
            // absent. Preserve those aliases, as the normal dependency porter
            // does; active day aliases were required to resolve above.
            if donor.is_file(&format!("textures/{name}.dds")) {
                names.insert(name.to_owned(), format!("ported/v021d/{name}"));
            } else {
                eprintln!("retained unused retail probe alias: {name}");
            }
        }
        for (name, destination) in &names {
            let base = format!("textures/{name}");
            for path in vostok_resource_porter::dependency::texture_companions(&base, &donor, &stock) {
                ensure!(donor.is_file(&path), "missing donor probe companion {path}");
                let suffix = path.strip_prefix(&base).context("probe companion base")?;
                let destination = format!("textures/{destination}{suffix}");
                let bytes = donor.read(&path)?;
                if current.is_file(&destination) {
                    ensure!(
                        current.read(&destination)? == bytes,
                        "existing donor texture differs: {destination}"
                    );
                } else {
                    resources.insert(destination, bytes);
                }
            }
        }
        rewrite(&mut restored, &names);
        let restored = array(&restored)?;
        ensure!(!restored.is_empty(), "no donor lighting");
        let mut files = Vec::new();
        let mut counts = Vec::new();
        for mode in [2, 4, 8] {
            let path = format!("projects/v021d_{level}/day_{mode}/client_project");
            let before = BinaryConfig::parse(&current.read(&path)?)?;
            let objects = array(
                before
                    .root
                    .named("client_objects")
                    .context("client objects")?,
            )?;
            let remaining: Vec<_> = objects.iter().filter(|object| !lighting(object)).collect();
            for object in objects.iter().filter(|object| lighting(object)) {
                remove.push(json!({"file":path, "list":"client_objects", "where":{
                    "game_object_type":text(object,"game_object_type")?, "full_name":text(object,"full_name")?}}));
            }
            files.push(path.clone());
            counts.push(remaining.len());
            if let Some(config) = before.root.named("srp_lamps") {
                lamps.push(json!({"file":path,"lamps":edit_value(config.named("lamps").context("lamp rows")?)?}));
            }
            let envs: Vec<_> = remaining
                .iter()
                .enumerate()
                .filter(|(_, object)| text(object, "game_object_type").ok() == Some("environment"))
                .collect();
            ensure!(envs.len() == 1, "exactly one current environment required");
            let properties = envs[0]
                .1
                .named("properties")
                .context("environment properties")?;
            let height = match properties.named("mie_fog_base_height") {
                Some(ConfigValue::Float(v)) => Some(f64::from(*v)),
                Some(ConfigValue::Integer(v)) => Some(f64::from(*v)),
                None => None,
                _ => anyhow::bail!("unexpected fog-height type"),
            };
            ensure!(
                height.is_none_or(|value| value >= -32021.0),
                "input already selects legacy shaders; compose inputs before these restoration edits"
            );
            let originals: Vec<_> = array(
                source
                    .root
                    .named("client_objects")
                    .context("donor objects")?,
            )?
            .iter()
            .filter(|object| text(object, "game_object_type").ok() == Some("environment"))
            .collect();
            ensure!(
                originals.len() == 1,
                "exactly one original environment required"
            );
            environments.push(json!({"level":level,"mode":mode,"index":envs[0].0,
                "before":edit_value(envs[0].1.named("properties").context("environment properties")?)?,
                "source":edit_value(originals[0].named("properties").context("donor environment properties")?)?}));
            println!(
                "{path}: {} native lighting objects -> {} original; other {} client objects retained",
                objects.len() - remaining.len(),
                restored.len(),
                remaining.len()
            );
        }
        for (index, object) in restored.iter().enumerate() {
            let expected: Vec<_> = counts.iter().map(|count| count + index).collect();
            append.push(
                json!({"files":files,"list":"client_objects","expected_counts":expected,
                "unique_key":"full_name","vectors":true,"value":edit_value(object)?}),
            );
        }
    }
    fs::create_dir_all(&args[3])?;
    let mut operations = Vec::new();
    for (path, bytes) in resources {
        let output = args[3].join("files").join(&path);
        fs::create_dir_all(output.parent().context("resource parent")?)?;
        fs::write(output, &bytes)?;
        println!("{path}: {}", sha256_hex(&bytes));
        operations.push(Operation::AddFile { path, source: None });
    }
    let pack = PackManifest {
        format_version:1, id:"maps-021d-original-probes".into(), version:"2".into(), target_build:TARGET_BUILD.into(),
        description:Some("Original day probe textures for Vostok and Mamayev; lighting objects restored by source edit lists".into()),
        requires:vec![], base_db_sha256:Some(stock_hash.into()), operations, server:None,
        notes:vec!["Donor textures only. No project, geometry or gameplay resource is replaced by this pack. The older maps-021d content is supplied by the manifest's published base, not an active pack dependency; the companion source edits require its projects and exact object identities.".into()],
    };
    for (name, data) in [
        ("pack.json", serde_json::to_value(pack)?),
        (
            "v021d-two-maps-native-lights-remove-1.json",
            json!({"_about":"Remove the earlier native-lighting substitution on the two older 0.21 ports. Follow immediately with v021d-two-maps-original-lights-1.json; source: resource-porter examples/restore-021-day-lights.rs.","remove_entry":remove}),
        ),
        (
            "v021d-two-maps-original-lights-1.json",
            json!({"_about":"Original 0.21 lights and probes, shared native schema conversion and donor cubemap aliases. No intensity/tint tuning. Source: resource-porter examples/restore-021-day-lights.rs.","append_entry":append}),
        ),
        ("environments.json", json!(environments)),
        ("lamps.json", json!(lamps)),
    ] {
        fs::write(args[3].join(name), serde_json::to_vec_pretty(&data)?)?;
    }
    Ok(())
}
