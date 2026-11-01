# Weapons of 0.46c2 for v0.26g

Ten guns of Survarium 0.46c2 (April 2017) as 0.26g items, in the content pack `weapons-046`
(skhid-srp's `docs/notes/weapons-046.md` has their balance and the owner's rules): the H&K MP5, SVU,
L85A2, Fort-401, Pancor Jackhammer, CZ 75, MP-443 Grach, Beretta 93R, Glock 17 and Colt M1911.
The port is data, `ports/weapons-046c2.json`, built by the shared weapon core (`port-weapons`);
this build's part is its donor, `src/weapons_026/donor_046.rs`.

## What 0.46c2 changes, and what the donor serves instead

0.46c2 sits between 0.31a2 and 0.69d0: its weapon meshes are complete (no modules yet) and in
0.26g's vertex layout, but its files are written as the later builds write them.

| 0.46c2 | 0.26g | Done by |
|---|---|---|
| 36 `resources.NN.db` volumes, the last its patches | one archive and patches | `Donor046::open` (the game folder or a `.split`) |
| item configs inside `gameplay/game.options` (`weapons.<name>`) | a config file a weapon | `item_config` |
| every config in 16-byte value records; integral floats kept as ints | 24-byte records | `read` (re-encoded); the core conforms to the template's types |
| clip event headers of 12 bytes | 16 (a pointer slot the engine fills) | `src/clip_026.rs`, each bone curve and event checked after |
| mesh chunks 110 (4 bytes) and 113 (72 bytes) after 3 and 4 | 3 and 4 only | `mesh_chunks` |
| locators `fire_point`, `muzzle_point`, `picatini_scope_point`; `barrel_point` on a few models is a barrel module's mount, mid-gun | the exe reads `barrel_point` (the muzzle: within 2.5 cm of 0.46c2's `fire_point` on the AK-12 and H&K 416) and `scope_point` | `add_locators`: `barrel_point` at `fire_point`, 0.46c2's own kept as `barrel_mount_point` |
| lobby icons 128x128, drawn diagonally, the gun's right side | 171x83 cells, level, muzzle right | `lobby_cell`: turned by the silhouette's main axis (a pistol by 40 degrees, its grip pulls the axis), mirrored, fitted |
| state types above 311 one lower (reload 313, grenade 315, melee 320) | 314, 316, 321 | the core's rules, as 0.31a2's |

Checked against 0.26g (the tests, with `VOSTOK_026G_DB` and `VOSTOK_046C2_GAME`): the AK-12 clips
0.46c2 did not re-animate, converted, are 0.26g's own bytes (runtime slots aside); the MP5's surface
and locators read as 0.26g's.

## Build

```sh
L="--target-patch .../resources.db.patch_12 ..."   # every patch players have, in order
vostok-port port-weapons --manifest ports/weapons-046c2.json \
  --donor <0.46c2 game folder> --target-db <stock 0.26g resources.db> $L --output BUNDLE
vostok-port pack from-bundle --base-db <stock> ${L//--target-patch/--base-patch} \
  --bundle BUNDLE --output PACK --id weapons-046 --version <n>
```

## Not ported

- Premium and skin copies; the `attaches`, `upgrade_possibilities` and `overrides` tables (0.46c2's
  attachment system; 0.26g has none).
- 0.46c2's shared `weapons/base/*` clips (melee, grenade, lobby poses, recoil): the template's.
