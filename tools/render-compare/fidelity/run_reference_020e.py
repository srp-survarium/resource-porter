#!/usr/bin/env python3
"""Capture fresh original 0.20e references (static, UI hidden by the hook, no input).

    run_reference_020e.py REFERENCE_ROOT --poses POSES.json --frames N --output DIR --wine WINE \
        --viewport VIEWPORT.exe [--texture-quality 2]

REFERENCE_ROOT is the prepared private reference (`prepare_reference.py`):
client, wineprefix, server-source and sessions.txt. The run refuses an active
prefix or a busy port, restores the profile, and stops only its own prefix's
Wine server and its own server/client process groups.
"""
import argparse, importlib.util, json, os, signal, socket, subprocess, time
from pathlib import Path


def win(path):
    return 'Z:'+str(path).replace('/', '\\')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root', type=Path); ap.add_argument('--poses', type=Path, required=True)
    ap.add_argument('--frames', type=int, default=1); ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--wine', type=Path, required=True); ap.add_argument('--viewport', type=Path, required=True)
    ap.add_argument('--texture-quality', default='2')
    a = ap.parse_args()
    r = a.root.resolve(); prefix = r/'wineprefix'
    poses = json.loads(a.poses.read_text())
    assert all(p.get('buffers') is False for p in poses) and len({p['name'] for p in poses}) == len(poses)
    frames = []
    for pose in poses:
        frames.append(dict(pose, name=pose['name']+('~f0' if a.frames > 1 else '')))
        frames += [dict(pose, name=f"{pose['name']}~f{i}", settle_ms=400) for i in range(1, a.frames)]
    for proc in Path('/proc').iterdir():
        if proc.name.isdigit() and int(proc.name) != os.getpid():
            try:
                if ('WINEPREFIX='+str(prefix)).encode() in (proc/'environ').read_bytes().split(b'\0'):
                    raise SystemExit('reference prefix is already active')
            except (OSError, PermissionError):
                pass
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(('127.0.0.1', 2264))
    spec = importlib.util.spec_from_file_location('capture', Path(__file__).resolve().parents[1]/'capture.py')
    capture = importlib.util.module_from_spec(spec); spec.loader.exec_module(capture)
    cfg = prefix/'drive_c/users/sheep/Documents/Survarium/user.cfg'
    original_cfg = cfg.read_bytes()
    lines = [l for l in original_cfg.decode('cp1251').splitlines() if not l.startswith('r_texture_quality ')]
    a.output.mkdir(parents=True, exist_ok=False)
    control = a.output/'camera.txt'
    env = {k: v for k, v in os.environ.items() if not k.startswith(('SRP_', 'WINEPREFIX'))}
    env.update(WINEPREFIX=str(prefix), DISPLAY=':0', WINEDEBUG='-all',
               WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
               VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',
               DXVK_FRAME_RATE='60', SRP_CAPTURE_BUILD='020e', SRP_CAPTURE_CONTROL=win(control))
    env.pop('VK_DRIVER_FILES', None)
    processes, logs = [], []
    try:
        cfg.write_text('\n'.join(lines+[f'r_texture_quality {a.texture_quality}'])+'\n', encoding='cp1251')
        for label, cmd, cwd in [
                ('server', [str(r/'server-source/target/release/match-server-021d'), str(r/'sessions.txt'), '2264', '1'], a.output),
                ('client', [str(a.wine), win(r/'client/binaries/x86/survarium-capture.exe'), '-client=127.0.0.1:2264',
                            '-spectator', '-localization=english', '-no_splash_screen',
                            '-minimum_resources_memory_size=768'], r/'client/binaries/x86')]:
            log = (a.output/(label+'.log')).open('w'); logs.append(log)
            processes.append(subprocess.Popen(cmd, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                                              stdin=subprocess.DEVNULL, start_new_session=True))
        (a.output/'launch.json').write_text(json.dumps({'pids': [p.pid for p in processes], 'prefix': str(prefix),
                                                         'texture_quality': a.texture_quality}, indent=2)+'\n')
        status_path = Path(str(control)+'.status.json')
        deadline, ready_since, previous = time.monotonic()+240, None, None
        while time.monotonic() < deadline:
            if any(p.poll() is not None for p in processes):
                raise RuntimeError('reference process exited before scene readiness')
            status = json.loads(status_path.read_text()) if status_path.exists() else {}
            if status.get('error'):
                raise RuntimeError(status['error'])
            log = (a.output/'server.log').read_text(errors='replace')
            counters = (status.get('camera_calls', 0), status.get('present_calls', 0))
            if 'static setup sent' in log and min(counters) > 200:
                if previous and all(x > y for x, y in zip(counters, previous)):
                    ready_since = ready_since or time.monotonic()
                    if time.monotonic()-ready_since > 10:
                        (a.output/'readiness.json').write_text(json.dumps(status, indent=2)+'\n'); break
                else:
                    ready_since = None
            previous = counters
            time.sleep(.5)
        else:
            raise TimeoutError('scene readiness not established')
        subprocess.run([str(a.wine), win(a.viewport)], env=env, check=True, timeout=15)
        warm = dict(frames[0], name='readiness-warmup', settle_ms=8000)
        capture.capture(control, [warm], a.output/'warmup', timeout=60)
        skipped, groups = {}, {}
        for pose in frames:
            groups.setdefault(pose['name'].split('~')[0], []).append(pose)
        for base, group in groups.items():
            try:
                capture.capture(control, group, a.output/'poses', timeout=40)
            except (TimeoutError, RuntimeError) as error:
                skipped[base] = str(error)
        (a.output/'skipped.json').write_text(json.dumps(skipped, indent=2)+'\n')
        (a.output/'final-status.json').write_bytes(status_path.read_bytes())
        (a.output/'profile-during.cfg').write_bytes(cfg.read_bytes())
    finally:
        subprocess.run([str(a.wine.parent/'wineserver'), '-k'], env=env, timeout=15, check=False)
        subprocess.run([str(a.wine.parent/'wineserver'), '-w'], env=env, timeout=30, check=False)
        for p in processes:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGTERM)
                try: p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid, signal.SIGKILL); p.wait(timeout=5)
        for log in logs: log.close()
        cfg.write_bytes(original_cfg)
    print(json.dumps({'output': str(a.output), 'frames': len(frames)}))


if __name__ == '__main__':
    main()
