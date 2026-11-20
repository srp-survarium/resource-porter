// SPDX-License-Identifier: GPL-3.0-or-later
//! A project in the editor form 0.32 introduced, written in the form 0.26g to
//! 0.31 use.
//!
//! The two forms describe the same objects (`game_object_type` is the same in
//! both). What differs, read off objects matched by `full_name` between
//! 0.31a2's and 0.32a4's London:
//!
//! - an object carries its editor id (`guid_path`) and refers to other
//!   objects by id (`link_item_ptr` tables); the old form refers by name;
//! - `lib_name` is `base:<kind>` and the asset moved to a field of its own
//!   (`particle_name`, `model_name`); the old form's `lib_name` is the asset,
//!   or `point_light` / `spot_light` for a light;
//! - a battery point lists what belongs to it as ids of whole groups
//!   (`other_list`); the old form lists each member by kind;
//! - statics name their model in `model_name`, the old form in `lib_name`;
//! - the members of a placed group (a lamp's light, a fire's flame and barrel)
//!   are all called `<folder>/<name>/<name>`, alike in every copy of the
//!   group, and are told apart by id; the old form calls them
//!   `<folder>/<group>/<name>`. The group's own name is not in the project, so
//!   it is made here: its first member's name, numbered when taken.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};

use crate::binary_config::{ConfigValue, NamedValue};

/// Fields only the editor reads.
const EDITOR_ONLY: &[&str] = &["guid_path", "is_visible_in_editor", "__type", "__removed", "is_shadower"];

/// What an editor id names.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// An object of a client or server list.
    Object { kind: String, name: String },
    /// A static's render visual or collision (`<id>/render`, `<id>/collision`).
    Static { name: String },
}

#[derive(Default)]
struct Ids {
    /// `guid_path` to what it names; a static's two entries share one id.
    by_id: BTreeMap<String, Target>,
    /// A group's id to the name made for it.
    groups: BTreeMap<String, String>,
}

/// The id of the group an id's owner belongs to, if it is a member of one.
fn group_of(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(group, _)| group)
}

/// A static's id without what tells its render visual, collision and
/// walker collision apart.
fn static_owner(id: &str) -> Option<(&str, &str)> {
    ["/render", "/collision", "/walker_collision"].iter().find_map(|part| id.strip_suffix(part).map(|owner| (owner, &part[1..])))
}

fn text<'a>(object: &'a ConfigValue, key: &str) -> Option<&'a str> {
    match object.named(key) {
        Some(ConfigValue::String(s)) => Some(s),
        _ => None,
    }
}

fn items<'a>(parent: &'a ConfigValue, key: &str) -> &'a [ConfigValue] {
    match parent.named(key) {
        Some(ConfigValue::Indexed(items)) => items,
        _ => &[],
    }
}

fn named(fields: Vec<(&str, ConfigValue)>) -> ConfigValue {
    ConfigValue::Named(fields.into_iter().map(|(name, value)| NamedValue { name: name.into(), value }).collect())
}

fn name_entry(name: &str) -> ConfigValue {
    named(vec![("name", ConfigValue::String(name.into()))])
}

impl Ids {
    /// The old form's name of the owner of `id`, called `full_name` in the
    /// new form with the short `name`: a group's member takes its group's name.
    fn old_name(&mut self, id: &str, full_name: &str, name: &str, taken: &mut BTreeMap<String, usize>) -> String {
        let Some(group) = group_of(id) else { return full_name.to_owned() };
        let group_name = match self.groups.get(group) {
            Some(known) => known.clone(),
            None => {
                // `<folder>/<name>/<name>`: the folder the group stands in.
                let folder = full_name
                    .strip_suffix(name)
                    .and_then(|f| f.strip_suffix('/'))
                    .and_then(|f| f.strip_suffix(name))
                    .and_then(|f| f.strip_suffix('/'))
                    .or_else(|| full_name.rsplit_once('/').map(|(f, _)| f))
                    .unwrap_or_default();
                let made = numbered(&format!("{folder}/{name}"), taken);
                self.groups.insert(group.to_owned(), made.clone());
                made
            }
        };
        numbered(&format!("{group_name}/{name}"), taken)
    }

    fn of(project: &ConfigValue) -> Result<Self> {
        let mut ids = Self::default();
        let mut taken = BTreeMap::new();
        let mut object = |ids: &mut Self, o: &ConfigValue| -> Result<()> {
            let Some(id) = text(o, "guid_path") else { return Ok(()) };
            let full = text(o, "full_name").or_else(|| text(o, "name")).context("an object has an id and no name")?;
            let short = text(o, "name").unwrap_or(full);
            let kind = text(o, "game_object_type").unwrap_or_default();
            let name = ids.old_name(id, full, short, &mut taken);
            ids.by_id.insert(id.into(), Target::Object { kind: kind.into(), name });
            Ok(())
        };
        for o in items(project, "client_objects") {
            object(&mut ids, o)?;
        }
        if let Some(ConfigValue::Named(lists)) = project.named("server_objects") {
            for list in lists {
                if let ConfigValue::Indexed(objects) = &list.value {
                    for o in objects {
                        object(&mut ids, o)?;
                    }
                }
            }
        }
        // A static has a collision called by its name, a render visual called
        // `render_<name>` and maybe a walker's collision called `walker_<name>`.
        for (list, key, prefix) in [("temporary_collision_objects", "name", ""), ("temporary_render_visuals", "render_name", "render_")] {
            for entry in items(project, list) {
                let (Some(id), Some(full)) = (text(entry, "guid_path"), text(entry, key)) else { continue };
                let Some((owner, part)) = static_owner(id) else { continue };
                if part == "walker_collision" || ids.by_id.contains_key(owner) {
                    continue;
                }
                let full = full.strip_prefix(prefix).unwrap_or(full);
                let short = full.rsplit_once('/').map_or(full, |(_, n)| n);
                let name = ids.old_name(owner, full, short, &mut taken);
                ids.by_id.insert(owner.into(), Target::Static { name });
            }
        }
        Ok(ids)
    }

    /// The old form's name of the owner of `id`.
    fn name_of(&self, id: &str) -> Option<&str> {
        match self.by_id.get(id)? {
            Target::Object { name, .. } | Target::Static { name } => Some(name),
        }
    }

    /// The id a link's `path` spells.
    fn id_of(link: &ConfigValue) -> Result<String> {
        let ConfigValue::Indexed(path) = link.named("path").context("a link has no path")? else {
            bail!("a link's path is not a list")
        };
        let parts: Vec<&str> = path
            .iter()
            .map(|p| match p {
                ConfigValue::String(s) => Ok(s.as_str()),
                _ => bail!("a link's path holds something other than ids"),
            })
            .collect::<Result<_>>()?;
        Ok(parts.join("/"))
    }

    fn exact(&self, id: &str) -> Option<&Target> {
        self.by_id.get(id)
    }

    /// Everything inside the group `id` names, with what is left of its id.
    fn members<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Target> + 'a {
        let prefix = format!("{id}/");
        self.by_id
            .range(prefix.clone()..)
            .take_while(move |(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v)
    }
}

/// `name`, or `name(n)` with the first n that is free.
fn numbered(name: &str, taken: &mut BTreeMap<String, usize>) -> String {
    let n = taken.entry(name.to_owned()).or_default();
    *n += 1;
    if *n == 1 {
        return name.to_owned();
    }
    let made = format!("{name}({})", *n - 1);
    taken.insert(made.clone(), 1);
    made
}

/// A link: the id it points at as `path`, with or without the editor's
/// `__type` and `__removed` beside it.
fn is_link(value: &ConfigValue) -> bool {
    let ConfigValue::Named(fields) = value else { return false };
    matches!(value.named("path"), Some(ConfigValue::Indexed(path)) if path.iter().all(|p| matches!(p, ConfigValue::String(_))))
        && fields.iter().all(|f| matches!(f.name.as_str(), "path" | "__type" | "__removed"))
}

/// A table of links keyed by id.
fn is_link_table(value: &ConfigValue) -> bool {
    matches!(value, ConfigValue::Named(entries) if !entries.is_empty() && entries.iter().all(|e| is_link(&e.value)))
}

/// What the form change could not carry over, by kind of gap.
#[derive(Debug, Default, serde::Serialize)]
pub struct SchemaReport {
    /// Links to ids no object of the project has, by the field holding them.
    pub dangling_links: BTreeMap<String, usize>,
    pub links_resolved: usize,
    pub battery_points: usize,
}

struct Converter<'a> {
    ids: &'a Ids,
    report: SchemaReport,
}

impl Converter<'_> {
    /// One link as the old form's `{ name }`.
    fn link(&mut self, link: &ConfigValue, at: &str) -> Result<Option<ConfigValue>> {
        let id = Ids::id_of(link)?;
        let name = match self.ids.exact(&id) {
            Some(Target::Object { name, .. } | Target::Static { name }) => name.clone(),
            None => {
                *self.report.dangling_links.entry(at.into()).or_default() += 1;
                return Ok(None);
            }
        };
        self.report.links_resolved += 1;
        Ok(Some(name_entry(&name)))
    }

    fn value(&mut self, value: &ConfigValue, at: &str) -> Result<ConfigValue> {
        Ok(match value {
            v if is_link(v) => self.link(v, at)?.unwrap_or_else(|| named(vec![])),
            v if is_link_table(v) => {
                let ConfigValue::Named(entries) = v else { unreachable!() };
                let mut out = Vec::new();
                for entry in entries {
                    if let Some(named) = self.link(&entry.value, at)? {
                        out.push(named);
                    }
                }
                ConfigValue::Indexed(out)
            }
            ConfigValue::Named(fields) => {
                let mut out = Vec::new();
                for field in fields {
                    if EDITOR_ONLY.contains(&field.name.as_str()) {
                        continue;
                    }
                    out.push(NamedValue { name: field.name.clone(), value: self.value(&field.value, &field.name)? });
                }
                ConfigValue::Named(out)
            }
            ConfigValue::Indexed(list) => {
                ConfigValue::Indexed(list.iter().map(|v| self.value(v, at)).collect::<Result<_>>()?)
            }
            other => other.clone(),
        })
    }

    /// An object: its fields without the editor's, links by name, and the
    /// old form's `lib_name`, `type` and `is_temporary`.
    fn object(&mut self, object: &ConfigValue) -> Result<ConfigValue> {
        self.object_without(object, &[])
    }

    fn object_without(&mut self, object: &ConfigValue, skip: &[&str]) -> Result<ConfigValue> {
        let ConfigValue::Named(fields) = object else { bail!("an object is not a table") };
        let kind = text(object, "game_object_type").unwrap_or_default().to_owned();
        let lib = text(object, "lib_name").unwrap_or_default();
        let lib = lib.strip_prefix("base:").unwrap_or(lib);
        // The asset the old form names in `lib_name`.
        let (lib_name, moved) = match kind.as_str() {
            "particle" => (text(object, "particle_name").context("a particle names no particle")?.to_owned(), "particle_name"),
            "skeleton_visual" => (text(object, "model_name").context("a skeleton visual names no model")?.to_owned(), "model_name"),
            "light" => {
                let name = match object.named("light_type") {
                    Some(ConfigValue::Integer(0)) => "point_light",
                    Some(ConfigValue::Integer(1)) => "spot_light",
                    other => bail!("a light of type {other:?}: neither a point nor a spot light"),
                };
                (name.to_owned(), "")
            }
            _ => (lib.to_owned(), ""),
        };
        let mut out = Vec::new();
        for field in fields {
            let name = field.name.as_str();
            if EDITOR_ONLY.contains(&name) || name == moved || skip.contains(&name) {
                continue;
            }
            let value = match name {
                "lib_name" => ConfigValue::String(lib_name.clone()),
                "full_name" => match text(object, "guid_path").and_then(|id| self.ids.name_of(id)) {
                    Some(old) => ConfigValue::String(old.to_owned()),
                    None => field.value.clone(),
                },
                _ => self.value(&field.value, name)?,
            };
            out.push(NamedValue { name: name.into(), value });
        }
        let mut out = ConfigValue::Named(out);
        if kind == "light" && out.named("color_curve").is_none() {
            // The old form always writes the ramp: one white key.
            let key = named(vec![("value", ConfigValue::Float4([1.0; 4])), ("key", ConfigValue::Integer(0))]);
            let ramp = named(vec![("source", named(vec![("data", named(vec![("ramp", named(vec![("key0", key)]))]))]))]);
            push(&mut out, "color_curve", ramp);
        }
        if out.named("type").is_none() {
            push(&mut out, "type", ConfigValue::String("object".into()));
        }
        if out.named("is_temporary").is_none() {
            push(&mut out, "is_temporary", ConfigValue::Boolean(false));
        }
        Ok(out)
    }

    /// A battery point's `spawners_list` and `other_list` as the old form's
    /// lists by kind. A linked id is an object, a static, or a group whose
    /// members (its lights, particles, sounds, animated models and statics)
    /// the old form lists one by one beside the group's name.
    fn battery_point(&mut self, point: &ConfigValue, boxes: &[String]) -> Result<ConfigValue> {
        let mut out = self.object_without(point, &["spawners_list", "other_list"])?;
        let mut lists: BTreeMap<&str, Vec<ConfigValue>> = BTreeMap::new();
        let mut client: BTreeMap<&str, Vec<ConfigValue>> = BTreeMap::new();
        for list in ["spawners_list", "other_list"] {
            let Some(ConfigValue::Named(entries)) = point.named(list) else { continue };
            for entry in entries {
                let id = Ids::id_of(&entry.value)?;
                match self.ids.exact(&id) {
                    Some(Target::Object { kind, name }) => {
                        let to = match kind.as_str() {
                            "victory_item_spawner" => "spawners",
                            "victory_items_spawn_box" => "spawn_boxes",
                            "wire_set" => "wire_sets",
                            other => bail!("a battery point links a {other} ({name}): no list for it in the old form"),
                        };
                        lists.entry(to).or_default().push(name_entry(name));
                    }
                    Some(Target::Static { name }) => {
                        let to = if boxes.contains(name) { "boxes" } else { "collision_geometries" };
                        lists.entry(to).or_default().push(name_entry(name));
                    }
                    None => {
                        let mut group = self.ids.groups.get(&id).cloned();
                        for member in self.ids.members(&id) {
                            let (to, name) = match member {
                                Target::Static { name } => ("collision_geometries", name),
                                Target::Object { kind, name } => match kind.as_str() {
                                    "light" => ("lights", name),
                                    "particle" => ("particles", name),
                                    "object_sound" => ("sounds", name),
                                    "skeleton_visual" => ("skeletons", name),
                                    other => bail!("a battery point's group holds a {other} ({name}): no list for it in the old form"),
                                },
                            };
                            client.entry(to).or_default().push(name_entry(name));
                            group.get_or_insert_with(|| name.rsplit_once('/').map_or("", |(g, _)| g).to_owned());
                        }
                        match group {
                            Some(group) => lists.entry("composits").or_default().push(name_entry(&group)),
                            None => *self.report.dangling_links.entry(list.into()).or_default() += 1,
                        }
                    }
                }
                self.report.links_resolved += 1;
            }
        }
        let table = |lists: BTreeMap<&str, Vec<ConfigValue>>| {
            ConfigValue::Named(lists.into_iter().map(|(name, v)| NamedValue { name: name.into(), value: ConfigValue::Indexed(v) }).collect())
        };
        push(&mut out, "attached_c_objects", table(client));
        push(&mut out, "attached_objects", table(lists));
        self.report.battery_points += 1;
        Ok(out)
    }
}

fn push(object: &mut ConfigValue, name: &str, value: ConfigValue) {
    if let ConfigValue::Named(fields) = object {
        fields.push(NamedValue { name: name.into(), value });
    }
}

fn rename(object: &mut ConfigValue, from: &str, to: &str) {
    if let ConfigValue::Named(fields) = object
        && let Some(field) = fields.iter_mut().find(|f| f.name == from)
    {
        field.name = to.into();
    }
}

/// A static entry: its model in `lib_name`; a render visual's `visual_name`
/// is the model unless a material set is chosen.
fn static_entry(entry: &ConfigValue, list: &str, ids: &Ids) -> Result<ConfigValue> {
    let ConfigValue::Named(fields) = entry else { bail!("a static is not a table") };
    let mut out = ConfigValue::Named(fields.iter().filter(|f| !EDITOR_ONLY.contains(&f.name.as_str())).cloned().collect());
    ensure!(out.named("lib_name").is_none(), "a static has both model_name and lib_name");
    rename(&mut out, "model_name", "lib_name");
    if list == "render_visuals" && out.named("visual_name").is_none() {
        let model = text(&out, "lib_name").context("a render visual names no model")?.to_owned();
        push(&mut out, "visual_name", ConfigValue::String(model));
    }
    // A group's static is called after its group, as its links call it.
    if let Some((owner, part)) = text(entry, "guid_path").and_then(static_owner)
        && let Some(name) = ids.name_of(owner)
    {
        let (key, name) = match part {
            "render" => ("render_name", format!("render_{name}")),
            "walker_collision" => ("name", format!("walker_{name}")),
            _ => ("name", name.to_owned()),
        };
        if let Some(field) = out.named_mut(key) {
            *field = ConfigValue::String(name);
        }
    }
    Ok(out)
}

/// A container's box: the old form keeps its placement inside the link, with
/// a name. The new form's link names nothing the project has: the box is
/// called after its container.
fn container(mut object: ConfigValue) -> ConfigValue {
    let label = text(&object, "full_name").or_else(|| text(&object, "name")).map(|n| format!("{n}/box"));
    let props = match &mut object {
        ConfigValue::Named(fields) => fields.iter().position(|f| f.name == "visual_geometry_props").map(|at| fields.remove(at).value),
        _ => None,
    };
    if let (Some(ConfigValue::Named(props)), Some(ConfigValue::Named(link))) = (props, object.named_mut("visual_geometry")) {
        if !link.iter().any(|f| f.name == "name")
            && let Some(label) = label
        {
            link.push(NamedValue { name: "name".into(), value: ConfigValue::String(label) });
        }
        for prop in props {
            if prop.name != "scale" {
                link.push(prop);
            }
        }
    }
    object
}

/// Whether the project is in the new editor form.
pub fn is_new_form(project: &ConfigValue) -> bool {
    items(project, "client_objects").iter().any(|o| o.named("guid_path").is_some())
        || items(project, "render_visuals").iter().any(|v| v.named("model_name").is_some())
}

/// The project without the objects of the kinds in `kinds`, the groups they
/// stand in (an anomaly's zone, shapes, effects and artefact places are
/// members of its group) and what only those link (a zone's shape). Returns
/// what went, by kind.
pub fn without_kinds(project: &ConfigValue, kinds: &[String]) -> Result<(ConfigValue, BTreeMap<String, usize>)> {
    let mut gone: BTreeMap<String, usize> = BTreeMap::new();
    if kinds.is_empty() {
        return Ok((project.clone(), gone));
    }
    let objects = |project: &ConfigValue| -> Vec<ConfigValue> {
        let mut all: Vec<ConfigValue> = items(project, "client_objects").to_vec();
        if let Some(ConfigValue::Named(lists)) = project.named("server_objects") {
            for list in lists {
                if let ConfigValue::Indexed(o) = &list.value {
                    all.extend(o.iter().cloned());
                }
            }
        }
        all
    };
    let all = objects(project);
    let is_kind = |o: &ConfigValue| text(o, "game_object_type").is_some_and(|k| kinds.iter().any(|x| x == k));
    let mut groups: Vec<String> = Vec::new();
    for o in all.iter().filter(|o| is_kind(o)) {
        if let Some(group) = text(o, "guid_path").and_then(group_of) {
            groups.push(format!("{group}/"));
        }
    }
    let in_group = |id: &str| groups.iter().any(|g| id.starts_with(g.as_str()));
    let leaves = |o: &ConfigValue| is_kind(o) || text(o, "guid_path").is_some_and(&in_group);
    // What the leaving objects link and no staying object does.
    fn links(value: &ConfigValue, out: &mut Vec<String>) {
        match value {
            v if is_link(v) => out.extend(Ids::id_of(v).ok()),
            ConfigValue::Named(fields) => fields.iter().for_each(|f| links(&f.value, out)),
            ConfigValue::Indexed(list) => list.iter().for_each(|v| links(v, out)),
            _ => {}
        }
    }
    let (mut by_leaving, mut by_staying) = (Vec::new(), Vec::new());
    for o in &all {
        links(o, if leaves(o) { &mut by_leaving } else { &mut by_staying });
    }
    by_leaving.retain(|id| !by_staying.contains(id));
    let goes = |entry: &ConfigValue| -> bool {
        let Some(id) = text(entry, "guid_path") else { return is_kind(entry) };
        let owner = static_owner(id).map_or(id, |(owner, _)| owner);
        is_kind(entry) || in_group(id) || by_leaving.iter().any(|l| l == owner)
    };
    let mut keep = |list: &[ConfigValue]| -> Vec<ConfigValue> {
        let mut kept = Vec::new();
        for entry in list {
            if goes(entry) {
                *gone.entry(text(entry, "game_object_type").unwrap_or("static").to_owned()).or_default() += 1;
            } else {
                kept.push(entry.clone());
            }
        }
        kept
    };
    let ConfigValue::Named(top) = project else { bail!("a project is not a table") };
    let mut out = Vec::new();
    for field in top {
        let value = match (field.name.as_str(), &field.value) {
            ("client_objects" | "temporary_render_visuals" | "temporary_collision_objects", ConfigValue::Indexed(list)) => ConfigValue::Indexed(keep(list)),
            ("server_objects", ConfigValue::Named(lists)) => ConfigValue::Named(
                lists
                    .iter()
                    .map(|l| NamedValue {
                        name: l.name.clone(),
                        value: match &l.value {
                            ConfigValue::Indexed(list) => ConfigValue::Indexed(keep(list)),
                            other => other.clone(),
                        },
                    })
                    .collect(),
            ),
            (_, value) => value.clone(),
        };
        out.push(NamedValue { name: field.name.clone(), value });
    }
    Ok((ConfigValue::Named(out), gone))
}

/// The project in the old form.
pub fn to_old_form(project: &ConfigValue) -> Result<(ConfigValue, SchemaReport)> {
    let ConfigValue::Named(top) = project else { bail!("a project is not a table") };
    let ids = Ids::of(project)?;
    let mut convert = Converter { ids: &ids, report: SchemaReport::default() };
    // The statics a battery box shows: the old form lists them as `boxes`.
    let mut boxes = Vec::new();
    for o in items(project, "client_objects") {
        if text(o, "game_object_type") == Some("victory_items_spawn_box")
            && let Some(link) = o.named("visual_geometry")
            && let Some(Target::Static { name }) = ids.exact(&Ids::id_of(link)?)
        {
            boxes.push(name.clone());
        }
    }
    let mut out = Vec::new();
    for field in top {
        let value = match (field.name.as_str(), &field.value) {
            ("client_objects", ConfigValue::Indexed(objects)) => {
                ConfigValue::Indexed(objects.iter().map(|o| convert.object(o)).collect::<Result<_>>()?)
            }
            ("render_visuals" | "temporary_render_visuals" | "collision_objects" | "temporary_collision_objects", ConfigValue::Indexed(list)) => {
                ConfigValue::Indexed(list.iter().map(|v| static_entry(v, &field.name, &ids)).collect::<Result<_>>()?)
            }
            ("server_objects", ConfigValue::Named(lists)) => {
                let mut server = Vec::new();
                for list in lists {
                    let ConfigValue::Indexed(objects) = &list.value else {
                        server.push(list.clone());
                        continue;
                    };
                    let mut converted = Vec::new();
                    for o in objects {
                        converted.push(match list.name.as_str() {
                            "victory_item_spawn_points" => convert.battery_point(o, &boxes)?,
                            "victory_items_containers" => container(convert.object(o)?),
                            _ => convert.object(o)?,
                        });
                    }
                    server.push(NamedValue { name: list.name.clone(), value: ConfigValue::Indexed(converted) });
                }
                ConfigValue::Named(server)
            }
            (_, value) => value.clone(),
        };
        out.push(NamedValue { name: field.name.clone(), value });
    }
    Ok((ConfigValue::Named(out), convert.report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> ConfigValue {
        ConfigValue::String(x.into())
    }
    fn link(path: &[&str]) -> ConfigValue {
        named(vec![
            ("__type", s("link_item_ptr")),
            ("__removed", ConfigValue::Boolean(false)),
            ("path", ConfigValue::Indexed(path.iter().map(|p| s(p)).collect())),
        ])
    }
    fn table(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
        named(entries)
    }

    fn project() -> ConfigValue {
        let fire = named(vec![
            ("guid_path", s("<a>")),
            ("is_visible_in_editor", ConfigValue::Boolean(true)),
            ("full_name", s("logic/fire/zone")),
            ("game_object_type", s("damage_zone")),
            ("lib_name", s("base:damage_zone")),
            // A link without the editor's type, as a resupply zone's is written.
            ("collision_geometries", table(vec![("<b>", named(vec![("__removed", ConfigValue::Boolean(false)), ("path", ConfigValue::Indexed(vec![s("<b>")]))]))])),
        ]);
        let shape = named(vec![
            ("guid_path", s("<b>")),
            ("full_name", s("logic/fire/shape")),
            ("game_object_type", s("collision_geometry")),
            ("lib_name", s("base:collision_geometry")),
        ]);
        let flame = named(vec![
            ("guid_path", s("<g>/<p>")),
            ("__type", s("subobject")),
            ("is_temporary", ConfigValue::Boolean(true)),
            ("full_name", s("props/3/bonfire/bonfire")),
            ("name", s("bonfire")),
            ("game_object_type", s("particle")),
            ("lib_name", s("base:particle")),
            ("particle_name", s("fx/lights/bonfire")),
        ]);
        let lamp = named(vec![
            ("guid_path", s("<g>/<l>")),
            ("full_name", s("props/3/point_light/point_light")),
            ("name", s("point_light")),
            ("game_object_type", s("light")),
            ("lib_name", s("base:light")),
            ("light_type", ConfigValue::Integer(0)),
            ("is_shadower", ConfigValue::Boolean(false)),
        ]);
        let spawn_box = named(vec![
            ("guid_path", s("<sb>")),
            ("full_name", s("logic/mode_3/box")),
            ("game_object_type", s("victory_items_spawn_box")),
            ("lib_name", s("base:victory_item_spawn_box")),
            ("visual_geometry", link(&["<box>"])),
        ]);
        let point = named(vec![
            ("guid_path", s("<pt>")),
            ("name", s("victory_item_spawn_point")),
            ("game_object_type", s("victory_item_spawn_point")),
            ("lib_name", s("base:victory_item_spawn_point")),
            ("spawners_list", table(vec![("<sp>", link(&["<sp>"]))])),
            ("other_list", table(vec![("<g>", link(&["<g>"])), ("<box>", link(&["<box>"])), ("<sb>", link(&["<sb>"])), ("<prop>", link(&["<prop>"]))])),
        ]);
        let spawner = named(vec![
            ("guid_path", s("<sp>")),
            ("full_name", s("logic/mode_3/spawner")),
            ("game_object_type", s("victory_item_spawner")),
            ("lib_name", s("base:victory_item_spawner")),
        ]);
        let collision = |id: &str, name: &str| named(vec![("guid_path", s(id)), ("name", s(name)), ("model_name", s("resources/models/x.model/collision"))]);
        named(vec![
            ("client_objects", ConfigValue::Indexed(vec![fire, shape, flame, lamp, spawn_box])),
            ("render_visuals", ConfigValue::Indexed(vec![named(vec![("model_name", s("props/x")), ("use_as_occluder", ConfigValue::Boolean(false))])])),
            (
                "temporary_collision_objects",
                ConfigValue::Indexed(vec![
                    collision("<box>/collision", "props/3/box"),
                    collision("<prop>/collision", "props/3/cap"),
                    collision("<g>/<b1>/collision", "props/3/barrel/barrel"),
                    collision("<g>/<b1>/walker_collision", "walker_props/3/barrel/barrel"),
                ]),
            ),
            ("server_objects", named(vec![("victory_item_spawn_points", ConfigValue::Indexed(vec![point])), ("victory_item_spawners", ConfigValue::Indexed(vec![spawner]))])),
        ])
    }

    fn names(list: Option<&ConfigValue>) -> Vec<String> {
        match list {
            Some(ConfigValue::Indexed(items)) => items.iter().filter_map(|i| text(i, "name").map(str::to_owned)).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn links_become_names_and_the_asset_moves_into_lib_name() {
        let (old, report) = to_old_form(&project()).unwrap();
        let objects = items(&old, "client_objects");
        assert_eq!(names(objects[0].named("collision_geometries")), ["logic/fire/shape"]);
        assert_eq!(text(&objects[0], "lib_name"), Some("damage_zone"));
        assert!(objects[0].named("guid_path").is_none() && objects[0].named("is_visible_in_editor").is_none());
        assert_eq!(objects[0].named("is_temporary"), Some(&ConfigValue::Boolean(false)));
        assert_eq!(text(&objects[2], "lib_name"), Some("fx/lights/bonfire"));
        assert!(objects[2].named("particle_name").is_none());
        assert_eq!(objects[2].named("is_temporary"), Some(&ConfigValue::Boolean(true)));
        assert_eq!(text(&objects[3], "lib_name"), Some("point_light"));
        assert_eq!(text(&objects[4].named("visual_geometry").unwrap().clone(), "name"), Some("props/3/box"));
        assert!(report.dangling_links.is_empty());
        let visual = &items(&old, "render_visuals")[0];
        assert_eq!((text(visual, "lib_name"), text(visual, "visual_name")), (Some("props/x"), Some("props/x")));
    }

    #[test]
    fn a_kind_leaves_with_its_group_and_what_only_it_links() {
        let mut p = project();
        let anomaly = named(vec![("guid_path", s("<an>/<core>")), ("full_name", s("a/core/core")), ("name", s("core")), ("game_object_type", s("timeout_anomaly_core"))]);
        let zone = named(vec![
            ("guid_path", s("<an>/<z>")),
            ("full_name", s("a/zone/zone")),
            ("name", s("zone")),
            ("game_object_type", s("damage_zone")),
            ("collision_geometries", table(vec![("<zs>", link(&["<zs>"]))])),
        ]);
        let zone_shape = named(vec![("guid_path", s("<zs>")), ("full_name", s("a/shape")), ("game_object_type", s("collision_geometry"))]);
        if let Some(ConfigValue::Indexed(objects)) = p.named_mut("client_objects") {
            objects.extend([anomaly, zone, zone_shape]);
        }
        let (kept, gone) = without_kinds(&p, &["timeout_anomaly_core".to_owned()]).unwrap();
        let names: Vec<&str> = items(&kept, "client_objects").iter().filter_map(|o| text(o, "full_name")).collect();
        // The fire's zone and its shape stay: no anomaly stands in their group.
        assert_eq!(names, ["logic/fire/zone", "logic/fire/shape", "props/3/bonfire/bonfire", "props/3/point_light/point_light", "logic/mode_3/box"]);
        assert_eq!(gone.get("damage_zone"), Some(&1));
        assert_eq!(gone.get("collision_geometry"), Some(&1));
    }

    #[test]
    fn a_battery_point_lists_its_groups_members_by_kind() {
        let (old, report) = to_old_form(&project()).unwrap();
        let point = &items(old.named("server_objects").unwrap(), "victory_item_spawn_points")[0];
        let attached = point.named("attached_objects").unwrap();
        let client = point.named("attached_c_objects").unwrap();
        assert_eq!(names(attached.named("spawners")), ["logic/mode_3/spawner"]);
        assert_eq!(names(attached.named("spawn_boxes")), ["logic/mode_3/box"]);
        assert_eq!(names(attached.named("boxes")), ["props/3/box"]);
        assert_eq!(names(attached.named("collision_geometries")), ["props/3/cap"]);
        // The group is called after its first member, each member after the group.
        assert_eq!(names(attached.named("composits")), ["props/3/bonfire"]);
        assert_eq!(names(client.named("collision_geometries")), ["props/3/bonfire/barrel"]);
        assert_eq!(names(client.named("particles")), ["props/3/bonfire/bonfire"]);
        assert_eq!(names(client.named("lights")), ["props/3/bonfire/point_light"]);
        let statics = items(&old, "temporary_collision_objects");
        assert_eq!(text(&statics[2], "name"), Some("props/3/bonfire/barrel"));
        assert_eq!(text(&statics[3], "name"), Some("walker_props/3/bonfire/barrel"));
        assert_eq!(text(&items(&old, "client_objects")[2], "full_name"), Some("props/3/bonfire/bonfire"));
        assert!(point.named("other_list").is_none());
        assert_eq!(report.battery_points, 1);
    }
}
