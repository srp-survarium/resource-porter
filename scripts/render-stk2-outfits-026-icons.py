#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the STK2 outfits' items (examples/build-stk2-outfits-026) into their cells of the
lobby's equipment atlas.

Requires numpy and Pillow. `--bundle` is one outfit's bundle (`<output>/<outfit>`), whose
equipment.json names each item's model, surfaces and atlas cell; `--maps` is the builder's
`<output>/textures-work` (the maps as the bundle's materials have them, before compression).
`--lobby-swf` is the lobby movie the pack is made over (the released game's). Each piece is
posed as the stock icons of its kind are: jackets and trousers from the front, boots as a pair
from the side (the exo boots' framing, scripts/exo-gear of skhid-srp), one glove from its back
with the fingers to the left, the backpack from behind, the hood from the front. Every other
pixel and every other tag of the movie stays; the movie goes into the bundle's overlay and its
port-manifest.json.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import zlib

import numpy as np
from PIL import Image, ImageFilter

spec = importlib.util.spec_from_file_location('duty_render', Path(__file__).with_name('render-duty-026-icons.py'))
renderer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(renderer)

CELL = (171, 83)
ROW_PITCH = 91
# (yaw, pitch, share of the cell) by kind; boots and gloves are framed on their own.
VIEWS = {'torso': (12, 4), 'legs': (8, 4), 'back': (200, 10), 'helmet': (25, 5)}
# How the stock icons of each slot fill their cell (the medians of the released atlas's stock
# items: the item's height in pixels, centred at x 85.5, y 41), and the widest a piece may be
# (the stock range's top); a piece is fitted to that height unless it would be wider.
FRAMES = {'torso': (81, 169), 'legs': (81, 77), 'boots': (76, 144), 'gloves': (77, 126),
          'back': (81, 105), 'helmet': (79, 110)}
CENTRE = (85.5, 41)
# Items drawn smaller than their slot's stock fill: the low shoes (the owner, 2026-10-30: the sneakers).
SHRINK = {1866: 0.72, 1875: 0.8}
# The stock boots' icon pose (Zubr UM-4's), with a soft shadow to the left.
# The stock boot icons show no shaft opening; the camera stands level.
BOOTS = {'yaw': 75, 'pitch': 2, 'together': .065,
         'shadow_offset': (-7, 1), 'shadow_blur': 4, 'shadow_strength': .45, 'shadow_colour': (20, 16, 12)}
# Parts of a piece the icon leaves out, as UV boxes (u0, v0, u1, v1) of a surface.
HIDDEN = {}


def surface(bundle, model, name, maps, ns):
    path = bundle / f'overlay/models/{model}.skinned_model/render/{name}/converted_model'
    b = path.read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    raw = np.frombuffer(b[12:12 + 48 * n], np.uint8).reshape(n, 48)
    floats = raw.copy().view('<f4').reshape(n, 12)
    points = floats[:, :3].astype(float)
    uv = floats[:, 10:12].astype(float)
    normals = raw[:, 28:31][:, ::-1].astype(float) / 127.5 - 1
    count = struct.unpack_from('<I', b, 20 + 48 * n)[0]
    faces = np.frombuffer(b, dtype='<u2', offset=24 + 48 * n, count=count).reshape(-1, 3).astype(np.int64)
    box = HIDDEN.get((ns, model.rsplit('/', 1)[1], name))
    if box:
        centre = uv[faces].mean(1)
        inside = (centre[:, 0] > box[0]) & (centre[:, 1] > box[1]) & (centre[:, 0] < box[2]) & (centre[:, 1] < box[3])
        faces = faces[~inside]
    texture = np.asarray(Image.open(maps / f'{ns}_{name}.tga').convert('RGB'), dtype=float) / 255
    return points, uv, normals, faces, texture


def as_meshes(surfaces, keep=None):
    out = []
    for points, uv, normals, faces, texture in surfaces:
        if keep is not None:
            faces = faces[keep(points[faces])]
        out.append((points, uv, normals, faces, texture, np.unique(faces)))
    return out


# Where the stock trousers' icons are cut: each stock trousers model rendered with this camera and
# its silhouette fitted to its icon's opaque body (the 27 that fit, IoU over 0.85, of the 53 in
# the released atlas) puts the cut at 0.30-0.50 m, median 0.40 m: a hand below the knee (the
# rig's knee at 0.48-0.50 m), 0.62 of the waist-to-floor height (quartiles 0.59-0.63). Ours are
# cut there, then fitted to the stock height.
LEGS_KEEP = 0.62


# Trousers are open at the waist: from a camera a little above it the inside of the back
# shows over the front band, unlit, as a black band where the stock icons show the belt line.
WAIST_FROM = 0.95


def waist_inside_hidden(meshes, yaw, pitch):
    """The faces at the waist (above WAIST_FROM) that turn away from the camera left out."""
    y, p = np.radians([yaw, pitch])
    toward = np.array([np.sin(y) * np.cos(p), np.sin(p), np.cos(y) * np.cos(p)])
    out = []
    for points, uv, normals, faces, texture, used in meshes:
        inside = (points[faces][:, :, 1].mean(1) > WAIST_FROM) & (normals[faces].mean(1) @ toward < 0)
        out.append((points, uv, normals, faces[~inside], texture, used))
    return out


def cut_legs(picture, surfaces):
    """Trousers cut below the knee, as the stock icons show them: rows below LEGS_KEEP of the
    waist-to-floor height left out (the picture's rows follow the model's heights; the camera
    is nearly level)."""
    ys_model = np.concatenate([s[0][np.unique(s[3])][:, 1] for s in surfaces])
    top, bottom = ys_model.max(), ys_model.min()
    cut = top * (1 - LEGS_KEEP)
    if cut <= bottom:
        return picture
    alpha = np.asarray(picture)[:, :, 3]
    rows = np.nonzero(alpha.any(1))[0]
    row = rows.min() + int(round((rows.max() - rows.min()) * (top - cut) / (top - bottom)))
    out = np.asarray(picture).copy()
    out[row:] = 0
    return Image.fromarray(out)


def framed(picture, kind, shrink=1.0):
    """`picture` cut to the piece and fitted into a cell as the stock icons of `kind` are."""
    alpha = np.asarray(picture)[:, :, 3]
    ys, xs = np.nonzero(alpha)
    crop = picture.crop((xs.min(), ys.min(), xs.max() + 1, ys.max() + 1))
    height, widest = FRAMES[kind]
    scale = min(height / crop.height, widest / crop.width) * shrink
    size = (max(1, round(crop.width * scale)), max(1, round(crop.height * scale)))
    out = Image.new('RGBA', CELL, (0, 0, 0, 0))
    out.alpha_composite(crop.resize(size, Image.Resampling.LANCZOS),
                        (round(CENTRE[0] - size[0] / 2), round(CENTRE[1] - size[1] / 2)))
    return out


# Every stock item icon sits on a faint black halo, even all round (no offset), the same in every
# slot. Measured on the released atlas (248 armour icons), outside the opaque body its alpha
# falls from 60 at 2 px to 34 at 4, 19 at 6, 10 at 8, 4 at 10 and 1 at 12, all black: the
# item's own alpha blurred (gaussian, sigma 5 px) at 0.6 of full strength gives 61, 38, 21, 10,
# 4, 2 on the stock bodies; ours at 0.55, a shade fainter. The atlas cells' top and bottom rows
# stay clear, the halo cut there, as on stock.
HALO_SIGMA = 5.0
HALO_PEAK = 0.55


def blurred(values, sigma):
    """`values` (2-D) blurred by a gaussian of `sigma` px, the outside taken as 0."""
    radius = int(4 * sigma + 0.5)
    x = np.arange(-radius, radius + 1)
    kernel = np.exp(-x * x / (2 * sigma * sigma))
    kernel /= kernel.sum()
    padded = np.pad(values, radius)
    rows = np.apply_along_axis(lambda r: np.convolve(r, kernel, mode='valid'), 1, padded)
    return np.apply_along_axis(lambda c: np.convolve(c, kernel, mode='valid'), 0, rows)


def haloed(icon):
    """`icon` over its halo: black, its alpha the item's own blurred, under the item."""
    rgba = np.asarray(icon).astype(float) / 255
    alpha = rgba[:, :, 3]
    halo = blurred(alpha, HALO_SIGMA) * HALO_PEAK
    out_alpha = alpha + halo * (1 - alpha)
    rgb = rgba[:, :, :3] * (alpha / np.maximum(out_alpha, 1e-6))[:, :, None]
    out = np.dstack([rgb, out_alpha])
    out[[0, -1]] = 0
    return Image.fromarray((out * 255).round().astype(np.uint8))


# Boots drawn as first rendered (v1: the camera a little above, nothing left out or
# recoloured): the owner's pick for the sneakers (2026-10-31), only smaller.
AS_FIRST = {1866: 6}


def boots_icon(surfaces, shrink=1.0, as_first=None):
    if as_first is not None:
        kept = list(surfaces)
    else:
        # Each boot whole up to its own top. Of the faces whose texels are black (the shaft top
        # STK2's trousers covered): those facing into the boot (its lining) are left out, those
        # of its outer surface keep their place and take the leather around the black.
        kept = []
        for points, uv, normals, faces, texture in surfaces:
            h, w = texture.shape[:2]
            lum = texture[np.clip((uv[faces][:, :, 1] * h).astype(int), 0, h - 1),
                          np.clip((uv[faces][:, :, 0] * w).astype(int), 0, w - 1)].mean(2).mean(1)
            tri = points[faces]
            middle = tri.mean(1)
            normal = np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0])
            inward = np.zeros(len(faces), bool)
            for side in (middle[:, 0] > 0, middle[:, 0] <= 0):
                if not side.any():
                    continue
                axis = points[np.unique(faces[side])].mean(0)
                radial = middle[side] - axis
                radial[:, 1] = 0
                inward[side] = (normal[side] * radial).sum(1) < 0
            black = (lum < GLOVE_BLACK) & (middle[:, 1] > 0.08)
            outer_black = black & ~inward
            texture = neighbour_coloured(points, uv, faces, texture, lum, outer_black)
            kept.append((points, uv, normals, faces[~(black & inward)], texture))
    surfaces = kept
    moved = []
    for points, uv, normals, faces, texture in surfaces:
        points = points.copy()
        points[:, 0] -= np.sign(points[:, 0]) * BOOTS['together']
        moved.append((points, uv, normals, faces, texture))
    pitch = BOOTS['pitch'] if as_first is None else as_first
    picture = renderer.render(as_meshes(moved), BOOTS['yaw'], pitch, size=(CELL[0] * 8, CELL[1] * 8))
    body = framed(picture, 'boots', shrink)
    shade = Image.new('L', CELL, 0)
    shade.paste(body.getchannel('A'), BOOTS['shadow_offset'])
    shade = shade.filter(ImageFilter.GaussianBlur(BOOTS['shadow_blur']))
    shade = shade.point(lambda value: int(value * BOOTS['shadow_strength']))
    shadow = Image.new('RGBA', CELL, BOOTS['shadow_colour'] + (0,))
    shadow.putalpha(shade)
    out = Image.new('RGBA', CELL, (0, 0, 0, 0))
    out.alpha_composite(shadow)
    out.alpha_composite(body)
    return out


# The stock glove icons: one hand, the back of it square to the camera (the knuckle line in
# the picture's plane), its long axis 25-41 degrees above the horizontal (median 32) with the
# fingers down to the left.
GLOVE_AXIS = 32.0
# Texels darker than this at a glove's wrist end are its cuff's black band and inside, which
# STK2 hid under the sleeve: the icon leaves those faces out, so the glove ends cleanly.
GLOVE_BLACK = 0.06


def camera_for(points, faces):
    """(yaw, pitch, roll) of the renderer's camera that looks at the back of one hand square
    on, the fingers on the picture at GLOVE_AXIS below the horizontal to the left."""
    used = points[np.unique(faces)]
    centre = used.mean(0)
    _, _, axes = np.linalg.svd(used - centre)
    along, across, normal = axes
    if along[0] < 0:  # toward the fingertips (the +x hand)
        along = -along
    if normal[1] < 0:  # the back of the hand: up, the palms face down in the bind pose
        normal = -normal
    yaw = np.degrees(np.arctan2(normal[0], normal[2]))
    pitch = np.degrees(np.arcsin(np.clip(normal[1], -1, 1)))
    # The renderer's picture axes for that yaw and pitch, before roll.
    y, p = np.radians([yaw, pitch])
    toward = np.array([np.sin(y) * np.cos(p), np.sin(p), np.cos(y) * np.cos(p)])
    right = np.cross([0, 1, 0], toward)
    right /= np.linalg.norm(right)
    up = np.cross(toward, right)
    shown = np.degrees(np.arctan2(along @ up, along @ right))
    # Rolling by r turns the picture's axes, so the fingers show at shown - r.
    roll = shown - (180 + GLOVE_AXIS)
    return yaw, pitch, roll


def largest_piece(points, faces):
    """The faces of the largest connected piece (vertices welded by position): what a cut
    leaves of a glove, without the stray bits of its cuff."""
    key = {}
    weld = np.array([key.setdefault(tuple(np.round(p, 4)), i) for i, p in enumerate(points)])
    parent = list(range(len(points)))

    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    for t in faces:
        a, b, c = (root(weld[v]) for v in t)
        parent[b] = a
        parent[root(c)] = a
    roots = np.array([root(weld[t[0]]) for t in faces])
    values, counts = np.unique(roots, return_counts=True)
    return faces[roots == values[counts.argmax()]]


def silhouette_axis(picture):
    """The long axis of a picture's opaque part, in degrees above the horizontal (0-180)."""
    ys, xs = np.nonzero(np.asarray(picture)[:, :, 3] > 64)
    d = np.stack([xs - xs.mean(), ys - ys.mean()], 1)
    a = np.linalg.eigh(d.T @ d)[1][:, 1]
    return np.degrees(np.arctan2(-a[1], a[0])) % 180


def neighbour_coloured(points, uv, faces, texture, lum, paint):
    """`texture` with each face of `paint` (its texels black) given the colour of the faces
    around it on the mesh that are not black (within 2.5 cm, else 5 cm), over the texels its
    UV triangle covers: the band reads as the cloth or leather it belongs to."""
    h, w = texture.shape[:2]
    out = texture.copy()
    middle = points[faces].mean(1)
    corners = texture[np.clip((uv[faces][:, :, 1] * h).astype(int), 0, h - 1),
                      np.clip((uv[faces][:, :, 0] * w).astype(int), 0, w - 1)].mean(1)
    known = lum >= GLOVE_BLACK
    for f in np.nonzero(paint)[0]:
        d = np.linalg.norm(middle - middle[f], axis=1)
        for reach in (0.025, 0.05, 0.1):
            near = known & (d < reach)
            if near.any():
                break
        if not near.any():
            continue
        colour = np.median(corners[near], 0)
        box = uv[faces[f]]
        x0, y0 = np.floor(box.min(0) * [w, h]).astype(int) - 1
        x1, y1 = np.ceil(box.max(0) * [w, h]).astype(int) + 1
        region = out[max(y0, 0):min(y1, h), max(x0, 0):min(x1, w)]
        dark = region.mean(2) < GLOVE_BLACK
        region[dark] = colour
    return out


# A glove face whose three corners are all this dark lies wholly on the cuff's black end
# band or inside the cuff (STK2 hid both under the sleeve).
CUFF_BLACK = 0.03


def trimmed_cuff(points, faces, reach=0.03):
    """`faces` without the flaps left standing at the cuff's end: within `reach` of the end
    (along the hand's long axis), a face with two or more open edges (a point or a lip on the
    outline) goes, repeatedly, until the edge is a smooth run of faces."""
    used = points[np.unique(faces)]
    centre = used.mean(0)
    along = np.linalg.svd(used - centre)[2][0]
    if along[0] < 0:
        along = -along
    depth = (points - centre) @ along
    end = depth[np.unique(faces)].min()
    key = {}
    weld = np.array([key.setdefault(tuple(np.round(p, 4)), i) for i, p in enumerate(points)])
    while True:
        count = {}
        for t in faces:
            for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
                e = (min(weld[a], weld[b]), max(weld[a], weld[b]))
                count[e] = count.get(e, 0) + 1
        open_edges = np.array([sum(count[(min(weld[a], weld[b]), max(weld[a], weld[b]))] == 1
                                   for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0]))) for t in faces])
        near = depth[faces].mean(1) < end + reach
        flap = near & (open_edges >= 2)
        if not flap.any():
            break
        faces = faces[~flap]
    count = {}
    for t in faces:
        for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
            e = (min(weld[a], weld[b]), max(weld[a], weld[b]))
            count[e] = count.get(e, 0) + 1
    # The cuff's edge itself smoothed (each open-edge vertex near the end drawn halfway to
    # the middle of its two neighbours on the edge, three times): no corner stands out of
    # the line. The points are this icon's copy.
    rim = {}
    for t in faces:
        for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
            if count.get((min(weld[a], weld[b]), max(weld[a], weld[b]))) == 1 and \
                    depth[a] < end + reach and depth[b] < end + reach:
                rim.setdefault(weld[a], set()).add(weld[b])
                rim.setdefault(weld[b], set()).add(weld[a])
    for _ in range(3):
        moved = {}
        for v, nb in rim.items():
            if len(nb) == 2:
                a, b = sorted(nb)
                moved[v] = 0.5 * points[v] + 0.25 * (points[a] + points[b])
        for v, q in moved.items():
            points[weld == v] = q
    return faces


def glove_icon(surfaces):
    """One glove up to its cuff. The faces wholly on the cuff's black end band or inside it
    (all three corners black) are left out: the band is whole rows of the mesh, so the glove
    ends along an edge loop, smooth, where its cloth ends. Faces part black at that edge take
    the colour of the glove around them on the mesh."""
    one = []
    for points, uv, normals, faces, texture in surfaces:
        faces = faces[points[faces][:, :, 0].mean(1) > 0]
        h, w = texture.shape[:2]
        corner = texture[np.clip((uv[faces][:, :, 1] * h).astype(int), 0, h - 1),
                         np.clip((uv[faces][:, :, 0] * w).astype(int), 0, w - 1)].mean(2)
        points = points.copy()
        faces = trimmed_cuff(points, largest_piece(points, faces[corner.max(1) >= CUFF_BLACK]))
        lum = texture[np.clip((uv[faces][:, :, 1] * h).astype(int), 0, h - 1),
                      np.clip((uv[faces][:, :, 0] * w).astype(int), 0, w - 1)].mean(2).mean(1)
        texture = neighbour_coloured(points, uv, faces, texture, lum, lum < GLOVE_BLACK)
        one.append((points, uv, normals, faces, texture))
    yaw, pitch, roll = camera_for(one[0][0], np.vstack([f for _, _, _, f, _ in one]))
    # The picture's long axis follows the outline (fingers, thumb), not the mesh's: settle it
    # on the stock angle (rolling one degree more turns it one degree less).
    for _ in range(3):
        picture = renderer.render(as_meshes(one), yaw, pitch, roll=roll)
        error = silhouette_axis(picture) - GLOVE_AXIS
        error = (error + 90) % 180 - 90
        if abs(error) < 0.5:
            break
        roll += error
    return framed(picture, 'gloves')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--maps', type=Path, required=True)
    ap.add_argument('--lobby-swf', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    items = json.loads((a.bundle / 'equipment.json').read_text())
    prefix, tags = renderer.swf_tags(a.lobby_swf)
    tag_index, char_id, argb = renderer.atlas_tag(tags)
    h, w = argb.shape[:2]
    atlas = argb.copy()
    cells = []
    for item in items:
        row, col = item['icon']
        y0, x0 = (row - 1) * ROW_PITCH, (col - 1) * CELL[0]
        assert x0 + CELL[0] <= w and y0 + CELL[1] <= h, f"{item['name']}: cell {row},{col} outside the {w}x{h} atlas"
        assert not atlas[y0:y0 + CELL[1], x0:x0 + CELL[0]].any(), f"{item['name']}: atlas cell {row},{col} is taken"
        ns = item['model'].split('/')[1]
        surfaces = [surface(a.bundle, item['model'], name, a.maps, ns) for name in item['surfaces']]
        kind = item['category']
        if kind == 'boots':
            icon = boots_icon(surfaces, SHRINK.get(item['dict_id'], 1.0), AS_FIRST.get(item['dict_id']))
        elif kind == 'gloves':
            icon = glove_icon(surfaces)
        else:
            yaw, pitch = VIEWS[kind]
            meshes = as_meshes(surfaces)
            if kind == 'legs':
                meshes = waist_inside_hidden(meshes, yaw, pitch)
            picture = renderer.render(meshes, yaw, pitch, size=(CELL[0] * 6, CELL[1] * 6))
            if kind == 'legs':
                picture = cut_legs(picture, surfaces)
            icon = framed(picture, kind)
        icon = haloed(icon)
        icon.save(a.output / f"{item['dict_id']}.png")
        rgba = np.asarray(icon)
        alpha = rgba[:, :, 3:4].astype(np.uint16)
        # The atlas holds premultiplied ARGB.
        atlas[y0:y0 + CELL[1], x0:x0 + CELL[0], 0] = rgba[:, :, 3]
        atlas[y0:y0 + CELL[1], x0:x0 + CELL[0], 1:] = (rgba[:, :, :3].astype(np.uint16) * alpha + 127) // 255
        cells.append((row, col))
        print(f"{item['dict_id']} {item['name']}: [{row}, {col}]")
    sheet = Image.new('RGBA', (CELL[0] * 2, CELL[1] * ((len(items) + 1) // 2)), (40, 44, 50, 255))
    for i, item in enumerate(items):
        sheet.alpha_composite(Image.open(a.output / f"{item['dict_id']}.png"), ((i % 2) * CELL[0], (i // 2) * CELL[1]))
    sheet.resize((sheet.width * 2, sheet.height * 2)).save(a.output / 'icons.png')
    tags[tag_index] = (36, struct.pack('<HBHH', char_id, 5, w, h) + zlib.compress(atlas.tobytes(), 9))
    body = prefix[8:] + b''.join(struct.pack('<HI', (kind << 6) | 63, len(payload)) + payload for kind, payload in tags)
    swf = b'CWS' + prefix[3:4] + struct.pack('<I', 8 + len(body)) + zlib.compress(body, 9)
    target = a.bundle / 'overlay/flash_movies/lobby_menu.swf'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(swf)
    _, checktags = renderer.swf_tags(target)
    _, _, check = renderer.atlas_tag(checktags)
    changed = np.zeros((h, w), bool)
    for row, col in cells:
        changed[(row - 1) * ROW_PITCH:(row - 1) * ROW_PITCH + CELL[1], (col - 1) * CELL[0]:col * CELL[0]] = True
    assert np.array_equal(check[~changed], argb[~changed])
    assert all(x == y for j, (x, y) in enumerate(zip(tags, checktags)) if j != tag_index)
    record = json.loads((a.bundle / 'port-manifest.json').read_text())
    path = 'flash_movies/lobby_menu.swf'
    record['files'] = [f for f in record['files'] if f['path'] != path]
    record['files'].append({'path': path, 'source': 'rendered:STK2-outfits-026', 'size': len(swf),
                            'sha256': hashlib.sha256(swf).hexdigest()})
    record['files'].sort(key=lambda f: f['path'])
    (a.bundle / 'port-manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    print('base lobby movie', hashlib.sha256(a.lobby_swf.read_bytes()).hexdigest())


if __name__ == '__main__':
    main()
