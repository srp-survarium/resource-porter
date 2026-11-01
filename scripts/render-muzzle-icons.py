#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the lobby icons of the ported muzzle devices from a built bundle (port-muzzles).

Requires numpy and Pillow; reuses render-weapons-069d-icons.py's rasteriser and atlas reader.
Reads each device's converted static model and diffuse texture from the bundle, its icon cell
from `port-manifest.json`, and the players' `lobby_menu.swf` (the cells, which must be empty,
and the native scopes' icons in LobbyMenu_AmmoIcons column 4, for the exposure). Writes each icon
(a three-quarter view from the right, muzzle right) as a straight-alpha PNG into
`<bundle>/icons/` with `icons/icons.json`, which `vostok-port pack from-bundle` draws into
LobbyMenu_AmmoIcons (widening it to the new columns).

    render-muzzle-icons.py --bundle B --lobby-swf lobby_menu.swf
"""
import argparse
import importlib.util
import json
import struct
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('icons069', HERE / 'render-weapons-069d-icons.py')
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)

NATIVE_SCOPES = [(1, 4), (2, 4), (3, 4)]


def static_mesh(path):
    """A 0.26g static mesh: positions, normals, UVs (half2), triangles."""
    b = path.read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    raw = np.frombuffer(b[12:12 + 28 * n], np.uint8).reshape(n, 28)
    pos = raw[:, 0:12].copy().view('<f4').astype(float)
    words = raw[:, 12:16].copy().view('<u4')[:, 0]
    normal = np.stack([(words >> 16) & 255, (words >> 8) & 255, words & 255], 1) / 127.5 - 1
    uv = raw[:, 24:28].copy().view('<f2').astype(float)
    at = 12 + 28 * n + 8
    m = struct.unpack_from('<I', b, at)[0]
    idx = np.frombuffer(b[at + 4:at + 4 + 2 * m], '<u2').reshape(-1, 3)
    return pos, normal, uv, idx


def device_parts(bundle, model):
    o = bundle / 'overlay'
    root = Path(f'models/{model}.model')
    settings = icons.config_strings((o / root / 'settings').read_bytes())
    parts = []
    for key, surface in sorted((k, v) for k, v in settings.items() if k.startswith('lod_hierrarchy.LOD0.surfaces.')):
        material = settings[f'material_settings.{surface}.material_name']
        mat = icons.config_strings((o / f'material_instances/{material}.material').read_bytes())
        tex = mat['material.g_stage.effect.texture_diffuse.value']
        pos, normal, uv, idx = static_mesh(o / root / f'render/{surface}/converted_model')
        parts.append((surface, pos, normal, uv, idx, icons.dxt(o / f'textures/{tex}.dds')))
    return parts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--lobby-swf', type=Path, required=True)
    args = ap.parse_args()
    out = args.bundle / 'icons'
    out.mkdir(parents=True, exist_ok=True)
    _, tags = icons.swf_tags(args.lobby_swf.read_bytes())
    _, _, argb = icons.atlas(tags, icons.AMMO_SYMBOL)
    rgba = argb[:, :, [1, 2, 3, 0]]
    W, H, P = icons.CELL_W, icons.CELL_H, icons.PITCH

    def cell(r, c):
        return (slice((r - 1) * P, (r - 1) * P + H), slice((c - 1) * W, c * W))

    def taken(r, c):
        rows, cols = cell(r, c)
        if cols.start >= argb.shape[1]:
            return False  # a column the pack adds
        return argb[rows, cols][:, :, 0].max() > 0

    target = np.mean([icons.mean_luma(rgba[cell(*rc)]) for rc in NATIVE_SCOPES])
    view = icons.three_quarter(yaw=30, pitch=12)
    listed = []
    for d in json.loads((args.bundle / 'port-manifest.json').read_text())['devices']:
        r, c = d['icon']
        assert not taken(r, c), f'icon cell {r},{c} is taken'
        parts = device_parts(args.bundle, d['model'])
        size = (W - 24, H - 16)
        probe = np.asarray(icons.render(parts, view, size))
        exposure = target / max(icons.mean_luma(probe), 1e-6)
        icon = icons.render(parts, view, size, exposure=exposure)
        canvas = Image.new('RGBA', (W, H))
        canvas.paste(icon, ((W - icon.size[0]) // 2, (H - 4 - icon.size[1]) // 2))
        icons.with_shadow(canvas).save(out / f"{d['item']}.png")
        listed.append({'symbol': icons.AMMO_SYMBOL.decode(), 'row': r, 'col': c, 'source': f"icons/{d['item']}.png"})
        print(f"{d['item']}: cell [{r}, {c}], exposure {exposure:.2f}")
    (out / 'icons.json').write_text(json.dumps(listed, indent=2) + '\n')


if __name__ == '__main__':
    main()
