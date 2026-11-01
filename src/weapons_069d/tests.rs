// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn row(id: i32, name: &str, cfg: &str, category: i32) -> V {
    V::Named(vec![
        field("cfg_name", s(cfg)),
        field("is_stack", V::Boolean(category == 19)),
        field("name", s(name)),
        field("item_category", V::Integer(category)),
        field("dict_id", V::Integer(id)),
        field("base_cost", V::Integer(1000)),
        field("is_premium", V::Boolean(false)),
    ])
}

fn compat(n: u32, first: i32, second: i32) -> NamedValue {
    field(
        &format!("compatibility_{n}"),
        V::Named(vec![
            field("second_item", V::Integer(second)),
            field("first_item", V::Integer(first)),
        ]),
    )
}

/// The live rows the guns' templates and ammo name.
fn dictionary() -> V {
    let mut rows = vec![];
    let mut pairs = vec![];
    let mut id = 100;
    for gun in GUNS {
        id += 1;
        rows.push(row(id, &format!("template_{}", gun.name), gun.template, 8));
        if let Ammo::Like(peer) = gun.ammo {
            id += 1;
            rows.push(row(
                id,
                peer,
                &format!("gameplay/items/weapons/wpn_{peer}.options"),
                8,
            ));
            pairs.push((id, 7));
            pairs.push((id, 8));
        }
    }
    rows.push(row(
        7,
        "ammo_a",
        "gameplay/items/weapons/ammo/ammo_a.options",
        19,
    ));
    rows.push(row(
        8,
        "ammo_b",
        "gameplay/items/weapons/ammo/ammo_b.options",
        19,
    ));
    rows.push(row(
        22,
        "ammo_12mm_buck",
        "gameplay/items/weapons/ammo/ammo_12mm_buck.options",
        19,
    ));
    V::Named(vec![
        field("items_dict", V::Indexed(rows)),
        field(
            "items_compatibility",
            V::Named(
                pairs
                    .iter()
                    .enumerate()
                    .map(|(n, (a, b))| compat(n as u32 + 1, *a, *b))
                    .collect(),
            ),
        ),
    ])
}

#[test]
fn every_gun_gets_a_row_and_its_peers_ammo() {
    let mut d = dictionary();
    let added = dictionary_rows(&mut d, &GUNS).unwrap();
    let rows = list(d.named("items_dict"));
    for gun in GUNS {
        let row = row_of(rows, "dict_id", &V::Integer(gun.dict_id)).unwrap();
        assert_eq!(row.named("cfg_name"), Some(&s(gun.config)));
        assert_eq!(
            row.named("base_cost"),
            Some(&V::Integer(gun.stats.base_cost))
        );
    }
    let ammo = row_of(rows, "dict_id", &V::Integer(AMMO_23MM.dict_id)).unwrap();
    assert_eq!(ammo.named("name"), Some(&s("ammo_23mm_r")));
    assert_eq!(ammo.named("item_category"), Some(&V::Integer(19)));
    // Five guns take their peer's two ammo, the KS-23M its own.
    assert_eq!(added.len(), 5 * 2 + 1);
    let V::Named(c) = d.named("items_compatibility").unwrap() else {
        panic!()
    };
    assert!(c.iter().any(
        |r| r.value.named("first_item") == Some(&V::Integer(KS23M.dict_id))
            && r.value.named("second_item") == Some(&V::Integer(AMMO_23MM.dict_id))
    ));
}

#[test]
fn ids_names_and_configs_must_be_free() {
    let mut d = dictionary();
    dictionary_rows(&mut d, &GUNS).unwrap();
    assert!(dictionary_rows(&mut d, &[&AKMSU]).is_err());
}

#[test]
fn ids_are_the_batchs_claimed_ones() {
    // Shared claims of the release-41 weapon batches: weapons 2290..=2299,
    // the 23 mm ammo 2253..=2259.
    for gun in GUNS {
        assert!((2290..=2299).contains(&gun.dict_id), "{}", gun.name);
    }
    assert!((2253..=2259).contains(&AMMO_23MM.dict_id));
    let mut ids: Vec<i32> = GUNS.iter().map(|g| g.dict_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), GUNS.len());
    let mut cells: Vec<[i32; 2]> = GUNS.iter().map(|g| g.icon).collect();
    cells.sort_unstable();
    cells.dedup();
    assert_eq!(cells.len(), GUNS.len());
}

#[test]
fn attachments_are_never_baked() {
    for (id, model) in [
        (
            "akmsu_m24x15_muzzlebrake",
            "weapons/modules/muzzlebrakes/akmsu_m24x15_muzzlebrake",
        ),
        ("x_silencer", "weapons/modules/silencers/x"),
        ("rk2_foregrip", "weapons/modules/foregrips/rk2"),
        ("pk01vi_scope", "weapons/modules/scopes/pk01vi"),
    ] {
        assert!(is_attachment(id, model), "{id}");
    }
    for (id, model) in [
        (
            "akmsu_default_forend",
            "weapons/modules/forends_handguards/akmsu_forend",
        ),
        (
            "ak_762x39_30_round_sb7_default_mag",
            "weapons/modules/mags/ak_762x39_30_round_sb7_mag",
        ),
        (
            "akm_sb8_wood_default_pistolgrip",
            "weapons/modules/pistolgrips/akm_sb8_wood_pistolgrip",
        ),
        (
            "ks_23_short_barrel",
            "weapons/modules/barrels/ks_23_barrel_rifling_short",
        ),
    ] {
        assert!(!is_attachment(id, model), "{id}");
    }
    // Every gun lists its muzzle device as separable; none lists a part.
    for gun in GUNS {
        for id in gun.separable {
            assert!(is_attachment(id, ""), "{}: {id}", gun.name);
        }
    }
}

#[test]
fn new_files_go_under_the_ports_folder() {
    assert_eq!(
        ns_name("weapons/assault/akmsu/akmsu").unwrap(),
        "weapons/v069d/assault/akmsu/akmsu"
    );
    assert!(ns_name("characters/x").is_err());
    assert_eq!(
        ns_clip("resources/animations/single/weapons/g3a3/item/stand/3rd_view/idle").unwrap(),
        "resources/animations/single/weapons/v069d/g3a3/item/stand/3rd_view/idle"
    );
    assert_eq!(AKMSU.model_ref(), "weapons/v069d/akmsu/akmsu");
}

#[test]
fn aimed_shots_have_no_026g_slot() {
    assert!(drops_clip(
        "resources/animations/single/weapons/mauser/item/stand/3rd_view/shoot_calim"
    ));
    assert!(!drops_clip(
        "resources/animations/single/weapons/mauser/item/stand/3rd_view/shoot_empty"
    ));
    assert!(drops_clip(
        "resources/animations/single/weapons/ks_23/item/stand/3rd_view/reload_finalround"
    ));
    assert!(!drops_clip(
        "resources/animations/single/weapons/ks_23/item/stand/3rd_view/reload_start"
    ));
}

#[test]
fn the_pump_shotgun_takes_a_chambering_time() {
    assert!(KS23M.stats.chamber_a_round_time.is_some());
    assert!(KS23M.states.iter().any(|p| p.state == "chamber_a_round"));
    for gun in GUNS {
        if gun.name != "ks_23m" {
            assert!(gun.stats.chamber_a_round_time.is_none(), "{}", gun.name);
        }
    }
}

/// The runtime pointer slots of a 0.26g clip (filled on load; retail files
/// hold whatever the exporter's memory did) and the padding after each event
/// name, zeroed.
fn without_runtime_bytes(b: &[u8]) -> Vec<u8> {
    let h = convert::clip_header(b).unwrap();
    let mut out = b.to_vec();
    for slot in 0..h.bones * 9 {
        out[4 + slot * 8 + 4..4 + slot * 8 + 8].fill(0);
    }
    let base = 4 + h.bones * 0x48;
    for event in 0..h.events {
        let at = base + event * 16;
        out[at + 4..at + 8].fill(0);
        out[at + 12..at + 16].fill(0);
    }
    for e in convert::events_026(b).unwrap() {
        // Name, NUL, then padding to four bytes.
        let needle = [e.name.as_bytes(), &[0]].concat();
        let start = out
            .windows(needle.len())
            .rposition(|w| w == needle.as_slice())
            .unwrap();
        let mut at = start + needle.len();
        while at % 4 != 0 && at < out.len() {
            out[at] = 0;
            at += 1;
        }
    }
    out
}

/// With the stock 0.26g database (`VOSTOK_026G_DB`) and an archive of
/// 0.69d0's `animations/single/weapons/ak_12` (`VOSTOK_069D_AK12_DB`, from
/// `extract-bundle` and `pack-donor-tree`): the AK-12's
/// clips that both builds ship, converted, are 0.26g's own bytes.
#[test]
fn shared_ak12_clips_convert_to_the_026g_bytes() {
    let (Some(stock), Some(donor)) = (
        std::env::var_os("VOSTOK_026G_DB"),
        std::env::var_os("VOSTOK_069D_AK12_DB"),
    ) else {
        eprintln!("skipped: set VOSTOK_026G_DB and VOSTOK_069D_AK12_DB");
        return;
    };
    let mut stock = ResourceDb::open(stock).unwrap();
    let mut donor = ResourceDb::open(donor).unwrap();
    for clip in [
        "item/stand/3rd_view/show",
        "item/stand/3rd_view/hide",
        "player/stand/3rd_view/on_site/hands_only/hide",
    ] {
        let path = format!("animations/single/weapons/ak_12/{clip}.b-spline");
        let native = stock.read(&path).unwrap();
        let converted = convert::bspline_069_to_026(&donor.read(&path).unwrap()).unwrap();
        assert_eq!(
            without_runtime_bytes(&converted),
            without_runtime_bytes(&native),
            "{clip}"
        );
        let names = format!("animations/single/weapons/ak_12/{clip}.bones_names");
        assert_eq!(
            clip_bones(&donor.read(&names).unwrap()).unwrap(),
            clip_bones(&stock.read(&names).unwrap()).unwrap()
        );
    }
}
