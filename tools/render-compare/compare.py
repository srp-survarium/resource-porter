#!/usr/bin/env python3
"""Compare one matching camera pair without changing either captured image.

Pixel differences include animated vegetation, particles, clouds and noise;
they are diagnostics, not an automatic visual-parity acceptance threshold.
Requires Pillow. Inputs are capture.py's per-image JSON records.
"""
import argparse
import hashlib
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageStat


def read_record(path):
    record = json.loads(path.read_text())
    image_path = path.with_suffix('.bmp')
    digest = hashlib.sha256(image_path.read_bytes()).hexdigest()
    if record.get('sha256') != digest:
        raise ValueError(f'image hash mismatch: {image_path}')
    return record, Image.open(image_path).convert('RGB')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reference', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--regions', type=Path, help='JSON object of name: [left, top, right, bottom]')
    args = parser.parse_args()
    original, a = read_record(args.reference)
    current, b = read_record(args.candidate)
    for field in ('position', 'forward', 'fov', 'aspect', 'near', 'far'):
        if original['pose'].get(field) != current['pose'].get(field):
            raise ValueError(f'camera mismatch: {field}')
    if a.size != b.size:
        raise ValueError('image dimensions differ')
    regions = {'whole_image': [0, 0, *a.size]}
    if args.regions:
        regions.update(json.loads(args.regions.read_text()))
    report = {'reference': str(args.reference.resolve()), 'candidate': str(args.candidate.resolve()),
              'pose': original['pose'], 'regions': {},
              'limitation': 'Animated content is not synchronized; these are raw display-RGB diagnostics.'}
    for name, rect in regions.items():
        x0, y0, x1, y1 = rect
        if not (0 <= x0 < x1 <= a.width and 0 <= y0 < y1 <= a.height):
            raise ValueError(f'invalid rectangle: {name}')
        aa, bb = a.crop(rect), b.crop(rect)
        error = ImageStat.Stat(ImageChops.difference(aa, bb)).mean
        report['regions'][name] = {'rectangle': rect,
            'reference_mean_rgb': ImageStat.Stat(aa).mean,
            'candidate_mean_rgb': ImageStat.Stat(bb).mean,
            'mean_absolute_rgb_error': error, 'mean_absolute_error': sum(error)/3}
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output/'comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    pair = Image.new('RGB', (a.width*2, a.height))
    pair.paste(a, (0, 0)); pair.paste(b, (a.width, 0))
    pair.save(args.output/'original-left-candidate-right.png')
    ImageChops.difference(a, b).save(args.output/'absolute-difference.png')
    print(json.dumps(report['regions'], indent=2))


if __name__ == '__main__':
    main()
