# Rebuilding map databases

Generated bundles, extracted resources and packed databases belong under
`.work/`. Keep your original game inputs and any unique local changes outside
disposable build directories. The source code and reviewed manifests are
included in this repository; game installations and target databases are
supplied separately.

## Inputs

Set paths for the families you want to build:

```sh
export TARGET=/path/to/v0100b/resources.db
export SOC=/path/to/STALKER-Shadow-of-Chernobyl
export CS=/path/to/STALKER-Clear-Sky
export FEAR=/path/to/FEAR
export PATH="$PWD/target/release:$PATH"
```

The reviewed build 802 target database has SHA-256
`160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409`.
If you have the Vostok repository beside this one, its Nix flake can resolve
the pinned target:

```sh
TARGET=$(nix build --no-link --print-out-paths ../vostok#survarium.resources)/resources.db
sha256sum "$TARGET"
```

## Build, compose, apply and verify

For the thirteen Shadow of Chernobyl maps, build one bundle per manifest,
collect the explicit bundle paths, then compose and pack:

```sh
bundles=()
for manifest in ports/soc-*.json; do
  name=$(basename "$manifest" .json)
  bundle=".work/soc-bundles/$name"
  vostok-port build-soc-map --manifest "$manifest" \
    --source-root "$SOC" --target-db "$TARGET" --output "$bundle"
  bundles+=(--bundle "$bundle")
done
vostok-port compose "${bundles[@]}" --output .work/soc-composed
vostok-port apply --base-db "$TARGET" --bundle .work/soc-composed \
  --output-db .work/soc-release/resources.db
vostok-port verify --db .work/soc-release/resources.db --bundle .work/soc-composed
```

These are Bash commands. Use fresh output paths for a rebuild; compose is
first-wins for shared resources, so the order of bundles matters.

Other families use the same final three stages:

| Family | Build command | Manifest selection | Source root |
| --- | --- | --- | --- |
| Clear Sky | `build-soc-map` | `ports/cs-training_camp-to-0100b.json` | `$CS` |
| F.E.A.R. | `build-lilith-map` | `ports/lilith-*.json` | `$FEAR` |

The Clear Sky meadow addon is an optional second bundle built against the
packed base map:

```sh
vostok-port build-soc-map-addon --source-root "$CS" --level mp_training_camp \
  --base-db .work/cs-release/resources.db \
  --flower-png assets/cs-training-camp-addons/wildflower-atlas.png \
  --output .work/cs-meadow-bundle
```

Apply and verify this addon using the packed base map as `--base-db`.

## Combined offline package

`scripts/build-all-maps-package.sh` builds the reviewed combined package,
including later Survarium donor maps. Supply paths explicitly to avoid its
local archive-drive defaults:

```sh
SOC=/path/to/STALKER CS=/path/to/Clear-Sky FEAR=/path/to/FEAR \
DONORS=/path/to/survarium-donors TARGET=/path/to/v0100b/resources.db \
  ./scripts/build-all-maps-package.sh
```

`DONORS` contains the expected `0.1.1e/app`, `0.20f` and `0.23h` database
directories. Inspect the script's input comments for `OUT`, `RELEASE`,
`FEAR_WORLDS` and `CS_FROM_DB` overrides. It validates the retail target hash
and audits project collision before packing.

## Client validation

The [README runtime instructions](../README.md#run-in-the-client) describe
the external client and SRP prerequisites. For a selected launcher profile:

```sh
./scripts/run-map-demo.sh soc-rostok --check
```

The launcher expects family-specific database paths; select the appropriate
database environment override when your output name differs. Its `--check`
mode validates the local runtime layout without starting Wine. Hash and
resource checks do not replace loading and walking the map in the client.
