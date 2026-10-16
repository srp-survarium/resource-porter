import sys, subprocess, collections, struct, json
db, listing = sys.argv[1], sys.argv[2]
porter = '/tmp/resource-porter-render-compare/target/release/vostok-port'
f = open(db, 'rb')
def stat(p):
    out = subprocess.run([porter, 'stat', '--db', db, '--path', p], capture_output=True, text=True).stdout
    return dict(l.split('=', 1) for l in out.splitlines() if '=' in l)
def options(p):
    out = subprocess.run([porter, 'dump-db-config', '--db', db, '--path', p, '--leaves'], capture_output=True, text=True).stdout
    return dict((l.split('\t')[0].replace('options.', ''), l.split('\t')[1]) for l in out.splitlines() if '\t' in l)
c = collections.Counter(); ex = {}
for p in [l.strip() for l in open(listing) if l.strip()]:
    d = stat(p)
    pos, size = int(d['pos_in_db']), int(d['size_in_db'])
    f.seek(pos); hdr = f.read(128); f.seek(pos + size - 1); last = f.read(1)[0]
    o = options(p[:-4] + '.options')
    fourcc = hdr[84:88].decode('latin1')
    key = ('ported' if '/ported/' in p else 'native', fourcc, 'srgb=' + o.get('srgb', '?'), 'fmt=' + o.get('format', '?'), 'last=%d' % last)
    c[key] += 1; ex.setdefault(key, p)
for k, v in sorted(c.items(), key=str): print(v, k, ex[k])
