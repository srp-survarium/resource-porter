#!/usr/bin/env python3
"""Exercise original probe packet boundaries with captured front/back draws."""
import argparse
import json
from pathlib import Path
import re
import shutil
import struct
import tempfile

from replay_probe_011e import prepare


def check(draws, fixture):
    results = []
    for original in draws:
        match = re.fullmatch(r'(.+)\.probe-draw(-\d+)?', original.name)
        frame, variant = match.groups()
        variant = variant or ''
        with tempfile.TemporaryDirectory(prefix='check-original-probe-') as directory:
            root = Path(directory)
            prefixes = (original.name + '.', frame + '.probe-before' + variant + '.', frame + '.probe-output' + variant + '.')
            for path in original.parent.iterdir():
                if path.is_file() and path.name.startswith(prefixes):
                    shutil.copyfile(path, root / path.name)
            draw = root / original.name
            valid = root / 'valid'
            prepare(draw, fixture, valid)
            rejected = []

            def rejects(name, output=None):
                target = output or root / name
                try:
                    prepare(draw, fixture, target)
                except (ValueError, FileNotFoundError):
                    assert target == valid or not target.exists()
                    rejected.append(name)
                else:
                    raise AssertionError(name + ' accepted')

            def mutate(suffix, name, transform):
                path = Path(str(draw) + suffix)
                old = path.read_bytes()
                path.write_bytes(transform(old))
                try:
                    rejects(name)
                finally:
                    path.write_bytes(old)

            rejects('overwrite', valid)
            mutate('.dxbc', 'wrong-pixel-shader', lambda b: b[:-1] + bytes([b[-1] ^ 1]))
            mutate('.vs.dxbc', 'wrong-vertex-shader', lambda b: b[:-1] + bytes([b[-1] ^ 1]))
            mutate('.cb0.bin', 'short-constants', lambda b: b[:-4])
            mutate('.geometry.json', 'wrong-volume-stride', lambda b: json.dumps(dict(json.loads(b), stride=16)).encode())
            mutate('.indices.bin', 'out-of-range-vertex', lambda b: b'\xff\xff' + b[2:])
            mutate('.vertices.bin', 'nonfinite-volume', lambda b: struct.pack('<f', float('nan')) + b[4:])
            mutate('.depth-state.json', 'wrong-stencil-reference', lambda b: json.dumps(dict(json.loads(b), reference=0)).encode())
            mutate('.depth.rt00.bin', 'truncated-stencil', lambda b: b[:-4])
            mutate('.blend-state.bin', 'wrong-alpha-blend', lambda b: b[:12] + struct.pack('<I', 2) + b[16:])
            mutate('.rt00.bin', 'truncated-position', lambda b: b[:-2])
            mutate('.rt03.face5.mip00.bin', 'truncated-cube-face', lambda b: b[:-16])
            last_mip = json.loads(Path(str(draw) + '.targets.json').read_text())[3]['mip_levels'] - 1
            mip = Path(str(draw) + f'.rt03.face5.mip{last_mip:02}.bin')
            old = mip.read_bytes()
            mip.unlink()
            try:
                rejects('missing-last-cube-mip')
            finally:
                mip.write_bytes(old)
            mutate('.targets.json', 'missing-ao', lambda b: json.dumps([r for r in json.loads(b) if r['slot'] != 4]).encode())
            results.append(dict(draw=str(original), rejected=rejected))
    print(json.dumps(dict(preparation='passed', results=results), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixture', type=Path)
    parser.add_argument('draws', type=Path, nargs='+')
    args = parser.parse_args()
    check(args.draws, args.fixture)
