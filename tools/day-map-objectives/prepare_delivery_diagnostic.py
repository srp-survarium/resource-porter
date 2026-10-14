#!/usr/bin/env python3
"""Prepare a reversible, same-length collector-delivery diagnostic; never writes an archive.

DIAGNOSTIC ONLY. For each team it moves every player respawn point to one
floor position facing that team's collector, narrows the battery spawners to
the listed authored records (dense point_id, native body origin kept at
parent + rotate(rotation, [0,0.12,0])) placed at the given floor positions,
and sets the mode's victory_items_count to their number. Collectors, their
zones and meshes, and all other project leaves are unchanged; an independent
decoder verifies that. Apply/revert with tools/render-compare/patch_archive_entries.py.

    prepare_delivery_diagnostic.py ARCHIVE SPEC.json OUTPUT
SPEC: {"project": RES, "mode": RES,
       "teams": {"0": {"position": [x,y,z], "face": [x,z]}, "1": {...}},
       "batteries": [{"source_index": i, "position": [x,y,z]}, ...]}
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_gameplay_patch import (ENV, PORTER, count, f32, kind, leaf, read_resource,  # noqa: E402
                                    rotated, vector)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def decoded(porter, file):
    text = subprocess.check_output([porter, 'dump-config', '--file', str(file), '--leaves'], env=ENV, text=True)
    return {k: json.loads(v.split(':', 1)[1]) for k, v in (line.split('\t', 1) for line in text.splitlines())}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('archive', type=Path)
    ap.add_argument('spec', type=Path)
    ap.add_argument('output', type=Path)
    ap.add_argument('--porter', default=PORTER)
    a = ap.parse_args()
    if a.output.exists():
        raise SystemExit('choose a fresh output directory')
    spec = json.loads(a.spec.read_text())
    with a.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    project, project_entry = read_resource(a.porter, a.archive, fat, spec['project'])
    mode, mode_entry = read_resource(a.porter, a.archive, fat, spec['mode'])
    fixed = bytearray(project)
    expected = {}
    written = {}

    def put3(path, value):
        pointer, _ = vector(project, path)
        value = f32(value)
        if written.get(pointer, value) != value:
            raise SystemExit('shared float storage with conflicting values: ' + path)
        written[pointer] = value
        struct.pack_into('<3f', fixed, pointer, *value)
        expected[path] = list(value)

    # Player spawns: every point of a team moves to its diagnostic position.
    root = 'server_objects.respawn_points'
    for i in range(count(project, root)):
        team = struct.unpack_from('<i', project, leaf(project, f'{root}[{i}].team'))[0]
        target = spec['teams'][str(team)]
        p, face = target['position'], target['face']
        yaw = math.atan2(-(face[0] - p[0]), face[1] - p[2])
        put3(f'{root}[{i}].position', p)
        put3(f'{root}[{i}].rotation', [0.0, yaw, 0.0])
    # Batteries: copy the chosen authored records first, then relocate them.
    root = 'server_objects.victory_item_spawners'
    table, _, _, table_kind, total = struct.unpack_from('<QQIHH', project, leaf(project, root))
    batteries = spec['batteries']
    if table_kind != 4 or not 1 <= len(batteries) <= min(10, total):
        raise SystemExit('invalid battery selection')
    records = [project[table + 24*b['source_index']:table + 24*(b['source_index']+1)] for b in batteries]
    for index, (record, battery) in enumerate(zip(records, batteries)):
        fixed[table + 24*index:table + 24*(index+1)] = record
        source = f"{root}[{battery['source_index']}]"
        point = leaf(project, source + '.point_id')
        if kind(project, source + '.point_id') != 1:
            raise SystemExit('point_id is not an integer')
        struct.pack_into('<i', fixed, point, index)
        _, angles = vector(project, source + '.rotation')
        body = [battery['position'][j] + d for j, d in enumerate(rotated([0.0, 0.12, 0.0], angles))]
        put3(source + '.position', battery['position'])
        put3(source + '.spawned_item.position', body)
    struct.pack_into('<H', fixed, leaf(project, root) + 22, len(batteries))
    changed_mode = bytearray(mode)
    count_leaf = leaf(mode, 'mode.parameters.victory_items_count')
    struct.pack_into('<i', changed_mode, count_leaf, len(batteries))

    a.output.mkdir(parents=True)
    entries = []
    for name, (original, data, entry) in {'project': (project, bytes(fixed), project_entry),
                                         'mode': (mode, bytes(changed_mode), mode_entry)}.items():
        (a.output / f'{name}.baseline').write_bytes(original)
        (a.output / f'{name}.diagnostic').write_bytes(data)
        entries.append(dict(entry, baseline=f'{name}.baseline', fixed=f'{name}.diagnostic',
                            baseline_sha256=digest(original), fixed_sha256=digest(data)))
    old = decoded(a.porter, a.output / 'project.baseline')
    new = decoded(a.porter, a.output / 'project.diagnostic')
    prefix = root + '['
    # Expected spawner leaves after narrowing: copies of the chosen sources.
    want = {k: v for k, v in old.items() if not k.startswith(prefix)}
    for path, value in expected.items():
        if not path.startswith(prefix):
            want[path] = value
    for index, battery in enumerate(batteries):
        source = f"{root}[{battery['source_index']}]"
        for key, value in old.items():
            if key.startswith(source + '.'):
                field = key[len(source):]
                want[f'{root}[{index}]{field}'] = expected.get(key, value)
        want[f'{root}[{index}].point_id'] = index
    def norm(d):
        return {k: (f32(v) if isinstance(v, list) and len(v) == 3 else v) for k, v in d.items()}
    got, intended = norm(new), norm(want)
    if got != intended:
        diff = sorted(k for k in set(got) | set(intended) if got.get(k) != intended.get(k))[:6]
        raise SystemExit(f'decoded diagnostic differs from the intended edit: {diff}')
    old_mode = decoded(a.porter, a.output / 'mode.baseline')
    old_mode['mode.parameters.victory_items_count'] = len(batteries)
    if decoded(a.porter, a.output / 'mode.diagnostic') != old_mode:
        raise SystemExit('mode diagnostic changed more than the battery count')
    plan = {'diagnostic_only': True, 'archive': str(a.archive), 'spec': spec, 'victory_items_count': len(batteries),
            'entries': entries, 'scope': __doc__.split('\n\n')[1]}
    (a.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(json.dumps({'plan': str(a.output / 'plan.json'), 'batteries': len(batteries)}))


if __name__ == '__main__':
    main()
