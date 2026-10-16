#!/usr/bin/env python3
"""Propose interior box probes for converted maps from sky visibility.

    interior_probes.py --overlay BUNDLE/overlay --spec ports/soc-cs-dm-to-026g.json \
        [--cells DIR] [--map 200 ...] [--write] [--preview DIR]

0.10b lit every converted X-Ray room like open ground: its skylight stage is
an ambient cube scaled only by SSAO. The skylight probe that ports it to 0.26
(src/survarium_026/skylight.rs) is therefore flat indoors. Native 0.26 maps
darken interiors with box probes; this tool places them where a walker is
under cover.

For every 1 m cell of the floor a walker reaches from the respawns (the
minimap renderer's reachability; its `<key>-cells.npz` caches are reused with
--cells), it marches 48 cosine-distributed upper-hemisphere rays from 1.2 m
above the floor through a 0.5 m voxelization of the converted collision, up
to 60 m. The fraction that escapes is the cell's sky visibility. Cells under
cover (visibility < COVERED) are grouped per storey into axis-aligned boxes:
connected regions are split until each box is mostly covered, small boxes
are dropped and at most MAX_BOXES (the largest) are kept per map. A box runs
from its lowest floor to its highest ceiling (vertical rays) and reaches
PAD metres past its covered cells, so the probe's 1 m falloff
(INTERIOR_SIDE_WIDTH) ends near the walls and in doorways. Its diffuse
multiplier follows the box's mean visibility: ENCLOSED for a closed room up to
OPEN for a hall with large windows or a broken roof.

Prints the entries per map as JSON; --write replaces each map's
`skylight_probe.interiors` in the spec (other spec text is kept);
--preview DIR draws the boxes over each map's area.
"""
import argparse
import json
import math
from pathlib import Path
import sys

import numpy as np
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_minimap as mm  # noqa: E402

VOXEL = 0.5          # occupancy grid (m)
RAYS = 48
REACH = 60.0         # ray length (m)
EYE = 1.2            # ray origin above the floor (m)
COVERED = 0.2        # sky visibility below which a cell is under cover
GRID = 1.0           # analysis cell (m)
MIN_AREA = 24.0      # smallest box footprint kept (m2)
MIN_FILL = 0.7       # covered fraction a box must reach before it is kept whole
MAX_BOXES = 12
PAD = 0.5            # metres past the covered cells (the falloff is 1 m)
ENCLOSED, OPEN = 0.45, 0.75
STOREY = 2.5         # floor height difference that separates storeys (m)
HALL, HALL_BONUS = 600.0, 0.1   # footprint (m2) above which a box gets +0.1


def hemisphere(n):
    """Cosine-distributed directions above 8 degrees elevation (y up)."""
    out = []
    golden = math.pi * (3 - math.sqrt(5))
    for i in range(n):
        r = math.sqrt((i + 0.5) / n)                 # cosine weighting
        phi = i * golden
        x, z = r * math.cos(phi), r * math.sin(phi)
        y = math.sqrt(max(0.0, 1 - r * r))
        y = max(y, math.sin(math.radians(8)))
        v = np.array([x, y, z])
        out.append(v / np.linalg.norm(v))
    return np.array(out)


def voxelize(tris, lo, shape):
    """Occupancy of triangles in a VOXEL grid starting at lo (x, y, z)."""
    occ = np.zeros(shape, bool)
    hi = lo + np.array(shape) * VOXEL
    keep = (tris.max(1) >= lo).all(1) & (tris.min(1) <= hi).all(1)
    t = tris[keep]
    while len(t):
        edge = np.linalg.norm(t - np.roll(t, 1, 1), axis=2).max(1)
        small = edge <= VOXEL * 0.8
        s = t[small]
        pts = np.concatenate([s.reshape(-1, 3), s.mean(1)])
        idx = np.floor((pts - lo) / VOXEL).astype(np.int64)
        ok = (idx >= 0).all(1) & (idx < np.array(shape)).all(1)
        occ[idx[ok, 0], idx[ok, 1], idx[ok, 2]] = True
        b = t[~small]
        mid = lambda i, j: (b[:, i] + b[:, j]) / 2
        a01, a12, a20 = mid(0, 1), mid(1, 2), mid(2, 0)
        t = np.concatenate([np.stack(q, 1) for q in (
            [b[:, 0], a01, a20], [a01, b[:, 1], a12], [a20, a12, b[:, 2]], [a01, a12, a20])]) if len(b) else b
    return occ


def march(occ, lo, origins, dirs, length):
    """Distance to the first occupied voxel along each (origin, dir) pair, or
    inf. origins (N,3), dirs (M,3) -> (N,M)."""
    steps = np.arange(VOXEL * 0.5, length, VOXEL * 0.5)
    shape = np.array(occ.shape)
    hit = np.full((len(origins), len(dirs)), np.inf)
    for j, d in enumerate(dirs):
        open_ = np.ones(len(origins), bool)
        for s in steps:
            p = origins[open_] + d * s
            idx = np.floor((p - lo) / VOXEL).astype(np.int64)
            inside = (idx >= 0).all(1) & (idx < shape).all(1)
            blocked = np.zeros(len(p), bool)
            blocked[inside] = occ[idx[inside, 0], idx[inside, 1], idx[inside, 2]]
            rows = np.nonzero(open_)[0][blocked]
            hit[rows, j] = s
            open_[rows] = False
            if not open_.any():
                break
    return hit


def occluder_triangles(overlay, project):
    """Collision triangles that block sky light: everything but glass (the
    halls' windows and glazed roofs are glass faces in the converted
    collision; X-Ray lit through them)."""
    out = []
    for o in project.get('collision_objects', []):
        path = o['lib_name'].removeprefix('resources/').split('#')[0]
        root = overlay / path
        if not (root / 'vertices').exists():
            continue
        vs = np.frombuffer(mm.chunks((root / 'vertices').read_bytes())[25], dtype='<f4').reshape(-1, 3)
        ix = np.frombuffer(mm.chunks((root / 'indices').read_bytes())[26], dtype='<u4').reshape(-1, 3)
        keep = np.ones(len(ix), bool)
        model = root.parent
        if (root / 'face_data').exists() and (model / 'settings').exists():
            fd = mm.chunks((root / 'face_data').read_bytes())
            names = fd[27][1:].split(b'\0')[:fd[27][0]]
            per_face = np.frombuffer(fd[28], '<u2')
            materials = mm.config((model / 'settings').read_bytes()).get('game_material_settings', {})
            glass = [i for i, n in enumerate(names)
                     if materials.get(n.decode(), {}).get('game_material_name') == 'glass']
            if len(per_face) == len(ix) and glass:
                keep = ~np.isin(per_face, glass)
        out.append(mm.place(vs.astype(np.float64), o)[ix[keep]])
    return np.concatenate(out)


def boxes_from_mask(mask):
    """Split a boolean mask into rectangles (r0, c0, r1, c1) that are each at
    least MIN_FILL covered, recursively along the longer side's emptiest cut."""
    out = []
    labels, n = ndimage.label(mask)
    for k, sl in enumerate(ndimage.find_objects(labels), 1):
        stack = [(sl[0].start, sl[1].start, sl[0].stop, sl[1].stop)]
        region = labels == k
        while stack:
            r0, c0, r1, c1 = stack.pop()
            sub = region[r0:r1, c0:c1]
            if not sub.any():
                continue
            rows, cols = np.nonzero(sub.any(1))[0], np.nonzero(sub.any(0))[0]
            r0, r1, c0, c1 = r0 + rows[0], r0 + rows[-1] + 1, c0 + cols[0], c0 + cols[-1] + 1
            sub = region[r0:r1, c0:c1]
            if (r1 - r0) * (c1 - c0) * GRID * GRID < MIN_AREA:
                continue
            if sub.mean() >= MIN_FILL:
                out.append((r0, c0, r1, c1))
                continue
            axis = 0 if (r1 - r0) >= (c1 - c0) else 1
            profile = sub.mean(1 - axis)
            m = len(profile)
            if m < 4:
                continue
            inner = profile[2:m - 2] if m > 6 else profile[1:m - 1]
            off = 2 if m > 6 else 1
            cut = off + int(np.argmin(inner))
            if axis == 0:
                stack += [(r0, c0, r0 + cut, c1), (r0 + cut, c0, r1, c1)]
            else:
                stack += [(r0, c0, r1, c0 + cut), (r0, c0 + cut, r1, c1)]
    return out


def propose(overlay, spec_map, cells_dir=None):
    key = mm.KEYS[spec_map['id']]
    project = mm.config((overlay / 'projects' / f"v010b_{spec_map['donor_project']}" / 'day_4' /
                         'client_project').read_bytes())
    tris = mm.collision_triangles(overlay, project)
    spawns = [r[:3] for r in spec_map['deathmatch']['respawns']]
    cache = cells_dir / f'{key}-cells.npz' if cells_dir else None
    if cache and cache.exists():
        z = np.load(cache)
        floor_h, bounds = z['floor_h'], tuple(z['bounds'])
    else:
        sx = [s[0] for s in spawns]; sz = [s[2] for s in spawns]
        bounds = (min(sx) - 150, min(sz) - 150, max(sx) + 150, max(sz) + 150)
        floor_h, _ = mm.play_area(tris, spawns, bounds)
    x0, z0, x1, z1 = bounds
    # Downsample the 0.25 m reachability to GRID: the lowest floor per cell.
    f = int(round(GRID / mm.CELL))
    h, w = floor_h.shape[0] // f * f, floor_h.shape[1] // f * f
    blocks = floor_h[:h, :w].reshape(h // f, f, w // f, f)
    with np.errstate(all='ignore'):
        floor = np.nanmin(blocks, axis=(1, 3))
        count = np.isfinite(blocks).sum((1, 3))
    floor[count < f * f // 3] = np.nan
    jj, ii = np.nonzero(np.isfinite(floor))
    if not len(jj):
        return key, [], None
    cx = x0 + (ii + 0.5) * GRID
    cz = z1 - (jj + 0.5) * GRID
    cy = floor[jj, ii]
    lo = np.array([cx.min() - REACH, cy.min() - 3, cz.min() - REACH])
    hi = np.array([cx.max() + REACH, cy.max() + 25, cz.max() + REACH])
    shape = tuple(np.ceil((hi - lo) / VOXEL).astype(int))
    occ = voxelize(occluder_triangles(overlay, project), lo, shape)
    origins = np.stack([cx, cy + EYE, cz], 1)
    dirs = hemisphere(RAYS)
    vis = np.isinf(march(occ, lo, origins, dirs, REACH)).mean(1)
    ceiling = march(occ, lo, origins, np.array([[0.0, 1.0, 0.0]]), 20.0)[:, 0] + EYE
    grid_vis = np.full(floor.shape, np.nan); grid_vis[jj, ii] = vis
    grid_ceil = np.full(floor.shape, np.nan); grid_ceil[jj, ii] = np.where(np.isfinite(ceiling), ceiling, np.nan)
    covered = np.isfinite(grid_vis) & (grid_vis < COVERED) & np.isfinite(grid_ceil)
    covered = ndimage.binary_opening(covered, np.ones((2, 2), bool))
    # Storeys: split covered cells into floor-height bands so a gallery and
    # the hall under it do not share one box.
    entries = []
    heights = np.where(covered, floor, np.nan)
    levels = np.unique(np.round(heights[np.isfinite(heights)] / STOREY))
    for level in levels:
        band = covered & (np.abs(heights / STOREY - level) <= 0.5)
        for r0, c0, r1, c1 in boxes_from_mask(band):
            cells = band[r0:r1, c0:c1]
            fl = floor[r0:r1, c0:c1][cells]
            ce = (floor + grid_ceil)[r0:r1, c0:c1][cells]
            v = grid_vis[r0:r1, c0:c1][cells]
            ylo, yhi = float(np.nanmin(fl)) - 0.5, float(np.nanpercentile(ce, 90)) + 0.2
            if not np.isfinite(yhi) or yhi - ylo < 2.6:
                yhi = ylo + 4.0
            xa, xb = x0 + c0 * GRID - PAD, x0 + c1 * GRID + PAD
            za, zb = z1 - r1 * GRID - PAD, z1 - r0 * GRID + PAD
            entries.append({
                'area': (xb - xa) * (zb - za),
                'centre': [round((xa + xb) / 2, 2), round((ylo + yhi) / 2, 2), round((za + zb) / 2, 2)],
                'half': [round((xb - xa) / 2, 2), round((yhi - ylo) / 2, 2), round((zb - za) / 2, 2)],
                # Large halls gather more bounce light than closed rooms.
                'diffuse': round(float(np.clip(ENCLOSED + (OPEN - ENCLOSED) * np.mean(v) / COVERED
                                               + (HALL_BONUS if (xb - xa) * (zb - za) >= HALL else 0),
                                               ENCLOSED, OPEN)), 2),
                'visibility': round(float(np.mean(v)), 3),
                'fill': round(float(cells.mean()), 2),
            })
    entries.sort(key=lambda e: -e['area'])
    entries = entries[:MAX_BOXES]
    out = []
    for n, e in enumerate(entries):
        out.append({
            'name': f'room_{n:02d}',
            'centre': e['centre'], 'half': e['half'], 'diffuse': e['diffuse'],
            'reason': (f"interior_probes.py: {e['area']:.0f} m2 under cover, mean sky visibility "
                       f"{e['visibility']:.2f} ({e['fill']:.0%} of the box covered)"),
        })
    return key, out, (grid_vis, covered, bounds, floor.shape)


def preview(path, key, entries, analysis):
    from PIL import Image, ImageDraw
    grid_vis, covered, bounds, shape = analysis
    x0, z0, x1, z1 = bounds
    v = np.nan_to_num(grid_vis, nan=-1)
    img = np.zeros(shape + (3,), np.uint8)
    seen = v >= 0
    img[seen] = (40 + 180 * np.clip(v[seen], 0, 1)[:, None]).astype(np.uint8)
    img[covered] = (60, 60, 150)
    im = Image.fromarray(img).resize((shape[1] * 4, shape[0] * 4), Image.NEAREST)
    d = ImageDraw.Draw(im)
    for e in entries:
        cx, _, cz = e['centre']; hx, _, hz = e['half']
        u0, u1 = (cx - hx - x0) / GRID * 4, (cx + hx - x0) / GRID * 4
        v0, v1 = (z1 - (cz + hz)) / GRID * 4, (z1 - (cz - hz)) / GRID * 4
        d.rectangle([u0, v0, u1, v1], outline=(255, 200, 0), width=2)
        d.text((u0 + 3, v0 + 2), f"{e['name']} {e['diffuse']}", fill=(255, 255, 0))
    nz = np.nonzero(seen)
    if len(nz[0]):
        pad = 20
        im = im.crop((max(0, nz[1].min() * 4 - pad), max(0, nz[0].min() * 4 - pad),
                      min(im.width, nz[1].max() * 4 + pad), min(im.height, nz[0].max() * 4 + pad)))
    im.save(path)


def write_spec(spec_path, results):
    """Replace skylight_probe.interiors per map in place: only each map's
    one-object `"skylight_probe": {...}` value is rewritten, every other
    line of the spec is kept byte for byte."""
    import re
    text = spec_path.read_text()
    spec = json.loads(text)
    pattern = re.compile(r'^( *)"skylight_probe": ', re.M)
    starts = [m for m in pattern.finditer(text)]
    maps = [m for m in spec['maps'] if 'skylight_probe' in m]
    if len(starts) != len(maps):
        raise SystemExit('cannot locate every skylight_probe in the spec')
    decoder = json.JSONDecoder()
    out, last = [], 0
    for m, start in zip(maps, starts):
        value_at = start.end()
        value, end = decoder.raw_decode(text, value_at)
        key = mm.KEYS.get(m['id'])
        if key in results:
            value = dict(value, interiors=results[key])
            indent = start.group(1)
            body = json.dumps(value, indent=2)
            # Keep coordinate triples on one line.
            body = re.sub(r'\[\s*(-?[0-9.]+),\s*(-?[0-9.]+),\s*(-?[0-9.]+)\s*\]', r'[\1, \2, \3]', body)
            body = body.replace('\n', '\n' + indent)
            out += [text[last:value_at], body]
        else:
            out += [text[last:end]]
        last = end
    out.append(text[last:])
    new = ''.join(out)
    json.loads(new)
    spec_path.write_text(new)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--overlay', type=Path, required=True)
    ap.add_argument('--spec', type=Path, required=True)
    ap.add_argument('--cells', type=Path, help='render_minimap.py output with <key>-cells.npz caches')
    ap.add_argument('--map', type=int, action='append')
    ap.add_argument('--write', action='store_true')
    ap.add_argument('--preview', type=Path)
    a = ap.parse_args()
    spec = json.loads(a.spec.read_text())
    results = {}
    for m in spec['maps']:
        if a.map and m['id'] not in a.map:
            continue
        key, entries, analysis = propose(a.overlay, m, a.cells)
        results[key] = entries
        print(json.dumps({'map': key, 'interiors': entries}), flush=True)
        if a.preview:
            a.preview.mkdir(parents=True, exist_ok=True)
            preview(a.preview / f'preview-{key}.png', key, entries, analysis)
    if a.write:
        write_spec(a.spec, results)


if __name__ == '__main__':
    main()
