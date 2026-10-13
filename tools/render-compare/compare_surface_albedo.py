#!/usr/bin/env python3
"""Compare regional albedo and pre-fog composition from the two captured schemas."""
import argparse
import json
from pathlib import Path
from audit_surface_light import REGIONS, packed_rgb


def compare(original, current):
    result = {'scope': 'Separate-frame regional means, not pixel or perceptual parity. Albedo uses each captured composition shader\'s decoding equation.', 'views': {}}
    for pose, regions in REGIONS.items():
        old, new = original / (pose + '.bmp'), current / (pose + '.bmp')

        def read(base, suffix):
            return Path(str(base) + suffix).read_bytes()

        for base, slot in [(old, 2), (new, 1)]:
            metadata = {item['slot']: item for item in json.loads(read(base, '.composition-draw.targets.json'))}
            assert metadata[slot]['format'] == metadata[slot]['view_format'] == 28
            assert metadata[slot]['width'] == 1280 and metadata[slot]['height'] == 720
            target = json.loads(read(base, '.composition-output.targets.json'))[0]
            assert target['format'] == 26
        a, b = read(old, '.composition-draw.rt02.bin'), read(new, '.composition-draw.rt01.bin')
        oc, nc = read(old, '.composition-output.rt00.bin'), read(new, '.composition-output.rt00.bin')
        old_ao = read(old, '.ssao-temporal-output.rt00.bin')
        new_ao = read(new, '.hba-draw.rt00.bin')
        assert len(a) == len(b) == len(oc) == len(nc) == 1280 * 720 * 4
        assert len(old_ao) == len(new_ao) == 1280 * 720 * 2
        view = {}
        for name, (x0, y0, x1, y1) in regions.items():
            indices = [y * 1280 + x for y in range(y0, y1) for x in range(x0, x1)]

            def average(function):
                totals = [0., 0., 0.]
                for i in indices:
                    for c, value in enumerate(function(i)):
                        totals[c] += value
                return [value / len(indices) for value in totals]

            view[name] = {
                'original_linear_albedo': average(lambda i: [(a[4*i+c]/255)**2.2 for c in range(3)]),
                'current_linear_albedo': average(lambda i: [((b[4*i+c]/255+.055)/1.055)**2.4 for c in range(3)]),
                'original_composition': average(lambda i: packed_rgb(oc, i)),
                'current_composition': average(lambda i: packed_rgb(nc, i)),
                'original_ao_red_mean': sum(old_ao[2*i] for i in indices) / len(indices) / 255,
                'current_ao_red_mean': sum(new_ao[2*i] for i in indices) / len(indices) / 255,
            }
        result['views'][pose] = view
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path)
    parser.add_argument('current', type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.original, args.current), indent=2))
