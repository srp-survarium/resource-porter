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
Merge only the explicitly approved groups into the final bundle before
packing; these candidates still require settled runtime validation.

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
