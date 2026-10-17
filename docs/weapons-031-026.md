# H&K 416A5 (and MSBS Radon) for v0.26g, from 0.31a2

The H&K 416A5 of Survarium 0.31a2 (2015), its 5.56x45 round and the EOTech
552 it aims through, as 0.26g items, in the content pack `weapons-hk416`.
0.31a2 keeps 0.26g's file formats (24-byte binary configs, the same meshes,
animations, textures and sounds), so this is a copy, not a conversion. An
earlier attempt from 0.69d0 (2022: other configs, meshes and PBR textures) is
superseded and removed.

| Item | ID | Config | Category |
| --- | --- | --- | --- |
| H&K 416A5 | 763 | `gameplay/items/weapons/assault/hk416.options` | 8 (assault rifle) |
| 5.56x45 | 841 | `gameplay/items/weapons/ammo/ammo_5.56x45.options` | 9 (ammo) |
| EOTech 552 | 819 | `gameplay/items/scopes/eotech_552.options` | 22 (scope, the H&K's fixed addon) |

The ids are 0.31a2's own. None is used by 0.26g, the Skhid patches 12 and 14
or revision 17's packs; keeping them keeps every donor reference (addons,
compatibility) as it is. The MSBS Radon (926, `msbs_carbine`) is in the
builder (`--port radon`) but not delivered yet.

## Build

```sh
# The 0.31a2 files the port reads, as one archive (list: port-manifest.json `files`).
cargo run --release --example pack-donor-tree -- DONOR_TREE donor031.db
cargo run --release --example build-weapons-026 -- --port hk416 --donor-db donor031.db \
  --target-db STOCK/resources.db --target-patch resources.db.patch_12 \
  --target-patch resources.db.patch_14 --output BUNDLE
vostok-port pack from-bundle --base-db STOCK/resources.db --base-patch ...12 --base-patch ...14 \
  --bundle BUNDLE --output PACK --id weapons-hk416 --version N
vostok-port pack check --pack PACK --base-db STOCK/resources.db --base-patch ...12 --base-patch ...14
```

## What is copied, and what 0.26g needs instead

* **Meshes, skeleton, textures.** Copied. 0.31's mesh chunk 110
  (4-byte statistics) is stripped; 0.26g has none.
* **Animations.** All twelve clips the H&K has in 0.31a2: show, hide, idle,
  shoot, reload (empty) and reload_tactic, for the item and for the hands.
  The hands clips drive the 70-bone player rig, identical in both builds.
  0.26g's reload picks the clip by slot parity: eight hands slots
  `[reload_tactic, reload] x 4`, item clips `[reload_tactic, reload]`.
  What every 0.31 rifle shares from `animations/single/weapons/base/` (aim,
  move, sprint, melee, grenade, recoil) is not in 0.26g; those states stay the
  native AK-12's, as for every 0.26g rifle.
* **State types.** 0.31 renumbers them (reload 313, grenade 315, melee 320);
  the config keeps the template's 0.26g types and takes the donor's clips.
  0.31's `data.type` is one higher (ammo 11, scope 6): the template's is kept.
* **Materials.** 0.31 adds effect keys (alpha cut-off, parallax) 0.26g's
  shaders do not take: each material is the native one of its base material
  (`ak_12`, `1p_69_giperon_grid`), stage by stage, with the donor's textures.
* **Sounds.** Collections and files copied; `raw_options` drop 0.31's `pitch`.
  The firing composite's reverb layer (`weapons/environments/*`, 0.31's
  per-location `sound_environment` zones, none on 0.26g maps) becomes the
  native AK-12's `akm_n` tails.
* **Config.** The native AK-12 config (states, melee, grenade, recoil, lobby
  poses) with the donor's model, clips, sounds, addon and the stats below;
  renamed fields: `health_modifier` -> `pain_health_modifier`,
  `stamina_regeneration_speed_modifier` -> `stamina_regenation_speed_modifier`.
* **Sights.** The 0.31 H&K has no iron sights (folded rear, no front); it
  aims through the EOTech 552, a fixed addon by 0.26g's scope mechanism (as
  the sniper rifles' scopes) with the weapon kept visible
  (`hide_weapon_on_aim` false, `fov_factor` and `near_plane_factor` 1).
* **Ammo.** 5.56x45 from the native 5.45x39 7N6M config with 0.31's
  values; the H&K takes it through `items_compatibility`.
* **Icons.** 0.31a2's own icons into `LobbyMenu_WeaponIcons` [7, 3] and
  `LobbyMenu_AmmoIcons` [16, 3] (both empty in stock and every Skhid patch).
  The bundle lists them in `icons/icons.json`; `pack from-bundle` adds one
  `atlas_icons_add` per atlas, and the composer draws all atlases of a movie
  into one copy of it. The kill-feed icon is the AK-74M's (31): 0.31's 63 is
  past 0.26g's table.
* **Texts.** The names are 0.31a2's; the descriptions are new (0.31a2 has
  none for these items), in English, the original, Russian, Ukrainian,
  German and Polish.

## Statistics (provisional, owner to confirm)

Sold by the Fringe Settlers at item level 9 (srp: rank 4), balanced against
their native level-8 to level-11 weapons.

| | H&K 416A5 | UMP45 | PP-2000 | SV-98 | Vepr | RPK |
| --- | --- | --- | --- | --- | --- | --- |
| Item level | 9 | 8 | 9 | 9 | 10 | 11 |
| Damage | 0.40 | 0.55 | 0.40 | 1.20 | 0.50 | 0.50 |
| Pierce | 0.30 | 0.25 | 0.20 | 0.50 | 0.30 | 0.30 |
| Stopping power | 0.15 | 0.40 | 0.30 | 0.60 | 0.15 | 0.15 |
| Rate of fire | 850 | 600 | 750 | 220 | 650 | 600 |
| Magazine | 30 | 25 | 20 | 10 | 30 | 45 |
| Weight | 3.5 | 2.5 | 1.4 | 6.2 | 3.1 | 5.0 |
| Reload | 3.0 | 3.7 | 2.8 | 5.0 | 3.167 | 3.167 |
| Aim time | 0.30 | 0.25 | 0.20 | 0.35 | 0.30 | 0.35 |
| Aim zoom | 1.2 | 1.1 | 1.1 | 1.0 | 1.1 | 1.2 |
| Dispersion | 0.20 | 0.30 | 0.30 | 0.05 | 0.20 | 0.15 |
| Damage distance | 20-50 | 15-40 | 15-40 | 30-100 | 20-50 | 30-50 |
| Recoil power | 1.0 | 0.6 | 0.6 | 1.0 | 0.8 | 0.8 |
| Drop weight | 70 | 80 | 70 | 70 | 60 | 50 |
| Price | 45 000 | 27 720 | 42 000 | 63 000 | 52 500 | 69 300 |

5.56x45: item level 9, drop weight 160, 30 a pack, 0.3 kg, 70 (srp sells it
by Edge at rank 4 at 70 / 35, as the PP-2000's 9x19 7N31).

## Not ported

* 0.31's shared `weapons/base/` clips (0.26g has its own per rifle).
* Suppressed sounds and the per-location reverbs.
* Skins.
