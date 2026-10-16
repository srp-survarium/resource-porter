#!/usr/bin/env python3
"""Place a native charger (two stacked box_atlant12) and an ammo crate against a wall.

Usage: place_base_kit.py BUNDLE_OVERLAY EYE_X EYE_Y EYE_Z [--label NAME] [--first charger|crate]

From a player's camera position (the eye, 1.6-1.7 m above the floor):
- the floor under the eye (movement collision);
- the nearest wall: horizontal probes every 2 degrees at 0.5 m and 1.2 m
  above the floor; a line is fitted through the hits within 30 degrees of
  the nearest one, giving the wall direction and the room-side normal; the
  opposite wall gives the free width (a tunnel's);
- the charger and the crate one after the other along that wall, backs
  --gap from it, long axes parallel to it, --spacing apart, slid along the
  wall (0.25 m steps, up to --slide) until both footprints have floor
  support within 8 cm everywhere, no collision from 5 cm to their height
  (plus 3 cm), and at least --walkway of free width in front.
Prints the placement (positions, yaws, wall offsets, free width) as JSON.
Footprints from the native collision: box_atlant12 x 0.56 / z 0.88 half,
0.84 m high (two stacked: 1.61 m); container_closed x 0.74 / z 0.57 half,
0.68 m high. The charger's long axis is its local z, the crate's local x.
"""
import argparse, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_respawns import World, config, tri_box
import audit_respawns
_prims = {}
_read = audit_respawns.config
def _cached_config(data):
    key = hash(data)
    if key not in _prims: _prims[key] = _read(data)
    return _prims[key]
audit_respawns.config = _cached_config   # primitive_distance/primitive_hit re-read exported_primitives per call

KIT = {'charger': dict(along=0.88, depth=0.56, height=1.61), 'crate': dict(along=0.74, depth=0.57, height=0.68)}

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('overlay', type=Path); ap.add_argument('eye', type=float, nargs=3)
    ap.add_argument('--label', default='spot'); ap.add_argument('--first', choices=['charger', 'crate'], default='charger')
    ap.add_argument('--gap', type=float, default=0.15); ap.add_argument('--spacing', type=float, default=0.4)
    ap.add_argument('--slide', type=float, default=2.0); ap.add_argument('--walkway', type=float, default=1.2)
    ap.add_argument('--max-residual', type=float, default=0.12, help='floor unevenness under a footprint (half sinks, half floats)')
    a = ap.parse_args()
    ex, ey, ez = a.eye
    project = config((a.overlay/'projects/v011e_level_04/day_2/client_project').read_bytes())
    full = World(a.overlay, project)
    class Sub(World):
        def __init__(self, parts): self.parts = parts; self.root = a.overlay
    world = Sub(full.near([ex-9, ey-6, ez-9], [ex+9, ey+4, ez+9]))
    fl = world.floor(ex, ey - 1.65, ez, below=0.8, above=0.6)
    assert fl, 'no floor under the eye'
    f = fl[0]
    def solid(x, y, z, r=0.03):
        lo, hi = [x-r, y-r, z-r], [x+r, y+r, z+r]
        for n, o, p in world.near(lo, hi):
            if p['kind'] == 'triangle':
                if tri_box(p['vertices'], lo, hi): return True
            elif world.primitive_distance(o, p, x, y, z) <= r: return True
        return False
    def probe(x, z, angle, reach=6.0):
        dx, dz = math.cos(angle), math.sin(angle)
        for k in range(1, int(reach / 0.05) + 1):
            t = k * 0.05
            if solid(x + dx*t, f + 0.5, z + dz*t) and solid(x + dx*t, f + 1.2, z + dz*t): return t
        return None
    hits = {d: probe(ex, ez, math.radians(d)) for d in range(0, 360, 2)}
    near = {d: t for d, t in hits.items() if t is not None}
    assert near, 'no wall within 6 m'
    dmin = min(near, key=near.get)
    pts = [(ex + math.cos(math.radians(d))*t, ez + math.sin(math.radians(d))*t) for d, t in near.items()
           if min(abs(d - dmin), 360 - abs(d - dmin)) <= 30]
    mx = sum(p[0] for p in pts)/len(pts); mz = sum(p[1] for p in pts)/len(pts)
    sxx = sum((p[0]-mx)**2 for p in pts); szz = sum((p[1]-mz)**2 for p in pts); sxz = sum((p[0]-mx)*(p[1]-mz) for p in pts)
    theta = 0.5 * math.atan2(2*sxz, sxx - szz)
    u = (math.cos(theta), math.sin(theta))
    n = (-u[1], u[0])
    if (ex - mx)*n[0] + (ez - mz)*n[1] < 0: n = (-n[0], -n[1])
    wall_d = (ex - mx)*n[0] + (ez - mz)*n[1]           # eye to wall along the normal
    opposite = probe(ex, ez, math.atan2(n[1], n[0]), reach=12.0)
    width = wall_d + (opposite if opposite is not None else 12.0)
    def footprint_ok(cx, cz, k):
        """Fit the floor plane under the footprint; the object rests on it, tilted to it."""
        d = KIT[k]; samples = []
        for i in range(-3, 4):
            for j in range(-3, 4):
                sa, sn = d['along']*i/3, d['depth']*j/3
                px, pz = cx + u[0]*sa + n[0]*sn, cz + u[1]*sa + n[1]*sn
                s = world.floor(px, f, pz, below=0.4, above=0.4)
                if not s: return False, f'no floor at ({px:.2f}, {pz:.2f})', None
                samples.append((sa, sn, s[0]))
        m = len(samples); sa_ = sum(x[0] for x in samples)/m; sn_ = sum(x[1] for x in samples)/m; sh_ = sum(x[2] for x in samples)/m
        saa = sum((x[0]-sa_)**2 for x in samples); snn = sum((x[1]-sn_)**2 for x in samples)
        ga = sum((x[0]-sa_)*(x[2]-sh_) for x in samples)/saa; gn = sum((x[1]-sn_)*(x[2]-sh_) for x in samples)/snn
        h0 = sh_ - ga*sa_ - gn*sn_
        resid = max(abs(x[2] - (h0 + ga*x[0] + gn*x[1])) for x in samples)
        if resid > a.max_residual: return False, f'floor not planar under the {k} ({resid:.2f} m)', None
        if math.hypot(ga, gn) > 0.14: return False, f'floor slope {math.degrees(math.atan(math.hypot(ga, gn))):.1f} deg under the {k}', None
        for sa, sn, _ in samples:
            px, pz = cx + u[0]*sa*0.96 + n[0]*sn*0.96, cz + u[1]*sa*0.96 + n[1]*sn*0.96
            base = h0 + ga*sa + gn*sn
            for h in (0.06 + resid, d['height']/2, d['height'] + 0.03):
                if solid(px, base + h, pz, r=0.04): return False, f'collision at ({px:.2f}, {base+h:.2f}, {pz:.2f})', None
        above = max(x[2] - (h0 + ga*x[0] + gn*x[1]) for x in samples)
        return True, '', dict(height=h0 + above / 2, along_slope=ga, normal_slope=gn, residual=resid)
    walls = [(u, n, wall_d)]
    if opposite is not None and width < 6.0:
        walls.append(((-u[0], -u[1]), (-n[0], -n[1]), opposite))
    best = None; tried = []
    for wi, (uu, nn, dd) in enumerate(walls):
        for gap in (a.gap, a.gap + 0.15, a.gap + 0.3, a.gap + 0.45):
            for first in (a.first, 'crate' if a.first == 'charger' else 'charger'):
                order = [first, 'crate' if first == 'charger' else 'charger']
                length = KIT[order[0]]['along']*2 + a.spacing + KIT[order[1]]['along']*2
                base = (ex - nn[0]*dd, ez - nn[1]*dd)
                for step in sorted({round(t*0.25, 2) for t in range(-int(a.slide/0.25), int(a.slide/0.25)+1)}, key=abs):
                    u, n = uu, nn
                    centres, planes, ok, why = {}, {}, True, ''
                    pos = step - length/2
                    for k in order:
                        c = pos + KIT[k]['along']; off = gap + KIT[k]['depth']
                        centres[k] = (base[0] + u[0]*c + n[0]*off, base[1] + u[1]*c + n[1]*off)
                        good, why, plane = footprint_ok(*centres[k], k)
                        if not good: ok = False; break
                        planes[k] = plane
                        pos += 2*KIT[k]['along'] + a.spacing
                    free = width - gap - max(KIT[k]['depth'] for k in order)*2
                    if ok and free < a.walkway: ok, why = False, f'free width {free:.2f} m < {a.walkway}'
                    tried.append((wi, gap, first, step, why))
                    if ok:
                        score = (max(pl['residual'] for pl in planes.values()), abs(step), gap)
                        if best is None or score < best[0]: best = (score, wi, uu, nn, dd, gap, first, step, dict(centres), dict(planes), free)
    if not best:
        print(json.dumps(dict(label=a.label, error='no clear placement', floor=f, floor_model=fl[1], free_width=round(width, 2),
              tried=tried[:40])), file=sys.stderr); sys.exit(1)
    _, wi, u, n, wall_d, gap, first, step, centres, planes, free = best
    # Yaws: the charger's local z and the crate's local x along u; local x / z toward the room.
    yaw_charger = math.atan2(-u[0], u[1])
    if math.cos(yaw_charger)*n[0] + math.sin(yaw_charger)*n[1] < 0: yaw_charger += math.pi
    yaw_crate = math.atan2(u[1], u[0])
    if -math.sin(yaw_crate)*n[0] + math.cos(yaw_crate)*n[1] < 0: yaw_crate += math.pi
    norm = lambda v: round(math.atan2(math.sin(v), math.cos(v)), 4)
    print(json.dumps(dict(label=a.label, eye=a.eye, floor=round(f, 3), floor_model=fl[1], wall_direction=[round(u[0], 4), round(u[1], 4)],
        room_normal=[round(n[0], 4), round(n[1], 4)], eye_to_wall=round(wall_d, 2), free_width=round(width, 2), walkway_left=round(free, 2),
        wall='nearest' if wi == 0 else 'opposite', gap_to_wall=gap, first_along_wall=first, slid_along_wall=step, **{k: dict(position=[round(centres[k][0], 3), round(planes[k]['height'], 3), round(centres[k][1], 3)], yaw=norm(yaw),
                 floor_slope_along_wall=round(planes[k]['along_slope'], 4), floor_slope_from_wall=round(planes[k]['normal_slope'], 4),
                 floor_residual=round(planes[k]['residual'], 3))
           for k, yaw in (('charger', yaw_charger), ('crate', yaw_crate))}), indent=1))

if __name__ == '__main__':
    main()
