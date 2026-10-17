#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""The closed Keeper helmet's own diffuse texture: the native one without the
darkness baked under the raised visor.

Requires numpy and Pillow. With the visor down, the parts it covered show:
the front of the dome, the brow wedge, the visor's underside and the arms'
inner faces. Their texels carry the occlusion of the raised visor (near
black), so each of those UV islands is lifted to the lit level of the shell:
where a texel's surroundings are dark, lit metal cloned from the dome's lit
middle (mirror-tiled, softened) takes over; brighter features (rims, edges)
keep their own texels, lifted by a gain. Every other texel is the native
one. Writes a PNG for `examples/build-keeper-closed-026.rs --diffuse`.
"""
import argparse
import struct
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

# Native vertex in each island to relight, and the luminance it is lifted to
# (the lit level of the dome's middle and of the shell around it).
ISLANDS = {
    'dome': (67, 0.19),
    'brow wedge': (203, 0.18),
    'visor underside': (40, 0.16),
    'right arm inner face': (44, 0.16),
    'left arm inner face': (386, 0.16),
}
# The dome's lit middle, below its three triangles: the metal cloned in.
SOURCE_UV = (0.385, 0.725, 0.585, 0.86)
LUMA = np.array([0.299, 0.587, 0.114])


def mesh(path):
    b = Path(path).read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    stride = (struct.unpack_from('<I', b, 4)[0] - 4) // n
    assert stride == 36, stride
    raw = np.frombuffer(b[12:12 + n * stride], np.uint8).reshape(n, stride)
    at = 12 + n * stride
    count = struct.unpack_from('<I', b, at + 8)[0]
    faces = np.frombuffer(b, '<u2', count, at + 12).reshape(-1, 3).astype(int)
    uv = raw[:, 28:36].copy().view('<f4').reshape(n, 2).astype(float)
    return n, faces, uv


def islands(n, faces):
    parent = list(range(n))

    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    for f in faces:
        a = root(f[0])
        for v in f[1:]:
            parent[root(v)] = a
    return np.array([root(i) for i in range(n)])


def labels(faces, uv, island, size):
    """Each texel's island (the root vertex), -1 where no triangle is."""
    w, h = size
    image = Image.new('I', size, -1)
    draw = ImageDraw.Draw(image)
    for f in faces:
        u = uv[f, 0]
        u = u - 1 if u.max() > 1 else u  # the left half's mirrored charts
        draw.polygon([(u[k] * w, uv[f[k], 1] * h) for k in range(3)], fill=int(island[f[0]]))
    return np.asarray(image)


def blur(a, r):
    out = a.astype(float)
    for _ in range(3):
        c = np.cumsum(np.pad(out, ((r + 1, r), (0, 0)), mode='edge'), 0)
        out = (c[2 * r + 1:] - c[:-2 * r - 1]) / (2 * r + 1)
        c = np.cumsum(np.pad(out, ((0, 0), (r + 1, r)), mode='edge'), 1)
        out = (c[:, 2 * r + 1:] - c[:, :-2 * r - 1]) / (2 * r + 1)
    return out


def masked_blur(v, mask, r):
    return blur(v * mask, r) / np.maximum(blur(mask.astype(float), r), 1e-6)


def mirror_tile(src, shape, origin):
    h, w = src.shape[:2]

    def fold(i, n):
        i = i % (2 * n)
        return np.where(i >= n, 2 * n - 1 - i, i)
    return src[fold(np.arange(shape[0]) - origin[0], h)][:, fold(np.arange(shape[1]) - origin[1], w)]


def relight(rgb, label, targets, box, radius=10, grow=3):
    lum = rgb @ LUMA
    out = rgb.copy()
    x0, y0, x1, y1 = box
    tile = mirror_tile(rgb[y0:y1, x0:x1], rgb.shape[:2], (y0, x0))
    tile = 0.55 * tile + 0.45 * np.dstack([blur(tile[..., c], 6) for c in range(3)])
    scale_base = np.mean(tile[y0:y1, x0:x1] @ LUMA)
    for key, target in targets:
        m = label == key
        assert m.any(), key
        low = masked_blur(lum, m, radius)
        weight = masked_blur(np.clip((target - low) / (0.5 * target), 0, 1), m, 2)
        gain = np.clip(target / np.maximum(low, 1e-3), 1, 2.0)
        metal = np.clip(tile * (target / scale_base), 0, 1)
        inside = (1 - weight)[..., None] * np.clip(rgb * gain[..., None], 0, 1) + weight[..., None] * metal
        # The rim of unused texels around the island (DXT blocks and mip
        # filtering read them) gets the island's values.
        zone = (blur(m.astype(float), grow) > 0.01) & ((label == key) | (label < 0))
        rim_w, rim_g = masked_blur(weight, m, grow + 1), masked_blur(gain, m, grow + 1)
        rim = (1 - rim_w)[..., None] * np.clip(rgb * rim_g[..., None], 0, 1) + rim_w[..., None] * metal
        out[zone] = np.where(m[zone][:, None], inside[zone], rim[zone])
    return np.clip(out, 0, 1)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--model', type=Path, required=True,
                    help="the native edge_assault_05_helmet_s converted_model")
    ap.add_argument('--texture', type=Path, required=True, help='the native edge_assault_05_helmet.dds')
    ap.add_argument('--output', type=Path, required=True, help='the PNG to write')
    ap.add_argument('--compare', type=Path, help='also a native-above-new comparison here')
    a = ap.parse_args()
    n, faces, uv = mesh(a.model)
    island = islands(n, faces)
    rgb = np.asarray(Image.open(a.texture).convert('RGB'), float) / 255
    h, w = rgb.shape[:2]
    label = labels(faces, uv, island, (w, h))
    targets = [(int(island[v]), lum) for v, lum in ISLANDS.values()]
    box = (int(SOURCE_UV[0] * w), int(SOURCE_UV[1] * h), int(SOURCE_UV[2] * w), int(SOURCE_UV[3] * h))
    out = relight(rgb, label, targets, box)
    for name, (v, _) in ISLANDS.items():
        m = label == island[v]
        print(f'{name}: {m.sum()} texels, luminance {np.mean(rgb[m] @ LUMA):.3f} -> {np.mean(out[m] @ LUMA):.3f}')
    a.output.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray((out * 255 + 0.5).astype(np.uint8)).save(a.output)
    if a.compare:
        Image.fromarray((np.vstack([rgb, out]) * 255 + 0.5).astype(np.uint8)).save(a.compare)
    print('wrote', a.output)


if __name__ == '__main__':
    main()
