# Radar 0.20e missing stair sunlight

The frozen `hangar-stairs` pose is at [-8, 7.6, -13], forward [0, .12, -1],
FOV 65, 1280 by 720. The historical capture name is misleading: this location
lies inside `level/03/antenna/antenna_base.model`, not the separate hangar.
Original sunlight reaches the stair wall; both accepted ambient and cloud-off
candidates still omit that patch. Cloud phase alone does not explain it.

`audit_stair_casters.py SOURCE_TREE CONVERTED_OVERLAY OUTPUT_JSON` compares the
antenna base and the two overhead tree/leaf models. All three model settings
match after namespace relocation, including their LOD hierarchy. All 52 render
surface payloads are byte-identical to the donor. Across 34 referenced materials,
source shadow, alpha and two-sided fields are preserved. The only recorded
change is a forward bullet-decal transparency texture alias, unrelated to the
window/leaf shadow-caster flags. Target-only fields are listed for review.

This rules out changed geometry or authored shadow flags for these casters;
target-only defaults and runtime LOD, rasterizer, shader and visibility choices
still require checking.
The authored sun elevation 26.07 and azimuth 222.2 are also preserved. Source
`get_sun_direction` is at VA 0x203841; its disassembly reads environment offsets
0x3c/0x40, uses constants 1/180, pi and .5, and normalizes the result. Do not
change the authored angles to imitate the screenshot without checking the
actual native direction and shadow inputs.

The next native diagnostic should capture the stair sun-shadow inputs and
compare its visibility with the lit region in the frozen source. If the native
mask is dark, trace caster LOD/rasterization before changing lighting energy.
No shadow fix is applied yet. Audit and source disassembly are preserved at
`resource-porter/.work/radar-020e-stair-shadow-20261014/` with hashes.

## 2026-10-15 offline result

The native mask is dark because the 0.26 sun shadow blocks the wall with the
antenna slab (`stucco_ceiling_s` facing down, `roof_03_s_vc` facing up), 3–8 m
along the sun ray. Neither a different sun direction (grid over all azimuths
and elevations, every axis and sign swap) nor any face-culling mode reproduces
the original pattern. Every caster omitted from the original's sun shadow uses
a material with `g_stage.with_static_vertex_color_meshes=true`. Dropping those
materials as casters matches 227 of 288 wall pixels; most of the rest are
explained. Cascade sizes and bias, portals and the other caster filters match
between builds. The mechanism is not yet traced in code, so no adapter is
proposed. See [the calibration notes](day-map-calibration-20261015.md) and
`/tmp/radar-020e-stair-shadow/fork-analysis/FINDINGS.txt`.
