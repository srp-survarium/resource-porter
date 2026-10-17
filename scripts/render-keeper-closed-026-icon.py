#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the closed Keeper helmet's equipment icon in the native icon's style.

Requires numpy and Pillow. The native helmet's icon (row 15, column 3 of the
lobby's equipment atlas) is the reference: the open model is rendered from the
camera whose silhouette matches that icon best, the colours of that render are
fitted to the icon's, and the closed model (`keeper_026`'s pack) is rendered
from the same camera with the same colours. The icon is written with straight
alpha, as `atlas_icons_add` takes it; a sheet of the native, the refitted open
and the closed icon goes beside it.
"""
import argparse
import struct
import zlib
from pathlib import Path

import numpy as np
from PIL import Image

ICON = (171, 83)
NATIVE_CELL = (15, 3)
SUPER = 4


def swf_bitmap(path, symbol=b'LobbyMenu_EquipmentIcons'):
    b = Path(path).read_bytes()
    if b[:3] == b'CWS':
        b = b[:8] + zlib.decompress(b[8:])
    at = 8 + (5 + 4 * (b[8] >> 3) + 7) // 8 + 4
    tags = []
    while at < len(b):
        header = struct.unpack_from('<H', b, at)[0]; at += 2
        size = header & 63
        if size == 63:
            size = struct.unpack_from('<I', b, at)[0]; at += 4
        tags.append((header >> 6, b[at:at + size])); at += size
    char = None
    for kind, payload in tags:
        if kind != 76:
            continue
        at = 2
        for _ in range(struct.unpack_from('<H', payload)[0]):
            ident = struct.unpack_from('<H', payload, at)[0]; at += 2
            end = payload.index(0, at)
            if payload[at:end] == symbol:
                char = ident
            at = end + 1
    for kind, payload in tags:
        if kind == 36 and struct.unpack_from('<H', payload)[0] == char:
            _, fmt, w, h = struct.unpack_from('<HBHH', payload)
            assert fmt == 5
            return np.frombuffer(zlib.decompress(payload[7:]), np.uint8).reshape(h, w, 4)
    raise SystemExit('no equipment atlas in the movie')


def native_icon(swf):
    """The native cell as straight-alpha RGBA floats."""
    atlas = swf_bitmap(swf)
    row, col = NATIVE_CELL
    argb = atlas[(row - 1) * 91:(row - 1) * 91 + 83, (col - 1) * 171:col * 171].astype(float) / 255
    alpha = argb[:, :, 0]
    rgb = argb[:, :, 1:] / np.maximum(alpha[:, :, None], 1e-6)
    return np.dstack([np.clip(rgb, 0, 1), alpha])


def load_mesh(path, texture):
    """A 36-byte-vertex converted model: position, bone, normal, tangent, binormal, UV."""
    b = Path(path).read_bytes()
    n = struct.unpack_from('<I', b, 8)[0]
    stride = (struct.unpack_from('<I', b, 4)[0] - 4) // n
    assert stride == 36, stride
    raw = np.frombuffer(b[12:12 + n * stride], np.uint8).reshape(n, stride)
    at = 12 + n * stride
    count = struct.unpack_from('<I', b, at + 8)[0]
    faces = np.frombuffer(b, '<u2', count, at + 12).reshape(-1, 3).astype(int)
    p = raw[:, :12].copy().view('<f4').reshape(n, 3).astype(float)
    uv = raw[:, 28:36].copy().view('<f4').reshape(n, 2).astype(float)
    basis = [raw[:, at:at + 3][:, ::-1].astype(float) / 127.5 - 1 for at in (16, 20, 24)]
    return dict(p=p, uv=uv, n=basis[0], t=basis[1], b=basis[2], f=faces, tex=texture)


def camera(yaw, pitch):
    yaw, pitch = np.radians([yaw, pitch])
    toward = np.array([np.sin(yaw) * np.cos(pitch), np.sin(pitch), np.cos(yaw) * np.cos(pitch)])
    right = np.cross([0, 1, 0], toward); right /= np.linalg.norm(right)
    up = np.cross(toward, right)
    return np.stack([right, up, toward], 1), right, up, toward


def render(mesh, view, scale, center, size):
    """Orthographic, z-buffered; diffuse, normal and specular maps, one key light."""
    basis, right, up, toward = view
    w, h = size
    out = np.zeros((h, w, 4))
    depth = np.full((h, w), -np.inf)
    light = right * -0.45 + up * 0.75 + toward * 0.8; light /= np.linalg.norm(light)
    pp = mesh['p'] @ basis
    screen = (pp[:, :2] - center) * [scale, -scale] + [w / 2, h / 2]
    tex, nmap, smap = mesh['tex']
    half = light + toward; half /= np.linalg.norm(half)
    for face in mesh['f']:
        q = screen[face]; z = pp[face, 2]
        x0, y0 = np.maximum(np.floor(q.min(0)).astype(int), 0)
        x1, y1 = np.minimum(np.ceil(q.max(0)).astype(int), [w - 1, h - 1])
        if x0 > x1 or y0 > y1:
            continue
        den = (q[1, 1] - q[2, 1]) * (q[0, 0] - q[2, 0]) + (q[2, 0] - q[1, 0]) * (q[0, 1] - q[2, 1])
        if abs(den) < 1e-10:
            continue
        yy, xx = np.mgrid[y0:y1 + 1, x0:x1 + 1] + 0.5
        a = ((q[1, 1] - q[2, 1]) * (xx - q[2, 0]) + (q[2, 0] - q[1, 0]) * (yy - q[2, 1])) / den
        c2 = ((q[2, 1] - q[0, 1]) * (xx - q[2, 0]) + (q[0, 0] - q[2, 0]) * (yy - q[2, 1])) / den
        c3 = 1 - a - c2
        dz = a * z[0] + c2 * z[1] + c3 * z[2]
        visible = (a >= 0) & (c2 >= 0) & (c3 >= 0) & (dz > depth[y0:y1 + 1, x0:x1 + 1])
        if not visible.any():
            continue
        weights = np.stack([a[visible], c2[visible], c3[visible]], 1)
        tuv = weights @ mesh['uv'][face]
        tx = np.clip((tuv[:, 0] % 1) * (tex.shape[1] - 1), 0, tex.shape[1] - 1).astype(int)
        ty = np.clip((tuv[:, 1] % 1) * (tex.shape[0] - 1), 0, tex.shape[0] - 1).astype(int)
        unit = lambda v: v / np.maximum(np.linalg.norm(v, axis=1)[:, None], 1e-8)
        n, t, bn = (unit(weights @ mesh[k][face]) for k in ('n', 't', 'b'))
        # Tangent-space normal map, x in alpha and y in green (DXT5nm).
        nx, ny = nmap[ty, tx, 0], nmap[ty, tx, 1]
        nz = np.sqrt(np.clip(1 - nx * nx - ny * ny, 0, 1))
        normal = unit(t * nx[:, None] + bn * ny[:, None] + n * nz[:, None])
        facing = np.sign(normal @ toward)[:, None]
        normal = normal * np.where(facing == 0, 1, facing)
        diffuse = 0.32 + 0.8 * np.maximum(normal @ light, 0)
        specular = 2.2 * smap[ty, tx] * np.maximum(normal @ half, 0) ** 24
        patch = out[y0:y1 + 1, x0:x1 + 1]
        patch[visible, :3] = np.clip(tex[ty, tx] * diffuse[:, None] + specular[:, None], 0, 1)
        patch[visible, 3] = 1
        depth[y0:y1 + 1, x0:x1 + 1][visible] = dz[visible]
    return out


def shrink(image, size):
    """Supersampled RGBA to `size`, colour averaged by coverage (straight alpha)."""
    h, w = image.shape[:2]
    fy, fx = h // size[1], w // size[0]
    blocks = image[:size[1] * fy, :size[0] * fx].reshape(size[1], fy, size[0], fx, 4)
    alpha = blocks[..., 3].mean((1, 3))
    rgb = (blocks[..., :3] * blocks[..., 3:4]).sum((1, 3)) / np.maximum(blocks[..., 3:4].sum((1, 3)), 1e-6)
    return np.dstack([rgb, alpha])


def blur(alpha, radius):
    """Three box blurs: close to a Gaussian."""
    out = alpha
    for _ in range(3):
        padded = np.pad(out, radius, mode='constant')
        out = np.mean([padded[dy:dy + alpha.shape[0], dx:dx + alpha.shape[1]]
                       for dy in range(2 * radius + 1) for dx in range(2 * radius + 1)], 0)
    return out


def with_halo(icon, strength):
    """The native icons' soft black halo about the silhouette, under it."""
    alpha = icon[:, :, 3]
    halo = np.clip(blur(alpha, 1) * strength, 0, 1)
    total = alpha + halo * (1 - alpha)
    rgb = icon[:, :, :3] * (alpha / np.maximum(total, 1e-6))[..., None]
    return np.dstack([rgb, total])


def framing(meshes, view, size, margin):
    """Scale and centre that fit every mesh in `size` less `margin` pixels."""
    pts = np.concatenate([m['p'] for m in meshes]) @ view[0]
    lo, hi = pts.min(0)[:2], pts.max(0)[:2]
    scale = min((size[0] - 2 * margin[0]) / (hi[0] - lo[0]), (size[1] - 2 * margin[1]) / (hi[1] - lo[1]))
    return scale, (lo + hi) / 2


def iou(a, b):
    a, b = a > 0.5, b > 0.5
    return (a & b).sum() / max((a | b).sum(), 1)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--open-model', type=Path, required=True, help="the native surface's converted_model")
    ap.add_argument('--closed-model', type=Path, required=True, help="the pack's surface converted_model")
    ap.add_argument('--texture', type=Path, required=True, help='edge_assault_05_helmet.dds (or a PNG of it)')
    ap.add_argument('--normal-map', type=Path, required=True, help='edge_assault_05_helmet_nmap.dds')
    ap.add_argument('--specular-map', type=Path, required=True, help='edge_assault_05_helmet_smap.dds')
    ap.add_argument('--lobby-swf', type=Path, required=True, help='the lobby movie players have now')
    ap.add_argument('--output', type=Path, required=True, help='the icon PNG (171x83, straight alpha)')
    ap.add_argument('--work', type=Path, required=True, help='folder for the renders and the comparison')
    a = ap.parse_args()
    a.work.mkdir(parents=True, exist_ok=True)
    reference = native_icon(a.lobby_swf)
    rgba = lambda path: np.asarray(Image.open(path).convert('RGBA'), float) / 255
    normals = rgba(a.normal_map)
    texture = (rgba(a.texture)[:, :, :3], np.dstack([normals[:, :, 3] * 2 - 1, normals[:, :, 1] * 2 - 1]),
               rgba(a.specular_map)[:, :, 0])
    open_mesh = load_mesh(a.open_model, texture)
    closed_mesh = load_mesh(a.closed_model, texture)

    # The native icon's camera: the best silhouette over yaw, pitch and
    # framing (the icon's own margins), searched coarse then fine.
    ref_alpha = reference[:, :, 3]
    ys, xs = np.where(ref_alpha > 0.5)
    margin = (min(xs.min(), ICON[0] - 1 - xs.max()), min(ys.min(), ICON[1] - 1 - ys.max()))
    shift = ((xs.min() + xs.max() + 1) / 2 - ICON[0] / 2, (ys.min() + ys.max() + 1) / 2 - ICON[1] / 2)

    def attempt(yaw, pitch):
        view = camera(yaw, pitch)
        scale, center = framing([open_mesh], view, ICON, (margin[0] - 0.5, margin[1] - 0.5))
        center = center - np.array([shift[0], -shift[1]]) / scale
        alpha = render(open_mesh, view, scale, center, ICON)[:, :, 3]
        return iou(alpha, ref_alpha), view, scale, center

    best = max((attempt(yaw, pitch) + (yaw, pitch) for yaw in range(20, 91, 10) for pitch in range(-10, 41, 10)),
               key=lambda r: r[0])
    for step in (5, 2, 1):
        yaw0, pitch0 = best[4], best[5]
        best = max([best] + [attempt(yaw, pitch) + (yaw, pitch)
                             for yaw in (yaw0 - step, yaw0, yaw0 + step)
                             for pitch in (pitch0 - step, pitch0, pitch0 + step)], key=lambda r: r[0])
    score, view, scale, center, yaw, pitch = best
    print(f'native camera: yaw {yaw}, pitch {pitch}, silhouette IoU {score:.3f}')
    # The closed visor reaches further down: keep both in the cell.
    fit, _ = framing([open_mesh, closed_mesh], view, ICON, (2, 2))
    scale = min(scale, fit)

    big = (ICON[0] * SUPER, ICON[1] * SUPER)
    open_icon = shrink(render(open_mesh, view, scale * SUPER, center, big), ICON)
    closed_icon = shrink(render(closed_mesh, view, scale * SUPER, center, big), ICON)

    # The colours. The native icons are graded warm and dark: the open
    # render's luminance is mapped onto the native icon's distribution
    # (histogram matching), coloured by the native icon's mean colour at that
    # luminance, and keeps part of its own colour about its grey (the
    # helmet's teal lining, its red panels). The closed render gets the same.
    luma = lambda rgb: rgb @ np.array([0.299, 0.587, 0.114])
    both = (open_icon[:, :, 3] > 0.95) & (ref_alpha > 0.95)
    target = reference[both, :3]
    quantiles = np.linspace(0, 1, 257)
    full_ref = reference[ref_alpha > 0.95, :3]
    curve = (np.quantile(luma(open_icon[open_icon[:, :, 3] > 0.95, :3]), quantiles),
             np.quantile(luma(full_ref), quantiles))
    bins = np.linspace(0, 1, 33)
    level = np.digitize(luma(full_ref), bins[1:-1])
    tint = np.array([full_ref[level == k].mean(0) / max(luma(full_ref[level == k].mean(0)), 1e-3)
                     if (level == k).sum() >= 4 else [np.nan] * 3 for k in range(32)])
    centres = (bins[:-1] + bins[1:]) / 2
    ok = ~np.isnan(tint[:, 0])
    tint = np.stack([np.interp(centres, centres[ok], tint[ok, c]) for c in range(3)], 1)

    def grade(icon, own=0.8):
        y = luma(icon[:, :, :3])
        matched = np.interp(y, *curve)
        hue = np.stack([np.interp(matched, centres, tint[:, c]) for c in range(3)], -1)
        own_colour = (icon[:, :, :3] - y[..., None]) * (matched / np.maximum(y, 1e-3))[..., None]
        # Only clearly coloured parts keep theirs: the greenish metal takes
        # the native warm tint.
        saturation = (icon[:, :, :3].max(-1) - icon[:, :, :3].min(-1)) / np.maximum(icon[:, :, :3].max(-1), 1e-3)
        keep = own * np.clip((saturation - 0.25) / 0.25, 0, 1)
        rgb = matched[..., None] * hue + keep[..., None] * own_colour
        return np.dstack([np.clip(rgb, 0, 1), icon[:, :, 3]])

    open_graded, closed_graded = grade(open_icon), grade(closed_icon)
    # The halo: as strong as the native icon's, by the count of its
    # part-covered pixels.
    semi = lambda alpha: ((alpha > 0.01) & (alpha < 0.9)).sum()
    strength = min(np.linspace(0.2, 3, 57), key=lambda k: abs(semi(with_halo(open_graded, k)[:, :, 3]) - semi(ref_alpha)))
    print(f'halo strength {strength:.2f}')
    open_graded, closed_graded = with_halo(open_graded, strength), with_halo(closed_graded, strength)
    error = np.abs(open_graded[both, :3] - target).mean() * 255
    print(f'colour fit: mean error {error:.1f}/255 over {both.sum()} pixels')

    def png(icon):
        return Image.fromarray((np.clip(icon, 0, 1) * 255 + 0.5).astype(np.uint8), 'RGBA')

    a.output.parent.mkdir(parents=True, exist_ok=True)
    png(closed_graded).save(a.output)
    png(open_graded).save(a.work / 'open-refit.png')
    sheet = Image.new('RGBA', (ICON[0] * 3 * 3 + 40, ICON[1] * 3 + 20), (52, 52, 52, 255))
    for i, icon in enumerate([reference, open_graded, closed_graded]):
        tile = png(icon).resize((ICON[0] * 3, ICON[1] * 3), Image.Resampling.NEAREST)
        sheet.alpha_composite(tile, (10 + i * (ICON[0] * 3 + 10), 10))
    sheet.save(a.work / 'icon-comparison.png')
    print('wrote', a.output)


if __name__ == '__main__':
    main()
