#!/usr/bin/env python3
"""Run prepared, screenshot-only original 0.11e references after a GPU handoff.

Without --launch, validates the staged recipe only. Never modifies a database.
The dedicated old-original prefix must be stopped before use; its profile is
restored after both maps, and only that prefix's Wine server is stopped.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def win(path):
    return 'Z:'+str(path).replace('/', '\\')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('recipe', type=Path)
    p.add_argument('--maps', nargs='+', choices=['chemical', 'radar'], default=['chemical', 'radar'])
    p.add_argument('--launch', action='store_true')
    p.add_argument('--poses-dir', type=Path, help='use DIR/<map>.json instead of the recipe poses')
    p.add_argument('--output', type=Path, help='write DIR/capture-<map> instead of next to the recipe')
    p.add_argument('--frames', type=int, default=1, help='frames per pose (later frames reuse the settled camera)')
    p.add_argument('--stages', action='store_true',
                   help='diagnostic hook only: capture per-stage buffers (SRP_CAPTURE_011E_STAGES=1) for poses that ask')
    a = p.parse_args()
    recipe = json.loads(a.recipe.read_text())
    root = a.recipe.resolve().parent
    binary = root/'client/binaries/win32'
    prefix = Path(recipe['prefix'])
    exe = binary/'survarium-capture.exe'
    wine = Path(recipe['wine'])
    server = Path(recipe['server'])
    for file, digest in recipe['pinned_files'].items():
        assert sha(Path(file)) == digest, f'input changed: {file}'
    assert (root/'client/resources.db').resolve() == Path(recipe['database']).resolve()
    assert (prefix/'system.reg').is_file()
    def load_poses(name):
        poses = json.loads(((a.poses_dir/(name+'.json')) if a.poses_dir else (root/(name+'-poses.json'))).read_text())
        assert 1 <= len(poses) <= (60 if a.poses_dir else 18) and (a.stages or all(p['buffers'] is False for p in poses))
        assert len({p['name'] for p in poses}) == len(poses)
        frames = []
        for pose in poses:
            frames.append(dict(pose, name=pose['name']+('~f0' if a.frames > 1 else '')))
            frames += [dict(pose, name=f"{pose['name']}~f{i}", settle_ms=400) for i in range(1, a.frames)]
        return frames
    for name in a.maps:
        load_poses(name)
    print('Verified bounded original011e recipe; screenshot-only views.', flush=True)
    if not a.launch:
        return
    # Never attach to an active prefix, including one used for a previous attempt.
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            values = (process/'environ').read_bytes().split(b'\0')
        except (OSError, PermissionError):
            continue
        if ('WINEPREFIX='+str(prefix)).encode() in values and int(process.name) != os.getpid():
            raise RuntimeError('dedicated reference prefix is already active')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1', 2254))
    cfg = prefix/'drive_c/users/sheep/Documents/survarium/user.cfg'
    original_cfg = cfg.read_bytes()
    spec = importlib.util.spec_from_file_location('capture', Path(__file__).resolve().parents[1]/'render-compare/capture.py')
    capture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    env = {k: v for k, v in os.environ.items() if not k.startswith(('SRP_', 'WINEPREFIX'))}
    env.update(WINEPREFIX=str(prefix), DISPLAY=':0', WINEDEBUG='-all',
               WINEDLLOVERRIDES='mscoree,mshtml=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
               VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',
               DXVK_FRAME_RATE='60', PULSE_LATENCY_MSEC='60', SRP_CAPTURE_BUILD='011e',
               LANG='ru_RU.UTF-8', LC_ALL='ru_RU.UTF-8',
               LOCALE_ARCHIVE_2_27=recipe['locale_archive'])
    if a.stages:
        env['SRP_CAPTURE_011E_STAGES'] = '1'
        env['SRP_CAPTURE_BUFFERS'] = '1'
    env.pop('VK_DRIVER_FILES', None)
    try:
        cfg.write_bytes((root/'reference-user.cfg').read_bytes())
        for name in a.maps:
            out = (a.output or root)/('capture-'+name)
            out.mkdir(exist_ok=False)
            (out/'target/debug').mkdir(parents=True)
            control = out/'camera.txt'
            env.update(SRP_CAPTURE_CONTROL=win(control), SRP_MAP_PROFILE=recipe['maps'][name]['server_profile'])
            processes = []
            logs = []
            try:
                for label, cmd, cwd in [
                    ('server', [str(server)], out),
                    ('client', [str(wine), win(exe), '-client=127.0.0.1:2254', '-spectator',
                                '-no_splash_screen', '-max_resources_size=2560',
                                '-no_warning_on_page_file_size'], binary)]:
                    log = (out/(label+'.log')).open('w'); logs.append(log)
                    processes.append(subprocess.Popen(cmd, cwd=cwd, env=env, stdout=log,
                                     stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True))
                (out/'launch.json').write_text(json.dumps({'pids': [p.pid for p in processes],
                    'recipe': recipe, 'map': name, 'prefix': str(prefix)}, indent=2)+'\n')
                deadline = time.monotonic()+180
                ready_since = None
                previous = None
                status_path = Path(str(control)+'.status.json')
                while time.monotonic() < deadline:
                    if any(p.poll() is not None for p in processes):
                        raise RuntimeError('reference process exited before scene readiness')
                    status = json.loads(status_path.read_text()) if status_path.exists() else {}
                    if status.get('error'):
                        raise RuntimeError(status['error'])
                    log = (out/'server.log').read_text(errors='replace')
                    counters = (status.get('camera_calls', 0), status.get('present_calls', 0))
                    if 'join_match' in log and 'sync_response' in log and min(counters) > 10:
                        if previous and all(x > y for x, y in zip(counters, previous)):
                            ready_since = ready_since or time.monotonic()
                            if time.monotonic()-ready_since > 2:
                                (out/'readiness.json').write_text(json.dumps(status, indent=2)+'\n')
                                break
                        else:
                            ready_since = None
                    previous = counters
                    time.sleep(.5)
                else:
                    raise TimeoutError('scene readiness not established')
                # The helper runs in the same isolated prefix; it cannot resize other prefixes.
                subprocess.run([str(wine), win(root/'viewport.exe')], env=env, check=True, timeout=15)
                poses = load_poses(name)
                warmup = dict(poses[0], name='readiness-warmup', settle_ms=5000)
                capture.capture(control, [warmup], out/'warmup', timeout=30)
                if a.poses_dir:
                    # One stuck view must not lose the rest: skip it and record why.
                    skipped, groups = {}, {}
                    for pose in poses:
                        groups.setdefault(pose['name'].split('~')[0], []).append(pose)
                    for base, group in groups.items():
                        try:
                            capture.capture(control, group, out/'poses', timeout=30)
                        except (TimeoutError, RuntimeError) as error:
                            skipped[base] = str(error)
                    (out/'skipped.json').write_text(json.dumps(skipped, indent=2)+'\n')
                else:
                    capture.capture(control, poses, out/'poses', timeout=30)
                (out/'final-status.json').write_bytes(status_path.read_bytes())
            finally:
                # Stop only the prefix owned by this recipe, then its own server process.
                subprocess.run([str(wine.parent/'wineserver'), '-k'], env=env, timeout=15, check=False)
                subprocess.run([str(wine.parent/'wineserver'), '-w'], env=env, timeout=30, check=False)
                for process in processes:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        try: process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL); process.wait(timeout=5)
                for log in logs: log.close()
                (out/'profile-after.cfg').write_bytes(cfg.read_bytes())
    finally:
        cfg.write_bytes(original_cfg)


if __name__ == '__main__':
    main()
