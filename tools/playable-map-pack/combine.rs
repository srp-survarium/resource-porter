//! Consolidate frozen Factory/Chemical resources over the School database.
//! Isolate day/evening Factory resources: 81 materials differ from rain.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};
use vostok_resource_porter::{BinaryConfig, ResourceDb, binary_config::ConfigValue};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn rewrite(v: &mut ConfigValue) {
    match v {
        ConfigValue::String(s) => *s = s.replace("v010b", "v010w"),
        ConfigValue::Named(xs) => {
            for x in xs {
                rewrite(&mut x.value)
            }
        }
        ConfigValue::Indexed(xs) => {
            for x in xs {
                rewrite(x)
            }
        }
        _ => {}
    }
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn emit(
    root: &Path,
    path: &str,
    bytes: &[u8],
    records: &mut BTreeMap<String, serde_json::Value>,
    source: &str,
) -> Result<()> {
    if path.starts_with('/') || path.split('/').any(|p| p == "..") || records.contains_key(path) {
        return Err(format!("unsafe or duplicate output {path}").into());
    }
    let output = root.join(path);
    fs::create_dir_all(output.parent().unwrap())?;
    fs::write(output, bytes)?;
    records.insert(
        path.into(),
        serde_json::json!({"sha256":hash(bytes),"size":bytes.len(),"source":source}),
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 7 {
        return Err("usage: combine SCHOOL RAIN WEATHER CHEMICAL WORK OUTPUT".into());
    }
    let work = PathBuf::from(&args[5]);
    let output = PathBuf::from(&args[6]);
    if work.exists() || output.exists() {
        return Err("output/work already exists".into());
    }
    let overlay = work.join("overlay");
    fs::create_dir_all(&overlay)?;
    let mut records = BTreeMap::new();
    let mut base = ResourceDb::open(&args[1])?;
    let mut dictionary = BinaryConfig::parse(&base.read("gameplay/db_static_dictionaries")?)?;
    let ConfigValue::Indexed(rows) = dictionary
        .root
        .named_mut("maps_dict")
        .ok_or("no maps_dict")?
    else {
        return Err("maps_dict not array".into());
    };
    for (source, namespace, rename, ids) in [
        (&args[2], "v010b", false, vec![182]),
        (&args[3], "v010b", true, vec![180, 181]),
        (&args[4], "v011a", false, vec![193]),
    ] {
        let mut db = ResourceDb::open(source)?;
        let paths: Vec<_> = db
            .entries()
            .keys()
            .filter(|p| db.is_file(p) && p.contains(namespace) && !p.starts_with("gameplay/"))
            .cloned()
            .collect();
        let mut configs = 0;
        let mut changed = 0;
        for path in paths {
            let mut bytes = db.read(&path)?;
            let target = if rename {
                path.replace(namespace, "v010w")
            } else {
                path.clone()
            };
            if rename {
                if let Ok(mut config) = BinaryConfig::parse(&bytes) {
                    let old = config.clone();
                    rewrite(&mut config.root);
                    configs += 1;
                    if config != old {
                        bytes = config.to_bytes()?;
                        assert_eq!(BinaryConfig::parse(&bytes)?, config);
                        changed += 1;
                    }
                } else if bytes
                    .windows(namespace.len())
                    .any(|w| w == namespace.as_bytes())
                {
                    return Err(format!("unparsed namespace reference: {path}").into());
                }
            }
            emit(&overlay, &target, &bytes, &mut records, source)?;
        }
        let dict = BinaryConfig::parse(&db.read("gameplay/db_static_dictionaries")?)?;
        let ConfigValue::Indexed(source_rows) = dict
            .root
            .named("maps_dict")
            .ok_or("source maps_dict missing")?
        else {
            return Err("source maps_dict not array".into());
        };
        for id in ids {
            let found: Vec<_> = source_rows
                .iter()
                .filter(|r| r.named("id") == Some(&ConfigValue::Integer(id)))
                .collect();
            if found.len() != 1 {
                return Err(format!("missing/duplicate map {id}").into());
            }
            let mut row = found[0].clone();
            let ConfigValue::String(mode) = row.named("cfg_mode_name").ok_or("no cfg_mode_name")?
            else {
                return Err("bad mode name".into());
            };
            let mode_path = format!("gameplay/game_modes/{mode}");
            let mut mode_config = BinaryConfig::parse(&db.read(&mode_path)?)?;
            if rename {
                rewrite(&mut row);
                rewrite(&mut mode_config.root)
            }
            let bytes = mode_config.to_bytes()?;
            assert_eq!(BinaryConfig::parse(&bytes)?, mode_config);
            emit(
                &overlay,
                &if rename {
                    mode_path.replace(namespace, "v010w")
                } else {
                    mode_path
                },
                &bytes,
                &mut records,
                source,
            )?;
            rows.retain(|r| r.named("id") != Some(&ConfigValue::Integer(id)));
            rows.push(row);
        }
        println!("Selected {source}; parsed {configs} configs, rewrote {changed}");
    }
    let bytes = dictionary.to_bytes()?;
    assert_eq!(BinaryConfig::parse(&bytes)?, dictionary);
    emit(
        &overlay,
        "gameplay/db_static_dictionaries",
        &bytes,
        &mut records,
        "merged map registrations",
    )?;
    fs::write(
        work.join("selected-resources.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    let source = resources_db::pack::build_src_tree(&overlay)?;
    let overlay_db = work.join("overlay.db");
    println!("Packing {} selected resources", records.len());
    resources_db::pack::assemble_to_file(&source, &overlay_db)?;
    println!("Applying to preserved School base archive");
    let stats = resources_db::overlay::apply_db_overlay(Path::new(&args[1]), &overlay_db, &output)?;
    println!("{stats:?}");
    let mut result = ResourceDb::open(&output)?;
    for (path, record) in &records {
        let bytes = result.read(path)?;
        if hash(&bytes) != record["sha256"].as_str().unwrap() {
            return Err(format!("verification failed: {path}").into());
        }
    }
    // The existing School project and all its namespaced resources must remain exact.
    let school: Vec<_> = base
        .entries()
        .keys()
        .filter(|p| base.is_file(p) && p.contains("v021d"))
        .cloned()
        .collect();
    for path in &school {
        if base.read(path)? != result.read(path)? {
            return Err(format!("School changed: {path}").into());
        }
    }
    fs::write(
        work.join("verification.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"success":true,"selected_resources":records.len(),"school_resources_preserved":school.len(),"map_ids":[180,181,182,193,165],"factory_weather_namespace":"v010w","output":output}),
        )?,
    )?;
    println!(
        "Verified {} selected resources and {} unchanged School resources",
        records.len(),
        school.len()
    );
    Ok(())
}
