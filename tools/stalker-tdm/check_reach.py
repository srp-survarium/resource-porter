#!/usr/bin/env python3
"""Where a walker gets on a built STALKER map, and whether he gets back.

    check_reach.py --overlay BUNDLE026/overlay --spec SPEC.json --map ID
                   [--target-db 0.26g resources.db] [--vostok-port VP]
                   [--points FILE.tsv] [--ladders FILE.json] [--png OUT.png]
                   [--trace X,Y,Z] [--without-added] [--design MAPS.json KEY]

The built map's collision (the project's collision objects: the level, the
walker ramps and boxes, the native props read from --target-db) on 0.25 m
cells, as render_minimap.play_area finds floors (a walkable face with 1.3 m
free over it: a crouching walker's height; render_minimap's 1.7 m is a
standing one's). Two floors of neighbouring cells are joined both ways when
they are at most 0.5 m apart (0.26g steps 0.6 m); one way up when the higher
one is 0.5 to 1.3 m above and a body stands there (a jump onto a floor
with floor within 0.5 m on 5 of the 8 cells round it: the edge of a floor, not the top of a wall; its headroom not checked), and one way
down when the lower one is 0.5 to 4.6 m below (a drop a walker survives); a
drop or a jump lands on a floor a walker stands on (1.7 m free), not in a
crawl space, and needs the way open (no floor in the lower cell up to where
the walker falls from, none over the jump's start below where it lands).
0.26g cannot climb: a ladder joins nothing.

From the map's spawns (the spec's teams, and the x/y/z of --points):

- walked: the floors reached walking only, and every spawn on one of them;
- dead ends: floors reached with drops from which no way leads back to the
  spawns' floors, in areas (joined cells), each with its size and place;
- not reached: floors in the frame of the walked floors that nothing reaches,
  in areas of 10 m2 or more (roofs only a ladder reaches, closed rooms),
  with their size, height and place;
- ladders (--ladders: the level's climbable objects as a list of name, x, z,
  foot and top): whether a floor at the ladder's top is reached, walked, or
  neither;
- --trace: the shortest way from the walked floors to the floor at X,Y,Z,
  each drop and jump on it;
- --without-added: the map as the level alone has it, without the added
  walker ramps and boxes (models/.../srp/) and native props: what was a dead
  end before them (the cuts stay);
- --design: the added geometry of maps.json's KEY (walker_ramps,
  walker_boxes, props) in place of the built one, made as the importer makes
  it (survarium_026.rs ramp_box; a prop's collision and walker collision):
  a design checked before it is built. Its cuts must be built already.

Exit 1 when a spawn is not on a walked floor or a dead end is 1 m2 or more
and has room for a standing body (4 x 4 cells of standing floor, 1 m: a gutter or a ledge along a
wall holds nobody).
"""
import argparse
import csv
import inspect
import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from scipy import ndimage
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import breadth_first_order, connected_components

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'playable-map-pack'))
import render_minimap as rm  # noqa: E402

CELL = rm.CELL
STEP = 0.5
JUMP = 1.3
DROP = 4.6
MARGIN = 20.0
CROUCH = 1.3


def floors(tris, bounds, ylo, yhi, headroom=None):
    """Every floor (cell index, height) of render_minimap.play_area, reached or not, and the grid width."""
    src = inspect.getsource(rm.play_area)
    src = src.replace('def play_area(', 'def all_floors_of(')
    src = src.replace("    ylo = min(s[1] for s in spawns) - 10; yhi = max(s[1] for s in spawns) + 20\n", '')
    head, _ = src.split('    # Graph: 4-neighbour cells, floors within STEP.')
    ns = dict(vars(rm))
    ns.update(ylo=ylo, yhi=yhi, HEADROOM=CROUCH if headroom is None else headroom)
    exec(head + '    return fp, fh, w, h\n', ns)
    return ns['all_floors_of'](tris, None, bounds, all_floors=True, slab_tops=True)


def ramp_box(frm, to, width):
    """survarium_026.rs ramp_box: the slab's rotation, half extents and centre; its top is the line from-to."""
    frm, to = np.asarray(frm, float), np.asarray(to, float)
    d = to - frm
    u = d / np.linalg.norm(d)
    up = np.array([-u[0] * u[1], 1 - u[1] ** 2, -u[2] * u[1]])
    n = up / np.linalg.norm(up)
    wv = np.cross(u, n)
    rot = [math.atan2(-n[2], wv[2]), math.asin(max(-1.0, min(1.0, u[2]))), math.atan2(-u[1], u[0])]
    return rot, [np.linalg.norm(d) / 2 + 0.3, 0.1, width / 2], (frm + to) / 2 - n * 0.1


def design_triangles(m, target_db, vp):
    """A map entry's walker ramps and boxes and its props' collision, as the importer places them."""
    out = []
    for r in m.get('walker_ramps', []):
        rot, half, centre = ramp_box(r['from'], r['to'], r['width'])
        out.append(rm.box_triangles({'position': list(centre), 'scale': half, 'rotation': rot}))
    for b in m.get('walker_boxes', []):
        out.append(rm.box_triangles({'position': b['centre'], 'scale': b['half'], 'rotation': [0, b.get('yaw', 0.0), 0]}))
    props = m.get('props', [])
    if props:
        native = Path(tempfile.mkdtemp(prefix='check-reach-design-'))
        objects = []
        for q in props:
            for part in ('collision', 'walker_collision'):
                lib = f"models/{q['model']}.model/{part}"
                for f in ('vertices', 'indices', 'exported_primitives'):
                    (native / lib).mkdir(parents=True, exist_ok=True)
                    subprocess.run([vp, 'extract', '--db', str(target_db), '--path', f'{lib}/{f}', '--output', str(native / lib / f)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                objects.append({'lib_name': lib, 'position': q['position'], 'scale': [1, 1, 1],
                                'rotation': q.get('rotation', [0, q.get('yaw', 0.0), 0])})
        out.append(rm.collision_triangles(native, {'collision_objects': objects}))
    return np.concatenate(out).reshape(-1, 3, 3) if out else np.zeros((0, 3, 3))


def neighbours(fp, w, h, diagonal=False):
    """Pairs (a, b) of floors in 4-neighbouring cells (8 with diagonal)."""
    out = []
    steps = [(1, 0), (-1, 0), (0, 1), (0, -1)] + ([(1, 1), (1, -1), (-1, 1), (-1, -1)] if diagonal else [])
    for di, dj in steps:
        d = di + dj * w
        tgt = fp + d
        ok = (tgt >= 0) & (tgt < w * h) & (fp % w + di >= 0) & (fp % w + di < w)
        lo = np.searchsorted(fp, tgt, 'left'); hi = np.searchsorted(fp, tgt, 'right')
        n = np.where(ok, hi - lo, 0)
        for k in range(int(n.max()) if len(n) else 0):
            s = np.nonzero(n > k)[0]
            out.append(np.stack([s, lo[s] + k]))
    return np.concatenate(out, 1)


def areas(mask, fp, w, pairs, fh):
    """Joined areas of the floors in mask: [(m2, (x, y, z) of the lowest, cells)]."""
    idx = np.nonzero(mask)[0]
    if not len(idx):
        return []
    pos = -np.ones(len(fp), int); pos[idx] = np.arange(len(idx))
    a, b = pairs
    keep = mask[a] & mask[b] & (np.abs(fh[a] - fh[b]) <= STEP)
    g = coo_matrix((np.ones(keep.sum()), (pos[a[keep]], pos[b[keep]])), shape=(len(idx), len(idx)))
    _, lab = connected_components(g, directed=False)
    out = []
    for c in range(lab.max() + 1):
        k = idx[lab == c]
        lowest = k[np.argmin(fh[k])]
        out.append((len(np.unique(fp[k])) * CELL * CELL, lowest, k))
    return sorted(out, key=lambda t: -t[0])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--overlay', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--map', type=int, required=True)
    ap.add_argument('--target-db', type=Path)
    ap.add_argument('--vostok-port', default='vostok-port')
    ap.add_argument('--points', type=Path, help='a TSV with x, y, z columns: more spawns to check (Free for All)')
    ap.add_argument('--ladders', type=Path, help="JSON: a list of the level's ladders (name, x, z, foot, top)")
    ap.add_argument('--png', type=Path)
    ap.add_argument('--png-band', help='LO,HI: draw only the floors between these heights')
    ap.add_argument('--without-added', action='store_true', help='leave out the added walker ramps, boxes and props')
    ap.add_argument('--design', nargs=2, metavar=('MAPS', 'KEY'), help="maps.json and a map's key: its added geometry in place of the built one")
    ap.add_argument('--trace', action='append', default=[], help='X,Y,Z (repeatable; --trace=-1,2,3): the way from the walked floors to the floor there')
    a = ap.parse_args()
    m = next(m for m in json.loads(a.spec.read_text())['maps'] if m['id'] == a.map)
    project = rm.config((a.overlay / 'projects' / f"v010b_{m['donor_project']}" / 'day_4' / 'client_project').read_bytes())
    if a.without_added or a.design:
        project = dict(project, collision_objects=[o for o in project['collision_objects']
                                                   if '/srp/' not in o['lib_name'] and '/props/' not in o['lib_name']])
    native = None
    models = sorted({o['lib_name'].removeprefix('resources/').split('#')[0] for o in project['collision_objects']
                     if not (a.overlay / o['lib_name'].removeprefix('resources/').split('#')[0]).exists()})
    if models:
        if a.target_db is None:
            sys.exit(f'{len(models)} collision models are native 0.26g ones: pass --target-db')
        native = Path(tempfile.mkdtemp(prefix='check-reach-'))
        for model in models:
            for part in ('vertices', 'indices', 'exported_primitives'):
                rel = f'{model}/{part}'
                (native / rel).parent.mkdir(parents=True, exist_ok=True)
                subprocess.run([a.vostok_port, 'extract', '--db', str(a.target_db), '--path', rel, '--output', str(native / rel)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tris = rm.collision_triangles(a.overlay, project, native)
    if a.design:
        tris = np.concatenate([tris, design_triangles(json.loads(Path(a.design[0]).read_text())['maps'][a.design[1]],
                                                      a.target_db, a.vostok_port)])
    spawns = [s[:3] for team in m['deathmatch']['teams'] for s in team]
    named = [(f'team {t} #{i}', s[:3]) for t, team in enumerate(m['deathmatch']['teams']) for i, s in enumerate(team)]
    if a.points:
        with a.points.open() as f:
            for row in csv.DictReader(f, delimiter='\t'):
                named.append((f"point {row.get('point', row.get('id', '?'))}", [float(row[k]) for k in 'xyz']))
    sx = [s[0] for _, s in named]; sz = [s[2] for _, s in named]; sy = [s[1] for _, s in named]
    bounds = (min(sx) - 60, min(sz) - 60, max(sx) + 60, max(sz) + 60)
    fp, fh, w, h = floors(tris, bounds, min(sy) - 15, max(sy) + 25)
    # Standing floors (render_minimap's 1.7 m free): a drop or a jump lands on one, never in a crawl space.
    sp, sh, _, _ = floors(tris, bounds, min(sy) - 15, max(sy) + 25, rm.HEADROOM)
    skey = sp * 1e4 + sh
    at = np.searchsorted(skey, fp * 1e4 + fh - 0.05)
    standing = (at < len(skey)) & (np.abs(skey[np.minimum(at, len(skey) - 1)] - (fp * 1e4 + fh)) <= 0.05)
    x0, _, _, z1 = bounds
    X = x0 + (fp % w + .5) * CELL; Z = z1 - (fp // w + .5) * CELL
    pairs = neighbours(fp, w, h)
    a_, b_ = pairs
    dy = fh[b_] - fh[a_]
    walk = np.abs(dy) <= STEP
    a8, b8 = neighbours(fp, w, h, diagonal=True)
    level = np.abs(fh[b8] - fh[a8]) <= STEP
    stand = np.bincount(a8[level], minlength=len(fp)) >= 5   # floor on 5 of the 8 cells round it: an edge, not a wall top
    # The floor above each in its own cell: a drop needs the lower cell open up to where it falls
    # from, a jump the higher one's cell open over its own floor (nothing drops through a roof).
    above = np.where(np.r_[fp[1:] == fp[:-1], False], np.r_[fh[1:], np.inf], np.inf)
    clear = np.where(dy < 0, above[b_] > fh[a_] + 0.5, above[a_] > fh[b_] + 0.5)
    oneway = (((dy < -STEP) & (dy >= -DROP)) | ((dy > STEP) & (dy <= JUMP) & stand[b_])) & standing[b_] & clear
    n = len(fp)
    W = coo_matrix((np.ones(walk.sum()), (a_[walk], b_[walk])), shape=(n, n)).tocsr()
    D = (W + coo_matrix((np.ones(oneway.sum()), (a_[oneway], b_[oneway])), shape=(n, n))).tocsr()

    def node(x, y, z):
        c = int((z1 - z) // CELL) * w + int((x - x0) // CELL)
        lo, hi = np.searchsorted(fp, c, 'left'), np.searchsorted(fp, c, 'right')
        best = [k for k in range(lo, hi) if abs(fh[k] - y) < 0.6]
        return min(best, key=lambda k: abs(fh[k] - y)) if best else None

    _, lab = connected_components(W, directed=False)
    seeds = [k for k in (node(*s) for s in spawns) if k is not None]
    walked = np.isin(lab, list({lab[k] for k in seeds}))
    fwd = np.zeros(n, bool); back = np.zeros(n, bool)
    for k in seeds:
        if not fwd[k]:
            fwd[breadth_first_order(D, k, directed=True, return_predecessors=False)] = True
    # Back to the spawns' floors: the reverse graph searched from a root joined to every walked floor.
    seed_set = np.nonzero(walked)[0]
    root = n
    rr = coo_matrix((np.ones(len(seed_set)), (np.full(len(seed_set), root), seed_set)), shape=(n + 1, n + 1))
    T = D.T.tocoo()
    R2 = (coo_matrix((T.data, (T.row, T.col)), shape=(n + 1, n + 1)) + rr).tocsr()
    back[breadth_first_order(R2, root, directed=True, return_predecessors=False)[1:]] = True
    dead = fwd & ~back
    bad = 0
    A = CELL * CELL
    print(f"map {a.map} {m['donor_project']}: {len(tris)} collision triangles; walked {len(np.unique(fp[walked])) * A:.0f} m2, "
          f"reached with drops {len(np.unique(fp[fwd])) * A:.0f} m2, floors {fh[walked].min():.1f}..{fh[walked].max():.1f} m")
    off = [(name, s) for name, s in named if (k := node(*s)) is None or not walked[k]]
    for name, s in off:
        print(f"  NOT WALKED: {name} at ({s[0]:.2f}, {s[1]:.2f}, {s[2]:.2f})")
    bad += len(off)
    print(f"  spawns: {len(named) - len(off)} of {len(named)} on walked floors")
    for m2, k, cells in areas(dead, fp, w, pairs, fh):
        # A body (0.76 m across) stands only where 4 x 4 cells are floor: an area
        # with no such place (a gutter, a ledge along a wall) holds nobody.
        ci, cj = fp[cells] % w, fp[cells] // w
        grid = np.zeros((cj.max() - cj.min() + 3, ci.max() - ci.min() + 3), bool)
        up = standing[cells]
        grid[cj[up] - cj.min() + 1, ci[up] - ci.min() + 1] = True
        room = ndimage.binary_erosion(grid, np.ones((4, 4), bool)).any()
        flag = m2 >= 1.0 and room
        bad += flag
        kind = 'DEAD END' if flag else 'dead end, too narrow to stand in' if not room else 'dead-end speck'
        into = np.isin(b_, cells) & ~np.isin(a_, cells) & oneway
        src = a_[into]
        entry = (f"; entered from x {X[src].min():.2f}..{X[src].max():.2f}, z {Z[src].min():.2f}..{Z[src].max():.2f}, "
                 f"y {fh[src].min():.2f}..{fh[src].max():.2f} ({len(src)} edges)") if flag and len(src) else ''
        print(f"  {kind}: {m2:.1f} m2 at ({X[k]:.1f}, {fh[k]:.1f}, {Z[k]:.1f}), "
              f"x {X[cells].min():.1f}..{X[cells].max():.1f}, z {Z[cells].min():.1f}..{Z[cells].max():.1f}{entry}")
    # Reached only by a drop or a jump, and walked back from (not dead ends): where from.
    for m2, k, cells in areas(fwd & ~walked & ~dead, fp, w, pairs, fh):
        if m2 < 10:
            break
        into = np.isin(b_, cells) & walked[a_] & oneway
        src = a_[into]
        frm = f", from ({X[src[0]]:.1f}, {fh[src[0]]:.1f}, {Z[src[0]]:.1f})" if len(src) else ''
        print(f"  off the walked floors, back on foot: {m2:.0f} m2 at ({X[k]:.1f}, {fh[k]:.1f}, {Z[k]:.1f}), "
              f"x {X[cells].min():.0f}..{X[cells].max():.0f}, z {Z[cells].min():.0f}..{Z[cells].max():.0f}{frm}")
    # Not reached: inside the walked floors' frame.
    wx, wz = X[walked], Z[walked]
    inside = (X > wx.min() - MARGIN) & (X < wx.max() + MARGIN) & (Z > wz.min() - MARGIN) & (Z < wz.max() + MARGIN)
    for m2, k, cells in areas(~fwd & inside, fp, w, pairs, fh):
        if m2 < 10:
            break
        print(f"  not reached: {m2:.0f} m2 at ({X[k]:.1f}, {fh[k]:.1f}, {Z[k]:.1f}), floors {fh[cells].min():.1f}..{fh[cells].max():.1f} m")
    if a.trace:
        rows = coo_matrix((np.ones(len(seed_set)), (np.full(len(seed_set), root), seed_set)), shape=(n + 1, n + 1))
        Dd = D.tocoo()
        G = (coo_matrix((Dd.data, (Dd.row, Dd.col)), shape=(n + 1, n + 1)) + rows).tocsr()
        order, pred = breadth_first_order(G, root, directed=True, return_predecessors=True)
    for x, y, z in (map(float, t.split(',')) for t in a.trace):
        goal = node(x, y, z)
        if goal is None or pred[goal] < 0:
            print(f'  trace: no way to ({x}, {y}, {z})')
        else:
            path = [goal]
            while pred[path[-1]] != root:
                path.append(pred[path[-1]])
            path = path[::-1]
            print(f'  trace to ({x}, {y}, {z}): from the walked floor at ({X[path[0]]:.2f}, {fh[path[0]]:.2f}, {Z[path[0]]:.2f}), {len(path)} cells')
            for u, v in zip(path, path[1:]):
                if abs(fh[v] - fh[u]) > STEP:
                    print(f"    {'drop' if fh[v] < fh[u] else 'jump'} {fh[v] - fh[u]:+.2f} m: ({X[u]:.2f}, {fh[u]:.2f}, {Z[u]:.2f}) -> ({X[v]:.2f}, {fh[v]:.2f}, {Z[v]:.2f})")
    if a.ladders:
        for lad in json.loads(a.ladders.read_text()):
            if lad.get('top') is None:
                continue
            near = (X - lad['x']) ** 2 + (Z - lad['z']) ** 2 <= 1.6 ** 2
            top = near & (fh > lad['top'] - 2.2) & (fh < lad['top'] + 0.7)
            state = 'walked' if (top & walked).any() else 'reached' if (top & fwd).any() else 'NOT reached'
            print(f"  ladder {lad['name']} ({lad['x']:.1f}, {lad['z']:.1f}; {lad['foot']:.1f} -> {lad['top']:.1f}): its top {state}")
    if a.png:
        from PIL import Image
        cx, cz = (fp % w), (fp // w)
        i0, i1 = cx[inside].min(), cx[inside].max(); j0, j1 = cz[inside].min(), cz[inside].max()
        img = np.zeros((j1 - j0 + 1, i1 - i0 + 1, 3), np.uint8) + 20
        order = np.argsort(fh)
        if a.png_band:
            lo_, hi_ = map(float, a.png_band.split(','))
            order = order[(fh[order] >= lo_) & (fh[order] <= hi_)]
        for mask, col in ((inside & ~fwd, (90, 60, 110)), (fwd & ~walked, (200, 160, 60)), (walked, None), (dead, (230, 40, 40))):
            k = order[mask[order]]
            if col is None:
                t = np.clip((fh[k] - fh[walked].min()) / max(np.ptp(fh[walked]), 1), 0, 1)
                img[cz[k] - j0, cx[k] - i0] = np.stack([60 + 100 * t, 120 + 120 * t, 80 + 60 * t], 1).astype(np.uint8)
            else:
                img[cz[k] - j0, cx[k] - i0] = col
        for _, s in named:
            c = node(*s)
            if c is not None:
                img[max(cz[c] - j0 - 1, 0):cz[c] - j0 + 2, max(cx[c] - i0 - 1, 0):cx[c] - i0 + 2] = (255, 255, 255)
        Image.fromarray(img).save(a.png)
        print(f'  picture: {a.png} (green by height: walked; amber: reached only by a drop or a jump; red: dead end; purple: not reached)')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
