# Radar evening and rain: original 0.11e contract

Work in progress. No playable weather port or dynamic-rain parity has been
validated yet. The goal is the authored original appearance and motion, using
the original shaders with the necessary binding adapters.

`tools/render-compare/audit_011e_weather.py PORTER ORIGINAL_DB OUTPUT` extracts
and hashes the three original projects and their actual post-effect materials.
It refuses different project/material bytes. The archive is build 884,
SHA256 `694e1772a0595b6686cc9810afa438ed25b21360c591c0e26aa8e35dc03a4955`.

| Variant | Original project | Post material | Filmic A,B,C,D,E,F |
| --- | --- | --- | --- |
| Day | `level_03` | `level_03_day` | .08, 6.3, .3, 6, 2, .04 |
| Evening | `level_03_evn` | `level_03_evn` | .05, 7, .3, 6.2, 2, .02 |
| Rain | `level_03_rain` | `level_03_rain` | .01, 6.2, .3, 6.2, 2, .02 |

Dark-desaturation limits are 3.5, 4.5 and 6 respectively. The evening sun is
RGB (.973,.573,.322), intensity .75; rain is (.973,.973,1), intensity .07.
Evening uses `sky/evening`, rotation -14.2, multiplier 2.5. Rain uses
`sky/cgskies_cloud`, rotation 180, multiplier 10, authored RGB 2.0295568.
These values must be interpreted by the original shader equations; copying
them into unrelated native parameters does not prove the same lighting.

Rain uses five shells: start index 5, exclusive end 10, radius scale 1.1,
density .8, speed 2, UV scale (1,1), zero authored tilts, random rotation
radius .1, rotation speed .2 and base offset .1. Wet intensity is 2.
The original deterministic random seed and camera-dependent shell motion
are described in `factory-rain-010b.md` and implemented by the diagnostic
`legacy_rain_motion.py` / `legacy_rain_math.h` helpers. `legacy_rain_shells.c`
defaults to an auxiliary render target, but its explicit
`SRP_LEGACY_SOURCE_RAIN_LIVE=1` mode replaces native particles after validating
the complete source draw and its inputs. The previous Factory investigation
verified 241 applied frames with the director disabled (4.017 seconds), not
walking-camera parity. This private D3D observer is not yet a scene-owned
production integration. See `../factory-rain-shell-review.md` and
`encoded-rain-lighting.md`; encoded specular remains incomplete.

## Conversion baseline, 2026-11-12

`ports/survarium-011e-radar-weather-to-026g.json` builds the authored evening
and rain projects under independent `v011w` asset paths, with Protective
Device / Team Deathmatch IDs 249–252. The first bundle contains 5,984 files
and 261 materials. It retains the original geometry/material choices before
the renderer experiments; native shading is only a comparison baseline.
Measured static geometry totals are 48.2 MiB evening and 48.0 MiB rain,
below the engine's 88 MB pool. Neither number proves runtime residency.

The 0.11e archive omits Radar server projects. The manifest pins the 0.11a
weather server files and permits exactly one reviewed zone-mesh difference:
the primary client widens the first box of
`logic/team1/team_1_enemy_respawn_collision`. All non-mesh fields must match;
the converter copies that zone's primary meshes before strict validation of
the remaining zones, spawns and collectors. Unknown or already matching
exceptions fail. The affected converter tests pass (46). The mip selector's
two additional tests cover byte preservation and invalid/stale requests.
Clippy reports 66 errors versus the clean parent's 67: the texture-loop change
removes one existing collapsible-if warning and adds none. This is not a clean
lint result.

The private content build also keeps evening fog's original fractional
`density_offset=5.2`. Stock 0.26g's verified volume-fog loader accepts both
float and integer types; the content manifest's exact file/path exception
records that evidence. No source value was rounded to satisfy the type check.

`texture_mip_limits` explicitly selects the three vehicle atlas textures'
existing 2048 mip chains. The converter's output matches the merged Radar
texture fix byte for byte, including every retained mip and the sRGB trailer.
Only those three DDS files differ between the initial and corrected bundles;
the 4096x1024 cloud texture keeps its original dimensions.

`extract_011e_weather_shaders.py` extracted 79 unchanged programs across 23
families, checking both archived blob and embedded DXBC hashes and recording
reflection layouts and input/output signatures. No recompilation is involved.
The quality-2 0.11e post shader's
texture slots differ from native: source frame color is t3, luminance t4,
grain t1 and grading t7. Its filmic constants are c17/c18; source scalar
values alone cannot be copied into native constant-buffer offsets.

The original `copy_image.vs` is also extracted (SHA256
`29abfee6fb6bbf38274bcaca3dce5bb647a6099751b83a0beb6e2e571b2e3714`).
It consumes POSITIONT, COLOR and TEXCOORD and outputs UV in register 2.
The original post PS uses TEXCOORD0 in register 2, while the captured native
PS uses register 1. A bytecode swap must therefore verify the vertex-stage
linkage and geometry as well as the textures and constants. The original VS
uses `screen_res` at static-global offset 288.

Stock 0.26g still registers `filmic_tonemap_packed_parameters_0/1` at
scene-constant receiver offsets +0x14/+0x18, but its setter no longer
uploads them. The original 0.11e setter does. The Skhid re-kit check
`tools/re-kit/v026g/check_post_constants.py` executes the stock constructor
and setter under Unicorn: 64 cases confirm the 11 registered hosts, eight
upload calls, argument order and stack cleanup. Backend operations are
intercepted; this does not validate GPU output. Grain remains uploaded,
but at a different constant-buffer offset (native 176 versus original 256).
The captured native post also lacks original position/grain/blur/lens-flare
resource bindings and instead uses `t_bloom_combine`. Those producer
differences must be resolved before treating a post-stage adapter as parity.

Version 3 explicitly imported seven original programs: post PS,
`copy_image.vs`, present PS, rain VS/PS, and `rain_resolve` VS/PS. Each gets
a new `_v011w` family with the native unmasked 67-underscore key; no existing
shader family is replaced. Both the complete wrapper and embedded DXBC are
hash pinned, and their extents and shader stages are checked. The original
64x64 grain and 512x512 scroll-rain DDS/options also keep their complete bytes
under `textures/ported/v011w/engine/`. Three importer tests pass; the affected
46 converter tests pass; all-targets Clippy still has the same 66 existing
errors, with none in the new importer.

The retained version-3 bundle contains 5,995 files: the 5,984 version-2 files are
unchanged, with seven shaders and four texture/options files added. Pack
`v011e-radar-weather@3` passes the private quick content build and patch
readback. The stock 0.26 shader-wrapper instructions also select the exact
DXBC pointer and size for all seven resources in Unicorn. These files are
ready for an explicit adapter; the port still uses native shaders at runtime.

`make_011e_weather_poses.py` generated 128 collision-checked positions in both
ported weather worlds: 43 spawns, 30 interiors, 20 terrain, 19 roofs, six mode
objects, five fog views and five horizon views. It rejected 48 candidates.
This verifies camera placement only. Both initial original batches completed after the owner explicitly authorized
concurrent isolated rendering
on 2026-11-12 ("no you have enough GPU"). Render jobs remain sequential and
retain the 2.5 GiB free-VRAM check; 4,122 MiB remained free with both games
running. No weather image-parity result exists yet.
The source sweep is prepared as 48/48/29 new poses per weather, with three
identical initial cameras reused rather than retaken, and queued after the
initial comparison jobs. Every completed original batch is hash-frozen.

## Exact-build evidence

The reconstructed C++ tree identifies itself as 0.100b. In the original PDB
source catalogs, `stage_rain.cpp` has the same MD5 in 0.10b and 0.11e:
`1f5732ed9be99c77c27d57e8674e9bbb`. The archived rain VS, PS, resolve VS/PS
and skylight PS also match byte for byte. Rain PS SHA256:
`e7ca858153aac900b1205eb8d3da914aa6aef06acee569f2b657076c59798f68`;
VS: `a7fc8c504884f2934fc257bea25fc5e8fb5f4032d69effeb44ffda2c4da02ceb`.

`stage_pre_rain.cpp` changed: 0.10b MD5
`1f8f7bf8fb082e3333756e368b48f9a7`, 0.11e
`4bd95a23fff60ed3ab4d8ffd049af001`. Its original shadow selection/projection
must therefore be verified against build 884 before adapting the older helper.
Likewise the 0.11e post, SSAO and translucency contracts differ from 0.10b;
see `radar-chemical-011e-post.md` and `011e-ambient-lighting-20261014.md`.

## Validation still required

Freeze original evening/rain captures at generated locations spanning interiors,
terrain, roofs, horizon and spawns, including moving-camera rain sequences.
Build both variants through the porter/content manifest, measure geometry pool
usage, and compare the native baseline and each shader iteration at the same
cameras. New assets need their own namespace so original-finish materials do
not replace the published day map's chosen native finish. Final checks include
the map-wide sweep, rain bindings/geometry/time behavior, and local play.

## Quality-3 post correction (version 4)

The weather projects enable bokeh DOF, bokeh image and image grain. The
original executable selector at `0x608445..0x60847a` and the complete
original catalog prove these select `8696aa4b...` (6,744 bytes) at quality 3.
The previous `3eefb587...` is the quality-2 grain program; quality-3 bokeh
needs a different texture layout. The Skhid re-kit check
`tools/re-kit/v011e/check_post_selection.py` passes 32 quality/feature cases.
Live captures now confirm this exact quality-3 shader in both weather variants.

The porter now accepts a validated optional shader `variant` suffix, giving
each program a separate native family with the same unmasked key. All nine
distinct original post programs are included, alongside fullscreen VS,
presentation, rain and rain-resolve: 15 programs total. The unsuffixed post
family is quality-3 bokeh/image/grain; other variants carry explicit suffixes.
This avoids relying on incompatible native shader configuration bits.

Quality-3 uses position t0, grain t1, frame t2, luminance t3, scattering t4,
bokeh image t5, lens flares t6 and grading t7. It samples the sharp frame for
bokeh rather than the lower-quality blur texture. Its bokeh vector is at
constant offset 176, grain at 256, filmic vectors at 272 and 288. The original
128x128 BC1 `fx/bokeh_image` and its options are now imported unchanged.

Version-4 bundle `/tmp/v011-weather-bundle-v4` has 6,005 files: all 5,984
version-2 map files are identical; 15 shaders and six texture/options files
are added. Pack `v011e-radar-weather@4` SHA256 is
`f8873d65239db4fd5eaec99375a6fc22812fd5e5dbb7a24f54f7cbdfe0815c54`.
The manifest content build/readback passes, as do all 15 native shader-wrapper
checks and the importer tests. Strict Clippy reproduces the same 66 existing
diagnostics, with no new ones. These are staged assets, not active runtime
shaders or visual parity evidence.

## Original post replay: GPU round trip passed (2026-11-12)

The owner explicitly authorized concurrent isolated GPU captures ("no you have
enough GPU"). Jobs still run sequentially on dummy Xorg with at least 2.5 GiB
free VRAM. Both initial reference batches completed: 32 poses × two frames per
weather, 128 frames total. All actual positions, directions, FOVs and resolutions
match; see `/tmp/v011-weather-reference/original-camera-validation.json`.
Initial references are frozen and unchanged. Their frozen-manifest hashes:

- evening: `a674c3bb21645b503662a7b3af60146d6a7aa4204e811b2d4c58f1db37e79a65`;
- rain: `8bf442cb50ee0c7e34e53d8daefd094c944bdd7a51d868375168916ccecb41fd`.

Live diagnostics found two capture defects. The 0.11e initialization path never
armed buffer readbacks, so the first diagnostic batches contained only images.
It now enables the original-client observer without native stage hooks, and the
runner rejects missing post-stage evidence. The grading volume has three mips;
readback now records the full volume chain, preserving the existing mip-zero
format. Replay reads the actual `.post-output` filename. Additional diagnostic
captures went into new `rich/armed-*` and `rich/full-*` folders; earlier frozen
images were not replaced.

Full observer SHA256:
`43b89a5d33ff10e054b5d7409a6f8f73ab8e4ee04348dd4dae18b7b4d385e8bc`.
It builds with `-Wall -Wextra -Werror`. The CPU packet/comparator check passes,
including eight rejection cases and missing volume-mip coverage.

All eight original post draws (two poses × two frames × two weather variants)
use unchanged quality-3 PS `8696aa4b...` and copy-image VS `29abfee6...`.
Their full-mip packets replay on the GPU with **zero differing bytes** in every
3,686,400-byte 1280×720 R11G11B10 target. Results and input hashes:
`/tmp/v011-weather-reference/post-replays/results.json`, SHA256
`520206f6dc04ce5a3b0b1bc8dfa61b70b3e41d15c820cf791d97e614e79cd313`.
This proves the original post draw with original inputs; it does not establish
native scene-input adaptation, whole-renderer parity or moving-rain parity.

Original rain diagnostics confirm five shells in each captured frame, radii
5, 6.6000004, 8.4700003, 10.6480007 and 13.1769009, density 0.8 and speed 2.
All five share one scene time per frame, and time advances between frames.
`original-rain-layers.json` records the constants, geometry and camera evidence.
These are fixed-camera captures, not a walking test.

The native baseline uses the existing scratch content build 57. Its first
server startup correctly rejected minimum revision 56. Only the private capture
EXE's declared revision and PE checksum were aligned to scratch 57, with the
server and test bot matching; source REVISION and release state are unchanged.
`native-scratch-revision.json` records exact hashes and changed offsets.

The current queue is `/tmp/v011-weather-reference/complete_queue.py`, log
`complete-queue.log`: native baseline plus shader-state captures, image
comparison, then the original 128-position sweep. Native renderer integration
and walking-rain verification remain outstanding; nothing was released.

## Native baseline and pre-post deficit (2026-11-12)

Both native baseline batches completed: 32 matched poses × two frames for
both weather variants. These use the version-1 content baseline, not the
version-4 atlas/shader asset pack. Camera validation and comparison completed;
`/tmp/v011-weather-reference/baseline-report/{evening,rain}.json` contain all
64 per-weather results, with `*-worst.png` contact sheets. Mean display RGB
RMSE on the fixed comparison mask is 0.2033916655 for evening and 0.1366797850
for rain. These are baseline measurements, not acceptance thresholds.

The deficit is present before post-processing. Matching captured
`t_frame_color` inputs are both R11G11B10_FLOAT; the fixed comparison mask has
no nonfinite pixels. Mean linear luminance:

| Weather / view | Original | Native baseline | Native/original |
| --- | ---: | ---: | ---: |
| Evening / factory interior | 0.0164660024 | 0.0051916427 | 0.315295 |
| Evening / north yard | 0.0246467974 | 0.0090967289 | 0.369084 |
| Rain / factory interior | 0.0123134693 | 0.0036365242 | 0.295329 |

Receipts are `evening-post-frame-comparison.json`,
`evening-yard-post-frame-comparison.json` and `rain-post-frame-comparison.json`
in `/tmp/v011-weather-reference`. This localizes a significant difference
upstream of post; it does not identify one lighting producer as the sole cause.
Original composition consumes encoded R10G10B10A2 light accumulators, whereas
native composition consumes linear R11G11B10 accumulators and adds sun lighting
there. Their raw accumulator bytes cannot be compared as equivalent radiance.

Native evening grain is 0.06 at this resolution although the imported project
correctly retains the authored intensity 0.09. The stock environment loader
`0x34e1e0` reads `image_grain_intensity` at `0x34f36f` and stores it at
record+0x20c (`0x34f399`); record and config are entry stack+4/+8. The native
post stage at `0x1b24da..0x1b251c` multiplies authored intensity by a selected
integer rendering option and the stock float 1/1920. For selected value 1280,
this explains 0.09 becoming 0.06. The option's identity is not established by
this bounded check. `check_post_constants.py` passes 30 real-instruction grain
cases in addition to the existing 64 scene and 64 DOF cases. Loader and input
findings are recorded in the 0.26g re-kit.

The post replay preparer now supports explicit named texture substitutions
from another captured draw. It preserves the original shaders, geometry,
constants and all other textures, requires identical storage/view contracts,
hashes every input, and labels the result as an input sensitivity experiment.
CPU checks pass unchanged-input identity, changed-input propagation and 11
rejection cases. Eight native-frame/native-frame-plus-luminance experiments
(two views × two weather variants) are prepared at
`/tmp/v011-weather-reference/input-substitution`, queued after the original
sweep. They have **not yet executed on the GPU** at this checkpoint and do not
constitute a runtime adapter.

The current native stage captures lack vertex geometry/full-mip/rain-count
flags. The private runner now sets those for subsequent launches; the existing
stage buffers remain useful for the measurements above. Production renderer
integration, a final native sweep and moving-camera rain verification remain
outstanding. Nothing was released.

## Live original-post diagnostic bridge (2026-11-12, tests queued)

The porter now has a reusable original-post command list (`post_replay.h`) and
an opt-in native observer bridge (`original_post_live.c`). Both compile under
MinGW with `-Wall -Wextra -Werror`. This is diagnostic code outside the client
patch arena; no production runtime hooks or release change are included.

The bridge selects the exact observed native composition/post bytecodes,
retains current-frame depth, and validates texture dimensions, formats and mip
chains before using live frame/luminance/grading data. It packs native linear
scattering for the original shader, uses original grain/bokeh assets and
weather constants, updates grain phase, executes the unchanged original VS/PS,
and copies the private result into the native post target. Native drawing is
retained when an input contract fails. It runs only with the private capture
camera enabled. Lens flares are absent and native downstream presentation
remains; neither full appearance nor moving-rain parity is established.

The reusable command-list harness checks malformed packets, repeated draws and
restoration of viewport/rasterizer/CB/SRV/VS/RT/topology. Eight GPU comparisons
against the existing exact original post outputs are queued in
`run_input_substitutions.py`, after the remaining source sweep. Only if all
match does `run_live_original_post.py` launch two poses × two frames for each
weather in `/tmp/v011-weather-native-original-post`, using the unchanged
version-1 baseline content to isolate the shader change. No live-test pass is
claimed yet. Build inputs and DLL/EXE hashes are in
`/tmp/v011-weather-reference/live-post-build.json`.

All three evening reference sweep batches are now frozen (125 new locations,
plus three reused initial cameras). Rain sweep batch 1 is running at this
checkpoint. Initial references remain unchanged.

## Original gamma copy and pack 5 (2026-11-12)

Captured final-copy instructions establish another adapter requirement:
original `copy_image.ps` SHA256
`685c00b3451a1fb3291f40696e7b4ae34b0b18f6bad32ab62e634b5f8f9cb451`
computes `pow(abs(color), 1/(2.2*gamma))`; native `495af559...` computes
`1.055*pow(abs(color), 1/(2.4*gamma))-0.055`. Their UV inputs are registers 2
and 1 respectively. The observer calls this `.present-draw`, but the original
archive's separate `present.ps` (`0a747779...`) is a plain blit. It cannot
supply the observed gamma conversion. The captured post output equals the
captured gamma-copy input byte-for-byte in all eight inspected combinations
(two views × two weather variants × original/native); those views provide no
evidence of another intermediate image modification.

Pack `v011e-radar-weather@5` adds the unchanged original gamma-copy shader.
All 6,005 version-4 overlay files remain byte-identical; only
`shaders/sm_4_0/copy_image_v011w.ps/<67 underscores>` is added (6,006 total).
Pack JSON SHA256:
`e002c5352c079e6c00183ccab24d79af5edadf8bd93969ff70cd4f4992347a7a`.
The manifest row uses version 5. The stock-wrapper Unicorn check passes all
16 original shaders. The manifest content build passed at
`/tmp/v011-weather-content-gamma-v5`. This is a local QUICK-BUILD: mounted
players-file verification, site pages and item tables were not requested; it
is not release validation. Patch SHA256:
`79603a9fd8c419a0b768131e4ce137a41692f90e4fc147b309e4e8c2e686ad15`;
server-data SHA256:
`993e40504ef82d95040133820e41008aa7799870b69db06520570982238bb8d7`.

The diagnostic bridge now requires paired post/gamma-copy packets, validates
the current device and full viewport, and draws the original gamma copy only
after a successful original post in the same frame. It copies the live gamma
constant and current input, preserving native context state. Both programs
compile with warnings treated as errors. CPU packet checks pass presentation
preparation plus 13 rejection cases, including missing/different-frame quad
sources. The presentation packet explicitly derives its TL quad from the same
frame's original post capture. All eight original-post and eight original
gamma-copy command-list GPU comparisons passed byte-for-byte, including
repeated execution and native-context restoration. This establishes geometry
reuse for these eight captured gamma-copy draws. Result manifests:
`command-replays/results.json` SHA256
`423a9e41c81f21f2fd89c3028ca5def92e30e26125c1420e1f0a8dcfb2340787`,
`presentation-command-replays/results.json` SHA256
`b83216d5b833b45ed1df5de609fe5d6c831c2279e7c95c405f15f8dd5a78d52a`,
both under `/tmp/v011-weather-reference`.

The original skylight bytecode provides a useful next boundary: its pixel
shader reads only normal XY and performs the same sphere-map reconstruction,
then uses original skylight basis vectors. Its original vertex shader is
`9dab4eced49b2980e4e70843a6f42e9f59a84bf6e696121bc0c931e685a9f6f4`,
with POSITION float4/TEXCOORD float2 inputs, UV output register 1 and basis
outputs 2/3/4. Its sole constant is `skylight_parameters6` at b0 offset 0.
This shader was extracted and hash-checked against the 0.11e catalog in
`/tmp/v011-weather-source/skylight-vs-original.{dxbc,json}`. This is binding
analysis, not a skylight implementation or a visual-parity result.


## Completed GPU diagnostics and source sweep (2026-11-12)

The earlier queued checks are complete. The live original post/gamma bridge
executed 660 paired draws for evening and 480 for rain, with zero rejections.
Each capture has two views and two frames; camera position, direction, FOV and
resolution agree with the original and native baseline. Reports and contact
sheets are `/tmp/v011-weather-reference/live-post-comparison/{evening,rain}.{json,png}`.
The live test deliberately retains version-1 content to isolate the shader
change; it does not validate version-5 content in play.

Display RGB RMSE (baseline → bridge), first frame:

| Weather / view | Baseline | Original post + gamma |
| --- | ---: | ---: |
| Evening factory | 0.104025 | 0.099595 |
| Evening yard | 0.112383 | 0.111317 |
| Rain factory | 0.083476 | 0.076492 |
| Rain yard | 0.066650 | 0.064147 |

The modest improvement does not resolve the dark lighting. Rain factory mean
luma is 0.07751 versus original 0.12991; rain yard is 0.09692 versus 0.15071.
Screenshots were visually inspected. Lighting before post-processing remains
the next boundary; neither full renderer parity nor moving-rain parity is
established. The bridge remains a diagnostic, with CPU scattering conversion,
absent lens flares and no production scene lifecycle integration.

Eight controlled native-frame/native-frame-plus-luminance substitutions also
finished. Each pair produces identical output in these views, consistent with
the observed exposure clamp; this is not a general luminance-independence claim.
Metrics: `/tmp/v011-weather-reference/input-substitution/metrics.json`, SHA256
`5a1d4b621db33957f5243b45765bab6d0f4d3e499e2cf530488b98a4d8951c57`.

All six original sweep batches completed. Frozen file hashes and actual camera
metadata were verified: 500 new frames plus 12 reused initial frames, covering
128 locations per weather variant. Receipt:
`/tmp/v011-weather-reference/sweep/completion-receipt.json`. Initial originals
remain unchanged. A final native sweep is still required after renderer fixes.
Nothing was released.


## Exact original skylight replay (2026-11-12)

`replay_skylight.py` now prepares the unchanged original skylight VS/PS for the
reusable D3D11 command list. The first diagnostic reconstructed the clip quad
and omitted depth/stencil state. RGB on pixels changed by the captured draw
agreed within one 10-bit step, but alpha and about 1,000 untouched pixels per
view differed. This was not accepted as full equality.

Additional isolated original captures established the missing contract:
RGB-only write mask 7; depth disabled; stencil enabled with reference 0,
read mask 255, write mask 0, NOT_EQUAL on both faces and KEEP for every stencil
operation. The bound surface is R24G8_TYPELESS with a D24_UNORM_S8_UINT view.
The original quad uses POSITION float4/TEXCOORD float2, stride 24, six R16
indices and front-face culling. Its captured VS bytecode equals the catalog's
`9dab4eced49b2980e4e70843a6f42e9f59a84bf6e696121bc0c931e685a9f6f4`;
its VS constant equals the original PS's `skylight_parameters6` bytes.

The default preparer now requires the captured geometry, VS constant,
write mask and stencil state/surface. Reconstruction remains an explicitly
labelled experiment. Eight complete GPU replays (two views × two frames ×
two weather variants) match every target byte: zero differing bytes out of
3,686,400 per draw. Both existing post/gamma regression paths also remain
byte-identical after extending the shared command-list implementation.

Evidence under `/tmp/v011-weather-reference`:

- `skylight-state-replays/results.json`, SHA256
  `857a012bb1745fb03a5a591f5ee6775e2f01bff3ed1f364bced4dccfd7644ede`.
- `skylight-state-replays/restore.json`, SHA256
  `be5ed04dc18f8f26c665ec70a2d99705cfe871f917556ff1bb9e98318ca5768a`:
  additional GPU test verifies depth/stencil reference, blend factors/mask and
  the previously checked context state after two executions; output is exact.
- `skylight-state/completion.json`: frozen file hashes and actual camera
  metadata verified. Frozen manifests: evening
  `13c7a4ed39ff857dd1181b662c27d6f47a736f919842afe37feee0001d81e897`,
  rain `1114c17ffd9430c4dbdd941a689279d4013b03f5ebab79411852aed52a102e2b`.
  Earlier original references remain unchanged.

The DLL and command-list harness compile with `-Wall -Wextra -Werror`.
CPU preparation checks pass the valid captured draw and 14 rejection cases.
This verifies an original pass in isolation, not native skylight integration.
Next: provide its native depth/normal/AO inputs, preserve the original encoded
light accumulation and probe ordering, then compare the live result. The
post/gamma bridge remains the only live original-shader diagnostic added by
this weather work; production integration, final native sweep and moving-rain
parity are still outstanding. No release or weather PR was made.
