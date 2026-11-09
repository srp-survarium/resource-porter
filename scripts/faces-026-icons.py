#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""The face picker's icons, drawn into a faces pack made by `vostok-port pack from-bundle`.

    faces-026-icons.py --bundle B --renderer target/release/examples/render-faces-026 --pack P

Renders every face of the bundle's render/scene.json (the native face 0 first) bare, a
little turned, on a clear background, at the lobby's equipment cell size (171x83), as
P/icons/face_NN.png, and appends one `atlas_icons_add` operation to P/pack.json drawing
face i into cell [i + 1, 16] of LobbyMenu_EquipmentIcons: the cells gameplay/bodyparts'
heads[i].icon names (faces_026::ICON_COLUMN). Prints the new pack.json sha256.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

COLUMN = 16


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--renderer', type=Path, required=True)
    ap.add_argument('--pack', type=Path, required=True)
    a = ap.parse_args()
    render = a.bundle / 'render'
    scene = json.loads((render / 'scene.json').read_text())
    faces = {0: [{'surface': scene['native']['surface'], 'diffuse': scene['native']['diffuse']}]}
    for f in scene['faces']:
        if f['state'] != 'bare':
            continue
        hair = 'hair' in f['surface']
        faces.setdefault(f['index'], []).append(
            {'surface': f['surface'], 'diffuse': '../' + f['diffuse'], 'two_sided': hair, 'alpha_test': hair})
    tiles = [{'out': f'icons/face_{i:02}.png', 'yaw': 18, 'pitch': -4, 'meshes': m} for i, m in sorted(faces.items())]
    plan = {'size': 83, 'aspect': 171 / 83, 'centre': [0.0, 1.68, 0.0], 'height': 0.30, 'transparent': True,
            'tiles': tiles}
    (render / 'icons-plan.json').write_text(json.dumps(plan))
    subprocess.run([str(a.renderer), '--plan', str(render / 'icons-plan.json')], check=True)
    (a.pack / 'icons').mkdir(exist_ok=True)
    icons = []
    for i in sorted(faces):
        name = f'icons/face_{i:02}.png'
        (a.pack / name).write_bytes((render / name).read_bytes())
        icons.append({'row': i + 1, 'col': COLUMN, 'source': name})
    manifest = json.loads((a.pack / 'pack.json').read_text())
    if any(op['op'] == 'atlas_icons_add' for op in manifest['operations']):
        raise SystemExit('the pack draws icons already')
    manifest['operations'].append({'op': 'atlas_icons_add', 'icons': icons})
    raw = (json.dumps(manifest, ensure_ascii=False, indent=2) + '\n').encode()
    (a.pack / 'pack.json').write_bytes(raw)
    print(f'{len(icons)} icons; pack.json {hashlib.sha256(raw).hexdigest()}')


if __name__ == '__main__':
    main()
