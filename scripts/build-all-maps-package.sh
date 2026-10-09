#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# One v0.100b resources.db carrying every ported map family for the offline
# playtest client: the twelve S.T.A.L.K.E.R. multiplayer maps and Cordon, the
# Clear Sky Training Camp with its summer meadow, the outdoor F.E.A.R. worlds,
# and the Survarium 0.1.1e / 0.20f / 0.23h levels. Every project's collision
# is checked against the result before packing (prune-collision): a collision
# name with no node behind it crashes the client.
# Every client project's camera is placed on the spawn (--camera-at-spawn),
# because that is where the offline client inserts the player.
#
# Inputs (override by environment):
#   TARGET  untouched retail resources.db, SHA-256 160dc718...28409
#   SOC     S.T.A.L.K.E.R. Shadow of Chernobyl install (gamedata.db0..dbd)
#   CS      S.T.A.L.K.E.R. Clear Sky install (mp/mp_training_camp.db)
#   FEAR    F.E.A.R. Ultimate Shooter Edition install (Default.archcfg)
#   DONORS  directory with v011e/app, v020f and v023h resources.db
#   OUT     work directory for bundles
#   RELEASE the packed database; an existing one is replaced
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=${TARGET:-../game-v0.100b/resources.db}
SOC=${SOC:-$HOME/.local/share/Steam/steamapps/common/STALKER Shadow of Chernobyl}
CS=${CS:-$HOME/.local/share/Steam/steamapps/common/STALKER Clear Sky}
FEAR=${FEAR:-$HOME/.local/share/Steam/steamapps/common/FEAR Ultimate Shooter Edition}
DONORS=${DONORS:-.work/map-donors}
OUT=${OUT:-.work/all-maps}
RELEASE=${RELEASE:-$OUT/release-v2/resources.db}
VP=${VP:-target/release/vostok-port}
FEAR_WORLDS=${FEAR_WORLDS:-"asylum train_yard deadwood"}

[ "$(sha256sum "$TARGET" | cut -d' ' -f1)" = \
  160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409 ] ||
  { echo "refusing $TARGET: not the retail v0.100b resources.db" >&2; exit 1; }

B=$OUT/bundles
mkdir -p "$B"
build() { # kind manifest-name extra-args...
  local kind=$1 name=$2; shift 2
  [ -e "$B/$name/port-manifest.json" ] && return
  rm -rf "${B:?}/$name"
  "$VP" "$kind" --manifest "ports/$name.json" --target-db "$TARGET" \
    --output "$B/$name" --camera-at-spawn "$@"
}

# Compose is first-wins on shared paths.
order=()
for l in 01 02 03 04 05; do
  build build-map "level$l-day-020f-to-0100b" --donor-db "$DONORS/v020f/resources.db"
  order+=("level$l-day-020f-to-0100b")
done
build build-map level03-day-023h-to-0100b --donor-db "$DONORS/v023h/resources.db"
build build-map level04-011e-to-0100b --donor-db "$DONORS/v011e/app/resources.db"
order+=(level03-day-023h-to-0100b level04-011e-to-0100b)

# The summer Training Camp: the converted map, then the meadow addon built
# against it. The addon rewrites the map's projects and post-process
# material, so it composes ahead of the map.
build build-soc-map cs-training_camp-to-0100b --source-root "$CS"
meadow=cs-training_camp-meadow
if [ ! -e "$B/$meadow/port-manifest.json" ]; then
  rm -rf "$OUT/cs-base" "${B:?}/$meadow"
  "$VP" apply --base-db "$TARGET" --bundle "$B/cs-training_camp-to-0100b" \
    --work-dir "$OUT/cs-base/work" --output-db "$OUT/cs-base/resources.db"
  "$VP" build-soc-map-addon --source-root "$CS" --level mp_training_camp \
    --base-db "$OUT/cs-base/resources.db" \
    --flower-png assets/cs-training-camp-addons/wildflower-atlas.png \
    --output "$B/$meadow"
  rm -rf "$OUT/cs-base"
fi
order+=("$meadow" cs-training_camp-to-0100b)
for m in ports/soc-mp_pool-to-0100b.json ports/soc-*.json; do
  name=$(basename "$m" .json)
  build build-soc-map "$name" --source-root "$SOC"
  [[ " ${order[*]} " == *" $name "* ]] || order+=("$name")
done
for w in $FEAR_WORLDS; do
  build build-lilith-map "lilith-$w-to-0100b" --source-root "$FEAR"
  order+=("lilith-$w-to-0100b")
done

rm -rf "$OUT/composed" "$OUT/apply-work"
"$VP" compose $(printf -- "--bundle $B/%s " "${order[@]}") --output "$OUT/composed"
"$VP" prune-collision --base-db "$TARGET" --bundle "$OUT/composed"
mkdir -p "$(dirname "$RELEASE")"
"$VP" apply --base-db "$TARGET" --bundle "$OUT/composed" \
  --work-dir "$OUT/apply-work" --output-db "$RELEASE" --force
"$VP" verify --db "$RELEASE" --bundle "$OUT/composed"
"$VP" validate-models --db "$RELEASE"
rm -rf "$OUT/apply-work"
