# Original unmasked vertex blending

The 0.21 moss/rock transition was sharper after conversion because the native
material template added a tiled mask and strength 2. This is material ABI
compatibility, not a per-map colour or lighting adjustment.

Original `gbuffer_pass.ps` diffuse and normal mixing is
`lerp(base, blended, 1 - vertex_colour.x)`. Native technique 1 uses base weight
`saturate(pow(abs(vertex_colour.x), falloff * (1 - mask.r)^2) * strength)`.
For original authored falloff 1, a constant-black mask and strength 1 make the
native base weight the original vertex value. Specular/roughness remain in the
existing separate shader tail; this rule does not claim full shader equivalence.

The rule is only called at the 0.21 conversion boundary. It requires original
unmasked diffuse or normal vertex blending, native `g_stage_default` technique 1,
existing tiled-mask permutation, and equal original/converted falloff 1. It
changes only two fields absent from the original schema:
`constant_vertex_blended_strength` and `texture_vertex_blended_tiled_mask`.
Other permutations/falloffs remain unchanged. The generated linear DXT1 zero
mask is `textures/ported/v021d/compat/linear_vertex_blend_mask.{dds,options}`;
its options are the unchanged native tiled-mask options. No surface artwork,
shader, geometry, authored light or native material is edited.

`vertex_blend_021::classify` runs before dependency discovery; applying its
result runs after discovery. The original template texture remains resolvable
while scanning, and the generated mask is emitted with converted materials.
Eligible imports cannot be reused as native material aliases. The build record
lists `linear_vertex_materials`.

For already published imports, `examples/linear-vertex-021-pack.rs` applies the
same rule to the effective layered materials under
`material_instances/ported/v021d/`, matching each to its original donor path.
This namespace is shared by Rudnya, Vostok, Mamayev, School and Fort, including
weather/modes that reuse their materials. There is no map, pose, object or
position list. The release-64 input selects 171 materials. Four original
unmasked candidates are unchanged because the native permutation has no tiled
mask (disabling the mask otherwise would use base diffuse alpha):

- `tiled/metal/metal_column_05_destr_red.material`
- `tiled/stucco/stucco_bricks_vc.material`
- `tiled/trash/trash_dirt_moss_crack_vc.material`
- `tiled/wall/paper_01_palki_vc.material`

## Evidence and limits

Original shader:
`shaders/sm_4_0/gbuffer_pass.ps/_0___1100000001__00____2_____0__0_0101____00000____000__________0__`.
Native shader:
`shaders/sm_4_0/gbuffer_pass.ps/_0___1100_00001_00____2_____0__0_01111____00000____000__________0__`.

The reviewed native `effect_gstage_default_materials::compile` is VA `0x001ddd50`.
Its reads at `0x001df764` and `0x001df78f` place falloff and strength at stack
`+0x140` and `+0x144`; `0x001df98f` binds `vertex_blend_parameters`. They become
cb0[14].x / .y respectively; the tiled-mask scale is .z. The new observation is
recorded in skhid-srp's `tools/re-kit/v026g/abi_inputs.tsv`. VAs include PE base
`0x10000`; they are not RVAs.

Four permanent Mamayev poses were recaptured silently at the exact owner
settings. The first three clearly improve the moss transition. The fourth
retains moving smoke, atmosphere and foliage differences, and whole-frame
brightness is farther from the original; no light retuning is justified.
Texture-quality-only and original-alpha-mip controls did not fix these edges.
The latter did identify different lower mip alpha coverage but is not shipped.
No additional map visual sweep is claimed. Unit tests cover the blend identity
for all 256 vertex byte values, exact field isolation, idempotence and unsupported
contracts. Layered resource comparison checks the actual candidate differs only
at its 171 imported materials and the two generated mask files.

## Regeneration

From this porter checkout, with `patches` containing the published archives in
`release/patches.tsv` order followed by the accepted release-64 archive:

```sh
nix develop --command cargo run --release --example linear-vertex-021-pack -- \
  "$donor_021_db" "$new_pack_directory" "$stock_026_db" "${patches[@]}"
```

The output directory must be new. Use the generated `pack.json` hash in
skhid-srp's `release/content.tsv`; build content through `release/build-content`.
The shared rule regenerates exactly the diagnostic's operations and file bytes;
pack descriptions/serialization may differ without changing composed resources.

Private review artifacts are under
`/home/sheep/Projects/survarium/skhid-srp/private/v021d-moss-smoothness/`:
`comparisons.html`, `review/provenance.json`, `composed-isolation.json`,
`source-regeneration.json` (exact argv and input archive order), and `repro/`
(capture scripts, settings, shader audit and build/test logs). They are not
published game assets. Captures were stopped after the four requested views.
