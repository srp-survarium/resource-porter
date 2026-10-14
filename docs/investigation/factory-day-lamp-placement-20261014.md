# Factory day lamp report, 2026-10-14

Release scope is Factory 0.10b day and Chemical Plant 0.11a day. This
investigation concerns the hanging lamps in the user's
`2026-10-13-20-39-37.png`; Chemical Plant collision and Factory materials/water
are separate tasks.

## Placement audit

The packed release project is
`projects/v010w_level_03/day_2/client_project`, read directly from
`/mnt/hdd/Projects/survarium-map-pack-20261013/resources.db`.
Comparing decoded objects against the preserved original
`.work/early-to-026/010b-projects_level_03_client_project.json` finds:

- All 121 lamp model instances preserve position, rotation and scale.
- All 12 `editor/cone` instances preserve position, rotation and scale.
- All 18 local lights preserve position, rotation, scale, type, range and
  cone angles. Their added native `offset` transforms are identity.

`tools/render-compare/audit_factory_light_placement.py SOURCE.json TARGET.json`
repeats this audit. It was also checked against injected cone displacement,
lamp rotation and light offset; each alteration was rejected. This checks
project placement, not runtime shader transforms or apparent beam edges.
No lamp/cone has been moved.

The original cone mesh's apex is at local Y -0.00947; prior geometry audit
found identical positions and packed tangent bases, with UV quantization
error below 0.00044. See the preserved
`reproduction/source-volume-post-contract-checkpoint/lamp-cone-placement/uv-audit.json`.

## Missing day compatibility path

The day preset has neither `SRP_LEGACY_VOLUME_CONE` nor
`SRP_LEGACY_VOLUME_ASSETS`. Its frozen DLL also lacks those environment
variable names. The rain preset includes both assets and a newer DLL.
Consequently the current day package still uses native 0.26 cone opacity.
This is also observed in all three saved `026g/day-final` volume pixel shaders
(factory-interior, factory-upper, officer-terrace): each is the 3,752-byte
native shader with DXBC identity `2addeb6df02aa8db8c8101786dde95e9`.

The original cone/sphere opacity adapter already exists in `legacy_volume.hlsl`
and `legacy_volume.c`. It preserves the geometry, blend state and dust motion,
but restores the original falloff textures and alpha equation. Previous rain
validation reduced above-lamp regional luma error from 0.03544 to 0.01094;
that does not establish the day result or justify moving any geometry.

The packed day material `material_instances/ported/v010w/fx/cone_volume_material.material`
retains attenuation scale 13 and transparency 1, as required by that adapter.
The target cone shader does not use its attenuation constant; captured bytes
at that unused constant are not evidence of an authored setting.

## Pending controlled comparison

Use three small screenshot batches with the same two hangar poses:
packaged day DLL; current DLL without the cone override; current DLL with
original cone shader and falloff assets. This separates any general newer-hook
behavior from the cone change. Compare lamp/beam origin and shape against the
existing original references where poses match. A new original capture is
only useful if the reported lamps lack matching stored views.

The current isolated hook builds with `-Wall -Wextra -Werror`. No release
preset or packed database has been modified, and no visual fix is claimed yet.

The candidate package builder imports only the two cone assets into the day
recipe and selects the newer frozen Factory hook. An isolated execution of
that recipe loop confirms it does not import rain environment settings and
does not alter the evening recipe. Runtime validation is still pending.
