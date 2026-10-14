#!/usr/bin/env python3
"""Compare decoded project collision identities and their archived shape payloads."""
import argparse
from collections import Counter
import hashlib
import json
import itertools
import math
import re
from pathlib import Path
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
    out = {}
    for i in range(count):
        child = data+i*24
        p = struct.unpack_from('<Q', b, child+8)[0]
        out[b[p:b.index(b'\0', p)].decode('cp1251')] = config(b, child)
    return out


def entries(path):
    out = {}
    for line in path.read_text().splitlines():
        row = line.split(maxsplit=4)
        if len(row) != 5 or row[0] != 'file': continue
        if row[1] != row[2]: raise ValueError('compressed resource unsupported')
        out[row[4]] = int(row[3]), int(row[1])
    return out


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def resource(obj):
    return obj['lib_name'].removeprefix('resources/').split('#')[0]


def primitive_bounds(primitive, obj):
    def rotation(a):
        sx, sy, sz = map(math.sin, a); cx, cy, cz = map(math.cos, a)
        return [[cy*cz, -cy*sz, sy], [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
                [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]]
    def transform(p, r, t, scale=(1, 1, 1)):
        return [sum(p[i]*scale[i]*r[i][j] for i in range(3))+t[j] for j in range(3)]
    kind, d = primitive['type'], primitive['scale']
    if kind == 0: half = [d[0]]*3
    elif kind == 1: half = d
    elif kind == 2: half = [d[0], d[1], d[0]]
    elif kind == 3: half = [d[0], d[0]+d[1]/2, d[0]]
    else: raise ValueError(f'unknown primitive type {kind}')
    suffix = obj['lib_name'].split('#', 1)
    scale = [float(x) for x in re.findall(r'\[([^]]+)\]', suffix[1])] if len(suffix) == 2 else [1, 1, 1]
    if len(scale) != 3: raise ValueError('invalid resource scale suffix')
    points = [transform(transform(v, rotation(primitive['rotation']), primitive['position']),
                        rotation(obj['rotation']), obj['position'], scale)
              for v in itertools.product(*[(-h, h) for h in half])]
    return [[min(p[i] for p in points) for i in range(3)],
            [max(p[i] for p in points) for i in range(3)]]


def regional_changes(report, old_project, new_project, region):
    projects = [json.loads(p.read_text()) for p in (old_project, new_project)]
    result = []
    for payload in report['payloads']:
        for version, key in enumerate(('removed_primitives', 'added_primitives')):
            for primitive in payload.get(key, []):
                for index, obj in enumerate(projects[version]['collision_objects']):
                    if resource(obj) != payload['path'].rsplit('/', 1)[0]: continue
                    bound = primitive_bounds(primitive, obj)
                    if all(bound[0][i] <= region[1][i] and bound[1][i] >= region[0][i] for i in range(3)):
                        result.append(dict(resource=payload['path'], change=key, project_index=index,
                            primitive=primitive, bounds=bound, movement_filter=bool(obj['cgroup'] & 2 and obj['cmask'] & 4)))
    return result


def audit(old_project, new_project, old_db, new_db, old_entries, new_entries):
    projects = [json.loads(p.read_text()) for p in (old_project, new_project)]
    objects = [p['collision_objects'] for p in projects]
    full = [Counter(map(canonical, rows)) for rows in objects]
    identities = [Counter(canonical({k: o[k] for k in ('lib_name', 'position', 'rotation', 'scale')}) for o in rows) for rows in objects]
    roots = [{resource(o) for o in rows} for rows in objects]
    indexes = [entries(p) for p in (old_entries, new_entries)]
    payloads = []
    with old_db.open('rb') as fa, new_db.open('rb') as fb:
        def read(i, path):
            off, length = indexes[i][path]; f = (fa, fb)[i]; f.seek(off)
            b = f.read(length)
            if len(b) != length: raise ValueError('short archive read')
            return b
        paths = sorted({p for i in range(2) for p in indexes[i] if p.rsplit('/', 1)[0] in roots[0] | roots[1]})
        for path in paths:
            row = dict(path=path, referenced_old=path.rsplit('/', 1)[0] in roots[0], referenced_new=path.rsplit('/', 1)[0] in roots[1])
            blobs = [read(i, path) if path in indexes[i] else None for i in range(2)]
            row['sha256'] = [hashlib.sha256(b).hexdigest() if b is not None else None for b in blobs]
            row['bytes'] = [len(b) if b is not None else None for b in blobs]
            if any(b is None for b in blobs): row['status'] = 'added' if blobs[0] is None else 'removed'
            else: row['status'] = 'identical' if blobs[0] == blobs[1] else 'changed'
            if path.endswith('/exported_primitives') and all(b is not None for b in blobs):
                decoded = [config(b) for b in blobs]
                row['decoded_equal'] = decoded[0] == decoded[1]
                if row['decoded_equal'] and row['status'] == 'changed': row['status'] = 'config-layout-only'
                row['primitive_counts'] = [len(d['primitives']) for d in decoded]
                row['cylinder_counts'] = [sum(p['type'] == 2 for p in d['primitives']) for d in decoded]
                row['zero_z_cylinders'] = [sum(p['type'] == 2 and p['scale'][2] == 0 for p in d['primitives']) for d in decoded]
                if not row['decoded_equal']:
                    ca, cb = [Counter(map(canonical, d['primitives'])) for d in decoded]
                    row['removed_primitives'] = [json.loads(k) for k in (ca-cb).elements()]
                    row['added_primitives'] = [json.loads(k) for k in (cb-ca).elements()]
            payloads.append(row)
    return dict(scope='Exact resource-plus-transform identities, all collision instance fields and referenced archived payloads. No runtime contact claim; no authored placements modified.',
        project_sha256=[hashlib.sha256(p.read_bytes()).hexdigest() for p in (old_project, new_project)],
        instance_counts=[len(x) for x in objects], exact_instances_common=(full[0]&full[1]).total(),
        resource_transform_identities_common=(identities[0]&identities[1]).total(),
        removed_instances=[json.loads(k) for k in (full[0]-full[1]).elements()],
        added_instances=[json.loads(k) for k in (full[1]-full[0]).elements()],
        referenced_shape_roots=[len(r) for r in roots], payload_status=dict(Counter(p['status'] for p in payloads)), payloads=payloads)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    for name in ('old-project', 'new-project', 'old-db', 'new-db', 'old-entries', 'new-entries', 'output'):
        ap.add_argument('--'+name, type=Path, required=True)
    ap.add_argument('--region', type=float, nargs=6, help='Native conservative bounds: min XYZ then max XYZ')
    a = ap.parse_args(); result = audit(a.old_project, a.new_project, a.old_db, a.new_db, a.old_entries, a.new_entries)
    if a.region:
        result['region'] = [a.region[:3], a.region[3:]]
        result['regional_primitive_changes'] = regional_changes(result, a.old_project, a.new_project, result['region'])
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ('payloads', 'removed_instances', 'added_instances', 'regional_primitive_changes')}, indent=2))

if __name__ == '__main__': main()
