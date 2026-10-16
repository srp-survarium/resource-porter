# Duty exoskeleton for v0.26g

The v16 build is a **set of two items**, like the native level 15
exoskeleton sets (Zubr UM-4, Pilgrim): 0.26g has no set ID or set bonus,
so a set is separate items of one faction and level.

| Item | ID | Slot | Covers (linked cells) | Model |
| --- | --- | --- | --- | --- |
| Duty exoskeleton | 1805 | torso (2), category 3 | back, legs, gloves, boots | torso, frame, back unit, legs, hands, boots |
| Duty exoskeleton helmet | 1800 | helmet (0), category 1 | mask | the original combined helmet and mask |

Profile slots are helmet 0, mask 1, torso 2, back 3, legs 4, gloves 5 and
boots 6. Each piece lists its covered slots in `data.additional_model_slots`,
the mechanism Zubr's and Pilgrim's helmets use for the mask. The client maps
those names through the table at 0xA79870 (`back` is 3). srp reads the same
list (`item_params::covered_slots`): equipping a piece takes off what it
covers; equipping a covered slot's item takes the piece off. The two pieces
cover disjoint slots, so they are worn together.

Up to v15 the suit was one item in the **back** slot (category 4, cloned from
`ren_back_1`) covering the other six slots. Players saw it as a backpack.
The reason was oxygen: 0.100b's `player.cpp` activates oxygen with
`inventory.action(back_slot, true)`. 0.26g does not. Its quick-bar keys
(`0x4000 << n`, n = 0..5) act on the n-th usable item found by 0x2FE7F0.
That function walks quick slots 13 to 18, then armour slots 3, 0, 1, 2, 4, 5
and 6 (table 0x925FD8; caller 0x2D8D88). The body keeps native oxygen data
(`data.type = 1`, an `oxygen_tank`; `amount_time_sec = 30`; `influences`
from `scavenger_oxygen_1`), so its oxygen is found in the torso slot.
Armour (`hit_params`) and weight are read per worn item, not per slot.
Oxygen in a toxic area has not been playtested.

The helmet is the v10 split helmet: same ID, config path, internal name and
name string, and the original combined head surface (byte-identical to
v10/v13), with `remove_head_model`. Unlike v10, it also covers the mask
slot. Zubr's `additional_models` balaclava head is not used: the Duty head
covers the face. The body does not hide the native head.

Accounts saved with the v15 suit in the back slot lose it from that slot at
load (srp `Account::repair`: the item no longer fits slot 3). It stays in
the inventory and is re-equipped in the torso slot.

## Statistics

| Property | Body | Helmet | Set |
| --- | --- | --- | --- |
| Weight | 30 kg | 5 kg | 35 kg |
| Armour | 87 (10 body regions) | 72 (brain, face) | |
| Isolation | 70 | 70 | |
| Price | 225 000 | 75 000 | 300 000 |
| Oxygen | 30 seconds | | |
| Default modifiers | 1030, 1050, 1170, 1040 | none | |

The set keeps the v15 suit's totals: weight, price, armour per region,
isolation, modifiers and oxygen. The split follows the native sets. The
helmet costs the average of the Zubr and Pilgrim helmets (77 500 and
72 500). Its weight is between their 6 and 5 kg. The modifiers add a pocket,
an ammo bag, pocket capacity and an artefact slot. They sit on the body, as
native torso modifiers do. Each of Zubr UM-4's 12 hit regions is authored
once, on one piece. Both pieces have Renaissance faction, item and
matchmaking level 15, and `drop_weight` 0: they are sold, never dropped.
Camouflage and decals remain disabled: imported textures have no
customization masks.

## Icons and gallery

The lobby atlas gains a sixth column. The body is cell [1, 6], the Bandit
balaclava [2, 6] and the helmet [3, 6]. Both icons reuse earlier renders
(`--icon`). The body uses `.work/duty-026/thumbnails-v7/torso.png`, the
torso without legs, frame or back unit. The helmet uses
`thumbnails-v7/helmet.png` of the combined head. Bundle v10 used these
icons. `--gallery DIR` also renders `body-without-helmet.png`, `helmet.png`
and `full-set.png`.

## Inputs and conversion

Source models: `~/Downloads/___surva_char`, six native 69-bone model archives.
Textures were recovered from `/mnt/toshiba/surv-builds/stk_2.7z` into
`.work/duty-026-inputs/resources/sources/textures/character/human/dolg`.
The current map-preserving baseline is
`.work/duty-026/resources-v10-base.db` (SHA-256
`06ceeec5e32bc03ab3bedf779ebc6a4d7d98dc082530fe66123f786d87f516df`).
The earlier `.work/early-to-026/rollback-010b/resources.db` was deleted during
cleanup; the retained v10 baseline replaces it for this update.

* Editor configs reverse name/data pointers in 24-byte nodes; decode this
  before serialization.
* Rotate positions and packed tangent bases 180 degrees about Y to match
  native coordinates. Preserve four bone weights, UVs and triangle winding.
* The original combined head surface is copied intact after that rotation.
  It is byte-identical to v10. The rejected v11/v12 independent helmet/mask
  fit adjustments are not used.
* Frame and back-unit components remain separate render surfaces of the
  same suit. Their union preserves all 10,003 original attachment triangles.
  Whole welded components are selected across UV seams; reviewed anchors
  are guarded by the source hash.
* v0.26g DDS headers require both `DDSD_PITCH` and `DDSD_LINEARSIZE`; the size
  includes the trailing color-space byte. Head/legs use their own textures,
  boots use the leg atlas, and hands use the torso atlas.
* Approved matte materials retain specular multiplier 0.025, Fresnel 0.008
  and roughness 0.98. Identical installed materials are reused by updates.
* Existing Duty IDs may be replaced only when their config path and internal
  name match an earlier Duty row (`check_reused_id`): a split preview item
  1800–1806, the v13–v15 back-slot suit, or a piece of this set. The builder
  removes those rows and installs the body at 1805 and the helmet at 1800.
  Existing matching modifier/localized strings are reusable; conflicting
  definitions fail closed.
* Every original atlas pixel and every other SWF tag is preserved.

## Rebuild

The renderer needs NumPy and Pillow.
Use new output directories rather than overwriting a running client's database.

```sh
cargo build --release --locked --example build-duty-026
target/release/examples/build-duty-026 \
  --source-root "$HOME/Downloads/___surva_char" \
  --texture-dir .work/duty-026-inputs/resources/sources/textures/character/human/dolg \
  --target-db .work/duty-026/resources-v10-base.db \
  --output .work/duty-026/new-geometry
python3 scripts/render-duty-026-icons.py \
  --bundle .work/duty-026/new-geometry \
  --textures .work/duty-026-inputs/resources/sources/textures/character/human/dolg \
  --lobby-swf .work/duty-026/lobby-mod.swf \
  --icon body=.work/duty-026/thumbnails-v7/torso.png \
  --icon helmet=.work/duty-026/thumbnails-v7/helmet.png \
  --gallery .work/duty-026/new-gallery \
  --output .work/duty-026/new-thumbnails
# Repeat the builder with a new output and:
# --icons-swf .work/duty-026/new-thumbnails/lobby_menu.swf
vostok-port pack from-bundle \
  --base-db <0.26g>/game/resources.db \
  --build-base-db .work/duty-026/resources-v10-base.db \
  --bundle <bundle> --output <pack> --id duty-exo --version <n>
```

The bundle has 72 resources. The pack has 69 `add_file` operations,
including the five materials it carries from the build base, plus two atlas
icons, two dictionary rows and four strings. It composes over the stock
database (with the Bandit pack), not over an earlier patch holding 1805.

## Local server

```sh
python3 scripts/prepare-duty-026-lobby.py \
  --srp-root ../skhid-srp/srp-026g \
  --bundle <bundle> \
  --output .work/duty-026/new-lobby-server
```

Both staging scripts predate srp's data-driven equipment and need review
against current srp. srp itself needs no Duty-specific code: the shop,
slots and covered slots come from the dictionary row (`item_category`,
`base_cost`) and the item configs (`faction`, `item_level`,
`data.additional_model_slots`). The direct-match staging script equips
1805 in the torso slot and 1800 in the helmet slot.

## Validation

Builder tests: `duty_body_is_torso_armour_and_keeps_its_oxygen`,
`duty_helmet_covers_the_mask_like_zubrs`,
`duty_set_splits_the_suit_without_overlap` and
`only_earlier_duty_rows_are_replaced`. The set has not yet been seen in
the lobby or a match. Check there that the back, legs, gloves, boots and
mask cells are locked, and that oxygen is on the quick bar.

Earlier, v13 (one item in the back slot) was checked in the native lobby:
`.work/duty-026/duty-v13-equipped.png` and `duty-v13-removed.png`.
