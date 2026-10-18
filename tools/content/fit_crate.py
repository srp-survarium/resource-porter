#!/usr/bin/env python3
"""fit_crate.py ROOT PROJECT X Z HEADING_FROM_ROTATION_JSON [--y-near Y]

The 0.21d floor under an ammo crate at (X, Z) turned to the heading of a native crate rotation (its local x
projected on the ground): every floor level found from 60 m down to -20 m at (X, Z), and for the one nearest
--y-near (default the highest), the 5 x 5 footprint samples (crate collision half 0.734 x 0.567) fitted with a
plane: centre height, slopes along the crate's local x and z, residual, heading phi (local x along (cos phi,
sin phi) in (x, z)). Prints JSON for make_fill (position, yaw, slope_x, slope_z)."""
import argparse, json, math, os, sys
from pathlib import Path
T = os.environ.get('TOOLS', '/mnt/hdd/Projects/release-20/crates-021d/tools-a0b5933/tools') + '/day-map-objectives'
sys.path.insert(0, T); sys.path.insert(0, T + '/../chemical-plant')
from fast_world import FastWorld
from audit_respawns import config
from audit_collision_region import rotation

ap = argparse.ArgumentParser(); ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
ap.add_argument('x', type=float); ap.add_argument('z', type=float); ap.add_argument('rotation', type=json.loads)
ap.add_argument('--y-near', type=float); a = ap.parse_args()
R = rotation(a.rotation); phi = math.atan2(R[0][2], R[0][0])
full = FastWorld(a.root, config(a.project.read_bytes()))
w = FastWorld(a.root, parts=full.near([a.x - 3, -30, a.z - 3], [a.x + 3, 80, a.z + 3]))
levels, y = [], 60.0
while y > -20:
    f = w.floor(a.x, y, a.z, below=1.0, above=0.0)
    if f: levels.append(round(f[0], 3)); y = f[0] - 0.3
    else: y -= 1.0
base = min(levels, key=lambda h: abs(h - a.y_near)) if a.y_near is not None else levels[0]
c, s = math.cos(phi), math.sin(phi); HX, HZ = 0.734, 0.567
samples = []
for i in range(5):
    for j in range(5):
        lx, lz = HX * (i / 2 - 1), HZ * (j / 2 - 1)
        px, pz = a.x + c * lx - s * lz, a.z + s * lx + c * lz
        f = w.floor(px, base, pz, below=0.6, above=0.6)
        if f: samples.append((lx, lz, f[0]))
n = len(samples); mh = sum(q[2] for q in samples) / n
gx = sum(q[0] * (q[2] - mh) for q in samples) / sum(q[0] ** 2 for q in samples)
gz = sum(q[1] * (q[2] - mh) for q in samples) / sum(q[1] ** 2 for q in samples)
res = max(abs(q[2] - (mh + gx * q[0] + gz * q[1])) for q in samples)
print(json.dumps(dict(levels=levels, floor_samples=n, position=[a.x, round(mh, 3), a.z], yaw=round(phi, 4),
                      slope_x=round(gx, 4), slope_z=round(gz, 4), residual=round(res, 3))))
