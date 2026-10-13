#!/usr/bin/env python3
"""Compare actual original and adapted fog bindings and saved regional outputs."""
import argparse
import json
from pathlib import Path
import struct

from audit_surface_light import REGIONS, packed_rgb


def read(base, suffix):
    return Path(str(base) + suffix).read_bytes()


def constant(base, name):
    code = read(base, '.dxbc')
    for (offset,) in struct.iter_unpack('<I', code[32:32 + 4 * struct.unpack_from('<I', code, 28)[0]]):
        if code[offset:offset + 4] == b'RDEF':
            raw = code[offset + 8:offset + 8 + struct.unpack_from('<I', code, offset + 4)[0]]
            break
    else:
        raise ValueError('missing shader reflection')
    def string(offset):
        return raw[offset:raw.index(b'\0', offset)].decode()
    cb_count, cb_offset, binding_count, binding_offset = struct.unpack_from('<4I', raw)
    slots = {}
    for i in range(binding_count):
        row = struct.unpack_from('<8I', raw, binding_offset + 32 * i)
        if row[1] == 0:
            slots[string(row[0])] = row[5]
    for i in range(cb_count):
        cb_name, count, variables, _, _, _ = struct.unpack_from('<6I', raw, cb_offset + 24 * i)
        for j in range(count):
            n, offset, size, _, _, _ = struct.unpack_from('<6I', raw, variables + 24 * j)
            if string(n) == name:
                return struct.unpack_from('<' + 'f' * (size // 4), read(base, f'.cb{slots[string(cb_name)]}.bin'), offset)
    raise ValueError('missing constant: ' + name)


def audit(original, current):
    result = {'scope': 'Observed bindings and regional linear RGB means; not visual parity or a shader replay.', 'views': {}}
    for pose, regions in REGIONS.items():
        a, b = original / (pose + '.bmp'), current / (pose + '.bmp')
        old_fog, new_fog = Path(str(a) + '.fog-add-draw'), Path(str(b) + '.fog-draw')
        old_s = {x['slot']: x for x in json.loads(read(old_fog, '.samplers.json'))}
        new_s = {x['slot']: x for x in json.loads(read(new_fog, '.samplers.json'))}
        assert (old_s[1]['filter'], old_s[1]['address']) == (21, [3, 3, 3])
        assert (new_s[0]['filter'], new_s[0]['address']) == (21, [3, 3, 3]), 'adapted fog must clamp'
        assert constant(old_fog, 'to_sun_direction_parameter') == (0, 1, 0, 1)
        distance = constant(old_fog, 'inscatter_parameters')[0]
        assert struct.unpack_from('<f', read(new_fog, '.cb1.bin'), 16 * 16 + 4)[0] == distance
        composition_continuity = {}
        for label, base, suffix in [('original', a, '.fog-mul-before.rt00.bin'), ('current', b, '.fog-before.rt00.bin')]:
            left, right = read(base, '.composition-output.rt00.bin'), read(base, suffix)
            assert len(left) == len(right)
            composition_continuity[label] = sum(x != y for x, y in zip(struct.iter_unpack('<I', left), struct.iter_unpack('<I', right)))
        assert read(a, '.fog-mul-output.rt00.bin') == read(a, '.fog-add-before.rt00.bin')
        tables = {}
        for slot, name in [(1, 'rayleigh'), (2, 'mie')]:
            aa, bb = read(old_fog, f'.rt{slot:02}.bin'), read(new_fog, f'.rt{slot:02}.bin')
            assert len(aa) == len(bb) == 256 * 128 * 8
            old = list(struct.iter_unpack('<4e', aa)); new = list(struct.iter_unpack('<4e', bb))
            ratios = [y / x for p, q in zip(old, new) for x, y in zip(p[:3], q[:3]) if x > .0001]
            tables[name] = {'byte_identical': aa == bb, 'rgb_ratio_min_mean_max': [min(ratios), sum(ratios) / len(ratios), max(ratios)]}
        samples = {}
        buffers = {name: read(base, suffix) for name, base, suffix in [
            ('original_before', a, '.fog-mul-before.rt00.bin'), ('original_after', a, '.fog-add-output.rt00.bin'),
            ('current_before', b, '.fog-before.rt00.bin'), ('current_after', b, '.fog-output.rt00.bin')]}
        for name, rect in regions.items():
            x0, y0, x1, y1 = rect
            indices = [y * 1280 + x for y in range(y0, y1) for x in range(x0, x1)]
            values = {}
            for key, data in buffers.items():
                assert len(data) == 1280 * 720 * 4
                rgb = [packed_rgb(data, i) for i in indices]
                values[key] = [sum(v[c] for v in rgb) / len(rgb) for c in range(3)]
            samples[name] = {'rectangle': rect, 'linear_rgb_means': values}
        result['views'][pose] = {'composition_to_fog_changed_pixels': composition_continuity, 'fog_sampler_matches': True, 'inscatter_distance': distance, 'tables': tables, 'regions': samples}
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path)
    parser.add_argument('current', type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.original, args.current), indent=2))
