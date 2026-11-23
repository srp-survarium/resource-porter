// SPDX-License-Identifier: GPL-3.0-or-later
//! The released scope pairs 0.69d0 does not give (`port-modules --released`).
//!
//! Up to skhid-srp's release 68 a scope went on a weapon by a variant: a
//! scope item made for one pair, its idle model the optic's mesh moved by
//! one translation in the frame of the weapon model's `scope_point`, with
//! the rail it needs in the same model. Every such pair is one a player
//! may wear, so every one is a fit of the table. Where 0.69d0's own data
//! gives the pair, the plan has it and nothing here runs. Where it does
//! not, the pair is a **legacy row**:
//!
//! 1. *As its base gun.* A premium item with links of its own but no optic
//!    among them (the SR-3M's) takes the optic as its base gun does, when
//!    0.69d0's rules place it on the item's own parts: a fit of the plan as
//!    any other, checked against 0.69d0.
//! 2. *Where its variant put it.* Else the optic stands where the released
//!    variant's model has it: the optic's 0.69d0 mesh is found in the
//!    variant's idle model (vertex for vertex, else by its box), which gives
//!    the translation; the gun's record gets a slot of the optic's own
//!    (the first slot its module names that the gun lacks: the Docter's is
//!    0.69d0's `none`, which no gun has) at `scope_point` moved by it, and a
//!    row of its own for the optic in that slot, its rules 0.69d0's for it.
//!    The variant's other surfaces (a rail) become a carrier of that gun
//!    alone, a model made of them as they are, in a second slot at
//!    `scope_point`; it brings the optic's slot.
//!
//! Nothing is typed per pair: the list is the released pairs (skhid-srp's
//! `scripts/scope-fits/released-pairs.tsv`), the places are read from the
//! players' files. A pair that can be neither given nor placed is a row of
//! [`Lost`] with its reason, and the build says so.

use super::*;

/// A released (weapon, scope) pair and the variant that showed it.
#[derive(Clone, Debug)]
pub struct Released {
    /// The weapon item's name.
    pub weapon: String,
    /// The scope item's dict id.
    pub scope: u16,
    /// The variant's dict id and config.
    pub variant: u16,
    pub variant_config: String,
}

/// `released-pairs.tsv`: weapon, its dict id, scope, its dict id, the
/// variant's dict id, the variant's config (`#` comments).
pub fn read_released(path: &Path) -> Result<Vec<Released>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            let [weapon, _, _, scope, variant, config] = cols[..] else {
                bail!("{}: a row of {} columns: {l}", path.display(), cols.len())
            };
            Ok(Released {
                weapon: weapon.to_owned(),
                scope: scope.parse().with_context(|| format!("scope dict id {scope}"))?,
                variant: variant.parse().with_context(|| format!("variant dict id {variant}"))?,
                variant_config: config.to_owned(),
            })
        })
        .collect()
}

/// A legacy row of the table.
#[derive(Clone, Debug, Serialize)]
pub struct LegacyReport {
    pub gun: String,
    pub module: String,
    /// `base gun <item>` or `variant <config>`.
    pub how: String,
    /// The slot the optic goes in.
    pub slot: String,
    /// How the optic was found in the variant's model (`vertices`, `box`) and how far its
    /// vertices (its box's sides) are from 0.69d0's moved by the translation, mm.
    pub found: String,
    pub residual_mm: f64,
    /// The translation, in the frame of the variant's model.
    pub moved: [f64; 3],
    /// The variant's rail surfaces, drawn as the gun's own carrier, and the slot it stands in.
    pub rails: Vec<String>,
    pub rail_slot: String,
    /// Through the written table: how far `resolve`'s optic frame is from the variant's, mm.
    pub optic_mm: f64,
}

/// A released pair the table does not have.
#[derive(Clone, Debug, Serialize)]
pub struct Lost {
    pub gun: String,
    pub scope: u16,
    pub why: String,
}

/// The seats a legacy rail may take: 0.69d0's rail-seat slots, the first
/// the gun lacks.
const RAIL_SEATS: [&str; 4] = ["scope_picatinnyrail", "scope_dovetail", "g3a3_scope_picatinnyrail", "mosin_scope"];
/// The slots a legacy optic takes under another id when every slot it names is one some fit of the
/// gun names too (the XPS2 on the Kedr, whose every rail slot the aperture sight names before its
/// own short rail): ids only 0.69d0's piggyback and angled sights name, none of them fits here.
const SPARE: [&str; 4] = ["none", "elcan_collimator_mount", "angled_collim_short_l", "angled_collim_short_r"];
/// The optic's vertices in a variant are 0.69d0's moved by one translation
/// within this (metres), or its box has 0.69d0's sides within it.
const SAME: f32 = 0.001;

/// What a legacy row of the second kind adds to a gun's record.
pub(super) struct Placed {
    pub report: LegacyReport,
    /// The optic's row.
    pub row: usize,
    /// Where the variant has the optic: what `resolve`'s frame is checked against.
    pub at: Placement,
}

/// A static model of the players' files: its LOD0 surfaces, each with its
/// material and mesh.
fn target_static(target: &mut ResourceDb, model: &str) -> Result<Vec<(String, String, Vec<u8>)>> {
    let root = format!("models/{model}.model");
    let settings = parse_config(target, &format!("{root}/settings"), "a variant's model")?.root;
    lod0_surfaces(&settings, &root)?
        .into_iter()
        .map(|(surface, material)| {
            let bytes = target.read(&format!("{root}/render/{surface}/converted_model"))?;
            Ok((surface, material, bytes))
        })
        .collect()
}

/// Where 0.69d0's optic (`donor`: its surfaces' vertices) is in a variant's
/// model (`variant`): the translation, how it was found, how well it fits
/// (metres), and which of the variant's surfaces is the optic.
fn find(donor: &[(String, Vec<[f32; 3]>)], variant: &[(String, Vec<[f32; 3]>)]) -> Option<([f32; 3], &'static str, f32, String)> {
    let mut by_size: Vec<&(String, Vec<[f32; 3]>)> = donor.iter().collect();
    by_size.sort_by_key(|(_, points)| std::cmp::Reverse(points.len()));
    // Vertex for vertex.
    for (_, ours) in &by_size {
        for (name, theirs) in variant.iter().filter(|(_, theirs)| theirs.len() == ours.len()) {
            let n = ours.len() as f32;
            let by: [f32; 3] = std::array::from_fn(|k| theirs.iter().zip(ours.iter()).map(|(a, b)| a[k] - b[k]).sum::<f32>() / n);
            let worst = theirs
                .iter()
                .zip(ours.iter())
                .map(|(a, b)| (0..3).map(|k| (a[k] - b[k] - by[k]).powi(2)).sum::<f32>().sqrt())
                .fold(0.0, f32::max);
            if worst <= SAME {
                return Some((by, "vertices", worst, name.clone()));
            }
        }
    }
    // The same shape written otherwise: a surface whose box has the sides of a surface of 0.69d0's.
    for (_, ours) in &by_size {
        let Some(a) = Bounds::of(ours.iter().copied()) else { continue };
        for (name, theirs) in variant {
            let Some(b) = Bounds::of(theirs.iter().copied()) else { continue };
            let worst = (0..3)
                .map(|k| ((b.max[k] - b.min[k]) - (a.max[k] - a.min[k])).abs())
                .fold(0.0, f32::max);
            if worst <= SAME {
                let by = std::array::from_fn(|k| (b.min[k] + b.max[k] - a.min[k] - a.max[k]) / 2.0);
                return Some((by, "box", worst, name.clone()));
            }
        }
    }
    None
}

/// A legacy row of the second kind for `released` on `live` (the gun `b`,
/// its record `gun` so far): the optic `module` where the variant has it.
#[allow(clippy::too_many_arguments)]
pub(super) fn place(
    released: &Released,
    module: &ModuleDef,
    b: &Built,
    live: &str,
    weapon_config: &V,
    donor: &mut Donor069,
    target: &mut ResourceDb,
    work: &Path,
    maker: &ModuleMaker,
    rows: &mut Rows,
    gun: &mut GunSource,
    named: &[SlotId],
) -> Result<Placed> {
    let item = &b.item;
    let variant = parse_config(target, &released.variant_config, "a variant's config")?.root;
    let idle = string_of(get(&variant, "data.idle_model"))?.to_owned();
    // The weapon model the variant sets (its `scope_point` is the optic's frame), else the gun's own.
    let set = list(get(&variant, "modifications"))
        .iter()
        .flat_map(|row| list(row.named("modifiers")))
        .find(|m| {
            let path: Vec<&str> = list(m.named("path")).iter().filter_map(|p| string_of(Some(p)).ok()).collect();
            path == ["object", "model"]
        })
        .and_then(|m| string_of(m.named("value")).ok());
    let weapon_model = match set {
        Some(model) => model.to_owned(),
        None => string_of(get(weapon_config, "object.model"))?.to_owned(),
    };
    let props = parse_config(
        target,
        &format!("models/{weapon_model}.skinned_model/render/export_properties"),
        "the variant's weapon model",
    )?
    .root;
    let all = locators(&props)?;
    let point = all
        .iter()
        .find(|l| l.name == "scope_point")
        .with_context(|| format!("{weapon_model} has no scope_point"))?;
    let bone = usize::try_from(point.bone_idx)?;
    ensure!(
        b.composed.ship_bind.get(bone).is_some_and(|(name, _)| *name == point.bone),
        "{weapon_model}: scope_point hangs on bone {bone} `{}`, which the remodel's rig has otherwise",
        point.bone
    );
    // The optic in the variant's model.
    let model = module.models_on(item).model.with_context(|| format!("{} has no model", module.id))?;
    let source = format!("models/{model}.model");
    let settings = db_config(&mut donor.db, &format!("{source}/settings"))?;
    let mut ours = vec![];
    for (surface, _) in lod0_surfaces(&settings, &source)? {
        let bytes = donor.db.read(&format!("{source}/render/{surface}/converted_model"))?;
        ours.push((surface, Mesh::parse(&super::super::convert::static_069_to_026(&bytes)?)?.positions));
    }
    let surfaces = target_static(target, &idle)?;
    let theirs: Vec<(String, Vec<[f32; 3]>)> = surfaces
        .iter()
        .map(|(name, _, bytes)| Ok((name.clone(), Mesh::parse(bytes)?.positions)))
        .collect::<Result<_>>()?;
    let (by, found, residual, optic_surface) =
        find(&ours, &theirs).with_context(|| format!("{idle}: 0.69d0's {} is not in it (neither its vertices nor its box)", module.id))?;
    // The variant's vertex `v + by` is drawn at `half turn x scope_point x bone`: the optic's own
    // frame there is the record `translation(half turn of by) x scope_point` on that bone.
    let moved = HALF_TURN.apply(by.map(f64::from));
    let seat = Placement {
        bone: weapon_modules::BoneIndex(u16::try_from(bone)?),
        offset: locator_frame(point).affine(),
    };
    let at = Placement {
        bone: seat.bone,
        offset: Frame::translation(moved.map(|c| c as f32)).then(&locator_frame(point)).affine(),
    };
    // The slots: the optic's own first that the gun lacks; a rail's seat likewise. Of those, first
    // one no module the gun takes names (`named`): `resolve` puts a module in the first slot it
    // names that the gun has, so a slot another fit names would draw that fit onto this rail.
    let has = |gun: &GunSource, slot: SlotId| gun.slots.iter().any(|(s, _, _)| *s == slot) || gun.carriers.iter().any(|(s, _)| *s == slot);
    let free = |gun: &GunSource, choices: &[SlotId], but: Option<SlotId>| {
        let lacking: Vec<SlotId> = choices.iter().copied().filter(|id| !has(gun, *id) && Some(*id) != but).collect();
        lacking.iter().find(|id| !named.contains(id)).or(lacking.first()).map(|id| (id.name(), *id))
    };
    let names: Vec<SlotId> = module.attaches_on(item).iter().filter_map(|a| SlotId::from_name(&a.slot)).collect();
    let slot = free(gun, &names, None).with_context(|| format!("{}: the gun has every slot it names", module.id))?;
    // Every slot it names is another fit's: its attach for that slot under a spare id.
    let spare = if named.contains(&slot.1) {
        let spares: Vec<SlotId> = SPARE.iter().filter_map(|name| SlotId::from_name(name)).collect();
        let lacking: Vec<SlotId> = spares.into_iter().filter(|id| !has(gun, *id) && !named.contains(id)).collect();
        Some(*lacking.first().with_context(|| format!("{}: no spare slot", module.id))?)
    } else {
        None
    };
    let optic_names: BTreeSet<&str> = ours.iter().map(|(name, _)| name.as_str()).collect();
    let rails: Vec<&(String, String, Vec<u8>)> = surfaces
        .iter()
        .filter(|(name, _, _)| *name != optic_surface && !optic_names.contains(name.as_str()) && !name.starts_with(LENS))
        .collect();
    // (The optic's row first: nothing of the gun's record is changed when it cannot be made.)
    let mut row = maker
        .source(donor, target, work, module, Role::Fitted(weapon_modules::Kind::Scope), item, Some(&[slot.0]))
        .with_context(|| format!("{live}: legacy row {}", module.id))?;
    let id = spare.unwrap_or(slot.1);
    for attach in &mut row.attaches {
        attach.slot = id;
    }
    let mut rail_slot = String::new();
    if rails.is_empty() {
        gun.slots.push((id, at.bone.0, at.offset));
    } else {
        let seats: Vec<SlotId> = RAIL_SEATS.iter().filter_map(|name| SlotId::from_name(name)).collect();
        let seat_slot = free(gun, &seats, Some(id)).context("the gun has every rail seat")?;
        // The rail: the variant's own surfaces as a model of its own, at `scope_point`.
        let name = idle.rsplit('/').next().unwrap_or(&idle);
        let path = format!("{MODULES_ROOT}/legacy/{name}_rail");
        let root = format!("models/{path}.model");
        if !donor.files.contains_key(&format!("{root}/settings")) {
            let own: Vec<(String, String, Vec<u8>)> = rails.iter().map(|r| (*r).clone()).collect();
            donor.put_static(&root, &format!("skhid-srp release 68: models/{idle}.model"), &own, target)?;
        }
        let mut attach = AttachSource::new(seat_slot.1);
        // The optic's slot on it: `by`, as a module's own locator (`half turn x L`).
        attach.adds.push((id, HALF_TURN.then(&Frame::translation(by)).affine()));
        let rail = rows.row(ModuleSource {
            dict: released.variant,
            role: Role::Carrier,
            model: path,
            alt: None,
            aimed: None,
            attaches: vec![attach],
            hand: None,
        });
        gun.slots.push((seat_slot.1, seat.bone.0, seat.offset));
        gun.carriers.push((id, vec![(rail, seat_slot.1)]));
        rail_slot = seat_slot.0.to_owned();
    }
    let row = rows.row(row);
    Ok(Placed {
        report: LegacyReport {
            gun: live.to_owned(),
            module: module.id.clone(),
            how: format!("variant {}", released.variant_config),
            slot: spare.map_or_else(|| slot.0.to_owned(), |id| format!("{} (its {} attach)", id.name(), slot.0)),
            found: format!("{found} of {optic_surface}"),
            residual_mm: f64::from((residual * 1e5).round() / 100.0),
            moved: by.map(|c| f64::from((c * 1e5).round() / 1e5)),
            rails: rails.iter().map(|(name, _, _)| name.clone()).collect(),
            rail_slot,
            optic_mm: -1.0,
        },
        row,
        at,
    })
}
