#!/usr/bin/env python3
"""Candidate camera poses for the deathmatch maps' loading-screen shots.

    loading_poses.py --overlay BUNDLE/overlay --spec ports/soc-cs-dm-to-026g.json \
        --output DIR [--map 200 ...] [--preview]

For every landmark in LANDMARKS the tool samples camera positions around the
landmark (rings of radii and heights above the floor below the camera) and
keeps the ones where, against the map's converted collision:
  - the camera is inside the play area and at least CLEARANCE from any
    surface (rays in 26 directions),
  - the landmark point is in line of sight,
  - the view is open: the frustum rays (100 x 68 degrees, the loading
    screen's 16:9 capture) travel far before they hit (score).
It writes DIR/<map key>.json in capture_dm_gallery.py's pose format, best
first, at most three per map and 45 degrees apart per landmark:
  {"name", "position", "target", "fov": vertical degrees, "aspect": 16/9}
With --preview it also draws each pose (textured, unlit, z-buffered render
of the converted geometry) to DIR/<map key>-<name>.png, cropped to the
1920:798 loading-screen band.
"""
import argparse
import json
import math
from pathlib import Path
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_minimap as rm  # noqa: E402

ASPECT = 16 / 9
HFOV = 100.0                                      # degrees, the native shots' width
VFOV = math.degrees(2 * math.atan(math.tan(math.radians(HFOV / 2)) / ASPECT))   # 67.7
CLEARANCE = 0.6
BAND = 798 / 1920                                 # loading image height / width

# Landmark: target point, camera radii, heights above the floor under the
# camera, whether the shot is indoors (the camera must stay under a roof),
# how close to the target the line of sight must reach (`near`, m) and the
# steepest upward look (`max_pitch`, degrees; default 3).
LANDMARKS = {
    197: [dict(name='pool-hall', target=[20.0, 0.5, -37.0], radii=[8, 11, 14, 17], heights=[2.5, 4, 6], indoor=True)],
    198: [dict(name='camp-overview', target=[24.0, 3.0, 4.0], radii=[30, 40, 50, 60], heights=[3, 6, 10], indoor=False),
          dict(name='meadow-forest', target=None, radii=[20, 30, 40], heights=[1.8, 3, 5], indoor=False, near=15)],
    199: [dict(name='depot-yard', target=None, radii=[15, 22, 30, 38], heights=[3, 6, 9], indoor=False)],
    200: [dict(name='pool-hall', target=[-91.0, -2.5, -36.0], radii=[6, 9, 12, 15], heights=[2.5, 4, 6, 8], indoor=True),
          dict(name='sports-hall', target=[-62.0, 0.5, -36.0], radii=[8, 12, 16, 20], heights=[2.5, 4, 6, 8], indoor=True)],
    # The tower: the tall block at x 40-52, z 6-22 (collision to 37 m) that
    # the ladder chain clmbl#7-#10 climbs; seen from the yard to the west.
    201: [dict(name='fire-tower', target=[45.0, 22.0, 12.0], radii=[25, 32, 40, 50, 60], heights=[1.8, 3.5, 6],
               indoor=False, near=8, max_pitch=25)],
}


class Scene:
    """Collision triangles of a region with a coarse grid for ray casts."""

    def __init__(self, tris, cell=4.0):
        self.t = tris.astype(np.float64)
        self.e1 = self.t[:, 1] - self.t[:, 0]
        self.e2 = self.t[:, 2] - self.t[:, 0]
        self.lo = self.t.min(1)
        self.hi = self.t.max(1)

    def cast(self, origins, directions, far=80.0):
        """Distance to the first hit per ray (far where none)."""
        out = np.full(len(origins), far)
        for i in range(0, len(origins), 32):
            o = origins[i:i + 32, None, :]
            d = directions[i:i + 32, None, :]
            # Cull triangles outside the rays' joint bounding box.
            ends = origins[i:i + 32] + directions[i:i + 32] * far
            blo = np.minimum(origins[i:i + 32], ends).min(0) - 0.1
            bhi = np.maximum(origins[i:i + 32], ends).max(0) + 0.1
            sel = np.all((self.hi >= blo) & (self.lo <= bhi), axis=1)
            if not sel.any():
                continue
            v0, e1, e2 = self.t[sel, 0][None], self.e1[sel][None], self.e2[sel][None]
            p = np.cross(d, e2)
            det = (e1 * p).sum(-1)
            ok = np.abs(det) > 1e-9
            inv = np.where(ok, 1 / np.where(ok, det, 1), 0)
            s = o - v0
            u = (s * p).sum(-1) * inv
            q = np.cross(s, e1)
            v = (d * q).sum(-1) * inv
            t = (e2 * q).sum(-1) * inv
            hit = ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t > 1e-4) & (t < far)
            t = np.where(hit, t, far)
            out[i:i + 32] = np.minimum(out[i:i + 32], t.min(1))
        return out


def unit(v):
    v = np.asarray(v, np.float64)
    return v / np.linalg.norm(v, axis=-1, keepdims=True)


def basis(forward):
    f = unit(forward)
    r = unit(np.cross([0, 1, 0], f))       # left-handed: x right, y up, z forward
    u = np.cross(f, r)
    return f, r, u


def frustum(forward, nx=9, ny=5):
    """Rays over the loading-screen band of a 16:9 frame."""
    f, r, u = basis(forward)
    xs = np.linspace(-1, 1, nx) * math.tan(math.radians(HFOV / 2))
    ys = np.linspace(-1, 1, ny) * math.tan(math.radians(VFOV / 2)) * (BAND * ASPECT)
    return unit(np.array([f + x * r + y * u for y in ys for x in xs]))


SPHERE = unit(np.array([[x, y, z] for x in (-1, 0, 1) for y in (-1, 0, 1) for z in (-1, 0, 1)
                        if (x, y, z) != (0, 0, 0)], np.float64))


def floor_below(scene, x, z, top):
    d = scene.cast(np.array([[x, top, z]]), np.array([[0.0, -1.0, 0.0]]), far=200)[0]
    return top - d if d < 200 else None


def landmark_target(spec_map, landmark, project):
    if landmark['target'] is not None:
        return np.array(landmark['target'])
    if landmark['name'] == 'meadow-forest':
        # The near forest belt: median position of the single conifers.
        trees = [v['position'] for v in project['render_visuals'] if 'flora/elka' in v['lib_name']
                 and v.get('position') and not (v['position'][0] == 0 and v['position'][2] == 0)]
        trees = np.array(trees)
        centre = np.array([24.0, 0.0, 4.0])
        d = np.linalg.norm(trees[:, [0, 2]] - centre[[0, 2]], axis=1)
        near = trees[d < np.percentile(d, 25)]
        # The quarter of the belt with the most trees.
        ang = np.arctan2(near[:, 2] - centre[2], near[:, 0] - centre[0])
        hist, edges = np.histogram(ang, bins=8, range=(-math.pi, math.pi))
        a = (edges[hist.argmax()] + edges[hist.argmax() + 1]) / 2
        r = np.median(d[d < np.percentile(d, 25)])
        return np.array([centre[0] + r * math.cos(a), near[:, 1].mean() + 4, centre[2] + r * math.sin(a)])
    if landmark['name'] == 'depot-yard':
        # Ground-level respawns among the depot buildings (north cluster).
        pts = np.array([p[:3] for p in spec_map['deathmatch']['respawns'] if -1 < p[1] < 1.5 and p[2] > -60])
        c = pts.mean(0)
        return np.array([c[0], c[1] + 1.0, c[2]])
    raise ValueError(landmark['name'])


def search(scene, region, spec_map, landmark, target, project):
    x0, z1, cell = region['x0'], region['z1'], region['cell']
    mask = region['mask']

    floor_h = region['floor']

    def inside(x, z):
        i, j = int((x - x0) // cell), int((z1 - z) // cell)
        return 0 <= j < mask.shape[0] and 0 <= i < mask.shape[1] and mask[j, i]

    def ground_at(x, z):
        """Lowest reachable floor within 1 m (NaN off the walkable area)."""
        i, j = int((x - x0) // cell), int((z1 - z) // cell)
        w = floor_h[max(j - 4, 0):j + 5, max(i - 4, 0):i + 5]
        return float(np.nanmin(w)) if np.isfinite(w).any() else None
    out = []
    for radius in landmark['radii']:
        for k in range(24):
            a = 2 * math.pi * k / 24
            x, z = target[0] + radius * math.cos(a), target[2] + radius * math.sin(a)
            if not inside(x, z):
                continue
            ground = ground_at(x, z)
            if ground is None:
                continue
            for h in landmark['heights']:
                cam = np.array([x, ground + h, z])
                if landmark['indoor']:
                    up = scene.cast(cam[None], np.array([[0, 1.0, 0]]), far=25)[0]
                    if up >= 25:
                        continue
                    if up < 1.0:
                        continue
                if scene.cast(np.repeat(cam[None], len(SPHERE), 0), SPHERE, far=CLEARANCE).min() < CLEARANCE:
                    continue
                to = target - cam
                dist = np.linalg.norm(to)
                # In sight: the ray reaches the landmark or a surface within
                # `near` metres of it (a closed tower, a forest on a hill).
                if scene.cast(cam[None], unit(to)[None], far=dist)[0] < dist - landmark.get('near', 1.0):
                    continue
                # Aim at the target, but nearly level (pitch -18..+3 degrees),
                # as the native shots: the frame holds the room or the sky.
                flat = math.hypot(to[0], to[2])
                pitch = min(max(math.atan2(to[1], flat), math.radians(-18)),
                            math.radians(landmark.get('max_pitch', 3)))
                forward = np.array([to[0] / flat * math.cos(pitch), math.sin(pitch), to[2] / flat * math.cos(pitch)])
                rays = frustum(forward)
                d = scene.cast(np.repeat(cam[None], len(rays), 0), rays, far=60)
                score = float(np.mean(np.minimum(d, 40)) - 6 * np.mean(d < 2.5))
                out.append(dict(score=score, angle=a, position=cam, forward=forward, distance=dist))
    out.sort(key=lambda c: -c['score'])
    chosen = []
    for c in out:
        if all(abs((c['angle'] - o['angle'] + math.pi) % (2 * math.pi) - math.pi) >= math.radians(45) for o in chosen):
            chosen.append(c)
        if len(chosen) == 3:
            break
    return chosen


def preview(overlay, project, pose, path, width=960):
    """Unlit textured perspective render of the converted geometry."""
    height = int(width / ASPECT)
    cam = np.array(pose['position'])
    f, r, u = basis(np.array(pose['target']) - cam)
    fy = 1 / math.tan(math.radians(VFOV / 2))
    fx = fy / ASPECT
    textures = rm.Textures(overlay)
    pix_all, depth_all, col_all = [], [], []
    for tri, uv, texture, flat in rm.render_surfaces(overlay, project, textures):
        rel = tri - cam
        zc = rel @ f
        keep = (zc > 0.3).all(1)
        if not keep.any():
            continue
        rel, zc, uv = rel[keep], zc[keep], uv[keep]
        sx = (rel @ r) / zc * fx
        sy = (rel @ u) / zc * fy
        vis = ~((sx < -1.2).all(1) | (sx > 1.2).all(1) | (sy < -1.2).all(1) | (sy > 1.2).all(1)) & (zc < 400).all(1)
        if not vis.any():
            continue
        screen = np.stack([(sx[vis] + 1) * width / 2, (1 - sy[vis]) * height / 2, -zc[vis]], -1)
        p, dpt, a = rm.rasterize(screen, uv[vis], width, height)
        if not len(p):
            continue
        if texture is not None:
            res = texture.shape[0]
            col = texture[(np.floor(a[:, 1] * res).astype(np.int64)) % res,
                          (np.floor(a[:, 0] * res).astype(np.int64)) % res]
        else:
            col = np.broadcast_to(flat, (len(p), 3))
        pix_all.append(p); depth_all.append(dpt); col_all.append(col)
    img = np.zeros((width * height, 3), np.float32)
    img[:] = (150, 170, 190)                      # sky
    if pix_all:
        p, idx = rm.top(np.concatenate(pix_all), np.concatenate(depth_all), width * height)
        dz = -np.concatenate(depth_all)[idx]
        fog = np.clip(dz / 300, 0, 0.6)[:, None]
        img[p] = np.concatenate(col_all)[idx] * (1 - fog) + np.array([150, 170, 190]) * fog
    img = img.reshape(height, width, 3)
    band = int(round(width * BAND))
    top = (height - band) // 2
    Image.fromarray(img[top:top + band].clip(0, 255).astype(np.uint8)).save(path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--overlay', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--map', type=int, action='append')
    ap.add_argument('--preview', action='store_true')
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    spec = json.loads(a.spec.read_text())
    for m in spec['maps']:
        if a.map and m['id'] not in a.map:
            continue
        key = rm.KEYS[m['id']]
        project = rm.config((a.overlay / 'projects' / f"v010b_{m['donor_project']}" / 'day_4' /
                             'client_project').read_bytes())
        tris = rm.collision_triangles(a.overlay, project)
        spawns = [p[:3] for p in m['deathmatch']['respawns']]
        sx = [s[0] for s in spawns]; sz = [s[2] for s in spawns]
        bounds = (min(sx) - 150, min(sz) - 150, max(sx) + 150, max(sz) + 150)
        floor_h, _ = rm.play_area(tris, spawns, bounds)
        reach = np.isfinite(floor_h)
        from scipy import ndimage
        mask = ndimage.binary_fill_holes(ndimage.binary_closing(reach, rm.disk(6)))
        ys, xs = np.nonzero(mask)
        x0, z1 = bounds[0], bounds[3]
        lo = (x0 + xs.min() * rm.CELL - 30, z1 - ys.max() * rm.CELL - 30)
        hi = (x0 + xs.max() * rm.CELL + 30, z1 - ys.min() * rm.CELL + 30)
        c = tris.mean(1)
        scene = Scene(tris[(c[:, 0] > lo[0]) & (c[:, 0] < hi[0]) & (c[:, 2] > lo[1]) & (c[:, 2] < hi[1])])
        region = dict(x0=x0, z1=z1, cell=rm.CELL, mask=mask, floor=floor_h)
        poses = []
        for landmark in LANDMARKS[m['id']]:
            target = landmark_target(m, landmark, project)
            for n, cand in enumerate(search(scene, region, m, landmark, target, project)):
                poses.append(dict(name=f"{landmark['name']}-{n + 1}",
                                  position=[round(float(v), 3) for v in cand['position']],
                                  target=[round(float(v), 3) for v in cand['position'] + cand['forward'] * cand['distance']],
                                  fov=round(VFOV, 2), aspect=round(ASPECT, 6),
                                  landmark=[round(float(v), 2) for v in target],
                                  score=round(cand['score'], 2)))
        # Two or three per map: the best of each landmark first, then the rest.
        firsts = [p for p in poses if p['name'].endswith('-1')]
        rest = sorted([p for p in poses if not p['name'].endswith('-1')], key=lambda p: -p['score'])
        poses = (firsts + rest)[:max(3, len(firsts))]
        (a.output / f'{key}.json').write_text(json.dumps(poses, indent=1) + '\n')
        print(key, [(p['name'], p['score']) for p in poses], flush=True)
        if a.preview:
            for p in poses:
                preview(a.overlay, project, p, a.output / f"{key}-{p['name']}.png")


if __name__ == '__main__':
    main()
