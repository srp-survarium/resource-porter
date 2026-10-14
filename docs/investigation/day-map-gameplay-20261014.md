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
