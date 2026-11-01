# Weapons of 0.69d0 for v0.26g

`weapons_069d` builds 0.69d0 weapons as 0.26g items: a bundle (`overlay/`,
`port-manifest.json`, `equipment.json`, `attach-points.json`) that `pack from-bundle`
turns into a content pack. The guns are a table (`GUNS`): the AKMSU, FAMAS, H&K G3SG/1,
Mauser M.712, KS-23M (with its 23x75R ammo) and PPSH-41 "Drum" (skhid-srp's pack
weapons-069, its `docs/notes/weapons-069.md`: stats, ids, icons). The converter was first
written for the H&K 416A5 and MSBS Radon (commits 7eabfa8, 9314977), dropped when those came
from 0.31a2 instead, and is revived here for any 0.69d0 weapon.

## Build

```sh
# 1. The 0.69d0 files the weapons draw (a reader of 0.69d0's compressed volumes: the
#    weapon survey's vostok-port, extract-bundle), as one archive.
scripts/weapons-069d-donor.py <survey vostok-port> full069.split paths069.txt LIST akmsu famas g3a3 mauser ks_23 ppsh_drum
vostok-port extract-bundle --db full069.split $(sed 's/^/--prefix /' LIST) --output TREE
cargo run --release --example pack-donor-tree -- TREE/overlay donor.db
# 2. The bundle over what packs are composed over (the stock db and skhid-srp's base patches),
#    its lobby icons, then the pack: skhid-srp's scripts/weapons-069/make_pack.sh does all.
cargo run --release --example build-weapons-069d -- --donor donor.db --target-db STOCK \
  --target-patch resources.db.patch_12 ... --icons --output B
scripts/render-weapons-069d-icons.py --bundle B --lobby-swf <players' lobby_menu.swf>
```

## What converts, and how

* **The item config.** 0.69d0 keeps every item in `gameplay/game.options`
  (`weapons.<key>`, `weapon_modules.<id>`). Each port starts from a live peer's config (its
  `template`: states, melee, grenades, recoil, lobby poses, card) and takes, state by state
  (`StatePlan`), either the peer's clips and sounds or the donor's. Item clips of 0.69d0-only
  states are left out (aimed `_calim` shots, the pump's `reload_finalround`). The stats are the
  table's (fitted to the live balance); the configs carry the camouflage and decal data the
  live weapons get from skhid-srp's `weapon-camo-1` and `weapon-decals-1`.
* **Rigs.** `Rig::Live`: the donor rig is a live weapon's skeleton bone for bone (checked);
  the port ships the live model's bind pose, skeleton file and hand locators, so the live clips
  play on it (AKMSU on the AK, FAMAS on the Ash-12, the drum PPSh on the PPSh). `Rig::Donor`:
  the donor's skeleton, bind pose and clips (G3SG/1, Mauser, KS-23M); its hands clips drive
  0.26g's 70-bone player rig (checked per clip).
* **Meshes.** 0.69d0 skins weapons with one bone a vertex in 32 bytes (position, bone, three
  basis floats, half2 UV) or two in 36 (two `u8` bones, the first one's weight); 0.26g's are 36
  bytes, one bone (position, bone, three packed basis words, float2 UV). A two-bone vertex goes
  to its heavier bone. A basis float is `(x << 16 | y << 8 | z) / 2^24`.
* **Modules.** The structural bundled and default modules are baked as surfaces bound to their
  socket bone, at the shipped bind frame of that bone plus the locator (`bind + R_y(pi) *
  locator`): in the game a bone's vertices are drawn at clip * bind^-1, and 0.69d0 drew the
  module at clip * locator (the AKMSU's magazine bone rests 15 cm off in 0.69d0's bind pose,
  where no clip takes it). A socket turned about x turns its module (the PPSh's drum well);
  `module_turns` turns a module authored facing another way (the 71-round drum). Every LOD0
  surface of a module is baked (a magazine with its cartridges). **Muzzle devices, foregrips,
  scopes and suppressors are never baked** (refused unless `separable`): `attach-points.json`
  records where each gun takes them (`muzzle_point`, the rails); suppressors are not ported.
* **Locators.** 0.26g's contract: `aim` (the donor's), `barrel_point` (the bare barrel's
  `muzzle_point`, else `fire_point`), `scope_point` (the scope rail, else the aim),
  `handguard_point` and the hand locators (`_idle`, `_lobby`, `_reload`, `_reload_pivot`).
* **Clips.** The same memory image but the event-channel headers (12 bytes in 0.69d0, 16 in
  0.26g). The AK-12's clips both builds ship convert to 0.26g's own bytes (test
  `shared_ak12_clips_convert_to_the_026g_bytes`). A reload item clip's sound cues must index its
  state's list.
* **Textures.** 0.69d0 is metalness PBR. Diffuse `0.514 * (albedo * (1 - 0.8 * metalness)) ^
  0.75` (re-encoded DXT1), gloss 0.36 x smoothness, Fresnel 0.162 x metalness^0.388 (both in the
  DXT endpoints), normal maps copied, on the native AK-12 material.
* **Sounds.** A donor sound 0.26g has is used as it is; one it lacks is carried (collections,
  singles, composites without 0.69d0's per-location reverb layers `weapons/environments/*`).
  Suppressed variants are left out.
* **Files.** Every new file is under a `v069d` folder (models, materials, textures, skeletons,
  clips, sounds), and the dictionary's compatibility rows are named
  `compatibility_v069d_<weapon>_<ammo>`, so no path or row is shared with another pack.
* **Icons.** Rendered from the converted meshes (right side, muzzle right, pistols at 0.6),
  exposure matched to native rifle icons, into the cells the configs name; a new ammo takes a
  live ammo's icon (`icon_like`).
