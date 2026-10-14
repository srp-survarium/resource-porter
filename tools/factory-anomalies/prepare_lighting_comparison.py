#!/usr/bin/env python3
"""Prepare a private map196 A/B harness; never write the combined release DB."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil

APPLY_SHA = '94b548fecc000ea0b7cbf04eeeb1678afbefcfe5dce699fa51b6cbbd4942f07e'


def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(8*1024*1024), b''):
            h.update(block)
    return h.hexdigest()


def prepare(source, output, hook, apply, expected_db_sha):
    if sha(apply) != APPLY_SHA:
        raise ValueError('not the exact original020e ambient apply shader')
    catalog = json.loads((source/'maps.json').read_text())
    item, = [m for m in catalog['maps'] if m['map_id'] == 196]
    recipe_path = Path(item['preset'])
    baseline_bytes = (source/recipe_path).read_bytes()
    recipe = json.loads(baseline_bytes)
    if recipe['environment'] or recipe['assets']:
        raise ValueError('map196 baseline is no longer native; review before comparing')
    output.mkdir(exist_ok=False)
    for name in ('launch.py',):
        shutil.copy2(source/name, output/name)
    for name in ('client', 'server'):
        shutil.copytree(source/name, output/name)
    for name in (item['database'], 'wineprefix.tar.gz'):
        os.link(source/name, output/name)  # shared immutable bytes; no second8GiB DB
    shutil.copytree(source/recipe_path.parent, output/recipe_path.parent)
    baseline = output/'baseline-preset.json'
    baseline.write_bytes(baseline_bytes)
    experiment = output/'experiment'
    experiment.mkdir()
    shutil.copy2(hook, experiment/'camera_hook.dll')
    shutil.copy2(apply, experiment/'apply_ambient_lights.ps')
    candidate = dict(recipe)
    candidate.update(hook='experiment/camera_hook.dll',
                     environment={'SRP_LEGACY_020E_AMBIENT': '1'},
                     assets={'SRP_LEGACY_020E_AMBIENT_APPLY': 'experiment/apply_ambient_lights.ps'})
    (output/'candidate-preset.json').write_text(json.dumps(candidate, indent=2)+'\n')
    catalog.update(maps=[item], default_runtime='/tmp/factory-020e-lighting-runtime',
                   default_port=2264, default_ui='off')
    (output/'maps.json').write_text(json.dumps(catalog, indent=2)+'\n')
    report = dict(source=str(source), database=item['database'], database_sha256=expected_db_sha,
                  preset=str(recipe_path), baseline_sha256=sha(baseline),
                  candidate_sha256=sha(output/'candidate-preset.json'),
                  native_hook_sha256=sha(output/recipe['hook']),
                  candidate_hook_sha256=sha(experiment/'camera_hook.dll'),
                  apply_sha256=APPLY_SHA,
                  scope='Private recipe copies; database hard-linked read-only by convention. '
                        'Wait for parent restoration and GPU lease, then verify DB hash before launch.')
    (output/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    # Launcher verifies the Wine template before extracting a fresh prefix.
    (output/'manifest.json').write_text(json.dumps({'files': {
        'wineprefix.tar.gz': {'size': (output/'wineprefix.tar.gz').stat().st_size,
                              'sha256': sha(output/'wineprefix.tar.gz')}
    }}, indent=2)+'\n')
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'output', 'hook', 'apply'):
        p.add_argument(name, type=Path)
    p.add_argument('--expected-db-sha', required=True)
    a = p.parse_args()
    print(json.dumps(prepare(a.source.resolve(), a.output.resolve(), a.hook, a.apply,
                             a.expected_db_sha), indent=2))
