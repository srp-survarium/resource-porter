#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# One STALKER map's v0.100b bundle, re-runnable from a compiled X-Ray level
# folder (e.g. one the owner recompiled in the SDK), then the maps' one
# Skhid content pack again (scripts/stalker-tdm/build-pack.sh). Same path for
# every map and every re-export: no hand edits.
#
#   scripts/stalker-tdm/port-map.sh MAP [LEVEL_FOLDER [TEXTURES_FOLDER]] [--version N] [--no-pack]
#
# MAP: a key of ports/stalker-tdm/maps.json (cs-mp_bath, cs-mp_military_1,
# training-camp-cs-dm, rembasa-cs-dm, dl-mp_station, dl-mp_depository).
# LEVEL_FOLDER: the compiled level (level, level.cform, level.game,
# level.spawn, level.geom, ...); its files are read over the game's own
# archives. Without it the installed game's level is used (the community
# maps need their folder: they are in no installed archive).
# TEXTURES_FOLDER: gamedata/textures files the level brings (a user map's
# terrain, intro and map pictures), read over the game's.
# --no-pack: the bundle only (several maps in a row, then build-pack.sh once).
#
# Writes under $OUT (never into the inputs):
#   sources/MAP.sha256   every source file used, with its sha256
#   bundles/MAP          build-soc-map (v0.100b); Training Camp also its
#                        meadow addon (bundles/MAP-addon), as build-soc-dm-026.sh
#
# Inputs (environment): TARGET010 (retail v0.100b resources.db), SOC and CS
# (the installations), VP (this branch's vostok-port), PYTHON, OUT
# (default /mnt/hdd/Projects/stalker-maps-026/out); build-pack.sh's own.
set -euo pipefail
cd "$(dirname "$0")/../.."

MAP=${1:?map key}; shift
LEVEL_FOLDER=''; TEXTURES=''; PACK_ARGS=(); NO_PACK=''
while (($#)); do
  case $1 in
    --version) PACK_ARGS+=(--version "$2"); shift 2 ;;
    --no-pack) NO_PACK=1; shift ;;
    *) if [ -z "$LEVEL_FOLDER" ]; then LEVEL_FOLDER=$1; else TEXTURES=$1; fi; shift ;;
  esac
done
OUT=${OUT:-/mnt/hdd/Projects/stalker-maps-026/out}
TARGET010=${TARGET010:?retail v0.100b resources.db}
SOC=${SOC:-$HOME/.local/share/Steam/steamapps/common/STALKER Shadow of Chernobyl}
CS=${CS:-$HOME/.local/share/Steam/steamapps/common/STALKER Clear Sky}
VP=${VP:?vostok-port of this branch}
PYTHON=${PYTHON:-python3}

field() { "$PYTHON" -c "import json,sys; m=json.load(open('ports/stalker-tdm/maps.json'))['maps']['$MAP']; v=m.get('$1'); print('' if v is None else v)"; }
RECIPE=$(field recipe); GAME=$(field game); LEVEL=$(field level); ADDON=$(field addon)
[ -n "$RECIPE" ] || { echo "unknown map $MAP" >&2; exit 1; }
case $GAME in soc) GAME_ROOT=$SOC ;; cs) GAME_ROOT=$CS ;; *) echo "game $GAME" >&2; exit 1 ;; esac
mkdir -p "$OUT"/{sources,bundles,logs,src}

# 1. The source: the installation, or the level folder over it.
SRC=$GAME_ROOT
if [ -n "$LEVEL_FOLDER" ]; then
  SRC=$OUT/src/$MAP
  rm -rf "$SRC"; mkdir -p "$SRC/overlay/levels/$LEVEL"
  ln -s "$GAME_ROOT" "$SRC/base"
  for f in "$LEVEL_FOLDER"/*; do ln -s "$(realpath "$f")" "$SRC/overlay/levels/$LEVEL/"; done
  if [ -n "$TEXTURES" ]; then ln -s "$(realpath "$TEXTURES")" "$SRC/overlay/textures"; fi
  { (cd "$LEVEL_FOLDER" && find -L . -type f -print0 | sort -z | xargs -0 sha256sum | sed "s|  \./|  levels/$LEVEL/|")
    if [ -n "$TEXTURES" ]; then (cd "$TEXTURES" && find -L . -type f -print0 | sort -z | xargs -0 sha256sum | sed "s|  \./|  textures/|"); fi
  } > "$OUT/sources/$MAP.sha256"
else
  ARCHIVE=$(ls "$GAME_ROOT"/mp/"$LEVEL".db 2>/dev/null || true)
  [ -n "$ARCHIVE" ] || { echo "$MAP: no installed archive for $LEVEL; give its level folder" >&2; exit 1; }
  sha256sum "$ARCHIVE" | sed "s|  .*/|  mp/|" > "$OUT/sources/$MAP.sha256"
fi
echo "sources: $OUT/sources/$MAP.sha256 ($(wc -l < "$OUT/sources/$MAP.sha256") files)"

# 2. The v0.100b bundle (and Training Camp's meadow, built against it).
B=$OUT/bundles/$MAP
rm -rf "$B" "$OUT/bundles/$MAP-addon"
"$VP" build-soc-map --manifest "ports/$RECIPE.json" --source-root "$SRC" --target-db "$TARGET010" \
  --output "$B" --camera-at-spawn > "$OUT/logs/$MAP-0100b.log"
if [ "$ADDON" = meadow ]; then
  rm -rf "$OUT/base-$MAP"
  "$VP" apply --base-db "$TARGET010" --bundle "$B" --work-dir "$OUT/base-$MAP/work" \
    --output-db "$OUT/base-$MAP/resources.db" > "$OUT/logs/$MAP-base-apply.log"
  "$VP" build-soc-map-addon --source-root "$SRC" --level "$LEVEL" --base-db "$OUT/base-$MAP/resources.db" \
    --flower-png assets/cs-training-camp-addons/wildflower-atlas.png --native-foliage \
    --near-tree-limit 1200 --far-tree-limit 2300 --deathmatch --output "$OUT/bundles/$MAP-addon" \
    > "$OUT/logs/$MAP-addon.log"
  rm -rf "$OUT/base-$MAP"
fi
echo "$MAP: bundle $B"

[ -n "$NO_PACK" ] || exec bash scripts/stalker-tdm/build-pack.sh "${PACK_ARGS[@]}"
