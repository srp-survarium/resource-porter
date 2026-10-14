#!/usr/bin/env python3
"""Prepare, but do not apply, a reversible character-spawn diagnostic patch."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
from audit_collision_region import config


def leaf(data, path):
    offset = 0
    for part in re.findall(r'[^.\[\]]+', path):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind == 4:
            if not 0 <= int(part) < count: raise ValueError('array index outside config')
            offset = table+24*int(part)
        elif kind == 3:
            matches = []
            for i in range(count):
                child = table+24*i
                name = struct.unpack_from('<Q', data, child+8)[0]
                if data[name:data.index(0, name)].decode('cp1251') == part: matches.append(child)
            if len(matches) != 1: raise ValueError(f'ambiguous/missing field {path}')
            offset = matches[0]
        else: raise ValueError(f'not a table: {path}')
    return offset


def prepare(porter, archive, output, position, resource='projects/v011a_level_04/day_2/client_project', rotation=None):
    output.mkdir(exist_ok=False)
    baseline_path = output/'baseline.project'
    subprocess.run([str(porter), 'extract', '--db', str(archive), '--path', resource,
                    '--output', str(baseline_path)], check=True)
    baseline = baseline_path.read_bytes(); fixed = bytearray(baseline)
    original = config(baseline)
    points = original['server_objects']['respawn_points']
    if not points: raise ValueError('project has no player spawn points')
    changes = []
    for i, point in enumerate(points):
        for field, value in [('position', position), ('rotation', rotation)]:
            if value is None: continue
            path = f'server_objects.respawn_points[{i}].{field}'
            offset = leaf(baseline, path)
            if struct.unpack_from('<H', baseline, offset+20)[0] != 7: raise ValueError('spawn not float3')
            vector = struct.unpack_from('<Q', baseline, offset)[0]
            struct.pack_into('<3f', fixed, vector, *value)
            changes.append(dict(path=path, before=point[field], after=value))
    decoded = config(fixed)
    for point, before in zip(decoded['server_objects']['respawn_points'], points):
        point['position'] = before['position']
        if rotation is not None: point['rotation'] = before['rotation']
    if decoded != original: raise ValueError('patch changes more than player spawn transforms')
    stat = subprocess.check_output([str(porter), 'stat', '--db', str(archive), '--path', resource], text=True)
    pos = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
    with archive.open('rb') as db:
        header = db.read(24); fat = db.read(struct.unpack_from('<I', header, 20)[0])
        db.seek(pos)
        if db.read(len(baseline)) != baseline: raise ValueError('archive changed while preparing')
    needle = struct.pack('<I4xQ', len(baseline), pos); at = fat.find(needle)
    if at < 0 or fat.find(needle, at+1) >= 0: raise ValueError('archive entry not unique')
    (output/'diagnostic.project').write_bytes(fixed)
    entry = dict(resource=resource, position=pos, crc_offset=at+40,
                 baseline='baseline.project', fixed='diagnostic.project',
                 baseline_sha256=hashlib.sha256(baseline).hexdigest(),
                 fixed_sha256=hashlib.sha256(fixed).hexdigest(), changes=changes)
    (output/'patch.json').write_text(json.dumps(dict(scope='Diagnostic only; restore before release. '
        'Moves actual character spawns, not the render camera. Existing collision and objectives remain unchanged.',
        entries=[entry]), indent=2)+'\n')
    print(json.dumps(dict(spawns=len(points), position=position, plan=str(output/'patch.json'))))


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('porter', type=Path); ap.add_argument('archive', type=Path); ap.add_argument('output', type=Path)
    ap.add_argument('--position', nargs=3, type=float, default=[-64.5, 5.72, 27.5])
    ap.add_argument('--resource', default='projects/v011a_level_04/day_2/client_project')
    ap.add_argument('--rotation', nargs=3, type=float)
    a = ap.parse_args(); prepare(a.porter, a.archive, a.output, a.position, a.resource, a.rotation)
