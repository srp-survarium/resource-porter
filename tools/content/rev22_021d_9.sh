#!/usr/bin/env bash
# Skhid revision 22: maps-021d-9 = maps-021d-8 + one more artefact at once on Vostok Radar Station (v0.21), 166 and
# its TDM 152 (the owner: 5 on the map at once, -8 gives 1 + 1 + 2): the core with the most containers, the rust
# field's (anomaly/anomaly_rust/rust_anomaly, client_objects[189], 10 containers; corridor_1 has 3, the electric
# anomaly 5), max_artefacts_count 2 -> 3 in day_2 and day_4 (examples/set-project-leaves.rs, checked against the old
# value). Its pool, 5 spring (74) + 2 onyx (76) = 7, stays above the max. Then version 9 and a note; proof against -8:
# only those two projects (and the version, the note), and in them only that leaf; the pack checks as -8's
# (rev22_021d_8.sh): with maps-v011e-14, and -9 over stock + every released patch but 20; files.sha256, SHA256SUMS.021d-9.
#   rev22_021d_9.sh OUT_DIR
set -euo pipefail
OUT=$(realpath -m "$1"); mkdir -p "$OUT"
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=${PYTHON:-/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3}
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}
SET=${SET:-/mnt/hdd/Projects/release-20/crates-021d/target-crates/release/examples/set-project-leaves}
R22=/mnt/hdd/Projects/release-22
M021D_8=${M021D_8:-/mnt/hdd/Projects/release-22/packs/maps-021d-8}
V011E=${V011E:-$R22/packs/maps-v011e-14}
BP=$R22/base-patches
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
(cd $R22 && grep ' resources.db.patch_22$' SHA256SUMS | sha256sum -c --quiet)
[ "$(sha256sum $M021D_8/pack.json | cut -d' ' -f1)" = a6c3b87c13ceb872504d9dc56497808785651acf5ac8901fe19ea858288792b9 ] || { echo "unexpected maps-021d-8"; exit 1; }
NAME=maps-021d-9; N=$OUT/$NAME
rm -rf "${N:?}"; cp -a --reflink=auto "$M021D_8" "$N"
"$SET" "$N" "$HERE/tools/content/rev22_021d_9.leaves.json" | tee "$OUT/$NAME.leaves.log"
$P - "$N" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1]); p = json.loads((out / 'pack.json').read_text()); assert p['version'] == '8'
p['version'] = '9'
p['notes'].append('maps-021d-9: Vostok Radar Station (v0.21), 166 and 152: the rust anomaly (10 containers) holds at most 3 artefacts '
                  'at once (was 2), so the map holds 5 (1 + 1 + 3); nothing else differs from maps-021d-8')
(out / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
PY
# Proof against -7: files, manifest, and the projects' leaves.
for w in day_2 day_4; do
  for v in 8 9; do d=$M021D_8; [ $v = 9 ] && d=$N; $PACK_TOOL dump-config --file $d/files/projects/v021d_level_03/$w/client_project --leaves > "$OUT/.$w.$v.leaves"; done
  diff "$OUT/.$w.8.leaves" "$OUT/.$w.9.leaves" > "$OUT/$NAME.$w.leaves.diff" || true
done
$P - "$M021D_8" "$N" "$OUT" <<'PY'
import hashlib, json, sys
from pathlib import Path
old, new, out = map(Path, sys.argv[1:])
def files(p): return {str(f.relative_to(p)): hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted((p / 'files').rglob('*')) if f.is_file()}
fa, fb = files(old), files(new)
changed = sorted(k for k in set(fa) | set(fb) if fa.get(k) != fb.get(k))
po, pn = (json.loads((d / 'pack.json').read_text()) for d in (old, new))
keys = sorted(k for k in set(po) | set(pn) if po.get(k) != pn.get(k))
assert changed == [f'files/projects/v021d_level_03/day_{w}/client_project' for w in (2, 4)], changed
assert keys == ['notes', 'version'] and pn['notes'][:-1] == po['notes'], keys
expect = {'client_objects[189].max_artefacts_count': ('i32:2', 'i32:3')}
leaves = {}
for w in ('day_2', 'day_4'):
    a = dict(l.rstrip('\n').split('\t', 1) for l in open(out / f'.{w}.8.leaves')); b = dict(l.rstrip('\n').split('\t', 1) for l in open(out / f'.{w}.9.leaves'))
    got = {k: (a.get(k), b.get(k)) for k in set(a) | set(b) if a.get(k) != b.get(k)}
    assert got == expect, (w, got)
    leaves[w] = {k: f'{x} -> {y}' for k, (x, y) in sorted(got.items())}
report = dict(old=str(old), new=str(new), files_changed=changed, manifest_keys_changed=keys, leaves_changed=leaves, files=len(fb))
(out / 'maps-021d-9.diff.json').write_text(json.dumps(report, indent=1) + '\n'); print(json.dumps(report, indent=1))
PY
rm -f "$OUT"/.day_?.?.leaves
echo "== pack check: $NAME + $(basename $V011E) together (manifests, files, cross-pack conflicts)"
$PACK_TOOL pack check --pack "$N" --pack "$V011E" | tail -3
echo "== pack check: $NAME over stock + 12 14 17 19 21 22 (every released patch but 20, which ships the 0.21d maps)"
B=(); for n in 12 14 17 19 21; do B+=(--base-patch $BP/resources.db.patch_$n); done; B+=(--base-patch $R22/resources.db.patch_22)
$PACK_TOOL pack check --pack "$N" --base-db $STOCK "${B[@]}" | tail -3
(cd "$OUT" && find $NAME -type f -print0 | sort -z | xargs -0 sha256sum > $NAME.files.sha256 \
  && sha256sum $NAME/pack.json $NAME.files.sha256 > SHA256SUMS.021d-9)
cat "$OUT/SHA256SUMS.021d-9"
