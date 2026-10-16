import sys, struct, collections, subprocess, json
sys.path.insert(0, '/tmp/resource-porter-render-compare/deps')
db, listing = sys.argv[1], sys.argv[2]
porter = '/tmp/resource-porter-render-compare/target/release/vostok-port'
paths = [l.strip() for l in open(listing) if l.startswith('textures/') and l.strip().endswith('.dds')]
# read headers via stat + raw reads from the db (uncompressed entries)
f = open(db, 'rb')
res = collections.Counter(); bad = []
def stat(p):
    out = subprocess.run([porter, 'stat', '--db', db, '--path', p], capture_output=True, text=True).stdout
    d = dict(l.split('=', 1) for l in out.splitlines() if '=' in l)
    return d
for i, p in enumerate(paths):
    d = stat(p)
    if 'compressed: false' not in d.get('kind', ''):
        res[('compressed-or-inline', p.startswith('textures/ported'))] += 1; continue
    f.seek(int(d['pos_in_db'])); b = f.read(160); size = int(d['size_in_db'])
    if b[:4] != b'DDS ': res[('not-dds', p.startswith('textures/ported'))] += 1; continue
    h, w = struct.unpack_from('<II', b, 12); mips = max(1, struct.unpack_from('<I', b, 28)[0]); fourcc = b[84:88]
    flags = struct.unpack_from('<I', b, 80)[0]
    hdr = 128 + (20 if fourcc == b'DX10' else 0)
    caps2 = struct.unpack_from('<I', b, 112)[0]; faces = 6 if caps2 & 0x200 else 1
    depth = max(1, struct.unpack_from('<I', b, 24)[0]) if caps2 & 0x200000 else 1
    if flags & 4:
        bs = 8 if fourcc in (b'DXT1', b'ATI1', b'BC4U') else 16
        tot = faces * sum(max(1, (max(1, w >> m) + 3) // 4) * max(1, (max(1, h >> m) + 3) // 4) * bs * max(1, depth >> m) for m in range(mips))
    else:
        bpp = struct.unpack_from('<I', b, 88)[0] // 8
        tot = faces * sum(max(1, w >> m) * max(1, h >> m) * bpp * max(1, depth >> m) for m in range(mips))
    extra = size - hdr - tot
    key = ('ported' if p.startswith('textures/ported') else 'native', fourcc.decode('latin1') if flags & 4 else 'rgb', extra if extra in (0, 1) else 'other')
    res[key] += 1
    if extra not in (0, 1): bad.append((p, w, h, mips, fourcc.decode('latin1'), size, hdr + tot))
for k, v in sorted(res.items(), key=str): print(k, v)
print('inconsistent:', len(bad)); [print(' ', x) for x in bad[:30]]
