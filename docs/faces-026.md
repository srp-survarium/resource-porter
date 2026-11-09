# Player faces for v0.26g

`faces_026` builds the faces of skhid-srp's face choice: today's head (face 0),
the eight faces of 0.69d0 and two bare faces of the STK2 characters export, each
in the variants the game draws under a helmet and under a mask, fitted under every
item of the released game that draws the head. `face_fit` is the measure and the
fix; `examples/build-faces-026.rs` the builder, `examples/render-faces-026.rs` and
`scripts/render-faces-026.py` the pictures, `scripts/faces-026-icons.py` the
picker's icons.

## What the game reads

`gameplay/bodyparts/default`, `body_parts.heads`, in 0.31a2's form:

```
heads[i].data         {base_model, part_name, additional_models}   the bare face (+ hair)
heads[i].data_helmet  {base_model, part_name}                      under a helmet, no mask
heads[i].data_mask    {base_model, part_name}                      under a mask (with or without a helmet)
heads[i].icon         [row, col]                                   picker icon, LobbyMenu_EquipmentIcons
```

A missing `data_helmet` or `data_mask` means the bare face. `heads[0]` is the
native head (`character/human/base/base_character_head`, `black_market_01_head_s`);
`body_parts.head` stays as it is. The models are `models/ported/faces/<face>[_helmet|_mask]`.

| i | face | from | variants |
|---|---|---|---|
| 0 | native | 0.26g `base_character_head` | none |
| 1 | `069d_head_default` | 0.69d0 `head_default` | helmet, mask |
| 2-4 | `069d_head_01` .. `_03` | 0.69d0 `head_01` .. `head_03` | as the fit needs |
| 5 | `069d_head_04` | 0.69d0 `head_04` | helmet (`head_04` is its own mask variant) |
| 6 | `069d_head_05` | 0.69d0 `head_05` + hair `head_05_hair_vc` | helmet, mask |
| 7 | `069d_head_06` | 0.69d0 `head_06` | helmet, mask |
| 8 | `069d_head_07` | 0.69d0 `head_07` + hair `head_07_hair_vc` | helmet, mask |
| 9 | `stk2_bandit_01_head_03` | STK2 export | made here |
| 10 | `stk2_neutral_01_head` | STK2 export (the Novice under his knit cap) | made here |

## How a face is made

- **0.69d0.** The surfaces bind the native rig in its bind pose
  (`face_items_069d::same_rig_for`) and convert in place
  (`v069d_skinned::to_026_four_bone`). Their necks hang 4 to 5 cm below the native
  one, inside every collar: they are cut 3 mm above the native neck's open edge,
  which runs aslant from 1.480 m under the chin to 1.524 m at the nape
  (`NeckEdge`, `cut_neck`), and below 1.56 m the neck takes the native neck's
  skin (all Neck at 1.52 m and below, `native_neck_skin`), so it turns with the
  head as the native neck does and as the collars were made for.
- **STK2.** The export's converted models, on the native rig turned half a turn
  about Y (`duty_026::rotate_mesh`), moved onto the native skull: the
  translation (x = 0) laying the face above the Head bone onto the native head,
  both ways point to surface (`align_onto`: bandit 3.1 mm down, 18.9 mm back;
  neutral 8.6 mm down, 6.1 mm back). `neutral_01_head`'s neck ended 36 mm above
  the native one and is stretched down to it (`stretch_neck`), then both are cut
  along the native neck's edge as 0.69d's are. `neutral_01_head` is the Novice's
  head with his knit cap modelled on it (its scalp ends at 1.725 m under the
  cap): the cap stays in every variant, pulled in under helmets and masks. Their helmet and
  mask variants start from the bare face and are pulled in by the fit, as
  0.69d's were made by hand (skull top in under helmets, face slimmer under masks).
- **Materials.** The native head's material (subsurface scattering) with the
  donor's colour and normal maps; 0.69d's smoothness map as the gloss map, its
  range set so its 90th percentile is the native head's; the Fresnel the native
  head's mean, constant. STK2's TGAs are drawn at the native 1024. The hair is a
  copy of `edge_hunter_01_torso_alpha` (alpha test), with the hair's maps.

## The fit (`face_fit`)

The items are every `items_dict` row's helmet, mask, torso (each distinct sleeve
variant) and back piece in the game's view (the stock db and the released
patches), but those with `remove_head_model`. Worn sets: bare = each torso or
back; helmet = each of those and each helmet; mask = each of those, each mask,
and each helmet that leaves the mask slot over each mask that leaves the helmet
slot.

- Rays from the skull's middle at the eyes' height (skinned to Head), or for
  the neck, horizontally from the neck's axis at the sample's height (Neck to
  Head); more than 2.5 cm below the native chin always the neck's axis, above
  it whichever ray crosses the face's surface more squarely. Samples: every
  vertex and every triangle's middle.
- An item's skin along a ray: its first surface facing out beyond 1 cm inside the
  native head (the face where the native head is not on the ray) (liners face in; goggles stacked on a shell are
  beyond the shell). Worn together, the outermost item's skin covers.
- The rule: where an item covers a ray, the face stays 1 mm (bind pose) or
  0.2 mm (sampled poses) under its skin, or no more than 0.2 mm beyond the
  native head on the same ray: every live item was made over the native head.
  Where the native head itself shows through an item in a pose, a face may show
  through it as deep in the same region of the head (30 x 22.5 degree regions
  round the skull).
- Poses: the bind pose and 24 poses of each of `bandit_026::POSE_CLIPS` (457),
  every surface skinned through them; an item bone the native rig lacks (the
  helmets' render bone of skhid-srp's `headwear-1` edit) follows Head.
- The fix pulls vertices in along their rays by what the worst pose needs,
  spread over four rings of neighbours (60 % a ring), at most 4 mm a round and
  30 mm in all, round after round until every pose passes. The bare faces are
  fitted under the torsos first; helmet and mask variants start from the
  donor's variant or the fitted bare face without hair.
- The build stops if any face still shows through anything. `faces-fit.json` is
  the record: per face and state, the least margins, gaps and where.

## Build

```sh
nix develop
cargo build --release --example build-faces-026 --example render-faces-026
P=$(for f in <released patches>; do printf -- '--patch %s ' "$f"; done)
target/release/examples/build-faces-026 \
  --source-069d <0.69d0 files> --stk2 <STK2 characters export> \
  --stk2-textures <STK2 textures> --target-db <stock 0.26g resources.db> $P \
  --output .work/faces
vostok-port pack from-bundle --base-db <stock db> $P --bundle .work/faces \
  --output .work/faces-pack --id faces --version 1
python3 scripts/faces-026-icons.py --bundle .work/faces \
  --renderer target/release/examples/render-faces-026 --pack .work/faces-pack
python3 scripts/render-faces-026.py --bundle .work/faces \
  --renderer target/release/examples/render-faces-026 --output <pictures>
```

- 0.69d0 files: `models/character/human/head`, `material_instances/characters/human/head`
  and `textures/character/human/head` extracted at their paths from 0.69d0's
  `resources.01.db`, `resources.00.db` and `resources.04.db`
  (`vostok-port extract`).
- STK2: the characters export (`___surva_stk2_characters_port`) and STK2's
  `resources/sources/textures/character/human` (`bandit/bandit_01/bandit_01_head*`,
  `neutral/neutral_01_novice/neutral_01_novice_head*`).

## Result (2026-11-09, over the stock db and skhid-srp's patches 12 .. 53)

- 29 helmets, 18 masks, 111 torso and back models; 866 worn sets (bare 111, helmet 140,
  mask 615); 8 items draw no head. 457 poses.
- Before the fix the donors' own models showed through most worn sets (the 0.69d0 bare faces
  through 88 of 111 torsos, up to 12-18 mm at the nape and throat; `head_05`'s mohawk 39 mm
  under hooded torsos). After it every face passes in every state: least margins 0.00 to
  0.25 mm; deepest pulls 4.2 to 39.5 mm (the mohawk under the STK2 loner's hood); every face
  ends with helmet and mask variants. Today's head itself shows through some collars by up to
  10.9 mm in crouched poses (the regional allowance). `faces-fit.json` has every number.
- Dev switches while working on a fit (never for a pack): `FACES_BIND_ONLY=1` measures the bind
  pose alone; `FACES_SKIP_FIT=1` writes the donors' variants unfitted, for pictures.
