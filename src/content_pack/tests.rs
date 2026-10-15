// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::Read as _;
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
        let source = resources_db::pack::build_src_tree(&tree).unwrap();
        resources_db::pack::assemble_to_file(&source, &base).unwrap();
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
        check(packs, Some(&self.base))
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
    assert!(error_text(check(&[a, b], None)).contains("both add or replace misc/ported/new.bin"));
}

#[test]
fn add_file_rejects_an_existing_base_path() {
    let fixture = Fixture::new();
    let pack = fixture.pack(
        "a",
        json!([{"op": "add_file", "path": TEXTURE}]),
        &[(&format!("files/{TEXTURE}"), b"x")],
    );
    assert!(check(std::slice::from_ref(&pack), None).is_ok());
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
    assert!(error_text(check(&[a.clone(), other], None)).contains("differently"));
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
    assert!(error_text(check(&[a, b], None)).contains("conflict"));
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
    verify(&fixture.temp.path().join("append.db"), &[a, b]).unwrap();
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
    assert!(error_text(check(&[a.clone(), b], None)).contains("both add items_dict id 1810"));
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
    verify(&fixture.temp.path().join("dict.db"), &[a, map]).unwrap();
}

#[test]
fn localization_add_conflicts_on_different_text() {
    let fixture = Fixture::new();
    let add = |text: &str| json!([{"op": "localization_add", "strings": {"st_new": text}}]);
    let a = fixture.pack("a", add("New"), &[]);
    let same = fixture.pack("same", add("New"), &[]);
    let other = fixture.pack("other", add("Other"), &[]);
    assert!(error_text(check(&[a.clone(), other], None)).contains("different text"));
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
    assert!(error_text(check(&[a.clone(), b], None)).contains("both draw atlas cell [1, 3]"));
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
    verify(&fixture.temp.path().join("atlas.db"), &[a, c]).unwrap();
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
    assert!(error_text(check(&[adder.clone(), editor], None)).contains("declare `requires`"));
    let dependent = fixture.pack_with("dependent", edit, &[], json!({"requires": ["adder"]}));
    assert!(
        error_text(check(&[dependent.clone(), adder.clone()], None))
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
    resources_db::pack::assemble_to_file(
        &resources_db::pack::build_src_tree(&earlier_tree).unwrap(),
        &build_base,
    )
    .unwrap();

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
    verify(&fixture.temp.path().join("converted.db"), &[output]).unwrap();
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

fn release_pack(fixture: &Fixture) -> PathBuf {
    let scratch = fixture.temp.path().join("release-icons");
    let (_, red) = icon_png(&scratch, "red.png", 200);
    fixture.pack(
        "release",
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

fn release_options<'a>(
    fixture: &'a Fixture,
    packs: &'a [PathBuf],
    exclude: &'a [String],
    sign_command: Option<&'a str>,
    name: &str,
) -> release::ReleaseOptions<'a> {
    let work: &'a Path = Box::leak(
        fixture
            .temp
            .path()
            .join(format!("rwork-{name}"))
            .into_boxed_path(),
    );
    let out: &'a Path = Box::leak(
        fixture
            .temp
            .path()
            .join(format!("rout-{name}"))
            .into_boxed_path(),
    );
    let export: &'a Path = Box::leak(
        fixture
            .temp
            .path()
            .join(format!("rexport-{name}"))
            .into_boxed_path(),
    );
    release::ReleaseOptions {
        base_db: &fixture.base,
        packs,
        work_dir: work,
        output_dir: out,
        exclude,
        server_export: Some(export),
        sign_command,
        deep_verify: true,
        force: false,
    }
}

#[test]
fn release_reads_like_the_composed_database() {
    let fixture = Fixture::new();
    let packs = [release_pack(&fixture)];
    fixture.compose("full", &packs).unwrap();
    let exclude = [atlas::DEFAULT_SWF.to_owned()];
    let options = release_options(&fixture, &packs, &exclude, None, "plain");
    let manifest = release::release(&options).unwrap();
    assert_eq!(manifest.signature.status, "unsigned");
    assert_eq!(manifest.excluded.len(), 1);
    let out = options.output_dir;
    let base_len = fs::metadata(&fixture.base).unwrap().len();
    assert_eq!(manifest.tail.offset, base_len);
    assert_eq!(manifest.output.size, base_len + manifest.tail.size);
    assert!(manifest.plan_lines[0].starts_with(&format!(
        "region game/resources.db {base_len} 0 {} ",
        manifest.head.blob.size
    )));

    let overlaid = resources_db::headroom::OverlaidFile::open(
        &fixture.base,
        &out.join(release::HEAD),
        &out.join(release::TAIL),
    )
    .unwrap();
    let mut released = ResourceDb::from_reader(Box::new(overlaid)).unwrap();
    let mut composed = fixture.output("full");
    let mut base = ResourceDb::open(&fixture.base).unwrap();
    let paths: Vec<String> = composed
        .entries()
        .keys()
        .filter(|p| composed.is_file(p))
        .cloned()
        .collect();
    for path in &paths {
        let expected = if path == atlas::DEFAULT_SWF {
            base.read(path).unwrap()
        } else {
            composed.read(path).unwrap()
        };
        assert_eq!(released.read(path).unwrap(), expected, "{path}");
    }
    // Base bytes past the head are untouched.
    let base_bytes = fs::read(&fixture.base).unwrap();
    let mut whole = Vec::new();
    resources_db::headroom::OverlaidFile::open(
        &fixture.base,
        &out.join(release::HEAD),
        &out.join(release::TAIL),
    )
    .unwrap()
    .read_to_end(&mut whole)
    .unwrap();
    let head = manifest.head.blob.size as usize;
    assert_eq!(&whole[head..base_bytes.len()], &base_bytes[head..]);

    let content: Json = serde_json::from_slice(
        &fs::read(options.server_export.unwrap().join("content.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        content["items"],
        json!([{"config": "gameplay/items/armour/torso/new.options",
                "additional_model_slots": ["helmet", "mask"]}])
    );

    // Deterministic, and refuses to overwrite without --force.
    assert!(release::release(&options).is_err());
    let again = release::release(&release::ReleaseOptions {
        force: true,
        ..release_options(&fixture, &packs, &exclude, None, "plain")
    })
    .unwrap();
    assert_eq!(again.head.blob.sha256, manifest.head.blob.sha256);
    assert_eq!(again.tail.sha256, manifest.tail.sha256);
}

#[cfg(unix)]
fn signer(fixture: &Fixture, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = fixture.temp.path().join(name);
    fs::write(&path, format!("#!/bin/sh\nset -e\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

#[cfg(unix)]
#[test]
fn release_signs_only_the_signature_slot() {
    let fixture = Fixture::new();
    let packs = [release_pack(&fixture)];
    let exclude = [atlas::DEFAULT_SWF.to_owned()];
    let good = signer(
        &fixture,
        "good.sh",
        "printf 'S%.0s' $(seq 256) | dd of=\"$1\" bs=1 seek=\"$2\" conv=notrunc status=none",
    );
    let manifest = release::release(&release_options(
        &fixture,
        &packs,
        &exclude,
        Some(&good),
        "good",
    ))
    .unwrap();
    assert_eq!(manifest.signature.status, "signed");
    let head = fs::read(fixture.temp.path().join("rout-good").join(release::HEAD)).unwrap();
    let slot = manifest.signature.offset as usize;
    assert!(head[slot..slot + 256].iter().all(|b| *b == b'S'));
    let table = u32::from_le_bytes(head[20..24].try_into().unwrap()) as usize;
    assert_eq!(slot, 24 + table, "the slot follows the file table");

    let stray = signer(
        &fixture,
        "stray.sh",
        "printf 'X' | dd of=\"$1\" bs=1 seek=0 conv=notrunc status=none",
    );
    assert!(
        error_text(release::release(&release_options(
            &fixture,
            &packs,
            &exclude,
            Some(&stray),
            "stray"
        )))
        .contains("outside the signature slot")
    );
    let lazy = signer(&fixture, "lazy.sh", "true");
    assert!(
        error_text(release::release(&release_options(
            &fixture,
            &packs,
            &exclude,
            Some(&lazy),
            "lazy"
        )))
        .contains("left the signature slot empty")
    );
}

#[test]
fn lobby_region_keeps_size_crc_and_other_tags() {
    let fixture = Fixture::new();
    let packs = [release_pack(&fixture)];
    let mut base = ResourceDb::open(&fixture.base).unwrap();
    let movie = base.read(atlas::DEFAULT_SWF).unwrap();
    // The retail entry: the movie padded with room for new icons.
    let mut original = movie.clone();
    original.resize(movie.len() + 4096, 0x5A);
    let dir = fixture.temp.path();
    fs::write(dir.join("original.swf"), &original).unwrap();
    // What players have now: the same movie, differently padded.
    let mut current = movie.clone();
    current.resize(original.len(), 0);
    fs::write(dir.join("current.swf"), &current).unwrap();
    let options = release::LobbyRegionOptions {
        swf: &dir.join("current.swf"),
        original: &dir.join("original.swf"),
        packs: &packs,
        output: &dir.join("region.bin"),
        expect_width: Some(2 * atlas::CELL_WIDTH),
        base_db: None,
    };
    let region = release::lobby_region(&options).unwrap();
    let bytes = fs::read(dir.join("region.bin")).unwrap();
    assert_eq!(bytes.len(), original.len());
    assert_eq!(
        resources_db::pack::crc32(&bytes),
        resources_db::pack::crc32(&original)
    );
    assert_eq!(crc32fast::hash(&bytes), crc32fast::hash(&original));
    assert_eq!(region.cells, vec![(2, 3)]);
    assert_eq!(
        region.atlas_width,
        (2 * atlas::CELL_WIDTH, 3 * atlas::CELL_WIDTH)
    );
    let swf = atlas::Swf::parse(&bytes).unwrap();
    assert!(
        swf.same_except_bitmap(&atlas::Swf::parse(&movie).unwrap(), atlas::DEFAULT_SYMBOL)
            .unwrap()
    );
    let bitmap = swf.bitmap(atlas::DEFAULT_SYMBOL).unwrap();
    assert_eq!(bitmap.cell(2, 3).unwrap(), icon(200));
    assert_eq!(bitmap.cell(1, 1).unwrap(), icon(1));

    // Drawing again onto the result collides with the cell it drew.
    let again = release::LobbyRegionOptions {
        swf: &dir.join("region.bin"),
        expect_width: None,
        output: &dir.join("region2.bin"),
        ..options
    };
    assert!(error_text(release::lobby_region(&again)).contains("already occupied"));
    let narrow = release::LobbyRegionOptions {
        expect_width: Some(855),
        output: &dir.join("region3.bin"),
        ..options
    };
    assert!(error_text(release::lobby_region(&narrow)).contains("expected 855"));
}
