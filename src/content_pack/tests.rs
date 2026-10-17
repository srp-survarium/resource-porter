// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use tempfile::TempDir;

use super::*;
use crate::content_pack::from_bundle::{FromBundleOptions, from_bundle};

const ITEM: &str = "gameplay/items/armour/torso/stock.options";
const TEXTURE: &str = "misc/stock/body.bin";
const MODE: &str = "gameplay/game_modes/level_01_day_2.options";

fn config(json: Json) -> Vec<u8> {
    BinaryConfig {
        root: value::to_config(&json, None).unwrap(),
    }
    .to_bytes()
    .unwrap()
}

fn dictionary(items: Json, maps: Json) -> Vec<u8> {
    config(json!({"table": {
        "items_dict": items,
        "maps_dict": maps,
        "categories_dict": {"table": {"category_2": {"table": {"id": 2}}}},
    }}))
}

fn item_entry(id: i32, cfg: &str) -> Json {
    json!({"table": {"cfg_name": cfg, "dict_id": id, "is_stack": false, "base_cost": 100}})
}

fn localization(strings: Json) -> Vec<u8> {
    config(json!({"table": {"strings": {"table": strings}}}))
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn icon(fill: u8) -> Vec<u8> {
    vec![fill; atlas::CELL_WIDTH * atlas::CELL_HEIGHT * 4]
}

/// A small v0.26g-shaped archive.
struct Fixture {
    temp: TempDir,
    base: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let tree = temp.path().join("base-tree");
        write(
            &tree,
            DICTIONARY,
            &dictionary(
                json!([item_entry(3, ITEM)]),
                json!([{"table": {"id": 61, "cfg_mode_name": "level_01_day_2.options"}}]),
            ),
        );
        for language in ["english", "russian"] {
            write(
                &tree,
                &localization_path(language),
                &localization(json!({"st_stock": "Stock"})),
            );
        }
        write(
            &tree,
            ITEM,
            &config(json!({"table": {
                "parameters": {"table": {"armor": 0.5, "weight": 2.0, "slot": 2}},
                "ui_desc": {"table": {"icon": [1, 1]}},
                "list": [1, 2],
            }})),
        );
        write(
            &tree,
            MODE,
            &config(json!({"table": {"mode": {"table": {"name": "level_01_2"}}}})),
        );
        write(&tree, TEXTURE, b"stock texture");
        let mut swf = atlas::Swf::parse(&atlas::tests::synthetic_swf(342, 182)).unwrap();
        let mut bitmap = swf.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
        bitmap.put_cell(1, 1, &icon(1)).unwrap();
        swf.set_bitmap(atlas::DEFAULT_SYMBOL, &bitmap).unwrap();
        write(&tree, atlas::DEFAULT_SWF, &swf.to_bytes().unwrap());
        let base = temp.path().join("base.db");
        crate::archive::write_folder(&tree, &base).unwrap();
        Self { temp, base }
    }

    fn pack(&self, id: &str, operations: Json, files: &[(&str, &[u8])]) -> PathBuf {
        self.pack_with(id, operations, files, json!({}))
    }

    fn pack_with(
        &self,
        id: &str,
        operations: Json,
        files: &[(&str, &[u8])],
        extra: Json,
    ) -> PathBuf {
        let dir = self.temp.path().join("packs").join(id);
        for (path, bytes) in files {
            write(&dir, path, bytes);
        }
        let mut manifest =
            json!({"id": id, "version": "1", "target_build": "0.26g", "operations": operations});
        for (key, value) in extra.as_object().unwrap() {
            manifest[key] = value.clone();
        }
        write(
            &dir,
            MANIFEST_FILE,
            &serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        dir
    }

    fn compose(&self, name: &str, packs: &[PathBuf]) -> Result<InstallRecord> {
        compose(&ComposeOptions {
            base_db: &self.base,
            packs,
            work_dir: &self.temp.path().join(format!("work-{name}")),
            output_db: &self.temp.path().join(format!("{name}.db")),
            server_export: None,
            force: false,
        })
    }

    fn output(&self, name: &str) -> ResourceDb {
        ResourceDb::open(self.temp.path().join(format!("{name}.db"))).unwrap()
    }

    fn check(&self, packs: &[PathBuf]) -> Result<CheckReport> {
        check(packs, Some(&self.base), &[])
    }
}

fn read_config(db: &mut ResourceDb, path: &str) -> ConfigValue {
    BinaryConfig::parse(&db.read(path).unwrap()).unwrap().root
}

fn error_text(result: Result<impl Sized>) -> String {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => format!("{error:#}"),
    }
}

#[test]
fn add_file_composes_and_is_deterministic() {
    let fixture = Fixture::new();
    let pack = fixture.pack(
        "new-texture",
        json!([{"op": "add_file", "path": "misc/ported/new.bin"}]),
        &[("files/misc/ported/new.bin", b"new texture")],
    );
    let first = fixture
        .compose("first", std::slice::from_ref(&pack))
        .unwrap();
    let second = fixture.compose("second", &[pack]).unwrap();
    assert_eq!(first.output_db.sha256, second.output_db.sha256);
    assert_eq!(first.added_files, vec!["misc/ported/new.bin".to_owned()]);
    assert_eq!(
        fixture.output("first").read("misc/ported/new.bin").unwrap(),
        b"new texture"
    );
    assert_eq!(
        fixture.output("first").read(TEXTURE).unwrap(),
        b"stock texture"
    );
    let record: Json = serde_json::from_slice(
        &fs::read(install_record_path(&fixture.temp.path().join("first.db"))).unwrap(),
    )
    .unwrap();
    assert_eq!(record["packs"][0]["id"], "new-texture");
}

#[test]
fn two_packs_adding_the_same_path_conflict() {
    let fixture = Fixture::new();
    let operations = json!([{"op": "add_file", "path": "misc/ported/new.bin"}]);
    let a = fixture.pack(
        "a",
        operations.clone(),
        &[("files/misc/ported/new.bin", b"a")],
    );
    let b = fixture.pack("b", operations, &[("files/misc/ported/new.bin", b"b")]);
    assert!(
        error_text(check(&[a, b], None, &[])).contains("both add or replace misc/ported/new.bin")
    );
}

#[test]
fn add_file_rejects_an_existing_base_path() {
    let fixture = Fixture::new();
    let pack = fixture.pack(
        "a",
        json!([{"op": "add_file", "path": TEXTURE}]),
        &[(&format!("files/{TEXTURE}"), b"x")],
    );
    assert!(check(std::slice::from_ref(&pack), None, &[]).is_ok());
    assert!(error_text(fixture.check(&[pack])).contains("already exists in the base"));
}

#[test]
fn replace_file_detects_drift() {
    let fixture = Fixture::new();
    let good = sha256_hex(b"stock texture");
    let stale = sha256_hex(b"older texture");
    let file: &[(&str, &[u8])] = &[("files/misc/stock/body.bin", b"replacement")];
    let drifted = fixture.pack(
        "drifted",
        json!([{"op": "replace_file", "path": TEXTURE, "base_sha256": stale}]),
        file,
    );
    assert!(error_text(fixture.check(&[drifted])).contains("drift"));
    let current = fixture.pack(
        "current",
        json!([{"op": "replace_file", "path": TEXTURE, "base_sha256": good}]),
        file,
    );
    let record = fixture.compose("replace", &[current]).unwrap();
    assert_eq!(record.replaced_files, vec![TEXTURE.to_owned()]);
    assert_eq!(
        fixture.output("replace").read(TEXTURE).unwrap(),
        b"replacement"
    );
}

#[test]
fn config_set_edits_fields_and_conflicts_on_different_values() {
    let fixture = Fixture::new();
    let set = |value: Json| json!([{"op": "config_set", "path": ITEM, "values": {"parameters.armor": value}}]);
    let a = fixture.pack("a", set(json!(1)), &[]);
    let same = fixture.pack("same", set(json!(1)), &[]);
    let other = fixture.pack("other", set(json!(0.9)), &[]);
    let weight = fixture.pack(
        "weight",
        json!([{"op": "config_set", "path": ITEM, "values": {"parameters.weight": 3.5, "parameters.new_flag": true}}]),
        &[],
    );
    assert!(error_text(check(&[a.clone(), other], None, &[])).contains("differently"));
    fixture.compose("set", &[a, same, weight]).unwrap();
    let root = read_config(&mut fixture.output("set"), ITEM);
    // The integer literal follows the stock float type.
    assert_eq!(
        value_at(&root, "parameters.armor"),
        Some(&ConfigValue::Float(1.0))
    );
    assert_eq!(
        value_at(&root, "parameters.weight"),
        Some(&ConfigValue::Float(3.5))
    );
    assert_eq!(
        value_at(&root, "parameters.new_flag"),
        Some(&ConfigValue::Boolean(true))
    );
    assert_eq!(
        value_at(&root, "parameters.slot"),
        Some(&ConfigValue::Integer(2))
    );
}

#[test]
fn overlapping_config_paths_from_different_packs_conflict() {
    let fixture = Fixture::new();
    let a = fixture.pack(
        "a",
        json!([{"op": "config_set", "path": ITEM, "values": {"parameters.armor": 1.0}}]),
        &[],
    );
    let b = fixture.pack(
        "b",
        json!([{"op": "config_set", "path": ITEM, "values": {"parameters": {"table": {"armor": 1.0}}}}]),
        &[],
    );
    assert!(error_text(check(&[a, b], None, &[])).contains("conflict"));
}

#[test]
fn config_append_extends_arrays() {
    let fixture = Fixture::new();
    let a = fixture.pack(
        "a",
        json!([{"op": "config_append", "path": ITEM, "array_path": "list", "values": [3]}]),
        &[],
    );
    let b = fixture.pack(
        "b",
        json!([{"op": "config_append", "path": ITEM, "array_path": "list", "values": [4]}]),
        &[],
    );
    fixture.compose("append", &[a.clone(), b.clone()]).unwrap();
    let root = read_config(&mut fixture.output("append"), ITEM);
    let expected: Vec<ConfigValue> = (1..=4).map(ConfigValue::Integer).collect();
    assert_eq!(root.named("list"), Some(&ConfigValue::Indexed(expected)));
    verify(&fixture.temp.path().join("append.db"), &[], &[a, b]).unwrap();
}

#[test]
fn dictionary_add_checks_ids_against_base_and_other_packs() {
    let fixture = Fixture::new();
    let add = |id: i32, cfg: &str| {
        json!([{"op": "dictionary_add", "table": "items_dict",
                "entries": [{"cfg_name": cfg, "dict_id": id, "is_stack": false, "base_cost": 5}]}])
    };
    let a = fixture.pack("a", add(1810, "gameplay/items/a.options"), &[]);
    let b = fixture.pack("b", add(1810, "gameplay/items/b.options"), &[]);
    assert!(error_text(check(&[a.clone(), b], None, &[])).contains("both add items_dict id 1810"));
    let taken = fixture.pack("taken", add(3, "gameplay/items/c.options"), &[]);
    assert!(error_text(fixture.check(&[taken])).contains("already occupied in the base"));
    let map = fixture.pack(
        "map",
        json!([
            {"op": "dictionary_add", "table": "maps_dict", "entries": [{"id": 194, "cfg_mode_name": "v_level_2.options"}]},
            {"op": "add_file", "path": "gameplay/game_modes/v_level_2.options"},
        ]),
        &[("files/gameplay/game_modes/v_level_2.options", &config(json!({"table": {"mode": {"table": {"name": "v"}}}})))],
    );
    fixture.compose("dict", &[a.clone(), map.clone()]).unwrap();
    let root = read_config(&mut fixture.output("dict"), DICTIONARY);
    let Some(ConfigValue::Indexed(items)) = root.named("items_dict") else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    let Some(ConfigValue::Indexed(maps)) = root.named("maps_dict") else {
        panic!()
    };
    assert_eq!(maps[1].named("id"), Some(&ConfigValue::Integer(194)));
    verify(&fixture.temp.path().join("dict.db"), &[], &[a, map]).unwrap();
}

#[test]
fn localization_add_conflicts_on_different_text() {
    let fixture = Fixture::new();
    let add = |text: &str| json!([{"op": "localization_add", "strings": {"st_new": text}}]);
    let a = fixture.pack("a", add("New"), &[]);
    let same = fixture.pack("same", add("New"), &[]);
    let other = fixture.pack("other", add("Other"), &[]);
    assert!(error_text(check(&[a.clone(), other], None, &[])).contains("different text"));
    let base_clash = fixture.pack(
        "clash",
        json!([{"op": "localization_add", "strings": {"st_stock": "Changed"}}]),
        &[],
    );
    assert!(error_text(fixture.check(&[base_clash])).contains("already exists in the base"));
    let russian = fixture.pack(
        "russian",
        json!([{"op": "localization_add", "strings": {"st_ru": "Новый"}, "languages": ["russian"]}]),
        &[],
    );
    fixture.compose("loc", &[a, same, russian]).unwrap();
    let mut output = fixture.output("loc");
    for language in ["english", "russian"] {
        let root = read_config(&mut output, &localization_path(language));
        assert_eq!(
            value_at(&root, "strings.st_new"),
            Some(&ConfigValue::String("New".into()))
        );
    }
    let english = read_config(&mut output, &localization_path("english"));
    assert_eq!(value_at(&english, "strings.st_ru"), None);
    let russian = read_config(&mut output, &localization_path("russian"));
    assert_eq!(
        value_at(&russian, "strings.st_ru"),
        Some(&ConfigValue::String("Новый".into()))
    );
}

fn icon_png(dir: &Path, name: &str, fill: u8) -> (String, Vec<u8>) {
    let pixels = icon(fill);
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    atlas::write_icon_png(&path, &pixels).unwrap();
    (
        fs::read(&path).unwrap().len().to_string(),
        fs::read(path).unwrap(),
    )
}

#[test]
fn atlas_icons_compose_and_reject_cell_collisions() {
    let fixture = Fixture::new();
    let scratch = fixture.temp.path().join("icons");
    let (_, red) = icon_png(&scratch, "red.png", 200);
    let (_, blue) = icon_png(&scratch, "blue.png", 100);
    let icons = |row: u32, col: u32| json!([{"op": "atlas_icons_add", "icons": [{"row": row, "col": col, "source": "icons/i.png", "premultiplied": true}]}]);
    let a = fixture.pack("a", icons(1, 3), &[("icons/i.png", &red)]);
    let b = fixture.pack("b", icons(1, 3), &[("icons/i.png", &blue)]);
    assert!(error_text(check(&[a.clone(), b], None, &[])).contains("both draw atlas cell [1, 3]"));
    let stock = fixture.pack("stock", icons(1, 1), &[("icons/i.png", &blue)]);
    assert!(error_text(fixture.check(&[stock])).contains("already occupied"));
    let c = fixture.pack("c", icons(2, 3), &[("icons/i.png", &blue)]);
    fixture.compose("atlas", &[a.clone(), c.clone()]).unwrap();
    let swf =
        atlas::Swf::parse(&fixture.output("atlas").read(atlas::DEFAULT_SWF).unwrap()).unwrap();
    let bitmap = swf.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    assert_eq!(bitmap.width, 3 * atlas::CELL_WIDTH);
    assert_eq!(bitmap.cell(1, 1).unwrap(), icon(1));
    assert_eq!(bitmap.cell(1, 3).unwrap(), icon(200));
    assert_eq!(bitmap.cell(2, 3).unwrap(), icon(100));
    verify(&fixture.temp.path().join("atlas.db"), &[], &[a, c]).unwrap();
}

#[test]
fn editing_another_packs_file_requires_a_dependency() {
    let fixture = Fixture::new();
    let new = "gameplay/items/new.options";
    let bytes = config(json!({"table": {"armor": 0.5}}));
    let adder = fixture.pack(
        "adder",
        json!([{"op": "add_file", "path": new}]),
        &[(&format!("files/{new}"), &bytes)],
    );
    let edit = json!([{"op": "config_set", "path": new, "values": {"armor": 0.75}}]);
    let editor = fixture.pack("editor", edit.clone(), &[]);
    assert!(error_text(check(&[adder.clone(), editor], None, &[])).contains("declare `requires`"));
    let dependent = fixture.pack_with("dependent", edit, &[], json!({"requires": ["adder"]}));
    assert!(
        error_text(check(&[dependent.clone(), adder.clone()], None, &[]))
            .contains("must be composed before it")
    );
    fixture.compose("requires", &[adder, dependent]).unwrap();
    let root = read_config(&mut fixture.output("requires"), new);
    assert_eq!(root.named("armor"), Some(&ConfigValue::Float(0.75)));
}

#[test]
fn server_export_holds_effective_server_content() {
    let fixture = Fixture::new();
    let item = "gameplay/items/armour/torso/new.options";
    let item_bytes = config(json!({"table": {"parameters": {"table": {"armor": 0.25}}}}));
    let pack = fixture.pack_with(
        "equipment",
        json!([
            {"op": "add_file", "path": item},
            {"op": "dictionary_add", "table": "items_dict", "entries": [{"cfg_name": item, "dict_id": 1900, "is_stack": false, "base_cost": 1}]},
            {"op": "config_set", "path": ITEM, "values": {"parameters.armor": 0.75}},
            {"op": "config_set", "path": MODE, "values": {"mode.name": "renamed"}},
        ]),
        &[(&format!("files/{item}"), &item_bytes)],
        json!({"server": {"items": [{"dict_id": 1900, "sellers": [[2, 2]]}]}}),
    );
    let export = fixture.temp.path().join("server");
    let output_db = fixture.temp.path().join("export.db");
    let record = compose(&ComposeOptions {
        base_db: &fixture.base,
        packs: &[pack],
        work_dir: &fixture.temp.path().join("work-export"),
        output_db: &output_db,
        server_export: Some(&export),
        force: false,
    })
    .unwrap();
    let mut output = fixture.output("export");
    for path in [DICTIONARY, item, ITEM, MODE] {
        assert_eq!(
            fs::read(export.join(path)).unwrap(),
            output.read(path).unwrap(),
            "{path}"
        );
    }
    assert!(!export.join(TEXTURE).exists());
    let content: Json =
        serde_json::from_slice(&fs::read(export.join("content.json")).unwrap()).unwrap();
    assert_eq!(content["server"]["equipment"]["items"][0]["dict_id"], 1900);
    assert_eq!(content["output_db"]["sha256"], record.output_db.sha256);
    assert_eq!(
        content["packs"][0]["manifest_sha256"],
        record.packs[0].manifest_sha256
    );
}

/// A legacy bundle built on a database that already held another port.
#[test]
fn from_bundle_keeps_only_owned_shared_changes() {
    let fixture = Fixture::new();
    // Build base: the target base plus an earlier port (item 1805, its string,
    // its icon and a material the new bundle reuses without shipping).
    let earlier_tree = fixture.temp.path().join("earlier-tree");
    let mut base = ResourceDb::open(&fixture.base).unwrap();
    base.extract_all(&earlier_tree).unwrap();
    let earlier_items = json!([
        item_entry(3, ITEM),
        item_entry(1805, "gameplay/items/earlier.options")
    ]);
    let maps = json!([{"table": {"id": 61, "cfg_mode_name": "level_01_day_2.options"}}]);
    write(
        &earlier_tree,
        DICTIONARY,
        &dictionary(earlier_items, maps.clone()),
    );
    write(
        &earlier_tree,
        "material_instances/ported/new/body.material",
        &config(json!({"table": {"material": {"table": {"effect": "reused"}}}})),
    );
    write(
        &earlier_tree,
        "gameplay/items/earlier.options",
        &config(json!({"table": {"x": 1}})),
    );
    for language in ["english", "russian"] {
        write(
            &earlier_tree,
            &localization_path(language),
            &localization(json!({"st_stock": "Stock", "st_earlier": "Earlier"})),
        );
    }
    let mut swf = atlas::Swf::parse(&base.read(atlas::DEFAULT_SWF).unwrap()).unwrap();
    let mut bitmap = swf.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    bitmap.put_cell(1, 3, &icon(50)).unwrap();
    swf.set_bitmap(atlas::DEFAULT_SYMBOL, &bitmap).unwrap();
    write(&earlier_tree, atlas::DEFAULT_SWF, &swf.to_bytes().unwrap());
    let build_base = fixture.temp.path().join("earlier.db");
    crate::archive::write_folder(&earlier_tree, &build_base).unwrap();

    // The bundle: whole copies of every shared file, carrying the earlier port too.
    let bundle = fixture.temp.path().join("bundle");
    let overlay = bundle.join("overlay");
    let new_item = "gameplay/items/armour/torso/new.options";
    let mut files = Vec::new();
    let mut put = |path: &str, bytes: Vec<u8>| {
        write(&overlay, path, &bytes);
        files.push(json!({"path": path, "sha256": sha256_hex(&bytes), "size": bytes.len(), "source": "test"}));
    };
    put(
        new_item,
        config(json!({"table": {
            "data": {"table": {"material": "ported/new/body"}},
            "ui_desc": {"table": {"icon": [2, 3], "name": "st_new_name"}},
        }})),
    );
    put(
        DICTIONARY,
        dictionary(
            json!([
                item_entry(3, ITEM),
                item_entry(1805, "gameplay/items/earlier.options"),
                item_entry(1810, new_item)
            ]),
            maps,
        ),
    );
    for (language, text) in [("english", "New"), ("russian", "Новый")] {
        put(
            &localization_path(language),
            localization(
                json!({"st_stock": "Stock", "st_earlier": "Earlier", "st_new_name": text}),
            ),
        );
    }
    put(
        ITEM,
        config(json!({"table": {
            "parameters": {"table": {"armor": 0.75, "weight": 2.0, "slot": 2}},
            "ui_desc": {"table": {"icon": [1, 1]}},
            "list": [1, 2, 3],
        }})),
    );
    bitmap.put_cell(2, 3, &icon(77)).unwrap();
    swf.set_bitmap(atlas::DEFAULT_SYMBOL, &bitmap).unwrap();
    put(atlas::DEFAULT_SWF, swf.to_bytes().unwrap());
    write(
        &bundle,
        "port-manifest.json",
        &serde_json::to_vec(&json!({"asset_id": "test", "files": files})).unwrap(),
    );
    write(&bundle, "equipment.json", br#"[{"dict_id": 1810}]"#);

    let output = fixture.temp.path().join("converted");
    let manifest = from_bundle(&FromBundleOptions {
        base_db: &fixture.base,
        base_patches: &[],
        build_base_db: Some(&build_base),
        bundle: &bundle,
        output: &output,
        id: "new-item",
        version: "8",
    })
    .unwrap();
    let operations = serde_json::to_value(&manifest.operations).unwrap();
    let text = operations.to_string();
    assert!(text.contains("\"dict_id\":1810"));
    assert!(
        !text.contains("1805"),
        "inherited dictionary row leaked: {text}"
    );
    assert!(!text.contains("st_earlier"), "inherited string leaked");
    assert!(text.contains("\"row\":2"));
    assert!(!text.contains("\"row\":1"), "inherited icon leaked");
    assert!(
        text.contains("material_instances/ported/new/body.material"),
        "reused material not carried"
    );
    assert!(text.contains("parameters.armor"));
    assert!(text.contains("config_append"));
    assert!(text.contains("\"languages\":[\"english\"]"));
    assert_eq!(
        manifest.server.as_ref().unwrap()["items"][0]["dict_id"],
        1810
    );

    fixture
        .compose("converted", std::slice::from_ref(&output))
        .unwrap();
    verify(&fixture.temp.path().join("converted.db"), &[], &[output]).unwrap();
    let mut composed = fixture.output("converted");
    // Field-level edits reproduce the bundle's own copy exactly.
    assert!(semantic_eq(
        &read_config(&mut composed, ITEM),
        &BinaryConfig::parse(&fs::read(overlay.join(ITEM)).unwrap())
            .unwrap()
            .root,
    ));
    assert!(composed.contains("material_instances/ported/new/body.material"));
    assert!(!composed.contains("gameplay/items/earlier.options"));
}

fn new_item_pack(fixture: &Fixture) -> PathBuf {
    let scratch = fixture.temp.path().join("patch-icons");
    let (_, red) = icon_png(&scratch, "red.png", 200);
    fixture.pack(
        "new-item",
        json!([
            {"op": "add_file", "path": "gameplay/items/armour/torso/new.options"},
            {"op": "add_file", "path": "misc/new/payload.bin"},
            {"op": "dictionary_add", "table": "items_dict",
             "entries": [{"cfg_name": "gameplay/items/armour/torso/new.options",
                          "dict_id": 1805, "is_stack": false, "base_cost": 7}]},
            {"op": "localization_add", "strings": {"st_new": "New"}},
            {"op": "atlas_icons_add", "icons": [{"row": 2, "col": 3, "source": "icons/i.png", "premultiplied": true}]},
        ]),
        &[
            (
                "files/gameplay/items/armour/torso/new.options",
                &config(json!({"table": {
                    "parameters": {"table": {"armor": 0.9, "weight": 35.0}},
                    "data": {"table": {"additional_model_slots": ["helmet", "mask"]}},
                }})),
            ),
            ("files/misc/new/payload.bin", &b"payload ".repeat(5000)),
            ("icons/i.png", &red),
        ],
    )
}

fn patch_options<'a>(
    fixture: &'a Fixture,
    base_patches: &'a [PathBuf],
    packs: &'a [PathBuf],
    replace: &'a [(String, PathBuf)],
    name: &str,
    number: u32,
) -> patch::PatchOptions<'a> {
    let leak = |path: PathBuf| -> &'a Path { Box::leak(path.into_boxed_path()) };
    patch::PatchOptions {
        base_db: &fixture.base,
        base_patches,
        packs,
        replace,
        out_dir: leak(fixture.temp.path().join(format!("patch-{name}"))),
        archive: Some(leak(
            fixture
                .temp
                .path()
                .join(format!("{name}/resources.db.patch_{number}")),
        )),
        force: false,
    }
}

fn folder(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    crate::archive::folder_files(dir)
        .unwrap()
        .into_iter()
        .map(|(path, file)| (path, fs::read(file).unwrap()))
        .collect()
}

#[test]
fn a_patch_holds_what_differs_and_mounts_as_the_composition() {
    let fixture = Fixture::new();
    let packs = [new_item_pack(&fixture)];
    fixture.compose("full", &packs).unwrap();
    let options = patch_options(&fixture, &[], &packs, &[], "plain", 14);
    let record = patch::patch(&options).unwrap();
    assert!(patch::record_path(options.out_dir).exists());

    // The added files and the edited shared ones, nothing the base has.
    let files = folder(options.out_dir);
    let mut expected = vec![
        DICTIONARY.to_owned(),
        atlas::DEFAULT_SWF.to_owned(),
        "gameplay/items/armour/torso/new.options".to_owned(),
        localization_path("english"),
        localization_path("russian"),
        "misc/new/payload.bin".to_owned(),
    ];
    expected.sort();
    assert_eq!(files.keys().cloned().collect::<Vec<_>>(), expected);
    let kinds: BTreeMap<_, _> = record
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.kind))
        .collect();
    assert_eq!(kinds[DICTIONARY], "override");
    assert_eq!(kinds["misc/new/payload.bin"], "add");

    // The archive is the folder, and the game's view (stock, then the
    // patch) reads like the whole composed database.
    let archive = options.archive.unwrap();
    let mut packed = ResourceDb::open(archive).unwrap();
    for (path, bytes) in &files {
        assert_eq!(&packed.read(path).unwrap(), bytes, "{path}");
    }
    let mut mounted =
        ResourceDb::open_layers(&[fixture.base.clone(), archive.to_path_buf()]).unwrap();
    let mut composed = fixture.output("full");
    let paths: Vec<String> = composed.paths().map(str::to_owned).collect();
    assert_eq!(mounted.paths().count(), paths.len());
    for path in &paths {
        assert_eq!(
            mounted.read(path).unwrap(),
            composed.read(path).unwrap(),
            "{path}"
        );
    }
    assert_eq!(
        record.archive.as_ref().unwrap().sha256,
        sha256_file(archive).unwrap()
    );

    // Deterministic, and nothing is overwritten without --force.
    assert!(error_text(patch::patch(&options)).contains("already exists"));
    let again = patch::patch(&patch::PatchOptions {
        force: true,
        ..patch_options(&fixture, &[], &packs, &[], "plain", 14)
    })
    .unwrap();
    assert_eq!(
        again.archive.unwrap().sha256,
        record.archive.unwrap().sha256
    );
}

fn earlier_layers(fixture: &Fixture, earlier: &[PathBuf]) -> Vec<PathBuf> {
    patch::base_layers(&fixture.base, earlier).unwrap()
}

#[test]
fn a_patch_goes_over_earlier_patches_and_is_named_as_the_game_mounts_it() {
    let fixture = Fixture::new();
    let packs = [new_item_pack(&fixture)];
    let first = patch_options(&fixture, &[], &packs, &[], "first", 14);
    patch::patch(&first).unwrap();
    let earlier = [first.archive.unwrap().to_path_buf()];

    // The pack's files are in the base now: adding them again is refused.
    assert!(
        error_text(patch::patch(&patch_options(
            &fixture,
            &earlier,
            &packs,
            &[],
            "again",
            15
        )))
        .contains("already exists in the base")
    );
    // A replacement the base already has as it is makes no patch.
    let mut mounted = ResourceDb::open_layers(&earlier_layers(&fixture, &earlier)).unwrap();
    let same = fixture.temp.path().join("same.bin");
    fs::write(&same, mounted.read(DICTIONARY).unwrap()).unwrap();
    let replace = [(DICTIONARY.to_owned(), same)];
    assert!(
        error_text(patch::patch(&patch_options(
            &fixture,
            &earlier,
            &[],
            &replace,
            "same",
            15
        )))
        .contains("no patch")
    );

    // A new string goes onto the earlier patch's localization, not the
    // stock one: the earlier patch's string stays.
    let more = [fixture.pack(
        "more",
        json!([{"op": "localization_add", "strings": {"st_more": "More"}}]),
        &[],
    )];
    let second = patch_options(&fixture, &earlier, &more, &[], "second", 15);
    let record = patch::patch(&second).unwrap();
    let files = folder(second.out_dir);
    assert_eq!(
        files.keys().cloned().collect::<Vec<_>>(),
        [localization_path("english"), localization_path("russian")]
    );
    assert_eq!(record.base_patches.len(), 1);
    let strings = BinaryConfig::parse(&files[&localization_path("english")])
        .unwrap()
        .root;
    let strings = strings.named("strings").unwrap();
    assert!(strings.named("st_new").is_some() && strings.named("st_more").is_some());

    // Names the game does not mount, or a patch mounted before an earlier
    // one, are refused.
    let wrong = fixture.temp.path().join("resources.db.patch_01");
    assert!(
        error_text(patch::patch(&patch::PatchOptions {
            archive: Some(&wrong),
            ..patch_options(&fixture, &earlier, &more, &[], "wrong", 15)
        }))
        .contains("mounts only")
    );
    assert!(
        error_text(patch::patch(&patch_options(
            &fixture,
            &earlier,
            &more,
            &[],
            "low",
            14
        )))
        .contains("mounted before")
    );
    let renamed = fixture.temp.path().join("earlier.db");
    fs::copy(&earlier[0], &renamed).unwrap();
    assert!(
        error_text(patch::patch(&patch_options(
            &fixture,
            &[renamed],
            &more,
            &[],
            "renamed",
            15
        )))
        .contains("mounts only")
    );
}

#[test]
fn a_patch_takes_the_live_files_and_draws_icons_on_the_live_lobby() {
    let fixture = Fixture::new();
    let packs = [new_item_pack(&fixture)];
    let mut base = ResourceDb::open(&fixture.base).unwrap();
    let dir = fixture.temp.path();
    // What players have now: the stock texture as it is, a lobby with a cell
    // of the server's own, and a changed config.
    fs::write(dir.join("texture.bin"), base.read(TEXTURE).unwrap()).unwrap();
    let mut live = atlas::Swf::parse(&base.read(atlas::DEFAULT_SWF).unwrap()).unwrap();
    let mut bitmap = live.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    bitmap.put_cell(2, 1, &icon(7)).unwrap();
    live.set_bitmap(atlas::DEFAULT_SYMBOL, &bitmap).unwrap();
    fs::write(dir.join("lobby.swf"), live.to_bytes().unwrap()).unwrap();
    let mode = config(json!({"table": {"mode": {"table": {"name": "level_01_live"}}}}));
    fs::write(dir.join("mode.options"), &mode).unwrap();
    let replace = [
        (TEXTURE.to_owned(), dir.join("texture.bin")),
        (atlas::DEFAULT_SWF.to_owned(), dir.join("lobby.swf")),
        (MODE.to_owned(), dir.join("mode.options")),
    ];
    let options = patch_options(&fixture, &[], &packs, &replace, "live", 14);
    let record = patch::patch(&options).unwrap();
    assert_eq!(record.unchanged, [TEXTURE], "the stock bytes: not patched");
    assert_eq!(record.replaced.len(), 3);
    let files = folder(options.out_dir);
    assert!(!files.contains_key(TEXTURE));
    assert_eq!(files[MODE], mode);
    // The pack's icon is drawn on the live movie: its own cell stays.
    let movie = atlas::Swf::parse(&files[atlas::DEFAULT_SWF]).unwrap();
    let drawn = movie.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    assert_eq!(drawn.cell(2, 1).unwrap(), icon(7));
    assert_eq!(drawn.cell(1, 1).unwrap(), icon(1));
    assert_eq!(drawn.cell(2, 3).unwrap(), icon(200));

    // A pack may not add or replace a replaced file whole.
    let other = [fixture.pack(
        "texture",
        json!([{"op": "replace_file", "path": TEXTURE,
                "base_sha256": sha256_hex(&base.read(TEXTURE).unwrap())}]),
        &[(&format!("files/{TEXTURE}"), b"other texture")],
    )];
    assert!(
        error_text(patch::patch(&patch_options(
            &fixture,
            &[],
            &other,
            &replace[..1],
            "whole",
            14
        )))
        .contains("also replaced from outside")
    );
}

/// The release case: a bundle built on the stock database, converted over
/// the patches players already have (an earlier release of the same item, a
/// texture the server changed, the item's icon), then patched over them.
#[test]
fn a_bundle_built_on_stock_converts_over_an_earlier_release() {
    let fixture = Fixture::new();
    let live = fixture.pack(
        "live",
        json!([{"op": "replace_file", "path": TEXTURE,
                "base_sha256": sha256_hex(b"stock texture")}]),
        &[(&format!("files/{TEXTURE}"), b"live texture")],
    );
    let released_packs = [new_item_pack(&fixture), live];
    let first = patch_options(&fixture, &[], &released_packs, &[], "first", 12);
    patch::patch(&first).unwrap();
    let earlier = [first.archive.unwrap().to_path_buf()];
    let mut mounted = ResourceDb::open_layers(&earlier_layers(&fixture, &earlier)).unwrap();
    let released = atlas::Swf::parse(&mounted.read(atlas::DEFAULT_SWF).unwrap())
        .unwrap()
        .bitmap(atlas::DEFAULT_SYMBOL)
        .unwrap()
        .cell(2, 3)
        .unwrap();
    assert_eq!(released, icon(200));

    // The bundle: the stock files with the item's new revision.
    let mut stock = ResourceDb::open(&fixture.base).unwrap();
    let bundle = fixture.temp.path().join("bundle-stock");
    let overlay = bundle.join("overlay");
    let item = "gameplay/items/armour/torso/new.options";
    let mut files = Vec::new();
    let mut put = |path: &str, bytes: Vec<u8>| {
        write(&overlay, path, &bytes);
        files.push(json!({"path": path, "sha256": sha256_hex(&bytes), "size": bytes.len(), "source": "test"}));
    };
    put(
        item,
        config(json!({"table": {
            "parameters": {"table": {"armor": 0.95, "weight": 33.0}},
            "data": {"table": {"additional_model_slots": ["helmet", "mask"]}},
            "ui_desc": {"table": {"icon": [2, 3]}},
        }})),
    );
    put("misc/new/payload.bin", b"payload v2 ".repeat(4000));
    put(
        DICTIONARY,
        dictionary(
            json!([item_entry(3, ITEM), {"table": {"cfg_name": item, "dict_id": 1805, "is_stack": false, "base_cost": 9}}]),
            json!([{"table": {"id": 61, "cfg_mode_name": "level_01_day_2.options"}}]),
        ),
    );
    for language in ["english", "russian"] {
        put(
            &localization_path(language),
            localization(json!({"st_stock": "Stock", "st_new": "New", "st_newer": "Newer"})),
        );
    }
    // The stock texture, as the bundle's build base has it.
    put(TEXTURE, stock.read(TEXTURE).unwrap());
    let mut swf = atlas::Swf::parse(&stock.read(atlas::DEFAULT_SWF).unwrap()).unwrap();
    let mut bitmap = swf.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    bitmap.put_cell(2, 3, &icon(90)).unwrap();
    swf.set_bitmap(atlas::DEFAULT_SYMBOL, &bitmap).unwrap();
    put(atlas::DEFAULT_SWF, swf.to_bytes().unwrap());
    write(
        &bundle,
        "port-manifest.json",
        &serde_json::to_vec(&json!({"asset_id": "test", "files": files})).unwrap(),
    );

    let output = fixture.temp.path().join("converted-over");
    let manifest = from_bundle(&FromBundleOptions {
        base_db: &fixture.base,
        base_patches: &earlier,
        build_base_db: Some(&fixture.base),
        bundle: &bundle,
        output: &output,
        id: "new-item",
        version: "2",
    })
    .unwrap();
    let operations = serde_json::to_value(&manifest.operations).unwrap();
    let text = operations.to_string();
    // The released files are replaced against the released bytes.
    let payload = mounted.read("misc/new/payload.bin").unwrap();
    assert!(text.contains(&sha256_hex(&payload)), "{text}");
    assert!(!text.contains("add_file"), "{text}");
    // The texture the server changed stays the server's.
    assert!(!text.contains(TEXTURE), "{text}");
    // The released icon is redrawn, named by its hash.
    assert!(text.contains(&sha256_hex(&released)), "{text}");
    assert!(
        text.contains("st_newer") && !text.contains("st_stock"),
        "{text}"
    );

    let second = patch_options(
        &fixture,
        &earlier,
        std::slice::from_ref(&output),
        &[],
        "over",
        15,
    );
    patch::patch(&second).unwrap();
    let mut view = ResourceDb::open_layers(&earlier_layers(
        &fixture,
        &[earlier[0].clone(), second.archive.unwrap().to_path_buf()],
    ))
    .unwrap();
    assert_eq!(view.read(TEXTURE).unwrap(), b"live texture");
    assert_eq!(
        view.read("misc/new/payload.bin").unwrap(),
        b"payload v2 ".repeat(4000)
    );
    let dictionary = read_config(&mut view, DICTIONARY);
    let ConfigValue::Indexed(items) = dictionary.named("items_dict").unwrap() else {
        panic!()
    };
    let row = items
        .iter()
        .find(|row| row.named("dict_id") == Some(&ConfigValue::Integer(1805)))
        .unwrap();
    assert_eq!(row.named("base_cost"), Some(&ConfigValue::Integer(9)));
    assert_eq!(items.len(), 2);
    let cell = atlas::Swf::parse(&view.read(atlas::DEFAULT_SWF).unwrap())
        .unwrap()
        .bitmap(atlas::DEFAULT_SYMBOL)
        .unwrap()
        .cell(2, 3)
        .unwrap();
    assert_eq!(cell, icon(90));
    let strings = read_config(&mut view, &localization_path("english"));
    let strings = strings.named("strings").unwrap();
    assert!(strings.named("st_new").is_some() && strings.named("st_newer").is_some());
}

#[test]
fn an_icon_redraws_only_the_cell_it_names() {
    let fixture = Fixture::new();
    let scratch = fixture.temp.path().join("redraw-icons");
    let (_, blue) = icon_png(&scratch, "blue.png", 100);
    let redraw = |sha: String| {
        json!([{"op": "atlas_icons_add", "icons": [{"row": 1, "col": 1, "source": "icons/i.png",
                "premultiplied": true, "replaces_sha256": sha}]}])
    };
    // The base's cell [1, 1] holds icon(1).
    let right = fixture.pack(
        "right",
        redraw(sha256_hex(&icon(1))),
        &[("icons/i.png", &blue)],
    );
    fixture
        .compose("redraw", std::slice::from_ref(&right))
        .unwrap();
    let swf =
        atlas::Swf::parse(&fixture.output("redraw").read(atlas::DEFAULT_SWF).unwrap()).unwrap();
    assert_eq!(
        swf.bitmap(atlas::DEFAULT_SYMBOL)
            .unwrap()
            .cell(1, 1)
            .unwrap(),
        icon(100)
    );
    // Another icon there, or an empty cell, is drift.
    let wrong = fixture.pack(
        "wrong",
        redraw(sha256_hex(&icon(2))),
        &[("icons/i.png", &blue)],
    );
    assert!(error_text(fixture.check(&[wrong])).contains("not the icon it replaces"));
    let empty = fixture.pack(
        "empty",
        json!([{"op": "atlas_icons_add", "icons": [{"row": 2, "col": 2, "source": "icons/i.png",
                "premultiplied": true, "replaces_sha256": sha256_hex(&icon(1))}]}]),
        &[("icons/i.png", &blue)],
    );
    assert!(error_text(fixture.check(&[empty])).contains("empty cell"));
}

/// A bundle that draws into the weapon atlas becomes an `atlas_icons_add` of
/// that atlas; the equipment atlas is left alone.
#[test]
fn from_bundle_finds_the_weapon_icon_atlas() {
    let temp = TempDir::new().unwrap();
    let tree = temp.path().join("base-tree");
    write(
        &tree,
        DICTIONARY,
        &dictionary(json!([item_entry(3, ITEM)]), json!([])),
    );
    write(
        &tree,
        &localization_path("english"),
        &localization(json!({"st_stock": "Stock"})),
    );
    let weapons = "LobbyMenu_WeaponIcons";
    let base_swf =
        atlas::tests::synthetic_swf_with(&[(atlas::DEFAULT_SYMBOL, 342, 182), (weapons, 513, 728)]);
    write(&tree, atlas::DEFAULT_SWF, &base_swf);
    let base = temp.path().join("base.db");
    crate::archive::write_folder(&tree, &base).unwrap();

    let bundle = temp.path().join("bundle");
    let mut swf = atlas::Swf::parse(&base_swf).unwrap();
    let mut bitmap = swf.bitmap(weapons).unwrap();
    bitmap.put_cell(7, 3, &icon(77)).unwrap();
    swf.set_bitmap(weapons, &bitmap).unwrap();
    let bytes = swf.to_bytes().unwrap();
    write(&bundle.join("overlay"), atlas::DEFAULT_SWF, &bytes);
    write(
        &bundle,
        "port-manifest.json",
        &serde_json::to_vec(&json!({"asset_id": "rifles", "files": [
            {"path": atlas::DEFAULT_SWF, "sha256": sha256_hex(&bytes), "size": bytes.len(), "source": "test"}
        ]}))
        .unwrap(),
    );
    let output = temp.path().join("pack");
    let manifest = from_bundle(&FromBundleOptions {
        base_db: &base,
        base_patches: &[],
        build_base_db: None,
        bundle: &bundle,
        output: &output,
        id: "rifles",
        version: "1",
    })
    .unwrap();
    let operations = serde_json::to_value(&manifest.operations).unwrap();
    assert_eq!(operations.as_array().unwrap().len(), 1, "{operations}");
    assert_eq!(operations[0]["op"], "atlas_icons_add");
    assert_eq!(operations[0]["symbol"], weapons);
    assert_eq!(operations[0]["icons"][0]["row"], 7);
    assert_eq!(operations[0]["icons"][0]["col"], 3);

    let installed = temp.path().join("out.db");
    compose(&ComposeOptions {
        base_db: &base,
        packs: &[output],
        work_dir: &temp.path().join("work"),
        output_db: &installed,
        server_export: None,
        force: false,
    })
    .unwrap();
    let movie = atlas::Swf::parse(
        &ResourceDb::open(&installed)
            .unwrap()
            .read(atlas::DEFAULT_SWF)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(movie.bitmap(weapons).unwrap().cell(7, 3).unwrap(), icon(77));
    assert!(
        movie
            .same_except_bitmap(&atlas::Swf::parse(&base_swf).unwrap(), weapons)
            .unwrap()
    );
}

/// Two atlases of one movie, drawn by one pack: both cells land (each
/// atlas used to start again from the base movie, the last one winning).
#[test]
fn icons_of_two_atlases_of_one_movie_all_land() {
    let temp = TempDir::new().unwrap();
    let tree = temp.path().join("base-tree");
    write(
        &tree,
        DICTIONARY,
        &dictionary(json!([item_entry(3, ITEM)]), json!([])),
    );
    write(
        &tree,
        &localization_path("english"),
        &localization(json!({"st_stock": "Stock"})),
    );
    let (weapons, ammo) = ("LobbyMenu_WeaponIcons", "LobbyMenu_AmmoIcons");
    let base_swf = atlas::tests::synthetic_swf_with(&[(ammo, 513, 1456), (weapons, 513, 728)]);
    write(&tree, atlas::DEFAULT_SWF, &base_swf);
    let base = temp.path().join("base.db");
    crate::archive::write_folder(&tree, &base).unwrap();
    let pack = temp.path().join("pack");
    for (name, fill) in [("w.png", 60u8), ("a.png", 90)] {
        let path = pack.join("icons").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        atlas::write_icon_png(&path, &icon(fill)).unwrap();
    }
    write(
        &pack,
        MANIFEST_FILE,
        &serde_json::to_vec(&json!({"format_version": 1, "id": "two", "version": "1", "target_build": "0.26g", "operations": [
            {"op": "atlas_icons_add", "symbol": weapons, "icons": [{"row": 7, "col": 3, "source": "icons/w.png", "premultiplied": true}]},
            {"op": "atlas_icons_add", "symbol": ammo, "icons": [{"row": 16, "col": 3, "source": "icons/a.png", "premultiplied": true}]},
        ]}))
        .unwrap(),
    );
    let installed = temp.path().join("out.db");
    compose(&ComposeOptions {
        base_db: &base,
        packs: &[pack],
        work_dir: &temp.path().join("work"),
        output_db: &installed,
        server_export: None,
        force: false,
    })
    .unwrap();
    let movie = atlas::Swf::parse(
        &ResourceDb::open(&installed)
            .unwrap()
            .read(atlas::DEFAULT_SWF)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(movie.bitmap(weapons).unwrap().cell(7, 3).unwrap(), icon(60));
    assert_eq!(movie.bitmap(ammo).unwrap().cell(16, 3).unwrap(), icon(90));
}
