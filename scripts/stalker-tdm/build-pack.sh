#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The STALKER team-deathmatch maps as one Skhid content pack, stalker-maps@N:
# every map of ports/stalker-tdm/maps.json from its bundle (port-map.sh),
# with the spawns and ammo stashes of ports/stalker-tdm/spots.json (the map
# spot planner's list). One pack, because the maps share converted textures
# and materials (separate packs would add the same files twice).
#
#   scripts/stalker-tdm/build-pack.sh [--version N] [--keep-donor]
#
# --keep-donor: reuse donor/stalker-maps.db when no bundle changed (new
# spots, names or ids only).
#
# Writes under $OUT:
#   donor/stalker-maps.db        compose (Training Camp's meadow first), prune-collision, apply, verify
#   specs/stalker-maps.json      material templates matched on the donor + make_spec.py
#   bundles026/stalker-maps      build-survarium-maps-026 (team deathmatch, ids of maps.json)
#   packs/stalker-maps-N         pack from-bundle, checked over every live patch
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

VERSION=1; KEEP_DONOR=''
while (($#)); do
  case $1 in
    --version) VERSION=$2; shift 2 ;;
    --keep-donor) KEEP_DONOR=1; shift ;;
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

MAPS=$("$PYTHON" -c "import json; print(' '.join(json.load(open('ports/stalker-tdm/maps.json'))['maps']))")
BUNDLES=(); PREFIXES=()
for map in $MAPS; do
  [ -d "$OUT/bundles/$map" ] || { echo "$map: no bundle; run port-map.sh $map first" >&2; exit 1; }
  [ ! -d "$OUT/bundles/$map-addon" ] || BUNDLES=(--bundle "$OUT/bundles/$map-addon" "${BUNDLES[@]}")   # first wins
  BUNDLES+=(--bundle "$OUT/bundles/$map")
  recipe=$("$PYTHON" -c "import json; print(json.load(open('ports/stalker-tdm/maps.json'))['maps']['$map']['recipe'])")
  PREFIXES+=("material_instances/$("$PYTHON" -c "import json; print(json.load(open('ports/$recipe.json'))['namespace'])")/")
done

# 1. One v0.100b donor database of every map.
DONOR=$OUT/donor/stalker-maps.db
if [ -n "$KEEP_DONOR" ] && [ -f "$DONOR" ]; then
  echo "keeping $DONOR"
else
rm -rf "$OUT/composed" "$OUT/donor-work" "$DONOR"
"$VP" compose "${BUNDLES[@]}" --output "$OUT/composed" > "$OUT/logs/compose.log"
"$VP" prune-collision --base-db "$TARGET010" --bundle "$OUT/composed" > "$OUT/logs/prune.log"
"$VP" apply --base-db "$TARGET010" --bundle "$OUT/composed" --work-dir "$OUT/donor-work" \
  --output-db "$DONOR" > "$OUT/logs/donor.log"
"$VP" verify --db "$DONOR" --bundle "$OUT/composed" > "$OUT/logs/donor-verify.log"
rm -rf "$OUT/donor-work" "$OUT/composed"
fi
DONOR_SHA=$(sha256sum "$DONOR" | cut -d' ' -f1)

# 2. The spec: material templates matched on the donor (the meadow's as the
#    reviewed soc-cs-dm spec names them), spawns and stashes from the planner.
"$PYTHON" tools/playable-map-pack/xray_material_templates.py --porter "$VP" "$DONOR" "$TARGET026" \
  "${PREFIXES[@]}" > "$OUT/specs/stalker-maps.templates.json" 2> "$OUT/logs/templates.log"
"$PYTHON" - "$OUT/specs/stalker-maps.templates.json" <<'EOF'
import json, sys
p = sys.argv[1]; t = json.load(open(p))
old = json.load(open('ports/soc-cs-dm-to-026g.json'))['material_templates']
t.update({k: v for k, v in old.items() if not k.startswith('material_instances/ported/') and k not in t})
json.dump(t, open(p, 'w'), indent=1)
EOF
"$PYTHON" tools/stalker-tdm/make_spec.py $MAPS --donor-sha256 "$DONOR_SHA" \
  --templates "$OUT/specs/stalker-maps.templates.json" --output "$OUT/specs/stalker-maps.json"

# 3. The 0.26g maps.
rm -rf "$OUT/bundles026/stalker-maps"
"$VP" build-survarium-maps-026 --manifest "$OUT/specs/stalker-maps.json" --donor-db "$DONOR" \
  --target-db "$TARGET026" --output "$OUT/bundles026/stalker-maps" > "$OUT/logs/026g.log"

# 4. The content pack, checked over every live patch.
PACK=$OUT/packs/stalker-maps-$VERSION
rm -rf "$PACK"
"$PACK_VP" pack from-bundle --base-db "$STOCK_DB" --build-base-db "$TARGET026" \
  --bundle "$OUT/bundles026/stalker-maps" --output "$PACK" --id stalker-maps --version "$VERSION" > "$OUT/logs/pack.log"
LIVE=()
for p in $(ls "$PATCHES" | grep -E '^resources\.db\.patch_[0-9]+$' | sort -t_ -k2 -n); do LIVE+=(--base-patch "$PATCHES/$p"); done
"$PACK_VP" pack check --pack "$PACK" --base-db "$STOCK_DB" "${LIVE[@]}"

# 5. The HUD minimaps (0.26 style, framed on what a walker reaches from the spawns).
for id in $("$PYTHON" -c "import json; print(' '.join(str(m['id']) for m in json.load(open('$OUT/specs/stalker-maps.json'))['maps']))"); do
  PYTHONDONTWRITEBYTECODE=1 "$PYTHON" tools/playable-map-pack/render_minimap.py --overlay "$OUT/bundles026/stalker-maps/overlay" \
    --spec "$OUT/specs/stalker-maps.json" --output "$OUT/minimaps" --map "$id" > "$OUT/logs/minimap-$id.log" 2>&1
done
echo "pack $PACK (pack.json sha256 $(sha256sum "$PACK/pack.json" | cut -d' ' -f1)); minimaps in $OUT/minimaps"
