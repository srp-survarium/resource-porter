#!/usr/bin/env python3
"""Compare three same-frame HDR composition recipes to frozen source output."""
import argparse
import json
import math
from pathlib import Path
import struct

from audit_surface_light import REGIONS
from compare_composition import unsigned_float

FLOAT11 = [unsigned_float(i, 6) for i in range(2048)]
FLOAT10 = [unsigned_float(i, 5) for i in range(1024)]
RECIPES = ('displayed_approximation', 'native_same_inputs', 'full_encoded_diffuse')
MASKS = ('fixed_mask_geometry', 'exact_depth', 'exact_depth_3x3')


def unpack(value):
    return (FLOAT11[value & 2047], FLOAT11[(value >> 11) & 2047], FLOAT10[value >> 22])


def empty():
    return {'samples': 0, 'source_sum': [0., 0., 0.], 'target_sum': [0., 0., 0.],
            'absolute': 0., 'squared': 0., 'mapped_absolute': 0., 'pixel_errors': []}


def accumulate(stats, source, target, exposure):
    errors = [abs(a-b) for a, b in zip(source, target)]
    stats['samples'] += 1
    stats['absolute'] += sum(errors)
    stats['squared'] += sum(e*e for e in errors)
    stats['pixel_errors'].append(sum(errors)/3)
    for i, (a, b) in enumerate(zip(source, target)):
        stats['source_sum'][i] += a
        stats['target_sum'][i] += b
        mapped = lambda x: (exposure*x/(1+exposure*x))**(1/2.2)
        stats['mapped_absolute'] += abs(mapped(a)-mapped(b))


def finish(stats):
    n = stats['samples']
    errors = sorted(stats['pixel_errors'])
    return {'samples': n, 'fewer_than_2000_samples': n < 2000,
            'source_hdr_rgb_mean': [v/n for v in stats['source_sum']] if n else None,
            'target_hdr_rgb_mean': [v/n for v in stats['target_sum']] if n else None,
            'hdr_rgb_mae': stats['absolute']/(3*n) if n else None,
            'hdr_rgb_rmse': math.sqrt(stats['squared']/(3*n)) if n else None,
            'p95_pixel_hdr_rgb_mae': errors[int(.95*(n-1))] if n else None,
            'common_exposure_rgb_mae': stats['mapped_absolute']/(3*n) if n else None}


def audit(index, original, stride, exposure):
    stem = str(index).removesuffix('.composition-output.targets.json')
    old = original/Path(stem).name
    def read(base, suffix):
        return Path(str(base)+suffix).read_bytes()
    views = json.loads(index.read_text())
    expected = ('override', 'native_same_inputs', 'comparison_source_diffuse', 'restored_override')
    if tuple(v['name'] for v in views) != expected:
        raise ValueError(f'{index}: incomplete diagnostic output')
    old_view = json.loads(read(old, '.composition-output.targets.json'))[0]
    for view in views+[old_view]:
        if (view['format'], view['width'], view['height'], view['written']) != (26, 1280, 720, True):
            raise ValueError('composition output contract changed')
    source = read(old, '.composition-output.rt00.bin')
    targets = [read(stem, f'.composition-output.rt{i:02d}.bin') for i in range(4)]
    if any(len(b) != 1280*720*4 for b in [source]+targets):
        raise ValueError('incomplete composition output')
    restored_equal = targets[0] == targets[3]
    nonfinite = {}
    for name, data in zip(('original',)+RECIPES+('restored',), [source]+targets):
        nonfinite[name] = sum(any(not math.isfinite(v) for v in unpack(word))
                              for (word,) in struct.iter_unpack('<I', data))
    if any(nonfinite.values()):
        raise ValueError(f'nonfinite composition pixels: {nonfinite}')
    od = read(old, '.composition-draw.rt00.bin')
    nd = read(stem, '.composition-draw.rt03.bin')
    if len(od) != 1280*720*2 or len(nd) != len(od):
        raise ValueError('depth dimensions differ')
    depths = [tuple(v[0] for v in struct.iter_unpack('<e', b)) for b in (od, nd)]
    ds_view = json.loads(read(stem, '.sun-encoded-0.targets.json'))[1]
    ds = read(stem, '.sun-encoded-0.rt01.bin')
    if not ds_view['written'] or (ds_view['width'], ds_view['height']) != (1280, 720):
        raise ValueError('missing native geometry stencil')
    if ds_view['format'] in (44, 45):
        stencil = ds[3::4]
    elif ds_view['format'] in (19, 20):
        stencil = ds[4::8]
    else:
        raise ValueError('unknown native geometry stencil format')
    if len(stencil) != 1280*720:
        raise ValueError('incomplete native geometry stencil')
    # Frozen Factory references share this finite far-clear value. Equal sky
    # depths are not matching geometry: source composition precedes sky while
    # native composition preserves an already drawn sky.
    clear_depths = sorted({z for z, s in zip(depths[1], stencil) if not s})
    if clear_depths != [499.75]:
        raise ValueError(f'far-clear contract changed: {clear_depths}')
    geometry = bytearray(bool(s) and all(0 < d[i] < 499.75 for d in depths)
                         for i, s in enumerate(stencil))
    matched = bytearray(geometry[i] and a == b for i, (a, b) in enumerate(zip(*depths)))
    totals = {mask: {recipe: empty() for recipe in RECIPES} for mask in MASKS}
    rects = REGIONS.get(Path(stem).stem, {})
    regions = {region: {recipe: empty() for recipe in RECIPES} for region in rects}
    for y in range(1, 719, stride):
        for x in range(1, 1279, stride):
            i = y*1280+x
            if not geometry[i]:
                continue
            if y >= 544 or (x < 224 and y < 224) or (x >= 640 and y >= 384) or (608 <= x < 672 and 320 <= y < 384):
                continue
            masks = ['fixed_mask_geometry']
            if matched[i]:
                masks.append('exact_depth')
            if all(matched[i+dy*1280+dx] for dy in (-1, 0, 1) for dx in (-1, 0, 1)):
                masks.append('exact_depth_3x3')
            reference = unpack(struct.unpack_from('<I', source, i*4)[0])
            for recipe, data in zip(RECIPES, targets):
                target = unpack(struct.unpack_from('<I', data, i*4)[0])
                for mask in masks:
                    accumulate(totals[mask][recipe], reference, target, exposure)
                if matched[i]:
                    for name, (x0, y0, x1, y1) in rects.items():
                        if x0 <= x < x1 and y0 <= y < y1:
                            accumulate(regions[name][recipe], reference, target, exposure)
    return {'pose': Path(stem).name, 'displayed_restored_byte_equal': restored_equal,
            'geometry_gate': {'native_stencil_nonzero_pixels': sum(bool(s) for s in stencil),
                              'far_clear_depth': clear_depths[0],
                              'both_depths_geometry_pixels': sum(geometry)},
            'full_frame_nonfinite_pixels': nonfinite,
            'masks': {mask: {r: finish(s) for r, s in recipes.items()} for mask, recipes in totals.items()},
            'predefined_regions_exact_depth': {name: {'rectangle': rects[name],
                'recipes': {r: finish(s) for r, s in recipes.items()}} for name, recipes in regions.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', type=Path)
    parser.add_argument('--original', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--stride', type=int, default=4)
    parser.add_argument('--exposure', type=float, default=1.)
    args = parser.parse_args()
    if args.stride < 1 or not math.isfinite(args.exposure) or args.exposure <= 0:
        parser.error('stride and exposure must be positive and finite')
    files = sorted(args.captures.glob('*.bmp.composition-output.targets.json'))
    if not files:
        parser.error('no completed composition comparisons')
    rows = [audit(path, args.original, args.stride, args.exposure) for path in files]
    result = {'scope': 'Pre-fog HDR composition, gated by actual native nonzero stencil and both depths below verified 499.75 far-clear. Only diffuse accumulation changes between displayed and full recipes; native albedo/specular stay fixed. Original geometry/material/shadow equivalence is not implied by quantized depth equality. Fixed conservative HUD/weapon mask is used even though buffers precede UI.',
              'captures': str(args.captures.resolve()), 'original': str(args.original.resolve()),
              'grid_stride': args.stride, 'common_exposure': args.exposure,
              'common_exposure_mapping': '(exposure*x/(1+exposure*x))^(1/2.2), shared across all recipes; diagnostic metric, not game tone mapping',
              'views': rows}
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'report': str(args.output), 'views': len(rows),
                      'displayed_restored_byte_equal': all(row['displayed_restored_byte_equal'] for row in rows)}))
    if not all(row['displayed_restored_byte_equal'] for row in rows):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
