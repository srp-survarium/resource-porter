#!/usr/bin/env python3
"""Check a paired composition draw that changes only the translucency base term."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
from audit_surface_light import REGIONS, packed_rgb


def audit(folder, shader, comparison):
    result = {}
    for base in sorted(folder.glob('*.bmp')):
        def read(suffix):
            return Path(str(base) + suffix).read_bytes()

        assert read('.composition-draw.dxbc') == shader.read_bytes()
        assert read('.composition-output.comparison.dxbc') == comparison.read_bytes()
        metadata = json.loads(read('.composition-output.targets.json'))
        assert [(m['slot'], m['format'], m['written']) for m in metadata] == [(0, 26, True), (1, 26, True), (2, 26, True)]
        current, previous = read('.composition-output.rt00.bin'), read('.composition-output.rt02.bin')
        parameters = read('.composition-draw.rt02.bin')
        assert len(current) == len(previous) == len(parameters) == 1280 * 720 * 4
        opaque_changed = translucent_changed = nonfinite = 0
        for i, ((a,), (b,)) in enumerate(zip(struct.iter_unpack('<I', current), struct.iter_unpack('<I', previous))):
            if a != b:
                if parameters[4*i+2]:
                    translucent_changed += 1
                else:
                    opaque_changed += 1
            for value in (a, b):
                nonfinite += any(((value >> shift) & 31) == 31 for shift in (6, 17, 27))
        regions = {}
        for name, (x0, y0, x1, y1) in REGIONS.get(base.stem, {}).items():
            indices = [y * 1280 + x for y in range(y0, y1) for x in range(x0, x1)]
            regions[name] = {key: [sum(packed_rgb(data, i)[c] for i in indices) / len(indices) for c in range(3)]
                             for key, data in [('previous', previous), ('current', current)]}
        result[base.name] = {'pixels': 1280 * 720, 'opaque_changed': opaque_changed,
                             'translucent_changed': translucent_changed, 'nonfinite_pixels': nonfinite,
                             'shader_sha256': hashlib.sha256(shader.read_bytes()).hexdigest(),
                             'comparison_sha256': hashlib.sha256(comparison.read_bytes()).hexdigest(),
                             'regions': regions}
    assert result, 'no captures'
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('--shader', required=True, type=Path)
    parser.add_argument('--comparison', required=True, type=Path)
    args = parser.parse_args()
    result = audit(args.capture, args.shader, args.comparison)
    print(json.dumps(result, indent=2))
    if any(r['opaque_changed'] or r['nonfinite_pixels'] for r in result.values()):
        raise SystemExit('paired translucency isolation check failed')
