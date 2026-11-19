#!/usr/bin/env bash
# Chemical Plant (level_04) of retail 0.21d as the pack maps-021d-chemical, built over the live patches the way
# maps-021d-school-fort and maps-021d-rudnya were (school_fort_021d.sh has each step's reason):
#   1. build-survarium021-maps with ports/survarium-021d-chemical-to-026g.json (the shared settings; donor local
#      lighting; maps_dict 266, map number 31, st_level_31_name);
#   2. pack from-bundle against the stock archive, then drop_live_files.py: what the live patches already hold of
#      ported/v021d is shared, the paths of chemical_021d.keep-live.txt keep the live bytes;
#   3. add-021d-deathmatch: Team Deathmatch 267 (day_4);
#   4. native_crates.py + examples/edit-built-pack: native 0.26 level_04's ammo crates at its spots on 0.21d's floor;
#   5. the description, the pack check over stock + the live patches, SHA256SUMS.
#   chemical_021d.sh BUILD_DIR SKHID_SRP   (SKHID_SRP: release/patches.tsv names the live patches)
set -euo pipefail
BUILD=$(realpath -m "$1"); SRP=$(realpath "$2"); mkdir -p "$BUILD"
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
PORTER=${PORTER:-$HERE/target/release/vostok-port}
EDIT=${EDIT:-$HERE/target/release/examples/edit-built-pack}
export PORTER TOOLS=${TOOLS:?resource-porter a0b5933 tools (git archive a0b5933 tools/day-map-objectives tools/chemical-plant)}
P=${PYTHON:-python3}
STOCK=${STOCK:-/home/sheep/Projects/survarium/srp/0.26g/game/resources.db}
DONOR=${DONOR:-"/mnt/hdd/Projects/Survarium Builds/survarium_full_021d/game/resources.db"}
PATCH_DIR=${PATCH_DIR:-$HOME/.cache/skhid-release/patches}
PATCHES=(); for n in $(awk -F'\t' '!/^#/ && NF {print $1}' "$SRP/release/patches.tsv"); do PATCHES+=("$PATCH_DIR/$n"); done
VERSION=2; ID=maps-021d-chemical; OUT=$BUILD/$ID-$VERSION
KEEP=(); while read -r path; do [[ -z $path || $path == \#* ]] || KEEP+=(--keep-live "$path"); done < "$HERE/tools/content/chemical_021d.keep-live.txt"

rm -rf "$BUILD/bundle" "$OUT" "$BUILD/fill"
"$PORTER" build-survarium021-maps --manifest "$HERE/ports/survarium-021d-chemical-to-026g.json" \
  --donor-db "$DONOR" --target-db "$STOCK" --output "$BUILD/bundle"
"$PORTER" pack from-bundle --base-db "$STOCK" --build-base-db "$STOCK" --bundle "$BUILD/bundle" --output "$OUT" \
  --id "$ID" --version "$VERSION"
$P -I "$HERE/tools/content/drop_live_files.py" "$OUT" "$PORTER" "${KEEP[@]}" "${PATCHES[@]}"
"$PORTER" add-021d-deathmatch --pack "$OUT" --target-db "$STOCK" --variant level_04:day:267 --version "$VERSION"
$P -I "$HERE/tools/content/native_crates.py" "$STOCK" "$BUILD/bundle/overlay" "$BUILD/fill" level_04:266:267
(cd "$HERE" && "$EDIT" "$OUT" "$BUILD/fill/plan.json" "$STOCK")

$P - "$OUT" <<'PY'
import json, sys
from pathlib import Path
p = Path(sys.argv[1]) / 'pack.json'; m = json.loads(p.read_text())
m['description'] = ('Chemical Plant (level_04) of retail 0.21d, day: maps_dict 266 (batteries), 267 (Team Deathmatch), '
                    'map number 31, as maps-021d-school-fort made School and Tarakanovsky Fort (0.21 local lighting and '
                    'probes, 0.26 material finish, native ammo crates at native spots); the ported/v021d files the live '
                    'patches hold are shared, not repeated')
p.write_text(json.dumps(m, indent=2, ensure_ascii=False) + '\n')
PY
BASE=(); for f in "${PATCHES[@]}"; do BASE+=(--base-patch "$f"); done
"$PORTER" pack check --pack "$OUT" --base-db "$STOCK" "${BASE[@]}" | tail -3
(cd "$BUILD" && find "$ID-$VERSION" -type f -print0 | sort -z | xargs -0 sha256sum > "$ID-$VERSION.files.sha256" \
  && sha256sum "$ID-$VERSION/pack.json" "$ID-$VERSION.files.sha256" > "$ID-$VERSION.SHA256SUMS")
cat "$BUILD/$ID-$VERSION.SHA256SUMS"
