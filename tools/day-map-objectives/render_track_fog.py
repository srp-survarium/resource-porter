#!/usr/bin/env python3
"""Overhead stage renders and a side profile of the track fog, before and after (offline).

Usage: render_track_fog.py COLLISION_ROOT BEFORE_PROJECT AFTER_PROJECT OUT_DIR
OUT_DIR gets stages-before/after.png (t = 180 ... 600 s), full-before-after.png, overview-after.png and
profile-x12.9/x18.6.png (vertical slices of the movement collision with the final damage boxes).
"""
import json, math, sys
from pathlib import Path
T = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(T/'day-map-objectives')); sys.path.insert(0, str(T/'chemical-plant'))
from audit_respawns import config
from audit_collision_region import collision_parts
import numpy as np
from PIL import Image, ImageDraw

root, before_p, after_p, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4])
out.mkdir(parents=True, exist_ok=True)
X0, X1, Z0, Z1, PX = -6.0, 42.0, -26.0, 142.0, 5
OX0, OX1, OZ0, OZ1, OPX = -100.0, 130.0, -60.0, 160.0, 3
ROOT = 'logic/fill/track_anomaly/'

def load(p):
    pr = config(p.read_bytes())
    return pr, {o['full_name']: o for o in pr['client_objects'] if 'full_name' in o}

def track(objs):
    stages, steps = [], []
    for n, o in objs.items():
        if n.startswith(ROOT) and n.endswith('/controller') and '/stage_' in n:
            fog = objs[o['states'][1]['groups'][0]['zones'][0]]
            stages.append((o['states'][0]['active_time_sec'], [[fog['position'][i] + e['position'][i] for i in range(3)] for e in fog['effect_zones']]))
    ctl = objs[ROOT + 'damage/controller']; t = 0
    for k, st in enumerate(ctl['states']):
        if k:
            zone = objs[st['groups'][0]['zones'][0]]; boxes = []
            for g in zone['collision_geometries']:
                geo = objs[g['name']]
                for m in geo['meshes']:
                    c = [geo['position'][i] + m['position'][i] for i in range(3)]
                    boxes.append((c, m['scale']))
            steps.append(dict(at=t, hit=zone['max_hit'], boxes=boxes))
        t += st['active_time_sec']
    return sorted(stages, key=lambda s: s[0]), steps

before, bobjs = load(before_p); after, aobjs = load(after_p)
bst, bsteps = track(bobjs); ast, asteps = track(aobjs)
rust = [o['position'] for n, o in aobjs.items() if n.startswith('fx/fill_rust_fog/')]
moss = aobjs['logic/anomaly/toxic_moss_collision']

# Movement collision: a top-down height map (max surface) and the parts for the profile.
tris, prims = [], []
for obj in after['collision_objects']:
    if not (obj['cgroup'] & 2 and obj['cmask'] & 4): continue
    try: parts = list(collision_parts(root, obj))
    except FileNotFoundError: continue
    name = obj['lib_name'].split('/models/')[-1].split('/')[-2] if '/models/' in obj['lib_name'] else obj['lib_name']
    for p in parts:
        lo, hi = p['bounds']
        if hi[0] < OX0 or lo[0] > OX1 or hi[2] < OZ0 or lo[2] > OZ1: continue
        if p['kind'] == 'triangle': tris.append((p['vertices'], name))
        else: prims.append((lo, hi, name))
def raster(X0, X1, Z0, Z1, PX):
  W, H = int((X1 - X0) * PX), int((Z1 - Z0) * PX)
  top = np.full((H, W), -1e9)
  def cell(x, z): return int((x - X0) * PX), int((Z1 - z) * PX)
  for v, name in tris:
      if 'level_border' in name: continue
      (x1, y1, z1), (x2, y2, z2), (x3, y3, z3) = v
      d = (z2 - z3) * (x1 - x3) + (x3 - x2) * (z1 - z3)
      if abs(d) < 1e-9: continue
      i0, i1 = max(0, int((min(x1, x2, x3) - X0) * PX)), min(W - 1, int((max(x1, x2, x3) - X0) * PX) + 1)
      j0, j1 = max(0, int((Z1 - max(z1, z2, z3)) * PX)), min(H - 1, int((Z1 - min(z1, z2, z3)) * PX) + 1)
      if i0 > i1 or j0 > j1: continue
      xs = X0 + (np.arange(i0, i1 + 1) + .5) / PX; zs = Z1 - (np.arange(j0, j1 + 1) + .5) / PX
      gx, gz = np.meshgrid(xs, zs)
      a_ = ((z2 - z3) * (gx - x3) + (x3 - x2) * (gz - z3)) / d; b_ = ((z3 - z1) * (gx - x3) + (x1 - x3) * (gz - z3)) / d; c_ = 1 - a_ - b_
      m = (a_ >= -1e-6) & (b_ >= -1e-6) & (c_ >= -1e-6)
      h = a_ * y1 + b_ * y2 + c_ * y3
      sub = top[j0:j1 + 1, i0:i1 + 1]
      np.copyto(sub, np.where(m & (h < 20) & (h > sub), h, sub))
  for lo, hi, name in prims:
      if 'level_border' in name or hi[1] > 20: continue
      i0, j0 = cell(lo[0], hi[2]); i1, j1 = cell(hi[0], lo[2])
      sub = top[max(0, j0):min(H, j1 + 1), max(0, i0):min(W, i1 + 1)]
      np.copyto(sub, np.maximum(sub, hi[1]))
  shade = np.clip((top - 1.0) / 9.0, 0, 1)
  base = np.where(top < -1e8, 60, 225 - shade * 150).astype(np.uint8)
  return Image.fromarray(np.stack([base, base, (base * 0.96).astype(np.uint8)], -1), 'RGB')
base_img = raster(X0, X1, Z0, Z1, PX)

def sx(x): return (x - X0) * PX
def sz(z): return (Z1 - z) * PX

def panel(project, objs, stages, steps, t, title):
    img = base_img.copy().convert('RGBA'); ov = Image.new('RGBA', img.size, (0, 0, 0, 0)); d = ImageDraw.Draw(ov)
    for c in rust: d.ellipse([sx(c[0]) - 5 * PX, sz(c[2]) - 5 * PX, sx(c[0]) + 5 * PX, sz(c[2]) + 5 * PX], fill=(120, 170, 120, 40))
    for at, clouds in stages:
        if at <= t:
            for c in clouds: d.ellipse([sx(c[0]) - 5 * PX, sz(c[2]) - 5 * PX, sx(c[0]) + 5 * PX, sz(c[2]) + 5 * PX], fill=(80, 120, 230, 40))
    img = Image.alpha_composite(img, ov); d = ImageDraw.Draw(img)
    # Native moss damage zone (always on).
    for m in moss['meshes']:
        c = [moss['position'][i] + m['position'][i] for i in range(3)]; s = m['scale']
        if m['type'] == 1: d.rectangle([sx(c[0] - s[0]), sz(c[2] + s[2]), sx(c[0] + s[0]), sz(c[2] - s[2])], outline=(0, 140, 60), width=1)
        else: d.ellipse([sx(c[0] - s[0]), sz(c[2] + s[0]), sx(c[0] + s[0]), sz(c[2] - s[0])], outline=(0, 140, 60), width=1)
    act = [s for s in steps if s['at'] <= t]
    if act:
        for c, s in act[-1]['boxes']:
            d.rectangle([sx(c[0] - s[0]), sz(c[2] + s[2]), sx(c[0] + s[0]), sz(c[2] - s[2])], outline=(220, 30, 30), width=1)
    for at, clouds in stages:
        for c in clouds:
            col = (30, 60, 200) if at <= t else (150, 150, 170)
            d.ellipse([sx(c[0]) - 3, sz(c[2]) - 3, sx(c[0]) + 3, sz(c[2]) + 3], fill=col)
    for r in project['server_objects']['respawn_points']:
        x, _, z = r['position']; col = (0, 90, 255) if r['team'] == 0 else (230, 90, 0)
        d.ellipse([sx(x) - 4, sz(z) - 4, sx(x) + 4, sz(z) + 4], outline=col, width=2)
    for c in project['server_objects']['victory_items_containers']:
        x, _, z = c['visual_geometry']['position']; d.rectangle([sx(x) - 5, sz(z) - 5, sx(x) + 5, sz(z) + 5], fill=(0, 0, 0))
    for n, o in objs.items():
        if o.get('game_object_type') == 'resupply_zone':
            x, _, z = o['position']; d.rectangle([sx(x) - 4, sz(z) - 4, sx(x) + 4, sz(z) + 4], fill=(200, 160, 0))
    for g in range(int(math.ceil(Z0 / 10) * 10), int(Z1) + 1, 10):
        d.line([0, sz(g), 12, sz(g)], fill=(0, 0, 0)); d.text((14, sz(g) - 6), f'z {g}', fill=(0, 0, 0))
    hit = f"damage {act[-1]['hit']:.2f}" if act else 'no damage yet'
    d.rectangle([0, 0, img.size[0], 30], fill=(255, 255, 255)); d.text((4, 2), title, fill=(0, 0, 0)); d.text((4, 16), f't = {t} s: {sum(1 for at, _ in stages if at <= t)} stages, {hit}', fill=(0, 0, 0))
    return img.convert('RGB')

def sheet(panels, path):
    w = sum(p.size[0] for p in panels) + 6 * (len(panels) - 1); h = max(p.size[1] for p in panels)
    s = Image.new('RGB', (w, h), (255, 255, 255)); x = 0
    for p in panels: s.paste(p, (x, 0)); x += p.size[0] + 6
    s.save(path); print(path, s.size)

times = [180, 240, 300, 360, 420, 480, 540, 600]
sheet([panel(before, bobjs, bst, bsteps, t, 'before (maps-v011e-4)') for t in times], out / 'stages-before.png')
sheet([panel(after, aobjs, ast, asteps, t, 'after (rev 20 track fog)') for t in times], out / 'stages-after.png')
sheet([panel(before, bobjs, bst, bsteps, 900, 'before, full'), panel(after, aobjs, ast, asteps, 900, 'after, full')], out / 'full-before-after.png')

def overview(path):
    img = raster(OX0, OX1, OZ0, OZ1, OPX).convert('RGBA'); ov = Image.new('RGBA', img.size, (0, 0, 0, 0)); d = ImageDraw.Draw(ov)
    ox = lambda x: (x - OX0) * OPX; oz = lambda z: (OZ1 - z) * OPX
    for c in rust + [c for _, cl in ast for c in cl]: d.ellipse([ox(c[0] - 5), oz(c[2] + 5), ox(c[0] + 5), oz(c[2] - 5)], fill=(80, 120, 230, 50))
    img = Image.alpha_composite(img, ov); d = ImageDraw.Draw(img)
    for c, s_ in asteps[-1]['boxes']: d.rectangle([ox(c[0] - s_[0]), oz(c[2] + s_[2]), ox(c[0] + s_[0]), oz(c[2] - s_[2])], outline=(220, 30, 30))
    for proj, objs, tag in ((after, aobjs, '206'),):
        for r in proj['server_objects']['respawn_points']:
            x, _, z = r['position']; col = (0, 90, 255) if r['team'] == 0 else (230, 90, 0)
            d.ellipse([ox(x) - 3, oz(z) - 3, ox(x) + 3, oz(z) + 3], outline=col, width=2)
        for c in proj['server_objects'].get('victory_items_containers', []):
            x, _, z = c['visual_geometry']['position']; d.rectangle([ox(x) - 5, oz(z) - 5, ox(x) + 5, oz(z) + 5], fill=(0, 0, 0)); d.text((ox(x) + 7, oz(z) - 6), 'container', fill=(0, 0, 0))
        for n, o in objs.items():
            if o.get('game_object_type') == 'resupply_zone':
                x, _, z = o['position']; d.rectangle([ox(x) - 4, oz(z) - 4, ox(x) + 4, oz(z) + 4], fill=(200, 160, 0)); d.text((ox(x) + 6, oz(z) + 2), 'crate', fill=(120, 90, 0))
    d.rectangle([0, 0, img.size[0], 30], fill=(255, 255, 255))
    d.text((4, 2), 'Chemical Plant (v0.11) 206, fully grown track fog (blue, 5 m cloud radius) and final damage boxes (red)', fill=(0, 0, 0))
    pts = [r['position'] for r in after['server_objects']['respawn_points']] + [c['visual_geometry']['position'] for c in after['server_objects'].get('victory_items_containers', [])]
    pts += [o['position'] for o in aobjs.values() if o.get('game_object_type') == 'resupply_zone']
    nearest = min(math.hypot(c[0] - p_[0], c[2] - p_[2]) - 5 for p_ in pts for _, cl in ast for c in cl)
    d.text((4, 16), f'spawns: team 0 blue rings, team 1 orange rings; containers black; ammo crates yellow. Nearest of them: {nearest:.0f} m from the fog.', fill=(0, 0, 0))
    img.convert('RGB').save(path); print(path, img.size)
overview(out / 'overview-after.png')

# Side profiles: the vertical slice of the movement collision at x, the final damage boxes, a standing face.
def profile(x0, za, zb, path, title):
    PZ, PY, Y0, Y1 = 10, 40, 0.5, 9.0
    Wp, Hp = int((zb - za) * PZ), int((Y1 - Y0) * PY) + 40
    img = Image.new('RGB', (Wp, Hp), (250, 250, 248)); d = ImageDraw.Draw(img)
    px = lambda z: (z - za) * PZ; py = lambda y: 40 + (Y1 - y) * PY
    for y in range(1, 10): d.line([0, py(y), Wp, py(y)], fill=(225, 225, 225)); d.text((2, py(y) - 11), f'y {y}', fill=(150, 150, 150))
    for z in range(int(za), int(zb) + 1, 5): d.line([px(z), 40, px(z), Hp], fill=(235, 235, 235)); d.text((px(z) + 2, Hp - 12), f'z {z}', fill=(150, 150, 150))
    for c, s in asteps[-1]['boxes']:
        if abs(x0 - c[0]) <= s[0]:
            d.rectangle([px(c[2] - s[2]), py(c[1] + s[1]), px(c[2] + s[2]), py(c[1] - s[1])], fill=(255, 215, 215), outline=(220, 40, 40))
    for c, s in bsteps[-1]['boxes']:
        if abs(x0 - c[0]) <= s[0]:
            d.rectangle([px(c[2] - s[2]), py(c[1] + s[1]), px(c[2] + s[2]), py(c[1] - s[1])], outline=(120, 120, 120))
    for v, name in tris:
        if 'level_border' in name: continue
        pts = []
        for p, q in ((v[0], v[1]), (v[1], v[2]), (v[2], v[0])):
            if (p[0] - x0) * (q[0] - x0) < 0 or p[0] == x0:
                t_ = 0 if q[0] == p[0] else (x0 - p[0]) / (q[0] - p[0]); pts.append((p[2] + t_ * (q[2] - p[2]), p[1] + t_ * (q[1] - p[1])))
        if len(pts) >= 2:
            col = (40, 40, 40) if any(k in name for k in ('terrain', 'moss', 'railroad_0')) else (40, 90, 200)
            d.line([(px(pts[0][0]), py(pts[0][1])), (px(pts[1][0]), py(pts[1][1]))], fill=col, width=2)
    for lo, hi, name in prims:
        if lo[0] <= x0 <= hi[0] and hi[2] >= za and lo[2] <= zb and hi[1] < 20:
            d.rectangle([px(lo[2]), py(hi[1]), px(hi[2]), py(lo[1])], outline=(40, 90, 200))
    for c, s_ in asteps[-1]['boxes']:
        pass
    roofs = [(lo, hi) for lo, hi, name in prims if 'hopper' in name and lo[0] <= x0 <= hi[0] and hi[1] > 6.5]
    for lo, hi in roofs[:1]:
        d.text((px((lo[2] + hi[2]) / 2) - 40, py(hi[1]) - 14), f'wagon roof {hi[1]:.1f}', fill=(40, 90, 200))
    old = [(c, s_) for c, s_ in bsteps[-1]['boxes'] if abs(x0 - c[0]) <= s_[0]]
    for c, s_ in old[:1]:
        d.text((px(c[2] - s_[2]) + 4, py(c[1] + s_[1]) + 2), f'old box top {c[1] + s_[1]:.2f} (before): roofs inside', fill=(110, 110, 110))
    d.text((px(60), py(5.2)), 'new boxes: 0.3 m below the lowest to 2.2 m above the highest bed under each box', fill=(200, 30, 30))
    d.rectangle([0, 0, Wp, 36], fill=(255, 255, 255))
    d.text((4, 2), title, fill=(0, 0, 0))
    d.text((4, 18), 'black: track bed (terrain, moss, rails); blue: wagons and other collision; red fill: final capped damage boxes (after); grey outline: the old box (before)', fill=(0, 0, 0))
    img.save(path); print(path, img.size)
profile(18.6, 0, 135, out / 'profile-x18.6.png', 'side profile along the east track (x 18.6): hopper wagons at z 51-62, 95-120')
profile(12.9, 0, 135, out / 'profile-x12.9.png', 'side profile along the west track (x 12.9): hopper wagons at z 20-33, 70-80')
