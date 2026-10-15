#!/usr/bin/env python3
"""Rank clear floor spots for gameplay-fill props around a centre point.

Usage: find_spots.py BUNDLE_OVERLAY PROJECT X Y Z --rmin R --rmax R [--half H] [--avoid-spawns M]
A spot needs movement-collision support within 0.3 m of Y and no movement
collision in a square of half size H from 0.25 to 1.8 m above it (exact
triangle tests, oriented primitives). Spots closer than --avoid-spawns to a
respawn point are skipped. Candidates for screenshots, not final placements.
"""
import argparse, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from audit_respawns import World, tri_box, config, intersects

def clear(world, x, f, z, half):
    lo, hi = [x-half, f+0.25, z-half], [x+half, f+1.8, z+half]
    for n, o, p in world.near(lo, hi):
        if p['kind'] == 'triangle':
            if tri_box(p['vertices'], lo, hi): return False
        else:
            for dx in (-half, 0, half):
                for dz in (-half, 0, half):
                    if world.primitive_hit(o, p, x+dx, f, z+dz, 0.05): return False
    return True

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
    ap.add_argument('xyz', type=float, nargs=3)
    ap.add_argument('--rmin', type=float, default=3); ap.add_argument('--rmax', type=float, default=10)
    ap.add_argument('--half', type=float, default=0.9); ap.add_argument('--avoid-spawns', type=float, default=2.5)
    ap.add_argument('--top', type=int, default=8)
    a = ap.parse_args()
    project = config(a.project.read_bytes()); world = World(a.root, project)
    spawns = [p['position'] for p in project['server_objects']['respawn_points']]
    cx, cy, cz = a.xyz; out = []
    steps = int(a.rmax * 2)
    for i in range(-steps, steps + 1):
        for j in range(-steps, steps + 1):
            x, z = cx + i * 0.5, cz + j * 0.5
            r = math.hypot(x - cx, z - cz)
            if not a.rmin <= r <= a.rmax: continue
            if min(math.hypot(x - s[0], z - s[2]) for s in spawns) < a.avoid_spawns: continue
            fl = world.floor(x, cy, z, below=0.4, above=0.4)
            if not fl: continue
            if clear(world, x, fl[0], z, a.half):
                out.append(dict(position=[round(x, 2), round(fl[0], 3), round(z, 2)], distance=round(r, 2), support=fl[1]))
    out.sort(key=lambda s: s['distance'])
    print(json.dumps(out[:a.top], indent=0))
    print(f'{len(out)} clear spots', file=sys.stderr)

if __name__ == '__main__':
    main()
