#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage an isolated 0.26g game and run a local server with two Wine clients.

Keep this command running. Ctrl-C stops the server and the two owned Wine
prefixes. Inputs are copied, never modified; logs and staged files are retained.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import time

REPO = Path(__file__).resolve().parents[1]


def executable(parser, value):
    found = shutil.which(value)
    if not found:
        parser.error(f'executable not found: {value}')
    return str(Path(found).absolute())


def stop(server, prefixes, wineserver):
    # Popen owns this process; never infer a PID from a previous run's state.
    if server is not None and server.poll() is None:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
    for prefix in prefixes:
        env = dict(os.environ, WINEPREFIX=str(prefix))
        subprocess.run([wineserver, '-k'], env=env, check=False, timeout=20)
        subprocess.run([wineserver, '-w'], env=env, check=False, timeout=20)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client-dir', type=Path, required=True, help='Source game directory containing binaries/x86 and resources')
    parser.add_argument('--client-exe', default='survarium-srp.exe', help='Reviewed SRP executable filename in binaries/x86')
    parser.add_argument('--db', type=Path, required=True, help='Database produced by build-026-maps.py')
    parser.add_argument('--map', type=int, required=True, choices=[67, *range(160, 178)])
    parser.add_argument('--server', default=str(REPO.parent / 'srp/target/debug/match-server-026'))
    parser.add_argument('--port', type=int, default=2246)
    parser.add_argument('--wine', default='wine')
    parser.add_argument('--wineserver', default='wineserver', help='Must come from the same Wine installation')
    parser.add_argument('--wine-prefix-template', type=Path, help='Optional configured, stopped Wine prefix to copy for each client')
    parser.add_argument('--work-dir', type=Path, required=True, help='New directory for the isolated game, prefixes and logs')
    parser.add_argument('--check', action='store_true', help='Validate inputs and print the plan without writing or launching')
    args = parser.parse_args()
    source = args.client_dir.resolve()
    work = args.work_dir.resolve()
    database = args.db.resolve()
    if not database.is_file() or not (source / 'resources').is_dir():
        parser.error('supply an existing --db and a complete --client-dir with resources/')
    if Path(args.client_exe).name != args.client_exe:
        parser.error('--client-exe must be a filename inside binaries/x86')
    if work.exists() or work.is_relative_to(source):
        parser.error('--work-dir must be new and outside the source client directory')
    if args.wine_prefix_template:
        args.wine_prefix_template = args.wine_prefix_template.resolve()
        if not (args.wine_prefix_template / 'system.reg').is_file():
            parser.error('--wine-prefix-template must be an initialized, stopped Wine prefix')
        if work.is_relative_to(args.wine_prefix_template):
            parser.error('--work-dir cannot be inside the prefix being copied')
    if not 1024 <= args.port <= 65535:
        parser.error('--port must be between 1024 and 65535')
    server_bin = executable(parser, args.server)
    wine = executable(parser, args.wine)
    wineserver = executable(parser, args.wineserver)
    patch_path = REPO / 'tools/patch-026-map-resources.py'
    spec = importlib.util.spec_from_file_location('patch_026', patch_path)
    patcher = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(patcher)
    try:
        patched, _ = patcher.patch((source / 'binaries/x86' / args.client_exe).read_bytes())
    except (OSError, ValueError) as error:
        parser.error(str(error))
    plan = {'map': args.map, 'database': str(database), 'work_dir': str(work),
            'endpoint': f'127.0.0.1:{args.port}', 'clients': 2, 'language': 'english'}
    print(json.dumps(plan, indent=2), flush=True)
    if args.check:
        return
    if not os.environ.get('DISPLAY'):
        parser.error('DISPLAY must select an X11/XWayland session')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(('127.0.0.1', args.port))
    work.mkdir(parents=True)
    game = work / 'game'
    shutil.copytree(source / 'binaries', game / 'binaries')
    shutil.copytree(source / 'resources', game / 'resources')
    (game / 'binaries/x86/survarium-ported.exe').write_bytes(patched)
    (game / 'resources.db').symlink_to(database)
    prefixes = [work / f'wineprefix-{i}' for i in (1, 2)]
    for prefix in prefixes:
        if args.wine_prefix_template:
            shutil.copytree(args.wine_prefix_template, prefix, symlinks=True)
        else:
            prefix.mkdir()
    (work / 'run.json').write_text(json.dumps(plan, indent=2) + '\n')
    server = None
    clients = []
    started_prefixes = []

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        with (work / 'server.log').open('w') as log:
            server = subprocess.Popen([server_bin, '--bind', plan['endpoint'], '--players', '2',
                                       '--start-players', '2', '--map', str(args.map), '--snapshots',
                                       '--checkpoints', '--recover', '--clock-sync'],
                                      stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        time.sleep(1)
        if server.poll() is not None:
            raise RuntimeError(f'server exited; inspect {work / "server.log"}')
        for i, prefix in enumerate(prefixes, 1):
            env = dict(os.environ, WINEPREFIX=str(prefix), WINEDEBUG='-all',
                       WINEDLLOVERRIDES=os.environ.get('WINEDLLOVERRIDES',
                           'mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b'))
            # The reviewed client generates sessions randomly; omit fixed-roster settings.
            env.pop('SRP_MATCH_SESSION', None)
            with (work / f'client-{i}.log').open('w') as log:
                clients.append(subprocess.Popen([wine, 'explorer', f'/desktop=Porter-026-{i},1280x720',
                    'survarium-ported.exe', '-localization=english', '-splash_screen',
                    f'-server={plan["endpoint"]}', '-mt_allocator=768', '-hash_check_interval_in_ms=50'],
                    cwd=game / 'binaries/x86', env=env, stdin=subprocess.DEVNULL,
                    stdout=log, stderr=subprocess.STDOUT))
            started_prefixes.append(prefix)
        print(f'Running; Ctrl-C stops both clients and the server. Logs: {work}', flush=True)
        while server.poll() is None:
            time.sleep(1)
        raise RuntimeError(f'server exited; inspect {work / "server.log"}')
    except KeyboardInterrupt:
        pass
    finally:
        signal.signal(signal.SIGTERM, previous)
        stop(server, started_prefixes, wineserver)
        for client in clients:
            try:
                client.wait(timeout=10)
            except subprocess.TimeoutExpired:
                client.terminate()
                client.wait(timeout=10)


if __name__ == '__main__':
    main()
