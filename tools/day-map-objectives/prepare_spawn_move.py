#!/usr/bin/env python3
"""Prepare a reversible, same-length plan that moves every player spawn to one spot.

DIAGNOSTIC ONLY (a "spawn me here" aid for visual checks). Every
server_objects.respawn_points record of each team gets the team's position
and a yaw facing the given point; nothing else changes, which an independent
decoder verifies. Batteries, collectors and the game mode are untouched.
Apply/revert with tools/render-compare/patch_archive_entries.py; the plan
pins the project's baseline and diagnostic SHA-256.

    prepare_spawn_move.py ARCHIVE SPEC.json OUTPUT
SPEC: {"project": RES, "teams": {"0": {"position": [x,y,z], "face": [x,z]}, "1": {...}}}
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_gameplay_patch import PORTER, count, f32, leaf, read_resource, vector  # noqa: E402
from prepare_delivery_diagnostic import decoded  # noqa: E402


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
    project, entry = read_resource(a.porter, a.archive, fat, spec['project'])
    fixed = bytearray(project)
    expected, written = {}, {}

    def put3(path, value):
        pointer, _ = vector(project, path)
        value = f32(value)
        if written.get(pointer, value) != value:
            raise SystemExit('shared float storage with conflicting values: ' + path)
        written[pointer] = value
        struct.pack_into('<3f', fixed, pointer, *value)
        expected[path] = list(value)

    root = 'server_objects.respawn_points'
    for i in range(count(project, root)):
        team = struct.unpack_from('<i', project, leaf(project, f'{root}[{i}].team'))[0]
        target = spec['teams'][str(team)]
        p, face = target['position'], target['face']
        put3(f'{root}[{i}].position', p)
        put3(f'{root}[{i}].rotation', [0.0, math.atan2(-(face[0] - p[0]), face[1] - p[2]), 0.0])
    a.output.mkdir(parents=True)
    (a.output / 'project.baseline').write_bytes(project)
    (a.output / 'project.diagnostic').write_bytes(bytes(fixed))
    norm = lambda d: {k: (f32(v) if isinstance(v, list) and len(v) == 3 else v) for k, v in d.items()}
    want = decoded(a.porter, a.output / 'project.baseline')
    want.update(expected)
    got = norm(decoded(a.porter, a.output / 'project.diagnostic'))
    if got != norm(want):
        diff = sorted(k for k in set(got) | set(want) if got.get(k) != norm(want).get(k))[:6]
        raise SystemExit(f'decoded diagnostic differs from the intended move: {diff}')
    digest = lambda b: hashlib.sha256(b).hexdigest()
    plan = {'diagnostic_only': True, 'archive': str(a.archive), 'spec': spec, 'moved_points': count(project, root),
            'entries': [dict(entry, baseline='project.baseline', fixed='project.diagnostic',
                             baseline_sha256=digest(project), fixed_sha256=digest(bytes(fixed)))]}
    (a.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(json.dumps({'plan': str(a.output / 'plan.json'), 'moved_points': plan['moved_points']}))


if __name__ == '__main__':
    main()
