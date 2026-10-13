#!/usr/bin/env python3
from pathlib import Path
import argparse, json, struct
parser = argparse.ArgumentParser(description='Verify the projection binding contract and index pinned sun-shadow DXBC files.')
parser.add_argument('index', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
rows = []
for item in json.loads(args.index.read_text()):
    p = Path(item['path'])
    p = p if p.is_absolute() else args.index.parent / p
    b = p.read_bytes()
    assert b[:4] == b'DXBC' and struct.unpack_from('<I', b, 24)[0] == len(b)
    r = None
    for off, in struct.iter_unpack('<I', b[32:32 + 4 * struct.unpack_from('<I', b, 28)[0]]):
        if b[off:off + 4] == b'RDEF':
            r = b[off + 8:off + 8 + struct.unpack_from('<I', b, off + 4)[0]]
            break
    assert r is not None, f'{p}: missing RDEF'

    def string(o):
        return r[o:r.index(b'\x00', o)].decode()
    cnt, off, nb, bo = struct.unpack_from('<4I', r)
    used = {}
    bindings = {}
    for i in range(nb):
        name, kind, _, _, _, slot, _, _ = struct.unpack_from('<8I', r, bo + 32 * i)
        bindings[string(name)] = (kind, slot)
    assert bindings['t_position'] == (2, 1) and bindings['$Globals'] == (0, 0)
    for i in range(cnt):
        name, n, vs, *_ = struct.unpack_from('<6I', r, off + 24 * i)
        if string(name) != '$Globals':
            continue
        for j in range(n):
            name, v_off, size, flags, *_ = struct.unpack_from('<6I', r, vs + 24 * j)
            if flags & 2:
                used[string(name)] = (v_off, size)
    assert used['view_to_shadow'] == (256, 64) and used['eye_ray_corner'] == (352, 12)
    assert not any((k.startswith('m_shadow') for k in used))
    rows.append('    {%d,{%s}},' % (len(b), ','.join(('0x%02x' % n for n in b[4:20]))))
header = '// Pinned 0.26 sun-shadow variants: b0 projection at256, eye ray at352, depth at t1.\nstatic const struct { unsigned size; unsigned char digest[16]; } sun_projection_shaders[] = {\n' + '\n'.join(rows) + '\n};\n'
args.output.write_text(header)
print('Verified projection binding layout in', len(rows), 'archived sun shaders')
