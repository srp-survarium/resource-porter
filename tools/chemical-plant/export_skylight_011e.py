#!/usr/bin/env python3
"""Export the audited symmetric 011e Chemical/Radar sky constant layout.

This deliberately rejects other skies: a representative horizontal sun direction
is only equivalent when the four horizontal cube faces match and wrap light is
zero. The binary is for the opt-in source ambient replay, not a map edit.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def export(material):
    p = material['material']['post_process']
    def value(name):
        return p[name]['value']
    horizontal = [value('environment_skycolor_'+name)[:3]
                  for name in ('x', 'minus_x', 'z', 'minus_z')]
    if any(v != horizontal[0] for v in horizontal):
        raise ValueError('asymmetric sky needs the actual source sun direction')
    if any(value('environment_suncolor')[:3]):
        raise ValueError('nonzero wrap light needs the actual source sun inputs')
    color = [v**2.2*value('skylight_multiplier')
             for v in value('skylight_color')[:3]]
    h = horizontal[0]
    up, down = value('environment_skycolor_up')[:3], value('environment_skycolor_down')[:3]
    vectors = [color+[0.], [value('skylight_lower'), value('skylight_upper')-value('skylight_lower'),
                           value('skylight_power'), 0.],
               h+[h[0]], h+[h[1]], up+[h[2]], h+[down[0]],
               [0., 0., 1., down[1]], [0., 0., 0., down[2]]]
    if vectors[1][1] <= 0 or vectors[1][2] < 0:
        raise ValueError('unsupported height interval/power')
    return vectors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('material', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    raw = args.material.read_bytes()
    material = json.loads(raw)
    if 'inputs' in material:
        material = material['inputs']['new']['post']
    vectors = export(material)
    data = struct.pack('<32f', *(v for row in vectors for v in row))
    args.output.write_bytes(data)
    args.output.with_suffix('.json').write_text(json.dumps({
        'material': str(args.material), 'material_sha256': hashlib.sha256(raw).hexdigest(),
        'constants_sha256': hashlib.sha256(data).hexdigest(), 'vectors': vectors,
        'contract': '011e skylight DXBC 21c28f59ef738c2db31065650ff8249f; '
                    'stage_ambient_lighting::execute VA625d41..6261b4 uploads source layout; '
                    'ambient multiplier1. Symmetric horizontal faces and zero wrap make azimuth irrelevant.',
        'scope': 'Initial sky only; probe attenuation, SSAO, local lights and final composition are separate.'
    }, indent=2)+'\n')


if __name__ == '__main__':
    main()
