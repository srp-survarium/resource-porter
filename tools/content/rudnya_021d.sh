#!/usr/bin/env bash
# Rudnya (level_01) of retail 0.21d as the pack maps-021d-rudnya, built over the live patches the way
# maps-021d-school-fort made School and Tarakanovsky Fort (school_fort_021d.sh):
#   1. build-survarium021-maps with ports/survarium-021d-rudnya-to-026g.json (the two-maps manifest's shared
#      settings; donor local lighting; maps_dict 260, map number 29, st_level_29_name);
#   2. pack from-bundle against the stock archive (over the live patches the bundle would seem to remove their
#      dictionaries), then drop_live_files.py: the shared ported/v021d files patch_20 already holds are left out;
#      metal_column_05.material and floor_trash_burned_vc.material keep the live bytes (patch_20's bind 0.26's
#      floor_trash_spec where this build would bind 0.21d's; the live maps share the material);
#   3. add-021d-deathmatch: Team Deathmatch 261 (day_4);
#   4. native_crates.py + examples/edit-built-pack: native 0.26's ammo crates of the same level, weather and mode at
#      native's spots on 0.21d's floor (the layouts match: the importer's layout offset is 0), team 0's moved with
#      0.21d's team 0 spawns (27 m from native's);
#   5. the description, the pack check over stock + the live patches, SHA256SUMS.
#   rudnya_021d.sh BUILD_DIR SKHID_SRP   (SKHID_SRP: release/patches.tsv names the live patches)
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
OUT=$BUILD/maps-021d-rudnya-3

rm -rf "$BUILD/bundle" "$OUT" "$BUILD/fill"
"$PORTER" build-survarium021-maps --manifest "$HERE/ports/survarium-021d-rudnya-to-026g.json" \
  --donor-db "$DONOR" --target-db "$STOCK" --output "$BUILD/bundle"
"$PORTER" pack from-bundle --base-db "$STOCK" --build-base-db "$STOCK" --bundle "$BUILD/bundle" --output "$OUT" \
  --id maps-021d-rudnya --version 3
$P -I "$HERE/tools/content/drop_live_files.py" "$OUT" "$PORTER" \
  --keep-live material_instances/ported/v021d/tiled/metal/metal_column_05.material \
  --keep-live material_instances/ported/v021d/tiled/floor/floor_trash_burned_vc.material "${PATCHES[@]}"
"$PORTER" add-021d-deathmatch --pack "$OUT" --target-db "$STOCK" --variant level_01:day:261 --version 3
$P -I "$HERE/tools/content/native_crates.py" "$STOCK" "$BUILD/bundle/overlay" "$BUILD/fill" level_01:260:261
(cd "$HERE" && "$EDIT" "$OUT" "$BUILD/fill/plan.json" "$STOCK")

$P - "$OUT" <<'PY'
import json, sys
from pathlib import Path
p = Path(sys.argv[1]) / 'pack.json'; m = json.loads(p.read_text())
m['description'] = ('Rudnya (level_01) of retail 0.21d, day: maps_dict 260 (batteries), 261 (Team Deathmatch), '
                    'map number 29, as maps-021d-school-fort made School and Tarakanovsky Fort (0.21 local lighting and 0.26 material '
                    'finish, native ammo crates, team 0\'s moved with 0.21d\'s team 0 spawns); the ported/v021d '
                    'files patch_20 holds are shared, not repeated')
p.write_text(json.dumps(m, indent=2, ensure_ascii=False) + '\n')
PY
BASE=(); for f in "${PATCHES[@]}"; do BASE+=(--base-patch "$f"); done
"$PORTER" pack check --pack "$OUT" --base-db "$STOCK" "${BASE[@]}" | tail -3
(cd "$BUILD" && find maps-021d-rudnya-3 -type f -print0 | sort -z | xargs -0 sha256sum > maps-021d-rudnya-3.files.sha256 \
  && sha256sum maps-021d-rudnya-3/pack.json maps-021d-rudnya-3.files.sha256 > maps-021d-rudnya-3.SHA256SUMS)
cat "$BUILD/maps-021d-rudnya-3.SHA256SUMS"
