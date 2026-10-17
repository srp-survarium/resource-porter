//! Reviewed edits of a content pack already built, in place: the projects'
//! objects and spawn points and the mode options, by a plan file, so that a
//! pack version can be derived from the one before with exactly those
//! changes (each one's result is printed for the record).
//!
//!     cargo run --release --example edit-built-pack -- PACK_DIR PLAN.json STOCK_DB
//!
//! PLAN.json: `{"projects": {"<path>": [op, ...]}, "options": {"<path>": {"<leaf.path>": value}}}`.
//! Ops, in order:
//! - `{"op": "remove_instances", "model": M, "near": [x,y,z], "radius": r}`: the render visuals and
//!   collision objects of model M within r of the point go;
//! - `{"op": "move", "anchor": [x,y,z], "to": {position, yaw, pitch, roll}, "client_objects": [full
//!   name, ...], "container_visuals": [container full name, ...], "instances": [{"model", "near",
//!   "radius"}, ...]}`: one rigid move (anchor to position, the yaw/pitch/roll turn about it) of
//!   those client objects, those victory containers' `visual_geometry` and those model instances;
//!   with `to.from_rotation` (the main object's rotation now) the turn is the one that makes that
//!   rotation level at `to.yaw`;
//! - `{"op": "respawn_move", "full_name": N, "position": [x,y,z], "yaw": a}`: with `"from": [x,y,z]`
//!   the point must be there now (within 1 mm), or already at `position` (then nothing changes and it
//!   says so); without `yaw` its rotation stays;
//! - `{"op": "respawn_zone", "full_name": N, "kind": "ally_zones"|"enemy_zones", "from": Z, "to": Z2}`:
//!   the point names zone Z2 where it named Z (Z2 must be a collision_geometry client object); already
//!   naming Z2: nothing changes;
//! - `{"op": "zone_move", "full_name": N, "from": [x,y,z], "position": [x,y,z]}`: a collision_geometry
//!   client object (a respawn point's zone) moves, its rotation and meshes as they are; `from` as above;
//! - `{"op": "respawn_add", "like": N, "full_name": N2, "position": [x,y,z], "yaw": a}`: a copy of
//!   respawn point N (its team, zones and settings) with the next free point_id;
//! - `{"op": "remove_client_prefix", "prefix": P}`: client objects whose full name starts with P go;
//! - `{"op": "fill", "pack": FILE}`: a gameplay-fill pack (`survarium_026::fill`), its native
//!   template project read from STOCK_DB and checked against its hash.
//! Every op must change something; a respawn yaw is written as rotation [0, yaw, 0].
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use vostok_resource_porter::binary_config::{BinaryConfig, ConfigValue};
use vostok_resource_porter::skeleton_animation::{basis_to_euler, euler_to_basis, multiply};
use vostok_resource_porter::survarium_026::fill;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    #[serde(default)]
    projects: BTreeMap<String, Vec<Op>>,
    #[serde(default)]
    options: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct To {
    position: [f32; 3],
    #[serde(default)]
    yaw: f32,
    #[serde(default)]
    pitch: f32,
    #[serde(default)]
    roll: f32,
    /// The rotation the group's main object has now: the turn is then the one that takes it to a level
    /// rotation [0, yaw, 0] (pitch and roll unused), so a tilted object ends up level.
    #[serde(default)]
    from_rotation: Option<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instances {
    model: String,
    near: [f32; 3],
    radius: f32,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Op {
    RemoveInstances { model: String, near: [f32; 3], radius: f32 },
    Move {
        anchor: [f32; 3],
        to: To,
        #[serde(default)]
        client_objects: Vec<String>,
        #[serde(default)]
        container_visuals: Vec<String>,
        #[serde(default)]
        instances: Vec<Instances>,
    },
    RespawnMove {
        full_name: String,
        position: [f32; 3],
        #[serde(default)]
        yaw: Option<f32>,
        #[serde(default)]
        from: Option<[f32; 3]>,
        #[serde(default)]
        why: Option<String>,
    },
    RespawnZone { full_name: String, kind: String, from: String, to: String, #[serde(default)] why: Option<String> },
    ZoneMove { full_name: String, from: [f32; 3], position: [f32; 3], #[serde(default)] why: Option<String> },
    RespawnAdd { like: String, full_name: String, position: [f32; 3], yaw: f32 },
    RemoveClientPrefix { prefix: String },
    Fill { pack: String },
}

fn f3(o: &ConfigValue, key: &str) -> Option<[f32; 3]> {
    match o.named(key) {
        Some(ConfigValue::Float3(v)) => Some(*v),
        _ => None,
    }
}

fn s<'a>(o: &'a ConfigValue, key: &str) -> Option<&'a str> {
    match o.named(key) {
        Some(ConfigValue::String(v)) => Some(v),
        _ => None,
    }
}

fn items<'a>(root: &'a mut ConfigValue, key: &str) -> Result<&'a mut Vec<ConfigValue>> {
    match root.named_mut(key) {
        Some(ConfigValue::Indexed(v)) => Ok(v),
        _ => bail!("{key} is not an array"),
    }
}

fn respawns(root: &mut ConfigValue) -> Result<&mut Vec<ConfigValue>> {
    match root.named_mut("server_objects").and_then(|g| g.named_mut("respawn_points")) {
        Some(ConfigValue::Indexed(v)) => Ok(v),
        _ => bail!("no server_objects.respawn_points"),
    }
}

fn model_of(o: &ConfigValue) -> Option<String> {
    let name = s(o, "lib_name").or_else(|| s(o, "model_name"))?;
    let name = name.trim_start_matches("resources/models/");
    Some(name.split(".model/").next().unwrap_or(name).to_owned())
}

fn within(o: &ConfigValue, at: [f32; 3], r: f32) -> bool {
    f3(o, "position").is_some_and(|p| (0..3).map(|i| (p[i] - at[i]).powi(2)).sum::<f32>() <= r * r)
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= 1e-3)
}

fn transpose(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| m[j][i]))
}

fn moved(o: &mut ConfigValue, anchor: [f32; 3], to: To) -> Result<()> {
    let turn = match to.from_rotation {
        Some(r) => multiply(&transpose(&euler_to_basis(r)), &euler_to_basis([0.0, to.yaw, 0.0])),
        None => euler_to_basis([to.pitch, to.yaw, to.roll]),
    };
    if let Some(p) = f3(o, "position") {
        let local = [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
        let q: [f32; 3] = std::array::from_fn(|j| (0..3).map(|i| local[i] * turn[i][j]).sum::<f32>() + to.position[j]);
        o.set_path("position", ConfigValue::Float3(q))?;
    }
    if let Some(r) = f3(o, "rotation") {
        o.set_path("rotation", ConfigValue::Float3(basis_to_euler(&multiply(&euler_to_basis(r), &turn))))?;
    }
    Ok(())
}

fn json_leaf(v: &serde_json::Value, like: Option<&ConfigValue>) -> Result<ConfigValue> {
    use serde_json::Value as J;
    Ok(match (v, like) {
        (J::Number(n), Some(ConfigValue::Integer(_))) => ConfigValue::Integer(n.as_i64().context("integer")? as i32),
        (J::Number(n), Some(ConfigValue::Float(_))) => ConfigValue::Float(n.as_f64().unwrap() as f32),
        (J::String(t), Some(ConfigValue::String(_))) => ConfigValue::String(t.clone()),
        (J::Bool(b), Some(ConfigValue::Boolean(_))) => ConfigValue::Boolean(*b),
        (v, like) => bail!("{v} cannot replace {like:?}"),
    })
}

fn get<'a>(root: &'a ConfigValue, path: &str) -> Option<&'a ConfigValue> {
    path.split('.').try_fold(root, |v, k| v.named(k))
}

fn apply(root: &mut ConfigValue, op: &Op, stock: &mut vostok_resource_porter::archive::ResourceDb) -> Result<String> {
    Ok(match op {
        Op::RemoveInstances { model, near, radius } => {
            let mut gone = 0;
            for key in ["render_visuals", "collision_objects"] {
                let v = items(root, key)?;
                let before = v.len();
                v.retain(|o| !(model_of(o).as_deref() == Some(model.as_str()) && within(o, *near, *radius)));
                gone += before - v.len();
            }
            ensure!(gone > 0, "no {model} within {radius} m of {near:?}");
            format!("removed {gone} instances of {model} near {near:?}")
        }
        Op::Move { anchor, to, client_objects, container_visuals, instances } => {
            let mut n = 0;
            for name in client_objects {
                let o = items(root, "client_objects")?
                    .iter_mut()
                    .find(|o| s(o, "full_name") == Some(name.as_str()))
                    .with_context(|| format!("no client object {name}"))?;
                moved(o, *anchor, *to)?;
                n += 1;
            }
            for name in container_visuals {
                let ConfigValue::Indexed(cs) = root
                    .named_mut("server_objects")
                    .and_then(|g| g.named_mut("victory_items_containers"))
                    .context("no victory_items_containers")?
                else {
                    bail!("victory_items_containers is not an array")
                };
                let c = cs.iter_mut().find(|c| s(c, "full_name") == Some(name.as_str())).with_context(|| format!("no container {name}"))?;
                moved(c.named_mut("visual_geometry").context("no visual_geometry")?, *anchor, *to)?;
                n += 1;
            }
            for inst in instances {
                let mut found = 0;
                for key in ["render_visuals", "collision_objects"] {
                    for o in items(root, key)?.iter_mut() {
                        if model_of(o).as_deref() == Some(inst.model.as_str()) && within(o, inst.near, inst.radius) {
                            moved(o, *anchor, *to)?;
                            found += 1;
                        }
                    }
                }
                ensure!(found > 0, "no {} within {} m of {:?}", inst.model, inst.radius, inst.near);
                n += found;
            }
            format!("moved {n} objects from {anchor:?} to {:?} (yaw {})", to.position, to.yaw)
        }
        Op::RespawnMove { full_name, position, yaw, from, why } => {
            let p = respawns(root)?
                .iter_mut()
                .find(|p| s(p, "full_name") == Some(full_name.as_str()))
                .with_context(|| format!("no respawn point {full_name}"))?;
            let was = f3(p, "position").context("respawn point without a position")?;
            if let Some(from) = from {
                if near(was, *position) && yaw.is_none() {
                    return Ok(format!("respawn {full_name}: already at {position:?}"));
                }
                ensure!(near(was, *from), "respawn {full_name} is at {was:?}, not {from:?}: the pack is not the one this plan was made for");
            }
            p.set_path("position", ConfigValue::Float3(*position))?;
            if let Some(yaw) = yaw {
                p.set_path("rotation", ConfigValue::Float3([0.0, *yaw, 0.0]))?;
            }
            format!("respawn {full_name}: {was:?} -> {position:?}{}{}", yaw.map(|y| format!(" yaw {y}")).unwrap_or_default(),
                why.as_deref().map(|w| format!(" ({w})")).unwrap_or_default())
        }
        Op::RespawnZone { full_name, kind, from, to, why } => {
            ensure!(kind == "ally_zones" || kind == "enemy_zones", "zone kind {kind}");
            ensure!(
                items(root, "client_objects")?
                    .iter()
                    .any(|o| s(o, "full_name") == Some(to.as_str()) && s(o, "game_object_type") == Some("collision_geometry")),
                "no collision_geometry {to}"
            );
            let p = respawns(root)?
                .iter_mut()
                .find(|p| s(p, "full_name") == Some(full_name.as_str()))
                .with_context(|| format!("no respawn point {full_name}"))?;
            let Some(ConfigValue::Indexed(zs)) = p.named_mut(kind).and_then(|k| k.named_mut("collision_geometries")) else {
                bail!("respawn {full_name} has no {kind}")
            };
            if zs.iter().any(|z| s(z, "name") == Some(to.as_str())) && !zs.iter().any(|z| s(z, "name") == Some(from.as_str())) {
                return Ok(format!("respawn {full_name}: {kind} already names {to}"));
            }
            let z = zs
                .iter_mut()
                .find(|z| s(z, "name") == Some(from.as_str()))
                .with_context(|| format!("respawn {full_name}: {kind} does not name {from}: the pack is not the one this plan was made for"))?;
            z.set_path("name", ConfigValue::String(to.clone()))?;
            format!("respawn {full_name}: {kind} {from} -> {to}{}", why.as_deref().map(|w| format!(" ({w})")).unwrap_or_default())
        }
        Op::ZoneMove { full_name, from, position, why } => {
            let o = items(root, "client_objects")?
                .iter_mut()
                .find(|o| s(o, "full_name") == Some(full_name.as_str()))
                .with_context(|| format!("no client object {full_name}"))?;
            ensure!(s(o, "game_object_type") == Some("collision_geometry"), "{full_name} is not a collision_geometry");
            let was = f3(o, "position").context("zone without a position")?;
            if near(was, *position) {
                return Ok(format!("zone {full_name}: already at {position:?}"));
            }
            ensure!(near(was, *from), "zone {full_name} is at {was:?}, not {from:?}: the pack is not the one this plan was made for");
            o.set_path("position", ConfigValue::Float3(*position))?;
            format!("zone {full_name}: {was:?} -> {position:?}{}", why.as_deref().map(|w| format!(" ({w})")).unwrap_or_default())
        }
        Op::RespawnAdd { like, full_name, position, yaw } => {
            let points = respawns(root)?;
            ensure!(!points.iter().any(|p| s(p, "full_name") == Some(full_name.as_str())), "{full_name} exists");
            let next = points
                .iter()
                .filter_map(|p| match p.named("point_id") {
                    Some(ConfigValue::Integer(i)) => Some(*i),
                    _ => None,
                })
                .max()
                .unwrap_or(-1)
                + 1;
            let mut p = points.iter().find(|p| s(p, "full_name") == Some(like.as_str())).with_context(|| format!("no respawn point {like}"))?.clone();
            p.set_path("full_name", ConfigValue::String(full_name.clone()))?;
            p.set_path("name", ConfigValue::String(full_name.rsplit('/').next().unwrap().to_owned()))?;
            p.set_path("position", ConfigValue::Float3(*position))?;
            p.set_path("rotation", ConfigValue::Float3([0.0, *yaw, 0.0]))?;
            p.set_path("point_id", ConfigValue::Integer(next))?;
            points.push(p);
            format!("respawn {full_name} (like {like}, point_id {next}) at {position:?} yaw {yaw}")
        }
        Op::RemoveClientPrefix { prefix } => {
            let v = items(root, "client_objects")?;
            let before = v.len();
            v.retain(|o| !s(o, "full_name").is_some_and(|n| n.starts_with(prefix.as_str())));
            ensure!(v.len() < before, "no client object under {prefix}");
            format!("removed {} client objects under {prefix}", before - v.len())
        }
        Op::Fill { pack } => {
            let fill: fill::FillPack = serde_json::from_reader(std::fs::File::open(pack).with_context(|| pack.clone())?)?;
            let bytes = stock.read(&fill.template_project)?;
            ensure!(format!("{:x}", Sha256::digest(&bytes)) == fill.template_sha256, "{pack}: template changed");
            let native = BinaryConfig::parse(&bytes)?.root;
            let applied = fill::apply(root, &native, &fill)?;
            format!("fill {}: {}", fill.name, applied.notes.join("; "))
        }
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(args.len() == 3, "usage: PACK_DIR PLAN.json STOCK_DB");
    let files = Path::new(&args[0]).join("files");
    let plan: Plan = serde_json::from_reader(std::fs::File::open(&args[1])?)?;
    let mut stock = vostok_resource_porter::archive::ResourceDb::open(Path::new(&args[2]))?;
    for (path, ops) in &plan.projects {
        let file = files.join(path);
        let mut project = BinaryConfig::parse(&std::fs::read(&file).with_context(|| file.display().to_string())?)?;
        let before = project.clone();
        for op in ops {
            let note = apply(&mut project.root, op, &mut stock).with_context(|| path.clone())?;
            println!("{path}: {note}");
        }
        if project != before {
            std::fs::write(&file, project.to_bytes()?)?;
        } else {
            println!("{path}: unchanged (every op already applied)");
        }
    }
    for (path, leaves) in &plan.options {
        let file = files.join(path);
        let mut options = BinaryConfig::parse(&std::fs::read(&file).with_context(|| file.display().to_string())?)?;
        for (leaf, value) in leaves {
            let old = get(&options.root, leaf).with_context(|| format!("{path}: no {leaf}"))?.clone();
            let new = json_leaf(value, Some(&old))?;
            ensure!(new != old, "{path}: {leaf} is already {value}");
            options.root.set_path(leaf, new)?;
            println!("{path}: {leaf} {old:?} -> {value}");
        }
        std::fs::write(&file, options.to_bytes()?)?;
    }
    Ok(())
}
