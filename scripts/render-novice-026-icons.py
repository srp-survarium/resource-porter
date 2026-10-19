#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the converted Novice cap and jacket into free cells of the lobby's
equipment atlas.

Requires numpy and Pillow. The atlas is the one of `--lobby-swf`, which must
be the lobby movie players have (patch 22's, 7bf0a4c5...): the cap takes
[18, 6] and the jacket [19, 6] of its sixth column; rows 1-9 are the
exoskeletons' and the balaclava's, 10-17 are reserved for the 0.69d masks and
the closed Keeper helmet. Every other pixel and every other tag stays.
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

CELLS = {'cap': (18, 6), 'jacket': (19, 6)}
# Native thumbnails of the same kind, for the comparison sheet.
NATIVE = {'cap': (12, 5), 'jacket': (10, 1)}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--textures', type=Path, required=True, help='the donor texture folder (neutral_01_novice)')
    ap.add_argument('--lobby-swf', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args(); a.output.mkdir(parents=True, exist_ok=True)

    def mesh(model, texture):
        p = a.bundle / f'overlay/models/ported/novice_026/{model}.skinned_model/render/{model.split("_")[0]}/converted_model'
        b = p.read_bytes(); n = struct.unpack_from('<I', b, 8)[0]
        raw = np.frombuffer(b[12:12 + n * 48], np.uint8).reshape(n, 48)
        floats = raw.copy().view('<f4').reshape(n, 12)
        pos = floats[:, :3].astype(float)
        faces = np.frombuffer(b, dtype='<u2', offset=24 + 48 * n, count=struct.unpack_from('<I', b, 20 + 48 * n)[0]).reshape(-1, 3)
        # A map the build cut to the UVs' window is the one the UVs index.
        cropped = a.bundle / f'cropped-textures/novice_026_{model.split("_")[0]}.tga'
        source = cropped if cropped.exists() else a.textures / f'neutral_01_novice_{texture}.tga'
        tex = np.asarray(Image.open(source).convert('RGB'), dtype=float) / 255
        return pos, floats[:, 10:12].astype(float), raw[:, 28:31][:, ::-1].astype(float) / 127.5 - 1, faces, tex, np.unique(faces)

    # (name, meshes, yaw, pitch, share of the cell): helmets fill about two
    # thirds of theirs, torsos all of it.
    specs = [('cap', [mesh('hat', 'head')], 25, 12, 0.7), ('jacket', [mesh('torso', 'torso')], 18, 4, 1.0)]
    prefix, tags = renderer.swf_tags(a.lobby_swf)
    tag_index, char_id, argb = renderer.atlas_tag(tags)
    h, w = argb.shape[:2]
    assert (w, h) == (1026, 2048), (w, h)
    atlas = argb.copy()
    comparison = Image.new('RGB', (684, 166 * len(specs)), (30, 33, 36))
    for i, (name, meshes, yaw, pitch, share) in enumerate(specs):
        row, col = CELLS[name]
        y0, x0 = (row - 1) * 91, (col - 1) * 171
        assert not atlas[y0:y0 + 83, x0:x0 + 171].any(), f'{name}: atlas cell {row},{col} is taken'
        render = renderer.render(meshes, yaw, pitch)
        if share < 1:
            small = render.resize((round(render.width * share), round(render.height * share)), Image.Resampling.LANCZOS)
            render = Image.new('RGBA', render.size, (0, 0, 0, 0))
            render.paste(small, ((render.width - small.width) // 2, (render.height - small.height) // 2))
        render.save(a.output / f'{name}-render.png')
        shown = render.resize((342, 166)); comparison.paste(shown, (342, i * 166), shown)
        nrow, ncol = NATIVE[name]
        og = Image.fromarray(argb[:, :, [1, 2, 3, 0]]).crop(((ncol - 1) * 171, (nrow - 1) * 91, ncol * 171, (nrow - 1) * 91 + 83)).resize((342, 166))
        comparison.paste(og, (0, i * 166), og)
        icon = np.asarray(render.resize((171, 83), Image.Resampling.LANCZOS)); alpha = icon[:, :, 3:4].astype(np.uint16)
        Image.fromarray(icon).save(a.output / f'r{row:02d}c{col:02d}.png')
        # The atlas holds premultiplied ARGB.
        atlas[y0:y0 + 83, x0:x0 + 171, 0] = icon[:, :, 3]
        atlas[y0:y0 + 83, x0:x0 + 171, 1:] = (icon[:, :, :3].astype(np.uint16) * alpha + 127) // 255
    comparison.save(a.output / 'comparison.png')
    tags[tag_index] = (36, struct.pack('<HBHH', char_id, 5, w, h) + zlib.compress(atlas.tobytes(), 9))
    body = prefix[8:] + b''.join(struct.pack('<HI', (kind << 6) | 63, len(payload)) + payload for kind, payload in tags)
    swf = b'CWS' + prefix[3:4] + struct.pack('<I', 8 + len(body)) + zlib.compress(body, 9)
    target = a.bundle / 'overlay/flash_movies/lobby_menu.swf'; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(swf)
    _, checktags = renderer.swf_tags(target); _, _, check = renderer.atlas_tag(checktags)
    changed = np.zeros((h, w), bool)
    for row, col in CELLS.values():
        changed[(row - 1) * 91:(row - 1) * 91 + 83, (col - 1) * 171:col * 171] = True
    assert np.array_equal(check[~changed], argb[~changed])
    assert all(x == y for j, (x, y) in enumerate(zip(tags, checktags)) if j != tag_index)
    record = json.loads((a.bundle / 'port-manifest.json').read_text())
    path = 'flash_movies/lobby_menu.swf'
    record['files'] = [f for f in record['files'] if f['path'] != path]
    record['files'].append({'path': path, 'source': 'rendered:Novice-026', 'size': len(swf), 'sha256': hashlib.sha256(swf).hexdigest()})
    record['files'].sort(key=lambda f: f['path'])
    (a.bundle / 'port-manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    print('base lobby movie', hashlib.sha256(a.lobby_swf.read_bytes()).hexdigest())
    print('icons drawn at', CELLS)


if __name__ == '__main__':
    main()
