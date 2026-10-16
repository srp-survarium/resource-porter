# Duty exoskeleton for v0.26g

The current v13 build is **one complete inventory item**, dictionary ID 1805.
The user chose a single suit after reviewing separate helmet/mask prototypes.
The original combined head geometry is restored, with no positional adjustment,
and the native head is hidden. The split preview items are retired.

The item uses the native Zubr `data.additional_models` and
`data.additional_model_slots` mechanism to cover every armor slot. Its real
slot is **back (3)**; helmet, mask, torso, trousers, gloves and boots show the
same item as disabled linked cells. This choice preserves native oxygen
activation: `player.cpp` calls `inventory.action(back_slot, true)`. There is
one inventory instance, one weight contribution and one set of bonuses.
Remove the suit through its active back slot.

## Statistics

| Property | Value |
| --- | --- |
| Weight | 27 kg |
| Body armor | 80 |
| Head/face armor | 65 |
| Isolation | 70 |
| Carry capacity | +35 kg; 65 kg total before skills |
| Movement speed | +5% |
| Energy costs | -5% |
| Item pockets | +2; 3 total before skills |
| Ammo bags | +1; 4 total before skills |
| Pocket capacity | +1 |
| Artifact slots | +1 |
| Oxygen | 30 seconds |

Default modifiers are 1031, 1050, 1170, 1040, 1114 and 1165. Modifier 1165 is
an additive default-only -5% energy variant cloned from native 1162; its drop
weight is zero. The armor and isolation values are written to the union of
12 native hit regions, once per region. All previous standalone part bonuses
are aggregated once, including the trousers' ammo bag. Purchase cost is the
sum of the six earlier pieces' costs.

The oxygen config retains native `data.type = 1`, `amount_time_sec = 30` and
`influences` from `scavenger_oxygen_1`, alongside the displayed capacity.
Oxygen depletion in a toxic area and multiplayer combat balance have not yet
been playtested. Camouflage and decals remain disabled: imported textures
have no customization masks.

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
  name match. The builder removes old split-item dictionary entries 1800–1806
  and installs the complete suit at 1805. Existing matching modifier/localized
  strings are reusable; conflicting definitions fail closed.
* The complete-suit thumbnail uses the actual textured geometry. It occupies
  row 1, column 6 of the native Flash atlas. Every original atlas pixel and
  every other SWF tag is preserved.

## Rebuild

ImageMagick must be on PATH. The renderer also needs NumPy and Pillow.
Use new output directories rather than overwriting a running client's database.

```sh
cargo build --locked --example build-duty-026
target/debug/examples/build-duty-026 \
  --source-root "$HOME/Downloads/___surva_char" \
  --texture-dir .work/duty-026-inputs/resources/sources/textures/character/human/dolg \
  --target-db .work/duty-026/resources-v10-base.db \
  --output .work/duty-026/new-geometry
python3 scripts/render-duty-026-icons.py \
  --bundle .work/duty-026/new-geometry \
  --textures .work/duty-026-inputs/resources/sources/textures/character/human/dolg \
  --lobby-swf .work/duty-026/lobby-mod.swf \
  --output .work/duty-026/new-thumbnails
# Repeat the builder with a new output and:
# --icons-swf .work/duty-026/new-thumbnails/lobby_menu.swf
vostok-port apply \
  --base-db .work/duty-026/resources-v10-base.db \
  --bundle .work/duty-026/bundle-v13 \
  --output-db .work/duty-026/new-resources.db
vostok-port verify --db .work/duty-026/new-resources.db \
  --bundle .work/duty-026/bundle-v13
```

The v13 update bundle has 72 resources. A fresh retail-targeted build also
emits the five materials; map retention depends on the chosen baseline.

## Local server

```sh
python3 scripts/prepare-duty-026-lobby.py \
  --srp-root ../skhid-srp/srp-026g \
  --bundle .work/duty-026/bundle-v13 \
  --output .work/duty-026/new-lobby-server
cargo build --offline --manifest-path .work/duty-026/new-lobby-server/Cargo.toml \
  -p srp --bin srp
```

The staged server embeds the dictionary, item parameters and modifiers. New
local accounts receive one equipped suit, stock armor, three artifacts, all
120 native decals, and 5,000 gold. Decals are for compatible stock armor.
`SRP_DB=off` makes preview accounts ephemeral; bind every service to localhost.
Keep `SRP_TUTORIAL=on`: the account constructor marks the tutorial skipped so
native Equipment controls remain enabled.

The server enforces linked slots: equipping the suit clears the six covered
armor slots; equipping a standalone armor piece removes the suit. The generated
`duty_compound_suit_moves_as_one_item` test checks replacement through each
covered slot, re-equipping, complete removal and unchanged aggregate weight.
The direct-match staging script also equips only item 1805 and clears other
armor slots. Upstream server checkouts are untouched.

## Validation

All 72 emitted resources passed packed verification; four focused porter tests
and the staged server's compound-suit relocation test pass.
Database SHA-256:
`7ee132d65d2f8258ad5c9a0dc7b76abb03d338276f93801a1d9759cc66069ce5`.
The active client database is `.work/duty-026/resources.db`.

Native lobby renders the whole suit and shows the same item in all seven armor
cells, with weight 28.30 / 65 kg including starter pistol/ammunition. The head
uses its complete original mask/helmet appearance and no exposed base face.

Removing the suit through Gear clears all seven cells, restores the native
base outfit, and reduces weight to 1.30 / 30 kg. Screenshots:
`.work/duty-026/duty-v13-equipped.png` and `duty-v13-removed.png`.
