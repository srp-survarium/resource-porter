#!/usr/bin/env python3
"""Check every walker ramp of the deathmatch spec against 0.26g's walker.

    check_ladder_ramps.py --bundle OVERLAY --spec SPEC [--step 0.45]

Reachability from the respawns over the converted collision plus the walker
ramps (render_minimap.play_area, 0.25 m cells), with neighbouring floors at
most STEP apart: 0.26g's controller steps 0.6 m (`cc_step_height`), walks 50
deg and stands on 40 deg; 0.45 per 0.25 m cell is the margin used here. For
each ramp, its foot, its middle and its top (and 0.4 m past the top) must be
reachable floors at the ramp's height. Prints one line per ramp; exit 1 if
any end is unreachable.
"""
import argparse, json, math, sys
from pathlib import Path
import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_minimap as rm


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--step', type=float, default=0.45)
    a = ap.parse_args()
    bad = 0
    for m in json.loads(a.spec.read_text())['maps']:
        ramps = m.get('walker_ramps', [])
        if not ramps:
            continue
        project = rm.config((a.bundle / 'projects' / f"v010b_{m['donor_project']}" / 'day_4' / 'client_project').read_bytes())
        spawns = [r[:3] for r in m['deathmatch']['respawns']]
        sx = [s[0] for s in spawns]; sz = [s[2] for s in spawns]
        bounds = (min(sx) - 150, min(sz) - 150, max(sx) + 150, max(sz) + 150)
        x0, z0, x1, z1 = bounds
        _, _, (rp, rh, w) = rm.play_area(rm.collision_triangles(a.bundle, project), spawns, bounds, step=a.step, all_floors=True)
        def reach(x, y, z, tol=0.35):
            c = int((z1 - z) // rm.CELL) * w + int((x - x0) // rm.CELL)
            lo, hi = np.searchsorted(rp, c, 'left'), np.searchsorted(rp, c, 'right')
            return any(abs(rh[k] - y) < tol for k in range(lo, hi))
        for r in ramps:
            f, t = r['from'], r['to']
            d = [t[i] - f[i] for i in range(3)]; run = math.hypot(d[0], d[2])
            u = [d[0] / run, 0, d[2] / run]
            pts = {'foot': [f[0] - u[0] * 0.3, f[1], f[2] - u[2] * 0.3],
                   'middle': [(f[i] + t[i]) / 2 for i in range(3)],
                   'top': list(t),
                   'past top': [t[0] + u[0] * 0.4, t[1], t[2] + u[2] * 0.4]}
            res = {k: reach(*p) for k, p in pts.items()}
            ok = all(res.values())
            bad += not ok
            deg = math.degrees(math.atan2(d[1], run))
            print(f"{m['id']} {r['name']:20s} {deg:5.1f} deg rise {d[1]:5.2f} run {run:5.2f}  "
                  + '  '.join(f"{k}:{'ok' if v else 'NO'}" for k, v in res.items()))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
