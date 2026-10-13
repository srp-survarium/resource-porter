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
Native target baselines must be retained before adapting the connected pipeline.

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

The native baselines have UI visible. Subsequent target sampling uses the native
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
channel is a separate contract to investigate; it does not justify complementing
all materials. Captured target material draws 24 (constant) and 20 (textured)
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
