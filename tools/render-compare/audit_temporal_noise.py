#!/usr/bin/env python3
"""Measure stationary-frame variation in explicit image regions; not a noise-source classifier."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics

from compare_weather import bmp


def camera(path):
    record = json.loads(path.with_suffix('.json').read_text())
    pose, status = record['pose'], record['status']
    for field in ('position', 'forward'):
        if len(pose[field]) != 3 or len(status[field]) != 3:
            raise ValueError(f'invalid {field}: {path}')
        if any(not math.isfinite(a) or abs(a-b) > .001
               for a, b in zip(status[field], pose[field])):
            raise ValueError(f'camera {field} differs from request: {path}')
    if abs(status['fov']-pose.get('fov', 65)) > .001:
        raise ValueError(f'camera fov differs from request: {path}')
    return pose['position'] + pose['forward'] + [pose.get(k, default) for k, default in
           [('fov', 65), ('aspect', 1280/720), ('near', .05), ('far', 500)]]


def audit(files, regions, step):
    if len(files) < 3 or len(set(files)) != len(files) or step < 1:
        raise ValueError('need at least three distinct frames and a positive grid step')
    reference = camera(files[0])
    frames = []
    provenance = []
    for path in files:
        actual = camera(path)
        if any(not math.isfinite(a) or abs(a-b) > .001 for a, b in zip(actual, reference)):
            raise ValueError(f'camera changed during stationary sequence: {path}')
        frames.append(bmp(path))
        provenance.append({'file': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
    results = {}
    for name, bounds in regions.items():
        x0, y0, x1, y1 = bounds
        if not (1 <= x0 < x1 <= 1279 and 1 <= y0 < y1 <= 719):
            raise ValueError('regions must exclude the outermost pixel border')
        samples, spatial = [], []
        for pixel in frames:
            def luma(x, y):
                return sum(c*w for c, w in zip(pixel(x, y), (.2126, .7152, .0722)))
            values, highpass = [], []
            for y in range(y0, y1, step):
                for x in range(x0, x1, step):
                    value = luma(x, y)
                    values.append(value)
                    highpass.append(value - (luma(x-1, y)+luma(x+1, y)+luma(x, y-1)+luma(x, y+1))/4)
            samples.append(values)
            spatial.append(math.sqrt(statistics.mean(v*v for v in highpass)))
        means = list(map(statistics.mean, samples))
        centered = [[v-mean for v in row] for row, mean in zip(samples, means)]
        temporal = [statistics.pvariance(values) for values in zip(*samples)]
        residual = [statistics.pvariance(values) for values in zip(*centered)]
        results[name] = {'bounds_xyxy': bounds, 'sample_count': len(samples[0]),
                         'frame_luma_means': means, 'frame_spatial_highpass_rms': spatial,
                         'temporal_luma_rms': math.sqrt(statistics.mean(temporal)),
                         'temporal_luma_rms_after_frame_mean_removal': math.sqrt(statistics.mean(residual))}
    return {'scope': 'Fixed-camera display luma. Regions are supplied by the caller, not inferred depth. '
                     'Rain, foliage, clouds, lighting, grain and capture timing can all contribute. '
                     'Frame-mean removal separates uniform brightness changes, not their causes.',
            'frames': provenance, 'regions': results}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('frames', nargs='+', type=Path)
    parser.add_argument('--region', action='append', help='NAME:X0,Y0,X1,Y1 (exclusive upper bounds)')
    parser.add_argument('--step', type=int, default=4)
    args = parser.parse_args()
    regions = {}
    for spec in args.region or ['image:1,1,1279,719']:
        name, bounds = spec.split(':', 1)
        if not name or name in regions:
            parser.error('region names must be nonempty and unique')
        regions[name] = list(map(int, bounds.split(',')))
    print(json.dumps(audit(args.frames, regions, args.step), indent=2))
