#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline preview of decals projected onto a 0.26g model (no game).

    render-decal-026-preview.py --surface MODEL TEXTURE [--surface ...] \\
        --decal DIFFUSE.dds [--decal ...] --at X,Y,Z --size METRES \\
        [--facing X,Y,Z] [--yaw 0] [--pitch 0] [--width 512] --output SHEET.png

Each MODEL is a 0.26g `converted_model` (48-byte vertices, as
render-duty-026-icons.py reads them) and TEXTURE its diffuse (a DDS or any
image Pillow reads). The decal is put the way the lobby's decal editor puts
one: a box projection along the surface normal at the point of the model
nearest to `--at` (a point in the bounding box, as fractions 0..1 of each
axis), `--size` wide, its up the model's +Y. Pixels whose normal turns away
from the decal's take none of it (as the engine's angle cut). The decal's
diffuse alpha blends it over the surface's diffuse; lighting is the same
for both. One render a decal, side by side, in the sheet.

This shows how a decal texture reads at its size on a model. It is not the
game's bake (no normal, smoothness or fmap, a plain light).

Requires numpy and Pillow.
"""
import argparse
from pathlib import Path
import struct

import numpy as np
from PIL import Image


# Vertex layouts by size: (normal offset, uv offset). 48 bytes: a skinned character
# surface (four bone weights and indices); 36: a weapon's (one bone).
LAYOUTS = {48: (28, 40), 36: (16, 28)}


def load_surface(model, texture):
    b = Path(model).read_bytes()
    size, n = struct.unpack_from('<II', b, 4)
    stride = (size - 4) // n
    if stride not in LAYOUTS or (size - 4) % n:
        raise SystemExit(f'{model}: {size} vertex bytes for {n} vertices is no layout this reads')
    normal_at, uv_at = LAYOUTS[stride]
    raw = np.frombuffer(b[12:12 + stride * n], np.uint8).reshape(n, stride)
    p = raw[:, :12].copy().view('<f4').astype(float)
    uv = raw[:, uv_at:uv_at + 8].copy().view('<f4').astype(float)
    normals = raw[:, normal_at:normal_at + 3][:, ::-1].astype(float) / 127.5 - 1
    faces = np.frombuffer(b, dtype='<u2', offset=24 + stride * n).reshape(-1, 3)
    tex = np.asarray(Image.open(texture).convert('RGB'), dtype=float) / 255
    return p, uv, normals, faces, tex


def place(surfaces, at, facing=None):
    """The decal's centre, normal and axes: the vertex nearest to `at` (fractions of the box);
    with `facing`, of the vertices whose normal is within about 35 degrees of it, and that normal."""
    points = np.concatenate([s[0] for s in surfaces])
    normals = np.concatenate([s[2] for s in surfaces])
    normals = normals / np.maximum(np.linalg.norm(normals, axis=1)[:, None], 1e-8)
    lo, hi = points.min(0), points.max(0)
    target = lo + (hi - lo) * np.asarray(at)
    distance = np.linalg.norm(points - target, axis=1)
    if facing is not None:
        facing = np.asarray(facing, float) / np.linalg.norm(facing)
        distance[normals @ facing < 0.82] = np.inf
    index = np.argmin(distance)
    centre = points[index]
    normal = facing if facing is not None else normals[index]
    up = np.array([0.0, 1.0, 0.0]) - normal * normal[1]
    if np.linalg.norm(up) < 1e-3:
        up = np.array([0.0, 0.0, 1.0]) - normal * normal[2]
    up /= np.linalg.norm(up)
    right = np.cross(up, normal)
    return centre, normal, right, up


def render(surfaces, decal, frame, size, yaw, pitch, image_size):
    centre, dnormal, dright, dup = frame
    yaw, pitch = np.radians([yaw, pitch])
    toward = np.array([np.sin(yaw) * np.cos(pitch), np.sin(pitch), np.cos(yaw) * np.cos(pitch)])
    right = np.cross([0, 1, 0], toward)
    right /= np.linalg.norm(right)
    up = np.cross(toward, right)
    basis = np.stack([right, up, toward], axis=1)
    allp = np.concatenate([s[0] for s in surfaces]) @ basis
    lo, hi = allp.min(0), allp.max(0)
    w, h = image_size
    scale = min((w - 32) / (hi[0] - lo[0]), (h - 32) / (hi[1] - lo[1]))
    mid = (lo + hi) / 2
    out = np.zeros((h, w, 4), np.uint8)
    depth = np.full((h, w), -np.inf)
    light = right * -0.45 + up * 0.75 + toward * 0.8
    light /= np.linalg.norm(light)
    for p, uv, normals, faces, tex in surfaces:
        pp = p @ basis
        screen = (pp[:, :2] - mid[:2]) * [scale, -scale] + [w / 2, h / 2]
        for face in faces:
            q = screen[face]
            z = pp[face, 2]
            x0, y0 = np.maximum(np.floor(q.min(0)).astype(int), 0)
            x1, y1 = np.minimum(np.ceil(q.max(0)).astype(int), [w - 1, h - 1])
            if x0 > x1 or y0 > y1:
                continue
            den = (q[1, 1] - q[2, 1]) * (q[0, 0] - q[2, 0]) + (q[2, 0] - q[1, 0]) * (q[0, 1] - q[2, 1])
            if abs(den) < 1e-10:
                continue
            yy, xx = np.mgrid[y0:y1 + 1, x0:x1 + 1]
            xx = xx + .5
            yy = yy + .5
            a = ((q[1, 1] - q[2, 1]) * (xx - q[2, 0]) + (q[2, 0] - q[1, 0]) * (yy - q[2, 1])) / den
            b = ((q[2, 1] - q[0, 1]) * (xx - q[2, 0]) + (q[0, 0] - q[2, 0]) * (yy - q[2, 1])) / den
            c = 1 - a - b
            dz = a * z[0] + b * z[1] + c * z[2]
            visible = (a >= 0) & (b >= 0) & (c >= 0) & (dz > depth[y0:y1 + 1, x0:x1 + 1])
            if not visible.any():
                continue
            weights = np.stack([a[visible], b[visible], c[visible]], axis=1)
            tuv = weights @ uv[face]
            tx = np.clip((tuv[:, 0] % 1) * (tex.shape[1] - 1), 0, tex.shape[1] - 1).astype(int)
            ty = np.clip((tuv[:, 1] % 1) * (tex.shape[0] - 1), 0, tex.shape[0] - 1).astype(int)
            rgb = tex[ty, tx]
            normal = weights @ normals[face]
            normal /= np.maximum(np.linalg.norm(normal, axis=1)[:, None], 1e-8)
            world = weights @ p[face]
            d = world - centre
            s = d @ dright / size + 0.5
            t = 0.5 - d @ dup / size
            inside = (s >= 0) & (s < 1) & (t >= 0) & (t < 1) & (np.abs(d @ dnormal) < size / 2) & \
                (normal @ dnormal > 0.35)
            if inside.any():
                dx = np.clip((s[inside] * decal.shape[1]).astype(int), 0, decal.shape[1] - 1)
                dy = np.clip((t[inside] * decal.shape[0]).astype(int), 0, decal.shape[0] - 1)
                sample = decal[dy, dx]
                alpha = sample[:, 3:4]
                rgb[inside] = rgb[inside] * (1 - alpha) + sample[:, :3] * alpha
            illumination = .38 + .75 * np.maximum(normal @ light, 0)
            rgb = np.clip(rgb * illumination[:, None] * [1.05, 1.01, .94], 0, 1)
            patch = out[y0:y1 + 1, x0:x1 + 1]
            patch[visible, :3] = (rgb * 255).astype(np.uint8)
            patch[visible, 3] = 255
            depth[y0:y1 + 1, x0:x1 + 1][visible] = dz[visible]
    return Image.fromarray(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--surface', nargs=2, action='append', required=True, metavar=('MODEL', 'TEXTURE'))
    ap.add_argument('--decal', action='append', required=True, type=Path)
    ap.add_argument('--at', required=True)
    ap.add_argument('--facing', help='X,Y,Z: the decal faces this way (the surface where it goes)')
    ap.add_argument('--size', required=True, type=float)
    ap.add_argument('--yaw', type=float, default=0)
    ap.add_argument('--pitch', type=float, default=0)
    ap.add_argument('--width', type=int, default=512)
    ap.add_argument('--height', type=int, default=512)
    ap.add_argument('--output', required=True, type=Path)
    args = ap.parse_args()
    surfaces = [load_surface(model, texture) for model, texture in args.surface]
    facing = [float(v) for v in args.facing.split(',')] if args.facing else None
    frame = place(surfaces, [float(v) for v in args.at.split(',')], facing)
    renders = []
    for path in args.decal:
        decal = np.asarray(Image.open(path).convert('RGBA'), dtype=float) / 255
        renders.append(render(surfaces, decal, frame, args.size, args.yaw, args.pitch,
                              (args.width, args.height)))
    sheet = Image.new('RGBA', (args.width * len(renders), args.height), (48, 48, 48, 255))
    for i, image in enumerate(renders):
        sheet.alpha_composite(image, (i * args.width, 0))
    sheet.save(args.output)
    print(f'{args.output}: {len(renders)} renders, decal at {frame[0].round(3)} facing {frame[1].round(2)}')


if __name__ == '__main__':
    main()
