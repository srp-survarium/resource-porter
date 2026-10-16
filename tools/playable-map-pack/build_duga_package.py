#!/usr/bin/env python3
"""Assemble the Duga (0.69d level_11 -> 0.26g) test package.

    build_duga_package.py PACKAGE DATABASE BUNDLE BASE_PACKAGE

PACKAGE      new directory (on a Linux filesystem: the Wine prefix and runtime
             must not live on NTFS)
DATABASE     the packed and verified 0.26g database containing map 205
BUNDLE       the bundle it was packed from (port-manifest.json, summary)
BASE_PACKAGE an existing play-maps package (three-day candidate) supplying the
             SRP direct-match client, DXVK, BugTrap, the match server, the
             Wine prefix template and launch.py; every file it supplies is
             checked against that package's manifest first.

Large files are hard-linked when source and package share a filesystem.
The package is one map, one client by default, UI on, the native 0.26 look
(no legacy shader overrides).
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import time

if len(sys.argv) != 5:
    raise SystemExit(__doc__)
PACKAGE, DATABASE, BUNDLE, BASE = (Path(a).resolve() for a in sys.argv[1:])
PACKAGE.mkdir(parents=True, exist_ok=False)
records = {}


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        while block := f.read(8 << 20):
            h.update(block)
    return h.hexdigest()


def place(source, relative, expected=None):
    source = Path(source).resolve()
    target = PACKAGE/relative
    target.parent.mkdir(parents=True, exist_ok=True)
    sha = digest(source)
    if expected and sha != expected:
        raise SystemExit(f'{source}: checksum {sha} differs from its manifest')
    try:
        os.link(source, target)
    except OSError:
        shutil.copy2(source, target)
    records[str(relative)] = {'size': target.stat().st_size, 'sha256': sha, 'source': str(source)}


base_manifest = json.loads((BASE/'manifest.json').read_text())['files']
for relative in ['client/binaries/x86/bugtrap.dll', 'client/binaries/x86/d3d11.dll', 'client/binaries/x86/dxgi.dll',
                 'client/binaries/x86/survarium-capture.exe', 'client/resources/sources/splash_screens/development_engine.bmp',
                 'client/resources/ssl/survarium_login_server.crt', 'server/match-server-026', 'wineprefix.tar.gz', 'launch.py']:
    place(BASE/relative, relative, base_manifest[relative]['sha256'])
# Native look: the base package's hook with no legacy-shader environment.
preset_dir = Path('presets/duga-069d-day')
place(BASE/'presets/radar-020e-day/camera_hook.dll', preset_dir/'camera_hook.dll',
      base_manifest['presets/radar-020e-day/camera_hook.dll']['sha256'])
place(BASE/'presets/radar-020e-day/user.cfg', preset_dir/'user.cfg',
      base_manifest['presets/radar-020e-day/user.cfg']['sha256'])
preset = {'environment': {}, 'assets': {}, 'hook': str(preset_dir/'camera_hook.dll'), 'config': str(preset_dir/'user.cfg')}
(PACKAGE/preset_dir/'preset.json').write_text(json.dumps(preset, indent=2)+'\n')

place(DATABASE, 'resources.db')
for name in ['port-manifest.json', 'scene-summary.json']:
    place(BUNDLE/name, Path('bundle')/name)

base_maps = json.loads((BASE/'maps.json').read_text())
catalog = {
    'format': 1,
    'created': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
    'maps': [{
        'key': 'duga-069d-day',
        'title': 'Duga radar station 0.69d / day, team deathmatch',
        'map_id': 205,
        'database': 'resources.db',
        'preset': str(preset_dir/'preset.json'),
        'validation': 'Static checks pass (shader variants, material textures, spawns); runtime: see README.txt.',
    }],
    'wine': base_maps['wine'],
    'python': base_maps['python'],
    'default_runtime': '/tmp/duga-069d-runtime',
    'default_port': 2291,
    'default_ui': 'on',
    'camera': 'actual character; no director request file is created',
    'notes': ['0.69d level_11 (Duga) ported down to 0.26g as map 205, mode 4 (team deathmatch), 0.69d TDM spawns.',
              'Native 0.26 look: no legacy shader overrides; see README.txt for what is and is not converted.'],
}
(PACKAGE/'maps.json').write_text(json.dumps(catalog, indent=2)+'\n')
(PACKAGE/'play-maps').write_text('#!/bin/sh\nset -eu\nexec '+catalog['python']+' "$(dirname -- "$0")/launch.py" "$@"\n')
(PACKAGE/'play-maps').chmod(0o755)
readme = Path(__file__).with_name('duga-package-README.txt')
shutil.copy2(readme, PACKAGE/'README.txt')
place(Path(__file__), 'build_duga_package.py')
for path in sorted(PACKAGE.rglob('*')):
    relative = str(path.relative_to(PACKAGE))
    if path.is_file() and relative not in records:
        records[relative] = {'size': path.stat().st_size, 'sha256': digest(path)}
(PACKAGE/'manifest.json').write_text(json.dumps({'files': records}, indent=2)+'\n')
print(json.dumps({'package': str(PACKAGE), 'files': len(records),
                  'resources.db': records['resources.db']['sha256']}, indent=2))
