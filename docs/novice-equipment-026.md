# STK2 Novice cap and jacket for 0.26g

Two items from the cancelled S.T.A.L.K.E.R. 2's novice stalker
(`neutral_01_novice`), on the native v0.26g player:

| id | slot | item | config | model |
| --- | --- | --- | --- | --- |
| 1821 | helmet | Novice's knit cap | `gameplay/items/armour/helmet/novice_026.options` | `ported/novice_026/hat` |
| 1822 | torso | Novice's field jacket | `gameplay/items/armour/torso/novice_026.options` | `ported/novice_026/torso`, `_short`, `_long` |

Build (the bundle is made over the released game: stock plus every released
patch, so its dictionary and texts are those plus the two items):

```
cargo run --release --example build-novice-026 -- \
    --source-root STK2 --target-db resources.db \
    --patch resources.db.patch_12 … --patch resources.db.patch_22 --output bundle
python3 scripts/render-novice-026-icons.py --bundle bundle \
    --textures STK2/resources/sources/textures/character/human/neutral/neutral_01_novice \
    --lobby-swf lobby_menu.swf --output icons     # patch 22's movie, 7bf0a4c5…
vostok-port pack from-bundle --base-db resources.db --base-patch … --bundle bundle \
    --output novice-1 --id novice --version 1
```

Run it from a folder that holds the STK2 sources: the sandboxed ImageMagick
reads only below the working directory.

## The cap

- The donor's head surface is his face and the cap in one. Only the cap's own
  piece is kept (its UV band at the top of the head texture, v ≤ 0.375).
- It is worn on the native head, not with the donor face. The donor face is
  laid onto the native head (trimmed ICP, translation only: the cap keeps its
  size); the cap moves with it, 3.8 cm down and 4 cm back.
- Then it is pushed out, smoothly, where the native head (4 mm) or any mask
  in the game (1.5 mm) would show through: the stock masks and the released
  Bandit balaclava. A mask part standing well in front of the hem (a brow
  plate) stays in front of it. Pushes are at most 11.5 mm (over the Keeper
  mask's top), and a push over 15 mm fails the build.
- It takes the native head's skin weights, and is checked through the
  balaclava's 19 lobby and match clips: the head and every mask stay under it.
- Matte knit (`Finish::CLOTH`).

## The jacket

- The donor's torso surface without his hands: the gloves slot draws those.
  The hands are cut by texture (the gloves' UV block, on the arms), not by
  weight: the weight rule takes the forearm roll bone for a hand and cut the
  sleeves short.
- His collar was made for a head that stood higher: it is lowered 2.9 cm, from
  the shoulders up, to a stock high collar's height.
- Pushed out (never in) over every trousers' waist in the game, 4 mm clear.
  Holsters and pouches that stand more than 2.5 cm out are left crossing the
  hem, as stock long jackets cross them (Quilted jacket, Reinforced padded
  jacket).
- The collar is pushed out over the native neck and the masks' neck hems,
  3 mm clear, 1.2 cm at most; parts in front of the throat (respirators, a
  mask's jaw) stay in front of it.
- One variant per glove class, as stock torsos: sleeves over wrist-length
  cuffs (`data_full`), into short and long gauntlets (`data_short`,
  `data_long`). Inside gauntlets the sleeve ends 3 cm past the deepest
  opening of its class, as the stock gauntlet variants are cut short: deeper
  in, it could only show through the gauntlet.
- Backpack straps sink into the shoulders about as much as on stock jackets.
- Matte cloth.

## Stats

Both are the Scavengers' (faction 1), level 2: the server's rule sells them at
the Scavengers' second rank. No seller override.

| | level | armour | isolation | kg | price | drop weight |
| --- | --- | --- | --- | --- | --- | --- |
| Novice's knit cap (1821) | 2 | 10 | 10 | 0.3 | 2,000 | 140 |
| Mounting helmet (330, Scav) | 4 | 20 | 5 | 1.0 | 8,080 | 120 |
| Biker's helmet (27, BM) | 3 | 20 | 0 | 1.2 | 4,730 | 130 |
| Amber respirator (43, Scav mask) | 3 | 5 | 30 | 0.5 | 1,190 | 130 |
| Novice's field jacket (1822) | 2 | 20 | 10 | 4.5 | 3,300 | 140 |
| Quilted jacket (31, Scav) | 1 | 10 | 10 | 3.0 | 1,900 | 150 |
| Leather sleeveless jacket (29, BM) | 1 | 15 | 5 | 3.0 | 2,100 | 150 |
| Chemical protection jacket (34, Scav) | 3 | 20 | 50 | 6.0 | 4,750 | 130 |
| Survivor's jacket (325, Scav) | 3 | 30 | 10 | 5.0 | 4,750 | 1 |

Icons: [18, 6] and [19, 6] of the equipment atlas (rows 10-17 of column 6 are
reserved for the 0.69d masks and the closed Keeper helmet).
