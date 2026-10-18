#!/usr/bin/env python3
"""Offline top-down estimate of the track fog's opacity (no game): rust_smoke snapshots splatted with the
real smoke1 texture alpha and the particle's alpha/size over life, depth-tested against the movement
collision's top surface (smoke under a wagon is hidden by it). No lighting: opacity only.

Usage: fog_sim_track.py COLLISION_ROOT PROJECT OUT_PREFIX --texture smoke1.dds [--window X0,X1,Z0,Z1] [--px M] [--snapshots N]
Model (fx/level_3_anomaly/rust_smoke): 100 particles per cloud, uniform in a flat r 5 m disc at the
emitter, age uniform; size 1.96 -> 3.17 m (full width) over life; alpha 0 -> peak -> 0 (peak at 0.487
of life, per particle between 0.132 and 0.191); random rotation; texture alpha of fx/lighted_smoke/smoke1.
"""
import argparse, json, math, struct, sys
from pathlib import Path
import numpy as np
from PIL import Image
T = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(T / 'day-map-objectives')); sys.path.insert(0, str(T / 'chemical-plant'))
from audit_respawns import config
from audit_collision_region import collision_parts

ap = argparse.ArgumentParser(); ap.add_argument('root', type=Path); ap.add_argument('project', type=Path); ap.add_argument('out')
ap.add_argument('--window', default='4,27,40,100'); ap.add_argument('--px', type=float, default=0.1)
ap.add_argument('--snapshots', type=int, default=6); ap.add_argument('--alpha-mult', type=float, default=1.0, help='the track clouds\' particle alpha over rust_smoke (track_fog)'); ap.add_argument('--seed', type=int, default=1)
ap.add_argument('--texture', type=Path, required=True, help='textures/fx/lighted_smoke/smoke1.dds (vostok-port extract)')
a = ap.parse_args()
X0, X1, Z0, Z1 = (float(v) for v in a.window.split(','))
P = a.px; W, H = int(round((X1 - X0) / P)), int(round((Z1 - Z0) / P))
project = config(a.project.read_bytes())
objs = {o['full_name']: o for o in project['client_objects'] if 'full_name' in o}
clouds = []
for n, o in objs.items():
    if n.startswith('logic/fill/track_anomaly/') and n.endswith('/controller') and '/stage_' in n:
        for st in o['states'][1:]:
            for zn in st['groups'][0]['zones']:
                z = objs[zn]
                if z.get('idle_particle', '').endswith(('/rust_smoke', '/track_fog')):
                    clouds += [[z['position'][i] + e['position'][i] for i in range(3)] for e in z['effect_zones']]
track_n = len(clouds)
clouds += [o['position'] for n, o in objs.items() if n.startswith('fx/fill_rust_fog/cloud_')]
# Top surface of the movement collision (for the depth test and the background).
top = np.full((H, W), -1e9)
for obj in project['collision_objects']:
    if not (obj['cgroup'] & 2 and obj['cmask'] & 4): continue
    try: parts = list(collision_parts(a.root, obj))
    except FileNotFoundError: continue
    for p in parts:
        lo, hi = p['bounds']
        if hi[0] < X0 or lo[0] > X1 or hi[2] < Z0 or lo[2] > Z1 or 'level_border' in obj['lib_name']: continue
        if p['kind'] == 'triangle':
            (x1, y1, z1), (x2, y2, z2), (x3, y3, z3) = p['vertices']
            d = (z2 - z3) * (x1 - x3) + (x3 - x2) * (z1 - z3)
            if abs(d) < 1e-9: continue
            i0, i1 = max(0, int((lo[0] - X0) / P)), min(W - 1, int((hi[0] - X0) / P) + 1)
            j0, j1 = max(0, int((Z1 - hi[2]) / P)), min(H - 1, int((Z1 - lo[2]) / P) + 1)
            if i0 > i1 or j0 > j1: continue
            gx, gz = np.meshgrid(X0 + (np.arange(i0, i1 + 1) + .5) * P, Z1 - (np.arange(j0, j1 + 1) + .5) * P)
            u = ((z2 - z3) * (gx - x3) + (x3 - x2) * (gz - z3)) / d; v = ((z3 - z1) * (gx - x3) + (x1 - x3) * (gz - z3)) / d; w = 1 - u - v
            m = (u >= -1e-6) & (v >= -1e-6) & (w >= -1e-6); h = u * y1 + v * y2 + w * y3
            sub = top[j0:j1 + 1, i0:i1 + 1]; np.copyto(sub, np.where(m & (h < 20) & (h > sub), h, sub))
        elif hi[1] < 20:
            i0, i1 = max(0, int((lo[0] - X0) / P)), min(W - 1, int((hi[0] - X0) / P))
            j0, j1 = max(0, int((Z1 - hi[2]) / P)), min(H - 1, int((Z1 - lo[2]) / P))
            if i0 <= i1 and j0 <= j1: np.maximum(top[j0:j1 + 1, i0:i1 + 1], hi[1], out=top[j0:j1 + 1, i0:i1 + 1])
def dxt5_alpha(path):
    """The alpha channel of a DXT5 .dds (top mip)."""
    b = path.read_bytes(); assert b[:4] == b'DDS ' and b[84:88] == b'DXT5', path
    h, w = struct.unpack_from('<II', b, 12); out = np.zeros((h, w)); q = 128
    for by in range((h + 3) // 4):
        for bx in range((w + 3) // 4):
            a0, a1 = b[q], b[q + 1]; bits = int.from_bytes(b[q + 2:q + 8], 'little')
            pal = [a0, a1] + ([((6 - i) * a0 + (1 + i) * a1) / 7 for i in range(6)] if a0 > a1 else [((4 - i) * a0 + (1 + i) * a1) / 5 for i in range(4)] + [0, 255])
            for i in range(16): out[by * 4 + i // 4, bx * 4 + i % 4] = pal[(bits >> (3 * i)) & 7] / 255
            q += 16
    return out
tex = dxt5_alpha(a.texture)
rng = np.random.default_rng(a.seed)
def snapshot():
    trans = np.ones((H, W))
    for ci, c in enumerate(clouds):
        mult = a.alpha_mult if ci < track_n else 1.0
        if not (X0 - 7 < c[0] < X1 + 7 and Z0 - 7 < c[2] < Z1 + 7): continue
        r = 5 * np.sqrt(rng.random(100)); th = rng.random(100) * 2 * np.pi
        px, pz = c[0] + r * np.cos(th), c[2] + r * np.sin(th)
        age = rng.random(100); row = rng.random(100)
        peak = (0.191 + (0.132 - 0.191) * row) * mult
        alpha = np.where(age < 0.487, age / 0.487, (1 - age) / (1 - 0.487)) * peak
        size = 1.955 + (3.167 - 1.955) * age; rot = rng.random(100) * 2 * np.pi
        for k in range(100):
            half = size[k] / 2; n = int(math.ceil(half / P))
            ci, cj = (px[k] - X0) / P, (Z1 - pz[k]) / P
            i0, i1 = max(0, int(ci - n)), min(W, int(ci + n) + 1); j0, j1 = max(0, int(cj - n)), min(H, int(cj + n) + 1)
            if i0 >= i1 or j0 >= j1: continue
            gi, gj = np.meshgrid(np.arange(i0, i1) + .5 - ci, np.arange(j0, j1) + .5 - cj)
            cr, sr = math.cos(rot[k]), math.sin(rot[k])
            u = (gi * cr - gj * sr) * P / size[k] + .5; v = (gi * sr + gj * cr) * P / size[k] + .5
            inside = (u >= 0) & (u < 1) & (v >= 0) & (v < 1)
            ta = np.where(inside, tex[np.clip((v * 256).astype(int), 0, 255), np.clip((u * 256).astype(int), 0, 255)], 0)
            visible = top[j0:j1, i0:i1] < c[1] + 0.3   # hidden under a wagon or a roof
            trans[j0:j1, i0:i1] *= 1 - alpha[k] * ta * visible
    return trans
snaps = [snapshot() for _ in range(a.snapshots)]
op = 1 - np.mean(snaps, axis=0)
open_bed = (top > -1e8) & (top < 3.2)
corr = np.zeros_like(open_bed); corr[:, int((9.9 - X0) / P):int((21.6 - X0) / P)] = True
sel = open_bed & corr
stats = dict(project=str(a.project), clouds=len(clouds), window=[X0, X1, Z0, Z1],
             opacity_open_bed_mean=round(float(op[sel].mean()), 3), p10=round(float(np.percentile(op[sel], 10)), 3),
             p50=round(float(np.percentile(op[sel], 50)), 3), p90=round(float(np.percentile(op[sel], 90)), 3),
             share_over_0_8=round(float((op[sel] > 0.8).mean()), 3), share_under_0_5=round(float((op[sel] < 0.5).mean()), 3),
             snapshot_mean=round(float((1 - snaps[0])[sel].mean()), 3),
             # A ray at angle th from the vertical crosses 1/cos(th) times the billboards of a flat layer.
             oblique={f'{d}deg': dict(mean=round(float((1 - (1 - op[sel]) ** (1 / math.cos(math.radians(d)))).mean()), 3),
                                      p10=round(float(np.percentile(1 - (1 - op[sel]) ** (1 / math.cos(math.radians(d))), 10)), 3)) for d in (45, 60)})
print(json.dumps(stats))
shade = np.clip((top - 1.0) / 8.0, 0, 1); base = np.where(top < -1e8, 40, 170 - shade * 110)
ground = np.stack([base * 0.75, base * 0.85, base * 0.6], -1)
one = 1 - snaps[0]
for name, o in (('snapshot', one), ('mean', op)):
    img = ground * (1 - o[..., None]) + 235 * o[..., None]
    Image.fromarray(img.clip(0, 255).astype(np.uint8)).save(f'{a.out}-{name}.png')
json.dump(stats, open(f'{a.out}.json', 'w'), indent=1)
