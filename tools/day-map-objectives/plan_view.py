#!/usr/bin/env python3
"""Top-down plan of a converted map's area, from its movement collision (offline, no game).

Usage: plan_view.py ROOT PROJECT X0 X1 Z0 Z1 OUT.png [--overlay JSON] [--step M] [--px N] [--floor-y Y]

ROOT: a collision root (pack files or audit_spawns_026's merged root); PROJECT: the client_project.
Cells: dark grey no floor within 3 m of --floor-y; brown blocked at 0.5 or 1.2 m above the floor; else the
floor, darker the higher. Every respawn point of the project is drawn (team 0 blue, team 1 red; small for a
safe-respawn group point) with a 1.2 m facing tick.
--overlay JSON: {"boxes": [{"x","z","yaw","half_along","half_across","color","label","dash"}],
                 "points": [{"x","z","color","label","r"}], "paths": [{"points": [[x,z],...], "color"}],
                 "rects": [{"x0","x1","z0","z1","color","label"}], "title": "..."}
North (+z) is up; a 1 m grid, 5 m lines darker.
"""
import argparse, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config, tri_box
from PIL import Image, ImageDraw


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
    for k in ('x0', 'x1', 'z0', 'z1'): ap.add_argument(k, type=float)
    ap.add_argument('out', type=Path); ap.add_argument('--overlay', type=Path); ap.add_argument('--step', type=float, default=0.25)
    ap.add_argument('--px', type=int, default=14); ap.add_argument('--floor-y', type=float, default=None)
    a = ap.parse_args()
    project = config(a.project.read_bytes())
    full = FastWorld(a.root, project)
    w = FastWorld(a.root, parts=full.near([a.x0 - 1, -50, a.z0 - 1], [a.x1 + 1, 80, a.z1 + 1]))
    pts = [p for p in project['server_objects']['respawn_points'] if a.x0 <= p['position'][0] <= a.x1 and a.z0 <= p['position'][2] <= a.z1]
    fy = a.floor_y if a.floor_y is not None else (sorted(p['position'][1] for p in pts)[len(pts) // 2] if pts else 0.0)

    def solid(x, y, z, r=0.12):
        lo, hi = [x - r, y - r, z - r], [x + r, y + r, z + r]
        for n, o, p in w.near(lo, hi):
            if p['kind'] == 'triangle':
                if tri_box(p['vertices'], lo, hi): return True
            elif w.primitive_distance(o, p, x, y, z) <= r: return True
        return False
    W, H = int((a.x1 - a.x0) * a.px), int((a.z1 - a.z0) * a.px)
    img = Image.new('RGB', (W, H), (236, 233, 224)); d = ImageDraw.Draw(img)
    sx = lambda x: (x - a.x0) * a.px
    sz = lambda z: (a.z1 - z) * a.px
    nx, nz = int((a.x1 - a.x0) / a.step), int((a.z1 - a.z0) / a.step)
    for j in range(nz):
        for i in range(nx):
            x, z = a.x0 + (i + .5) * a.step, a.z0 + (j + .5) * a.step
            fl = w.floor(x, fy, z, below=3.0, above=3.0)
            if not fl: col = (70, 70, 70)
            else:
                h = fl[0]
                if solid(x, h + 0.5, z) or solid(x, h + 1.2, z): col = (100, 78, 66)
                else:
                    g = int(228 - max(0, min(1, (h - fy + 1) / 4)) * 90); col = (g, g, g - 8)
            d.rectangle([sx(x - a.step / 2), sz(z + a.step / 2), sx(x + a.step / 2), sz(z - a.step / 2)], fill=col)
    for g in range(math.ceil(a.x0), int(a.x1) + 1): d.line([sx(g), 0, sx(g), H], fill=(150, 145, 130) if g % 5 == 0 else (205, 200, 190))
    for g in range(math.ceil(a.z0), int(a.z1) + 1): d.line([0, sz(g), W, sz(g)], fill=(150, 145, 130) if g % 5 == 0 else (205, 200, 190))
    for g in range(math.ceil(a.x0 / 5) * 5, int(a.x1) + 1, 10): d.text((sx(g) + 2, 2), f'x {g}', fill=(60, 60, 60))
    for g in range(math.ceil(a.z0 / 5) * 5, int(a.z1) + 1, 10): d.text((2, sz(g) + 2), f'z {g}', fill=(60, 60, 60))
    ov = json.loads(a.overlay.read_text()) if a.overlay else {}
    for r in ov.get('rects', []):
        d.rectangle([sx(r['x0']), sz(r['z1']), sx(r['x1']), sz(r['z0'])], outline=tuple(r.get('color', (120, 0, 160))), width=2)
        if r.get('label'): d.text((sx(r['x0']) + 3, sz(r['z1']) + 3), r['label'], fill=tuple(r.get('color', (120, 0, 160))))
    for p in project['server_objects']['respawn_points']:
        x, _, z = p['position']
        if not (a.x0 <= x <= a.x1 and a.z0 <= z <= a.z1): continue
        col = (30, 90, 210) if p['team'] == 0 else (210, 60, 30)
        r = 0.3 * a.px if 'safe' in p.get('full_name', '') else 0.45 * a.px
        d.ellipse([sx(x) - r, sz(z) - r, sx(x) + r, sz(z) + r], outline=col, width=2)
        yaw = p['rotation'][1] if abs(p['rotation'][0]) < 1 else math.pi - p['rotation'][1]
        d.line([sx(x), sz(z), sx(x - math.sin(yaw) * 1.2), sz(z + math.cos(yaw) * 1.2)], fill=col, width=2)
    for pth in ov.get('paths', []):
        q = [(sx(x), sz(z)) for x, z in pth['points']]
        if len(q) > 1: d.line(q, fill=tuple(pth.get('color', (160, 0, 160))), width=2)
        for u, v in q: d.ellipse([u - 3, v - 3, u + 3, v + 3], fill=tuple(pth.get('color', (160, 0, 160))))
    for b in ov.get('boxes', []):
        u = (math.cos(b['yaw']), math.sin(b['yaw'])); n = (-u[1], u[0])
        corners = [(sx(b['x'] + u[0] * s * b['half_along'] + n[0] * t * b['half_across']), sz(b['z'] + u[1] * s * b['half_along'] + n[1] * t * b['half_across']))
                   for s, t in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
        col = tuple(b.get('color', (0, 140, 60)))
        if b.get('dash'):
            for k in range(4): d.line([corners[k], corners[(k + 1) % 4]], fill=col, width=1)
        else: d.polygon(corners, outline=col, width=3)
        if b.get('label'): d.text((sx(b['x']) + 0.6 * a.px, sz(b['z']) - 0.6 * a.px), b['label'], fill=col)
    for p in ov.get('points', []):
        r = p.get('r', 0.35) * a.px; col = tuple(p.get('color', (0, 0, 0)))
        d.ellipse([sx(p['x']) - r, sz(p['z']) - r, sx(p['x']) + r, sz(p['z']) + r], fill=col)
        if p.get('label'): d.text((sx(p['x']) + r + 2, sz(p['z']) - 5), p['label'], fill=col)
    if ov.get('title'): d.rectangle([0, H - 16, W, H], fill=(255, 255, 255)); d.text((4, H - 14), ov['title'], fill=(0, 0, 0))
    a.out.parent.mkdir(parents=True, exist_ok=True); img.save(a.out); print(a.out, img.size)


if __name__ == '__main__':
    main()
