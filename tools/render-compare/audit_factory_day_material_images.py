#!/usr/bin/env python3
"""Compare three fixed material views without treating full-frame lighting as a material metric."""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image


# Rectangles selected from the baseline before viewing the candidate.
REGIONS = {
    'pump-east': {'motor': [730, 338, 875, 410], 'tank_control': [140, 330, 275, 425],
                  'wall_control': [475, 160, 600, 225], 'crate_control': [750, 570, 850, 615]},
    'skeleton-central-path': {'remains': [530, 183, 785, 454],
                              'ground_control': [850, 330, 1000, 470]},
    'skeleton-lower-path': {'remains': [505, 195, 800, 425],
                            'ground_control': [825, 450, 985, 580]},
}


def read(directory, name):
    path = directory / (name + '.bmp')
    if not path.exists():
        path = directory / (name + '.png')
    image = Image.open(path).convert('RGB')
    assert image.size == (1280, 720)
    pose = json.loads((directory / (name + '.json')).read_text())['pose']
    return image, np.asarray(image, dtype=float) / 255, pose


def audit(args):
    args.output.mkdir(parents=True, exist_ok=True)
    result = {}
    for name, regions in REGIONS.items():
        bi, baseline, bp = read(args.baseline, name)
        ci, candidate, cp = read(args.candidate, name)
        for field in ('position', 'forward', 'fov'):
            assert np.allclose(bp[field], cp[field], atol=1e-6), (name, field)
        source = None
        if args.source and any((args.source / (name + suffix)).exists() for suffix in ('.bmp', '.png')):
            _, source, sp = read(args.source, name)
            for field in ('position', 'forward', 'fov'):
                assert np.allclose(bp[field], sp[field], atol=1e-6), (name, field)
        result[name] = {}
        for label, (x0, y0, x1, y1) in regions.items():
            before, after = baseline[y0:y1, x0:x1], candidate[y0:y1, x0:x1]
            bl, cl = (x @ np.array([.2126, .7152, .0722]) for x in (before, after))
            row = {'rect': [x0, y0, x1, y1], 'pixels': int(bl.size),
                   'mean_luma': [float(bl.mean()), float(cl.mean())],
                   'p95_luma': [float(np.quantile(bl, .95)), float(np.quantile(cl, .95))],
                   'rgb_mae': float(abs(before - after).mean())}
            if source is not None:
                original = source[y0:y1, x0:x1]
                row['source_rgb_mae'] = [float(abs(x - original).mean()) for x in (before, after)]
            result[name][label] = row
        panel = Image.new('RGB', (1280, 1440))
        panel.paste(bi, (0, 0))
        panel.paste(ci, (0, 720))
        panel.save(args.output / (name + '-before-after.png'))
    report = {'note': 'Screenshot-space fixed rectangles include post-processing, animated shadows '
                      'and some background. Source geometry/projection phase is not asserted identical; '
                      'these metrics support visual A/B, not shader-equation parity.', 'results': result}
    (args.output / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    audit(parser.parse_args())
