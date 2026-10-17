#!/usr/bin/env python3
"""Check every walker ramp of the deathmatch spec against 0.26g's walker.

    check_ladder_ramps.py --bundle OVERLAY --spec SPEC [--step 0.45]

Reachability from the respawns over the converted collision plus the walker
ramps (render_minimap.play_area, 0.25 m cells), with neighbouring floors at
most STEP apart: 0.26g's controller steps 0.6 m (`cc_step_height`), walks 50
deg and stands on 40 deg; 0.45 per 0.25 m cell is the margin used here. For
each ramp, its foot, its middle and its top (and 0.4 m past the top) must be
reachable floors at the ramp's height. Hatch routes: with --target-db, the
native props (crates stacked as steps to a plank) are walkable and each
stack's top must be reachable; every cut box must hold no collision left.
Prints one line per ramp, prop and cut; exit 1 if any check fails.
"""
import argparse, json, math, subprocess, sys, tempfile
from pathlib import Path
import numpy as np
sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_minimap as rm


def column(tris, x, z):
    """Heights where the vertical line at (x, z) crosses the triangles."""
    a, b, c = tris[:, 0], tris[:, 1], tris[:, 2]
    d = (b[:, 2] - c[:, 2]) * (a[:, 0] - c[:, 0]) + (c[:, 0] - b[:, 0]) * (a[:, 2] - c[:, 2])
    ok = np.abs(d) > 1e-12
    d = np.where(ok, d, 1)
    l1 = ((b[:, 2] - c[:, 2]) * (x - c[:, 0]) + (c[:, 0] - b[:, 0]) * (z - c[:, 2])) / d
    l2 = ((c[:, 2] - a[:, 2]) * (x - c[:, 0]) + (a[:, 0] - c[:, 0]) * (z - c[:, 2])) / d
    l3 = 1 - l1 - l2
    hit = ok & (l1 >= 0) & (l2 >= 0) & (l3 >= 0)
    return (l1 * a[:, 1] + l2 * b[:, 1] + l3 * c[:, 1])[hit].tolist()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--step', type=float, default=0.45)
    ap.add_argument('--target-db', type=Path, help='0.26g resources.db holding the native prop models')
    ap.add_argument('--vostok-port', default='vostok-port')
    a = ap.parse_args()
    bad = 0
    maps = json.loads(a.spec.read_text())['maps']
    native = None
    if any(m.get('props') for m in maps):
        if a.target_db is None:
            sys.exit('the spec places native props: pass --target-db')
        native = Path(tempfile.mkdtemp(prefix='ladder-ramps-'))
        for model in sorted({p['model'] for m in maps for p in m.get('props', [])}):
            for part in ('collision', 'walker_collision'):
                rel = f'models/{model}.model/{part}/exported_primitives'
                (native / rel).parent.mkdir(parents=True, exist_ok=True)
                subprocess.run([a.vostok_port, 'extract', '--db', str(a.target_db), '--path', rel,
                                '--output', str(native / rel)], check=True, stdout=subprocess.DEVNULL)
    for m in maps:
        ramps = m.get('walker_ramps', [])
        if not ramps:
            continue
        project = rm.config((a.bundle / 'projects' / f"v010b_{m['donor_project']}" / 'day_4' / 'client_project').read_bytes())
        spawns = [r[:3] for r in m['deathmatch']['respawns']]
        sx = [s[0] for s in spawns]; sz = [s[2] for s in spawns]
        bounds = (min(sx) - 150, min(sz) - 150, max(sx) + 150, max(sz) + 150)
        tris = rm.collision_triangles(a.bundle, project, native)
        props = m.get('props', [])
        # Crate treads (0.54 m) alias on 0.25 m cells: with props, a floor
        # also counts when it is reachable on the grid shifted half a cell.
        phases = (0.0, rm.CELL / 2) if props else (0.0,)
        grids = []
        for shift in phases:
            b = (bounds[0] - shift, bounds[1], bounds[2], bounds[3] + shift)
            grids.append((b, rm.play_area(tris, spawns, b, step=a.step, all_floors=True, slab_tops=True)[2]))
        def reach(x, y, z, tol=0.35):
            for (x0, _, _, z1), (rp, rh, w) in grids:
                c = int((z1 - z) // rm.CELL) * w + int((x - x0) // rm.CELL)
                lo, hi = np.searchsorted(rp, c, 'left'), np.searchsorted(rp, c, 'right')
                if any(abs(rh[k] - y) < tol for k in range(lo, hi)):
                    return True
            return False
        def on_prop(x, z):
            return any(abs(q['position'][0] - x) < 0.45 and abs(q['position'][2] - z) < 0.3 for q in props)
        for r in ramps:
            f, t = r['from'], r['to']
            d = [t[i] - f[i] for i in range(3)]; run = math.hypot(d[0], d[2])
            u = [d[0] / run, 0, d[2] / run]
            # A ramp that starts on a crate stack has its foot on the crate.
            pts = {'foot': list(f) if on_prop(f[0], f[2]) else [f[0] - u[0] * 0.3, f[1], f[2] - u[2] * 0.3],
                   'middle': [(f[i] + t[i]) / 2 for i in range(3)],
                   'top': list(t),
                   'past top': [t[0] + u[0] * 0.4, t[1], t[2] + u[2] * 0.4]}
            res = {k: reach(*p) for k, p in pts.items()}
            ok = res['foot'] and res['middle'] and res['top']  # 'past top' is informational
            bad += not ok
            deg = math.degrees(math.atan2(d[1], run))
            print(f"{m['id']} {r['name']:20s} {deg:5.1f} deg rise {d[1]:5.2f} run {run:5.2f}  "
                  + '  '.join(f"{k}:{'ok' if v else 'NO'}" for k, v in res.items()))
        # Hatch routes: from the lowest crate of the stack a plank starts on,
        # a walk confined to 3 m around the plank must reach both the floor
        # under the crates and the floor past the plank's top.
        tops = []
        for p in props:
            x, y, z = p['position']
            if not any(math.hypot(q['position'][0] - x, q['position'][2] - z) < 0.1 and q['position'][1] > y for q in props):
                hs = [h for h in column(tris, x, z) if y < h < y + 0.6]
                if hs:
                    tops.append((x, max(hs), z))
        for r in ramps:
            f, t = r['from'], r['to']
            if not on_prop(f[0], f[2]):
                continue
            stack = [q for q in tops if math.hypot(q[0] - f[0], q[2] - f[2]) < 2.5]
            start = min(stack, key=lambda q: q[1])
            d = [t[i] - f[i] for i in range(3)]; run = math.hypot(d[0], d[2])
            past = [t[0] + d[0] / run * 0.4, t[1], t[2] + d[2] / run * 0.4]
            box = (min(f[0], t[0], start[0]) - 3, min(f[2], t[2], start[2]) - 3,
                   max(f[0], t[0], start[0]) + 3, max(f[2], t[2], start[2]) + 3)
            ok_up = ok_down = False
            for shift in phases:
                b = (box[0] - shift, box[1], box[2], box[3] + shift)
                rp, rh, w = rm.play_area(tris, [start], b, step=a.step, all_floors=True, slab_tops=True)[2]
                cell = lambda x, z: int((b[3] - z) // rm.CELL) * w + int((x - b[0]) // rm.CELL)
                c = cell(past[0], past[2])
                lo, hi = np.searchsorted(rp, c, 'left'), np.searchsorted(rp, c, 'right')
                ok_up |= any(abs(rh[k] - past[1]) < 0.35 for k in range(lo, hi))
                ok_down |= bool((rh < start[1] - 0.3).any())
            bad += not (ok_up and ok_down)
            print(f"{m['id']} route {r['name']:20s} crate top {start[1]:.2f}: floor below {'ok' if ok_down else 'NO'}, "
                  f"past plank top {'ok' if ok_up else 'NO'}")
        for p in props:
            # Only the top of each stack is a floor; its top face is within a
            # step of its origin.
            x, y, z = p['position']
            if any(math.hypot(q['position'][0] - x, q['position'][2] - z) < 0.1 and q['position'][1] > y for q in props):
                continue
            tops = [h for h in column(tris, x, z) if y < h < y + 0.6]
            ok = bool(tops) and reach(x, max(tops), z)
            bad += not ok
            print(f"{m['id']} prop {p['name']:20s} top {max(tops) if tops else float('nan'):5.2f}  {'ok' if ok else 'NO'}")
        # The map's own static collision: no walker ramps (srp/) or props.
        static = rm.collision_triangles(a.bundle, {'collision_objects': [
            o for o in project['collision_objects'] if '/ported/' in o['lib_name'] and '/srp/' not in o['lib_name']]})
        for c in m.get('cuts', []):
            lo = np.array(c['centre']) - np.array(c['half']) + 0.02
            hi = np.array(c['centre']) + np.array(c['half']) - 0.02
            centroid = static.mean(axis=1)
            left = int(np.all((centroid > lo) & (centroid < hi), axis=1).sum())
            bad += left > 0
            print(f"{m['id']} cut {c['name']:20s} collision faces left inside: {left}  {'ok' if not left else 'NO'}")
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
