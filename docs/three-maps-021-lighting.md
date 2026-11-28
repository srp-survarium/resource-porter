# Shared day-map lighting compatibility

`examples/lighting-021-pack.rs` makes the small shader pack described by
`ports/three-maps-021-lighting.json`. It reuses the fingerprinted original
shader adapters; it does not rebuild the map packs or change individual
objects. Version 4 has 88 guarded shader replacements and covers all five
ported 0.21 day maps: Rudnya, School, Vostok, Fort and Mamayev. The pack keeps
its historical `three-maps-021-lighting` identifier.

The acceptance target is a close match using stock 0.21 values and as few
renderer changes as practical, not pixel identity. The owner approved the
@3 Rudnya interior comparisons as the desired standard on 2026-11-17.
Further changes require a noticeable compatibility defect, not merely a
small numerical image difference. The final five-map sweep and native-map
isolation review remain pending.

| Pass | Resources / scope |
| --- | --- |
| Geometry fog | 1, original arithmetic and authored Rayleigh colour |
| Tone curve | 4 supported native variants, original dark-scene behavior; an exposure profile (a 0.69d scene) also in the 6 of post-process quality 0 and 1 |
| Luminance metering, first pass | 1 (`gather_log_luminance`), only with an exposure profile: its scene's logarithm of the luminance; native sum for every other scene |
| Luminance metering, last pass | 1 (`gather_exp_luminance`), only with an exposure profile: its scene's mean of the logarithms turned back (the geometric mean); native mean for every other scene |
| Sky and atmospheric lookup | 2, original sky and scattering equations |
| Clouds | 8, original lit/unlit programs |
| Ambient occlusion | 1, original quality-1 generator normalized for native application |
| God rays | 4, original radial falloff; native mask and application remain |
| Sun lighting | 4, original direct-sun diffuse/specular block; native foliage block remains |
| Particle fog | 24 supported batched variants, original fog interpolation |
| Water fog | 8 supported variants, original fog; native water surface lighting remains |
| Point/spot lighting | 32 programs, original diffuse/specular and shadow sampling; native shadow-map generation remains |

Build the example under `nix develop`, then run:

```sh
cargo build --release --example lighting-021-pack
target/release/examples/lighting-021-pack \
  ports/three-maps-021-lighting.json DONOR_DB STOCK_DB OUT LIVE_PATCHES...
```

Pass the published patches in mounting order. Both archives are pinned by
hash, and each shader adapter refuses an unknown native/source program.
Every replacement records the current layered native file's hash. The pack
requires the existing School/Fort and Rudnya map packs, and leaves their
files alone. The content manifest, rather than a hand-built patch directory,
composes it with those packs and the environment edit lists.

The environment lists select a whole day map using `mie_fog_base_height`:

| Donor project | Marker |
| --- | --- |
| `level_01/day` | -32281 |
| `level_02/day` | -32195 |
| `level_03/day` | -32187 |
| `level_05/day` | -32198 |
| `level_06/day` | -32196 |

Every day-mode alias of a level uses its same marker. Original authored fog
inputs must be preserved when enabling it. Other scenes keep the native
shader instruction bodies. These markers follow the existing adapter's
`-32021 - map_id` convention and are runtime selectors, not visual tuning
parameters.

## Exposure profiles (0.69d scenes)

A 0.69d map is lit for 0.69d's exposure, which follows each view's mean luminance between
the environment's minimum and maximum exposure values and holds outside them; 0.26g's tone
block follows the adapted mean between 0.05 and 2 only. `exposure_profiles` in the manifest
(`ExposureProfile` in `tone_map.rs`) gives such a scene the native tone block with the mean
clamped to its own range and its own key, behind its marker (`mie_fog_base_height`, a whole
number of at least 1000: 33000 plus the 0.69d level's number, 33100 for 0.69d's lobby). A floor equal
to the ceiling is a fixed exposure (the lobby's). The row's `floor`, `ceiling`
and `key` are the numbers resource-porter's 0.69d map builder prints in its notes ("tone:
...") for that map; what the curve then shows is made 0.69d's by the scene's own colour
grading texture, which the map's pack carries.

| Scene | Marker | floor | ceiling | key |
| --- | --- | --- | --- | --- |
| Rudnya v0.69 (0.69d `level_01` day, EV 6 to 12.5) | 33001 | 0.001151 | 0.104167 | 0.3367 |
| The lobby room v0.69 (0.69d `lobby_scene/day`, EV fixed at 9.5) | 33100 | 0.104167 | 0.104167 | 0.2625 |
| School v0.69 (0.69d `level_02` day, EV 11.5 to 14) | 33002 | 0.018414 | 0.104167 | 0.1232 |
| Vostok Radar Station v0.69 (0.69d `level_03` day, EV 10 to 12) | 33003 | 0.026042 | 0.104167 | 0.1232 |
| Cologne v0.69 (0.69d `level_08` day, EV 5 to 11; @20) | 33008 | 0.001628 | 0.104167 | 0.1232 |

The curve is `1 - 2^(-X)`: sm_4_0's `exp` is `2^x` and its `log` is `log2` (the native block
has no `log2(e)` before them), and a key is for that base. Up to @14 the map builder took the
curve as `1 - e^(-X)` for its key and its grading texture, so a scene showed 0.69d's look for
`ln(2)^(1/1.2)` = 0.74 of its light. @15 takes Rudnya's key from the builder that reads it in
base 2 (0.2334 to 0.3367, with its grading texture in rudnya-069d after @8). @17 takes School's
and Vostok's from that builder too (0.0854 to 0.1232 each, with school-069d@3 and vostok-069d@3,
whose grading textures it made), and the lobby room's: 0.1820 to 0.2625, the builder's own
number for `lobby_scene/day` (no live scene carries its marker: the shipped lobby predates the
0.69d rules; its grading texture comes with its rebuild).

A row's `sky` (1 when absent) scales the native sky behind its marker (`scale_sky`, after
`build_sky`): 0.69d's sky scatters its tinted sun times `sky_intensity`, 0.26g's Rayleigh sky
follows `sun_intensity^1.4`, so under the port's sun 0.69d's sky is that factor times 0.26g's
(resource-porter's 0.69d map builder prints it, `DonorSky::scale_over_026`). School v0.69 1.301,
Cologne v0.69 0.8589; Rudnya, Vostok and the lobby room 1 (no change to the program for them).

A row's `vignette_intensity` (none when absent) is 0.69d's environment value; its branch scales the
light before the curve by `1 / (1 + (k (u - 0.5))^2 + (k (v - 0.5))^2)^2`, k ten times it, as 0.69d's
`tone_mapping.ps` does (@18, @19). School v0.69 0.11, Vostok v0.69 and Cologne v0.69 (@20) 0.12;
Rudnya v0.69 none (it stays as it draws, the owner's ruling) and the lobby room none.

Unlike the 0.21 curve and the key profiles, an exposure profile is also drawn by the tone
program of post-process quality 0 and 1 (six paths, one program: the same tone block
without the vignette): under the native block such a scene is about ten times too bright.
The two depth-of-field programs (`__3__...10`, `__3__...11`) are not adapted, and need not
be: they never draw a scene without `use_bokeh_dof`. `stage_postprocess::execute` (0x1B2270
of the 0.26g exe) takes the tone effect from four at `stage+0xA4` by
`2 * dof + bokeh_template` (0x1B29F0..0x1B2A35), and `dof` is 1 only when the scene's
environment has `use_bokeh_dof` (`+0x248` of its environment properties, read at 0x34F5B5;
0 by default, 0x34D9A0) with `r_post_process_quality` above 0, or while the lobby's decal
close-up sets the byte at 0x1F1DA26 (0x265EA4, cleared at 0x265F2B: the lobby's scene, never
a map's). No 0.69d scene sets `use_bokeh_dof` (the converter writes the donor's environment,
which has no such property); 209 native and ported environments set it true (rain,
pre-rain, the ported STALKER and 0.11e maps), and no gameplay effect does. An exposure profile for a
scene that sets it would need these two programs adapted too.

### Metering

The mean the tone block follows is metered as 0.69d meters it (`metering.rs`). 0.26g's is
the arithmetic mean of 256 x 256 points of the frame: `gather_log_luminance` writes
`dot(clamp(c, 0, 1e6), luma) + 0.001` (no logarithm), eight 2 x 2 reductions
(`gather_luminance`, the last `gather_exp_luminance`) sum it and the last multiplies by
1/65536. 0.69d's is a weighted geometric mean: `sample_luminance.ps` writes
`w ln(max(luminance, 0.01))` and `w` of 64 x 64 points of its pre-exposed frame (`w` 1
within two half-heights of the frame's centre, 0 outside: nothing of a 16:10 frame is left
out, 0.1 % of a 16:9 one, its corners, and 18 % of a 21:9 one, its sides), six reductions
average both, and the game takes `exp(x / w)` and divides it by the exposure the frame was
drawn with (`hdr_downsample_stage`, 0x20051CCBE of the retail exe; 0x200504640 and
0x200504517). A dark room with a sunlit doorway is so exposed for the room by 0.69d and for
the doorway by 0.26g: the church of Rudnya drew at a mean of 4/255 where 0.69d shows 52/255.

With an exposure profile the three programs of the chain are replaced. For the profile's
scene the first writes `w log2(max(luminance, floor))` into x and `w` into w (the frame's
aspect from its own size, `resinfo`), the reductions sum w as they sum x, and the last
writes `2^(x / w)`: the weighted geometric mean of the frame in 0.26g's light scale, the
quantity 0.69d's exposure value is the logarithm of, so the profile's `floor`, `ceiling`
and `key` stay as they are. y and z (the least and the greatest luminance) and every other
scene keep the native instructions (and w its native 0).

- `floor` is 0.69d's 0.01, which is of its pre-exposed frame: `0.01 * Lc / ceiling` in
  0.26g's scale, with `Lc` the profile's clamped mean. The first program does not have the
  adapted mean (its pass binds the frame alone, and a texture is bound only to the program
  current when the stage sets it: `backend::set_ps_texture`, 0x11B630, looks the name up in
  the bound program's reflection, 0x3D6960), so it uses the least of them,
  `0.01 * floor / ceiling` (Rudnya v0.69: 0.00011), and meters a point darker than 0.69d's
  floor of the moment below what 0.69d meters it at: the mean is then lower than 0.69d's,
  by `(Lc / floor)^p` at most for a part `p` of the frame that dark. Exact needs the stage
  to bind the adapted luminance in that pass (an executable change).
- Where no weight reaches the last program (a target that dropped w), it divides by 65536:
  the unweighted mean, never a division by 0.
- The programs had no constant buffer, so the marker's `static_globals` is their first and
  is declared at b0, not b1: the game binds a program's buffers by their order in its
  reflection (`constants_handler<1>::apply`, 0x3D5A30 of the 0.26g exe), and every one of
  the 14745 stock programs has each buffer at its own index.
- The two depth-of-field tone programs, not adapted, would read the same mean for such a
  scene; they never draw one (above).

`tools/content/three_maps_021_environment.py` generates the two source edit
lists from the donor and canonical map packs. It verifies authored numeric
colour/intensity fields have not been calibrated in the packs, preserves
them, converts cloud coordinates/defaults, and supplies original defaults
for omitted height-ambient colours. Original constructor 0x215157 initializes
low RGBA at +178h to (0,0,0,1), high RGBA at +188h to (1,1,1,1); the optional
loader at 0x2157F7 leaves them alone when their keys are absent. Filling these
from the native map instead introduced its colours into Rudnya and School.

The separate executable hook restores multiplicative ambient-light semantics
above intensity 1. This pack addresses original fog arithmetic and the tone
curve, whose native dark-scene luminance floor differs from the original.
It changes no geometry, local light intensity, material finish or gameplay.
Probe shading, remaining native permutations and the
parts named in the table are not claimed to be transplanted. Visual acceptance is recorded in the
Skhid map review, not inferred from successful shader generation.

The older Vostok/Mamayev day ports substituted native lighting objects.
`examples/restore-021-day-lights.rs DONOR_DB STOCK_DB COMPOSED_LAYERS_JSON OUT`
restores their donor lights and probes through `original_lighting_objects`,
which calls the same schema/mode-alias conversion as a new port. The composed
input is used only for removal identities, counts, environment before-values
and fixture bindings: no geometry or gameplay objects are copied from it.
Use the pre-restoration layered content; the tool rejects an already marked
project. It emits guarded source edits plus `maps-021d-original-probes@2`
(240 original DDS/options resources, including implicit diffuse cubemaps). That pack has no active-pack dependency:
the older `maps-021d@7` content is already in the published base.

`tools/content/two_maps_021_environment.py OUT EDIT_OUTPUT` uses the same
environment conversion, preserving original numeric values and current
resource bindings. An absent marker field is added with native integer type.
Skhid's `scripts/breakable-lamps/rebind_021.py` then uses the existing shared
nearest-fixture rule to update six guarded light-coordinate scalars: one lamp
in each map, across its three day modes. Fixture identities/counts remain.

The native fallback check reads every built shader, resolves only the explicit
marker branches to their unmarked path, and compares all executable
instructions with the pinned native input. It permits only the additional
scratch registers/static-globals declaration (and AO's expanded static buffer)
needed by the guarded branch. Run it with:

```sh
python3 tools/content/check_021_shader_fallback.py \
  PORTER NATIVE_DB PACK ports/three-maps-021-lighting.json CACHE_DIR
```

All 88 resources passed against the stock archive used for this pack. This
checks native fallback instructions; final rendering review remains separate.

For local lights, `src/weather_fog_021/local_lights.tsv` pins every source/target
pair. The adapter verifies reflected resource names and used constant offsets
and sizes, remaps only binding slots, and replaces clip-XY/W input with native
screen UV reconstruction. Its original arithmetic, including shadow sampling,
is otherwise copied verbatim from the user's donor archive. Each guarded
branch returns before the unchanged native fallback.

An independent disassembly check compares all five marked branches against
each original point/spot program, permitting only these binding/input changes:

```sh
python3 tools/content/check_021_local_lights.py \
  PORTER DONOR_DB NATIVE_DB PACK ports/three-maps-021-lighting.json CACHE_DIR VKD3D_COMPILER
```

All 32 pairs passed. `check_021_profile_preservation.py OLD_PACK OLD_MANIFEST
NEW_PACK NEW_MANIFEST` additionally proves that all 88 resources preserve every
executable instruction for the three previously approved map profiles.
Clippy retained the 44 pre-existing library warnings and introduced none.

## Fill-light facing (0.69d scenes, @13)

Behind a 0.69d scene's marker (an exposure profile's, `atmosphere_parameters3.w`, marked used in
each program's reflection), the 8 dot_normal `ambient_light.ps` variants draw 0.69d's facing term
`sat(sat(x/2 + 1/2)(1 + cutoff) - cutoff)^power` to the light's centre in place of `max(N.L, 0)`,
for a light whose `attenuation_power` carries it (`ambient_wrap.rs`, `src/ambient_facing.rs`:
bits `0x49800000 + payload`, the distance power n/128, the facing power n*0.1, the cutoff
(n-128)*0.01; 0.26g's own maps use 0.5 to 13.2). Every other scene and light runs the native
instructions. Rudnya's bus stop wall, turned from its fill lights, drew black (2.4 against 0.69d's
27, region mean 0-255).

## Fill-light boxes (0.69d scenes, @14)

In the same marked branch the 4 box variants draw 0.69d's own box falloff
(`ambient_light_additive.ps` / `_multiplicative.ps`, read from the bytecode) in place of
0.26g's smoothstep: per axis `(1 - t)^p` from the inner face `max(1 - s, 0.01 / half)` (0.69d
keeps the inner box 0.01 m from the centre, exe 0x20029BBBC) to the outer one, and for a light
with `gradient_attenuation` times `sat(0.5 - 0.5 y)^p` along its local y: full at one face,
nothing at the other, the light from a window or door falling off into a room. 0.26g's box
program has no power, so a box's `attenuation_power` carries its power (bits 0..11, n/128)
and the gradient (bit 12) beside the facing term (`src/ambient_facing.rs`, `Falloff::Box`); a
box without 0.69d's `dot_normal` carries the term 1 (cutoff -1). Rudnya's church floor, barn,
apiary and two sheds have 9 gradient boxes, which had drawn uniform at full strength (2 to 5
times 0.69d's light, hard-edged); School (v0.69) has 15.

A cylinder (0.69d's geometry 2; 0.26g has none) is placed as the box around it and carries
bit 11: the box branch then takes the distance from its axis, `sqrt(p_x^2 + p_z^2)`, for the
x and z factors (0.69d's `ambient_light_*.ps` geometry 2). Drawn as the plain box, Rudnya's
three near-black darkening cylinders in the church towers also darkened the box's corners
outside their disc.

## Probe tiers (0.69d scenes, @16)

0.69d composes its environment probes in tiers (`layer`): in a tier every probe adds its light
times its weight and the weight, the tier's light is `sum(w L) / max(W, 1)`, and the tier below
(global) shows only where `1 - W` remains. 0.26g lerps every probe over the ones before it
(SRC_ALPHA / INV_SRC_ALPHA, compiled in the exe), so between two abutting local probes the
outdoor light of the global probes shows through their falloff bands: the pale line across
Rudnya's church nave (0.69d 83 -> 87-94 -> 100, the port 83 -> 160-293 -> 100). skhid's exe patch
202 draws a 0.69d scene's local probes (the converter marks them by a radius below 0) three more
times, with three program families this pack adds (`src/weather_fog_021/probe_tiers.rs`):

| Family | Pass | Blend |
| --- | --- | --- |
| `environment_probe_tier_weight` | rt 32 (accum_ambient_lights) minus w/4, w 0.69d's linear falloff over the side width (a cylinder's in its circumscribed box) | ONE / ONE, reverse subtract |
| `environment_probe_tier_damp` | the light under the tier times (1 - min(W, 1))^(w/W) | ZERO / SRC_ALPHA |
| `environment_probe_tier_light` | the stock probe program's diffuse and specular, times w / max(W, 1) | ONE / ONE |

Each family has the stock `environment_probe_lighting.ps`'s mask and its 8 box keys (geometry 1,
LOD_INDEX 0; GLOBAL_LIGHTING_QUALITY 0 and 3, affect_diffuse, blend), each blob the stock blob's
include records with the program's DXBC. The DXBC is compiled from skhid's
`srp-026g/shaders/sm_4_0/environment_probe_lighting.ps` (`probes.sh build`, the June 2010 SDK's
fxc as every 0.26g blob) into `assets/probe-tiers/`; that source's stock program compiles back to
the shipped one (`probes.sh check`: 6 of 8 byte-identical, the other two one mul's operands
apart). The light pass is the stock pass times w / max(W, 1) and nothing else: the normal, the
diffuse by the normal and the specular by the reflection vector as the stock program has them.
No native program changes, and no scene without patch 203's local probes draws these: the pack
and the exe patch ship together.
