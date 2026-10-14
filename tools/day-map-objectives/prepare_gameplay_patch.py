#!/usr/bin/env python3
"""Prepare a guarded, same-length gameplay correction plan; never writes an archive.

Two converter repairs can be carried to an already packed candidate:

* ``--respawn-seconds RESOURCE``: early (0.11a/0.11e) respawn protection is
  authored in milliseconds, while native 0.26g multiplies the value by 1000
  (0x2F460F..0x2F4630). 2000 becomes 2.
* ``--battery-body RESOURCE``: a spawner whose nested ``spawned_item`` still
  equals its parent mesh origin receives the native body origin
  ``parent + rotate(rotation, collision_geometries[0].position)`` from the
  archive's ``gameplay/victory_items/default.options``.

The output ``plan.json`` uses the entry format of
``tools/render-compare/patch_archive_entries.py``. An independent config decoder
proves that only the intended leaves change.
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
INTEGER, FLOAT3, NAMED, INDEXED = 1, 7, 3, 4


def leaf(data, path):
    offset = 0
    for part in re.findall(r'[^.\[\]]+', path):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind == INDEXED:
            if not 0 <= int(part) < count:
                raise ValueError('index outside config: ' + path)
            offset = table + 24 * int(part)
        elif kind == NAMED:
            found = []
            for i in range(count):
                child = table + 24 * i
                name = struct.unpack_from('<Q', data, child + 8)[0]
                if data[name:data.index(0, name)].decode('cp1251') == part:
                    found.append(child)
            if len(found) != 1:
                raise ValueError('missing or ambiguous field: ' + path)
            offset = found[0]
        else:
            raise ValueError('not a table: ' + path)
    return offset


def kind(data, path):
    return struct.unpack_from('<H', data, leaf(data, path) + 20)[0]


def count(data, path):
    at = leaf(data, path)
    if struct.unpack_from('<H', data, at + 20)[0] != INDEXED:
        raise ValueError('not an indexed table: ' + path)
    return struct.unpack_from('<H', data, at + 22)[0]


def vector(data, path):
    at = leaf(data, path)
    pointer, _, _, value_kind, _ = struct.unpack_from('<QQIHH', data, at)
    if value_kind != FLOAT3:
        raise ValueError('not a float3: ' + path)
    value = struct.unpack_from('<3f', data, pointer)
    if not all(math.isfinite(v) for v in value):
        raise ValueError('non-finite float3: ' + path)
    return pointer, value


def rotated(v, angles):
    sx, sy, sz = map(math.sin, angles)
    cx, cy, cz = map(math.cos, angles)
    r = [[cy*cz, -cy*sz, sy],
         [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
         [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]]
    return [sum(v[i]*r[i][j] for i in range(3)) for j in range(3)]


def f32(values):
    return struct.unpack('<3f', struct.pack('<3f', *values))


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_resource(porter, archive, fat, path):
    stat = subprocess.check_output([porter, 'stat', '--db', str(archive), '--path', path], env=ENV, text=True)
    if 'compressed: false, inlined: false' not in stat:
        raise ValueError('resource is compressed or inlined: ' + path)
    size = int(re.search(r'^size_in_db=(\d+)$', stat, re.M)[1])
    position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
    with archive.open('rb') as db:
        db.seek(position)
        data = db.read(size)
    needle = struct.pack('<I4xQ', size, position)
    found = fat.find(needle)
    if found < 0 or fat.find(needle, found + 1) >= 0:
        raise ValueError('archive entry is not unique: ' + path)
    return data, {'resource': path, 'position': position, 'crc_offset': 24 + found + 16}


def decoded(porter, file):
    text = subprocess.check_output([porter, 'dump-config', '--file', str(file), '--leaves'], env=ENV, text=True)
    return {k: json.loads(v.split(':', 1)[1]) for k, v in (line.split('\t', 1) for line in text.splitlines())}


def respawn_seconds(original, fixed):
    root = 'server_objects.respawn_points'
    changes = {}
    for i in range(count(original, root)):
        path = f'{root}[{i}].invulnerability_time'
        if kind(original, path) != INTEGER:
            raise ValueError('respawn invulnerability is not an integer: ' + path)
        at = leaf(original, path)
        milliseconds = struct.unpack_from('<i', original, at)[0]
        if milliseconds < 0 or milliseconds % 1000:
            raise ValueError(f'{path}={milliseconds} has no exact whole-second value (already converted?)')
        struct.pack_into('<i', fixed, at, milliseconds // 1000)
        changes[path] = (milliseconds, milliseconds // 1000)
    return changes


def battery_body(original, fixed, offset):
    root = 'server_objects.victory_item_spawners'
    changes = {}
    for i in range(count(original, root)):
        prefix = f'{root}[{i}]'
        parent_ptr, parent = vector(original, prefix + '.position')
        _, angles = vector(original, prefix + '.rotation')
        ptr, before = vector(original, prefix + '.spawned_item.position')
        _, body_angles = vector(original, prefix + '.spawned_item.rotation')
        if ptr == parent_ptr:
            raise ValueError('aliased parent/body storage needs repacking: ' + prefix)
        if body_angles != angles:
            raise ValueError('body rotation differs from parent: ' + prefix)
        after = f32([parent[j] + d for j, d in enumerate(rotated(offset, angles))])
        if before != parent:
            if max(abs(a - b) for a, b in zip(before, after)) > 1e-5:
                raise ValueError('unexpected existing body offset: ' + prefix)
            continue
        struct.pack_into('<3f', fixed, ptr, *after)
        changes[prefix + '.spawned_item.position'] = (list(before), list(after))
    return changes


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('archive', type=Path)
    ap.add_argument('output', type=Path)
    ap.add_argument('--porter', default=PORTER)
    ap.add_argument('--respawn-seconds', action='append', default=[], metavar='RESOURCE')
    ap.add_argument('--battery-body', action='append', default=[], metavar='RESOURCE')
    ap.add_argument('--archive-sha256', help='Expected archive SHA-256, checked before reading')
    a = ap.parse_args()
    if a.output.exists():
        raise SystemExit('choose a fresh output directory')
    if a.archive_sha256:
        h = hashlib.sha256()
        with a.archive.open('rb') as db:
            while block := db.read(16 << 20):
                h.update(block)
        if h.hexdigest() != a.archive_sha256:
            raise SystemExit('archive hash mismatch')
    with a.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    a.output.mkdir(parents=True)
    options, _ = read_resource(a.porter, a.archive, fat, 'gameplay/victory_items/default.options')
    offset = vector(options, 'collision_geometries[0].position')[1]
    entries, report = [], []
    for path in dict.fromkeys(a.respawn_seconds + a.battery_body):
        original, entry = read_resource(a.porter, a.archive, fat, path)
        fixed = bytearray(original)
        changes = {}
        if path in a.respawn_seconds:
            changes.update(respawn_seconds(original, fixed))
        if path in a.battery_body:
            changes.update(battery_body(original, fixed, offset))
        if not changes:
            raise SystemExit('no change needed: ' + path)
        stem = path.replace('/', '_')
        baseline_file, fixed_file = a.output / (stem + '.baseline'), a.output / (stem + '.fixed')
        baseline_file.write_bytes(original)
        fixed_file.write_bytes(bytes(fixed))
        old, new = decoded(a.porter, baseline_file), decoded(a.porter, fixed_file)
        if old.keys() != new.keys():
            raise SystemExit('decoded structure changed: ' + path)
        changed = {k for k in old if old[k] != new[k]}
        if changed != set(changes):
            raise SystemExit(f'unexpected decoded changes in {path}: {sorted(changed ^ set(changes))[:5]}')
        for key, (_, after) in changes.items():
            # The decoder prints the shortest decimal of each f32.
            actual = new[key] if isinstance(after, int) else f32(new[key])
            if actual != (after if isinstance(after, int) else f32(after)):
                raise SystemExit(f'decoder disagrees for {key}: {new[key]} != {after}')
        entry.update(baseline=baseline_file.name, fixed=fixed_file.name,
                     baseline_sha256=digest(original), fixed_sha256=digest(bytes(fixed)))
        entries.append(entry)
        report.append({'resource': path, 'changed_leaves': len(changes),
                       'changes': {k: {'before': b, 'after': f} for k, (b, f) in sorted(changes.items())}})
    plan = {'archive': str(a.archive), 'archive_sha256': a.archive_sha256, 'release_correction': True,
            'battery_local_body_offset': offset, 'entries': entries, 'resources': report,
            'validation': 'Independent config decoder: identical keys; only the listed leaves changed.'}
    (a.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(json.dumps({'plan': str(a.output / 'plan.json'),
                      'resources': {r['resource']: r['changed_leaves'] for r in report}}, indent=2))


if __name__ == '__main__':
    main()
