#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage a separate camera-enabled client; does not launch or alter source inputs."""
import argparse
from pathlib import Path
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('root', type=Path, help='Verified output containing resources.db')
parser.add_argument('--client-dir', type=Path, required=True)
parser.add_argument('--prefix-template', type=Path, required=True)
parser.add_argument('--capture-exe', type=Path, required=True)
parser.add_argument('--camera-hook', type=Path, required=True)
args = parser.parse_args()
root = args.root.resolve()
runtime = root / 'runtime'
if runtime.exists() or not (root / 'resources.db').is_file():
    parser.error('root must contain resources.db and must not already contain runtime/')
if not (args.prefix_template / 'system.reg').is_file():
    parser.error('--prefix-template must be an initialized Wine prefix')
for p in (args.capture_exe, args.camera_hook):
    if not p.is_file():
        parser.error(f'missing capture input: {p}')
shutil.copytree(args.prefix_template, runtime / 'wineprefix', symlinks=True,
                ignore=shutil.ignore_patterns('*.log', '*.dmp', '*.zip'))
binaries = runtime / 'client/binaries/x86'
binaries.mkdir(parents=True)
for p in (args.client_dir / 'binaries/x86').glob('*.dll'):
    shutil.copy2(p, binaries / p.name)
shutil.copy2(args.capture_exe, binaries / 'survarium-capture.exe')
shutil.copy2(args.camera_hook, binaries / 'camera_hook.dll')
shutil.copytree(args.client_dir / 'resources', runtime / 'client/resources', symlinks=True)
(runtime / 'client/resources.db').symlink_to(root / 'resources.db')
print(runtime)
