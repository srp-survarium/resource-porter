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
  bracelet on its attachment surface). The bomber takes only their forearms.

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
  - boots: low ones (`data_full`: the sneakers, the loner's boots, one of which is low
    and the other wrapped in a rag) inside every full-length hem; the Novice's
    (`data_medium`, 26 cm) over every tucked `data_medium` leg;
  - trousers: `data_full` lengthened where the hem stands above 12 cm (the Novice's
    and the loner's end at 23-29 cm over their own boots: a gap over low boots), then
    the hems over every low boot; `data_medium` and `data_long` into every boot of the
    class, cut 3 cm past the deepest boot opening;
  - jackets: the waist over every trousers, the collar over the native neck and every
    mask (the Novice jacket's settings), the sleeves over wrist-length gloves and into
    gauntlets (cut 3 cm past the deepest opening). The bomber's sleeves are pushed up:
    its bare forearms (from `bandit_01_hand`, without the hands) go into every glove,
    only cut;
  - the hood covers the mask slot (as the Marauder, Molot and Zubr helmets do: its rim
    and flaps lie where every mask is) and is pushed out only where the native skull
    under it comes through (one point, 0.9 mm: 62 vertices, up to 5.3 mm);
  - the backpack is not fitted (its own two straps, from the export).
- Items: each a stock piece of the same slot and faction copied whole (stats, level,
  price, drop weight, bonuses, card, sounds), with the model, icon and texts its own;
  camouflage and decals off (no masks); the hood's `additional_model_slots` `mask`.

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
    --bundle bundles/<outfit> --output stk2-<outfit>-1 --id stk2-<outfit> --version 1
```

`export` is the archive's `___surva_stk2_characters_port`; `stk2` holds STK2's
`resources/sources/textures/character/human/{bandit/bandit_01,neutral/neutral_01,neutral/neutral_01_novice}/*.tga`;
`lobby_menu.swf` is the released game's (patch 39's,
`72c0cd0faf9a9bf0b27340c487d6885254c80aa19f94a39cc60480ee2bb5768e`). The build is
reproducible: the same inputs make the same packs byte for byte.

## Checked offline only

Bind-pose renders of every piece with stock pieces of each class (sleeves, cuffs, hems,
shafts, waist, collar, backpack, hood). None of the fits has been seen in the game or
through the animation clips.
