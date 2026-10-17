#!/usr/bin/env bash
# Skhid revision 20: maps-v011e-4 and maps-021d-5, each derived from the pack before it with only the
# owner's changes, then checked together over stock + the released patches 12, 14, 17, 19.
#   maps-v011e-4 = maps-v011e-3 + flash_movies/lobby_menu.swf rebuilt by v011e_ui_swf.py over the newest
#                  released movie (patch 19's): the 0.11e loading picture as before plus the Matches window's
#                  levels 9-13; its replace precondition names patch 19's bytes. hud.swf (newest: patch 14)
#                  and every other file and operation as in -3.
#   maps-021d-5  = maps-021d-4 with English st_level_10_name "Vostok Radar Station (v0.21)" (the owner's
#                  rule: native 0.26 name + source version); every other string, file and operation as in -4.
#   rev19_packs.sh OUT_DIR
set -euo pipefail
OUT=$1; mkdir -p "$OUT"
HERE=$(cd -- "$(dirname -- "$0")/../.." && pwd)
P=/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3
export PYTHONPATH=/nix/store/017x90g6glr0vrs1dws38wasaayhh0ba-python3.13-pillow-12.3.0/lib/python3.13/site-packages
export LD_LIBRARY_PATH=/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib
PACK_TOOL=${PACK_TOOL:-/tmp/rev17-metal-fix/target/release/vostok-port}
V011E_3=${V011E_3:-/mnt/hdd/Projects/rev18-v011-maps/packs/maps-v011e-3}
M021D_4=${M021D_4:-/mnt/hdd/Projects/rev17-metal-fix/pack/maps-021d-4}
BP=${BASE_PATCHES:-/mnt/hdd/Projects/release-20/base-patches}
PATCHES=(12 14 17 19)
B="/mnt/hdd/Projects/Survarium Builds/pdb-structure-inputs"
STOCK=/home/sheep/Projects/survarium/srp/0.26g/game/resources.db
[ "$(sha256sum $STOCK | cut -d' ' -f1)" = c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c ] || { echo "unexpected stock archive"; exit 1; }
(cd $BP && sha256sum -c --quiet SHA256SUMS)
W=${WORK:-$OUT.work}; rm -rf "${W:?}" "${OUT:?}/maps-v011e-4" "${OUT:?}/maps-021d-5"; mkdir -p "$W"

# maps-v011e-4
cp -a --reflink=auto "$V011E_3" "$OUT/maps-v011e-4"
# Each movie as players have it now: from the newest released patch that ships it.
for s in lobby_menu hud; do
  for ((i = ${#PATCHES[@]} - 1; i >= 0; i--)); do
    if $PACK_TOOL list --db $BP/resources.db.patch_${PATCHES[i]} | grep -x "flash_movies/$s.swf" >/dev/null; then
      $PACK_TOOL extract --db $BP/resources.db.patch_${PATCHES[i]} --path flash_movies/$s.swf --output $W/base-$s.swf >/dev/null
      echo "base $s.swf: patch_${PATCHES[i]}"; break
    fi
  done
done
for s in match_making hud; do $PACK_TOOL extract --db "$B/011e/app/resources.db" --path flash_movies/$s.swf --output $W/donor-$s.swf >/dev/null; done
$P $HERE/tools/content/v011e_ui_swf.py tags $W/donor-match_making.swf $W/donor-hud.swf $W/base-lobby_menu.swf $W/base-hud.swf $W/ui
$P $HERE/tools/content/v011e_ui_swf.py code $W/ui
cmp $W/ui/hud.swf "$V011E_3/files/flash_movies/hud.swf"           # hud.swf is -3's, byte for byte (no newer hud.swf)
cp $W/ui/lobby_menu.swf "$OUT/maps-v011e-4/files/flash_movies/lobby_menu.swf"
# maps-021d-5
cp -a --reflink=auto "$M021D_4" "$OUT/maps-021d-5"

$P - "$OUT" "$V011E_3" "$M021D_4" "$W" <<'PY'
import hashlib, json, sys
from pathlib import Path
out, v3, d4, work = map(Path, sys.argv[1:])
def load(p): return json.loads((p / 'pack.json').read_text())
def save(p, data): (p / 'pack.json').write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
sys.path.insert(0, str(Path(sys.argv[0]).parent))
# maps-v011e-4: version, a note; the lobby movie's replace precondition becomes the newest released bytes.
p = load(out / 'maps-v011e-4'); assert p['version'] == '3'
base = hashlib.sha256((work / 'base-lobby_menu.swf').read_bytes()).hexdigest()
lobby = [o for o in p['operations'] if o['op'] == 'replace_file' and o['path'] == 'flash_movies/lobby_menu.swf']
assert len(lobby) == 1
lobby[0]['base_sha256'] = base
p['version'] = '4'
p['notes'].append('maps-v011e-4: flash_movies/lobby_menu.swf rebuilt over the newest released movie (resources.db.patch_19, '
                  f'sha256 {base}) with the same edits plus the Matches window\'s levels 9-13 (BottomPanel.matchesNameTable(), '
                  'v011e_ui_swf.py MATCHES_LEVELS); nothing else differs from maps-v011e-3')
save(out / 'maps-v011e-4', p)
# maps-021d-5: one string.
p = load(out / 'maps-021d-5'); assert p['version'] == '4'
eng = [o for o in p['operations'] if o['op'] == 'localization_add' and o['languages'] == ['english']]
assert len(eng) == 1 and eng[0]['strings']['st_level_10_name'] == 'RLS Vostok (v0.21)'
eng[0]['strings']['st_level_10_name'] = 'Vostok Radar Station (v0.21)'
p['version'] = '5'
p['notes'].append('maps-021d-5: English st_level_10_name "Vostok Radar Station (v0.21)" (the native 0.26 name of level_03 '
                  'with the source version, the owner\'s rule for revision 19); nothing else differs from maps-021d-4')
save(out / 'maps-021d-5', p)

# Proof: the new packs against the old ones, whole manifests and every file.
def files(p): return {str(f.relative_to(p)): hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted((p / 'files').rglob('*')) if f.is_file()}
def diff(a, b, path=''):
    if type(a) != type(b): return [f'{path}: {a!r} -> {b!r}']
    if isinstance(a, dict):
        return [d for k in sorted(set(a) | set(b)) for d in (diff(a.get(k), b.get(k), f'{path}.{k}') if k in a and k in b else [f'{path}.{k}: {"added" if k in b else "removed"}'])]
    if isinstance(a, list):
        if len(a) != len(b): return [f'{path}: {len(a)} -> {len(b)} entries'] + [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{path}[{i}]')]
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{path}[{i}]')]
    return [] if a == b else [f'{path}: {a!r} -> {b!r}']
for old, new in ((v3, out / 'maps-v011e-4'), (d4, out / 'maps-021d-5')):
    fa, fb = files(old), files(new)
    changed = sorted(k for k in set(fa) | set(fb) if fa.get(k) != fb.get(k))
    report = dict(old=str(old), new=str(new), manifest=[d if len(d) < 300 else d[:300] + '…' for d in diff(load(old), load(new))], files_changed=changed, files=len(fb))
    (out / f'{new.name}.diff.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(json.dumps(report, indent=1, ensure_ascii=False))
PY
BASE=(); for n in "${PATCHES[@]}"; do BASE+=(--base-patch $BP/resources.db.patch_$n); done
$PACK_TOOL pack check --pack "$OUT/maps-021d-5" --pack "$OUT/maps-v011e-4" --base-db $STOCK "${BASE[@]}" | tail -3
# What the two packs put over the released patches' own files (the release worker's override list).
for n in "${PATCHES[@]}"; do $PACK_TOOL list --db $BP/resources.db.patch_$n | sed "s/\$/\tpatch_$n/"; done | sort > $W/released.txt
$P - "$OUT" $W/released.txt <<'PY'
import json, sys
from pathlib import Path
out, released = Path(sys.argv[1]), sys.argv[2]
have = {}
for line in open(released):
    path, patch = line.rstrip('\n').split('\t'); have.setdefault(path, []).append(patch)
rows = [f"{o['path']}\t{' '.join(have[o['path']])}\t{name}"
        for name in ('maps-021d-5', 'maps-v011e-4') for o in json.loads((out / name / 'pack.json').read_text())['operations']
        if 'path' in o and o['path'] in have]
(out / 'overrides-of-released-patches.txt').write_text(''.join(r + '\n' for r in rows)); print('overrides of released patches:', rows)
PY
(cd "$OUT" && for p in maps-v011e-4 maps-021d-5; do find $p -type f -print0 | sort -z | xargs -0 sha256sum > $p.files.sha256; done \
  && sha256sum maps-v011e-4/pack.json maps-v011e-4.files.sha256 maps-021d-5/pack.json maps-021d-5.files.sha256 > SHA256SUMS)
cat "$OUT/SHA256SUMS"
