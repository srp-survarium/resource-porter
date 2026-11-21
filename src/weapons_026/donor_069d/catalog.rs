// SPDX-License-Identifier: GPL-3.0-or-later
//! 0.69d0's module system as its `gameplay/game.options` has it, read once
//! into types (`docs/modules-069d-026.md`).
//!
//! A gun (`weapons.<item>`) is a base mesh with slots; every other part is a
//! module (`weapon_modules.<id>`) in a slot, the gun's own parts too
//! (`default_modules`). A module names the slots it goes in, each with the
//! rules it brings there (a named `attach_params` block). Which gun may take
//! which module is `module_to_item_links`; what a bought module does to the
//! gun's numbers is `modules_drop_options` ([`Catalog::stats`]).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use super::{int_of, list, string_of};
use crate::ConfigValue as V;
use crate::weapons_026::get;

/// A slot of a gun, or one a module brings: its id and the locator (of the
/// gun's model, or the module's) it is at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotDef {
    pub id: String,
    pub locator: String,
    /// `plane_slot` of an override: the slot of the gun whose plane the
    /// point keeps (`body_aim`, the gun's own eye: an open sight's line of
    /// sight at the gun's eye relief, `get_slot_transform` of 0.69d0's exe).
    pub plane: Option<String>,
}

/// One of a gun's own modules: `default_modules` or `bundled_modules`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub module: String,
    /// The slot the entry names, when it names one (the A-545's `mag1`, `mag2`).
    pub slot: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GunDef {
    /// `object.model`.
    pub model: String,
    pub slots: Vec<SlotDef>,
    pub defaults: Vec<Entry>,
    pub bundled: Vec<Entry>,
    /// The item it is a skin of (`parameters.base_item_dict_id`, another item's dict id): a
    /// premium or camouflage item's base gun. None for an item that is its own base.
    pub base: Option<String>,
}

/// A module's models: its shape, its alternative one (`model_param1`: a
/// folded sight), and an optic's in the aimed view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Models {
    pub model: Option<String>,
    pub alt: Option<String>,
    pub aimed: Option<String>,
    pub aimed_alt: Option<String>,
}

/// A module's rules in one slot (`slots_to_attach_to[]` with its
/// `attach_params` block).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttachDef {
    pub slot: String,
    /// `slots_to_add`: the slots the module brings, at its own locators.
    pub adds: Vec<SlotDef>,
    /// `slots_to_override`: a point of the gun moved to the module's locator.
    pub overrides: Vec<SlotDef>,
    /// `slots_to_change_param`: the slots whose module shows `model_param1`.
    pub alters: Vec<(String, i32)>,
    /// `slots_to_disable`: the slots closed or emptied.
    pub closes: Vec<String>,
    /// `effects_to_override.particles.shoot`.
    pub shoot: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ModuleDef {
    pub id: String,
    pub dict_id: i32,
    pub group: String,
    /// `parameters.cost`: 0 for a part 0.69d0 does not sell.
    pub cost: i32,
    pub models: Models,
    /// `exceptions.<item>`: the models it shows on that gun instead.
    pub exceptions: BTreeMap<String, Models>,
    /// `exceptions.<item>.slots_to_attach_to` laid over its own entries,
    /// place by place: the slots it goes in on that gun (the rail bracket on
    /// the RPD and the PKP, whose first entry is the grip rail a second time).
    /// LIKELY: read so because 0.69d0 links the PKP its own foregrip, which
    /// has a slot there only if the bracket still brings `picatinny_grip`.
    pub attach_exceptions: BTreeMap<String, Vec<AttachDef>>,
    /// In its own order of preference.
    pub attaches: Vec<AttachDef>,
    /// A grip's sixteen locator names: fifteen fingers, then the hand.
    pub finger_locators: Vec<String>,
    /// The suppressor whose material every surface of its model draws, for a module of muzzle
    /// slots that is none by its group or name: that suppressor in another shape (the VSSK's
    /// heavy and long barrels). Read from the models ([`Catalog::find_suppressor_shapes`]);
    /// None until then.
    pub suppressor_shape: Option<String>,
}

impl ModuleDef {
    /// Its models on `item`: an exception's where it names one.
    pub fn models_on(&self, item: &str) -> Models {
        let e = self.exceptions.get(item).cloned().unwrap_or_default();
        Models {
            model: e.model.or_else(|| self.models.model.clone()),
            alt: e.alt.or_else(|| self.models.alt.clone()),
            aimed: e.aimed.or_else(|| self.models.aimed.clone()),
            aimed_alt: e.aimed_alt.or_else(|| self.models.aimed_alt.clone()),
        }
    }

    /// The slots it goes in on `item`, in its order of preference.
    pub fn attaches_on(&self, item: &str) -> &[AttachDef] {
        self.attach_exceptions.get(item).unwrap_or(&self.attaches)
    }

    /// Its rules in `slot` on `item`.
    pub fn attach_on(&self, item: &str, slot: &str) -> Option<&AttachDef> {
        self.attaches_on(item).iter().find(|a| a.slot == slot)
    }

    /// An optic: it moves the eye and is of 0.69d0's scope groups (`scope`,
    /// `scope_collimator`, `scope_4x` and the other zooms). A rail mount, an
    /// iron sight and one grip move the eye too, and every one of them has
    /// the optics' zoom keys.
    pub fn is_optic(&self) -> bool {
        let moves_eye = |a: &AttachDef| a.overrides.iter().any(|o| o.id == "aim");
        let zoom = self.group.strip_prefix("scope_").is_some_and(|z| z.ends_with('x'));
        (zoom || matches!(self.group.as_str(), "scope" | "scope_collimator")) && self.attaches.iter().any(moves_eye)
    }

    /// A sound suppressor: by its group or its name, or by its shape
    /// (`suppressor_shape`). Never ported (the owner, 2026-11-01).
    pub fn is_suppressor(&self) -> bool {
        self.named_suppressor() || self.suppressor_shape.is_some()
    }

    fn named_suppressor(&self) -> bool {
        self.group == "silencer" || self.id.contains("suppressor") || self.id.contains("silencer")
    }

    /// It goes in muzzle threads and nowhere else.
    pub fn muzzle_only(&self) -> bool {
        !self.attaches.is_empty() && self.attaches.iter().all(|a| a.slot.ends_with("_muzzle"))
    }
}

/// One modifier of a bought module (`modules_drop_options`): the config key
/// path, the value at the bought grade and the modifier's type (1 adds,
/// 2 scales).
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub path: Vec<String>,
    pub value: f32,
    pub kind: i32,
}

pub struct Catalog {
    pub guns: BTreeMap<String, GunDef>,
    pub modules: BTreeMap<String, ModuleDef>,
    /// `module_to_item_links`: by gun, the modules it may take, in 0.69d0's order.
    pub links: BTreeMap<String, Vec<String>>,
    /// By a module's dict id: its bought modifiers, `[group, grade]` (grade 7 on every module).
    bought: BTreeMap<i32, Vec<(i32, i32)>>,
    /// By modifier group and grade: its modifiers.
    grades: BTreeMap<(i32, i32), Vec<Modifier>>,
}


fn text(v: &V, key: &str) -> Result<String> {
    Ok(string_of(v.named(key))
        .with_context(|| format!("no {key}"))?
        .to_owned())
}

fn optional(v: &V, key: &str) -> Result<Option<String>> {
    match v.named(key) {
        Some(x) => Ok(Some(string_of(Some(x))?.to_owned())),
        None => Ok(None),
    }
}

fn models(v: &V) -> Result<Models> {
    Ok(Models {
        model: optional(v, "model")?,
        alt: optional(v, "model_param1")?,
        aimed: optional(v, "aimed_model")?,
        aimed_alt: optional(v, "aimed_model_param1")?,
    })
}

fn slot_defs(v: Option<&V>) -> Result<Vec<SlotDef>> {
    list(v)
        .iter()
        .map(|s| {
            Ok(SlotDef {
                id: text(s, "id")?,
                locator: text(s, "locator")?,
                plane: optional(s, "plane_slot")?,
            })
        })
        .collect()
}

fn entries(v: Option<&V>) -> Result<Vec<Entry>> {
    list(v)
        .iter()
        .map(|m| {
            Ok(Entry {
                module: text(m, "module")?,
                slot: optional(m, "slot")?,
            })
        })
        .collect()
}

fn attach(module: &V, entry: &V) -> Result<AttachDef> {
    // An entry that is the slot's name alone brings no rules (one module's).
    if let V::String(slot) = entry {
        return Ok(AttachDef {
            slot: slot.clone(),
            ..AttachDef::default()
        });
    }
    let slot = text(entry, "slot")?;
    let block = text(entry, "attach_params")?;
    // A module with no rules names a block it does not have.
    let Some(params) = module.named(&block) else {
        return Ok(AttachDef {
            slot,
            ..AttachDef::default()
        });
    };
    let alters = list(params.named("slots_to_change_param"))
        .iter()
        .map(|c| Ok((text(c, "id")?, int_of(c.named("param"))?)))
        .collect::<Result<Vec<_>>>()?;
    let closes = list(params.named("slots_to_disable"))
        .iter()
        .map(|c| text(c, "id"))
        .collect::<Result<Vec<_>>>()?;
    let shoot = match get(params, "effects_to_override.particles.shoot") {
        Some(v) => Some(string_of(Some(v))?.to_owned()),
        None => None,
    };
    Ok(AttachDef {
        slot,
        adds: slot_defs(params.named("slots_to_add"))?,
        overrides: slot_defs(params.named("slots_to_override"))?,
        alters,
        closes,
        shoot,
    })
}

/// Entries with one rule set a slot: a module in a slot brings what every
/// entry naming that slot says (the rail bracket on the RPD and the PKP names
/// its grip rail twice, once for each slot it brings there).
fn merged(entries: Vec<AttachDef>) -> Vec<AttachDef> {
    let mut out: Vec<AttachDef> = vec![];
    for entry in entries {
        match out.iter_mut().find(|a| a.slot == entry.slot) {
            Some(first) => {
                for add in entry.adds {
                    if !first.adds.iter().any(|a| a.id == add.id) {
                        first.adds.push(add);
                    }
                }
                for o in entry.overrides {
                    if !first.overrides.iter().any(|a| a.id == o.id) {
                        first.overrides.push(o);
                    }
                }
                for a in entry.alters {
                    if !first.alters.contains(&a) {
                        first.alters.push(a);
                    }
                }
                for c in entry.closes {
                    if !first.closes.contains(&c) {
                        first.closes.push(c);
                    }
                }
                first.shoot = first.shoot.take().or(entry.shoot);
            }
            None => out.push(entry),
        }
    }
    out
}

fn number(v: Option<&V>) -> Option<f32> {
    match v {
        Some(V::Float(f)) => Some(*f),
        Some(V::Integer(i)) => Some(*i as f32),
        _ => None,
    }
}

impl Catalog {
    /// 0.69d0's `gameplay/game.options`, parsed.
    pub fn read(game: &V) -> Result<Self> {
        let named = |key: &str| match game.named(key) {
            Some(V::Named(fields)) => Ok(fields),
            _ => bail!("0.69d0's game.options has no {key}"),
        };
        let mut guns = BTreeMap::new();
        // By dict id: the item. A skin names its base item so (`base_item_dict_id`).
        let mut by_dict: BTreeMap<i32, &str> = BTreeMap::new();
        for w in named("weapons")? {
            if let Some(V::Integer(dict)) = get(&w.value, "parameters.dict_id") {
                ensure!(by_dict.insert(*dict, &w.name).is_none(), "0.69d0 has two weapons of dict id {dict}");
            }
        }
        for w in named("weapons")? {
            // An entry without a model or slots is no gun (a grenade's row).
            let Ok(model) = string_of(get(&w.value, "object.model")) else {
                continue;
            };
            let base = match get(&w.value, "parameters.base_item_dict_id") {
                Some(V::Integer(dict)) => by_dict.get(dict).filter(|base| **base != w.name),
                _ => None,
            };
            let gun = GunDef {
                model: model.to_owned(),
                slots: slot_defs(w.value.named("slots")).with_context(|| w.name.clone())?,
                defaults: entries(w.value.named("default_modules"))?,
                bundled: entries(w.value.named("bundled_modules"))?,
                base: base.map(|base| (*base).to_owned()),
            };
            guns.insert(w.name.clone(), gun);
        }
        let mut modules = BTreeMap::new();
        for m in named("weapon_modules")? {
            let v = &m.value;
            let def = (|| -> Result<ModuleDef> {
                let own = list(v.named("slots_to_attach_to"))
                    .iter()
                    .map(|a| attach(v, a))
                    .collect::<Result<Vec<_>>>()?;
                let (mut exceptions, mut attach_exceptions) = (BTreeMap::new(), BTreeMap::new());
                if let Some(V::Named(items)) = v.named("exceptions") {
                    for e in items {
                        exceptions.insert(e.name.clone(), models(&e.value)?);
                        if let Some(slots) = e.value.named("slots_to_attach_to") {
                            // An exception's entries lie over the module's own, place by place.
                            let mut entries = own.clone();
                            for (at, entry) in list(Some(slots)).iter().enumerate() {
                                let entry = attach(v, entry)?;
                                match entries.get_mut(at) {
                                    Some(place) => *place = entry,
                                    None => entries.push(entry),
                                }
                            }
                            attach_exceptions.insert(e.name.clone(), merged(entries));
                        }
                    }
                }
                Ok(ModuleDef {
                    id: m.name.clone(),
                    dict_id: int_of(get(v, "parameters.dict_id"))?,
                    group: text(v, "group")?,
                    cost: int_of(get(v, "parameters.cost"))?,
                    models: models(v)?,
                    exceptions,
                    attach_exceptions,
                    attaches: merged(own),
                    finger_locators: list(v.named("finger_locators"))
                        .iter()
                        .map(|n| string_of(Some(n)).map(str::to_owned))
                        .collect::<Result<_>>()?,
                    suppressor_shape: None,
                })
            })()
            .with_context(|| format!("weapon_modules.{}", m.name))?;
            ensure!(
                def.finger_locators.is_empty() || def.finger_locators.len() == 16,
                "{}: {} finger locators",
                def.id,
                def.finger_locators.len()
            );
            modules.insert(m.name.clone(), def);
        }
        let mut links: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for l in list(game.named("module_to_item_links")) {
            // (0.69d0 has a link twice here and there: the first stands.)
            let (of, module) = (links.entry(text(l, "item")?).or_default(), text(l, "module")?);
            if !of.contains(&module) {
                of.push(module);
            }
        }
        // A bought module's modifiers: `premium_modifiers`, `[group, grade]` pairs.
        let mut bought = BTreeMap::new();
        for info in list(get(game, "modules_drop_options.modules_additional_infos")) {
            let pairs = list(info.named("premium_modifiers"))
                .iter()
                .map(|pair| match list(Some(pair)) {
                    [group, grade] => Ok((int_of(Some(group))?, int_of(Some(grade))?)),
                    other => bail!("a premium modifier of {} values", other.len()),
                })
                .collect::<Result<Vec<_>>>()?;
            bought.insert(int_of(info.named("dict_id"))?, pairs);
        }
        let mut grades: BTreeMap<(i32, i32), Vec<Modifier>> = BTreeMap::new();
        for m in list(game.named("modifications")) {
            let (Some(V::Integer(group)), Some(V::Integer(grade))) = (m.named("group"), m.named("grade")) else {
                continue;
            };
            let modifiers = list(m.named("modifiers"))
                .iter()
                .map(|m| {
                    Ok(Modifier {
                        path: list(m.named("path"))
                            .iter()
                            .map(|p| match p {
                                V::String(s) => Ok(s.clone()),
                                V::Integer(i) => Ok(i.to_string()),
                                other => bail!("a modifier path holds {other:?}"),
                            })
                            .collect::<Result<_>>()?,
                        value: number(m.named("value")).context("a modifier's value")?,
                        kind: int_of(m.named("type"))?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            grades.entry((*group, *grade)).or_default().extend(modifiers);
        }
        Ok(Self {
            guns,
            modules,
            links,
            bought,
            grades,
        })
    }

    pub fn gun(&self, item: &str) -> Result<&GunDef> {
        self.guns
            .get(item)
            .with_context(|| format!("0.69d0 has no weapons.{item}"))
    }

    pub fn module(&self, id: &str) -> Result<&ModuleDef> {
        self.modules
            .get(id)
            .with_context(|| format!("0.69d0 has no weapon_modules.{id}"))
    }

    /// The modules 0.69d0 lets `item` take, in its order: the item's own links, then its base
    /// item's. A skin's own links are its own-paint parts alone (the SR-3M premium's
    /// camouflage foregrip, the red VSSK's two barrels, a camouflage magazine), and most
    /// skins have none: what the gun takes is linked to its base item.
    pub fn linked(&self, item: &str) -> Vec<&str> {
        let of = |item: &str| self.links.get(item).into_iter().flatten().map(String::as_str);
        let mut out: Vec<&str> = of(item).collect();
        if let Some(base) = self.guns.get(item).and_then(|gun| gun.base.as_deref()) {
            for module in of(base) {
                if !out.contains(&module) {
                    out.push(module);
                }
            }
        }
        out
    }

    /// Marks the suppressors 0.69d0 has in another shape (`ModuleDef::suppressor_shape`): a
    /// module of muzzle slots, no suppressor by group or name, every surface of whose model
    /// draws a material a suppressor's model draws. `materials` gives a module model's
    /// materials (its LOD0 surfaces'), or None for a model the archive lacks.
    ///
    /// In 0.69d0 these are the VSSK's four barrels: each is one surface in the material of the
    /// gun's own suppressor (`weapons/sniper/vssk/vssk_barrel`, the red one's twin), in the
    /// slot that suppressor stands in. Nothing else in their rows says so (their groups are
    /// `heavy_barrel` and `long_barrel`, and they set no shot effect, as a choke sets none).
    pub fn find_suppressor_shapes(
        &mut self,
        materials: &mut dyn FnMut(&str) -> Result<Option<BTreeSet<String>>>,
    ) -> Result<()> {
        let mut drawn_by: BTreeMap<String, String> = BTreeMap::new();
        for module in self.modules.values().filter(|m| m.named_suppressor()) {
            let Some(model) = &module.models.model else { continue };
            for material in materials(model)?.into_iter().flatten() {
                drawn_by.entry(material).or_insert_with(|| module.id.clone());
            }
        }
        for module in self.modules.values_mut().filter(|m| m.muzzle_only() && !m.named_suppressor()) {
            let Some(model) = &module.models.model else { continue };
            let Some(own) = materials(model)? else { continue };
            if !own.is_empty() && own.iter().all(|m| drawn_by.contains_key(m)) {
                module.suppressor_shape = own.iter().next().map(|m| drawn_by[m].clone());
            }
        }
        Ok(())
    }

    /// What a bought module does to a gun's numbers: the modifiers of its
    /// `premium_modifiers` (each of its modifier groups at grade 7). Empty
    /// for a part 0.69d0 rolls nothing on (a gun's own brake).
    pub fn stats(&self, module: &ModuleDef) -> Vec<Modifier> {
        self.bought
            .get(&module.dict_id)
            .into_iter()
            .flatten()
            .filter_map(|pair| self.grades.get(pair))
            .flatten()
            .cloned()
            .collect()
    }

    /// The module's modifier groups (0.69d0's numbers), as the design's
    /// stat table names a family.
    pub fn stat_groups(&self, module: &ModuleDef) -> Vec<i32> {
        self.bought
            .get(&module.dict_id)
            .into_iter()
            .flatten()
            .map(|(group, _)| *group)
            .collect()
    }

    /// Whether 0.69d0 rolls stats on the module: one a player buys.
    pub fn has_stats(&self, module: &ModuleDef) -> bool {
        self.bought.get(&module.dict_id).is_some_and(|pairs| !pairs.is_empty())
    }
}
