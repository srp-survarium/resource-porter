#!/usr/bin/env bash
# The exo-oxygen-hud content pack: the Duty and Freedom exoskeletons' oxygen slot shows the exo's
# helmet (Duty 1805 -> 1800's icon, Freedom 1807 -> 1808's) in the HUD, as the native oxygen tanks
# show theirs. tools/content/exo_oxygen_hud_swf.py edits hud.swf as the maps-v011e pack ships it (a
# layered replace_file: the pack requires maps-v011e, its base_sha256 is that pack's hud.swf), with
# the helmets' icons cut from the lobby movie of the same pack. Then pack check of the revision's
# packs together over stock + the released patches 12, 14, 17, 19.
#   [PACK_VERSION=N] build_exo_oxygen_hud_pack.sh OUT_DIR       (writes OUT_DIR/exo-oxygen-hud-N)
# PACK_TOOL must have layered replace_file (this branch, fix/exo-oxygen-hud-icon).
set -euo pipefail
OUT=$1
N=${PACK_VERSION:-1}; PK=exo-oxygen-hud-$N
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export PYTHONPATH=/nix/store/017x90g6glr0vrs1dws38wasaayhh0ba-python3.13-pillow-12.3.0/lib/python3.13/site-packages   # Pillow 12.3.0: the atlas JPEG
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-$HERE/../target/release/vostok-port}
V011E=${V011E:-/mnt/hdd/Projects/release-20/packs/maps-v011e-4}
M021D=${M021D:-/mnt/hdd/Projects/release-20/packs/maps-021d-5}
BP=${BASE_PATCHES:-/mnt/hdd/Projects/release-20/base-patches}
PATCHES=(12 14 17 19)
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
mkdir -p "$OUT"; rm -rf "${OUT:?}/$PK"
mkdir -p "$OUT/$PK/files/flash_movies"
HUD=$V011E/files/flash_movies/hud.swf
$P $HERE/tools/content/exo_oxygen_hud_swf.py $HUD $V011E/files/flash_movies/lobby_menu.swf "$OUT/$PK/files/flash_movies/hud.swf"
$P - "$OUT/$PK" "$V011E" <<'PY'
import hashlib, json, sys
from pathlib import Path
pack, v011e = Path(sys.argv[1]), Path(sys.argv[2])
lower = json.loads((v011e / 'pack.json').read_text())
assert lower['id'] == 'maps-v011e'
hud = v011e / 'files/flash_movies/hud.swf'
assert [o for o in lower['operations'] if o.get('path') == 'flash_movies/hud.swf'][0]['op'] == 'replace_file'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
manifest = {
    'format_version': 1, 'id': 'exo-oxygen-hud', 'version': pack.name.rsplit('-', 1)[1], 'target_build': '0.26g',
    'description': "The exoskeletons' oxygen slot in the HUD shows the exo's helmet (Duty 1805 -> helmet 1800, Freedom 1807 -> helmet 1808)",
    'requires': ['maps-v011e'],
    'base_db_sha256': 'c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c',
    'operations': [{'op': 'replace_file', 'path': 'flash_movies/hud.swf', 'base_sha256': sha(hud)}],
    'notes': [
        f"flash_movies/hud.swf: tools/content/exo_oxygen_hud_swf.py over {lower['id']}-{lower['version']}'s hud.swf (sha256 {sha(hud)}), "
        "the helmets' icons from that pack's lobby_menu.swf (LobbyMenu_EquipmentIcons [3, 6] and [5, 6]): hudItems cells 28/29 "
        "(the atlas re-stored lossless, cells 0-27 as the JPEG decodes), QuickSlots.hudIcons ids 110/113 at 28/29, "
        "HUD.init's grey copies of 28-29, QuickItemRenderer.updateSlot's oxygen-tank test for 110/113; every other tag is byte-identical",
        'The game fills the oxygen slot with the oxygen item\'s ui_desc.icon; the exoskeletons carry it in the body (torso): '
        'Duty 1805 [1, 6] = id 110, Freedom 1807 [4, 6] = id 113, ids the native HUD has no icon for',
    ],
}
(pack / 'pack.json').write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
PY
BASE=(); for n in "${PATCHES[@]}"; do BASE+=(--base-patch $BP/resources.db.patch_$n); done
$PACK_TOOL pack check --pack "$M021D" --pack "$V011E" --pack "$OUT/$PK" --base-db $STOCK "${BASE[@]}" | tail -3
(cd "$OUT" && find $PK -type f | sort | xargs sha256sum > $PK.files.sha256 && sha256sum $PK/pack.json $PK.files.sha256 > $PK.SHA256SUMS)
cat "$OUT/$PK.SHA256SUMS" "$OUT/$PK.files.sha256"
