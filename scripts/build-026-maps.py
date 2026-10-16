#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build, pack and verify a playable 0.26g map database from local retail data."""
import argparse
from pathlib import Path
import shutil
import subprocess

REPO = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('profile', choices=['fog-school', '021-maps'])
    parser.add_argument('--target-db', type=Path, required=True)
    parser.add_argument('--donor-db', type=Path, help='Retail 0.21d; required for 021-maps')
    parser.add_argument('--output', type=Path, required=True, help='New directory for bundle, work files and resources.db')
    parser.add_argument('--porter', default=str(REPO / 'target/release/vostok-port'))
    args = parser.parse_args()
    if not args.target_db.is_file():
        parser.error('--target-db must be an existing retail database')
    if args.profile == '021-maps' and (not args.donor_db or not args.donor_db.is_file()):
        parser.error('021-maps requires an existing --donor-db')
    if args.output.exists():
        parser.error('--output must be a new directory; existing releases are preserved')
    porter = shutil.which(args.porter)
    if not porter:
        parser.error('vostok-port not found; run cargo build --locked --release or pass --porter')
    output = args.output.resolve()
    bundle = output / 'bundle'
    target = args.target_db.resolve()
    if args.profile == 'fog-school':
        command = ['build-survarium026-fog']
    else:
        command = ['build-survarium021-maps', '--manifest', str(REPO / 'ports/survarium-021d-to-026g.json'),
                   '--donor-db', str(args.donor_db.resolve())]
    subprocess.run([porter, *command, '--target-db', str(target), '--output', str(bundle)], check=True)
    subprocess.run([porter, 'apply', '--base-db', str(target), '--bundle', str(bundle),
                    '--output-db', str(output / 'resources.db')], check=True)
    subprocess.run([porter, 'verify', '--db', str(output / 'resources.db'), '--bundle', str(bundle)], check=True)
    print(f'Playable database: {output / "resources.db"}')
    print('Map ID: 67' if args.profile == 'fog-school' else 'Map IDs: 160–177 (see docs/survarium-021-to-026.md)')


if __name__ == '__main__':
    main()
