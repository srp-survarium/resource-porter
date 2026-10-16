#!/usr/bin/env python3
"""Capture a JSON list of camera poses through an initialized game hook."""
import argparse
import json
import math
import time
from pathlib import Path


def capture(control, poses, output, timeout=30, size=(1280, 720)):
    output.mkdir(parents=True, exist_ok=True)
    status_path = control.with_name(control.name + '.status.json')
    results = []
    for index, pose in enumerate(poses):
        request = (int(time.time() * 1000) + index) % 0xffffffff or 1
        name = pose['name']
        if not name or Path(name).name != name or '\\' in name:
            raise ValueError('pose name must be a filename component')
        destination = (output / (name + '.bmp')).resolve()
        if destination.exists():
            raise FileExistsError(destination)
        values = pose['position'] + pose['forward'] + [pose.get('fov', 65), pose.get('aspect', size[0]/size[1]), pose.get('near', .05), pose.get('far', 500)]
        if len(values) != 10 or not all(math.isfinite(v) for v in values):
            raise ValueError('invalid camera values')
        temporary = control.with_suffix('.tmp')
        mode = pose.get('mode', 1 if pose.get('buffers', True) else 2)
        temporary.write_text(f'{request} {mode} ' + ' '.join(map(str, values)) + f" {pose.get('settle_ms', 3000)}\nZ:" + str(destination).replace('/', '\\') + '\n')
        temporary.replace(control)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            status = json.loads(status_path.read_text())
            if status['error']:
                raise RuntimeError(status['error'])
            if status['captured_id'] == request:
                if not destination.is_file() or destination.stat().st_size < 54:
                    raise RuntimeError('hook acknowledged a missing image')
                for actual, expected in zip(status['position'], pose['position']):
                    if mode != 3 and abs(actual - expected) > .001:
                        raise RuntimeError('captured camera position differs from request')
                if mode != 3 and abs(status['fov'] - pose.get('fov', 65)) > .001:
                    raise RuntimeError('captured field of view differs from request')
                if (status['width'], status['height']) != tuple(size):
                    raise RuntimeError(f'expected a {size[0]}x{size[1]} backbuffer; run viewport.exe')
                record = {'pose': pose, 'status': status, 'image': str(destination)}
                (output / (name + '.json')).write_text(json.dumps(record, indent=2) + '\n')
                results.append(record)
                print(f"{name}: {status['width']}x{status['height']}", flush=True)
                break
            time.sleep(.1)
        else:
            raise TimeoutError(f'no capture acknowledgment for {name}')
    return results

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--control', type=Path, required=True)
    parser.add_argument('--poses', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=30,
                        help='seconds allowed per pose, including GPU buffer readback')
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('timeout must be positive and finite')
    capture(args.control, json.loads(args.poses.read_text()), args.output, args.timeout)
