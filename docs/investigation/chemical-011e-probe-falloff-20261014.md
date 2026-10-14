# Chemical Plant 0.11e sphere falloff

Keep the donor's explicit `radius` and `probe_side_width`. The apparent reversed
falloff interval is present in 0.11e itself; adjusting the width to make that
interval positive would change source behavior. This finding applies to sphere
radial attenuation, not complete probe-lighting equivalence.

## Authored object and equations

`environment_and_lights/probes/environment_probe` is a sphere at
`[41.4560013, 23.7940006, 25.3649998]`, with radius `340.9010009765625`, width
`0.5`, `clip_by_normal=false`, and `with_shadows=false`.

For distance `d`, the original computes:

```
inner = radius - width - .05
outer = .98 * radius
t = saturate((d - inner) / (outer - inner))
radial_alpha = 1 - t*t*(3 - 2*t)
```

Here `inner=340.351001` and `outer=334.082981`. This explicit shader arithmetic
remains defined when the interval is reversed: radial alpha is zero below the
outer boundary, one above the inner boundary, and one-half at their midpoint.
These are radial-factor values, not a prediction of the final rendered colour.

Native 0.26 computes the same factor after uploading
`outer=.98*(radius+delta)` and `inner=radius-width+delta-.05`. Its existing
per-probe perturbation is in `[0,.1)`. Preserving the fields therefore preserves
the source interval apart from that native perturbation and floating-point
rounding. No normalization, compensating width, or global sphere change is
introduced.

## Executable and shader evidence

All addresses below are actual VAs with PE image base `0x10000`. Some recovered
function metadata used `0x400000`; adding its RVA to the actual image base is
necessary. Original executable SHA256:
`83d6cd0728e3ab71acfa8507b9458c23373ac25ebace9ca4bf1539e53f9f2396`.

- Original `object_environment_probe::insert`, `0x78a6b0`: `0x78a708` loads
  authored radius from object+`0x26c`; `0x78a71f` subtracts width at +`0x278`.
- Original deferred `stage_ambient_lighting::execute`, `0x625ca0`:
  `0x6266df/0x6266ed` read radius and inner radius from probe+`0x1a0/+0x1a4`;
  `0x626829` subtracts the `.05` constant at `0x93e45c`. Calls at
  `0x6268f2/0x62690b` upload radius and adjusted inner radius.
- All eight unique archived **sphere** pixel shaders contain the same seven
  instruction radial block: `mad, add, div, mul_sat, mad, mul, mad`. The first
  instruction computes `.98*light_range-inner_radius`. Coverage includes
  quality 0/3 and both clip/shadow switches; other branches can subsequently
  affect alpha. The authored global sphere has both switches disabled.
- High-quality unshadowed original SHA256:
  `9dd5d3bf4807c947420a67a839d19ea22e9faa09b4fe941f27ccd56fad76664c`; its resource key is in `probe-extracted.json`.
  Reflection locates inner radius at `cb0[19].w` (byte316), and light range at
  `cb3[1].x` (byte16). Its falloff block begins at SHDR word690.
- Native constructor at `0x1c2f70` creates the small perturbation; constants
  `0xa2d9ac=2^-20` and `0x921948=.1`. `set_properties` at `0x1c36f0` adds it to
  both radius and inner radius.
- Native deferred upload: `0x180b96/0x180baa` read probe+`0x320/+0x324`;
  `0x180ef5` multiplies radius by `.98` from `0xa2dc84`; `0x180f11` subtracts
  `.05` from inner radius. Constant uploads are at `0x18141c/0x1814c1`.
- Frozen native sphere PS SHA256
  `7d04e85246882cbdd517e7cb757602bcf6a9b21de3d938957fb46eee4237b661` uses the
  uploaded outer directly, with the same saturated cubic. Reflection locates
  inner at `cb0[11].w` and outer at `cb2[1].x`.

The existing native Chemical capture independently validates the upload. Its
matched sphere has authored radius `340.901001` and earlier adapter width
`7.818019867`, captured outer `334.119537354`, and inner `333.070281982`.
The two boundaries imply perturbations `.037295810` and `.037300873`, agreeing
within float rounding. **That earlier width is a 0.11a adapter value, not the
0.11e donor value or a recommendation for the new candidate.**

## Reproduction and limits

Small frozen evidence bundle (about 0.6 MB):
`/tmp/resource-porter-chemical-011/.work/probe-sphere-011e`.
It contains source/native DXBC, reflection, decoded instruction listings,
CPU disassembly, matched authored probe subsets with full-project hashes,
captured native constant buffers, audit output, and SHA256 manifest.

From this worktree:

```sh
python3 tools/chemical-plant/audit_probe_sphere_011e.py \
  --evidence .work/probe-sphere-011e \
  --native-capture .work/probe-sphere-011e/native.probe-draw \
  --native-project .work/probe-sphere-011e/native-probe.json \
  --source-project .work/probe-sphere-011e/source-probe.json \
  --output .work/probe-sphere-011e/verified-report.json
```

The audit checks all eight source blocks from raw SHDR tokens, the pinned native
shader block, and consistency of the captured boundaries. CPU upload semantics
also require the preserved disassembly review above. The partial token decoder
marks unsupported operands as raw; the audited radial blocks fully decode.

No new original client launch was needed. No 0.11e runtime capture or complete
lighting match is claimed. Original probe-index rejection, normal clipping,
shadow branches, diffuse/specular encodings, and ordering remain separate
contracts. In particular, preserving this inverted interval must not be used
as grounds to remove the native probe perturbation or change unrelated spheres.
