# Vostok resource porter

`vostok-port` builds explicit, reproducible cross-version asset bundles for the
Survarium v0.100b / build 802 client. The goal is to add weapons and maps as new
content, not replace old resources.

The repository deliberately keeps two layers separate:

- `deps/vostok-resources-db` reads and writes the Vostok archive container;
- this tool owns dependency discovery, binary-config transformations,
  version-specific schemas, dictionary IDs, localization, manifests, and
  validation.

## Current vertical slice

`ports/akmn-020e-to-0100b.json` adds:

- AKMN as dictionary ID 74;
- 7.62x39 ammunition as dictionary ID 75;
- old-schema weapon/ammunition configs generated from v0.100b templates;
- AKMN model, both body and wooden-stock materials, textures, and skeleton from
  v0.20e;
- Russian localization entries;
- a provenance manifest containing every output file's size and SHA-256.

`ports/sks-020e-to-0100b.json` then adds SKS as dictionary ID 76. It reuses
ammunition ID 75, converts the newer weapon definition to the old schema, and
copies the SKS model, material, textures, original animation skeleton, and SKS
item/reload animations. The first-pass configuration is semi-automatic with a
10-round magazine. The old state machine is retained, but its AK-74u item and
reload paths are translated to compatible SKS donor animations. Its material is
regenerated from the old `ak74_u` material schema with SKS texture
substitutions; copying the newer material directly left `shader_source` null in
the v0.100b renderer.

`ports/dolg-exoskeleton-stk2-to-0100b.json` adds the cancelled STK2 Duty
exoskeleton as torso armor ID 77. The importer reads its nested
`.skinned_model` archive and reproduces Vostok's PC hardware-skinning
conversion for all six surfaces. The first runtime bundle deliberately uses
existing v0.100b character materials and the `scavengers_01` animation
skeleton; original texture conversion follows after geometry and bone-order
validation.

It intentionally shares the v0.100b AK animation, particle, and sound resources
for the first runtime milestone. Those are shared dependencies, not replacement
assets. Dedicated donor sounds, animation conversion, and inventory-atlas
entries can be introduced after the new items load and fire reliably. AKMN
currently uses the old AK-74u inventory icon; SKS requests donor icon index 10,
which still needs an old-client atlas check.

## Build

Clone with the archive-format submodule:

```sh
git clone --recursive <repository>
cd vostok-resource-porter
nix develop
cargo build --release
```

## Build weapon bundles

```sh
vostok-port build-weapon \
  --manifest ports/akmn-020e-to-0100b.json \
  --target-db /path/to/v0100b/resources.db \
  --donor-db /path/to/v020e/resources.db \
  --output .work/akmn-bundle
```

This creates:

```text
.work/akmn-bundle/
  overlay/              generated and copied virtual resource tree
  port-manifest.json    provenance, hashes, IDs, dependency report
  server/
    srp-content.json    additive SRP catalog and test loadout
```

The command rejects occupied dictionary IDs, existing localization keys,
missing donor resources, unknown manifest fields, unsafe paths, unsupported
text encodings, and nonempty output directories.

Build SKS against the already-ported AKMN database so its manifest can verify
the shared ammunition entry:

```sh
vostok-port build-weapon \
  --manifest ports/sks-020e-to-0100b.json \
  --target-db .work/resources-v0100b-akmn.db \
  --donor-db /path/to/v020e/resources.db \
  --output .work/sks-bundle
```

## Build and verify a target database

```sh
vostok-port apply \
  --base-db /path/to/v0100b/resources.db \
  --bundle .work/akmn-bundle \
  --work-dir .work/akmn-pack \
  --output-db .work/resources-akmn.db
```

`apply` extracts the target tree once, overlays the bundle, packs it using the
pinned v0.100b writer, then reopens the result and verifies every bundle entry
against its recorded SHA-256. Existing output/work paths are not reused unless
`--force` is supplied.

## Build the STK2 exoskeleton

Extract only the externally supplied compiled model with the CLI:

```sh
7z x -y -o.work/stk2-character-source /path/to/stk_2.7z \
  resources/exported/models/character/human/dolg/dolg_exo.skinned_model
```

Then build the additive armor bundle against the current combined database:

```sh
vostok-port build-armor \
  --manifest ports/dolg-exoskeleton-stk2-to-0100b.json \
  --donor-model-archive \
    .work/stk2-character-source/resources/exported/models/character/human/dolg/dolg_exo.skinned_model \
  --donor-texture-dir \
    .work/stk2-character-source/resources/sources/textures/character/human/dolg \
  --target-db .work/resources-v0100b-akmn-sks.db \
  --output .work/dolg-exoskeleton-bundle
```

The converter validates four-weight source geometry, retargets it against the
v0.100b bind pose, converts 76-byte donor vertices to the 48-byte hardware
layout, compiles target-schema model/material settings, converts STK2 texture
pixels to DXT1 and DXT5nm DDS resources with target `.options`, and records
every output hash.

## Other useful commands

```sh
vostok-port inventory --db /path/to/resources.db

vostok-port stat \
  --db /path/to/resources.db \
  --path game_materials/game.materials

vostok-port scan \
  --target-db /path/to/v0100b/resources.db \
  --donor-db /path/to/donor/resources.db \
  --root gameplay/items/weapons/wpn_akmn.options

vostok-port verify \
  --db .work/resources-akmn.db \
  --bundle .work/akmn-bundle
```

## Runtime integration

The packed client resource is only one half of a genuinely new item. SRP branch
`feature/akmn-content`, based directly on `0.100b`, adds weapon IDs 74 and 76
and ammunition ID 75 to the lobby inventory, declares both compatibility
pairs, and exposes all three IDs through the shop catalog. It also adds
exoskeleton armor ID 77. Commits `a86099f33`, `3627de347`, and `72b4405a6`
contain those changes and their preservation tests. Existing weapons,
ammunition, and armor remain available.

Each generated bundle also includes `server/srp-content.json` as a
machine-readable additive catalog for batch importers and newer SRP consumers.
The tested `0.100b` branch predates runtime content-manifest support, so the
current launcher uses the dedicated SRP branch above.

## Inputs and host prerequisites

The repository does not redistribute retail game data or SRP. Supply these
outside Git:

- a complete v0.100b/build 802 Windows client;
- its original `resources.db`;
- a v0.20e `resources.db` for the current AKMN donor;
- an SRP checkout at `../srp`, or set `SRP_REPO_DIR`;
- the four SRP binaries built from `feature/akmn-content`;
- SRP's `certs/survarium_login_server.{crt,key}` files.

The Linux runtime needs a Wine build capable of running 32-bit Windows
programs, working 32-bit graphics-driver libraries, X11 or XWayland, `unshare`,
`ip`, `ss`, and `curl`. Unprivileged user namespaces must be enabled. Entering
`nix develop` supplies the command-line tools and Wine; GPU driver integration
remains a host-system responsibility.

Use throwaway login details. The experimental v0.100b SRP login server currently
writes the supplied account name and password to its local log.

## Local v0.100b runtime layout

Large generated artifacts stay out of Git under `.work/`, but remain in the
`surv` project tree:

```text
.work/
  akmn-bundle/
  sks-bundle/
  client-v0100b-akmn/
  resources-v0100b-akmn-sks.db
  srp-0100b-target/debug/
  wineprefix/
```

On Linux, the stock client requires the browser service on privileged port 80.
The launcher puts SRP and Wine in one unprivileged user/network namespace, where
that port can be used without `sudo`, capabilities, or host firewall changes:

```sh
./scripts/run-akmn-demo.sh
```

The runtime server is built from SRP branch `feature/akmn-content`, based
directly on `0.100b`. Its four logs are written to `.work/srp-0100b-logs/`.
The client log is `.work/client-v0100b-akmn.log`. The launcher kills any
prefix-local wineserver before entering the namespace; this is required so
Wine and SRP genuinely share the same `127.0.0.1`. Closing Wine also terminates
all four private server processes.

Before starting SRP or Wine, the launcher patches build 802's retired
Vostok Games crash-report recipient to the non-resolving
`crash-reports@localhost.invalid` address. It refuses an unknown executable and
retains a stock-image backup. See
[`docs/crash-report-email-safety.md`](docs/crash-report-email-safety.md) for
the exact bytes, hashes, and standalone verification command.

The private namespace includes both loopback and a dummy `10.100.0.1/24`
adapter. With loopback alone, build 802 under Wine stayed at `Connecting...`
without opening a login connection, although SRP was listening. The dummy
adapter reproduces the non-loopback interface available on Windows. An HTTP
probe is run before Wine starts and retained at
`.work/browser-loopback-probe.txt`; startup aborts if it cannot reach SRP.

The tested path is:

1. launch with `./scripts/run-akmn-demo.sh`;
2. enter any throwaway email-shaped account and password;
3. allow the first TLS attempt to retry if necessary;
4. inspect `.work/srp-0100b-logs/` if the lobby does not appear.

The successful Linux run reached login, browser, and lobby services. The client
then equipped inventory instance 48 / dictionary ID 74 (AKMN) and instance 49 /
dictionary ID 75 (7.62x39), confirming that both added resources are recognized
by the v0.100b client. The current SRP branch also issues instance 50 /
dictionary ID 76 (SKS) and instance 51 / dictionary ID 77 (Duty exoskeleton).

## Canonical retail → STK2 map + weapons demo

The canonical combined demo has one explicit resource chain:

```text
untouched retail v0.100b resources.db
  + runtime-tested STK2 A01 map module
  + retail-derived STK2 tir1/tir2/tir3 weapon module
  = one verified resources.db used by the matching SRP map profile
```

Build, validate, and launch the complete chain with:

```sh
./scripts/run-canonical-stk2-a01-weapons-demo.sh
```

Use `--build-only` to stop after database verification, or
`--rebuild-weapons` to regenerate the weapon module from the extracted STK2
source. The script refuses a retail database whose SHA-256 is not
`160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409`.
It intentionally excludes the exoskeleton and all earlier AKMN/SKS experiments.
The SRP `stk2-weapons` profile gives tir1 and tir2 to the local player and tir3
to the remote test character.

An extended checkpoint keeps that database intact and adds an exoskeleton
module built against its final dictionary:

```sh
./scripts/run-canonical-stk2-a01-mixed-roster-demo.sh
```

The `stk2-mixed-roster` profile gives the local player stock Black Market
equipment with STK2 exoskeleton gloves, an AKS-74U, and a Remington 870. Two
complete STK2 Duty exoskeleton guards carrying tir1 and tir2 start symmetrically
in front of the player at the railroad. Seven additional stock/mixed references
are distributed across A01 with tir3 and a selection of stock weapons, for ten
characters in total. Client suicide is handled by SRP and respawns the local
player on the central concrete structure after ten seconds. Use `--reuse` to
launch the runtime-verified database (SHA-256
`5cfecbd2f850ddefc2fe1f01ba42ecf63c512fb9f562e25b2a6a9a7b40384e3f`)
without rebuilding the resource chain.

The subsequent masonry checkpoint fixes the close-range concrete, brick, floor,
metal-tile, ruberoid, and wood-roof transition without changing the verified
map/content/server chain:

```sh
./scripts/run-canonical-stk2-a01-masonry-nearfix-demo.sh
```

Use `--build-only` to rebuild and validate the material-only patch, or `--reuse`
to launch the runtime-tested database (SHA-256
`4a51b1a80540e93766851674c9c4e9041960a66ef15f06139a6183f2e0795174`).
The patch uses a v0.100b material template that supports static vertex-color
meshes while disabling the unrelated vertex-blended dirt layer. Other
vertex-painted props and debris remain unchanged intentionally.

## STK2 character experiments

The later Survarium databases contain five exoskeleton material definitions and
one Bloodsucker-eye material, but not the corresponding model and textures.
The complete source is in the externally supplied `stk_2.7z`/`STK2.7z`
archives:

- exported `dolg_exo.skinned_model` and `dolg_exo_static.model`;
- exported `krovosos/krovosos.model`;
- exoskeleton and Bloodsucker materials;
- their high-resolution source textures;
- Bloodsucker rig/pose source scenes.

These archives must be indexed and selectively extracted with the `7z` CLI;
the launcher never opens a desktop archive program. The intended first pass is
deliberately permissive: add exoskeleton as switchable armor and Bloodsucker as
a switchable whole-body player visual, retain their original available rig
data, and accept attachment, IK, camera, and hitbox bugs. Neither replaces the
normal player assets. See `docs/stk2-character-assets.md` for exact paths and
conversion details.

The next importer stages are:

1. runtime AKMN load/equip/fire validation;
2. dedicated sound and icon import;
3. generalized batch weapon manifests;
4. static-model conversion for STK2;
5. render-only map conversion, followed by collision and gameplay objects.
