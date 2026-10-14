# Day map objectives, 2026-10-14

Release scope: Factory 0.10b day (180), Chemical Plant 0.11a day (193).

## Battery locations

The packaged projects contain the original 26 Factory and 33 Chemical battery
spawners. Their IDs, priorities, names, positions and rotations match the donor
server projects. The new `spawned_item` transform also matches each source
spawner. These are not the 0.26 map's authored locations. This proves the stored
configuration, not the positions consumed by the running client or accessibility.
The user's reported battery placement defect remains open pending runtime checks.

Factory's historical `0.10b` donor is the build-802 resource database at
`/nix/store/5yk74ndm49i0vrrrmdrypzwrh8qkflxm-survarium-resources/resources.db`;
older repository documentation also calls that build `0.100b`. This is the donor
used by the preserved weather comparison scripts. Chemical's donor is
`/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs/011a/app/resources.db`.

## Confirmed Chemical collector defect

Both original Chemical collectors reference
`logic/team1/victory_container_collision`. This duplication exists in both the
original client and server projects. The separate team2 zone and collector mesh
exist but are unused by that reference. Copying this data preserves an original
gameplay defect.

The converter also selected a collector's mesh using the collector editor marker.
Both Chemical markers are in the west hall, so both collectors acquired the same
west mesh. Those markers are not authoritative interaction positions. In Factory,
one marker is 28 metres from its actual mesh, but the old nearest-mesh heuristic
happened to choose the correct model.

| Team enum | Original Chemical player spawns (X range) | Correct zone position | Collector mesh position |
|---|---|---|---|
| 0 | −84.584 .. −71.127 (22 points) | [−73.901, 5.976, 44.204] | [−73.942, 5.611, 44.038] |
| 1 | 102.779 .. 112.894 (22 points) | [102.606, 6.87, 79.873] | [102.664, 6.54, 79.938] |

The guarded Chemical correction assigns enum 1's collector to the authored team2
zone. This is a release gameplay correction, not an original-render parity change.
The converter then selects each mesh beside its named interaction zone and rejects
shared mesh bindings or models more than five metres from the zone. The repaired
project removes the east mesh from static visuals once the collector owns it.
Factory's collectors remain unchanged.

## Reproduce the audit and project overlay

`repair-day-map-objectives TARGET_DB FACTORY_SOURCE_DB CHEMICAL_SOURCE_DB OUTPUT`

Build with `cargo build --bin repair-day-map-objectives`. The tool reads the current
package, verifies battery placement fields, emits only the changed Chemical
project under `OUTPUT/overlay`, and records input/output resource hashes plus
collector changes in `OUTPUT/audit.json`. It does not modify a database. Re-run
against the latest package before integration to preserve other workers' changes.
The normal early-map build uses the same collector matching and Chemical repair.

Validation: two focused regression tests cover overlapping editor markers,
missing/distant zones, guarded Chemical repair and idempotence. The full
`survarium_026` unit-test group passes. Packed-project audit and binary-config
round trip pass. Runtime battery positions and collecting/delivering for both
teams remain to be checked.

## Factory runtime checkpoint check

The lamp worker's map180 run supplied three complete E2 checkpoints (50, 8,000,
13,000ms). All ten initial battery positions exactly match source spawners
19, 1, 2, 11, 7, 5, 15, 6, 18 and 9. At 13 seconds their serialized positions are
within 0.25m of the authored origins. No inheritance of native 0.26 spawn locations
or large displacement occurred in this run. The remaining 16 candidate positions
and actual pickup/delivery accessibility remain untested. Exact coordinates and
checkpoint hashes: `tools/day-map-objectives/factory-runtime-20261014.json`.

The repair tool also has a focused fixture verifying that a zone-only change is
written even when collector visuals are already correct, followed by a no-op
check using the repaired project. Its change accounting must match the emitted
project, including zone repairs and removed static visuals.

## Complete battery gallery diagnostics

`export_gallery_spawns.py` writes `/tmp/battery-gallery-spawns.json`: every original
candidate (26 Factory, 33 Chemical), with names, IDs and transforms checked against
the current package. `prepare_gallery_batches.py SOURCE_JSON OUTPUT` produces
seven reversible same-length project/mode plans and the compact gallery manifest.
Factory batches contain 10/10/6 candidates; Chemical batches contain 10/10/10/3.
No archive is changed by either tool. Plans must be regenerated after release
project changes; each is based on the same untouched project and must be reverted
before applying the next one.

The native gather mask is u16, so activating all 26/33 candidates simultaneously
would exceed its supported count. Each private batch narrows only the indexed
spawner array, preserves authored transforms, renumbers its IDs to 0..N−1, and
sets the mode's battery count to N. Dense IDs are mandatory: build2777's cook reads
the array count at 0x2F294B, allocates that many pointers at 0x2F298A and writes
`vector[point_id]` at 0x2F29A4..0x2F29CE. The selection routine at 0x315130 shuffles
points and marks each selected point (+0x60), preventing reuse. The batch plans'
configs have been independently decoded and checked; activation still requires
native checkpoint validation before claiming coverage.

After each batch starts, use the server's `--packet-log` output:

```
python tools/day-map-objectives/extract_snapshots.py PACKET_LOG SNAPSHOT_DIR
python tools/day-map-objectives/validate_gallery_batch.py MANIFEST MAP BATCH SNAPSHOT_DIR \
  --decoder /tmp/decode-battery-snapshot-json --players 1 --output UPDATED_MANIFEST
```

The decoder source is `decode_snapshot.rs`, linked against the existing SRP
`survarium` crate (v026g match snapshot decoder); its CLI is
`FILE TIME_MS PLAYERS BATTERIES`. The matcher requires every initial battery to
match exactly one authored batch candidate and two later checkpoints at least a
second apart, both after five seconds. Their maximum movement must be ≤0.03m.
It records active battery IDs, actual `runtime_position`, displacement, original
and settled times, then copies complete raw checkpoints and decoded JSON beneath
`UPDATED_MANIFEST`'s parent `/checkpoints/MAP/batch-NN` (or `--evidence-dir`).
Candidate metadata never becomes active-battery evidence without this validation.
A wrong-batch fixture is rejected without writing a manifest; the ten-battery
Factory native checkpoint fixture passes and preserves all complete snapshots.

## Factory candidate 00 sinks into the floor

The first complete gallery batch exposed an actual placement failure. Candidate
`victory_item_spawner(21)` starts at `[-19.012, 10.675, -12.495]` and settles at
`[-18.930687, 10.257141, -12.588287]`; displacement is 0.436m. Gallery views 01
and 03 show the intact floor with no visible battery. Stability of a checkpoint
therefore does **not** establish visibility or pickup accessibility.

A bounded offline audit found no conversion shift at this location. All eight
nearby collision instances and their twelve payloads match the source (primitive
configs compared semantically, triangle buffers byte-for-byte). At the settled
XZ, `level/03/antenna/antenna_base.model/collision` triangle 3896 is the top surface
at Y10.674022786; triangle 3906 is the slab underside at Y10.155880928. The top
triangle normal points upward. The source and target visible
`floor_linoleum_02_s_vc` triangle 1870 is Y10.674022812. Four intersecting visible
surfaces retain identical vertex-position bytes and indices after conversion.
Thus the battery settles inside the original half-metre floor slab, about 0.10m
above its underside and 0.42m below its top. Exact geometry and resource hashes
are in `tools/day-map-objectives/factory-spawn00-floor-20261014.json`.

The old and native static electrobox mesh bounds are identical: local Y
0.00118..0.23545. The old `victory_item_core::put` inserts its usable collision
and sets the authored transform; it does not perform the new dynamic settling.
The native options add a box of half extents [0.3, 0.12, 0.16] at local Y0.12.
This options box is not yet proved to be the dynamic rigid body's actual shape.
Native build2777 also has a separate body/model transform offset: the free-item
getter at 0x300A20 subtracts the vector at +0x284 from its +0x204 transform. Do not
assume a serialized body translation is exactly the static mesh's bottom pivot.

Initial penetration on the slightly uneven floor and a change in dynamic-body
collision filtering are still competing explanations; neither is proved by this
geometry audit. The next useful isolated runtime experiment is this candidate
alone with a small upward initial clearance, retaining XZ and the map geometry,
then checking settled visibility and pickup. Do not apply a global spawn lift or
change the map's collision meshes based only on this case. Complete the ongoing
original-coordinate gallery before testing changes.

Factory gallery batch 1 exposed falls in candidates 12 and 15. At 5.3 seconds
these had reached Y−99.8528 and Y−120.6251; at 10.3 seconds both were back near
spawn. Candidate 14 instead stayed stable below its authored floor (Y10.2508
versus Y10.656). A final position alone therefore hides both falling and resets.

`validate_gallery_batch.py --record-unsettled` is an explicit gallery diagnostic
mode. It still requires exact initial candidate matching, complete free-item
records and two later checkpoints. It records motion per battery and maximum
vertical fall across **all** checkpoints. If late motion exceeds 0.03m or any
fall exceeds 3m, `runtime_position` and `settled_position` remain null, the camera
falls back to the authored spawn, and `latest_runtime_position` retains the
actual latest coordinates. `runtime_state`, `runtime_note` and the camera basis
make the failure visible. The default mode rejects these failures without writing
a manifest. The real batch-1 fixture rejected by default and recorded only 12/15
as unsettled with the flag; the earlier stable fixture still passes and an
incorrect batch still fails even with the flag. All four raw checkpoints and
decoded records are retained. Stable coordinates never imply pickup accessibility.

## Native battery origin contract identified

The previous uncertainty is now resolved at the resource/ABI boundary. All 50
unmodified native Factory spawners have
`spawned_item.position = position + rotate(rotation, [0, 0.12, 0])`, within the
resource's millimetre rounding (maximum measured error 0.000579m). For example,
parent `[-11.18, 6.741, -9.071]` has nested `[-11.18, 6.861, -9.071]`. The parent
remains the mesh's floor origin; the nested position is the physical body's
centre. Exact original records and recomputed results are stored in
`tools/day-map-objectives/native-battery-origin-contract-20261014.json`.

Native build2777 proves why:

- 0x30029C..0x3002DA reads `collision_geometries[0].position` into battery +0x284.
  The default options supply [0, 0.12, 0].
- 0x3002E0..0x30037C reads `meshes[0].scale`, multiplies by 0.9, and creates type1
  box geometry through 0x7A390: half extents [0.27, 0.108, 0.144]. The dynamic body
  constructed at 0x300463 is stored at +0x2A8.
- `put` at 0x300900 calls `set_transform` at 0x3009B0. The latter copies the input
  into +0x204/+0x244 and forwards it to the body unchanged.
- The free-item transform getter at 0x300A20 subtracts rotated +0x284 to obtain
  the mesh origin. The spawn rule at 0x315480..0x31549D passes the selected
  spawn's stored transform (+0x18) directly to virtual `put` (+0x9C).

The early converter currently copies the parent position directly into
`spawned_item.position`. Its preservation of the original parent coordinates is
correct, but its generated nested body coordinates miss this rotated 0.12m
translation. On a flat surface the resulting rigid box initially penetrates
roughly 0.108m. This explains why many batteries are pushed up while others sink
or fall. A correction must preserve the authored mesh origin and generate the
native body origin, rather than apply an arbitrary map-wide clearance. The same
conversion is used for Chemical Plant. Runtime confirmation of a corrected batch
and pickup is still required; no release changes were made in this investigation.
Gallery initial matching must also distinguish original mesh coordinates from
converted body coordinates when that correction is tested.

Factory candidates12 and15 lie essentially on their original support geometry:
terrain_p_03 triangle1155 is Y5.4869144 under candidate12 (origin5.487);
antenna_base triangle4573 is Y5.9585224 under candidate15 (origin5.956).
These heights further support the missing centre offset, not relocated spawns.
The frozen gallery/package EXE hash is
`5eb0fc97b983ecc5d98282444076c7dc8180798429c58c4ac35849cb110c993a`.
Its pickup bytes at 0x2D84C9, 0x2D8545, 0x2D861C and 0x2D85BE match original2777;
the other session's experimental downward-pickup recovery patch is absent and
cannot explain the falling/reset evidence. Factory work is paused at the user's
request while the Chemical Plant gallery proceeds.

The converter now reads the target battery's collision offset from
`gameplay/victory_items/default.options` and rotates it into world space when
creating a missing `spawned_item`. Existing nested transforms are preserved, so
0.21/native inputs do not receive an offset twice. The existing battery test
now checks all50 native authored examples and conversion idempotence; all21
survarium_026 tests pass without increasing the test count.

`prepare_body_origin_patch.py BASELINE_PLAN NATIVE_OPTIONS OUTPUT` prepares a
same-length project-only plan from a saved gallery baseline. It verifies source
hashes, rejects aliasing between parent and body position storage, and uses the
independent Rust config decoder to prove that only nested position leaves change.
The unapplied Chemical plan `/tmp/chemical-battery-body-origin-fix/plan.json`
corrects33 body origins. Gallery batch manifests now retain both `position`
(authored mesh origin) and `initial_body_position` (native checkpoint origin).
Initial validation compares against the latter; camera/runtime positions remain
native body centres without a second offset. This corrects the earlier audit's
inference: exact checkpoints at source coordinates proved matching numbers, not
correct mesh placement under the native contract.

## Settled Chemical footprint and camera checks

The private checkpoint decoder can now retain each free battery's exact native
body basis. It locates the already decoded gather payload uniquely and checks
all extracted transform translations against the library's decoded positions.
`validate_gallery_batch.py` retains `runtime_body_basis` in the gallery manifest.
`audit_battery_geometry.py GEOMETRY_ROOT PROJECT MANIFEST --output REPORT` uses
that basis for an oriented box/triangle separating-axis check. It also detects
body centres inside authored box primitives and upward-facing mesh surfaces
above the body. Contacts within2cm are reported but not treated as deep burial;
non-box primitives remain explicit review items. This is resource-level geometry
evidence, not a claim that every collision group blocks the battery physically.

The tool's `Geometry.local`, `segment_triangle`, `Geometry.footprint` and
`Geometry.cameras` share the parsed geometry. Camera selection tries16 azimuths,
first at standing height and then lower crouched heights, choosing distinct close
and context views with clear collision segments. Render-only occluders still
require image review. The first corrected Chemical batch has10 stable batteries,
zero missing collision resources, and two collision-clear suggestions for each.
The conservative footprint flags candidate0's wagon walker-only geometry and a
2.7cm terrain intersection at candidate4 for visual/contact review; the other8
have no deep intersection. Do not call these flags proved placement failures or
call the complete33 clear until all native checkpoint batches are checked.

Native battery insertion at0x301CB0 passes filter group0xA18 and mask0x418 to
`world::add` at0x301CF5..0x301D00. Walker-only cgroup2/cmask4 therefore does not
collide with the battery, although it can obstruct player access. Geometry rows
now label this distinction instead of treating every mesh as a physics contact.
Camera suggestions prefer crouched views when a vertical segment detects cover
above the battery, which matters for the wagon's undercarriage.

Visual camera rays now exclude controller-only walker proxies, including their
camera-inside-box rejection. Those invisible volumes can enclose an intended
visible wagon gap; they remain fully included in the physical footprint audit.
Each suggested pose records that exclusion policy. Low views also try eye height
body+0.2m. A direct fixture verifies two visual poses can be chosen while the same
walker box still produces a physical overlap review flag. No batteries moved.
