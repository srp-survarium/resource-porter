# 0.26 gameplay fill and respawn-point audit (2026-10-15)

Gameplay fill adds native 0.26 gameplay objects to the ported day maps. It is
a design addition, not fidelity conversion, and is kept separate from the port:
`build-survarium-maps-026` applies a pack only when it is named with
`--gameplay-fill PACK` (repeatable). Without packs the build is the faithful
port, checked byte for byte: a pack changes only its map's project.

## Native 0.26 conventions (retail `c74078be…`, all eight battery maps)

| Element | Native objects | Placement convention |
|---|---|---|
| Ammo resupply | per team: `resupply_zone` (`team`, `resupply_time_in_ms` 3000) → `resupply_collision` box (half 0.83×0.51×0.66), `ammo_decal` (`decals/editor_decals_bullet_for_box_base`), on an `item/quest/container_closed` crate (render + collision) | one per team, 13–42 m from the team's nearest spawn, often within 6–40 m of its collector |
| Battery collectors | 2 per map, one per team: `victory_items_container` with `visual_geometry` `item/storage_device/box_atlant12` (charger), zone `container_collision` (half ≈1.0×0.85×0.7, origin on the floor, centre ≈0.8 m up), charger render + `collision`/`walker_collision` at the same transform | at each team's base |
| Artefacts | `generic_anomaly_core` controllers (artefact `dict_id`/`count`, `max_artefacts_count` 1–2, first spawn 60–70 s, respawn 60–80 s, chance 0.5–1) → `artefact_container`s (search 6 s, cooldown 80 s) with their own zones; `damage_zone`s | level_04: 2 controllers, 27 containers; level_03: 5 controllers, 16 containers |
| Anomalous fog | `particle` objects around the artefact field: level_04 `fx/level_4_anomaly/{spori,spori_mini,radial_smoke,bugs}` (30); level_03 `fx/level_3_anomaly/rust_smoke[_dark]` (10) | spread over the anomaly area |

More than two collectors, or a neutral one, is not established. The native
rule data always has two team-owned containers, and SRP's own snapshot decoder
(`Gather::CONTAINERS`) assumes two. Layouts here therefore move the two team
collectors and add none.

## Chemical 194 packs (`ports/gameplay-fill/`)

- `chemical-011e-anomaly.json`: native Chemical's toxic-moss anomaly, copied
  verbatim (88 objects: 2 controllers, 27 artefact containers with zones,
  damage/idle zones, 30 spore/smoke/bug particles). All 27 containers, both
  damage zones and both controllers land on the 194 ground at their native
  coordinates, which is the same moss field (moss.model densest cell
  [17.7,3.1,6.9]; native controller [16.0,2.1,2.0]).
- `chemical-011e-resupply.json`: per team, the native resupply group (zone,
  collision, decal, crate and its collision), moved rigidly to clear floor 4 m
  from that team's collector: team 0 [-73.94,5.611,40.04], team 1
  [98.66,6.59,79.44].
- `chemical-011e-collectors-base.json` (layout A): native chargers replace the
  old containers in place, with the native zone and collision; the old
  container collision is removed.
- `chemical-011e-collectors-train.json` (layout B, the user's suggestion):
  both chargers at the central train, in the gap between the two hopper-wagon
  lines (the wagon tops have no clear standing area). Team 0 [15.2,2.02,66.5],
  100 m straight-line from its spawn centroid; team 1 [15.7,2.10,70.0], 93 m.
  Team 1 is about 7 m closer. Path distances are not measured.

The packs build and pass the converter's anomaly-link and collector-zone
validation. Static views of every proposed spot: see the gallery path below.
Opening crates, refilling ammo, delivering to a charger and picking up
artefacts need a user play test.

195: native level_03 anomaly zones fit only partly (rust containers 6/10,
electric 5/6, damage zones 5/9 on ground); 0.20e's set fits no better. Its
anomaly needs hand-placed spots. Resupply and chargers can follow 194's pattern.
196 already has the 0.20e anomalies and artefacts; nothing is duplicated.

## Respawn-point audit (all 109 points)

`audit_respawns.py` tests each point against movement collision. It checks
support under the feet, a standing capsule (r 0.35 m, 0.25–1.8 m; exact
triangle tests and oriented primitive tests, cylinders as 0.26's [x,y,x]),
facing clearance at eye height, the map-border volumes and team side.
Earlier conservative versions produced false flags (primitive bounding boxes);
the final pass is exact.

| Map | Points | Audit flags (final exact pass) | Native check (point alone, real character, two samples 10 s apart) |
|---|---|---|---|
| 194 | 44 (22/22) | #29 grazes a lying pipe by ≈5 cm; #37 touches a sandbag cover | #29 nudged 0.30 m clear, then stable; #37 and #0 exact; all at authored height, zero drift |
| 195 | 43 (22/21) | none | 6 points, including the earlier false flags (#6 wall, #13/#16/#26 "below support"), exact |
| 196 | 22 (11/11) | #5 overlaps the map-border box by 0.13 m; #10 support reads 0.13 m high | #5, #6, #7, #10, #13, #14, #21 all spawn at the authored height (moved ≤ 0.07 m, zero drift) |

No point needs a fix: every flagged point spawns, settles at its authored
height and stays put. Facing was audited at eye height within 2 m; none
faces a wall once the facing test is exact. All points are on their own
team's side; only 196 #5 touches the border, harmlessly.
Two-client sessions on 196 stalled repeatedly (a background client stops
rendering and the lockstep waits); single-client checks, with the diagnostic
point re-teamed to the one client, avoid that.

Galleries (static camera-override views, two per point):
`/tmp/day-map-respawn-gallery-20261015/index.html`.

Proposal gallery for 194 (crates, layout A and B chargers, anomaly centre,
sample artefact containers): `/tmp/day-map-fill-proposals-20261015/194/index.html`.

## User test package (layout A)

`/tmp/day-map-fill-test-20261015`: the next candidate (`25ab9683…`) with only
the 194 project replaced through a guarded overlay
(`/tmp/day-map-gameplay-20261014/fill-194-overlay-v2`; the replaced project
`3704ad63…` equals this branch's faithful output). It contains 2 resupply
zones, 2 native chargers (no old container collision left), and 2 anomaly
controllers with 27 artefact containers. Database `d3c3a588…`. Start with
`./play-maps start chemical-011e-day --clients 1 --runtime /tmp/day-map-fill-test-runtime --port 2274`.
