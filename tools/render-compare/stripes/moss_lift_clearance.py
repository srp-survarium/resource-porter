import struct, sys, glob, os, collections, numpy as np
G = sys.argv[1]
def load(f):
    b = open(f, 'rb').read(); tag, size, n = struct.unpack_from('<III', b, 0)
    if tag != 3 or n == 0 or (size - 4) % n: return None
    st = (size - 4) // n
    if st < 16: return None
    v = np.frombuffer(b, np.uint8, count=n * st, offset=12).reshape(n, st)
    pos = np.frombuffer(v[:, 0:12].tobytes(), np.float32).reshape(n, 3).astype(np.float64)
    nr = v[:, 12:16].astype(float); N = np.stack([nr[:, 2], nr[:, 1], nr[:, 0]], 1) / 127.5 - 1
    N /= np.maximum(np.linalg.norm(N, axis=1, keepdims=True), 1e-6)
    end = 8 + size
    if b[end:end + 4] != struct.pack('<I', 4): return pos, N, None
    isize, icount = struct.unpack_from('<II', b, end + 4)
    idx = np.frombuffer(b, np.uint16, count=icount, offset=end + 12).astype(np.int64).reshape(-1, 3)
    return pos, N, idx
scene = []
for f in glob.glob(f'{G}/*.bin'):
    name = os.path.basename(f)
    if 'moss' in name or 'anomaly_liquid' in name or 'stump' in name: continue
    r = load(f)
    if r is None or r[2] is None: continue
    p, _, idx = r
    if idx.max() >= len(p): continue
    scene.append(p[idx])
T = np.concatenate(scene); print('scene triangles', len(T))
CELL = 4.0
lo = T.min(1); hi = T.max(1)
grid = collections.defaultdict(list)
for i, (a, b) in enumerate(zip(np.floor(lo[:, [0, 2]] / CELL).astype(int), np.floor(hi[:, [0, 2]] / CELL).astype(int))):
    if (b - a).max() > 8: continue  # giant triangles (terrain) handled below
    for x in range(a[0], b[0] + 1):
        for z in range(a[1], b[1] + 1): grid[(x, z)].append(i)
big = [i for i, (a, b) in enumerate(zip(np.floor(lo[:, [0, 2]] / CELL).astype(int), np.floor(hi[:, [0, 2]] / CELL).astype(int))) if (b - a).max() > 8]
def raycast(o, d, tris, tmax):
    # Moller-Trumbore, returns nearest t in (1e-4, tmax] or inf, for one ray vs many triangles
    if len(tris) == 0: return np.inf
    v0, v1, v2 = T[tris, 0], T[tris, 1], T[tris, 2]
    e1, e2 = v1 - v0, v2 - v0
    pv = np.cross(d, e2); det = (e1 * pv).sum(1)
    ok = np.abs(det) > 1e-12; inv = np.where(ok, 1 / np.where(ok, det, 1), 0)
    tv = o - v0; u = (tv * pv).sum(1) * inv
    qv = np.cross(tv, e1); v = (qv * d).sum() * 0 + (qv @ d) * inv
    t = (qv * e2).sum(1) * inv
    hit = ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t > 1e-4) & (t <= tmax)
    return t[hit].min() if hit.any() else np.inf
def cells(p, r=1):
    c = np.floor(p[[0, 2]] / CELL).astype(int); out = set()
    for x in range(c[0] - r, c[0] + r + 1):
        for z in range(c[1] - r, c[1] + r + 1): out.update(grid.get((x, z), []))
    return np.array(sorted(out) + big, dtype=np.int64)
for model in ['moss', 'moss2', 'moss3', 'moss4']:
    for sg, current in [('anomaly_moss_base_sg', 0.0), ('anomaly_moss_lod_sg', 0.10)]:
        p, n, idx = load(f'{G}/{model}.model~{sg}.bin')  # authored geometry (no lift)
        p = p + current * n   # what the LOD1 candidate already has
        edges = collections.Counter(tuple(sorted(e)) for t in idx for e in ((t[0], t[1]), (t[1], t[2]), (t[0], t[2])))
        boundary = sorted({v for e, k in edges.items() if k == 1 for v in e})
        back = np.full(len(p), np.inf); front = np.full(len(p), np.inf)
        for i in range(len(p)):
            tr = cells(p[i])
            back[i] = raycast(p[i], -n[i], tr, 1.0)
            front[i] = raycast(p[i], n[i], tr, 0.25)
        res = []
        for extra in (0.05, 0.10):
            gap = back[boundary] + extra; gap = gap[np.isfinite(gap)]
            inter = int(np.sum((front <= extra) & (front > 0.03))); exiting = int(np.sum(front <= min(extra, 0.03)))
            res.append(f'+{int(extra*100)}cm: boundary gap p50 {np.median(gap)*100:.1f} p95 {np.percentile(gap,95)*100:.1f} max {gap.max()*100:.1f} cm (n={len(gap)}), verts reaching other geometry {inter} (plus {exiting} leaving their own wall within 3 cm) of {len(p)}')
        cur = back[boundary]; cur = cur[np.isfinite(cur)]
        print(f'{model} {sg}: now boundary gap p50 {np.median(cur)*100:.1f} max {cur.max()*100:.1f} cm; ' + ' | '.join(res))
