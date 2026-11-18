#!/usr/bin/env python3
"""Record authored Radar weather inputs from the original 0.11e archive.

This is an archive audit, not a claim of runtime or visual parity. Keep the
result alongside the frozen reference captures and their camera metadata.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

PROJECT_HASHES = {
    'day': '25a44a842ac1535b678b5e817a4e83bc4ea962dc69eda7d1b6559acc01a80671',
    'evening': '300845c5bdb3a734cf8d2fc90bff3dfa776de21acdc8218d7bc47b6c8fcd9010',
    'rain': '3590614cfe9971a9cf86ae72b57821038d52ccec37ce7ccb505e98fe7cab37b2',
}
POST_HASHES = {
    'day': '9974962cff3a509c904465bcf0ff3bbc49230a64486dc13b9e2d8de75580cbdc',
    'evening': '01d6c0d260ad44fbcd0312e13afc9a99e664498041456f40a0dfe4ce286e7029',
    'rain': '026a29b212764db792ca62651117ad459f4635a986f21741db9f9f89df884de3',
}


def audit(porter, database, output):
    decoder = Path(__file__).resolve().parents[1] / 'chemical-plant/audit_collision_region.py'
    spec = importlib.util.spec_from_file_location('weather_config', decoder)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    records = {}
    with tempfile.TemporaryDirectory(prefix='011e-weather-') as temporary:
        def read(resource):
            path = Path(temporary) / resource
            path.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([str(porter), 'extract', '--db', str(database),
                            '--path', resource, '--output', str(path)],
                           check=True, stdout=subprocess.DEVNULL)
            data = path.read_bytes()
            return module.config(data), hashlib.sha256(data).hexdigest()

        for weather, project in [('day', 'level_03'), ('evening', 'level_03_evn'),
                                 ('rain', 'level_03_rain')]:
            resource = f'projects/{project}/client_project'
            scene, scene_sha = read(resource)
            if scene_sha != PROJECT_HASHES[weather]:
                raise ValueError(f'{resource}: not the reviewed original 0.11e project')
            objects = scene['objects']
            if isinstance(objects, dict):
                objects = list(objects.values())
            environments = [o for o in objects if o.get('game_object_type') == 'environment']
            if len(environments) != 1:
                raise ValueError(f'{resource}: expected exactly one environment')
            post_path = 'material_instances/' + environments[0]['post_effect'] + '.material'
            material, material_sha = read(post_path)
            if material_sha != POST_HASHES[weather]:
                raise ValueError(f'{post_path}: not the reviewed original 0.11e material')
            properties = {key: value['value'] for key, value in
                          material['material']['post_process'].items()
                          if isinstance(value, dict) and 'value' in value}
            records[weather] = dict(project=resource, project_sha256=scene_sha,
                                    post_material=post_path, post_sha256=material_sha,
                                    properties=properties,
                                    sun=[o for o in objects if o.get('lib_name') == 'sun'])
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(dict(source_build='0.11e-884', maps=records),
                                     indent=2, sort_keys=True) + '\n')
    print(f'Wrote three authored weather contracts to {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('porter', 'database', 'output'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    audit(args.porter, args.database, args.output)
