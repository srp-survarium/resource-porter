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
parity adjustments without changing the preserved baseline manifest. So far it
changes only Mamayev evening's `mie_fog_base_height` from the inherited 75 to 0.
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
`math::lerp<environment_properties>` (RVA `0x1abd99`) zero-initializes its output
and skips the `mie_fog_height_falloff` member at offset `0x15c`.
`renderer_context::set_static_frame_parameters` (RVA `0xef8e2`) then uploads
`1 / height_falloff`. The loader and copy operations do include this member.
At the current positive-height camera, the original draw contributes no Mie
fog. Treating the authored value as the observed renderer value would be wrong.

The accepted four-map manifest remains iteration 1. Shader compatibility is
being checked separately before enabling it there. Other maps, weather and
camera elevations still need verification.

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
