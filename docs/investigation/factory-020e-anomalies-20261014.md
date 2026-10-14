# Factory 0.20e anomalies in 0.26

Status: conversion and offline audit in progress; native gameplay is not yet
verified. The user's acceptance condition is functioning anomalies, including
hazards and artefacts. An attractive map or visible particles alone do not meet it.

The initial request named 0.20d. No exact build was found in the local builds or
archive catalog. The user subsequently authorized 0.20e, with 0.21d as a possible
fallback. This work uses 0.20e build 1916, not a relabeled 0.21d map.

## Inputs and reproduction

- Donor: `survarium_full_020e/Survarium/game/resources.db` under the local
  `Survarium Builds` directory. SHA-256:
  `5f871046e3d24c8b42d538b864d34b34ab004e4bbd4c36e84b494bbfea258485`.
- Target: retail 0.26g0 build 2777, SHA-256:
  `c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c`.
- Manifest: `ports/survarium-020e-factory-to-026g.json`; Factory day, ID 196,
  `projects/v020e_level_03/day_2/client_project`, resources under `ported/v020e`.

```sh
vostok-port build-survarium-maps-026 \
  --manifest ports/survarium-020e-factory-to-026g.json \
  --donor-db "$DONOR_020E_DB" --target-db "$RETAIL_026_DB" \
  --output /tmp/factory-020e-bundle
vostok-port extract --db "$DONOR_020E_DB" \
  --path projects/level_03/day/client_project --output /tmp/factory-020e.project
python3 tools/factory-anomalies/audit.py /tmp/factory-020e.project \
  --output /tmp/factory-020e-anomalies.json
```

Only an additive bundle is needed for offline investigation. Do not overwrite
the two-map release or duplicate a complete database before runtime testing.

## Complete authored anomaly inventory

The day project has five controllers, eighteen damage zones and eighteen
artefact containers. Every state/group/zone/container/collision link resolves
in the donor. The three small electrical controllers have the same editor
position; their actual damage-zone geometry determines their separate locations.

| Controller under `anomaly/` | Authored behavior | Artefacts |
| --- | --- | --- |
| `random_anomalies/antena/entarance/electro_anomaly_1` | Two alternating electrical zones; active 40–60 seconds, recharge 15 seconds; shock hit 0.6 at 1000 ms interval | Disabled |
| `random_anomalies/antena/coridor_2/electro_anomaly_1` | Two alternating electrical zones, same timing/damage | Disabled |
| `random_anomalies/antena/coridor_1/electro_anomaly_1` | Two alternating electrical zones, same timing/damage | Three containers; Onyx, authored count 1; respawn 120 seconds, chance 0.2 |
| `electro/electric_anomaly` | Twenty-second idle state followed by one-second strike; eight lightning particle locators, one zero-damage idle zone and one shock zone (0.9, 2000 ms interval) | Five containers; Spring, authored count 3; respawn 120 seconds, chance 0.5 |
| `anomaly_rust/rust_anomaly` | Persistent fire puddle (0.15/1000 ms) and toxic gas (0.2/1000 ms), authored hit/speed curves | Ten containers; Spring count 5 and Onyx count 2; respawn 120 seconds |

The units of `max_hit` are the native damage scale, not an independently verified
HUD health fraction. Eight lightning locator zones and the dummy zone knowingly
author zero damage: they are state-controlled audiovisual components.

Factory 0.21d contains the same forty-one named objects and identical anomaly
controllers. Only `speed_curve` and `max_hit` change for gas, `speed_curve` and
`damage_type` for the puddle, and `speed_curve` for the electrical dummy zone.
It offers no simpler anomaly schema than 0.20e.

## Conversion defect found

The importer previously filled every absent field from the first native object
of matching type/library. For anomalies, this could import another controller's
artefact references, cap, first-spawn delay and random respawn variation. For
damage zones it could add unrelated particle/sound fields to an intentionally
silent zone. A container inherited a new search cooldown.

The native loaders explicitly support these absent fields. Retaining the
source fields and setting only `is_temporary=false` for anomaly controllers,
damage zones and artefact containers uses those compatibility defaults instead.
One regression test checks these three object types. Scene-name validation now
checks controller-to-zone, controller-to-container and zone/container-to-collision
links, independently of the resource dependency scanner.

Evidence from native 0.26g disassembly (image base 0x10000):

- `generic_anomaly_core::load` at `0x33bd50`: `max_artefacts_count` is optional;
  when absent it defaults to the source container-array size (`0x33bd81`–`0x33bdee`).
- `artefacts_respawn_variation_sec` and `artefacts_first_spawn_time_sec` are
  optional and default to zero (`0x33be1d`–`0x33beac`).
- `energy_maximum` is optional and defaults to zero (`0x33bfaf`–`0x33bfe9`);
  the energy increment clamps only when this cap is positive (`0x33bcff`–`0x33bd1e`).
- `artefact_container_core::load` at `0x31c480`: optional
  `artefacts_cooldown_time_sec` defaults to zero (`0x31c4ce`–`0x31c52b`).
- The 0.20e PDB identifies `generic_anomaly_core::load` at `0x1dbf2b`; its
  executable has none of those five new field names. Both older and native
  implementations still load authored spawn chance and respawn interval.

For this donor, particle and sound dependencies are imported from source,
including same-named resources; new particle properties use the existing
0.26 compatibility adapter. Runtime gameplay/item classes remain native.

## Runtime feasibility and outstanding proof

0.26 retains the generic anomaly, damage-zone and artefact-container classes.
Existing reverse engineering in `skhid-srp/srp-026g/docs/notes/static_info.md`
identifies native anomaly seeding at `0x1f6680`. The match server already sends
the same match seed to all clients. This supports using the native simulation;
it does not prove the converted objects synchronize correctly.

Before shipping, an owned private session must demonstrate:

1. All five controllers instantiate, and both clients retain matching world
   checkpoints through an electrical state transition.
2. A real character entering each hazard family takes the authored kind of
   damage; leaving ends it. A camera teleport cannot demonstrate contact.
3. The large lightning field activates its particles and sound on the strike
   phase. The zero-damage locators must not gain accidental damage or effects.
4. At least one authored artefact container spawns, can be searched and yields
   its expected item; pickup, removal and respawn agree on both clients.
5. Any authored speed effect is observed or explicitly listed as unsupported.

No game, GPU capture or shared release database was touched during this audit.

## Offline result

The first completed bundle contains 5,213 files (826 MiB), 287 converted
materials and eighteen adapted source particle definitions. Comparing every
anomaly/controller/container field after namespace normalization finds no
changes in any of the forty-one objects or their twenty-seven sensor geometries.
All scene links resolve. The focused
`survarium_026` suite passes 22 tests, including one new regression test.

Five stale model collision paths use the source model's surviving counterpart:
the four `level/03/anomal_tree_0N_` models use `collision` instead of missing
`walker_collision`, and `props/electric/soviet_comp_01` uses `walker_collision`
instead of missing `collision`. These are the importer's existing explicit
repairs and remain traversal risks until walked. They do not change anomaly
sensor geometry or trigger links.

A private test package can be prepared with
`tools/factory-anomalies/prepare_package.py BASE_PACKAGE OUTPUT`, after packing
its single `OUTPUT/resources.db`. It copies the working patched client, silent
BugTrap and Wine template, uses map 196 and port 2264, enables packet logging,
and leaves all legacy shader overrides disabled. It starts through its own
`play-maps start factory-020e-day --clients 2 --ui on` command, only after the
runtime/GPU lease is available. The current two-map release is never modified.
