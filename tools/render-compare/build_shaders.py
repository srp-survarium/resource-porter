#!/usr/bin/env python3
"""Compile the optional legacy-atmosphere experiments with a local FXC."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wine', required=True)
    parser.add_argument('--fxc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {}
    def win(path):
        return 'Z:' + str(path.resolve()).replace('/', '\\')
    for source in sorted(Path(__file__).parent.joinpath('shaders').glob('legacy_*.hlsl')):
        target = args.output / (source.stem.replace('_', '-') + '.dxbc')
        subprocess.run([args.wine, str(args.fxc), '/nologo', '/WX', '/T', 'ps_4_0',
                        '/E', 'main', '/O3', '/Fo', win(target), win(source)], check=True)
        manifest[source.name] = {
            'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'output': target.name,
            'output_sha256': hashlib.sha256(target.read_bytes()).hexdigest(),
        }
    if not manifest:
        raise RuntimeError('no shader sources')
    (args.output / 'shader-build.json').write_text(json.dumps(manifest, indent=2) + '\n')
