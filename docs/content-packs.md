# Content packs (v0.26g)

A content pack is a change to the v0.26g client's resources. It can change
resources that already exist and add new ones, including items and maps.
Packs compose: the porter parses each shared base file once and applies
every pack's edits to it in order, then ships the result as a patch
(`pack patch`): the files that are new or differ from the base, whole, packed
as `resources.db.patch_<n>`. Legacy bundles each carried a whole copy of
`db_static_dictionaries`, the localizations and `lobby_menu.swf`, so two
bundles overwrote each other's copies. Packs edit those files field by field,
so this no longer happens.

The archive code (reader, the overlay of an archive and its patches, the
writer) is skhid-srp's `resources-db` crate.

```
my-pack/
  pack.json
  files/<resource path>     # sources of add_file / replace_file (default)
  icons/*.png               # atlas icons (any path; named in pack.json)
```

## pack.json

```json
{
  "format_version": 1,
  "id": "bandit",                  // [a-z0-9._-]
  "version": "8",
  "target_build": "0.26g",
  "requires": ["duty-exo"],        // must be composed earlier in the same run
  "base_db_sha256": "c74078be…",   // optional: pin the stock archive
  "operations": [ … ],
  "server": { … },                 // passed through untouched (see below)
  "notes": [ … ]                   // provenance, ignored by tools
}
```

## Operations

Operations run in order: pack by pack, in the order given on the command
line, then in each pack's listed order.

| op | fields | rules |
| --- | --- | --- |
| `add_file` | `path`, `source`? | The path must not exist in the base or be added by another pack. `source` defaults to `files/<path>`. |
| `replace_file` | `path`, `source`?, `base_sha256` | The base file must hash to `base_sha256` (drift check). It conflicts with any other add or replace of the same path. |
| `config_set` | `path`, `values: {"dotted.path": value}` | Sets fields of a binary config. A numeric segment indexes an array. A missing final named key is created. Two packs may set a field, or a parent/child of it, only to the same value. |
| `config_append` | `path`, `array_path`, `values: [..]` | Appends to an indexed table. Appends from several packs accumulate in order. |
| `dictionary_add` | `table`, `entries: [{..}]`, `id_field`? | Appends rows to an indexed table of `gameplay/db_static_dictionaries`. The id field defaults to `dict_id` for `items_dict` and `id` otherwise (`maps_dict`, `shop_bundles_dict`, …). An id already in the base is an error, unless the base row is identical (then it is a no-op). Two packs adding the same id is a conflict. |
| `localization_add` | `strings: {key: text}`, `languages`? | Adds to `localization/<lang>/localization` `strings`. Without `languages` it applies to every language in the base. A key that exists with different text, in the base or in another pack, is a conflict. Identical text is a no-op. |
| `atlas_icons_add` | `icons: [{row, col, source, premultiplied?}]`, `swf`?, `symbol`? | Draws 171x83 PNG icons into empty cells of the `LobbyMenu_EquipmentIcons` bitmap in `flash_movies/lobby_menu.swf`. Cells are 1-based `[row, col]` on a 171x91 grid, the same as an item's `ui_desc.icon`. The bitmap widens to new columns; existing cells never move. An occupied base cell, or two packs drawing the same cell, is a conflict, unless the icon gives `replaces_sha256`: the SHA-256 of the base cell's pixels (an icon an earlier patch drew); the base cell must then be exactly that one. PNGs hold straight alpha unless `premultiplied: true`. |

An edit of a file that another pack adds or replaces is allowed only when the
editing pack `requires` that pack.

### Config values

Plain JSON values map by shape: `true`, `3` (int), `3.0` (float), `"s"`, and
arrays (indexed tables). If the existing field is a float, an integer literal
becomes a float. A 2- to 4-number array becomes a float vector when the field
already is one. For anything new or ambiguous, use a tag:
`{"int": 1}`, `{"float": 1}`, `{"bool": true}`, `{"string": ""}`,
`{"float2"|"float3"|"float4": [..]}`, `{"array": [..]}`,
`{"table": {"name": value}}`. `dictionary_add` entries are plain objects
whose field types follow the table's existing rows.

Example: rebalance a stock item and add a map.

```json
{"op": "config_set", "path": "gameplay/items/armour/torso/blackmarket_torso_1.options",
 "values": {"parameters.armor": 0.35, "hit_params.torso.hit_types.ranged.armor": 0.3}}
{"op": "add_file", "path": "gameplay/game_modes/v011a_level_05_day_2.options"}
{"op": "dictionary_add", "table": "maps_dict",
 "entries": [{"id": 194, "cfg_mode_name": "v011a_level_05_day_2.options"}]}
```

## Commands

```
vostok-port pack check   --pack A [--pack B …] [--base-db DB [--base-patch P …]]
vostok-port pack patch   --base-db DB [--base-patch P …] --pack A [--pack B …] \
                         [--replace ARCHIVE_PATH=FILE …] --out-dir DIR \
                         [--archive resources.db.patch_N] [--force]
vostok-port pack verify  --db DB [--patch P …] --pack A --pack B …
vostok-port pack compose --base-db DB --pack A --pack B … --work-dir W \
                         --output-db OUT [--server-export DIR] [--force]
vostok-port pack from-bundle --base-db DB [--base-patch P …] [--build-base-db BUILT_ON] \
                         --bundle BUNDLE --output PACK --id ID --version V
```

`--base-patch` and `--patch` name `resources.db.patch_<n>` files laid over
the database as the game mounts them: in ascending n whatever their order on
the command line, a later archive's file winning. A file of another name is
refused (the game mounts none).

Without `--base-db`, `check` validates the manifests, sources and cross-pack
conflicts. With it, `check` also runs every base precondition: drift, ids,
strings and cells, against the database and its patches. `verify` confirms
that the database, with its patches over it, carries every operation of the
packs.

`compose` writes a whole database (local runs; players get a patch). It also
validates the effective overlay (`validate_effective_overlay`, in
`W/overlay`) and re-reads every touched resource. It writes `OUT.packs.json`,
the install record: pack ids, versions, manifest SHA-256s, base and output
identity, added/replaced/edited paths and `server` sections. It prints the
output SHA-256. The same inputs produce a byte-identical database.

`--server-export DIR` writes the effective server-side content under resource
paths, so the server never interprets operations:

* `gameplay/db_static_dictionaries`;
* the config of every `items_dict` row that is new or whose config changed;
* every added or changed `gameplay/game_modes/*.options`;
* `gameplay/items/modifications/modifications.options` and
  `gameplay/drops/drops.options` if changed;
* `content.json`: the install record, with `packs[]` in compose order (`id`,
  `version`, `manifest_sha256`, `server`) and `server` keyed by pack id.

### Server section

`server` is opaque to the porter. The srp-026g loader reads:

```json
{"items": [{"dict_id": 1810, "sellers": [[2, 2]]}],
 "maps":  [{"id": 180, "respawn_points": …, "container_teams": …}]}
```

`sellers` is a list of `[faction_id, reputation_level]` pairs and overrides
the item-level shop rule. Other item fields are ignored. In `maps`,
`respawn_points` and `container_teams` are optional.

## Converting legacy bundles

`from-bundle` diffs every overlay file against `--base-db`, the base the pack
will be composed onto. Files absent from that base become `add_file`.
Changed shared files become entry- and field-level operations:

* dictionary rows become `dictionary_add`, or `config_set` for a changed
  existing row;
* new strings become `localization_add`; one operation covers strings that
  are identical in all languages, and the rest get one per language;
* other binary configs become `config_set`/`config_append`, but only when
  applying them reproduces the bundle's file exactly;
* changed atlas cells become `atlas_icons_add`, but only when the SWF is
  otherwise unchanged (from the base, or from the build base when an
  earlier patch changed the movie since); a cell an earlier patch drew that
  the bundle's own item shows is redrawn with its `replaces_sha256`;
* everything else becomes `replace_file` with the base hash.

`equipment.json` becomes `server.items`, and the manifest's `maps` ids become
`server.maps`.

If the bundle was built on a database that already held other ports, pass
that database as `--build-base-db`. A row, string or icon is then kept only
if the bundle owns it: it differs from the build base, or the bundle's own
files reference it. Ownership evidence is an item's `cfg_name`, a map's
`cfg_mode_name`, a string key used in a config, or the bundle's
`ui_desc.icon` cells. Files that the bundle's configs reference, and that
exist in the build base but not in the target base, are carried in as
`add_file`. The Duty bundle needs this for its reused materials. A file
the bundle ships exactly as its build base has it is not the bundle's, and
is left as the target base has it.

For a release, `--base-patch` lays the patches players have over
`--base-db`, as `pack patch` does: a bundle built on the stock database then
becomes a pack of what it changes over those patches (its released files
`replace_file` against the released bytes, its released dictionary row a
`config_set`, its released icon a redraw). The
conversion records every dropped or carried item in `notes`.

Removals cannot be expressed: a deleted dictionary row, a removed config key
or a cleared atlas cell is noted but not carried over. A structural config
change falls back to `replace_file`.

## Patches for players

The players' 0.26g game (Skhid client revision 14 on) mounts the stock
`resources.db` and then every `resources.db.patch_<n>` beside it, n from 1 to
255 ascending: a later archive's file replaces an earlier one's, new files
are added, folders merge, and nothing is taken away. So a release of content
is one new patch:

```
vostok-port pack patch --base-db RETAIL/resources.db \
    [--base-patch RETAIL/resources.db.patch_14 …] \
    --pack A --pack B … [--replace ARCHIVE_PATH=FILE …] \
    --out-dir patch15 [--archive resources.db.patch_15] [--force]
```

* The base is the stock database with the earlier patches over it, as the
  game mounts them. The packs compose over it in the given order; an edit of
  a shared file (`dictionary_add`, `localization_add`, `atlas_icons_add`, …)
  lands on the base's current copy, the earlier patches' if they have one.
* `--replace ARCHIVE_PATH=FILE` gives a base file the bytes players have now
  (the server's own menus) before the packs apply; a pack's edits of that
  file (the lobby movie's `atlas_icons_add`) land on those bytes. A pack may
  not add or replace a replaced file whole.
* `--out-dir` gets every file the composition makes that is new or differs
  from the base, at its archive path, whole; a file the base already has as
  it is (a replacement equal to it) is left out. The folder is validated as an
  overlay of the base (`validate_effective_overlay`). Nothing to patch is an
  error.
* `--archive` packs the folder with the crate's writer
  (`resources_db::pack::pack`: deterministic, a zeroed signature slot after
  the table, payloads from the next 2048-byte boundary), reads it back, then
  reads the game's view (base, earlier patches, the new patch) and checks
  every composed file and every pack operation there. The name must be
  `resources.db.patch_<n>` with n above every base patch. skhid-srp's
  `srp-game-data pack --dir DIR` gives the same bytes from the same folder.
* `DIR.json`, beside the folder, is the patch's record: the base and the
  earlier patches (size, SHA-256), the packs with their versions, the
  replacements, every file of the patch (size, SHA-256, `add` or
  `override`), the files left out as unchanged, and the archive.
* An existing folder, record or archive is refused without `--force`. The
  same packs, replacements and base give the same bytes.

The server builds its game data from the same archives (skhid-srp's
`srp-game-data build`), so it needs no export from the porter.

## Runtime requirement

A patch needs the client that mounts patches (Skhid revision 14 on). A whole
database from `compose` or `apply` differs from retail and needs the
resource signature/hash check bypassed in the executable:
`scripts/run-026-map.py` applies `tools/patch-026-map-resources.py`, which
skips the check at VA 0x135d3. That patch is client-side and outside this
tool.
