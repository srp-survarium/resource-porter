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


def virtual_desktop(package, runtime, env, size):
    """Run the runtime prefix's windows in a Wine virtual desktop.

    On a bare Xvfb the monitor mode has no refresh rate and DXVK's dxgi
    divides by it at device creation; a virtual desktop reports one.
    """
    import tarfile
    prefix = runtime/'wineprefix'
    if not (prefix/'system.reg').exists():
        runtime.mkdir(parents=True, exist_ok=True)
        marker = runtime/'package.json'
        if not marker.exists():
            if any(runtime.iterdir()):
                raise SystemExit('refusing to prepare an unowned runtime directory')
            marker.write_text(json.dumps({'package': str(package.resolve())}, indent=2) + '\n')
        with tarfile.open(package/'wineprefix.tar.gz') as archive:
            archive.extractall(runtime, filter='fully_trusted')
    catalog = json.loads((package/'maps.json').read_text())
    wine = Path(catalog['wine'])
    env = dict(env, WINEPREFIX=str(prefix), WINEDEBUG='-all')
    for key, name, value in [(r'HKCU\Software\Wine\Explorer', 'Desktop', 'Default'),
                             (r'HKCU\Software\Wine\Explorer\Desktops', 'Default', size)]:
        subprocess.run([str(wine), 'reg', 'add', key, '/v', name, '/d', value, '/f'], env=env, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run([str(wine.parent/'wineserver'), '-w'], env=env, check=True)


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
    ap.add_argument('--window', default='1280x720', help='client resolution, e.g. 1920x1080 for loading screens')
    ap.add_argument('--set', action='append', default=[], metavar='KEY=VALUE',
                    help='extra user.cfg overrides passed to the launcher, e.g. r_motion_blur_quality=0')
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
    xvfb = subprocess.Popen([XVFB, a.display, '-screen', '0', '2560x1440x24', '-nolisten', 'tcp'],
                            stdout=log, stderr=subprocess.STDOUT, env=env)
    launcher = [str(a.package/'play-maps')]
    common = ['--runtime', str(a.runtime), '--port', str(a.port)]
    results = {'map': a.map, 'quality': a.quality, 'frame_rate_cap': a.frame_rate, 'poses': []}
    try:
        time.sleep(2)
        if xvfb.poll() is not None:
            raise SystemExit('Xvfb failed to start')
        size = tuple(int(v) for v in a.window.split('x'))
        # Leave room around the window: a window as large as the desktop is
        # shrunk by its frame and the backbuffer misses the requested size.
        desktop = a.window if size == (1280, 720) else f'{size[0] + 64}x{size[1] + 64}'
        virtual_desktop(a.package, a.runtime, env, desktop)
        extra = [f'--set=r_resolution={a.window}'] + [f'--set={item}' for item in a.set]
        subprocess.run(launcher + ['start', a.map, '--ui', 'off', '--quality', a.quality,
                                   '--frame-rate', str(a.frame_rate)] + extra + common, env=env, check=True,
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
            server_log = a.runtime/'server.log'
            started = server_log.exists() and 'Verified checkpoint' in server_log.read_text(errors='replace')
            if s and s.get('camera_calls', 0) > 0 and started:
                loaded = loaded or time.monotonic()
                if time.monotonic() - loaded > 20:
                    break
            time.sleep(1)
        else:
            raise SystemExit('the match camera never started; see the runtime logs')
        results['load_seconds'] = round(a.load_timeout - (deadline - time.monotonic()), 1)
        for pose in poses:
            record = capture(control, [pose], a.output, timeout=60, size=size)[0]
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
