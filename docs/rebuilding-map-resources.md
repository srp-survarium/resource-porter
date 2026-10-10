# Rebuilding the map resources in `.work`

`.work/` is disposable scratch. It is gitignored, it held ~174 GiB of built map
databases and their intermediates, and on 2026-08-28 all of the map output was
deleted to reclaim space. Nothing of value was lost: every deleted file is a
build product of committed inputs and the commands below. This document is the
recipe for getting any of it back.

## The recoverability model

A built map database is a pure function of three things, none of which live in
`.work`:

| source of truth | where it is | recoverable how |
| --- | --- | --- |
| the porter + the manifests | this repository (`src/`, `ports/*.json`) | git |
| the retail game installs | Steam (SoC, Clear Sky, F.E.A.R., Dirty Bomb) | reinstall |
| the v0.100b target `resources.db` | `nix build .#survarium.resources` in `../vostok` | the flake fetches + hashes it |

Given those three, the four-stage pipeline (build → compose → apply → verify)
regenerates any release database byte-for-deterministically. `.work` is only
where the intermediate and final artifacts are staged; deleting it costs time,
never information.

> **One gap to close first.** The manifests for the newer families are
> **untracked in git**: `ports/dirty-bomb-{overground,canal,gallery,castle}-to-0100b.json`
> and `ports/cs-training_camp-to-0100b.json`. They are the *only* non-recoverable
> input for those maps — the porter cannot re-derive them. Commit them (and the
> untracked `src/dirty_bomb/`, `src/soc_addon.rs`, `src/sdk.rs`, `src/bin/`)
> before relying on this document, or those maps stop being build-recoverable.

## What was kept in `.work`

These are not map resources and were left in place:

* `vostok-sdk-v0100b/` (14 GiB) — a checkout of `srp-survarium/vostok` with
  **uncommitted local edits** to `sources/*.cpp` and `docs/`. Not regenerable
  from git alone; do not delete without committing those edits first.
* `stk2-content-srp-worktree/` (5.9 GiB) — the SRP runtime the launcher runs
  from, including the `target/debug` match/lobby/login/browser servers the
  dirty-bomb profiles point at. Rebuildable with `cargo build`, but kept.
* `stk2-content-worktree/`, `srp-stack-content-worktree/` — SRP content trees.
* `client-v0100b-soc/` (36 MiB) — the base v0.100b client (`BASE_CLIENT_DIR`),
  extracted from the installer, not a build product of this repo.
* `clips/` (512 KiB) — converted ladder animation splines from an investigation,
  with no documented regen command.

Everything else under `.work` was deleted: all release `*.db`, all
`*-bundle*`/`*-apply*`/`*-composed*`/`*-pack*` intermediates, all `*-logs/`,
`*.log`, `*.out`, the derived `*-conversion*.json` / `*-probe*.json` stat dumps,
the `sdk-decompiled*`/`sdk-preview*` dumps, and the per-run `client-map-*`
symlink dirs. The Wine prefix `wineprefix-map-demo/` was deleted too — the
launcher recreates it (`wineboot -u`) on first run.

## Shared setup

```sh
SOC="$HOME/.local/share/Steam/steamapps/common/STALKER Shadow of Chernobyl"
CS="$HOME/.local/share/Steam/steamapps/common/STALKER Clear Sky"
FEAR="$HOME/.local/share/Steam/steamapps/common/FEAR Ultimate Shooter Edition"
DB="$HOME/.local/share/Steam/steamapps/common/Dirty Bomb/ShooterGame/CookedPC"
TARGET=$(nix build --no-link --print-out-paths ../vostok#survarium.resources)/resources.db
sha256sum "$TARGET"   # 160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409
```

## The demo script pins exact version numbers

`scripts/run-map-demo.sh` looks for specific filenames. A rebuild must land the
output at these paths (or the script must be updated to point at the new names),
or the profile fails its `--check` with "missing map runtime artifact":

| profile(s) | expected database |
| --- | --- |
| `soc-*` | `.work/soc-release/${SOC_DB:-resources-v0100b-soc-maps-v1.db}` — pass `SOC_DB=resources-v0100b-soc-maps-v4.db` for the canonical v4 |
| `cs-training-camp` | `.work/cs-release-v2/resources.db` |
| `dirty-bomb-overground` | `.work/dirty-bomb-overground-resources-v30.db` |
| `dirty-bomb-canal` | `.work/dirty-bomb-canal-resources-v8.db` |
| `dirty-bomb-gallery` | `.work/dirty-bomb-gallery-resources-v9.db` |
| `dirty-bomb-castle` | `.work/dirty-bomb-castle-resources-v1.db` |
| `fear-*` | `.work/fear-release/${FEAR_DB:-resources.db}` |

`scripts/run-map-demo.sh <profile> --check` validates the database and runtime
layout without starting Wine — run it after any rebuild.

## Shadow of Chernobyl — the thirteen-map release

Full recipe and per-map counters: [`the porting guide`](../the porting guide) ("The four
commands"). In brief:

```sh
for m in ports/soc-*.json; do
  n=$(basename "$m" .json)
  vostok-port build-soc-map --manifest "$m" \
    --source-root "$SOC" --target-db "$TARGET" --output ".work/bundles/$n"
done
vostok-port compose $(printf -- '--bundle .work/bundles/%s ' *) --output .work/composed
vostok-port apply --base-db "$TARGET" --bundle .work/composed \
  --work-dir .work/apply --output-db .work/soc-release/resources-v0100b-soc-maps-v4.db
vostok-port verify --db .work/soc-release/resources-v0100b-soc-maps-v4.db --bundle .work/composed
```

Land the output at `resources-v0100b-soc-maps-v4.db` (SHA-256
`f4ec46baf66794c334013ade2c7853bffa35aafaf5b38b8203aa6520b5c71c5c`), the release
the porting guide and the README call current.

## Clear Sky — `cs-training-camp`

Clear Sky is another X-Ray 1.x source, read by the same `src/soc` codec, so it
uses `build-soc-map` with a Clear Sky `--source-root` and the SoC-schema
manifest. The playable `.work/cs-release-v2/resources.db` is the base build; the
`cs-release-terrain-*-meadow-*` directories were meadow-addon experiments layered
on top with `build-soc-map-addon` (see `src/soc_addon.rs`).

```sh
vostok-port build-soc-map --manifest ports/cs-training_camp-to-0100b.json \
  --source-root "$CS" --target-db "$TARGET" --output .work/cs-bundles/training_camp
vostok-port compose --bundle .work/cs-bundles/training_camp --output .work/cs-composed
vostok-port apply --base-db "$TARGET" --bundle .work/cs-composed \
  --work-dir .work/cs-apply --output-db .work/cs-release-v2/resources.db
vostok-port verify --db .work/cs-release-v2/resources.db --bundle .work/cs-composed
```

The optional meadow addon (grass/conifer/wildflower batches) is a second pass:

```sh
vostok-port build-soc-map-addon --source-root "$CS" --level mp_training_camp \
  --base-db .work/cs-release-v2/resources.db --flower-png <atlas.png> \
  --output .work/cs-release-terrain-v4-meadow/resources.db
```

## F.E.A.R. — the twenty-four-world release

Full recipe: [`the porting guide`](../the porting guide) ("The F.E.A.R. port (lilith)") and
[`docs/fear-map-porting.md`](fear-map-porting.md). In brief, one
`build-lilith-map` per world, then the same compose / apply / verify:

```sh
for m in ports/lilith-*.json; do
  n=$(basename "$m" .json)
  vostok-port build-lilith-map --manifest "$m" \
    --source-root "$FEAR" --target-db "$TARGET" --output ".work/lilith-bundles/$n"
done
vostok-port compose $(printf -- '--bundle .work/lilith-bundles/%s ' *) --output .work/lilith-composed
vostok-port apply --base-db "$TARGET" --bundle .work/lilith-composed \
  --work-dir .work/lilith-apply --output-db .work/fear-release/resources.db
vostok-port verify --db .work/fear-release/resources.db --bundle .work/lilith-composed
```

## Dirty Bomb — four UE3 maps

Full recipe: [`README.md`](../README.md) ("Build a Dirty Bomb map"). Each map is
one bundle built and validated against the base:

```sh
vostok-port build-dirty-bomb-map \
  --manifest ports/dirty-bomb-overground-to-0100b.json \
  --source-root "$DB" --target-db "$TARGET" \
  --output .work/dirty-bomb-overground-bundle
vostok-port validate-overlay --base-db "$TARGET" \
  --overlay .work/dirty-bomb-overground-bundle/overlay
```

Then apply the bundle onto the base to produce the full database the demo script
names (`.work/dirty-bomb-<map>-resources-v<N>.db`); repeat for `canal`,
`gallery` (`EXE_Gallery`), and `castle` (`OBJ_Castle`). Probe/inventory commands
(`dirty-bomb-inventory`, `dirty-bomb-probe-map`, `dirty-bomb-probe-conversion`)
regenerate the deleted `*-probe*.json` / `*-conversion*.json` stat dumps.

## Every family in one database — the offline playtest package

`scripts/build-all-maps-package.sh` builds the twelve SoC multiplayer maps and
Cordon, the Clear Sky Training Camp with its summer meadow, the outdoor
F.E.A.R. worlds (`FEAR_WORLDS`, default `asylum train_yard deadwood`), and the
Survarium 0.1.1e / 0.20f / 0.23h levels, checks every project's collision
(`prune-collision`), and applies them onto untouched retail in one verified
chain. Every map is built with `--camera-at-spawn`, because the offline client
inserts the player at the client project's `camera.position`. The donor
databases are the ones `map-porting.txt` inventories (0.1.1e `694e1772…`, 0.20f
build 1923 `5f871046…`, 0.23h `8e5a1530…`).

Its inputs default to the archive drive (`ARCHIVE`, `/mnt/toshiba/surv-resources`):
`donor-games/stalker-soc` and `donor-games/fear` are copies of the Steam
install roots with the same layout (each beside a `.sha256` manifest), and the
Survarium donors sit at `0.1.1e/app`, `0.20f` and `0.23h`. Set `SOC`, `FEAR`,
`CS` or `DONORS` to point anywhere else. The F.E.A.R. copy made on 2026-10-10
lacks `FEARXP2/FEARM_XP.arch00` (lower-case extension), which holds the
multiplayer worlds' data: until it is added, `build-lilith-map` stops at
`Default.archcfg lists FEARM_XP.Arch00`.

| release | SHA-256 | what changed |
| --- | --- | --- |
| `release/` | `573c5d1c…a4` | first package; `level_04_011e` crashes on load |
| `release-v2/` | `80d0641d…25` | summer Training Camp, `level_04_011e` left out |
| `release-v3/` | `e3737dc1…64` | X-Ray terrain composition on SoC maps, collision pruning, `level_04_011e` back |
| `release-v4/` | `1e47d55d…c0` | X-Ray foliage without wind, normal maps repacked to DXT5nm, donor game-material ids rebound by name |

`release-v3` and `release-v4` carried the Training Camp over from `release-v2`
with `CS_FROM_DB` (`extract-bundle`), because the Clear Sky install was not
present; its foliage wind and normal packing are therefore still v2's.
`release-v4`'s `fear_asylum` and `fear_deadwood` are v3's bundles for the same
reason (the archive copy lacks `FEARM_XP.arch00`); `fear_train_yard` was built
with the repacked normals before the install was removed.

### Why maps crashed on load

The client cooks every `collision_objects[].lib_name` (class 32). A name with
**no node at all** fails the cook and `static_collision::insert` builds a rigid
body from the NULL shape (`physics::create_static_rigid_body` +0xb2, read of
0x108). An *empty* folder is fine — retail level_03 names three. The first
package hit this on `level_04_011e`
(`resources/models/flora/bush_s_002_sux.model/collision`,
`resources/models/props/cover/tyres_s1.model/walker_collision`,
`resources/models/props/cover/tyres_s2.model/walker_collision`) because compose
let a 0.20f bundle that only renders those models own them whole, and on
`level_05_day_020f` (`barrel_stove_01_fire.model/collision`, a donor hard link
no bundle carries). Compose now adopts a later bundle's collision folder when
the owner has none, and `prune-collision` drops what still cannot cook:
32 / 79 / 7 / 1 objects on levels 01 / 02 / 04 / 05 day 0.20f.
`audit-collision --db` repeats the check on any packed database.

## Running a rebuilt map

```sh
BASE_CLIENT_DIR=$PWD/.work/client-v0100b-soc \
SOC_DB=resources-v0100b-soc-maps-v4.db ./scripts/run-map-demo.sh soc-rostok
```

SRP must be built and running first; the launcher recreates the deleted Wine
prefix on its first launch. See the top-level README and the porting guide for the SRP
side (a map profile arm in `srp/servers/match-server/src/game.rs` and a case in
`scripts/run-map-demo.sh`).
