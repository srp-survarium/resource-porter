#!/usr/bin/env python3
"""Place a battery container (two stacked box_atlant12) and an ammo crate free-standing near a point (a
room's middle), offline: the best spot within --search of the target where both footprints have floor
support (fitted plane, residual under --max-residual, slope under 8 deg), no collision from 6 cm to their
height + 3 cm, every respawn point of the project at least --spawn-clear from either footprint, and at
least --walkway of free floor around the pair (the ring is sampled at 0.5 and 1.2 m above the floor).

Usage: place_kit_free.py ROOT PROJECT X Z FLOOR_Y [--yaw A] [--search M] [--layout side|end] [--label L]
Prints JSON: the container (charger) and crate centres, heights, slopes, yaws and the clearances found.
Footprints from the native collision (place_base_kit.py): box_atlant12 x 0.56 / z 0.88 half, 1.61 m (two
stacked); container_closed x 0.74 / z 0.57 half, 0.68 m. Yaw turns both about the vertical.
"""
import argparse, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config, tri_box

KIT = {'charger': dict(hx=0.56, hz=0.88, height=1.61), 'crate': dict(hx=0.74, hz=0.57, height=0.68)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
    for k in ('x', 'z', 'floor_y'): ap.add_argument(k, type=float)
    ap.add_argument('--yaw', type=float, default=0.0); ap.add_argument('--search', type=float, default=3.0)
    ap.add_argument('--layout', choices=['side', 'end'], default='side', help='crate beside the container (along x) or at its end (along z)')
    ap.add_argument('--gap', type=float, default=0.5); ap.add_argument('--spawn-clear', type=float, default=1.5)
    ap.add_argument('--walkway', type=float, default=1.0); ap.add_argument('--max-residual', type=float, default=0.08)
    ap.add_argument('--label', default='kit')
    ap.add_argument('--debug', action='store_true', help='print why each crate candidate fails')
    ap.add_argument('--crate-only', action='store_true', help='place only the crate (around --around, else near X Z)')
    ap.add_argument('--around', type=float, nargs=2, metavar=('CX', 'CZ'), help='an existing container: the crate goes 1.4-4 m from it, '
                    'its footprint at least 0.7 m clear of a 1.0 m circle round the container')
    a = ap.parse_args()
    project = config(a.project.read_bytes())
    full = FastWorld(a.root, project)
    w = FastWorld(a.root, parts=full.near([a.x - a.search - 6, a.floor_y - 4, a.z - a.search - 6], [a.x + a.search + 6, a.floor_y + 6, a.z + a.search + 6]))
    spawns = [p['position'] for p in project['server_objects']['respawn_points']]

    def solid(x, y, z, r=0.04):
        lo, hi = [x - r, y - r, z - r], [x + r, y + r, z + r]
        for n, o, p in w.near(lo, hi):
            if p['kind'] == 'triangle':
                if tri_box(p['vertices'], lo, hi): return True
            elif w.primitive_distance(o, p, x, y, z) <= r: return True
        return False
    frame = {'yaw': a.yaw}
    def axes():
        c, s = math.cos(frame['yaw']), math.sin(frame['yaw']); return (c, s), (-s, c)   # local x, local z in world (x, z)
    def world(cx, cz, lx, lz):
        ux, uz = axes(); return cx + ux[0] * lx + uz[0] * lz, cz + ux[1] * lx + uz[1] * lz

    def footprint(cx, cz, k):
        d = KIT[k]; samples = []
        for i in range(-3, 4):
            for j in range(-3, 4):
                lx, lz = d['hx'] * i / 3, d['hz'] * j / 3
                px, pz = world(cx, cz, lx, lz)
                f = w.floor(px, a.floor_y, pz, below=0.6, above=0.6)
                if not f: return None, f'no floor at ({px:.2f}, {pz:.2f})'
                samples.append((lx, lz, f[0], px, pz))
        m = len(samples); mx = sum(q[0] for q in samples) / m; mz = sum(q[1] for q in samples) / m; mh = sum(q[2] for q in samples) / m
        gx = sum((q[0] - mx) * (q[2] - mh) for q in samples) / sum((q[0] - mx) ** 2 for q in samples)
        gz = sum((q[1] - mz) * (q[2] - mh) for q in samples) / sum((q[1] - mz) ** 2 for q in samples)
        h0 = mh - gx * mx - gz * mz
        res = max(abs(q[2] - (h0 + gx * q[0] + gz * q[1])) for q in samples)
        if res > a.max_residual: return None, f'floor not planar ({res:.2f})'
        if math.hypot(gx, gz) > 0.14: return None, 'slope'
        for lx, lz, _, px, pz in samples:
            base = h0 + gx * lx + gz * lz
            for h in (0.06 + res, d['height'] / 2, d['height'] + 0.03):
                if solid(px, base + h, pz, r=0.04): return None, f'collision at ({px:.2f}, {base + h:.2f}, {pz:.2f})'
        top = max(q[2] - (h0 + gx * q[0] + gz * q[1]) for q in samples)
        return dict(height=round(h0 + top / 2, 3), slope_x=round(gx, 4), slope_z=round(gz, 4), residual=round(res, 3)), ''

    def corners(cx, cz, k):
        d = KIT[k]; return [world(cx, cz, sx * d['hx'], sz * d['hz']) for sx in (-1, 1) for sz in (-1, 1)]

    def clear_of_spawns(cx, cz, k):
        d = KIT[k]; worst = 1e9; ux, uz = axes()
        for p in spawns:
            lx = (p[0] - cx) * ux[0] + (p[2] - cz) * ux[1]; lz = (p[0] - cx) * uz[0] + (p[2] - cz) * uz[1]
            dx, dz = max(abs(lx) - d['hx'], 0), max(abs(lz) - d['hz'], 0)
            worst = min(worst, math.hypot(dx, dz))
        return worst

    def ring_free(cx, cz, k, margin):
        d = KIT[k]
        for t in range(24):
            ang = 2 * math.pi * t / 24
            lx, lz = math.cos(ang) * (d['hx'] + margin), math.sin(ang) * (d['hz'] + margin)
            px, pz = world(cx, cz, lx, lz)
            f = w.floor(px, a.floor_y, pz, below=0.6, above=0.6)
            if not f or solid(px, f[0] + 0.5, pz, 0.15) or solid(px, f[0] + 1.2, pz, 0.15): return False
        return True
    if a.crate_only:
        import collections; why_not = collections.Counter(); best = None; tried = 0
        cands = []
        if a.around:
            for k in range(36):
                ang = 2 * math.pi * k / 36
                for dist in (1.6, 2.0, 2.4, 2.8, 3.2, 3.6, 4.0):
                    cx, cz = a.around[0] + math.cos(ang) * dist, a.around[1] + math.sin(ang) * dist
                    for yw in (ang, ang + math.pi / 2): cands.append((dist, cx, cz, yw))
        else:
            steps = int(a.search / 0.25)
            for i in range(-steps, steps + 1):
                for j in range(-steps, steps + 1):
                    d0 = math.hypot(i * 0.25, j * 0.25)
                    if d0 <= a.search:
                        for yw in (a.yaw, a.yaw + math.pi / 2): cands.append((d0, a.x + i * 0.25, a.z + j * 0.25, yw))
        cands.sort(key=lambda c: c[0])
        for d0, cx, cz, yw in cands:
            if best and d0 > best[0] + 1e-6: break
            tried += 1
            frame['yaw'] = yw; ux, uz = axes()
            if a.around:
                corner = [(cx + ux[0] * sx * 0.74 + uz[0] * sz * 0.57, cz + ux[1] * sx * 0.74 + uz[1] * sz * 0.57) for sx in (-1, 1) for sz in (-1, 1)]
                if min(math.dist(q, a.around) for q in corner) < 1.7: why_not['too close to the container'] += 1; continue
            sp = clear_of_spawns(cx, cz, 'crate')
            if sp < a.spawn_clear: why_not['a spawn point within --spawn-clear'] += 1; continue
            f2, why = footprint(cx, cz, 'crate')
            if not f2:
                why_not['crate: ' + why.split(' at ')[0].split(' (')[0]] += 1
                if a.debug: print('rejected', round(cx, 3), round(cz, 3), round(yw, 4), why, file=sys.stderr)
                continue
            if not ring_free(cx, cz, 'crate', a.walkway): why_not['no --walkway of free floor around'] += 1; continue
            if best is None: best = (d0, cx, cz, yw, f2, sp)
        if not best:
            print(json.dumps(dict(label=a.label, error='no free spot', tried=tried, rejected=dict(why_not)))); sys.exit(1)
        d0, cx, cz, yw, f2, sp = best
        print(json.dumps(dict(label=a.label, target=[a.x, a.z], around=a.around, crate=dict(position=[round(cx, 3), f2['height'], round(cz, 3)], yaw=round(yw, 4), **f2),
                              distance=round(d0, 2), nearest_spawn_from_footprint=round(sp, 2)), indent=1))
        return
    best, tried = None, 0
    import collections; why_not = collections.Counter()
    steps = int(a.search / 0.25)
    for i in range(-steps, steps + 1):
        for j in range(-steps, steps + 1):
            ox, oz = i * 0.25, j * 0.25
            if math.hypot(ox, oz) > a.search: continue
            tried += 1
            # The pair's middle at the candidate; container on one side, crate on the other.
            if a.layout == 'side':
                off = (KIT['charger']['hx'] + KIT['crate']['hx'] + a.gap) / 2
                ch = world(a.x + ox, a.z + oz, -off, 0); cr = world(a.x + ox, a.z + oz, off, 0)
            else:
                off = (KIT['charger']['hz'] + KIT['crate']['hz'] + a.gap) / 2
                ch = world(a.x + ox, a.z + oz, 0, -off); cr = world(a.x + ox, a.z + oz, 0, off)
            sp = min(clear_of_spawns(*ch, 'charger'), clear_of_spawns(*cr, 'crate'))
            if sp < a.spawn_clear: why_not['a spawn point within --spawn-clear'] += 1; continue
            f1, why = footprint(*ch, 'charger')
            if not f1: why_not['container: ' + why.split(' at ')[0]] += 1; continue
            f2, why = footprint(*cr, 'crate')
            if not f2: why_not['crate: ' + why.split(' at ')[0]] += 1; continue
            if not (ring_free(*ch, 'charger', a.walkway) and ring_free(*cr, 'crate', a.walkway)): why_not['no --walkway of free floor around'] += 1; continue
            score = (math.hypot(ox, oz), -sp)
            if best is None or score < best[0]: best = (score, ch, cr, f1, f2, sp)
    if not best:
        print(json.dumps(dict(label=a.label, error='no free spot', tried=tried, rejected=dict(why_not)))); sys.exit(1)
    _, ch, cr, f1, f2, sp = best
    print(json.dumps(dict(label=a.label, target=[a.x, a.z], yaw=a.yaw, layout=a.layout,
        charger=dict(position=[round(ch[0], 3), f1['height'], round(ch[1], 3)], **f1),
        crate=dict(position=[round(cr[0], 3), f2['height'], round(cr[1], 3)], **f2),
        offset_from_target=round(best[0][0], 2), nearest_spawn_from_footprints=round(sp, 2)), indent=1))


if __name__ == '__main__':
    main()
