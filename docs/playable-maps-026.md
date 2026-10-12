# Playable maps for Survarium 0.26g

This collection combines map builders, a client patcher and a two-client local
launcher for **0.26g0 build 2777**. Supply the game databases, Windows client and
[SRP development match server](https://github.com/srp-survarium/srp) locally.
The repository contains source and recipes; generated game resources and
executables are not included.

## Choose a map set

| Build profile | Database contents | Match map ID | Runtime evidence |
| --- | --- | --- | --- |
| `fog-school` | Native School/day with overcast lighting and dense volume fog | 67 | Two clients; user confirmed stable iron-sight aiming with noise disabled. |
| `021-maps` | All native maps plus six imported 0.21 maps, each in three weather variants | 160–177 | All six imported day maps, level_03/rain and School/rain loaded; remaining variants are build/audit checked. |

These profiles produce separate databases. The fog variant changes native
School/day, while the importer adds namespaced maps. For imported School use
163 (day), 164 (evening) or 165 (rain). See the [complete ID table and conversion
limits](survarium-021-to-026.md#registration-and-runtime). Physical deposits in
the imported battery collectors have not been verified.

Fog uses native 0.26 shaders with `density = 0.06`,
`transparency_multiplier = 0.85`, `height_falloff_offset = 0.01` and
**`noise_scale = 0.0`**. Noise at 0.8 caused extra whitening during iron-sight
aiming; disabling it resolved the issue in the user's playtest. Dense fog and
height falloff remain, but animated noise is omitted. Magnified scopes have
not been tested. See the [investigation](survarium-021-to-026.md#native-026g-school-with-dense-volume-fog).

## Build and verify

Requirements: stable Rust, a C linker and Python 3.9 or newer. Build the porter:

```sh
cargo build --locked --release
```

The native target `resources.db` must have SHA-256
`c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c`.
The optional 0.21d donor must have SHA-256
`e28cc2878e0e2f04ab9ab0a1861034622af487a491bf230ea24f44e9af9715a4`.
The builders reject other inputs. Always start from retail, not an earlier
modified database.

```sh
python3 scripts/build-026-maps.py fog-school \
  --target-db /path/to/retail-026g/resources.db \
  --output .work/fog-school

python3 scripts/build-026-maps.py 021-maps \
  --target-db /path/to/retail-026g/resources.db \
  --donor-db /path/to/retail-021d/resources.db \
  --output .work/021-maps
```

Each command builds an additive `bundle/`, packs `resources.db`, and reopens it
to verify the emitted hashes. Use a new output directory each time. Allow space
for the bundle and the complete database: the fog database is about 6.3 GB and
the imported map database about 9 GB. `--porter` selects a different compiled
porter. Lower-level build/apply/verify commands remain available for composing
custom workflows; these scripts keep the build fingerprint and map metadata.

## Prepare the client and server

Use a complete extracted 0.26g0 game directory with `binaries/x86/` and
`resources/` (including the original certificate and splash assets). Install
the SRP direct-match client as `binaries/x86/survarium-srp.exe`.
The reviewed executable has SHA-256
`45baf0a591cf6535a9f7afa073ae59551979d92904670a4e1adf70336c1004e9`.
This exact input is required: newer SRP patch combinations may differ and
require a separate compatibility review. Follow SRP's client setup instructions
for its direct-match/snapshot support and Wine/DXVK dependencies.

Build `match-server-026` in the external SRP checkout:

```sh
cargo build --locked --manifest-path ../srp/Cargo.toml -p match-server-026
```

The launcher applies `tools/patch-026-map-resources.py` to a copied executable.
The resulting `survarium-ported.exe` has SHA-256
`6a35b09ed3f9e48b18e19cdd4bf8ba8a5518eceeb75bba9049ad80150658e4cf`.
The patch skips the archive RSA/hash-verifier call; archive resource CRCs and
multiplayer state hashes remain enabled. The source game directory is untouched.

## Launch two clients

Requirements: Linux, an X11/XWayland display, working Wine graphics support,
and `wine`/`wineserver` from the same installation. A known-working, stopped
Wine prefix can be supplied with `--wine-prefix-template`; each client gets
its own copy. Without it, Wine initializes two fresh prefixes. DXVK DLLs, if
used, must be installed in the source client or template prefix.

```sh
python3 scripts/run-026-map.py \
  --client-dir /path/to/026g-game \
  --db .work/fog-school/resources.db --map 67 \
  --server ../srp/target/debug/match-server-026 \
  --work-dir .work/play-fog-school
```

For old School in rain, use `--db .work/021-maps/resources.db --map 165` and
a different work directory. Add `--check` to validate inputs and print the plan
without launching or creating files. Use `--wine`, `--wineserver`,
`--wine-prefix-template` and `--port` as needed; graphics environment variables
are inherited, with default DLL overrides for the reviewed Wine setup.

Both clients use English and the match starts when both have loaded. Keep the
launcher running; **Ctrl-C stops its server and its two Wine prefixes**. It
does not stop other Wine sessions. Logs, the copied client, prefixes and
`run.json` remain in the work directory. Use a new work directory for each run.
The native match server uses loopback port 2246 by default; no login/browser
service or privileged port is required.

The reviewed client chooses random sessions, so this launcher omits a fixed
roster. Imported map IDs have no reviewed finish/results fixtures; the launcher
also omits automatic match-finish flags. This is a local map playtest setup.

## Checks without game assets

```sh
cargo test --locked --all-targets
python3 -m unittest discover -s scripts/tests -p 'test_026_*.py'
python3 scripts/build-026-maps.py --help
python3 scripts/run-026-map.py --help
```

Archive verification establishes byte integrity. The playtest evidence above
describes individual maps and interactions, not complete visual or gameplay
coverage of all eighteen imported variants.

Publication checks: 270 porter tests, three archive-library tests and five
script tests passed. The `fog-school` wrapper completed a retail-to-packed
rebuild and verification; its project bytes match the user-tested noise-free
variant. That rebuilt database's SHA-256 is
`405e77810cd3e19e4180d0f6f21d7191c5a6f7d263e3a7bf0018b10498ca636b`.
The portable launcher's `--check` accepted the reviewed local client; its
process cleanup has automated coverage. The native playtests used the earlier
local launcher with the same server and client arguments.
