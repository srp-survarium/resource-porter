#!/usr/bin/env python3
"""Select a frozen map recipe and manage only this package's private game."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import time
import uuid

PACKAGE = Path(__file__).resolve().parent


def read(path):
    return json.loads(path.read_text())


def atomic(path, value):
    temporary = path.with_suffix(path.suffix+'.tmp')
    temporary.write_text(json.dumps(value, indent=2)+'\n')
    temporary.replace(path)


def win(path):
    return 'Z:'+str(path).replace('/', '\\')


def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as source:
        while block := source.read(8*1024*1024):
            h.update(block)
    return h.hexdigest()


def start_ticks(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
    except FileNotFoundError:
        return None


def live(state):
    return bool(state.get('pid') and state.get('start_ticks') and
                start_ticks(state['pid']) == state['start_ticks'])


def checked_file(relative):
    path = PACKAGE/relative
    if not path.is_file() or not path.resolve().is_relative_to(PACKAGE):
        raise RuntimeError('missing or external package file: '+relative)
    return path


def plan(catalog, key, runtime, port):
    item = next((m for m in catalog['maps'] if m['key'] == key), None)
    if item is None:
        raise RuntimeError('unknown map; use list')
    recipe = read(checked_file(item['preset']))
    for name in ('database',):
        checked_file(item[name])
    for name in ('hook', 'config'):
        checked_file(recipe[name])
    for relative in recipe['assets'].values():
        path = PACKAGE/relative
        if not path.exists() or not path.resolve().is_relative_to(PACKAGE):
            raise RuntimeError('missing or external shader asset: '+relative)
    checked_file('client/binaries/x86/survarium-capture.exe')
    checked_file('client/binaries/x86/bugtrap.dll')
    checked_file('server/match-server-026')
    checked_file('wineprefix.tar.gz')
    if not Path(catalog['wine']).is_file():
        raise RuntimeError('pinned Wine runtime is unavailable')
    if not 1024 <= port <= 65535:
        raise RuntimeError('invalid local port')
    return item, recipe


def run(catalog, args):
    runtime = args.runtime.resolve()
    item, recipe = plan(catalog, args.map, runtime, args.port)
    runtime.mkdir(parents=True, exist_ok=True)
    marker = runtime/'package.json'
    if marker.exists():
        if read(marker)['package'] != str(PACKAGE):
            raise RuntimeError('runtime belongs to another package')
    elif any(runtime.iterdir()):
        raise RuntimeError('refusing to reuse an unowned runtime directory')
    else:
        atomic(marker, {'package': str(PACKAGE)})
    lock = (runtime/'run.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(('127.0.0.1', args.port))
    state_path = runtime/'state.json'
    state = {'package': str(PACKAGE), 'pid': os.getpid(), 'start_ticks': start_ticks(os.getpid()),
             'token': args.token, 'map': item['key'], 'map_id': item['map_id'], 'status': 'preparing',
             'runtime': str(runtime), 'port': args.port, 'camera': 'actual character', 'ui': args.ui,
             'client_count': args.clients}
    atomic(state_path, state)
    server = None
    clients = []
    prepared = []
    prefix = runtime/'wineprefix'
    wine = Path(catalog['wine'])
    env = {k:v for k,v in os.environ.items() if not k.startswith(('SRP_LEGACY_', 'SRP_CAPTURE_', 'SRP_DIAGNOSTIC_'))}
    env.update(WINEPREFIX=str(prefix), DISPLAY=os.environ.get('DISPLAY', ':0'), WINEDEBUG='-all',
               WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
               VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json', DXVK_FRAME_RATE='60')
    env.pop('VK_DRIVER_FILES', None)
    env.pop('SRP_MATCH_SESSION', None)
    env.update(recipe['environment'])
    for key, relative in recipe['assets'].items():
        env[key] = win(PACKAGE/relative)
    env.update(SRP_CAPTURE_BUILD='026g', SRP_CAPTURE_CONTROL=win(runtime/'camera.txt'),
               SRP_CAPTURE_BUFFERS='0', SRP_CAPTURE_MATERIALS='0', SRP_CAPTURE_FRAME_STATE='0')

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        for index in range(args.clients):
            slot = runtime if index == 0 else runtime/'peer2'
            slot.mkdir(parents=True, exist_ok=True)
            prefix = slot/'wineprefix'
            env = dict(env, WINEPREFIX=str(prefix), SRP_CAPTURE_CONTROL=win(slot/'camera.txt'))
            if prefix.exists():
                subprocess.run([str(wine.parent/'wineserver'), '-k'], env=env, timeout=20)
                subprocess.run([str(wine.parent/'wineserver'), '-w'], env=env, check=True, timeout=20)
            game = slot/'client'
            shutil.copytree(PACKAGE/'client', game, dirs_exist_ok=True)
            binary = game/'binaries/x86'
            shutil.copy2(PACKAGE/recipe['hook'], binary/'camera_hook.dll')
            link = game/'resources.db'
            if link.exists() or link.is_symlink():
                if not link.is_symlink():
                    raise RuntimeError('runtime database is not the managed symlink')
                link.unlink()
            link.symlink_to(PACKAGE/item['database'])
            if not (prefix/'system.reg').exists():
                expected = read(PACKAGE/'manifest.json')['files']['wineprefix.tar.gz']['sha256']
                if sha(PACKAGE/'wineprefix.tar.gz') != expected:
                    raise RuntimeError('Wine template checksum failed')
                # The package-owned, hash-verified template intentionally contains
                # Wine's c: relative link and z: absolute filesystem mapping.
                with tarfile.open(PACKAGE/'wineprefix.tar.gz') as archive:
                    archive.extractall(slot, filter='fully_trusted')
            config = prefix/'drive_c/users/sheep/Documents/Survarium/user.cfg'
            config.parent.mkdir(parents=True, exist_ok=True)
            lines = [s for s in (PACKAGE/recipe['config']).read_text().splitlines() if s.split()[:1] != ['ui']]
            config.write_text('\n'.join(lines)+f'\nui {args.ui}\n')
            for control in slot.glob('camera.txt*'):
                if control.is_file():
                    control.unlink()
            prepared.append((slot, binary, prefix, dict(env)))
        runtime_server = runtime/'match-server-026'
        shutil.copy2(PACKAGE/'server/match-server-026', runtime_server)
        runtime_server.chmod(0o755)
        with (runtime/'server.log').open('w') as log:
            server = subprocess.Popen([str(runtime_server), '--bind', f'127.0.0.1:{args.port}',
                '--players', str(args.clients), '--start-players', str(args.clients), '--map', str(item['map_id']),
                '--snapshots', '--checkpoints', '--recover', '--clock-sync'],
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        for index, (slot, binary, prefix, env) in enumerate(prepared, 1):
            with (slot/'client.log').open('w') as log:
                clients.append(subprocess.Popen([str(wine), win(binary/'survarium-capture.exe'), '-localization=english',
                    '-splash_screen', f'-server=127.0.0.1:{args.port}', '-mt_allocator=768', '-hash_check_interval_in_ms=50'],
                    env=env, cwd=binary, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT))
        state.update(status='running', server_pid=server.pid, client_pid=clients[0].pid,
                     prefix=str(prepared[0][2]), database=item['database'], preset=item['preset'],
                     clients=[{'number': i+1, 'pid': child.pid, 'prefix': str(prepared[i][2]),
                               'log': str(prepared[i][0]/'client.log')} for i, child in enumerate(clients)])
        atomic(state_path, state)
        while server.poll() is None and any(child.poll() is None for child in clients):
            time.sleep(.5)
        if server.poll() is not None:
            state['error'] = 'match server exited; inspect server.log'
    except KeyboardInterrupt:
        pass
    except Exception as error:
        state['error'] = str(error)
        raise
    finally:
        cleanup_errors = []
        for child, (_, _, _, client_env) in zip(clients, prepared):
            for operation in ('-k', '-w'):
                try:
                    subprocess.run([str(wine.parent/'wineserver'), operation], env=client_env, timeout=20)
                except (OSError, subprocess.TimeoutExpired) as error:
                    cleanup_errors.append(str(error))
            try:
                child.wait(timeout=20)
            except subprocess.TimeoutExpired as error:
                cleanup_errors.append(str(error))
        if server is not None and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill(); server.wait()
        state['status'] = 'stopped'
        if cleanup_errors:
            state.update(status='cleanup incomplete', cleanup_errors=cleanup_errors)
        atomic(state_path, state)
        lock.close()


def main():
    catalog = read(PACKAGE/'maps.json')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['list', 'start', 'status', 'stop', 'verify', '_run'])
    parser.add_argument('map', nargs='?')
    parser.add_argument('--runtime', type=Path, default=Path(catalog['default_runtime']))
    parser.add_argument('--port', type=int, default=catalog['default_port'])
    parser.add_argument('--ui', choices=['on', 'off'], default='on')
    parser.add_argument('--clients', type=int, choices=[1, 2], default=1)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--token', default='')
    args = parser.parse_args()
    runtime = args.runtime.resolve()
    state_path = runtime/'state.json'
    if args.command == 'list':
        for item in catalog['maps']:
            print(f"{item['key']:24} {item['title']} [map {item['map_id']}]")
    elif args.command == 'verify':
        records = read(PACKAGE/'manifest.json')['files']
        for relative, record in records.items():
            path = checked_file(relative)
            if path.stat().st_size != record['size'] or sha(path) != record['sha256']:
                raise RuntimeError('checksum failed: '+relative)
        for item in catalog['maps']:
            plan(catalog, item['key'], runtime, args.port)
        print(json.dumps({'verified_files': len(records), 'maps': len(catalog['maps'])}))
    elif args.command == '_run':
        run(catalog, args)
    elif args.command in ('status', 'stop'):
        state = read(state_path) if state_path.exists() else {'status': 'not started'}
        if state.get('package', str(PACKAGE)) != str(PACKAGE):
            raise RuntimeError('state belongs to another package')
        if args.command == 'stop' and live(state):
            cmdline = Path(f"/proc/{state['pid']}/cmdline").read_bytes().split(b'\0')
            if str(PACKAGE/'launch.py').encode() not in cmdline or state['token'].encode() not in cmdline or b'_run' not in cmdline:
                raise RuntimeError('supervisor identity changed; refusing to signal it')
            os.kill(state['pid'], signal.SIGTERM)
            deadline = time.monotonic()+45
            while live(state) and time.monotonic()<deadline:
                time.sleep(.2)
            if live(state):
                raise RuntimeError('supervisor has not completed shutdown')
            state = read(state_path)
        print(json.dumps(dict(state, supervisor_alive=live(state)), indent=2))
    else:
        item, _ = plan(catalog, args.map, runtime, args.port)
        if args.check:
            print(json.dumps({'map': item['key'], 'map_id': item['map_id'], 'runtime': str(runtime),
                              'camera': 'actual character', 'ui': args.ui, 'clients': args.clients, 'database': item['database']}))
            return
        if state_path.exists() and live(read(state_path)):
            raise RuntimeError('a package client is already running; stop it before switching maps')
        token = uuid.uuid4().hex
        runtime.parent.mkdir(parents=True, exist_ok=True)
        supervisor_log = runtime.parent/(runtime.name+'-supervisor.log')
        with supervisor_log.open('w') as log:
            child = subprocess.Popen([sys.executable, str(PACKAGE/'launch.py'), '_run', args.map,
                '--runtime', str(runtime), '--port', str(args.port), '--ui', args.ui, '--clients', str(args.clients), '--token', token],
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            if state_path.exists():
                state = read(state_path)
                if state.get('token') == token:
                    print(json.dumps(state, indent=2)); return
            if child.poll() is not None:
                raise RuntimeError('launcher exited; inspect '+str(supervisor_log))
            time.sleep(.1)
        raise RuntimeError('launcher did not publish state; inspect '+str(supervisor_log))


if __name__ == '__main__':
    main()
