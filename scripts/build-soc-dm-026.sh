#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The S.T.A.L.K.E.R. deathmatch maps for Survarium 0.26g, all team
# deathmatch (mode 4): Shadow of Chernobyl's mp_pool (197) and Clear Sky's
# Training Camp with its summer meadow and forest (198), Repair depot
# mp_rembasa (199), Sports center mp_sport_center (200) and Fire station
# mp_firestation (201).
#
# Route: X-Ray -> v0.100b (build-soc-map, the meadow addon in native-foliage
# deathmatch mode) -> one 0.10b donor database -> build-survarium-maps-026
# with ports/soc-cs-dm-to-026g.json -> one additive 0.26g resources.db.
# See docs/soc-deathmatch-026.md.
#
# Inputs (override by environment):
#   TARGET010  untouched retail v0.100b resources.db (160dc718...28409)
#   TARGET026  untouched retail 0.26g resources.db (c74078be...5492c)
#   SOC        Shadow of Chernobyl root (gamedata.db0..dbd)
#   CS         S.T.A.L.K.E.R. Clear Sky install (resources/, levels/, mp/)
#   OUT        work directory; every stage is skipped when its output exists
set -euo pipefail
cd "$(dirname "$0")/.."

ARCHIVE=${ARCHIVE:-/mnt/toshiba/surv-resources}
TARGET010=${TARGET010:?retail v0.100b resources.db}
TARGET026=${TARGET026:-$ARCHIVE/0.26g/resources.db}
SOC=${SOC:-$ARCHIVE/donor-games/stalker-soc}
CS=${CS:-$HOME/.local/share/Steam/steamapps/common/STALKER Clear Sky}
CS_MAPS=(cs-training_camp-to-0100b cs-mp_rembasa-to-0100b cs-mp_sport_center-to-0100b cs-mp_firestation-to-0100b)
OUT=${OUT:?work directory}
VP=${VP:-target/release/vostok-port}

check() { [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$2" ] || { echo "refusing $1: not $3" >&2; exit 1; }; }
check "$TARGET010" 160dc7180b3e89279570d087442c3ad867aa5fe25bb2bf7789a8f7aaea628409 "retail v0.100b"
check "$TARGET026" c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c "retail 0.26g"

B=$OUT/bundles
mkdir -p "$B" "$OUT/logs"

# 1. The maps in v0.100b.
[ -e "$B/soc-mp_pool-to-0100b/port-manifest.json" ] || "$VP" build-soc-map \
  --manifest ports/soc-mp_pool-to-0100b.json --source-root "$SOC" \
  --target-db "$TARGET010" --output "$B/soc-mp_pool-to-0100b" --camera-at-spawn \
  > "$OUT/logs/pool-0100b.log"
for name in "${CS_MAPS[@]}"; do
  [ -e "$B/$name/port-manifest.json" ] || "$VP" build-soc-map \
    --manifest "ports/$name.json" --source-root "$CS" \
    --target-db "$TARGET010" --output "$B/$name" --camera-at-spawn > "$OUT/logs/$name.log"
done
CS_MAP=$B/cs-training_camp-to-0100b

# 2. The meadow and forest, built against the carried map: grass and flowers
#    as vegetation layers, conifers as LOD'd instances plus a thinned far
#    forest, and no victory items (deathmatch).
if [ ! -e "$B/cs-training_camp-meadow-dm/port-manifest.json" ]; then
  rm -rf "$OUT/cs-base"
  "$VP" apply --base-db "$TARGET010" --bundle "$CS_MAP" \
    --work-dir "$OUT/cs-base/work" --output-db "$OUT/cs-base/resources.db" \
    > "$OUT/logs/cs-base-apply.log"
  "$VP" build-soc-map-addon --source-root "$CS" --level mp_training_camp \
    --base-db "$OUT/cs-base/resources.db" \
    --flower-png assets/cs-training-camp-addons/wildflower-atlas.png \
    --native-foliage --near-tree-limit 1200 --far-tree-limit 2300 --deathmatch \
    --output "$B/cs-training_camp-meadow-dm" \
    > "$OUT/logs/cs-meadow-dm.log"
  rm -rf "$OUT/cs-base"
fi

# 3. One v0.100b donor database. Compose is first-wins: the addon rewrites
#    the map's projects and post-process material, so it goes first.
DONOR=$OUT/donor-0100b/resources.db
if [ ! -e "$DONOR" ]; then
  rm -rf "$OUT/composed-0100b" "$OUT/donor-0100b"
  "$VP" compose --bundle "$B/cs-training_camp-meadow-dm" \
    $(printf -- "--bundle $B/%s " "${CS_MAPS[@]}") \
    --bundle "$B/soc-mp_pool-to-0100b" --output "$OUT/composed-0100b"
  "$VP" prune-collision --base-db "$TARGET010" --bundle "$OUT/composed-0100b"
  "$VP" apply --base-db "$TARGET010" --bundle "$OUT/composed-0100b" \
    --work-dir "$OUT/donor-0100b/work" --output-db "$DONOR" > "$OUT/logs/donor-apply.log"
  "$VP" verify --db "$DONOR" --bundle "$OUT/composed-0100b"
  rm -rf "$OUT/donor-0100b/work"
fi
sha256sum "$DONOR"

# 4. The 0.26g maps, then the packed database.
if [ ! -e "$OUT/resources.db" ]; then
  rm -rf "$B/soc-cs-dm-026g" "$OUT/packing"
  "$VP" build-survarium-maps-026 --manifest ports/soc-cs-dm-to-026g.json \
    --donor-db "$DONOR" --target-db "$TARGET026" --output "$B/soc-cs-dm-026g" \
    > "$OUT/logs/soc-cs-dm-026g.log"
  "$VP" apply --base-db "$TARGET026" --bundle "$B/soc-cs-dm-026g" \
    --work-dir "$OUT/packing" --output-db "$OUT/resources.db" > "$OUT/logs/pack-026g.log"
  "$VP" verify --db "$OUT/resources.db" --bundle "$B/soc-cs-dm-026g"
  rm -rf "$OUT/packing"
fi
sha256sum "$OUT/resources.db"
