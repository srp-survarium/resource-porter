#!/usr/bin/env python3
"""Replay original translucency math on saved inputs to identify helper channels."""
from pathlib import Path
import argparse
import hashlib
import json
import math
import struct
from audit_fog_capture import constant


parser = argparse.ArgumentParser(description='Compare both helper-channel hypotheses against captured original translucency draws. No stencil readback: rejected fragments can remain outliers.')
parser.add_argument('capture', type=Path)
parser.add_argument('--stride', type=int, default=13)
args = parser.parse_args()
if args.stride < 1:
    parser.error('stride must be positive')
root = args.capture
out = {}
for image in sorted(root.glob('*.bmp')):
    name = image.stem
    b = root / (name + '.bmp')
    stage = Path(str(b) + '.translucency-draw')
    if not Path(str(stage) + '.dxbc').exists():
        continue

    def read(s):
        return Path(str(b) + s).read_bytes()
    dep = read('.translucency-draw.rt00.bin')
    norm = read('.translucency-draw.rt01.bin')
    albedo = read('.translucency-draw.rt02.bin')
    helper = read('.translucency-draw.rt03.bin')
    before = read('.translucency-before.rt00.bin')
    after = read('.translucency-output.rt00.bin')
    digest = hashlib.sha256(Path(str(stage) + '.dxbc').read_bytes()).hexdigest()
    assert digest == '3baca72d9f65014b2d1d64c78a951280f0e4156694e5c79d2e7ad3b5aa71ce6e', 'unverified original shader'
    for suffix in ['before', 'output']:
        target = json.loads(read('.translucency-' + suffix + '.targets.json'))[0]
        assert target['format'] == 24 and target['written']
    metadata = json.loads(read('.translucency-draw.targets.json'))
    assert [(v['slot'], v['format'], v.get('written')) for v in metadata[:4]] == [(0, 54, True), (1, 24, True), (2, 28, True), (3, 34, True)]
    assert len(dep) == 1280 * 720 * 2 and all((len(v) == 1280 * 720 * 4 for v in [norm, albedo, helper, before, after]))
    matrices = [constant(stage, 'm_shadow' + str(i)) for i in range(4)]
    ray = constant(stage, 's_eye_ray_corner')
    sun = constant(stage, 'sun_direction')
    color = constant(stage, 'sun_color')
    stats = [{'n': 0, 'sum_abs_error': 0, 'max_abs_error': 0, 'within_one_step': 0, 'attenuation_sum': 0, 'outliers': []} for _ in range(2)]
    for i in range(0, 1280 * 720, args.stride):
        packed = struct.unpack_from('<I', norm, i * 4)[0]
        translucent = albedo[i * 4 + 3] >> 7
        if not translucent:
            continue
        tr = ((packed >> 20 & 1023) >> 7 | packed >> 30 << 3) / 31
        tr = min(tr, 1) ** 2.2
        if tr == 0:
            continue
        x, y = (i % 1280, i // 1280)
        z = struct.unpack_from('<e', dep, i * 2)[0]
        position = [(1 - 2 * (x + 0.5) / 1280) * ray[0] / ray[2] * z, (1 - 2 * (y + 0.5) / 720) * ray[1] / ray[2] * z, z, 1]
        shadowdepth = 0
        for j, m in enumerate(matrices):
            q = [sum((m[k * 4 + n] * position[n] for n in range(4))) for k in range(4)]
            tc = [q[n] / q[3] for n in range(3)]
            if all((-0.01 <= v <= 1.01 for v in tc[:2 if j == 3 else 3])):
                shadowdepth = tc[2]
                break
        f = [(packed & 1023) / 1023 * 4 - 2, (packed >> 10 & 1023) / 1023 * 4 - 2]
        r = sum((v * v for v in f))
        normal = [f[0] * math.sqrt(max(0, 1 - r * 0.25)), f[1] * math.sqrt(max(0, 1 - r * 0.25)), r * 0.5 - 1]
        length = math.sqrt(sum((v * v for v in normal)))
        normal = [v / length for v in normal]
        back = sum((a * c for a, c in zip(normal, sun)))
        length = math.sqrt(sum((v * v for v in position[:3])))
        forward = max(0, min(1, -sum((a * c for a, c in zip(position, sun))) / length))
        h = struct.unpack_from('<ee', helper, i * 4)
        a = struct.unpack_from('<I', before, i * 4)[0]
        bvalue = struct.unpack_from('<I', after, i * 4)[0]
        for slot, st in enumerate(stats):
            att = (1 - max(0, min(1, (shadowdepth - h[slot]) / (tr * 0.035)))) ** 10
            result = [tr * 0.1 + c * back * att * tr * 1.5 + tr * back * 0.1 * c + c * forward ** 3 * tr * att * 2 for c in color]
            encoded = [math.sqrt(max(0, min(1, v * 0.125))) for v in result]
            errors = [abs(min(1, (a >> shift & 1023) / 1023 + v) - (bvalue >> shift & 1023) / 1023) for shift, v in zip([0, 10, 20], encoded)]
            if slot == 1 and max(errors) > 1 / 1023:
                st['outliers'].append({'xy': [x, y], 'error': errors, 'before': a, 'after': bvalue, 'translucency': tr, 'attenuation': att, 'projected_depth': shadowdepth, 'helper': h})
            st['n'] += 1
            st['sum_abs_error'] += sum(errors) / 3
            st['max_abs_error'] = max(st['max_abs_error'], max(errors))
            st['within_one_step'] += max(errors) <= 1 / 1023
            st['attenuation_sum'] += att
    for st in stats:
        if st['n']:
            st['mean_abs_error'] = st.pop('sum_abs_error') / st['n']
            st['attenuation_mean'] = st.pop('attenuation_sum') / st['n']
    out[name] = {'stride': args.stride, 'shader_sha256': digest, 'helper_red_raw_depth': stats[0], 'helper_green_visibility': stats[1]}
if not out:
    raise SystemExit('no original translucency captures found')
print(json.dumps(out, indent=2))
