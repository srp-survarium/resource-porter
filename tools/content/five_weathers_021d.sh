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
# A day map not released yet: the quick build-content patch of the same checkout that holds its pack
# (the weather projects' models and materials resolve over the live patches and it).
for patch in ${UNRELEASED_DAY_PATCH:-}; do LIVE+=(--live-patch "$patch"); done
# Each pack's folder is named by its manifest's id and version. WEATHER_MANIFEST: another map's weathers
# by the same builder (ports/survarium-021d-chemical-weathers-to-026g.json).
field() { sed -n 's/.*"'"$2"'": *"\([^"]*\)".*/\1/p' "$1" | head -1; }
WEATHER_MANIFEST=${WEATHER_MANIFEST:-$HERE/ports/survarium-021d-weathers-to-026g.json}
LIGHTING_MANIFEST=$HERE/ports/three-maps-021-lighting.json
LIGHTING_PACK=$BUILD/$(field "$LIGHTING_MANIFEST" id)-$(field "$LIGHTING_MANIFEST" version)
WEATHER_PACK=$BUILD/$(field "$WEATHER_MANIFEST" id)-$(field "$WEATHER_MANIFEST" version)
"$LIGHTING" "$LIGHTING_MANIFEST" "$DONOR" "$STOCK" "$LIGHTING_PACK" "${SHADER_BASE[@]}"
"$PORTER" build-021d-weather --manifest "$WEATHER_MANIFEST" \
    --donor-db "$DONOR" --target-db "$STOCK" "${BASE[@]}" "${LIVE[@]}" \
    --project-source "$SOURCE" --work "$BUILD/weather-work" --output "$WEATHER_PACK"
sha256sum "$LIGHTING_PACK/pack.json" "$WEATHER_PACK/pack.json"
