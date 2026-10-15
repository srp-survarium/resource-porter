#!/usr/bin/env python3
"""find_wall_holes.py BUNDLE_OVERLAY PROJECT : enclosed openings in vertical render walls.

Every LOD0 render triangle of the project's render visuals, in world space.
Vertical triangles are grouped per model by wall plane (2 deg, 0.3 m buckets); each
plane with enough area is rasterised in wall coordinates (0.25 m cells);
empty regions fully enclosed by wall are openings. Prints openings at least
MIN_D m across with their centre, size, fill ratio (pi/4 for a disc,
1 for a rectangle) and the wall normal.
"""
import sys, os, math, struct, glob, collections, json
sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import chunks, config, rotation, transform
from audit_visible_positions import positions
from pathlib import Path
root = Path(sys.argv[1]); proj = config(Path(sys.argv[2]).read_bytes())
MIN_D = float(os.environ.get('MIN_D', 3.0)); CELL = 0.25
cache = {}
def model_tris(lib):
    if lib not in cache:
        out = []
        for f in glob.glob(str(root / 'models' / lib) + '.model/render/*/converted_model'):
            surf = f.split('/render/')[1].split('/')[0]
            if surf.startswith(('lod1', 'lod2', 'lod3')) or '_lod' in surf: continue
            d = open(f, 'rb').read()
            try: vs = [struct.unpack('<fff', b) for b in positions(d, False)]
            except Exception: continue
            ib = chunks(d)[4]; n = struct.unpack_from('<I', ib)[0]
            fmt = 'H' if len(ib) == 4 + 2*n else 'I'
            idx = struct.unpack_from('<%d%s' % (n, fmt), ib, 4)
            out += [(vs[idx[k]], vs[idx[k+1]], vs[idx[k+2]], surf) for k in range(0, n - 2, 3)]
        cache[lib] = out
    return cache[lib]
planes = collections.defaultdict(list)
for o in proj['render_visuals']:
    lib = o['lib_name']; r = rotation(o.get('rotation', [0, 0, 0])); t = o.get('position', [0, 0, 0]); sc = o.get('scale', [1, 1, 1])
    for a, b, c, surf in model_tris(lib):
        A, B, C = (transform(v, r, t, sc) for v in (a, b, c))
        u = [B[i]-A[i] for i in range(3)]; w = [C[i]-A[i] for i in range(3)]
        n = [u[1]*w[2]-u[2]*w[1], u[2]*w[0]-u[0]*w[2], u[0]*w[1]-u[1]*w[0]]; L = math.sqrt(sum(x*x for x in n))
        if L < 1e-9 or abs(n[1]) / L > 0.2: continue
        nx, nz = n[0], n[2]; h = math.hypot(nx, nz); nx, nz = nx/h, nz/h
        th = math.atan2(nz, nx) % math.pi
        if abs(math.atan2(nz, nx) - th) > 1e-6: nx, nz = -nx, -nz          # canonical normal for the bucket
        d = nx*A[0] + nz*A[2]
        planes[(lib, round(math.degrees(th) / 2) % 90, round(d / 0.3))].append(((A, B, C), lib, surf, L / 2))
def raster(tris, nx, nz):
    ux, uz = -nz, nx; cells = set()
    for (A, B, C), *_ in tris:
        P = [((p[0]*ux + p[2]*uz) / CELL, p[1] / CELL) for p in (A, B, C)]
        x0, x1 = math.floor(min(p[0] for p in P)), math.ceil(max(p[0] for p in P))
        y0, y1 = math.floor(min(p[1] for p in P)), math.ceil(max(p[1] for p in P))
        (ax, ay), (bx, by), (cx, cy) = P; den = (by-cy)*(ax-cx) + (cx-bx)*(ay-cy)
        if abs(den) < 1e-12: continue
        for i in range(x0, x1):
            for j in range(y0, y1):
                px, py = i + .5, j + .5
                l1 = ((by-cy)*(px-cx) + (cx-bx)*(py-cy)) / den; l2 = ((cy-ay)*(px-cx) + (ax-cx)*(py-cy)) / den
                if l1 >= -.02 and l2 >= -.02 and 1-l1-l2 >= -.02: cells.add((i, j))
    return cells
results = []
done = set()
for key, tris in sorted(planes.items(), key=lambda kv: -sum(t[3] for t in kv[1])):
    area = sum(t[3] for t in tris)
    if area < 30: break
    if key in done: continue
    lib0, th, db = key; group = []
    for dt in (-1, 0, 1):
        for dd in (-1, 0, 1):
            k2 = (lib0, (th + dt) % 90, db + dd); group += planes.get(k2, []); done.add(k2)
    ang = math.radians(th * 2); nx, nz = math.cos(ang), math.sin(ang)
    # split the plane into connected wall pieces (same plane, far apart)
    cells = raster(group, nx, nz)
    if not cells: continue
    seen = set(); 
    for start in list(cells):
        if start in seen: continue
        comp = []; st = [start]; seen.add(start)
        while st:
            c = st.pop(); comp.append(c)
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    q = (c[0]+dx, c[1]+dy)
                    if q in cells and q not in seen: seen.add(q); st.append(q)
        if len(comp) * CELL * CELL < 25: continue
        cs = set(comp); xs = [c[0] for c in comp]; ys = [c[1] for c in comp]
        X0, X1, Y0, Y1 = min(xs)-1, max(xs)+1, min(ys)-1, max(ys)+1
        outside = set(); st = [(X0, Y0)]; outside.add((X0, Y0))
        while st:
            c = st.pop()
            for q in ((c[0]+1, c[1]), (c[0]-1, c[1]), (c[0], c[1]+1), (c[0], c[1]-1)):
                if X0 <= q[0] <= X1 and Y0 <= q[1] <= Y1 and q not in cs and q not in outside: outside.add(q); st.append(q)
        hole_cells = set((i, j) for i in range(X0, X1+1) for j in range(Y0, Y1+1)) - cs - outside
        hs = set()
        for start2 in list(hole_cells):
            if start2 in hs: continue
            h = []; st = [start2]; hs.add(start2)
            while st:
                c = st.pop(); h.append(c)
                for q in ((c[0]+1, c[1]), (c[0]-1, c[1]), (c[0], c[1]+1), (c[0], c[1]-1)):
                    if q in hole_cells and q not in hs: hs.add(q); st.append(q)
            hx = [c[0] for c in h]; hy = [c[1] for c in h]
            w = (max(hx)-min(hx)+1) * CELL; ht = (max(hy)-min(hy)+1) * CELL
            if min(w, ht) < MIN_D: continue
            fill = len(h) * CELL * CELL / (w * ht)
            u = (sum(hx)/len(h) + .5) * CELL; v = (sum(hy)/len(h) + .5) * CELL; dpl = db * 0.3
            cx, cz = nx*dpl - nz*u, nz*dpl + nx*u
            libs = collections.Counter(t[1] for t in group)
            results.append(dict(centre=[round(cx, 2), round(v, 2), round(cz, 2)], width=w, height=ht, fill=round(fill, 2),
                                normal=[round(nx, 3), 0, round(nz, 3)], wall=libs.most_common(2)))
for r in sorted(results, key=lambda r: -r['width']*r['height']): print(json.dumps(r))
