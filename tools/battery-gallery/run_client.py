#!/usr/bin/env python3
"""Run the frozen package with UI off and a private battery checkpoint log."""
import argparse
import importlib.util
from pathlib import Path
import uuid


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('package', type=Path)
    ap.add_argument('map')
    ap.add_argument('--runtime', type=Path, required=True)
    ap.add_argument('--port', type=int, default=2262)
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location('owned_launcher', a.package/'launch.py')
    launcher = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(launcher)
    popen = launcher.subprocess.Popen

    def with_packets(command, *args, **kwargs):
        if isinstance(command, list) and Path(command[0]).name == 'match-server-026':
            command = command + ['--packet-log', str(a.runtime.resolve()/'server-packets.log')]
        return popen(command, *args, **kwargs)

    launcher.subprocess.Popen = with_packets
    a.runtime = a.runtime.resolve()
    a.clients = 1
    a.ui = 'off'
    a.token = uuid.uuid4().hex
    launcher.run(launcher.read(a.package/'maps.json'), a)


if __name__ == '__main__':
    main()
