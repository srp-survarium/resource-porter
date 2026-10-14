# Original 0.11e probe attenuation

The current Chemical 194 preset uses the exact earlier 011a last-mip shaders:
sphere SHA256 `c07da5b4b520f2da8f8e53bb9f0eec54a47b37b8678e004a84c1e827eefaeb06`,
box `01bf69a93b2d366d6ef5b24d57e512d34373f18907b3deec456718acfa980edc`.
Those shaders restore exponential 010b/011a falloff. Original 011e uses cubic
falloff, so inheriting the old attenuation is a donor-contract mismatch.

Evidence is frozen under `.work/probe-sphere-011e`. The existing sphere audit
checks eight distinct sphere permutations. `audit_probe_box_011e.py` now checks
the cubic block in all eight distinct box permutations and pins their hashes.
For quality 3, sphere `030000-9dd5d3bf.tokens.txt` words 690–744 and box
`030001-a0298850.tokens.txt` words 451–567 establish:

```text
sphere: t = saturate((distance - inner_radius) / (.98*radius - inner_radius))
        alpha = 1 - t*t*(3 - 2*t)
box:    q = world_to_outer_probe * world_position
        t = saturate((abs(q)*outer_sides - inner_sides) / (outer_sides-inner_sides))
        alpha = product(max(1 - t*t*(3 - 2*t), 0))
```

The original box object builds inner dimensions by subtracting authored
`probe_side_width` from each transform-axis length at VA `0x78a877–0x78a8d3`.
Its renderer still computes the sort radius as `max(axis_lengths)*.75` at
`0x703b7a`; the executable constant at `0x981bec` is exactly .75. This supports
retaining the source-derived sort key rather than reverting to native radius.

The original box PS reads outer/inner side sizes at b0 bytes 288/304 and its
world-to-outer matrix at byte 144. The existing native adapter ABI provides the
same quantities at bytes 272/288 and 128. Sphere inner radius is native b0 byte
188; native b2 byte 16 already contains the .98-contracted outer radius. The
native engine's previously documented sub-.1 radius perturbation remains; this
candidate does not claim byte-identical original bounds.

The optional `legacy_probe_011e_falloff{,_box}.hlsl` entries change only
attenuation in the inherited last-mip probe experiment. They keep donor RGBM
decoding, eight-sample gather, Fresnel, existing native specular and current AO
handling unchanged. Turning off `CONFIG_SOURCE_FALLOFF` altogether would also
change diffuse Fresnel and is not an isolated attenuation experiment. No archive
or shared preset has been changed.

Chemical has fifteen boxes and one sphere. Its sphere's authored radius 340.901
and width .5 produce inner 340.351 and outer 334.083. Preserve this inverted
interval exactly. At the twelve captured camera locations (35–126 units from
the sphere center), source alpha is zero whereas inherited 011a alpha is nearly
one. `.work/reference-011e/probe-falloff/camera-sphere-alpha.json` records those
camera-point values; they are not measurements at rendered surface positions.
The candidate can therefore darken some regions while restoring local-box
coverage. It must be assessed separately from the large postprocessing mismatch.

Other source contracts remain open. The sphere shader has a probe-index-mask
early exclusion that this inherited adapter lacks. Source AO at t4 is multiplied
by 2.5 before diffuse modulation; its producer must be traced before mapping it
to native t3. Specular uses the source BRDF/cube equations rather than native
specular. Falloff alone is not full probe parity or proof that all darkness is
fixed. The 29 frozen source frames remain authoritative; no additional original
session is needed to evaluate this candidate.

## Runtime result: retain as a diagnostic

All four candidate shaders (two probes and root's two map-specific post tails)
compiled with FXC `ps_4_0 /O3 /WX`. The Chemical comparison ran post-only,
falloff-only and combined arms, each at the same three frozen cameras after a
separate warmup. The first view of each arm includes stage readbacks; all six
active post/probe shader identities match the intended compiled files byte for
byte. Actual camera values also match source exactly. The managed private
client/server stopped after every arm, and shared database/presets stayed intact.

| View | Baseline RGB MAE | Post only | Falloff only | Combined |
| --- | ---: | ---: | ---: | ---: |
| East hall 090 | .1133 | .0673 | .1163 | .0701 |
| West factory 000 | .1139 | .0744 | .1191 | .0809 |
| South yard 000 | .1364 | .0885 | .1584 | .1059 |

Post-only improves all three; isolated falloff worsens all three. Keep the
falloff change optional pending the missing original probe/ambient inputs.
Correcting one equation within the inherited approximation does not establish
better final-image fidelity. Artifact root `.work/reference-011e/ab` contains
the three arms, `audit.json` and `contact.png` (source, baseline, post, probe,
combined columns). `ab-sha256.json` freezes all 767 capture/evidence files.

The donor gather is independently verified: quality-3 box DXBC word 1007 bounds
the loop at eight, word 1095 divides the sum by eight, and word 980 subtracts
two from `num_mips` for sampling. `011e-calc-mips.asm` shows VA `0x703710`
calculating the full chain from authored resolution, called at `0x7037f5`.
For the global Chemical cube, authored resolution 128 gives count 8; its actual
DDS has six physical levels ending at 4×4. Requested LOD 6 therefore clamps to
the last level, equivalent to mip 2 of the converted 16×16 tail. Radar's inherited
010b baseline differs in gather/mip configuration; reusing this candidate there
must be labelled a source-011e-probe arm, not attenuation-only.

The candidate compilation uses the existing last-mip implementation already
integrated on root (`64fe545`); its exact source snapshot is preserved with the
build. The falloff commit leaves this worktree's older last-mip working change
separate rather than restaging it.
