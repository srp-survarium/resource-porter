#!/usr/bin/env python3
"""Check original sun diffuse BRDF/encoding against frozen original buffers.

Uses the final captured sun visibility, so this does not validate cascade
selection, shadow filtering or raster coverage independently.
"""
import argparse
import json
import math
import struct
from pathlib import Path
from audit_local_light_authored import constants


def normalize(values):
    length = math.sqrt(sum(v * v for v in values))
    return [v / length for v in values] if length else [0, 0, 0]


def audit(directory, stride):
    rows, colors = [], []
    for path in sorted(directory.glob('*.sun-draw.dxbc')):
        digest, cb = constants(path)
        if digest != '757f44b1236addd431785c0cc8407e28':
            raise ValueError('unverified original sun shader: ' + str(path))
        stem = str(path).removesuffix('.sun-draw.dxbc')
        def read(suffix):
            return Path(stem + suffix).read_bytes()
        for phase in ('before', 'output'):
            target = json.loads(read('.sun-' + phase + '.targets.json'))[0]
            if target['format'] != 24 or not target['written']:
                raise ValueError('sun accumulator contract')
        depth, normal, albedo = [read(f'.sun-draw.rt{i:02}.bin') for i in range(3)]
        helper = read('.translucency-draw.rt03.bin')
        helper_view = json.loads(read('.translucency-draw.targets.json'))[3]
        if helper_view['format'] != 34 or not helper_view['written']:
            raise ValueError('original sun visibility contract')
        before, after = read('.sun-before.rt00.bin'), read('.sun-output.rt00.bin')
        if len(depth) != 1280 * 720 * 2 or any(len(b) != 1280 * 720 * 4 for b in (normal, albedo, helper, before, after)):
            raise ValueError('unexpected capture size')
        light = [-x for x in cb['light_direction']]
        ray = cb['s_eye_ray_corner']
        eye = cb.get('eye_position_view_space', (0, 0, 0))
        color = [abs(x) ** 2.2 * cb['light_intensity'][0] * cb['light_diffuse_influence_factor'][0] for x in cb['light_color']]
        _, translucent = constants(Path(stem + '.translucency-draw.dxbc'))
        colors.append(color + [cb['shadow_transparency'][0]] + list(translucent['sun_color']) + [0])
        stats = {key: {'pixels': 0, 'maximum_error': 0, 'sum_error': 0, 'within_one_step': 0} for key in ('changed', 'unchanged')}
        for pixel in range(0, 1280 * 720, stride):
            a = struct.unpack_from('<I', before, pixel * 4)[0]
            b = struct.unpack_from('<I', after, pixel * 4)[0]
            group = stats['changed' if a != b else 'unchanged']
            z = struct.unpack_from('<e', depth, pixel * 2)[0]
            x, y = pixel % 1280, pixel // 1280
            position = [(1 - 2 * (x + .5) / 1280) * ray[0] / ray[2] * z,
                        (1 - 2 * (y + .5) / 720) * ray[1] / ray[2] * z, z]
            packed = struct.unpack_from('<I', normal, pixel * 4)[0]
            f = [(packed & 1023) / 1023 * 4 - 2, ((packed >> 10) & 1023) / 1023 * 4 - 2]
            radius = sum(v * v for v in f)
            n = normalize([f[0] * math.sqrt(abs(1 - radius * .25)), f[1] * math.sqrt(abs(1 - radius * .25)), radius * .5 - 1])
            view = normalize([e - p for e, p in zip(eye, position)])
            halfway = normalize([v + l for v, l in zip(view, light)])
            f0 = (albedo[pixel * 4 + 3] & 127) / 127
            fresnel = f0 + (1 - f0) * (1 - max(sum(v * h for v, h in zip(view, halfway)), 0)) ** 5
            diffuse = min(1, max(0, sum(l * v for l, v in zip(light, n)))) * (1 if fresnel > .99 else 1 - fresnel)
            shadow = struct.unpack_from('<ee', helper, pixel * 4)[1] + cb['shadow_transparency'][0]
            encoded = [math.sqrt(min(1, max(0, c * diffuse * .125))) * shadow for c in color]
            error = max(abs(min(1, ((a >> shift) & 1023) / 1023 + value) - ((b >> shift) & 1023) / 1023) for shift, value in zip((0, 10, 20), encoded))
            group['pixels'] += 1
            group['maximum_error'] = max(group['maximum_error'], error)
            group['sum_error'] += error
            group['within_one_step'] += error <= 1 / 1023
        for group in stats.values():
            group['mean_error'] = group.pop('sum_error') / max(group['pixels'], 1)
        rows.append({'pose': Path(stem).name, 'shader_digest': digest, **stats})
    if not rows or any(color != colors[0] for color in colors):
        raise ValueError('missing original captures or weather constants vary')
    return {'stride': stride, 'scope': 'Original diffuse equation and additive encoding using captured final visibility; unchanged pixels may be outside the observed cascade/stencil coverage.', 'sun_constants': colors[0], 'views': rows}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', type=Path)
    parser.add_argument('--stride', type=int, default=17)
    parser.add_argument('--constants-output', type=Path)
    args = parser.parse_args()
    if args.stride < 1:
        parser.error('stride must be positive')
    result = audit(args.captures, args.stride)
    if args.constants_output:
        args.constants_output.write_bytes(struct.pack('<8f', *result['sun_constants']))
    print(json.dumps(result, indent=2))
