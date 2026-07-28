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
copies the SKS model, material, and textures. The first-pass configuration is
semi-automatic with a 10-round magazine. It deliberately uses the old
`weapons/ak_test` animation skeleton because the first run with the newer SKS
animation skeleton crashed immediately after equip.

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

## Other useful commands

```sh
vostok-port inventory --db /path/to/resources.db

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
pairs, and exposes all three IDs through the shop catalog. Commits `a86099f33`
and `3627de347` contain those changes and their preservation tests. Existing
weapons and ammunition remain available.

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
dictionary ID 76 (SKS).

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
the geometry-conversion boundary found during inspection.

The next importer stages are:

1. runtime AKMN load/equip/fire validation;
2. dedicated sound and icon import;
3. generalized batch weapon manifests;
4. static-model conversion for STK2;
5. render-only map conversion, followed by collision and gameplay objects.
