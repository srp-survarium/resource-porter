#!/usr/bin/env python3
"""Prepare the three day-map candidate around an already verified resources.db."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


def fingerprint(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        while block := source.read(8 * 1024 * 1024):
            digest.update(block)
    return {'size': path.stat().st_size, 'sha256': digest.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-package', type=Path, required=True)
    parser.add_argument('--anomaly-package', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    if not (out / 'resources.db').is_file():
        parser.error('pack and verify resources.db first')
    if any(path.name not in {'resources.db', 'packing'} for path in out.iterdir()):
        parser.error('output already contains package files')
    bundle = read(args.bundle / 'port-manifest.json')
    if sorted(item['id'] for item in bundle['maps']) != [194, 195, 196]:
        parser.error('bundle must contain exactly the three reviewed map registrations')
    base = args.base_package.resolve()
    for name in ('client', 'server'):
        shutil.copytree(base / name, out / name)
    for name in ('wineprefix.tar.gz', 'play-maps'):
        shutil.copy2(base / name, out / name)
    shutil.copy2(Path(__file__).with_name('launch.py'), out / 'launch.py')
    catalog = read(base / 'maps.json')
    catalog.update(maps=[], default_runtime='/tmp/three-day-map-runtime', default_port=2266,
                   notes=['Candidate: native loading and donor-specific visual calibration pending.',
                          'Radar was previously mislabelled Factory in investigation scripts.'])
    recipes = [
        (base, 'chemical-011-day', 'chemical-011e-day', 'Chemical Plant 0.11e / day', 194),
        (base, 'factory-010-day', 'radar-011e-day', 'Vostok Radar Station 0.11e / day', 195),
        (args.anomaly_package.resolve(), 'factory-020e-day', 'radar-020e-day',
         'Vostok Radar Station 0.20e / day with anomalies', 196),
    ]
    provenance = []
    for source, old, key, title, map_id in recipes:
        old_prefix, new_prefix = f'presets/{old}', f'presets/{key}'
        shutil.copytree(source / old_prefix, out / new_prefix)
        recipe = read(source / old_prefix / 'preset.json')
        for field in ('hook', 'config'):
            recipe[field] = recipe[field].replace(old_prefix + '/', new_prefix + '/', 1)
        recipe['assets'] = {name: path.replace(old_prefix + '/', new_prefix + '/', 1)
                            for name, path in recipe['assets'].items()}
        write(out / new_prefix / 'preset.json', recipe)
        catalog['maps'].append({'key': key, 'title': title, 'map_id': map_id,
                                'database': 'resources.db', 'preset': new_prefix + '/preset.json',
                                'validation': 'Inherited diagnostic recipe; donor-specific runtime review pending.'})
        provenance.append({'map_id': map_id, 'source_package': str(source),
                           'source_preset': old_prefix,
                           'source_recipe': fingerprint(source / old_prefix / 'preset.json')})
    write(out / 'maps.json', catalog)
    write(out / 'validation/composition.json', bundle)
    write(out / 'validation/recipe-provenance.json', provenance)
    records = {str(path.relative_to(out)): fingerprint(path) for path in sorted(out.rglob('*'))
               if path.is_file() and 'packing' not in path.relative_to(out).parts}
    write(out / 'manifest.json', {'files': records, 'candidate': True})
    print(json.dumps({'package': str(out), 'maps': len(catalog['maps']), 'files': len(records)}))


if __name__ == '__main__':
    main()
