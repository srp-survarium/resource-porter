#!/usr/bin/env python3
"""Prepare a same-length body-origin correction from a saved gallery baseline.

Never writes an archive. The input plan supplies archive offsets and baseline
hashes; only nested spawned_item.position floats may change in the output.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess

PORTER = '/home/sheep/Projects/survarium/resource-porter/target/release/vostok-port'
ENV = dict(os.environ, LD_LIBRARY_PATH='/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib')

def leaf(data, path):
    offset = 0
    for part in re.findall(r'[^.\[\]]+', path):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind == 4:
            assert 0 <= int(part) < count
            offset = table + 24 * int(part)
        else:
            assert kind == 3
            found = []
            for i in range(count):
                child = table + 24*i
                name = struct.unpack_from('<Q', data, child+8)[0]
                if data[name:data.index(0, name)].decode() == part:
                    found.append(child)
            assert len(found) == 1, path
            offset = found[0]
    return offset

def vector(data, path):
    at = leaf(data, path)
    ptr, _, _, kind, _ = struct.unpack_from('<QQIHH', data, at)
    assert kind == 7, path
    result = struct.unpack_from('<3f', data, ptr)
    assert all(math.isfinite(v) for v in result)
    return ptr, result

def rotated(v, angles):
    sx, sy, sz = map(math.sin, angles)
    cx, cy, cz = map(math.cos, angles)
    r = [[cy*cz, -cy*sz, sy],
         [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
         [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]]
    return [sum(v[i]*r[i][j] for i in range(3)) for j in range(3)]

def decoded(file):
    text = subprocess.check_output([PORTER, 'dump-config', '--file', str(file), '--leaves'], env=ENV, text=True)
    return {k: json.loads(v.split(':', 1)[1]) for k, v in (line.split('\t', 1) for line in text.splitlines())}

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('baseline_plan', type=Path)
    ap.add_argument('battery_options', type=Path, help='Extracted native gameplay/victory_items/default.options')
    ap.add_argument('output', type=Path)
    a = ap.parse_args()
    assert not a.output.exists(), 'Choose a fresh output directory'
    plan = json.loads(a.baseline_plan.read_text())
    entry = next(e for e in plan['entries'] if e['resource'].endswith('/client_project'))
    original = (a.baseline_plan.parent / entry['baseline']).read_bytes()
    digest = lambda b: hashlib.sha256(b).hexdigest()
    assert digest(original) == entry['baseline_sha256']
    offset = vector(a.battery_options.read_bytes(), 'collision_geometries[0].position')[1]
    fixed = bytearray(original)
    root = 'server_objects.victory_item_spawners'
    count = struct.unpack_from('<H', original, leaf(original, root)+22)[0]
    changes = []
    for i in range(count):
        prefix = f'{root}[{i}]'
        parent_ptr, parent = vector(original, prefix+'.position')
        _, angles = vector(original, prefix+'.rotation')
        ptr, before = vector(original, prefix+'.spawned_item.position')
        assert ptr != parent_ptr, 'Aliased parent/nested storage needs archive repacking'
        _, body_angles = vector(original, prefix+'.spawned_item.rotation')
        assert body_angles == angles
        delta = rotated(offset, angles)
        after = struct.unpack('<3f', struct.pack('<3f', *[parent[j]+delta[j] for j in range(3)]))
        assert before == parent or all(abs(before[j]-after[j]) < 1e-6 for j in range(3)), 'Unexpected existing body offset'
        if before != after:
            struct.pack_into('<3f', fixed, ptr, *after)
            changes.append({'index': i, 'parent_position': parent, 'body_before': before, 'body_after': after,
                            'rotation': angles, 'byte_offset': ptr})
    a.output.mkdir(parents=True)
    baseline = a.output / 'baseline.project'; baseline.write_bytes(original)
    target = a.output / 'body-origins.project'; target.write_bytes(fixed)
    old, new = decoded(baseline), decoded(target)
    changed = {k for k in old if old[k] != new[k]}
    expected = {f'{root}[{r["index"]}].spawned_item.position' for r in changes}
    assert old.keys() == new.keys() and changed == expected, (changed, expected)
    result = {'archive': plan['archive'], 'map': plan['map'], 'runtime_validated': False,
              'entries': [dict(entry, baseline=baseline.name, fixed=target.name, fixed_sha256=digest(fixed))],
              'local_body_offset': offset, 'changed_nested_positions': changes,
              'validation': 'Independent config decoder verifies only nested body positions changed; parents, rotations and all other fields preserved.'}
    (a.output / 'plan.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'plan': str(a.output/'plan.json'), 'spawns': count, 'changed': len(changes)}))

if __name__ == '__main__':
    main()
