#!/usr/bin/env python3
"""Check original SSAO packet boundaries from three captured passes (CPU only)."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile

from replay_ssao_011e import prepare, PASSES


def check(frame, fixture):
    results = []
    for phase in PASSES:
        with tempfile.TemporaryDirectory(prefix='check-original-ssao-') as tmp:
            root = Path(tmp)
            for path in frame.parent.glob(frame.name + '.' + phase + '-*'):
                if path.is_file():
                    shutil.copy2(path, root / path.name)
            draw = root / (frame.name + '.' + phase + '-draw')
            output = root / 'valid'
            receipt = prepare(draw, fixture, output)
            assert receipt['phase'] == phase and (output / 'packet.bin').read_bytes()[:8] == b'SSA011E1'
            rejected = []

            def rejects(name, target=None):
                target = target or root / name
                try:
                    prepare(draw, fixture, target)
                except (ValueError, FileNotFoundError):
                    assert target == output or not target.exists()
                    rejected.append(name)
                else:
                    raise AssertionError(name + ' accepted')

            def mutate(suffix, name, transform):
                path = Path(str(draw) + suffix)
                original = path.read_bytes()
                path.write_bytes(transform(original))
                try:
                    rejects(name)
                finally:
                    path.write_bytes(original)

            rejects('overwrite', output)
            mutate('.dxbc', 'wrong-pixel-shader', lambda b: b[:-1] + bytes([b[-1] ^ 1]))
            mutate('.vs.dxbc', 'wrong-vertex-shader', lambda b: b[:-1] + bytes([b[-1] ^ 1]))
            mutate('.cb0.bin', 'short-constants', lambda b: b[:1])
            mutate('.indices.bin', 'short-indices', lambda b: b[:1])
            mutate('.rt00.bin', 'short-depth', lambda b: b[:-2])
            mutate('.targets.json', 'missing-input', lambda b: json.dumps(json.loads(b)[1:]).encode())
            mutate('.post-state.json', 'wrong-viewport', lambda b: json.dumps(dict(json.loads(b), viewport=[0, 0, 1, 1, 0, 1])).encode())
            mutate('.post-state.json', 'blended-output', lambda b: json.dumps(dict(json.loads(b), blend_enabled=True)).encode())
            if phase == 'ssao-temporal':
                mutate('.depth-state.json', 'wrong-stencil-reference', lambda b: json.dumps(dict(json.loads(b), reference=1)).encode())
                mutate('.depth.rt00.bin', 'short-stencil', lambda b: b[:-4])
                mutate('.rt02.bin', 'short-history', lambda b: b[:-2])
                path = Path(str(draw).removesuffix('-draw') + '-output.rt01.bin')
                old = path.read_bytes()
                path.write_bytes(old[:-2])
                try:
                    rejects('short-secondary-output')
                finally:
                    path.write_bytes(old)
            results.append(dict(phase=phase, rejected=rejected))
    print(json.dumps(dict(preparation='passed', passes=results, scope='CPU boundaries, not GPU or temporal-sequence parity'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('frame', type=Path, help='captured .bmp base')
    parser.add_argument('fixture', type=Path)
    args = parser.parse_args()
    check(args.frame, args.fixture)
