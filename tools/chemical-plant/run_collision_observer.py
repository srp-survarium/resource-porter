#!/usr/bin/env python3
"""Run an owned Chemical diagnostic without modifying packaged renderer assets."""
import argparse
import hashlib
import importlib.util
from pathlib import Path
import sys
import uuid


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('package', type=Path); ap.add_argument('observer_bundle', type=Path)
    ap.add_argument('--runtime', type=Path, required=True)
    ap.add_argument('--port', type=int, default=2261)
    ap.add_argument('--map', default='chemical-011-day')
    a = ap.parse_args()
    package = a.package.resolve(); bundle = a.observer_bundle.resolve()
    spec = importlib.util.spec_from_file_location('owned_map_launcher', package/'launch.py')
    launcher = importlib.util.module_from_spec(spec); spec.loader.exec_module(launcher)
    original_read = launcher.read
    catalog = original_read(package/'maps.json')
    item = next(m for m in catalog['maps'] if m['key'] == a.map)
    preset_path = package/item['preset']
    preset = original_read(preset_path)
    frozen = package/preset['hook']
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    if digest(frozen) != digest(bundle/'camera_hook_render.dll'):
        raise ValueError('observer renderer is not the current frozen Chemical DLL')
    if not (bundle/'camera_hook.dll').is_file(): raise ValueError('missing observer proxy')

    def diagnostic_read(path):
        result = original_read(path)
        if Path(path) == preset_path:
            result['environment']['SRP_CAPTURE_COLLISION'] = '1'
        return result

    copy2 = launcher.shutil.copy2
    def diagnostic_copy(source, target, *args, **kwargs):
        if Path(source) == frozen and Path(target).name == 'camera_hook.dll':
            copy2(bundle/'camera_hook_render.dll', Path(target).with_name('camera_hook_render.dll'))
            return copy2(bundle/'camera_hook.dll', target, *args, **kwargs)
        return copy2(source, target, *args, **kwargs)

    popen = launcher.subprocess.Popen
    def diagnostic_popen(command, *args, **kwargs):
        if isinstance(command, list) and Path(command[0]).name == 'match-server-026':
            command = command + ['--packet-log', str(a.runtime.resolve()/'server-packets.log')]
        return popen(command, *args, **kwargs)

    launcher.read = diagnostic_read
    launcher.shutil.copy2 = diagnostic_copy
    launcher.subprocess.Popen = diagnostic_popen
    sys.argv = [str(package/'launch.py'), '_run', a.map,
                '--runtime', str(a.runtime.resolve()), '--port', str(a.port),
                '--clients', '1', '--ui', 'on', '--token', uuid.uuid4().hex]
    launcher.main()


if __name__ == '__main__':
    main()
