#!/usr/bin/env bash
# Authored source export: skhid-srp/scripts/v021d-weather/source_projects.py.
# Builds only content packs, never a players' patch or a release.
set -euo pipefail
BUILD=$(realpath -m "${1:?build output}")
SRP=$(realpath "${2:?skhid source checkout}")
SOURCE=$(realpath "${3:?authored day source export}")
HERE=$(cd "$(dirname "$0")/../.." && pwd)
PORTER=${PORTER:-$HERE/target/debug/vostok-port}
LIGHTING=${LIGHTING:-$HERE/target/debug/examples/lighting-021-pack}
STOCK=${STOCK:-/home/sheep/Projects/survarium/srp/0.26g/game/resources.db}
DONOR=${DONOR:-/mnt/hdd/Projects/Survarium Builds/survarium_full_021d/game/resources.db}
PATCH_DIR=${PATCH_DIR:-$HOME/.cache/skhid-release/patches}
mkdir -p "$BUILD"
[[ -f "$SOURCE/source.json" ]] || { echo 'Not an authored source export' >&2; exit 1; }
BASE=(); SHADER_BASE=(); LIVE=()
while IFS=$'\t' read -r kind name rest; do
    if [[ $kind == base ]]; then
        BASE+=(--base-patch "$PATCH_DIR/$name")
        SHADER_BASE+=("$PATCH_DIR/$name")
    fi
done < "$SRP/release/content.tsv"
while IFS=$'\t' read -r name rest; do
    [[ $name == resources.db.patch_* ]] || continue
    LIVE+=(--live-patch "$PATCH_DIR/$name")
done < "$SRP/release/patches.tsv"
"$LIGHTING" "$HERE/ports/three-maps-021-lighting.json" "$DONOR" "$STOCK" \
    "$BUILD/three-maps-021-lighting-5" "${SHADER_BASE[@]}"
"$PORTER" build-021d-weather --manifest "$HERE/ports/survarium-021d-weathers-to-026g.json" \
    --donor-db "$DONOR" --target-db "$STOCK" "${BASE[@]}" "${LIVE[@]}" \
    --project-source "$SOURCE" --work "$BUILD/weather-work" --output "$BUILD/maps-021d-weathers-2"
sha256sum "$BUILD/three-maps-021-lighting-5/pack.json" "$BUILD/maps-021d-weathers-2/pack.json"
