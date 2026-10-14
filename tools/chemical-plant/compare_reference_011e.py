#!/usr/bin/env python3
"""Freeze hashes and compare the 29 matched, UI-free 011e/026 screenshots.

Uses unmodified display RGB. Metrics are descriptive single-frame differences;
weather, grain, vegetation and simulation phases are not synchronized.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('panoramas', Path(__file__).with_name('panoramas.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    output = args.root/'comparison'
    output.mkdir(exist_ok=False)
    report = {'sampling': 'full-frame RGB grid, stride 4; normalized display values',
              'limitations': 'Unsynchronized grain, wind and simulation phases; no lighting change applied.',
              'pair_order': ['original011e', 'candidate026'], 'maps': {}}
    hashes = {}
    for name in ['chemical', 'radar']:
        poses = json.loads((args.root/(name+'-poses.json')).read_text())
        results = []
        contacts = [bytearray(1280*((len(poses)+3)//4)*180*3) for _ in range(2)]
        for index, pose in enumerate(poses):
            files = [args.root/(phase+'-'+name)/'poses'/(pose['name']+'.bmp')
                     for phase in ['capture', 'target']]
            images = [module.load_bmp(path) for path in files]
            assert all(image[:2] == (1280, 720) for image in images)
            metadata = [json.loads(path.with_suffix('.json').read_text()) for path in files]
            # The capture helper saves the requested camera alongside runtime state.
            assert metadata[0]['pose'] == metadata[1]['pose'], pose['name']
            total_error = 0
            sums = [0, 0]
            samples = 0
            for y in range(180):
                for x in range(320):
                    pixels = [image[2](x*4, y*4) for image in images]
                    at = (((index//4)*180+y)*1280+(index%4)*320+x)*3
                    for side in range(2):
                        contacts[side][at:at+3] = bytes(pixels[side])
                        sums[side] += sum(c*w for c, w in zip(pixels[side], [.2126, .7152, .0722]))
                    total_error += sum(abs(a-b) for a,b in zip(*pixels))
                    samples += 1
            row = {'name': pose['name'], 'rgb_mae': total_error/(samples*3*255),
                   'source_luma': sums[0]/(samples*255), 'target_luma': sums[1]/(samples*255)}
            if 'skeleton' in pose['name'] or 'lamp' in pose['name']:
                paired = bytearray()
                for y in range(720):
                    for image in images:
                        for x in range(1280):
                            paired.extend(image[2](x, y))
                pair_path = output/(pose['name']+'.png')
                module.buffers.png(pair_path, 2560, 720, paired)
                row['pair'] = str(pair_path.relative_to(args.root))
            results.append(row)
        for side, phase in enumerate(['capture', 'target']):
            module.buffers.png(output/(name+'-'+phase+'-contact.png'), 1280,
                               ((len(poses)+3)//4)*180, contacts[side])
            for path in sorted((args.root/(phase+'-'+name)).rglob('*')):
                if path.is_file():
                    hashes[str(path.relative_to(args.root))] = hashlib.sha256(path.read_bytes()).hexdigest()
        report['maps'][name] = results
    (output/'metrics.json').write_text(json.dumps(report, indent=2)+'\n')
    (output/'capture-sha256.json').write_text(json.dumps(hashes, indent=2)+'\n')
    print(json.dumps({name: {'views': len(rows), 'mean_rgb_mae': sum(r['rgb_mae'] for r in rows)/len(rows)}
                      for name, rows in report['maps'].items()}, indent=2))


if __name__ == '__main__':
    main()
