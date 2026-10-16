#!/usr/bin/env python3
"""Offline NaN audit of a built 0.26g map bundle.

    verify_finite_bundle.py --bundle BUNDLE [--vostok-port VP] [--json OUT]

Per map (the model or project directory under ported/v010b), counts:
- static vertices with a non-finite position, an Inf/NaN half UV, or a tangent
  frame 0.26's g-buffer cannot use (a packed vector shorter than 0.5, or two
  of normal/tangent/binormal within 8.1 degrees of parallel), and triangles of
  zero area;
- non-finite floats in every material, project, model settings and options
  config (decoded with `vostok-port dump-config --leaves`);
- environment values a 0.26 shader turns into NaN: an empty height-ambient
  band or power <= 0 with height ambient on, mie_fog_sun_color_falloff <= 0,
  fog far distances <= 0 (see early::environment_hazards).
Exits 1 when anything is found. Needs numpy. Reads only the bundle.
"""
import argparse
import collections
import glob
import json
import math
import os
import struct
import subprocess
import sys

import numpy as np

BASIS_MIN_LENGTH = 0.5
BASIS_PARALLEL = 0.99


def map_key(path):
    parts = path.split('/')
    if 'v010b' in parts:
        rest = parts[parts.index('v010b') + 1:]
        if rest[0] == 'ported' and len(rest) > 2:
            return rest[2]
        return rest[0].split('.')[0]
    if 'projects' in parts:
        return parts[parts.index('projects') + 1]
    return parts[0]


def audit_geometry(path):
    b = open(path, 'rb').read()
    cid, size, count = struct.unpack_from('<III', b, 0)
    if cid != 3 or count == 0:
        raise SystemExit(f'{path}: not a static vertex chunk')
    stride = (size - 4) // count
    if stride not in (28, 32) or 4 + stride * count != size:
        raise SystemExit(f'{path}: unsupported stride')
    v = np.frombuffer(b, dtype=np.uint8, count=count * stride, offset=12).reshape(count, stride)
    pos = v[:, 0:12].copy().view('<f4').reshape(count, 3)
    uv_bits = v[:, 24:28].copy().view('<u2').reshape(count, 2)

    def basis(o):
        return np.stack([v[:, o + 2], v[:, o + 1], v[:, o]], 1).astype(np.float32) / 127.5 - 1

    frame = [basis(o) for o in (12, 16, 20)]
    lengths = [np.linalg.norm(x, axis=1) for x in frame]
    short = np.any([l < BASIS_MIN_LENGTH for l in lengths], axis=0)
    unit = [x / np.maximum(l, 1e-9)[:, None] for x, l in zip(frame, lengths)]
    parallel = np.zeros(count, bool)
    for a, c in ((0, 1), (0, 2), (1, 2)):
        parallel |= np.abs((unit[a] * unit[c]).sum(1)) > BASIS_PARALLEL
    out = collections.Counter(
        vertices=count,
        nonfinite_position=int((~np.isfinite(pos)).any(1).sum()),
        nonfinite_uv=int(((uv_bits & 0x7c00) == 0x7c00).any(1).sum()),
        degenerate_frame=int((short | parallel).sum()),
    )
    o = 8 + size
    if o + 12 <= len(b):
        cid, _, n = struct.unpack_from('<III', b, o)
        if cid == 4:
            idx = np.frombuffer(b, dtype='<u2', count=n, offset=o + 12).reshape(-1, 3).astype(np.int64)
            p = pos[idx]
            area = np.linalg.norm(np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]), axis=1)
            out['zero_area_triangles'] = int((area < 1e-10).sum())
    return out


def leaves(vp, path):
    text = subprocess.run([vp, 'dump-config', '--file', path, '--leaves'], check=True,
                          capture_output=True, text=True).stdout
    for line in text.splitlines():
        key, _, value = line.partition('\t')
        yield key, value


def floats(value):
    kind, _, raw = value.partition(':')
    if kind == 'f32':
        return [float(raw)]
    if kind.startswith('float'):
        return [float(x) for x in raw.strip('[]').split(',')]
    return []


def number(value):
    kind, _, raw = value.partition(':')
    return float(raw) if kind in ('f32', 'i32') else None


def environment_hazards(props):
    hazards = []
    if props.get('use_height_based_ambient') == 'bool:true':
        low = number(props.get('height_based_ambient_low_limit', ''))
        high = number(props.get('height_based_ambient_high_limit', ''))
        if low is None or high is None or not high > low:
            hazards.append(f'height ambient band {low}..{high} is empty')
        power = number(props.get('height_based_ambient_power', ''))
        if power is None or not power > 0:
            hazards.append('height_based_ambient_power <= 0')
    falloff = number(props.get('mie_fog_sun_color_falloff', ''))
    if falloff is not None and falloff <= 0:
        hazards.append('mie_fog_sun_color_falloff <= 0')
    for key in ('mie_fog_far_distance', 'rayleigh_fog_far_distance'):
        d = number(props.get(key, ''))
        if d is not None and d <= 0:
            hazards.append(f'{key} <= 0')
    return hazards


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', required=True)
    ap.add_argument('--vostok-port', default='target/release/vostok-port')
    ap.add_argument('--json')
    a = ap.parse_args()
    overlay = os.path.join(a.bundle, 'overlay')
    per = collections.defaultdict(collections.Counter)
    findings = []
    for path in sorted(glob.glob(f'{overlay}/**/converted_model', recursive=True)):
        rel = os.path.relpath(path, overlay)
        c = audit_geometry(path)
        per[map_key(rel)].update(c)
        bad = {k: v for k, v in c.items() if k != 'vertices' and v}
        if bad:
            findings.append({'resource': rel, **bad})
    configs = [p for p in glob.glob(f'{overlay}/**/*', recursive=True) if os.path.isfile(p) and (
        p.endswith(('.material', '.options', '/client_project', '/server_project', '/settings')))]
    for path in sorted(configs):
        rel = os.path.relpath(path, overlay)
        key = map_key(rel)
        per[key]['configs'] += 1
        env = collections.defaultdict(dict)
        bad = []
        for name, value in leaves(a.vostok_port, path):
            if any(not math.isfinite(x) for x in floats(value)):
                bad.append(name)
            if '.properties.' in name and name.startswith('client_objects['):
                obj, _, prop = name.partition('.properties.')
                env[obj][prop] = value
        if bad:
            per[key]['nonfinite_config_values'] += len(bad)
            findings.append({'resource': rel, 'nonfinite': bad[:20]})
        for obj, props in env.items():
            if 'use_height_based_ambient' not in props and 'mie_fog_far_distance' not in props:
                continue
            hazards = environment_hazards(props)
            if hazards:
                per[key]['environment_hazards'] += len(hazards)
                findings.append({'resource': rel, 'object': obj, 'hazards': hazards})
    keys = ['vertices', 'nonfinite_position', 'nonfinite_uv', 'degenerate_frame', 'zero_area_triangles',
            'configs', 'nonfinite_config_values', 'environment_hazards']
    print('map'.ljust(28) + ''.join(k.rjust(len(k) + 2) for k in keys))
    for m in sorted(per):
        print(m.ljust(28) + ''.join(str(per[m][k]).rjust(len(k) + 2) for k in keys))
    defects = sum(per[m][k] for m in per for k in keys[1:] if k not in ('configs', 'zero_area_triangles'))
    if a.json:
        with open(a.json, 'w') as f:
            json.dump({'maps': per, 'findings': findings}, f, indent=1)
    print(f'{defects} defects ({len(findings)} resources)')
    return 1 if defects else 0


if __name__ == '__main__':
    sys.exit(main())
