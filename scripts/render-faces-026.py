#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Labelled pictures of the faces' fit, front and back, from a faces bundle.

    render-faces-026.py --bundle B --renderer target/release/examples/render-faces-026 --output DIR

Every face (the native face 0 first) bare, under a representative set of
helmets, under masks and under a helmet over a mask, each over one torso, in
the bind pose: one sheet a face and a state group (DIR/<nn>-<face>-<group>.png),
and an overview of every face under each item (DIR/all-<item>.png). The
bundle's render/scene.json names the surfaces and colour maps (written by
faces_026::build). Requires Pillow.
"""
import argparse
import json
import subprocess
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

TORSO = 'torso/scavenger_torso_1'
HELMETS = ['helmet/renesanse_helmet_2', 'helmet/scavenger_helmet_3', 'helmet/edge_helmet_4',
           'helmet/novice_026', 'helmet/eq046_edge_helmet_7', 'helmet/blackmarket_helmet_1']
MASKS = ['mask/face_069d_balaclava', 'mask/bandit_026', 'mask/scavenger_resp_1', 'mask/eq069_league_shadow_mask']
PAIRS = [('helmet/renesanse_helmet_2', 'mask/face_069d_balaclava'), ('helmet/edge_helmet_4', 'mask/army_026'),
         ('helmet/scavenger_helmet_3', 'mask/scavenger_resp_1')]
VIEWS = [('front', 0), ('back', 180)]
SIZE = 360


def font(size):
    for f in ['/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf', '/usr/share/fonts/TTF/DejaVuSans.ttf']:
        if Path(f).exists():
            return ImageFont.truetype(f, size)
    return ImageFont.load_default(size)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--renderer', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    render = a.bundle / 'render'
    scene = json.loads((render / 'scene.json').read_text())
    covers = {c['item']: c for c in scene['covers']}
    for name in [TORSO, *HELMETS, *MASKS, *[x for p in PAIRS for x in p]]:
        if name not in covers:
            raise SystemExit(f'{name} is not among the covers')

    def item_meshes(name):
        return [{'surface': s['surface'], 'diffuse': s['diffuse']} for s in covers[name]['surfaces']]

    faces = {}
    for f in scene['faces']:
        faces.setdefault((f['index'], f['face']), {}).setdefault(f['state'], []).append(f)
    native = [{'surface': scene['native']['surface'], 'diffuse': scene['native']['diffuse']}]

    def face_meshes(key, state):
        if key[0] == 0:
            return native
        states = faces[key]
        drawn = states.get(state) or states['bare']
        out = []
        for s in drawn:
            hair = 'hair' in s['surface']
            out.append({'surface': s['surface'], 'diffuse': '../' + s['diffuse'] if not s['diffuse'].startswith('/') else s['diffuse'],
                        'two_sided': hair, 'alpha_test': hair})
        return out

    columns = [('bare', 'bare', [])]
    columns += [(h.split('/')[1], 'helmet', [h]) for h in HELMETS]
    columns += [(m.split('/')[1], 'mask', [m]) for m in MASKS]
    columns += [(f"{h.split('/')[1]} + {m.split('/')[1]}", 'mask', [h, m]) for h, m in PAIRS]
    keys = [(0, 'native')] + sorted(faces)
    tiles = []
    for key in keys:
        for ci, (label, state, items) in enumerate(columns):
            meshes = face_meshes(key, state) + item_meshes(TORSO) + [m for i in items for m in item_meshes(i)]
            for vname, yaw in VIEWS:
                tiles.append({'out': f'tiles/{key[0]:02}-{ci:02}-{vname}.png', 'yaw': yaw, 'meshes': meshes})
    plan = {'size': SIZE, 'centre': [0, 1.66, 0], 'height': 0.42, 'tiles': tiles}
    (render / 'plan.json').write_text(json.dumps(plan))
    subprocess.run([str(a.renderer), '--plan', str(render / 'plan.json')], check=True)

    a.output.mkdir(parents=True, exist_ok=True)
    title, small = font(26), font(17)
    groups = [('bare-helmets', range(0, 1 + len(HELMETS))), ('masks', range(1 + len(HELMETS), len(columns)))]

    def sheet(path, heading, cells):
        # cells: [(label, [front, back])], one column each
        w, h = SIZE * len(cells), 60 + 34 + SIZE * 2
        img = Image.new('RGB', (w + 70, h), (30, 31, 34))
        d = ImageDraw.Draw(img)
        d.text((10, 14), heading, fill=(235, 235, 235), font=title)
        for r, vname in enumerate(['front', 'back']):
            d.text((6, 94 + r * SIZE + SIZE // 2), vname, fill=(200, 200, 200), font=small)
        for c, (label, files) in enumerate(cells):
            x = 70 + c * SIZE
            d.text((x + 6, 64), label[:40], fill=(220, 220, 220), font=small)
            for r, f in enumerate(files):
                img.paste(Image.open(f), (x, 94 + r * SIZE))
        img.save(path)

    for key in keys:
        for gname, cols in groups:
            cells = [(columns[c][0], [render / f'tiles/{key[0]:02}-{c:02}-{v}.png' for v, _ in VIEWS]) for c in cols]
            name = 'face 0: native (reference)' if key[0] == 0 else f'face {key[0]}: {key[1]}'
            sheet(a.output / f'{key[0]:02}-{key[1]}-{gname}.png', f'{name} - {gname} (over {TORSO})', cells)
    for c, (label, state, items) in enumerate(columns):
        cells = [(f'{k[0]} {k[1]}', [render / f'tiles/{k[0]:02}-{c:02}-{v}.png' for v, _ in VIEWS]) for k in keys]
        safe = label.replace(' + ', '+').replace('/', '_')
        sheet(a.output / f'all-{c:02}-{safe}.png', f'every face: {label} (over {TORSO})', cells)
    print(f'{len(tiles)} tiles, sheets in {a.output}')


if __name__ == '__main__':
    main()
