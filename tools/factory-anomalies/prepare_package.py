#!/usr/bin/env python3
"""Prepare an isolated anomaly test package around an already packed database."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base_package', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    base, out = args.base_package.resolve(), args.output.resolve()
    if not (out / 'resources.db').is_file() or {p.name for p in out.iterdir()} != {'resources.db'}:
        parser.error('output must contain only the new private resources.db')
    for name in ('client', 'server'):
        shutil.copytree(base / name, out / name)
    for name in ('wineprefix.tar.gz', 'play-maps'):
        shutil.copy2(base / name, out / name)
    launcher = (Path(__file__).resolve().parents[1] / 'playable-map-pack/launch.py').read_text()
    old = "'--snapshots', '--checkpoints', '--recover', '--clock-sync'],"
    new = "'--snapshots', '--checkpoints', '--recover', '--clock-sync', '--packet-log', str(runtime/'packets.log')],"
    if launcher.count(old) != 1:
        raise RuntimeError('private server launch signature changed')
    (out / 'launch.py').write_text(launcher.replace(old, new))
    preset = out / 'presets/factory-020e-day'
    preset.mkdir(parents=True)
    for name in ('camera_hook.dll', 'user.cfg'):
        shutil.copy2(base / 'presets/factory-010-day' / name, preset / name)
    recipe = {'environment': {}, 'assets': {},
              'hook': 'presets/factory-020e-day/camera_hook.dll',
              'config': 'presets/factory-020e-day/user.cfg'}
    (preset / 'preset.json').write_text(json.dumps(recipe, indent=2) + '\n')
    catalog = json.loads((base / 'maps.json').read_text())
    catalog.update(maps=[{'key': 'factory-020e-day', 'title': 'Factory 0.20e / anomaly test',
                         'map_id': 196, 'database': 'resources.db',
                         'preset': 'presets/factory-020e-day/preset.json',
                         'validation': 'Offline audit only; gameplay acceptance pending.'}],
                   default_runtime='/tmp/factory-020e-anomaly-runtime', default_port=2264,
                   default_ui='on', notes=['Private anomaly experiment; native 0.26 shaders.'])
    (out / 'maps.json').write_text(json.dumps(catalog, indent=2) + '\n')
    prefix_hash = hashlib.sha256((out / 'wineprefix.tar.gz').read_bytes()).hexdigest()
    (out / 'manifest.json').write_text(json.dumps({'files': {'wineprefix.tar.gz': {'sha256': prefix_hash}}}, indent=2) + '\n')
    print(out / 'play-maps')


if __name__ == '__main__':
    main()
