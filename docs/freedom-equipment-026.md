# Freedom exoskeleton for v0.26g (texture set and integration plan)

A Freedom look for the ported STK2 Duty exoskeleton (`duty-equipment-026.md`).
This change adds only the texture tooling in `scripts/freedom_026/`. The
builder (`src/duty_026.rs`) and the Duty items are unchanged. The plan at the
end applies once the Duty body/helmet split (body in the torso slot, helmet
1800) has landed.

Owner's decisions (2026-10-16):

* The look is **`freedom-bright`**: dark paint lifted to the reference's
  brightness. `freedom` keeps the source lightness, and it stays available
  as an alternative.
* **No chest disc.** The Freedom set omits the `sign_s` surface (the brass
  ring with the red star) entirely. The torso texture under it is an
  ordinary, continuous plate with the strap end. It has no hole and no
  mounting mark, so nothing needs filling. The star-free `dol_sign` textures
  below are kept only for a possible disc-keeping variant. The builder does
  not use them for Freedom.
* **Fringe Settlers, slightly different stats**: still heavy, +3% movement
  speed and −4% stamina use, no pocket capacity, other utility as Duty's, ballistic protection
  0.85 / 0.70 and anomaly protection 0.80 (the armour is provisional). See
  "Faction" and "Characteristics" under the integration plan.

## What changes

| STK2 texture (`.work/duty-026-inputs/.../dolg`) | Format | Change |
| --- | --- | --- |
| `dolg_exo_torso.tga` | 2048² TGA, BGRA 32-bit, bottom-up; alpha all 255 | red paint → Freedom yellow; Duty patch replaced |
| `dolg_exo_torso_nmap.tga` | 2048² TGA, RGBA (A unused by the builder) | patch relief only (texels 690–781 × 235–343) |
| `dolg_exo_legs.tga` | 2048² TGA, BGRA; alpha = cutout | red stripes and belt tab → yellow |
| `dolg_exo_helmet.tga` | 2048² TGA, BGRA; alpha = cutout | red hose → yellow |
| `dold_exo_attach.tga` | 2048² TGA, BGRA; alpha = cutout | red canisters and markers → yellow |
| `dol_sign.tga` | 512² TGA, BGRA; alpha = disc cutout | unused by Freedom (disc omitted); red rim edge → yellow, star painted out |
| `dol_sign _nmap.tga` | 512² TGA, RGB 24-bit | unused by Freedom; star relief painted out, knurl continued |

Every other file is unchanged: the `*_nmap` normals for legs, helmet and
attach, and the `fmap`/`gmap`/`spec`/`smap`/`mmap` maps, which the builder
does not read. Rewritten TGAs keep the source header, the TGA 2.0 footer, the
size, the pixel order and every alpha byte. Only colour bytes change. The
builder converts them exactly as it does Duty's. The DDS files it makes
(DXT1 diffuse, DXT5 normal, the same header, mip count and size as Duty's)
are byte-identical to Duty's for the four unchanged normals.

Where the red is:

* Fabric accents (sleeve and leg stripes, the belt piping, glove stitching):
  OKLCh hue about 12–20°, C/L about 0.13.
* Faction paint on metal (shoulder pads, the round plate, the mask hose, the
  back canisters, the frame markers): hue about 30–41°, C/L about 0.19, with
  grey paint chips.
* The Duty patch: torso texels 701–770 × 246–332 (UV 0.3423–0.3765 ×
  0.1201–0.1626). A shield with its top edge at −6.9°, on the right upper
  sleeve. Its UV island is upright and not mirrored on the model: "ДОЛГ"
  reads correctly in the lobby.
* The red star: `dol_sign` is the chest disc, a copper star in a brass ring.
  The STK2 `dol_sign.psd` has the star shape as layers 3 and 4. Those layers
  give the exact mask. The disc is its own render surface, `sign_s` of
  `duty_03_torso_s1`, with its own material `{NS}/sign`. The Freedom set
  drops the whole surface.
* Not faction colour, and left alone: bare fingers on the torso atlas
  (excluded by rectangles), rust and brown dirt (hue 50–80°, excluded by hue),
  and the crotch armour's brown grime (excluded by rectangle, except its red
  belt tab).

STK2 has no Freedom (Svoboda) assets. Its human factions are `dolg`, `bandit`
and `neutral` (`.work/stk2-full-inventory.txt`).

## Method

`make_freedom.py SRC SIGN_LAYERS PATCH OUT [--gain G]` takes about 15
seconds and writes `OUT/textures` as a drop-in `--texture-dir`: changed TGAs
plus symlinks to the STK2 sources for the rest. It also writes the masks and
`stats.json`.

* Paint mask: a smooth OKLCh hue window (−28…−16° fading in, 44…52° fading
  out) × a chroma ramp (0.014–0.030) × a lightness ramp, with spatial
  exclusions, blurred 0.6 px. Grey chips and dirt have no chroma, so they
  stay out of the mask.
* Recolour: OKLab lightness L is kept per texel. The hue is set to the
  reference hue, and 35% of the texel's deviation from the texture's median
  hue is kept. C/L is scaled so the texture's median C/L reaches the
  reference value; per-texel variation is kept, the result is capped at 1.5×
  the target and gamut-fitted by reducing chroma. The result is blended in
  linear RGB by the mask. `--gain` (bright variant 1.5) lifts dark paint
  only: `L + (gL − L)·clamp((0.75 − L)/0.35)`.
* Target yellow, from the S.T.A.L.K.E.R.: Call of Pripyat Freedom exoskeleton
  (`sample_ref.py`; ten painted regions with saturated amber pixels only):
  the median of the region medians is sRGB #A05C26 (160, 92, 38), linear
  (0.3515, 0.1070, 0.0194), HSV 26.7°/0.76/0.63. Lit panels are about
  #DE872E. The pooled OKLCh is h 54.9°, C/L 0.206, L 0.568.
* Star removal: hole = star layers dilated by 3 px inside the dark disc. Low
  frequencies come from a rotationally symmetric model of the disc: the
  radial profile for colour, and radial f(r) plus tilt for the normal XY.
  High frequencies are quilted from the visible disc. In the
  normal map they are aligned to the knurl's own repeat lattice, (3,2) and
  (7,−7), found by autocorrelation, so the cross-hatch continues without
  seams.
* Patch: the Duty shield (the convex hull of its red embroidery) is removed
  with the same filler. The sleeve weave has lattice (3,−4)/(3,4), so its dot
  grid continues. The Freedom patch is placed with the same area, centre and
  tilt: 80×80 texels at (698, 244). It is resampled at 4× with a slightly
  roughened, softened outline and texel-scale thread blur. It is graded
  toward the Duty patch's wear (lightness ratio to Duty's median, chroma ×
  ≤0.85). It takes the cloth's folds and dirt from the patch-free fabric,
  8% weave dust and a drop shadow. Normal map: a raised patch plus an
  embroidery height from its luminance, scaled (strength and green sign) to
  the Duty patch border's measured relief and added onto the patch-free
  fabric normal.
* Previews: `render_preview.py` uses the icon rasteriser's mesh loading with
  bilinear sampling and output mirrored to match the game. Note:
  `scripts/render-duty-026-icons.py` renders mirrored. The chest disc and
  mask canister swap sides compared with the lobby, which only matters if
  someone reads text on an icon.

## Sources and licence

See `/mnt/hdd/Projects/freedom-exo-20261016/SOURCES.md`. The emblem is the
official S.T.A.L.K.E.R. 2 Freedom patch (256×256), from the S.T.A.L.K.E.R.
Fandom wiki, `File:Patch_s2_freedom.png`. The wiki page says a GSC community
manager provided it through Discord. No licence is given. It is GSC Game
World IP, like the STK2 assets themselves. It is fine for a private fan
server, but it is not licensed for redistribution.

## Integration plan (after the Duty split lands)

**Done (2026-10-16)**: `duty_026::FREEDOM` (namespace `ported/freedom_026`,
body 1807, helmet 1808, no `sign_s`, per-variant `reusable` IDs and
`check_reused_id`). Build it with
`build-duty-026 --variant freedom --texture-dir '.../freedom-bright/textures'`
against the stock database, then `pack from-bundle` as `freedom-exo` and add
the `atlas_icons_add` cells (4, 6) and (5, 6) with `icon-body.png` and
`icon-helmet.png`. The materials take the shared `outfit_026::Finish`. The
plan below is kept as written.

Recommended: a **new, separate item set** next to Duty. Duty stays unchanged.

| Item | ID | Slot | Config | Dictionary name | Icon |
| --- | --- | --- | --- | --- | --- |
| Freedom exoskeleton | 1807 | torso (2), category 3 | `gameplay/items/armour/torso/freedom_exo_026.options` | `freedom_exo_suit` | [4, 6] |
| Freedom exoskeleton helmet | 1808 | helmet (0), category 1 | `gameplay/items/armour/helmet/freedom_exo_026.options` | `freedom_exo_helmet` | [5, 6] |
| Freedom gloves | 1809 | gloves (5), category 6 | `gameplay/items/armour/gloves/freedom_exo_026.options` | `freedom_exo_gloves` | [8, 6] |
| Freedom boots | 1811 | boots (6), category 7 | `gameplay/items/armour/boots/freedom_exo_026.options` | `freedom_exo_boots` | [9, 6] |

Since v3 (owner, 2026-10-17) the gloves and boots are pieces of their own,
on Pilgrim's (`edge_gloves_6`, `edge_boots_6`): 0.4 kg and 1.9 kg, 23 750
each, drop weight 10, no modifications. The body (26.2 kg, 177 500) covers
the back and legs only; the set stays 33 kg and 300 000. 1809 and 1811 are
free in stock and patches 12 and 14 (1810 is the balaclava). Their icons are
Duty's v7 gloves and boots thumbnails with the Freedom recolour (gain 1.5;
the gloves' bare fingers excluded).

Stock items end at 737. Duty owns and retires 1800–1806. The Bandit pack
uses 1810–1814 (balaclava, jacket, jeans, hands, shoes). 1807 and 1808 are
free, and they are outside the range Duty's builder deletes. Covered slots,
oxygen (30 s), level 15, cost and the drops (every piece as its
template, 10) copy Duty's `BODY`/`HELMET`.
The faction and the characteristics differ (next section). 0.26g has no
Freedom faction. Owner's decision (2026-10-16): the Freedom exo belongs to
the **Fringe Settlers**.

### Faction: Fringe Settlers (owner's decision, 2026-10-16)

Read from 0.26g's `resources.db` (`gameplay/db_static_dictionaries`
`factions_dict`, `localization/english`, the item configs) and from srp
(`crates/survarium/src/v026g/prices.rs`).

| | Duty 1805/1800 | Freedom 1807/1808 |
| --- | --- | --- |
| Faction (`parameters.faction`) | 3, `st_renaissance_faction` "The Renaissance Army", from the templates `renesanse_torso_6` / `renesanse_helmet_6` | **4**, `st_border_faction` "The Fringe Settlers" (srp `faction::EDGE`), from the templates **`edge_torso_6` / `edge_helmet_6`** (the "Pilgrim" vest and helmet) |
| `item_level`, `match_making_level` | 15, 15 | 15, 15 |
| Rank needed in the shop | 10 "Colonel" (`st_renaissance_level_10`, 35 250 reputation) | **10 "Magister"** (`st_edge_level_10`, 35 250). Rank 9 is "Keeper" (29 340). |
| How srp lists it | seller = config `faction`, level = `item_level − 5` (`seller_level`, Renaissance and Edge); no `server.items` sellers entry | the same rule gives seller 4 at level 10, so no override is needed. To be explicit, the pack's `server.items` can say `"sellers": [[4, 10]]` |
| `faction_affinity` | body 30, helmet 20 (template) | body 30, helmet 20 (template; the same values) |
| Drops | `drop_weight` 10 for every piece, as Zubr UM-4's | 10 for every piece, as Pilgrim's |
| Faction access | Renaissance opens only after a basic faction's top rank (srp `rules.rs` `faction_unlocked`) | the same rule for the Fringe Settlers |

The level-15 native sets: "Zubr UM-4" (Renaissance, `renesanse_*_6`,
dict 316–320, 287 500 for five pieces) and "Pilgrim" (Fringe Settlers,
`edge_*_6` and the container `edge_back_6`, dict 310–315, 310 000 for six).
Duty's balance was already set between the two.

### Characteristics (proposal)

The owner's direction: still a heavy exo, a bit faster, more utility,
armour a bit below Duty, and anomaly protection a bit below Pilgrim.

What the native sets emphasise:

* **Renaissance (Zubr):** ballistic 0.90 (head 0.75) and anomaly 0.50. It
  is heavier (torso 20, boots 3, helmet 6). Its utility is ammo: torso +2
  ammo bags and pocket size +1, legs +1 pocket and +1 ammo bag.
* **Fringe Settlers (Pilgrim):** ballistic 0.85 (head 0.70) and anomaly 0.90.
  It is lighter per piece (torso 17, boots 1.9, helmet 5). Its utility is
  artefacts and sustain: torso +1 artefact slot, container +1 artefact slot
  and +15% health regeneration, legs +1 pocket.
* **Lower-tier Fringe gear:** level-14 `edge_*_5` adds mobility and handling,
  for example boots +3% movement speed (1112) and gloves −4% dispersion
  (1071). Renaissance level-14 boots instead have −2% stamina spending
  (1160).
* **Set bonuses: none.** 0.26g has no set id or set bonus: no such field in
  any item config or dictionary. A "set" is only same-faction, same-level
  pieces.

Final values (2026-10-16): **user-approved; the armour numbers are
provisional** (the owner may still tweak them). Compared with Duty, the
Freedom set has 2 points less ballistic protection (equal to Pilgrim),
10 points more anomaly protection (still 10 below Pilgrim), and is 2 kg
lighter. Its utility changes are mobility, +3% movement speed and −4%
stamina use, and no pocket capacity (Duty's 1170 is left out; owner,
2026-10-16). Quick slots, ammo bags and the artefact slot stay as Duty's. There is no health regeneration and no second artefact slot (an
earlier proposal had them; the owner removed them). The set's oxygen stays.

| Stat (config field) | Zubr UM-4 | Pilgrim | Duty | **Freedom (final)** |
| --- | --- | --- | --- | --- |
| **Body**: template | `renesanse_torso_6` | `edge_torso_6` (+ legs, gloves, boots, back) | `renesanse_torso_6` | **`edge_torso_6`** |
| `parameters.weight` (body pieces) | 28.6 (20 + 5 + 0.6 + 3) | 30.3 (17 + 4 + 0.4 + 1.9 + 7) | 30 | **28.5** |
| Ballistic: `armor`, `arms_armor`, `back_armor`; `hit_params.*.{ranged,slash,explosive}.armor` | 0.90 | 0.85 | 0.87 | **0.85** (provisional) |
| Anomaly: `ano_armor`; `hit_params.*.{fire,shock,radiation,toxic}.armor` | 0.50 | 0.90 | 0.70 | **0.80** (provisional) |
| Quick slots `pocket_slots_modifier` | +1 (legs, 1030) | +1 (legs, 1030) | +1 (1030) | +1 (1030) |
| Pocket size `pocket_slot_size_modifier` | +1 (1170) | — | +1 (1170) | **—** |
| Ammo bags `ammo_bags_modifier` | +3 (1051, 1050) | — | +1 (1050) | +1 (1050) |
| Artefact slots `artefact_slots_modifier` | — | +2 (1040 torso, 1040 back) | +1 (1040) | +1 (1040) |
| Health regen `health_regeneration_speed_modifier` | — | +0.15 (1094, back) | — | — |
| Movement `movement_speed_modifier` | — | — | — | **+0.03 (1112)** |
| Stamina `stamina_spending_speed_modifier` | — | — | — | **−0.04 (1161)** |
| Oxygen (`data.amount_time_sec`) | — | — | 30 s | 30 s |
| Cost (`base_cost`, body pieces) | 210 000 (4 pieces) | 237 500 (5 pieces) | 225 000 | 225 000 |
| **Helmet**: template | `renesanse_helmet_6` | `edge_helmet_6` | `renesanse_helmet_6` | **`edge_helmet_6`** |
| `parameters.weight` | 6 | 5 | 5 | **4.5** |
| Ballistic (brain, face) | 0.75 | 0.70 | 0.72 | **0.70** (provisional) |
| Anomaly (brain, face) | 0.50 | 0.90 | 0.70 | **0.80** (provisional) |
| Modifications | — | — | — | — |
| Cost | 77 500 | 72 500 | 75 000 | 75 000 |
| **Set total weight** | 34.6 | 35.3 | 35 | **33** |

Native costs: Zubr torso 105 000, legs 52 500, gloves 26 250, boots
26 250, helmet 77 500. Pilgrim torso 95 000, container 47 500, legs 47 500,
gloves 23 750, boots 23 750, helmet 72 500. Zubr has no level-15 back
piece. Set totals: Zubr 287 500, Pilgrim 310 000, Duty and Freedom 300 000.

Why it is about the same power as Duty:

* Ballistic protection carries the most weight in PvP, and Freedom gives up
  2 points of it on all 12 regions. In exchange it gets 10 anomaly points,
  which matter only in anomalies, fire and gas.
* Mobility, modestly: 2 kg less (the set is still over the 30 kg
  `max_carried_weight` of `default.player`, so it stays a heavy suit with an
  overweight penalty), +3% movement speed (the same modification as the
  level-14 Fringe boots), and −4% stamina use.
* No pocket capacity (1170). Every other utility value is Duty's.

How the modifications reach the game: the client applies an item's
`default_modifications` from its own config tables. srp's `modifications.rs`
notes they are not on the wire, and the lobby's "rank" counts only rolled
ones. 1112 is `st_modify_movement_speed`,
`movement_speed_modifier` +0.03. Native gear uses it as a default:
`edge_boots_5`, the level-14 Fringe boots. 1161 is
`st_modify_stamina_spending` −0.04, the same type as `renesanse_boots_5`'s
1160. Six defaults is within the eight that srp's `item_params.rs` test
allows native items. The deployed `duty-exo` pack v13 (the back-slot
suit) still carried 1031, 1114 and 1165. The current split drops speed,
stamina and carry bonuses on purpose. Freedom brings back smaller ones, as
its faction's style, with only speed and stamina.

Per-variant values the builder needs (in addition to the namespace):

* `SetPiece.template`: `gameplay/items/armour/torso/edge_torso_6.options`
  and `gameplay/items/armour/helmet/edge_helmet_6.options`. They give
  faction 4, `faction_affinity` 30/20, the dictionary row's `item_category`
  3/1, and `edge_helmet_6`'s mask cover.
* `SetPiece.weight` 28.5 / 4.5; `cost` 225 000 / 75 000.
* `SetPiece.modifications` (body) `[1030, 1050, 1040, 1112, 1161]`;
  helmet `[]`.
* The protection now hard-coded in `split_protection` and `duty_set`
  (0.87 / 0.72 ballistic, 0.70 anomaly) becomes variant fields: Freedom
  0.85 / 0.70 and 0.80 (provisional). `ui_desc.props_list` (prop 6 armour, 8 anomaly, 15
  weight) follows them.
* The Zubr pieces stay the source of the 12 hit regions, with the same
  region names in Pilgrim.

Changes to `src/duty_026.rs`:

1. Add a variant descriptor, for example
   `struct Variant { ns, asset_id, set: [SetPiece; 2], reusable: RangeInclusive<i32>, strings: [(&str, &str); 4], chest_disc: bool, ballistic: (f32, f32), anomaly: f32 }`,
   with `DUTY` (the current values) and `FREEDOM`: `ns =
   "ported/freedom_026"`, `asset_id = "freedom-exoskeleton-026g"`,
   `reusable = 1807..=1808`, `chest_disc = false`, `ballistic = (0.85,
   0.70)`, `anomaly = 0.80`, and pieces with the Fringe templates, weights
   and modifications above. Pass it into `build()`.
2. Use the variant's namespace wherever `NS` is used today: the model paths
   (`base_model`, `base_model_hud`, `models/{NS}/…`), every surface's
   `material_name`, `material_instances/{NS}/{part}.material`, and
   `textures/{NS}/{part}_{role}`. Freedom needs its own copy of the six models
   because each surface names its material. Meshes are small; the textures
   (about 34 MB of DDS) dominate either way.
3. Make the retire loop (`for id in 1800..=1806`, `items.retain`) and
   `check_reused_id` use the variant. Freedom may replace only its own
   earlier rows, never Duty's. The legacy split-preview branch stays
   Duty-only.
4. Strings: `st_freedom_026_suit_name` "Freedom exoskeleton",
   `st_freedom_026_suit_desc`, `st_freedom_026_helmet_name` "Freedom
   exoskeleton helmet", `st_freedom_026_head_desc`. Suggested descriptions:
   "An experimental military exoskeleton repainted in Freedom colours. Never
   mass-produced due to its cost and design flaws; small batches still leave
   underground workshops. Comes with a built-in artifact container." and
   "The sealed helmet of the Freedom exoskeleton, with a built-in respirator.
   Covers the whole head and face."
5. Omit the chest disc when `!variant.chest_disc`. Three places use it
   (line numbers are at 6598c20, `fix/duty-exo-body-slot`):
   * `("sign_s", "sign")` in the `torso_s1` surface list of `parts` (~682).
     The model then has no `sign_s` LOD0 surface, no `material_settings`
     entry and no copied `render/sign_s/converted_model`.
   * `model("torso_s1", "sign_s")` in the body's `data.additional_models`
     (~615).
   * `("sign", "dol_sign", "dol_sign _nmap")` in `texture_sets` (~849). This
     gives no `{ns}/sign` material and no `sign_diffuse`/`sign_normal` DDS
     (about 0.5 MB less).

   This drops the surface rather than making it transparent. The cloned
   native material has `texture_alpha_influence` 0 and no alpha test, and
   the diffuse is DXT1. A transparent `dol_sign` alpha would not hide the
   disc. Add a test that the Freedom body's `additional_models` and
   `torso_s1` surfaces have no `sign_s`.
6. Material settings, including the other worker's gloss/sparkle changes,
   are built per part in the same loop, so Freedom inherits them unchanged.
7. `examples/build-duty-026.rs`: add `--variant duty|freedom` (default
   `duty`).
8. `scripts/render-duty-026-icons.py`: add cells (4, 6) and (5, 6). The icons
   are ready: `.work/freedom-026-inputs/freedom-bright/icon-body.png` and
   `icon-helmet.png`. They are the Duty v7 icons with the same recolour and
   the chest disc painted out (`icon_star.py … 20.6 71.3 5.2 disc`). Pass
   them with `--icon`. If the icons are re-rendered from meshes instead,
   leave out `sign_s`.

Build and pack (Freedom is its own pack, composed with Duty and Bandit):

```sh
target/release/examples/build-duty-026 --variant freedom \
  --source-root "$HOME/Downloads/___surva_char" \
  --texture-dir .work/freedom-026-inputs/freedom-bright/textures \
  --target-db .work/duty-026/resources-v10-base.db \
  --icons-swf <lobby_menu.swf with cells 4,6 and 5,6> \
  --output .work/freedom-026/bundle
vostok-port pack from-bundle --base-db <0.26g>/game/resources.db \
  --build-base-db .work/duty-026/resources-v10-base.db \
  --bundle .work/freedom-026/bundle --output .work/packs/freedom-exo \
  --id freedom-exo --version 1
```

`freedom/textures` is the darker, source-lightness alternative. The two looks differ
only in the five diffuse TGAs. srp needs no code. If Duty has a
`server.items` sellers entry, give 1807 and 1808 the same entry.

Alternative: a replacement, where Duty becomes Freedom. Build the Duty variant
with `--texture-dir .work/freedom-026-inputs/freedom/textures` and change
its four strings. This loses the Duty look and is not recommended.

## Validation still open

* Not yet seen in the lobby: an in-lobby check is a runtime step (one
  headless client on Xvfb, the lobby only, no match).
* The patch is legible at the sleeve's lobby size in offline renders. Check
  the lobby's mip level in game.
