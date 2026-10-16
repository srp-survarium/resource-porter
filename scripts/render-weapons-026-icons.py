#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the ported 0.69d rifles (H&K 416A5, MSBS Radon) from a built bundle.

Requires numpy and Pillow. Reads the bundle's converted meshes and textures
(the 0.26g files, after the albedo calibration) and the players' current
`lobby_menu.swf`. Draws each rifle's icon into its cell of the
`LobbyMenu_WeaponIcons` bitmap (right side, muzzle right, as the native
rifles) and checks that every other atlas pixel and every other SWF tag is
unchanged. `--gallery DIR` also writes larger renders: both sides, the sight
picture from behind, and a three-quarter view.

    render-weapons-026-icons.py --bundle B --lobby-swf lobby_menu.swf \
        --output OUT [--gallery DIR]
"""
import argparse
import struct
import zlib
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

CELL_W, CELL_H, PITCH = 171, 83, 91
SYMBOL = b'LobbyMenu_WeaponIcons'
# key -> (model root, icon cell [row, col])
RIFLES = {
    'hk416': ('models/weapons/assault/hk416_a5/hk416_a5.skinned_model', (7, 3)),
    'radon': ('models/weapons/assault/msbs_carbine/msbs_carbine.skinned_model', (8, 3)),
}
NATIVE_REFERENCE = [(3, 3), (10, 2), (16, 2)]  # AK-12, AK-74M, A-545


def swf_tags(data):
    assert data[:3] in (b'CWS', b'FWS')
    if data[:3] == b'CWS':
        data = data[:8] + zlib.decompress(data[8:])
    at = 8 + (5 + 4 * (data[8] >> 3) + 7) // 8 + 4
    prefix, tags = data[:at], []
    while at < len(data):
        header = struct.unpack_from('<H', data, at)[0]; at += 2
        size = header & 63
        if size == 63:
            size = struct.unpack_from('<I', data, at)[0]; at += 4
        tags.append((header >> 6, data[at:at + size])); at += size
    return prefix, tags


def atlas(tags):
    char_id = None
    for kind, payload in tags:
        if kind != 76:
            continue
        at = 2
        for _ in range(struct.unpack_from('<H', payload)[0]):
            ident = struct.unpack_from('<H', payload, at)[0]; at += 2
            end = payload.index(0, at)
            if payload[at:end] == SYMBOL:
                char_id = ident
            at = end + 1
    assert char_id is not None, 'weapon icon bitmap missing'
    for index, (kind, payload) in enumerate(tags):
        if kind == 36 and struct.unpack_from('<H', payload)[0] == char_id:
            ident, fmt, w, h = struct.unpack_from('<HBHH', payload)
            assert fmt == 5
            argb = np.frombuffer(zlib.decompress(payload[7:]), np.uint8).reshape(h, w, 4).copy()
            return index, ident, argb
    raise ValueError('weapon icon bitmap is not lossless ARGB')


def encode_tag(kind, payload):
    if len(payload) < 63:
        return struct.pack('<H', kind << 6 | len(payload)) + payload
    return struct.pack('<HI', kind << 6 | 63, len(payload)) + payload


# --- resources -------------------------------------------------------------

def config_strings(b):
    """Every (key path, string) of a 24-byte binary config (enough to read
    settings and materials)."""
    out = {}

    def cstr(at):
        return b[at:b.index(0, at)].decode('latin-1')

    def walk(at, path):
        data, name, _, kind, count = struct.unpack_from('<QQIHH', b, at)
        if kind in (3, 4):
            for i in range(count):
                child = data + 24 * i
                key = cstr(struct.unpack_from('<Q', b, child + 8)[0]) if kind == 3 else str(i)
                walk(child, path + [key])
        elif kind == 5:
            out['.'.join(path)] = cstr(data)
    walk(0, [])
    return out


def mesh(path):
    b = path.read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    raw = np.frombuffer(b[12:12 + 36 * n], np.uint8).reshape(n, 36)
    f = raw.copy().view('<f4')
    pos = f[:, 0:3].astype(float)
    words = raw[:, 16:20].copy().view('<u4')[:, 0]
    normal = np.stack([(words >> 16) & 255, (words >> 8) & 255, words & 255], 1) / 127.5 - 1
    uv = f[:, 7:9].astype(float)
    at = 12 + 36 * n + 8
    m = struct.unpack_from('<I', b, at)[0]
    idx = np.frombuffer(b[at + 4:at + 4 + 2 * m], '<u2').reshape(-1, 3)
    return pos, normal, uv, idx


def dxt(path):
    """Decode mip 0 of a DXT1/DXT5 DDS to RGBA."""
    b = path.read_bytes()
    h, w = struct.unpack_from('<II', b, 12)
    four = b[84:88]
    size = 8 if four == b'DXT1' else 16
    blocks = np.frombuffer(b[128:128 + (w // 4) * (h // 4) * size], np.uint8).reshape(-1, size)
    colour = blocks[:, size - 8:]
    c = colour[:, :4].copy().view('<u2')
    c0, c1 = c[:, 0].astype(int), c[:, 1].astype(int)

    def rgb(v):
        return np.stack([(v >> 11) * 255 // 31, ((v >> 5) & 63) * 255 // 63, (v & 31) * 255 // 31], 1)
    p0, p1 = rgb(c0), rgb(c1)
    four_colour = (c0 > c1) | (size == 16)
    pal = np.stack([p0, p1,
                    np.where(four_colour[:, None], (2 * p0 + p1) // 3, (p0 + p1) // 2),
                    np.where(four_colour[:, None], (p0 + 2 * p1) // 3, 0)], 1)
    bits = colour[:, 4:8].copy().view('<u4')[:, 0]
    sel = (bits[:, None] >> (2 * np.arange(16))) & 3
    px = np.take_along_axis(pal, sel[:, :, None].repeat(3, 2), 1)
    alpha = np.full((len(blocks), 16), 255)
    if size == 16:
        a0, a1 = blocks[:, 0].astype(int), blocks[:, 1].astype(int)
        ab = blocks[:, 2:8].astype(np.uint64)
        abits = sum(ab[:, i] << np.uint64(8 * i) for i in range(6))
        asel = ((abits[:, None] >> (np.uint64(3) * np.arange(16, dtype=np.uint64))) & np.uint64(7)).astype(int)
        apal = np.zeros((len(blocks), 8), int)
        apal[:, 0], apal[:, 1] = a0, a1
        for i in range(1, 7):
            apal[:, i + 1] = np.where(a0 > a1, ((7 - i) * a0 + i * a1) // 7, ((5 - i) * a0 + i * a1) // 5)
        apal[:, 6] = np.where(a0 > a1, apal[:, 6], 0)
        apal[:, 7] = np.where(a0 > a1, apal[:, 7], 255)
        alpha = np.take_along_axis(apal, asel, 1)
    img = np.concatenate([px, alpha[:, :, None]], 2).reshape(h // 4, w // 4, 4, 4, 4)
    return img.transpose(0, 2, 1, 3, 4).reshape(h, w, 4).astype(np.uint8)


def rifle_parts(bundle, root):
    o = bundle / 'overlay'
    settings = config_strings((o / root / 'settings').read_bytes())
    parts = []
    for key, surface in sorted((k, v) for k, v in settings.items() if k.startswith('lod_hierrarchy.LOD0.surfaces.')):
        material = settings[f'material_settings.{surface}.material_name']
        mat = config_strings((o / f'material_instances/{material}.material').read_bytes())
        tex = mat['material.g_stage.effect.texture_diffuse.value']
        pos, normal, uv, idx = mesh(o / root / f'render/{surface}/converted_model')
        parts.append((surface, pos, normal, uv, idx, dxt(o / f'textures/{tex}.dds')))
    return parts


# --- rasteriser ------------------------------------------------------------

def render(parts, view, size, margin=0.03, light=(0.35, 0.75, 0.55), exposure=1.0, super_sample=3):
    W, H = size[0] * super_sample, size[1] * super_sample
    view = np.asarray(view, float)
    allp = np.concatenate([p[1] for p in parts]) @ view.T
    lo, hi = allp.min(0), allp.max(0)
    scale = min(W * (1 - 2 * margin) / (hi[0] - lo[0]), H * (1 - 2 * margin) / (hi[1] - lo[1]))
    cx, cy = (lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2
    rgb = np.zeros((H, W, 3)); cov = np.zeros((H, W)); zb = np.full((H, W), -1e9)
    L = np.asarray(light, float); L /= np.linalg.norm(L)
    for _, pos, normal, uv, idx, tex in parts:
        p = pos @ view.T; n = normal @ view.T
        sx = (p[:, 0] - cx) * scale + W / 2; sy = H / 2 - (p[:, 1] - cy) * scale; sz = p[:, 2]
        th, tw = tex.shape[:2]
        for a, b, c in idx:
            xs, ys = sx[[a, b, c]], sy[[a, b, c]]
            x0, x1 = max(0, int(xs.min())), min(W - 1, int(np.ceil(xs.max())))
            y0, y1 = max(0, int(ys.min())), min(H - 1, int(np.ceil(ys.max())))
            if x0 > x1 or y0 > y1:
                continue
            d = (ys[1] - ys[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (ys[0] - ys[2])
            if abs(d) < 1e-12:
                continue
            gx, gy = np.meshgrid(np.arange(x0, x1 + 1) + .5, np.arange(y0, y1 + 1) + .5)
            w0 = ((ys[1] - ys[2]) * (gx - xs[2]) + (xs[2] - xs[1]) * (gy - ys[2])) / d
            w1 = ((ys[2] - ys[0]) * (gx - xs[2]) + (xs[0] - xs[2]) * (gy - ys[2])) / d
            w2 = 1 - w0 - w1
            m = (w0 >= 0) & (w1 >= 0) & (w2 >= 0)
            if not m.any():
                continue
            z = w0 * sz[a] + w1 * sz[b] + w2 * sz[c]
            sub = zb[y0:y1 + 1, x0:x1 + 1]
            m &= z > sub
            if not m.any():
                continue
            sub[m] = z[m]
            u = w0 * uv[a, 0] + w1 * uv[b, 0] + w2 * uv[c, 0]
            v = w0 * uv[a, 1] + w1 * uv[b, 1] + w2 * uv[c, 1]
            colour = tex[(np.mod(v[m], 1) * th).astype(int) % th, (np.mod(u[m], 1) * tw).astype(int) % tw, :3] / 255.0
            nn = w0[m][:, None] * n[a] + w1[m][:, None] * n[b] + w2[m][:, None] * n[c]
            nn /= np.maximum(np.linalg.norm(nn, axis=1, keepdims=True), 1e-9)
            shade = 0.55 + 0.6 * np.clip(np.abs(nn @ L), 0, 1)
            rgb[y0:y1 + 1, x0:x1 + 1][m] = colour * shade[:, None] * exposure
            cov[y0:y1 + 1, x0:x1 + 1][m] = 1
    img = Image.fromarray(np.dstack([np.clip(rgb, 0, 1) * 255, cov * 255]).astype(np.uint8), 'RGBA')
    return img.resize(size, Image.LANCZOS)


def with_shadow(icon, offset=(2, 3), blur=2.5, strength=0.55):
    a = np.asarray(icon)[:, :, 3].astype(float) / 255
    shadow = Image.fromarray((a * 255 * strength).astype(np.uint8)).filter(ImageFilter.GaussianBlur(blur))
    shadow = np.roll(np.roll(np.asarray(shadow).astype(float) / 255, offset[1], 0), offset[0], 1)
    rgba = np.asarray(icon).astype(float) / 255
    out_a = rgba[:, :, 3] + shadow * (1 - rgba[:, :, 3])
    out_rgb = rgba[:, :, :3] * rgba[:, :, 3:4] / np.maximum(out_a[:, :, None], 1e-9)
    return Image.fromarray((np.dstack([out_rgb, out_a]) * 255).astype(np.uint8), 'RGBA')


# Views: rows are screen x, screen y, depth (larger is nearer). Meshes face
# +z (muzzle) with +x on the weapon's left.
RIGHT = [[0, 0, 1], [0, 1, 0], [-1, 0, 0]]
LEFT = [[0, 0, -1], [0, 1, 0], [1, 0, 0]]
BEHIND = [[-1, 0, 0], [0, 1, 0], [0, 0, -1]]


def three_quarter(yaw=35, pitch=15):
    y, p = np.radians(yaw), np.radians(pitch)
    r = np.array([[np.cos(y), 0, -np.sin(y)], [0, 1, 0], [np.sin(y), 0, np.cos(y)]])
    t = np.array([[1, 0, 0], [0, np.cos(p), np.sin(p)], [0, -np.sin(p), np.cos(p)]])
    return t @ r @ np.asarray(RIGHT, float)


def mean_luma(rgba):
    a = rgba[:, :, 3] > 200
    return (rgba[:, :, :3][a] @ [0.299, 0.587, 0.114]).mean()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--lobby-swf', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--gallery', type=Path)
    args = ap.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    prefix, tags = swf_tags(args.lobby_swf.read_bytes())
    tag_index, char_id, argb = atlas(tags)
    original = argb.copy()
    rgba = argb[:, :, [1, 2, 3, 0]]

    def cell(r, c):
        return (slice((r - 1) * PITCH, (r - 1) * PITCH + CELL_H), slice((c - 1) * CELL_W, c * CELL_W))
    target_luma = np.mean([mean_luma(rgba[cell(*rc)]) for rc in NATIVE_REFERENCE])
    for key, (root, (r, c)) in RIFLES.items():
        parts = rifle_parts(args.bundle, root)
        assert argb[cell(r, c)][:, :, 0].max() == 0, f'icon cell {r},{c} is taken'
        probe = np.asarray(render(parts, RIGHT, (CELL_W - 8, CELL_H - 10)))
        exposure = target_luma / max(mean_luma(probe), 1e-6)
        icon = render(parts, RIGHT, (CELL_W - 8, CELL_H - 10), exposure=exposure)
        canvas = Image.new('RGBA', (CELL_W, CELL_H))
        canvas.paste(icon, (3, 3))
        canvas = with_shadow(canvas)
        canvas.save(args.output / f'{key}-icon.png')
        px = np.asarray(canvas)
        # The atlas holds premultiplied ARGB, as the SWF bitmap does.
        a = px[:, :, 3:4].astype(float) / 255
        argb[cell(r, c)] = np.dstack([px[:, :, 3], (px[:, :, :3] * a).round().astype(np.uint8)])
        print(f'{key}: cell [{r}, {c}], exposure {exposure:.2f}')
        if args.gallery:
            args.gallery.mkdir(parents=True, exist_ok=True)
            for name, view, size in [('right', RIGHT, (1600, 700)), ('left', LEFT, (1600, 700)),
                                     ('sights-from-behind', BEHIND, (900, 900)),
                                     ('three-quarter', three_quarter(), (1600, 900))]:
                img = render(parts, view, size, exposure=exposure, super_sample=2)
                bg = Image.new('RGBA', size, (70, 70, 70, 255))
                bg.alpha_composite(img)
                bg.convert('RGB').save(args.gallery / f'{key}-{name}.png')
    changed = np.argwhere((argb != original).any(2))
    for y, x in changed:
        row, col = y // PITCH + 1, x // CELL_W + 1
        assert (row, col) in [rc for _, rc in RIFLES.values()], f'pixel {x},{y} outside the new cells'
    payload = struct.pack('<HBHH', char_id, 5, argb.shape[1], argb.shape[0]) + zlib.compress(argb.tobytes(), 9)
    tags[tag_index] = (36, payload)
    body = prefix[8:] + b''.join(encode_tag(k, p) for k, p in tags)
    swf = b'CWS' + prefix[3:4] + struct.pack('<I', 8 + len(body)) + zlib.compress(body, 9)
    (args.output / 'lobby_menu.swf').write_bytes(swf)
    _, check = swf_tags(swf)
    _, _, again = atlas(check)
    assert (again == argb).all()
    for (k, p), (k2, p2) in zip(swf_tags(args.lobby_swf.read_bytes())[1], check):
        assert k == k2 and (p == p2 or k == 36), 'a tag other than the atlas changed'
    print(f'wrote {args.output / "lobby_menu.swf"}')


if __name__ == '__main__':
    main()
