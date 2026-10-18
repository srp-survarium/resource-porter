# RLS Vostok and Mamayev weather parity

Work in progress: the baseline converts and loads, but is not visually matched.

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
Evening and rain are in scope; night is not.

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
iteration uses 0.26 captures only. Extra original diagnostic runs must not
become routine recaptures of the reference set.

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
