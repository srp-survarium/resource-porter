#!/usr/bin/env python3
"""Decode captured 0.10b rain layers using the archived shader contracts."""
import argparse
import hashlib
import json
import struct
from pathlib import Path
from verify_rain_capture import verify

RAIN_PS = '9004247427fb3a6a9e5dd19aea0c46d5'


def audit(report):
    coverage = verify(report)
    if coverage['observed_draws'][1:] != [0, 0]:
        raise ValueError('reference contains an unexpected particle/resolve draw')
    prefix = str(report).removesuffix('.rain-counts.json')
    layers = []
    for index in range(coverage['observed_draws'][0]):
        base = prefix + '.rain-draw' + (f'-{index}' if index else '')
        shader = Path(base + '.dxbc').read_bytes()
        if shader[:4] != b'DXBC' or shader[4:20].hex() != RAIN_PS:
            raise ValueError('unexpected rain shader; constant offsets are version-specific')
        pixel = Path(base + '.cb0.bin').read_bytes()
        vertex = Path(base + '.vs-cb0.bin').read_bytes()
        uv_u, uv_v, radius, density, speed = struct.unpack_from('<5f', pixel, 16)
        layers.append({
            'index': index, 'uv_scale': [uv_u, uv_v], 'radius': radius,
            'density': density, 'speed': speed,
            'view_to_shadow_rows': [list(struct.unpack_from('<4f', pixel, 48 + row * 16)) for row in range(4)],
            'world_rows': [list(struct.unpack_from('<4f', vertex, 112 + row * 16)) for row in range(3)],
            'geometry': json.loads(Path(base + '.geometry.json').read_text()),
            'geometry_sha256': {kind: hashlib.sha256(Path(base + '.' + kind + '.bin').read_bytes()).hexdigest()
                                for kind in ['vertices', 'indices']},
        })
    return {'report': str(report), 'coverage': coverage, 'layers': layers,
            'scope': 'Live reference draw constants and geometry; does not establish target parity.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports', nargs='+', type=Path)
    args = parser.parse_args()
    print(json.dumps([audit(report) for report in args.reports], indent=2))
