#!/usr/bin/env python3
"""The gloss of every ported material of a built bundle: none is a mirror.

    gloss_check.py --bundle BUNDLES026/PACK --vostok-port VP --db TARGET026 [--max 0.3]

In 0.26g a material's "roughness" is its gloss: with `use_troughness` the
g-buffer pass writes 1 - (constant_roughness_min + (max - min) x the map's
green) and the composition takes that as the sun's GGX roughness, so a gloss
near 1 is a mirror that turns a whole surface the sun's or the sky's colour.
Without a map the gloss is `constant_roughness`. For each material under
material_instances/ported/ the mean gloss over its map (the top mip, linear
as stored) is computed, the map read from the bundle's overlay, else from DB.

A material whose gloss map is the glass template's (`tiled/window/glass`: an
X-Ray translucent surface, the `translucent` family of the recipes) keeps
glass's gloss and is listed for review: windows, and the metal fences, grates,
barbed wire, beds and wagons X-Ray draws translucent. Any other material above
--max stops it (exit 1): a mirror-like surface the port made. Stock 0.26g's own
leaves (flora/poplar_leaves_02, its colour as gloss map over 0.1..1) have about
0.34; the ported leaves, that template's map over 0..1, 0.26. It prints every
material over 0.1 and a count by range.
"""
import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image

GLASS = 'ported/v010b/tiled/window/glass'


def leaves(vp, path):
    out = subprocess.run([vp, 'dump-config', '--leaves', '--file', str(path)], check=True,
                         capture_output=True, text=True).stdout
    values = {}
    for line in out.splitlines():
        key, _, value = line.partition('\t')
        if key.startswith('material.g_stage.effect.') and key.endswith('.value'):
            name = key[len('material.g_stage.effect.'):-len('.value')]
            kind, _, raw = value.partition(':')
            values[name] = raw.strip('"') if kind == 'string' else raw
    return values


def number(v):
    return float(v) if v not in (None, '') else None


def green_mean(texture, overlay, vp, db, cache, tmp):
    if texture in cache:
        return cache[texture]
    path = overlay / 'textures' / f'{texture}.dds'
    if not path.is_file():
        path = Path(tmp) / 'texture.dds'
        r = subprocess.run([vp, 'extract', '--db', str(db), '--path', f'textures/{texture}.dds', '--output', str(path)],
                           capture_output=True)
        if r.returncode != 0:
            cache[texture] = None
            return None
    with Image.open(path) as im:
        cache[texture] = float(np.asarray(im.convert('RGBA'), dtype=np.float32)[..., 1].mean() / 255)
    return cache[texture]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--bundle', type=Path, required=True)
    ap.add_argument('--vostok-port', required=True)
    ap.add_argument('--db', type=Path, required=True)
    ap.add_argument('--max', type=float, default=0.3)
    a = ap.parse_args()
    overlay = a.bundle / 'overlay'
    cache, rows = {}, []
    with tempfile.TemporaryDirectory() as tmp:
        for path in sorted((overlay / 'material_instances/ported').rglob('*.material')):
            e = leaves(a.vostok_port, path)
            name = str(path.relative_to(overlay / 'material_instances'))[:-len('.material')]
            textured = e.get('use_troughness') == 'true' and e.get('texture_roughness')
            if textured:
                g = green_mean(e['texture_roughness'], overlay, a.vostok_port, a.db, cache, tmp)
                if g is None:
                    print(f'{name}: gloss map {e["texture_roughness"]} not found', file=sys.stderr)
                    sys.exit(1)
                lo, hi = number(e.get('constant_roughness_min')) or 0.0, number(e.get('constant_roughness_max'))
                hi = 1.0 if hi is None else hi
                gloss, how = lo + (hi - lo) * g, f'{lo:g}..{hi:g} x {e["texture_roughness"]} ({g:.3f})'
            else:
                gloss = number(e.get('constant_roughness'))
                if gloss is None:
                    continue
                how = 'constant'
            rows.append((gloss, name, how, e.get('texture_diffuse', ''), bool(textured) and e['texture_roughness'] == GLASS))
    over = [r for r in rows if r[0] > a.max and not r[4]]
    for gloss, name, how, diffuse, glossy in sorted(rows, reverse=True):
        if gloss > 0.1:
            print(f'{gloss:.3f}  {name}  {how}  colour {diffuse}' + ('  (glass template: review)' if glossy else '')
                  + ('  OVER' if (gloss > a.max and not glossy) else ''))
    bands = [(0, 0.1), (0.1, 0.3), (0.3, 0.6), (0.6, 1.01)]
    print(f'{len(rows)} ported materials; gloss ' + ', '.join(
        f'{lo:g}-{min(hi, 1):g}: {sum(lo <= r[0] < hi for r in rows)}' for lo, hi in bands)
        + f'; {sum(r[4] for r in rows)} of the glass template; {len(over)} others over {a.max:g}')
    sys.exit(1 if over else 0)


if __name__ == '__main__':
    main()
