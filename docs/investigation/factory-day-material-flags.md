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

Directly restoring all five booleans was rejected by the native shader
archive during the controlled runtime test. The missing alpha-test,
intensity-mask, constant-roughness permutation is:

```
_3___1101_00100_00____1_____0__0_00000____00000____000__________0__
```

Its otherwise-identical textured-roughness permutation is shipped:
`_3___1101_00101_00____1_____0__0_00000____00000____000__________0__`,
DXBC identity `e900e3e72a32c961f56a73ed8393a6c7`. It binds the base, normal,
roughness and specular-intensity maps. The revised plan retains the native
roughness switch and sets both range endpoints to the source constant 0.5.
Thus `min + (max-min) * texture` is exactly the authored constant, while
using the supported shader. The failed plan was reverted before handoff.

`prepare_factory_day_material_flags.py` extracts only three small material
entries, verifies the original switches and constant, and produces a
fixed-size reversible plan: three switches and four range endpoints.
It also verifies the exact shipped 4,120-byte native remains shader at
both lighting-quality keys. It never writes the archive.
Use `patch_archive_entries.py ARCHIVE PLAN inspect|apply|revert` while the
owned client is stopped. The patcher checks material hashes, positions and
stored CRCs before writes. Other namespaces and materials are untouched.

The revised plan loaded and captured all three material views successfully;
three independent lamp-emission views shared the same run. There were no
missing shader or patched-material errors. Both plans were reverted and the
owned client stopped afterward. The package is not left patched.

Both material arms use `SRP_CAPTURE_BUFFERS=1`, screenshot-only control mode
2, UI off, the same frozen day hook/preset, and no raw dumps. Recorded
override counters are positive. Earlier activation-off packaged captures
are excluded. The original day client supplied matching pump/outdoor-remains
references during the lights investigation.

The mirror-like remains highlights disappear, but the remains become much
darker than the source image. In fixed rectangles selected before the
candidate, pump/source RGB MAE improves 0.0731→0.0569; outdoor remains MAE
worsens 0.0689→0.0717. Cellar remains mean luma changes 0.0512→0.0377,
while its floor control changes 0.0369→0.0361. This establishes a working
material contract correction, not complete appearance parity.

The one-second material poses also captured incomplete texture streaming:
control foliage, trunks and crate textures show different mip detail.
Do not promote the candidate as an exact visual match until settled-texture
views separate this from the remaining diffuse-lighting difference. No
global gain, roughness fitting or texture rewrite was used to hide it.

The remains have a specific streaming compatibility problem in the pinned
0.26g executable (`ac33d993…153`), beyond a short warmup. Both source DDSs
are 2048² BC2/DXT3 with ten mips and sRGB option 1. Native logs allocate
64² versions outside the texture pool. That warning alone is harmless:
`0x3bd53c` creates a standalone texture and `0x3bd706` uploads its mips.
However, pool initialization at `0x11b090` creates only BC1_TYPELESS (70)
and BC3_TYPELESS (76) entries. `scene::gather_streamable_textures` calls
`texture_storage::supported` at `0x1550b3` and skips streaming registration
when that format/dimension key is absent. BC2 therefore cannot request
higher mips through this path with the pool enabled.

`prepare_factory_remains_bc3.py` prepares a separate, unapplied two-texture
candidate. It guards the exact original DDS hashes, preserves every RGB
compression block, dimensions, mip count and sRGB byte, and fits only BC2
alpha into BC3 palettes. The payload length is unchanged. Independent Pillow
decoding confirms identical RGB and zero texel crossings at the authored
alpha reference 0.25 (and 0.5) across all ten mips. Full-resolution alpha
MAE is 0.000117/0.014664 on a 0–255 scale for skeleton/rag; maximum errors
are 10/17. Bilinear alpha-edge coverage can still differ.
This is a format compatibility experiment, not lossless alpha conversion.
Runtime validation must confirm actual bound SRV dimensions and settled
appearance before promotion. Material and lighting candidates stay separate.

A concrete remaining lighting mismatch explains why correct intensity masks
can expose dark remains: their authored Fresnel is 1. Native day composition
multiplies sun diffuse by `1-F`, producing zero. Original `sun.ps:251`
instead uses `N.L * (F > 0.99 ? 1 : 1-F)`, retaining full diffuse at this
sentinel. Its Fresnel uses the seven-bit material value and view/halfway
angle. This is distinct from the source probe's roughness-dependent rule.
The diffuse DDS files also match the original byte-for-byte, including
sRGB options, ruling out an accidental albedo rewrite.

`legacy_composition_sun_sentinel.hlsl` is an opt-in, uncompiled diagnostic
sibling of the frozen day composition. Only the source sentinel branch is
restored; other current sun, specular, ambient and encoded-addition terms
remain unchanged. It has not been applied to the package. Broad trigger
auditing must account for any other materials whose conversion lost a
Fresnel map; target F0=1 alone does not prove authored F0=1.

Reproduction inputs are `poses-factory-day-materials.json`, the preparation
script, `patch_archive_entries.py`, and `audit_factory_day_material_images.py`.
The latter checks matching camera requests and reports fixed material/control
rectangles; it does not claim identical source jitter or animated shadows.
