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
names are identifiers, not verified architectural labels. Frame capture retains
the game's HUD and player model. Compare static scene regions away from those
features; moving the view does not teleport the player. Match weather, texture,
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
