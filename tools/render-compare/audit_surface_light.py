#!/usr/bin/env python3
"""Measure fixed surface regions and bound the missing legacy skylight underlay.

The bound assumes the captured first sphere covers the region and subsequent
probes use ordinary alpha blending. It is not a comparison to old GPU buffers.
"""
import argparse
import json
import math
from pathlib import Path
import statistics
import struct

from compare_composition import unsigned_float

REGIONS = {
    'yard-south': {'shadow_wall': [350, 260, 450, 430], 'sunlit_wall': [550, 270, 610, 330]},
    'yard-north': {'wall': [600, 250, 900, 430]},
    'factory-interior': {'back_wall': [980, 180, 1050, 320]},
    'factory-upper': {'forest': [100, 200, 300, 350]},
}


def packed_rgb(data, pixel):
    value = struct.unpack_from('<I', data, pixel * 4)[0]
    return [unsigned_float(value & 2047, 6), unsigned_float((value >> 11) & 2047, 6),
            unsigned_float(value >> 22, 5)]


def stats(values):
    return {'minimum': min(values), 'median': statistics.median(values),
            'mean': statistics.fmean(values), 'maximum': max(values)}


def audit(folder):
    result = {}
    for pose, regions in REGIONS.items():
        base = folder / (pose + '.bmp')
        def read(suffix):
            return Path(str(base) + suffix).read_bytes()
        metadata = {v['slot']: v for v in json.loads(read('.probe-draw.targets.json'))}
        width, height = metadata[2]['width'], metadata[2]['height']
        assert metadata[2]['format'] == 54
        cb0, cb1, cb2 = [read(f'.probe-draw.cb{i}.bin') for i in range(3)]
        ray = struct.unpack_from('<3f', cb0, 7 * 16)
        center = struct.unpack_from('<3f', cb0, 11 * 16)
        radius = struct.unpack_from('<f', cb2, 16)[0] / .98
        matrix = [struct.unpack_from('<4f', cb1, (11 + i) * 16) for i in range(3)]
        blend = json.loads(read('.probe-draw.blend.json'))
        assert blend == {'enabled': 1, 'source': 5, 'destination': 6, 'operation': 1}
        depth = read('.probe-draw.rt02.bin')
        encoded = read('.before-hba.rt31.bin')
        ambient = read('.after-hba.rt31.bin')
        ao = read('.hba-draw.rt00.bin')
        albedo = read('.composition-draw.rt01.bin')
        pose_result = {'global_probe_center': center, 'source_radius': radius, 'regions': {}}
        for name, rect in regions.items():
            x0, y0, x1, y1 = rect
            assert 0 <= x0 < x1 <= width and 0 <= y0 < y1 <= height
            samples = {k: [] for k in ['distance', 'world_height', 'underlay_retention',
                'ambient_rgb_sum', 'ao', 'albedo_linear_rgb_sum', 'skylight_added_rgb_sum_bound']}
            for y in range(y0, y1):
                for x in range(x0, x1):
                    i = y * width + x
                    z = struct.unpack_from('<e', depth, i * 2)[0]
                    view = [(1 - 2 * (x + .5) / width) * ray[0] / ray[2] * z,
                            (1 - 2 * (y + .5) / height) * ray[1] / ray[2] * z, z, 1]
                    world = [sum(a * b for a, b in zip(row, view)) for row in matrix]
                    distance = math.dist(center, world)
                    retention = -math.expm1(-8 * (distance / (.95 * radius)) ** 8)
                    # Old sky cube colors <= 1, multiplier .5, wrap color zero.
                    # Its R8 AO is at most 1 and is multiplied by 2.5 in the PS.
                    ramp = min(max(world[1] / 5, 0), 1) ** .1
                    delta = math.sqrt(.5 * 2.5 * ramp / 8) * retention
                    e = packed_rgb(encoded, i)
                    samples['distance'].append(distance)
                    samples['world_height'].append(world[1])
                    samples['underlay_retention'].append(retention)
                    samples['ambient_rgb_sum'].append(sum(packed_rgb(ambient, i)))
                    samples['ao'].append(ao[i] / 255)
                    samples['albedo_linear_rgb_sum'].append(sum(((v / 255 + .055) / 1.055) ** 2.4
                                                               for v in albedo[i * 4:i * 4 + 3]))
                    samples['skylight_added_rgb_sum_bound'].append(
                        sum(8 * ((v + delta) ** 2 - v * v) * ao[i] / 255 for v in e))
            info = {'rectangle': rect, 'pixels': (x1 - x0) * (y1 - y0),
                    **{k: stats(v) for k, v in samples.items()}}
            info['mean_added_ambient_fraction_bound'] = (
                info['skylight_added_rgb_sum_bound']['mean'] / info['ambient_rgb_sum']['mean'])
            info['all_samples_inside_global_sphere'] = max(samples['distance']) < radius
            if not info['all_samples_inside_global_sphere']:
                info['mean_added_ambient_fraction_bound'] = None
            pose_result['regions'][name] = info
        result[pose] = pose_result
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.capture), indent=2))
