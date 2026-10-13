# Factory rain caster selection

`SRP_DIAGNOSTIC_FACTORY_RAIN_CASTERS=1` enables a default-off diagnostic for the
pinned 0.26 client. It recomputes the original 0.10b size predicate for static
model instances. It does not reproduce the original factor cache and is not a
validated rendering preset. The five-view runtime comparison below improves
some depths but leaves most outdoor caster discrepancies unresolved.

## Verified contracts

The original executable's rain loop at `0x62bb9f..0x62bbc4` first checks Hi-Z
occlusion only when the option is enabled, then rejects cached screen factor
below `.004`. `is_occluded` at `0x63bd50` confirms the option gate. The saved
original reference config has both Hi-Z and shader LODs enabled.

Original `screen_factor` at `0x647730..0x64780a` and its inlined caller compute:

```
world_bounds = transform(parent_model.local_aabb, instance.transform)
factor = clamp(max(world_bounds.half_extents)
               / max(squared_distance(camera, world_bounds.center), 1e-6), 0, 1)
```

The inlined caller at `0x647c0b..0x647d9e` obtains the parent model's whole local
AABB, transforms it and stores the factor. It sums squared distance X, Z, Y.
The original `aabb::modify` accumulates transformed side vectors into min/max;
the diagnostic uses the same operation order.

Native rain at `0x17c7cd` already has the analogous conditional occlusion test.
Its size check at `0x17c7e1` instead tests parent `m_clip_by_screen_factor` at
`+0x116`, then rejects instance `+0x58` below **10.0** (constant `0xa2d75c`).
That field's meaning changed: `0x158d80..0x1591eb` projects eight bounding-box
corners, computes the maximum screen width/height, applies `pow(extent,1)` and
multiplies by 100. Skeletal vertex types multiply it by another six. Changing
its threshold to `.004` would therefore be incorrect.

The 0.21 PDB names these fields `m_dynamic_screen_factor` and
`m_clip_by_screen_factor`; its named calculation has the same projected-size
structure. Native layout was independently traced:

| Value | Native location / evidence |
| --- | --- |
| Surface's parent | instance `+0x3c` |
| Surface's world matrix pointer | instance `+0x50`, rain draw `0x17c82b` |
| Static parent vtable | `0x9f2b24`, verified through RTTI |
| Whole-AABB getter | vtable `+0x54` → `0x16e420` |
| Original model pointer | parent `+0x290` |
| Whole local AABB | original model `+0x108`, six floats |
| Main camera position | rain entry saves context `+0x54a4` at loop ESP `+0x5c` |

## Diagnostic behavior and limits

The hook replaces only the native size predicate. Conditional occlusion stays
intact. It preserves registers, flags and floating-point state, checks pinned
instructions/vtable layout, validates finite affine bounds and uses the
unchanged native predicate for unknown parent layouts. It never writes model
bounds, screen-factor caches, materials or geometry.

During an armed rich capture, `<image>.rain-casters.jsonl` records up to 2,048
unique post-occlusion candidates: instance/parent/original-model pointers,
source and native decisions, both factors, bounds, transform, saved camera,
frame counters and cumulative fallback/disagreement counts. Decisions 0/1 are
source skip/keep; 2 means native fallback. These records cover candidates that
reach the size test, not geometry or occlusion exclusions earlier in the loop.

Original factors were cached only for visible G-buffer surfaces while shader
LODs were enabled. Unseen instances could retain old/default values. The
experiment recomputes at rain time, so a remaining caster mismatch may require
restoring that update cadence. Native static-model calculation is at
`0x16f340`, calling surface calculation at `0x16f370`; scene task `0x150240`
and dispatcher `0x150270` call the model's vtable `+0x64`. These are update
sites to investigate, not proof that native cadence matches the original.

Validation so far: C builds with warnings as errors; emitted bridge/fallback
instructions inspected; four analytic and 2,000 seeded affine numerical cases
pass. Maximum world-AABB error against independently transformed corners was
2.12e-5 and factor error 1.77e-7. No game tests were added.

Evidence bundle: `.work/chemical-plant-011a/comparison/checkpoint/factory-rain-casters`.

## Five-view runtime result

The isolated observer combined root `6299478` (validated rain-shadow camera)
with `79ecae1` (caster diagnostic), producing validation commit `bab8d42`.
All five loaded, UI-free poses completed with the diagnostic active. Saved main
camera positions match the requested poses within 8e-6; all 1,293 logged
candidates matched the proven static layout, with zero native fallbacks.
World-to-shadow matrices still match original references within 5.97e-8.
Reserved client 2247 was stopped and both prior DLLs restored by SHA-256.

| Pose | Extra caster texels, camera only → diagnostic | Depth MAE, camera only → diagnostic |
| --- | --- | --- |
| Interior | 8 → 0 | .0079825 → .0072657 |
| Yard north | 4,025 → 4,025 | .1836555 → .1836496 |
| Yard south | 2,022 → 1,976 | .0930310 → .0900853 |
| Upper | 5,848 → 5,340 | .2588021 → .2358587 |
| Streetlamp | 0 → 0 | .0010927 → .0003972 |

Extra casters mean original-clear depth texels filled in the target. The south
view also gains seven missing-caster texels (2 → 9). This is useful evidence for
the size-contract difference, not grounds to adopt the diagnostic generally.

CPU replay then compared **250 original static surface payloads** with the
actual runtime archive: every position and index payload matched exactly.
Rasterizing those original triangles with captured transforms explains
3,347/4,025 remaining extra texels in north, 1,844/1,976 in south and
4,478/5,340 in upper, within .0001 normalized depth. The dominant matching
surfaces are `terrain_p_09/l3terr_` in both yard views and
`terrain_p_06/l3terr_`, `terrain_p_04/l3terr_` in upper. Their recomputed source
factors (.05267, .006291, .01356) exceed .004. Geometry conversion therefore
does not explain these terrain discrepancies.

The replay removes the lookup matrix's original -.001 Z bias before comparing
raster depth (`stage_pre_rain.cpp:201`). It considers both winding directions
and all source LOD surfaces; it proves matching geometry at those depths,
not the exact native draw/LOD owner. Remaining source cache history, visibility,
Hi-Z results and earlier candidate selection are unresolved. No occlusion
change or additional original-client launch was made.

Capture directory: `.work/render-compare/026g/pipeline-factory-rain-casters-source`.
The evidence bundle includes the launch/restoration scripts, exact observer
sources/DLL, numerical checks, per-pose caster decisions, matrix/depth/geometry
reports, source references and SHA-256 manifests for all five frozen captures.

## Additional selection-camera difference

Original rain call `0x62bb32..0x62bb57` passes context `+0x4204` to
`scene::select_models` after installing the shadow view. This is the shadow
camera: `renderer_context::set_v` updates `m_view_pos`. Native
`0x17c6e7..0x17c710` instead passes the saved main camera. The source static
model `select_lod` uses this argument for distance-based geometry LODs.
For example, unchanged `terrain_p_09` settings specify distances 80/200/300;
at the yard poses the main camera is 29.79 units from its center, while the
rain camera is over 100 units away. These choose different LOD surfaces.
The original profile uses geometry quality 1, so the quality-zero .75 scale
does not apply.

This also interacts with the per-surface cached factor. At the preceding
interior pose, terrain09's formula would yield .002076 (<.004); at the yard
poses terrain06/04 would yield .002643/.003021, before their later upper-view
values exceed .004. These are conditional calculations, **not observed cache
values**: updating requires that exact LOD surface to be selected for the
G-buffer. Frozen captures lack per-instance original visibility/cache logs,
and readiness began at another pose. The camera-argument difference is
proven; its role in each missing caster still requires a controlled selection
or cache observation. Do not replace a cache with guessed pose history.

## Independent selection diagnostic

`SRP_DIAGNOSTIC_FACTORY_RAIN_SELECTION=1` is a separate default-off hook at
`0x17c6e7`. It replaces the saved-main-camera pointer with the installed context
shadow-camera pointer (`+0x54a4`) only for this rain `select_models` call.
Guards verify the context load, replaced instructions, shadow-view setter,
setter's position write and selection call. The bridge restores all registers,
flags and floating-point state before executing the replacement LEA and the
unchanged following LEA. `<image>.rain-selection.json` records both camera
positions during rich captures; `<control>.factory-rain-selection.json`
records installation. This hook does not modify factor caches, size rejection,
geometry, occlusion or other selection calls.

CPU validation reads the three implicated terrain models' **actual runtime
archive settings** and original settings, asserting exact LOD hierarchy and
switch-distance equality. Source distance-selection boundary behavior and
empty-LOD fallback are reproduced, with geometry quality 1. Shadow camera
positions are reconstructed from the captured original lookup matrices by
undoing the -.001 texture-space bias and .1 near-plane offset. The tested
source selections are:

| Pose | Terrain09 main → shadow | Terrain06 main → shadow | Terrain04 main → shadow |
| --- | --- | --- | --- |
| Interior | 1 → 1 | 0 → 1 | 0 → 1 |
| Yard north/south | 0 → 1 | 1 → 1 | 1 → 1 |
| Upper | 1 → 1 | 0 → 1 | 0 → 1 |
| Streetlamp | 0 → 1 | 1 → 1 | 1 → 1 |

The intended five-pose A/B uses the validated shadow projection in both runs:
selection-camera correction alone, then selection correction plus the separate
recomputed-factor diagnostic. Existing camera-only and factor-only captures
remain controls. CPU LOD predictions do not prove original cached factors or
actual per-surface visibility; runtime outcomes are recorded below.


## Selection-camera runtime A/B

Both five-view batches completed on validation build `5c87c35`. Their recorded
shadow-camera arguments match the frozen original matrix-derived positions
within 2e-5, and saved main-camera positions match the requested views within
8e-6. The reserved client/server were stopped after each batch and exact prior
sun-v2 DLL bytes restored (client SHA-256
`164d337d6a1f472f591b0661af6b9e1edcd13ff88e8240f3e52168186f0efe29`).

| Pose | Shadow projection only | + Selection camera | + Selection camera and recomputed factor |
| --- | --- | --- | --- |
| Interior depth MAE | .00798250 | .00057604 | .00019889 |
| Yard north | .18365555 | .18047304 | .18047116 |
| Yard south | .09303097 | .09306666 | .08978055 |
| Upper | .25880209 | .25652793 | .23379353 |
| Streetlamp | .00109267 | .00084401 | .00021410 |

Selection-camera correction accounts for much of the interior geometry-depth
mismatch. It does not remove the major outdoor extra casters: the combined
variant still has 4,025 north, 1,976 south and 5,340 upper extra texels. Unlike
the factor-only variant, the combined south view introduces no missing-caster
texels. Source per-LOD cache/visibility behavior remains unresolved. Both
switches remain opt-in diagnostics; the ordinary playable recipe is unchanged.

Frozen captures: `.work/render-compare/026g/pipeline-factory-rain-selection`
and `pipeline-factory-rain-selection-factor`. Full four-way reports, exact
observer build, launch/restoration scripts, source/runtime LOD settings,
per-pose argument and decision logs, depth grid and capture hashes are in
`.work/chemical-plant-011a/comparison/checkpoint/factory-rain-selection`.
