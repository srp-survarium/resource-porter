#!/usr/bin/env python3
"""Refuse map projects whose environment probes do not resolve.

    verify_probe_textures.py --porter VOSTOK_PORT DB PROJECT... [--json OUT]

0.26g renders NaN when a probe it draws has no cube: on native Cologne
(level_08 prerain) two probes name cubes the database lacks, and the game
shows a black screen when firing plus flickering squares that spread through
TAA. A scene with no probe at all leaves forward passes without a cube too.
For every PROJECT (a client_project path inside DB) this checks that

* the scene has at least one enabled environment probe, and
* the cube each environment probe uses for the project's variant (its
  directory, e.g. `day_4`: `textures.<variant>`, else `textures.default`,
  else `texture`), and its `_diffuse` companion, exists in DB as
  `textures/<name>.dds` with its `.options`.

Exit status 1 lists every unresolved name.
"""
import argparse
import json
import subprocess
import sys
from collections import defaultdict


def leaves(porter, db, path):
    out = subprocess.run([porter, 'dump-db-config', '--db', db, '--path', path, '--leaves'],
                         capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        if '\t' in line:
            key, value = line.split('\t', 1)
            yield key, value


def string(value):
    return value[len('string:"'):-1] if value.startswith('string:"') else None


def probes(rows):
    """{object index: {field: value}} for the environment_probe client objects."""
    objects = defaultdict(dict)
    for key, value in rows:
        if not key.startswith('client_objects['):
            continue
        index, _, field = key[len('client_objects['):].partition('].')
        objects[int(index)][field] = value
    return {i: o for i, o in objects.items() if o.get('game_object_type') == 'string:"environment_probe"'}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--porter', required=True)
    ap.add_argument('db')
    ap.add_argument('project', nargs='+')
    ap.add_argument('--json')
    a = ap.parse_args()
    listing = set(subprocess.run([a.porter, 'list', '--db', a.db, '--prefix', 'textures/'],
                                 capture_output=True, text=True, check=True).stdout.split())
    report, failed = {}, False
    for project in a.project:
        found = probes(leaves(a.porter, a.db, project))
        # The project directory is the variant the game looks probes up by
        # (`day_4`, `prerain_3`): textures.<variant>, else textures.default,
        # else texture. Cubes of other variants are not loaded for it.
        variant = project.rstrip('/').split('/')[-2]
        names = set()
        for o in found.values():
            for field in (f'textures.{variant}', 'textures.default', 'texture'):
                if (name := string(o.get(field, ''))):
                    names.add(name)
                    break
        missing = sorted(f'textures/{n}{suffix}.{ext}' for n in names for suffix in ('', '_diffuse')
                         for ext in ('dds', 'options') if f'textures/{n}{suffix}.{ext}' not in listing)
        enabled = sum(1 for o in found.values() if o.get('enabled', 'bool:true') == 'bool:true')
        problems = missing + ([] if enabled else ['no enabled environment probe'])
        report[project] = {'probes': len(found), 'enabled': enabled, 'textures': len(names), 'problems': problems}
        failed |= bool(problems)
        print(f'{project}: {len(found)} probes ({enabled} enabled), {len(names)} cubes, '
              f'{len(problems)} problems' + ''.join(f'\n  {p}' for p in problems))
    if a.json:
        with open(a.json, 'w') as out:
            json.dump(report, out, indent=2)
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
