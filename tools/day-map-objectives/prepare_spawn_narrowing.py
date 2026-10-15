#!/usr/bin/env python3
"""Prepare a reversible, same-length plan that narrows player spawns to chosen points.

DIAGNOSTIC ONLY. The respawn array keeps the listed authored records (one or
more per team), renumbered with dense point_id, and its count is lowered. All
authored transforms and every other leaf stay unchanged; an independent decoder
verifies that. Apply/revert with tools/render-compare/patch_archive_entries.py.

    prepare_spawn_narrowing.py ARCHIVE PROJECT_RESOURCE OUTPUT --points I,J,...
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_gameplay_patch import PORTER, leaf, read_resource  # noqa: E402
from prepare_delivery_diagnostic import decoded  # noqa: E402


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('archive', type=Path)
    ap.add_argument('resource')
    ap.add_argument('output', type=Path)
    ap.add_argument('--points', required=True)
    ap.add_argument('--porter', default=PORTER)
    ap.add_argument('--team', type=int, help='Diagnostic: let this team spawn at the chosen points (one-client checks)')
    a = ap.parse_args()
    if a.output.exists():
        raise SystemExit('choose a fresh output directory')
    chosen = [int(i) for i in a.points.split(',')]
    with a.archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    project, entry = read_resource(a.porter, a.archive, fat, a.resource)
    root = 'server_objects.respawn_points'
    at = leaf(project, root)
    table, _, _, kind, total = struct.unpack_from('<QQIHH', project, at)
    if kind != 4 or len(set(chosen)) != len(chosen) or not all(0 <= i < total for i in chosen):
        raise SystemExit('invalid point selection')
    fixed = bytearray(project)
    records = [project[table + 24 * i:table + 24 * (i + 1)] for i in chosen]
    for index, (record, source) in enumerate(zip(records, chosen)):
        fixed[table + 24 * index:table + 24 * (index + 1)] = record
        struct.pack_into('<i', fixed, leaf(project, f'{root}[{source}].point_id'), index)
        if a.team is not None:
            struct.pack_into('<i', fixed, leaf(project, f'{root}[{source}].team'), a.team)
    struct.pack_into('<H', fixed, at + 22, len(chosen))
    a.output.mkdir(parents=True)
    (a.output / 'project.baseline').write_bytes(project)
    (a.output / 'project.diagnostic').write_bytes(bytes(fixed))
    old, new = decoded(a.porter, a.output / 'project.baseline'), decoded(a.porter, a.output / 'project.diagnostic')
    prefix = root + '['
    want = {k: v for k, v in old.items() if not k.startswith(prefix)}
    for index, source in enumerate(chosen):
        for key, value in old.items():
            if key.startswith(f'{root}[{source}].'):
                want[f'{root}[{index}]' + key[len(f'{root}[{source}]'):]] = value
        want[f'{root}[{index}].point_id'] = index
        if a.team is not None:
            want[f'{root}[{index}].team'] = a.team
    if new != want:
        raise SystemExit('decoded diagnostic differs from the intended narrowing')
    digest = lambda b: hashlib.sha256(b).hexdigest()
    plan = {'diagnostic_only': True, 'archive': str(a.archive), 'points': chosen, 'team_override': a.team,
            'entries': [dict(entry, baseline='project.baseline', fixed='project.diagnostic',
                             baseline_sha256=digest(project), fixed_sha256=digest(bytes(fixed)))]}
    (a.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(json.dumps({'plan': str(a.output / 'plan.json'), 'points': chosen}))


if __name__ == '__main__':
    main()
