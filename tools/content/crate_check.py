#!/usr/bin/env python3
"""crate_check.py ROOT WORLD_PROJECT CRATES_PROJECT [--spots JSON] > report.json

Each ammo crate (render visual item/quest/container_closed of CRATES_PROJECT, or each {position, rotation} of
--spots) against the movement collision of WORLD_PROJECT (the map without the crates) in ROOT:
- floor: the crate's collision box bottom (native exported primitive: half 0.734 x 0.339 x 0.567) sampled 5 x 5;
  gap above the floor (floating) and depth below it, per sample;
- inside: movement collision inside the box from 6 cm above its bottom to its top (5 x 5 x 3 samples, r 4 cm);
- spawns: every respawn point of CRATES_PROJECT: horizontal distance from the point to the box footprint minus the
  standing capsule radius 0.35 (negative: the capsule overlaps the crate), when their heights overlap;
- room around it: from each side face, at 0.5 and 1.2 m above the floor, how far the floor stays free (no
  collision within 12 cm, a floor within 0.6 m), up to 4 m: the width left beside the crate."""
import argparse, json, math, sys
from pathlib import Path
import os
T = os.environ.get('TOOLS', '/mnt/hdd/Projects/release-20/crates-021d/tools-a0b5933/tools') + '/day-map-objectives'
sys.path.insert(0, T); sys.path.insert(0, T + '/../chemical-plant')
from fast_world import FastWorld
from audit_respawns import config, tri_box
from audit_collision_region import rotation, transform

C, H = [-0.002333194, 0.33881423, -0.003938198], [0.73424494, 0.33861992, 0.56657296]

def main():
    ap = argparse.ArgumentParser(); ap.add_argument('root', type=Path); ap.add_argument('world', type=Path); ap.add_argument('crates', type=Path)
    ap.add_argument('--spots', type=Path); a = ap.parse_args()
    wp, cp = config(a.world.read_bytes()), config(a.crates.read_bytes())
    crates = json.loads(a.spots.read_text()) if a.spots else [dict(position=v['position'], rotation=v['rotation']) for v in cp['render_visuals'] if v['lib_name'] == 'item/quest/container_closed']
    full = FastWorld(a.root, wp)
    spawns = cp['server_objects']['respawn_points']
    out = []
    for c in crates:
        R, t = rotation(c['rotation']), list(c['position'])
        w = FastWorld(a.root, parts=full.near([t[0] - 6, t[1] - 3, t[2] - 6], [t[0] + 6, t[1] + 4, t[2] + 6]))
        if c.get('snap'):   # stand it on the highest floor under its 5 x 5 bottom samples (level crate)
            hs = [w.floor(t[0] + R[0][0] * H[0] * (i / 2 - 1) + R[2][0] * H[2] * (j / 2 - 1), t[1], t[2] + R[0][2] * H[0] * (i / 2 - 1) + R[2][2] * H[2] * (j / 2 - 1), below=0.6, above=0.6) for i in range(5) for j in range(5)]
            hs = [h[0] for h in hs if h]
            if hs: t[1] = round(max(hs), 3)
        def solid(x, y, z, r=0.04):
            lo, hi = [x - r, y - r, z - r], [x + r, y + r, z + r]
            for n, o, p in w.near(lo, hi):
                if p['kind'] == 'triangle':
                    if tri_box(p['vertices'], lo, hi): return n
                elif w.primitive_distance(o, p, x, y, z) <= r: return n
            return None
        L = lambda lx, ly, lz: transform([C[0] + lx, C[1] + ly, C[2] + lz], R, t)
        gaps = []
        for i in range(5):
            for j in range(5):
                b = L(H[0] * (i / 2 - 1), -H[1], H[2] * (j / 2 - 1))
                f = w.floor(b[0], b[1], b[2], below=0.5, above=0.3)
                gaps.append(None if f is None else round(b[1] - f[0], 3))
        inside = set()
        for i in range(5):
            for j in range(5):
                for k in range(3):
                    lyb = -H[1] + 0.06 + (2 * H[1] - 0.06) * k / 2
                    q = L(H[0] * (i / 2 - 1) * 0.95, lyb, H[2] * (j / 2 - 1) * 0.95)
                    n = solid(*q)
                    if n: inside.add(n)
        bottom = L(0, -H[1], 0)[1]
        ux, uz = [R[0][0], R[0][2]], [R[2][0], R[2][2]]
        nx, nz = math.hypot(*ux), math.hypot(*uz); ux = [v / nx for v in ux]; uz = [v / nz for v in uz]
        clear = []
        for sp in spawns:
            p = sp['position']
            if not (p[1] - 0.3 < bottom + 2 * H[1] and p[1] + 1.8 > bottom): continue
            dx, dz = p[0] - t[0], p[2] - t[2]
            lx, lz = dx * ux[0] + dz * ux[1], dx * uz[0] + dz * uz[1]
            d = math.hypot(max(abs(lx) - H[0], 0), max(abs(lz) - H[2], 0)) - 0.35
            clear.append((round(d, 2), sp['full_name']))
        clear.sort()
        room = {}
        for name, axis, half in (('+x', ux, H[0]), ('-x', [-ux[0], -ux[1]], H[0]), ('+z', uz, H[2]), ('-z', [-uz[0], -uz[1]], H[2])):
            free = 0.0
            for s in range(1, 41):
                d = s * 0.1; px, pz = t[0] + axis[0] * (half + d), t[2] + axis[1] * (half + d)
                f = w.floor(px, bottom, pz, below=0.6, above=0.6)
                if not f or solid(px, f[0] + 0.5, pz, 0.12) or solid(px, f[0] + 1.2, pz, 0.12): break
                free = d
            room[name] = round(free, 1)
        good = [g for g in gaps if g is not None]
        out.append(dict(position=t, rotation=c['rotation'], floor_samples=len(good), no_floor=gaps.count(None),
                        max_float=round(max(good), 3) if good else None, max_sunk=round(-min(good), 3) if good else None,
                        inside=sorted(inside), nearest_spawns=clear[:3], room_beside=room))
    print(json.dumps(out, indent=1))

main()
