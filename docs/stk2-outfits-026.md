# STK2 outfits for v0.26g (bandit, Novice, loner)

The rest of the STK2 characters export (`___surva_stk2_characters_port`, the owner's
download) as 0.26g equipment: three outfits, one content pack each. The builder is
`src/stk2_outfits_026.rs` (`examples/build-stk2-outfits-026`); the icons are
`scripts/render-stk2-outfits-026-icons.py`. skhid-srp's `docs/notes/stk2-outfits.md`
has the items, their stats and the owner's open questions.

## The export

- Its models are 0.26g skinned models on the native 69-bone rig, turned 180 degrees
  about Y (as the Duty set's are: `duty_026::rotate_mesh`, `check_bind`).
- `bandit_01` wears STK2's `bandit_01` textures; `neutral_01` wears STK2's
  `neutral_01_novice` (the Novice); `neutral_03` wears STK2's `neutral_01` (the
  loner). The textures are not in the export: they come from STK2's
  `resources/sources/textures/character/human` (`/mnt/toshiba/surv-builds/STK2.7z`,
  `xr2-prog`), uncompressed TGAs.
- `_s1`, `_s2` and `_s3` of a torso or trousers are byte-identical meshes (only their
  `mesh_name` differs): the names of the native variants (torso: `data_full`,
  `data_short`, `data_long`; trousers: `data_full`, `data_medium`, `data_long`), never
  cut. Each piece is built here in those variants. `_hud` hands are the third-person
  hands. `torso_v2` is another jacket (two belt pouches in place of the canteen, knife
  and pouch of `torso_s*`'s attachment).

## Not built

- Already shipped: `bandit_01_head_01`'s balaclava (1810), `bandit_01_head_02`'s
  (1823), the Novice's knit cap (1821, from STK2's whole `neutral_01_novice`), the
  `duty_03` set (the exoskeletons).
- Donor faces with no headwear: `bandit_01_head_03`, `neutral_03_head`, the face under
  the knit cap of `neutral_01_head`, the eyes and faces under the balaclavas
  (`bandit_01_head_0{1,2}`'s `head_s`).
- Bare skin: `bandit_01_hand` and `hand_v2` (bare hands, one with a watch and a
  bracelet on its attachment surface).

## What the builder does

- Reads each part, turns it onto the native rig, checks the rig.
- Maps: each cut to the power-of-two window its surfaces use (the bandit's attachment
  atlas: its chain only), its alpha dropped (a cutout the character material does not
  read: it blacked out the sneakers), halved to at most 1024 texels (the native
  characters' size), compressed as the Novice cap's (DXT1, DXT5 normal map, the tank
  torso's material with the matte cloth finish). The Novice jeans' waist, painted
  black where STK2's jacket covered it, is painted over with the denim below it,
  reflected (`Map::black_band`).
- Fits, against the released game's pieces (stock db and patches) and each other's,
  accessories first:
  - gloves (all wrist-length, `data_full`): the cuffs inside every stock
    wrist-length sleeve;
  - boots: all three outfits' are `data_full`, inside every full-length hem: the
    sneakers, the loner's (low shoes: the export's left boot also carried the bottom of the
    camo pants' leg, a wrapped cuff on the pants' part of the map; it is replaced by the
    right shoe mirrored, its bones swapped Left/Right) and the Novice's
    (26 cm, worn under the jeans as in STK2: the top of its shaft is painted black
    where they covered it). A `data_medium` or `data_long` boot would be fitted over
    every tucked leg of its class (`BOOTS_OVER_LEGS`);
  - trousers: `data_full` lengthened where the hem stands above 12 cm (the Novice's
    and the loner's end at 23-29 cm over their own boots: a gap over low boots), then
    the hems over every low boot; `data_medium` and `data_long` into every boot of the
    class, cut 3 cm past the deepest boot opening;
  - jackets: the waist over every trousers, the collar over the native neck and every
    mask (the Novice jacket's settings), the sleeves over wrist-length gloves and into
    gauntlets (cut 3 cm past the deepest opening). The bomber's sleeves are pushed up:
    below them each variant draws the native body's forearms
    (`base_character_torso_s1`/`_s2`/`_s3` from 5 cm below the elbow, with the native
    skin material), as the stock torsos with bare arms do: every stock glove's cuff is
    made for them. (Version 1 took the donor's forearms from `bandit_01_hand`; they
    stood through the gloves' cuffs in the game.)
  - the Novice jeans end at 0.937-0.964 m, where STK2's long jacket covered them; under
    0.26g's shorter jackets the open top showed the background. The top edge grows
    straight up to 1.04 m (the native legs' top ring, which every stock torso covers)
    at the jeans' own width: to 1.0 m its denim reflected up from below the edge and
    its skin blending into the native ring's, the last 4 cm a waistband with the native
    ring's skin, mapped to a strip of the map that is free (above the jeans, right of
    the boots) and painted there: the jeans' calf denim a shade lighter, a stitched
    seam along each edge and the belt line under it (`close_waist`, `grow`,
    `paint_waistband`; under 0.985 only, the other trousers end at 1.014 and 1.036);
  - the hood's bottom was open where STK2's collar covered it: its shell's own bottom
    hem grows down round the neck to the collar line (1.47 m, 1.44 in front), 6 mm off
    the native neck, and the two flap tips are joined across the throat with a seam
    down the middle; every new vertex takes the hood's canvas reflected across the edge
    it grows from, and its skin blends into the stock Scavenger hood's hem's
    (`grow_cowl`);
  - the hood covers the mask slot (as the Marauder, Molot and Zubr helmets do: its rim
    and flaps lie where every mask is) and is pushed out only where the native skull
    under it comes through (one point, 0.9 mm: 62 vertices, up to 5.3 mm);
  - the backpack is not fitted (its own two straps, from the export).
- Items: each a stock piece of the same slot at the owner's level copied whole (stats,
  level, price, drop weight, bonuses, card, sounds; the Novice's set level 4, the bandit's
  and the loner's 6, `LEVELS` where no stock piece is at it), with the model, icon and
  texts its own; no faction (0, the Independent traders': srp sells them nowhere, they
  only drop); camouflage and decals off (no masks); the hood's `additional_model_slots`
  `mask`.

## Rebuild

From a folder holding the export and the textures (the sandboxed ImageMagick reads only
below the working directory):

```
cargo run --release --example build-stk2-outfits-026 -- \
    --source-root export --textures stk2 \
    --target-db resources.db --patch resources.db.patch_12 ... --patch resources.db.patch_39 \
    --output bundles
python3 scripts/render-stk2-outfits-026-icons.py --bundle bundles/<outfit> \
    --maps bundles/textures-work --lobby-swf lobby_menu.swf --output icons-<outfit>
vostok-port pack from-bundle --base-db resources.db --base-patch ... \
    --bundle bundles/<outfit> --output stk2-<outfit>-2 --id stk2-<outfit> --version 2
```

`export` is the archive's `___surva_stk2_characters_port`; `stk2` holds STK2's
`resources/sources/textures/character/human/{bandit/bandit_01,neutral/neutral_01,neutral/neutral_01_novice}/*.tga`;
`lobby_menu.swf` is the released game's (patch 39's,
`72c0cd0faf9a9bf0b27340c487d6885254c80aa19f94a39cc60480ee2bb5768e`). The build is
reproducible: the same inputs make the same packs byte for byte.

## Icons

Framed as the stock icons of each slot are (the released atlas's medians: 81 px high,
boots 76, gloves 77, centred); the loner's shoes 20 % smaller. Trousers are cut as the
stock ones are: a hand below the knee, at 0.62 of the waist-to-floor height, then fitted
to the stock 81 px (the width follows: 55-64 px opaque, the stock median 61). The 0.62
is measured: each stock trousers model rendered with the icon camera and its silhouette
fitted to its icon's opaque body; of the 53 in the released atlas, the 27 that fit (IoU
over 0.85) are cut at 0.30-0.50 m, median 0.40 m, the rig's knee being at 0.48-0.50 m.
Every icon sits on the stock icons' faint black halo, even all round: measured on the
released atlas (248 armour icons), outside the opaque body its alpha falls from 60 at 2 px
to 34 at 4, 19 at 6, 10 at 8 and 4 at 10, which is the item's own alpha blurred (gaussian,
sigma 5 px) at 0.6 of full strength; ours at 0.55, a shade fainter (56, 35, 19, 10, 4).
The cells' top and bottom rows stay clear, cutting the halo there, as on stock.
The inside of the open waist (the faces above 0.95 m that
turn away from the camera) is left out of the trousers' icons: from the camera a little
above the waist it showed, unlit, as a black band over the front. Boots are seen level,
whole up to their own tops: of their black faces (the shaft top STK2's trousers covered)
those facing into the boot are left out and the outer ones take the colour of the faces
around them on the mesh. The sneakers are drawn as first rendered (the camera 6 degrees
above, nothing left out; the owner's pick), 28 % smaller. Gloves: one hand, its back
square to the camera (the hand's own plane, from its vertices), the outline's long axis
settled on 32 degrees above the horizontal with the fingers down to the left (the stock
glove icons' median, 25-41); the faces wholly on the cuff's black end band or inside it
(all three corners darker than 0.03) are left out, so the glove ends along an edge loop;
flaps left at that end go and the edge itself is smoothed; the faces part black at that
edge take the colour of the glove around them.

## Checked offline only

Bind-pose renders of every piece with stock pieces of each class (sleeves, cuffs, hems,
shafts, waist, collar, backpack, hood). None of the fits has been seen in the game or
through the animation clips.
