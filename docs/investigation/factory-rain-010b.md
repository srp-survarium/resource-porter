# Factory rain: live 0.26 path versus original 0.10b

The five-view `026g/pipeline-rain-lamp-contract` batch observes one particle
draw per view, with 30,720 indices and no cone or resolve draw. Every rain
report covers one Present serial. `verify_rain_capture.py` verifies shader
files, geometry bounds and before/output buffers for all five views.
The frozen reproduction record is `reproduction/rain-lamp-observer-20261013`.
This identifies the current path; it does not restore original rain.

Original `stage_rain.cpp` constructs `sphere_geometry(24,3)`: 100 vertices,
432 indices, triangle lists, a 24-byte POSITION4/TEXCOORD2 vertex layout.
The settings call these layers cones. Each layer receives a scaled/rotated
world matrix, radius, density, scrolling speed, UV scale and rain-shadow
projection. Camera movement also changes scrolling offsets. A shader-only
replacement on native particle geometry cannot reproduce this behavior.

The native rain stage hardcodes technique 1 and renders particle geometry.
Its effect still contains the older technique 0, but no corresponding console
selector was found. Switching that index alone would retain incompatible
geometry and constants. The School worker records the instruction evidence
in `docs/survarium-021-school-rain-reference.md` in its worktree.

Archived cone VS instructions and input/output signatures match between
0.10b and 0.26. The original PS is 3,620 bytes with DXBC identity
`9004247427fb3a6a9e5dd19aea0c46d5`. Its PS constants need explicit remapping:

| Value | Original byte offset | Native byte offset |
|---|---:|---:|
| b0 UV scale | 16 | 0 |
| b0 radius / density | 24 / 28 | 8 / 12 |
| b0 speed | 32 | 16 |
| b0 shadow matrix | 48 | 32 |
| b1 scene time | 284 | 400 |
| b1 screen size | 288 | 224 |

Both cone shaders bind depth/rain/frame/shadow textures at t0–t3. The old
shader additionally requires diffuse lighting at t4, with material sampler s3
and shadow comparison sampler s4. It uses one shadow comparison and lighting
in its opacity calculation. Native cone shading uses four biased comparisons
and different color/opacity equations. Restore the old lighting dependency;
an arbitrary stale t4 binding is not sufficient.

The archived `engine/scroll_rain_0` texture also differs: both are 512² DXT5,
but 13,428 DDS bytes differ in the pixel payload. Original and native copies
and hashes are saved in
`reproduction/factory-rain-source-contract-20261013/textures`, alongside the
source renderer and exact shader/binding audit in that checkpoint.
Use the original texture when reconstructing the original layered renderer.

Next validation needs one targeted original capture batch with the new rain
observer to record live layer matrices/constants and the outdoor lamp view
missing from the frozen reference set. Reuse existing references elsewhere.
