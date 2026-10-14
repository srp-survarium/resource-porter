#!/usr/bin/env python3
# Usage: audit_cylinders.py BUNDLE_OVERLAY PROJECT
"""Classify early movement cylinders by 0.11e flat-AABB behaviour and by protrusion past the model's bullet collision."""
import sys, json, math, itertools, collections, struct
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config, rotation, transform, chunks
from pathlib import Path
root = Path(sys.argv[1]); project = config(Path(sys.argv[2]).read_bytes())
cache = {}
def prims(path):
    f = root/path/'exported_primitives'
    if path not in cache: cache[path] = config(f.read_bytes())['primitives'] if f.exists() else None
    return cache[path]
def local_bounds(path):
    """Model-local bounds of a collision directory (primitives as boxes plus mesh)."""
    pts = []
    for p in prims(path) or []:
        d = p['scale']; t = p['type']
        half = [d[0]]*3 if t == 0 else d if t == 1 else [d[0], d[1], d[0]] if t == 2 else [d[0], d[0]+d[1]/2, d[0]]
        pts += [transform(v, rotation(p['rotation']), p['position']) for v in itertools.product(*[(-h, h) for h in half])]
    v = root/path/'vertices'
    if v.exists(): pts += list(struct.iter_unpack('<fff', chunks(v.read_bytes())[25]))
    if not pts: return None
    return [[min(q[i] for q in pts) for i in range(3)], [max(q[i] for q in pts) for i in range(3)]]
stats = collections.Counter(); flagged = collections.Counter(); seen = set()
for obj in project['collision_objects']:
    if not (obj['cgroup'] & 2 and obj['cmask'] & 4): continue
    path = obj['lib_name'].removeprefix('resources/').split('#')[0]
    ps = prims(path)
    if not ps: continue
    Ro = rotation(obj['rotation'])
    model = path.rsplit('/', 1)[0]
    bullet = local_bounds(model + '/collision') if path.endswith('walker_collision') else None
    for i, p in enumerate(ps):
        if p['type'] != 2: continue
        zw = transform(transform([0, 0, 1], rotation(p['rotation']), [0, 0, 0]), Ro, [0, 0, 0])
        band = 'flat-axis-vertical(0.11e also blocks)' if abs(zw[1]) > 0.7 else 'flat-axis-horizontal(0.11e porous band)'
        stats[band] += 1
        if bullet:
            d = p['scale']
            corners = [transform(v, rotation(p['rotation']), p['position']) for v in itertools.product((-d[0], d[0]), (-d[1], d[1]), (-d[0], d[0]))]
            # protrusion of the solid cylinder's box beyond the model's bullet collision bounds
            out = max(max(bullet[0][k] - c[k], c[k] - bullet[1][k], 0) for c in corners for k in range(3))
            if out > 0.3 and (model, i) not in seen:
                seen.add((model, i)); flagged[(model.split('/models/')[-1], band[:24])] += 1
print(json.dumps(dict(stats), indent=0))
print('walker cylinders whose box sticks out > 0.3 m past the model bullet collision (model, kind): count')
for k, n in flagged.most_common(): print(n, k)
