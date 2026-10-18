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
# The released patches, oldest first (release/patches.tsv): revision 20 builds over 12, 14, 17 and 19.
BPD=${BASE_PATCHES:-/mnt/hdd/Projects/release-20/base-patches}
PATCHES=($BPD/resources.db.patch_12 $BPD/resources.db.patch_14 $BPD/resources.db.patch_17 $BPD/resources.db.patch_19)
declare -A WANT=([resources.db]=c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c
  [resources.db.patch_12]=2b771e4b2040707434bb17aa27b1965aacba2ca355eb38634f504b61d6c8fab3
  [resources.db.patch_14]=60a69dcc6b661d09c3143fb74d6806f6f1ba6d465af3c29bfa669a58ce9a404c
  [resources.db.patch_17]=807b0e784c679a41009940d56ee76919d2bb5f14559154b927faf43405624081
  [resources.db.patch_19]=8bb0e952014e8420ecd1b959631ec9e62e2f7c6ad04e14293fd8d726a6f9840b)
for f in $STOCK "${PATCHES[@]}"; do
  h=$(sha256sum "$f" | cut -d' ' -f1); [ "$h" = "${WANT[$(basename $f)]}" ] || { echo "unexpected $(basename $f): $h"; exit 1; }
done
G=$HERE/ports/gameplay-fill; W=${WORK:-$OUT.work}; mkdir -p $W $OUT
FILLS=(); for p in chemical-011e-anomaly chemical-011e-rust-fog chemical-011e-track-anomaly chemical-011e-resupply chemical-011e-containers \
                   radar-011e-anomaly radar-011e-resupply radar-011e-containers; do FILLS+=(--gameplay-fill $G/$p.json); done
# TRACK_FILL: another chemical-011e-track-anomaly.json (e.g. -4's, so that the track fog script applies after).
[ -n "${TRACK_FILL:-}" ] && FILLS=("${FILLS[@]/$G\/chemical-011e-track-anomaly.json/$TRACK_FILL}")
rm -rf $W/bundle $OUT/$PK
(cd $HERE && $V build-survarium-maps-026 --manifest ports/survarium-011e-v011-release-to-026g.json --donor-db "$B/011e/app/resources.db" \
  --server-donor-db "$B/011a/app/resources.db" --target-db $STOCK --output $W/bundle "${FILLS[@]}") | tail -1
BP=(); for f in "${PATCHES[@]}"; do BP+=(--base-patch $f); done
$PACK_TOOL pack from-bundle --base-db $STOCK "${BP[@]}" --build-base-db $STOCK --bundle $W/bundle --output $OUT/$PK --id maps-v011e --version $N | tail -1
# The two movies as players have them now (each from the newest released patch that ships it), and 0.11e's.
for s in lobby_menu hud; do
  for ((i = ${#PATCHES[@]} - 1; i >= -1; i--)); do
    src=$([ $i -ge 0 ] && echo ${PATCHES[i]} || echo $STOCK)
    if $PACK_TOOL list --db $src | grep -x "flash_movies/$s.swf" >/dev/null; then
      $PACK_TOOL extract --db $src --path flash_movies/$s.swf --output $W/base-$s.swf >/dev/null; echo "base $s.swf: $(basename $src)"; break
    fi
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
p['notes'].append('flash_movies/lobby_menu.swf and hud.swf: tools/content/v011e_ui_swf.py over the newest released bytes of each: '
                  'added the 0.11e loading picture (Level_v011e, loading_screen_id 29), the Matches window\'s levels 9-13 and the 0.11e minimaps '
                  '(v011e_level_04 / v011e_level_03); every other tag is byte-identical')
(pack / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
PY
$PACK_TOOL pack check --pack $OUT/$PK "${@/#/--pack=}" --base-db $STOCK "${BP[@]}" | tail -3
(cd $OUT && find $PK -type f -print0 | sort -z | xargs -0 sha256sum > $PK.files.sha256 && sha256sum $PK/pack.json $PK.files.sha256 > SHA256SUMS.$PK)
cat $OUT/SHA256SUMS.$PK
