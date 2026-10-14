# Radar 0.11e remains and motor audit

The source level03 is Vostok Radar Station. Earlier investigation paths and
script names called it Factory; those stable artifact paths are retained.

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
`vostok-port compose`. Its stock composer gives the first bundle precedence,
and a two-file texture bundle was verified through that CLI. The release
composer instead rejects conflicting files; use its explicit guarded `--patch`
mechanism with expected original SHA256, never rely on bundle order there.

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

### Local diffuse exception candidate (not promoted)

The source 010b and 011e point/spot shaders use `F > .99 ? 1 : 1-F`
for diffuse. Native 026 has separate Fresnel terms: `N.L` for diffuse,
`V.H` for specular. Testing the former against .99 would not reproduce the
source sentinel.

`prepare_local_fresnel_one.py` therefore prepares only the exact `F0 == 1`
subset: source Fresnel is identically 1 at every angle for the two authored
remains materials. Twelve pinned native point/spot bytecodes receive two
instructions and one temporary register. The first remembers whether the
sampled material F0 equals 1; the second selects diffuse multiplier 1 at the
existing `1-F` gate. Every original instruction, resource binding, input/output
signature, shadow operation and specular computation is preserved. All F0<1
pixels retain native behavior. This is intentionally not a complete old BRDF
conversion, and cannot recover source material values lost during packing.

The optional hook loads these exact generated identities from
`SRP_LOCAL_FRESNEL_ONE_DIR`; default behavior is unchanged. It rejects a
simultaneous private encoded local-light replay and writes bounded per-pose
metadata for shader variants actually drawn. The generated manifest records
original/replacement SHA256, insertion offsets and inverse structural checks.
Candidate assets are currently under
`/tmp/daylight-material-review/local-sentinel/candidate`. Material and sun
changes remain unpromoted.


### Radar011e native A/B: local exception

The two screenshot-only arms used ID195 in the new three-map candidate,
BC3 textures and the same supported materials/sun sentinel. The hook was
rebuilt on release commit `f14c580` plus `eb660f7`, retaining the root sun,
rain and composition paths. Only `SRP_LOCAL_FRESNEL_ONE_DIR` differed. Both
settled poses actually bound the patched shadowed-spot identity `33204e90…`.
All driver shader creations succeeded; offline generation also passed
checksum, deterministic regeneration, inverse-instruction and tamper checks.

| Fixed region | Pixels | Before mean luma | After mean luma |
|---|---:|---:|---:|
| Cellar remains | 67,850 | .034276 | .037377 |
| Cellar floor control | 20,800 | .034157 | .034100 |
| Outdoor remains | 69,105 | .108544 | .109259 |
| Outdoor ground control | 21,000 | .090927 | .090618 |

These are fixed screenshot rectangles, not exact material masks; they include
background and phase/streaming variation. The cellar recovers some detail but
remains dark. The outdoor source010 image has materially different foliage
shadow coverage from011e, so cross-donor MAE is not a release criterion. The
upcoming single original011e reference batch includes both exact poses; no
brightness adjustment or whole-BRDF parity claim is justified meanwhile.

The candidate is an exact subset of the source sentinel, not its full
`F>.99` behavior at grazing angles when F0<1. The native lower-F0 BRDF and
specular remain unchanged. The two-material plan, generated shaders, loaded
hook, guarded combined recipe, screenshots and restoration report are
preserved under `.work/factory-011e-materials-20261014/local-diffuse/`. Both
clients/server stopped, and the package hook, composition and two materials
were restored exactly. The independently accepted BC3 textures remain in
the candidate package.
