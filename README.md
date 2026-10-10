# Vostok resource porter

Tools for converting maps and assets into additive resource bundles for
Survarium v0.100b / build 802. The porter reads source game files, converts
geometry, collision, textures and configuration, then overlays the bundle onto
a target `resources.db`. Every emitted resource is recorded with its origin,
size and SHA-256 and can be verified after packing.

Supported conversion paths include:

- S.T.A.L.K.E.R.: Shadow of Chernobyl maps and Clear Sky Training Camp;
- F.E.A.R. multiplayer worlds;
- Dirty Bomb cooked UE3 maps;
- later Survarium maps, weapons and other assets;
- STK2 static maps, weapons and character assets.

Support and visual fidelity vary by source and target build. Resource
verification checks the packed bytes; client playtesting checks rendering,
collision and gameplay. Supply game installations and databases separately.
Converted bundles and client binaries are not included.

## Build

The archive reader and writer are included under `deps/vostok-resources-db`;
no submodule initialization is needed. Clone the repository, then build with
a stable Rust toolchain and a C linker installed:

```sh
git clone https://github.com/srp-survarium/resource-porter.git
cd resource-porter
cargo build --locked --release
cargo test --locked --all-targets
export PATH="$PWD/target/release:$PATH"
vostok-port --help
```

The binaries are `vostok-port` and `vostok-sdk-backend`. On Linux, `nix develop`
provides Rust, a C compiler, ImageMagick and the runtime tools. ImageMagick's
`magick` command is required for normal-map repacking and image conversion.
Building and running unit tests does not require game data, Wine or SRP.

## Build a map bundle

For Shadow of Chernobyl, `--source-root` accepts either the retail installation
containing `gamedata.db0` through `gamedata.dbd` or an unpacked `gamedata` tree:

```sh
vostok-port build-soc-map \
  --manifest ports/soc-cordon-to-0100b.json \
  --source-root /path/to/STALKER \
  --target-db /path/to/v0100b/resources.db \
  --output .work/cordon-bundle
```

For F.E.A.R., use the installation directory containing `Default.archcfg`:

```sh
vostok-port build-lilith-map \
  --manifest ports/lilith-office-to-0100b.json \
  --source-root /path/to/FEAR \
  --target-db /path/to/v0100b/resources.db \
  --output .work/office-bundle
```

For Dirty Bomb, point at the cooked package directory:

```sh
vostok-port build-dirty-bomb-map \
  --manifest ports/dirty-bomb-overground-to-0100b.json \
  --source-root /path/to/DirtyBomb/ShooterGame/CookedPC \
  --target-db /path/to/v0100b/resources.db \
  --output .work/overground-bundle
```

Each bundle contains an `overlay/` virtual resource tree and a
`port-manifest.json` provenance record. The reviewed manifests in `ports/`
declare source levels, namespaces, target templates and conversion overrides.

## Pack and verify

```sh
vostok-port apply \
  --base-db /path/to/v0100b/resources.db \
  --bundle .work/cordon-bundle \
  --work-dir .work/cordon-apply \
  --output-db .work/cordon-release/resources.db

vostok-port verify \
  --db .work/cordon-release/resources.db \
  --bundle .work/cordon-bundle
```

`apply` validates and packs the overlaid tree, then reopens the database and
checks the bundle's hashes. Existing work/output paths require `--force`.

Compose multiple bundles before applying. The first bundle wins shared paths:

```sh
vostok-port compose \
  --bundle .work/cordon-bundle \
  --bundle .work/office-bundle \
  --output .work/composed
```

## Inspect source data

These commands inspect Shadow of Chernobyl without a target database:

```sh
vostok-port soc-inventory --install-root /path/to/STALKER
vostok-port soc-probe-level --source-root /path/to/STALKER --level all
vostok-port soc-audit-shaders --source-root /path/to/STALKER --level all
```

Use `vostok-port COMMAND --help` for other source families, weapon and armor
conversion, material patches, dependency scans and model validation.
`vostok-sdk-backend` exposes map decompile/preview operations through JSON
requests; examples are in `examples/sdk/`.

## Run in the client

The scripts under `scripts/` support local Wine/SRP demos. They require a
complete build 802 Windows client, the matching database, an external SRP
checkout and built server binaries. Place SRP at `../srp` or set
`SRP_REPO_DIR`; server content and map profiles must match the database and
spawn points.

The Linux launchers use unprivileged user/network namespaces so the browser
service can bind port 80. Wine needs 32-bit graphics support and X11/XWayland.
Logs and runtime data live under `.work/`. Use throwaway login credentials:
experimental SRP versions may log them.

## Documentation

- [Shadow of Chernobyl conversion](docs/soc-map-porting.md)
- [F.E.A.R. conversion](docs/fear-map-porting.md)
- [Rebuilding map databases](docs/rebuilding-map-resources.md)
- [Engine resources and converter architecture](docs/engine-resource-primer.md)
- [Water conversion](docs/porting-water.md)
- [Retargeting another Survarium build](docs/retargeting-a-later-build.md)
- [SDK backend](docs/sdk-recovery.md)
- [Texture defect reports](docs/texture-defect-vocabulary.md)
- [Contributing](CONTRIBUTING.md)

## License

The project's code, manifests, documentation and included generated flower
atlas use the [GNU General Public License, version 3 or later](LICENSE),
matching Vostok. See [THIRD_PARTY.md](THIRD_PARTY.md) for dependency provenance.

Input game data and assets copied or converted into output bundles remain
subject to their rights holders' terms. The project license covers the
contributors' work and does not grant rights to those game assets.
