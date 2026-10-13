# Encoded diffuse lighting for original Factory rain

Original `rain.ps` samples `t_diffuse_lighting` directly. Its alpha uses
`min(value * length(encoded_lighting) * 10 + value, 1)`. This resource is the
R10G10B10A2 diffuse accumulator, not linear lighting or albedo-lit scene color.
Original `resolve_lighting.ps:68–69` decodes it as `saturate(E)^2 * 8`.

The native compatibility path currently decodes the probe accumulator before
native local lights. Composition then re-encodes their combined linear sum.
That cannot recover separate source contributions: two unshadowed unit lights
produce encoded sum `2 * sqrt(1/8) = .707107`, whereas encoding their linear sum
gives `sqrt(2/8) = .5`. Per-draw 10-bit clamping and rounding are also lost.

## Producer contracts

All source shader paths below are under the pinned v0.10b
`vostok/resources/sources/shaders/sm_4_0`; effect paths are under
`vostok/sources/vostok/render/engine/sources`.

| Producer | Source contract | Required auxiliary behavior |
| --- | --- | --- |
| Skylight | `skylight.ps:64–76`: apply AO and height ramp before `sqrt(saturate(L/8))` | Initialize the encoded target using the source sky equation. |
| Probe | `environment_probe_lighting.ps:154–158,229–232`: AO before the `min(L,10)` and Fresnel; encode diffuse; alpha carries falloff | Replay source probe order and SRC_ALPHA/INV_SRC_ALPHA blend (`effect_environment_probe_lighting.h:33,49`). |
| Point/spot | `point_light_accumulator.ps:19–33,85–91,210`: source BRDF, attenuation and shadow factor are applied before encoding | Replay each light separately with ONE/ONE into the auxiliary target. Do not encode a sum of lights. |
| Sun | `sun.ps:303–314`: encode unshadowed diffuse first, then multiply by shadow plus transparency | Add the source sun contribution separately; moving shadow inside the square root changes its falloff. |
| Translucency | `translucency.ps:74–83`: encode its separate scattering result | Add this source contribution after the original stencil test. |

The existing native point shader is not the same BRDF with a different output
encoding. Archived identity `64f2a63c0797cd222f2ff2c52b5a52da` (5,320 bytes)
uses N·L for diffuse Fresnel; source point lighting uses V·H and the >.99
Fresnel fallback. Native output is linear, multiplied by the bound color and
intensity. Source uses its linearized authored color and diffuse influence.
Other shadowed and area-light permutations need their actual bindings checked.
The archive has 19 identities across five native local-light families; an
archive permutation alone does not prove that Factory draws it.

## Isolated implementation plan

1. Capture the active local-light draws with `SRP_CAPTURE_LOCAL_LIGHTS=1` in
   the already planned lamp/hangar batch. Preserve each light's geometry,
   PS/VS bindings, depth/stencil reference, culling and blend state. The
   observer records explicit zero and overflow; do not interpret missing files
   as zero light contribution. No new original capture is required for this.
2. Allocate a separate full-resolution R10G10B10A2_UNORM RTV/SRV and clear it
   each frame. Do not repurpose the native diffuse target. Replay source
   skylight/probe equations into this auxiliary target, including AO at the
   source position before nonlinear clamps. The existing encoded ambient copy
   is useful evidence but applying sqrt(AO) afterward is only equivalent when
   the per-probe clamp does not intervene.
3. For each verified active native local-light draw, run the native draw
   normally, then replay its geometry into the auxiliary RTV with a pinned
   source-diffuse adapter and original additive blend. Preserve depth/stencil,
   viewport/scissor and rasterizer state. Remap the native G-buffer's normal
   and material fields explicitly, including source 7-bit Fresnel packing.
   Map color/intensity/influence from proved upload contracts, not shader
   symbol names. Restore all OM/PS bindings afterward. A scratch replay of
   native linear output followed by encoding can diagnose the encoding alone,
   but does not reproduce the source BRDF.
4. Add source sun and translucency contributions separately, with source
   shadow/packing conventions. The existing composition equations are useful
   starting points, but must write this accumulator before albedo, wetness,
   fog or tone mapping. Preserve per-draw 10-bit quantization rather than
   summing all contributions in float and quantizing once.
5. Supply this SRV to restored rain. An optional composition variant may
   decode the same auxiliary buffer, avoiding the current aggregate
   ambient/local-light inverse. Keep the existing native composition recipe
   available for A/B. Validate intermediate encoded buffers and masked final
   images, including lamp interiors and unlit outdoor regions.

## Active Factory contracts and isolated local replay

The three lamp/hangar views captured 56 draws, all the same shadowed spot
identity `33204e907519b2ad0afa798e6154aebc`: 16 streetlamp, 10 south partition,
30 north partition. All reports are single-frame with zero overflow. The
paired draws per light use different culling/depth tests and stencil INVERT
operations. A replay must copy the bound DSV before the native draw and use
that private snapshot; merely disabling stencil writes after the native draw
would still test against the wrong stencil contents.

`audit_local_light_authored.py ORIGINAL_PROJECT.json CAPTURE_DIRECTORY`
matches world-space positions reconstructed from the captured view constants.
Across all 56 draws, maximum position error is 8.68e-6, native color matches
`pow(round(authored_color*255)/255,2.2)` within 1.53e-7, and range matches
`authored_range / sin(penumbra/2)` within 1.05e-6. Intensity and attenuation
power match exactly. Every matched source diffuse influence is 1. Source
`light.cpp:125–131` also uses byte color divided by 255. Archived original
shadowed spot identity `cf1f1efdbb715fb563b4fccda358c400` confirms its sixteen
Poisson taps, normal offset of 12/map_size, and encoded output.

`legacy_spot_diffuse_encoded.hlsl` uses those native bindings with the source
diffuse Fresnel, attenuation, angular falloff and shadow equations. The active
native shader uses N·L for diffuse Fresnel, four shadow taps, and offset
`normalize(L + .75*N) * 24/map_size`. The adapter uses V·H, the source >.99
Fresnel fallback, sixteen Poisson taps and `N * 12/map_size`. It repacks the
supplied target Fresnel into seven bits; it cannot recover source material
values or normals already lost or changed in the target G-buffer.

`SRP_LEGACY_LOCAL_DIFFUSE=PATH_TO_COMPILED_ADAPTER` enables the isolated helper.
It keeps native draws intact, copies depth/stencil before each matched draw,
then replays into its own R10G10B10A2 buffer with the private DSV. It restores
OM targets, PS and the comparison sampler. The encoded target starts at zero
each Present. The source comparison sampler uses linear comparison filtering,
clamp addressing and LESS_EQUAL. Unknown local-light identities and resource
contract failures are counted, not silently treated as supported.

Rich composition captures save `.local-encoded.json` and the auxiliary buffer;
metadata labels it **local-light-only, zero ambient**. The helper does not bind
this buffer to composition or rain. Its C build passes warnings as errors and
the shader passes FXC `/WX /O3 /T ps_4_0`. The three-view runtime check below
validates the local-only replay; the complete source accumulator remains open.

Full ambient initialization remains open: the current source SSAO helper runs
after probes, beside `copy_encoded_ambient`. Faithful source probe replay must
instead obtain current-frame SSAO before probes, or retain each probe's draw
state for later replay. No aggregate inverse or post-blend AO shortcut is
promoted as equivalent. All seven new original views confirm rain's t4,
resolve's t3 and translucency-output rt0 are byte-identical, so the completed
source accumulator can serve both consumers.


## Local-only runtime validation

`026g/pipeline-lamp-hangar-encoded-local` contains three completed matched
captures using root commit `51ef0cf`. All 56 draws replayed into a full-size
R10G10B10A2 target with zero unsupported draws or resource failures. Observer
coverage is complete and its Present serial matches the helper in every view.
Native draws and targets remain intact; the helper is not displayed.

| View | Replayed draws | Native diffuse changed pixels | Helper nonzero pixels |
| --- | ---: | ---: | ---: |
| Streetlamp | 16 | 4,956 | 21,343 |
| South hangar | 10 | 455 | 1,393 |
| North hangar | 30 | 487 | 576 |

Every native-changed pixel also has a nonzero helper contribution. A CPU replay
of the source equations covers all 5,898 such pixels across four contributing
draws. Maximum discrepancy is 0.585 of one 10-bit step, including source
Fresnel, attenuation, sixteen shadow taps and output encoding. Rounding the
comparison reference to D16 resource precision is essential: leaving it at
CPU float precision produced false shadow-edge outliers up to 13.16 steps.

This is equation evidence on native-changed pixels, not independent geometry
or stencil validation. Extra helper pixels can include source shadow-filter
and low-light encoding differences; they are outside that numeric claim.
Source/native G-buffer differences and full source light accumulation remain
open. Native and helper images are from the same captured frame; separate
animated runs are not claimed byte-identical.

`reproduction/lamp-hangar-encoded-local-20261013` preserves 26 hashed checkpoint
files and a manifest of 4,868 capture files (4,693,926,506 bytes). It includes
the exact loaded DLL, compiled shader and source, launch recipe, CPU replay,
coverage and numeric results. Paths are relative to the main resource-porter
`.work/render-compare` directory. The private client/server stopped cleanly.
## Preparing ambient replay before local lights

`SRP_LEGACY_SSAO_EARLY_PROBES=1` is an opt-in scheduling experiment. It requires
the existing source SSAO assets/decoder and both source probe replacements on
026g. It recognizes only marked replacement probe shaders, obtains their
bound R16F depth at t2, and renders the existing source SSAO implementation
before the first probe. The later ambient hook reuses the result only when
both the Present serial and depth texture match. Ambient decoding rejects
stale SSAO. Context-state swapping preserves the interrupted native draw.
Rich captures add `.ssao-live.state.json` with rendered/captured Present
serials; this schedule has passed the C build but needs runtime validation.

The separate `legacy_probe_ambient_aux.hlsl` and
`legacy_probe_box_ambient_aux.hlsl` variants apply source AO to each probe
before its clamp, Fresnel and encoded output. They repack supplied smoothness
and Fresnel into source seven-bit fields and write diffuse only. They are
auxiliary replay shaders, **not replacements for native MRT probe draws**:
their private target must retain source probe ordering and alpha blend while
the existing native specular output continues normally.

`legacy_skylight_ambient_aux.hlsl` initializes an auxiliary target with the
010b source cube, wrap and height-ramp equation. It uses the native probe's
G-buffer/view bindings and an additional b3 containing the eight original
sky parameter vectors. Native skylight is a constant-only shader, so its
unused legacy fields cannot substitute for these source weather constants.
The initial sky replay must preserve source coverage/stencil; a fullscreen
unmasked initialization is not yet justified. None of these auxiliary
ambient shaders is wired to the scene or rain in this preparation commit.
All three compile with FXC `/WX /O3 /T ps_4_0 /E main`; the observer C build
also passes warnings as errors. No game tests were added.

Reproduce the source equation audit and export the weather CB with:

```sh
python tools/render-compare/audit_original_skylight.py ORIGINAL_POSES \
  --output skylight-audit.json --constants source-skylight.bin
```

This pins original shader `21c28f59ef738c2db31065650ff8249f`, validates input
formats and unblended output, and refuses mixed-weather constant exports.
On the seven frozen Factory-rain references (stride 8), all 92,658 changed
sample pixels agree within one 10-bit step; maximum channel error is .000550.
The report also preserves all 8,142 unchanged samples separately: many do not
match an unconditional shader evaluation, so this validates shader arithmetic,
not stencil behavior. The common 128-byte CB SHA256 is
`c440f33e565ef28dc19d3aae3f52b10eb5b93fbf6f5461ec7c976da6fd126bcf`.

Factory's original authored project contains 36 environment probes and no
ambient-light objects. That does not prove runtime absence of additional
ambient producers. School 021 has authored ambient lights and byte-identical
021/026 skylight shaders; this 010b sky equation must not be applied there.

## Sun and translucency preparation

`audit_original_sun_diffuse.py` reconstructs the original sun diffuse BRDF,
encoding and additive blend from frozen source G-buffer/constants. It uses the
captured final sun visibility, so it does not independently verify cascades,
shadow filtering or stencil coverage. At stride 17, all 79,848 changed samples
across seven original views agree within one 10-bit step (maximum .000563).
Unchanged samples are reported separately; one interior sample differs beyond
that tolerance, consistent with a coverage difference that remains unproven.

The optional 32-byte constants export contains source linear diffuse color
and shadow transparency, followed by the original translucency color. The
original source first quantizes the weather color to bytes. For these captures,
source sun diffuse is [.065842059,.065842059,.070000000], while translucency
uses [.064392105,.064392105,.070000000]. All seven views agree. Native
composition uses [.065909237,.065909237,.070000000]; the earlier translucency
helper used [.064481713,.064481713,.070000000]. Using unquantized authored
color therefore introduces a small additional mismatch.

`legacy_sun_diffuse_encoded.hlsl` and `legacy_translucency_encoded.hlsl` prepare
separate additive outputs on the native composition bindings, with the exported
source constants at b3. Sun uses source V·H Fresnel and applies visibility after
encoding. Translucency keeps its separate encoding and source five-bit material
packing. These shaders are not wired into the live accumulator yet.

Their native visibility/projection inputs still differ from original shadow
rasterization; they are not a replacement for that work. The caller must also
reproduce source coverage: sun uses stencil NOT_EQUAL zero, and translucency
uses EQUAL with reference 0xff/read mask 0x04. Native stencil-bit mapping must
be proved before enabling the latter. A nonzero translucency parameter alone
does not prove source coverage.

Both auxiliary shaders pass FXC `/WX /O3 /T ps_4_0`. The 19-file checkpoint
`reproduction/source-sun-aux-20261013` preserves their exact compiled outputs,
source, compiler commands, exported constants, original source contracts and
CPU audit results; its hashes verify. Target runtime validation remains open.
## Opt-in ambient initialization and probe replay

The next isolated module, `legacy_ambient_diffuse.c`, adds the prepared
ambient producers to the same private target used by local lights. Enable it
with `SRP_LEGACY_AMBIENT_DIFFUSE_DIR=ASSET_DIRECTORY` alongside the existing
source local adapter, source probes/SSAO and `SRP_LEGACY_SSAO_EARLY_PROBES=1`.
The directory contains the three compiled `legacy-*-ambient-aux.dxbc` files
and the exported `source-skylight.bin`. Shader identities are pinned; constants
must belong to the matching source weather.

The first verified probe supplies current G-buffer/view bindings. Before its
native draw, the helper copies the DSV. After that native draw it initializes
the private target using source skylight, then replays the probe with source
AO and alpha blend. Sky uses the source effect's explicit stencil rule:
NOT_EQUAL reference 0, read mask 0xff, no stencil writes, depth disabled and
RGB-only output. Each probe uses the untouched pre-native-draw private DSV,
native geometry/depth/culling and the source SRC_ALPHA/INV_SRC_ALPHA blend.
All changed PS, OM, sampler/resource, fullscreen VS/GS/IA/raster and constant
bindings are restored. Native MRT probe/specular draws still run normally.

The common buffer clears every Present, even when no probe or local light is
observed. Missing initialization, stale AO, unsupported probe identities and
probes after local-light accumulation make `source_ambient_valid()` false;
they cannot silently reuse a previous frame. The helper does not bind its
result to composition or rain. Runtime validation is still required.

Rich captures save `.sky-encoded` immediately after source sky and before the
first probe, paired with that frame's first `.probe-draw` inputs and
`.ssao-live` AO. Its metadata records the native probe stencil state too.
`.ambient-encoded` is sealed before the first local draw, with a composition
fallback for a zero-local frame. `.local-encoded` then contains ambient plus
local lights and explicitly reports whether ambient initialization was valid.
Future source sun/scattering helpers must call `capture_ambient_diffuse()`
before their first addition, including frames without local lights. This
preserves a reviewable ambient-only boundary before the full accumulator.

The first five-view runtime (`pipeline-factory-encoded-ambient`) completed
with one sky initialization each, 112 probe draws and 72 local draws, zero
failures and matching AO/ambient/local Present serials. Reproduce the audit:

```sh
python tools/render-compare/audit_ambient_auxiliary.py CAPTURE_DIRECTORY \
  --output ambient-auxiliary-audit.json
```

All 64,475 nonzero sampled sky pixels match the source equation using the
actual target G-buffer and current-frame source AO within one 10-bit step;
maximum channel error is .000549. The first probe's captured depth/stencil
state matches the source effect in every view. Local accumulation never
decreases RGB across all 4,608,000 pixels. Yard north/south receive no local
RGB change; interior, upper and streetlamp gain 132,755, 9,371 and 21,819
pixels respectively. This verifies sky arithmetic, scheduling and monotone
encoded addition. It does not prove all probe equations, exact local raster
coverage or original-vs-target material parity. Native changed-pixel masks
can omit source-only illumination when the shadow kernels differ.

## Source and native stencil meanings

Source `geometry_type.h:7–18` assigns 0x80 plus geometry class: terrain 1,
static 2, speedtree 3, translucency 4, skeletal 5 and particles 6.
`stage_gbuffer.cpp:370–382` explicitly selects the static/skeletal/translucent
references for their batches. `effect_gstage_default_materials.cpp:116–126`
also selects 0x84 when the material uses a translucency texture. Source sky
and sun use NOT_EQUAL 0 across all stencil bits. Source translucency uses
EQUAL reference 0xff with read mask 0x04, which includes classes 4, 5 and 6.
This is not an independent material boolean bit.

Native 026 has a different producer contract. In the shipped default-material
compiler, VA 0x1de6ec–0x1de6fe masks a byte to six bits, compares it with
0x800, and computes reference 0x81; VA 0x1de749 writes that reference.
The six-bit value cannot equal 0x800. Terrain (VA 0x1e4ab3) and no-material
(VA 0x1bfdc2) compilers explicitly write 0x81 too. Native
`stage_gbuffer::execute` at VA 0x177a20 no longer contains the three source
stencil-reference batches. Therefore copying source translucency's 0x04
mask onto native stencil is unjustified. Final native stencil can still
support a nonzero-geometry sun diagnostic, but its actual values must be
captured. Temporary volume INVERT operations turn 0x81 into 0x7e, changing
bit 0x04; use the actual post-light DSV, not an arbitrary pre-light snapshot.

Source `translucency.ps:68–70` separately checks the original albedo alpha's
high bit. The packed five-bit translucency amount alone is insufficient:
non-translucent source materials can still store a nonzero amount. Native
material shaders `93d3718d4821d653d85cf4c1e4ea58ae` and
`c7681c727775f38ce61f2d72ab3cc80a` write parameters.z from the translucency
texture's red channel; inspected non-translucent producers write zero.

`audit_translucency_gate.py ORIGINAL_POSES NATIVE_POSES --output REPORT.json`
compares those semantics on the same-weather, exactly matching R16F depth
subset. Across the five Factory views, that subset contains 2,148,079 pixels.
Of these, 1,977,056 have a nonzero original packed amount with its material
flag off. Native positive-z versus original flag-off occurs at 170 pixels:
163 in upper foliage and 7 near the streetlamp. Quantized depth equality
does not establish matching material/UV at moving foliage edges, so these
exceptions remain unresolved. Native positive-z is a useful candidate gate,
not a proof of source stencil coverage or a replacement for that audit.
