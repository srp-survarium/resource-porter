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

The completed `pipeline-factory-encoded-sun-v2` captures resolve the final
native stencil contract: all five views contain only 0, 1, 2, 4 and 8. Upper
contains respectively 15,133, 114,159, 230,209, 298,506 and 263,593 pixels.
Native `stage_shadow_mask::execute` at VA 0x1a1910 constructs the reference
array {1,2,4,8} at 0x1a1ada–0x1a1af6, writes each cascade's reference to
backend+0x154 at 0x1a1cb7–0x1a1cc1 before its volume draw, and reuses it
for the resolve at 0x1a1e4f–0x1a1e59. `effect_shadow_mask::compile` at
0x1d1b00 uses full stencil masks, ALWAYS comparison and REPLACE on depth
failure (0x1d1bb1–0x1d1bf2). The cascade pass has overwritten the earlier
geometry/light-volume references. In particular, final bit 0x04 identifies
a cascade, not the source translucent material class.

## Sun/translucency auxiliary replay and source-stage comparison

`SRP_DIAGNOSTIC_SOURCE_SUN_DIR` enables the guarded source sun shaders and
captured constants after the private sky/probe/local accumulator is ready.
`SRP_DIAGNOSTIC_TRANSLUCENCY_PARAMETER_GATE=1` adds the diagnostic material
gate described above. Both paths preserve the displayed scene and rain.
The helper copies the actual final native depth/stencil at composition,
checks current-frame ambient and sun-projection readiness, and restores all
modified D3D state. Loading screens without scene probes are skipped;
an armed, settled capture without those inputs reports an error.

`audit_sun_auxiliary.py CAPTURE_DIRECTORY --output REPORT.json` checks both
changed and unchanged sun samples against the source equation on captured
native inputs. All 271,060 stride-17 samples in five views are within one
10-bit step (maximum .000549118), including 70,541 changed samples.
This establishes arithmetic and scheduling, not original shadow coverage.
`reproduction/factory-encoded-sun-20261013` preserves the exact loaded DLL,
helper source, shader assets, launch scripts, CPU reports and capture hashes.

```sh
python tools/render-compare/audit_translucency_auxiliary.py CAPTURE_DIRECTORY \
  --original ORIGINAL_POSES --output translucency-auxiliary-audit.json
```

The script checks current-Present readiness, actual native DSV, source
constants and input formats before replaying the material-gated source
translucency equation. Native red shadow visibility is compared with the
captured projected depth; the amount is reduced to the original five-bit
gamma packing. RGB is encoded and added to the preceding 10-bit buffer.
Changed and unchanged samples are both checked; a full-frame audit rejects
alpha changes and writes outside nonzero stencil or positive parameters.z.

All 354,465 stride-13 samples in the five completed v2 views match within
one 10-bit step; maximum channel error is .000548161. Across 4,608,000
pixels there are no gate or alpha violations. Changed pixel counts are
17,976 interior, 465,529 upper, 134,153 streetlamp, 8,415 north and 151 south.
The first attempt without the v2 suffix stopped during loading and is not
valid evidence. This verifies arithmetic on native inputs under the stated
diagnostic gate, not exact original material or raster coverage.

Original `renderer.cpp:449–455` orders ambient, sun, local lights, then
translucency. Consequently original `sun-before` is the ambient boundary,
`translucency-before` includes ambient+sun+local, and composition input t3
is the final accumulator. The last boundary is byte-identical to captured
translucency output in all seven source views. The auxiliary currently adds
locals before sun. Nonnegative encoded addition is commutative in exact
arithmetic, but order-dependent finite precision and per-draw conversion
remain a limit; an aggregate square-root approximation is not used here.

With the fixed HUD/weapon exclusion and a stricter 3×3 exact-R16F-depth
mask, final encoded RGB MAE is .004460 interior, .005958 upper, .001681
streetlamp, .005501 north and .003981 south. Sample counts vary markedly:
1,507, 1,496, 32,366, 32,896 and 27,205 respectively. The predefined forest
region's exact-depth subset has close mean encoded RGB (source
.19531/.19912/.19924, auxiliary .19731/.20038/.19866), but per-pixel MAE
is still .02134. Matching quantized depth and means cannot establish
matching foliage, normals, material values or shadow phase. The report
retains all three stage boundaries, looser masks and regional errors so
these remaining differences are not hidden by an overall mean.

## Same-frame composition comparison

`SRP_DIAGNOSTIC_COMPOSITION_SOURCE_DIFFUSE=1` pairs with
`SRP_LEGACY_COMPOSITION_REFERENCE` compiled from
`legacy_composition_control.hlsl` with `CONFIG_FULL_ENCODED_DIFFUSE=1` and
`CONFIG_NO_RAIN_DARKENING=1`. The diagnostic binds the complete private
UNORM accumulator at t15 and replaces only diffuse lighting with `8*E*E`.
Native albedo conversion, specular accumulation, sun specular and material
tint remain fixed. AO, sun and translucency are already in E and are not
added again. This is a pre-fog comparison, not full composition parity.

The replay requires current-frame ambient, both sun/translucency draws and
zero local failures. It restores t15 before redrawing the displayed shader.
`pipeline-factory-full-diffuse-comparison` completed all five views with no
hook errors. Outputs are rt00 preserved approximation, rt01 native, rt02
full diffuse, and rt03 restored approximation. Every rt03 is byte-identical
to rt00. The exact loaded build and reproduction scripts are preserved in
`reproduction/factory-full-diffuse-comparison-20261013`.

`audit_full_diffuse_composition.py CAPTURES --original ORIGINAL_POSES
--output REPORT.json` compares R11G11B10_FLOAT outputs from the displayed
approximation, native composition and the full encoded diffuse diagnostic.
The audit also checks every pixel for nonfinite
values and requires the restored displayed output to equal its original
bytes. Known zero/unit HDR decoding and error metrics were checked directly;
no game test functions were added.

The completed five-view `pipeline-factory-full-diffuse-comparison` passes
restoration equality and finiteness for all outputs. Geometry-only HDR RGB
MAE against original composition, on the fixed exclusion mask and stride-4
grid:

| View | Samples | Displayed approximation | Full encoded diffuse |
| --- | ---: | ---: | ---: |
| Interior | 33,704 | .001595 | .001252 |
| Upper | 32,734 | .015502 | .008635 |
| Streetlamp | 8,446 | .010417 | .006029 |
| Yard north | 33,669 | .001612 | .001525 |
| Yard south | 33,688 | .003235 | .003458 |

The south view regresses 6.9%; the fixed back-wall and forest regions also
regress slightly (.000568→.000621 and .006644→.006817, respectively). These
regions have only 630 and 323 exact-depth samples. Upper's strict 3×3
exact-depth geometry subset contains just 646 samples. All counts and
regional errors remain in the report. A common exposure diagnostic metric
uses the same `((exposure*x)/(1+exposure*x))^(1/2.2)` mapping for every image;
it supports the same whole-view improvement/regression directions without
claiming to reproduce the game's tone mapping.

Sky needs explicit rejection here: both clients store finite far-clear
depth 499.75, but original composition precedes sky while native composition
preserves already drawn sky. Exact depth alone therefore admits a large
false mismatch. The audit requires actual native nonzero stencil and both
depths below that verified clear value, and refuses an unexpected clear
contract. The first sky-contaminated report is retained as excluded evidence.
Earlier encoded-buffer comparisons used a looser finite-depth mask; their
counts must not be substituted for these geometry-only counts. This remains
a pre-fog diffuse experiment, not proof of downstream rain/fog or full
material parity, and the displayed recipe was not changed.

## Wall residual decomposition

`audit_wall_composition.py CAPTURES --original ORIGINAL_POSES --output REPORT`
uses every exactly matching depth pixel in the predefined south shadow-wall
and interior back-wall rectangles: 17,000 and 9,800 respectively. In both
clients, final encoded diffuse equals ambient encoded diffuse at every one
of these pixels; direct sun/local/translucency additions cannot explain the
residual. Component substitutions below are CPU counterfactuals, not fitted
gains or proposed image replacements.

South's native linear albedo averages .3380/.3332/.3399 versus source
.3274/.3215/.3239. Substituting only the frozen source albedo reduces HDR
MAE from .002748 to .001162; substituting lighting alone gives .002674.
Changing the target albedo decode to gamma 2.2 instead makes it worse,
.002858. The mismatch is upstream of resolve; identify the bound material
and its G-buffer production before changing it. A global gamma correction
is not supported by this evidence.

Back-wall linear albedo means agree within .000113. Its private ambient
blue lighting is .03967 versus source .03723. Substituting source lighting
reduces HDR MAE .000618→.000327; source albedo alone gives .000589. This
points to ambient/probe production and the remaining native specular path,
not the same albedo issue as the south wall.

To separate specular from output rounding, the audit also checks exact
source and native composition arithmetic on shadowed dielectric pixels
where both specular tint formulas simplify. All 17,000 south and 9,332
back-wall samples replay within one R11G11B10 channel ULP. South source
specular averages about .0000009 per channel versus native .000395;
back-wall source averages .0000065 versus native .000148. Native sun
visibility is zero on this subset, so the excess is in the accumulated
specular input. These are small absolute terms but materially affect these
dark surfaces.

`environment_probe_lighting.ps:224–233` applies source `(1-gloss^3)`, AO to
the fifth power, a cap of four and encoded alpha blending to specular.
The retained native branch in `legacy_probe_diffuse.hlsl` has different
cube decoding, BRDF/mip selection and linear AO, and lacks that cubic
suppression. The next justified lighting experiment is a separate source
specular producer/accumulator; adding an empirical gain to full diffuse
would conceal two distinct remaining contracts.

## Correct original diffuse cube mip

Before changing specular, the original probe upload exposed a separate,
precise ambient discrepancy. Every frozen first probe uses shader
`cc45055d632b75e94160d8a1595b5393`, uploads integer `num_mips=9`, and binds
a 256px cube with seven stored mips ending at 4×4. The archived DXBC
subtracts two from that integer and samples LOD 7, which clamps to stored
LOD 6. Source sampler s6 is linear-clamp with minLOD 0 and maxLOD FLT_MAX.
`environment_probe.cpp:45–60` derives the upload from authored resolution,
including the absent 2×2 and 1×1 levels. All 36 authored Factory cubes
confirm this: 35 are 128px/six stored mips, one is 256px/seven stored mips;
each source diffuse request resolves to its 4×4 tail.

The ported diffuse cube preserves the 16×16, 8×8 and 4×4 tail verbatim.
The prior adapter sampled mip 1 (8×8); the correct source request for these
assets is mip 2 (4×4). The earlier comment calling 8×8 the original sample
was incorrect. `CONFIG_SOURCE_LAST_CUBE_MIP=1` enables this correction only
when compiling an explicit diagnostic variant. Runtime opt-in is
`SRP_DIAGNOSTIC_SOURCE_AMBIENT_LAST_MIP=1`, paired with the corresponding
`SRP_LEGACY_AMBIENT_DIFFUSE_DIR`. It pins both new shader identities, checks
every bound diffuse cube is BC3_UNORM 16×16 with exactly three mips and six
faces, and records `diffuse_cube_mip` in ambient metadata. The displayed
probe shader and previous checkpoints remain unchanged.

Both sphere and box auxiliary variants compile with FXC `/WX /O3 /T ps_4_0`
and the define above; C builds with warnings as errors. Assets and their
source/hash manifest are isolated under `encoded-rain-lighting/mip-last`.
The source skylight shader and constants are unchanged.

`audit_original_probe_mip.py` compares the two mip hypotheses on frozen
original data. The first captured probe before/output is unchanged in all
seven views, so it supplies no arithmetic samples. Instead the audit uses
final ambient versus initial sky only outside conservative bounds of every
other authored probe. Three-face cube corners are excluded, and ordinary
edges map neighboring-face texels. Across 23,591 samples from six views,
4×4 has RGB MAE .00032–.00057 versus .00342–.00693 for 8×8. Some corrected
errors exceed one 10-bit step (maximum .00305); filtering/raster precision
is not fully reproduced. Yard south contributes zero samples after the
coverage restriction. These limits are explicit: the exact mip contract
comes from shader/upload/SRV/sampler/asset evidence, not a claimed perfect
CPU replay or an image-based fit. Target runtime validation remains pending.

### Last-mip target validation

`pipeline-factory-source-rain-mip-last` completed five views with mip 2,
one sky initialization each, and 16/32/30/8/26 probes. Ambient, SSAO,
local lights, final native depth and sun/translucency share each view's
Present serial. No failed or unhandled draws were recorded. The sky and
translucency CPU replays remain within one 10-bit step; all five restored
composition outputs are byte-identical to their displayed counterparts.

Against `pipeline-factory-full-diffuse-comparison`, full-diffuse pre-fog
HDR MAE on the fixed geometry mask changes as follows. These are separate
captures, not an isolated same-frame mip substitution:

| View | Prior mip 1 | Corrected mip 2 |
| --- | ---: | ---: |
| Interior | .001252 | .000933 |
| Upper | .008635 | .008474 |
| Streetlamp | .006029 | .005790 |
| North | .001525 | .001676 |
| South | .003458 | .002361 |

North's broad-mask regression remains explicit; its exact-depth error
instead slightly improves (.001453→.001431). Outdoor depth and material
bytes differ between captures, so these aggregate changes cannot all be
attributed to mip choice. The encoded-stage audit now also rejects the
499.75 far-clear depth and zero native stencil, matching the composition
geometry gate. Previous encoded metrics admitting clear sky are superseded.

The two predefined static wall regions give stronger local evidence.
All 17,000 south-wall and 9,800 back-wall depths still match the source,
and neither region receives local/sun/translucency contributions. Corrected
south diffuse RGB means are [.052860, .056429, .055789] versus source
[.052871, .056425, .055791]. Back-wall means are [.042311, .042037, .037202]
versus [.042307, .042056, .037231], removing its previous blue excess.
Back-wall albedo means are unchanged between target runs; HDR MAE improves
.000618→.000328. South albedo differs between runs and remains above the
source, so its .002748→.001467 HDR improvement has mixed causes. Native
specular and source material decoding remain unresolved; accurate diffuse
does not establish complete composition parity.

Reports are preserved under `encoded-rain-lighting/` as
`mip-last-readiness.json`, `ambient-mip-last-audit.json`,
`translucency-mip-last-audit.json`, `mip-stage-comparison.json`,
`full-diffuse-mip-last-audit.json`, and `wall-mip-last-audit.json`.

`audit_common_encoded_pixels.py ORIGINAL PRIOR CURRENT --output REPORT`
additionally intersects exact-depth geometry masks across both target
runs. Requiring identical target normal/albedo/parameter bytes leaves
31,361 interior, 5,573 upper and 2,831 streetlamp samples. Their encoded
ambient MAE improves .003390→.000696, .011267→.010594 and
.006915→.005447 respectively; final encoded MAE also improves. North has
only 28 such pixels and south zero, so that stronger subset cannot support
an outdoor-wall claim. The report explicitly retains these counts and
notes that identical G-buffer bytes do not establish identical AO or probe
coverage. `mip-common-pixels.json` preserves the result.

### Source ambient-specular preparation

`legacy_probe_specular_aux.hlsl` and its box wrapper are isolated, unbound
source010 high-quality specular producers using the verified native probe
constant-buffer layout. Both compile with `/WX /O3 /T ps_4_0 /E main`.
Sphere DXBC is 6,188 bytes, identity
`28a92a5b1e1a392624b972fd0c4b3cce`; box is 6,160 bytes,
`ca8b68c29830a8ba101a1f5534a387e9`. Build hashes and assembly are preserved
in `encoded-rain-lighting/ambient-spec/build.json`.

The shaders restore source gamma-2.2 cube decoding, seven-bit Fresnel and
gloss, the high-quality BRDF and its mip selection, cubic gloss suppression,
fifth-power saturated AO, cap of four and encoded output. They do not alter
the existing diffuse shaders or displayed recipe. The source sky initializes
no specular: its accumulator starts at zero before the probes blend.

The source-only replay in `audit_original_probe_mip.py` now reports the
specular branch separately. On the conservative global-probe-only subset,
23,545 samples have RGB MAE .00021–.00027. Fourteen exceed one 10-bit step;
the maximum .001237 is about 1.27 steps. This supports the source arithmetic
but remains an approximate cube-filter replay, not a target runtime pass.

Runtime integration must allocate its own encoded specular target and clear
it every Present. It must replay using a second copy of the *pre-native*
DSV: the existing diffuse replay can invert stencil, so replaying specular
against its already modified snapshot changes coverage. Preserve native
MRTs, shader, AO binding and blend state; seal and report the ambient result
before local lights. An absent initialization or unsupported source feature
must produce invalid metadata, never silently retain a prior frame.

Six of the 36 authored Factory probes enable source clip-by-normal; none
has source cube shadows. Their authored control/center must be mapped before
claiming a complete ambient-specular accumulator. The draft does not yet
implement those features or bind itself at runtime. Local and sun specular
also need separate producers. Finally, source composition's packed five-bit
material specular amount is not recoverable from native parameter z alone:
opaque source materials retain a specular-intensity texture there while
native z is zero. Producer validation and material-resolve parity therefore
remain separate work.

### Guarded probe controls and material amount mapping

`audit_probe_controls.py PROJECT CAPTURES... --output REPORT` resolves a
sphere from its active `probe_location` (b0 byte176). A box center is the
solution of `A*center=-translation` for `world_to_outer_probe` (b0 byte128),
not its unused `probe_location` field. Box scale comes from the active
`outer_cube_side_sizes` or the reciprocal row norms of that inverse matrix.
Matching requires a unique authored center and geometry kind within .001,
and box dimensions within .001; unknown or ambiguous identities fail.

Across 154 frozen root Factory probe draws, these checks identify ten of
36 authored probes. Maximum center error is .00000373 and box-size error
.00000079. None of the six clip-enabled probes occurs among these captured
first-per-kind draws. Their absence from that observer is not proof that
they never render. The authored controls to recognize are:

| Probe | Kind | Center | Box half-size / sphere radius |
| --- | --- | --- | --- |
| 26 | box | 31.824, 1.908, 7.192 | 6.288, 6.754, 6.119 |
| 36 | box | -36.515, 2.150, -93.385 | 4.221, 2.093, 5.512 |
| 10 | box | -48.615, 2.360, -100.006 | 4.200, 1.979, 12.529 |
| 11 | sphere | -32.573, 8.002, -6.428 | 5.700 |
| 28 | box | -36.515, 2.150, -103.909 | 4.221, 2.093, 5.512 |
| 31 | sphere | -21.537, 28.220, -13.724 | 7.400 |

All six have zero authored rotation and no cube shadows. For these boxes,
the expected inverse matrix has diagonal `1/scale` and translation
`-center/scale`; off-diagonal terms are zero. A future replay can upload
source center and clip flag through its own restored constant buffer only
after matching these controls and confirming the bound cube identity and
source multipliers. The source attenuation multiplier is exactly
`saturate(dot(normalize(center-world_position),world_normal)+.75)`, applied
before probe shadowing. Do not reuse a rain control table for day/evening:
those projects contain different probe placements. `probe-controls.json`
retains all authored controls, captured identities and the missing coverage.

`audit_specular_amount.py ORIGINAL CURRENT --output REPORT` isolates the
material-resolve issue on the two fixed wall rectangles. All 17,000 south
pixels have source amount31, translucent flag0, native z0/w255: their
material amount agrees at one, so the large specular difference there is a
producer discrepancy. Backwall has 7,410 such pixels, but the remaining
2,390 retain source amounts0–20 with native z0/w255. Twenty-five identical
native parameter vectors map to multiple source amounts. For example,
[10,252,0,255] covers 988 pixels with source amounts3,4 or31. Thus even the
whole native parameter vector cannot uniquely reconstruct the original
amount on these frozen surfaces.

Original `gbuffer_pass.ps:342–345` builds opaque amount from
`abs(specular_color_parameter.x)^2.2`, optionally base-texture red and the
specular-intensity texture/range, then stores five gamma-encoded bits.
Translucent materials override that input. The inspected native static
producer instead stores texture/range times its specular-color parameter
in parameter w, without the source packing. It has z0 for opaque materials.
Neither a global w-to-source conversion nor copying z is justified. The
backwall pixels still need exact material/draw and feature attribution;
these histograms do not establish whether conversion, a shader feature,
or producer semantics removed the amount. `specular-amount-mapping.json`
preserves counts and all ambiguous vectors. No material or runtime patch
is applied by either audit.

### Factory rain cloud coordinates

The streetlamp's large final-screen sky residual is a separate coordinate
contract. `audit_factory_cloud_coordinates.py` replays fixed, shader-derived
phase/vertical-scale hypotheses against frozen cloud before/output buffers.
It decodes the original BC3 sRGB texture and its three sampled mips, uses
the source captured view-projection matrix, and restricts samples to common
far-clear sky with the source minimap excluded. Sphere interpolation and BC3
filtering are approximate; these are not exact raster arithmetic checks.

In `pipeline-factory-live-diffuse-v2`, source and target cloud mip0 bytes
are identical (SHA256 `c3e8db1ee38b307815a03f471665ca6512a92b208e095e9ed7622a85a2a47edc`).
Source tint47.455070 equals the target4.745507 times the adapter's factor10;
fade parameters and multiplicative blend also match. The captured source
VS uploads `(cos180,sin180)=(-1,~0)` and has no clock input. The native VS
contract uploads `(sin,cos)`, multiplies mesh V by .96, and can add
`.01*rotation_speed*scene_time` to U. The early converter sets that speed
to zero; the next native VS capture must confirm the live upload.

Across 1,613 common streetlamp sky samples, the source pass agrees best
with the equation-derived world-U phase+.75 and Vscale1 (HDR MAE .00431),
versus .06163 with native coordinates. The target pass independently agrees
with phase+.5 and Vscale.96 (MAE .00463); source coordinates against the
unchanged target output give .06022. Other quarter-turn/sign hypotheses are
retained in `factory-cloud-phase.json`, along with final screenshot sky-only
RGB MAE .09034. This supports the same quarter-turn/vertical-scale correction
found independently for Chemical Plant, rather than a time-alignment fit.

`legacy_rain_clouds_source_coordinates.hlsl` is an opt-in Factory010 rain
wrapper adding U+.25 and dividing V by .96 through the existing coordinate
adapter. It leaves texture, tint, fade, blend and prior shader assets intact;
compilation rejects the day/evening/Chemical weather defines. Its assumptions
are source U tiling1, zero native cloud rotation speed and the verified native
VS ABI. Runtime validation and actual native VS upload capture are pending;
this CPU evidence does not promote it into the preserved displayed recipe.

The coordinate candidate compiled with `/WX /O3`: 1,580 bytes, DXBC
`8bedbb39b27f204457b180888029e8e0`, SHA256
`b7e9de309913721cce3b5f20dc6279dc8d57bbbae5c73b67178f4d9a662b3087`.
`encoded-rain-lighting/cloud-source-coordinates/build.json` pins source and
assembly hashes. Both captured cloud samplers are linear wrap (filter21,
address1/1/1) with unrestricted mip range; both texture SRVs are BC3 sRGB,
4096×1024, nine mips starting at zero. The portable audit now guards these
contracts and, when native VS capture is present, requires its known identity
and the 180-degree, zero-speed upload before interpreting the phase result.
Using the target pre-cloud sky with corrected coordinates predicts original
cloud-output MAE .00750 versus .06207 with current coordinates. That remains
a CPU counterfactual; final-screen improvement awaits target capture.

`pipeline-factory-live-rain-v2` supplies the missing live vertex contract.
All five draws use native VS `0a351e7ae0d6ca39d2286ec07af70b62`, identity
m_W and cloud parameters `[-8.74228e-8,-1,0,100000]`: rotation180, zero
speed, and the same source sphere radius100000. Scene time advances from
18.68 to95.00 seconds without changing these controls. The guarded replay
passes and repeats the coordinate result on 1,576 common lamp sky samples:
source+.75/V1 MAE .00433, native+.5/V.96 MAE .00480. Final sky RGB MAE
remains .09055. This closes the upload ambiguity and rules out native cloud
clock phase for this captured recipe.

`audit_cloud_sky_comparison.py ORIGINAL BASELINE CURRENT --output REPORT`
prepares final candidate validation. It uses the intersection of source,
baseline and current far-clear depth, excludes fixed HUD/weapon regions,
and reports sky, geometry and full fixed-mask screenshot errors separately.
When current depth is absent for a screenshot-only pose, that limitation is
explicit and the original/baseline mask is reused. Available HDR cloud
before/output buffers get separate sky metrics; later rain/post effects in
screenshots cannot then be confused with the cloud pass itself.

For the two uncorrected live runs, the shared lamp sky mask has 1,542 pixels
and screenshot MAE .09181/.09124. Upper has only53 sky samples; the other
three views have1–3 and cannot substantiate a broad sky claim. Reports
`factory-cloud-live-rain-phase.json` and
`factory-cloud-live-rain-comparison.json` preserve those counts. The next
candidate run should reuse these views and report its geometry regions too;
no new original capture is required.

The corrected `pipeline-factory-live-rain-clouds` run validates the GPU
cloud pass. On the same1,542 lamp sky pixels, cloud-output HDR MAE falls
.061651→.005928 (90.4%) and RMSE .073396→.008401. The pre-cloud target
sky is identical between the two runs on that mask, isolating the coordinate
change from underlying atmosphere. The guarded replay now selects the
source-derived phase+.75/V1 for the actual corrected target (MAE .00430).
The active pixel shader is the compiled coordinate candidate.

The lamp's rich readback exceeded the initial30-second wrapper deadline,
so its cloud buffers are complete but its final screenshot is absent.
`--pass-only` audits those completed cloud buffers without pretending a
final image exists. Four other final screenshots completed; upper sky
MAE improves .08664→.03236 on53 samples, while the other three views
have only1–3 sky samples. Final lamp validation awaits the separate recovery
batch. Reports `factory-cloud-corrected-pass.json`,
`factory-cloud-corrected-pass-comparison.json` and
`factory-cloud-corrected-four-comparison.json` preserve this boundary.
