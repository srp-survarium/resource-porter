# Chemical Plant visual comparison

The frozen comparison is in
`.work/chemical-plant-011a/comparison`. This is a measured improvement, not a
claim of complete renderer, material or gameplay parity.

## Coverage and original reference

`poses.json` contains six stations spanning approximately 187 by 128 metres,
eight horizontal headings per station, and four lamp/shadow/overlook details.
Every view uses a 1280x720 viewport, 65-degree vertical FOV, explicit position,
forward vector and clipping planes. Three stations are held out from diagnostic
work. The original 0.11a executable/archive hashes are pinned by the initial
Chemical Plant checkpoint. The original renderer and archive remain untouched.

The old local reference server's player payload is incompatible with 0.11a:
it crashes in `weapon_core::deserialize`. The private reference server therefore
omits outgoing SpawnPlayer messages and the client runs with `-spectator`.
This permits scene rendering without pretending to implement the new player
protocol. The private server source and binary are preserved. The verified
0.11a camera-director RVA is `0x5c6050`; its register/prologue contract was
checked against its PDB and machine code, not inferred from 0.10b addresses.

All 52 original views were captured once and reused for three target batches:
`native`, `source-lighting`, and `source-lighting-uv`. Beauty frames, camera
metadata, pass bytecode, constant buffers, samplers and available render targets
are retained. UI is disabled in both clients. Camera overrides are independent
of the network player's position and cannot be used as a gameplay collision test.

`tools/chemical-plant/panoramas.py` constructs six 360-by-60-degree panoramas
from the eight calibrated headings. It preserves display RGB without exposure
normalization or seam blending; independent animation frames can produce seams.
Individual frames remain authoritative. `panorama-comparisons` stacks the
original above the target. The original and native panoramas are complete;
lighting+UV panoramas are also preserved.

## Changes and measurements

The adapter uses the existing atmosphere/fog, probe, SSAO, composition and post
experiments with Chemical Plant's authored post constants. The shipped 0.11a
post source multiplies shoulder strength by .01, giving a toe offset of .0004;
linear strength/angle are 6.8/.5, toe strength/numerator/denominator 6.2/1.7/.06.
The 0.11a high-quality diffuse probe averages eight samples over eight, unlike
0.10b's eight over nine. Native height-based ambient is disabled in the saved
project patch when the encoded-probe ambient decoder is enabled.

The separate UV correction is documented in `chemical-plant-011a.md`. It fixes
half-precision collapse on six moss surfaces; all 1,244 newly collapsed UV
triangles disappear. Production conversion reproduces the runtime-tested blobs
exactly. Existing geometry regression extended; 279 library tests pass.

The existing four-pixel image grid gives the following display-luma MAE / block
SSIM averages. It retains historical HUD/weapon exclusions even though UI is
disabled, and uses separate animated frames; it is not a parity proof.

| Views | Native | Lighting adapter | Lighting plus UV fix |
|---|---|---|---|
| Diagnostic/detail (28) | .0867 / .4851 | .0497 / .7181 | .0334 / .8241 |
| Held-out stations (24) | .1046 / .4748 | .0364 / .8116 | .0364 / .8135 |

All 52 lighting+UV captures use the expected composition, post, fog, sky,
present and ambient-decode bytecode hashes. Shader creation counters alone do
not prove a pass was drawn.

## Runtime contract checks

- **UV consumer review completed:** material flags show no active detail tiling, sequence,
  scrolling, variation or vertex-blended masks on the six moss surfaces. A
  dedicated capture identified the actual moss pixel shader through its authored
  diffuse and fuzziness constants. All four active texture samples use the same
  unmodified UV with anisotropic wrap; detail/scroll/displacement constants are
  unused. Bound UV tiling is [1,1]. The captured native vertex shader passes the
  source UV directly (`mov o7.xy, v4.xyxx`), with no upstream scaling, offset or
  UV-derived masks. The matching pixel shader samples all four textures directly
  from `v7.xy` using anisotropic WRAP. This proves integer-offset periodicity for
  these six opted-in moss surfaces; it is not a general claim for other materials.
  Evidence: `026g/vertex-cloud-contract/poses/south_yard_000.bmp.material-draw-16.*`.

## Remaining limits

- **Sky:** four additional upward-facing headings and a twelve-frame fixed-camera
  sequence were captured in each engine. Before the grain and cloud corrections,
  the target lacked clouds and had stronger temporal noise. Mean temporal luma standard
  deviation is .00567 original versus .01408 target; spatial high-pass RMS is
  .00646 versus .01671. Cloud projection/binding, grain, post processing and
  native god rays remain separate hypotheses. The original alpha-blend cloud
  pass was missing from the observer's identity table; optimized target cloud
  constants also escaped tagging. Target cloud tagging is now runtime-verified: the
  authored BGRA cloud texture uses sRGB view 91 and source-alpha blending. The
  native final aberration pass added duplicate grain; the opt-in correction and
  matched stationary measurements are documented in [duplicate-native-grain.md](duplicate-native-grain.md).
  A separate cloud input-layout error is now corrected: the compact adapted
  pixel shader placed TEXCOORD3 at register 1, while the native vertex shader
  supplies that UV at register 4. Preserving the complete native input structure
  restores the expected layout. The isolated two-view A/B changed cloud-pass
  contribution from zero pixels in both views to 131,953 ground-view pixels and
  916,356 upward-view pixels. Texture, constants, equations and geometry were
  unchanged; the active replacement bytecode was captured in both views.
  Evidence: `026g/vertex-cloud-contract` and `026g/cloud-full-input`.
  The Chemical profile also restores the source quarter-turn cloud orientation
  and reverses native vertical UV scaling by .96. All four frozen original sky
  headings independently support the quarter-turn, and four corrected runtime
  views achieve block SSIM .896–.921. At sky_000 the coordinate correction improves
  MAE .06344 to .03239 and SSIM .78381 to .91481 relative to the input-layout fix
  alone. The stronger pink sun glare remains unresolved. Evidence:
  `026g/cloud-source-coordinates-background/sky`.
- **Glossy pipes/rails:** the user's screenshots are preserved with timestamps.
  A source-red/native-green roughness-channel adaptation must retain sRGB and
  be checked against the exact material/SRV. A separate audit found 167 authored
  Float values lost to Integer templates in 83 materials. A reversible experiment
  restored 166, excluding translucency pending semantics. Six matched views gave
  mixed/worse results (east_hall_180 MAE .01656 to .04641), so the archive patch
  was reverted before final panoramas. Numeric preservation alone does not prove
  compatible runtime semantics; exact active-material correspondence remains
  necessary. No global gloss adjustment adopted. Baseline and experiment buffers:
  `026g/cloud-source-coordinates-background/material-baseline` and
  `026g/material-scalars/poses`.
- **Localized invisible collision:** all 599 retained collision instances preserve
  source position, rotation and scale exactly; no added instances or new
  duplicates. The original has 600; the sole omission is the documented absent
  soviet_comp_01 payload. All 445 retained collision payload files were checked:
  285 are byte-identical and 160 primitive configs have identical decoded values
  after serialization. This does not rule out per-mesh consumption differences
  or authored blockers. Runtime traversal with camera override disabled remains
  necessary.
