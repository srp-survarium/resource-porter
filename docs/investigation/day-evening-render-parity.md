# Day and evening renderer comparison

The user requested an independent continuation of the v0.10b/v0.26 lighting
comparison for authored day and evening variants, with broader sampling to
avoid tuning to the four rain views. This investigation is incomplete.

## Isolation and references

Branch `codex/compare-day-evening` started at `f719d78`. The rain worker's client,
archive, prefix, server and original captures are not modified. Independent
runtime files are under `.work/render-compare/day-evening/`; the original client
uses port 2251 and the target uses 2252. The first attempted original launch
found 2249 occupied by an unrelated server. That process was left untouched;
only this worker's failed launch was stopped. A bounded error-log excerpt is
retained instead of its repeated panic output.

The source projects are `projects/level_03/client_project` (day) and
`projects/level_03_evn/client_project` (evening). Target map IDs are 180 and 181,
with projects `v010b_level_03/day_2` and `v010b_level_03_evn/evening_2`. These are
actual authored variants, not rain with a different tint.

The private original server was rebuilt from `/tmp/srp-render-reference` source,
with port 2251 and the evening profile restored to `level_03_evn`. Its day profile
selects `level_03`. Original shader overrides remain disabled. Both Wine prefixes
have private Documents folders so client settings cannot modify the user's
profile or the rain worker's settings.

## Sampling

`tools/render-compare/poses-day-evening.json` preserves the initial 18-view plan:
ten distinct positions, with six views at three positions held out. Those
positions used farthest-spaced authored spawn points plus the existing upper,
interior and yard views. The user then requested coverage beyond the spawn
clusters. The original plan is retained; `poses-day-evening-coverage.json` adds
14 spatially distinct positions, and `poses-day-evening-expanded.json` combines
them into 32 views across 24 positions.

Added locations span the hospital paths, officer roof/terrace, central vehicle
route, chemical yard and east route, southeast rise, garage east route, upper
central courtyard, west terrace, northwest route, north gate and northeast
route. Their eye heights use an intersection with actual original collision
triangles plus 1.65 metres. Coordinates span x [-60,77.857], z [-106.277,40],
with eye heights [1.961,15.5]. Terrain itself extends farther; these coordinates
are evidence of spatial coverage, not a claim to sample every accessible place.

The initial day capture finished before the broader-coverage request arrived.
It remains frozen in `010b/day/`; only the 14 new positions are added in a
separate `010b/day-coverage/` session. Evening uses one complete 32-view session
in `010b/evening/`. The view selection uses map geometry, not measured renderer
mismatch. The coverage extension is marked separately as held out.

## Observer changes

Commit `68c3081` adds R10G10B10A2_UNORM readback and matching channel decoding.
Original normal, diffuse-light and specular-light buffers were previously
skipped. The observer now recognizes 65 archived shader identities across 16
families, including probe, sun, translucency, skylight, sun mask and volume cone.
Capture retains the first draw of each reference family, not a complete frame
trace or every probe instance. Outputs and bound inputs are kept unchanged.

The C tools build with warnings as errors. Packed-format decoder checks cover
zero, full-scale and independently populated channels. No game tests were added.
Frozen manifests validate each written resource's dimensions/byte count and
hash each pose's artifacts. The initial day batch contains 342 packed 10-bit
buffers; the full evening batch contains 608.

## Authored post-processing differences

Actual original post-draw constant buffers confirm these filmic parameters:

| Variant | Shoulder | Linear strength | Linear angle | Toe strength | Toe numerator | Toe denominator |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Day | .3 | 6.2 | .2 | 5.9 | 1.5 | .1 |
| Evening | .2 | 7 | .2 | 6.2 | 2 | .01 |
| Rain reference | .15 | 6.2 | .2 | 5.9 | 1.5 | .04 |

Day uses `color_grading/nt`; evening uses `color_grading/evn`. Both captured
original variants upload grain strength .08 and dark-desaturation limit .03.
The rain shader's fixed tone curve must not be applied unchanged to these maps.
Native target baselines were retained before adapting the connected pipeline.

The optional original translucency base experiment was brought in from rain.
Its initial probe complement was subsequently disproved by producer evidence;
`bf67e33` applies the verified direct-smoothness mapping from `8bb781c`.

## Completed baseline evidence

Both target native baselines now contain all 32 views, with 5,692 hashed files
and 1,825 written buffers per variant. These use native shaders and the existing
58 numeric scalar repairs across 41 materials; they are not unmodified retail
captures. Original manifests contain 3,568 files for initial day, 2,830 for day
coverage, and 6,398 for evening. All written texture dimensions and byte counts
were checked before freezing the manifests.

One evening capture acknowledged its requested pose after a controller timeout.
The same process eventually produced a matching camera ID, dimensions and pose;
that record is explicitly marked `recovered_late_acknowledgment`. The remaining
views continued from that client. No missing frame was silently counted.

The native baselines have UI visible. All subsequent target sampling recipes, including native, use the native
`ui off` setting in the private prefix. That setting suppresses Flash HUD/UI;
weapon geometry remains. Image comparison masks exclude the original HUD/player
regions. The metric tool samples a fixed four-pixel grid, with display RGB RMSE,
luma error, block contrast and 32-pixel block SSIM. These are diagnostics over
separate animated frames, not proof of visual parity.

## Verified surface contracts

The original normal buffer stores seven-bit **smoothness**, not roughness.
Both archived native G-buffer producers and actual captured producers confirm
that target `parameters.y` is also smoothness:

```
constant: o2.y = (1 - roughness) * smoothness_multiplier
textured: o2.y = (1 - (roughness_min + roughness_range * texture.g))
                  * smoothness_multiplier
```

Original textured material code samples `texture.r`. The different texture
channel is a separate contract mismatch; it does not justify complementing
all materials. Four original/ported roughness DDS pairs are byte-identical.
Their unweighted mip-zero red/green MAE is .0165 for fir_translucence, .0402
for flora_tree_bough, .0474 for bark_sosna and .0271 for poplar_leaves_02.
These differences are not yet visibility/material-range weighted, and no
texture-channel correction has been applied. Captured target material draws 24 (constant) and 20 (textured)
provide active-producer evidence, in addition to archived bytecode.

Across 64 matched day/evening views, depth-matched samples have mean per-view
smoothness MAE .0223/.0219 with direct mapping, versus .8864/.8865 with a
complement. This invalidates the earlier downstream-name-based interpretation
of `parameters.y` as roughness. The probe correction now uses direct mapping.

The original translucency scalar is five-bit gamma encoded. Across 137,654 day
and 138,734 evening depth-matched translucent samples, original decoded means
are .1558/.1578, compared with native parameter-z means .1645/.1671. Re-encoding
and truncating the native scalar to the old five-bit representation gives means
.1545/.1571. Corresponding MAE falls from .0252/.0256 to .0198/.0201. Applying an
extra gamma power to native z is substantially worse (.0909/.0911 MAE). Packing
explains much of the mean difference; filtering, animation and material
correspondence still limit per-pixel agreement.

The rain worker independently replayed original translucency using these frozen
references. All 86,334 sampled pixels across 64 views match within one ten-bit
step when the helper's **green shadow-visibility** channel is used in the old
attenuation equation. Using red raw depth does not reproduce the original.
This establishes a source contract, not completed target scattering parity.

## Weather adapter draft and reproducibility

`build_shaders.py --weather day|evening` selects actual authored post curves and
cloud settings. Rain remains the default. The day/evening cloud fade band is
.15 with power 10, versus rain .5/power 2. Their authored cloud multipliers are
1 and 2.5. Original CPU code uploads `pow(color, 2.2) * multiplier`; the native
cloud upload adds a different sunlight multiplier, so the weather adapter uses
the original authored upload. Day/evening use alpha blending, whereas the
existing rain path uses modulation. The original observer did not recognize the
alpha-cloud shader, so these cloud contracts are supported by original source,
archive material and whole images, not a captured alpha-cloud intermediate.

Both weather shader sets compiled with warnings as errors. The private archive
has guarded project edits for the sun-azimuth convention, original ambient
pipeline and seven probe extents. No rain project/archive is modified. The
connected draft uses the optional original translucency **base** term; it is
not the complete original directional scattering equation.

Inputs are preserved in
`.work/render-compare/day-evening/reproduction/connected-v1/`, including 142
hashed files, shader source/build outputs, recipes, project bytes and patch
plan, tools and producer evidence. Historical launch helpers still reference
scratch paths; that checkpoint is not yet a portable one-command replay.

## Runtime stall investigation

The first connected day attempt produced no images: camera and present counters
stopped before the first capture. An earlier material-observer diagnostic also
failed. Both are retained as failures, not validation results. A native-shader,
UI-on diagnostic also stalled, so neither the weather shader math nor UI
suppression alone explains the problem.

Linux ptrace is prohibited for both attachment and debugger-launched children.
A temporary observer instead sampled its own thread contexts and stack address
words. The busy logic thread repeatedly writes journal record type 2
(`logic_wait`) while waiting for rendering; the main render thread waits for a
renderer command-queue entry. This accounts for high CPU and a growing journal
but does not yet identify the originating fault. Bounded journal excerpts and
thread samples are retained. Explicit X11 focus on the verified private PID did
not resume rendering. A native `-debug_single_thread` diagnostic also failed to finish initialization.
The private client/server were then stopped to permit a rain-only diagnostic.
No behavior change is being presented as a lighting fix.

The durable support bundle `reproduction/weather-support-20261013/` preserves
the original server source and both server/client binaries, normal target
launch/capture helpers without scratch-directory dependencies, metric reports
and temporary diagnostics. `README.txt` gives exact replay commands. Existing
private runtimes and pinned Nix dependencies remain required; archived original
references should be reused. All launch helpers now set `ui off` unconditionally
and record that command, including for native recipes.

## Sequential connected validation

With the rain client stopped, the unchanged connected day recipe completed all
32 views. Evening then completed all 32 in the same private prefix. Each frozen
batch contains 5,784 hashed files and 2,013 written buffers. The private client
and server were stopped afterwards. A later rain-only retry also stalled, so concurrency is not a complete
explanation. Sequential execution produced these successful batches but is not
a proven fix for the intermittent renderer/logic wait.

Actual captured DXBC matches the saved weather recipe for seven shader families
in every view: composition, post, fog, sky, present, ambient decode and probe.
Launch records confirm `ui off`; visual inspection confirms no HUD, minimap,
crosshair or text. The target weapon remains separate foreground geometry.

The revised common scene mask excludes the original minimap, original
crosshair, lower HUD strip and target-only weapon. This correction is applied to
both baselines and both connected variants. Earlier metric files remain as v1;
`*-metrics-v2.json` records the exact common mask. Contact sheets retain whole
images and use columns original / native / connected. All 64 triplets were
inspected, including every distant coverage location.

| Variant / views | Native luma MAE | Connected luma MAE | Native block SSIM | Connected block SSIM |
| --- | ---: | ---: | ---: | ---: |
| Day, all 32 | .1388 | .0357 | .2423 | .8164 |
| Day, 6 initial held-out | .1535 | .0338 | .2922 | .8209 |
| Day, 14 distant held-out | .1661 | .0440 | .2222 | .7857 |
| Evening, all 32 | .1610 | .0334 | .1425 | .8299 |
| Evening, 6 initial held-out | .1280 | .0228 | .1137 | .8622 |
| Evening, 14 distant held-out | .1949 | .0463 | .1851 | .7875 |

These are means of per-view fixed-grid diagnostics. Every day and evening view
improves both luma MAE and block SSIM over its native baseline. Day's mean scene
luma is 5.6% above original; evening is 2.9% below. Light placement, shadowed
surfaces and atmosphere are substantially closer across buildings and exterior
routes. Residual differences remain in foliage/filtering, atmospheric haze,
scattering, and evening red rays (subsequently traced below). This is broad improvement, not full
renderer parity or validation of the separate full-scattering experiment.

The paired base-term audit covers all 64 full-resolution frames. Exact shader
identities, buffer formats and lengths are checked. No pixel with zero native
translucency changes, and no nonfinite packed-light value is introduced by the base-term
counterfactual. This isolates that equation's effect; it does not validate every
remaining native producer. No game tests were added.

### CPU roughness follow-up

Frozen same-weather connected captures give smoothness MAE .02101 day/.02033
evening on depth-matched samples. Requiring matching encoded normals, albedo
and translucency lowers this to .00886/.00950 over 24,561/40,546 samples;
old seven-bit quantization lowers it further to .00789/.00858. The strongest
repeatable translucent residuals are hospital-south-path (~.022) and
officer-terrace (~.019) in both variants. These selective metrics are not
whole-image parity scores.

All four authored roughness textures retain their sRGB flag. Applying their
material ranges, unweighted mip-zero channel differences produce smoothness
MAE .00298/.02931/.03387/.01665 for fir/bough/bark/poplar. Forcing the original
red channel linear instead produces .03184/.23848/.21281/.20448. Transfer
changes therefore dominate the intended channel correction.

All 78 textured draws sampled per rain recipe use smoothness multiplier 1.
The corrected adapter is bound through sRGB view 91 in four draws, but those
resources are only 64×64. Original references lack material SRV captures;
resource identity, mip correspondence and per-pixel material attribution
remain unverified. Draw ordinals differ between runs. Five traced foliage
materials preserve their roughness ranges and texture-selection flags.

The separate `reproduction/roughness-contracts-20261013` bundle contains ten
hashed files: reports, standard-library Python audits, poses and replay
instructions. The same-weather replay reproduces its JSON byte-for-byte.
No clients were launched, runtimes changed or game tests added for this audit.

### Evening red rays: optional native pass

All 68 frozen original post shaders (rain, day and evening) omit the scattering
texture; the original profile has `r_use_god_rays off`. Native post slot3 is
zero in all 64 connected weather captures, so excluding it from the adapted
post shader corrects the contract but does not remove the visible red streaks.

The separate native `god_rays_apply.ps` adds half-resolution scattering earlier,
using ONE/ONE additive blending. Four evening diagnostic captures retain its
input, before/output targets, sampler, blend and exact DXBC. CPU bilinear replay
agrees within one R11/G11/B10 quantization step for all 2,764,800 sampled channel
values. The yard-north doorway receives +.02219 red and +.00032 green, explaining
the streak rather than an authored lamp or general ambient-light change.

`SRP_LEGACY_DISABLE_GOD_RAYS=1` omits only the pinned apply shader when its blend
is additive ONE/ONE. Native defaults and original-client capture stay unchanged.
Four validation views have byte-identical apply before/output buffers; the red
doorway streak disappears. Whole-image MAE/SSIM changes are mixed across these
separately animated frames, so this is a verified feature-setting correction,
not a claim of uniform whole-image improvement. Broad weather validation remains
pending. Original lens flares are a separate, still-unmatched feature.

Artifacts: `026g/evening-god-rays-diagnostic`, `evening-god-rays-disabled`, and
`evening-god-rays-overlap` beneath the day/evening comparison root. The overlap
experiment completed two worker readbacks while root stalled after one; this
does not establish safe concurrent capture. Compiler checks use existing
`-Wall -Wextra -Werror`; no game tests were added.

### Stationary noise at a distance

The user reports noise on distant surfaces while standing still, across maps;
this is not limited to sky or camera movement. Factory rain's four saved views
also differ after the gamma-copy draw. Final/gamma spatial high-pass RMS ratios
are .654/.656/.763/.660 in the original and .794/.774/.903/.890 in 0.26
(interior/north/south/upper). The target's final mean luma rises .0051–.0132;
the original falls .0006–.0012. These single-frame statistics include edges,
so they establish a late-stage mismatch, not temporal or distance attribution.

A shared candidate is duplicate grain: the compatibility post restores source
grain, while native `aberration.ps` has grain-enabled permutations downstream.
The original source's aberration shader has no grain; its complex post blend
adds grain once using frame-index-driven random offsets. Active native shader
capture and a selective correction are pending. Preserve intentional original
grain rather than removing noise indiscriminately.

The root `reproduction/source-volume-post-contract-checkpoint` preserves 397
files and hashes 1,575 successful solo-capture files plus 425 files from the
failed overlap batch. Its four-view beam check verifies original sphere slot14
and all six mips; bulb size and beam brightness still differ. During the overlap
stall, logic/render/channel counters all stayed 895 for about 26 seconds while
matching queue head/tail addresses cycled. GPU usage was 17–21%, with 4720 MiB of
8192 MiB used. This does not show GPU exhaustion or the previously hypothesized
logic/render counter disparity; it does not prove concurrency caused the stall.

### Fractional material scalars

A follow-up over 247 authored/converted materials finds 156 Float-to-Integer-template
value losses in 82 materials. The earlier scalar fix handled only the reverse
conversion. Grass roughness minimum is one active example: authored 0.3 became 0;
the shader reader uses a float minimum/range, so truncating it is incorrect.
The config reader accepts either integer or float scalar storage.

The converter now preserves the authored Float type for numeric parameters,
while shader feature selectors remain native. Rebuilding all 82 materials with
their previous explicit templates retains all 156 authored values. The existing
merge test now checks fractional preservation through binary serialization and
selector exclusion; all 270 library tests pass with no added test functions.

This is a conversion correction, not blanket runtime validation. In particular,
two textured-translucency constants need separate cross-version semantics:
the original texture overrides the constant, while the native producer may
multiply by it. Do not replace their effective multiplier1 without checking
that contract. Audit/rebuild artifacts are in the heldout-materials checkpoint.

Flat material paths exposed a second template-selection issue: the filename in
`flora/big_oak_tile.material` was treated as a semantic subdirectory. Thus the
same-name parallax-enabled template outranked a closer feature match. Subdomain
ranking now uses directories only. With authored values merged, oak tile selects
`big_oak_tile_rust`, roots select `big_oak_det`; each changes only the runtime
shader switch `use_parallax` from true to the authored false. Existing oak detail
and rusty-tile selections stay unchanged. All 270 tests pass; the existing
selection test includes flat-directory matching. Runtime validation is pending.

Four matched day views now validate the isolated material changes with duplicate
native grain and unwanted god rays already disabled in both runs. The private
patch restores 154 fractional values and disables unauthored parallax on two oak
materials; the two textured-translucency constants remain excluded. MAE/SSIM
improve in yard-north, hospital-south-path and officer-terrace. Terrace SSIM rises
from .787 to .824; factory-upper falls from .752 to .737 across independently
animated foliage frames. This is not uniform visual parity. Both recipes and
full captures are preserved as `day-material-baseline`/`day-material-corrected`.

### Moving background tree cards

The user's two stationary screenshots show hillside tree tops moving 6–9 pixels
in 1.96 seconds while the building and foreground trunk remain fixed. These are
Factory `level/03/back_01` through `back_05` meshes, whose original and converted
settings bind `flora/sosna_bark_lod`. Its authored `wind_motion=0` became native
`wind_motion=2` through the earlier same-name template choice. The native
`sosna_bark_lod_not_move` template has the required zero selector; feature-first
ranking now prefers it. This is separate from modeled foliage wind and camera
billboarding.

`prepare_static_tree_wind.py` prepares an archive-specific, reversible correction
for the frozen port. It verifies all five source/converted model bindings and
the source/native selector contracts, then changes exactly one material data
byte from 2 to 0. Original `is_cast_shadow=true` remains intact; the stationary
native template's false shadow flag is not copied. Apply/revert with
`patch_archive_entries.py` only while that archive's client is stopped.

Six fixed-camera frames before and after the correction reproduce the user's
issue at officer-terrace. Three hillside patches move up to 6/6/9 pixels before;
all five comparisons per patch are (0,0) afterward. Their demeaned temporal luma
RMS falls .03039/.04580/.02191 to .00460/.00430/.00372. Building/trunk remain
stationary in both runs, and other foliage remains animated. Residual grain and
lighting variation mean zero temporal image difference is not expected.

`reproduction/tree-cards-20261013` preserves the user's screenshots, source and
converted contracts, one-byte patch, scripts, metrics and hashed capture
manifests. Runtime sequences are `026g/day-tree-cards-baseline` and
`026g/day-tree-cards-fixed`. The corrected material remains applied in the worker's
private archive for final weather validation. No game test functions were added.
