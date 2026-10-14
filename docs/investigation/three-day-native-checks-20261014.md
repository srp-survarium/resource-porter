# Focused native checks for the combined day candidate

The combined candidate's Radar 0.11e map195 passed native settling and support
checks for all 21 battery candidates. Chemical Plant 0.11e map194 allowed an actual character to
cross the formerly blocked wall opening. These are bounded checks, not a
whole-map collision or pickup certification.

## Radar: all 21 candidates checked

The reversible diagnostic selected original indices3,19,14,17,18, preserved
their transforms, assigned dense IDs0–4, and set the mode count to5.
Strict checkpoint validation matched every initial native body centre, then
found zero motion between late checkpoints at5.45 and10.45seconds. The largest
settling displacement was0.0834m; no falling fallback was enabled.

Two saved views per candidate show batteries above their support surfaces.
The exact triangle/box audit finds no burial for14,17,18. The cylindrical barrel
supports at3/19 remain conservatively boxed by the CPU helper; native stable
positions and screenshots resolve their support concern without pretending
those conservative boxes are an exact cylinder test. No pickup interaction
was attempted. Authored positions remain unchanged.

The remaining 16 candidates subsequently passed two strict native batches:
0,1,2,4,5,6,7,8,9,10, then 11,12,13,15,16,20. Both batches had zero late
motion. Their largest settling distances were 0.09101m and 0.024815m.
Every settled centre was outside exact boxes, with no deep intersection or
upward mesh surface above it. No collision resources were missing.
Only 0,2,8 had nearby curved primitives that the helper does not test exactly;
two close views of each show batteries above their supporting furniture.
All 21 authored locations remain unchanged. This checks support and burial;
walking to every battery and pickup interactions were not tested.

The two plans and full evidence are under
`/tmp/three-day-native-checks/remaining-run` and `remaining-battery-plans`.
The entire combined database was hashed before and after both batches:
`241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`.
The exact match verifies complete restoration, including entry CRCs. The final
supervisor, client and server PIDs were absent before the water-worker handoff.

## Chemical: actual-character wall-gap replay

Only44 diagnostic player spawn transforms were overridden to
[-63.5,7.45,27.9907], yaw−π/2. The observer delegated rendering to a hash-identical
copy of the packaged renderer. A mode0 request reset observation to the live
character camera. Its forward vector was approximately[0.99993,−0.00333,−0.01158].
The exact request is stored in the accompanying result JSON.

The owned-window input helper held W for503ms (500ms requested). Camera X moved
from−63.5463 to−61.6230 while Z stayed27.94189. Native checkpoints independently
corroborate movement: the length-prefixed slot0 player state contains one
plausible position triple at byte591 of this675-byte state. At70,600ms it is
[-63.5,8.25547886,−27.99069977]; at75,600ms it is
[-61.57664490,8.24722004,−27.99070358], unchanged through105,650ms.
The1.92335510m X delta and−0.00825882m Y delta match the live camera exactly.
The Z sign and height fit the native Bullet controller centre. This field's
semantic is inferred from the known spawn and matching movement; the complete
controller serializer ABI was not independently decoded.

Both native state and live camera cross the old wall's X interval
[-62.643,−62.401] near the intended Z27.99 opening. The bounded movement-callback
observer recorded no contact candidates. That empty log adds no claim about
other collision areas, step support, or the separate boiler cylinders.

## Evidence and restoration

Small checkpoint fixtures, results and hashes are committed in
`tools/day-map-objectives/native-checks-20261014`. Full packets, screenshots,
geometry reports, exact plans and decoder outputs are in
`/tmp/three-day-native-checks`; its `index.html` browses the ten Radar images.

After each run, the actual supervisor, client and server processes exited and
the launcher completed its owned-prefix wineserver stop/wait. The alternate
collision-observer supervisor required a start-ticks and command-line guarded
SIGTERM because the normal launcher correctly rejected its different script
identity. Only after process exit were the diagnostic entries reverted.
Both projects and the Radar mode subsequently passed baseline hash inspection.
No packaged preset, material, authored pickup or collision geometry changed.
