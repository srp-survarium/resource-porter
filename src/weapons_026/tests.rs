// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::{Path, PathBuf};

use super::clips::{EVENT_HEADER_026, clip_header, engine_words_cleared, events_12_to_16};
use super::config::{conform_schema, schema, weapon_config};
use super::donor::{RULES_029A3, RULES_031A2};
use super::manifest::{KillFeed, Weapon, WeaponPort};
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
    // A float stays a float where the template has an integer (1.98 recoil).
    assert_eq!(conform(&V::Float(1.98), &V::Integer(2)), V::Float(1.98));
}

#[test]
fn a_key_some_native_weapon_reads_is_kept_though_the_template_lacks_it() {
    let template = t(vec![("parameters", t(vec![("weight", V::Float(3.0))]))]);
    let other = t(vec![("parameters", t(vec![("aim_offset", V::Float(0.1))]))]);
    let schema = schema([&template, &other]);
    let donor = t(vec![(
        "parameters",
        t(vec![
            ("weight", V::Float(2.5)),
            ("aim_offset", V::Float(0.06)),
            ("faction_data", V::Integer(2)),
        ]),
    )]);
    assert_eq!(
        conform_schema(&donor, &template, &schema),
        t(vec![(
            "parameters",
            t(vec![
                ("weight", V::Float(2.5)),
                ("aim_offset", V::Float(0.06))
            ])
        )])
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
}

/// A clip of one bone (nine curves of one pair each) and `events` event
/// channels in the later builds' 12-byte layout.
fn clip_12(events: usize) -> Vec<u8> {
    let bones = 1;
    let mut b = vec![bones as u8, 0, events as u8, 1];
    let base = 4 + bones * 0x48;
    let data = base + events * 12;
    for slot in 0..9 {
        let at = 4 + slot * 8;
        let target = data + slot * 12;
        b.extend(((target - at) as u32).to_le_bytes());
        b.extend(0u32.to_le_bytes());
    }
    let events_at = data + 9 * 12;
    let mut curves = vec![];
    for _ in 0..9 {
        curves.extend(1u32.to_le_bytes());
        curves.extend(0.0f32.to_le_bytes());
        curves.extend(1.0f32.to_le_bytes());
    }
    let mut channel_at = vec![];
    let mut channels = vec![];
    for e in 0..events {
        channel_at.push(events_at + channels.len());
        channels.extend(0.5f32.to_le_bytes());
        channels.push(e as u8);
        channels.extend(format!("ch{e}\0").as_bytes());
        while channels.len() % 4 != 0 {
            channels.push(0);
        }
    }
    for (e, at) in channel_at.iter().enumerate() {
        let header = base + e * 12;
        b.extend(((at - header) as u32).to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend([1u8, 0, 1, 2]);
    }
    b.extend(curves);
    b.extend(channels);
    b
}

#[test]
fn clip_event_headers_grow_to_16_bytes_and_every_slot_keeps_its_data() {
    for events in [0, 1, 3] {
        let later = clip_12(events);
        let ours = events_12_to_16(&later).unwrap();
        let h = clip_header(&ours).unwrap();
        assert_eq!(ours.len(), later.len() + 4 * events, "{events}");
        let names = clips::event_names(&ours).unwrap();
        assert_eq!(names.len(), h.events);
        for (e, name) in names.iter().enumerate() {
            assert_eq!(name, &format!("ch{e}"));
        }
        let base = 4 + 0x48;
        for e in 0..events {
            let at = base + e * EVENT_HEADER_026;
            assert_eq!(&ours[at + 8..at + 12], &[1, 0, 1, 2]);
            assert_eq!(&ours[at + 12..at + 16], &[0; 4]);
        }
    }
}

#[test]
fn two_clips_that_differ_only_in_engine_words_compare_equal() {
    let a = events_12_to_16(&clip_12(2)).unwrap();
    let mut b = a.clone();
    // Patch 19's clips keep the 12-byte offsets unshifted.
    b[4] ^= 0x08;
    b[4 + 0x48 + 4] = 0x35;
    assert_eq!(
        engine_words_cleared(&a).unwrap(),
        engine_words_cleared(&b).unwrap()
    );
    let mut c = a.clone();
    let last = c.len() - 8;
    c[last] ^= 1;
    assert_ne!(
        engine_words_cleared(&a).unwrap(),
        engine_words_cleared(&c).unwrap()
    );
}

#[test]
fn state_types_map_by_build() {
    assert_eq!(RULES_031A2.state_type(313).unwrap(), 314);
    assert_eq!(RULES_031A2.state_type(315).unwrap(), 316);
    assert_eq!(RULES_031A2.state_type(320).unwrap(), 321);
    assert_eq!(RULES_031A2.state_type(305).unwrap(), 305);
    assert_eq!(RULES_029A3.state_type(314).unwrap(), 314);
    assert!(RULES_031A2.state_type(399).is_err());
}

#[test]
fn reverb_composites_become_outdoor_collections() {
    assert_eq!(
        environment_layer("weapons/environments/weapons_1st_view").as_deref(),
        Some("weapons/environments/level_01/automatic/outdoor/rudnya_outdoor")
    );
    assert_eq!(
        environment_layer("weapons/environments/weapons_3rd_view_one_shot").as_deref(),
        Some("weapons/environments/level_01/automatic/outdoor/rudnya_outdoor_one_shot_distant")
    );
    assert_eq!(environment_layer("weapons/environments/indoor_x"), None);
}

#[test]
fn globs() {
    use super::compare::glob;
    assert!(glob("a/*/c", "a/b/c"));
    assert!(glob("*.material", "x/y.material"));
    assert!(glob("wpn_*.options", "wpn_ak.options"));
    assert!(!glob("wpn_*.options", "wpn_ak.material"));
    assert!(glob("*", "anything"));
}

fn rifle_state(kind: i32, clip: &str) -> V {
    t(vec![
        ("state_type", V::Integer(kind)),
        (
            "animations",
            list(vec![s(format!(
                "resources/animations/single/weapons/{clip}"
            ))]),
        ),
    ])
}

fn template() -> V {
    t(vec![
        (
            "parameters",
            t(vec![
                ("bullet_damage", V::Float(0.5)),
                ("bullet_pierce", V::Float(0.3)),
                ("stopping_power", V::Float(0.2)),
                ("aim_time", V::Float(0.3)),
                ("base_dispersion", V::Float(0.2)),
                ("min_damage_distance", V::Integer(50)),
                ("rounds_per_minute", V::Integer(650)),
                ("weight", V::Float(3.4)),
                ("magazine_capacity", V::Integer(30)),
                ("stamina_damage", V::Integer(20)),
                ("pain_health_modifier", V::Integer(0)),
                ("degrade_per_match", V::Float(0.01)),
                ("degrade_per_use", V::Float(0.0005)),
                ("repair_cost_ratio", V::Float(0.1)),
                ("available_modifications_mask", s("111")),
                ("min_damage_amount", V::Float(0.6)),
                ("aim_offset", V::Float(0.028)),
            ]),
        ),
        (
            "states",
            t(vec![
                ("reload", rifle_state(303, "ak/item/stand/3rd_view/reload")),
                ("melee", rifle_state(321, "ak/player/kick")),
            ]),
        ),
        ("melee", t(vec![("k_damage", V::Float(1.5))])),
        ("particles", t(vec![("shoot_pfx_count", V::Integer(3))])),
        (
            "ui_desc",
            t(vec![
                ("icon", list(vec![V::Integer(1), V::Integer(1)])),
                ("combat_log_icon", V::Integer(49)),
                ("props_list", list(vec![])),
            ]),
        ),
    ])
}

fn donor() -> V {
    let mut reload = rifle_state(313, "ots14/item/stand/3rd_view/reload_tactic");
    reload
        .set_path("emotion_animations", list(vec![s("x")]))
        .unwrap();
    t(vec![
        (
            "parameters",
            t(vec![
                ("bullet_damage", V::Float(0.35)),
                ("bullet_pierce", V::Float(0.45)),
                ("stopping_power", V::Float(0.3)),
                ("aim_time", V::Float(0.2)),
                ("base_dispersion", V::Float(0.2)),
                ("min_damage_distance", V::Integer(65)),
                ("rounds_per_minute", V::Integer(700)),
                ("weight", V::Float(2.75)),
                ("magazine_capacity", V::Integer(20)),
                ("stamina_damage", V::Float(0.3)),
                ("health_modifier", V::Integer(4)),
                ("degrade_per_match", V::Float(0.03)),
                ("faction_data", t(vec![("faction_id", V::Integer(2))])),
            ]),
        ),
        (
            "states",
            t(vec![
                ("reload", reload),
                (
                    "melee",
                    rifle_state(320, "base/player/1st_view/melee/kick_grip1"),
                ),
                ("emote", rifle_state(330, "ots14/emote")),
            ]),
        ),
        ("melee", t(vec![("k_damage", V::Integer(5))])),
        ("compatible_ammo", list(vec![V::Integer(300)])),
        (
            "ui_desc",
            t(vec![
                ("icon", list(vec![V::Integer(6), V::Integer(5)])),
                ("combat_log_icon", V::Integer(72)),
                ("props_list", list(vec![])),
            ]),
        ),
    ])
}

fn weapon() -> Weapon {
    serde_json::from_value(serde_json::json!({
        "name": "ots14", "dict_id": 2261,
        "donor_item": "gameplay/items/weapons/assault/ots14.options",
        "config": "gameplay/items/weapons/wpn_ots14.options",
        "template": "gameplay/items/weapons/wpn_val.options",
        "base_cost": 50000, "ammo": [300],
        "set": {"parameters.bullet_damage": 0.5, "parameters.weight": 2.7},
        "lobby_icon": {"row": 6, "col": 5, "donor_png": "x.png"},
        "kill_feed": {"icon": 72, "donor_png": null}
    }))
    .unwrap()
}

#[test]
fn a_gun_takes_the_donors_states_on_026gs_types_and_keys() {
    let template = template();
    let schema = schema([&template]);
    let (cfg, notes) = weapon_config(
        &RULES_031A2,
        &weapon(),
        &donor(),
        &template,
        &schema,
        &|_| false,
    )
    .unwrap();
    // The donor's reload, on 0.26g's type, less the keys no native state has.
    assert_eq!(
        get(&cfg, "states.reload.state_type"),
        Some(&V::Integer(314))
    );
    assert_eq!(
        string_of(get(&cfg, "states.reload.animations.0")),
        Some("resources/animations/single/weapons/ots14/item/stand/3rd_view/reload_tactic")
    );
    assert!(get(&cfg, "states.reload.emotion_animations").is_none());
    // The melee plays weapons/base: the template's, and 0.26g's melee values.
    assert_eq!(get(&cfg, "states.melee.state_type"), Some(&V::Integer(321)));
    assert_eq!(get(&cfg, "melee.k_damage"), Some(&V::Float(1.5)));
    assert!(get(&cfg, "states.emote").is_none());
    assert!(notes.iter().any(|n| n.contains("states.emote")));
    // Renamed and rescaled keys; the native systems' values; the balance.
    assert_eq!(
        get(&cfg, "parameters.pain_health_modifier"),
        Some(&V::Integer(4))
    );
    assert_eq!(
        get(&cfg, "parameters.stamina_damage"),
        Some(&V::Integer(30))
    );
    assert_eq!(
        get(&cfg, "parameters.degrade_per_match"),
        Some(&V::Float(0.01))
    );
    assert!(get(&cfg, "parameters.faction_data").is_none());
    // The template's aim offset fits its model, not the donor's.
    assert!(get(&cfg, "parameters.aim_offset").is_none());
    assert!(get(&cfg, "compatible_ammo").is_none());
    assert_eq!(get(&cfg, "parameters.bullet_damage"), Some(&V::Float(0.5)));
    // Icons and the card from the final values.
    assert_eq!(get(&cfg, "ui_desc.icon.0"), Some(&V::Integer(6)));
    assert_eq!(get(&cfg, "ui_desc.combat_log_icon"), Some(&V::Integer(72)));
    let Some(V::Indexed(rows)) = get(&cfg, "ui_desc.props_list") else {
        panic!("no card");
    };
    let card: Vec<(i32, String)> = rows
        .iter()
        .map(|r| match (r.named("prop_id"), r.named("prop_value")) {
            (Some(V::Integer(i)), Some(V::String(v))) => (*i, v.clone()),
            _ => panic!("card row {r:?}"),
        })
        .collect();
    assert_eq!(card[0], (1, "50".into()));
    assert_eq!(card[1], (3, "45".into()));
    // The magazine row the live cards have (134, after the rate of fire).
    assert_eq!(card[6], (4, "700".into()));
    assert_eq!(card[7], (134, "20".into()));
    assert_eq!(card[8], (15, "2.7".into()));
}

#[test]
fn a_copy_of_a_native_weapon_keeps_the_native_states() {
    let template = template();
    let schema = schema([&template]);
    let mut w = weapon();
    w.copy_of_template = true;
    let (cfg, _) =
        weapon_config(&RULES_031A2, &w, &donor(), &template, &schema, &|_| false).unwrap();
    assert_eq!(get(&cfg, "states"), get(&template, "states"));
}

#[test]
fn a_manifest_refuses_twice_used_ids_and_cells() {
    let mut port = WeaponPort {
        format_version: 1,
        id: "x".into(),
        donor_build: donor::DonorBuild::V031a2,
        weapons: vec![weapon()],
        modules: vec![],
        external_ammo: vec![],
        ammo_items: vec![],
    };
    port.check().unwrap();
    let mut second = weapon();
    second.name = "other".into();
    second.config = "gameplay/items/weapons/wpn_other.options".into();
    port.weapons.push(second.clone());
    assert!(port.check().unwrap_err().to_string().contains("2261"));
    second.dict_id = 2262;
    port.weapons[1] = second.clone();
    assert!(port.check().unwrap_err().to_string().contains("lobby cell"));
    second.lobby_icon = None;
    second.kill_feed = KillFeed {
        icon: 71,
        donor_png: None,
    };
    port.weapons[1] = second;
    port.check().unwrap();
}

#[test]
fn every_shipped_port_manifest_parses() {
    let ports = Path::new(env!("CARGO_MANIFEST_DIR")).join("ports");
    let mut seen = 0;
    for entry in std::fs::read_dir(ports).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("weapons-") && name.ends_with(".json") {
            WeaponPort::load(&path).unwrap();
            seen += 1;
        }
    }
    assert!(seen >= 1);
}

/// Patch 19's 0.29a3 weapons, ported again by the tool from the same donor
/// and compared with patch 19's files: every difference explained
/// (`ports/weapons-029a3-patch19.known.tsv`). Needs the donor
/// (`VOSTOK_029A3_DONOR`: its `.split` list), the stock database
/// (`VOSTOK_026G_DB`) and the patches (`VOSTOK_026G_PATCHES`: the folder of
/// `resources.db.patch_<n>`); skipped without them.
#[test]
fn the_tool_gives_patch_19s_weapons() {
    let (Some(donor), Some(stock), Some(patches)) = (
        std::env::var_os("VOSTOK_029A3_DONOR"),
        std::env::var_os("VOSTOK_026G_DB"),
        std::env::var_os("VOSTOK_026G_PATCHES"),
    ) else {
        eprintln!("skipped: set VOSTOK_029A3_DONOR, VOSTOK_026G_DB and VOSTOK_026G_PATCHES");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ports");
    let port = WeaponPort::load(&root.join("weapons-029a3-patch19.json")).unwrap();
    let patches = PathBuf::from(patches);
    let mut layers = vec![PathBuf::from(stock)];
    for n in [12, 14, 17] {
        layers.push(patches.join(format!("resources.db.patch_{n}")));
    }
    let mut donor = donor::ArchiveDonor::open(Path::new(&donor), port.donor_build).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("bundle");
    build(&port, &mut donor, &layers, &out).unwrap();
    let known = compare::load_known(&root.join("weapons-029a3-patch19.known.tsv")).unwrap();
    let mut reference = crate::ResourceDb::open(patches.join("resources.db.patch_19")).unwrap();
    let report = compare::compare(&out.join("overlay"), &mut reference, &known).unwrap();
    assert_eq!(report.unexplained().count(), 0, "{}", report.summary());
}

// --- Modules -------------------------------------------------------------

/// A 0.26g mesh of `quads` (each two triangles), 36-byte vertices.
fn mesh_bytes(quads: &[[[f32; 3]; 4]]) -> Vec<u8> {
    let mut vertices = ((quads.len() * 4) as u32).to_le_bytes().to_vec();
    let mut indices = ((quads.len() * 6) as u32).to_le_bytes().to_vec();
    for (q, quad) in quads.iter().enumerate() {
        for p in quad {
            for c in p {
                vertices.extend(c.to_le_bytes());
            }
            vertices.extend([0u8; 24]);
        }
        let b = (q * 4) as u16;
        for i in [b, b + 1, b + 2, b, b + 2, b + 3] {
            indices.extend(i.to_le_bytes());
        }
    }
    [chunk(3, &vertices), chunk(4, &indices)].concat()
}

/// A quad across the barrel (x, y) at length `z`, `r` wide.
fn ring(z: f32, r: f32) -> [[f32; 3]; 4] {
    [[-r, -r, z], [r, -r, z], [r, r, z], [-r, r, z]]
}

fn module(kind: &str, name: &str, donor_model: &str) -> manifest::Module {
    serde_json::from_value(serde_json::json!({
        "name": name, "kind": kind, "dict_id": 2900,
        "donor_model": donor_model, "model": format!("weapons/attach/{name}"),
        "config": format!("gameplay/items/scopes/{name}.options"),
        "template": "gameplay/items/scopes/scope_pso1.options", "base_cost": 9000,
        "locator": null, "set": {}
    }))
    .unwrap()
}

#[test]
fn a_built_in_brake_is_cut_and_a_device_placed_at_the_muzzle() {
    use super::modules::{Mesh, cut_builtin, placed, placement};
    // Our gun: a barrel (base) and a brake past it (z 0.50..0.58), as a
    // 0.31a2 mesh with the device built in.
    let barrel = [ring(0.0, 0.01), ring(0.25, 0.01), ring(0.5, 0.01)];
    let brake = [ring(0.52, 0.015), ring(0.56, 0.015)];
    let ours = Mesh::parse(&mesh_bytes(&[barrel.as_slice(), brake.as_slice()].concat())).unwrap();
    let base = Mesh::parse(&mesh_bytes(&barrel)).unwrap();
    // 0.69d0's stock brake module, origin at its rear, placed at muzzle_point.
    let device = Mesh::parse(&mesh_bytes(&[ring(0.02, 0.015), ring(0.06, 0.015)])).unwrap();
    let at = placement([0.0, 0.0, 0.5], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    let (kept, cut) = cut_builtin(&ours, &base.positions, &placed(&device, at));
    assert_eq!(cut, 4, "the brake's two quads");
    assert_eq!(kept.len(), 6, "the barrel stays");
    // A device elsewhere cuts nothing.
    let (kept, cut) = cut_builtin(&ours, &base.positions, &placed(&device, [0.0, 0.3, 0.0]));
    assert_eq!((kept.len(), cut), (10, 0));
    // Written back: the same vertices, the kept triangles.
    let bare = Mesh::parse(&ours.with_triangles(&kept)).unwrap();
    assert_eq!(bare.positions, ours.positions);
    assert_eq!(bare.triangles.len(), 10);
}

#[test]
fn a_render_shows_the_gun_and_its_module() {
    use super::modules::{Mesh, png, render_side};
    let gun = Mesh::parse(&mesh_bytes(&[[
        [0.0, -0.02, 0.0],
        [0.0, -0.02, 0.5],
        [0.0, 0.02, 0.5],
        [0.0, 0.02, 0.0],
    ]]))
    .unwrap();
    let (w, h, rgba) =
        render_side(&[(&gun.positions, &gun.triangles, [200, 200, 200])], 200).unwrap();
    assert_eq!(w, 200);
    let lit = rgba.chunks(4).filter(|p| p[3] == 255).count();
    assert!(lit > w * h / 2, "{lit} of {}", w * h);
    assert!(png(w, h, &rgba).unwrap().starts_with(b"\x89PNG"));
}

fn scope_template() -> V {
    t(vec![
        ("parameters", t(vec![("type", s("scope"))])),
        (
            "data",
            t(vec![
                ("idle_model", s("weapons/attach/pso1")),
                ("aimed_model", s("weapons/attach/pso1_aim")),
                ("fov_factor", V::Float(0.6)),
            ]),
        ),
    ])
}

#[test]
fn one_module_of_each_kind_gets_its_item_config() {
    use super::modules::module_config;
    // A scope: its views.
    let mut scope = module("scope", "srp_scope_x", "weapons/modules/scopes/x");
    scope
        .set
        .insert("data.fov_factor".into(), serde_json::json!(0.5));
    let cfg = module_config(&scope, &scope_template()).unwrap();
    assert_eq!(
        string_of(get(&cfg, "data.idle_model")),
        Some("weapons/attach/srp_scope_x")
    );
    assert_eq!(get(&cfg, "data.fov_factor"), Some(&V::Float(0.5)));
    // A muzzle brake: the kit's keys, at muzzle_point.
    let brake = module(
        "muzzle_brake",
        "srp_brake_x",
        "weapons/modules/muzzle_brakes/x_muzzlebrake",
    );
    let cfg = module_config(&brake, &scope_template()).unwrap();
    assert_eq!(
        string_of(get(&cfg, "data.srp_muzzle_model")),
        Some("weapons/attach/srp_brake_x")
    );
    assert_eq!(
        string_of(get(&cfg, "data.srp_locator")),
        Some("muzzle_point")
    );
    assert_eq!(string_of(get(&cfg, "parameters.type")), Some("srp_muzl"));
    // A grip.
    let grip = module("grip", "srp_grip_x", "weapons/modules/foregrips/x");
    let cfg = module_config(&grip, &scope_template()).unwrap();
    assert_eq!(
        string_of(get(&cfg, "data.srp_grip_model")),
        Some("weapons/attach/srp_grip_x")
    );
    assert_eq!(
        string_of(get(&cfg, "data.srp_locator")),
        Some("picatinny_grip_point")
    );
    // No suppressors.
    let can = module("muzzle_brake", "srp_can", "weapons/modules/silencers/pbs_1");
    assert!(module_config(&can, &scope_template()).is_err());
}

// --- Resources a config names ------------------------------------------

#[test]
fn a_config_naming_a_missing_resource_is_refused() {
    use super::resources::{check_resources, resource_files};
    let cfg = t(vec![
        (
            "object",
            t(vec![
                ("model", s("weapons/g/g")),
                ("skeleton", s("weapons/g/g")),
            ]),
        ),
        (
            "particles",
            t(vec![
                ("shoot", s("fx/weapons/m4a1")),
                ("bullet_shells", s("shells/5_45x040")),
            ]),
        ),
        (
            "states",
            t(vec![(
                "fire",
                t(vec![
                    (
                        "animations",
                        list(vec![s("resources/animations/single/weapons/g/shoot")]),
                    ),
                    (
                        "sounds",
                        t(vec![(
                            "first_view",
                            list(vec![list(vec![V::Integer(257), s("weapons/g")])]),
                        )]),
                    ),
                ]),
            )]),
        ),
        (
            "user_animations",
            s("gameplay/items/weapons/common/rifles.user_animations"),
        ),
        (
            "ui_desc",
            t(vec![(
                "text_descriptions",
                t(vec![("name", s("st_g_name"))]),
            )]),
        ),
    ]);
    let files: Vec<String> = resource_files(&cfg)
        .unwrap()
        .into_iter()
        .map(|(_, f)| f)
        .collect();
    assert_eq!(
        files,
        [
            "models/weapons/g/g.skinned_model/settings",
            "animations/skeletons/weapons/g/g.skeleton",
            "particles/fx/weapons/m4a1.particle",
            "animations/single/weapons/g/shoot.b-spline",
            "sounds/composite/weapons/g.composite_sound",
            "gameplay/items/weapons/common/rifles.user_animations",
        ]
    );
    let all_but_fx = |f: &str| f != "particles/fx/weapons/m4a1.particle";
    let no_live = |_: &str, _: &str| false;
    let error = check_resources("wpn_g", &cfg, &all_but_fx, &no_live)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("particles.shoot: particles/fx/weapons/m4a1.particle"),
        "{error}"
    );
    check_resources("wpn_g", &cfg, &|_| true, &no_live).unwrap();
    // A missing name the live template has at the same key is the game's
    // own (the stock pistols' death clips): kept; at another key it is not.
    let live =
        |k: &str, f: &str| k == "particles.shoot" && f == "particles/fx/weapons/m4a1.particle";
    check_resources("wpn_g", &cfg, &all_but_fx, &live).unwrap();
    let elsewhere =
        |k: &str, f: &str| k == "object.model" && f == "particles/fx/weapons/m4a1.particle";
    assert!(check_resources("wpn_g", &cfg, &all_but_fx, &elsewhere).is_err());
    // A path under a key of no known kind is refused, not let through.
    let odd = t(vec![("data", t(vec![("strange", s("some/where"))]))]);
    assert!(resource_files(&odd).is_err());
}

/// The lobby's `preview_item` is the gun's own idle, not the template's
/// gun's (the PKP's lobby idle on the Fort-401 crashed the lobby).
#[test]
fn preview_item_is_the_guns_own_idle() {
    let mut cfg = t(vec![
        (
            "states",
            t(vec![(
                "idle",
                t(vec![(
                    "animations",
                    list(vec![
                        s("resources/animations/single/weapons/g/item/stand/3rd_view/idle"),
                        s("resources/animations/single/weapons/g/item/stand/3rd_view/idle_empty"),
                    ]),
                )]),
            )]),
        ),
        (
            "user_animations_in_place",
            t(vec![(
                "preview_item",
                s("resources/animations/single/weapons/pkp/item/stand/3rd_view/idle"),
            )]),
        ),
    ]);
    let mut notes = vec![];
    super::config::own_preview_item(&mut cfg, &mut notes).unwrap();
    assert_eq!(
        get(&cfg, "user_animations_in_place.preview_item"),
        Some(&s(
            "resources/animations/single/weapons/g/item/stand/3rd_view/idle"
        ))
    );
    assert_eq!(notes.len(), 1);
    // A config without the key (no lobby pose) is left alone.
    let mut bare = t(vec![]);
    super::config::own_preview_item(&mut bare, &mut notes).unwrap();
    assert_eq!(bare, t(vec![]));
}

/// A clip is checked against the skeleton it is played on: the gun's for a
/// state's `animations` and `preview_item`, the player's for the rest.
#[test]
fn clips_of_another_skeleton_are_refused() {
    use super::resources::{ClipOwner, check_clip_skeletons};
    use std::collections::{BTreeMap, BTreeSet};
    assert_eq!(ClipOwner::of("states.idle.animations"), ClipOwner::Gun);
    assert_eq!(
        ClipOwner::of("user_animations_in_place.preview_item"),
        ClipOwner::Gun
    );
    assert_eq!(
        ClipOwner::of("states.idle.user_animations"),
        ClipOwner::Player
    );
    assert_eq!(
        ClipOwner::of("user_animations_in_place.preview"),
        ClipOwner::Player
    );
    assert_eq!(ClipOwner::of("recoil_animations"), ClipOwner::Player);
    let set = |b: &[&str]| b.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    let gun = set(&["wpn_g", "offset", "bn_g_slide"]);
    let player = set(&["bip01", "bip01_r_hand"]);
    let clips: BTreeMap<&str, BTreeSet<String>> = BTreeMap::from([
        (
            "animations/g/idle.bones_names",
            set(&["object_movement", "wpn_g", "offset", "bn_g_slide"]),
        ),
        (
            "animations/pkp/idle.bones_names",
            set(&["object_movement", "wpn_pkp", "offset", "bn_pkp_belt"]),
        ),
        (
            "animations/hands/idle.bones_names",
            set(&["object_movement", "bip01", "bip01_r_hand"]),
        ),
    ]);
    let mut bones_of = |path: &str| Ok(clips.get(path).cloned());
    let cfg = |preview: &str, hands: &str| {
        t(vec![
            (
                "states",
                t(vec![(
                    "idle",
                    t(vec![
                        ("animations", list(vec![s("resources/animations/g/idle")])),
                        ("user_animations", list(vec![s(hands)])),
                    ]),
                )]),
            ),
            (
                "user_animations_in_place",
                t(vec![("preview_item", s(preview))]),
            ),
        ])
    };
    check_clip_skeletons(
        "wpn_g",
        &cfg(
            "resources/animations/g/idle",
            "resources/animations/hands/idle",
        ),
        &gun,
        &player,
        &mut bones_of,
    )
    .unwrap();
    let error = check_clip_skeletons(
        "wpn_g",
        &cfg(
            "resources/animations/pkp/idle",
            "resources/animations/hands/idle",
        ),
        &gun,
        &player,
        &mut bones_of,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("preview_item: animations/pkp/idle"),
        "{error}"
    );
    assert!(error.contains("bn_pkp_belt, wpn_pkp"), "{error}");
    // A gun clip played on the character is refused as well.
    let error = check_clip_skeletons(
        "wpn_g",
        &cfg("resources/animations/g/idle", "resources/animations/g/idle"),
        &gun,
        &player,
        &mut bones_of,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("user_animations: animations/g/idle"),
        "{error}"
    );
    // A clip no archive has is the missing-resource check's.
    check_clip_skeletons(
        "wpn_g",
        &cfg(
            "resources/animations/g/idle",
            "resources/animations/gone/idle",
        ),
        &gun,
        &player,
        &mut bones_of,
    )
    .unwrap();
}

/// A resource key naming nothing is refused.
#[test]
fn an_empty_resource_name_is_refused() {
    use super::resources::{resource_file, sound_file};
    assert!(resource_file("object.model", "").is_err());
    assert!(resource_file("particles.shoot", "").is_err());
    assert!(resource_file("states.idle.animations", "").is_err());
    assert!(sound_file(257, "").is_err());
    // An empty string under a key that names no resource is no reference.
    assert_eq!(
        resource_file("ui_desc.text_descriptions.name", "").unwrap(),
        None
    );
}
