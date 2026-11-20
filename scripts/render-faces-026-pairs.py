#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Labelled before | after pictures of the faces, from an earlier faces pack and a faces bundle.

    render-faces-026-pairs.py --before PACK --before-name faces@2 --bundle B --after-name faces@3 \
        --renderer target/release/examples/render-faces-026 --output DIR [--icons-069d DIR]

Every face bare, under helmets, under masks and under a helmet over a mask, over
one torso, in the bind pose, from the front, the side and the back: each cell a
pair from one camera under one flat light (the colour map, no shading), the
earlier pack's face on the left and the bundle's on the right. On the right an
item is the model made for that face where the bundle has one (scene.json
`for_faces`: the game draws those only once its cook reads `data_head_<face>`);
on the left it is the game's. --icons-069d: 0.69d0's own pictures of its faces
(`flash_movies/textures/ui_icons/heads/head_<n>.png`), put beside each title.
Requires Pillow.
"""
import argparse
import json
import subprocess
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

TORSO = 'torso/scavenger_torso_1'
HELMETS = ['helmet/renesanse_helmet_2', 'helmet/scavenger_helmet_3', 'helmet/edge_helmet_4', 'helmet/novice_026']
MASKS = ['mask/face_069d_balaclava', 'mask/bandit_026', 'mask/scavenger_resp_1', 'mask/eq069_league_shadow_mask']
PAIRS = [('helmet/renesanse_helmet_2', 'mask/face_069d_balaclava'), ('helmet/scavenger_helmet_3', 'mask/scavenger_resp_1')]
VIEWS = [('front', 0), ('side', 90), ('back', 180)]
SIZE = 300
KEYS = {'bare': 'data', 'helmet': 'data_helmet', 'mask': 'data_mask'}


def font(size):
    for f in ['/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf', '/usr/share/fonts/TTF/DejaVuSans.ttf']:
        if Path(f).exists():
            return ImageFont.truetype(f, size)
    return ImageFont.load_default(size)


def pack_faces(pack):
    """index -> state -> meshes, from a faces pack's heads and files."""
    manifest = json.loads((pack / 'pack.json').read_text())
    heads = next(op for op in manifest['operations'] if op['op'] == 'config_set'
                 and op['path'] == 'gameplay/bodyparts/default')['values']['body_parts.heads']
    files = pack / 'files'
    out = {}
    for index, head in enumerate(heads):
        if index == 0:
            continue
        head = head['table']
        states = {}
        for state, key in KEYS.items():
            entry = head.get(key, head['data'])['table']
            parts = [entry] + [e['table'] for e in entry.get('additional_models', [])]
            meshes = []
            for p in parts:
                face = Path(p['base_model']).name
                for suffix in ('_helmet', '_mask'):
                    face = face.removesuffix(suffix)
                hair = 'hair' in p['part_name']
                tex = face + ('_hair' if hair else '_scalp' if p['part_name'] == 'scalp_s' else '')
                meshes.append({
                    'surface': str(files / 'models' / f"{p['base_model']}.skinned_model/render/{p['part_name']}/converted_model"),
                    'diffuse': str(files / 'textures/ported/faces' / f'{tex}.dds'),
                    'two_sided': hair, 'alpha_test': hair})
            states[state] = meshes
        out[index] = states
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--before', type=Path, required=True)
    ap.add_argument('--before-name', required=True)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--after-name', required=True)
    ap.add_argument('--renderer', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--icons-069d', type=Path)
    a = ap.parse_args()
    render = a.bundle.resolve() / 'render'
    scene = json.loads((render / 'scene.json').read_text())
    covers = {c['item']: c for c in scene['covers']}

    def item_meshes(name, face=None):
        made = {f['part']: f['surface'] for f in scene.get('for_faces', []) if f['item'] == name and f['face'] == face}
        return [{'surface': str(render / made.get(k, s['surface'])), 'diffuse': s['diffuse'] and str(render / s['diffuse'])}
                for k, s in enumerate(covers[name]['surfaces'])]

    after = {}
    names = {}
    for f in scene['faces']:
        hair = 'hair' in f['surface']
        names[f['index']] = f['face']
        after.setdefault(f['index'], {}).setdefault(f['state'], []).append({
            'surface': str(render / f['surface']), 'diffuse': str(a.bundle.resolve() / f['diffuse']),
            'two_sided': hair, 'alpha_test': hair})
    before = pack_faces(a.before.resolve())
    made_for = {(f['face'], f['item']) for f in scene.get('for_faces', [])}

    columns = [('bare', 'bare', [])]
    columns += [(h.split('/')[1], 'helmet', [h]) for h in HELMETS]
    columns += [(m.split('/')[1], 'mask', [m]) for m in MASKS]
    columns += [(f"{h.split('/')[1]} + {m.split('/')[1]}", 'mask', [h, m]) for h, m in PAIRS]
    tiles = []
    for index in sorted(after):
        for ci, (_, state, items) in enumerate(columns):
            sides = {
                'before': before[index][state] + item_meshes(TORSO) + [m for i in items for m in item_meshes(i)],
                'after': (after[index].get(state) or after[index]['bare']) + item_meshes(TORSO)
                + [m for i in items for m in item_meshes(i, names[index])],
            }
            for side, meshes in sides.items():
                for vname, yaw in VIEWS:
                    tiles.append({'out': f'pairs/{index:02}-{ci:02}-{side}-{vname}.png', 'yaw': yaw, 'meshes': meshes})
    plan = {'size': SIZE, 'centre': [0, 1.66, 0], 'height': 0.42, 'tiles': tiles}
    (render / 'pairs-plan.json').write_text(json.dumps(plan))
    subprocess.run([str(a.renderer), '--plan', str(render / 'pairs-plan.json')], check=True)

    a.output.mkdir(parents=True, exist_ok=True)
    title, small, tiny = font(24), font(15), font(13)
    groups = [('bare-helmets', range(0, 1 + len(HELMETS))), ('masks', range(1 + len(HELMETS), len(columns)))]
    for index in sorted(after):
        icon = a.icons_069d and a.icons_069d / f'head_{index}.png'
        for gname, cols in groups:
            cols = list(cols)
            left, top, gap = 60, 150, 14
            w = left + len(cols) * (2 * SIZE + gap)
            img = Image.new('RGB', (w, top + len(VIEWS) * SIZE + 8), (30, 31, 34))
            d = ImageDraw.Draw(img)
            x = 12
            if icon and icon.exists():
                pic = Image.open(icon).convert('RGBA')
                pic = pic.resize((int(pic.width * 96 / pic.height), 96))
                img.paste(pic, (12, 8), pic)
                d.text((12, 106), "0.69d0's own picture", fill=(170, 170, 170), font=tiny)
                x = 12 + max(pic.width, 150) + 16
            d.text((x, 14), f'face {index}: {names[index]} - {gname.replace("-", ", ")} (over {TORSO})', fill=(235, 235, 235), font=title)
            d.text((x, 50), f'each pair: {a.before_name} on the left | {a.after_name} on the right; one camera, flat light (colour map only, bind pose)',
                   fill=(190, 190, 190), font=small)
            d.text((x, 72), '* the item is the model made for this face (drawn by the game once its cook reads data_head_<face>)',
                   fill=(190, 190, 190), font=small)
            for r, (vname, _) in enumerate(VIEWS):
                d.text((6, top + r * SIZE + SIZE // 2), vname, fill=(200, 200, 200), font=small)
            for c, ci in enumerate(cols):
                label, _, items = columns[ci]
                star = ' *' if any((names[index], i) in made_for for i in items) else ''
                x0 = left + c * (2 * SIZE + gap)
                d.text((x0 + 4, top - 44), (label + star)[:70], fill=(225, 225, 225), font=small)
                d.text((x0 + 4, top - 22), a.before_name, fill=(170, 170, 170), font=tiny)
                d.text((x0 + SIZE + 4, top - 22), a.after_name, fill=(170, 170, 170), font=tiny)
                for r, (vname, _) in enumerate(VIEWS):
                    for k, side in enumerate(['before', 'after']):
                        img.paste(Image.open(render / f'pairs/{index:02}-{ci:02}-{side}-{vname}.png'), (x0 + k * SIZE, top + r * SIZE))
            img.save(a.output / f'{index:02}-{names[index]}-{gname}.png')
    print(f'{len(tiles)} tiles, sheets in {a.output}')


if __name__ == '__main__':
    main()
