#!/usr/bin/env python3
"""Capture one private diagnostic batch, stopping it before restoring the DB."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read(path):
    return json.loads(path.read_text())


def windows():
    return json.loads(subprocess.check_output(['niri', 'msg', '-j', 'windows'], text=True))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('package', type=Path)
    ap.add_argument('gallery', type=Path)
    ap.add_argument('map')
    ap.add_argument('batch', type=int)
    ap.add_argument('--runtime', type=Path, default=Path('/tmp/battery-gallery-runtime'))
    ap.add_argument('--port', type=int, default=2262)
    ap.add_argument('--limit', type=int)
    a = ap.parse_args()
    root = Path(__file__).resolve().parents[1]
    manifest = a.gallery/'manifest.json'
    item = next(m for m in read(manifest)['maps'] if m['key'] == a.map)
    batch = next(b for b in item['batches'] if b['index'] == a.batch)
    patcher = load_module('patcher', root/'render-compare/patch_archive_entries.py')
    plan = Path(batch['plan'])
    archive = a.package/'resources.db'
    evidence = a.gallery/'runs'/a.map/f'batch-{a.batch:02}'
    evidence.mkdir(parents=True, exist_ok=True)
    previous = {w['id'] for w in windows()}
    child = None
    applied = False
    catalog = read(a.package/'maps.json')
    try:
        print(json.dumps(patcher.patch(archive, plan, 'apply')), flush=True)
        applied = True
        with (evidence/'launcher.log').open('w') as log:
            child = subprocess.Popen([sys.executable, str(root/'battery-gallery/run_client.py'),
                str(a.package), a.map, '--runtime', str(a.runtime), '--port', str(a.port)],
                stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic()+100
        selected = None
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError('private client launcher exited; see launcher.log')
            found = [w for w in windows() if w['id'] not in previous and
                     w['app_id'] == 'survarium-capture.exe' and 'Survarium' in w['title']]
            if len(found) == 1:
                selected = found[0]['id']
                subprocess.run(['niri', 'msg', 'action', 'focus-window', '--id', str(selected)], check=True)
                break
            time.sleep(1)
        if selected is None:
            raise RuntimeError('owned capture window did not appear')
        env = dict(os.environ, WINEPREFIX=str(a.runtime/'wineprefix'), WINEDEBUG='-all', DISPLAY=os.environ.get('DISPLAY', ':0'))
        viewport = root/'render-compare/viewport.exe'
        subprocess.run([catalog['wine'], str(viewport), '--exe', 'survarium-capture.exe'], env=env, check=True, timeout=25)
        deadline = time.monotonic()+100
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError('private launcher exited while loading')
            log = a.runtime/'server.log'
            if log.exists() and log.read_text().count('Verified checkpoint') >= 3:
                break
            time.sleep(1)
        else:
            raise RuntimeError('native checkpoints did not arrive')
        # Freeze only complete packet-log lines before decoding.
        data = (a.runtime/'server-packets.log').read_bytes()
        (evidence/'server-packets.log').write_bytes(data[:data.rfind(b'\n')+1])
        snapshots = evidence/'snapshots'
        snapshots.mkdir(exist_ok=True)
        for old in snapshots.glob('match026g_*_battery.bin'):
            old.unlink()
        subprocess.run([sys.executable, str(root/'day-map-objectives/extract_snapshots.py'),
                        str(evidence/'server-packets.log'), str(snapshots)], check=True)
        subprocess.run([sys.executable, str(root/'day-map-objectives/validate_gallery_batch.py'),
                        str(manifest), a.map, str(a.batch), str(snapshots),
                        '--decoder', str(a.gallery/'reproduce/decode-battery-snapshot-json'),
                        '--output', str(manifest)], check=True)
        capture = [sys.executable, str(root/'battery-gallery/gallery.py'), str(manifest), str(a.gallery),
                   '--map', a.map, '--batch', str(a.batch), '--control', str(a.runtime/'camera.txt')]
        if a.limit:
            capture += ['--limit', str(a.limit)]
        subprocess.run(capture, check=True)
        for path in a.runtime.glob('camera.txt*.json'):
            shutil.copy2(path, evidence/path.name)
        overrides = read(evidence/'camera.txt.shader-overrides.json')
        if not overrides.get('legacy_sky_uses') or not overrides.get('legacy_composition_uses'):
            raise RuntimeError('legacy shader activation was not observed')
        shutil.copy2(a.runtime/'server.log', evidence/'server.log')
        shutil.copy2(a.runtime/'client.log', evidence/'client.log')
    finally:
        if child is not None:
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=55)
            state = read(a.runtime/'state.json')
            if state.get('status') != 'stopped':
                raise RuntimeError('client cleanup incomplete; keep diagnostic DB until it is stopped')
        if applied:
            print(json.dumps(patcher.patch(archive, plan, 'revert')), flush=True)


if __name__ == '__main__':
    main()
