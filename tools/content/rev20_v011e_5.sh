#!/usr/bin/env bash
# Skhid revision 20: maps-v011e-<VERSION> (VERSION=5 or 6; default 5), derived from maps-v011e-4 with only the reviewed changes, in this order:
#   1. KIT_PLAN (default ports/rev20/maps-v011e-<VERSION>-kit-and-loading.json, edit-built-pack): 208/209 team 0 (Bob's) battery
#      container and ammo crate to the middle of the team's spawn hall; 206/207 loading_screen_id 29 -> 5
#      (Chemical gets the native Level4_day picture; Radar keeps 0.11e's, id 29);
#   2. FOG_PLAN (edit-built-pack; the track fog worker's): 206/207's growing track anomaly;
#   3. SPAWN_PLAN (edit-built-pack; the spawn worker's): respawn points of 206-209 only;
#   4. the lighting fixes (open-reversed-sphere-probes with the port manifest: 206/207's global probe opened,
#      environment_probe(2) specular 5 -> 1), then checked present in both projects;
#   5. hud.swf: the exoskeletons' oxygen slot (exo_oxygen_hud_swf.py of resource-porter
#      fix/exo-oxygen-hud-icon e6e0fca, over -4's hud.swf and -5's lobby_menu.swf);
# then a note and version 5, the diff against -4 (files and manifest), the pack check with the 0.21d pack over
# stock + 12, 14, 17, 19, and SHA256SUMS.
#   rev20_v011e_5.sh OUT_DIR FOG_PLAN|FOG_SCRIPT.sh|- SPAWN_PLAN|SPAWN_SCRIPT.sh|- [M021D_PACK]
set -euo pipefail
OUT=$1; FOG=$2; SPAWN=$3; M021D=${4:-/mnt/hdd/Projects/release-20/packs/maps-021d-5}
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export PYTHONPATH=/nix/store/017x90g6glr0vrs1dws38wasaayhh0ba-python3.13-pillow-12.3.0/lib/python3.13/site-packages
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
TARGET=${CARGO_TARGET_DIR:-$HERE/../target}
EDIT=$TARGET/release/examples/edit-built-pack; PROBES=$TARGET/release/examples/open-reversed-sphere-probes
EXO=${EXO_TOOL:-/mnt/hdd/Projects/exo-oxygen-icon/porter/tools/content/exo_oxygen_hud_swf.py}
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}
V4=${V011E_4:-/mnt/hdd/Projects/release-20/packs/maps-v011e-4}
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
BP=${BASE_PATCHES:-/mnt/hdd/Projects/release-20/base-patches}
(cd $BP && sha256sum -c --quiet SHA256SUMS)
[ -x $EDIT ] && [ -x $PROBES ] || { echo "build the examples edit-built-pack and open-reversed-sphere-probes first"; exit 1; }
[ "$(sha256sum $V4/files/flash_movies/hud.swf | cut -c1-16)" = 3be3ba15ce912c5b ] || { echo "-4's hud.swf is not 3be3ba15"; exit 1; }
VERSION=${VERSION:-5}; KIT_PLAN=${KIT_PLAN:-ports/rev20/maps-v011e-$VERSION-kit-and-loading.json}
NAME=maps-v011e-$VERSION
W=${WORK:-$OUT.work$VERSION}; mkdir -p "$W" "$OUT"; rm -rf "${OUT:?}/$NAME"
cp -a --reflink=auto "$V4" "$OUT/$NAME"
N=$OUT/$NAME
echo "== 1. kit and loading ($KIT_PLAN)"; (cd $HERE && $EDIT $N $KIT_PLAN $STOCK)
# FOG / SPAWN: an edit-built-pack plan (.json) or the worker's apply script (.sh: run with the pack dir).
if [ "$FOG" != - ]; then echo "== 2. fog ($FOG)"
  case "$FOG" in *.sh) (cd $HERE && bash "$FOG" "$N" $STOCK) ;; *) (cd $HERE && $EDIT $N "$FOG" $STOCK) ;; esac; fi
if [ "$SPAWN" != - ]; then echo "== 3. spawns ($SPAWN)"
  case "$SPAWN" in *.sh) (cd $HERE && bash "$SPAWN" "$N") ;; *) (cd $HERE && $EDIT $N "$SPAWN" $STOCK) ;; esac; fi
echo "== 4. lighting"; (cd $HERE && $PROBES $N ports/survarium-011e-v011-release-to-026g.json)
for d in day_2 day_4; do
  L=$($PACK_TOOL dump-config --leaves --file $N/files/projects/v011e_level_04/$d/client_project)
  grep -q 'environment_probe(2)' <<<"$L" || { echo "206/207 $d: no environment_probe(2)"; exit 1; }
  $P - "$N/files/projects/v011e_level_04/$d/client_project" "$PACK_TOOL" <<'PY'
import subprocess, sys, re
leaves = subprocess.run([sys.argv[2], 'dump-config', '--leaves', '--file', sys.argv[1]], capture_output=True, text=True, errors='replace').stdout
probes = {}
for line in leaves.splitlines():
    m = re.match(r'(client_objects\[\d+\])\.(full_name|specular_multiplier|probe_side_width)\t(.*)$', line)
    if m: probes.setdefault(m.group(1), {})[m.group(2)] = m.group(3)
named = {v.get('full_name', '').strip('string:"'): v for v in probes.values()}
p2 = next(v for k, v in probes.items() if v.get('full_name', '').endswith('environment_probes/environment_probe(2)"') or v.get('full_name', '').endswith('probes/environment_probe(2)"'))
p1 = next(v for k, v in probes.items() if v.get('full_name', '').endswith('probes/environment_probe"'))
num = lambda v: float(v.split(':', 1)[1])
assert num(p2['specular_multiplier']) == 1.0, p2
assert abs(num(p1['probe_side_width']) - 7.818) < 1e-3, p1
print(sys.argv[1].split('/projects/')[1], 'lighting leaves present:', p2['specular_multiplier'], p1['probe_side_width'])
PY
done
echo "== 5. exo oxygen HUD icon"
cp $V4/files/flash_movies/hud.swf $W/hud-4.swf
$P $EXO $W/hud-4.swf $N/files/flash_movies/lobby_menu.swf $W/hud-5.swf
cp $W/hud-5.swf $N/files/flash_movies/hud.swf
$P - "$N" "$V4" "$W" "$FOG" "$SPAWN" "$VERSION" "$HERE/$KIT_PLAN" <<'PY'
import hashlib, json, sys
from pathlib import Path
new, old, work, fog, spawn, version = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4], sys.argv[5], sys.argv[6]
kit = json.loads(Path(sys.argv[7]).read_text())['projects']['projects/v011e_level_03/day_2/client_project']
p = json.loads((new / 'pack.json').read_text()); assert p['version'] == '4'
p['version'] = version
hud = [o for o in p['operations'] if o['op'] == 'replace_file' and o['path'] == 'flash_movies/hud.swf']
assert len(hud) == 1    # the precondition stays the released patch-14 hud.swf
p['notes'].append(f'maps-v011e-{version}: 208/209 team 0 battery container and ammo crate moved to the team\'s spawn hall '
                  f'(container {tuple(kit[0]["to"]["position"])}, crate {tuple(kit[1]["to"]["position"])}, both level at yaw pi/2; 209 the crate); '
                  '206/207 ui.loading_screen_id 5 (native Level4_day; 0.11e has no Chemical picture); '
                  + ('206/207 growing track fog rebuilt (track fog worker\'s plan); ' if fog != '-' else '')
                  + ('respawn point fixes of 206-209 (spawn worker\'s plan); ' if spawn != '-' else '')
                  + 'lighting: 206/207 global probe opened (probe_side_width 0.5 -> 7.818), environment_probe(2) specular 5 -> 1; '
                  'hud.swf: the exoskeletons\' oxygen slot shows their helmet (exo_oxygen_hud_swf.py, fix/exo-oxygen-hud-icon e6e0fca: '
                  'hudItems cells 28/29, hudIcons 110/113, their grey copies, the empty-tank test)')
(new / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
def files(d): return {str(f.relative_to(d)): hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted((d / 'files').rglob('*')) if f.is_file()}
fa, fb = files(old), files(new)
changed = sorted(k for k in set(fa) | set(fb) if fa.get(k) != fb.get(k))
po = json.loads((old / 'pack.json').read_text())
manifest = [k for k in sorted(set(po) | set(p)) if po.get(k) != p.get(k)]
ops = [i for i, (x, y) in enumerate(zip(po['operations'], p['operations'])) if x != y] + (['count'] if len(po['operations']) != len(p['operations']) else [])
report = dict(old=str(old), new=str(new), files_changed=changed, manifest_keys_changed=manifest, operations_changed=ops)
(new.parent / f'maps-v011e-{version}.diff.json').write_text(json.dumps(report, indent=1) + '\n'); print(json.dumps(report, indent=1))
PY
BASE=(); for n in 12 14 17 19; do BASE+=(--base-patch $BP/resources.db.patch_$n); done
$PACK_TOOL pack check --pack "$M021D" --pack "$N" --base-db $STOCK "${BASE[@]}" | tail -3
(cd "$OUT" && find $NAME -type f -print0 | sort -z | xargs -0 sha256sum > $NAME.files.sha256 \
  && sha256sum $NAME/pack.json $NAME.files.sha256 > SHA256SUMS.v011e-$VERSION)
cat "$OUT/SHA256SUMS.v011e-$VERSION"
