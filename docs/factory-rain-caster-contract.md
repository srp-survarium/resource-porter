# Factory rain caster selection

`SRP_DIAGNOSTIC_FACTORY_RAIN_CASTERS=1` enables a default-off diagnostic for the
pinned 0.26 client. It recomputes the original 0.10b size predicate for static
model instances. It does not reproduce the original factor cache and is not a
validated rendering preset. Runtime depth comparison is still required.

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
2.12e-5 and factor error 1.77e-7. No game tests were added. No runtime capture
has been made with this diagnostic.

Evidence bundle: `.work/chemical-plant-011a/comparison/checkpoint/factory-rain-casters`.
