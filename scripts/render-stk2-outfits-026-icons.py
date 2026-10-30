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
VIEWS = {'torso': (12, 4, 1.0), 'legs': (8, 4, 0.98), 'back': (200, 10, 0.85), 'helmet': (25, 5, 0.72)}
# The stock boots' icon framing (Zubr UM-4's): the pair 77 pixels high centred at x 85, with a
# soft shadow to the left.
BOOTS = {'yaw': 75, 'pitch': 6, 'together': .065, 'height': 77, 'max_width': 140, 'centre_x': 85,
         'shadow_offset': (-7, 1), 'shadow_blur': 4, 'shadow_strength': .45, 'shadow_colour': (20, 16, 12)}
# Parts of a piece the icon leaves out, as UV boxes (u0, v0, u1, v1) of a surface: the
# bomber's bare forearms (the hands' corner of its torso map), which no stock jacket icon shows.
HIDDEN = {('stk2_bandit_026', 'torso', 'torso'): (0.74, 0.55, 1.0, 0.84)}


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


def shared(picture, share):
    if share >= 1:
        return picture
    small = picture.resize((round(picture.width * share), round(picture.height * share)), Image.Resampling.LANCZOS)
    out = Image.new('RGBA', picture.size, (0, 0, 0, 0))
    out.paste(small, ((picture.width - small.width) // 2, (picture.height - small.height) // 2))
    return out


def boots_icon(surfaces):
    moved = []
    for points, uv, normals, faces, texture in surfaces:
        points = points.copy()
        points[:, 0] -= np.sign(points[:, 0]) * BOOTS['together']
        moved.append((points, uv, normals, faces, texture))
    picture = renderer.render(as_meshes(moved), BOOTS['yaw'], BOOTS['pitch'], size=(CELL[0] * 8, CELL[1] * 8))
    alpha = np.asarray(picture)[:, :, 3]
    ys, xs = np.nonzero(alpha)
    crop = picture.crop((xs.min(), ys.min(), xs.max() + 1, ys.max() + 1))
    # Low shoes are long for their height: the pair keeps inside the stock pair's width.
    scale = min(BOOTS['height'] / crop.height, BOOTS['max_width'] / crop.width)
    height, width = max(1, round(crop.height * scale)), max(1, round(crop.width * scale))
    body = Image.new('RGBA', CELL, (0, 0, 0, 0))
    body.alpha_composite(crop.resize((width, height), Image.Resampling.LANCZOS),
                         (max(0, BOOTS['centre_x'] - width // 2), (CELL[1] - height) // 2))
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


GLOVE_ROLL = 50


def glove_icon(surfaces):
    # One hand (the mesh's +x one), its back seen from above, the fingers to the left and a
    # little down, as the stock glove icons show them.
    picture = renderer.render(as_meshes(surfaces, lambda tri: tri[:, :, 0].mean(1) > 0), 90, 80, roll=GLOVE_ROLL)
    return shared(picture, 0.9).resize(CELL, Image.Resampling.LANCZOS)


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
            icon = boots_icon(surfaces)
        elif kind == 'gloves':
            icon = glove_icon(surfaces)
        else:
            yaw, pitch, share = VIEWS[kind]
            icon = shared(renderer.render(as_meshes(surfaces), yaw, pitch), share).resize(CELL, Image.Resampling.LANCZOS)
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
