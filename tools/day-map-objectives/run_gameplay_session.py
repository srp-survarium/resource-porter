#!/usr/bin/env python3
"""Run a package's two-client match with a private server packet log.

Uses the package's own launcher (exclusive runtime lock, owned Wine prefixes,
cleanup on SIGTERM) and only appends ``--packet-log`` to the match server.
Stop it by sending SIGTERM to this process; the launcher stops its clients,
waits for their Wine servers and records ``status: stopped``.
"""
import argparse
import importlib.util
from pathlib import Path
import uuid


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('package', type=Path)
    ap.add_argument('map')
    ap.add_argument('--runtime', type=Path, required=True)
    ap.add_argument('--port', type=int, required=True)
    ap.add_argument('--clients', type=int, choices=[1, 2], default=2)
    ap.add_argument('--ui', choices=['on', 'off'], default='on')
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location('owned_launcher', a.package / 'launch.py')
    launcher = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(launcher)
    popen = launcher.subprocess.Popen
    a.runtime = a.runtime.resolve()

    def with_packets(command, *args, **kwargs):
        if isinstance(command, list) and Path(command[0]).name == 'match-server-026':
            command = command + ['--packet-log', str(a.runtime / 'server-packets.log')]
        return popen(command, *args, **kwargs)

    launcher.subprocess.Popen = with_packets
    a.token = uuid.uuid4().hex
    launcher.run(launcher.read(a.package / 'maps.json'), a)


if __name__ == '__main__':
    main()
