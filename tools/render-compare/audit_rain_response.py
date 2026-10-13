#!/usr/bin/env python3
"""Measure same-frame composition rain response over fixed surface regions."""
import argparse
import json
from pathlib import Path
import struct

from audit_surface_light import REGIONS, packed_rgb


def audit(folder):
    result = {}
    for pose, regions in REGIONS.items():
        base = folder / (pose + '.bmp')
        def read(suffix):
            return Path(str(base) + suffix).read_bytes()
        targets = json.loads(read('.composition-output.targets.json'))
        assert [t['name'] for t in targets] == ['override', 'native_same_inputs', 'comparison_same_inputs']
        assert all(t['format'] == 26 and t['written'] for t in targets)
        assert struct.unpack_from('<f', read('.composition-draw.cb0.bin'), 480)[0] == 1
        width, height = targets[0]['width'], targets[0]['height']
        current, reference = read('.composition-output.rt00.bin'), read('.composition-output.rt02.bin')
        assert len(current) == len(reference) == width * height * 4
        result[pose] = {}
        for name, (x0, y0, x1, y1) in regions.items():
            assert 0 <= x0 < x1 <= width and 0 <= y0 < y1 <= height
            sums = [0., 0.]
            for y in range(y0, y1):
                for x in range(x0, x1):
                    for index, data in enumerate((current, reference)):
                        sums[index] += sum(packed_rgb(data, y * width + x))
            result[pose][name] = {'rectangle': [x0, y0, x1, y1],
                                  'dry_to_rain_rgb_sum_ratio': sums[0] / sums[1]}
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.capture), indent=2))
