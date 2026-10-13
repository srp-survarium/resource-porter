#!/usr/bin/env python3
"""Capture actual player views before/after bounded WASD input in an owned client."""
import argparse
import json
import math
from pathlib import Path
import subprocess
import time


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--control', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--wine', required=True)
    parser.add_argument('--exe', required=True, help='Exact absolute Windows client executable path')
    parser.add_argument('--helper', type=Path, default=Path(__file__).with_name('walk_client.exe'))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    status_path = Path(str(args.control)+'.status.json')
    rain_path = Path(str(args.control)+'.source-rain-live.json')
    rows = []
    report = {'scope': 'Actual character camera; screenshot-only requests; bounded WASD input.',
              'samples': rows, 'success': False}

    def capture(name):
        request = int(time.time()*1000) % 0xffffffff
        destination = args.output.resolve()/(name+'.bmp')
        temporary = Path(str(args.control)+'.tmp')
        temporary.write_text(f'{request} 3 0 0 0 0 0 1 65 1.7777777778 .05 500 0\n'
                             +'Z:'+str(destination).replace('/', '\\')+'\n')
        temporary.replace(args.control)
        deadline = time.monotonic()+20
        while time.monotonic()<deadline:
            status = read_json(status_path)
            if status and status['error']:
                raise RuntimeError(status['error'])
            if status and status['captured_id']==request:
                row = {'name': name, 'status': status, 'rain': read_json(rain_path),
                       'monotonic_time': time.monotonic()}
                rows.append(row)
                destination.with_suffix('.json').write_text(json.dumps(row, indent=2))
                return row
            time.sleep(.1)
        raise RuntimeError('gameplay screenshot timeout')

    try:
        capture('start')
        # The live status is intentionally throttled to two seconds.
        time.sleep(2.2)
        capture('standing')
        for index, key in enumerate('wsad'):
            result = subprocess.run([args.wine, str(args.helper.resolve()), args.exe, key, '2500'],
                                    text=True, capture_output=True, timeout=12)
            report.setdefault('inputs', []).append({'key': key, 'returncode': result.returncode,
                                                    'stdout': result.stdout, 'stderr': result.stderr})
            if result.returncode:
                raise RuntimeError('owned-client input failed: '+result.stderr)
            capture(f'{index+1}-{key}')
        time.sleep(2.2)
        capture('settled')
        samples = [r['rain'] for r in rows[1:] if r['rain']]
        assert len(samples)==len(rows)-1, 'missing live rain telemetry'
        assert all(not r['director_enabled'] for r in samples), 'director override active'
        assert all(r['rejected_draws']==samples[0]['rejected_draws'] for r in samples), 'rain rejected moving camera'
        assert all(r['rendered_frames']==r['applied_frames']==r['suppressed_native_draws'] for r in samples), 'rain application counts differ'
        assert samples[-1]['applied_frames']>samples[0]['applied_frames'], 'rain stalled'
        positions = [r['status']['position'] for r in rows]
        distances = [math.dist(positions[0], p) for p in positions]
        report['max_displacement'] = max(distances)
        assert max(distances)>.5, 'input did not move the actual character'
        report['rain_frames'] = samples[-1]['applied_frames']-samples[0]['applied_frames']
        report['rain_scene_seconds'] = samples[-1]['gpu_scene_time']-samples[0]['gpu_scene_time']
        assert report['rain_scene_seconds']>0, 'rain scene clock stalled'
        report['rain_frames_per_scene_second'] = report['rain_frames']/report['rain_scene_seconds']
        report['success'] = True
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        (args.output/'movement.json').write_text(json.dumps(report, indent=2))
    print(json.dumps({'output': str(args.output), 'max_displacement': report['max_displacement']}))


if __name__=='__main__':
    main()
