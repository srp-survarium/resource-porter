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
cube instances. It needs one visible sample of each active material; the
script prepares downward views above terrain2 and the higher cube.

CPU evidence and proposed poses: `/tmp/three-day-map-water-review/`.
Native verification is pending the shared GPU lease. No original client was
relaunched and no candidate resource or preset was modified by this audit.
