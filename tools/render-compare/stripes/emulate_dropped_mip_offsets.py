"""Emulate 0.26g's dropped-mip offset (0x3E17D0/0x3E16B0) for each DDS and compare with the true offset."""
import sys, subprocess, collections, struct
import numpy as np
db, listing = sys.argv[1], sys.argv[2]
porter = '/tmp/resource-porter-render-compare/target/release/vostok-port'
f = open(db, 'rb')
def stat(p):
    out = subprocess.run([porter, 'stat', '--db', db, '--path', p], capture_output=True, text=True).stdout
    return dict(l.split('=', 1) for l in out.splitlines() if '=' in l)
def true_sizes(w, h, mips, bs):
    return [max(1, (max(1, w >> m) + 3) // 4) * max(1, (max(1, h >> m) + 3) // 4) * bs for m in range(mips)]
def engine_offset(linear, mips, k):
    S = np.float32(0)
    for i in range(mips): S = np.float32(S + np.float32(1) / np.float32(4.0 ** i))
    base = np.float32(np.longdouble(linear - 1) / np.longdouble(S))
    base = int(base)  # truncation
    return sum(base // int(4 ** i) for i in range(k))
c = collections.Counter(); ex = {}
for p in [l.strip() for l in open(listing) if l.strip()]:
    d = stat(p); pos, size = int(d['pos_in_db']), int(d['size_in_db'])
    f.seek(pos); b = f.read(128)
    h, w, linear, depth, mips = struct.unpack_from('<IIIII', b, 12); fourcc = b[84:88]; flags = struct.unpack_from('<I', b, 80)[0]
    if not flags & 4: c[('uncompressed',)] += 1; continue
    bs = 8 if fourcc == b'DXT1' else 16
    sizes = true_sizes(w, h, max(1, mips), bs)
    lin_ok = linear == sum(sizes) + 1
    bad = [k for k in range(1, mips) if engine_offset(linear, mips, k) != sum(sizes[:k])]
    key = ('ported' if '/ported/' in p else 'native', 'linear=total+1' if lin_ok else 'linear=%s' % ('mip0' if linear == sizes[0] else 'other'),
           'square' if w == h else 'nonsquare', 'skip-ok' if not bad else 'skip-WRONG')
    c[key] += 1; ex.setdefault(key, (p, w, h, mips, fourcc.decode('latin1'), linear, sum(sizes), bad[:4]))
for k, v in sorted(c.items(), key=str): print(v, k, ex.get(k))
