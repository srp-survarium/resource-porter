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

This rules out a conversion change to these resources; it does not prove which
LOD, rasterizer state, shader or visibility samples the running renderer uses.
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
