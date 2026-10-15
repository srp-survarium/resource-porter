#!/usr/bin/env python3
"""Score matched views separately in shadowed and sunlit regions; nothing is fitted.

    measure_shadow_specular.py --column original=REF --column source=A \
        --column source-spec=B --views POSES.json --output DIR [--region VIEW=x0,y0,x1,y1]

The first column is the original reference. The masks come only from it:
"shadow" is reference display luma in [0.01, 0.18), "sunlit" is >= 0.35, and
the middle band is ignored, so sky and highlights stay out of the shadow
score. A --region (fractions of the frame) restricts a view to one surface,
such as the reported metal wall. For every column and mask the report gives
the mean luma, its ratio to the reference, the mean positive luma excess
over the reference (an additive sheen shows here), and the blue-minus-red
shift, since the reported sheen is a cool, sky-coloured reflection. The arms
differ only in their probe-specular shader, so their per-pixel difference
isolates the native ambient specular. Grain, wind and simulation phase are
not synchronised and remain in every number.
"""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image

LUMA = np.array([.2126, .7152, .0722])
SHADOW = (.01, .18)
SUNLIT = .35


def load(directory, name):
    for suffix in ('.bmp', '.png'):
        path = directory/(name+suffix)
        if path.exists():
            image = np.asarray(Image.open(path).convert('RGB'), dtype=np.float64)/255
            if image.shape != (720, 1280, 3):
                raise ValueError(f'{path}: expected 1280x720')
            return image
    raise FileNotFoundError(directory/name)


def pose(directory, name):
    record = json.loads((directory/(name+'.json')).read_text())
    if record.get('status', {}).get('error'):
        raise RuntimeError(f'{directory}/{name}: capture error')
    return record.get('pose', record)


def stats(image, reference, mask):
    if not mask.any():
        return None
    luma, ref_luma = image@LUMA, reference@LUMA
    shift = (image[..., 2]-image[..., 0])-(reference[..., 2]-reference[..., 0])
    return {'pixels': int(mask.sum()), 'mean_luma': float(luma[mask].mean()),
            'luma_ratio_to_reference': float(luma[mask].mean()/max(ref_luma[mask].mean(), 1e-6)),
            'mean_positive_excess': float(np.maximum(luma-ref_luma, 0)[mask].mean()),
            'blue_minus_red_shift': float(shift[mask].mean()),
            'rgb_mae': float(np.abs(image-reference)[mask].mean())}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--column', action='append', required=True, help='LABEL=DIRECTORY; first is the reference')
    parser.add_argument('--views', type=Path, required=True)
    parser.add_argument('--only', action='append', default=[], help='restrict to these view names')
    parser.add_argument('--region', action='append', default=[], help='VIEW=x0,y0,x1,y1 as frame fractions')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    columns = [(label, Path(path)) for label, path in (c.split('=', 1) for c in args.column)]
    regions = {}
    for item in args.region:
        view, box = item.split('=', 1)
        regions[view] = [float(v) for v in box.split(',')]
    views = [p['name'] for p in json.loads(args.views.read_text())]
    views = [v for v in views if not args.only or v in args.only]
    report = {'scope': __doc__.strip(), 'columns': {l: str(p) for l, p in columns},
              'thresholds': {'shadow': SHADOW, 'sunlit': SUNLIT}, 'regions': regions, 'views': {}}
    for name in views:
        try:
            ref_pose = pose(columns[0][1], name)
            reference = load(columns[0][1], name)
        except FileNotFoundError:
            report['views'][name] = 'reference missing'
            continue
        ref_luma = reference@LUMA
        window = np.ones(ref_luma.shape, bool)
        if name in regions:
            x0, y0, x1, y1 = regions[name]
            window[:] = False
            window[int(y0*720):int(y1*720), int(x0*1280):int(x1*1280)] = True
        masks = {'shadow': window & (ref_luma >= SHADOW[0]) & (ref_luma < SHADOW[1]),
                 'sunlit': window & (ref_luma >= SUNLIT)}
        row = {}
        for label, directory in columns:
            current = pose(directory, name)
            for field in ('position', 'forward', 'fov'):
                if not np.allclose(current[field], ref_pose[field], atol=1e-6):
                    raise ValueError(f'{label}/{name}: camera {field} differs from reference')
            image = load(directory, name)
            row[label] = {m: stats(image, reference, mask) for m, mask in masks.items()}
        report['views'][name] = row
    summary = {}
    measured = [v for v, r in report['views'].items() if isinstance(r, dict)]
    for label, _ in columns[1:]:
        summary[label] = {}
        for m in ('shadow', 'sunlit'):
            rows = [report['views'][v][label][m] for v in measured if report['views'][v][label][m]]
            if rows:
                summary[label][m] = {k: float(np.median([r[k] for r in rows]))
                                     for k in ('luma_ratio_to_reference', 'mean_positive_excess',
                                               'blue_minus_red_shift', 'rgb_mae')} | {'views': len(rows)}
    report['summary_median'] = summary
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output/'shadow-specular.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
