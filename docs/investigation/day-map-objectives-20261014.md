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
