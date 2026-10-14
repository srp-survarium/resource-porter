# 0.11e initial sky and encoded ambient replay

The post-only correction improves all three Chemical test views and all seven
Radar controls. Correcting probe falloff alone does not restore the missing
indoor light. The current inherited recipe omits the donor's initial sky pass.

The private Chemical diagnostic restores that pass and source cubic probes in a
separate 10-bit accumulator. It does **not** activate full diffuse composition.
The shared playable package and archive are unchanged.

## Donor contracts

- Original `skylight.ps` DXBC `21c28f59ef738c2db31065650ff8249f`
  (3,908 bytes, SHA256
  `ced5345f1cf23b1c8ce9eb050debc71c4fb281f0072aae2d813cc7a5d2232b11`)
  is identical to the already audited 0.10b sky shader. It uses height, an
  authored six-face sky color, wrap light, and source AO before encoding.
- Original executable `stage_ambient_lighting::execute` VA
  `625d41..6261b4` uploads the same eight-vector layout. `625d53` calls the
  component-wise power helper `68850`; constant `981eac` is 2.2.
  Constructor `626b3b..626b5d` initializes multiplier1 from `a75e14`.
- Both donor maps have identical sky colors: horizontal faces
  `[.509804,.529412,.564706]`, upper/lower faces white, multiplier.5,
  lower0, upper5, power.1, and zero wrap light. A representative horizontal
  sun direction is therefore equivalent for this pass. The exporter rejects
  asymmetric horizontal faces or nonzero wrap light. Across120 direction/normal
  cases, changing azimuth changes evaluated RGB by at most `5.56e-17`.
- Both frozen source profiles disable LPV. The missing contribution must not be
  attributed to an absent LPV simulation.
- All eight 011e point/spot shader permutations are byte-identical to extracted
  010b shaders. Sun quality3/shadow0 is also identical (`757f44b1...`).
  Native visibility and G-buffer packing remain separate limitations.
- 011e SSAO accumulation differs from the inherited shader. Its used inputs are
  depth, the donor's four-by-four rotation texture, screen size, saturation and
  radius. The rotation texels also differ. Spatial upsample is byte-identical.
  Source execute `704bc9..704c02` uploads saturation then radius directly:
  Chemical1.3/1.3; Radar1.5/1.5. The prior replay used2/1.5.
- 011e translucency is **not** the 010b contract. It uses shadow-depth attenuation
  `1-pow(saturate((projected-shadowDepth)/.0015),.11)`, changed sun terms and a
  world-normal/eye-direction term. Native green has an additional .001 depth
  bias. The old translucency helper must not be presented as a complete011e fix.

## Implementation and checks

`export_skylight_011e.py MATERIAL --output source-skylight.bin` exports the
checked layout and hashes its input/output. `SRP_DIAGNOSTIC_SOURCE_AMBIENT_011E=1`
selects exact pinned ambient DXBC identities, eight-tap division by8, the last
physical cube mip, source cubic falloff, and AO before probe encoding. It also
retains the existing16-square/three-mip diffuse-cube guard.

`SRP_LEGACY_SSAO_PARAMETERS=saturation,radius` explicitly selects positive,
finite authored values. The original default remains unchanged. Captures record
both values. Use the corresponding donor shader and noise assets as well.

`SRP_LEGACY_POINT_DIFFUSE` enables only the compiled unshadowed point adapter on
exact native identity `64f2a63c...`, alongside the existing shadowed spot path.
Other local identities remain unhandled, and full composition refuses an
incomplete accumulator. Native MRTs, specular and depth/stencil are restored.
The point adapter has not yet been exercised by a visible point draw.

FXC `/WX /O3 ps_4_0` and the strict C build (`-Wall -Wextra -Werror`) pass.
The exporter rejects asymmetric/wrap-lit inputs and produces identical vectors
for the two maps. No additional game tests were added.

## Private runtime, not full lighting validation

Artifact root: `/tmp/resource-porter-chemical-011e-lighting/.work/ambient-011e`.
`runtime/chemical-011e-ambient-diagnostic` contains three matching views, only the
first with full buffers. Owned2265 client/server/prefix stopped and waited;
all three recorded PIDs are absent. The private archive was read-only.

East hall Present767 has one sky initialization, eight probes, same-frame AO,
and zero ambient failures. All14,400 sampled nonzero sky pixels agree with the
source CPU equation within one10-bit step: maximum error.000548303 and
RGB MAE.000241470. Six supported spots replay; two unshadowed spots
(`648947ae...`) are deliberately reported unhandled. Thus local coverage is
incomplete and full diffuse stays disabled. No point draw appears in this pose.

`audit_ambient_auxiliary.py --allow-incomplete-locals` checks this private sky
math while reporting the missing local coverage; its default still rejects it.
The six supported spot contributions never decrease the encoded accumulator.
Source shader/math correctness is separate from matching original geometry,
visibility, original material packing, and the final image.

The diagnostic screenshot uses the donor SSAO shader/noise/parameters and the
post correction. Private sky/probe/local RGB is not yet copied into the live
scene, so its screenshot is not evidence of the final ambient improvement.
Fixed-exposure buffer previews are separately labeled.

## Frozen references

All29 original011e and29 baseline026 views, actual camera metadata, quality
configs, contact sheets and exact post/probe builds are copied and hash-verified
under main `.work/render-compare/reproduction/reference-011e-20261014`:
222 files,297,588,932 bytes. The original source does not need another session.
