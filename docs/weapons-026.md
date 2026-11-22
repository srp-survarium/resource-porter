# Weapons of later builds for v0.26g

`vostok-port port-weapons` ports guns from a later Survarium build to 0.26g from a port manifest
(`ports/weapons-*.json`); `pack from-bundle` makes the content pack of its bundle;
`compare-weapons` checks a bundle against a patch players have; `render-weapon` draws a model's
side view. The code: `src/weapons_026/` (its module docs say each rule and why).

## What a port does

| part | how |
|---|---|
| files | everything a gun's config names (model, skeleton, clips, sounds, materials, textures), carried from the donor unless the target (the stock db and the players' patches) has the path |
| clips | the later builds write an event channel's header in 12 bytes, 0.26g in 16 (`clips.rs`) |
| meshes | 0.31's statistics chunk 110 dropped; otherwise 0.26g's layout |
| materials | the donor's on a native material of the same base material's effect keys |
| sound options | the donor's, less the keys 0.26g's exe does not read (`pitch`, `add_rms`) |
| firing sounds | the reverb layer `weapons/environments/weapons_<view>_view*` (per-location reverbs 0.26g's maps have no zones for) becomes the outdoor collection of its kind (patch 19's choice) |
| item config | the donor's on a native template's keys and the natives' schema (a key some native weapon has is kept); the build's state types, renamed and rescaled keys (`donor.rs` `Rules`); a state or part that plays the later builds' shared `weapons/base` clips is the template's; durability, repair, modification mask and melee are 0.26g's (`config.rs` `NATIVE_PATHS`); then the manifest's `set`, icons and the card |
| dictionary | a row like the template's (id, name, config, price), an `items_compatibility` row `compatibility_<gun>_<round>` a round |
| icons | the lobby cell (`icons/`, `atlas_icons_add`) and the kill feed image (`kill-feed/CombatLogWeapon<n>.png`, RGBA) for skhid-srp's hud |
| texts | `texts.tsv`: the keys the configs name with the live and donor texts; the texts themselves go into skhid-srp's localization tables |

## Adding a gun

1. **The donor:** a `resources.db`, or a split build's volumes listed in a `.split` file (one path
   a line, relative to the list). A build in 0.26g's formats with configs as files (0.29a3,
   0.31a2) is `ArchiveDonor`; another build is an `impl Donor` of its own (its readers apart from
   the core) and a `DonorBuild` variant with its `Rules`.
2. **The ids and cells:** a free dict id, a free `LobbyMenu_WeaponIcons` cell (the donor's own if
   free) and the kill feed icon number (the donor's: skhid-srp's hud keeps a bitmap a number).
   Claim them beside the other ports.
3. **The template:** a stock config of the same class (Val, AK-12, RPD, ...): its keys, its
   dictionary row, its native systems.
4. **The balance:** `scripts/weapons/peer_scale.py` scales the donor's stats by a peer both builds
   ship (live peer x donor / donor peer); add `faction`, `item_level`, `match_making_level` and
   `drop_weight` by the live peers of the class and calibre; put the result in `set`.
5. **The manifest:** a `weapons` entry (see `ports/weapons-031a2-batch1.json`): `name`, `dict_id`,
   `donor_item`, `config` (`gameplay/items/weapons/wpn_<name>.options`), `template`, `base_cost`,
   `ammo` (0.26g ids), `set`, `lobby_icon` (`row`, `col`, `donor_png`), `kill_feed` (`icon`,
   `donor_png`), `notes`. `copy_of_template` for a skin of a stock weapon (keeps the stock states);
   `file_values` to fix a donor file (a material's key, a model's empty material, an aim
   locator's `position` / `rotation`: a list of numbers stays the file's float3). A new round
   is an `ammo_items` entry on a native round's config; its card's bullets in a pack (prop 16)
   and weight (prop 15) are written from its own `clip_size` and `clip_weight`.
6. **Build, pack, look:**

```sh
vostok-port port-weapons --manifest ports/weapons-X.json --donor DONOR.split \
  --target-db STOCK/resources.db --target-patch resources.db.patch_12 ... --output BUNDLE
vostok-port pack from-bundle --base-db STOCK/resources.db --base-patch ... \
  --bundle BUNDLE --output PACK --id weapons-X --version 1
vostok-port render-weapon --model BUNDLE/overlay/models/weapons/G/G.skinned_model --output G.png
```

The build prints what it left out or kept native per config; read it.

## Modules (scopes, flash hiders, muzzle brakes, grips)

`modules.rs`: a module's item config from a native template of its kind (a scope's idle and
aimed models; a muzzle device's and a grip's kit keys, `data.srp_<slot>_model` and
`data.srp_locator`), its placement at the gun's locator of its kind (`muzzle_point`,
`picatinny_grip_point`, `picatinny_scope_point`: the donor's own, 0.69d0's where our mesh is
retail's), the removal rule for a device built into a gun's mesh (`cut_builtin`: our triangles
missing from the 0.69d0 base mesh inside the placed stock module), and a side render for checking.
No suppressors: a module whose model names one is refused.

## Tactical reloads of live guns

`vostok-port port-tactical-reload` (`tactical.rs`; `ports/tactical-reload-029a3.json`, then
`ports/tactical-reload-069d.json` for the guns 0.29a3 lacks): live configs whose reload is 0.26g's
one-clip one (303) take a donor's two-clip reload (314), as the live vz. 58 and the other 0.29a3
ports reload: the donor's tactical and empty clips (a pair one build made; the engine plays both at
the tactical clip's speed over `reload_time`, which stays), its sound lists, the clips at
`<path>_<donor>` where 0.26g has another clip of that name. No exe change. A gun is refused unless
its clips drive 0.26g's rigs and start and end with the gun's parts where 0.26g has them (0.69d0
moved some guns in the hands: its hands' `Weapon` bone carries what 0.26g's clips keep in the gun's
`offset` bone). The 0.69d0 donor: the reload states' files, `scripts/weapons-069d-donor.py
--reload`. skhid's packs: `tactical-clips-029`, `tactical-clips-069`.

## Newer animations of a live gun

`vostok-port port-animation-set` (`animation_set.rs`, `ports/animations-*.json`): a live config's
listed states take a later build's clips of the same rig, under a renamed weapon folder, and its
models the donor model's listed hand points (`*_idle`, bone index by our bind pose). Refused unless
the clips drive 0.26g's rigs, the sound lists their events index are ours (or none live), and the
new idle holds the gun where ours does. skhid's pack: `pkp-animations-069` (the PKP, 0.69d0).

## The check against patch 19

`ports/weapons-029a3-patch19.json` is patch 19's 33 weapons read back as a manifest
(`scripts/weapons/patch19_manifest.py`: the porter's decisions, its ids, prices and balance, from
patch 19); the tool ports them again from the 0.29a3 donor and `compare-weapons` compares the
bundle with patch 19 file by file. Result (2026-11-01): 0 unexplained; of 2,451 files 2,086 identical,
150 equivalent (102 clips differ only in the words the engine overwrites on load: patch 19 left the
12-byte offsets unshifted; 48 configs hold the same values encoded otherwise), and 215 differ, each
explained in
`ports/weapons-029a3-patch19.known.tsv` with the side that is right. Where the tool is right:
0.26g's melee (patch 19 wrote k_damage 2 for the natives' 1.5). Where patch 19 is right: a bolt
action's card rate, the Saiga skin's missing material, the shotguns' template. The rest are
choices or invisible (sound keys 0.26g does not read, int against float, rounding).

The test `the_tool_gives_patch_19s_weapons` runs it with `VOSTOK_029A3_DONOR` (the donor's
`.split`), `VOSTOK_026G_DB` and `VOSTOK_026G_PATCHES` set (the 0.29a3 donor: `Survarium29a` of
the owner's `Survarium_archives.zip`).
