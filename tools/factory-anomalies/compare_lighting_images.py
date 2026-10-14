#!/usr/bin/env python3
"""Compare matched static source/baseline/candidate frames without hiding residuals."""
import argparse
import json
from pathlib import Path
import re
import statistics
import subprocess


def compare(source, target, magick, excluded):
    poses = json.loads((source/'poses.json').read_text())
    results = []
    for pose in poses:
        name = pose['name']
        reference = source/(name+'.png')
        row = {'name': name, 'camera_visually_matched': name not in excluded}
        for arm in ('baseline', 'candidate'):
            folder = target/arm
            captured = json.loads((folder/(name+'.json')).read_text())
            if captured['pose'] != pose:
                raise ValueError('pose changed: '+arm+'/'+name)
            bmp, png = folder/(name+'.bmp'), folder/(name+'.png')
            if not png.exists():
                subprocess.run([magick, str(bmp), str(png)], check=True)
            result = subprocess.run([magick, 'compare', '-metric', 'MAE',
                                     str(reference), str(png), 'null:'],
                                    capture_output=True, text=True)
            if result.returncode not in (0, 1):
                raise RuntimeError(result.stderr)
            metric = re.fullmatch(r'\s*[0-9.eE+-]+ \(([0-9.eE+-]+)\)\s*', result.stderr)
            if not metric:
                raise ValueError('unexpected ImageMagick MAE output: '+result.stderr)
            row[arm] = float(metric[1])
        row['improved'] = row['candidate'] < row['baseline']
        results.append(row)
    valid = [r for r in results if r['camera_visually_matched']]
    if not valid or set(excluded)-{p['name'] for p in poses}:
        raise ValueError('exclusions leave no comparison or name an unknown pose')
    report = {'scope': 'sRGB full-frame MAE at identical cameras; static disconnected original reference. '
                       'Texture quality, foliage/particle phase and exposure history are not synchronized. '
                       'Inspect all residuals; a lower mean is not proof of complete parity.',
              'poses': results, 'invalid_baseline_views': sorted(excluded),
              'improved': sum(r['improved'] for r in valid),
              'count': len(valid),
              'mean': {a: statistics.mean(r[a] for r in valid) for a in ('baseline', 'candidate')},
              'median': {a: statistics.median(r[a] for r in valid) for a in ('baseline', 'candidate')}}
    (target/'image-comparison.json').write_text(json.dumps(report, indent=2)+'\n')
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('source', type=Path)
    p.add_argument('target', type=Path)
    p.add_argument('--magick', required=True)
    p.add_argument('--exclude-view', action='append', default=[],
                   help='pose rejected by visual camera validation; excluded from aggregates')
    a = p.parse_args()
    print(json.dumps(compare(a.source, a.target, a.magick, set(a.exclude_view)), indent=2))
