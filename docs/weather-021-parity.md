# RLS Vostok and Mamayev weather parity

The parity conversion is implemented and validated for both maps in evening
and rain. Parity20 contains 7,013 resources and 299 converted materials. All
four variants have 18 reviewed fixed-camera comparisons, including elevated
panoramas and close views. Slight evening differences remain within the user's
accepted scope; foliage motion, particles and small surface differences remain.
No executable renderer patch is retained. The original baseline transformation
is preserved separately; the sections below record the investigation.

`ports/survarium-021d-two-maps-weather-to-026g.json` preserves the first
evening/rain conversion separately from the released day manifest. Both use
the same 0.21d build 2010 donor and 0.26g build 2777 target, pinned by hash.
The weather baseline retains donor lighting (`native_lighting: false`) and
the released importer's native material finishes. IDs are 167/168 for RLS
Vostok and 176/177 for Mamayev; map numbers remain 10/11.

Build with `build-survarium021-maps`, apply against the pinned stock target,
and verify, as in [the conversion documentation](survarium-021-to-026.md).
The preserved baseline contains 6,957 resources. Its applied database hash is
`731b578a8f53066693102f62d2a5450b91a1bbb3fc10b54d525319eb2e585683`.
Keep this manifest/output when introducing parity improvements: the user
explicitly requested preserving the existing 0.21-to-0.26 transformation.

## Capture procedure

`tools/render-compare/build.sh` builds a screenshot/camera-only diagnostic DLL,
a viewport helper and the original 0.21 reference server. It does not replace
shaders. `prepare_reference.py` stages an isolated original client;
`prepare_client.py` pins the executable and records diagnostic compatibility
patches. The 0.21 direct-spectator patches skip unavailable lobby callbacks and
a null statistics reset, and retain full texture mips under Wine.

`run_reference.py` accepts a prepared runtime, a project and fixed camera JSON.
For a port capture add `--build 026g --map ID --server MATCH_SERVER`.
Explicit Wine, viewport and xdotool paths are required. Only the runner's own
prefix and process groups are stopped. Capture manifests hash the runtime and
images. Camera position, direction, FOV and 1280x720 dimensions are checked.
Loading/gameplay directors differ in 0.26; readiness observes all camera calls,
then the first request selects the active director. Win32 restoration handles
windows that remain suspended after an X map/focus operation.

`make_overviews.py` proposes elevated views from authored gameplay positions.
They require visual inspection: terrain and trees can obstruct a candidate.
Keep wide panoramas and closer elevated views with multiple visible gameplay
features. These are individual perspectives, not 360-degree panoramas.

Optimize one matching screenshot at a time. Freeze the original reference;
recapture the same pose after a conversion change, then recheck earlier views.
Evening and rain are in scope; night is not. The original reference sets are
complete and frozen: do not launch 0.21 or capture more original frames.
Continue comparisons by capturing only the 0.26 port at those saved poses.

The user accepts slight evening differences and prefers no executable renderer
patches. Keep the accepted resource/shader conversion and prioritize the rain
audits. Do not pursue executable renderer changes to remove the remaining
evening differences.

## First observation

Mamayev evening `mamayev-close-03` has a clear view over paths, rocks, foliage
and the horizon. The baseline 0.26 image has flatter grey lighting and a pale
band on distant hills compared with 0.21. Geometry and camera framing align.
Investigate environment/shader semantics before tuning material constants.

The user screenshot `2026-10-18-06-25-27.png` depicts a conversion result to
preserve, not an original reference. Original 0.21 captures remain the parity
target. Local evidence is retained under `private/map-weather-20261018` in the
companion skhid-srp workspace, including the baseline database, build/apply/
verification logs, camera poses and capture manifests.

## Parity iteration 1: fog height

The separate `survarium-021d-two-maps-weather-parity-to-026g.json` tracks accepted
parity adjustments without changing the preserved baseline manifest. The first
iteration changed Mamayev evening's `mie_fog_base_height` from inherited 75 to 0.
The manifest's `environment_overrides` can edit existing environment properties
only, with compatible scalar/vector types; unknown keys and invalid types fail.

Disassembly of the retail `atmospheric_scattering_on_geometry_temporary.ps`
shows original 0.21's height factor `exp(-eye.y * falloff)`, while 0.26 uses
`min(exp(-(eye.y - base_height) * falloff), 1)`. The old shader has no base-height
offset. The first correction therefore removes a target-only inherited offset.

On `mamayev-close-03`, this removes the pale band and reduces raw display-RGB
mean absolute error from 39.05 to 19.12 out of 255. This is a diagnostic trend,
not parity acceptance: particles, clouds and vegetation are unsynchronized,
and sky/distant-terrain differences remain. Original 0.21's fog also differs
in its distance function, Rayleigh color source and final RGB scale; resetting
height does not make the shader equations identical.

`compare.py REFERENCE.json CANDIDATE.json --output DIRECTORY` verifies both
capture hashes and matching camera parameters before producing an original-left/
candidate-right image, an absolute-difference image and numeric diagnostics.
Optional named region rectangles support separate foreground/horizon checks.
New captures record the resource database hash as well as runtime/image hashes.

## Read-only fog audit

`run_reference.py --audit-fog` tags the pinned retail fog shader by its DXBC
digest and saves its pixel constant buffer at the requested settled camera.
It checks the uploaded eye position against the requested pose so a lobby or
loading draw cannot be mistaken for gameplay. The audit does not replace
shaders or constants. Raw bytes retain nonfinite values; the JSON view writes
them as `null`. Required audit files are hashed in the capture manifest.

On the first close view, both builds upload identical eye position, sun color,
sun direction, Rayleigh near/far distances and density. However, 0.21 uploads
Rayleigh RGB `[2.74699569, 2.74699569, 2.74699569]`, while 0.26 uploads zero
there and samples a scattering texture instead. Merely retaining the authored
Rayleigh color property therefore does not retain the original appearance.

The original Mie height multiplier is **positive infinity**, not the reciprocal
of its authored 25.37. Recovered 0.21 symbols and retail code explain why:
`math::lerp<environment_properties>` (VA `0x1abd99`) zero-initializes its output
and skips the `mie_fog_height_falloff` member at offset `0x15c`.
`renderer_context::set_static_frame_parameters` (VA `0xef8e2`) then uploads
`1 / height_falloff`. The loader and copy operations do include this member.
These are disassembly virtual addresses with image base `0x10000`, not RVAs.
At the current positive-height camera, the original draw contributes no Mie
fog. Treating the authored value as the observed renderer value would be wrong.

The four-map parity manifest now enables the checked fog and sky compatibility
for Mamayev evening. Other maps, weather and additional camera elevations still
need verification.

## Opt-in original fog arithmetic

`legacy_fog_021: true` enables a guarded original-fog branch for an individual
map. It requires donor lighting and rejects simultaneous `mie_fog_*` or
`rayleigh_fog_*` overrides. The default and preserved baseline emit no shader
change. Retail shader bytes come from the user's pinned databases.

The porter verifies both retail fog resources, retains the native camera/depth
input convention, and transplants the original fog instructions. Three binding
changes supply the original authored Rayleigh RGB, the observed infinite height
multiplier, and the Mie far distance at its new location. Original RGB is derived
with the original `pow(color, 2.2)` conversion. The map receives a unique marker
in the target-only base-height field; all its authored colors remain intact.
Unmarked maps execute the unchanged native instruction body. Native reflection,
input/output signatures and resource metadata are retained, and the DXBC
checksum is rebuilt. Input hashes and instruction-match counts fail closed.

Trial 03 tested Rayleigh alone and restored the missing distant haze. Trial 04
uses the complete original arithmetic through the porter and reproduces that
result. The live draw audit confirms the generated shader and expected map
marker, with the original sun and fog constants intact. Whole-image MAE is
19.09/255 versus iteration 1's 19.12; this small overall difference is not an
acceptance claim. The sky remains substantially darker/oranger than the original.
All captures still use the same frozen `mamayev-close-03` reference.

For a generated shader, add `--audit-fog --fog-digest DXBC_DIGEST` to capture.
The observer then selects only that shader, and the runner verifies its identity.
This option observes constants only; it never substitutes rendering data.

## Opt-in original sky arithmetic

`legacy_sky_021: true` adds the original sky calculation for that map and requires
`legacy_fog_021` so both use the same map marker. The original and native shader
input/output signatures match. The porter copies the complete original sky
instruction body and retains the native body for unmarked maps. It enables four
existing reflection variables used by the added branch, preserving their names,
types and offsets. The native scattering textures are still used.

In trial 05, this lowers the first close view's MAE from 19.09 to 13.70/255;
the fixed sky region falls from 51.55 to 24.65. Live source/target audits confirm
the direction, sun color and fog parameters required by the original sky math.
The original draw constant's unused fourth component differs between builds;
the original shader reads only its direction components. Cloud/sky differences
remain, so this is progress rather than final parity.

The integrated four-map build emits byte-identical fog and sky shaders to the
tested experiments. It builds, applies and verifies as 6,959 resources; its
database SHA-256 is
`f9ff9d07398dab64f7c6fe4d78c3066c64a95b42991daaefd597d4c66b30f5b1`.
Only Mamayev evening opts in; the other three variants remain on the preserved
baseline behavior.

The integrated build was checked at `mamayev-close-03`, `mamayev-southwest`,
`mamayev-east` and `mamayev-close-06`, with matching poses and active shader
audits. Their raw RGB errors are 13.62, 11.40, 15.37 and 14.41 respectively.
The wide views expose remaining cloud placement and color differences, and
unsynchronized smoke/vegetation still affect these measurements. These four
checks support retaining the improvement, not calling Mamayev evening finished.

## Experimental original clouds

`legacy_clouds_021` requires the fog marker and restores the original cloud
pixel instructions in all eight native variants. Texture/sampler slots and
the lit variant's sun constant are remapped to their native bindings. The
original fog-power row is marked used in reflection, so the engine supplies
it. Unmarked maps retain their native instruction body.

The converter reads the donor before filling target properties. Missing cloud
fog color defaults to white and missing cloud lighting to false, matching the
observed original Mamayev draw. It neutralizes the target-only sunlight tint
multiplier and converts both layer rotations from the original `(cos, sin)`
upload convention to native `(sin, cos)`. Authored rotation speeds remain.

Trial 07 builds, applies and verifies. Its settled cloud audit matches the
original tint `[2.00530, 1.44464, 1.04453]`, fog power/up-limit `[2, 0.25]`,
white fog color and static-layer rotation/radius. However, close-03 raw RGB
error is 14.53 versus the accepted build's 13.62, and sky error is 29.18.
This cloud-only trial was not accepted. The option was subsequently enabled
together with the scattering correction below, after four-view validation.
Correcting individual inputs is not sufficient for visual acceptance.

The read-only draw audit accepts a comma-separated set of candidate shader
digests and also records vertex constants and render-target blending. This
helps distinguish native shader variants and diagnose compositing differences.

All 342 ordinary porter tests pass; the separately invoked retail integration
test verifies fog/sky output hashes and unchanged native cloud instruction
bodies/signatures for all three distinct cloud programs. Retail inputs remain
local and are not included with the tests.

## Original atmospheric lookup generation

The cloud investigation exposed a separate input change in the sky lookup
generator. Original 0.21 uploads Mamayev evening's authored Rayleigh sunlight
multiplier, 4.67. Native 0.26 uploads sun intensity, 2, in that position; its
three other lookup parameters remain `[42.78, 2.27, 75.09]`. Live startup
draw audits confirm this. The native upload at VA `0x1a446a` loads environment
offset `0x48`, then uploads the four-component lookup parameter at `0x1a44a7`.
The native shader also changes two green-channel scattering coefficients.

Changing only the coefficients (trial 08) barely changes the screenshot.
Restoring the authored gain as well (trial 09) makes both generated lookup
textures byte-identical to the original. The frozen close view's sky error
falls from 29.18 to 3.39/255; whole-image error falls to 10.11. A fresh original
capture differs from the frozen reference by 1.86 in the sky and 4.24 overall.
These measurements retain normal animation and image noise.

`legacy_scattering_021` requires `legacy_sky_021`. Its original instruction
branch retains the original coefficients and uses the donor's authored gain.
The adapter adds the native sky's existing static-global buffer layout to the
lookup shader, enabling only the existing environment marker. It retains both
native lookup parameter bindings and the unchanged native instruction body.
No authored environment parameter is repurposed for the additional guard.
The integration test checks native fallback instructions and signatures,
including a second marked profile and zero authored gain. The ordinary suite
now passes 343 tests.

The first guarded build exposed a reflection-reader constraint: its resource
binding table must precede the referenced names/type records. Appending the
table after them caused a startup failure. The corrected adapter keeps the
table in place and retains the retail `atmosphere_rayleigh_mie_parameters`
name. Independent reflection checks cover D3DCompiler 43 and 47; the retail
integration test checks the table order and unchanged native lookup bindings.

The corrected four-map build (6,968 resources) verifies with database SHA-256
`8c183ae9e39c97303d8c10c32f3997dcbdf0744051dc95ffcc8f285729367c95`.
Only Mamayev evening enables all four atmosphere options. Both lookup textures
are byte-identical to the original in the integrated runtime as well. Four
matching views improve over the previous accepted atmosphere build:

| View | Previous error | Original clouds + scattering |
| --- | ---: | ---: |
| close-03 | 13.62 | 9.51 |
| southwest | 11.40 | 6.49 |
| east | 15.37 | 5.30 |
| close-06 | 14.41 | 11.96 |

Cloud placement and sky color now agree closely in the wide views. Remaining
terrain/foliage shading differences and unsynchronized animation still affect
the close views. Other weather variants remain to be matched.

An unchanged Mamayev rain control confirms the native fallback's appearance:
before/after error is 2.43 overall and 0.55 in the sky, with the visible changes
confined to animated rain, smoke and vegetation. The parity manifest now retains
all four atmosphere options for Mamayev evening; rain conversion is next.

## Mamayev rain investigation

Applying the four atmosphere options to rain restores its overcast sky (the
baseline rendered it black), reducing the first close view's sky error from
174.76 to 7.48. The lookup textures match the original within small floating
point differences. This candidate is **not accepted**: persistent bright blue
smoke and darker ground remain; a repeated capture reproduces both.

`run_reference.py --trace-after-audit` adds small read-only target snapshots
after each subsequent draw in the audited frame. It requires the three audit
options, records pixel/vertex DXBC identities, stops at the screenshot frame
boundary, and caps recording at 1,024 draws. These snapshots use every eighth
pixel and are diagnostic thumbnails, not parity references. The capture
manifest hashes the trace and its raw targets. `cargo run --release --example
shader-digests -- resources.db` maps those identities back to retail resources.

This trace identifies `offscreen_particle_lighting.ps` as the source of the
blue smoke. Its relevant original/native pixel programs differ only in
constant-buffer layout. The vertex shader supplies their fog interpolation:
0.26 samples the sky lookup, while 0.21 uses the authored Rayleigh color and
multiplies combined fog color by 0.2. Restoring rain's authored sky lookup gain
therefore exposes another missing atmosphere conversion in particle vertices.
The geometry fog pass precedes this effect and contains no blue smoke.

The captured original vertex digest is `9b11960c94a13675b92cbebf3d4dc919`;
the corresponding native digest is `2a73464d45e748c014a63b25f5eaa660`.
A diagnostic trial transplanted only the original vertex fog block under
Mamayev rain's existing marker, retaining native particle geometry, lighting,
declarations and unmarked behavior. This removed the large blue smoke plume
and reduced whole-frame error from 28.73 to 15.22. Extending the same checked
block to the remaining active variants removes the smaller blue artifacts too
(15.16 whole-frame error; 3.60 sky error). Close ground remains too dark.

`legacy_particle_fog_021` implements this opt-in conversion and requires
`legacy_fog_021`. It reads the pinned original and native template from retail
databases, matches the complete native fog block and its following register
reset, and adds separate scratch registers for the original arithmetic.
All 24 matching variants in the 480-variant native particle family are
converted; other variants remain unchanged. The builder checks both counts.
The original color, far-distance binding and observed infinite height falloff
are restored, including the original 0.2 particle-fog color multiplier.
The native fog block remains intact for unmarked maps. Retail integration
tests verify native metadata, subsequent lighting and fallback instructions.

The full porter bundle contains 6,992 files; its database hash is
`d67a2c139ae5a6bef6980dd8d0fdbb1365622fd725a42e33d60c60db33f88ed7`.
Build, apply, verify and four rain views pass. Errors against the originals
are 15.20 (close03), 16.45 (close06), 6.97 (east) and 6.53 (southwest).
Wide sky and haze are much closer; close ground remains darker, and small
particle differences remain. This is an accepted improvement, not final parity.

The four earlier evening views retain their appearance (errors 9.92, 12.29,
5.29 and 6.51 in the same order). The accepted parity manifest now enables all
five options for Mamayev rain; Mamayev evening retains its four atmosphere
options. Vostok evening and rain remain on the preserved conversion pending
their individual comparisons. Validation: 344 ordinary tests passed, plus both
retail shader integration tests invoked explicitly.

## Vostok evening

The first view is `vostok-east-quarter`, a closer elevated view containing the
dish, surrounding gameplay structures, forest and sky. Its original 0.21
capture is frozen. The preserved conversion scores 26.01 whole-frame error
and 47.63 in the sky region. Enabling the five checked atmosphere options
reduces these to 7.31 and 5.29. The read-back sky lookup differences are at
floating-point rounding scale (mean absolute errors below 0.000001).

The subsequent regression retains the improvement: east-quarter 7.76,
south-quarter 7.43, close03 9.35 and close06 6.46. The close views include the
concrete bridge and metal walkways inside the playable area. Remaining
differences include foliage/material shading, moving shadows and stronger
native sun flares. These frames support retaining the atmosphere conversion,
not declaring complete visual parity.

The accepted manifest enables the five options for Vostok evening. Its
6,992-file database is
`8366f29194eb5dc698c27594ab7b33f4747599eeb174dd58b496b0c45e6884a8`;
build, apply and verify pass. Vostok rain's individual comparison and the final
cross-map regression are next.

For diagnosis, `--audit-render-target` reads the selected draw's color target
and its first two supported uncompressed pixel textures, with formats and
dimensions recorded. `--audit-after-draw` selects the output immediately after
the draw. Startup lookup constants are recorded separately in JSONL, since
those draws precede the settled camera. These options copy GPU resources for
observation and do not substitute shaders, uniforms or render state.

## Vostok rain and forward water

The five atmosphere options restore Vostok rain's sky: first east-quarter
error falls from 51.67 to 9.31 overall and from 162.00 to 3.01 in the sky.
That intermediate candidate is rejected because foreground water renders
bright blue-white. A read-only draw trace identifies `forward_water.ps`,
before particles, as the pass introducing these patches. Its observed native
digest is `915734a684b44d6d7f5bb3a2e8945e50`; the original matching draw is
`e4f112a3322ff4772e0fa9404d5a7f9f` (both draw the same 63 vertices).

The water shader embeds the same changed atmospheric equations as particle
vertices: native lookup sampling replaces authored Rayleigh color, height and
far-distance arithmetic differ, and the original combined fog color carries
a 0.2 multiplier. A guarded transplant of only the original terminal fog
block restores the orange-brown surfaces. First-view error becomes 7.18
overall, 2.61 in the sky. The diagnostic four-view check scores 7.60
(east-quarter), 6.86 (south-quarter), 22.51 (close03), 17.55 (close06).
Close rain surfaces remain visibly darker than the originals.

`legacy_water_fog_021` requires `legacy_fog_021`. It converts eight exactly
matching variants in the 120-variant native water family. Original and native
templates are fingerprinted; the entire native terminal block must match
and end at the return instruction. The adapter preserves native waves,
reflections, lighting, resource bindings and all unmarked behavior. It shares
the checked operand remapper with particle fog, with separate scratch space
and the proper input/constant-buffer bindings for each stage. No retail
bytecode is distributed in source. Retail tests check metadata, all preceding
material instructions, multi-profile branching and the complete native
fallback, plus rejection of altered templates and duplicate profile markers.

Validation: 344 ordinary tests pass; all three retail shader tests pass when
invoked explicitly. The full four-map porter bundle contains 7,000 files.
Build, apply and verify pass. Its database is byte-identical to the diagnostic
archive used for the four-view check, with SHA-256
`34fbff6b41084837111d1cf17b0d8ba2a61dbfaf653a4111fa63fd3e19f4d561`.
The parity manifest retains all six options for Vostok rain. Close wet-surface
shading and stronger native evening flares remain; parity is incomplete.

The original 0.21 screenshots are fixed references, captured once. Reuse the
saved shader constants, draw traces and raw targets as well; further visual
iteration uses 0.26 captures only. Do not launch new 0.21 captures.

## Remaining rain shading investigation

Saved original and native wet-surface audits agree on the authored surface
intensity (0.37 for Mamayev rain). Both shaders modify gloss and normals,
using different equations. A target-only diagnostic restored the original
body under map177's marker, remapping the density and time bindings. The
active shader was confirmed, but close03 error changed only from 15.14 to
14.64, and foreground rock error from 24.16 to 23.79. The obvious darkness
remained, so this trial was not retained in the porter.

Shader observation can validate unused/stale eye-position constants against
the inverse-view matrix's translation and forward direction. JSON records
which validation succeeded. R16G16_FLOAT readback is now supported for the
wet pass's normal target; the observer still only copies GPU resources.

The subsequent target-only trace covers wet surfaces, probes, ambient lights,
AO and the later fog/effects passes. The active high-quality probe shaders
are `a70e272e38e42666e139c6c9db335166` and
`4f5e7dc1cf82ffa2512c125a42fe1be5`. Their diffuse equations largely match
the original; the AO application differs by the original 2.5 multiplier.
The upstream AO calculation also changed, so restoring that multiplier is
not a faithful conversion on its own. A target-only diagnostic with the
original 2.5 application multiplier improves close03 error to 8.03, but the
original generator stores AO at 0.4 scale. That isolated brightness change
is rejected. Restoring the original quality-1 generator, normalized for
the native filter/application range, changes close03 error only to 14.10
and leaves the dark foreground. That diagnostic is also not retained.
The recovered original stage and effect compiler confirm quality 1 selects
the standard `ssao_accumulation` technique; no original recapture was needed.

## Mamayev water regression

The saved Mamayev rain draw trace uses the same forward-water shader as
Vostok. Enabling `legacy_water_fog_021` for map177 changes only eight water
variants; the other 6,992 files are byte-identical to the accepted archive.
Build, apply and verify pass. The archive SHA-256 is
`8fc5d6f4bb291f5ec9e36475da2bc182d35d41bf570d2eaa263c5d56d489f13e`.

Against the fixed originals, four target-only checks score 15.22 (close03),
16.49 (close06), 6.96 (east), and 6.49 (southwest). They remain stable relative
to the prior atmosphere correction. The parity manifest enables the original
water equations for Mamayev rain too. Some small bright specks and the dark
foreground persist; water fog does not account for those differences.

## Rain diffuse attenuation

The fixed post-fog raw targets locate the main close-ground mismatch before
post-processing: the native foreground's linear RGB is about 45% of the
original, while the sky is about 99%. A probe-specular diagnostic restores
the original sampling equations and inlines the original screen-space BRDF
factor. That changes close03 error only to 14.06 (foreground 23.28), so it is
not retained. In particular, original `t_probe_brdf` is a separately computed
screen-space factor; it must not be confused with native AO or the native
two-dimensional BRDF lookup.

The active native `composition.ps` has an additional terminal `use_rain`
block absent in original composition. It attenuates accumulated diffuse
lighting using the rain shadow map and material parameters. Ordinary
rain-exposed surfaces receive a 0.5 multiplier. The saved source composition
has neither this block nor its rain-shadow resources. A target-only audit
confirms `use_rain = 1` in the dark Mamayev view.

`legacy_rain_albedo_021` bypasses that extra attenuation for marked maps.
It requires rain weather and `legacy_fog_021` for the existing map marker.
The source and native templates are fingerprinted. Only four byte-identical
native composition permutations out of 64 are changed. Two instructions per
marker gate the existing rain condition; all sun lighting, material math,
the full native rain block, and every unmarked map remain intact. The native
reflection metadata marks only the existing map-marker constant as used.

The first view improves from 15.14 to 9.35 overall, and from 24.32 to 13.36
on the foreground. Four-view errors improve from 15.22 to 9.19 (close03),
16.49 to 9.88 (close06), 6.96 to 4.13 (east), and 6.49 to 3.84 (southwest).
The parity manifest enables this option for Mamayev rain. Remaining close
material/foliage differences and occasional bright specks are still visible.

Validation: 344 ordinary tests pass; all four retail shader tests pass when
invoked explicitly. The new retail check verifies exact equality with the
audited diagnostic shader, metadata and checksum, multiple markers, invalid
input rejection, and recovery of every native instruction after removing
only the profile guards and extra scratch register.

The complete four-map bundle contains 7,004 files. Its compiled composition
programs exactly match the rendered diagnostic; each variant retains its
own native asset header. The full stock-base archive SHA-256 is
`894e5ff966880bb927a1756c320f8bb8e44430fa66b5e9b26dff485f8bded0ce`.

Vostok rain is now validated with the same option. The close concrete view
improves from 22.51 to 10.51; the second close view from 17.55 to 6.09.
East-quarter improves from 7.60 to 5.47 and south-quarter from 6.86 to 5.17.
The first close frame was reused for the four-view comparison. All originals
remain fixed. The parity manifest enables the option on both rain maps.
The full 7,004-file bundle builds, applies and verifies; archive SHA-256 is
`56550a8c2f0aefde97e2f208a294fc479905707faf6a3d85800ff79668b490e3`.
Only the four composition variants differ from the prior accepted bundle.

## Ambient occlusion after the rain-lighting correction

The original and native ambient-volume shader programs are byte-identical
(their reflection metadata differs), as is `apply_ambient_lights.ps`.
Restoring the original finishes of the three raycast-identified foreground
ground materials changes close03 error only from 9.35 to 8.94 and leaves the
dark foreground; that material diagnostic is not retained.

Retesting the original quality-1 AO generator after removing the native rain
attenuation improves all four Mamayev rain views. Close03 changes from 9.19
to 8.18, close06 from 9.88 to 8.96, east from 4.13 to 3.90, and southwest
from 3.84 to 3.57. The separately measured foreground region improves from
13.36 to 11.03. The first close frame is reused in the regression comparison.
The read-only draw trace confirms the original generator and the accepted
rain-composition shader both execute. All original screenshots stay frozen.

`legacy_ambient_occlusion_021` requires `legacy_fog_021` for its map marker.
It restores the pinned original quality-1 generator inside the native
`ssao_hemisphere_optimized.ps`, retaining the full native body for unmarked
maps. Original generation stores AO at 0.4 scale and original application
multiplies by 2.5; the adapter normalizes generation to the native filter and
application range. It does not add a brightness multiplier to native AO.
The source noise and downsampled-depth resources are remapped by their
bindings. The native `s_base` and original `s_linear` names denote the same
linear/wrap sampler; the depth sampler remains `s_accumulator`.

The retail test reverses only those resource remaps and compares every
instruction with the independently rendered diagnostic. It also verifies
every native fallback instruction, single and multiple markers, reflection
metadata, asset-header preservation, DXBC checksum, and rejection of modified
templates or invalid markers. Validation passes: 344 ordinary tests and all
five explicit retail shader tests. The full four-map bundle builds with
7,005 files; all 7,004 files from the prior accepted bundle are unchanged.
Only Mamayev rain currently opts in. Quality-1 captures are the validated
scope; other AO techniques remain native.

The exact porter-built archive also passes apply and verify. Its guarded
close03 capture scores 8.21 overall and 11.01 on the foreground, consistent
with the diagnostic's 8.18 and 11.03. The trace confirms the guarded shader
`9a5f48515381083948b6efbc54cecde0` executes. The foreground retains about
72% of its pre-AO lighting, compared with 61% under the native generator;
the unguarded diagnostic retains about 73%. Remaining differences include
animated content, surface shading and small bright flecks.

This correction is retained for Mamayev rain. Archive SHA-256:
`a1e9a0a8386783062167b5e75c7d6688f98ecbcd9a966961f053e8bbac7cd3a4`.
Vostok and evening AO validation, evening flares, and the broad four-weather
visual audit remain outstanding; visual parity is not yet established.

The same original AO option is now validated on the other three weather
maps, one close view first, then the remaining three views. Each first frame
is reused in its regression comparison. No original captures were made.

| Map/weather | Close03, before → after | Close06 | East | South/southwest |
| --- | --- | --- | --- | --- |
| Vostok rain | 10.51 → 9.23 | 6.09 → 3.66 | 5.47 → 5.52 | 5.17 → 5.19 |
| Mamayev evening | 9.92 → 9.16 | 12.29 → 11.75 | 5.29 → 4.95 | 6.51 → 6.13 |
| Vostok evening | 9.35 → 7.92 | 6.46 → 4.77 | 7.76 → 6.99 | 7.43 → 7.19 |

Both Vostok rain close views improve; its wide views remain essentially
stable. All evening views improve. The parity manifest retains original AO
on all four maps. Only the shared AO shader changes from the preceding
7,005-file bundle; the other 7,004 files are identical. Full build, apply and
verify pass. The archive SHA-256 is
`2fd62e556a0028da2c6900247e00cae7db1cfd0e67953d69a339c69a46671b31`.
The rain trace confirms the four-marker shader
`0eaab3f3cfd481f4389dacd6f9f5bf97`; evening audits confirm the expected
map markers and fixed camera positions.

The remaining sun-facing warm glow has now been localized in Vostok's
south-quarter trace. It visibly appears in the post-fog `god_rays_apply.ps`
draw, before water, particles and bloom. In one right-forest region, linear
red rises from 0.109 to 0.260 at that draw. `lens_flares.ps` does not execute
in any of the three traced evening regression views, consistent with the
authored `use_dynamic_lens_flare = false`. The separate lens-flare programs
do differ between versions, but changing those would not address this
observed glow. Original god-rays generation and its postprocess application
are the next comparison; surface shading and the broad visual audit remain
unfinished.

### Original sun-ray radial falloff

`legacy_god_rays_falloff_021` requires `legacy_fog_021` and restores only the
radial falloff in all four pinned native `god_rays.ps` quality variants.
The original shader uses `exp(-20 * distance^attenuation_power)`; native
0.26 uses `(1 - min(distance, 1))^attenuation_power`. For Vostok evening's
authored power 2.13, at screen distance 0.5 these are 0.0104 and 0.2285.
That difference explains the broad orange light over the sun-facing forest.

The adapter copies the five original attenuation instructions, remaps
`god_rays_parameters2.x` from original CB0[3] to native CB0[2], and guards
them with the existing map marker. Projection, mask, sample count and
quality-specific sampling, authored color/intensity, application ordering,
and unmarked maps retain their native instructions. All eight source/native
blobs are SHA-256 pinned. Retail tests cover every quality, multiple map
markers, source arithmetic, complete native fallback, metadata, checksums,
and rejection of unsupported or altered inputs. Quality 2 produces exactly
the shader bytes used in the independently rendered diagnostic.

The first Vostok evening south-quarter view improves from MAE 7.193 to
4.225, with the broad forest glow visibly closer to the frozen original.
The live shader audit confirms marker -32188, the fixed camera and the
corrected shader. Remaining views were checked afterwards, reusing that
first frame: close03 8.186 (previously 7.920), close06 5.023 (4.774), and
east-quarter 7.187 (6.991). These views look similar. Their saved native
traces each contain 3,600 zero sun-ray samples, consistent with the sun
being behind the camera; the small image-error increases should not be
interpreted as proof of a falloff regression.

This is a scoped falloff correction, not complete original sun-ray pipeline
parity. The original mask uses a depth threshold of 400, native 300; their
application ordering also differs. Those are separate investigations. No
new original captures were made. All 344 ordinary tests and all six retail
shader tests pass (eight weather tests including the two ordinary ones).

The parity manifest enables this option for Vostok evening only. Full
build/apply/verify pass with 7,009 files and 299 materials; all 7,005 files
from parity14 are unchanged. The four additions are the quality variants
of `god_rays.ps`. The active quality-2 program is byte-identical to the
validated diagnostic. Archive SHA-256:
`44609b89e28117eba309e37f64091975b35673cdea68a3748ef0970d7ec83ee6`.
Mamayev evening validation and the remaining surface/foliage and broad
visual audit are still outstanding.

The same falloff option is now validated for Mamayev evening. Its first
sun-facing southwest view improves from MAE 6.133 to 2.970. The runtime audit
confirms the corrected two-map shader `459a95be5d74a8827915d8afcbc474f0`,
marker -32197, and the fixed camera. The other three regression views remain
visually similar: close03 9.293 (previously 9.157), close06 11.808 (11.749),
and east 5.168 (4.952). The four quality variants are the only changed files;
7,005 other files remain identical. Full build/apply/verify pass; archive
SHA-256:
`e8b981343dbe7a44ab1da8c2944ad7f1c71cb6f223b7f12eb82e692bf753ea38`.
The parity manifest now enables original radial falloff on both evening
maps. The existing multiple-marker shader tests cover this marker pair.

A broader Mamayev evening audit then checked all 18 usable saved reference
views: 12 elevated panoramas and six closer gameplay-area views. The first
four frames were reused, so this required only 14 additional target frames
and no original captures. Sky, clouds, haze and sun-facing glow align
closely. Wide-view errors range from 2.970 to 9.159; the largest wide error
includes moving smoke. Close-view errors range from 9.293 to 13.627, with
close09 the largest. Nearby grass and foliage remain noticeably different,
so the broad audit does not establish finished parity. Some bright flecks
also remain, although several white gaps are present in the originals too.

Static investigation found another concrete difference in the sun-enabled
original composition (`ec9315c36c9ae2477109a5de7cc57ded`). Its diffuse term
uses Fresnel from the view/halfway angle; native composition uses the
light/normal angle. Native already computes the former for specular, so an
isolated diffuse-angle comparison can retain all other shading operations.
The original sun-disabled composition used in the earlier rain investigation
is a separate permutation; all four unique original permutations lack the
native rain attenuation. Original/native sun specular and foliage scattering
also differ, but should be tested separately. Close09 is the next fixed view
for the diffuse-angle experiment. Other maps' broad audits remain pending.

### Original direct-sun lighting

The isolated diffuse-angle experiment increased close09 error from 13.627
to 13.985 and was rejected. Original foliage scattering alone reduced it
to 13.283, but left the grass colour deficit; that diagnostic is also not
enabled. A fresh accepted-build control measured 13.668, illustrating the
smaller variation between captures.

The capture helper now reads back PS texture slots 0–2, allowing the material
parameters to be inspected alongside normals and albedo. In close09, the
left and central grass regions contain mostly opaque ground with zero
scattering; average roughness is 0.92–0.95. Static ray intersections identify
`ground_moss_s` beneath those regions. Its original and native diffuse RGB
pixels match at every mip level, and normal/roughness textures are identical.
The diffuse blobs differ in lower-mip alpha, not colour. These observations
favour examining rough-surface lighting over a colour-texture substitution.

`legacy_sun_lighting_021` restores the pinned original composition's complete
direct-sun diffuse/specular block, selected by the existing map marker. It
requires `legacy_fog_021`. The original block temporarily changes the material
roughness register; the adapter saves and restores it so later native foliage
and rain calculations retain their inputs. It composes with the accepted
rain-albedo correction and retains all native instructions for unmarked maps.
Original and template blobs are hash-pinned; four compatible native variants
are emitted. Other permutations remain unchanged.

On Mamayev evening, close09 improves from 13.627 to 13.257. Its left grass
region improves from 26.163 to 22.412, central grass from 16.318 to 15.379,
and right ground from 9.708 to 9.459. Subsequent regression views also improve:
close03 9.293 → 9.052, close06 11.808 → 11.384, east 5.168 → 4.865,
southwest 2.970 → 2.616. The five-view contact sheet was inspected; remaining
close foliage differences prevent a claim of completed parity. All original
frames were reused. Live audit confirms the expected marker and shader
`884c60376279964764ac5b498cafa2bb`.

The porter output is byte-identical to that independently rendered diagnostic.
Tests cover sun-only and rain-plus-sun programs, multiple markers, exact
original arithmetic, roughness restoration, complete native fallback,
metadata, checksums and altered/unsupported input rejection. All 344 ordinary
tests and seven retail shader tests pass. The option is enabled only
for the validated Mamayev evening map; other weather maps require separate
first-view validation. Remaining Mamayev views and the other maps' broad
audits are still outstanding.

Full parity17 build/apply/verify pass: 7,009 files and 299 materials. Only the
four composition programs differ from parity16; 7,005 other files are unchanged.
The entire rebuilt archive is byte-identical to diagnostic24 (SHA-256
`f07d88463618d104bc7e2898fcf1fa85e45480cea7677738deef60aee40f0178`).

The subsequent broad audit covers all 18 saved Mamayev evening views. Five
validated target frames were reused and 13 new target frames were captured.
Fourteen MAEs decrease; four increase by 0.045–0.140 (close07, north-quarter,
south-quarter and south). Inspection of all three contact-sheet pages shows
no broad lighting regression. Moving smoke and foliage still affect the
comparison, so small score changes alone are not conclusive. Mean error drops
from 4.972 to 4.864 for the 12 panoramas and 10.942 to 10.764 for the six close
views. Nearby foliage remains darker/more contrasty than the original, and
some existing bright geometry gaps remain. The first broad-capture launch
timed out before scene readiness and produced no images; its clean retry
completed all 13 frames. No original client was launched.

### Vostok sun-lighting rejection and broad audit

A full parity18 candidate enabled original direct-sun lighting for Vostok
evening as well. Build/apply/verify passed; only four composition programs
changed. Removing the added Vostok guard recovers every accepted instruction
and all metadata exactly. The live close03 audit confirmed marker -32188 and
shader `7d3af14e94501a6a81266fa099a409db`.

Close03 improves from 8.186 to 7.591, including the roof region (4.891 to
4.312). The other views do not support enabling it: close06 5.023 to 5.030,
east-quarter 7.187 to 7.583, south-quarter 4.225 to 4.390. Smoothing preserves
the panorama regressions. A fresh accepted-build east-quarter control gives
7.328, still better than the candidate. Parity18 is **not retained**; the
production manifest and current runtime remain parity17. Candidate manifest,
full overlay, four comparisons and raw audit are preserved for further work.

The accepted Vostok evening build now has a broad audit against 18 frozen
originals: eight close views and ten panoramas. Three valid target frames were
reused; 15 new target frames include the control. Close00 is obscured by grass,
southeast has a large white void in the original, and west-quarter is obstructed;
these three original candidates are excluded. All three contact-sheet pages
were inspected. Geometry, sky/cloud placement and distant haze generally align;
nearby foliage, some material shading and sun-facing glow still differ. Blue
anomaly effects occur at different animation phases in some frames.

Close07 has the highest raw error (14.764), but smoothing by 16 pixels reduces
it to 2.742. Southwest is a more useful next fixed view: its 9.449 raw error
remains 8.950 after the same smoothing. The original glow is whiter and the
port yellower. This persistent difference takes priority over chasing
unsynchronized leaf positions.

A target trace beginning at the active sky shader reveals two applications of
`god_rays_apply.ps`, with identical ray-generator output. In the sky pixels,
the first application adds mean linear RGB (0.552, 0.213, 0.016); the second
adds (0.547, 0.212, 0.016). Composition and fog leave those sky pixels unchanged
between applications. No separate sun/moon billboard shader appears from sky
through presentation. The initial billboard-specific audit produced no record;
the subsequent sky trace succeeded. This identifies a concrete pipeline
difference to investigate, not yet a validated correction.

Static original/native postprocess comparison also finds changed tone mapping
and bloom handling. Original tone mapping clamps luminance to 0–0.9 and uses
3 with an explicit conversion from exp to exp2; native clamps 0.05–2 and uses
4.5 without that conversion. Original bloom combines three scales with weights
0.5/0.3/0.2 and packs its output; active native bloom interpolates among six
scales and returns unpacked RGB. Original complex postprocess adds a ray
texture after bloom, whereas native applies rays earlier. Trial25 isolates the
24 original tone-map instructions in place of the 23 native ones, preserving
all other instructions and bindings. It is an unconditional local diagnostic,
not a production option; map guards and porter integration are required if
visual validation supports it.

Trial25 was rendered and rejected. The trace confirms the substituted program
`05f372b23306cfec84d8496725432c7b` at draw 152. Southwest error rises from the
fresh native control's 9.594 to 10.677; sky 11.397 to 13.095, left forest
8.454 to 9.226, right forest 7.811 to 8.483. Restoring tone-map arithmetic in
isolation does not fix the pipeline/input mismatch. The accepted parity17
archive is restored. The next investigation is a faithful correction of the
ray application/order, using the southwest view first. This must preserve
world lighting rather than globally halving ray intensity to compensate for
the double sky contribution. Both rain maps' broad audits and remaining close
material/foliage differences are still unfinished.

### Withdrawn duplicate-ray engine experiment

Native `stage_atmosphere::execute` (VA 0x1a4200) calls the ray routine at
0x1a5520 for both sky (stage type 0) and geometry (type 1). Diagnostic26
skipped the sky-stage call in an isolated capture executable. Vostok southwest
error fell from 9.594 to 5.858; sky and both forest regions improved. Four
regression views gave close03 8.212, close06 5.028, east-quarter 7.202 and
south-quarter 4.518. The last is slightly worse than the accepted 4.225.

Diagnostic27 guarded the skip by the existing Vostok marker -32188 at
scene+0x280. The selected-map trace confirmed one ray application and southwest
error 5.742. Mamayev retained both native applications, with error 2.636 versus
2.616 in the accepted capture. These are diagnostics, **not accepted changes**.
The user then clarified that slight evening differences are acceptable and
expressed a preference against patching the game renderer. Both experiments
are withdrawn. The unfinished porter implementation is archived only as local
evidence, removed from the source tree. The broad experimental run was stopped
after two captured frames. The original capture executable was restored and
hash-checked (`5eb0fc97b983ecc5d98282444076c7dc8180798429c58c4ac35849cb110c993a`).
Parity17 remains the accepted resource/shader conversion, with no executable
renderer correction. Remaining work prioritizes rain validation rather than
exact evening pixel parity.

### Resource-only rain broad audits

Both rain variants now have broad checks against 18 frozen original views,
using accepted parity17 and the restored capture executable. No original game
was launched. Mamayev's first run captured 15 views; three timed out when
rendering stopped advancing. Only those three were retried, with all original
and successful target frames reused through explicit provenance links. Vostok
captured all 18 in one run. Its two obstructed panorama candidates (southeast,
west-quarter) are excluded consistently with the evening audit.

All six ranked contact-sheet pages were reviewed. Wide sky, haze, clouds and
geometry closely align. Mean whole-image error is 4.682 for Mamayev's 12
panoramas and 4.440 for Vostok's ten. Close means are 10.268 across six Mamayev
views and 8.476 across eight Vostok views. These are display-RGB MAEs out of
255, not a percentage or an acceptance threshold.

Near ground/grass and some roofs remain darker and more contrasty. Mamayev
close09 is a useful fixed next view: its 12.253 error remains 11.052 after
16-pixel smoothing, so this is not just unsynchronized wind. Vostok close03
is 9.341 raw and 7.414 smoothed. Small blue-white fire particles and differing
anomaly animation phases are also visible. Some distant floating geometry and
holes already occur in the original views and are not new port regressions.
The next investigation stays in resources/shaders (original probe specular or
material shading), preserving the accepted atmosphere and the unmodified
executable renderer. Slight evening differences are acceptable per the user;
no more executable ray-stage work is planned.

## Original tone curve for Mamayev rain

Probe-only trial28 was rejected: close09 improved 12.253 to 11.506, but east
worsened 3.846 to 4.380 and southwest 3.519 to 3.877. A runtime trace instead
exposed a tone-curve mismatch: adapted luminance is only 0.01924 in close09,
below native 0.26's 0.05 floor. Original 0.21's curve has no such floor. The
previous evening tone-map rejection occurred around luminance 0.49, where that
floor was inactive, so it did not answer this dark-scene case.

`legacy_tonemap_021` replaces only the 23 native tone-curve instructions with
24 pinned original instructions on selected weather markers. It preserves
native bloom, texture slots, color grading, gamma and vignette. A separate
static-global binding supplies the existing fog marker; no executable renderer
patch is needed. The original body is used exactly, including its luminance
clamp range and exponential conversion. Unselected maps retain every native
tone-curve instruction.

Diagnostic29 improved close09 to 8.214 and all four regression views. Guarded
trial31 gave 8.160 and confirmed shader `f98904e99752908444970c03132df9aa` in
the live trace. Mamayev evening fallback was 13.230 versus the accepted 13.257.
The first guard's metadata returned reflection success but invalid binding
names: trial30 was never rendered. The corrected builder preserves the retail
binding-name pool before the buffer table, and all ten bindings were checked.

All 18 Mamayev rain comparisons improve. Six close views average 8.088 versus
10.268; twelve panoramas average 4.404 versus 4.682. Close09's smoothed error
falls from 11.052 to 5.228. Three ranked contact pages were reviewed. Four
previously validated diagnostic29 frames were reused after byte-exact body
checks and guarded runtime validation; the remaining 14 frames use trial31.
Small surface and particle differences remain. The parity manifest enables
this option only for Mamayev rain (177), pending separate Vostok validation.

The full parity19 build contains 7,013 resources and 299 materials. All 7,009
prior files are unchanged, and four added postprocess programs are byte-identical
to rendered trial31. Every overlay file hash was checked, and all 7,013 resources
were verified in the rendered archive; that same archive is retained as
`parity-19-resources.db` (SHA-256
`baffa45d64d7e9ee189b79281d50c99d2ce65f04014e3bdbb96b7fb92707fa9a`).
There was no need to rebuild an identical archive from stock. All 345 ordinary
tests and eight retail shader tests pass. Tests cover both native blob headers,
the rendered bytecode, original buffer/resource metadata, fallback instructions,
multiple map markers, checksums and malformed inputs. Baseline assets remain
unchanged.

## Vostok rain tone curve and final parity20

The same dark-scene mismatch applies to Vostok rain: adapted luminance in
close03 is 0.02437, below the native curve's 0.05 floor. All four initial
comparisons improved. Diagnostic32 extends the map guard to IDs 168 and 177;
its reflection metadata and all non-code shader chunks are byte-identical to
validated diagnostic31. The live trace confirms shader
`fad93cd504b1bdd080ef786053fee4b1`. Guarded close03 improves from 9.341 to
5.719, and its 16-pixel-smoothed error falls from 7.414 to 2.839.

All 18 Vostok rain views improve. Eight close views average 6.054 versus 8.476;
ten panoramas average 3.855 versus 4.440. Three ranked pages were reviewed.
Three validated diagnostic29 regression frames were reused after exact shader
body and runtime checks; the other 15 use the two-map guard. A final Mamayev
close09 check gives 8.174 versus the accepted single-map guard's 8.160, retaining
the previous improvement. The existing evening comparisons remain applicable:
their native tone branch is preserved, with prior evening fallback validation.

Parity20 enables `legacy_tonemap_021` for both rain maps only. All 7,013 full
bundle file hashes were checked: 7,009 match parity19 and four postprocess
programs exactly match rendered diagnostic32. The complete bundle verifies
against the rendered archive, retained as `parity-20-resources.db` (SHA-256
`4628237bbd954843cc3bdf62fcb1fa5a6c65cbb0da74956e4637ec31f31767c0`).
The retail test now also checks byte-exact two-map output for all four variants.
No additional original screenshots were captured. The capture executable still
has its pre-experiment hash `5eb0fc97b983ecc5d98282444076c7dc8180798429c58c4ac35849cb110c993a`.

Use `ports/survarium-021d-two-maps-weather-parity-to-026g.json` for this result.
The baseline manifest, baseline branch, original reference images and diagnostic
evidence remain preserved. This completes the requested porter conversion and
visual comparison work; deployment into a release is separate.
