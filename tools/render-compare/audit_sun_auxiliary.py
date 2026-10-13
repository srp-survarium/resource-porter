#!/usr/bin/env python3
"""Check private source sun addition on native G-buffer, visibility and stencil.

This verifies the adapter equation and native coverage, not original shadow or
geometry parity. It does not treat absent source translucency as complete.
"""
import argparse
from collections import Counter
import json
import math
from pathlib import Path
import struct
from audit_local_light_authored import constants


def normalize(values):
    size = math.sqrt(sum(v * v for v in values))
    return [v / size for v in values] if size else [0, 0, 0]


def audit(directory, stride):
    rows = []
    for report in sorted(directory.glob('*.sun-encoded.json')):
        state = json.loads(report.read_text())
        if not state['ready'] or state['draws'] not in (1, 2) or len({state[k] for k in ('present', 'native_depth_present', 'projection_present')}) != 1:
            raise ValueError('sun helper frame contract: ' + str(report))
        stem = str(report).removesuffix('.sun-encoded.json')
        def read(suffix):
            return Path(stem + suffix).read_bytes()
        local = json.loads(read('.local-encoded.json'))
        if local['present'] != state['present'] or not local['ambient_valid'] or local['failed_draws'] or local['unhandled_draws']:
            raise ValueError('source ambient/local frame contract')
        before, after = read('.sun-encoded-0.rt00.bin'), read('.sun-encoded-1.rt00.bin')
        if before != read('.local-encoded.rt00.bin'):
            raise ValueError('sun input is not captured ambient+local accumulator')
        depth_stencil = read('.sun-encoded-0.rt01.bin')
        views = json.loads(read('.sun-encoded-0.targets.json'))
        if views[0]['format'] != 24 or any(not v['written'] for v in views):
            raise ValueError('sun readback incomplete')
        fmt = views[1]['format']
        if fmt in (44, 45):
            stencil = depth_stencil[3::4]
        elif fmt in (19, 20):
            stencil = depth_stencil[4::8]
        else:
            raise ValueError('unsupported depth/stencil format: ' + str(fmt))
        _, cb = constants(Path(stem + '.composition-draw.dxbc'))
        inputs = json.loads(read('.composition-draw.targets.json'))
        for slot, fmt in ((0, 34), (2, 28), (3, 54), (6, 49)):
            view = next(v for v in inputs if v['slot'] == slot)
            if view['format'] != fmt or not view['written']:
                raise ValueError('composition input contract at slot ' + str(slot))
        normal, parameters, depth, shadow = [read(f'.composition-draw.rt{i:02}.bin') for i in (0, 2, 3, 6)]
        if len(before) != 1280 * 720 * 4 or len(after) != len(before) or len(stencil) != 1280 * 720:
            raise ValueError('unexpected output dimensions')
        sun = state['source_constants']; ray = cb['s_eye_ray_corner']; light = [-v for v in cb['light_direction']]
        groups = {k: {'samples': 0, 'maximum_error': 0, 'sum_error': 0, 'over_one_step': 0, 'examples': []} for k in ('changed', 'unchanged')}
        for pixel in range(0, len(stencil), stride):
            a, b = struct.unpack_from('<I', before, pixel * 4)[0], struct.unpack_from('<I', after, pixel * 4)[0]
            if a >> 30 != b >> 30:
                raise ValueError('sun changed unwritable alpha channel')
            encoded = [0, 0, 0]
            if stencil[pixel]:
                x, y = pixel % 1280, pixel // 1280
                z = struct.unpack_from('<e', depth, pixel * 2)[0]
                position = [(1 - 2 * (x + .5) / 1280) * ray[0] / ray[2] * z, (1 - 2 * (y + .5) / 720) * ray[1] / ray[2] * z, z]
                f = [v * 4 - 2 for v in struct.unpack_from('<ee', normal, pixel * 4)]; radius = sum(v * v for v in f)
                n = normalize([f[0] * math.sqrt(abs(1 - radius * .25)), f[1] * math.sqrt(abs(1 - radius * .25)), radius * .5 - 1])
                view = normalize([-v for v in position]); halfway = normalize([a + b for a, b in zip(light, view)])
                f0 = int(parameters[pixel * 4] / 255 * 127) / 127
                fresnel = f0 + (1 - f0) * (1 - max(sum(a * b for a, b in zip(view, halfway)), 0)) ** 5
                diffuse = min(1, max(0, sum(a * b for a, b in zip(light, n)))) * (1 if fresnel > .99 else 1 - fresnel)
                visibility = shadow[pixel * 2] / 255
                encoded = [math.sqrt(min(1, max(0, c * diffuse * .125))) * (visibility + sun[3]) for c in sun[:3]]
            error = max(abs(min(1, ((a >> shift) & 1023) / 1023 + value) - ((b >> shift) & 1023) / 1023) for shift, value in zip((0, 10, 20), encoded))
            group = groups['changed' if a != b else 'unchanged']; group['samples'] += 1
            group['maximum_error'] = max(group['maximum_error'], error); group['sum_error'] += error
            if error > 1 / 1023:
                group['over_one_step'] += 1
                if len(group['examples']) < 5:
                    group['examples'].append({'pixel': pixel, 'stencil': stencil[pixel], 'error': error})
        for group in groups.values():
            group['mean_error'] = group.pop('sum_error') / max(1, group['samples'])
        rows.append({'pose': Path(stem).name, 'present': state['present'], 'stencil_histogram': dict(Counter(stencil)), **groups})
    if not rows:
        raise ValueError('no sun auxiliary captures')
    return {'stride': stride, 'scope': __doc__.strip(), 'views': rows}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', type=Path)
    parser.add_argument('--stride', type=int, default=17)
    args = parser.parse_args()
    if args.stride < 1:
        parser.error('stride must be positive')
    print(json.dumps(audit(args.captures, args.stride), indent=2))
