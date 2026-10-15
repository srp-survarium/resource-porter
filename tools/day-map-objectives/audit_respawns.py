#!/usr/bin/env python3
"""Offline audit of every player respawn point against movement collision.

Usage: audit_respawns.py BUNDLE_OVERLAY PROJECT [--native-db DB] [--output JSON]
Checks support under the feet, a standing capsule (r 0.35 m, 0.25..1.8 m above
the feet, exact triangle tests; primitives as their oriented boxes), what the
spawn faces at eye height within 2 m, and the map-border volumes. Results are
candidates for review, not a runtime proof.
"""
import argparse, json, math, sys, os, subprocess, collections
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config, rotation, transform, collision_parts, intersects

def inv(v, r): return [sum(v[j]*r[i][j] for j in range(3)) for i in range(3)]

def tri_box(tri, lo, hi):
    """Separating-axis test of a triangle against an axis-aligned box."""
    c = [(lo[i]+hi[i])/2 for i in range(3)]; h = [(hi[i]-lo[i])/2 for i in range(3)]
    v = [[p[i]-c[i] for i in range(3)] for p in tri]
    e = [[v[(k+1) % 3][i]-v[k][i] for i in range(3)] for k in range(3)]
    def sep(axis):
        if all(abs(a) < 1e-12 for a in axis): return False
        p = [sum(axis[i]*q[i] for i in range(3)) for q in v]
        r = sum(h[i]*abs(axis[i]) for i in range(3))
        return min(p) > r or max(p) < -r
    for i in range(3):
        if sep([1 if j == i else 0 for j in range(3)]): return False
    n = [e[0][1]*e[1][2]-e[0][2]*e[1][1], e[0][2]*e[1][0]-e[0][0]*e[1][2], e[0][0]*e[1][1]-e[0][1]*e[1][0]]
    if sep(n): return False
    for a in range(3):
        for k in range(3):
            ax = [0, 0, 0]; ax[a] = 1
            cr = [ax[1]*e[k][2]-ax[2]*e[k][1], ax[2]*e[k][0]-ax[0]*e[k][2], ax[0]*e[k][1]-ax[1]*e[k][0]]
            if sep(cr): return False
    return True

class World:
    def __init__(self, root, project, native_db=None):
        self.parts = []
        self.root = root
        for obj in project['collision_objects']:
            if not (obj['cgroup'] & 2 and obj['cmask'] & 4): continue
            try: ps = list(collision_parts(root, obj))
            except FileNotFoundError: continue
            name = obj['lib_name'].split('/models/')[-1]
            for p in ps: self.parts.append((name, obj, p))
        print(f'{len(self.parts)} movement collision parts', file=sys.stderr)
    def near(self, lo, hi):
        return [(n, o, p) for n, o, p in self.parts if intersects(p['bounds'], [lo, hi])]
    def floor(self, x, y, z, below=1.0, above=0.4):
        best = None
        for n, o, p in self.near([x-.01, y-below, z-.01], [x+.01, y+above, z+.01]):
            if p['kind'] == 'triangle':
                (x1, y1, z1), (x2, y2, z2), (x3, y3, z3) = p['vertices']
                d = (z2-z3)*(x1-x3)+(x3-x2)*(z1-z3)
                if abs(d) < 1e-9: continue
                a = ((z2-z3)*(x-x3)+(x3-x2)*(z-z3))/d; b = ((z3-z1)*(x-x3)+(x1-x3)*(z-z3))/d; c = 1-a-b
                if min(a, b, c) < -1e-6: continue
                h = a*y1+b*y2+c*y3
            else:
                h = self.primitive_top(o, p, x, z, y - below, y + above)
                if h is None: continue
            if y-below <= h <= y+above and (best is None or h > best[0]): best = (h, n)
        return best
    def capsule_hits(self, x, f, z, r=0.35):
        lo, hi = [x-r, f+0.25, z-r], [x+r, f+1.8, z+r]
        hits = set()
        for n, o, p in self.near(lo, hi):
            if p['kind'] == 'triangle':
                if tri_box(p['vertices'], lo, hi): hits.add(n)
            elif self.primitive_hit(o, p, x, f, z, r):
                hits.add(n + f" (primitive {p['index']})")
        return sorted(hits)
    def primitive_top(self, obj, part, x, z, lo, hi, step=0.01):
        """Highest height in [lo, hi] where the vertical line at (x, z) is inside the primitive."""
        h = hi
        while h >= lo:
            if self.primitive_distance(obj, part, x, h, z) <= 0.0: return h
            h -= step
        return None
    def primitive_distance(self, obj, part, x, y, z):
        path = obj['lib_name'].removeprefix('resources/').split('#')[0]
        q = config((self.root / path / 'exported_primitives').read_bytes())['primitives'][part['index']]; d = q['scale']
        o = inv([x - obj['position'][0], y - obj['position'][1], z - obj['position'][2]], rotation(obj['rotation']))
        l = inv([o[i] - q['position'][i] for i in range(3)], rotation(q['rotation']))
        if q['type'] == 0: return max(math.sqrt(sum(c*c for c in l)) - d[0], 0)
        if q['type'] == 2: return math.hypot(max(math.hypot(l[0], l[2]) - d[0], 0), max(abs(l[1]) - d[1], 0))
        if q['type'] == 3:
            cy = max(-d[1]/2, min(d[1]/2, l[1])); return max(math.sqrt(l[0]**2 + (l[1]-cy)**2 + l[2]**2) - d[0], 0)
        return math.sqrt(sum(max(abs(l[i]) - d[i], 0)**2 for i in range(3)))
    def primitive_hit(self, obj, part, x, f, z, r):
        """Exact capsule test: segment samples against the oriented primitive
        (box half extents, sphere radius, Y-up cylinder [x,y,x] as in 0.26)."""
        path = obj['lib_name'].removeprefix('resources/').split('#')[0]
        prims = config((self.root / path / 'exported_primitives').read_bytes())['primitives']
        q = prims[part['index']]; d = q['scale']
        Ro, Rp = rotation(obj['rotation']), rotation(q['rotation'])
        for k in range(0, 31):
            y = f + 0.25 + r + (1.8 - 0.25 - 2*r) * k / 30
            o = inv([x - obj['position'][0], y - obj['position'][1], z - obj['position'][2]], Ro)
            l = inv([o[i] - q['position'][i] for i in range(3)], Rp)
            if q['type'] == 0: dist = max(math.sqrt(sum(c*c for c in l)) - d[0], 0)
            elif q['type'] == 2:
                dr = max(math.hypot(l[0], l[2]) - d[0], 0); dy = max(abs(l[1]) - d[1], 0); dist = math.hypot(dr, dy)
            elif q['type'] == 3:
                cy = max(-d[1]/2, min(d[1]/2, l[1])); dist = max(math.sqrt(l[0]**2 + (l[1]-cy)**2 + l[2]**2) - d[0], 0)
            else: dist = math.sqrt(sum(max(abs(l[i]) - d[i], 0)**2 for i in range(3)))
            if dist < r: return True
        return False
    def facing(self, x, f, z, yaw, reach=2.0):
        dx, dz = -math.sin(yaw), math.cos(yaw)
        for step in range(1, int(reach / 0.1) + 1):
            t = step * 0.1; px, pz = x + dx*t, z + dz*t
            lo, hi = [px-0.05, f+1.4, pz-0.05], [px+0.05, f+1.6, pz+0.05]
            for n, o, p in self.near(lo, hi):
                if p['kind'] == 'triangle':
                    if tri_box(p['vertices'], lo, hi): return round(t, 1), n
                elif self.primitive_hit(o, p, px, f + 1.15, pz, 0.05):
                    return round(t, 1), n
        return None

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
    ap.add_argument('--output', type=Path)
    a = ap.parse_args()
    project = config(a.project.read_bytes())
    world = World(a.root, project)
    border = [p for n, o, p in world.parts if 'level_border' in n]
    rows = []
    points = project['server_objects']['respawn_points']
    teams = collections.defaultdict(list)
    for p in points: teams[p['team']].append(p['position'])
    centre = {t: [sum(q[i] for q in v)/len(v) for i in range(3)] for t, v in teams.items()}
    for i, sp in enumerate(points):
        x, y, z = sp['position']; yaw = sp['rotation'][1] if abs(sp['rotation'][0]) < 1 else math.pi - sp['rotation'][1]
        fl = world.floor(x, y, z)
        f = fl[0] if fl else y
        hits = world.capsule_hits(x, f, z)
        face = world.facing(x, f, z, yaw)
        inside_border = any(intersects(p['bounds'], [[x-.01, f+.5, z-.01], [x+.01, f+1.5, z+.01]]) for p in border)
        other = min((math.dist(sp['position'], c), t) for t, c in centre.items())
        problems = []
        if fl is None: problems.append('no support within 1 m below')
        elif y - f > 0.3: problems.append(f'spawn {y-f:.2f} m above support')
        elif f - y > 0.1: problems.append(f'spawn {f-y:.2f} m below its support surface')
        if hits: problems.append('capsule intersects ' + ', '.join(hits))
        if face and face[0] < 0.8: problems.append(f'faces {face[1]} at {face[0]} m')
        if inside_border: problems.append('inside map-border volume')
        if other[1] != sp['team']: problems.append('closer to the other team centroid')
        rows.append(dict(index=i, name=sp['full_name'], team=sp['team'], point_id=sp['point_id'], position=sp['position'], rotation=sp['rotation'],
                         yaw=yaw, support=None if fl is None else dict(height=round(f, 3), collision=fl[1]), facing=face, problems=problems))
    report = dict(scope=__doc__.split('\n\n')[1], points=rows,
                  summary=dict(total=len(rows), flagged=sum(bool(r['problems']) for r in rows)))
    if a.output: a.output.write_text(json.dumps(report, indent=1) + '\n')
    for r in rows:
        if r['problems']: print(r['index'], r['team'], r['name'], [round(v, 2) for v in r['position']], r['problems'])
    print(json.dumps(report['summary']))

if __name__ == '__main__':
    main()
