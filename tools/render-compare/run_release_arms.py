#!/usr/bin/env python3
"""Capture fixed poses for prepared lighting arms in a private harness.

ARMS is JSON: {"arms": [{"name", "map", "preset", "poses", "patches": [...],
"require": [receipt names]}]}. `preset` and `poses` are paths relative to the
ARMS file; preset asset/hook paths are relative to the harness and must stay
inside it. `patches` are patch_archive_entries.py plans applied to the
harness's private database copy only.

Before launching, the runner takes the shared candidate runtime lock without
waiting and refuses to start while any other Survarium client runs; exit code
75 means "busy, retry later". It stops its owned client after every arm,
waits for the private Wine server, checks recorded PIDs are gone, reverts all
patches and restores the harness preset and database byte-for-byte.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

TOOLS = Path(__file__).resolve().parent
BUSY = 75


def load(name):
    spec = importlib.util.spec_from_file_location(name, TOOLS/(name+'.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(8*1024*1024), b''):
            h.update(block)
    return h.hexdigest()


def foreign_clients(owned):
    found = []
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit() or int(proc.name) in owned:
            continue
        try:
            argv = (proc/'cmdline').read_bytes().split(b'\0')
        except OSError:
            continue
        # Wine game processes run the Windows executable as argv[0]; shells
        # and editors merely mentioning the name are not clients.
        program = argv[0].lower()
        if program.endswith(b'.exe') and (b'survarium' in program or
                                          any(b'survarium' in a.lower() for a in argv[1:2])):
            found.append((int(proc.name), b' '.join(argv).decode(errors='replace')[:160]))
    return found


def wait_ready(runtime, deadline_seconds=150):
    deadline = time.monotonic()+deadline_seconds
    status_path = runtime/'camera.txt.status.json'
    while time.monotonic() < deadline:
        state_path = runtime/'state.json'
        if state_path.exists():
            state = json.loads(state_path.read_text())
            if state.get('status') in ('stopped', 'cleanup incomplete'):
                raise RuntimeError('launcher stopped: '+str(state.get('error')))
        if status_path.exists():
            try:
                status = json.loads(status_path.read_text())
            except json.JSONDecodeError:
                time.sleep(.25)
                continue
            if status.get('error'):
                raise RuntimeError(status['error'])
            log = runtime/'server.log'
            loaded = log.exists() and 'initial snapshot' in log.read_text(errors='replace')
            if loaded and status.get('present_calls', 0) > 20:
                return status
        time.sleep(.25)
    raise TimeoutError('renderer did not reach the loaded map')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('harness', type=Path)
    parser.add_argument('arms', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--only', action='append')
    parser.add_argument('--shared-lock', type=Path, default=Path('/tmp/three-day-map-runtime/run.lock'))
    args = parser.parse_args()
    harness = args.harness.resolve()
    record = json.loads((harness/'harness.json').read_text())
    catalog = json.loads((harness/'maps.json').read_text())
    runtime = Path(catalog['default_runtime'])
    spec = json.loads(args.arms.read_text())
    arms = [a for a in spec['arms'] if not args.only or a['name'] in args.only]
    if not arms:
        parser.error('no arms selected')
    base = args.arms.resolve().parent
    capture = load('capture')
    patcher = load('patch_archive_entries')
    database = harness/'resources.db'
    for arm in arms:
        preset = json.loads((base/arm['preset']).read_text())
        for relative in [preset['hook'], preset['config'], *preset['assets'].values()]:
            path = (harness/relative).resolve()
            if not path.exists() or not path.is_relative_to(harness):
                raise ValueError(f"{arm['name']}: missing or external {relative}")
        for plan in arm.get('patches', []):
            patcher.patch(database, Path(plan), 'inspect')
    # Guard the shared runtime lock so other package users see the GPU as busy.
    args.shared_lock.parent.mkdir(parents=True, exist_ok=True)
    lock = args.shared_lock.open('a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print(json.dumps({'busy': 'shared candidate runtime lock held'}))
        return BUSY
    others = foreign_clients({os.getpid()})
    if others:
        print(json.dumps({'busy': 'other Survarium clients running', 'processes': others}))
        return BUSY
    if digest(database) != record['database_sha256']:
        raise RuntimeError('harness database differs from its recorded copy')
    args.output.mkdir(parents=True, exist_ok=False)
    launch = [sys.executable, str(harness/'launch.py')]
    applied, results = [], {}
    restore = {}
    try:
        for arm in arms:
            preset_path = harness/'presets'/arm['map']/'preset.json'
            if preset_path not in restore:
                restore[preset_path] = preset_path.read_bytes()
            for plan in applied:
                patcher.patch(database, Path(plan), 'revert')
            applied = []
            for plan in arm.get('patches', []):
                patcher.patch(database, Path(plan), 'apply')
                applied.append(plan)
            body = (base/arm['preset']).read_bytes()
            preset_path.write_bytes(body)
            destination = args.output/arm['name']
            destination.mkdir()
            (destination/'preset.json').write_bytes(body)
            poses = json.loads((base/arm['poses']).read_text())
            started = time.time()
            stopped = False
            try:
                subprocess.run(launch+['start', arm['map'], '--ui', 'off'], check=True,
                               stdout=subprocess.DEVNULL)
                wait_ready(runtime)
                warm = dict(poses[0], name='warmup', buffers=False, settle_ms=5000)
                capture.capture(runtime/'camera.txt', [warm], destination/'warmup', timeout=40)
                capture.capture(runtime/'camera.txt', poses, destination, timeout=60)
            finally:
                state = json.loads((runtime/'state.json').read_text())
                subprocess.run(launch+['stop'], check=True, stdout=subprocess.DEVNULL)
                env = dict(os.environ, WINEPREFIX=state['prefix'])
                subprocess.run([str(Path(catalog['wine']).with_name('wineserver')), '-w'],
                               env=env, check=True, timeout=60)
                pids = [state.get(k) for k in ('pid', 'server_pid', 'client_pid') if state.get(k)]
                alive = [pid for pid in pids if Path(f'/proc/{pid}').exists()]
                if alive:
                    raise RuntimeError(f'owned processes still present: {alive}')
                stopped = True
                for path in runtime.glob('camera.txt*.json'):
                    (destination/path.name).write_bytes(path.read_bytes())
                for name in ('client.log', 'server.log', 'state.json'):
                    if (runtime/name).exists():
                        (destination/name).write_bytes((runtime/name).read_bytes())
            missing = [name for name in arm.get('require', []) if not (destination/name).exists()]
            if missing:
                raise RuntimeError(f"{arm['name']}: missing receipts {missing}")
            results[arm['name']] = {'map': arm['map'], 'preset_sha256': hashlib.sha256(body).hexdigest(),
                                    'patches': arm.get('patches', []), 'poses': len(poses),
                                    'seconds': round(time.time()-started, 1), 'owned_pids_absent': pids,
                                    'stopped': stopped}
    finally:
        for plan in applied:
            patcher.patch(database, Path(plan), 'revert')
        for path, body in restore.items():
            path.write_bytes(body)
        lock.close()
    restored = digest(database) == record['database_sha256']
    presets = all(digest(harness/p) == h for p, h in record['presets'].items())
    report = {'harness': str(harness), 'arms': results, 'database_restored': restored,
              'presets_restored': presets}
    (args.output/'run.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))
    if not (restored and presets):
        raise RuntimeError('harness restoration failed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
