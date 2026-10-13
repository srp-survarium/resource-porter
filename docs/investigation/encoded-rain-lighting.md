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

No producer adapter is enabled by this work. Active local-light contracts and
the source/native parameter upload mapping remain the next required evidence.
