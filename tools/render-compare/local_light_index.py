#!/usr/bin/env python3
"""Index exact native deferred local-light shaders for read-only capture."""
import argparse
import json
import struct
from pathlib import Path

FAMILIES = ('point_light_accumulator.ps', 'spot_light_accumulator.ps',
            'plane_spot_light_accumulator.ps', 'obb_light_accumulator.ps',
            'capsule_light_accumulator.ps')

def generate(database, index, output):
    rows = {}
    with database.open('rb') as stream:
        for name, (offset, length) in sorted(json.loads(index.read_text()).items()):
            parts = name.split('/')
            if len(parts) != 4 or parts[:2] != ['shaders', 'sm_4_0'] or parts[2] not in FAMILIES:
                continue
            stream.seek(offset)
            entry = stream.read(length)
            start = entry.find(b'DXBC')
            if start < 0 or start + 32 > len(entry):
                raise ValueError(name)
            size = struct.unpack_from('<I', entry, start + 24)[0]
            code = entry[start:start + size]
            if len(code) != size or size < 32:
                raise ValueError(name)
            key = (size, code[4:20])
            family = FAMILIES.index(parts[2])
            if key in rows and rows[key] != family:
                raise ValueError('ambiguous shader family: ' + name)
            rows[key] = family
    if set(rows.values()) != set(range(len(FAMILIES))):
        raise ValueError('missing native local-light family')
    lines = ['// Generated from the pinned 026g archive by local_light_index.py.',
             'static const struct { unsigned size, family; unsigned char digest[16]; } local_light_shaders[] = {']
    for (size, digest), family in sorted(rows.items()):
        lines.append('    {%d,%d,{%s}},' % (size, family, ','.join('0x%02x' % v for v in digest)))
    output.write_text('\n'.join(lines + ['};', '']))
    print(json.dumps({'identities': len(rows), 'families': len(FAMILIES)}))

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path, required=True)
    parser.add_argument('--index', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    generate(args.database, args.index, args.output)
