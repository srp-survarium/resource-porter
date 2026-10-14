#!/usr/bin/env python3
"""Prepare reversible, same-length gallery diagnostics; never modify an archive.

Each batch narrows the indexed spawner array and makes point_id dense. Build2777
uses point_id as an index into an allocation sized from that array's count
(0x2F294B..0x2F29CE). Original authored transforms are retained verbatim.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess

PORTER = '/home/sheep/Projects/survarium/resource-porter/target/release/vostok-port'
ENV = dict(os.environ, LD_LIBRARY_PATH='/nix/store/483x61iy35irm4wr2b7dwzihljhp6da2-zlib-1.3.2/lib')

def command(*args):
    return subprocess.check_output([PORTER, *map(str, args)], env=ENV, text=True)

def leaf(data, path):
    offset = 0
    for part in re.findall(r'[^.\[\]]+', path):
        table, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind == 4:
            index = int(part)
            assert 0 <= index < count
            offset = table + 24 * index
        else:
            assert kind == 3
            found = []
            for index in range(count):
                child = table + 24 * index
                name, = struct.unpack_from('<Q', data, child + 8)
                if data[name:data.index(0, name)].decode() == part:
                    found.append(child)
            assert len(found) == 1, path
            offset = found[0]
    return offset

def digest(data):
    return hashlib.sha256(data).hexdigest()

def decoded(file):
    return {k: json.loads(v.split(':', 1)[1]) for k, v in
            (line.split('\t', 1) for line in command('dump-config', '--file', file, '--leaves').splitlines())}

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source_manifest', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--gallery-manifest', type=Path, default=Path('/tmp/battery-gallery-manifest.json'))
args = parser.parse_args()
assert not args.output.exists(), 'Choose a fresh plan directory'
args.output.mkdir(parents=True)
source = json.loads(args.source_manifest.read_text())
gallery = {'schema_version': 1, 'screenshots_per_spawn': 10, 'maps': [], 'planned_screenshots': 590}
for map_info in source['maps']:
    archive = Path(map_info['packed_db'])
    with archive.open('rb') as db:
        header = db.read(24)
        fat = db.read(struct.unpack_from('<I', header, 20)[0])
    key = map_info['slug']
    project_path = map_info['packed_project']
    level = '03' if key.startswith('factory') else '04'
    ns = 'v010w' if key.startswith('factory') else 'v011a'
    mode_path = f'gameplay/game_modes/{ns}_level_{level}_day_2.options'
    resources = {}
    for kind, path in [('project', project_path), ('mode', mode_path)]:
        stat = command('stat', '--db', archive, '--path', path)
        assert 'compressed: false, inlined: false' in stat
        size = int(re.search(r'^size_in_db=(\d+)$', stat, re.M)[1])
        position = int(re.search(r'^pos_in_db=(\d+)$', stat, re.M)[1])
        with archive.open('rb') as db:
            db.seek(position)
            baseline = db.read(size)
        needle = struct.pack('<I4xQ', size, position)
        found = fat.find(needle)
        assert found >= 0 and fat.find(needle, found + 1) == -1
        name = f'{key}-baseline.{kind}'
        (args.output / name).write_bytes(baseline)
        resources[kind] = {'resource': path, 'position': position, 'crc_offset': 24 + found + 16,
                           'baseline': name, 'baseline_sha256': digest(baseline), 'data': baseline}
    baseline = resources['project']['data']
    root = leaf(baseline, 'server_objects.victory_item_spawners')
    table, _, _, kind, count = struct.unpack_from('<QQIHH', baseline, root)
    assert kind == 4 and count == map_info['candidate_count']
    mode = resources['mode']['data']
    count_leaf = leaf(mode, 'mode.parameters.victory_items_count')
    assert struct.unpack_from('<H', mode, count_leaf + 20)[0] == 1
    map_gallery = {'key': key, 'title': 'Factory 0.10b day' if level == '03' else 'Chemical Plant 0.11a day',
                   'map_id': map_info['map_id'], 'spawns': [], 'batches': []}
    for batch in map_info['diagnostic_batches']:
        selected = [s for s in map_info['candidates'] if s['diagnostic_batch'] == batch['batch']]
        n = len(selected)
        assert 1 <= n <= 10
        fixed = bytearray(baseline)
        # Copy the original array entries before overwriting any entries.
        records = [baseline[table + 24*s['source_array_index']:table + 24*(s['source_array_index']+1)] for s in selected]
        for index, (record, spawn) in enumerate(zip(records, selected)):
            fixed[table + 24*index:table + 24*(index+1)] = record
            point = leaf(baseline, f"server_objects.victory_item_spawners[{spawn['source_array_index']}].point_id")
            assert struct.unpack_from('<H', baseline, point + 20)[0] == 1
            struct.pack_into('<i', fixed, point, index)
        struct.pack_into('<H', fixed, root + 22, n)
        changed_mode = bytearray(mode)
        struct.pack_into('<i', changed_mode, count_leaf, n)
        entries = []
        for kind, data in [('project', fixed), ('mode', changed_mode)]:
            meta = dict(resources[kind])
            original = meta.pop('data')
            assert len(original) == len(data)
            name = f"{key}-batch-{batch['batch']:02d}.{kind}"
            (args.output / name).write_bytes(data)
            meta.update(fixed=name, fixed_sha256=digest(data))
            entries.append(meta)
        # Independent decoder verifies visibility, ID density, exact transforms,
        # and that no other decoded project leaf changed.
        original_leaves = decoded(args.output / resources['project']['baseline'])
        actual = decoded(args.output / entries[0]['fixed'])
        prefix = 'server_objects.victory_item_spawners['
        assert {k:v for k,v in actual.items() if not k.startswith(prefix)} == {k:v for k,v in original_leaves.items() if not k.startswith(prefix)}
        for index, spawn in enumerate(selected):
            p = f'{prefix}{index}]'
            old = f"{prefix}{spawn['source_array_index']}]"
            assert actual[p+'.point_id'] == index
            for field in ('position', 'rotation', 'spawned_item.position', 'spawned_item.rotation', 'priority', 'full_name'):
                assert actual[p+'.'+field] == original_leaves[old+'.'+field]
            assert actual[p+'.position'] == spawn['authored_position']
        actual_count = len([k for k in actual if k.startswith(prefix) and k.endswith('.point_id')])
        assert actual_count == n
        fixed_mode = decoded(args.output / entries[1]['fixed'])
        original_mode = decoded(args.output / resources['mode']['baseline'])
        original_mode['mode.parameters.victory_items_count'] = n
        assert fixed_mode == original_mode
        plan_name = f"{key}-batch-{batch['batch']:02d}.json"
        plan = {'diagnostic_only': True, 'archive': str(archive), 'map': key, 'batch': batch['batch'],
                'victory_items_count': n, 'original_source_indices': [s['source_array_index'] for s in selected],
                'entries': entries, 'runtime_validated': False,
                'validation': 'All transforms unchanged; point_id dense; only spawner selection/count and battery count differ.'}
        (args.output / plan_name).write_text(json.dumps(plan, indent=2)+'\n')
        map_gallery['batches'].append({'index': batch['batch'], 'plan': str((args.output / plan_name).resolve()), 'battery_count': n,
                                      'spawns': [s['source_array_index'] for s in selected]})
        for spawn in selected:
            map_gallery['spawns'].append({'index': spawn['source_array_index'], 'id': spawn['candidate_id'],
                                         'point_id': spawn['source_point_id'], 'name': spawn['source_name'],
                                         'position': spawn['authored_position'], 'rotation': spawn['authored_rotation'],
                                         'initial_body_position': original_leaves[f"server_objects.victory_item_spawners[{spawn['source_array_index']}].spawned_item.position"],
                                         'runtime_coordinate_basis': 'native rigid-body centre; authored position is mesh origin',
                                         'batch': batch['batch'], 'diagnostic_batch': batch['batch'], 'diagnostic_point_id': spawn['diagnostic_point_id'],
                                         'kind': 'candidate', 'active_runtime_battery_id': None, 'settled_position': None,
                                         'source_full_name': spawn['source_full_name']})
    gallery['maps'].append(map_gallery)
args.gallery_manifest.write_text(json.dumps(gallery, indent=2)+'\n')
print(json.dumps({'plans': sum(len(m['batches']) for m in gallery['maps']),
                  'spawns': sum(len(m['spawns']) for m in gallery['maps']), 'output': str(args.output),
                  'gallery_manifest': str(args.gallery_manifest)}, indent=2))
