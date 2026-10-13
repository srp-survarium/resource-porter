# Matched camera captures

Diagnostic tools for the pinned retail v0.10b and patched v0.26g executables.
They control the view at `base_game_scene::apply_camera` and capture the DXGI
backbuffer before Present. Original game archives are never patched by these
tools. Use local test servers and disposable executable copies.

1. Build `camera_hook.dll` with `bash build.sh` and an x86 MinGW compiler.
2. Run `python3 prepare_client.py --build 010b SOURCE_EXE NEW_EXE` (or `026g`).
   It checks the executable hash, appends a DLL import and startup stub, and
   refuses to overwrite an existing output. Put the DLL beside NEW_EXE.
3. Launch the copied game in its usual `binaries/win32` or `binaries/x86`
   directory with `SRP_CAPTURE_BUILD=010b|026g` and
   `SRP_CAPTURE_CONTROL=Z:\absolute\path\camera.txt` in its Wine environment.
   The startup stub initializes hooks outside the Windows loader lock.
4. Wait for the selected map to load. Confirm it in the game log and view;
   a successful capture alone does not prove that the correct map is running.
5. Use a 1280x720 client viewport in both games. v0.10b initially makes its
   outer window 1280x720, yielding a smaller backbuffer. Compile `viewport.c`
   to an x86 EXE and run it in that client's Wine prefix to correct this.
6. Run `python3 capture.py --control /path/camera.txt --poses poses.json
   --output /path/captures`. Repeat against the other client with the same
   pose file. Each BMP has a JSON record of the requested and applied camera.

The four supplied poses sample Factory's ground floor and upper area; their
names are identifiers, not verified architectural labels. For new 0.26 captures,
set `ui off` in that capture client's console or `Documents/Survarium/user.cfg`
before launch. Reapply it before each launch if the game rewrites the config,
and record `ui_command: "ui off"` in the capture's launch metadata. This native
switch disables Flash overlays (HUD, minimap, crosshair and text); the weapon
and player model remain scene geometry. Use `ui on` to restore the overlay.
Keep existing reference captures; exclude their UI from image comparisons.
Compare static scene regions away from player geometry; moving the view does
not teleport the player. Match weather, texture,
lighting, shading, shadow quality, and grading before attributing differences.
Allow exposure and streamed resources to settle after each camera move.

## Verified hook contract

| Build | Image base | apply_camera RVA | Camera pointer |
| --- | --- | --- | --- |
| 0.10b | 0x10000 | 0x5c72b0 | EDX |
| 0.26g | 0x10000 | 0x22ddd0 | EDX |

Both entries start `83 ec 40 53 8b 5c 24 48`; the hook checks these bytes before
writing. Inverse-view and projection matrices are at director +0x04 and +0x44.
These match the original v0.10b PDB/source and disassembly of both executables.
The shim preserves registers, flags, x87 and SSE state. Projection is D3D
left-handed with a vertical FOV; poses use world-space forward vectors.

The DLL writes `camera.txt.status.json` atomically. The controller changes the
request ID for each pose, waits for capture acknowledgment, and checks position.
The hook selects the first scene camera after each new request. Do not send a
request during loading or a scene transition. Unsupported backbuffer formats
and mismatched camera bytes fail explicitly. The supported capture formats are
single-sample RGBA8/BGRA8, including sRGB variants.

An initial CreateRemoteThread/WriteProcessMemory launcher was abandoned because
this host has `kernel.yama.ptrace_scope=2`. The current normal DLL import method
needs no change to that policy. Preserve the executable's embedded certificate:
both the blob and its PE directory entry are needed by v0.26 startup.

## Local reference runtime

The original v0.10b source registers `-client=HOST:PORT -spectator`, which connects
directly to a match server. It does not register `-localization`. A Russian UTF-8
locale avoids the legacy menu/font initialization failure seen with the default
locale. The local reference server is built from the existing legacy SRP source
with map name `level_03_rain` and port 2248. v0.26 uses map 182 at port 2247.
Restart the one-player 0.26 server with each client restart: its retained roster
can reject the next connection as full.

Runtime evidence is under the main checkout's `.work/render-compare/`.
The preserved artistic preset is separate:
`.work/lighting-presets/2026-10-13-atmosphere/README.md`.

## Intermediate lighting buffers (0.26 only)

Set `SRP_CAPTURE_BUFFERS=1` before launch to capture native-format GPU buffers
alongside each BMP. The hook first captures ambient inputs, then outputs, then
post-process inputs; Present acknowledges the screenshot only after these
captures finish. This stalls the GPU on requested frames only. It does not
change light values. Metadata lists actual target names, formats, dimensions,
write status and skipped resources. `.bin` files contain tightly packed mip-0
rows. `inspect_buffers.py IMAGE.bmp.targets.json` writes fixed-exposure previews
and statistics; two-channel previews display their first channel as grayscale.
Raw HDR values are not directly comparable between different engine contracts.

Verified against 0.21 symbols and the pinned 0.26 disassembly:

| Capture point | 0.26 VA | Stage pointer |
| --- | --- | --- |
| `apply_ssao_and_height_based_ambient` entry | 0x182530 | First stack argument |
| Immediately after that call in ambient execute | 0x1828bf | ESI |
| `stage_postprocess::execute` entry | 0x1b2270 | ECX |

Stage +4 points to context; context +0x119c points to the target array. Instances
have stride 0xa0, inline name +0x0c, and render-target pointer +0x98. The target's
D3D11 texture is at +0x0c. These offsets differ from 0.21 and must not be inferred
from its headers alone. Both ambient captures retain slots 31 (diffuse), 32
(ambient mask), and 34 (specular). Post-process capture includes the other named
lighting, normal, albedo, atmosphere and luminance buffers.

`python3 measure_ambient.py CAPTURE_FOLDER` reports per-pixel after/before ratios
for diffuse RGB sums above 0.0001. It measures attenuation within 0.26; it is not
a similarity score against 0.10b. Existing reference images in
`010b/poses-quality1` are frozen with `010b/reference-manifest.json`; further
iterations use only 0.26. A rendered map during countdown is sufficient.

## Active shader inputs and optional atmosphere experiments

Buffer capture also records the first recognized probe, cloud, sky, geometry-fog
and post-process draw after a pose settles. `*.{probe,cloud,sky,fog,post}-draw.*`
contains the actual DXBC, bound constant buffers, texture metadata and blend
state. Some bound slots are leftovers unused by that shader; consult the DXBC
reflection. Texture names include the view's DXGI format, which can differ from
the underlying typeless resource. Compressed textures and cubes are described
but not read back. Interior views may omit sky/cloud draws entirely.

`build_shaders.py --wine WINE --fxc FXC --output DIRECTORY` builds the diagnostic
shaders with `/WX /O3 /T ps_4_0`, recording hashes. Wine's Z drive must expose the
host root. In addition to `SRP_CAPTURE_BUFFERS=1`, set any of these variables to
the corresponding compiled shader's Windows path:

| Variable | Shader |
| --- | --- |
| `SRP_LEGACY_ATMOSPHERE_LUT` | `legacy-atmosphere-lut.dxbc` |
| `SRP_LEGACY_GEOMETRY_FOG` | `legacy-geometry-fog.dxbc` |
| `SRP_LEGACY_SKY` | `legacy-sky.dxbc` |
| `SRP_LEGACY_RAIN_CLOUDS` | `legacy-rain-clouds.dxbc` |
| `SRP_LEGACY_RAIN_TONEMAP` | `legacy-rain-tonemap.dxbc` |
| `SRP_LEGACY_PROBE_DIFFUSE` | `legacy-probe-diffuse.dxbc` |
| `SRP_LEGACY_PROBE_DIFFUSE_BOX` | `legacy-probe-diffuse-box.dxbc` |
| `SRP_LEGACY_COMPOSITION` | `legacy-composition-control.dxbc` or `legacy-composition-encoded.dxbc` |
| `SRP_LEGACY_AMBIENT_DECODE` | `legacy-ambient-decode.dxbc` (paired experiment below) |

These change rendering only when explicitly enabled. Replacement requires the
original shader's exact DXBC size and header digest. The hook records successful
creation counts in `camera.txt.shader-overrides.json`; inspect it every run.
Geometry fog adds the previously unbound Mie table at t2 for its draw and restores
the prior binding afterward. Its single alpha-blended draw reproduces the old
0.1 distance attenuation plus additive scattering. The observed native blend is
SRC_ALPHA / INV_SRC_ALPHA, ADD. Sky/cloud register layouts are adapted separately.

The cloud experiment is specific to Factory rain: its legacy tint multiplier 10,
UV scale 1, fade power 2 and fade band 0.5 are explicit shader constants because
the later stage does not upload those old fields. It is not a general map fix.
An early diagnostic also disabled height-based ambient and multiplied diffuse
probe intensity by 10, approximating the legacy 2.5 SSAO gain versus the later
0.25 probe decode. Probe exponent, material, exposure and grading differences
remain. Four-pose validation restores distant haze but shows overbright interior
surfaces and differing light patterns; this is not visual parity or a release.

The optional rain tone mapper isolates the old exposure and filmic curve while
retaining the target's bloom inputs, grading conversion and vignette. It reduces
some interior overbrightness but does not reproduce the full old post pipeline.
Its parameters are specific to Factory rain.

The subsequent sun-only comparison uses no shader overrides. The importer had
the wrong azimuth convention: target to-sun is
`(-cos(az)*cos(el), sin(el), -sin(az)*cos(el))`. Correcting Factory rain's azimuth
from 50.317974 to 219.682029 degrees reproduces the source direction in the
captured shader constants within 1e-5. The direct-light placement improves, but
unlit surfaces remain too dark. This is a confirmed conversion fix, separate
from the experimental renderer changes.

Ambient capture also recognizes the height-ambient draw (`hba-draw`) and the
skylight shader when drawn (`skylight-draw`). The probe diffuse experiment reads
mip 1 of the converted 16x16 cube (the source 8x8 mip) and decodes RGB/alpha with
power 2.2. It retains target specular and falloff; it is not a complete old probe
implementation. In particular, it cannot fix an inverted falloff interval.

Captured Factory rain constants revealed that the radius-300 probe had inner
radius 299.01 but outer radius 294.06, making its interior blend weight zero.
The target contracts the outer sphere by 2%; the inherited width of one metre
was too small. The importer now uses `1 + 0.02 * radius` for spherical probes.
Native-shader captures with that correction restore wall detail without changing
exposure. The old exponential falloff and encoded-light blending remain distinct
from the target's smoothstep/linear-light contract. See the main investigation
document and `reproduction/probe-width-checkpoint` for controlled comparisons.

Probe capture records the first draw of each distinct active probe shader, up
to 16 variants per pose. The first keeps `probe-draw`; subsequent variants use
`probe-draw-1`, etc. Factory rain uses two observed variants: sphere (6104 bytes)
and box (6116 bytes). Earlier sphere-only overrides left local box probes native.
`composition-draw` records the actual scene-composition shader and inputs; it
contains inline sun lighting as well as material response and rain darkening.

For a separate falloff experiment, set the two probe override variables to
`legacy-probe-source.dxbc` and `legacy-probe-source-box.dxbc`. They use the old
sphere/box falloff formulas, diffuse cap and Fresnel expression with the target
G-buffer parameters. Specular and linear accumulator blending remain native;
this is not complete old-pipeline equivalence. Include-file hashes are recorded
by `build_shaders.py` so the shared implementation is part of the build manifest.

The all-probe comparisons improve some regions and worsen others. Disabling
height ambient and replacing probe decode/falloff/tone mapping has not produced
a close four-view match. Captures and run recipes are preserved under
`reproduction/probe-variants-checkpoint`; none of these diagnostic combinations
is a release preset.

## Same-frame composition comparison

The composition override matches only the captured 8416-byte shader with digest
`de3e7a631834e15db4ae347359867c30`. It retains the native shader as a reference.
On a settled capture, the hook reads the override output, repeats the draw with
the native shader and identical inputs, reads that output, and redraws the
override before continuing. It refuses a blending pass. The two RT0 readbacks
are `composition-output.rt00.bin` (override) and `.rt01.bin` (native).

```sh
python3 compare_composition.py CAPTURE_FOLDER
```

The control reconstructs native composition. In `pipeline-composition-validated`,
three views match bit for bit; the upper view differs by one stored-value step
in one pixel. The encoded variant changes only the combination of ambient, sun
and scattering diffuse light: encode each with `sqrt(saturate(light/8))`, apply
sun visibility to its encoded value, add, then decode with `8*saturate(sum)^2`.
Materials, rain darkening, specular and the supplied ambient buffer remain fixed.
This restores the south-wall sun patch in `pipeline-composition-encoded`.

The source basis is `sun.ps`, `resolve_lighting.ps` and `effect_sun.cpp` in the
0.10b tree. Re-encoding a combined target ambient buffer does not reproduce
source blending between individual probes/local lights. The interior, north
view and distant forest still differ substantially from the frozen reference.
This is a controlled contract experiment, not complete renderer compatibility.

## Encoded probe blending

For the Factory rain experiment, enable all three together:

| Variable | Shader |
| --- | --- |
| `SRP_LEGACY_PROBE_DIFFUSE` | `legacy-probe-encoded.dxbc` |
| `SRP_LEGACY_PROBE_DIFFUSE_BOX` | `legacy-probe-encoded-box.dxbc` |
| `SRP_LEGACY_AMBIENT_DECODE` | `legacy-ambient-decode.dxbc` |

The hook rejects incomplete pairs by checking the probes' reflected
`encoded_color_parameters` marker. Use the project with height ambient disabled;
the decoder matches only the verified SSAO-only shader (660 bytes, digest
`e852c81d2cf9c9554a6046c531390ec8`). Each probe writes `sqrt(saturate(diffuse/8))`
before native alpha blending. At the SSAO stage, a GPU copy of the accumulator
is bound at t2 and decoded to `8*E*E*AO` with blending disabled. The hook restores
the original resource and blend state before later native linear-light passes.
Specular remains native.

`probe-initial` captures prove that the destination starts black in all four
reference views. This experiment does not implement a nonzero legacy skylight
underlay, the old SSAO algorithm or encoded addition of individual local lights.

```sh
python3 verify_ambient_decode.py CAPTURE_FOLDER
```

The verifier checks the initial buffer, exact GPU copy, overwrite blend state,
decode equation to one stored-value step, finite outputs and unchanged specular.
All four `pipeline-encoded-probes-sphere8` captures pass. The sun patch remains,
but the interior, north wall and distant forest still fail visual parity.

The final sphere falloff is `exp(-8*(d/(.95*r))^8)`. The earlier source adaptation
incorrectly used power 6. The original `environment_probe_lighting.ps` and DXBC
digest `d19929272cd49b9db3063352d97bed8c` confirm three squarings before the
exponential and two afterward, giving power 8 and coefficient -8. Historical
captures retain their original shaders; the correction is not retroactive.

## Material and texture-view diagnostics

`adapt_roughness_channels.py` builds an experimental v0.10b texture overlay for
original-red versus native-green roughness sampling. It clones selected textures
as BGRA8, copies decoded red into RGB at every authored mip, and changes only the
enabled roughness bindings. It preserves alpha and the source DDS/options sRGB
flags: captured native roughness views can be sRGB. Forcing linear sampling made
the first experiment worse. The revised overlay passed archive verification and
four UI-hidden rain captures, with the cloned textures bound through sRGB views.
Image metrics improve slightly overall but material correspondence remains
incomplete; it is not part of the accepted weather recipes.

`SRP_CAPTURE_MATERIALS=1` records the first draw of each distinct deferred-material
pixel shader, up to 128 variants per pose: bytecode, constants, blend state and
texture metadata. It does not enumerate every material instance or copy material
textures. Resource metadata for lighting inputs includes the resource mip count,
SRV format and visible mip range, so explicit shader LODs can be interpreted.

Buffer capture is armed at Present after the settling interval. This prevents
the interval expiring halfway through a frame and capturing lighting without
its preceding geometry. The four `pipeline-resource-views` poses recorded
26/18/22/29 material variants (interior/north/south/upper). Both probe shader
variants, the ambient decoder and composition match the requested binaries;
all four ambient buffer checks pass. Neither probe diffuse SRV hides a leading
mip: both expose the complete 16x16, three-level cube, making LOD 1 an 8x8 mip.

Visual parity still fails. These diagnostics establish intermediate behavior;
they do not validate the combined old/new rendering pipeline.

## Isolated composition rain response

`SRP_LEGACY_COMPOSITION_REFERENCE` optionally supplies a second composition
shader. On each requested capture it is drawn with the same inputs as the
override and native reference, then the override is restored. Its output is
`composition-output.rt02.bin`; its bytecode is `.comparison.dxbc`. A composition
override is required. Compare it with `compare_composition.py CAPTURE_FOLDER
--reference-slot 2`.

`legacy_composition_dry_response.hlsl` removes only the diffuse rain multiplier
from encoded composition. `pipeline-composition-rain-isolated` pairs it with
the unchanged encoded shader and retains the project's rain flag. All four
captures have finite outputs and no channels decreasing by more than one stored
step. `audit_rain_response.py` measures the same-frame RGB-sum increase: about
17% in the two south-wall regions, 17% in the interior back-wall region, 27% in
the north region and 58% in the forest region. This is intermediate HDR energy,
not final perceived brightness. All four views still differ from 0.10b.

`audit_surface_light.py` bounds the missing skylight underneath the global
probe using captured depth/transforms and source falloff. The bound is conditional
on sphere coverage and subsequent ordinary alpha blending. It is below 0.0004%
of ambient energy in both south-wall regions; it cannot explain that wall's
visible gap. Regions crossing outside the sphere have no aggregate bound.

Correction: the earlier SSAO source audit read the inactive `SSAO_METHOD==0`
branch. The file defines `SSAO_METHOD 1`; that active depth-based algorithm
agrees with the archived shader, including nonlinear scaling and the 0.5 cap.
The frozen profile has post-process quality 3, selecting the 32-sample variant.
Neither the active source nor the bytecode justifies a uniform 2.5 gain.

## Reference shader quality and offline SSAO

The reference profile's console settings are 1 for lighting/shading/shadows and
3 for post-processing. Retail `render_cc_u32::fill_macro` maps any nonzero value
other than 3 to shader macro 3. This is verified at reference-executable VA
0x12b7a0 (RVA 1161120, 350 bytes); the four macro-name strings and branch to
`push 3` were checked directly. Earlier single-tap probe experiments therefore
used the wrong source quality branch.

`legacy_probe_quality3.hlsl` and its box wrapper retain the encoded accumulation
contract and implement the archived quality-3 diffuse response: eight rotated
8x8 samples divided by nine, plus its halfway-vector Fresnel. Native specular
remains. The original sphere DXBC digest is `cc45055d632b75e94160d8a1595b5393`.
All four `pipeline-probe-quality3` captures use the requested sphere/box shaders
and pass the ambient-decoder check. The wall response is visibly closer, but
the four-view comparison still does not establish parity.

`replay_ssao.c` runs the unchanged archived SSAO and spatial-upsample bytecode
against captured target depth, without a client or window. It uses the original
4x4 noise texture, sampler modes, full-size screen constants, half-size RG16F
accumulation and full-size RG8 output. Factory-rain parameters are saturation 2
and radius scale 1.5. Build it with `build.sh`; `build_shaders.py` also produces
the fullscreen vertex shader. Supply the executable and matching DXVK DLLs in
the runtime directory:

```sh
python3 replay_ssao.py CAPTURE_FOLDER ORIGINAL_SHADER_ASSETS OUTPUT_FOLDER \
  --runtime REPLAY_RUNTIME --wine WINE --prefix DIAGNOSTIC_PREFIX
```

The Python driver checks a flat-depth analytic result, finite/ranged outputs,
and the depth channel's point-sampling footprint at every half-size pixel.
Those sample centers lie on full-size texel boundaries; interpolation rounding
may select any of the four adjacent texels. The first strict center-index check
failed on that assumption, and was corrected after verifying every sample lies
within that footprint. The flat case gives 0.386962890625 before upsampling and 99/255 after it;
the lighting factor after the original 2.5 scale is 0.9705882353.

On saved `pipeline-probe-quality3` depth, spatially filtered source AO means are
about 2–3% lower on the south wall, 13% higher at the interior back wall, 17%
higher in the north region and 60% higher in the forest region than captured
native AO. These compare algorithms on the same target geometry. They do not
reproduce old geometry or temporal history and have not been applied live.
The frozen profile disables light propagation volumes, so its quality-3 LPV
resolve branch receives the disabled stage's cleared buffer.

## Live archived SSAO experiment

Pair `SRP_LEGACY_AMBIENT_DECODE=.../legacy-ambient-decode-ssao.dxbc` with
`SRP_LEGACY_SSAO_DIR=.../ASSET_DIRECTORY`. That directory must contain
`fullscreen-vs.dxbc`, `old-ssao-3.dxbc`, `old-ssao-upsample.dxbc`, and the original
64-byte `old-ssao-noise.bgra`. The encoded sphere/box pair remains required.
The hook rejects a decoder/directory mismatch. This is the fixed 1280x720
Factory-rain experiment: radius 1.5, saturation 2, 32 samples, spatial filtering
only. It does not implement the old temporal filter or replace specular AO.

The hook allocates GPU resources on the scene depth texture's device, after
startup. The temporary device used to discover hook vtables is unsuitable.
It runs before ambient decode, in a separate D3D11.1 context state, and restores
native state before returning. Original raw RG8 AO replaces t0 only for the
decoder draw; the shader applies the old 2.5 normalization there. Values above
one remain representable. The native t0 binding is restored afterward.
The state isolation uses [SwapDeviceContextState](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11devicecontext1-swapdevicecontextstate).

Each requested frame saves `.ssao-live.rt00.bin` (exact R16F depth), `rt01.bin`
(half-size RG16F accumulation), and `rt02.bin` (full-size RG8 spatial result),
plus `.ssao-native.rt00.bin` (native AO before substitution). Run the existing
`replay_ssao.py` command with `--live` to require byte-identical half/full outputs
from an independent replay. `verify_ambient_decode.py` recognizes the paired
shader and verifies `8 * encoded^2 * raw_ao * 2.5`, exact AO binding, unchanged
specular and the overwrite blend. The original native-AO verifier remains valid.

All four views pass both checks. Final-image improvement is limited: the forest
region gains about 56% diffuse ambient energy but only 5% in display-encoded
luma; the interior remains substantially darker than the frozen reference.
The south wall becomes slightly darker. These measurements do not establish
visual parity, temporal quality, performance suitability, or other-map support.
The replayable checkpoint is `reproduction/live-ssao-checkpoint/` in the main
checkout's `.work/render-compare/` directory.

## Original rain grading and output conversion

The connected post-tail experiment requires all three Windows paths:

| Variable | Input |
| --- | --- |
| `SRP_LEGACY_RAIN_TONEMAP` | `legacy-rain-post-tail.dxbc` |
| `SRP_LEGACY_PRESENT_GAMMA` | `legacy-present-gamma.dxbc` |
| `SRP_LEGACY_POST_GRAIN` | Original `engine/noise_64x64.dds` BC1 mip-zero payload (2048 bytes, no DDS header) |

The loader rejects an incomplete set. Grain is allocated on the scene device
and bound at t12 only for the post draw. The source tail restores dark-range
desaturation (limit .03), positive grain (rain-material scale .08), 2.2-power
conversions around grading, and the final 2.2-power copy. These constants come
from the archived rain material; the noise phase follows current frame offsets.
The frozen original frame's phase is unknown. The matched LUT is unchanged:
all 16x16x16 BGRA texels equal the original `color_grading_rdn` asset.

Capture includes `.post-draw.vol01.bin`, sampler metadata, `.post-output.*`,
`.present-draw.*`, and `.present-output.*`. Volume rows are tightly packed with
Z slices outermost; `inspect_buffers.py` renders a vertical slice atlas.
`verify_present_gamma.py CAPTURE_FOLDER` checks the final copy before subsequent
AA/HUD passes, including exact post-to-copy input, RTV format, blend and gamma.
Every pixel in all four final views passes within one UNORM8 step. Ambient
verification also still passes; actual paired shaders and grain binding are
checked separately against the saved recipe.

`audit_post_chain.py BASELINE_CAPTURE --noise ORIGINAL_NOISE_DDS --output JSON`
is an experimental CPU reconstruction of the preceding hybrid. Its strict
one-R11G11B10-step check currently fails on some channels. It records that
failure and exits nonzero; its counterfactual means are estimates, not validation
of the full post shader. Texture filtering has finite precision, but the exact
remaining discrepancy has not been isolated. Do not relax its threshold merely
to make it pass.

Final regional brightness is closer at all five sampled rectangles; this does
not establish visual parity. Native bloom/DOF producers, temporal filtering,
material/specular behavior and distant atmosphere remain incomplete. The saved
recipe is `reproduction/post-tail-checkpoint/`; the runtime capture folder is
`026g/pipeline-original-post-tail-verified`. The beautiful preset and original
reference captures remain separate.

## Original-client intermediate reference and fog sampler

`SRP_CAPTURE_BUFFERS=1` also supports the pinned 0.10b client. It identifies
18 archived bytecodes across ten pass families using `reference_shader_index.h`.
`reference_shader_index.py --database DB --index ENTRIES_JSON --output HEADER`
regenerates that table. The original branch observes D3D draws and ignores all
shader overrides; it does not install the 0.26 target-layout hooks.

For each requested frame it saves the first recognized draw of each family,
including shader, constants, samplers, inputs and two render targets before and
after the draw. This is a bounded pass capture, not a complete GPU frame trace.
The original uses the temporal SSAO upsample, so the spatial-only family is
absent. Screenshot acknowledgment follows the final copy after post-processing.
The four original captures are preserved in `010b/poses-buffers`; reuse them.
The earlier frozen screenshot reference remains unchanged.

The captured original fog uses linear-clamp sampling; native 0.26 s0 wraps.
The geometry-fog adapter now binds a clamp sampler only during its draw and
restores the native sampler afterward. Mie coordinates near the first texel
previously blended with the opposite texture edge. No shader arithmetic or
brightness multiplier changed. Both clients' fog distance is 300; the adapter's
`globals[16].y` is an atmosphere parameter, not the camera far plane.

`audit_fog_capture.py ORIGINAL_FOLDER CURRENT_FOLDER` checks actual bindings,
compares scattering tables and reports regional before/after linear RGB means.
The current Rayleigh table remains about seven times the original; its input
mapping is unresolved. Mie tables agree to half-float rounding. The sampled
forest deficit also exists before fog. Between original composition and fog,
the saved interior difference contains the missing light shafts; the same target
interval is unchanged. The source stage order includes sky and forward draws in
that interval. This localizes the difference without identifying the missing draw.

`pipeline-fog-clamp` improves forest displayed brightness from about .1811 to
.1971 against .2519 in the frozen reference. North-wall brightness also improves;
south-wall/interior means move slightly farther away. These are separate-frame
regional statistics, not a perceptual similarity score. Gamma checks pass in all
four original and corrected target views; target ambient checks also pass. C
builds with warnings as errors. No game tests were added. Close visual parity
remains incomplete. Artifacts are in `reproduction/reference-buffers-checkpoint`.

## Numeric material conversion and light cones

`SRP_LEGACY_VOLUME_CONE` selects the experimental `legacy_volume.hlsl` shader for
the verified native cone PS identity. Pair it with `SRP_LEGACY_VOLUME_ASSETS`, a
directory containing original `old-spot_falloff.dds` and `old-sphere_falloff.dds`.
The adapter restores original cone/sphere opacity and sRGB falloff sampling while
preserving native geometry, blend state and dust animation. It currently pins
Factory's authored attenuation scale 13; it is not a general material adapter.
All 40 lamp/cone placements and both audited meshes match the source. Cone UV
quantization is below .00044, so no placement shift was applied. Four rain views
completed with the replacement shader. Above-lamp luma error falls .03544→.01094;
the beam's regional error falls .04780→.03743, but its average brightness remains
above the original. First-capture metadata omitted sphere slot 14; the observer
now records slots 0–15 and material mip ranges for subsequent validation.

The converter now copies integer visual values into float template fields while
preserving shader feature switches. An audit of 247 converted materials found
58 differing fields across 41 materials. Fresh conversion verifies all 58 fixes.
The existing regression test was extended; all 270 library tests pass, with no
increase in test count.

The cone's opacity was .05 instead of the original 1. Actual PS bindings confirm
both values before/after correction. `volume-draw`, `volume-before` and
`volume-output` capture the first matching cone draw per view. The native shader's
attenuation field is unused; its captured bytes are not a valid attenuation
measurement. Its alpha equation also differs from the original source.

The lamp rectangle's displayed luma changes .15342 -> .20930 (original .21605).
Applying all 58 numeric corrections yields .20914 there and .19707 in the forest
(original .25186). These separate-frame regional means do not establish visual
parity. Gamma and ambient checks pass in all four `pipeline-numeric-materials`
views. The outdoor deficit remains.

`patch_archive_entries.py ARCHIVE PLAN inspect|apply|revert` checks saved material
hashes, lengths, archive offsets and CRCs before applying a same-length experiment.
Stop the client first. `reproduction/numeric-material-checkpoint` contains the
replay script and baseline/fixed material bytes. Revert that plan before replaying
an earlier checkpoint that predates material corrections.

`compare_surface_albedo.py ORIGINAL CURRENT` compares decoded surface color,
composition output and AO red-channel means in the fixed regions. Forest albedo
means agree within .22% and AO means within .18%, while current pre-fog RGB is
36–39% lower. These statistics localize the remaining deficit to the lighting
calculation without identifying its precise producer. Original R10G10B10A2
normal/light inputs were skipped by the capture format handler, so their raw
data is unavailable in the saved reference; do not infer their contents.

## Probe smoothness and foliage base-light experiment

Both G-buffer producers store smoothness. The verified 0.26 constant-material
shader writes `(1 - roughness) * smoothness_multiplier` to `t_parameters.y`;
the textured shader applies the same complement after sampling texture green.
The original samples texture red and packs smoothness into seven bits.
Commit 8bb781c corrects the mistaken extra inversion introduced by a7d0612:
source probe gloss now uses `parameters.y` directly. Across 64 day/evening views,
the worker measured mean per-view smoothness error about .022, versus .886 for
the inverted field. Texture channels and packing remain separate differences.

`legacy-composition-translucency-base.dxbc` is a controlled partial experiment.
It replaces the native base scattering term `light_color * translucency * .02`
with original `translucency * .1`, verified against the archived `translucency.ps`.
Directional terms and shadow attenuation remain native. The original and native
shadow attenuation equations differ, so this is not a complete source-scattering
implementation. This material contribution is not a global exposure adjustment.

Set `SRP_LEGACY_COMPOSITION_REFERENCE` to the previous dry-response composition
for same-frame comparisons. `audit_translucency_base.py CAPTURE --shader DXBC
--comparison DXBC` validates identities, finite outputs and unchanged opaque
pixels. All four `pipeline-translucency-base` views pass: zero opaque pixels
change. Gamma and ambient checks pass too. Forest pre-fog RGB changes from
[.01495,.01354,.01033] to [.02345,.02143,.01650], original [.02392,.02208,.01746].
This local improvement does not prove whole-image or map-wide parity. Broader
day/evening coverage and held-out views are being captured independently.

## Native 0.26 UI suppression

The pinned executable's `renderer::render_flash_ui` at VA `0x13ced0` checks
byte `0xa71bc5` before submitting Flash movies. The `cc_bool` registration at
`0x84edb0` binds that byte to command `ui` (name at `0x9eefdc`). `ui off` in
the reserved client's user config was verified in four captures under
`.work/render-compare/026g/pipeline-ui-hidden/`; no shader changes were needed.
The earlier `pipeline-translucency-base` captures retain UI. The original
reference client was not relaunched for this change.

## Observed sun shadows and original translucency

The observer now saves the first native sun-shadow draw's shader, bindings,
inputs and before/output targets under `sun-shadow-*`. It captures R16 typeless
shadow textures with their explicit R16 UNORM SRV format; the preview decoder
uses that view format. Four UI-free rain views selected the same 4324-byte shader
(SHA256 b2f936a24eadb595e83cede97db93f796431e8d30751b785843d07d82ddf1341).
Its green output is `1 - pow(saturate((depth_delta - .001) / .0015), .11)`.
This capture samples the first draw, not every cascade.

The original behavior is different from the earlier raw-depth assumption:
original sun MRT2 stores `(filtered_depth, shadow_visibility)`, but translucency
reads **green**, so its attenuation subtracts visibility from projected depth.
Both captured bytecodes confirm this. `audit_original_translucency.py CAPTURE
--stride 97` replays both channel hypotheses against saved before/output buffers.
Across 32 day and 32 evening views, all 86,334 sampled translucent pixels match
within one 10-bit output step using green. Raw red does not. A denser four-view
sample retains two upper-view outliers with unchanged outputs; stencil rejection
is a possible explanation, not verified because stencil was not captured.

The corrected probe plus existing base-light experiment is captured in
`pipeline-probe-smoothness`. Gamma and ambient checks pass for all four views;
forest pre-fog RGB is [.023932,.021851,.016833] versus original
[.023915,.022080,.017464]. These regional means do not establish image parity.
Full source translucency, sun BRDF, fog and other remaining differences still
need work. No game tests were added.

### Source translucency experiment

`legacy-composition-translucency-source.dxbc` uses original five-bit material
packing, signed backlight, forward scattering and a separately supplied original
sun color. Pair it with `SRP_LEGACY_SUN_PROJECTION` and
`SRP_LEGACY_TRANSLUCENCY_CONSTANTS` (exactly four float32 values; RGB is authored
sun color cubed times intensity). The hook replays each verified native shadow
cascade into an R32_FLOAT projection target without writing native depth/stencil,
then binds that same-frame result at composition t13 and the color at b2.
The 26 accepted native shader identities are generated by
`sun_projection_index.py` from captured archive bytecodes and checked bindings.

This remains a hybrid: native cascade selection, projection, visibility and
sun BRDF are retained. Original translucency subtracts helper GREEN (visibility),
not its RED raw depth. The replacement uses native visibility for that term.
Do not interpret this as a complete original shadow/translucency pipeline.

`audit_sun_projection.py CAPTURE --stride 1` replays each captured cascade's
projection at changed pixels and verifies that the final helper reaches t13
byte-for-byte. Unchanged pixels cannot identify a cascade writer. The four-view
rain experiment checked 3,611,122 changed values with maximum absolute error
7.97e-8. The paired composition check found zero changed opaque pixels and no
nonfinite output, but the full composition formula has not yet been independently
replayed. The forest region gets darker than the partial base-term experiment;
a supported shader equation does not establish overall visual improvement.

Run diagnostic clients sequentially. After concurrent runs stalled, both the
rain experiment and the worker's broad day run completed with one client at a
time. This is an observed workaround, not a proven diagnosis of the stall.

## Optional god-ray parity

`SRP_LEGACY_DISABLE_GOD_RAYS=1` with buffer capture skips the exact archived
0.26 god-ray apply shader only with its verified additive blend. Use it only
for an original reference captured with `r_use_god_rays off`. The native effect
otherwise remains enabled. Captures include the apply/mask/ray shader inputs,
before/output buffers and compatibility metadata. Run
`python3 audit_native_god_rays.py CAPTURE_DIRECTORY` on an enabled diagnostic
batch to replay its bilinear additive pass. Disabled batches must instead have
identical apply before/output bytes, since their nonzero input is deliberately
not added.

`audit_temporal_noise.py FRAME.bmp ... --region far:X0,Y0,X1,Y1` measures
stationary temporal variation on a fixed pixel grid. It rejects sequences with
changed camera metadata and reports both raw temporal RMS and RMS after removing
each frame's regional mean. Specify near, distant and sky regions separately;
region names do not infer depth. Rain, foliage and exposure can also contribute,
so use matched fixed-camera sequences to validate a suspected noise correction.

## Private background capture

`SRP_CAPTURE_BACKGROUND_RENDER=1` opts the private 0.26 observer into bypassing
the verified inactive-window render gate. The default is off. It checks the
image base, native gate/getter instructions, engine vtable and writable data
page, then saves the original gate byte before clearing it. Activity flags and
render settings remain native. This does not support 0.21.

Write `0` to `CONTROL.background-render` to restore the saved gate, or `1` to
resume the opt-in override. The observer also restores on DLL detach. Its
`CONTROL.background-render.json` records application/restoration and write
count; frame-state diagnostics expose activity flags independently. Use this
only in isolated capture clients; initial compiler validation is not runtime
proof of recovery or permission to run uncoordinated concurrent clients.

`SRP_CAPTURE_EMISSIVES=1` adds read-only capture of native emissive draws,
recognized by 40 emitting shader identities from the native archive. It records up to 64 draws per
requested frame with separate shader, material inputs and before/output targets
(`emissive-draw-N`, `emissive-N-before`, `emissive-N-output`). The cap is explicit;
64 records may mean the capture is incomplete. This option changes no shaders.
Depth-only draws are excluded because they can share an archived emissive
shader identity. The first Factory diagnostic exhausted the cap on those draws;
its empty target records do not establish the lamp's emission contract.
The generator also excludes 23 permutations with emission disabled, checking
their feature key against the reflected emission constant. The corrected
interior capture records four emitting draws with color targets.

## Factory manual-play checkpoint, 2026-10-13

`reproduction/playable-factory-20261013` preserves the user-tested map182
variant with normal character controls and HUD. Its 70 hashed files include
the actual client DLL, checkpoint-local shader inputs, tree-wind patch, launch
recipe and four user screenshots. `reproduce.py --verify` checks those files
without launching; replay requires the existing prepared archive, client,
private prefix and server binary. It refuses to replace an active client or
server. The archive was not duplicated, and saving did not alter the live game.
Verification and CLI parsing passed; the new replay wrapper has not been
launched during the user's session.

The screenshots at 13:57:50 and 13:58:05/15/17 report green brick edges,
apparently low streetlamp glow and changing roof details. These remain open:

- Original and converted streetlamp geometry have identical 997 positions and
  indices. The audited cone and pole placements also match. Moving the authored
  geometry is not justified by this evidence; cone shading needs a matched view.
- The emitting interior draw retains color `[5,5,5,1]`, additive blending and
  a linear BC1 texture view. Its HDR output changes 99 pixels, 97 within the
  bulb region. This establishes emission, not parity of the later bloom/blur.
- Native-only vertex-blend settings are a candidate for the green brick edges.
  No matched draw has yet established the cause or validated a correction.
- Candidate building LOD memberships and thresholds match the original. Native
  default thresholds also match, and current shading quality disables shader
  LOD selection. The changing roof surface/pass still needs identification.
- Factory010 rain draws textured cones around the camera
  (`stage_rain.cpp:140–167` in the original source). School021 uses particle
  quads. School's independently prepared rain-shadow projection correction is
  not a replacement for Factory's original rain path.

The earlier four-view fidelity measurements predate the cloud input fix in
this playable variant; they do not validate its complete current appearance.
