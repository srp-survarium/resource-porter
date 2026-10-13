#!/usr/bin/env python3
"""Generate exact native emissive identities for optional read-only draw capture."""
import argparse
import json
from pathlib import Path
import struct

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--database', type=Path, required=True)
parser.add_argument('--index', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
rows = set()
catalog = set()
with args.database.open('rb') as stream:
    for name, (offset, length) in sorted(json.loads(args.index.read_text()).items()):
        if not name.startswith('shaders/sm_4_0/gbuffer_emissive_pass.ps/'):
            continue
        stream.seek(offset)
        entry = stream.read(length)
        start = entry.index(b'DXBC')
        size = struct.unpack_from('<I', entry, start+24)[0]
        code = entry[start:start+size]
        if size < 32 or len(code) != size:
            raise ValueError('truncated emissive shader: '+name)
        identity = (size, code[4:20])
        catalog.add(identity)
        key = name.rsplit('/', 1)[1]
        assert key[15] in '012'
        emitting = key[15] != '0'
        assert emitting == (b'solid_emission_color' in code)
        if emitting:
            rows.add(identity)
assert len(catalog) == 63 and len(rows) == 40, 'unexpected native emissive catalog'
lines = ['// Generated from the pinned native 0.26 archive by native_emissive_index.py.',
         'static const struct { unsigned size; unsigned char digest[16]; } native_emissive_shaders[] = {']
for size, digest in sorted(rows):
    lines.append('    {%d,{%s}},' % (size, ','.join('0x%02x' % x for x in digest)))
lines += ['};', 'static int is_native_emissive(const void *code,size_t size) {',
          '    for (unsigned i=0;i<sizeof(native_emissive_shaders)/sizeof(native_emissive_shaders[0]);++i)',
          '        if (size==native_emissive_shaders[i].size && !memcmp((const char *)code+4,native_emissive_shaders[i].digest,16)) return 1;',
          '    return 0;', '}']
args.output.write_text('\n'.join(lines)+'\n')
print('Pinned 40 emitting native shaders; excluded 23 zero-emission permutations')
