#!/usr/bin/env python3
"""Project frozen, calibrated eight-heading captures to a 360-degree panorama.

Nearest-heading/nearest-pixel sampling preserves display RGB; no exposure, seam
blending, or tone adjustment is applied. Animated objects can differ at seams.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import struct

spec = importlib.util.spec_from_file_location('buffers', Path(__file__).parents[1]/'render-compare/inspect_buffers.py')
buffers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(buffers)


def load_bmp(path):
    raw = path.read_bytes()
    offset = struct.unpack_from('<I', raw, 10)[0]
    width, height = struct.unpack_from('<ii', raw, 18)
    assert raw[:2] == b'BM' and struct.unpack_from('<H', raw, 28)[0] == 32
    def pixel(x, y):
        row = y if height < 0 else height-y-1
        at = offset+4*(row*width+x)
        return raw[at+2], raw[at+1], raw[at]
    return width, abs(height), pixel


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', type=Path)
    parser.add_argument('--poses', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--width', type=int, default=2048)
    parser.add_argument('--height', type=int, default=384)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    poses = json.loads(args.poses.read_text())
    stations = {}
    for pose in poses:
        if 'station' in pose:
            stations.setdefault(pose['station'], []).append(pose)
    manifest = {'projection': 'equirectangular', 'longitude_degrees': [0, 360],
                'latitude_degrees': [30, -30], 'sampling': 'nearest heading, nearest pixel; unmodified display RGB',
                'width': args.width, 'height': args.height, 'stations': []}
    for station, views in stations.items():
        views.sort(key=lambda p:p['heading_degrees'])
        assert [p['heading_degrees'] for p in views] == list(range(0, 360, 45))
        images = [load_bmp(args.captures/(p['name']+'.bmp')) for p in views]
        output = bytearray()
        for y in range(args.height):
            pitch = math.radians(30-60*(y+.5)/args.height)
            for x in range(args.width):
                yaw = 360*(x+.5)/args.width
                index = int((yaw+22.5)//45)%8
                delta = math.radians((yaw-views[index]['heading_degrees']+180)%360-180)
                width, height, pixel = images[index]
                scale = math.tan(math.radians(views[index]['fov']/2))
                u = math.tan(delta)/(scale*views[index]['aspect'])
                v = math.tan(pitch)/(math.cos(delta)*scale)
                sx = round((u+1)*width/2-.5)
                sy = round((1-v)*height/2-.5)
                assert 0 <= sx < width and 0 <= sy < height, (station, x, y, sx, sy)
                output.extend(pixel(sx, sy))
        buffers.png(args.output/(station+'.png'), args.width, args.height, output)
        manifest['stations'].append({'name': station, 'position': views[0]['position'],
                                    'split': views[0]['split'], 'frames': [p['name']+'.bmp' for p in views]})
        print(station, flush=True)
    (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')


if __name__ == '__main__':
    main()
