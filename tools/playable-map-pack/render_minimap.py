#!/usr/bin/env python3
"""Render 0.26-style HUD minimaps of converted maps from their own geometry.

    render_minimap.py --overlay BUNDLE/overlay --spec ports/soc-cs-dm-to-026g.json \
        --output DIR [--map 200 ...]

For each map it writes DIR/<key>.png (the RGBA minimap), DIR/<key>.json (the
HUD entry: size, posX, posY and the projection) and DIR/<key>-preview.png
(over a dark background, spawns marked).

The 0.26 HUD (hud.swf, survarium.controls.Minimap.setProps) shows a W x H
bitmap that covers 2*size metres in each direction around (posX, posY): with
s = W / (2*size) pixels per metre,

    u = (x - posX + size) * s        v = (posY - z + size) * s

i.e. +x to the right and +z up. The engine hands the HUD (x, -z) as the
player position; native maps confirm it (their indoor props fall inside the
drawn buildings, see minimap_world_to_pixel's test).

Style, measured on the native level_03 minimap (the template these maps used
before): open ground is a desaturated top-down photo at alpha 87, buildings
and anything not walkable inside the play area are flat (27,27,27) at alpha
184 with a 1 px light outline, the play-area border is a light line
(110,110,107) at alpha ~180, and outside the play area every channel is 0
(the tab map frames the bitmap with getColorBoundsRect on RGB).

The play area is what a walker can reach from the map's respawn points over
the converted collision (0.25 m cells, 0.5 m steps, 1.7 m headroom), including
the invisible walker ramps. Its holes (buildings, blocks) are filled.
"""
import argparse
import json
import math
from pathlib import Path
import struct
import sys

import numpy as np
from PIL import Image
from scipy import ndimage
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from audit_collision_region import config, chunks, rotation  # noqa: E402

KEYS = {197: 'pool-soc-dm', 198: 'training-camp-cs-dm', 199: 'rembasa-cs-dm',
        200: 'sport-center-cs-dm', 201: 'firestation-cs-dm'}
IMAGE = 512                 # native minimaps are 458-567 px; level_03 is 512
MAX_SCALE = 3.5             # natives: 2.08-2.78 px/m (level_03 2.56)
TAB_AREA = 440              # the tab map lays the bitmap 1:1 into 460 px
CELL = 0.25                 # reachability grid (m)
STEP = 0.5                  # walkable step between neighbouring cells (m)
HEADROOM = 1.7              # free height above a floor (m)
PHOTO = 8                   # photo raster resolution (px per m)
CLIP = 45                   # m around respawns where a map has no border

GROUND_ALPHA = 87
DARK = (27, 27, 27, 184)
LINE = (110, 110, 107)


def world_to_pixel(x, z, size, pos_x, pos_y, width, height):
    """Bitmap pixel of world (x, z) for a HUD entry (size, posX, posY)."""
    return ((x - pos_x + size) * width / (2 * size),
            (pos_y - z + size) * height / (2 * size))


def hud_entry(bounds, width=IMAGE, height=IMAGE):
    """size/posX/posY for a play area with (xmin, zmin, xmax, zmax) bounds."""
    xmin, zmin, xmax, zmax = bounds
    extent = max(xmax - xmin, zmax - zmin)
    scale = min(MAX_SCALE, TAB_AREA / extent)
    size = width / (2 * scale)
    return {'size': round(float(size), 3), 'posX': round(float(xmin + xmax) / 2, 3),
            'posY': round(float(zmin + zmax) / 2, 3)}


# ---------------------------------------------------------------- geometry

def euler(o):
    return np.array(rotation(o.get('rotation', [0, 0, 0])), dtype=np.float64)


def place(points, o):
    """Model space -> world, as the collision audits do (row vectors)."""
    s = np.array(o.get('scale', [1, 1, 1]), dtype=np.float64)
    return (points * s) @ euler(o) + np.array(o.get('position', [0, 0, 0]))


def box_triangles(p):
    """Triangles of a walker box primitive (rows of the rotation are axes)."""
    a = p['rotation']
    sx, cx = math.sin(a[0]), math.cos(a[0])
    sy, cy = math.sin(a[1]), math.cos(a[1])
    sz, cz = math.sin(a[2]), math.cos(a[2])
    r = np.array([[cy*cz, -cy*sz, sy],
                  [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
                  [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]])
    h = np.array(p['scale'])
    corners = np.array([[i, j, k] for i in (-1, 1) for j in (-1, 1) for k in (-1, 1)], dtype=float)
    world = np.array(p['position']) + (corners * h) @ r
    quads = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    tris = []
    for a0, b0, c0, d0 in quads:
        tris += [(a0, b0, c0), (a0, c0, d0)]
    return world[np.array(tris)]


def collision_triangles(overlay, project):
    out = []
    for o in project.get('collision_objects', []):
        path = o['lib_name'].removeprefix('resources/').split('#')[0]
        root = overlay / path
        if (root / 'vertices').exists():
            vs = np.frombuffer(chunks((root / 'vertices').read_bytes())[25], dtype='<f4').reshape(-1, 3)
            ix = np.frombuffer(chunks((root / 'indices').read_bytes())[26], dtype='<u4').reshape(-1, 3)
            out.append(place(vs.astype(np.float64), o)[ix])
        elif (root / 'exported_primitives').exists():
            for p in config((root / 'exported_primitives').read_bytes())['primitives']:
                if p['type'] == 1:
                    tri = box_triangles(p).reshape(-1, 3)
                    out.append(place(tri, o).reshape(-1, 3, 3))
    return np.concatenate(out)


def leaves(d, p=''):
    if isinstance(d, dict):
        for k, v in d.items():
            yield from leaves(v, f'{p}.{k}' if p else k)
    else:
        yield p, d


class Textures:
    def __init__(self, overlay):
        self.overlay = overlay
        self.cache = {}
        self.materials = {}

    def material(self, name):
        """(texture array or None, flat colour or None) for a material."""
        if name in self.materials:
            return self.materials[name]
        path = self.overlay / 'material_instances' / (name + '.material')
        result = (None, None)
        if path.exists():
            slots = {p.split('.')[-2]: v for p, v in leaves(config(path.read_bytes()))
                     if isinstance(v, str) and v and p.endswith('.value') and 'texture' in p}
            colour = slots.get('texture_diffuse') or slots.get('texture_detail_0')
            if colour and not any(t in colour for t in ('/glas', '/glow', '/lights', '/fx/')):
                # A terrain colour map spans the whole map: keep its detail.
                result = (self.texture(colour, 1024 if 'texture_diffuse' not in slots else 64), None)
            elif 'texture_reflection' in slots and len(slots) == 1:
                result = (None, np.array([62., 72., 78.]))       # water
        self.materials[name] = result
        return result

    def texture(self, name, res=64):
        if name not in self.cache:
            path = self.overlay / 'textures' / (name + '.dds')
            self.cache[name] = (np.asarray(Image.open(path).convert('RGB').resize((res, res), Image.BOX),
                                           dtype=np.float32) if path.exists() else None)
        return self.cache[name]


def render_surfaces(overlay, project, textures):
    """[(triangles world (N,3,3), uv (N,3,2), texture, flat colour)] of LOD0."""
    out = []
    for o in project.get('render_visuals', []):
        model = overlay / 'models' / (o['lib_name'] + '.model')
        if not (model / 'settings').exists():
            continue
        settings = config((model / 'settings').read_bytes())
        surfaces = settings.get('lod_hierrarchy', {}).get('LOD0', {}).get('surfaces', [])
        for surface in surfaces:
            data = model / 'render' / surface / 'converted_model'
            if not data.exists():
                continue
            material = settings['material_settings'][surface]['material_name']
            texture, flat = textures.material(material)
            if texture is None and flat is None:
                continue
            c = chunks(data.read_bytes())
            count = struct.unpack_from('<I', c[3])[0]
            stride = (len(c[3]) - 4) // count
            raw = np.frombuffer(c[3], dtype=np.uint8, offset=4).reshape(count, stride)
            pos = raw[:, :12].copy().view('<f4').reshape(-1, 3).astype(np.float64)
            uv = raw[:, 24:28].copy().view('<f2').reshape(-1, 2).astype(np.float64)
            ix = np.frombuffer(c[4], dtype='<u2', offset=4).reshape(-1, 3)
            out.append((place(pos, o)[ix], uv[ix], texture, flat))
    return out


# ------------------------------------------------------------- rasterizer

def subdivide(tri, attr, limit, width, height):
    """Split triangles until their pixel bounding box is at most limit,
    dropping pieces outside the width x height raster."""
    done_t, done_a = [], []
    while len(tri):
        lo, hi = tri[:, :, :2].min(1), tri[:, :, :2].max(1)
        seen = (hi[:, 0] >= 0) & (hi[:, 1] >= 0) & (lo[:, 0] <= width) & (lo[:, 1] <= height)
        tri, attr = tri[seen], attr[seen]
        span = tri[:, :, :2].max(1) - tri[:, :, :2].min(1)
        big = span.max(1) > limit
        done_t.append(tri[~big]); done_a.append(attr[~big])
        t, a = tri[big], attr[big]
        if not len(t):
            break
        m = lambda x, i, j: (x[:, i] + x[:, j]) / 2
        tm = [m(t, 0, 1), m(t, 1, 2), m(t, 2, 0)]
        am = [m(a, 0, 1), m(a, 1, 2), m(a, 2, 0)]
        pick = lambda v, x: np.stack(x, 1)
        tri = np.concatenate([pick(t, [t[:, 0], tm[0], tm[2]]), pick(t, [tm[0], t[:, 1], tm[1]]),
                              pick(t, [tm[2], tm[1], t[:, 2]]), pick(t, [tm[0], tm[1], tm[2]])])
        attr = np.concatenate([pick(a, [a[:, 0], am[0], am[2]]), pick(a, [am[0], a[:, 1], am[1]]),
                               pick(a, [am[2], am[1], a[:, 2]]), pick(a, [am[0], am[1], am[2]])])
    return np.concatenate(done_t), np.concatenate(done_a)


def rasterize(tri, attr, width, height):
    """Pixel-centre samples of screen triangles (u, v, height) with attributes.

    Returns (pixel index, height, interpolated attributes) for every covered
    pixel centre. Triangles are processed by bounding-box size class."""
    tri, attr = subdivide(tri, attr, 13, width, height)
    if not len(tri):
        return np.zeros(0, np.int64), np.zeros(0), np.zeros((0, attr.shape[2]))
    lo = np.floor(tri[:, :, :2].min(1) - 0.5).astype(np.int64)
    out_p, out_h, out_a = [], [], []
    for size in (2, 4, 8, 16, 32):
        span = np.ceil(tri[:, :, :2].max(1) - 0.5).astype(np.int64) - lo + 1
        sel = (span.max(1) <= size) & (span.max(1) > size // 2 if size > 2 else True)
        if not sel.any():
            continue
        t, a, l = tri[sel], attr[sel], lo[sel]
        gx, gy = np.meshgrid(np.arange(size), np.arange(size))
        px = l[:, None, 0] + gx.ravel()[None]
        py = l[:, None, 1] + gy.ravel()[None]
        cx, cy = px + 0.5, py + 0.5
        x0, y0 = t[:, 0, 0:1], t[:, 0, 1:2]
        x1, y1 = t[:, 1, 0:1], t[:, 1, 1:2]
        x2, y2 = t[:, 2, 0:1], t[:, 2, 1:2]
        den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        ok = np.abs(den) > 1e-12
        den = np.where(ok, den, 1.0)
        w0 = ((y1 - y2) * (cx - x2) + (x2 - x1) * (cy - y2)) / den
        w1 = ((y2 - y0) * (cx - x2) + (x0 - x2) * (cy - y2)) / den
        w2 = 1 - w0 - w1
        inside = ok & (w0 >= -1e-9) & (w1 >= -1e-9) & (w2 >= -1e-9) \
            & (px >= 0) & (py >= 0) & (px < width) & (py < height)
        k, j = np.nonzero(inside)
        w = np.stack([w0[k, j], w1[k, j], w2[k, j]], 1)
        out_p.append(py[k, j] * width + px[k, j])
        out_h.append((w * t[k, :, 2]).sum(1))
        out_a.append(np.einsum('ni,nij->nj', w, a[k]))
    if not out_p:
        return np.zeros(0, np.int64), np.zeros(0), np.zeros((0, attr.shape[2]))
    return np.concatenate(out_p), np.concatenate(out_h), np.concatenate(out_a)


def top(pixels, heights, n):
    """Index of the highest sample per pixel, as (pixels, sample index)."""
    order = np.lexsort((heights, pixels))
    p = pixels[order]
    last = np.r_[p[1:] != p[:-1], True]
    return p[last], order[last]


# ------------------------------------------------------------ reachability

def play_area(tris, spawns, bounds, step=None, all_floors=False):
    """Per CELL: the lowest floor a walker reaches from the spawns (NaN
    where none) and whether something lies 1.7-12 m above that floor.
    With all_floors, also every reachable (cell index, height) and the grid
    width."""
    step = STEP if step is None else step
    x0, z0, x1, z1 = bounds
    ylo = min(s[1] for s in spawns) - 10; yhi = max(s[1] for s in spawns) + 20
    w, h = int(math.ceil((x1 - x0) / CELL)), int(math.ceil((z1 - z0) / CELL))
    n = np.cross(tris[:, 1] - tris[:, 0], tris[:, 2] - tris[:, 0])
    area2 = np.linalg.norm(n, axis=1)
    ny = np.where(area2 > 0, n[:, 1] / np.maximum(area2, 1e-12), 0)
    walk = np.abs(ny) > 0.7         # either winding: X-Ray cform faces up or down
    flat = np.abs(ny) > 0.3
    # Flat-ish triangles: exact coverage of the cell centres.
    screen = np.stack([(tris[..., 0] - x0) / CELL, (z1 - tris[..., 2]) / CELL, tris[..., 1]], -1)
    keep = flat & (area2 > 0)
    pix, hgt, att = rasterize(screen[keep], np.repeat(walk[keep, None, None], 3, 1).astype(float), w, h)
    samples_p, samples_h, samples_w = [pix], [hgt], [att[:, 0] > 0.5]
    # Steep triangles (walls): split until edges are under 1 m, then sample
    # every 0.1 m so each cell a wall passes gets its heights.
    steep = tris[~flat & (area2 > 0)]
    while len(steep):
        lo, hi = steep.min(1), steep.max(1)
        keep = (hi[:, 0] >= x0) & (lo[:, 0] <= x1) & (hi[:, 2] >= z0) & (lo[:, 2] <= z1)
        steep, lo, hi = steep[keep], lo[keep], hi[keep]
        steep = steep[(hi[:, 1] >= ylo) & (lo[:, 1] <= yhi)]
        edge = np.linalg.norm(steep - np.roll(steep, 1, 1), axis=2).max(1)
        big = edge > 1.0
        t = steep[~big]
        m = 5
        i, j = np.meshgrid(np.arange(m + 1), np.arange(m + 1))
        ok = (i + j) <= m
        a, b = (i[ok] / m)[None, :, None], (j[ok] / m)[None, :, None]
        pts = (t[:, None, 0] * (1 - a - b) + t[:, None, 1] * a + t[:, None, 2] * b).reshape(-1, 3)
        cx = np.floor((pts[:, 0] - x0) / CELL).astype(np.int64)
        cz = np.floor((z1 - pts[:, 2]) / CELL).astype(np.int64)
        inb = (cx >= 0) & (cz >= 0) & (cx < w) & (cz < h)
        samples_p.append(cz[inb] * w + cx[inb]); samples_h.append(pts[inb, 1])
        samples_w.append(np.zeros(inb.sum(), bool))
        t = steep[big]
        mid = lambda i, j: (t[:, i] + t[:, j]) / 2
        a01, a12, a20 = mid(0, 1), mid(1, 2), mid(2, 0)
        steep = np.concatenate([np.stack(q, 1) for q in (
            [t[:, 0], a01, a20], [a01, t[:, 1], a12], [a20, a12, t[:, 2]], [a01, a12, a20])])
    p = np.concatenate(samples_p); hh = np.concatenate(samples_h); ww = np.concatenate(samples_w)
    key = p * 1e4 + hh                      # heights stay within +-5 km
    order = np.argsort(key, kind='stable'); p, hh, ww, key = p[order], hh[order], ww[order], key[order]
    # Floors: walkable samples with no sample in (h + 0.35, h + HEADROOM].
    q = np.nonzero(ww)[0]
    end = np.searchsorted(p, p[q], 'right')
    nxt = np.searchsorted(key, key[q] + 0.35, 'right')
    blocked = (nxt < end) & (hh[np.minimum(nxt, len(hh) - 1)] <= hh[q] + HEADROOM)
    # A floor is covered when anything lies 1.7..12 m above it.
    up = np.searchsorted(key, key[q] + HEADROOM, 'right')
    cover = (up < end) & (hh[np.minimum(up, len(hh) - 1)] <= hh[q] + 12)
    floor = q[~blocked]
    covered = np.zeros(len(p), bool); covered[q[cover]] = True
    fp, fh, fc = p[floor], hh[floor], covered[floor]
    # Merge floors closer than 0.2 m within a cell.
    keep = np.r_[True, (fp[1:] != fp[:-1]) | (fh[1:] - fh[:-1] > 0.2)]
    fp, fh, fc = fp[keep], fh[keep], fc[keep]
    # Graph: 4-neighbour cells, floors within STEP.
    edges = []
    for d in (1, w):
        a = np.arange(len(fp))
        target = fp + d
        valid = (target < w * h) & ((d != 1) | (fp % w != w - 1))
        lo = np.searchsorted(fp, target[valid], 'left'); hi = np.searchsorted(fp, target[valid], 'right')
        src = a[valid]
        span = hi - lo
        for k in range(int(span.max()) if len(span) else 0):
            ok = span > k
            j = lo[ok] + k
            close = np.abs(fh[j] - fh[src[ok]]) <= step
            edges.append(np.stack([src[ok][close], j[close]]))
    e = np.concatenate(edges, 1)
    graph = coo_matrix((np.ones(e.shape[1]), (e[0], e[1])), shape=(len(fp), len(fp)))
    _, label = connected_components(graph, directed=False)
    seeds = set()
    for x, y, z in spawns:
        c = int((z1 - z) // CELL) * w + int((x - x0) // CELL)
        lo, hi = np.searchsorted(fp, c, 'left'), np.searchsorted(fp, c, 'right')
        best = [k for k in range(lo, hi) if abs(fh[k] - y) < 0.6]
        if not best:
            raise SystemExit(f'spawn {x, y, z} has no floor cell')
        seeds.add(label[best[0]])
    reach = np.isin(label, list(seeds))
    rp, rh, rc = fp[reach], fh[reach], fc[reach]
    last = np.r_[True, rp[1:] != rp[:-1]]          # lowest reachable floor
    floor_h = np.full(w * h, np.nan); covered_ = np.zeros(w * h, bool)
    floor_h[rp[last]] = rh[last]; covered_[rp[last]] = rc[last]
    if all_floors:
        return floor_h.reshape(h, w), covered_.reshape(h, w), (rp, rh, w)
    return floor_h.reshape(h, w), covered_.reshape(h, w)


# ------------------------------------------------------------------ render

def disk(r):
    y, x = np.ogrid[-r:r + 1, -r:r + 1]
    return x * x + y * y <= r * r


def render_map(overlay, spec_map, out):
    key = KEYS[spec_map['id']]
    project = config((overlay / 'projects' / f"v010b_{spec_map['donor_project']}" / 'day_4' /
                      'client_project').read_bytes())
    spawns = [r[:3] for r in spec_map['deathmatch']['respawns']]
    sx = [s[0] for s in spawns]; sz = [s[2] for s in spawns]
    margin = 150
    bounds = (min(sx) - margin, min(sz) - margin, max(sx) + margin, max(sz) + margin)
    x0, z0, x1, z1 = bounds
    out.mkdir(parents=True, exist_ok=True)
    floor_h, covered = play_area(collision_triangles(overlay, project), spawns, bounds)
    reach = np.isfinite(floor_h)
    if reach[0].any() or reach[-1].any() or reach[:, 0].any() or reach[:, -1].any():
        # No border: open terrain runs past the margin (Training Camp's
        # meadow). Keep what lies within CLIP metres of a respawn point.
        near = np.zeros_like(reach)
        jj, ii = np.mgrid[0:reach.shape[0], 0:reach.shape[1]]
        for x, _, z in spawns:
            near |= (x0 + (ii + 0.5) * CELL - x) ** 2 + (z1 - (jj + 0.5) * CELL - z) ** 2 <= CLIP ** 2
        floor_h = np.where(near, floor_h, np.nan)
        reach = np.isfinite(floor_h)
    np.savez_compressed(out / f'{key}-cells.npz', floor_h=floor_h, covered=covered, bounds=np.array(bounds))
    # Close small gaps (fences, posts) and fill enclosed holes (buildings).
    region = ndimage.binary_fill_holes(ndimage.binary_closing(reach, disk(6)))
    region = ndimage.binary_opening(region, disk(2)) | reach
    ys, xs = np.nonzero(region)
    area = (x0 + xs.min() * CELL, z1 - (ys.max() + 1) * CELL, x0 + (xs.max() + 1) * CELL, z1 - ys.min() * CELL)
    entry = hud_entry(area)
    size, pos_x, pos_y = entry['size'], entry['posX'], entry['posY']
    scale = IMAGE / (2 * size)

    def cell_of(u, z):
        return np.floor((z1 - z) / CELL).astype(np.int64), np.floor((u - x0) / CELL).astype(np.int64)

    # Photo at PHOTO px/m: the highest render surface at most 1.2 m above
    # the cell's reachable floor (so interiors show their floor), the top
    # surface where nothing is reachable.
    fw = int(round(2 * size * PHOTO))
    cu = (np.arange(fw) + 0.5) / PHOTO + pos_x - size
    cz = pos_y + size - (np.arange(fw) + 0.5) / PHOTO
    ci = np.floor((cu - x0) / CELL).astype(np.int64); cj = np.floor((z1 - cz) / CELL).astype(np.int64)
    okx = (ci >= 0) & (ci < floor_h.shape[1]); oky = (cj >= 0) & (cj < floor_h.shape[0])
    cut = np.full((fw, fw), np.inf)
    sub = floor_h[np.clip(cj, 0, floor_h.shape[0] - 1)][:, np.clip(ci, 0, floor_h.shape[1] - 1)] + 1.2
    sub[~(oky[:, None] & okx[None])] = np.inf
    cut = np.where(np.isfinite(sub), sub, np.inf).ravel()
    textures = Textures(overlay)
    pix_all, h_all, col_all = [], [], []
    for tri, uv, texture, flat in render_surfaces(overlay, project, textures):
        screen = np.stack([(tri[..., 0] - (pos_x - size)) * PHOTO,
                           ((pos_y + size) - tri[..., 2]) * PHOTO, tri[..., 1]], -1)
        pix, hgt, a = rasterize(screen, uv, fw, fw)
        below = hgt <= cut[pix]
        pix, hgt, a = pix[below], hgt[below], a[below]
        if not len(pix):
            continue
        if texture is not None:
            res = texture.shape[0]
            u = (np.floor(a[:, 0] * res).astype(np.int64)) % res
            v = (np.floor(a[:, 1] * res).astype(np.int64)) % res
            col = texture[v, u]
        else:
            col = np.broadcast_to(flat, (len(pix), 3))
        pix_all.append(pix); h_all.append(hgt); col_all.append(col)
    pix, h, col = np.concatenate(pix_all), np.concatenate(h_all), np.concatenate(col_all)
    p, idx = top(pix, h, fw * fw)
    photo = np.zeros((fw * fw, 3), np.float32)
    photo[p] = col[idx]
    photo = photo.reshape(fw, fw, 3)
    # Native look: mostly desaturated, mid-grey.
    lum = photo @ np.array([0.299, 0.587, 0.114], np.float32)
    photo = lum[..., None] * 0.7 + photo * 0.3
    seen = photo.reshape(-1, 3)[p].mean(1)
    lo, hi = np.percentile(seen, 5), np.percentile(seen, 95)
    photo = 45 + (photo - lo) * (105 / max(hi - lo, 1.0))
    photo = np.asarray(Image.fromarray(photo.clip(0, 255).astype(np.uint8)).resize((IMAGE, IMAGE), Image.LANCZOS),
                       np.float32)

    def resample(mask):
        u = (np.arange(IMAGE) + 0.5) / scale + pos_x - size
        z = pos_y + size - (np.arange(IMAGE) + 0.5) / scale
        i = np.floor((u - x0) / CELL).astype(int); j = np.floor((z1 - z) / CELL).astype(int)
        okx = (i >= 0) & (i < mask.shape[1]); oky = (j >= 0) & (j < mask.shape[0])
        out = np.zeros((IMAGE, IMAGE), np.float32)
        sub = mask.astype(np.float32)[np.clip(j, 0, mask.shape[0] - 1)][:, np.clip(i, 0, mask.shape[1] - 1)]
        out[np.ix_(oky, okx)] = sub[np.ix_(oky, okx)]
        return out
    walk = ndimage.binary_opening(reach, disk(1)) & region
    dark = ndimage.binary_dilation(region & ~walk, disk(1)) & region
    labels, count = ndimage.label(dark)
    sizes = ndimage.sum(dark, labels, np.arange(1, count + 1))
    dark = np.isin(labels, 1 + np.nonzero(sizes >= 8)[0])      # drop specks under 0.5 m2
    indoor = walk & ndimage.binary_opening(covered, disk(2))
    region_i = ndimage.gaussian_filter(resample(region), 0.6)
    dark_i = ndimage.gaussian_filter(resample(dark), 0.5)
    indoor_i = ndimage.gaussian_filter(resample(indoor), 0.7) * (1 - dark_i)
    rgba = np.zeros((IMAGE, IMAGE, 4), np.float32)
    # Outdoor ground: photo at the native ground alpha. Indoor floors: the
    # same photo, darker and a little more opaque. Walls and blocks: flat dark.
    rgb = photo * (1 - 0.35 * indoor_i[..., None])
    alpha = GROUND_ALPHA + (120 - GROUND_ALPHA) * indoor_i
    dark_rgb = DARK[0] + 0.12 * (photo - photo.mean())
    rgb = rgb * (1 - dark_i[..., None]) + dark_rgb * dark_i[..., None]
    alpha = alpha * (1 - dark_i) + DARK[3] * dark_i
    edge_b = np.clip(ndimage.morphological_gradient(dark_i, size=2), 0, 1) * (region_i > 0.5)
    edge_r = np.clip(region_i * (1 - ndimage.grey_erosion(region_i, size=3)) * 1.6, 0, 1)
    for e, a in ((edge_b, 150), (edge_r, 180)):
        rgb = rgb * (1 - e[..., None]) + np.array(LINE) * e[..., None]
        alpha = alpha * (1 - e) + a * e
    alpha = alpha * np.clip(region_i * 2.5, 0, 1)
    rgba[..., :3] = rgb; rgba[..., 3] = alpha
    rgba[rgba[..., 3] < 1] = 0
    final = Image.fromarray(rgba.round().clip(0, 255).astype(np.uint8), 'RGBA')
    final.save(out / f'{key}.png')
    preview = Image.new('RGBA', final.size, (40, 40, 40, 255)); preview.alpha_composite(final)
    px = preview.load()
    for x, _, z in spawns:
        u, v = world_to_pixel(x, z, size, pos_x, pos_y, IMAGE, IMAGE)
        for du in range(-1, 2):
            for dv in range(-1, 2):
                if 0 <= u + du < IMAGE and 0 <= v + dv < IMAGE:
                    px[int(u + du), int(v + dv)] = (255, 220, 0, 255)
    preview.save(out / f'{key}-preview.png')
    record = dict(entry, key=key, map_id=spec_map['id'], width=IMAGE, height=IMAGE,
                  pixels_per_metre=round(scale, 4),
                  play_area={'xmin': round(area[0], 2), 'zmin': round(area[1], 2),
                             'xmax': round(area[2], 2), 'zmax': round(area[3], 2)},
                  projection='u = (x - posX + size) * W / (2 size), v = (posY - z + size) * H / (2 size)')
    (out / f'{key}.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--overlay', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--map', type=int, action='append')
    a = ap.parse_args()
    spec = json.loads(a.spec.read_text())
    for m in spec['maps']:
        if a.map and m['id'] not in a.map:
            continue
        print(json.dumps(render_map(a.overlay, m, a.output)), flush=True)


if __name__ == '__main__':
    main()
