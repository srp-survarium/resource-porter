#!/usr/bin/env python3
"""Prepare source020 Mie-field migration above cloud-off; never edit the DB."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct

from prepare_cloud_lighting_patch import prepare, config, RESOURCE


def entry_at(data, path):
    offset = 0
    for component in path:
        children, _, _, kind, count = struct.unpack_from('<QQIHH', data, offset)
        if kind == 4:
            index = int(component)
            if not 0 <= index < count:
                raise ValueError('indexed path outside table')
            offset = children+index*24
        elif kind == 3:
            matches = []
            for i in range(count):
                child = children+i*24
                name = struct.unpack_from('<Q', data, child+8)[0]
                if data[name:data.index(0, name)].decode('cp1251') == component:
                    matches.append(child)
            if len(matches) != 1:
                raise ValueError('named path is absent or ambiguous')
            offset = matches[0]
        else:
            raise ValueError('path traverses a leaf')
    return offset


def sky_patch(source, database, porter, output):
    prepare(source, database, porter, output)
    path = output/'bundle/overlay'/RESOURCE
    cloud_only = path.read_bytes()
    data = bytearray(cloud_only)
    expected = copy.deepcopy(config(data))
    donor = config(source.read_bytes())
    environments = {o['full_name']: o for o in donor['client_objects']
                    if o.get('game_object_type') == 'environment'}
    migrations = []
    for i, obj in enumerate(expected['client_objects']):
        if obj.get('game_object_type') != 'environment':
            continue
        src = environments[obj['full_name']]['properties']
        key = 'mie_fog_sun_color_falloff'
        value = src.get(key, src.get('mie_fog_falloff', 1.0))
        offset = entry_at(data, ['client_objects', str(i), 'properties', key])
        kind = struct.unpack_from('<H', data, offset+20)[0]
        if kind not in (1, 2) or (kind == 1 and int(value) != value):
            raise ValueError('native Mie exponent cannot represent donor value')
        numeric = '<i' if kind == 1 else '<f'
        before = struct.unpack_from(numeric, data, offset)[0]
        if before != 1.0 or value != 2.0:
            raise ValueError('pinned Radar day source/native exponent changed')
        struct.pack_into(numeric, data, offset, int(value) if kind == 1 else value)
        obj['properties'][key] = value
        migrations.append(dict(path=f'client_objects.{i}.properties.{key}',
                               before=before, after=value, byte_offset=offset))
    if not migrations or config(data) != expected:
        raise ValueError('migration missing or changed unrelated values')
    path.write_bytes(data)
    digest = hashlib.sha256(data).hexdigest()
    manifest_path = output/'bundle/port-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    manifest['asset_id'] = '020e-source-cloud-default-and-mie-field-migration'
    manifest['float_migrations'] = migrations
    manifest['files'][0].update(sha256=digest, source='binary-config:020e-source-environment')
    manifest_path.write_text(json.dumps(manifest, indent=2)+'\n')
    patch_path = output/'patch.json'
    patch = json.loads(patch_path.read_text())
    patch['entries'][0]['fixed_sha256'] = digest
    patch_path.write_text(json.dumps(patch, indent=2)+'\n')
    guard_path = output/'guard.json'
    guard = json.loads(guard_path.read_text())
    guard.update(after_sha256=digest, float_migrations=migrations,
                 cloud_only_sha256=hashlib.sha256(cloud_only).hexdigest())
    guard_path.write_text(json.dumps(guard, indent=2)+'\n')
    print(json.dumps(guard, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'database', 'porter', 'output'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    sky_patch(args.source, args.database, args.porter, args.output)
