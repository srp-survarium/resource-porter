#!/usr/bin/env python3
"""Footstep material of every top surface the 0.26 step query can hit.

Usage: audit_step_surfaces.py WORKDIR(gm_026.leaves, pairs026.leaves) BUNDLE_OVERLAY PROJECT X0 X1 Z0 Z1 STEP

0.26 survarium step handler (0x248ad0, same as 0.11e step_manager::on_step)
casts 2 m down from 1 m above the feet against the non-walker collision
(cgroup & 8), takes the hit surface's game material and plays the
collision_sound of get_pair(actor variant, material) (0x306d50: base_material
substituted, a missing pair falls back to the default material). Per grid
cell this takes the topmost such surface in two layers (y < 5 ground,
5..14 elevated) and reports area per model surface with its step status:
ok, "no pair -> default steps", SILENT <empty actor variants>, UNASSIGNED
(65535) or UNMAPPED (surface absent from the model settings).
Exact triangles and oriented boxes; spheres/cylinders use their boxes.
"""
import sys, struct, collections, json
HERE = __import__('pathlib').Path(__file__).resolve().parent
sys.path.insert(0, str(HERE)); sys.path.insert(0, str(HERE.parent / 'chemical-plant'))
from audit_collision_region import chunks, collision_parts, config, rotation, transform
import itertools, re
from pathlib import Path
WORK = sys.argv[1]; root = Path(sys.argv[2]); proj = config(Path(sys.argv[3]).read_bytes())
x0, x1, z0, z1, st = map(float, sys.argv[4:9])
parts = []                                   # the step query's collision: cgroup & 8
for o in proj['collision_objects']:
    if not o['cgroup'] & 8: continue
    try: parts.extend((o['lib_name'].split('/models/')[-1], o, q) for q in collision_parts(root, o))
    except FileNotFoundError: pass
import math
CELL = 2.0
index = collections.defaultdict(list)
for item in parts:
    (lx, ly, lz), (hx, hy, hz) = item[2]['bounds']
    if hx < x0 - 1 or lx > x1 + 1 or hz < z0 - 1 or lz > z1 + 1: continue
    for i in range(int(math.floor(lx / CELL)), int(math.floor(hx / CELL)) + 1):
        for j in range(int(math.floor(lz / CELL)), int(math.floor(hz / CELL)) + 1):
            index[(i, j)].append(item)
def near(lo, hi):
    return [it for it in index.get((int(math.floor(lo[0] / CELL)), int(math.floor(lo[2] / CELL))), [])
            if it[2]['bounds'][0][0] <= hi[0] and it[2]['bounds'][1][0] >= lo[0] and it[2]['bounds'][0][2] <= hi[2] and it[2]['bounds'][1][2] >= lo[2]]
cache = {}
def mat(obj, part):
    path = obj['lib_name'].removeprefix('resources/').split('#')[0]; model = path.rsplit('/', 1)[0]
    if path not in cache:
        sb = root / model / 'settings'; gms = (config(sb.read_bytes()).get('game_material_settings') or {}) if sb.exists() else {}
        names, per = [], []
        fd = root / path / 'face_data'
        if fd.exists():
            fc = chunks(fd.read_bytes()); b = fc[27]; n = struct.unpack_from('<H', b)[0]; at = 2
            for _ in range(n): e = b.index(0, at); names.append(b[at:e].decode('cp1251')); at = e + 1
            per = struct.unpack('<%dH' % (len(fc[28]) // 2), fc[28])
        pp = root / path / 'exported_primitives'
        prims = config(pp.read_bytes()) if pp.exists() else {}
        cache[path] = (gms, names, per, prims.get('mtl_list', []), prims.get('primitives', []))
    gms, names, per, ml, prims = cache[path]
    if part['kind'] == 'triangle': s = names[per[part['index']]] if per else '?'
    else: q = prims[part['index']]; s = ml[q['mtl']] if q['mtl'] < len(ml) else '?'
    g = gms.get(s)
    return f"{s}->{g['game_material_name'] or 'unassigned'}({g['game_material_id']})" if g else f"{s}->UNMAPPED"
prim_cache = {}
def box(o, p):
    key = (id(o), p['index'])
    if key not in prim_cache:
        path, *suffix = o['lib_name'].removeprefix('resources/').split('#')
        scale = [float(v) for v in re.findall(r'\[([^]]+)\]', suffix[0])] if suffix else [1, 1, 1]
        q = config((root/path/'exported_primitives').read_bytes())['primitives'][p['index']]
        kind = q['type']; dim = q['scale']
        half = [dim[0]]*3 if kind == 0 else dim if kind == 1 else [dim[0], dim[1], dim[0]] if kind == 2 else [dim[0], dim[0]+dim[1]/2, dim[0]]
        r = rotation(o['rotation']); pr = rotation(q['rotation'])
        cs = [transform(transform(v, pr, q['position']), r, o['position'], scale) for v in itertools.product(*[(-h, h) for h in half])]
        c = [sum(v[i] for v in cs)/8 for i in range(3)]
        axes = [[(cs[a][i]-cs[0][i])/2 for i in range(3)] for a in (4, 2, 1)]
        prim_cache[key] = (c, axes)
    return prim_cache[key]
def box_top(o, p, x, z):
    c, axes = box(o, p); lo, hi = -1e9, 1e9
    for ax in axes:
        n2 = sum(v*v for v in ax)
        if n2 < 1e-12: return None
        a = ((x-c[0])*ax[0] + (-c[1])*ax[1] + (z-c[2])*ax[2]) / n2; b = ax[1] / n2
        if abs(b) < 1e-12:
            if abs(a) > 1: return None
            continue
        t1, t2 = (-1-a)/b, (1-a)/b
        lo, hi = max(lo, min(t1, t2)), min(hi, max(t1, t2))
    return hi if lo <= hi else None
area = collections.Counter()
x = x0
while x < x1:
    z = z0
    while z < z1:
        best = {}
        for n, o, p in near([x-.01, -20, z-.01], [x+.01, 14, z+.01]):
            if p['kind'] == 'triangle':
                (xa, ya, za), (xb, yb, zb), (xc, yc, zc) = p['vertices']
                d = (zb-zc)*(xa-xc)+(xc-xb)*(za-zc)
                if abs(d) < 1e-9: continue
                a = ((zb-zc)*(x-xc)+(xc-xb)*(z-zc))/d; b = ((zc-za)*(x-xc)+(xa-xc)*(z-zc))/d; c = 1-a-b
                if min(a, b, c) < -1e-6: continue
                h = a*ya+b*yb+c*yc
            else:
                h = box_top(o, p, x, z)
                if h is None: continue
            layer = 'ground' if h < 5 else 'elevated' if h < 14 else None
            if layer and (layer not in best or h > best[layer][0]): best[layer] = (h, n, o, p)
        for layer, (h, n, o, p) in best.items():
            area[(layer, n.split('/models/')[-1].replace('ported/v011e/', '').replace('ported/v020e/', '').rsplit('.model', 1)[0], mat(o, p))] += st*st
        z += st
    x += st
argv = sys.argv; sys.argv = ['', WORK]
exec(open(HERE / 'footstep_pairs.py').read().split('if __name__')[0]); sys.argv = argv
def status(m):
    g = re.search(r'\((\d+)\)$', m)
    if not g: return 'UNMAPPED'
    mid = int(g.group(1))
    if mid not in byid: return 'UNASSIGNED(65535)'
    base = byid[mid].get('physic.base_material'); eff = ids.get(base, mid) if base else mid
    out = {name[a]: pairs.get((a, eff)) for a in actors}
    if all(v is None for v in out.values()): return 'no pair -> default steps'
    miss = [k for k in ('actor', 'actor_local', 'actor_sprint', 'actor_local_sprint') if not out.get(k)]
    return 'ok' if not miss else 'SILENT ' + ','.join(miss)
total = collections.Counter()
for (layer, model, m), a in area.items(): total[status(m).split(' ')[0]] += a
print(json.dumps({k: round(v) for k, v in sorted(total.items())}))
for (layer, model, m), a in sorted(area.items(), key=lambda kv: (kv[0][0], -kv[1])):
    if a >= 1: print(f'{layer:8s} {a:8.1f} m2  {model:45s} {m:55s} {status(m)}')
