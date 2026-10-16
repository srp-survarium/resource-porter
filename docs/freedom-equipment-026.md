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

Recommended: a **new, separate item set** next to Duty. Duty stays unchanged.

| Item | ID | Slot | Config | Dictionary name | Icon |
| --- | --- | --- | --- | --- | --- |
| Freedom exoskeleton | 1807 | torso (2), category 3 | `gameplay/items/armour/torso/freedom_exo_026.options` | `freedom_exo_suit` | [4, 6] |
| Freedom exoskeleton helmet | 1808 | helmet (0), category 1 | `gameplay/items/armour/helmet/freedom_exo_026.options` | `freedom_exo_helmet` | [5, 6] |

Stock items end at 737. Duty owns and retires 1800–1806, and the Bandit
balaclava is 1810. 1807 and 1808 are free, and they are outside the range
Duty's builder deletes. Stats copy Duty's `BODY`/`HELMET`: template, covered
slots, weight, cost, modifications, protection split, oxygen,
faction/level 15 and `drop_weight` 0. Changing the faction is a separate
balance decision. The 0.26g factions have no Freedom.

Changes to `src/duty_026.rs`:

1. Add a variant descriptor, for example
   `struct Variant { ns, asset_id, set: [SetPiece; 2], reusable: RangeInclusive<i32>, strings: [(&str, &str); 4], chest_disc: bool }`,
   with `DUTY` (the current values) and `FREEDOM`: `ns =
   "ported/freedom_026"`, `asset_id = "freedom-exoskeleton-026g"`,
   `reusable = 1807..=1808`, `chest_disc = false`, and pieces as in the
   table above. Pass it into `build()`.
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
