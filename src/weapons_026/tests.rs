// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn row(id: i32, name: &str, cfg: &str) -> V {
    V::Named(vec![
        field("cfg_name", s(cfg)),
        field("is_stack", V::Boolean(false)),
        field("name", s(name)),
        field("item_category", V::Integer(8)),
        field("dict_id", V::Integer(id)),
        field("base_cost", V::Integer(115_000)),
        field("is_premium", V::Boolean(false)),
    ])
}

fn dictionary() -> V {
    V::Named(vec![
        field(
            "items_dict",
            V::Indexed(vec![
                row(363, "ak_12", CONFIG_TEMPLATE),
                row(217, "ammo_5.45x39_7n10", "gameplay/items/weapons/ammo/ammo_5.45x39_7n10.options"),
                row(331, "ammo_5.45x39_7n22", "gameplay/items/weapons/ammo/ammo_5.45x39_7n22.options"),
            ]),
        ),
        field(
            "items_compatibility",
            V::Named(vec![field(
                "compatibility_93",
                V::Named(vec![
                    field("second_item", V::Integer(217)),
                    field("first_item", V::Integer(3)),
                ]),
            )]),
        ),
    ])
}

#[test]
fn both_rifles_get_rows_and_take_the_ak_74ms_545_ammo() {
    let mut dict = dictionary();
    let added = dictionary_rows(&mut dict, &RIFLES).unwrap();
    assert_eq!(
        added,
        ["compatibility_94", "compatibility_95", "compatibility_96", "compatibility_97"]
    );
    let V::Indexed(items) = dict.named("items_dict").unwrap() else {
        panic!()
    };
    let hk = items
        .iter()
        .find(|v| v.named("dict_id") == Some(&V::Integer(1820)))
        .unwrap();
    assert_eq!(hk.named("cfg_name"), Some(&s(HK416.config)));
    assert_eq!(hk.named("item_category"), Some(&V::Integer(8)), "an assault rifle");
    assert_eq!(hk.named("base_cost"), Some(&V::Integer(HK416.stats.base_cost)));
    let pairs: Vec<(i32, i32)> = match dict.named("items_compatibility").unwrap() {
        V::Named(c) => c
            .iter()
            .skip(1)
            .map(|c| {
                (
                    int_of(c.value.named("first_item")).unwrap(),
                    int_of(c.value.named("second_item")).unwrap(),
                )
            })
            .collect(),
        _ => panic!(),
    };
    assert_eq!(pairs, [(1820, 217), (1820, 331), (1821, 217), (1821, 331)]);
}

#[test]
fn ids_names_and_configs_must_be_free() {
    let mut dict = dictionary();
    dictionary_rows(&mut dict, &RIFLES).unwrap();
    // A second run finds its own rows.
    assert!(dictionary_rows(&mut dict, &RIFLES).is_err());
    let mut dict = dictionary();
    if let Some(V::Indexed(items)) = dict.named_mut("items_dict") {
        items[1] = row(217, "ammo_7.62x39", "x");
    }
    assert!(dictionary_rows(&mut dict, &RIFLES).is_err(), "217 must be 5.45x39");
}

#[test]
fn ids_stay_clear_of_the_equipment_packs() {
    for rifle in RIFLES {
        assert!((1820..=1821).contains(&rifle.dict_id));
        assert!(!(1800..=1810).contains(&rifle.dict_id));
    }
    assert_ne!(HK416.icon, RADON.icon);
    for cell in [HK416.icon, RADON.icon] {
        // The weapon atlas's third column is empty from row 7 (stock and
        // every Skhid patch); the equipment packs draw only equipment icons.
        assert!(cell[1] == 3 && (7..=22).contains(&cell[0]));
    }
}

#[test]
fn reload_cues_stay_inside_their_sound_lists() {
    for rifle in RIFLES {
        for events in [rifle.empty_events, rifle.tactical_events].into_iter().flatten() {
            assert!(events.iter().all(|e| usize::from(*e) < rifle.reload_sounds.len()));
        }
    }
    // The Radon's tactical reload stows the magazine instead of dropping it.
    let tactical: Vec<&str> = RADON
        .tactical_events
        .unwrap()
        .iter()
        .map(|i| RADON.reload_sounds[usize::from(*i)])
        .collect();
    assert!(tactical.contains(&"weapons/hk416/wear_line"));
    assert!(!tactical.contains(&"weapons/hk416/mag_fall"));
}

#[test]
fn every_text_exists_in_every_language() {
    for rifle in RIFLES {
        for (key, text) in &rifle.texts {
            for lang in LANGUAGES {
                let t = text.get(lang).unwrap();
                assert!(!t.is_empty(), "{key} {lang}");
                assert!(t.is_ascii() || lang != "english", "{key}: {t}");
            }
        }
    }
}

#[test]
fn composites_name_collections_with_their_offsets() {
    let c = composite(HK416.fire_first, HK416.fire_first_master);
    let items = match c.named("composite_sound").unwrap().named("sound_items") {
        Some(V::Indexed(i)) => i.clone(),
        _ => panic!(),
    };
    assert_eq!(items.len(), HK416.fire_first.len());
    assert_eq!(items[0].named("filename"), Some(&s("weapons/hk416/hk416_shutter")));
    assert_eq!(items[0].named("start_offset"), Some(&V::Integer(60)));
    assert_eq!(items[0].named("resource_type"), Some(&V::Integer(258)));
    // The master layer is the H&K's own report.
    let master = int_of(c.named("composite_sound").unwrap().named("master_layer_index")).unwrap();
    assert_eq!(HK416.fire_first[master as usize].collection, "weapons/hk416/hk416_loud");
}
