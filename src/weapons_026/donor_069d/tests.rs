// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

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
}

#[test]
fn only_069d_clips_and_suppressed_sounds_leave_the_item_config() {
    let clip = |c: &str| s(format!("resources/animations/single/weapons/{c}"));
    let cfg = V::Named(vec![field(
        "states",
        V::Named(vec![
            field(
                "fire",
                V::Named(vec![
                    field(
                        "animations",
                        V::Indexed(vec![
                            clip("mauser/item/stand/3rd_view/shoot"),
                            clip("mauser/item/stand/3rd_view/shoot_calim"),
                        ]),
                    ),
                    field(
                        "sounds",
                        V::Named(vec![field(
                            "first_view",
                            V::Indexed(vec![
                                V::Indexed(vec![V::Integer(257), s("weapons/ppsh")]),
                                V::Indexed(vec![V::Integer(257), s("weapons/akm_n_silencer")]),
                            ]),
                        )]),
                    ),
                ]),
            ),
            field(
                "reload",
                V::Named(vec![field(
                    "animations",
                    V::Indexed(vec![
                        clip("ks_23/item/stand/3rd_view/reload_start"),
                        clip("ks_23/item/stand/3rd_view/reload_finalround"),
                    ]),
                )]),
            ),
        ]),
    )]);
    let out = without_069d_only(&cfg);
    assert_eq!(list(get(&out, "states.fire.animations")).len(), 1);
    assert_eq!(list(get(&out, "states.fire.sounds.first_view")).len(), 1);
    assert_eq!(
        list(get(&out, "states.reload.animations")),
        &[clip("ks_23/item/stand/3rd_view/reload_start")]
    );
}

#[test]
fn a_socket_turned_about_x_turns_its_module_the_other_way_in_mesh_space() {
    let loc = Locator {
        name: "mag_point".into(),
        bone: "magazin".into(),
        bone_idx: 3,
        position: [0.0, -0.02, -0.02],
        rotation: [-0.0652655, 0.0, 0.0],
    };
    let r = socket_rotation(&loc).unwrap();
    assert!((r[1][2] - (-(0.0652655f32).sin())).abs() < 1e-6, "{r:?}");
    let turned = Locator {
        rotation: [0.0, 0.1, 0.0],
        ..loc.clone()
    };
    assert!(socket_rotation(&turned).is_err());
    // The socket: the bone's bind translation plus the locator, x and z
    // negated (mesh space is half a turn about y from the bind frame).
    let at = socket(&loc, &[("magazin".into(), [0.0, -0.03, 0.25])]).unwrap();
    assert!(
        (at[1] + 0.05).abs() < 1e-6 && (at[2] - 0.27).abs() < 1e-6,
        "{at:?}"
    );
}

#[test]
fn the_sidecar_parses() {
    let spec: Spec069 = serde_json::from_str(
        r#"{"format_version": 1,
            "guns": {"akmsu": {"rig": {"live": "models/weapons/ak74u.skinned_model"},
                               "separable": ["akmsu_m24x15_muzzlebrake"]},
                     "ppsh_drum": {"rig": "donor"}}}"#,
    )
    .unwrap();
    assert!(matches!(spec.guns["akmsu"].rig, Rig::Live(_)));
    assert!(matches!(spec.guns["ppsh_drum"].rig, Rig::Donor));
}

/// The port the repository ships parses, and its guns' donor items are the
/// sidecar's.
#[test]
fn the_069d_port_and_its_sidecar_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ports");
    let port = super::super::manifest::WeaponPort::load(&root.join("weapons-069d.json")).unwrap();
    let spec = Spec069::load(&root.join("donor-weapons-069d.json")).unwrap();
    assert_eq!(port.donor_build, DonorBuild::V069d0);
    for w in &port.weapons {
        assert!(spec.guns.contains_key(&w.donor_item), "{}", w.name);
        // The batch's claimed ids (release 41): guns 2290..=2299.
        assert!((2290..=2299).contains(&w.dict_id), "{}", w.name);
    }
    for a in &port.ammo_items {
        assert!((2253..=2259).contains(&a.dict_id), "{}", a.name);
    }
}

/// A clip with the engine's words cleared and the bytes after each event
/// name's NUL, to four, zeroed: the exporter left whatever its memory held
/// there.
fn without_padding(b: &[u8]) -> Vec<u8> {
    let mut out = clips::engine_words_cleared(b).unwrap();
    for name in clips::event_names(b).unwrap() {
        let needle = [name.as_bytes(), &[0]].concat();
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
/// `extract-bundle` and `pack-donor-tree`): the AK-12's clips both builds
/// ship, converted by the core, are 0.26g's own bytes.
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
        let converted = clips::events_12_to_16(&donor.read(&path).unwrap()).unwrap();
        assert_eq!(
            without_padding(&converted),
            without_padding(&native),
            "{clip}"
        );
    }
}

#[test]
fn a_rename_moves_a_file_and_every_name_of_it() {
    use super::renames::Renames;
    use crate::binary_config::{ConfigValue as V, NamedValue};
    let r: Renames = serde_json::from_str(
        r#"{"clips": {"animations/single/weapons/ppsh/": "animations/single/weapons/ppsh_drum/"},
            "sounds": {"weapons/akm_n": "weapons/akm_n_069"}}"#,
    )
    .unwrap();
    assert_eq!(
        r.donor_path("animations/single/weapons/ppsh_drum/item/idle.b-spline")
            .as_deref(),
        Some("animations/single/weapons/ppsh/item/idle.b-spline")
    );
    assert_eq!(
        r.donor_path("sounds/composite/weapons/akm_n_069_m.composite_sound")
            .as_deref(),
        Some("sounds/composite/weapons/akm_n_m.composite_sound")
    );
    assert_eq!(
        r.donor_path("sounds/single/weapons/akm_n_069/akm_n_reload_1.high.ogg")
            .as_deref(),
        Some("sounds/single/weapons/akm_n/akm_n_reload_1.high.ogg")
    );
    assert_eq!(r.donor_path("sounds/single/weapons/akm_nx.high.ogg"), None);
    assert_eq!(
        r.donor_path("sounds/composite/weapons/akm_n_069.composite_sound")
            .as_deref(),
        Some("sounds/composite/weapons/akm_n.composite_sound")
    );
    // The donor has everything but the template's clip.
    let has = |p: &str| !p.contains("template");
    let s = |t: &str| V::String(t.into());
    let mut cfg = V::Named(vec![
        NamedValue {
            name: "animations".into(),
            value: V::Indexed(vec![
                s("resources/animations/single/weapons/ppsh/item/idle"),
                s("resources/animations/single/weapons/ppsh/template"),
            ]),
        },
        NamedValue {
            name: "sounds".into(),
            value: V::Indexed(vec![V::Indexed(vec![
                V::Integer(257),
                s("weapons/akm_n_m"),
            ])]),
        },
        NamedValue {
            name: "skeleton".into(),
            value: s("weapons/akm_n/akm_n"),
        },
        NamedValue {
            name: "item".into(),
            value: V::Named(vec![NamedValue {
                name: "filename".into(),
                value: s("weapons/akm_n/layers/tail"),
            }]),
        },
    ]);
    r.apply(&mut cfg, &has);
    let V::Named(f) = &cfg else { unreachable!() };
    assert_eq!(
        f[0].value,
        V::Indexed(vec![
            s("resources/animations/single/weapons/ppsh_drum/item/idle"),
            s("resources/animations/single/weapons/ppsh/template"),
        ])
    );
    assert_eq!(
        f[1].value,
        V::Indexed(vec![V::Indexed(vec![
            V::Integer(257),
            s("weapons/akm_n_069_m")
        ])])
    );
    // A string that names no sound stays.
    assert_eq!(f[2].value, s("weapons/akm_n/akm_n"));
    assert_eq!(
        f[3].value,
        V::Named(vec![NamedValue {
            name: "filename".into(),
            value: s("weapons/akm_n_069/layers/tail")
        }])
    );
}

#[test]
fn a_repeated_suppressed_half_of_a_sound_list_goes() {
    use crate::binary_config::ConfigValue as V;
    let pair = |k: i32, n: &str| V::Indexed(vec![V::Integer(k), V::String(n.into())]);
    let twice = V::Indexed(vec![
        pair(257, "weapons/rem_870"),
        pair(258, "weapons/remington_870/remington_870_reload_in"),
        pair(257, "weapons/rem_870"),
        pair(258, "weapons/remington_870/remington_870_reload_in"),
    ]);
    assert_eq!(
        super::without_069d_only(&twice),
        V::Indexed(vec![
            pair(257, "weapons/rem_870"),
            pair(258, "weapons/remington_870/remington_870_reload_in"),
        ])
    );
    // A list whose halves differ stays whole.
    let other = V::Indexed(vec![pair(257, "weapons/a"), pair(257, "weapons/b")]);
    assert_eq!(super::without_069d_only(&other), other);
}

#[test]
fn a_diagonal_lobby_icon_comes_level_with_its_muzzle_right() {
    // A 64x64 icon: a bar from (8, 8) (the muzzle, up left) to (55, 55),
    // its muzzle end red.
    let (w, h) = (64usize, 64usize);
    let mut px = vec![0u8; w * h * 4];
    for i in 8..56 {
        for t in 0..3 {
            let (x, y) = (i + t, i);
            let at = (y * w + x) * 4;
            px[at..at + 4].copy_from_slice(&if i < 16 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            });
        }
    }
    let mut png_bytes = vec![];
    {
        let mut e = png::Encoder::new(&mut png_bytes, w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&px).unwrap();
    }
    let out = super::convert::level_lobby_icon(&png_bytes).unwrap();
    let mut d = png::Decoder::new(std::io::Cursor::new(&out[..]));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    let (ow, oh) = (info.width as usize, info.height as usize);
    assert!(ow > 4 * oh, "level: {ow}x{oh}");
    // The red end is on the right.
    let red_x: Vec<usize> = (0..ow * oh)
        .filter(|&i| buf[i * 4 + 3] > 128 && buf[i * 4] > 200 && buf[i * 4 + 2] < 60)
        .map(|i| i % ow)
        .collect();
    assert!(
        !red_x.is_empty() && red_x.iter().all(|&x| x > ow / 2),
        "{red_x:?}"
    );
}
