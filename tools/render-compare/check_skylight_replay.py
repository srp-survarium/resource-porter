#!/usr/bin/env python3
"""Check skylight packet boundaries using one original captured draw (CPU only)."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile

from replay_skylight import prepare


def check(draw, vertex_shader):
    with tempfile.TemporaryDirectory(prefix='check-skylight-') as tmp:
        root = Path(tmp)
        stem = str(draw).removesuffix('.skylight-draw')
        for path in draw.parent.glob(Path(stem).name + '.skylight-*'):
            if path.is_file():
                shutil.copy2(path, root / path.name)
        copied = root / draw.name
        output = root / 'valid'
        result = prepare(copied, vertex_shader, output)
        assert result['format'] == 24 and (output / 'packet.bin').read_bytes()[:8] == b'SKY011E2'
        rejected = []

        def rejects(name, source=copied, target=None):
            target = target or root / name
            try:
                prepare(source, vertex_shader, target)
            except (ValueError, FileNotFoundError):
                assert target == output or not target.exists()
                rejected.append(name)
            else:
                raise AssertionError(name + ' accepted')

        rejects('overwrite', target=output)
        rejects('wrong-stage', source=root / 'unknown.post-draw')

        def mutate(suffix, name, transform):
            path = Path(str(copied) + suffix)
            original = path.read_bytes()
            path.write_bytes(transform(original))
            try:
                rejects(name)
            finally:
                path.write_bytes(original)

        mutate('.dxbc', 'wrong-shader', lambda b: b[:-1] + bytes([b[-1] ^ 1]))
        mutate('.cb0.bin', 'truncated-constants', lambda b: b[:-16])
        mutate('.cb0.bin', 'nonfinite-weather', lambda b: bytes.fromhex('0000c07f') + b[4:])
        mutate('.rt01.bin', 'truncated-normals', lambda b: b[:-4])
        mutate('.blend.json', 'blended-pass', lambda b: json.dumps(dict(json.loads(b), enabled=1)).encode())

        def texture_change(key, value):
            def changed(data):
                rows = json.loads(data)
                rows[1][key] = value
                return json.dumps(rows).encode()
            return changed

        mutate('.targets.json', 'native-normal-format', texture_change('format', 34))
        mutate('.targets.json', 'uncaptured-input', texture_change('written', False))
        mutate('.targets.json', 'unavailable-mips', texture_change('mip_levels', 2))
        mutate('.depth-state.json', 'changed-stencil-test', lambda b: json.dumps(dict(json.loads(b), reference=1)).encode())
        mutate('.depth-state.bin', 'inconsistent-depth-state', lambda b: bytes([b[0] ^ 1]) + b[1:])
        mutate('.depth.rt00.bin', 'truncated-stencil-surface', lambda b: b[:-4])
        mutate('.indices.bin', 'truncated-geometry', lambda b: b[:1])
        print(json.dumps(dict(preparation='passed', rejected=rejected,
                              scope='CPU preparation checks; GPU comparison is separate'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('draw', type=Path)
    parser.add_argument('vertex_shader', type=Path)
    args = parser.parse_args()
    check(args.draw, args.vertex_shader)
