#!/usr/bin/env bash
# Skhid: the v0.11 maps as one content pack, over the stock archive and the released patches in BASE_PATCHES
# (revision 21: 12, 14, 17, 19, 20).
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
# The released patches, oldest first (release/patches.tsv), from BASE_PATCHES with its SHA256SUMS (checked).
BPD=${BASE_PATCHES:-/mnt/hdd/Projects/release-21/base-patches}
mapfile -t PATCHES < <(ls $BPD/resources.db.patch_* | sort -t_ -k2 -n)
(cd $BPD && sha256sum -c --quiet SHA256SUMS) || { echo "base patches do not match $BPD/SHA256SUMS"; exit 1; }
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
echo "base patches: $(for f in "${PATCHES[@]}"; do basename $f; done | tr '\n' ' ')"
G=$HERE/ports/gameplay-fill; W=${WORK:-$OUT.work}; mkdir -p $W $OUT
FILLS=(); for p in chemical-011e-anomaly chemical-011e-rust-fog chemical-011e-track-anomaly chemical-011e-resupply chemical-011e-containers \
                   radar-011e-anomaly radar-011e-resupply radar-011e-containers; do FILLS+=(--gameplay-fill $G/$p.json); done
# TRACK_FILL: another chemical-011e-track-anomaly.json (e.g. -4's, so that the track fog script applies after).
[ -n "${TRACK_FILL:-}" ] && FILLS=("${FILLS[@]/$G\/chemical-011e-track-anomaly.json/$TRACK_FILL}")
# SKIP_BUNDLE=1 keeps the bundle already built in the work dir (the same tree and inputs).
[ "${SKIP_BUNDLE:-}" = 1 ] && [ -d $W/bundle ] || { rm -rf $W/bundle; BUILD_BUNDLE=1; }
rm -rf $OUT/$PK
[ "${BUILD_BUNDLE:-}" = 1 ] && (cd $HERE && $V build-survarium-maps-026 --manifest ports/survarium-011e-v011-release-to-026g.json --donor-db "$B/011e/app/resources.db" \
  --server-donor-db "$B/011a/app/resources.db" --target-db $STOCK --output $W/bundle "${FILLS[@]}") | tail -1
# The bundle is converted over the patches up to 17, as -3 and -4 were: patch 19 (weapons and scopes) adds
# items_compatibility rows the stock-built bundle's dictionary does not have, which from-bundle reads as a
# removal. The check below runs over all of them, 19 included; the movies come from the newest that ships each.
BP=(); for f in "${PATCHES[@]:0:3}"; do BP+=(--base-patch $f); done
$PACK_TOOL pack from-bundle --base-db $STOCK "${BP[@]}" --build-base-db $STOCK --bundle $W/bundle --output $OUT/$PK --id maps-v011e --version $N | tail -1
BP=(); for f in "${PATCHES[@]}"; do BP+=(--base-patch $f); done
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
