#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render ported masks into free cells of the lobby's equipment atlas.

    render-mask-026-icons.py --bundle B --lobby-swf S --output O \\
        --item SURFACE TEXTURE ROW COL [--item ...] [--yaw 15] [--pitch 0]

SURFACE is a converted_model under the bundle's overlay (an archive path),
TEXTURE the image its UVs index (a TGA, PNG or DXT DDS; a TGA cut to an
atlas rectangle with `--crop U0,V0,U1,V1`, as the build cut the maps). The
atlas is the one of `--lobby-swf`, which must be the lobby movie players
have: each cell must be empty there; every other pixel and every other tag
stays. Writes the bundle's overlay/flash_movies/lobby_menu.swf (for
`vostok-port pack from-bundle`, which keeps only the new cells), each icon
as rRRcCC.png and a comparison sheet beside the native Corsair mask [19, 5].

Requires numpy and Pillow.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import zlib
import numpy as np
from PIL import Image

spec = importlib.util.spec_from_file_location('duty_render', Path(__file__).with_name('render-duty-026-icons.py'))
renderer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(renderer)

NATIVE = (19, 5)
CELL = (171, 91, 83)  # width, row pitch, height


def load_mesh(bundle, surface, texture, crop):
    b = (bundle / 'overlay' / surface).read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    raw = np.frombuffer(b[12:12 + n * 48], np.uint8).reshape(n, 48)
    floats = raw.copy().view('<f4').reshape(n, 12)
    count = struct.unpack_from('<I', b, 20 + 48 * n)[0]
    faces = np.frombuffer(b, dtype='<u2', offset=24 + 48 * n, count=count).reshape(-1, 3)
    img = Image.open(texture).convert('RGB')
    if crop:
        u0, v0, u1, v1 = crop
        w, h = img.size
        img = img.crop((round(u0 * w), round(v0 * h), round(u1 * w), round(v1 * h)))
    tex = np.asarray(img, dtype=float) / 255
    return floats[:, :3].astype(float), floats[:, 10:12].astype(float), raw[:, 28:31][:, ::-1].astype(float) / 127.5 - 1, faces, tex, np.unique(faces)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--lobby-swf', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--item', nargs=4, action='append', required=True, metavar=('SURFACE', 'TEXTURE', 'ROW', 'COL'))
    ap.add_argument('--crop', action='append', default=[], help='U0,V0,U1,V1 for the item of the same position (or "-")')
    ap.add_argument('--yaw', type=float, default=15)
    ap.add_argument('--pitch', type=float, default=0)
    a = ap.parse_args(); a.output.mkdir(parents=True, exist_ok=True)
    crops = [None if c == '-' else [float(x) for x in c.split(',')] for c in a.crop]
    crops += [None] * (len(a.item) - len(crops))

    prefix, tags = renderer.swf_tags(a.lobby_swf)
    tag_index, char_id, argb = renderer.atlas_tag(tags)
    h, w = argb.shape[:2]
    assert (w, h) == (1026, 2048), (w, h)
    cw, pitch, ch = CELL
    atlas = argb.copy()
    cells = []
    sheet = Image.new('RGB', (684, 166 * len(a.item)), (30, 33, 36))
    nrow, ncol = NATIVE
    native = Image.fromarray(argb[:, :, [1, 2, 3, 0]]).crop(((ncol - 1) * cw, (nrow - 1) * pitch, ncol * cw, (nrow - 1) * pitch + ch)).resize((342, 166))
    for i, ((surface, texture, row, col), crop) in enumerate(zip(a.item, crops)):
        row, col = int(row), int(col)
        y0, x0 = (row - 1) * pitch, (col - 1) * cw
        assert y0 + ch <= h and x0 + cw <= w, f'cell {row},{col} is outside the atlas'
        assert not atlas[y0:y0 + ch, x0:x0 + cw].any(), f'atlas cell {row},{col} is taken'
        mesh = load_mesh(a.bundle, surface, texture, crop)
        render = renderer.render([mesh], a.yaw, a.pitch)
        name = Path(surface).parent.name
        render.save(a.output / f'{name}-render.png')
        icon = np.asarray(render.resize((cw, ch), Image.Resampling.LANCZOS))
        Image.fromarray(icon).save(a.output / f'r{row:02d}c{col:02d}.png')
        alpha = icon[:, :, 3:4].astype(np.uint16)
        # The atlas holds premultiplied ARGB.
        atlas[y0:y0 + ch, x0:x0 + cw, 0] = icon[:, :, 3]
        atlas[y0:y0 + ch, x0:x0 + cw, 1:] = (icon[:, :, :3].astype(np.uint16) * alpha + 127) // 255
        sheet.paste(native, (0, i * 166), native)
        shown = render.resize((342, 166)); sheet.paste(shown, (342, i * 166), shown)
        cells.append((row, col))
    sheet.save(a.output / 'comparison.png')
    tags[tag_index] = (36, struct.pack('<HBHH', char_id, 5, w, h) + zlib.compress(atlas.tobytes(), 9))
    body = prefix[8:] + b''.join(struct.pack('<HI', (kind << 6) | 63, len(payload)) + payload for kind, payload in tags)
    swf = b'CWS' + prefix[3:4] + struct.pack('<I', 8 + len(body)) + zlib.compress(body, 9)
    target = a.bundle / 'overlay/flash_movies/lobby_menu.swf'; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(swf)
    _, checktags = renderer.swf_tags(target); _, _, check = renderer.atlas_tag(checktags)
    changed = np.zeros((h, w), bool)
    for row, col in cells:
        changed[(row - 1) * pitch:(row - 1) * pitch + ch, (col - 1) * cw:col * cw] = True
    assert np.array_equal(check[~changed], argb[~changed])
    assert all(x == y for j, (x, y) in enumerate(zip(tags, checktags)) if j != tag_index)
    record = json.loads((a.bundle / 'port-manifest.json').read_text())
    path = 'flash_movies/lobby_menu.swf'
    record['files'] = [f for f in record['files'] if f['path'] != path]
    record['files'].append({'path': path, 'source': 'rendered:mask-icons', 'size': len(swf), 'sha256': hashlib.sha256(swf).hexdigest()})
    record['files'].sort(key=lambda f: f['path'])
    (a.bundle / 'port-manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    print('base lobby movie', hashlib.sha256(a.lobby_swf.read_bytes()).hexdigest())
    print('icons drawn at', cells)


if __name__ == '__main__':
    main()
