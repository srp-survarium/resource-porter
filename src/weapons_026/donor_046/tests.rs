// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::binary_config::NamedValue;
use crate::weapons_026::clips;

fn field(name: &str, value: V) -> NamedValue {
    NamedValue {
        name: name.into(),
        value,
    }
}

fn chunk(id: u32, len: usize) -> Vec<u8> {
    let mut b = id.to_le_bytes().to_vec();
    b.extend_from_slice(&(len as u32).to_le_bytes());
    b.extend(std::iter::repeat_n(id as u8, len));
    b
}

#[test]
fn a_046_surface_keeps_its_vertices_and_indices_only() {
    let donor = [chunk(3, 36), chunk(4, 12), chunk(110, 4), chunk(113, 72)].concat();
    assert_eq!(
        mesh_chunks(&donor).unwrap(),
        [chunk(3, 36), chunk(4, 12)].concat()
    );
    let unknown = [chunk(3, 36), chunk(4, 12), chunk(114, 8)].concat();
    assert!(mesh_chunks(&unknown).is_err());
    assert!(mesh_chunks(&[chunk(4, 12), chunk(3, 36)].concat()).is_err());
}

fn locator(name: &str, at: [f32; 3]) -> V {
    V::Named(vec![
        field("rotation", V::Float3([0.0; 3])),
        field("name", V::String(name.into())),
        field("bone_idx", V::Integer(1)),
        field("bone_name", V::String("offset".into())),
        field("position", V::Float3(at)),
    ])
}

fn properties(locators: Vec<V>) -> V {
    V::Named(vec![field("locators", V::Indexed(locators))])
}

#[test]
fn the_muzzle_and_sight_get_the_names_026g_reads() {
    // The MP5's: no barrel_point, a picatinny scope point.
    let mut mp5 = properties(vec![
        locator("aim", [0.0, 0.096, 0.233]),
        locator("muzzle_point", [0.0, 0.040, -0.358]),
        locator("fire_point", [0.0, 0.040, -0.374]),
        locator("picatini_scope_point", [0.0, 0.08, -0.1]),
    ]);
    add_locators(&mut mp5).unwrap();
    assert_eq!(
        locator_position(&mp5, "barrel_point"),
        Some([0.0, 0.040, -0.374])
    );
    assert_eq!(
        locator_position(&mp5, "scope_point"),
        Some([0.0, 0.08, -0.1])
    );
    // Nothing else moves; a second pass adds nothing.
    let before = mp5.clone();
    add_locators(&mut mp5).unwrap();
    assert_eq!(mp5, before);
}

#[test]
fn a_046_barrel_mount_moves_aside_for_the_muzzle_and_a_model_without_a_sight_gets_it_at_zero() {
    // The SVU's: 0.46c2's barrel_point is the barrel module's mount, mid-gun.
    let mut svu = properties(vec![
        locator("barrel_point", [0.0, 0.017, -0.271]),
        locator("fire_point", [0.0, 0.017, -0.595]),
        locator("muzzle_point", [0.0, 0.017, -0.417]),
        locator("aim", [0.0, 0.09, 0.23]),
    ]);
    add_locators(&mut svu).unwrap();
    assert_eq!(
        locator_position(&svu, "barrel_point"),
        Some([0.0, 0.017, -0.595])
    );
    assert_eq!(
        locator_position(&svu, BARREL_MOUNT),
        Some([0.0, 0.017, -0.271])
    );
    assert_eq!(locator_position(&svu, "scope_point"), Some([0.0; 3]));
    let before = svu.clone();
    add_locators(&mut svu).unwrap();
    assert_eq!(svu, before);
    let mut bare = properties(vec![locator("aim", [0.0, 0.1, 0.2])]);
    assert!(add_locators(&mut bare).is_err(), "no muzzle locator at all");
}

#[test]
fn configs_come_out_in_24_byte_records() {
    let tree = V::Named(vec![field(
        "skeleton",
        V::String("resources/animations/skeletons/weapons/mp5a1/mp5a1".into()),
    )]);
    let classic = BinaryConfig { root: tree.clone() }.to_bytes().unwrap();
    let out = native_layout("models/weapons/x.skinned_model/skeleton", &classic).unwrap();
    assert!(!BinaryConfig::is_compact(&out));
    assert_eq!(BinaryConfig::parse(&out).unwrap().root, tree);
}

/// [`clips::engine_words_cleared`] with the padding after each event name
/// cleared too (exporters leave whatever was in memory there).
fn engine_words_and_padding_cleared(b: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = clips::engine_words_cleared(b)?;
    for name in clips::event_names(b)? {
        let needle = [name.as_bytes(), &[0]].concat();
        let start = out
            .windows(needle.len())
            .rposition(|w| w == needle.as_slice())
            .ok_or_else(|| anyhow::anyhow!("event name {name}"))?;
        let mut at = start + needle.len();
        while at % 4 != 0 && at < out.len() {
            out[at] = 0;
            at += 1;
        }
    }
    Ok(out)
}

/// With the stock 0.26g database (`VOSTOK_026G_DB`) and the 0.46c2 game
/// folder (`VOSTOK_046C2_GAME`, its `resources.NN.db`): the AK-12 clips
/// 0.46c2 did not re-animate, converted, are 0.26g's own bytes, and the
/// MP5's surface and locators read as 0.26g's.
#[test]
fn the_046c2_game_reads_in_026g_layout() {
    let (Some(stock), Some(game)) = (
        std::env::var_os("VOSTOK_026G_DB"),
        std::env::var_os("VOSTOK_046C2_GAME"),
    ) else {
        eprintln!("skipped: set VOSTOK_026G_DB and VOSTOK_046C2_GAME");
        return;
    };
    let mut stock = ResourceDb::open(stock).unwrap();
    let mut donor = Donor046::open(Path::new(&game)).unwrap();
    let mut later = 0;
    for clip in [
        "item/stand/3rd_view/show",
        "item/stand/3rd_view/idle",
        "player/stand/3rd_view/on_site/hands_only/show",
        "player/stand/3rd_view/on_site/hands_only/hide",
    ] {
        let path = format!("animations/single/weapons/ak_12/{clip}.b-spline");
        let raw = donor.read_raw(&path).unwrap();
        later += usize::from(clips::clip_header(&raw).unwrap().events > 0);
        assert_eq!(
            engine_words_and_padding_cleared(&donor.read(&path).unwrap()).unwrap(),
            engine_words_and_padding_cleared(&stock.read(&path).unwrap()).unwrap(),
            "{clip}"
        );
    }
    assert!(later > 0, "no clip with event channels");
    let model = "models/weapons/smg/mp5a1/mp5a1.skinned_model";
    let surface = donor
        .read(&format!("{model}/render/mp5a1_s/converted_model"))
        .unwrap();
    assert_eq!(&surface[..4], &3u32.to_le_bytes());
    let properties = BinaryConfig::parse(
        &donor
            .read(&format!("{model}/render/export_properties"))
            .unwrap(),
    )
    .unwrap();
    assert!(locator_position(&properties.root, "barrel_point").is_some());
    assert!(locator_position(&properties.root, "scope_point").is_some());
    let mp5 = donor.item_config("mp5a1").unwrap();
    assert_eq!(
        mp5.named("object").and_then(|o| o.named("model")),
        Some(&V::String("weapons/smg/mp5a1/mp5a1".into()))
    );
}
