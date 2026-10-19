#!/usr/bin/env bash
# Skhid revision 22: maps-021d-8 = maps-021d-7 + native 0.26 artefact tuning on Vostok Radar Station (v0.21), 166 and
# its TDM 152 (owner-approved): three anomaly cores of v021d_level_03 day_2 and day_4, found by full name at their
# index (examples/set-project-leaves.rs, each leaf checked against the value it replaces):
#   antenna corridor_1 (onyx): artefacts_respawn_chance f32 0.2 -> i32 1 (native's type), respawn 120 -> 60 s, artefacts[0].count 1 -> 5;
#   electric anomaly (spring):  chance f32 0.5 -> i32 1, respawn 120 -> 70 s;
#   rust anomaly:               chance f32 0.5 -> i32 1, respawn 120 -> 70 s, max_artefacts_count 1 -> 2.
# Then version 8 and a note; proof: against -7 only those two projects (and the version, the note) change, and in
# them only those 16 leaves (dump-config --leaves); the pack check of -8 with maps-v011e-14 (manifests, files,
# cross-pack conflicts), and of -8 over stock + every released patch but 20 (12, 14, 17, 19, 21, 22): patch 20
# ships the 0.21d maps -8 replaces, so over it each of -8's add_files is "already in the base"; and v0.11's
# preconditions name patch 20's and 21's movies while patch 22 ships the v0.11 maps, so no one base takes both;
# files.sha256 and SHA256SUMS.021d-8.
#   rev22_021d_8.sh OUT_DIR
set -euo pipefail
OUT=$(realpath -m "$1"); mkdir -p "$OUT"
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=${PYTHON:-/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3}
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}
SET=${SET:-/mnt/hdd/Projects/release-20/crates-021d/target-crates/release/examples/set-project-leaves}
R22=/mnt/hdd/Projects/release-22
M021D_7=${M021D_7:-/mnt/hdd/Projects/release-20/packs/maps-021d-7}
V011E=${V011E:-$R22/packs/maps-v011e-14}
BP=$R22/base-patches
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
(cd $R22 && grep ' resources.db.patch_22$' SHA256SUMS | sha256sum -c --quiet)
[ "$(sha256sum $M021D_7/pack.json | cut -d' ' -f1)" = c56b5e1172207c73893b6aeeaa5c3e89f1042a656b2ac9b31df5e70d7d48378e ] || { echo "unexpected maps-021d-7"; exit 1; }
NAME=maps-021d-8; N=$OUT/$NAME
rm -rf "${N:?}"; cp -a --reflink=auto "$M021D_7" "$N"
"$SET" "$N" "$HERE/tools/content/rev22_021d_8.leaves.json" | tee "$OUT/$NAME.leaves.log"
$P - "$N" <<'PY'
import json, sys
from pathlib import Path
out = Path(sys.argv[1]); p = json.loads((out / 'pack.json').read_text()); assert p['version'] == '7'
p['version'] = '8'
p['notes'].append('maps-021d-8: native 0.26 artefact tuning on Vostok Radar Station (v0.21), 166 and 152 (v021d_level_03 day_2, day_4): '
                  'antenna corridor_1 core chance 1, respawn 60 s, 5 onyx; electric anomaly chance 1, respawn 70 s; rust anomaly chance 1, '
                  'respawn 70 s, at most 2 artefacts (chances as native\'s i32 1); nothing else differs from maps-021d-7')
(out / 'pack.json').write_text(json.dumps(p, indent=2, ensure_ascii=False) + '\n')
PY
# Proof against -7: files, manifest, and the projects' leaves.
for w in day_2 day_4; do
  for v in 7 8; do d=$M021D_7; [ $v = 8 ] && d=$N; $PACK_TOOL dump-config --file $d/files/projects/v021d_level_03/$w/client_project --leaves > "$OUT/.$w.$v.leaves"; done
  diff "$OUT/.$w.7.leaves" "$OUT/.$w.8.leaves" > "$OUT/$NAME.$w.leaves.diff" || true
done
$P - "$M021D_7" "$N" "$OUT" <<'PY'
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
expect = {'client_objects[126].artefacts_respawn_chance': ('f32:0.2', 'i32:1'), 'client_objects[126].artefacts_respawn_time_sec': ('i32:120', 'i32:60'),
          'client_objects[126].artefacts[0].count': ('i32:1', 'i32:5'),
          'client_objects[149].artefacts_respawn_chance': ('f32:0.5', 'i32:1'), 'client_objects[149].artefacts_respawn_time_sec': ('i32:120', 'i32:70'),
          'client_objects[189].artefacts_respawn_chance': ('f32:0.5', 'i32:1'), 'client_objects[189].artefacts_respawn_time_sec': ('i32:120', 'i32:70'),
          'client_objects[189].max_artefacts_count': ('i32:1', 'i32:2')}
leaves = {}
for w in ('day_2', 'day_4'):
    a = dict(l.rstrip('\n').split('\t', 1) for l in open(out / f'.{w}.7.leaves')); b = dict(l.rstrip('\n').split('\t', 1) for l in open(out / f'.{w}.8.leaves'))
    got = {k: (a.get(k), b.get(k)) for k in set(a) | set(b) if a.get(k) != b.get(k)}
    assert got == expect, (w, got)
    leaves[w] = {k: f'{x} -> {y}' for k, (x, y) in sorted(got.items())}
report = dict(old=str(old), new=str(new), files_changed=changed, manifest_keys_changed=keys, leaves_changed=leaves, files=len(fb))
(out / 'maps-021d-8.diff.json').write_text(json.dumps(report, indent=1) + '\n'); print(json.dumps(report, indent=1))
PY
rm -f "$OUT"/.day_?.?.leaves
echo "== pack check: $NAME + $(basename $V011E) together (manifests, files, cross-pack conflicts)"
$PACK_TOOL pack check --pack "$N" --pack "$V011E" | tail -3
echo "== pack check: $NAME over stock + 12 14 17 19 21 22 (every released patch but 20, which ships the 0.21d maps)"
B=(); for n in 12 14 17 19 21; do B+=(--base-patch $BP/resources.db.patch_$n); done; B+=(--base-patch $R22/resources.db.patch_22)
$PACK_TOOL pack check --pack "$N" --base-db $STOCK "${B[@]}" | tail -3
(cd "$OUT" && find $NAME -type f -print0 | sort -z | xargs -0 sha256sum > $NAME.files.sha256 \
  && sha256sum $NAME/pack.json $NAME.files.sha256 > SHA256SUMS.021d-8)
cat "$OUT/SHA256SUMS.021d-8"
