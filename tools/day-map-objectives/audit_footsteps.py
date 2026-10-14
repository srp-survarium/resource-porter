#!/usr/bin/env python3
# Usage: audit_footsteps.py WORKDIR(with gm_026.leaves, pairs026.leaves) BUNDLE_OVERLAY PROJECT TARGET_DB CGROUP_MASK
"""Audit footstep sound coverage of every collision surface used by a converted map."""
import sys, struct, collections, json, math, os, subprocess
sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config, chunks, rotation, transform
from pathlib import Path
GROUP = int(sys.argv[5]); S = sys.argv[1]; root = Path(sys.argv[2]); project = config(Path(sys.argv[3]).read_bytes()); target_db = sys.argv[4]
sys.argv = ['', S]; exec(open(str(Path(__file__).with_name('footstep_pairs.py'))).read().split('if __name__')[0])
env = dict(os.environ, LD_LIBRARY_PATH='/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib')
def read(rel):
    p = root / rel
    if p.exists(): return p.read_bytes()
    out = Path(S) / 'tmp-native'
    native = rel.replace('models/ported/v011e/', 'models/').replace('models/ported/v020e/', 'models/')
    r = subprocess.run([f'{S}/target/release/vostok-port', 'extract', '--db', target_db, '--path', native, '--output', str(out)], env=env, capture_output=True)
    return out.read_bytes() if r.returncode == 0 else None
def names(blob):
    count = struct.unpack_from('<H', blob, 0)[0]; out, at = [], 2
    for _ in range(count):
        end = blob.index(0, at); out.append(blob[at:end].decode('cp1251')); at = end + 1
    return out
def walk(mid, seen=()):
    n, base, out = step(mid)
    if base and not any(out.values()) and base in ids and base not in seen:
        return walk(ids[base], seen + (base,))
    return n, out
area = collections.Counter(); detail = {}
for obj in project['collision_objects']:
    path = obj['lib_name'].removeprefix('resources/').split('#')[0]
    if not (obj['cgroup'] & GROUP): continue
    model = path.rsplit('/', 1)[0]
    sb = read(model + '/settings')
    gms = (config(sb).get('game_material_settings') or {}) if sb else {}
    Ro = rotation(obj['rotation']); t = obj['position']
    surfaces = []
    fd, vb, ib = read(path + '/face_data'), read(path + '/vertices'), read(path + '/indices')
    if fd and vb and ib:
        fc = chunks(fd); tbl = names(fc[27]); per = struct.unpack('<%dH' % (len(fc[28]) // 2), fc[28])
        vs = [transform(v, Ro, t) for v in struct.iter_unpack('<fff', chunks(vb)[25])]
        for k, tri in enumerate(struct.iter_unpack('<III', chunks(ib)[26])):
            a, b, c = (vs[i] for i in tri)
            u = [b[j]-a[j] for j in range(3)]; w = [c[j]-a[j] for j in range(3)]
            nrm = [u[1]*w[2]-u[2]*w[1], u[2]*w[0]-u[0]*w[2], u[0]*w[1]-u[1]*w[0]]
            ar = 0.5 * math.sqrt(sum(x*x for x in nrm))
            if ar > 0 and nrm[1] / (2*ar) > 0.5:                     # walkable-facing triangles
                surfaces.append((tbl[per[k]] if per[k] < len(tbl) else f'#{per[k]}', ar))
    pb = read(path + '/exported_primitives')
    if pb:
        pc = config(pb); ml = pc.get('mtl_list', [])
        for p in pc['primitives']:
            d = p['scale']; top = 4 * d[0] * (d[2] if p['type'] == 1 else d[0])
            surfaces.append((ml[p['mtl']] if p['mtl'] < len(ml) else f'#{p["mtl"]}', top))
    for surface, ar in surfaces:
        entry = gms.get(surface)
        if entry is None: key = ('UNMAPPED surface', model, surface)
        else:
            mid = entry['game_material_id']
            n, out = walk(mid) if mid in byid else (entry['game_material_name'], {})
            ok_remote = bool(out.get('actor')); ok_local = bool(out.get('actor_local'))
            ok_sprint = bool(out.get('actor_local_sprint')); key = ('ok' if ok_remote and ok_local and ok_sprint else 'NO LOCAL SPRINT STEP' if ok_remote and ok_local else f'NO STEP (remote={ok_remote}, local={ok_local})', model, f'{surface}->{n}({mid})')
        area[key] += ar
summary = collections.Counter()
for (status, model, s), ar in area.items(): summary[status] += ar
print(json.dumps({k: round(v, 1) for k, v in summary.items()}, indent=1))
for (status, model, s), ar in sorted(area.items(), key=lambda kv: -kv[1]):
    if status != 'ok' and ar > 1: print(f'{ar:9.1f} m2  {status:40s} {model.split("models/")[-1]}  {s}')
