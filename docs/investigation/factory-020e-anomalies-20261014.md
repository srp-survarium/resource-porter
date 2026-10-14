# Factory 0.20e anomalies in 0.26

Status: converted and running in native 0.26; electrical anomaly damage is
verified and the user accepted anomaly gameplay. Lighting calibration is in progress. The user's acceptance condition is functioning anomalies, including
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

Use an additive bundle for composition; keep experimental runtime databases isolated.
The release labels this location Radar Station. Historical Factory paths and
map ID 196 stay stable.

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

The initial inventory was CPU-only. Subsequent private runtime evidence is recorded below; the shared release database remains untouched.

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

Packed private database: 7,129,128,555 bytes, SHA-256
`4ba71bb9b8a1d93df954f601d4a1f4fdbfab92cf873ad5fe23cba8ada75ec13b`.
`apply` reopened and verified all 5,213 emitted resources. The runnable private
package and evidence are at
`/mnt/hdd/Projects/survarium-factory-020e-anomalies-20261014`.
The launcher map listing passes; no client has been started yet.

## Native gameplay run and user acceptance

On 2026-10-14, the private map196 session at port2264 loaded two real clients.
A reversible diagnostic changed only the22 player spawn positions to
[-55.767,5.6,24.819], inside the source electrical hit sensor. Anomaly objects,
geometry, timing, damage and artefact definitions were unchanged. Native
checkpoint statistics show both players dying once at23.8s, twice at43.9s,
three times at63.9s and four times at84s, matching the authored21-second
cycle. Blue electrical effects were visible. No attack input caused these deaths.

The final packet audit pairs nine same-wire-tick reports received within one
second across the two clients; all nine world CRCs match, through411.9s of
packet-log time. Unpaired reports are not proof of agreement. The server
verified checkpoints throughout the session and reported no mismatch. See
`tools/factory-anomalies/compare_hashes.py` and `decode_checkpoint.rs`.

The user independently played the map and stated "anomaly works". Further
retesting of that accepted behavior is unnecessary. Artefact pickup/respawn,
other hazard families and speed effects have not been independently measured.
A bounded E-key search returned no artefact award in the observed checkpoint;
this is inconclusive because the authored container may be empty and precise
look targeting was not established. Do not claim those separate observations.

Both clients closed. The diagnostic spawn patch was reverted and readback
verified; baseline and patch bytes remain in the private package's
`evidence/diagnostic-spawn`. Small screenshots, packet logs,91 checkpoints,
event summaries and CRC comparison are in `evidence/runtime-20261014`.
The user-operated window was left untouched once their activity was identified.

## Lighting calibration, not electrical effect calibration

Seven user screenshots at04:40:06,04:40:14,04:41:27,04:41:33,04:41:42,
04:41:57 and04:42:26 show the control room, stairs, main hangar, upper skeleton
room and outdoor anomaly tree. They document ordinary illumination and sharp
contrast; they are not screenshots of an electrical strike. The initial private
package deliberately used native026 shading with no old-renderer preset.

An offline source/converted-project audit found15 source spotlights inherited
an unrelated native lamp's local offset[.01,-.265,.002]. Source light placement
has no such offset. Conversion now supplies identity when that local transform
is absent while preserving an authored offset. The existing compatibility
regression was extended; all22 focused importer tests pass, with no increase
in test count. This repair is not yet applied to the private database.

The source has79 ambient lights;30 have intensity>1, ranging3.2–13.3. PDB and
original020e disassembly locate `render_ambient_lights` at0x10bdb0. It traverses
the complete visible list and multiplies color by full intensity at
0x10bf67–0x10bf97. `create_targets` allocates its target0x1f asRGBA8_UNORM
(format28) at0xfad6e–0xfad7b. Native026 instead partitions intensity>1 into
additive diffuse with intensity−1, the same stage mismatch proven by School.

However,020e is not identical to021: its exact704-byte
`apply_ambient_lights.ps` (SHA256
94b548fecc000ea0b7cbf04eeeb1678afbefcfe5dce699fa51b6cbbd4942f07e)
samples t0 and broadcasts red to both output targets. Original021/native026
use colored diffuse and a separate specular mask. The020e/native input and
output signatures match. An opt-in candidate pairs the guarded stage correction
(`SRP_LEGACY_020E_AMBIENT=1`) with this exact source shader
(`SRP_LEGACY_020E_AMBIENT_APPLY`); it is compile-checked but still needs matched
original/converted captures. No general brightness multiplier was added.

Source probes gained `affect_diffuse=true` and the environment gained native
diffuse multiplier1; these are suspects requiring source-contract validation,
not yet proven defects. Do not disable them merely because the fields are new.

For bounded original references, the020e PDB identifies `apply_camera` at
VA0x17312c/RVA0x16312c. Unlike010/026, its director arrives inEAX; the six-byte
prologue and matrix offsets are checked. `prepare_client.py --build020e` pins
the original executable SHA256 and patches the verified too-small-video-memory branch at RVA 0xb91d3 in
the disposable copy. It also suppresses three UI-only spectator callbacks that
dereference the absent lobby/chat movie: chat mode, loading-page selection and
loading-progress display. Their complete prologues and calling conventions are
checked. These diagnostics do not modify source assets or rendering code.
The latter two bypasses remain untested at this checkpoint.


## Release handoff and reference checkpoint

The corrected additive bundle is `/tmp/factory-020e-final-bundle`: 5,213 files,
287 materials, original anomaly/spawn data, identity missing light offsets.
Commit `33119b2` contains the converter repair. No candidate shader or ambient
stage change is baked into this bundle. All 22 focused converter tests passed;
the existing regression was extended without increasing test count.

Original reference attempts remain inconclusive: no usable frame was captured.
The private spectator connected and received its static map setup after adapting
its observed session-zero / `ffffffff` admission. Saved custom dumps identified
missing Flash UI at `chat_handler::set_local_player_name`, `set_mode`, then
`lobby_menu::switch_to_level_loading`. The guarded reference helper suppresses
only the encompassing chat-mode and two loading-UI callbacks. Start with UI on;
the observer disables the verified UI flag at the first capture request.

The source serializer differs from 0.21: 21 u16 score weights, no match-level
field, no protection field, no item modification bytes and no final local-slot
byte. Its single empty-equipment profile is for renderer references only.
The server is a private loopback adapter, not a port of 0.20 gameplay networking.
`prepare_reference.py` and `run_reference.py` preserve this experiment. Runtime
state lives under `/tmp/factory-020e-reference-ready`; clients/server were stopped
and the GPU handed to the lamp worker before further offline preparation.

Keep the ambient candidate disabled until original/converted views prove it.
The source has a red-channel ambient apply contract; borrowing the 0.21 colored
apply shader would be incorrect even though the stage partition issue matches.


## Frozen original renderer reference

The final guarded spectator retry rendered successfully. One original session
produced 12 fixed-camera, UI-off 1280×720 PNGs; four provisional views were
refined in that same session to remove foreground intersections. The frozen
set is `/tmp/factory-020e-reference-ready/frozen-reference`, mirrored in the
private package under `evidence/original-reference-20261014`. `poses.json`,
per-frame hook status, full game/server logs and SHA-256 manifest accompany it.
Present and camera counters continued advancing, with no error overlay.
The reference clock handshake disconnected after loading: these are static
visual references, not timing evidence for anomalies, wind or multiplayer.
The source renderer and source assets were unchanged.

Quality qualification: source 0.20 accepts texture quality 0–2 and defaults to2;
its console rejected the attempted3. A read-only external probe was denied by
the OS, and the client did not persist settings on graceful exit. Therefore the
live value and effective mip offset are unconfirmed. Both verified VRAM fallback
constants were raised from256 to1024MiB to avoid the known Wine fallback issue.
Future preparation requests2 explicitly. No target blur compensates this caveat.

The first successful source frame proves the three guarded spectator UI callback
bypasses work. Static map setup differs from021 as documented above. All owned
clients and servers stopped; the isolated wineserver was waited before handoff.
Lighting comparison and candidate validation remain outstanding.


## Prepared target comparison

`prepare_lighting_comparison.py` makes a private harness from the combined
candidate's map196 recipe, retaining the exact native preset. The shared database
is hard-linked, never edited; binaries/presets are private copies. The harness at
`/tmp/factory-020e-lighting-candidate` uses runtime
`/tmp/factory-020e-lighting-runtime`, port2264. `run_lighting_comparison.py`
checks the restored combined DB hash, replays the frozen screenshot-only poses
on baseline and ambient candidate, requires successful stage/apply receipts,
and restores the native preset byte-for-byte in `finally`. It also guards the
shared preset and verifies the database again afterward.

## Ambient comparison and remaining differences

Both 12-view target arms completed. The candidate receipts verify the guarded
stage changes and successful creation of the exact 704-byte source apply shader.
The candidate removes excess illumination from the control-room ceiling and
the interior yard wall. The matched control-room full-frame sRGB MAE changed
from 0.180703 to 0.0265482, and the interior yard view from 0.151773 to 0.0292041.
These are static image comparisons, subject to the source quality caveat above.
They do not establish full renderer parity. The sky remains visibly different;
the source has a sunlit patch on the stairs absent in both target arms. Active
electrical glow and foliage phase also differ from the disconnected source.

The first baseline image had a stale viewpoint despite matching hook receipts;
the second was also conservatively rejected pending inspection. A bounded
two-view retry fixed the first and confirmed the second's framing. The final
12-view mean sRGB MAE is 0.067820125 native versus 0.040391825 with source ambient;
median is 0.04290785 versus 0.02807465. Eight views improve; four worsen slightly,
including views with electrical glow or foliage-phase differences. The initial
aggregate containing the stale image must not be used.
`compare_lighting_images.py --exclude-view NAME` retains the
raw per-view numbers but omits rejected views from aggregates. Always inspect
the framing before interpreting the metric.

The initial candidate launch stalled because the observer retained the menu
director. The harness now waits for the server's initial world snapshot and
advancing Presents, then the first capture request reselects the active director.
It supports `--arm candidate` so a failed arm does not repeat a completed one.
The private package also includes the exact Wine-prefix manifest required by
the managed launcher. The final candidate run stopped cleanly, waited its isolated
Wine server, restored the baseline preset and verified the combined database
SHA-256 `241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`.
Shared preset bytes did not change.

Compact PNGs, camera receipts, logs, restoration proof and three comparison
sheets are preserved under the private package's
`evidence/ambient-comparison-20261014`. Sheet columns are original020e, native026,
source-ambient candidate. No raw GPU buffers or duplicate database were stored.

## Shader identities across 020e and 021d

`audit_shader_parity.py` compares DXBC containers after stripping the archive
metadata prefix. The saved audit covers 81 original020e permutations and 29
previously observed021d shaders. Twelve are byte-identical: all eight aberration
permutations, atmosphere, atmosphere clouds, complex-post-process blend and
SSAO temporal upsample. Atmosphere SHA-256 is
`6bf4683c2863d224d4fa236b6559aeb1500baff357c12a8cc47f7e962628d633`;
clouds is `98013dc55c5fa0f8cecfd14274df1f91957085cb6de4954ad3fc3a8d31002554`.
Composition, probes and ambient apply do not match. This supports investigating
the existing School sky/cloud adapters, but does not prove constant-buffer
bindings or visual parity. The old010 duplicate-grain fix must not be assumed
applicable to020e.

The School adapters themselves are not reusable wholesale: its sky shader is a
constant `(3,3,3)` justified by saturated rain captures, and its cloud shader
hardcodes that map's weather. Radar020e authors cloud blend mode1, rotation316.74,
speed0.1, RGB1.372, multiplier1, fog power2 and upper limit0.25. School uses mode0.
The source cloud family contains two distinct pixel shaders: unlit is the
1656-byte identity above; lit is2072 bytes, SHA-256
`89c11b1a3c1f93cee86c92df59c92cc001b6ac4f9ff13eb0ae65855474d6c3cb`.
`prepare_sky_audit.py` extracts all source/native sky/cloud variants and their
reflection into a small offline audit, without launching a client or writing
the database. The shader permutation is selected by the separate lighting
switch, not the blend-mode render state.

The source020 constant-only `skylight.ps` is already byte-identical to native026
(1056 bytes, SHA-256
`7ef4a1f49ee98f70dd76ecb8681da70c933bae5334060cfe2623b736cc423971`).
PDB `render_skylight` at VA0x10c42f fills its target with a stage scalar. This is
not the early010 skylight contract; adding that early shader would be unjustified.
Authored sun elevation26.07 and azimuth222.2 survive conversion, as do the nearby
glass material's cast-shadow flags. The missing sunlit stair patch remains an
open renderer/shadow difference, not a proven wrong sun angle.

### Proven cloud-lighting default and guarded candidate

The converted environment inherited `use_sky_clouds_lighting=true`, native
normal texture `sky/sky2_nmap`, cloud skylight1.46 and sunlight0.94. The original
project does not author those fields. Source absence alone was insufficient:
the020 executable does support a simpler cloud-lighting switch. Its exact loader
at VA0x20439a checks the key and writes environment offset0x118 at0x2043b8 only
when present. Constructor0x2032aa zeros ECX and writes CL=0 to that offset at
0x2035a3. The atmosphere stage reads the same field at0x11de15 (scene+0x230,
environment base+0x118) when selecting the lit/unlit effect. Therefore this map
uses the original unlit1656-byte shader, despite its alpha-blend mode1.

The scoped020 converter repair applies the source default before template fill,
preserving explicit authored true/false values. The existing object-inheritance
regression covers all three cases; 11 focused tests pass without a test-count
increase. `prepare_cloud_lighting_patch.py` prepares a pinned one-boolean overlay
and verifies every other decoded value. Candidate:
`/tmp/factory-020e-cloud-lighting-patch/bundle`. Original and patched project both
have1128262 bytes; SHA-256 changes from
`4d99a7c02e6846e82f483bed5470b655a15bf73fb1cbba570e9e665d1f77a765` to
`fce5eff2c8c69a64b41c6db2fab3e85f1da68b350fba12097f693268a8e55112`.
The guarded four-view comparison applied this single byte above the accepted
source ambient behavior, then restored the full database and both preset guards.
The final patch, including archive CRC guards, is
`/tmp/factory-020e-cloud-lighting-patch-v2`. The sky-view error against the frozen
original fell from0.065204 to0.0525691 (19.4%); the other three changed by less
than0.0009. Electrical and foliage phases are not synchronized. Mean error for
these four views fell from0.062558 to0.059860. This is a useful source-default
repair, not complete sky parity: native unlit clouds still use different fog
equations, and the stair direct-light deficit remains.

Evidence is frozen under the private package's
`evidence/cloud-default-comparison-20261014`: four PNGs with exact camera
receipts, source/baseline/candidate sheet, metrics, logs, patch guards and full
database restoration proof. The tested recipe is staged at
`/tmp/factory-020e-accepted-recipe`, with relative preset asset paths, source
ambient bytecode, tested hook, native configuration and the one-resource
cloud-default overlay. Its manifest pins every file. All clients and the
isolated Wine server stopped before restoration; the shared database retained
SHA-256 `241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d`.

The historical `hangar-stairs` evidence ID is inside the antenna building.
Keep that ID for reproducible comparisons; do not infer the model from its name.

### Parameterized cloud-coordinate and fog candidate

Source020 uses both stratosphere and generic cloud layers, with radii200000 and
100000. PDB identifies sine at VA0x29d236 and cosine at0x672ed6. Source calls at
0x11dfdb/0x11dff0 and0x11e204/0x11e219 store `(cos,sin,speed,radius)`. Native026
calls at0x1a4d3e/0x1a4d4e and0x1a4feb/0x1a4ffb instead store
`(sin,cos,speed,radius)`: its helper0x38ff0 contains `fsin`, helper0x39000 `fcos`.
Both vertex shaders use the same rotation arithmetic and0.96 V scale. This is
a general coordinate-convention difference, including Radar's316.74-degree
clouds and zero-degree stratosphere; School's fixed quarter-turn is insufficient.

The default-off `SRP_LEGACY_020E_CLOUDS=1` candidate swaps these four guarded
native calls, leaving authored angles, speeds and radii intact. Its paired
`factory_020e_clouds.hlsl` restores the source1656-byte unlit PS equation with
native texture t1 and unchanged b0 weather registers: three mip samples, authored
color multiplier, latitude fog power/upper limit and fog color. It contains no
Radar-specific color or angle constants. The hook compiles with
`-Wall -Wextra -Werror`; the capture harness requires both coordinate and shader
override receipts. Native comparison is pending. This adapter does not change
the separate atmosphere scattering shader or resolve the stair shadow difference.
