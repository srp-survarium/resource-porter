# Day-map gameplay: respawns, batteries, collectors (2026-10-14)

Scope: the combined day candidate `/tmp/three-day-map-candidate`. It holds
maps 194 (Chemical Plant 0.11e), 195 (Vostok Radar 0.11e, with the pinned
0.11a server project) and 196 (Vostok Radar 0.20e with anomalies). This note
separates offline (data/binary) evidence from native runtime evidence. Runtime
claims are made only where a native checkpoint shows them.

## Candidate database history

| SHA-256 | Change |
|---|---|
| `241a6c1c…cc08d` | Starting candidate (hard-linked to four lighting harnesses; untouched, kept as `/tmp/day-map-gameplay-20261014/backup/resources-241a6c1c.db`) |
| `bc411413…5196` | Same-length release plan `release-fix-plan/plan.json`: respawn seconds (194, 195), battery body origins (196) |
| `8e72816c…5b16` | Guarded overlay `boiler-collision-patch`: 0.26 successor walker collision for the factory boiler (194) |

Each step wrote a new file and renamed it into place, so the lighting harnesses
keep their own inodes. All 13,800 composed resources were re-read from
`8e72816c…` against the composition record. Only the four intended resources
differ, and three boiler mesh files are added (`vostok-port verify`,
`verify-composition/`). `manifest.json` records the new hash, and `launch.py
verify` passes.

## Findings and fixes

### Respawn protection was 2000 seconds on 194/195 (converter fix `1e43d53`)

0.11a/0.11e author `invulnerability_time = 2000` on every respawn point.
The 0.11e loader stores that integer unchanged at respawn +0x20
(0x6F7CD1..0x6F7CE9). Native 0.26g reads the same key and multiplies it by
1000 (`imul edx,edx,0x3e8` at 0x2F462A) before storing +0x20; native maps
author 6. The port therefore gave 2,000,000 ms of spawn protection. The
converter now converts exact whole-second values (2000 → 2) and rejects
inexact or already-converted values. 0.10b omits the field (native default 6).
0.20e uses `effect_zone` invulnerability instead and its respawns take the
native default 6, which is unchanged.

### Map 196 batteries lacked the native body origin

0.20e spawners have no `spawned_item`, and the 0.20e battery options place
their collision box at the mesh origin (offset 0). The candidate's 196 bundle
predates commit `164e913`, so all 45 bodies equalled their parents. The current
converter already emits parent + rotate(rotation, [0, 0.12, 0]). Many 0.20e
parents are byte-identical to the 0.11e Radar spawns validated natively
earlier. A fresh 0.20e rebuild matches the patched project in every gameplay
group. The only differences are two environment lighting leaves owned by the
lighting work (`use_sky_clouds_lighting`, `mie_fog_sun_color_falloff`), which
were not touched here.

### Collectors

All three maps have two collectors, one per team, bound to their own team's
zone and mesh:

| Map | Team 0 zone → mesh | Team 1 zone → mesh |
|---|---|---|
| 194 | `team1/victory_container_collision` [−73.901,5.976,44.204] → mesh 0.18 m away | `team2/victory_container_collision` [102.606,6.87,79.873] → mesh 0.11 m |
| 195 | `team1/team_1_vic_collision` [−48.406,0.512,−73.225] → 0.21 m | `team2/team_2_vic_collision` [49.924,0.234,14.527] → 0.21 m |
| 196 | `team1/team_1_vic_collision` [−23.041,0.512,−74.55] → 0.54 m | `team2/team_2_vic_collision` [38.245,0.234,2.466] → 0.33 m |

The 0.11a Chemical duplicate-zone defect is absent from the 0.11e donor; its
server project already references the team2 zone. No repair is needed.

### Respawn and battery inventories (offline, equal to donors)

| Map | Respawns team0/team1 | Battery candidates | Active per match | Body origin |
|---|---|---|---|---|
| 194 | 22/22 (0.11e server project) | 33 | 10 | applied (max error 5e-7) |
| 195 | 22/21 (0.11a fallback, equal to 0.11e client) | 21 | 10 | applied |
| 196 | 11/11 (0.20e) | 45 | 10 | applied by this work |

Respawn, spawner and collector groups equal the donor projects field for
field. The only additions are the native `layer`, `is_temporary`, check radii
and `spawned_item` fields. Point IDs are dense and global. The mode files are
native `level_0X_day_2` battery modes (mode 2, 10 batteries, 8 s respawn).

### User report: invisible collision at the Chemical boiler (194, converter fix `5d240c0`)

At 23:32:28 the user's client (roster slot 0, team 0) had its live camera at
[−66.484, 7.817, 25.290], forward [0.704, −0.521, 0.484]. The screenshots show
the domed `props/machines/factory_boiler`. The character stood on
`level/04/moss4.model/collision` (walker flags; ground 6.476 under the camera).
That mesh is byte-identical to 0.11e.

Cause: in 0.11e the boiler's walker collision models three of its four
vertical tanks as lying cylinders (primitives 11, 12, 14: radius ≈ 1.70,
half-length ≈ 1.65). Their square footprint reaches about 2.35 m from each
tank centre, against the visible 1.66 m tank. Along the user's heading a
0.35 m capsule already touches primitive 14 at the reported position.

Cylinder semantics were checked rather than assumed. The 0.11e factory
(0x7249CD) passes the saved [x,y,z] to the constructor at 0x74BFD0. That builds a
standard Bullet `btCylinderShape`: type 0x0D, up axis 1, implicit dimensions
= half extents × scale − margin. The support radius is x and the half-height
is y, exactly as native 0.26's [x,y,x] construction. Only the broadphase AABB
differed. Every build, native 0.26 included, stores cylinders with z = 0, and
the three maps contain hundreds of legitimately horizontal cylinders. A
blanket cylinder rewrite would therefore be wrong.

The developers fixed this asset. 0.21d and native 0.26 (identical payloads)
re-author the same model path with four vertical tanks (r ≈ 1.70, half 1.21)
plus a walker mesh. Bullet-collision extents of 0.11e and 0.26 agree within
0.1 m, which confirms the same geometry. Native 0.26 Chemical uses this
collision in retail play.

The converter gained `collision_successors`: a manifest maps a donor
collision directory to the target build's files for the same model. Both
primitive payloads are hash-pinned and the map must import the directory.
Only `factory_boiler.model/walker_collision` is listed, in both 0.11e
manifests. With the successor the capsule has no primitive contact over the
next 3 m and stays at least 0.86 m from the successor mesh. Regression test:
`collision_successor_replaces_only_a_pinned_imported_directory`; all 285
library tests pass.

Runtime walk verification is pending (see below). One attempted walk at
23:57 was discarded: the user was interacting with the windows (camera and
facing changed without scripted input, and a window closed).

## Runtime evidence so far

The first 194 session (23:25–23:48, unmodified candidate after the
release-fix plan) produced 271 checkpoints under `runs/194-clean`.

- Spawns: on entry the team-1 client stood 0.067 m from authored team-1
  `respawn_point(8)`, with the expected facing. The team-0 client was within
  the team-0 spawn area. The user then took over both windows. Suicide inputs
  sent during that time recorded no deaths; this is inconclusive, not a defect.
- Batteries: the 50 ms checkpoint's ten free batteries sit exactly at
  authored native body origins, for example spawner(20)
  [6.289, 2.244+0.12, 59.346].
- The user's own play: team-1 `first_pickup` became 1 at 753.3 s and 2 by
  888.6 s. A battery was carried at 888.6 s and free again at 898.6 s. No
  deliveries.

## Pending runtime batch (prepared, waiting for user approval)

`/tmp/day-map-gameplay-20261014/run_batch.sh` runs every step with hash-pinned
diagnostics. Each step does apply → two clients → stop → revert → verify the
database hash.

1. `194-boiler-after`: team-0 spawn 1 m behind the reported spot; walk the
   user's heading, then four waypoints along the boiler's east side.
2. `194-boiler-before`: the same walk on the pre-fix database (private copy
   `pkg-before`, port 2268, shared lock held).
3. `194/195/196-clean`: unmodified candidate. Entry spawn per team (roster →
   live camera → nearest authored point), ten battery positions against
   authored body origins, and two suicides (K) with respawns per client.
4. `194/195/196-delivery`: each team spawns beside its collector facing it,
   with one authored spawner record relocated 1–1.3 m in front. The
   character picks the battery up (E), walks to about 1.4 m from the zone,
   aims at it and uses it. Checkpoints decide the result: battery state
   carried→container, container items and `team_points`.

Diagnostic layouts were checked for floor and walker clearance offline
(`clearance.py` conservative AABBs). The 196 layouts sit north of the team-1
collector and east of the team-0 collector.

## Tools

- `prepare_gameplay_patch.py`: same-length, decoder-verified release plans.
- `prepare_overlay_patch.py`: hash-guarded add/replace overlay for `apply`.
- `prepare_delivery_diagnostic.py`: reversible delivery/spawn diagnostics.
- `run_gameplay_session.py`, `gameplay_ctl.py`, `drive_client.c`: the
  package launcher with a packet log, live camera status, roster mapping,
  closed-loop aim/approach and compositor screenshots.
- `decode_gameplay.rs`: per checkpoint, battery and container state, team
  points, pickup/put/solo-delivery events, deaths and respawn timers. It
  links the SRP `survarium` crate (srp-026g `0c551400`).

## Converter review follow-up (branch `gameplay-fixes`)

Each finding is checked against binaries, source and a rebuild of both
bundles from this branch (`/tmp/day-map-gameplay-20261014/rebuild-gf-*`).

### Cylinders (review #2): no general change; boiler successor kept

- v0.10b source (`vostok/physics/sources/collision_shapes.cpp`):
  `create_bt_primitive` builds `btCylinderShape(dim.x, dim.y, dim.z)`. The
  0.11e PDB maps the same path to 0x724930 → 0x74BFD0 → `setSafeMargin`
  (0x39490).
- Bundled Bullet: `setSafeMargin(halfExtents)` uses 0.1 × the smallest axis,
  so z = 0 gives margin 0. `CylinderLocalSupportY` (0.11e 0x3A240) reads only
  halfExtents[0] (radius) and [1] (half-height). The narrowphase shape is
  therefore a **solid** cylinder in 0.11e, as in 0.26's [x,y,x]. Only
  `getAabb` (0x74C0A0) is flat along local Z. The review's "flat shape" is
  incorrect.
- Movement uses `convexSweepTest` (ghost 0x720F30 / world 0x6A9050). It runs
  GJK only for objects whose AABB, expanded by the capsule AABB, the centre
  path crosses. When a cylinder's local Z is vertical, 0.11e blocked across
  ±capsule half-height (about the whole drum). When local Z is horizontal
  (upright posts and tanks), 0.11e blocked only within a capsule radius of
  its central plane and otherwise let the capsule enter and pushed it out.
- `audit_cylinders.py`: 194 has 692 movement cylinders (451 horizontal-flat,
  241 vertical-flat) and 195 has 269 (176/93), matching the review's counts.
  **None** of the 627 horizontal-flat cylinders sticks out more than 0.3 m past
  its model's bullet collision. 0.26's solid cylinders therefore stop players
  at visible geometry. Emulating the 0.11e band would let players walk into
  tanks and poles.
- The boiler drums 11/12/14 are vertical-flat: 0.11e blocked them too. The
  reported blocker is an authored defect that the developers re-authored
  later, not a conversion difference. `collision_successors` stays. The only
  other >0.3 m protrusion is one tyre-cover cylinder per map (vertical-flat,
  so unchanged from 0.11e).

### Respawn rules not in the source (review #9): fixed

0x2F4400 constructs respawn points with protection 0 and ally/enemy check
radius 0. 0x2F4580 overwrites them only when the key exists. `adapt_objects`
no longer lets `fill_missing` add `invulnerability_time` or the check radii
when the source lacks them. Authored values are kept, such as the converted
early 2 s. Rebuild: 194/195 have 2 s and no radii; 196 has no protection
(0.20e used the base `effect_zone`, which is preserved) and no radii. Test:
`objects_do_not_inherit_another_maps_gameplay_or_light_transform` (extended).

### 196 battery bodies (review #6): converter output, not only a patch

0.20e spawners have no `spawned_item`, so `battery_body_transform` runs. The
branch rebuild gives all 45 body origins at parent + rotate([0,0.12,0]),
maximum error 1e-6. The earlier database patch matches converter output.

### Physical materials (review #14)

- Armour/resistance live only in the global `game_materials/game.materials`.
  All native maps share it and the converter maps names and ids into it.
  0.11e `resistance` (concrete 1, brick 0.5), 0.20e `armor` (concrete 0.8)
  and 0.26 `armor` (0.23) belong to their builds' weapon balance. Restoring
  donor values would need new per-donor material ids and `material.pairs`
  rows, and would mix them with 0.26 penetration. Kept native and documented,
  not changed.
- Foliage: 0.11e `foliage` is id 24, which 0.20e/0.26 call
  `foliage_coniferous`; they add `foliage_deciduous` for poplars, bushes and
  anomaly-tree leaves. The two share ballistics (armour 0, width 0.1), so the
  difference is hit sound and effect pairs. Early foliage now takes, in
  order: a reviewed manifest binding (`game_material_overrides`, 22 models
  in the 0.11e manifests), then the target's foliage class for the same
  model surface, then coniferous with a recorded note. Rebuild: poplars and
  bushes deciduous; pines (sosna) and spruces (elka) coniferous. Test:
  `early_foliage_takes_reviewed_or_successor_class`.

### Game mode and server fixtures (review #15)

The 0.11e/0.20e donors have no game-mode configs (mode parameters were sent
by the server in that era), so the native mode template is the only source
and is kept. The server fixtures are in the srp repository, not the
converter. For `--finish-*`/`--results` on these maps, add:

```
# 026g-mode-timers.tsv
194	900	5	20	v011e_level_04_day_2.options	1a94d223d834a90b9c22932770b51bf10c92bdfb3293b060f7f9cc03f398ea64
195	900	5	20	v011e_level_03_day_2.options	7bd6b653d0262a5a43d4d6a40d64ecab72caa65bcdf1da4deff56a8838286b16
196	900	5	20	v020e_level_03_day_2.options	963fbf2cec5344073cc6c068b0bac0e1c8061b866f25f7b57f8ec6ec9c0a41f3
# 026g-result-modes.tsv
194	2	10	v011e_level_04_day_2.options	1a94d223d834a90b9c22932770b51bf10c92bdfb3293b060f7f9cc03f398ea64
195	2	10	v011e_level_03_day_2.options	7bd6b653d0262a5a43d4d6a40d64ecab72caa65bcdf1da4deff56a8838286b16
196	2	10	v020e_level_03_day_2.options	963fbf2cec5344073cc6c068b0bac0e1c8061b866f25f7b57f8ec6ec9c0a41f3
```

The candidate's launcher does not use those options; its matches run past
the 900 s limit, which was observed.

## Live user reports on map 194 (2026-10-15, offline diagnosis)

The user's session (lighting worker, port 2270) has no packet log, and its
camera status is only published on request, so it was not touched. The
reported spots were located from the user's screenshots and the data.

### No footsteps on the moss mounds and the cyan blob

Footsteps are the `collision_sound` of the (actor variant, surface
material) pair in `game_materials/material.pairs`. Native 0.26 has `actor`,
`actor_local`, `actor_sprint`, `actor_local_sprint` and the crouch variants.
0.11e's pair table has only `actor` and `actor_local`. In native 0.26,
`mud` (and both foliage classes) leave `actor_local_sprint` empty, so a local
player sprinting on mud is silent. The four 0.11e moss models (6,725 m² of
walkable mound; their `anomaly_liquid` render surfaces are the cyan blobs)
have one collision surface, `default` → `mud`, unchanged from 0.11e.
Native 0.26 re-authors the same models: the walkable mound is
`grass`/`grass_walkable`, and `mud` is kept only for separate liquid surfaces.

Fix: reviewed `game_material_overrides` bind moss…moss4 `default` → `grass`,
which has every actor pair. The liquid shares the mound's single collision
surface, so it also steps as grass; splitting it would need new collision
geometry. `audit_footsteps.py` measures walkable area per material against
the pair table. After the fix, 194 has no walkable surface without local or
sprint steps. 195's remaining rows are tree canopies. 196's
`terrain_all` `mud_walkable` is identical in the 0.20e donor and native 0.26.
Materials with no actor pair at all (`metal_arm_gran_*`, `restrictor`,
`wood_tree`, background `not assigned`) are native and shared by native maps.

### Invisible wall before the barbed-wire fence (north map edge)

`level_border.model/walker_collision` box 23 (Z 131.88–135.40, X −3.71…48.84)
blocked the only ground-level passage on that edge (X 43.4–48.1, between
`central_factory` and `admin_building`) 6.3 m before the visible
`fences_03` barbed-wire fence (Z 138.14). It is authored that way in 0.11e;
0.21d and 0.26 move it into the fence line (Z 138.32–141.45) and extend its
two corner boxes. 0.21d also re-authors the edge geometry, so the whole
border resource is not swapped. `collision_primitive_overrides` (hash-pinned
payload, exact original transforms) re-places only box 23 (0.21d values),
box 24 (0.21d values, east corner to Z 141.10) and box 22 (0.11e X extent
kept, Z extended to 140.92, because 0.21d's thinner box would expose a 0.22 m
strip of ground). The other 28 boxes are unchanged. Offline scan: before,
blocked from Z ≈ 132; after, walkable to the fence line and blocked from
Z 139 by fence + border. Both sides stay closed by building walls and there
is no ground beyond Z 140. Elevated building tops stay bounded by the moved
box. Test: `primitive_override_moves_only_audited_boxes`.

Other 0.11e→0.21d border changes (east edge 0.3–1.5 m outward; a west box
inside the team-0 base removed) are not adopted: the surrounding layout
changed in 0.21d and no user report covers them.

### No footsteps "running on the rails" (second report)

How 0.26 picks a step sound (reverse-engineered, matches 0.11e's PDB-named
`survarium::step_manager::on_step` at 0x6fac40 instruction for instruction):
the 0.26 handler at 0x248ad0 casts 2 m down from 1 m above the feet against
the non-walker collision (cgroup & 8; `walker_collision` is never queried),
reads the hit surface's game material and plays `collision_sound` (pair
+0x20) of `get_pair(actor variant, material)` at 0x306d50. `get_pair`
substitutes `physic.base_material` (so `metall_thick_walkable` steps as
`metall_thick`) and falls back to the default material when no pair exists.
A pair that exists with an empty sound is silent (mud/foliage local sprint,
`metal_arm_gran_*`, `restrictor`; identical in 0.11e's pair table). The
material table has 128 slots, so 0.26 indexes it out of range for 65535;
0.11e's `get_pair` replaced an unknown material by its default material.
`physic.walker_behaviour` is not involved (absent means 0, like ground).

The 194 rails are not mis-mapped: `railroad_01` rail tops are
`coll_metal_thick_mSG` → `metall_thick` and the sleepers `lambert2SG` →
`concrete`, exactly as in 0.11e; both have every walk/sprint pair, and the
sound files exist. Native 0.26 re-lays the tracks (different extent) with
`metall_thick_walkable` (same pairs through its base material) plus a
separate smooth walker mesh. The catwalks above the track
(`loading_station_01/02`) have the same layout and bindings as native 0.26:
walker-only `coll_restrictSG` (unassigned, not queried) over a cg-8 box
bound to `metall_thick`; their `metal_arm_gran_*` boxes are the railings.
So no data difference explains silence on the rails or the catwalk; the
spot needs a runtime check (below).

`audit_step_surfaces.py` grids every top surface the step query can hit
(0.5 m cells, ground and elevated layers) and classifies its step. Real
differences from the target's conventions on walkable areas, now fixed:

| Map | Model / surface | Source (0.11e) | Before | After | Area (top) |
|---|---|---|---|---|---|
| 194 | `test/mattest_cube01` `default` (two scaled 0.11e material-test boxes rendered with `water/water_green_shit`; the one with its top at Y 5.6, x −90…−70.6, z 46.5…72.1, is a closed room in the old factory; a player spawned there stands on green water) | 65535 (0.11e runtime: default steps) | 65535 (out-of-range lookup) | `default` (reviewed binding: what 0.11e played; not reachable in normal play) | 519 m² |
| 194 | `props/machines/turbine` `default` | 65535 | 65535 | `metall_thick` (native 0.26 binding of the same surface) | 5 m² |
| 194 | `flora/bush_s_006` `default`, `level/04/moss_alfa` `default` | 65535 | 65535 | `foliage_deciduous`, `ground` (native bindings; no walkable top) | 0 |

Rule (converter, early donors): a surface the donor leaves unassigned takes
the target's own binding of the same model surface when the developers
assigned one; otherwise it stays unassigned (test
`unassigned_early_surface_takes_assigned_target_binding_only`). Left as is,
because native 0.26 or 0.11e behave the same: background `bg_*`/
`buildings_bkg_*`/`lep_bkg` (65535 natively too, out of bounds), railing and
fence `metal_arm_gran_*` tops, tree canopies (foliage local sprint empty,
native), 196 `terrain_all` `mud_walkable` (native) and `restrictor` boxes.
Rebuilt 194/195 differ from the `-next` candidate only in these four model
settings (and the 2-map static dictionary, not used).

### Battery spawn "where I stand" (crashed hopper, 2026-10-15 13:42)

The screenshot is inside the crashed hopper of `level/04/wagons_crash` at
about (24.5, 3.4, 6.3), placed from the minimap (the six spherical tanks
are 13.3 m apart). 0.11e, 0.11a, 0.20e and our port all have
`victory_item_spawner(19)` at (21.38, 2.72, 4.61), about 1 m outside that
wagon's wall and 3.6 m from the spot; native 0.26 keeps the same spawner
0.6 m further out (20.95, 2.81, 4.14). The user confirmed the spawn works
in game; nothing was changed.

### Open item (parked by the user): silent second-floor walkways (third report: "all rails on the second floor")

The report means the tread-plate walkways above the train (`loading_station_01/02`,
`loading_station_balkony`) and the passages (`perehod*`), not the tracks.
Offline simulation of the step query (from 1 m above the movement floor,
2 m down, cgroup & 8): on the inner walkways the ray hits the cg-8 floor box
`lambert2SG` -> `metall_thick` 4.8 mm above the walker floor, and the native
0.26 models have the same layout (walker `coll_restrictSG` at Y 10.174,
visible `coll_metal_thik` -> `metall_thick` at 10.1788).

Tried and ruled out: binding the walkways' walker surfaces (`coll_restrictSG`,
`walker_col_s`, unlisted in 0.11e, 65535 in native 0.26) to
`metall_thick_walkable`. The user found the walkways still silent with it,
so it was reverted; the walker surfaces stay unlisted, as in native 0.26.

Parked: the user decided to leave the walkways silent for now. If it is
picked up again, the step log is ready but has never run in game
(`SRP_STEP_LOG=1`, `tools/render-compare/step_diagnostic.c`, off by
default; console test `test_step_diagnostic.c`). The 0.26 step handler
(VA 0x248ad0) casts the ray and calls `get_pair` (VA 0x306d50) with the
hit surface's 16-bit material value, and `get_pair` indexes its 128-slot
material table with all 16 bits (no `& 0x3fff`), so a surface that keeps
walker-behaviour bits would read outside the table. The log records per
step the raw value, the table entries, the pair and its sound, so one walk
on a walkway and on the ground next to it would show which of these differs.
A native comparison is map 79 (native level_04 day_2) with the same client.
