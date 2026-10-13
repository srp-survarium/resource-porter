#!/usr/bin/env python3
"""Measure diffuse attenuation across the captured SSAO/height-ambient pass."""
import argparse
import json
from pathlib import Path
import statistics

from inspect_buffers import pixels


def measure(folder):
    result = {}
    for before_path in sorted(folder.glob('*.bmp.before-hba.rt31.bin')):
        base = str(before_path).removesuffix('.before-hba.rt31.bin')
        before = list(pixels(before_path.read_bytes(), 26))
        after = list(pixels(Path(base + '.after-hba.rt31.bin').read_bytes(), 26))
        if len(before) != len(after) or not before:
            raise ValueError('empty or mismatched buffers')
        ratios = sorted(sum(b) / sum(a) for a, b in zip(before, after) if sum(a) > .0001)
        if not ratios:
            raise ValueError('no lit pixels')
        def mean(values):
            return [sum(pixel[channel] for pixel in values) / len(values) for channel in range(3)]
        result[Path(base).stem] = {
            'before_mean': mean(before), 'after_mean': mean(after),
            'included_pixels': len(ratios), 'total_pixels': len(before),
            'ratio_median': statistics.median(ratios),
            'ratio_p10': ratios[len(ratios) // 10],
            'ratio_p90': ratios[len(ratios) * 9 // 10],
        }
    if not result:
        raise ValueError('no captured ambient buffers')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('folder', type=Path)
    args = parser.parse_args()
    print(json.dumps(measure(args.folder), indent=2))
