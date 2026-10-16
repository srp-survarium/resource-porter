#!/usr/bin/env python3
"""Static, input-free captures of a package map on a private Xvfb display.

    capture_dm_gallery.py --package PKG --map KEY --poses POSES.json --output DIR
        [--quality preset|low|medium|high] [--frame-rate 0] [--fps-seconds 8]

Starts its own Xvfb (never the user's display), launches one client through
the package's launch.py with UI off, waits for the match camera, then for
each pose places the hook's fixed camera, saves a screenshot (no GPU buffer
readback) and, with --fps-seconds, counts presented frames while the camera
holds still. Nothing sends input. The package supervisor, its Wine prefix,
match server and the Xvfb are stopped afterwards. POSES is a JSON list of
{"name", "position": [x,y,z], "target": [x,y,z]} (or "forward").
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'render-compare'))
from capture import capture  # noqa: E402

XVFB = '/nix/store/vdpfrvny2hz0cd14yrsnk11xns9n35yp-xorg-server-21.1.24/bin/Xvfb'


def status(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--package', type=Path, required=True)
    ap.add_argument('--map', required=True)
    ap.add_argument('--poses', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--runtime', type=Path, default=Path('/tmp/soc-dm-capture-runtime'))
    ap.add_argument('--port', type=int, default=2298)
    ap.add_argument('--display', default=':93')
    ap.add_argument('--quality', default='preset')
    ap.add_argument('--frame-rate', type=int, default=60)
    ap.add_argument('--fps-seconds', type=float, default=0)
    ap.add_argument('--load-timeout', type=float, default=420)
    a = ap.parse_args()
    if Path(f'/tmp/.X11-unix/X{a.display[1:]}').exists():
        raise SystemExit(f'display {a.display} is in use')
    poses = json.loads(a.poses.read_text())
    for pose in poses:
        if 'forward' not in pose:
            d = [t - p for t, p in zip(pose['target'], pose['position'])]
            n = math.sqrt(sum(v*v for v in d))
            pose['forward'] = [v/n for v in d]
        pose.setdefault('buffers', False)
        pose.setdefault('settle_ms', 4000)
    a.output.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ('WAYLAND_DISPLAY', 'XDG_SESSION_TYPE')}
    env['DISPLAY'] = a.display
    log = (a.output/'xvfb.log').open('w')
    xvfb = subprocess.Popen([XVFB, a.display, '-screen', '0', '1920x1080x24', '-nolisten', 'tcp'],
                            stdout=log, stderr=subprocess.STDOUT, env=env)
    launcher = [str(a.package/'play-maps')]
    common = ['--runtime', str(a.runtime), '--port', str(a.port)]
    results = {'map': a.map, 'quality': a.quality, 'frame_rate_cap': a.frame_rate, 'poses': []}
    try:
        time.sleep(2)
        if xvfb.poll() is not None:
            raise SystemExit('Xvfb failed to start')
        subprocess.run(launcher + ['start', a.map, '--ui', 'off', '--quality', a.quality,
                                   '--frame-rate', str(a.frame_rate)] + common, env=env, check=True,
                       stdout=subprocess.DEVNULL)
        control = a.runtime/'camera.txt'
        status_path = control.with_name(control.name + '.status.json')
        deadline = time.monotonic() + a.load_timeout
        loaded = None
        while time.monotonic() < deadline:
            s = status(status_path)
            state = status(a.runtime/'state.json') or {}
            if state.get('status') == 'stopped':
                raise SystemExit('package stopped before the map loaded: ' + str(state.get('error')))
            if s and s.get('camera_calls', 0) > 200:
                loaded = loaded or time.monotonic()
                if time.monotonic() - loaded > 20:
                    break
            time.sleep(1)
        else:
            raise SystemExit('the match camera never started; see the runtime logs')
        results['load_seconds'] = round(a.load_timeout - (deadline - time.monotonic()), 1)
        for pose in poses:
            record = capture(control, [pose], a.output, timeout=60)[0]
            if a.fps_seconds:
                first = status(status_path)
                time.sleep(a.fps_seconds)
                last = status(status_path)
                frames = last['present_calls'] - first['present_calls']
                record['fps'] = round(frames / a.fps_seconds, 1)
                record['frame_ms'] = round(1000 * a.fps_seconds / frames, 2) if frames else None
            results['poses'].append({'name': pose['name'], 'image': record['image'],
                                     'fps': record.get('fps'), 'frame_ms': record.get('frame_ms')})
            print(json.dumps(results['poses'][-1]), flush=True)
    finally:
        subprocess.run(launcher + ['stop'] + common, env=env, stdout=subprocess.DEVNULL)
        xvfb.terminate()
        try:
            xvfb.wait(timeout=10)
        except subprocess.TimeoutExpired:
            xvfb.kill()
        (a.output/'capture.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
