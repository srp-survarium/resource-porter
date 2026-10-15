#!/usr/bin/env python3
"""Original 0.20e Chemical Plant (level_04/day) private session: one spectator client, UI on.

    play.py start | stop | status

The user drives it; nothing here sends input. Start refuses if this session's
prefix is already active or its port is busy. Stop ends only this session's
own process groups and this prefix's Wine server.

Root layout (prepared once, next to this script):
  client/resources, client/resources.db  -> original 0.20e game (read-only links)
  client/binaries/x86/survarium-play.exe -> prepare_client.py --build 020e --no-hook
      (Wine VRAM floor and the three direct-spectator UI callbacks only)
  client/binaries/x86/{bugtrap,d3d11,dxgi}.dll from the 0.20e reference package
  server/match-server-021d   reference server (factory-anomalies/prepare_reference.py)
      with src/setup.rs match options "level_04/day"
  sessions.txt "1 0";  PREFIX: copy of the reference wineprefix (ui on, windowed
  1280x720, r_texture_quality 2).
"""
import json, os, signal, socket, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PREFIX = Path('/tmp/chemical-020e-play-prefix')
PORT = 2280
WINE = Path('/nix/store/kd22mbkmwsnvfyn1w14r6xcwpw9fnwzv-wine-wow64-staging-11.8/bin/wine')
STATE = ROOT/'state.json'


def win(path):
    return 'Z:'+str(path).replace('/', '\\')


def env():
    e = {k: v for k, v in os.environ.items() if not k.startswith(('SRP_', 'WINEPREFIX', 'WAYLAND_DISPLAY'))}
    e.update(WINEPREFIX=str(PREFIX), DISPLAY=os.environ.get('DISPLAY', ':0'), WINEDEBUG='-all',
             WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
             VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json')
    e.pop('VK_DRIVER_FILES', None)
    return e


def prefix_active():
    for proc in Path('/proc').iterdir():
        if proc.name.isdigit() and int(proc.name) != os.getpid():
            try:
                if ('WINEPREFIX='+str(PREFIX)).encode() in (proc/'environ').read_bytes().split(b'\0'):
                    return int(proc.name)
            except (OSError, PermissionError):
                pass
    return None


def start():
    if prefix_active():
        sys.exit('this session is already running (prefix active); use stop first')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(('127.0.0.1', PORT))
    logs = ROOT/'logs'; logs.mkdir(exist_ok=True)
    groups = []
    for label, cmd, cwd in [
            ('server', [str(ROOT/'server/match-server-021d'), str(ROOT/'sessions.txt'), str(PORT), '1'], logs),
            ('client', [str(WINE), win(ROOT/'client/binaries/x86/survarium-play.exe'), f'-client=127.0.0.1:{PORT}',
                        '-spectator', '-localization=english', '-no_splash_screen',
                        '-minimum_resources_memory_size=768'], ROOT/'client/binaries/x86')]:
        log = (logs/(label+'.log')).open('w')
        p = subprocess.Popen(cmd, cwd=cwd, env=env(), stdout=log, stderr=subprocess.STDOUT,
                             stdin=subprocess.DEVNULL, start_new_session=True)
        groups.append({'label': label, 'pid': p.pid})
        if label == 'server':
            time.sleep(1)
    STATE.write_text(json.dumps({'groups': groups, 'port': PORT, 'prefix': str(PREFIX), 'started': time.time()}, indent=2)+'\n')
    print(f'started 0.20e level_04/day on 127.0.0.1:{PORT}; logs in {logs}')


def stop():
    state = json.loads(STATE.read_text()) if STATE.exists() else {'groups': []}
    for g in state['groups']:
        try:
            os.killpg(g['pid'], signal.SIGTERM)
        except ProcessLookupError:
            pass
    subprocess.run([str(WINE.parent/'wineserver'), '-k'], env=env(), timeout=20, check=False)
    subprocess.run([str(WINE.parent/'wineserver'), '-w'], env=env(), timeout=30, check=False)
    time.sleep(1)
    for g in state['groups']:
        try:
            os.killpg(g['pid'], signal.SIGKILL)
        except ProcessLookupError:
            pass
    STATE.unlink(missing_ok=True)
    left = prefix_active()
    print('stopped' if not left else f'warning: prefix still active (pid {left})')


def status():
    print(json.dumps({'state': json.loads(STATE.read_text()) if STATE.exists() else None,
                      'prefix_active_pid': prefix_active()}, indent=2))


if __name__ == '__main__':
    {'start': start, 'stop': stop, 'status': status}[sys.argv[1] if len(sys.argv) > 1 else 'status']()
