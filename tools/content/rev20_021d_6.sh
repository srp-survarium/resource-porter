#!/usr/bin/env bash
# Skhid revision 20: maps-021d-6 = maps-021d-5 + one native 0.26 ammo crate per team on each 0.21d map
# (+ the spawn worker's 0.21d spawn fixes), derived in place from the built -5 (not rebuilt from the bundle):
#   1. copy maps-021d-5;
#   2. the crates: edit-built-pack (resource-porter a0b5933, examples/edit-built-pack.rs) applies the four
#      gameplay-fill packs of rev20_021d_6.plan.json, one per project: native level_04's resupply set (zone,
#      collision box, ammo decal, item/quest/container_closed render + collision instance) per team, renamed
#      logic/fill/teamN/, moved rigidly to the reviewed spots (166/175 beside each team's battery container,
#      152/153 in each team's spawn area). The template is stock's projects/level_04/day_2, hash-checked; the
#      crate model is native, so no file is added;
#   3. the spawn fixes, after the crates (they re-check every point against the crates' new collision):
#      SPAWNS=<resource-porter rev20-spawn-check>/tools/spawn-check/apply-spawns-021d.sh, unless SPAWNS=none
#      (then the result is maps-021d-6-pre, crates only);
#   4. version 6 and a note;
#   5. proof: manifest and file diff against -5 (only the four projects change), per-object diff of each
#      project (only the crate objects and spawn objects), the pack check with maps-v011e (V011E, default the newest of -6, -5,
#      -4) over stock + 12, 14, 17, 19, files.sha256 and SHA256SUMS lines.
#   rev20_021d_6.sh BUILD_DIR
set -euo pipefail
BUILD=$(realpath -m "$1"); mkdir -p "$BUILD"
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=${PYTHON:-/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3}
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}
EDIT=${EDIT:-/mnt/hdd/Projects/release-20/crates-021d/target-a0b5933/release/examples/edit-built-pack}
SPAWNS=${SPAWNS:-/mnt/hdd/Projects/release-20/spawns/porter/tools/spawn-check/apply-spawns-021d.sh}
CHECKS=${CHECKS:-$HERE/tools/content}   # project_diff.py
R20=/mnt/hdd/Projects/release-20
M021D_5=${M021D_5:-$R20/packs/maps-021d-5}
V011E=${V011E:-$(for v in 6 5 4; do [ -d $R20/packs/maps-v011e-$v ] && { echo $R20/packs/maps-v011e-$v; break; }; done)}
BP=${BASE_PATCHES:-$R20/base-patches}
PATCHES=(12 14 17 19)
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
[ "$(sha256sum $M021D_5/pack.json | cut -d' ' -f1)" = 5675aa071c327f26857b3d7667212729c73e3b61b9afa93311ddfd1f5aef9f29 ] || { echo "unexpected maps-021d-5"; exit 1; }
NAME=maps-021d-6; [ "$SPAWNS" = none ] && NAME=maps-021d-6-pre
OUT=$BUILD/$NAME
rm -rf "${OUT:?}" "$OUT.spawn-check"
cp -a --reflink=auto "$M021D_5" "$OUT"

echo "== crates"
(cd "$HERE" && "$EDIT" "$OUT" tools/content/rev20_021d_6.plan.json $STOCK) | tee "$BUILD/$NAME.crates.log"
if [ "$SPAWNS" != none ]; then
  echo "== spawns ($SPAWNS, resource-porter $(git -C "$(dirname "$SPAWNS")" rev-parse --short HEAD))"
  "$SPAWNS" "$OUT" | tee "$BUILD/$NAME.spawns.log"
fi

$P - "$OUT" "$M021D_5" "$NAME" "$SPAWNS" <<'PY'
import json, sys
from pathlib import Path
out, old, name, spawns = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]
p = json.loads((out / 'pack.json').read_text()); assert p['version'] == '5'
p['version'] = '6'
p['notes'].append(f'{name}: one native 0.26 ammo crate per team (native level_04 logic/vip/teamN/resupply: resupply_zone, '
                  'resupply_collision, ammo_decal, item/quest/container_closed; renamed logic/fill/teamN/resupply) on each map: '
                  '166/175 beside each team\'s battery container, 152/153 in each team\'s spawn area '
                  '(ports/gameplay-fill/*-021d-*resupply.json)'
                  + ('' if spawns == 'none' else '; the revision 20 spawn fixes (respawn points and their zones, resource-porter rev20-spawn-check)')
                  + '; nothing else differs from maps-021d-5')
(out / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
PY

echo "== proof"
$P - "$OUT" "$M021D_5" "$BUILD" <<'PY'
import hashlib, json, sys
from pathlib import Path
new, old, build = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
def load(p): return json.loads((p / 'pack.json').read_text())
def files(p): return {str(f.relative_to(p)): hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted((p / 'files').rglob('*')) if f.is_file()}
def diff(a, b, path=''):
    if type(a) != type(b): return [f'{path}: {a!r} -> {b!r}']
    if isinstance(a, dict):
        return [d for k in sorted(set(a) | set(b)) for d in (diff(a.get(k), b.get(k), f'{path}.{k}') if k in a and k in b else [f'{path}.{k}: {"added" if k in b else "removed"}'])]
    if isinstance(a, list):
        if len(a) != len(b): return [f'{path}: {len(a)} -> {len(b)} entries'] + [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{path}[{i}]')]
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{path}[{i}]')]
    return [] if a == b else [f'{path}: {a!r} -> {b!r}']
fa, fb = files(old), files(new)
changed = sorted(k for k in set(fa) | set(fb) if fa.get(k) != fb.get(k))
manifest = diff(load(old), load(new))
report = dict(old=str(old), new=str(new), manifest=[d if len(d) < 300 else d[:300] + '…' for d in manifest], files_changed=changed, files=len(fb))
(build / f'{new.name}.diff.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
print(json.dumps(report, indent=1, ensure_ascii=False))
projects = {f'files/projects/v021d_level_0{l}/day_{w}/client_project' for l in (3, 6) for w in (2, 4)}
assert set(changed) == projects, f'files changed beyond the four projects: {sorted(set(changed) ^ projects)}'
assert [d.split(':')[0] for d in manifest] == ['.notes', '.version'], manifest
PY
$P -I "$CHECKS/project_diff.py" "$M021D_5" "$OUT" projects/v021d_level_0{3,6}/day_{2,4}/client_project > "$BUILD/$NAME.objects.json" \
  || { echo "a project change is neither a crate nor a spawn object: $BUILD/$NAME.objects.json"; exit 1; }
$P - "$BUILD/$NAME.objects.json" <<'PY'
import collections, json, sys
for p, changes in json.load(open(sys.argv[1])).items():
    c = collections.Counter(f"{x['change']} {x['path']}" for x in changes); print(p, dict(c))
PY

BASE=(); for n in "${PATCHES[@]}"; do BASE+=(--base-patch $BP/resources.db.patch_$n); done
echo "== pack check ($NAME + $(basename $V011E) over stock + ${PATCHES[*]})"
$PACK_TOOL pack check --pack "$OUT" --pack "$V011E" --base-db $STOCK "${BASE[@]}" | tail -3
(cd "$BUILD" && find $NAME -type f -print0 | sort -z | xargs -0 sha256sum > $NAME.files.sha256 \
  && sha256sum $NAME/pack.json $NAME.files.sha256 > $NAME.SHA256SUMS)
cat "$BUILD/$NAME.SHA256SUMS"
