// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn t(fields: Vec<(&str, V)>) -> V {
    V::Named(fields.into_iter().map(|(n, v)| field(n, v)).collect())
}
fn list(items: Vec<V>) -> V {
    V::Indexed(items)
}

#[test]
fn conform_keeps_the_templates_keys_with_the_donors_values() {
    let template = t(vec![
        ("a", V::Float(0.5)),
        ("b", t(vec![("c", V::Integer(1)), ("d", V::Integer(2))])),
        ("only_026", V::Boolean(true)),
    ]);
    let donor = t(vec![
        ("a", V::Integer(3)),
        ("b", t(vec![("c", V::Integer(9)), ("extra", V::Integer(7))])),
        ("only_031", V::Boolean(false)),
    ]);
    assert_eq!(
        conform(&donor, &template),
        t(vec![
            ("a", V::Float(3.0)),
            ("b", t(vec![("c", V::Integer(9)), ("d", V::Integer(2))])),
            ("only_026", V::Boolean(true)),
        ])
    );
}

fn chunk(id: u32, body: &[u8]) -> Vec<u8> {
    let mut b = id.to_le_bytes().to_vec();
    b.extend((body.len() as u32).to_le_bytes());
    b.extend_from_slice(body);
    b
}

#[test]
fn mesh_statistics_go_and_anything_else_unknown_is_refused() {
    let native = [chunk(3, &[1; 40]), chunk(4, &[2; 10])].concat();
    let donor = [native.clone(), chunk(110, &[0; 4])].concat();
    assert_eq!(strip_mesh_statistics(&donor).unwrap(), native);
    assert_eq!(strip_mesh_statistics(&native).unwrap(), native);
    assert!(strip_mesh_statistics(&[native.clone(), chunk(5, &[0; 4])].concat()).is_err());
    assert!(strip_mesh_statistics(&chunk(110, &[0; 4])).is_err());
}

fn state(kind: i32, clip: &str, sounds: Option<V>) -> V {
    let mut fields = vec![
        ("state_type", V::Integer(kind)),
        (
            "user_animations",
            list(vec![s(format!(
                "resources/animations/single/weapons/{clip}"
            ))]),
        ),
        (
            "animations",
            list(vec![s(format!(
                "resources/animations/single/weapons/{clip}_item"
            ))]),
        ),
    ];
    if let Some(sounds) = sounds {
        fields.push(("sounds", sounds));
    }
    t(fields)
}

fn sounds(name: &str) -> V {
    t(vec![
        (
            "first_view",
            list(vec![list(vec![V::Integer(258), s(name)])]),
        ),
        (
            "third_view",
            list(vec![list(vec![V::Integer(258), s(name)])]),
        ),
        ("simultaneous_sounds_queue_size", V::Integer(1)),
    ])
}

/// A rifle config with the 0.26g AK-12's shape.
fn template() -> V {
    let params: Vec<(&str, V)> = [
        "faction",
        "item_level",
        "match_making_level",
        "drop_weight",
        "rounds_per_minute",
        "magazine_capacity",
        "min_damage_distance",
        "max_damage_distance",
    ]
    .into_iter()
    .map(|k| (k, V::Integer(0)))
    .chain(
        [
            "bullet_damage",
            "bullet_pierce",
            "stopping_power",
            "weight",
            "reload_time",
            "aim_time",
            "aim_zoom_factor",
            "base_dispersion",
        ]
        .into_iter()
        .map(|k| (k, V::Float(0.0))),
    )
    .chain([
        ("pain_health_modifier", V::Integer(0)),
        ("stamina_regenation_speed_modifier", V::Integer(0)),
        (
            "fire_queue_types",
            list(vec![V::Integer(-1), V::Integer(1), V::Integer(3)]),
        ),
    ])
    .collect();
    t(vec![
        ("parameters", t(params)),
        (
            "states",
            t(vec![
                ("show", state(300, "ak_12/show", None)),
                ("hide", state(301, "ak_12/hide", None)),
                ("idle", state(302, "ak_12/idle", None)),
                (
                    "fire",
                    state(305, "ak_12/shoot", Some(sounds("weapons/ak_12"))),
                ),
                (
                    "reload",
                    state(303, "ak_12/reload", Some(sounds("weapons/ak_12/mag_out"))),
                ),
                ("melee", state(321, "ak/kick", None)),
                ("throw_grenade", state(316, "ak/grenade", None)),
            ]),
        ),
        (
            "object",
            t(vec![
                ("model", s("weapons/ak_12/ak_12")),
                ("skeleton", s("weapons/ak_12/ak_12")),
            ]),
        ),
        ("dispersion", t(vec![("base_dispersion", V::Float(0.2))])),
        (
            "ui_desc",
            t(vec![
                ("combat_log_icon", V::Integer(49)),
                (
                    "text_descriptions",
                    t(vec![
                        ("name", s("st_ak12_name")),
                        ("description", s("st_ak12_description")),
                    ]),
                ),
                ("icon", list(vec![V::Integer(3), V::Integer(3)])),
                ("props_list", list(vec![])),
            ]),
        ),
    ])
}

/// 0.31's H&K: renumbered states, renamed parameters, base/ melee.
fn donor() -> V {
    t(vec![
        (
            "parameters",
            t(vec![
                ("health_modifier", V::Integer(5)),
                ("stamina_regeneration_speed_modifier", V::Integer(6)),
                (
                    "fire_queue_types",
                    list(vec![V::Integer(-1), V::Integer(1)]),
                ),
                ("bleeding_chance", V::Integer(0)),
            ]),
        ),
        (
            "states",
            t(vec![
                ("show", state(300, "hk416_a5/show", None)),
                ("hide", state(301, "hk416_a5/hide", None)),
                ("idle", state(302, "hk416_a5/idle", None)),
                (
                    "fire",
                    state(305, "hk416_a5/shoot", Some(sounds("weapons/hk416"))),
                ),
                (
                    "reload",
                    state(
                        313,
                        "hk416_a5/reload",
                        Some(sounds("weapons/hk416/wear_line")),
                    ),
                ),
                ("melee", state(320, "base/kick_grip1", None)),
                ("throw_grenade", state(315, "base/grenade", None)),
            ]),
        ),
        (
            "object",
            t(vec![
                ("model", s("weapons/hk416_a5/hk416_a5")),
                ("skeleton", s("weapons/hk416_a5/hk416_a5")),
            ]),
        ),
        ("sway_parameters", t(vec![("use_sway", V::Boolean(true))])),
        (
            "recoil_animations",
            list(vec![s(
                "resources/animations/single/weapons/base/player/1st_view/recoil_horiz",
            )]),
        ),
        (
            "ui_desc",
            t(vec![
                ("combat_log_icon", V::Integer(63)),
                (
                    "text_descriptions",
                    t(vec![
                        ("name", s("st_hk416_name")),
                        ("description", s("st_hk416_description")),
                    ]),
                ),
                ("icon", list(vec![V::Integer(12), V::Integer(4)])),
            ]),
        ),
    ])
}

#[test]
fn a_rifle_takes_the_donors_states_with_0_26gs_state_types() {
    let cfg = rifle_config(&HK416, &donor(), &template()).unwrap();
    assert_eq!(
        get(&cfg, "states.reload.state_type"),
        Some(&V::Integer(303)),
        "0.31's 313 is 0.26g's reload 303"
    );
    assert_eq!(
        get(&cfg, "states.reload.user_animations.0"),
        Some(&s("resources/animations/single/weapons/hk416_a5/reload"))
    );
    assert_eq!(
        get(&cfg, "states.reload.sounds.first_view.0.1"),
        Some(&s("weapons/hk416/wear_line"))
    );
    // Melee and grenade stay native: the donor's play weapons/base/*.
    assert_eq!(get(&cfg, "states.melee.state_type"), Some(&V::Integer(321)));
    assert_eq!(
        get(&cfg, "states.melee.user_animations.0"),
        Some(&s("resources/animations/single/weapons/ak/kick"))
    );
    assert_eq!(
        get(&cfg, "states.throw_grenade.state_type"),
        Some(&V::Integer(316))
    );
    assert!(
        cfg.named("recoil_animations").is_none(),
        "base/ recoil clips are not taken"
    );
    // The renamed parameters keep the donor's values under 0.26g's names.
    assert_eq!(
        get(&cfg, "parameters.pain_health_modifier"),
        Some(&V::Integer(5))
    );
    assert_eq!(
        get(&cfg, "parameters.stamina_regenation_speed_modifier"),
        Some(&V::Integer(6))
    );
    assert!(
        get(&cfg, "parameters.bleeding_chance").is_none() && cfg.named("sway_parameters").is_none()
    );
    assert_eq!(
        get(&cfg, "parameters.fire_queue_types"),
        Some(&list(vec![V::Integer(-1), V::Integer(1)]))
    );
    // The donor's model and texts, this port's icon cell, the rebuilt firing sound.
    assert_eq!(
        get(&cfg, "object.model"),
        Some(&s("weapons/hk416_a5/hk416_a5"))
    );
    assert_eq!(
        get(&cfg, "ui_desc.text_descriptions.name"),
        Some(&s("st_hk416_name"))
    );
    assert_eq!(
        get(&cfg, "ui_desc.icon"),
        Some(&list(vec![V::Integer(7), V::Integer(3)]))
    );
    assert_eq!(
        get(&cfg, "ui_desc.combat_log_icon"),
        Some(&V::Integer(31)),
        "0.31's 63 is past 0.26g's HUD icons"
    );
    assert_eq!(
        get(&cfg, "states.fire.sounds.first_view.0.1"),
        Some(&s("weapons/hk416"))
    );
    assert_eq!(get(&cfg, "parameters.faction"), Some(&V::Integer(4)));
    assert_eq!(get(&cfg, "parameters.item_level"), Some(&V::Integer(9)));
    assert_eq!(get(&cfg, "parameters.bullet_damage"), Some(&V::Float(0.4)));
}

#[test]
fn the_card_shows_the_stats_as_native_rifles_do() {
    let shown: Vec<(i32, String)> = card_props(&HK416.stats).into_iter().collect();
    let expected = [
        (1, "40"),
        (3, "30"),
        (133, "0.3"),
        (132, "15"),
        (2, "0.2"),
        (24, "50"),
        (4, "850"),
        (15, "3.5"),
    ];
    assert_eq!(shown, expected.map(|(i, v)| (i, v.to_owned())));
}

#[test]
fn ammo_and_scope_take_0_26gs_data_types() {
    let template = t(vec![
        (
            "parameters",
            t(vec![
                ("item_level", V::Integer(5)),
                ("match_making_level", V::Integer(5)),
                ("drop_weight", V::Integer(160)),
                ("clip_size", V::Integer(30)),
                ("clip_weight", V::Float(0.25)),
            ]),
        ),
        ("data", t(vec![("type", V::Integer(10))])),
        (
            "ui_desc",
            t(vec![
                ("ammo_indicator_name", s("st_ammo_545x39_7n6m_name")),
                (
                    "text_descriptions",
                    t(vec![("name", s("x")), ("description", s("y"))]),
                ),
                ("icon", list(vec![V::Integer(6), V::Integer(2)])),
            ]),
        ),
    ]);
    let donor = t(vec![
        (
            "parameters",
            t(vec![
                ("clip_size", V::Integer(30)),
                ("clip_weight", V::Float(0.3)),
                ("item_level", V::Integer(0)),
            ]),
        ),
        ("data", t(vec![("type", V::Integer(11))])),
        (
            "ui_desc",
            t(vec![
                ("ammo_indicator_name", s("st_ammo_556x45_name")),
                (
                    "text_descriptions",
                    t(vec![
                        ("name", s("st_ammo_556x45_name")),
                        ("description", s("d")),
                    ]),
                ),
            ]),
        ),
    ]);
    let cfg = ammo_config(&AMMO_556, &donor, &template).unwrap();
    assert_eq!(
        get(&cfg, "data.type"),
        Some(&V::Integer(10)),
        "0.31's ammo type 11 is 0.26g's 10"
    );
    assert_eq!(get(&cfg, "parameters.item_level"), Some(&V::Integer(9)));
    assert_eq!(get(&cfg, "parameters.clip_weight"), Some(&V::Float(0.3)));
    assert_eq!(
        get(&cfg, "ui_desc.ammo_indicator_name"),
        Some(&s("st_ammo_556x45_name"))
    );
    assert_eq!(
        get(&cfg, "ui_desc.icon"),
        Some(&list(vec![V::Integer(16), V::Integer(3)]))
    );

    let scope_template = t(vec![(
        "data",
        t(vec![
            ("fov_factor", V::Float(0.25)),
            ("type", V::Integer(5)),
            ("idle_model", s("a")),
            ("aimed_model", s("b")),
        ]),
    )]);
    let scope_donor = t(vec![(
        "data",
        t(vec![
            ("type", V::Integer(6)),
            ("idle_model", s("weapons/attach/eotech_552/eotech_552")),
            ("aimed_model", s("weapons/attach/eotech_552/eotech_552")),
        ]),
    )]);
    let scope = scope_config(&scope_donor, &scope_template).unwrap();
    assert_eq!(get(&scope, "data.type"), Some(&V::Integer(5)));
    assert_eq!(get(&scope, "data.fov_factor"), Some(&V::Float(1.0)));
}

#[test]
fn materials_keep_only_0_26gs_effect_keys() {
    let value = |v: V| t(vec![("value", v), ("is_model_parameter", V::Boolean(true))]);
    let template = t(vec![(
        "material",
        t(vec![(
            "g_stage",
            t(vec![(
                "effect",
                t(vec![
                    ("texture_diffuse", value(s(""))),
                    ("use_nmap", value(V::Boolean(true))),
                ]),
            )]),
        )]),
    )]);
    let donor = t(vec![(
        "material",
        t(vec![(
            "g_stage",
            t(vec![(
                "effect",
                t(vec![
                    (
                        "texture_diffuse",
                        value(s("weapons/hk416_a5/hk416_receiver")),
                    ),
                    ("parallax_u_scale", value(V::Integer(1))),
                ]),
            )]),
        )]),
    )]);
    let out = conform_material(&donor, &template).unwrap();
    assert_eq!(
        get(&out, "material.g_stage.effect.texture_diffuse.value"),
        Some(&s("weapons/hk416_a5/hk416_receiver"))
    );
    assert!(get(&out, "material.g_stage.effect.parallax_u_scale").is_none());
    let lost = t(vec![(
        "material",
        t(vec![(
            "g_stage",
            t(vec![(
                "effect",
                t(vec![("texture_emissive_mask", value(s("x/y")))]),
            )]),
        )]),
    )]);
    assert!(
        conform_material(&lost, &template).is_err(),
        "a binding 0.26g cannot take is refused"
    );
}

fn row(id: i32, name: &str, cfg: &str, category: i32) -> V {
    t(vec![
        ("cfg_name", s(cfg)),
        ("is_stack", V::Boolean(category == 9)),
        ("name", s(name)),
        ("item_category", V::Integer(category)),
        ("dict_id", V::Integer(id)),
        ("base_cost", V::Integer(1)),
        ("is_premium", V::Boolean(false)),
    ])
}

#[test]
fn the_dictionary_gets_the_donors_ids_and_names_and_the_ammo_pair() {
    let mut dict = t(vec![
        (
            "items_dict",
            list(vec![
                row(363, "ak_12", RIFLE_TEMPLATE, 8),
                row(7, "ammo_5.45x39_7n6m", AMMO_TEMPLATE, 9),
                row(304, "scope_pso1", SCOPE_TEMPLATE, 22),
            ]),
        ),
        (
            "items_compatibility",
            t(vec![(
                "compatibility_93",
                t(vec![
                    ("second_item", V::Integer(7)),
                    ("first_item", V::Integer(3)),
                ]),
            )]),
        ),
    ]);
    let donor = t(vec![(
        "items_dict",
        list(vec![
            row(763, "hk416", HK416.config, 8),
            row(841, "ammo_5.56x45", AMMO_556.config, 9),
            row(819, "eotech_552", EOTECH.config, 22),
        ]),
    )]);
    let port = port("hk416").unwrap();
    assert_eq!(
        dictionary(&mut dict, &donor, port).unwrap(),
        ["compatibility_94"]
    );
    let V::Indexed(items) = dict.named("items_dict").unwrap() else {
        panic!()
    };
    let hk = items
        .iter()
        .find(|r| r.named("dict_id") == Some(&V::Integer(763)))
        .unwrap();
    assert_eq!(hk.named("name"), Some(&s("hk416")));
    assert_eq!(hk.named("base_cost"), Some(&V::Integer(45_000)));
    let ammo = items
        .iter()
        .find(|r| r.named("dict_id") == Some(&V::Integer(841)))
        .unwrap();
    assert_eq!(ammo.named("is_stack"), Some(&V::Boolean(true)));
    assert_eq!(ammo.named("base_cost"), Some(&V::Integer(70)));
    assert_eq!(
        get(&dict, "items_compatibility.compatibility_94.first_item"),
        Some(&V::Integer(763))
    );
    assert_eq!(
        get(&dict, "items_compatibility.compatibility_94.second_item"),
        Some(&V::Integer(841))
    );
    let sight = items
        .iter()
        .find(|r| r.named("dict_id") == Some(&V::Integer(819)))
        .unwrap();
    assert_eq!(sight.named("item_category"), Some(&V::Integer(22)));
    // Twice: the ids are taken.
    assert!(dictionary(&mut dict, &donor, port).is_err());
}

#[test]
fn ids_and_cells_are_free_and_distinct() {
    let ids = [
        HK416.dict_id,
        RADON.dict_id,
        AMMO_556.dict_id,
        EOTECH.dict_id,
    ];
    assert_eq!(ids, [763, 926, 841, 819], "0.31's own ids");
    for id in ids {
        // 0.26g + patches 12 and 14 end at 737, plus 1800-1810.
        assert!(id > 737 && id < 1800);
    }
    assert_ne!(HK416.icon, RADON.icon);
    // Weapon atlas column 3 is empty from row 7, the ammo atlas's from 16.
    assert!(HK416.icon[1] == 3 && HK416.icon[0] >= 7 && AMMO_556.icon == [16, 3]);
}

#[test]
fn a_donor_icon_is_centred_in_a_lobby_cell() {
    let mut src = vec![];
    {
        let mut e = png::Encoder::new(&mut src, 161, 68);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&vec![200; 161 * 68 * 4])
            .unwrap();
    }
    let out = cell_png(&src).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(out))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (171, 83));
    assert_eq!(buf[3], 0, "corner transparent");
    let centre = (41 * 171 + 85) * 4;
    assert_eq!(&buf[centre..centre + 4], &[200; 4]);
}

/// With the stock 0.26g database (`VOSTOK_026G_DB`) and the 0.31a2 donor
/// archive (`VOSTOK_031_DONOR_DB`): the H&K's clips and sounds all resolve,
/// its hands clips drive 0.26g's player rig, and its meshes become native.
#[test]
fn the_hk416_resolves_against_the_real_archives() {
    let (Some(stock), Some(donor)) = (
        std::env::var_os("VOSTOK_026G_DB"),
        std::env::var_os("VOSTOK_031_DONOR_DB"),
    ) else {
        eprintln!("skipped: set VOSTOK_026G_DB and VOSTOK_031_DONOR_DB");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(
        port("hk416").unwrap(),
        Path::new(&donor),
        &[PathBuf::from(stock.clone())],
        &dir.path().join("b"),
    )
    .unwrap();
    let mut stock = ResourceDb::open(stock).unwrap();
    let rig = BinaryConfig::parse(
        &stock
            .read("animations/single/weapons/ak_12/player/stand/3rd_view/on_site/hands_only/reload.bones_names")
            .unwrap(),
    )
    .unwrap();
    let overlay = dir.path().join("b/overlay");
    for clip in ["show", "hide", "idle", "reload", "reload_tactic", "shoot"] {
        let path = overlay.join(format!(
            "animations/single/weapons/hk416_a5/player/stand/3rd_view/on_site/hands_only/{clip}.bones_names"
        ));
        assert_eq!(
            BinaryConfig::parse(&fs::read(path).unwrap()).unwrap().root,
            rig.root,
            "{clip}"
        );
    }
    let mesh = fs::read(overlay.join(
        "models/weapons/hk416_a5/hk416_a5.skinned_model/render/hk416_receiver_s/converted_model",
    ))
    .unwrap();
    assert_eq!(
        strip_mesh_statistics(&mesh).unwrap(),
        mesh,
        "native chunks only"
    );
}
