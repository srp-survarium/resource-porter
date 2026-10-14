#!/usr/bin/env python3
"""Compare extracted collision resources and shortlist geometry near a camera path.

Bounds use the recovered transforms and the verified 0.26 cylinder dimension
construction. They are candidates, not proof of a runtime contact.
"""
import argparse
from collections import Counter
import itertools
import json
import math
from pathlib import Path
import re
import struct


def config(b, off=0):
    data, _, _, kind, count = struct.unpack_from('<QQIHH', b, off)
    if kind == 0: return bool(data)
    if kind == 1: return struct.unpack_from('<i', b, off)[0]
    if kind == 2: return struct.unpack_from('<f', b, off)[0]
    if kind == 5: return b[data:data+count-1].decode('cp1251')
    if kind in (6, 7, 8): return list(struct.unpack_from('<'+'f'*(kind-4), b, data))
    if kind == 4: return [config(b, data+i*24) for i in range(count)]
    if kind != 3: raise ValueError(f'unknown config kind {kind}')
    result = {}
    for i in range(count):
        child = data+i*24
        p = struct.unpack_from('<Q', b, child+8)[0]
        result[b[p:b.index(b'\0', p)].decode('cp1251')] = config(b, child)
    return result


def rotation(a):
    sx, sy, sz = map(math.sin, a)
    cx, cy, cz = map(math.cos, a)
    return [[cy*cz, -cy*sz, sy],
            [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
            [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]]


def transform(p, r, t, scale=(1, 1, 1)):
    return [sum(p[i]*scale[i]*r[i][j] for i in range(3))+t[j] for j in range(3)]


def bounds(points):
    points = list(points)
    return [[min(p[i] for p in points) for i in range(3)],
            [max(p[i] for p in points) for i in range(3)]]


def intersects(a, b):
    return all(a[0][i] <= b[1][i] and a[1][i] >= b[0][i] for i in range(3))


def chunks(b):
    out = {}; offset = 0
    while offset < len(b):
        kind, size = struct.unpack_from('<II', b, offset); offset += 8
        if offset+size > len(b): raise ValueError('truncated geometry chunk')
        out[kind] = b[offset:offset+size]; offset += size
    return out


def collision_parts(root, obj):
    path, *suffix = obj['lib_name'].removeprefix('resources/').split('#')
    scale = [float(x) for x in re.findall(r'\[([^]]+)\]', suffix[0])] if suffix else [1, 1, 1]
    if len(scale) != 3: raise ValueError('invalid model scale suffix')
    r = rotation(obj['rotation']); t = obj['position']
    world = lambda p: transform(p, r, t, scale)
    directory = root/path
    primitives = directory/'exported_primitives'
    if primitives.exists():
        for i, p in enumerate(config(primitives.read_bytes())['primitives']):
            kind = p['type']; dim = p['scale']; pr = rotation(p['rotation'])
            if kind == 0: half = [dim[0]]*3
            elif kind == 1: half = dim
            elif kind == 2: half = [dim[0], dim[1], dim[0]]
            elif kind == 3: half = [dim[0], dim[0]+dim[1]/2, dim[0]]
            else: raise ValueError(f'unsupported primitive {kind}')
            corners = (world(transform(v, pr, p['position']))
                       for v in itertools.product(*[(-h, h) for h in half]))
            part = dict(kind='primitive', index=i, primitive_type=kind, bounds=bounds(corners))
            if kind == 2:
                old_corners = (world(transform(v, pr, p['position']))
                               for v in itertools.product(*[(-h, h) for h in dim]))
                part['legacy_011a_bounds'] = bounds(old_corners)
            yield part
    vertices = directory/'vertices'; indices = directory/'indices'
    if vertices.exists() and indices.exists():
        vb = chunks(vertices.read_bytes())[25]; ib = chunks(indices.read_bytes())[26]
        vs = [world(p) for p in struct.iter_unpack('<fff', vb)]
        for i, triangle in enumerate(struct.iter_unpack('<III', ib)):
            points = [vs[j] for j in triangle]
            yield dict(kind='triangle', index=i, bounds=bounds(points), vertices=points)


def audit(source, target, region):
    old = config((source/'projects/level_04/client_project').read_bytes())
    new = config((target/'projects/v011a_level_04/day_2/client_project').read_bytes())
    norm = lambda o: json.dumps(o, sort_keys=True).replace('ported/v011a/', '')
    a, b = [Counter(norm(o) for o in p['collision_objects']) for p in (old, new)]
    matched = 0; changed = []; candidates = []
    for p in target.rglob('*'):
        if not p.is_file() or not ('/collision/' in str(p) or '/walker_collision/' in str(p)): continue
        rel = str(p.relative_to(target)); original = source/rel.replace('ported/v011a/', '')
        if not original.exists(): changed.append(dict(path=rel, reason='no original payload')); continue
        x, y = original.read_bytes(), p.read_bytes()
        same = config(x) == config(y) if p.name == 'exported_primitives' else x == y
        if same: matched += 1
        else: changed.append(dict(path=rel, reason='different payload'))
    render = {(o['lib_name'], tuple(o['position']), tuple(o['rotation']), tuple(o['scale'])) for o in new['render_visuals']}
    for i, obj in enumerate(new['collision_objects']):
        parts = [p for p in collision_parts(target, obj) if intersects(p['bounds'], region)]
        if not parts: continue
        model = obj['lib_name'].removeprefix('resources/models/').split('.model/')[0]
        candidates.append(dict(index=i, object=obj, source_controller_filter_matches=bool(obj['cgroup'] & 2 and obj['cmask'] & 4),
                               exact_render_transform=(model, tuple(obj['position']), tuple(obj['rotation']), tuple(obj['scale'])) in render,
                               nearby_parts=parts))
    return dict(scope='Recovered transforms and 0.26 cylinder dimensions; not native runtime contacts. Sphere/cylinder bounds are conservative boxes. Cylinder legacy_011a_bounds uses the old saved third dimension. Region includes camera path plus uncertain feet height.',
                region=region, source_instances=sum(a.values()), target_instances=sum(b.values()),
                missing=[json.loads(k) for k in (a-b).elements()], added_or_changed=[json.loads(k) for k in (b-a).elements()],
                matching_payloads=matched, different_payloads=changed, candidates=candidates)


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('source', type=Path); ap.add_argument('target', type=Path)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    result = audit(a.source, a.target, [[-69, 5, 26], [-61, 10, 32]])
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k != 'candidates'}, indent=2))
    for item in result['candidates']:
        print(item['index'], item['object']['lib_name'], 'parts', len(item['nearby_parts']),
              'walker', item['source_controller_filter_matches'], 'render transform', item['exact_render_transform'])
