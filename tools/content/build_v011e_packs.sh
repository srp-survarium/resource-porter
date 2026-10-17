#!/usr/bin/env bash
# Skhid revision 18: the v0.11 maps as one content pack, over the stock archive and the released
# patches 12, 14 and 17.
#   Chemical Plant (v0.11): 206 batteries, 207 Team Deathmatch (level 12)
#   Vostok Radar Station (v0.11): 208 batteries, 209 Team Deathmatch (level 13)
# with the gameplay fill, the 0.11e loading picture (loading_screen_id 29, lobby_menu.swf) and the
# 0.11e minimaps (hud.swf keys v011e_level_04 / v011e_level_03).
#   [PACK_VERSION=N] build_v011e_packs.sh OUT_DIR [CHECK_WITH_PACK ...]   (writes OUT_DIR/maps-v011e-N)
set -euo pipefail
OUT=$1; shift
N=${PACK_VERSION:-1}; PK=maps-v011e-$N
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export PYTHONPATH=/nix/store/017x90g6glr0vrs1dws38wasaayhh0ba-python3.13-pillow-12.3.0/lib/python3.13/site-packages   # Pillow 12.3.0: the loading picture's JPEG
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
V=${VOSTOK_PORT:-$HERE/../target/release/vostok-port}          # this branch: the 0.11e builder
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}   # content packs (fix/021-maps-native-finish)
B="/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs"
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
PATCHES=(/home/sheep/skhid-patches/resources.db.patch_12 /home/sheep/skhid-patches/resources.db.patch_14 /mnt/hdd/Projects/release-17/resources.db.patch_17)
declare -A WANT=([resources.db]=c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c
  [resources.db.patch_12]=2b771e4b2040707434bb17aa27b1965aacba2ca355eb38634f504b61d6c8fab3
  [resources.db.patch_14]=60a69dcc6b661d09c3143fb74d6806f6f1ba6d465af3c29bfa669a58ce9a404c
  [resources.db.patch_17]=807b0e784c679a41009940d56ee76919d2bb5f14559154b927faf43405624081)
for f in $STOCK "${PATCHES[@]}"; do
  h=$(sha256sum "$f" | cut -d' ' -f1); [ "$h" = "${WANT[$(basename $f)]}" ] || { echo "unexpected $(basename $f): $h"; exit 1; }
done
G=$HERE/ports/gameplay-fill; W=${WORK:-$OUT.work}; mkdir -p $W $OUT
FILLS=(); for p in chemical-011e-anomaly chemical-011e-rust-fog chemical-011e-track-anomaly chemical-011e-resupply chemical-011e-containers \
                   radar-011e-anomaly radar-011e-resupply radar-011e-containers; do FILLS+=(--gameplay-fill $G/$p.json); done
rm -rf $W/bundle $OUT/$PK
(cd $HERE && $V build-survarium-maps-026 --manifest ports/survarium-011e-v011-release-to-026g.json --donor-db "$B/011e/app/resources.db" \
  --server-donor-db "$B/011a/app/resources.db" --target-db $STOCK --output $W/bundle "${FILLS[@]}") | tail -1
BP=(); for f in "${PATCHES[@]}"; do BP+=(--base-patch $f); done
$PACK_TOOL pack from-bundle --base-db $STOCK "${BP[@]}" --build-base-db $STOCK --bundle $W/bundle --output $OUT/$PK --id maps-v011e --version $N | tail -1
# The two movies as players have them now (lobby_menu.swf: patch 17; hud.swf: patch 14), and 0.11e's.
for s in lobby_menu hud; do
  for src in "${PATCHES[@]:2:1}" "${PATCHES[@]:1:1}" "${PATCHES[@]:0:1}" $STOCK; do
    $PACK_TOOL extract --db $src --path flash_movies/$s.swf --output $W/base-$s.swf >/dev/null 2>&1 && break
  done
done
for s in match_making hud; do $PACK_TOOL extract --db "$B/011e/app/resources.db" --path flash_movies/$s.swf --output $W/donor-$s.swf >/dev/null; done
rm -rf $W/ui; $P $HERE/tools/content/v011e_ui_swf.py tags $W/donor-match_making.swf $W/donor-hud.swf $W/base-lobby_menu.swf $W/base-hud.swf $W/ui
$P $HERE/tools/content/v011e_ui_swf.py code $W/ui
mkdir -p $OUT/$PK/files/flash_movies
cp $W/ui/lobby_menu.swf $W/ui/hud.swf $OUT/$PK/files/flash_movies/
$P - $OUT/$PK $W <<'PY'
import hashlib, json, sys
from pathlib import Path
pack, work = Path(sys.argv[1]), Path(sys.argv[2])
p = json.loads((pack / 'pack.json').read_text())
for s in ('lobby_menu', 'hud'):
    p['operations'].append({'op': 'replace_file', 'path': f'flash_movies/{s}.swf',
                            'base_sha256': hashlib.sha256((work / f'base-{s}.swf').read_bytes()).hexdigest()})
p['description'] = 'Chemical Plant (v0.11) 206/207 and Vostok Radar Station (v0.11) 208/209, with gameplay fill, 0.11e loading picture and minimaps'
p['server']['maps'] = [{'id': i} for i in (206, 207, 208, 209)]
p['notes'].append('flash_movies/lobby_menu.swf and hud.swf: tools/content/v011e_ui_swf.py over the released bytes (patch 17 / patch 14): '
                  'added the 0.11e loading picture (Level_v011e, loading_screen_id 29) and minimaps (v011e_level_04 / v011e_level_03); every other tag is byte-identical')
(pack / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
PY
$PACK_TOOL pack check --pack $OUT/$PK "${@/#/--pack=}" --base-db $STOCK "${BP[@]}" | tail -3
(cd $OUT && find $PK -type f | sort | xargs sha256sum > $PK.files.sha256 && sha256sum $PK/pack.json $PK.files.sha256 > SHA256SUMS)
cat $OUT/SHA256SUMS
