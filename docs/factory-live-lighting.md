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

## Completed archive transfer

Three cold batches (`pipeline-factory-encoded-ambient`,
`pipeline-lamp-hangar-encoded-local`, `pipeline-lamp-hangar-local-light`) moved
to `/mnt/hdd/Projects/survarium-render-archive/2026-10-13/resource-porter-cold/`.
All 14,923 files were size/SHA256 verified before removing their SSD copies;
the old paths are symlinks. Logical payload is 15,335,329,853 bytes. The receipt
is `completed-lighting-receipt.json` alongside that archive directory.
NTFS rounds timestamps to 100ns; the verifier records both timestamps and
allows only that precision loss. SSD free space after transfer was 55GiB.
