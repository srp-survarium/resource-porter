#!/usr/bin/env python3
"""Compare composition overrides to native draws made with the same GPU inputs."""
import argparse
import json
import math
from pathlib import Path
import struct


def unsigned_float(bits, mantissa_bits):
    exponent = bits >> mantissa_bits
    mantissa = bits & ((1 << mantissa_bits) - 1)
    if exponent == 31:
        return math.nan if mantissa else math.inf
    if exponent == 0:
        return math.ldexp(mantissa, -14 - mantissa_bits)
    return math.ldexp(1 + mantissa / (1 << mantissa_bits), exponent - 15)


def compare(index):
    targets = json.loads(index.read_text())
    assert len(targets) == 2
    assert [t['name'] for t in targets] == ['override', 'native_same_inputs']
    for target in targets:
        assert target['format'] == 26 and target['written'], target
    width, height = targets[0]['width'], targets[0]['height']
    assert all((t['width'], t['height']) == (width, height) for t in targets)
    base = str(index).removesuffix('.targets.json')
    a, b = (Path(base + f'.rt{i:02d}.bin').read_bytes() for i in range(2))
    assert len(a) == len(b) == width * height * 4
    differences = []
    absolute_sum = 0
    max_absolute = 0
    different_pixels = nonfinite = over_one_ulp = 0
    for (va,), (vb,) in zip(struct.iter_unpack('<I', a), struct.iter_unpack('<I', b)):
        different_pixels += va != vb
        for shift, mantissa in [(0, 6), (11, 6), (22, 5)]:
            mask = (1 << (mantissa + 5)) - 1
            ba, bb = (va >> shift) & mask, (vb >> shift) & mask
            fa, fb = unsigned_float(ba, mantissa), unsigned_float(bb, mantissa)
            if not math.isfinite(fa) or not math.isfinite(fb):
                nonfinite += 1
                continue
            difference = abs(fa - fb)
            absolute_sum += difference
            max_absolute = max(max_absolute, difference)
            over_one_ulp += abs(ba - bb) > 1
            if fb > .00001:
                differences.append(difference / fb)
    differences.sort()
    return {
        'pixels': width * height,
        'different_pixels': different_pixels,
        'nonfinite_channels': nonfinite,
        'channels_over_one_ulp': over_one_ulp,
        'mean_absolute_channel_difference': absolute_sum / (width * height * 3),
        'maximum_absolute_channel_difference': max_absolute,
        'relative_p99_native_above_1e-5': differences[int(.99 * (len(differences) - 1))] if differences else None,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    args = parser.parse_args()
    indices = sorted(args.capture.glob('*.bmp.composition-output.targets.json'))
    if not indices:
        raise SystemExit('no same-frame composition captures')
    print(json.dumps({p.name: compare(p) for p in indices}, indent=2))
