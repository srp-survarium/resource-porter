# Live Factory source lighting

`SRP_LEGACY_FULL_DIFFUSE=1` enables the full encoded diffuse composition shader
as the displayed scene. It requires the paired shader and complete source
ambient/local/sun/translucency producers. The hook binds their current-frame
R10 result at t15, resolves `8*E*E`, and restores the previous binding after
composition and diagnostic replays. Native albedo and specular remain.
Menu/loading frames without a complete accumulator use native composition.

Five views in `026g/pipeline-factory-live-diffuse-v2` pass: current-frame
lighting in every capture, no fallback, finite HDR output, and byte-identical
displayed output before/after the same-frame native/previous-recipe replays.
The C build passes `-Wall -Wextra -Werror`. The previous playable checkpoint
is unchanged. This is a validated optional lighting variant, not full parity.

| View | Previous recipe HDR MAE | Live full diffuse HDR MAE |
| --- | ---: | ---: |
| Interior | .0015860 | .0009192 |
| Upper | .0154876 | .0084790 |
| Streetlamp | .0103961 | .0057848 |
| North | .0015949 | .0008010 |
| South | .0032317 | .0033522 |

These are same-frame diffuse counterfactuals against frozen original output,
with the established geometry/stencil and HUD/weapon exclusion mask. Four
views improve; south regresses 3.7%. Across separate final-screen captures,
upper luminance MAE improves .02582→.01755, interior .00670→.00564, and north
.00642→.00538. South worsens .00856→.00888. Animated phases differ; these
screen comparisons do not isolate only the lighting change. Streetlamp final
error remains .07362, so its later rendering stages still need investigation.

The first attempt, `pipeline-factory-live-diffuse`, stopped on a rain input
guard. Native particle VS b1 screen dimensions at byte224 became zero with
the new composition shader, although the actual target remained 1280×720.
View, eye, time and near/far stayed populated. Declared but unused constants
cannot be assumed initialized. The successful v2 batch disables the private
rain replay to isolate live lighting; native particles remain displayed.

`audit_full_diffuse_composition.py` accepts both diagnostic and live capture
layouts, checks live frame identity, and labels the previous approximation
separately from the displayed full result. Reports and loaded DLL/source are
preserved under `reproduction/factory-live-diffuse-20261013`.

## Combined live rain and corrected clouds

`SRP_LEGACY_SOURCE_RAIN_LIVE=1` uses the pinned original five-shell rain assets
and replaces native particles only after a complete same-frame replay and
target validation. Motion uses the actual GPU camera. Native camera matrices
can contain shear; their full affine inverse is validated against projection,
eye position and matrix-product residuals. The fixed-camera transpose shortcut
alone was insufficient for gameplay. Screen dimensions come from the validated
viewport because the native unused constant slot can be zero.

The Factory rain cloud wrapper corrects U by +.25 and V by / .96. Captured
native VS constants prove the rotation convention differs from the original;
animation speed is zero. The source and target texture bytes, tint, samplers
and blend agree. On 1,542 common sky pixels the corrected cloud pass reduces
HDR MAE .061651→.005928, with byte-identical pre-cloud sky between target
variants. Final sky RGB MAE improves .091236→.022251.

The combined checkpoint has five full-buffer views and two additional hangar
screenshots. Fixed-mask final screen luminance MAE / block SSIM:

| View | MAE | SSIM |
| --- | ---: | ---: |
| Interior | .00574 | .9713 |
| North | .00510 | .9727 |
| South | .00898 | .9371 |
| Upper | .01595 | .8664 |
| Streetlamp | .02048 | .9399 |
| Hangar south-up | .00635 | .9578 |
| Hangar north-up | .00580 | .9822 |

The lamp's prior live-rain screen MAE was .07365 (SSIM .7624). These are
separate animated frames; geometry-only and common-sky audits accompany the
screen metrics. Hangar screenshots have no new depth evidence. Reflections,
material packing, probe clipping, and remaining lamp/lighting differences
still prevent a general parity claim.

The standing character-camera check ran for six seconds after disabling the
director: 360 additional rain applications, changed GPU eye, no rejections,
and about 60 frames/s. This is a short standing check, not a movement or load
benchmark. Synchronous constant-buffer readbacks remain a performance risk.
Disabling the director now also clears its applied enabled flag/id, which had
otherwise left observer bookkeeping stale.

The first combined run acknowledged four views but its lamp readback exceeded
the original 30-second capture deadline. The remaining lamp and two hangars
were captured in a second session with the same DLL/assets and a 120-second
readback allowance. `capture.py --timeout` controls this deadline. The complete
directory hardlinks whole acknowledged pose groups and records their sessions
in `provenance.json`; no pose combines buffers from separate frames.

Evidence: `026g/pipeline-factory-live-rain-clouds-complete`; replay checkpoint:
`reproduction/factory-live-rain-clouds-20261013`. The checkpoint includes the
tested DLL, render-tool source, shader/constant assets, original launch scripts,
reports, capture hashes and a `replay.py` that resolves its assets locally.
It still requires the installed private client/archive, Wine runtime and match
server. The earlier preferred playable preset is preserved separately.

## Completed archive transfer

Three cold batches (`pipeline-factory-encoded-ambient`,
`pipeline-lamp-hangar-encoded-local`, `pipeline-lamp-hangar-local-light`) moved
to `/mnt/hdd/Projects/survarium-render-archive/2026-10-13/resource-porter-cold/`.
All 14,923 files were size/SHA256 verified before removing their SSD copies;
the old paths are symlinks. Logical payload is 15,335,329,853 bytes. The receipt
is `completed-lighting-receipt.json` alongside that archive directory.
NTFS rounds timestamps to 100ns; the verifier records both timestamps and
allows only that precision loss. SSD free space after transfer was 55GiB.
