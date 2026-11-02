# Weapons of 0.69d0 for v0.26g

0.69d0 is a donor of the shared weapon core (`src/weapons_026`, `docs/weapons-026.md`):
`weapons_026::donor_069d::Donor069` serves the core 0.69d0's files in 0.26g's layouts, and the
core makes the item configs, the dictionary rows, the sounds and the bundle as for any donor.
A port is `ports/weapons-069d.json` (the core's manifest: guns, ids, templates, balance, which
states stay the template's) and its sidecar `ports/donor-weapons-069d.json` (what only 0.69d0
needs: each gun's rig, the modules left out, modules to turn, new rounds). skhid-srp's packs
weapons-069 and weapons-069-camo are made from it (`scripts/weapons-069/make_pack.sh` there).

The converter was first written for the H&K 416A5 and MSBS Radon (7eabfa8, 9314977), dropped
when those came from 0.31a2 (6dedabe), revived beside the core (#24) and then folded into it.

## Build

```sh
# 1. The 0.69d0 files the guns draw (0.69d0's volumes are compressed: the weapon survey's
#    vostok-port reads them), as one archive with gameplay/game.options.
scripts/weapons-069d-donor.py <survey vostok-port> full069.split paths069.txt LIST akmsu famas g3a3 mauser ks_23 ppsh_drum
vostok-port extract-bundle --db full069.split $(sed 's/^/--prefix /' LIST) --output TREE
cargo run --release --example pack-donor-tree -- TREE/overlay donor.db
# 2. The bundle over what packs are composed over (the stock db and skhid-srp's base patches).
vostok-port port-weapons-069d --manifest ports/weapons-069d.json \
  --donor-spec ports/donor-weapons-069d.json --donor donor.db \
  --target-db STOCK --target-patch resources.db.patch_12 ... --output B
```

## What the donor does (0.69d0 only)

* **Layouts.** Configs (16-byte records) re-encoded in 24; clips through the core's
  `clips::events_12_to_16`. `DonorBuild` 0.69d0: 0.31a2's state types (two-clip idle, reload
  and fire 312..314 are 0.26g's 313..315; grenade 315 is 316) and melee 324 as 321.
* **Item configs** without what 0.26g cannot take: suppressed sounds (no suppressors) and clips
  of 0.69d0-only states (aimed `_calim` shots, the pump's `reload_finalround`).
* **Models**, composed at 0.69d0's model path. 0.69d0 skins weapons with one bone a vertex in
  32 bytes (position, bone, three basis floats, half2 UV) or two in 36 (two `u8` bones, the
  first one's weight); 0.26g's are 36 bytes, one bone (position, bone, three packed basis words,
  float2 UV). A two-bone vertex goes to its heavier bone. The **structural** bundled and default
  modules are baked as surfaces bound to their socket bone, at the shipped bind frame of that
  bone plus the locator (`bind + R_y(pi) * locator`): in the game a bone's vertices are drawn at
  clip * bind^-1, and 0.69d0 drew the module at clip * locator (the AKMSU's magazine bone rests
  15 cm off in 0.69d0's bind pose, where no clip takes it). A socket turned about x turns its
  module; `module_turns` turns a module authored facing another way (the 71-round drum). Every
  LOD0 surface of a module is baked. **Muzzle devices, foregrips, scopes and suppressors are
  never baked** (refused unless `separable`); `donor-069d.json` records each gun's modules and
  its attach points (`muzzle_point`, the rails) for the weapon kit.
* **Rigs.** `live`: the donor rig is a live weapon's skeleton bone for bone (checked); the model
  takes the live model's bind pose, skeleton file and hand locators, so the live clips play on
  it (the FAMAS on the Ash-12's, whose clips 0.69d0 plays too). `donor`: the donor's
  skeleton, bind pose, clips and hand points (the AKMSU, the Drum: a live rig's clips hold their
  bodies where the live gun's are, the Drum's left hand on the box magazine's place). Every
  carried clip is checked against its rig (hands: 0.26g's player rig; item: the gun's bones).
* **Renames** (the sidecar's `renames`, `donor_069d/renames.rs`). The core ships a donor file
  only when the target lacks its path, so 0.69d0's files under a name 0.26g also uses would play
  as 0.26g's older ones. A rename serves a donor file at a path of its own and renames every
  name of it in the donor's configs: clip folders (0.69d0's `ak/` as `akmsu_069/`, `ppsh/` as
  `ppsh_drum/`) and sound names with everything under them (`weapons/akm_n` as
  `weapons/akm_n_069`, which takes `weapons/akm_n_m` and `weapons/akm_n/...` along). A name
  is renamed only when the donor has its file; a template's name stays.
* **Clip events** (the sidecar's `clip_events`): a donor clip's event channel set anew (the
  Mauser's empty reload: 0.69d0 leaves its sound events' indices at 255, 0.26g plays none).
* **Locators.** 0.26g's contract: `aim`, `barrel_point` (the bare barrel's `muzzle_point`, else
  `fire_point`), `scope_point` (the scope rail, else the aim), `handguard_point`, and the hand
  locators (`_idle`, `_lobby`, `_reload`, `_reload_pivot`).
* **Materials.** 0.69d0 is metalness PBR. Each material the models draw is the native AK-12's
  with four maps, each pixel mapped and the map encoded DXT1 again: diffuse
  `0.76 * albedo * (1 - 0.5 * metalness)` (sRGB), gloss `0.36 * smoothness`, Fresnel
  `0.08 + 0.09 * sqrt(metalness)`, the normal map as it is. Fitted on the ten texture sets both
  builds ship with the same layout (`convert.rs`); the curves applied before to the DXT endpoints
  left the blocks they collapsed raw (raw metalness as Fresnel: 2 to 3 times stock).
* **New rounds** are the core manifest's `ammo_items`.
* **Icons.** Each gun's lobby cell is 0.69d0's own icon (`weapon_<y>_<x>.png` of its
  `ui_desc.icon`) through the core's `lobby_icon`; 0.69d0 draws them diagonally in a 128x128
  square, so the donor serves them level (mirrored, turned by the art's own axis, muzzle
  right, `convert::level_lobby_icon`) and the core's `cell_png` fits them.
