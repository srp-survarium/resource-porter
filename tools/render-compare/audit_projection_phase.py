#!/usr/bin/env python3
"""Report captured projection jitter without inferring a material discrepancy."""
import argparse
import json
import struct
from pathlib import Path


def phase(shader):
    code = shader.read_bytes()
    if code[:4] != b'DXBC':
        raise ValueError(shader)
    count = struct.unpack_from('<I', code, 28)[0]
    offsets = struct.unpack_from('<' + 'I' * count, code, 32)
    at = next(o for o in offsets if code[o:o + 4] == b'RDEF')
    data = code[at + 8:at + 8 + struct.unpack_from('<I', code, at + 4)[0]]
    def name(offset):
        return data[offset:data.index(0, offset)].decode()
    count, at = struct.unpack_from('<II', data, 8)
    slots = {}
    for i in range(count):
        row = struct.unpack_from('<8I', data, at + i * 32)
        if row[1] == 0:
            slots[name(row[0])] = row[5]
    count, at = struct.unpack_from('<II', data)
    for i in range(count):
        label, variables, begin = struct.unpack_from('<III', data, at + i * 24)
        if name(label) not in ('static_globals', 'StaticGlobals'):
            continue
        static_slot = slots[name(label)]
        raw = shader.with_suffix(f'.cb{static_slot}.bin').read_bytes()
        fields = {}
        for j in range(variables):
            label, offset, length = struct.unpack_from('<III', data, begin + j * 24)
            fields[name(label)] = (offset, length)
        # The adapter declares only used fields, but keeps the native 026 ABI:
        # m_V2W at176 and m_P at0 in the same unmodified runtime buffer.
        if 'm_P' not in fields and fields.get('m_V2W', (None,))[0] != 176:
            raise ValueError('unverified adapter static-globals ABI')
        matrix = struct.unpack_from('<16f', raw, fields.get('m_P', (0, 64))[0])
        screen = struct.unpack_from('<4f', raw, fields['screen_res'][0])
        return dict(shader=str(shader), static_slot=static_slot, projection_xy=[matrix[2], matrix[6]],
                    screen=list(screen[:2]), projection_pixel_offset=[matrix[2] * screen[0] / 2, -matrix[6] * screen[1] / 2])
    raise ValueError(f'no static globals: {shader}')


def compare(original, current):
    rows = []
    for target in sorted(current.glob('*.probe-draw.dxbc')):
        source = original / target.name
        if not source.exists():
            continue
        before, after = phase(source), phase(target)
        if before['screen'] != after['screen']:
            raise ValueError('capture sizes differ')
        delta = [b-a for a,b in zip(before['projection_pixel_offset'], after['projection_pixel_offset'])]
        rows.append(dict(pose=target.name.split('.bmp')[0], original=before, current=after,
                         delta_pixels=delta, same_phase=max(abs(v) for v in delta) < 1e-5))
    return dict(scope='Actual captured static_globals m_P[2]/m_P[6]. Equal director pose does not imply equal raster phase; this audit does not shift or resample images.',
                original=str(original), current=str(current), matched=len(rows), same_phase=sum(r['same_phase'] for r in rows), poses=rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path)
    parser.add_argument('current', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = compare(args.original, args.current)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(matched=report['matched'], same_phase=report['same_phase'])))
