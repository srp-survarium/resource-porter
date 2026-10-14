#!/usr/bin/env python3
"""Prepare isolated post/probe comparison presets; never launch or alter the input."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil

BASELINE_DB = '241a6c1c8777458d0e23c2478af90292e2169a3c85e4bdea12e25d0e351cc08d'


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(8 * 1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def prepare(package, output, key, post, sphere, box, runtime, port):
    if output.exists():
        raise ValueError('comparison output already exists')
    catalogue = json.loads((package / 'maps.json').read_text())
    entries = [entry for entry in catalogue['maps'] if entry['key'] == key]
    if len(entries) != 1 or entries[0]['map_id'] not in (194, 195):
        raise ValueError('expected exactly one original011e map')
    entry = entries[0]
    if entry['database'] != 'resources.db' or digest(package / 'resources.db') != BASELINE_DB:
        raise ValueError('shared candidate differs from the reviewed baseline')
    replacements = dict(SRP_LEGACY_RAIN_TONEMAP=post,
                        SRP_LEGACY_PROBE_DIFFUSE=sphere,
                        SRP_LEGACY_PROBE_DIFFUSE_BOX=box)
    for file in replacements.values():
        if file.read_bytes()[:4] != b'DXBC':
            raise ValueError('expected compiled DXBC: ' + str(file))
    source_preset = package / entry['preset']
    baseline = json.loads(source_preset.read_text())
    output.mkdir()
    for directory in ('client', 'server', 'presets'):
        shutil.copytree(package / directory, output / directory)
    for name in ('launch.py', 'manifest.json'):
        shutil.copy2(package / name, output / name)
    # The database and Wine template are read-only inputs to this harness.
    for name in ('resources.db', 'wineprefix.tar.gz'):
        os.link(package / name, output / name)
    keys = []
    for arm in ('post', 'source-probe', 'combined'):
        arm_key = key + '-' + arm
        keys.append(arm_key)
        folder = output / 'presets' / arm_key
        folder.mkdir()
        preset = copy.deepcopy(baseline)
        selected = list(replacements) if arm == 'combined' else (
            ['SRP_LEGACY_RAIN_TONEMAP'] if arm == 'post' else
            ['SRP_LEGACY_PROBE_DIFFUSE', 'SRP_LEGACY_PROBE_DIFFUSE_BOX'])
        for variable in selected:
            destination = folder / (variable + '.dxbc')
            shutil.copy2(replacements[variable], destination)
            preset['assets'][variable] = str(destination.relative_to(output))
        destination = folder / 'preset.json'
        destination.write_text(json.dumps(preset, indent=2) + '\n')
        variant = dict(entry, key=arm_key, preset=str(destination.relative_to(output)),
                       validation='Experimental source011e ' + arm + '; no visual parity claim.')
        catalogue['maps'].append(variant)
    catalogue['default_runtime'] = str(runtime)
    catalogue['default_port'] = port
    (output / 'maps.json').write_text(json.dumps(catalogue, indent=2) + '\n')
    report = dict(baseline_package=str(package), database_sha256=BASELINE_DB,
                  source_preset_sha256=digest(source_preset), baseline_preset=baseline,
                  inputs={name: dict(path=str(file), sha256=digest(file))
                          for name, file in replacements.items()}, keys=keys,
                  scope='Probe arm replaces inherited equations with the source011e candidate; '
                        'Radar also changes inherited gather/mip assumptions. Shared inputs unchanged.')
    (output / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(package=str(output), keys=keys)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('package', 'output', 'post', 'sphere', 'box', 'runtime'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--key', required=True)
    parser.add_argument('--port', required=True, type=int)
    args = parser.parse_args()
    prepare(args.package.resolve(), args.output.resolve(), args.key,
            args.post.resolve(), args.sphere.resolve(), args.box.resolve(),
            args.runtime.resolve(), args.port)
