# Source rain shell review

The host C helper matches all 35 original layer matrices within .0000992 and
the seven-pose sequence within the same tolerance. C and Python predictions
are identical after float32 conversion. Initial scroll state is inferred from
the first captured origin; unknown bootstrap history is not reconstructed.
The committed fixture contains the actual original matrices, radii, view
matrices and camera positions in capture order.

The review found one input bug: native particle VS b1 declares `m_V2W` at
byte176 but contains zeros there in all five views. Retained PS b1 is not a
safe substitute: it is only 96 bytes in yard-north. The runtime now reconstructs
the inverse from populated native `m_V`, verifies orthonormality and recovered
eye position, then calculates `world_to_shadow × inverse(m_V)`. CPU replay
matches original `view_to_shadow` within 1.27e-7 across the five views.

| Constant | Native input | Original destination |
| --- | --- | --- |
| Projection/view matrices | VS b1 bytes0–175 | b1 bytes0–175 |
| Scene time | VS b1 byte400 | b1 byte284 |
| Screen resolution | VS b1 byte224 | b1 byte288 |
| Shadow transform | PS b0 world transform × reconstructed inverse view | PS b0 byte48 |
| Layer world matrix | Motion helper | VS b0 byte112 |

RDEF and SHDR confirm that source VS dynamic reads are only cb0 rows7–9
(`m_W`); `m_WVP` and `m_WV` are unused. Source PS uses cb0 rows1–6 and cb1
rows8–10,17–18. Source near/far, inverse view, eye position/direction and fog
are unused in this exact shader pair. A declared field need not be populated.

All 35 original draws match the selected geometry byte-for-byte: stride24,
432 indices, R16_UINT, triangle list. The five pinned asset identities pass.
Captured samplers match linear-wrap, point-clamp, point-clamp, linear-clamp,
comparison-linear-min/mag-point-mip-clamp. Source registration confirms shadow
LESS_EQUAL; the effect uses depth testing without writes, no culling and
SRC_ALPHA/INV_SRC_ALPHA blending. The scroll shader samples only mip0.

The first five-view runtime passes byte identity checks for private step0
against native pre-rain color, sampled frame against step0, sampled encoded
lighting against same-frame `sun-encoded-2`, and sampled shadow against native
rain shadow. All 25 shell matrices agree with C/Python within 3.06e-5, radii
within 9.54e-7, and shadow remapping within 2.99e-8. Source frame serials agree
with the intercepted native draw. The subsequent native draw retains its
shader identities, stride36 geometry, native texture/shadow SRVs and samplers.
Private counts are declared 5×432 plus five captured output steps; they are
not a separate GPU primitive counter. OM/raster restoration is code-reviewed,
not independently captured.

`audit_rain_shell_runtime.py` checks these contracts and measures raw HDR rain
contributions over every pixel in a fixed 3×3 grid plus the full frame. It
compares each rain output minus its own pre-rain input. Different clocks,
scroll history, pre-rain color, lighting and depth prevent pixelwise phase
parity claims. The first result is close in upper/lamp views and differs in
the sparsely rainy yard views; the diagnostic is not a general recipe change.

Reproduce the independent math check by compiling a host shared library that
includes `legacy_rain_math.h` and exports `layers(float,const float*,float*,float*)`
and `advance(LegacyRainMotion*,const float*,const float*,float*)`, as in the
preserved `rain-math-test.c`. Run `audit_rain_math.py --motion-tools DIR
--c-math-library LIB --output REPORT`. Run the runtime audit with the same
arguments plus `--reference ORIGINAL_POSES --target TARGET_CAPTURE`.

Evidence: `.work/chemical-plant-011a/comparison/checkpoint/factory-rain-shell-review`.

## Full-pixel comparison of two runtime batches

All contract checks pass independently for both five-view batches. The table
shows full-frame **mean raw HDR luma added by rain**, not image MAE:

| View | Original shells | First private replay | Last-mip private replay | First native particles |
| --- | --- | --- | --- | --- |
| Interior | .00003107 | .00002573 | .00002324 | .000000582 |
| Upper | .00131534 | .00126206 | .00124173 | .00002254 |
| Streetlamp | .00057257 | .00057916 | .00060774 | .00001432 |
| Yard north | .00001020 | .00001185 | .00000909 | .000000944 |
| Yard south | .000000647 | .000002811 | .000001830 | .0000000315 |

Across the 45 predefined grid cells, the first replay's mean contribution is
closer to the original than native particles in 27 cells, farther in three,
and equal in 15. The last-mip variant has 25 closer, four farther and 16 equal.
Mean absolute differences between regional mean contributions are .00003316
and .00004445 respectively, versus native particles' .00037848 and .00037627.
These distribution comparisons support restoring the original rain's strength
and coverage, not matching individual drops or proving the mip variant better.
The variants have different clocks/history and pre-rain inputs.

Upper-view pixels changed by rain are 18.76% original, 18.84% first replay and
0.97% native particles. Streetlamp coverage is 8.90%, 9.57% and 0.61%. Sparse
south rain remains discrepant despite its small absolute contribution. Some
individual layers correctly produce no changed pixels in sheltered views;
all five output steps are preserved, including those unchanged steps.

Batches: `pipeline-factory-source-rain-shells` and
`pipeline-factory-source-rain-mip-last`. Separate complete reports are
`rain-shell-runtime-audit-full.json` and `rain-shell-mip-last-audit-full.json`
in the review checkpoint. The audit defaults to every pixel; optional coarser
sampling uses a globally aligned grid and should not replace full-pixel
statistics for narrow, sparse rain streaks.
