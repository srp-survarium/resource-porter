#!/usr/bin/env python3
"""Compare captured release arms with frozen original views; no gain is fitted.

Each column is a directory holding NAME.bmp|png and NAME.json capture records.
The first column is the original reference. Cameras must match the reference's
recorded position, forward vector and FOV. Metrics are full-frame mean absolute
display-RGB errors (0..1) plus mean luma; unsynchronized grain, wind and
simulation phases remain in every number. A contact sheet per view shows the
columns side by side at half size.
"""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

LUMA = np.array([.2126, .7152, .0722])


def load(directory, name):
    for suffix in ('.bmp', '.png'):
        path = directory/(name+suffix)
        if path.exists():
            return path, np.asarray(Image.open(path).convert('RGB'), dtype=np.float64)/255
    raise FileNotFoundError(directory/name)


def camera(directory, name):
    record = json.loads((directory/(name+'.json')).read_text())
    pose = record.get('pose', record)
    status = record.get('status', {})
    if status.get('error'):
        raise RuntimeError(f'{directory}/{name}: capture error {status["error"]}')
    return pose


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--column', action='append', required=True, help='LABEL=DIRECTORY; first is the reference')
    parser.add_argument('--views', type=Path, required=True, help='pose JSON naming the compared views')
    parser.add_argument('--exclude', action='append', default=[], help='view omitted from aggregates')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    columns = [(label, Path(path)) for label, path in (c.split('=', 1) for c in args.column)]
    views = [p['name'] for p in json.loads(args.views.read_text())]
    args.output.mkdir(parents=True, exist_ok=False)
    report = {'scope': __doc__.strip(), 'columns': {l: str(p) for l, p in columns}, 'views': {}}
    reference_label = columns[0][0]
    for name in views:
        ref_pose = camera(columns[0][1], name)
        images, row = [], {}
        for label, directory in columns:
            pose = camera(directory, name)
            for field in ('position', 'forward', 'fov'):
                if not np.allclose(pose[field], ref_pose[field], atol=1e-6):
                    raise ValueError(f'{label}/{name}: camera {field} differs from reference')
            path, image = load(directory, name)
            if image.shape != (720, 1280, 3):
                raise ValueError(f'{path}: expected 1280x720')
            images.append((label, image))
            row[label] = {'mean_luma': float((image@LUMA).mean())}
        reference = images[0][1]
        for label, image in images[1:]:
            row[label]['rgb_mae'] = float(np.abs(image-reference).mean())
        report['views'][name] = row
        sheet = Image.new('RGB', (640*len(images), 380))
        draw = ImageDraw.Draw(sheet)
        for index, (label, image) in enumerate(images):
            tile = Image.fromarray((image*255+.5).astype(np.uint8)).resize((640, 360))
            sheet.paste(tile, (640*index, 20))
            text = label if index == 0 else f"{label}  mae {row[label]['rgb_mae']:.4f}"
            draw.text((640*index+6, 4), text, fill='white')
        sheet.save(args.output/(name+'.png'))
    included = [v for v in views if v not in args.exclude]
    summary = {}
    for label, _ in columns[1:]:
        values = [report['views'][v][label]['rgb_mae'] for v in included]
        summary[label] = {'views': len(values), 'mean_rgb_mae': float(np.mean(values)),
                          'median_rgb_mae': float(np.median(values))}
    labels = [l for l, _ in columns[1:]]
    if len(labels) > 1:
        first = labels[0]
        for label in labels[1:]:
            better = [v for v in included if report['views'][v][label]['rgb_mae'] < report['views'][v][first]['rgb_mae']]
            summary[label]['views_better_than_'+first] = len(better)
    report['summary'] = summary
    report['excluded'] = args.exclude
    report['reference'] = reference_label
    (args.output/'metrics.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
