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

## Launcher activation defect and verification

The package launcher set `SRP_CAPTURE_BUFFERS=0`. Despite that variable's
name, `install_shader_capture` returns before loading **any** compatibility
shader unless it is `1`. `install_post` has the same gate. The initial
current-hook / source-volume comparison therefore did not execute either
recipe; it produced no override report. Those images are excluded as a test
of the cone adapter.

A separate run changed only the launcher gate to `1`, preserving the frozen
day DLL and preset. Three screenshot-only requests (mode 2) completed without
any raw `.bin` capture. Reports prove successful replacements: LUT 1, fog 1,
sky 1, cloud 4, post 3, probe 3, composition 1 and ambient decode 1; grain
replacement count 4, all HRESULT 0. Shader activation restores the previously
validated weather appearance. The root worker owns the permanent launcher fix.
Actual character mode and screenshot mode 2 do not request buffer readback;
rich mode 1 remains an explicit diagnostic action.

Artifacts are in this worktree's `.work/day-lamps-20261014/active`, with the
exact recipe, pre-test DLL, screenshot metadata and override reports. The
adjacent manifest hashes the bounded investigation. The target wrapper
restored package launcher, day preset and DLL after every run; it never
changed the packed database.

## Selective lamp check still pending

Three useful views were retained in `poses-factory-day-lamps.json`:

- East tank hangar: the native beam visibly begins above its fixture.
- West office: the native beam begins below the fixture, a placement control.
- South exterior: detached outdoor cone, a second setting and view angle.

An initial interior pose was wall-occluded and discarded. The exact user
screenshot camera has not been recovered. In the one new original-client
session, all three retained poses were captured at 1280×720 and saved under
`.work/render-compare/day-evening/010b/lamp-reference-20261014/matched`.
The same live reference session supplied two requested material views; no
additional original session was launched. A first incorrectly sized image
is outside `matched` and excluded.

Original references show subtler cones in both interior views, including the
apparent placement control. This means visual acceptability in the native
renderer is not proof of source fidelity. However, the globally enabled cone
adapter has **not** yet been tested after correcting shader activation; the
candidate must still preserve good controls before acceptance.

The proposed day-only recipe change imports only the two cone assets and the
newer compatible hook. Its isolated recipe-loop check excludes rain settings
and leaves evening unchanged. It remains unapplied and is not a released fix.

## Day-only bulb emission omission

The day lamp material still disables `use_emissive` and `use_emissive_map`.
Both are true in the original material and in the already repaired rain
namespace. The sampled `green_lamp_with_wire` model binds its `lamp_lights_s`
surface to this material. This explains the dark bulb in day captures without
requiring any transform change.

`prepare_factory_day_emission.py` prepares a reversible, unapplied two-byte
patch for `material_instances/ported/v010w/props/light/lamp_greenhouse_clone.material`.
It verifies the two source flags, compares all day/rain fields, and checks the
packed native shader archive. Once emission flags agree, remaining differences
are only the namespace and three runtime scalar values (Fresnel minimum/value
and roughness minimum); all selectors agree with the working rain material.
The source's `effect_gstage_default_materials.cpp:382–407` puts those scalars
in a constant buffer; lines 493–498 select the separate emission pass.

Four previously captured rain emission draws use the same 2,584-byte DXBC
`a2e26698335ab986a7241bcc68291a10` and `[5,5,5,1]` emission color. The combined
database contains that exact shader under two `gbuffer_emissive_pass.ps`
permutations with textured emission selector 2. The preparation script rejects
an archive lacking this observed shader. This supports permutation availability;
a runtime day comparison is still required.

The plan at `.work/day-lamps-20261014/emission-patch/patch.json` remains baseline
by CRC inspection. Only bytes 10152 and 12456 change from 0 to 1. No map geometry,
light positions, cone opacity, texture or scalar value changes. The material
worker will combine the pending lamp and material screenshot checks after the
battery gallery returns the runtime lease.
