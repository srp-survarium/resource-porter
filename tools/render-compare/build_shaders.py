#!/usr/bin/env python3
"""Compile the optional legacy-atmosphere experiments with a local FXC."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def source_hashes(source, found=None):
    found = {} if found is None else found
    if source.name in found:
        return found
    data = source.read_bytes()
    found[source.name] = hashlib.sha256(data).hexdigest()
    for name in re.findall(rb'^\s*#include\s+"([^"]+)"', data, re.MULTILINE):
        source_hashes(source.parent / name.decode(), found)
    return found


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wine', required=True)
    parser.add_argument('--fxc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--weather', choices=('rain', 'day', 'evening', 'chemical_day'), default='rain')
    parser.add_argument('--source-version', choices=('010b', '011a'), default='010b')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {}
    def win(path):
        return 'Z:' + str(path.resolve()).replace('/', '\\')
    shader_dir = Path(__file__).parent / 'shaders'
    sources = sorted(shader_dir.glob('legacy_*.hlsl')) + [shader_dir / 'fullscreen_vs.hlsl']
    for source in sources:
        target = args.output / (source.stem.replace('_', '-') + '.dxbc')
        profile = 'vs_4_0' if source.name == 'fullscreen_vs.hlsl' else 'ps_4_0'
        defines = [] if args.weather == 'rain' else ['/D', 'LEGACY_WEATHER_' + args.weather.upper()]
        if args.source_version == '011a': defines += ['/D', 'LEGACY_SOURCE_011A']
        subprocess.run([args.wine, str(args.fxc), '/nologo', '/WX', '/T', profile,
                        '/E', 'main', '/O3', *defines, '/Fo', win(target), win(source)], check=True)
        manifest[source.name] = {
            'profile': profile,
            'weather': args.weather,
            'source_version': args.source_version,
            'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'source_dependencies': source_hashes(source),
            'output': target.name,
            'output_sha256': hashlib.sha256(target.read_bytes()).hexdigest(),
        }
    if not manifest:
        raise RuntimeError('no shader sources')
    (args.output / 'shader-build.json').write_text(json.dumps(manifest, indent=2) + '\n')
