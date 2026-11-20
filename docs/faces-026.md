# Player faces for v0.26g

`faces_026` builds the faces of skhid-srp's face choice: today's head (face 0),
the eight faces of 0.69d0 as 0.69d0 draws them, and the STK2 Novice's face with
a 0.69d0 face's short scalp joined at his hairline. `face_shape` joins each
face's neck to the body where today's head's is and gives the masks a model for
a face: 0.69d0's own model of its face-wear for that face, else the game's mask
carried onto the face where it does not fit it; `face_fit` is the gate: the
build stops when a face shows through an item, or an item stands off a face,
that the open list does not name. `examples/build-faces-026.rs` is the builder, `examples/render-faces-026.rs`
with `scripts/render-faces-026.py` and `scripts/render-faces-026-pairs.py` the
pictures. The lobby's faces' window shows its own portraits (game captures in
skhid-srp's lobby movie, `bitmaps/SrpFace<i>.jpg`), so the pack draws no icon
(faces@1 to @5 drew `heads[i].icon` cells of the equipment atlas, `[i + 1, 16]`,
which the window showed scaled up: too few pixels on a large screen).

## What the game reads

`gameplay/bodyparts/default`, `body_parts.heads`, in 0.31a2's form (0.69d0 spells
the same three `heads[i].models.base`, `.helmet`, `.mask`):

```
heads[i].data         {base_model, part_name, additional_models}   the bare face (+ hair, or the scalp)
heads[i].data_helmet  {base_model, part_name, additional_models}   under a helmet, no mask
heads[i].data_mask    {base_model, part_name, additional_models}   under a mask (with or without a helmet)
```

A missing `data_helmet` or `data_mask` means the bare face, as in 0.69d0.
`heads[0]` is the native head (`character/human/base/base_character_head`,
`black_market_01_head_s`); `body_parts.head` stays as it is. The models are
`models/ported/faces/<face>[_helmet|_mask]`.

A mask's model for one face, in the item's own config
(`gameplay/items/armour/mask/<item>.options`), 0.69d0's
`equipment.<item>.models.head_<n>` in 0.26g's spelling:

```
data_head_<i>         the item's `data` with other models: the item as face i wears it
```

For 0.69d0's face-wear the entry also has `remove_head_model: true` and the face it shows
(`head_s`, a surface of its model with the face's material) as an additional model, as 0.69d0
draws its `head_<n>` models.

Its models are `models/ported/faces/items/<face>/<the item's model>` (every
file of the item's model; a carried model has every surface and level of detail
carried, 0.69d0's face-wear its own surface for the face in the place of the
game's one, the surface's `export_properties` counting its vertices); `<i>` is
the index into `heads`. A missing `data_head_<i>` means `data`, as 0.69d0's
missing `head_<n>` means `base`. As in 0.69d0, only masks have them (0.69d0
looks `head_<n>` up for masks and face-wear only); no helmet has one.

| i | face | from |
|---|---|---|
| 0 | native | 0.26g `base_character_head` |
| 1 | `069d_head_default` | 0.69d0 `head_default` |
| 2-4 | `069d_head_01` .. `_03` | 0.69d0 `head_01` .. `head_03` |
| 5 | `069d_head_04` | 0.69d0 `head_04` |
| 6 | `069d_head_05` | 0.69d0 `head_05` + hair `head_05_hair_vc` |
| 7 | `069d_head_06` | 0.69d0 `head_06` |
| 8 | `069d_head_07` | 0.69d0 `head_07` + hair `head_07_hair_vc` |
| 9 | `stk2_neutral_01_head` | STK2 export (the Novice), cap and hair off, a 0.69d0 scalp joined |

## How a face is made

- **0.69d0: a straight conversion.** Each surface (`models/character/human/head/<model>`)
  binds the native rig in its bind pose (`face_items_069d::same_rig_for`: the same 69 bones,
  only Root and Item differ, which no face uses) and is written in 0.26g's four-bone layout
  (`v069d_skinned::to_026_four_bone`) with 0.69d0's own positions, all its bone weights (at most
  three a vertex, on Chest, Neck and Head; 0.26g's skinned vertex takes four), normals,
  tangents, binormals and UVs. The teeth, the mouth and the eyes are meshes of their own inside
  each face's surface. The hair is drawn on the bare face only, as in 0.69d0.
- **The neck's seam** (`face_shape::join_native_neck`), the one place a face is changed. Every
  body of the game was made for the native head's neck: its collars stand round that neck and
  its neck hole ends on that neck's open edge. A 0.69d0 neck reaches 39 to 49 mm below that edge
  and is up to 21 mm wider there. Each face's skin is cut along the native edge, its last ring
  laid on that edge, and its neck laid onto the native neck with the native neck's skin, each
  vertex by how little it follows the Head: wholly from a Head weight of 0.5 down, not at all
  from 0.9 up (the neck, the nape, the throat under the jaw). A vertex that follows the Head by
  0.9 or more is not touched: the function fails if one differs from the donor's in any bit, so
  the face, the ears, the teeth, eyes and mouth and the skull are 0.69d0's, vertex for vertex.
- **Under a helmet** a face is 0.69d0's own helmet head where it has one (`head_default`,
  `head_04` .. `head_07`: the hair and scalp pressed, the face as it is), else the bare face.
- **Under a mask** a face is 0.69d0's own mask head where it has one (`head_default`,
  `head_05` .. `head_07`), as 0.69d0 draws a masked face, else the helmet head. 0.69d0's mask
  heads are the bare head with the face itself pushed in under masks (1,000 to 1,800 vertices
  by more than half a millimetre, up to 8 to 18 mm; the build prints each). 0.69d0's own
  face-wear for each face is made over them (its `head_s`, the face it draws in the eye holes,
  is the mask head's skin there), and the masks are carried onto them.
- **Covered skin** (`face_fit::covered`, `face_shape::cut_covered`). Skin no one can see under
  any helmet (96 sight lines from each vertex over the half space outside it, all of them
  meeting each helmet, and so for its neighbours two rings round) is left out of the helmet
  model; the same for the masks. It is little: the live helmets and masks include ones that
  cover almost nothing (`eq046_blackmarket_helmet_7`, `novice_026`, `eq046_edge_mask_6`), so the
  helmet models lose 0 to 58 triangles of the scalp's top and the mask models none.
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
  back), its neck (36 mm short of the native one) stretched down past the native neck's edge
  and joined to it as the others. His knit cap is modelled on his head (the mesh at its top,
  his skin open under it), its shadow painted on his forehead and his own longer hair below
  it. `graft_scalp_069` makes his head one surface with 0.69d0 `head_01`'s scalp
  (`NOVICE_SCALP`, chosen by its look on him; `FACES_NOVICE_SCALP=<head>` another):
  - his cap off, and his head cut along his hairline (`HAIRLINE`: across the forehead under the
    band the cap shadowed, over the ears, behind them down to the nape; his ears kept whole);
    the donor's head above the same line (its ears left out);
  - the scalp's cut edge laid on his edge vertex for vertex (the two edges matched by the
    least total move along them, each scalp edge vertex onto its match, the rest of the scalp
    moved with the edge), and a strip of triangles over the shared vertices closing the seam:
    no gap and no vertex on the side of another triangle (a T-joint cracks). Where the moves
    fold a scalp triangle over (round the ears), its free corners go to their neighbours'
    middle until it faces out again; the four that two of his edge vertices running back at
    corner of the ear leave folded (3 to 20 mm², behind the ear tops) are open (none shows in
    the game);
  - on the seam the scalp takes his skin weights and normals (a vertex skinned otherwise than
    the one it sits on parts from it when posed), and the normals 12 mm either side are made
    again from the joined surface; its UVs go into his cap's old place in his map;
  - one map and one material, his (`bake_novice_maps`, at 2048): his colour and normal maps,
    the cap's place painted with the donor's scalp (its colour toned to his forehead, a gain a
    channel), and over 15 mm either side of the seam each texel's tone giving way to both
    sides' mean at the nearest point of the seam, its detail thinning (60 % gone at the seam)
    and the normal map going flat: at the seam both sides are alike;
  - his map has the cap's shadow and brim painted under his hairline (to 84 texels under the
    cut at the temples): `skin_over_cap_band` paints his skin there, each column's skin from
    where it starts mirrored upwards (the normal map with it), so no edge is left where his
    real skin starts. Past each part of the map near the seam the edge texels are carried out
    48 texels (in the scalp's place too: past its part lies the rest of the donor's map, its
    face, which the filtering drew as a line along the seam).

## A mask for a face (`face_shape`, `face_fit::settle`)

A mask a face shows through, or whose seen edge stands off it, gets a model for that face;
0.69d0's face-wear gets its own for each 0.69d0 face whatever the fit. No helmet gets one.

- **0.69d0's own** (`face_wear_069d`), drawn as 0.69d0 draws it, with no head: the live `face_069d_balaclava`, `_balaclava_3_hole`,
  `_keffiyeh` and `_buff` are 0.69d0's `renesanse_balaclava`, `scavenger_balaclava`,
  `edge_balaclava` and `bm_balaclava` from their `_head_00` model; 0.69d0 has a model of each
  for each of its faces (`models/character/human/balaclavas/<kind>_balaclavas/<kind>_balaclava_head_0<n>`,
  the cloth surface and `head_s`, the face seen in the eye holes, which is the face's mask head
  there; 0.69d0 draws them with `remove_head_model`). Face n gets `_head_0<n>` converted as
  0.69d0 draws it (its vertices, skin, basis and UVs): the cloth in the place of the game's
  surface, `head_s` a surface of the same model with the face's material, and the entry
  `remove_head_model` with `head_s` an additional model. One change: the part of the cloth
  that bends with the neck is laid where the game's model has it (`onto_live_neck`: by UV,
  each vertex by `neck_share` of how little it follows the Head, as the faces' necks: the
  game's model is tucked into the game's collars).
- **Carried** (`Carry`): the game's mask moved from the native head onto the face (not
  0.69d0's face-wear: its rolled insides do not follow a face as its cloth does; the Novice,
  who has no 0.69d0 model, wears the game's). Each vertex
  moves along the native head's normal at the point of the native head nearest it, by how far
  out (or in) along that normal the face's skin nearest that point is (within 25 mm): it keeps
  the native head's place under it (an eye hole stays on the eyes, an edge on the same part of
  the head) and the distance from the skin its maker gave it. A vertex whose line meets no skin
  takes its neighbours' move. The moves are smoothed over the surface (6 rounds), but on its
  open edges, which keep their own. Every surface of the mask's models is carried, the lower
  levels of detail too.
- **Settled.** In the bind pose, where a sample of the face is still nearer the mask's skin than
  the gate's rule lets it be, the triangle over it moves out along the sample's ray by what is
  missing (its neighbours by half), round after round, at most 16 mm a vertex (a 0.69d0 ear
  farther back than the native one under a strap). Only the bind pose: settled in the sampled
  poses too, cloth that follows the neck was pushed centimetres out of the way of a jaw that
  only meets it when the head bows.

## The gate (`face_fit`)

The items are every `items_dict` row's helmet, mask, torso (each distinct sleeve
variant) and back piece in the game's view (the stock db and the released
patches), but those with `remove_head_model`. Worn sets: bare = each torso or
back; helmet = each of those and each helmet; mask = each of those, each mask,
and each helmet that leaves the mask slot over each mask that leaves the helmet
slot. Every face in every state is measured in the bind pose and 24 poses of each
of `bandit_026::POSE_CLIPS` (457 poses), a mask as the model made for the face
where there is one. Under 0.69d0's own face-wear for a face (and any helmet over
it) no head is drawn: nothing of the face is measured there. `faces-fit.json` records, per face and state, the
worn sets it shows through and the worst five, every misfit, the necks' joins,
what 0.69d0's own variants moved and the items made for faces.

- **Rays** from the skull's middle at the eyes' height (skinned to Head), or for
  the neck, horizontally from the neck's axis at the sample's height (Neck to
  Head); more than 2.5 cm below the native chin always the neck's axis.
  Samples: every vertex and every triangle's middle.
- **An item's skin** along a ray: its first surface facing out beyond 1 cm inside the
  native head. Worn together, the outermost item's skin covers.
- **The rule**: where an item covers a ray, the face stays 1 mm (bind pose) or
  0.2 mm (sampled poses) under its skin, or as far under it as the native head is
  under the game's item on the same ray (to 0.2 mm). On a ray where the native
  head itself is through the item, skin is what is seen there today, and a face
  shows skin there too; where the native head shows through an item in a pose, a
  face may show through as deep in the same region of the head.
- **Contact, recorded and not a misfit.** Skin a torso or back piece does not cover
  in the bind pose (within 2 cm under it: a collar covers the neck, not the chin)
  can come down into it in a pose, as the native chin meets a collar when the head
  bows. Nothing covers that skin, so nothing shows through; the record has, per
  face and state, how many collars a face's chin and jaw meet where the native ones
  stay out, and how deep.
- **Standing off.** An item's open edges that lie on the native head (within 12 mm)
  and can be seen (15 % of the sight lines from the point pass the item and the
  head) stand no more than 4 mm farther off a face than off the native head.
- **The open list** (`--open ports/faces-026-open.tsv`): the misfits that are known
  and why, a line each (`face`, `state`, `what`, the most it may be in mm, `why`).
  A misfit the list names, no deeper than it says, does not stop the build; any
  other does, and so does one that got deeper. A torso is named once (it is worn in
  every state); a helmet over a mask is open when either of the two is.
  `FACES_WRITE_OPEN=<file>` writes a build's misfits in that form.

`FACES_NO_NECK=1` and `FACES_NO_CUT=1` leave one step out, for the numbers
before (`--measure-only`); `FACES_ONLY=<face>,..` measures only those faces;
`FACES_REPORT_ONLY=1` writes the bundle whatever the gate says, to look at it
(never for a pack).

## Build

```sh
nix develop
cargo build --release --bin vostok-port --example build-faces-026 --example render-faces-026
P=$(for f in <released patches>; do printf -- '--patch %s ' "$f"; done)
target/release/examples/build-faces-026 \
  --source-069d <0.69d0 files> --stk2 <STK2 characters export> \
  --stk2-textures <STK2 textures> --target-db <stock 0.26g resources.db> $P \
  --open ports/faces-026-open.tsv --output .work/faces
target/release/vostok-port pack from-bundle --base-db <stock db> \
  $(for f in <released patches>; do printf -- '--base-patch %s ' "$f"; done) \
  --bundle .work/faces --output .work/faces-pack --id faces --version 6
# the masks' configs it edits (data_head_<i>) are added whole by earlier packs of the manifest:
python3 -c 'import json; p=".work/faces-pack/pack.json"; d=json.load(open(p)); d["requires"]=["army-balaclava","face-069d","equipment-029","equipment-046","equipment-069"]; json.dump(d,open(p,"w"),indent=2)'
python3 scripts/render-faces-026-pairs.py --before <the earlier pack's folder> --before-name faces@3 \
  --bundle .work/faces --after-name faces@4 --icons-069d <0.69d0's ui_icons/heads> \
  --renderer target/release/examples/render-faces-026 --output <pictures>
```

- 0.69d0 files: `models/character/human/head`, `models/character/human/balaclavas`,
  `material_instances/characters/human/head` and `textures/character/human/head` extracted at
  their paths from 0.69d0's `resources.01.db`, `resources.00.db` and `resources.04.db`
  (`vostok-port extract`).
- STK2: the characters export (`___surva_stk2_characters_port`) and STK2's
  `resources/sources/textures/character/human` (`neutral/neutral_01_novice/neutral_01_novice_head*`).
- `FACES_NO_REPORT=1` skips the gate while looking at pictures (never for a pack).
