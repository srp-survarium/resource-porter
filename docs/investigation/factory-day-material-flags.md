# Factory day material feature loss

The 2026-10-13 playtest screenshots `20-38-50` and `20-39-24` show bright
pump motors and metallic skeleton/cloth remains. The packed Factory day
namespace is `ported/v010w`.

The source `factory_pump.model` binds its motor to `tiled/metal/metal_radio`
and its tank to `tiled/metal/metal_green`. Both LODs of `skeleton_lying1.model`
bind `characters/human/remains/skeleton` and `rag`. These bindings identify
three material repairs; the green tank material is not included.

| Material | Switch | Packed value | Source value |
| --- | --- | --- | --- |
| skeleton, rag | `use_troughness` | true | false |
| skeleton, rag | `use_tspecular_intensity` | false | true |
| metal_radio | `use_tfresnel` | false | true |

The converter retained native template switches while copying old scalar
values and textures. For the remains, this activates a roughness range
0.1–0.2 instead of the authored constant 0.5 and drops the intensity mask.
For the motor, it replaces `0.4 + 0.6 * texture` Fresnel with constant 1.
No global lighting or arbitrary roughness adjustment is needed to test
these concrete differences.

The original shader's intensity mask feeds a packed five-bit field called
translucency; `resolve_lighting.ps` also uses that field as specular amount
for these non-translucent materials. Native G-buffer amount is separate.
Restoring the texture switch does not establish bit-exact old quantization
or BRDF parity. The native roughness/Fresnel channel is green rather than
the original red, but these three grayscale textures have mean R/G
differences below 0.0004; that is not the large reported defect.

Original and packed DDS bytes match exactly. Skeleton/rag textures retain
their sRGB flag; metal textures retain linear sampling. Their means are
approximately 0.0108/0.0080 after sRGB decoding for remains intensity, and
0.0852 for the linear metal-radio map. No texture rewrite is proposed.

`prepare_factory_day_material_flags.py` extracts only three small material
entries, verifies all five authored booleans against the original database,
and produces a fixed-size reversible plan. It never writes the archive.
Use `patch_archive_entries.py ARCHIVE PLAN inspect|apply|revert` while the
owned client is stopped. The patcher checks material hashes, positions and
stored CRCs before writes. Other namespaces and materials are untouched.

Runtime validation is pending. Three baseline views reproduce the pump,
the outdoor remains and a second skeleton in a cellar. They were captured
with the frozen packaged renderer and no material changes. A bounded
same-camera comparison must also verify that the native material
permutations load before this plan is included in a release.
