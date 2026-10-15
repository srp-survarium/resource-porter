# Content packs (v0.26g)

A content pack is a patch for the v0.26g client's `resources.db`. It can
change resources that already exist and add new ones, including items and
maps. Packs compose: `pack compose` parses each shared base file once and
applies every pack's edits to it in order. It then writes one overlay through
the non-lossy archive overlay writer (`resources_db::overlay`). Legacy bundles
each carried a whole copy of `db_static_dictionaries`, the localizations and
`lobby_menu.swf`, so two bundles overwrote each other's copies. Packs edit
those files field by field, so this no longer happens.

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
  "base_db_sha256": "c74078be…",   // optional: pin the whole base archive
  "materialize_hard_links": [],    // optional: base hard links to detach
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
| `atlas_icons_add` | `icons: [{row, col, source, premultiplied?}]`, `swf`?, `symbol`? | Draws 171x83 PNG icons into empty cells of the `LobbyMenu_EquipmentIcons` bitmap in `flash_movies/lobby_menu.swf`. Cells are 1-based `[row, col]` on a 171x91 grid, the same as an item's `ui_desc.icon`. The bitmap widens to new columns; existing cells never move. An occupied base cell, or two packs drawing the same cell, is a conflict. PNGs hold straight alpha unless `premultiplied: true`. |

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
vostok-port pack check   --pack A [--pack B …] [--base-db DB]
vostok-port pack compose --base-db DB --pack A --pack B … --work-dir W \
                         --output-db OUT [--server-export DIR] [--force]
vostok-port pack verify  --db OUT --pack A --pack B …
vostok-port pack from-bundle --base-db DB [--build-base-db BUILT_ON] \
                         --bundle BUNDLE --output PACK --id ID --version V
```

Without `--base-db`, `check` validates the manifests, sources and cross-pack
conflicts. With it, `check` also runs every base precondition: drift, ids,
strings and cells. `compose` also validates the effective overlay
(`validate_effective_overlay`) and re-reads every touched resource. It writes
`OUT.packs.json`, the install record: pack ids, versions, manifest SHA-256s,
base and output identity, added/replaced/edited paths and `server` sections.
It prints the output SHA-256. The same inputs produce a byte-identical
database.

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
  otherwise unchanged;
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
`add_file`. The Duty bundle needs this for its reused materials. The
conversion records every dropped or carried item in `notes`.

Removals cannot be expressed: a deleted dictionary row, a removed config key
or a cleared atlas cell is noted but not carried over. A structural config
change falls back to `replace_file`.

## Releases to installed clients

`compose` writes a whole new database, and a growing file table shifts every
payload, so the result differs from retail across the whole 6.3 GB file.
`pack release` instead produces what an installed client needs on top of
retail:

```
vostok-port pack release --base-db RETAIL --pack A --pack B … --work-dir W \
    --output-dir OUT [--exclude flash_movies/lobby_menu.swf] \
    [--server-export DIR] [--sign-command "CMD …"] [--deep-verify] [--force]
vostok-port pack lobby-region --swf CURRENT_LOBBY --original RETAIL_LOBBY \
    --pack A --pack B … --output OUT/lobby_menu-region.bin [--base-db RETAIL]
```

* `resources-head.bin` replaces the same-size range `[0, H)`: the header,
  the larger file table, the 256-byte signature slot right after the table
  (`[24 + table_size, +256)`) and zeros. Every retail payload keeps its
  offset, except the few that started inside the space the table now needs:
  they move to the tail. `H` is the end of the last moved payload.
* `resources-tail.bin` is appended at the retail end: new and changed
  payloads in overlay order, then the moved payloads.
* `release.json` records sizes and SHA-256s of base, head (and the base's
  bytes it replaces), tail and the resulting archive, the moved payloads,
  the signature slot and the updater plan lines (`region … resources_head`,
  `tail … resources_tail resources_head`), and `grow.json`, the same head and
  tail in the form skhid-srp's `make_client_026g.py --resource-grow` reads.
* Nothing writes a whole archive: verification reads retail + head + tail as
  one virtual file (`resources_db::headroom::OverlaidFile`). It checks every
  composed resource, every pack operation (`verify`), and that every other
  retail entry keeps its offset and bytes (`--deep-verify` compares the bytes
  of all of them).
* `--exclude` keeps a composed path out of the archive. The lobby movie ships
  as its own updater region instead: `pack lobby-region` draws the packs'
  atlas icons into the movie players have now, checks that nothing but the
  atlas changed, and pads it to the retail entry's size with four trailing
  bytes that keep its CRC, so the file table entry stays the retail one.
* `--sign-command` runs last as `<command words…> <head> <slot offset>` and
  may write only the slot. `scripts/sign-release-head.py --signer
  <skhid-srp>/srp-026g/client-patches/sign_resources_026g.py` signs with the
  Skhid key (read from its default location, never copied). Without a signer
  the release is recorded as `unsigned`.

## Runtime requirement

A client whose `resources.db` differs from retail needs the resource
signature/hash check bypassed in the executable. `scripts/run-026-map.py`
applies `tools/patch-026-map-resources.py`, which skips the check at
VA 0x135d3. That patch is client-side and outside this tool.
