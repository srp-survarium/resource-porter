#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build and verify the v0.11a Chemical Plant port in a new output directory."""
import argparse
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--porter', type=Path, required=True)
parser.add_argument('--donor-db', type=Path, required=True)
parser.add_argument('--target-db', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if args.output.exists():
    parser.error('--output must be new; previous evidence is preserved')
if not shutil.which('ffmpeg'):
    parser.error('FFmpeg must be on PATH for original ambient sound RMS caches')
for value in (args.porter, args.donor_db, args.target_db):
    if not value.is_file():
        parser.error(f'missing input: {value}')
repo = Path(__file__).resolve().parents[2]
output = args.output.resolve()
bundle = output / 'bundle'
def run(*arguments):
    subprocess.run([str(args.porter.resolve()), *map(str, arguments)], check=True)
run('build-survarium-maps-026', '--manifest', repo / 'ports/chemical-plant-011a-to-026g.json',
    '--donor-db', args.donor_db.resolve(), '--target-db', args.target_db.resolve(), '--output', bundle)
run('apply', '--base-db', args.target_db.resolve(), '--bundle', bundle,
    '--work-dir', output / 'apply', '--output-db', output / 'resources.db')
run('verify', '--db', output / 'resources.db', '--bundle', bundle)
print(f'Chemical Plant, map 193: {output / "resources.db"}')
