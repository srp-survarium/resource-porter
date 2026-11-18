// SPDX-License-Identifier: GPL-3.0-or-later
//! A ported map's weathers as a content pack: each weather project is the
//! shipped day with the donor's day-to-weather difference laid over it
//! (`weather_delta`), its mode config the shipped day's with the weather's
//! identity, its maps_dict row, and the donor files its new objects need that
//! players do not have.
//!
//! weather-delta-pack MANIFEST.json NEW_OUTPUT
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};
use vostok_resource_porter::{
    BinaryConfig, ResourceDb,
    binary_config::ConfigValue,
    content_pack::{self, Operation, PackManifest, TARGET_BUILD},
    dependency,
    weather_delta::{self, Variant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    pack: Pack,
    /// The donor's archives, the first at the bottom.
    donor: Vec<PathBuf>,
    /// What players have: the stock db, then every offered patch in order.
    shipped: Vec<PathBuf>,
    stock_sha256: String,
    map: String,
    weathers: Vec<Weather>,
    modes: Vec<Mode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pack {
    id: String,
    version: String,
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Weather {
    /// The donor's project folder prefix (`evn`, `rain`).
    donor: String,
    /// The shipped project folder prefix (`evening`, `rain`), as native maps.
    project: String,
    /// The mode config's infix (`evn`, `rain`), as native maps.
    options: String,
    layer: String,
    weather_id: i32,
    loading_screen_id: i32,
    /// A native 0.26g project of the same weather, for its environment's settings.
    native_environment_project: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mode {
    /// The project's number (`2` of `day_2`).
    number: u32,
    /// maps_dict id by weather (`project`).
    ids: BTreeMap<String, u16>,
}

fn cfg(db: &mut ResourceDb, path: &str) -> Result<ConfigValue> {
    Ok(BinaryConfig::parse(&db.read(path).with_context(|| format!("reading {path}"))?)
        .with_context(|| format!("parsing {path}"))?
        .root)
}

fn environment_of(project: &ConfigValue) -> Result<ConfigValue> {
    let Some(ConfigValue::Indexed(objects)) = project.named("client_objects") else {
        bail!("no client objects")
    };
    let mut found = objects
        .iter()
        .filter(|o| o.named("lib_name") == Some(&ConfigValue::String("environment".into())));
    let env = found.next().context("no environment")?.clone();
    ensure!(found.next().is_none(), "two environments");
    Ok(env)
}

/// The shipped day's mode config with the weather's identity.
fn weather_options(day: &ConfigValue, map: &str, project: &str, weather: &Weather) -> Result<ConfigValue> {
    let mut out = day.clone();
    out.set_path(
        "mode.match_making_params.map_weather_id",
        ConfigValue::Integer(weather.weather_id),
    )?;
    out.set_path(
        "mode.match_making_params.map_name",
        ConfigValue::String(format!("{map}/{project}")),
    )?;
    out.set_path("mode.ui.loading_screen_id", ConfigValue::Integer(weather.loading_screen_id))?;
    Ok(out)
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(args.len() == 2, "MANIFEST.json NEW_OUTPUT");
    let m: Manifest = serde_json::from_slice(&fs::read(&args[0])?)?;
    let output = &args[1];
    ensure!(!output.exists(), "output must be new");
    vostok_resource_porter::survarium_026::check_hash(&m.shipped[0], &m.stock_sha256)?;
    let mut donor = ResourceDb::open_layers(&m.donor)?;
    let mut shipped = ResourceDb::open_layers(&m.shipped)?;

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut dictionary = Vec::new();
    let mut summary = Map::new();
    let mut donor_files = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    let mut shadowed = BTreeSet::new();
    for weather in &m.weathers {
        let native = environment_of(&cfg(&mut shipped, &weather.native_environment_project)?)?;
        for mode in &m.modes {
            let n = mode.number;
            let project = format!("{}_{n}", weather.project);
            let id = *mode.ids.get(&weather.project).with_context(|| format!("no id for {project}"))?;
            let donor_day_path = format!("projects/{}/day_{n}/client_project", m.map);
            let donor_weather_path = format!("projects/{}/{}_{n}/client_project", m.map, weather.donor);
            let shipped_day = cfg(&mut shipped, &donor_day_path)?;
            let (built, report) = weather_delta::weather_project(
                &cfg(&mut donor, &donor_day_path)?,
                &cfg(&mut donor, &donor_weather_path)?,
                &shipped_day,
                &Variant {
                    project: &project,
                    layer: &weather.layer,
                    native_environment: &native,
                },
            )
            .with_context(|| format!("{}/{project}", m.map))?;
            let path = format!("projects/{}/{project}/client_project", m.map);
            ensure!(!shipped.contains(&path), "{path} is shipped already");
            let bytes = BinaryConfig { root: built }.to_bytes()?;
            BinaryConfig::parse(&bytes).context("the built project reads back")?;
            // The donor files its objects need that players do not have.
            let mut overrides = BTreeMap::new();
            overrides.insert(donor_weather_path.clone(), bytes.clone());
            let plan = dependency::scan_with_forced_donor_and_overrides(
                &mut donor,
                &mut shipped,
                [donor_weather_path.clone()],
                std::iter::empty(),
                &overrides,
            )?;
            for f in plan.donor_files.iter().filter(|f| **f != donor_weather_path) {
                ensure!(!f.starts_with("projects/"), "{f}: another project");
                donor_files.insert(f.clone());
            }
            unresolved.extend(plan.unresolved_references.iter().cloned());
            for f in &plan.target_files {
                if donor.contains(f) && !f.starts_with("projects/") && donor.read(f)? != shipped.read(f)? {
                    shadowed.insert(f.clone());
                }
            }
            files.insert(path.clone(), bytes);
            let options_name = format!("{}_{}_{n}.options", m.map, weather.options);
            let options_path = format!("gameplay/game_modes/{options_name}");
            ensure!(!shipped.contains(&options_path), "{options_path} is shipped already");
            let day_options = cfg(&mut shipped, &format!("gameplay/game_modes/{}_day_{n}.options", m.map))?;
            files.insert(
                options_path,
                BinaryConfig { root: weather_options(&day_options, &m.map, &project, weather)? }.to_bytes()?,
            );
            dictionary.push(match json!({"id": id, "cfg_mode_name": options_name}) {
                Json::Object(o) => o,
                _ => unreachable!(),
            });
            summary.insert(path, serde_json::to_value(&report)?);
            println!(
                "{}/{project} (maps_dict {id}): removed {:?}, changed {:?}, added {:?}, left out {}, conflicts {}, visuals -{} +{}, collisions -{} +{}, lamps renumbered {}, environment {} settings from the donor",
                m.map, report.removed, report.changed, report.added, report.left_out.len(), report.conflicts.len(),
                report.render_visuals.0, report.render_visuals.1, report.collision_objects.0, report.collision_objects.1,
                report.lamps_renumbered, report.environment_properties
            );
        }
    }
    for f in &donor_files {
        files.insert(f.clone(), donor.read(f)?);
    }
    // References the shipped day leaves unresolved too are no new gap.
    let mut baseline = BTreeSet::new();
    for mode in &m.modes {
        let day = format!("projects/{}/day_{}/client_project", m.map, mode.number);
        let mut overrides = BTreeMap::new();
        overrides.insert(day.clone(), shipped.read(&day)?);
        let plan = dependency::scan_with_forced_donor_and_overrides(&mut donor, &mut shipped, [day.clone()], std::iter::empty(), &overrides)?;
        baseline.extend(plan.unresolved_references);
    }
    let new_gaps: Vec<_> = unresolved.difference(&baseline).collect();
    ensure!(new_gaps.is_empty(), "unresolved references the shipped day resolves: {new_gaps:?}");

    // The shipped day's own conversion, checked against the rules used here.
    let mut check = Map::new();
    for mode in &m.modes {
        let path = format!("projects/{}/day_{}/client_project", m.map, mode.number);
        let differs = weather_delta::conversion_check(&cfg(&mut donor, &path)?, &cfg(&mut shipped, &path)?, &format!("day_{}", mode.number))?;
        check.insert(path, serde_json::to_value(differs)?);
    }

    let mut operations = Vec::new();
    for (path, bytes) in &files {
        let at = vostok_resource_porter::archive::safe_join(&output.join("files"), path)?;
        fs::create_dir_all(at.parent().context("parent")?)?;
        fs::write(&at, bytes)?;
        operations.push(Operation::AddFile { path: path.clone(), source: None });
    }
    operations.push(Operation::DictionaryAdd {
        table: "maps_dict".into(),
        id_field: None,
        entries: dictionary,
    });
    let pack = PackManifest {
        format_version: 1,
        id: m.pack.id.clone(),
        version: m.pack.version.clone(),
        target_build: TARGET_BUILD.into(),
        description: Some(m.pack.description.clone()),
        requires: vec![],
        base_db_sha256: Some(m.stock_sha256.clone()),
        operations,
        server: Some(json!({"maps": m.modes.iter().flat_map(|md| md.ids.values().map(|id| json!({"id": id}))).collect::<Vec<_>>()})),
        notes: vec![format!(
            "Built by examples/weather-delta-pack.rs from the donor's own day and weather projects of {} over the projects players have: {} donor files ({} referenced paths players have in another form keep theirs).",
            m.map,
            donor_files.len(),
            shadowed.len()
        )],
    };
    fs::write(output.join("pack.json"), serde_json::to_vec_pretty(&pack)?)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({"projects": summary, "day_conversion_check": check, "shadowed": shadowed}))?,
    )?;
    let _ = content_pack::DICTIONARY;
    println!("{} files ({} donor), {} referenced paths kept as players have them", files.len(), donor_files.len(), shadowed.len());
    Ok(())
}
