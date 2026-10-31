# Vostok resource porter

Map conversion tools and playable-map scripts for Survarium v0.100b / build 802
and v0.26g0 / build 2777. The porter reads source game files, converts
geometry, collision, textures and configuration, then overlays the bundle onto
a target `resources.db`. Every emitted resource is recorded with its origin,
size and SHA-256 and can be verified after packing.

Supported conversion paths include:

- S.T.A.L.K.E.R.: Shadow of Chernobyl maps and Clear Sky Training Camp;
- F.E.A.R. multiplayer worlds;
- later Survarium maps, weapons and other assets;
- STK2 static maps, weapons and character assets;
- retail 0.21d maps in 0.26g, plus a native 0.26g School variant with dense fog.

Support and visual fidelity vary by source and target build. Resource
verification checks the packed bytes; client playtesting checks rendering,
collision and gameplay. Supply game installations and databases separately.
Converted bundles and client binaries are not included.

## Build

The archive reader and writer are skhid-srp's `resources-db` crate
(`srp-026g/crates/resources-db`), a git dependency that cargo fetches. Clone
the repository, then build with a stable Rust toolchain and a C linker
installed:

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

## Playable maps for 0.26

The [0.26 script collection](docs/playable-maps-026.md) covers building a
database, preparing the matching client and running a local two-client match.

| Script | Purpose |
| --- | --- |
| `scripts/build-026-maps.py fog-school` | Native School with dense volume fog and an overcast sky; aiming fix included (`noise_scale = 0`). |
| `scripts/build-026-maps.py 021-maps` | Import six 0.21 maps with day, evening and rain variants, retaining 0.26 shaders. |
| `tools/patch-026-map-resources.py` | Prepare the reviewed SRP client to accept rebuilt resource archives. |
| `scripts/run-026-map.py` | Stage a separate client directory and run a server plus two English Wine clients. |

```sh
python3 scripts/build-026-maps.py fog-school \
  --target-db /path/to/retail-026g/resources.db --output .work/fog-school
```

Use the matching retail databases and SRP client described in the guide.
Other conversion families below continue to target build 802.

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

Each bundle contains an `overlay/` virtual resource tree and a
`port-manifest.json` provenance record. The reviewed manifests in `ports/`
declare source levels, namespaces, target templates and conversion overrides.

## Pack and verify

```sh
vostok-port apply \
  --base-db /path/to/v0100b/resources.db \
  --bundle .work/cordon-bundle \
  --output-db .work/cordon-release/resources.db

vostok-port verify \
  --db .work/cordon-release/resources.db \
  --bundle .work/cordon-bundle
```

`apply` validates the overlay and writes the whole database again with it
(for build 802, which mounts no patches), then reopens the database and
checks the bundle's hashes. An existing output requires `--force`.

## Content for 0.26g players: patches

The 0.26g game mounts the stock `resources.db` and then each
`resources.db.patch_<n>` beside it in ascending n; a later archive's file
wins. New items, balance changes and the server's menus therefore ship as a
patch: the files that differ from the stock database and the earlier
patches, whole, packed by the same writer:

```sh
vostok-port pack patch --base-db /path/to/retail-026g/resources.db \
  [--base-patch resources.db.patch_14 …] \
  --pack packs/duty-exo --pack packs/bandit-mask \
  [--replace flash_movies/lobby_menu.swf=live-lobby.swf …] \
  --out-dir .work/patch15 --archive .work/resources.db.patch_15
```

See [content packs](docs/content-packs.md).

The STK2 lobby scene (the blockpost hut of STK2's own `lobby_scene`) is
made straight as a pack, a lobby project of its own
(`src/stk2_lobby_026.rs` says what it makes and how):

```sh
vostok-port build-stk2-lobby-026 --manifest ports/stk2-lobby-to-026g.json \
  --stk2-root /path/to/extracted --target-db /path/to/retail-026g/resources.db \
  --output .work/lobby-stk2-1 --work .work/lobby-stk2-tex
```

`--stk2-root` holds the files the manifest names: `STK2/xr2-*` (extracted
from `STK2.7z`) and `resources/` (from the later `stk_2.7z`); ImageMagick
(`magick`, and Pillow for the run-length TGAs it refuses) converts the
textures. The same command makes the Bar (`ports/stk2-bar-lobby-to-026g.json`)
and the bomb shelter (`ports/stk2-bunker-lobby-to-026g.json`): a `slice` takes
the objects of an STK2 project inside a box, merged into one model, with the
project's lights and its environment's post-process; `--list-textures` says
which source textures a manifest needs before they are extracted.
A manifest's `camera_boxes` (`{what, centre, half_extents, yaw}` in the lobby
frame) are invisible boxes for the lobby camera's collision ray alone (0.26's
unit primitive `editor/walker_collision`, scaled; cgroup 8, cmask 16): a cage
inside the room and boxes around the props the eye would get into. Its
`native_props` (`{lib, position, yaw}`) place 0.26's own models, with their
collision (the Bar's workbench under the weapon's spot). skhid-srp's
`lobby-scenes.md` says what a lobby project must give.

Lighting (STK2 lit its rooms with a flat ambient colour and light
propagation volumes, which 0.26g lacks): `objects` may clone the classic
lobby's `environment_probe` and `ambient_light(4)`; `probe_cubes`
(`{object, up, side, down}`) give such a probe cubemaps made from a
floor-to-sky gradient and their irradiance (only box probes, `geometry` 1
with `scale` its half extents, light anything in 0.26g). An ambient light's
`color` may be three numbers (packed red-low, as 0.26g reads it); ambient
lights light the room but not the character, which only real lights and
probes reach. `light_overrides` (`{intensity, range, color, cast_shadow}` by
converted name, `stk2_light_<n>`) and `environment_overrides` (the
environment's fields, after the post-process) set what was balanced by eye.
A material rule's `emission` keeps the STK2 material's emission
(`emission_scale` per material scales it further); a slice's
`material_swaps` (`{lib, from, to}`) give an unlit STK2 lamp its emissive
variant, and its `moves` (`{id, what, position}`) put a project object
elsewhere (a prop in a camera path's way).

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

- [Playable 0.26 maps and launcher scripts](docs/playable-maps-026.md)
- [Retail 0.21 maps in 0.26](docs/survarium-021-to-026.md)
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
