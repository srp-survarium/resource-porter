// SPDX-License-Identifier: GPL-3.0-or-later
//! What 0.69d0's data says of one gun, worked out once: its own parts in
//! their slots, what a player may fit and through which carriers, and which
//! of its own parts a fit can displace (`docs/modules-069d-026.md`).
//!
//! The rules are 0.69d0's own, read from its tables; no gun is named here.
//!
//! 1. **Own parts.** The gun's modules attach in their order, each in the
//!    slot its entry names, else the first of its slots the gun has by then;
//!    its `slots_to_add` and `slots_to_override` then hold for the next.
//! 2. **Fits.** A module `module_to_item_links` gives the gun or its base
//!    item ([`Catalog::linked`]) is a fit when a player can fit its kind
//!    ([`kind`]: no suppressor) and has an item for it (`offered`). It goes
//!    in the first slot it names that the gun with its own parts has; else
//!    in the first it names that carriers
//!    bring: linked modules that are no fit themselves, the shortest chain,
//!    and among equals the one 0.69d0 links first (the owner's rule of
//!    2026-11-20). No slot either way: a refusal with its reason, never a
//!    guess.
//! 3. **Stock parts.** An own part is one a fit can displace when a fit or a
//!    carrier goes in its slot, closes its slot (`slots_to_disable`), shows
//!    it folded (`slots_to_change_param` and the part has a `model_param1`),
//!    or when it stands on a slot such a part brings. The gun's `bare` body
//!    is drawn without them, and they are modules of their own.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::catalog::{Catalog, ModuleDef};

/// What a player fits (the weapon kit's slots 3, 4 and 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Scope,
    Muzzle,
    Grip,
}

/// The carriers one fit brings, at most (`weapon_modules::MAX_CHAIN`).
pub const MAX_CHAIN: usize = 3;

/// The kind a module is fitted as, by what it is in 0.69d0's data: a grip
/// carries the left hand's locators (and is no forend); a muzzle device goes
/// in muzzle threads only; an optic moves the eye. A suppressor is none
/// (the owner, 2026-11-01), and so is every other module.
pub fn kind(module: &ModuleDef) -> Option<Kind> {
    if module.is_suppressor() || module.attaches.is_empty() {
        return None;
    }
    let slots = || module.attaches.iter().map(|a| a.slot.as_str());
    if !module.finger_locators.is_empty() {
        return (!slots().any(|s| s == "forend")).then_some(Kind::Grip);
    }
    if slots().all(|s| s.ends_with("_muzzle")) {
        // What brings slots there is a carrier (the P90's forend on its thread).
        return module.attaches.iter().all(|a| a.adds.is_empty()).then_some(Kind::Muzzle);
    }
    module.is_optic().then_some(Kind::Scope)
}

/// What a locator belongs to: the gun's model, or one of its parts'.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Base {
    Gun,
    /// An own part, by its place in [`GunPlan::parts`].
    Part(usize),
}

/// Where a slot is: a locator of the gun's model or of a part's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Site {
    pub locator: String,
    pub base: Base,
}

/// One of the gun's own parts, in its slot.
#[derive(Clone, Debug, Serialize)]
pub struct OwnPart {
    pub module: String,
    pub slot: String,
    /// Where its slot was when it went on.
    pub site: Site,
    /// A fit can displace it: left out of the `bare` body.
    pub stock: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Link {
    pub module: String,
    pub slot: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fit {
    pub module: String,
    pub kind: Kind,
    /// The slot it goes in.
    pub slot: String,
    /// The carriers that bring that slot, in the order they go on; empty
    /// when the gun with its own parts has it.
    pub chain: Vec<Link>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Refused {
    pub module: String,
    pub kind: Kind,
    pub why: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GunPlan {
    pub parts: Vec<OwnPart>,
    /// The slots of the gun with its own parts.
    pub slots: BTreeMap<String, Site>,
    pub fits: Vec<Fit>,
    pub refused: Vec<Refused>,
    /// By the slot they bring: the carriers, each with the slot it goes in.
    pub carriers: BTreeMap<String, Vec<Link>>,
}

impl GunPlan {
    /// The own parts a fit can displace.
    pub fn stock(&self) -> impl Iterator<Item = (usize, &OwnPart)> {
        self.parts.iter().enumerate().filter(|(_, p)| p.stock)
    }

    /// The part in `slot`, if one of the gun's own stands there.
    fn occupant(&self, slot: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.slot == slot)
    }
}

/// The gun's own modules (`own`, in attach order: 0.69d0's entries less what
/// the body leaves to the kit), each in its slot.
fn stand(catalog: &Catalog, item: &str, own: &[(String, Option<String>)]) -> Result<GunPlan> {
    let gun = catalog.gun(item)?;
    let mut slots: BTreeMap<String, Site> = gun
        .slots
        .iter()
        .map(|s| {
            let site = Site {
                locator: s.locator.clone(),
                base: Base::Gun,
            };
            (s.id.clone(), site)
        })
        .collect();
    let mut parts: Vec<OwnPart> = vec![];
    for (id, named) in own {
        let module = catalog.module(id)?;
        let attaches = module.attaches_on(item);
        let slot = named
            .as_ref()
            .filter(|s| slots.contains_key(*s))
            .or_else(|| attaches.iter().map(|a| &a.slot).find(|s| slots.contains_key(*s)))
            .with_context(|| format!("{item}: {id}: the gun has none of its slots"))?
            .clone();
        let at = parts.len();
        parts.push(OwnPart {
            module: id.clone(),
            slot: slot.clone(),
            site: slots[&slot].clone(),
            stock: false,
        });
        // A module in a slot its entry names without rules for it brings none.
        if let Some(rules) = module.attach_on(item, &slot) {
            for s in rules.overrides.iter().chain(&rules.adds) {
                let site = Site {
                    locator: s.locator.clone(),
                    base: Base::Part(at),
                };
                slots.insert(s.id.clone(), site);
            }
        }
    }
    Ok(GunPlan {
        parts,
        slots,
        fits: vec![],
        refused: vec![],
        carriers: BTreeMap::new(),
    })
}

/// The shortest chain of carriers that brings `want` to `item` with `slots`:
/// each a module of `candidates` (in 0.69d0's link order) in a slot the gun
/// has (the first) or the carrier before it brings.
fn chain(
    item: &str,
    slots: &BTreeMap<String, Site>,
    candidates: &[&ModuleDef],
    want: &str,
) -> Option<Vec<Link>> {
    // Breadth first: every chain of one, then of two, ...; the first found
    // is the shortest, and among equals the one of the earliest links.
    let mut level: Vec<(Vec<Link>, BTreeSet<&str>)> = vec![(vec![], BTreeSet::new())];
    for _ in 0..MAX_CHAIN {
        let mut next = vec![];
        for (links, brought) in &level {
            for module in candidates {
                if links.iter().any(|l| l.module == module.id) {
                    continue;
                }
                for attach in module.attaches_on(item) {
                    let stands = if links.is_empty() {
                        slots.contains_key(&attach.slot)
                    } else {
                        brought.contains(attach.slot.as_str())
                    };
                    if !stands {
                        continue;
                    }
                    let mut links = links.clone();
                    links.push(Link {
                        module: module.id.clone(),
                        slot: attach.slot.clone(),
                    });
                    if attach.adds.iter().any(|s| s.id == want) {
                        return Some(links);
                    }
                    next.push((links, attach.adds.iter().map(|s| s.id.as_str()).collect()));
                }
            }
        }
        level = next;
    }
    None
}

/// The plan of `item`, whose own modules are `own` (in attach order) and
/// whose fits are among `linked` (0.69d0's links of the item with its base
/// item's, [`Catalog::linked`], or another item's). `offered` says whether a
/// module has an item a player can fit.
pub fn plan(
    catalog: &Catalog,
    item: &str,
    own: &[(String, Option<String>)],
    linked: &[&str],
    offered: &dyn Fn(&ModuleDef) -> bool,
) -> Result<GunPlan> {
    let mut plan = stand(catalog, item, own)?;
    let linked: Vec<&ModuleDef> = linked.iter().map(|id| catalog.module(id)).collect::<Result<_>>()?;
    let fitted = |m: &ModuleDef| kind(m).is_some() && offered(m);
    let carriers: Vec<&ModuleDef> = linked
        .iter()
        .copied()
        .filter(|m| kind(m).is_none() && !m.is_suppressor() && !own.iter().any(|(id, _)| *id == m.id))
        .collect();
    // What goes on the gun, each with the slot it goes in: for the stock rule.
    let mut goes: Vec<(&ModuleDef, String)> = vec![];
    for module in linked.iter().copied().filter(|m| fitted(m)) {
        let kind = kind(module).context("a fit has a kind")?;
        let attaches = module.attaches_on(item);
        let direct = attaches.iter().map(|a| &a.slot).find(|s| plan.slots.contains_key(*s));
        let (slot, links) = match direct {
            Some(slot) => (slot.clone(), vec![]),
            None => {
                let brought = attaches
                    .iter()
                    .find_map(|a| chain(item, &plan.slots, &carriers, &a.slot).map(|links| (a.slot.clone(), links)));
                let Some(found) = brought else {
                    let names: Vec<&str> = attaches.iter().map(|a| a.slot.as_str()).collect();
                    plan.refused.push(Refused {
                        module: module.id.clone(),
                        kind,
                        why: format!(
                            "the gun has none of its slots ({}) and 0.69d0 links it no carrier that brings one",
                            names.join(", ")
                        ),
                    });
                    continue;
                };
                found
            }
        };
        if !links.is_empty() {
            match plan.carriers.get(&slot) {
                Some(known) => ensure!(
                    *known == links,
                    "{item}: two carrier chains bring {slot}: {known:?} and {links:?}"
                ),
                None => {
                    plan.carriers.insert(slot.clone(), links.clone());
                }
            }
        }
        for link in &links {
            goes.push((catalog.module(&link.module)?, link.slot.clone()));
        }
        goes.push((module, slot.clone()));
        plan.fits.push(Fit {
            module: module.id.clone(),
            kind,
            slot,
            chain: links,
        });
    }
    // Stock parts: displaced, closed or folded by something that goes on.
    for (module, slot) in &goes {
        let mut hit: Vec<usize> = plan.occupant(slot).into_iter().collect();
        if let Some(rules) = module.attach_on(item, slot) {
            hit.extend(rules.closes.iter().filter_map(|s| plan.occupant(s)));
            for (altered, _) in &rules.alters {
                if let Some(at) = plan.occupant(altered) {
                    let own = catalog.module(&plan.parts[at].module)?;
                    if own.models_on(item).alt.is_some() {
                        hit.push(at);
                    }
                }
            }
        }
        for at in hit {
            plan.parts[at].stock = true;
        }
    }
    // And whatever stands on one.
    loop {
        let on_stock: Vec<usize> = plan
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.stock && matches!(p.site.base, Base::Part(on) if plan.parts[on].stock))
            .map(|(at, _)| at)
            .collect();
        if on_stock.is_empty() {
            break;
        }
        for at in on_stock {
            plan.parts[at].stock = true;
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BinaryConfig;

    /// 0.69d0's own `gameplay/game.options` (a file), when the machine has
    /// it: `VOSTOK_069D_GAME_OPTIONS`.
    fn catalog() -> Option<Catalog> {
        let path = std::env::var_os("VOSTOK_069D_GAME_OPTIONS")?;
        let game = BinaryConfig::parse(&std::fs::read(path).unwrap()).unwrap().root;
        Some(Catalog::read(&game).unwrap())
    }

    /// A gun with every default module of its own, taking what 0.69d0 sells
    /// (it rolls stats on it) and every optic.
    fn planned(catalog: &Catalog, item: &str) -> GunPlan {
        let own: Vec<(String, Option<String>)> = catalog
            .gun(item)
            .unwrap()
            .defaults
            .iter()
            .map(|e| (e.module.clone(), e.slot.clone()))
            .collect();
        let offered = |m: &ModuleDef| kind(m) == Some(Kind::Scope) || catalog.has_stats(m);
        plan(catalog, item, &own, &catalog.linked(item), &offered).unwrap()
    }

    fn fit<'a>(plan: &'a GunPlan, module: &str) -> &'a Fit {
        plan.fits
            .iter()
            .find(|f| f.module == module)
            .unwrap_or_else(|| panic!("{module} is no fit"))
    }

    fn chain(fit: &Fit) -> Vec<(&str, &str)> {
        fit.chain.iter().map(|l| (l.module.as_str(), l.slot.as_str())).collect()
    }

    #[test]
    fn the_ak_74m_takes_a_brake_on_its_thread_and_a_grip_on_a_tactical_forend() {
        let Some(catalog) = catalog() else { return };
        let plan = planned(&catalog, "ak_74m");
        let stock: Vec<&str> = plan.stock().map(|(_, p)| p.module.as_str()).collect();
        assert_eq!(stock, ["ak74_m24x15_muzzlebrake", "ak74_plastic_forend"]);
        let brake = fit(&plan, "zenit_dtk3_m24x15_muzzlebrake");
        assert_eq!((brake.kind, brake.slot.as_str(), brake.chain.len()), (Kind::Muzzle, "m24x15_muzzle", 0));
        let grip = fit(&plan, "fabdefence_ptk_foregrip");
        assert_eq!(grip.kind, Kind::Grip);
        assert_eq!(chain(grip), [("ak_midwest_extended_tactical_forend", "forend")]);
        assert_eq!(chain(fit(&plan, "pso1_x4_scope")), []);
        // A rail optic goes on the dovetail mount, not on the forend's handguard rail.
        let optic = fit(&plan, "aimpoint_compm4_collimator");
        assert_eq!(optic.slot, "picatinny_scope_medium");
        assert_eq!(chain(optic), [("custom_dovetail_01_picatinnyrail", "scope_dovetail")]);
    }

    #[test]
    fn a_carrier_on_the_thread_brings_the_p90_its_devices_and_grips() {
        let Some(catalog) = catalog() else { return };
        let plan = planned(&catalog, "fn_p90");
        let forend = [("p90_acm_forend", "m12x1l_muzzle")];
        assert_eq!(chain(fit(&plan, "lantac_dragon_i05x28_muzzlebrake")), forend);
        assert_eq!(chain(fit(&plan, "fabdefence_ptk_foregrip")), forend);
        // Its own brake stands where the forend goes.
        assert!(plan.stock().any(|(_, p)| p.module == "fnp90_m12x1l_muzzlebrake"));
    }

    #[test]
    fn the_rail_bracket_brings_the_pkp_its_own_foregrip() {
        let Some(catalog) = catalog() else { return };
        let plan = planned(&catalog, "pkp_pecheneg");
        assert_eq!(
            chain(fit(&plan, "pkp_pecheneg_foregrip")),
            [("medium_picatinnyrail", "grip_picatinnyrail")]
        );
    }

    #[test]
    fn no_suppressor_is_fitted_or_carries() {
        let Some(catalog) = catalog() else { return };
        for item in ["a9_91", "vsk_94", "ak_74m", "mp7"] {
            let plan = planned(&catalog, item);
            let named = plan
                .fits
                .iter()
                .flat_map(|f| std::iter::once(&f.module).chain(f.chain.iter().map(|l| &l.module)));
            for module in named {
                assert!(!catalog.module(module).unwrap().is_suppressor(), "{item}: {module}");
            }
        }
        // The grips that hang on a suppressor's clamp are refused, with the reason.
        let refused = planned(&catalog, "a9_91").refused;
        assert!(refused.iter().any(|r| r.kind == Kind::Grip && r.why.contains("picatinny_grip")));
    }
}
