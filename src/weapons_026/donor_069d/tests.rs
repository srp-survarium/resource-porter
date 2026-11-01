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
fn a_quarter_turn_about_y_takes_z_to_x() {
    let r = turn_about_y(std::f32::consts::FRAC_PI_2);
    let v = [0.0, 0.0, 1.0];
    let x: Vec<f32> = (0..3)
        .map(|i| (0..3).map(|k| r[i][k] * v[k]).sum())
        .collect();
    assert!((x[0] - 1.0).abs() < 1e-6 && x[2].abs() < 1e-6, "{x:?}");
    let id = multiply3(&r, &turn_about_y(-std::f32::consts::FRAC_PI_2));
    assert!((id[0][0] - 1.0).abs() < 1e-6 && id[0][2].abs() < 1e-6);
}

#[test]
fn the_sidecar_parses() {
    let spec: Spec069 = serde_json::from_str(
        r#"{"format_version": 1,
            "guns": {"akmsu": {"rig": {"live": "models/weapons/ak74u.skinned_model"},
                               "separable": ["akmsu_m24x15_muzzlebrake"]},
                     "ppsh_drum": {"rig": "donor", "module_turns": {"ppsh_762x25_71_round_mag": 90.0}}},
            "rounds": []}"#,
    )
    .unwrap();
    assert!(matches!(spec.guns["akmsu"].rig, Rig::Live(_)));
    assert_eq!(
        spec.guns["ppsh_drum"].module_turns["ppsh_762x25_71_round_mag"],
        90.0
    );
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
    for r in &spec.rounds {
        assert!((2253..=2259).contains(&r.dict_id), "{}", r.name);
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
