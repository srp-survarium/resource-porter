# Factory rain: live 0.26 path versus original 0.10b

The five-view `026g/pipeline-rain-lamp-contract` batch observes one particle
draw per view, with 30,720 indices and no cone or resolve draw. Every rain
report covers one Present serial. `verify_rain_capture.py` verifies shader
files, geometry bounds and before/output buffers for all five views.
The frozen reproduction record is `reproduction/rain-lamp-observer-20261013`.
This identifies the current path; it does not restore original rain.

Original `stage_rain.cpp` constructs `sphere_geometry(24,3)`: 100 vertices,
432 indices, triangle lists, a 24-byte POSITION4/TEXCOORD2 vertex layout.
The settings call these layers cones. Each layer receives a scaled/rotated
world matrix, radius, density, scrolling speed, UV scale and rain-shadow
projection. Camera movement also changes scrolling offsets. A shader-only
replacement on native particle geometry cannot reproduce this behavior.

The native rain stage hardcodes technique 1 and renders particle geometry.
Its effect still contains the older technique 0, but no corresponding console
selector was found. Switching that index alone would retain incompatible
geometry and constants. The School worker records the instruction evidence
in `docs/survarium-021-school-rain-reference.md` in its worktree.

Archived cone VS instructions and input/output signatures match between
0.10b and 0.26. The original PS is 3,620 bytes with DXBC identity
`9004247427fb3a6a9e5dd19aea0c46d5`. Its PS constants need explicit remapping:

| Value | Original byte offset | Native byte offset |
|---|---:|---:|
| b0 UV scale | 16 | 0 |
| b0 radius / density | 24 / 28 | 8 / 12 |
| b0 speed | 32 | 16 |
| b0 shadow matrix | 48 | 32 |
| b1 scene time | 284 | 400 |
| b1 screen size | 288 | 224 |

Both cone shaders bind depth/rain/frame/shadow textures at t0–t3. The old
shader additionally requires diffuse lighting at t4, with material sampler s3
and shadow comparison sampler s4. It uses one shadow comparison and lighting
in its opacity calculation. Native cone shading uses four biased comparisons
and different color/opacity equations. Restore the old lighting dependency;
an arbitrary stale t4 binding is not sufficient.

The archived `engine/scroll_rain_0` texture also differs: both are 512² DXT5,
but 13,428 DDS bytes differ in the pixel payload. Original and native copies
and hashes are saved in
`reproduction/factory-rain-source-contract-20261013/textures`, alongside the
source renderer and exact shader/binding audit in that checkpoint.
Use the original texture when reconstructing the original layered renderer.

The targeted original batch is now frozen at
`010b/rain-lamp-contract-sized`, with reproduction tools and a 2,512-file
capture manifest in `reproduction/reference-rain-lamp-20261013`.
All seven views render loaded geometry at 1280×720. The source HUD is visible;
mask it in final-image comparisons. A preceding 1272×686 attempt is retained
at `010b/rain-lamp-contract` but excluded from matched-image comparisons.
The executable-scoped viewport helper corrected the private window without
resizing another client. Both runs stopped their private client/server and
restored the previous profile.

`audit_reference_rain.py` checks draw coverage and the original pixel-shader
identity before decoding live constants. Every view contains five shell
draws and no particle/resolve draw, all within one Present serial. All 35
draws use identical 100-vertex/432-index geometry. Live radii are 5, 6.6,
8.47, 10.648 and 13.176901; UV scales are (1,1), density 0.8, speed 2.
These match source layer indices 5–9 with a radius multiplier of 1.1.
Captured world matrices and shadow projections are preserved per draw;
they are evidence, not camera-independent replacement constants.

The matched outdoor lamp reference also has a dark head and a detached beam
below it. The current port's beam looks warmer and stronger; these images
do not justify moving its authored geometry. The observer's first native
volume draw belongs to another cone, so diagnosing the target lamp's shading
requires selecting that instance. The batch also adds two upward hangar
views for the separate brick/blending and disappearing-object investigation.

Restoring rain now requires the original layered geometry, camera-relative
transforms and movement offsets, original texture and lighting input, and
compatible shadow projection. The native particle draw is not equivalent.
Reuse this frozen reference batch for subsequent target-only iteration.

`legacy_rain_motion.py` now reproduces all 35 captured world matrices from
the original seed-1000 generator, layer scaling and scene-clock motion.
The original compiled `float2(random(), random())` evaluates the second
argument first: using textual x/y order fails the observed rotations.
Each layer's rotation phase is `scene_time * 0.2`; no fitted phase is needed.
The sequential replay also reproduces camera-relative scrolling across all
seven poses, starting from the first capture's observed scroll state.
It preserves `help_math::frac`'s magnitude/truncation behavior, including
negative offsets. The largest matrix-entry error is below 0.0002.

`prepare_legacy_rain.py` packages the exact source VS/PS, geometry and sRGB
BC3 rain texture mip zero. It checks the shader identity, layout and texture
view, and verifies geometry against `sphere_geometry(24,3)` (maximum vertex
error 0.000000321; indices exact). This prepares assets, not a target override.
The assets and replay evidence are preserved in
`reproduction/factory-rain-motion-20261013`.

The lighting input prevents a geometry-only restoration from being complete.
Original local lights add `sqrt(saturate(linear/8))` contributions into the
encoded diffuse buffer; resolve then decodes the clipped sum. Native local
lights accumulate linear contributions, and the current compatibility
composition re-encodes that aggregate. Inverting the native total does not
recover the original encoded sum. Original local-light Fresnel and shadow
placement also differ. An encoded auxiliary accumulator must reproduce the
original producers before it can supply faithful rain opacity; the local-light
observer is being extended to capture the active contracts.

The target-only `026g/pipeline-lamp-hangar-local-light` batch now records the
selected outdoor cone and both matched hangar views. All 56 local-light draws
use the shadowed spot family, with no overflow and one Present per view.
The 4,856 capture files, loaded DLL and reproduction inputs are preserved in
`reproduction/lamp-hangar-local-light-20261013`. The lamp selector matches
`(11.152,10.196,18.044)` after rejecting two other cones; neither hangar view
contains that cone, and no substitute volume is captured. Local lights use
two geometry passes with stencil writes, so auxiliary replay must preserve
the pre-draw depth/stencil state rather than reuse it after the native draw.

Rain-shadow coverage also differs. Multiplying each original captured
`view_to_shadow` by its captured view matrix gives world-space XY scale 1/64
in all five paired views, versus native 1/128. Original `stage_pre_rain.cpp`
places the shadow camera at `camera - rain_direction*100 + view_direction*32`,
including the vertical view component. The current native center differs:
the upward streetlamp view's shadow Z translation is 0.268874 versus source
0.322356. Therefore School's independently validated projection half-scale
hook alone cannot restore the older Factory shadow contract. Preserve both
projection and camera-position behavior before comparing caster depth maps.

## Private source-shell runtime

`SRP_DIAGNOSTIC_SOURCE_RAIN_DIR` now loads the verified source assets from
`prepare_legacy_rain.py`. It requires the complete source diffuse auxiliary
(including material-gated translucency) and `SRP_LEGACY_FACTORY_RAIN_SHADOW=1`.
The five-view run also enables the independently audited shadow-camera
selection and private factor cache described in
[`factory-rain-caster-contract.md`](../factory-rain-caster-contract.md).
All switches are opt-in and specific to the pinned 0.26 Factory experiment.

`legacy_rain_shells.c` intercepts the identified native particle pass. On a
capture frame it copies the current pre-rain color into a private target and
a fixed frame-input texture, then draws the original five shells there.
Depth comes from this frame's composition input, lighting from the complete
private encoded accumulator, and shadow depth from the actual rain pass.
The original shaders, 100 vertices, 432 indices, texture and sampler states
are retained. Source depth testing is LESS_EQUAL with no writes, culling is
disabled, and blending is SRC_ALPHA/INV_SRC_ALPHA. All modified D3D state is
restored before the native particle draw. The displayed scene is unchanged.

`legacy_rain_math.h` reproduces the seed-1000 rotations, layer scales and
signed camera-scroll wrapping. Independent host execution matches all 35
frozen source world matrices within .0001 and the seven-pose scroll sequence
within .000016 in origin. The initial source scroll state is inferred from
the first frozen capture; the live adapter starts from its own first rain
frame. This does not synchronize clock phase or camera history between games.

The native particle VS declares `m_V2W` and `eye_direction` but leaves both
zero in all five captured views. Its populated `m_V` must be inverted before
forming source `view_to_shadow = world_to_shadow * inverse(m_V)`.
The runtime verifies orthonormality and agreement with the uploaded eye.
Using the declared inverse matrix directly would produce invalid shadows.
Retained PS b1 is not a substitute: it can contain an unrelated 96-byte buffer.
Source pinned shaders use only m_W from their dynamic transform buffer;
unused m_WV/m_WVP rows need not be reconstructed.

`026g/pipeline-factory-source-rain-shells` completed all five 1280×720 views
without hook errors. Captures contain private step 0 before rain and steps
1–5 after each shell, all source inputs, globals, world/parameter buffers and
frame metadata. Private step 0 equals the independently captured native
pre-rain output byte for byte. The exact loaded DLL, source overlays, assets,
scripts, fixed-exposure previews and capture hashes are preserved in
`reproduction/factory-source-rain-shells-20261013`.

The private output restores dense source-like rain where native particles
are sparse. It still uses the current pre-rain scene color and native depth,
while source diffuse/material/shadow differences remain under investigation.
Comparisons of rain contribution must subtract each engine's own pre-rain
frame and retain the unsynchronized phase/history limitation. This runtime
establishes a working source-shell path, not final-image or temporal parity.
