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
