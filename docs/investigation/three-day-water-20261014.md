# Water contracts for the three day-map candidate

The release donor set changed to Radar 0.11e (ID 195), Chemical Plant 0.11e
(ID 194), and Radar 0.20e (ID 196). The old Factory/Radar 0.10b screenshots
prove the inherited-foam defect, but do not validate these new donor outputs.

`tools/render-compare/audit_day_map_water.py` reads the packed candidate and
original source databases without rebuilding them. It verifies all four
water materials and records the project instances and water-surface bounds.

| Source material | Original foam | Packed foam | Result |
| --- | --- | --- | --- |
| 0.11e `water/water` | absent | 0 | Added target layer disabled |
| 0.11e `water/water_green` | absent | 0 | Added target layer disabled |
| 0.11e `water/water_green_shit` | absent | 0 | Added target layer disabled |
| 0.20e `water/water_l03_green` | explicitly 0 | 0 | Authored value preserved |

All 15 original 0.11e simple-water pixel-shader permutations retain reflection
metadata and expose `packed_water_parameters_0`, but none has `t_foam` or
`foam_intensity`. Their hashes and paths are recorded in the audit. This
confirms that the absent 0.11e field represents an absent foam layer.

Both Radar versions retain the previously sampled water surface bounds
`[10.74,0.427,-53.05]..[40.6,0.496,-25]`, with identity placement. The confirmed
normal and oblique camera poses therefore remain applicable. Chemical Plant
uses `water` on three terrain surfaces and `water_green_shit` on two scaled
cube instances. Native review selected terrain7 triangle 25, where water is
0.114 m above the ground, and the raised green pool's western interior.

## Native verification

Six final screenshot-only views passed with UI hidden and the existing candidate
presets: normal and oblique views for each Radar version, plus normal views of
both active Chemical water materials. All normal views are within 0.6 degrees
of perpendicular. None shows the old dense white foam pattern. Radar 0.11e's
water is dark and muddy; 0.20e's is orange/brown with visible ripple/reflection
detail. Chemical's shallow puddle is brown and its raised pool is green. These
checks establish the reported artifact's absence, not exact source lighting or
water-color parity. No original client was relaunched.

Initial Chemical samples were obstructed by vegetation and structures; they
are retained under `chemical-011e-day-obscured*` and excluded from final coverage.
The final poses in the audit script avoid those obstructions. The helper
`find_exposed_water.py` in the evidence folder records terrain water depth and
explains the replacement location.

Evidence and capture commands are in `/tmp/three-day-map-water-review/`, including
PNGs, per-pose camera status, runtime tokens, preset/hook hashes, source shader
contracts, and `shutdown.json`. No resources or presets were modified. After
stopping and waiting for the owned Wine prefix, actual game/server/supervisor
processes were absent. The full database SHA256 remained
`241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`.
The GPU was handed to the Chemical lighting worker.

The compact evidence is also preserved at
`resource-porter/.work/three-day-water-20261014/`: 47 hash-verified files,
11.9 MB, with `checkpoint-hashes.json`. Root review of the final perpendicular
views agrees that the former dense foam pattern is absent; remaining darkness
must be revisited with the final lighting combination.
