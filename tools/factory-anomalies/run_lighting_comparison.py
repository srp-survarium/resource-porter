#!/usr/bin/env python3
"""Capture native map196 and its opt-in ambient candidate, then restore exactly.

Run only with the coordinated GPU lease and after the combined DB is restored.
No map, material, shader archive, spawn or gameplay data is modified.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(8*1024*1024), b''):
            h.update(block)
    return h.hexdigest()


def main(package, poses, output):
    report = json.loads((package/'comparison.json').read_text())
    preset = package/report['preset']
    source_preset = Path(report['source'])/report['preset']
    baseline = (package/'baseline-preset.json').read_bytes()
    if digest(preset) != report['baseline_sha256'] or digest(source_preset) != report['baseline_sha256']:
        raise ValueError('native preset changed; refusing comparison')
    if digest(package/report['database']) != report['database_sha256']:
        raise ValueError('combined database is not the restored reviewed candidate')
    output.mkdir(exist_ok=False)
    runtime = Path(json.loads((package/'maps.json').read_text())['default_runtime'])
    launch = [sys.executable, str(package/'launch.py')]
    script = Path(__file__).resolve().parents[1]/'render-compare/capture.py'
    spec = importlib.util.spec_from_file_location('capture', script)
    capture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    views = json.loads(poses.read_text())
    if any(p.get('buffers') is not False for p in views):
        raise ValueError('comparison permits screenshot-only poses')
    current_sha = report['baseline_sha256']
    try:
        for arm in ('baseline', 'candidate'):
            if digest(preset) != current_sha:
                raise ValueError('private preset changed unexpectedly')
            body = (package/(arm+'-preset.json')).read_bytes()
            if hashlib.sha256(body).hexdigest() != report[arm+'_sha256']:
                raise ValueError('prepared arm changed')
            preset.write_bytes(body)
            current_sha = report[arm+'_sha256']
            destination = output/arm
            destination.mkdir()
            try:
                subprocess.run(launch+['start', 'radar-020e-day', '--ui', 'off'], check=True)
                deadline = time.monotonic()+90
                status_path = runtime/'camera.txt.status.json'
                while time.monotonic() < deadline:
                    if status_path.exists():
                        status = json.loads(status_path.read_text())
                        if status.get('error'):
                            raise RuntimeError(status['error'])
                        if status.get('camera_calls', 0)>20 and status.get('present_calls', 0)>20:
                            break
                    time.sleep(.25)
                else:
                    raise TimeoutError('native renderer did not initialize')
                capture.capture(runtime/'camera.txt', views, destination, timeout=20)
                for name in ('camera.txt.status.json', 'camera.txt.ambient-020e.json',
                             'camera.txt.ambient-020e-apply.json', 'client.log', 'server.log'):
                    p = runtime/name
                    if p.exists():
                        (destination/name).write_bytes(p.read_bytes())
                if arm == 'candidate':
                    for name in ('camera.txt.ambient-020e.json', 'camera.txt.ambient-020e-apply.json'):
                        if not (destination/name).exists():
                            raise RuntimeError('candidate has no verified stage/apply receipt: '+name)
                    applied = json.loads((destination/'camera.txt.ambient-020e-apply.json').read_text())
                    if applied.get('hresult') != 0 or applied.get('shader_bytes') != 704:
                        raise RuntimeError('exact source ambient apply shader was not accepted')
            finally:
                subprocess.run(launch+['stop'], check=True)
    finally:
        if digest(preset) != current_sha:
            raise RuntimeError('private preset changed unexpectedly; refusing to overwrite it')
        preset.write_bytes(baseline)
        if digest(preset) != report['baseline_sha256'] or digest(source_preset) != report['baseline_sha256']:
            raise RuntimeError('preset restoration or source guard failed')
    if digest(package/report['database']) != report['database_sha256']:
        raise RuntimeError('database changed during comparison')
    (output/'restoration.json').write_text(json.dumps(dict(database_unchanged=True,
        native_preset_restored=True, shared_preset_unchanged=True, recipe=report), indent=2)+'\n')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('package', 'poses', 'output'):
        p.add_argument(name, type=Path)
    a = p.parse_args()
    main(a.package.resolve(), a.poses, a.output)
