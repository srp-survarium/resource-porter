# Player faces for v0.26g

`faces_026` builds the faces of skhid-srp's face choice: today's head (face 0),
the eight faces of 0.69d0 as 0.69d0 draws them, and the STK2 Novice's face with
a scalp grafted where his knit cap was. `face_fit` measures every face under
every item of the released game that draws the head, as a report: it never
changes a face. `examples/build-faces-026.rs` is the builder,
`examples/render-faces-026.rs` and `scripts/render-faces-026.py` the pictures,
`scripts/faces-026-icons.py` the picker's icons.

## What the game reads

`gameplay/bodyparts/default`, `body_parts.heads`, in 0.31a2's form:

```
heads[i].data         {base_model, part_name, additional_models}   the bare face (+ hair, or the scalp)
heads[i].data_helmet  {base_model, part_name}                      under a helmet, no mask
heads[i].data_mask    {base_model, part_name}                      under a mask (with or without a helmet)
heads[i].icon         [row, col]                                   picker icon, LobbyMenu_EquipmentIcons
```

A missing `data_helmet` or `data_mask` means the bare face, as in 0.69d0.
`heads[0]` is the native head (`character/human/base/base_character_head`,
`black_market_01_head_s`); `body_parts.head` stays as it is. The models are
`models/ported/faces/<face>[_helmet|_mask]`.

| i | face | from | variants (0.69d0's own) |
|---|---|---|---|
| 0 | native | 0.26g `base_character_head` | none |
| 1 | `069d_head_default` | 0.69d0 `head_default` | helmet, mask |
| 2-4 | `069d_head_01` .. `_03` | 0.69d0 `head_01` .. `head_03` | none |
| 5 | `069d_head_04` | 0.69d0 `head_04` | helmet |
| 6 | `069d_head_05` | 0.69d0 `head_05` + hair `head_05_hair_vc` | helmet, mask |
| 7 | `069d_head_06` | 0.69d0 `head_06` | helmet, mask |
| 8 | `069d_head_07` | 0.69d0 `head_07` + hair `head_07_hair_vc` | helmet, mask |
| 9 | `stk2_neutral_01_head` | STK2 export (the Novice), cap off, scalp grafted | none |

## How a face is made

- **0.69d0: a straight conversion.** Each surface (`models/character/human/head/<model>`)
  binds the native rig in its bind pose (`face_items_069d::same_rig_for`: the same 69 bones,
  only Root and Item differ, which no face uses) and is written in 0.26g's four-bone layout
  (`v069d_skinned::to_026_four_bone`) with 0.69d0's own positions, all its bone weights (at most
  three a vertex, on Chest, Neck and Head; 0.26g's skinned vertex takes four), normals,
  tangents, binormals and UVs. Nothing is cut, re-weighted or moved: the necks are 0.69d0's
  (they reach 4 to 5 cm lower than today's head, under the torso), and so are the teeth, the
  mouth and the eyes (meshes of their own inside each face's surface). The helmet and mask
  variants are only those 0.69d0 has; the hair is drawn on the bare face only, as 0.69d0's
  variants have none.
- **Materials, input for input.** 0.69d0's `g_stage_default` head and hair materials
  (`material_instances/characters/human/head/<model>[_hair]`) over a native template (the native
  head's for skin, `edge_hunter_01_torso_alpha` for hair): `t_albedo` to `texture_diffuse`,
  `t_normal` to `texture_normal` (both builds' normal maps are DXT5 with x in alpha and y in
  green, and both meshes' tangent is dP/du and binormal dP/dv), `t_smoothness` to
  `texture_roughness` with `min_smoothness`..`max_smoothness` as its range (0.26g's map is a
  gloss: the light's roughness is one less), or the constant `smoothness` where 0.69d0 binds no
  map; `t_metalness` (the `_fmap` maps, 0.04 to 0.07 over skin) to `texture_fresnel` as it is,
  or the constant `metalness`; `use_subsurface_scattering`; the hair's alpha test, its
  `alpha_ref` (0.2) and its two sides. The maps are 0.69d0's DDS unchanged. Not carried: the SSS
  mask and thickness maps the 0.69d0 materials name (`*_sssmmap`, `*_tmap`: not in the retail
  data) and the hair's `use_olta`.
- **The STK2 Novice.** The export's model on the native rig turned half a turn about Y
  (`duty_026::rotate_mesh`), moved onto the native skull (`align_onto`: 8.6 mm down, 6.1 mm
  back), its neck (36 mm short of the native one) stretched down to the native neck's bottom,
  cut along its edge and skinned as it (`stretch_neck`, `cut_neck`, `native_neck_skin`). His
  knit cap is modelled on his head (the mesh at its top, his skin open under it): `graft_scalp`
  takes it off and grafts the native head's scalp on, cut along his skin's open top (12 mm under
  it over the forehead, 16 mm at the nape, over the cap's painted shadow) and lifted 2 mm
  outside his skin where his skin stands out. The scalp keeps the native head's skin weights
  and UVs, so it fits every item as the native head does; its colour map is the native one with
  the forehead painted over by the native side hair (the hairline a forehead's width above his
  brows), toned to his own skin (`scalp_diffuse`). His face's material is the native head's with
  his maps (gloss and Fresnel the native skin's, constant).

## The fit report (`face_fit`)

The items are every `items_dict` row's helmet, mask, torso (each distinct sleeve
variant) and back piece in the game's view (the stock db and the released
patches), but those with `remove_head_model`. Worn sets: bare = each torso or
back; helmet = each of those and each helmet; mask = each of those, each mask,
and each helmet that leaves the mask slot over each mask that leaves the helmet
slot. Every face in every state (its variant, else its bare face) is measured in
the bind pose and 24 poses of each of `bandit_026::POSE_CLIPS`;
`faces-fit.json` records, per face and state, how many worn sets it shows
through and the worst five. The build never fails on it and never changes a
face: fitting the faces is 0.69d0's own system's work (its variants, its
per-face item models), not a reshaping of the faces.

- Rays from the skull's middle at the eyes' height (skinned to Head), or for
  the neck, horizontally from the neck's axis at the sample's height (Neck to
  Head); more than 2.5 cm below the native chin always the neck's axis.
  Samples: every vertex and every triangle's middle.
- An item's skin along a ray: its first surface facing out beyond 1 cm inside the
  native head. Worn together, the outermost item's skin covers.
- The rule: where an item covers a ray, the face stays 1 mm (bind pose) or
  0.2 mm (sampled poses) under its skin, or no more than 0.2 mm beyond the
  native head on the same ray; where the native head itself shows through an
  item in a pose, a face may show through as deep in the same region of the head.
- `face_fit::pull_in` (moving vertices in along their rays) was the first pack's fix. It pulled
  the bare faces' chins and lower lips back by up to 16 mm behind their teeth (a mesh of their
  own, which the pull never moved): the teeth through the lips. It is not used.

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
  --output .work/faces-pack --id faces --version 2
python3 scripts/faces-026-icons.py --bundle .work/faces \
  --renderer target/release/examples/render-faces-026 --pack .work/faces-pack
python3 scripts/render-faces-026.py --bundle .work/faces \
  --renderer target/release/examples/render-faces-026 --output <pictures>
```

- 0.69d0 files: `models/character/human/head`, `material_instances/characters/human/head`
  and `textures/character/human/head` extracted at their paths from 0.69d0's
  `resources.01.db`, `resources.00.db` and `resources.04.db` (`vostok-port extract`).
- STK2: the characters export (`___surva_stk2_characters_port`) and STK2's
  `resources/sources/textures/character/human` (`neutral/neutral_01_novice/neutral_01_novice_head*`).
- `FACES_NO_REPORT=1` skips the fit report while looking at pictures (never for a pack).
