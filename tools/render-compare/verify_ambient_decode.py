#!/usr/bin/env python3
"""Verify the encoded ambient pass against its captured inputs and output."""
import argparse
import json
import math
from pathlib import Path
import struct

from compare_composition import unsigned_float


def verify(base):
    def read(suffix):
        return Path(str(base) + suffix).read_bytes()

    textures = {t['slot']: t for t in json.loads(read('.hba-draw.targets.json'))}
    width, height = textures[0]['width'], textures[0]['height']
    original_ssao = b't_legacy_ssao\0' in read('.hba-draw.dxbc')
    assert textures[0]['format'] == (49 if original_ssao else 61) and textures[2]['format'] == 26
    assert (textures[2]['width'], textures[2]['height']) == (width, height)
    encoded, copied = read('.before-hba.rt31.bin'), read('.hba-draw.rt02.bin')
    actual, ao = read('.after-hba.rt31.bin'), read('.hba-draw.rt00.bin')
    assert len(encoded) == len(copied) == len(actual) == width * height * 4
    assert len(ao) == width * height * (2 if original_ssao else 1)
    if original_ssao:
        assert ao == read('.ssao-live.rt02.bin'), 'decoder did not consume the live archived AO'
        assert not any(ao[1::2]), 'unexpected second channel in archived AO'
        ao = ao[::2]
    assert encoded == copied, 'decoder copy differs from the encoded accumulator'
    assert not any(read('.probe-initial.rt00.bin')), 'initial ambient is not black'
    blend = json.loads(read('.hba-draw.blend.json'))
    assert blend == {'enabled': 0, 'source': 2, 'destination': 1, 'operation': 1}, blend
    relative = []
    outside_tolerance = nonfinite = 0
    maximum_error = 0
    for (before,), (after,), occlusion in zip(struct.iter_unpack('<I', encoded), struct.iter_unpack('<I', actual), ao):
        for shift, mantissa in [(0, 6), (11, 6), (22, 5)]:
            mask = (1 << (mantissa + 5)) - 1
            e = unsigned_float((before >> shift) & mask, mantissa)
            bits = (after >> shift) & mask
            result = unsigned_float(bits, mantissa)
            predicted = 8 * e * e * (occlusion / 255) * (2.5 if original_ssao else 1)
            if not math.isfinite(result) or not math.isfinite(predicted):
                nonfinite += 1
                continue
            error = abs(predicted - result)
            maximum_error = max(maximum_error, error)
            # Readbacks are quantized R11G11B10; allow one stored-value step.
            step = unsigned_float(bits + 1, mantissa) - result
            outside_tolerance += error > max(step, 1e-9)
            if predicted > .00001:
                relative.append(error / predicted)
    relative.sort()
    return {
        'pixels': width * height,
        'ao_source': 'archived_spatial_no_temporal' if original_ssao else 'native',
        'initial_ambient_black': True,
        'copy_identical': True,
        'overwrite_blend_verified': True,
        'specular_unchanged': read('.before-hba.rt34.bin') == read('.after-hba.rt34.bin'),
        'nonfinite_channels': nonfinite,
        'channels_outside_one_step': outside_tolerance,
        'maximum_absolute_error': maximum_error,
        'relative_p99_above_1e-5': relative[int(.99 * (len(relative) - 1))] if relative else None,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    args = parser.parse_args()
    bases = sorted(args.capture.glob('*.bmp'))
    if not bases:
        raise SystemExit('no captured poses')
    results = {p.name: verify(p) for p in bases}
    print(json.dumps(results, indent=2))
    if any(r['nonfinite_channels'] or r['channels_outside_one_step'] or not r['specular_unchanged'] for r in results.values()):
        raise SystemExit('ambient decode verification failed')
