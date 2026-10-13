# Frozen playable maps, 2026-10-13

Fidelity work is paused and the three rendering workers are stopped. The local
package lives at `/mnt/hdd/Projects/survarium-map-pack-20261013`. It uses one
`resources.db`, a private match server, and the normal character controller with
UI enabled. No director/free-camera request is written.

| Launcher key | Map ID | Preserved variant |
| --- | ---: | --- |
| factory-010-day | 180 | Factory 0.10b day |
| factory-010-evening | 181 | Factory 0.10b evening |
| factory-010-rain | 182 | Factory 0.10b rain |
| chemical-011-day | 193 | Chemical Plant 0.11a day |
| school-021-rain | 165 | School 0.21d rain |

These are five variants of three locations. The earlier eight-item inventory
counted the three Factory 0.11a variants as additional maps. Their source and
manifests remain preserved, but this compact package uses the refined 0.10b
Factory variants. The School base retains its pre-existing resource content.

Run `./play-maps list`, `./play-maps start school-021-rain`, `./play-maps status`,
or `./play-maps stop` in the package directory. `start KEY --check` validates a
launch without opening the game. `./play-maps verify` checks stored file hashes.
The launcher stops only its own supervisor, Wine prefix, and match server.
Port 2260 and `.work/playable-map-pack-runtime` are its defaults. Wine and Python
remain pinned local Nix dependencies; this is not a portable Windows installer.

## Combining and rebuilding

`tools/playable-map-pack/combine.rs` combines the finalized School database with
Factory rain, Factory day/evening, and Chemical Plant. Run it with:

```sh
cargo run --release --bin combine-playable-maps -- \
  SCHOOL_DB FACTORY_RAIN_DB FACTORY_WEATHER_DB CHEMICAL_DB WORK_DIR OUTPUT_DB
python3 tools/playable-map-pack/build_local.py OUTPUT_PACKAGE OUTPUT_DB
```

The combiner rejects existing output/work directories, rewrites and roundtrips
parsed configs, merges the four added map registrations, and verifies every
selected resource after packing. It also compares every existing `v021d` resource
with the School base. Native archive layout is preserved by the existing overlay
writer. The day/evening resources use `v010w` internally because 81 shared
materials differ from the frozen rain database; ordinary overwrite ordering
would change one of the saved appearances.

The four input databases can be rebuilt using the porting manifests and family
scripts. The research backup contains the exact worker source snapshots,
including uncommitted files, plus external analysis and scripts. The package
retains frozen compiled shader assets and per-map recipes: rebuilding the map
geometry alone does not reproduce those lighting changes.

## Preserved research and cleanup

`/mnt/hdd/Projects/survarium-map-research-20261013` contains verified source
snapshots for all four rendering worktrees, commit identities, uncommitted
patches, external reports, scripts, and PNG/JPEG comparisons. Existing BMP
screenshots and original-client reference buffers remain in their working or
archived locations. Earlier full capture archives remain under
`/mnt/hdd/Projects/survarium-render-archive/2026-10-13`.

With user authorization, large regenerable **0.26 target** GPU buffers were
removed after saving the analysis. The exact inventory and reclaimed physical
space are recorded in `removed-target-gpu-buffers.json`. Old capture hash
manifests describe historical evidence; they no longer imply every raw target
buffer is present. Re-running pixel analysis that requires deleted buffers needs
new target captures. Source reference captures and final shader recipe assets
were excluded from that deletion.

## Limits of this checkpoint

This preserves the latest selected appearances, not completed visual parity.
Factory still has some lighting/material differences; Chemical Plant retains
some shadow differences and localized invisible collisions. School includes the
original rain assets and latest focused probe correction, but the final broad
comparison was interrupted at the user's request. Factory 0.11a has no equally
refined weather recipes. Automated character movement remains unverified.
The combined archive is checked structurally and by resource hashes; the new
combined package has not yet been walked in-game.
