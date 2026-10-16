# H&K 416A5 and MSBS Radon for v0.26g (from 0.69d0)

Two assault rifles of Survarium 0.69d0 (2022) as 0.26g items:

| Item | ID | Config | Model |
| --- | --- | --- | --- |
| H&K 416A5 | 1820 | `gameplay/items/weapons/wpn_hk416_a5.options` | `weapons/assault/hk416_a5/hk416_a5` |
| MSBS Radon | 1821 | `gameplay/items/weapons/wpn_msbs_radon.options` | `weapons/assault/msbs_carbine/msbs_carbine` |

Neither rifle is in any build between 0.26g and 0.69d0 on hand (the H&K
first appears in the 0.29a notes). 0.69d keeps every item config inside
`gameplay/game.options` (latest copy in `resources.75.db`): `weapons.hk416`
(0.69d id 763) and `weapons.msbs_carbine` (926), with their default modules
and states. The builder takes art from the 0.69d archive and follows those
configs where 0.26g has the same mechanism.

## Build

```sh
# 1. The 0.69d files the rifles use, as one archive (from a duga-069d-to-026
#    reader over the 76 volumes; see the list in port-manifest.json `files`).
cargo run --release --example pack-donor-tree -- DONOR_TREE donor-weapons.db
# 2. Bundle, icons, bundle with icons (magick on PATH, NumPy + Pillow).
cargo run --release --example build-weapons-026 -- --donor-db donor-weapons.db \
  --target-db STOCK/resources.db --target-patch resources.db.patch_12 \
  --target-patch resources.db.patch_14 --output B0
scripts/render-weapons-026-icons.py --bundle B0 --lobby-swf <patch_14's lobby_menu.swf> \
  --output ICONS --gallery GALLERY
cargo run --release --example build-weapons-026 -- ... --icons-swf ICONS/lobby_menu.swf --output B
# 3. Pack over what players have.
vostok-port pack from-bundle --base-db STOCK/resources.db --base-patch ...12 --base-patch ...14 \
  --bundle B --output PACK --id weapons-hk416-radon --version N
```

## What converts, and how

* **Meshes.** Both builds draw weapons as one-bone skinned meshes (type 44).
  0.69d's vertices are 32 bytes (position, bone, three basis floats, half2 UV),
  0.26g's 36 (position, bone, three packed basis words, float2 UV). A basis
  float is `(x << 16 | y << 8 | z) / 2^24`, the bytes 0.26g packs (normal top
  byte 127, as every stock weapon; tangent/binormal 255, 0.69d stores none).
  0.69d's chunks 5/6 (used bones, per-bone boxes) and 110/114 (statistics) go.
  Converting the 0.69d AK-12 gives the same render as 0.26g's own.
* **Attachments.** 0.69d places modules at the rig's locators from the item
  config; 0.26g has no attachment system for rifles (scopes exist only as
  fixed sniper `addons`). The default modules are baked in as surfaces bound
  to their socket bone, at `bind_translation(bone) + R_y(pi) * locator` (the
  rigs keep their meshes a half turn about Y from their bind frame; checked
  exactly on the magazine module against the rig's own magazine surface):
  the muzzle brake, the rear and front iron sights (raised: 0.26g rifles aim
  through irons), and the magazine (the rig's magazine surface, the same mesh
  as the module). H&K: its own sights. Radon: the Troy micro battle sights,
  its 0.69d default (`troy_micronattlesight_*`).
* **Locators.** 0.26g's contract (every stock rifle): `aim`, `barrel_point`,
  `scope_point`, `handguard_point` and 32 `*_idle` hand locators. `barrel_point`
  is the brake's far end (0.69d's module "overrides fire_point"); `aim`
  rises to the iron sights' line, measured on the sight meshes from behind
  (aperture centre and post tip co-witness within 0.2 mm: H&K y = 0.1323, Radon
  0.1104; 0.69d's default sits on the rail, 3 cm lower).
* **Animations.** Same memory image except the event-channel headers (12 bytes
  in 0.69d, 16 in 0.26g). Offsets are relative to the slot that holds them, so
  bone slots shift by 4 x events and event header i by 4 x (events - i).
  The AK-12's clips both builds ship convert to 0.26g's own bytes (test
  `shared_ak12_clips_convert_to_the_026g_bytes`, needs `VOSTOK_026G_DB` and
  `VOSTOK_069D_WEAPONS_DB`). Every clip is ported: show, hide, idle, shoot,
  reload (empty) and reload_tactic, for the item and the hands.
* **Rig.** The hands clips drive 0.26g's 70-bone player rig, bone for bone in
  the same channel order (checked per clip against the AK-12's). 0.26g plays
  the hands clips in first and third person alike over the shared
  `rifles.user_animations` (aim, move, sprint, crouch, jump), as for every
  native rifle; 0.69d's equivalent shared set (`weapons/base/...`) is not
  per-weapon and stays 0.26g's.
* **Reloads.** 0.26g picks the chambered or the empty clip by slot parity:
  eight hands slots `[reload_tactic, reload] x 4` and item clips
  `[reload_tactic, reload]`, as the native Kiparis and TT-33 list them.
  Sounds are cued by each item clip's `sound_events` channel (0xff: the next
  sound; a number: that sound), the same in both builds; the lists are
  0.69d's own. There is one `reload_time` (3 s, 0.69d's empty reload; its
  2.2 s tactical time has no 0.26g field).
* **Textures.** Same DDS layout. 0.69d is metalness PBR, so bare metal is
  bright in its albedo. The diffuse is fitted per pixel on the seven
  assault-rifle texture sets both builds ship (their UV layouts match):
  `0.514 * (albedo * (1 - 0.8 * metalness)) ^ 0.75`, re-encoded DXT1. Gloss
  (`_smap`, 0.26g's `texture_roughness` is gloss) is 0.36 x smoothness and
  Fresnel 0.162 x metalness^0.388, both remapped in the DXT endpoints. Normal
  maps are copied. Materials are the native AK-12 material with these four maps.
* **Sounds.** Collections and their files are copied where 0.26g lacks them
  (H&K reload and fire layers, 0.69d's `akm_n/akmn_*`). The firing composites
  keep the donor's own layers; their 0.69d reverb layer
  (`weapons/environments/*`: per-location `sound_environment` items, which no
  0.26g map defines) becomes the native rifles' tails (the AK-12's first-person
  `akm_n` tails, the Vityaz's and AKM's third-person distance and forest
  layers). Show and hide play 0.69d's `show_hide_aimed_states/show_state(_m)`.
  Suppressed variants are not ported (0.26g has no suppressors).
* **Item configs.** The native AK-12's config (states, melee, grenade, recoil,
  lobby poses) with the ported clips, sounds, model and the stats below.
  Ammo: 5.45x39 7N10 and 7N22 (217, 331), the AK-74M's; 0.26g has no 5.56x45.
* **Icons.** Rendered from the converted meshes (right side, muzzle right),
  exposure matched to native rifle icons, into the weapon atlas
  `LobbyMenu_WeaponIcons` cells [7, 3] and [8, 3] (empty in stock and every
  Skhid patch). `pack from-bundle` now detects which icon atlas a bundle
  draws into. The kill-feed icon is a native rifle's (AK-12, AK-74M).

## Statistics (provisional, owner to confirm)

Balanced against the native 0.26g rifles; 0.69d's own values are in brackets
(0.69d's scale: its AK-12 deals 0.39, 0.26g's 0.55).

| | H&K 416A5 | MSBS Radon | AK-74M | A-545 | AK-12 |
| --- | --- | --- | --- | --- | --- |
| Faction | Fringe Settlers [Black Market] | Fringe Settlers [Scavengers] | Renaissance | Renaissance | Renaissance |
| Item level | 13 [4] | 13 [4] | 10 | 14 | 15 [5] |
| Damage | 0.50 [0.35] | 0.53 [0.38] | 0.50 | 0.50 | 0.55 |
| Pierce | 0.35 | 0.30 | 0.30 | 0.30 | 0.45 |
| Rate of fire | 850 [850] | 750 [750] | 650 | 900 | 650 |
| Fire modes | auto, single | auto, single, burst 3 | auto, single | auto, single, burst | auto, single, burst |
| Weight | 3.5 [3.5] | 4.0 [4.0] | 3.4 | 3.3 | 3.4 |
| Reload | 3.0 [3 / 2.2] | 3.0 [3 / 2.2] | 3.167 | 3.167 | 2.8 |
| Dispersion | 0.18 | 0.20 | 0.20 | 0.20 | 0.20 |
| Aim zoom | 1.2 [1.2] | 1.2 [1.2] | 1.1 | 1.1 | 1.1 |
| Drop weight | 30 | 30 | 60 | 20 | 10 |
| Price | 82 000 | 80 000 | 47 500 | 85 500 | 115 000 |
| Ammo | 5.45 7N10, 7N22 | 5.45 7N10, 7N22 | 5.45 7N10, 7N22 | 5.45 7N10, 7N22 | 6.5x39 |

Both 0.69d factions sell only items up to level 5 in 0.26g (srp sells an item
at `item_level - 5` for Renaissance and Fringe Settlers, at `item_level`
capped at the top rank for the others), so a level-13 rifle there would come
at their top rank, far too early. Fringe Settlers sell them at rank 8 and
have no assault rifle between levels 11 and 15.

## Not ported

* The 0.69d attachment system and its optional modules (scopes, collimators,
  suppressors, grips, lasers): no 0.26g mechanism for rifles.
* The skins (St Patrick's 2018, Summer 2021), suppressed sounds.
* 0.69d's tactical reload time, `aim_recoil` model and per-location reverbs.
* Inspect, jam, sprint and aim clips: 0.69d has none per weapon either
  (sprint and aim come from the shared set).
