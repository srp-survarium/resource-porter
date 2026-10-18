#!/usr/bin/env python3
"""Native ammo crates (item/quest/container_closed) at their exact native transforms, against a converted
map's movement collision, offline.

Usage: check_native_crates.py ROOT PROJECT X,Y,Z:RX,RY,RZ [...]

For each crate (position and engine XYZ Euler rotation; positions are local x basis rows, as the engine): the
floor under its four bottom corners and its centre (the v0.11 floor height there), and every collision the
crate's box (container_closed's: half 0.74 x 0.57, 0.68 tall, origin at the bottom) would intersect when its
origin sits on that floor: sampled every 0.08 m inside the box from 6 cm up (a 3 cm probe), reporting each
model, the hit's extent in the crate's frame and how far it reaches into the crate (the depth from the
nearest side face, or from the top). Prints JSON.
"""
import json, math, sys, collections
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config, tri_box

HX, HZ, H = 0.74, 0.57, 0.68


def e2b(a):
    sx, cx = math.sin(a[0]), math.cos(a[0]); sy, cy = math.sin(a[1]), math.cos(a[1]); sz, cz = math.sin(a[2]), math.cos(a[2])
    return [[cy * cz, -cy * sz, sy], [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy], [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy]]


def main():
    root, project = Path(sys.argv[1]), config(Path(sys.argv[2]).read_bytes())
    full = FastWorld(root, project)
    out = []
    for spec in sys.argv[3:]:
        p, r = spec.split(':'); p = [float(v) for v in p.split(',')]; r = [float(v) for v in r.split(',')]
        R = e2b(r)
        w = FastWorld(root, parts=full.near([p[0] - 3, p[1] - 3, p[2] - 3], [p[0] + 3, p[1] + 4, p[2] + 3]))
        world = lambda l: [sum(l[i] * R[i][j] for i in range(3)) + p[j] for j in range(3)]
        # the floor under the bottom corners and the centre
        floors = {}
        for name, l in (('centre', [0, 0, 0]), ('+x+z', [HX, 0, HZ]), ('+x-z', [HX, 0, -HZ]), ('-x+z', [-HX, 0, HZ]), ('-x-z', [-HX, 0, -HZ])):
            q = world(l); f = w.floor(q[0], q[1], q[2], below=0.6, above=0.6)
            floors[name] = None if not f else dict(height=round(f[0], 3), model=f[1].split('/models/')[-1], corner_y=round(q[1], 3))
        snap = floors['centre']['height'] if floors['centre'] else p[1]
        dy = snap - p[1]
        hits = collections.defaultdict(lambda: dict(n=0, lo=[9, 9, 9], hi=[-9, -9, -9], depth=0.0))
        steps = lambda h, s: [(-h + s * i) for i in range(int(2 * h / s) + 1)]
        for lx in steps(HX, 0.08):
            for lz in steps(HZ, 0.08):
                for ly in [0.06 + 0.08 * k for k in range(int((H - 0.06) / 0.08) + 1)]:
                    q = world([lx, ly, lz]); q[1] += dy
                    lo, hi = [q[0] - .03, q[1] - .03, q[2] - .03], [q[0] + .03, q[1] + .03, q[2] + .03]
                    for n, o, part in w.near(lo, hi):
                        hit = tri_box(part['vertices'], lo, hi) if part['kind'] == 'triangle' else w.primitive_distance(o, part, *q) <= 0.03
                        if not hit: continue
                        m = n.split('/models/')[-1]
                        e = hits[m]; e['n'] += 1
                        for i, v in enumerate((lx, ly, lz)): e['lo'][i] = min(e['lo'][i], v); e['hi'][i] = max(e['hi'][i], v)
                        e['depth'] = max(e['depth'], min(HX - abs(lx), HZ - abs(lz), H - ly))
        out.append(dict(position=p, rotation=r, local_up=[round(v, 3) for v in R[1]], snapped_y=round(snap, 3), dy=round(dy, 3), floors=floors,
                        hits={m: dict(samples=e['n'], local_lo=[round(v, 2) for v in e['lo']], local_hi=[round(v, 2) for v in e['hi']],
                                      deepest_into_crate=round(e['depth'], 2)) for m, e in hits.items()}))
    print(json.dumps(out, indent=1))


if __name__ == '__main__':
    main()
