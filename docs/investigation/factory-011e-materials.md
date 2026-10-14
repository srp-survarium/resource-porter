# Factory 0.11e remains and motor audit

The new release donor is 0.11e build 884. The candidate namespace is
`ported/v011e`; the earlier `ported/v010w` package offsets must not be reused.

The skeleton/rag diffuse DDSs and source materials are byte-identical to
0.10b. Both DDSs remain 2048² BC2, ten mips, sRGB option 1. The source uses
constant roughness 0.5, an intensity map, Fresnel 1 and alpha reference 0.25.
The initial 0.11e overlay repeats the material feature loss: textured
roughness 0.1–0.2 and no intensity mask. The native BC2 streaming limitation
therefore applies unchanged. See `factory-day-material-flags.md` for its
allocator/registration proof and BC3 alpha error bounds.

The pump motor needs no transferred 0.10b patch: the new overlay already
has `metal_radio.use_tfresnel=true`, matching 0.11e. Pump and remains model
binding settings are byte-identical between donors. The tested outdoor and
cellar remains placements, and both east pump placements, are unchanged;
their visual indices changed to 510/400 and 525/735 respectively.

`prepare_factory_011e_remains_overlay.py` prepares two independent overlay
groups without modifying its inputs: two remains materials and two DDSs.
It verifies the actual source hashes and native supported remains shader
at quality 0/3. It keeps textured roughness enabled with min=max=0.5 rather
than requesting the missing native constant-roughness permutation. Its
manifest records each input/output hash and every mip's alpha error.
It also emits standard `texture-bundle` and `material-bundle` inputs for
`vostok-port compose`. Put approved repair bundles before the base: current
composition gives the first bundle precedence. A two-file texture bundle
was composed and its output hashes verified through the actual CLI.

Lighting was checked independently of the materials. All four 0.11e point
and four spot accumulator bytecodes exactly match 0.10b. Two of eight sun
variants also match entirely. All eight sun variants have the same
24-instruction Fresnel/sentinel window as their matching 0.10b quality:
the `F > 0.99 ? 1 : 1-F` branch remains present. Quality-3 resolve is also
byte-identical. Other sun shadow variants differ, so this does not establish
whole-pass equivalence. The optional sun sentinel remains unpromoted until
an actual 0.11e/0.26 runtime comparison; it does not repair local-light
diffuse by itself.

CPU reports and the isolated candidate overlay are preserved under
`.work/factory-011e-materials-20261014/`. No 0.11e archive, preset, runtime or
source file was changed during this audit. The 0.10b evidence remains intact.

## Bounded runtime validation

The root-authorized transport test used the identical 0.10b DDSs, two fixed
five-second poses and screenshot-only captures. The default-off
`SRP_REMAINS_PREFIX` observer identifies bound textures by exact compressed
64² mip fingerprints and records actual resource/SRV descriptors; each
readback is only 4 KiB, capped at 512 candidate resources per request.
It makes no texture or render-state changes.

Baseline skeleton/rag both bind 64² BC2 resources with five mips and sRGB
view 75. The BC3-only outdoor arm binds skeleton 512²/eight mips and rag
2048²/ten mips, sRGB view 78. The subsequent supported-material arm confirms
those resolutions in both outdoor and cellar views. The first BC3-only
cellar descriptor window missed geometry and is excluded from that proof.
Rib and cloth cutouts visibly recover their authored detail. This supports
promoting the texture-only bundle; it does not calibrate the 0.11e map.

Material-only repair remains coupled to lighting. With BC3 held constant,
the original sun sentinel restores matte outdoor bones: the preselected
remains ROI's source RGB MAE changes 0.06189 (BC3 only) → 0.06882 (materials)
→ 0.04279 (materials plus sentinel); its ground control stays effectively
unchanged. Cellar bones still lose their direct local-light diffuse, which
the sun-only correction cannot restore. Do not promote the material bundle
or sentinel as a complete fix yet. These screenshot metrics include some
background, phase and unrelated streaming differences.

Both composition shaders compiled with `/WX /O3`; rebuilding the unchanged
day shader reproduced its exact frozen hash `5e59cd25…53d`. All four test
clients were stopped. Only the two test DDSs, two remains materials,
diagnostic hook and composition asset were restored to their exact prior
state. Existing water, bulb, motor and Chemical battery fixes were untouched.
