#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The STALKER team-deathmatch maps as Skhid content packs, <pack>@N: every
# map of ports/stalker-tdm/maps.json whose `pack` is <pack> (stalker-maps
# when a map names none) from its bundle (port-map.sh), with the spawns and
# ammo stashes of ports/stalker-tdm/spots.json (the map spot planner's list).
# A pack's maps share converted textures and materials (separate packs
# would add the same files twice); a released pack is never rebuilt with
# other maps (players' files would change), so maps added later go into a
# pack of their own (stalker-maps-2: Dark Valley, Vehicle Station, Fire
# Station, Sports Center).
#
# Every map gets what the port gives every map (no step names one): the
# importer's defaults for an early map (stock day film grain, the 0.11e sun,
# stock ambient occlusion strength, a colour texture as gloss map matte, at
# most 120,000 grass instances), the terrain as build-soc-map converts it,
# then its bushes as collision a foot step meets (bushes.py).
#
#   scripts/stalker-tdm/build-pack.sh [--pack NAME] [--version N] [--keep-donor] [--after PACK_DIR]
#
# --keep-donor: reuse donor/<pack>.db when no bundle changed (new
# spots, names or ids only).
# --after: a later pack's: the stalker-maps pack it is built after, when that
# is a new version (its files players do not have yet are left out too).
#
# Writes under $OUT:
#   donor/<pack>.db              compose (Training Camp's meadow first), prune-collision, apply, verify
#   specs/<pack>.json            material templates matched on the donor + make_spec.py
#   bundles026/<pack>            build-survarium-maps-026 (team deathmatch, ids of maps.json), then
#                                bushes.py (each map's bushes, placed in its project); gloss_check.py
#                                (no ported material a mirror but the glass template's)
#   packs/<pack>-N               pack from-bundle: stalker-maps whole, checked over the stock db
#                                (a new version replaces the one players have); a later pack
#                                without the files players have, checked over every live patch
#   minimaps/<project>.png/json  the HUD minimaps (render_minimap.py), for skhid's ui_images.py
#
# Inputs (environment):
#   TARGET010  retail v0.100b resources.db (160dc718...)
#   TARGET026  retail 0.26g resources.db (c74078be...)
#   STOCK_DB   the stock resources.db players have (skhid's SKHID_STOCK_DB)
#   PATCHES    folder of the live resources.db.patch_<n> (release/patches.tsv)
#   VP         this branch's vostok-port; PACK_VP one with `pack` (rev20-021d-crates)
#   OUT        work folder (default /mnt/hdd/Projects/stalker-maps-026/out)
#   PYTHON     python3 with numpy, scipy and Pillow (the minimap renderer)
set -euo pipefail
cd "$(dirname "$0")/../.."

VERSION=1; KEEP_DONOR=''; NAME=stalker-maps; AFTER=()
while (($#)); do
  case $1 in
    --pack) NAME=$2; shift 2 ;;
    --version) VERSION=$2; shift 2 ;;
    --keep-donor) KEEP_DONOR=1; shift ;;
    --after) AFTER=(--after "$2"); shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 1 ;;
  esac
done
OUT=${OUT:-/mnt/hdd/Projects/stalker-maps-026/out}
TARGET010=${TARGET010:?retail v0.100b resources.db}
TARGET026=${TARGET026:?retail 0.26g resources.db}
STOCK_DB=${STOCK_DB:?stock resources.db}
PATCHES=${PATCHES:?folder of the live patches}
VP=${VP:?vostok-port of this branch}
PACK_VP=${PACK_VP:?vostok-port with pack subcommands}
PYTHON=${PYTHON:-python3}
mkdir -p "$OUT"/{donor,specs,bundles026,packs,logs,minimaps}

MAPS=$("$PYTHON" -c "import json; print(' '.join(k for k, m in json.load(open('ports/stalker-tdm/maps.json'))['maps'].items() if m.get('pack', 'stalker-maps') == '$NAME'))")
[ -n "$MAPS" ] || { echo "no map of maps.json is in pack $NAME" >&2; exit 1; }
BUNDLES=(); PREFIXES=()
for map in $MAPS; do
  [ -d "$OUT/bundles/$map" ] || { echo "$map: no bundle; run port-map.sh $map first" >&2; exit 1; }
  [ ! -d "$OUT/bundles/$map-addon" ] || BUNDLES=(--bundle "$OUT/bundles/$map-addon" "${BUNDLES[@]}")   # first wins
  BUNDLES+=(--bundle "$OUT/bundles/$map")
  recipe=$("$PYTHON" -c "import json; print(json.load(open('ports/stalker-tdm/maps.json'))['maps']['$map']['recipe'])")
  PREFIXES+=("material_instances/$("$PYTHON" -c "import json; print(json.load(open('ports/$recipe.json'))['namespace'])")/")
done

# 1. One v0.100b donor database of every map.
DONOR=$OUT/donor/$NAME.db
if [ -n "$KEEP_DONOR" ] && [ -f "$DONOR" ]; then
  echo "keeping $DONOR"
else
rm -rf "$OUT/composed-$NAME" "$OUT/donor-work-$NAME" "$DONOR"
"$VP" compose "${BUNDLES[@]}" --output "$OUT/composed-$NAME" > "$OUT/logs/$NAME-compose.log"
"$VP" prune-collision --base-db "$TARGET010" --bundle "$OUT/composed-$NAME" > "$OUT/logs/$NAME-prune.log"
"$VP" apply --base-db "$TARGET010" --bundle "$OUT/composed-$NAME" --work-dir "$OUT/donor-work-$NAME" \
  --output-db "$DONOR" > "$OUT/logs/$NAME-donor.log"
"$VP" verify --db "$DONOR" --bundle "$OUT/composed-$NAME" > "$OUT/logs/$NAME-donor-verify.log"
rm -rf "$OUT/donor-work-$NAME" "$OUT/composed-$NAME"
fi
DONOR_SHA=$(sha256sum "$DONOR" | cut -d' ' -f1)

# 2. The spec: material templates matched on the donor (the meadow's as the
#    reviewed soc-cs-dm spec names them), spawns and stashes from the planner.
"$PYTHON" tools/playable-map-pack/xray_material_templates.py --porter "$VP" "$DONOR" "$TARGET026" \
  "${PREFIXES[@]}" > "$OUT/specs/$NAME.templates.json" 2> "$OUT/logs/$NAME-templates.log"
"$PYTHON" - "$OUT/specs/$NAME.templates.json" <<'EOF'
import json, sys
p = sys.argv[1]; t = json.load(open(p))
old = json.load(open('ports/soc-cs-dm-to-026g.json'))['material_templates']
t.update({k: v for k, v in old.items() if not k.startswith('material_instances/ported/') and k not in t})
json.dump(t, open(p, 'w'), indent=1)
EOF
"$PYTHON" tools/stalker-tdm/make_spec.py $MAPS --donor-sha256 "$DONOR_SHA" \
  --templates "$OUT/specs/$NAME.templates.json" --output "$OUT/specs/$NAME.json"

# 3. The 0.26g maps.
rm -rf "$OUT/bundles026/$NAME"
"$VP" build-survarium-maps-026 --manifest "$OUT/specs/$NAME.json" --donor-db "$DONOR" \
  --target-db "$TARGET026" --output "$OUT/bundles026/$NAME" > "$OUT/logs/$NAME-026g.log"
"$PYTHON" tools/stalker-tdm/bushes.py $MAPS --bundle "$OUT/bundles026/$NAME" | tee "$OUT/logs/$NAME-bushes.log"
"$PYTHON" tools/playable-map-pack/gloss_check.py --bundle "$OUT/bundles026/$NAME" --vostok-port "$VP" --db "$TARGET026" \
  > "$OUT/logs/$NAME-gloss.log"
tail -1 "$OUT/logs/$NAME-gloss.log"

# 4. The content pack. stalker-maps, the pack of the maps that name none, is
#    whole and checked over the stock db: its new version replaces the one
#    players have (release/content.tsv's row). A later pack is checked over
#    every live patch, and the files players have byte for byte (the v0.100b
#    template textures the first pack adds) are left out of it
#    (leave_out_live.py; a file that would differ stops it).
PACK=$OUT/packs/$NAME-$VERSION
rm -rf "$PACK"
"$PACK_VP" pack from-bundle --base-db "$STOCK_DB" --build-base-db "$TARGET026" \
  --bundle "$OUT/bundles026/$NAME" --output "$PACK" --id "$NAME" --version "$VERSION" > "$OUT/logs/$NAME-pack.log"
if [ "$NAME" = stalker-maps ]; then
  "$PACK_VP" pack check --pack "$PACK" --base-db "$STOCK_DB"
else
  LIVE=(); LIVE_PATCHES=()
  for p in $(ls "$PATCHES" | grep -E '^resources\.db\.patch_[0-9]+$' | sort -t_ -k2 -n); do LIVE+=(--base-patch "$PATCHES/$p"); LIVE_PATCHES+=(--patch "$PATCHES/$p"); done
  "$PYTHON" tools/stalker-tdm/leave_out_live.py --pack "$PACK" --stock "$STOCK_DB" "${LIVE_PATCHES[@]}" --vostok-port "$VP" "${AFTER[@]}" \
    > "$OUT/logs/$NAME-left-out.log"
  tail -1 "$OUT/logs/$NAME-left-out.log"
  "$PACK_VP" pack check --pack "$PACK" --base-db "$STOCK_DB" "${LIVE[@]}"
fi

# 5. The HUD minimaps (0.26 style, framed on what a walker reaches from the spawns).
for id in $("$PYTHON" -c "import json; print(' '.join(str(m['id']) for m in json.load(open('$OUT/specs/$NAME.json'))['maps']))"); do
  PYTHONDONTWRITEBYTECODE=1 "$PYTHON" tools/playable-map-pack/render_minimap.py --overlay "$OUT/bundles026/$NAME/overlay" \
    --spec "$OUT/specs/$NAME.json" --output "$OUT/minimaps" --map "$id" > "$OUT/logs/minimap-$id.log" 2>&1
done
echo "pack $PACK (pack.json sha256 $(sha256sum "$PACK/pack.json" | cut -d' ' -f1)); minimaps in $OUT/minimaps"
