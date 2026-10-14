//! Audit original battery transforms and repair collector visual bindings.
//! Writes only a small project overlay; never mutates the database.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};
use vostok_resource_porter::{
    BinaryConfig, ConfigValue as V, ResourceDb,
    survarium_026::{collector_visual, repair_chemical_collector_zone},
};
fn array(v: &V) -> Result<&[V]> {
    match v {
        V::Indexed(v) => Ok(v),
        _ => anyhow::bail!("expected array"),
    }
}
fn field<'a>(v: &'a V, key: &str) -> Result<&'a V> {
    v.named(key).with_context(|| format!("missing {key}"))
}
fn string<'a>(v: &'a V, key: &str) -> Result<&'a str> {
    match field(v, key)? {
        V::String(s) => Ok(s),
        _ => anyhow::bail!("string {key}"),
    }
}
fn config(db: &mut ResourceDb, path: &str) -> Result<V> {
    Ok(BinaryConfig::parse(&db.read(path)?)?.root)
}
fn shape(v: &V) -> Result<V> {
    let mut out = V::Named(vec![]);
    for key in ["position", "rotation", "scale", "lib_name"] {
        out.set_path(key, field(v, key)?.clone())?;
    }
    Ok(out)
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn emit_project_change(
    output: &Path,
    path: &str,
    before: &[u8],
    project: &V,
) -> Result<Option<Vec<u8>>> {
    if BinaryConfig::parse(before)?.root == *project {
        return Ok(None);
    }
    let bytes = BinaryConfig {
        root: project.clone(),
    }
    .to_bytes()?;
    ensure!(
        BinaryConfig::parse(&bytes)?.root == *project,
        "round-trip mismatch"
    );
    let file = output.join("overlay").join(path);
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(file, &bytes)?;
    Ok(Some(bytes))
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    ensure!(
        args.len() == 5,
        "usage: repair TARGET_DB FACTORY_SOURCE_DB CHEMICAL_SOURCE_DB OUTPUT"
    );
    let output = Path::new(&args[4]);
    ensure!(!output.exists(), "output exists");
    let mut target = ResourceDb::open(&args[1])?;
    let mut reports = vec![];
    for (name, source, level, namespace) in [
        ("factory", &args[2], "level_03", "v010w"),
        ("chemical", &args[3], "level_04", "v011a"),
    ] {
        let mut donor = ResourceDb::open(source)?;
        let client = config(&mut donor, &format!("projects/{level}/client_project"))?;
        let server = config(&mut donor, &format!("projects/{level}/server_project"))?;
        let mut original_groups = field(&server, "objects")?.clone();
        let path = format!("projects/{namespace}_{level}/day_2/client_project");
        let before = target.read(&path)?;
        let mut project = BinaryConfig::parse(&before)?.root;
        let mut groups = field(&project, "server_objects")?.clone();
        let original_spawns = array(field(&original_groups, "victory_item_spawners")?)?;
        let actual_spawns = array(field(&groups, "victory_item_spawners")?)?;
        ensure!(
            original_spawns.len() == actual_spawns.len(),
            "spawner count mismatch"
        );
        for (old, new) in original_spawns.iter().zip(actual_spawns) {
            for key in ["point_id", "priority", "position", "rotation", "full_name"] {
                ensure!(
                    field(old, key)? == field(new, key)?,
                    "{name} spawner {key} drift"
                );
            }
            for key in ["position", "rotation"] {
                ensure!(
                    field(old, key)? == field(field(new, "spawned_item")?, key)?,
                    "{name} nested spawner {key} drift"
                );
            }
        }
        let spawner_count = original_spawns.len();
        let zone_fixed = if name == "chemical" {
            let objects = array(field(&client, "objects")?)?;
            repair_chemical_collector_zone(&mut original_groups, objects)?;
            repair_chemical_collector_zone(&mut groups, objects)?
        } else {
            false
        };
        let source_collectors = array(field(&original_groups, "victory_items_containers")?)?;
        let source_objects = array(field(&client, "objects")?)?;
        let source_visuals = array(field(&client, "render_visuals")?)?;
        let mut visuals = array(field(&project, "render_visuals")?)?.to_vec();
        let mut selected = vec![];
        let mut bindings = vec![];
        let V::Indexed(collectors) = groups
            .named_mut("victory_items_containers")
            .context("collectors")?
        else {
            anyhow::bail!("collectors array")
        };
        let mut changed = 0;
        for collector in collectors.iter_mut() {
            let full_name = string(collector, "full_name")?.to_owned();
            let original = source_collectors
                .iter()
                .find(|o| string(o, "full_name").ok() == Some(&full_name))
                .context("matching original collector")?;
            for key in ["collision_geometries", "id", "team", "position", "rotation"] {
                ensure!(
                    field(original, key)? == field(collector, key)?,
                    "{name} collector {key} drift"
                );
            }
            let mut chosen = collector_visual(original, source_objects, source_visuals)?.clone();
            chosen.set_path(
                "lib_name",
                V::String(format!(
                    "ported/{namespace}/{}",
                    string(&chosen, "lib_name")?
                )),
            )?;
            ensure!(
                !selected.contains(&shape(&chosen)?),
                "duplicate selected mesh"
            );
            selected.push(shape(&chosen)?);
            let old = field(collector, "visual_geometry")?.clone();
            let mut geometry = old.clone();
            for key in ["position", "rotation", "scale", "lib_name"] {
                geometry.set_path(key, field(&chosen, key)?.clone())?;
            }
            geometry.set_path("model_name", field(&chosen, "lib_name")?.clone())?;
            if geometry != old {
                changed += 1;
            }
            bindings.push(serde_json::json!({"name":full_name,"before_position":format!("{:?}",field(&old,"position")?),"after_position":format!("{:?}",field(&geometry,"position")?),"team":format!("{:?}",field(collector,"team")?)}));
            collector.set_path("visual_geometry", geometry)?;
        }
        // Previously bound meshes must still be owned by a collector. Do not silently
        // lose unrelated geometry when repairing a different input package.
        for old in array(field(
            field(&project, "server_objects")?,
            "victory_items_containers",
        )?)? {
            ensure!(
                selected.contains(&shape(field(old, "visual_geometry")?)?),
                "old bound mesh would be lost"
            );
        }
        let prior_count = visuals.len();
        visuals.retain(|v| !selected.contains(&shape(v).expect("visual shape")));
        project.set_path("render_visuals", V::Indexed(visuals))?;
        project.set_path("server_objects", groups)?;
        let removed_visuals = prior_count - array(field(&project, "render_visuals")?)?.len();
        let emitted = emit_project_change(output, &path, &before, &project)?;
        ensure!(
            emitted.is_some() == (zone_fixed || changed > 0 || removed_visuals > 0),
            "project changes differ from the reported objective repairs"
        );
        reports.push(serde_json::json!({"map":name,"source_db":source,"project":path,"authored_battery_spawns_verified":spawner_count,"collector_changes":changed,"collector_zone_repaired":zone_fixed,"removed_duplicate_static_visuals":removed_visuals,"before_sha256":hash(&before),"after_sha256":hash(emitted.as_deref().unwrap_or(&before)),"bindings":bindings}));
    }
    fs::create_dir_all(output)?;
    fs::write(
        output.join("audit.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zone_only_change_is_written_and_a_repaired_project_is_not() {
        let mut original = V::Named(vec![]);
        original.set_path("collector", V::Named(vec![])).unwrap();
        original
            .set_path("collector.visual_geometry", V::Named(vec![]))
            .unwrap();
        original
            .set_path(
                "collector.visual_geometry.position",
                V::Float3([102.664, 6.54, 79.938]),
            )
            .unwrap();
        original
            .set_path(
                "collector.collision_zone",
                V::String("logic/team1/victory_container_collision".into()),
            )
            .unwrap();
        let before = BinaryConfig {
            root: original.clone(),
        }
        .to_bytes()
        .unwrap();
        let original = BinaryConfig::parse(&before).unwrap().root;
        let mut fixed = original.clone();
        fixed
            .set_path(
                "collector.collision_zone",
                V::String("logic/team2/victory_container_collision".into()),
            )
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = "projects/chemical/client_project";
        let bytes = emit_project_change(temp.path(), path, &before, &fixed)
            .unwrap()
            .unwrap();
        assert_eq!(
            fs::read(temp.path().join("overlay").join(path)).unwrap(),
            bytes
        );
        assert_eq!(BinaryConfig::parse(&bytes).unwrap().root, fixed);
        assert_eq!(
            field(field(&original, "collector").unwrap(), "visual_geometry").unwrap(),
            field(field(&fixed, "collector").unwrap(), "visual_geometry").unwrap()
        );
        let repeated = tempfile::tempdir().unwrap();
        assert!(
            emit_project_change(repeated.path(), path, &bytes, &fixed)
                .unwrap()
                .is_none()
        );
        assert!(!repeated.path().join("overlay").exists());
    }
}
