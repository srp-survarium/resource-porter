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
