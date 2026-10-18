#!/usr/bin/env bash
# Skhid revision 20's UI pack (ui-rev20-N): two fixes over the released movies, no map content.
#   flash_movies/hud.swf        patch 14's + the exoskeletons' oxygen slot icon (exo_oxygen_hud_swf.py)
#   flash_movies/lobby_menu.swf patch 19's + the Matches window's levels 9-13 (ui_rev20_swf.py)
# Each a replace_file of the released bytes, no `requires`; then pack check with the revision's map
# pack over stock + the released patches 12, 14, 17, 19.
#   [PACK_VERSION=N] [M021D=<maps-021d pack>] build_ui_rev20_pack.sh OUT_DIR   (writes OUT_DIR/ui-rev20-N)
set -euo pipefail
OUT=$1
N=${PACK_VERSION:-1}; PK=ui-rev20-$N
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export PYTHONPATH=/nix/store/017x90g6glr0vrs1dws38wasaayhh0ba-python3.13-pillow-12.3.0/lib/python3.13/site-packages   # Pillow 12.3.0: the HUD atlas JPEG
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-/tmp/r18/target/release/vostok-port}
M021D=${M021D:-/mnt/hdd/Projects/release-20/packs/maps-021d-7}
BP=${BASE_PATCHES:-/mnt/hdd/Projects/release-20/base-patches}
PATCHES=(12 14 17 19)
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
W=${WORK:-$OUT/$PK.work}; rm -rf "${W:?}" "${OUT:?}/$PK"; mkdir -p "$W" "$OUT/$PK/files/flash_movies"
# Each movie as players have it now: from the newest released patch that ships it.
for s in lobby_menu hud; do
  for ((i = ${#PATCHES[@]} - 1; i >= 0; i--)); do
    if $PACK_TOOL list --db $BP/resources.db.patch_${PATCHES[i]} | grep -x "flash_movies/$s.swf" >/dev/null; then
      $PACK_TOOL extract --db $BP/resources.db.patch_${PATCHES[i]} --path flash_movies/$s.swf --output $W/base-$s.swf >/dev/null
      echo "base $s.swf: patch_${PATCHES[i]} $(sha256sum $W/base-$s.swf | cut -d' ' -f1)"; break
    fi
  done
done
$P $HERE/tools/content/ui_rev20_swf.py $W/base-hud.swf $W/base-lobby_menu.swf $W/ui
cp $W/ui/hud.swf $W/ui/lobby_menu.swf "$OUT/$PK/files/flash_movies/"
$P - "$OUT/$PK" "$W" <<'PY'
import hashlib, json, sys
from pathlib import Path
pack, work = Path(sys.argv[1]), Path(sys.argv[2])
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
hud, lobby = sha(work / 'base-hud.swf'), sha(work / 'base-lobby_menu.swf')
manifest = {
    'format_version': 1, 'id': 'ui-rev20', 'version': pack.name.rsplit('-', 1)[1], 'target_build': '0.26g',
    'description': "Revision 20's UI fixes: the exoskeletons' oxygen slot shows the exo's helmet; the Matches window names levels 9-13",
    'base_db_sha256': 'c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c',
    'operations': [
        {'op': 'replace_file', 'path': 'flash_movies/hud.swf', 'base_sha256': hud},
        {'op': 'replace_file', 'path': 'flash_movies/lobby_menu.swf', 'base_sha256': lobby},
    ],
    'notes': [
        f'flash_movies/hud.swf: the released movie (resources.db.patch_14, sha256 {hud}) with tools/content/exo_oxygen_hud_swf.py: '
        "the Duty 1805 / Freedom 1807 oxygen slot (ids 110 / 113) shows helmet 1800's / 1808's lobby icon (hudItems cells 28/29, "
        'the atlas re-stored lossless, cells 0-27 as the JPEG decodes), greyed when empty as the native tanks; '
        'tags hudItems, HUD, QuickSlots, QuickItemRenderer change, every other tag is byte-identical',
        f'flash_movies/lobby_menu.swf: the released movie (resources.db.patch_19, sha256 {lobby}) with tools/content/ui_rev20_swf.py: '
        "BottomPanel.matchesNameTable() gets levels 9-13 in its five languages (12/13, the v0.11 maps, unused until they ship); "
        'only the BottomPanel DoABC tag changes',
    ],
}
(pack / 'pack.json').write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
PY
BASE=(); for n in "${PATCHES[@]}"; do BASE+=(--base-patch $BP/resources.db.patch_$n); done
$PACK_TOOL pack check --pack "$M021D" --pack "$OUT/$PK" --base-db $STOCK "${BASE[@]}" | tail -3
rm -rf "${W:?}"
(cd "$OUT" && find $PK -type f | sort | xargs sha256sum > $PK.files.sha256 && sha256sum $PK/pack.json $PK.files.sha256 > SHA256SUMS.$PK)
cat "$OUT/SHA256SUMS.$PK" "$OUT/$PK.files.sha256"
