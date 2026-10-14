#!/usr/bin/env python3
"""Drive and observe the owned clients of a running gameplay session.

Subcommands (SLOT is 1 or 2; slot 1 is team 0, slot 2 team 1):
  status SLOT                  live character camera (position/forward)
  shot SLOT NAME               actual-view screenshot (mode 3) + status into --evidence
  look SLOT NAME X Y Z FX FY FZ  diagnostic camera override screenshot (mode 2)
  release SLOT                 return the render camera to the character (mode 0)
  drive SLOT "STEP;..."        focus the client through niri, then drive_client.exe
  aim SLOT X Y Z               turn the character to face a world point (closed loop)
  checkpoints PLAYERS BATTERIES  extract and decode every complete server checkpoint
Only windows/prefixes under --runtime are touched.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import time

WINE = '/nix/store/kd22mbkmwsnvfyn1w14r6xcwpw9fnwzv-wine-wow64-staging-11.8/bin/wine'
GRIM = '/nix/store/rqcxjxlzkxlgkj34avkjbp7bp9ynxnjq-grim-1.5.0/bin/grim'
PYTHON = '/nix/store/0if41r2dp11y0v833p5yrpgr8mdanqjk-python3-3.13.15-env/bin/python3'
HERE = Path(__file__).resolve().parent


def win(path):
    return 'Z:' + str(path).replace('/', '\\')


def slot_dir(runtime, slot):
    return runtime if slot == 1 else runtime / 'peer2'


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def request(runtime, slot, mode, output, position=(0, 0, 0), forward=(0, 0, 1), settle=300, timeout=20):
    control = slot_dir(runtime, slot) / 'camera.txt'
    status_path = Path(str(control) + '.status.json')
    ident = (int(time.time() * 1000) + slot) % 0xfffffff0 + 1
    temporary = Path(str(control) + '.tmp')
    temporary.write_text(f'{ident} {mode} {position[0]} {position[1]} {position[2]} '
                         f'{forward[0]} {forward[1]} {forward[2]} 65 1.7777777778 .05 500 {settle}\n'
                         + win(output) + '\n')
    temporary.replace(control)
    deadline = time.monotonic() + timeout
    if mode == 0:
        # The packaged hook reports the live character camera in mode 0 but
        # does not acknowledge the id; require fresh camera calls instead.
        before = (read_json(status_path) or {}).get('camera_calls', 0)
        while time.monotonic() < deadline:
            time.sleep(0.5)
            status = read_json(status_path)
            if status and status.get('error'):
                raise RuntimeError(status['error'])
            if status and status.get('camera_calls', 0) > before + 10:
                return status
        raise RuntimeError(f'camera status stalled on slot {slot}')
    while time.monotonic() < deadline:
        status = read_json(status_path)
        if status and status.get('error'):
            raise RuntimeError(status['error'])
        if status and status.get('captured_id') == ident:
            return status
        time.sleep(0.1)
    raise RuntimeError(f'camera request {mode} timed out on slot {slot}')


def niri_windows():
    return json.loads(subprocess.check_output(['niri', 'msg', '-j', 'windows'], text=True))


def client_pid(runtime, slot):
    state = read_json(runtime / 'state.json')
    return state['clients'][slot - 1]['pid']


def helper_run(runtime, slot, helper, script):
    env = dict(os.environ, WINEPREFIX=str(slot_dir(runtime, slot) / 'wineprefix'), WINEDEBUG='-all')
    exe = win(slot_dir(runtime, slot) / 'client/binaries/x86/survarium-capture.exe')
    return subprocess.run([WINE, str(helper), exe, script], env=env, text=True, capture_output=True, timeout=120)


def focus(runtime, slot, helper):
    # Xwayland hides the Wine process from niri, so learn which compositor
    # window belongs to this slot: only the owned prefix's helper can see
    # its own window as foreground.
    cache = runtime / 'slot-windows.json'
    known = read_json(cache) or {}
    ids = {w['id'] for w in niri_windows() if w.get('app_id') == 'survarium-capture.exe'}
    order = [known[str(slot)]] if known.get(str(slot)) in ids else []
    order += sorted(ids - set(order) - {v for k, v in known.items() if k != str(slot)})
    for window in order:
        subprocess.run(['niri', 'msg', 'action', 'focus-window', '--id', str(window)], check=True)
        time.sleep(0.4)
        if helper_run(runtime, slot, helper, 'wait 50').returncode == 0:
            known[str(slot)] = window
            cache.write_text(json.dumps(known))
            return window
    raise RuntimeError(f'no owned window for slot {slot}')


def drive(runtime, slot, script, helper):
    focus(runtime, slot, helper)
    result = helper_run(runtime, slot, helper, script)
    if result.returncode:
        raise RuntimeError(f'drive failed: {result.stdout} {result.stderr}')
    return json.loads(result.stdout.strip().splitlines()[-1])


def yaw(forward):
    return math.atan2(forward[0], forward[2])


def aim(runtime, slot, target, helper, cpr, record):
    """Closed-loop yaw and pitch toward a world point using the live camera."""
    cpr_y = cpr
    for attempt in range(8):
        status = request(runtime, slot, 0, runtime / 'aim.bmp')
        p, f = status['position'], status['forward']
        dx, dy, dz = (target[i] - p[i] for i in range(3))
        want_yaw = math.atan2(dx, dz)
        want_pitch = math.atan2(dy, math.hypot(dx, dz))
        pitch = math.asin(max(-1, min(1, f[1])))
        error = (want_yaw - yaw(f) + math.pi) % (2 * math.pi) - math.pi
        error_pitch = want_pitch - pitch
        if abs(error) < 0.04 and abs(error_pitch) < 0.04:
            break
        cx = max(-4000, min(4000, round(-error * cpr))) if abs(error) >= 0.04 else 0
        cy = max(-4000, min(4000, round(-error_pitch * cpr_y))) if abs(error_pitch) >= 0.04 else 0
        drive(runtime, slot, f'mouse {cx} {cy};wait 300', helper)
        after = request(runtime, slot, 0, runtime / 'aim.bmp')['forward']
        turned = (yaw(after) - yaw(f) + math.pi) % (2 * math.pi) - math.pi
        lifted = math.asin(max(-1, min(1, after[1]))) - pitch
        if cx and abs(turned) > 0.02:
            cpr = max(50, min(20000, abs(cx / turned)))
        if cy and abs(lifted) > 0.02:
            cpr_y = max(50, min(20000, abs(cy / lifted)))
        record({'command': 'aim-step', 'slot': slot, 'yaw_error': error, 'pitch_error': error_pitch,
                'counts': [cx, cy], 'turned': turned, 'lifted': lifted, 'counts_per_radian': [cpr, cpr_y]})
    return {'command': 'aim', 'slot': slot, 'target': target, 'status': status, 'counts_per_radian': [cpr, cpr_y]}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--runtime', type=Path, default=Path('/tmp/three-day-map-runtime'))
    ap.add_argument('--evidence', type=Path)
    ap.add_argument('--helper', type=Path, default=Path('/tmp/day-map-gameplay-20261014/drive_client.exe'))
    ap.add_argument('--decoder', type=Path)
    ap.add_argument('--counts-per-radian', type=float, default=None)
    ap.add_argument('command')
    ap.add_argument('args', nargs='*')
    a = ap.parse_args()
    runtime = a.runtime.resolve()
    log = (a.evidence / 'actions.jsonl') if a.evidence else None

    def record(row):
        row = dict(row, wall_time=time.time())
        print(json.dumps(row))
        if log:
            a.evidence.mkdir(parents=True, exist_ok=True)
            with log.open('a') as out:
                out.write(json.dumps(row) + '\n')

    c, args = a.command, a.args
    if c == 'status':
        slot = int(args[0])
        scratch = runtime / f'status-{slot}.bmp'
        record({'command': c, 'slot': slot, 'status': request(runtime, slot, 0, scratch)})
    elif c in ('shot', 'look'):
        slot, name = int(args[0]), args[1]
        a.evidence.mkdir(parents=True, exist_ok=True)
        output = (a.evidence / f'{name}.bmp').resolve()
        if c == 'shot':
            status = request(runtime, slot, 3, output, settle=200)
        else:
            p, f = list(map(float, args[2:5])), list(map(float, args[5:8]))
            status = request(runtime, slot, 2, output, p, f, settle=1500)
        record({'command': c, 'slot': slot, 'name': name, 'file': str(output), 'status': status})
    elif c == 'release':
        slot = int(args[0])
        record({'command': c, 'slot': slot, 'status': request(runtime, slot, 0, runtime / 'release.bmp')})
    elif c == 'drive':
        slot = int(args[0])
        record({'command': c, 'slot': slot, 'script': args[1], 'result': drive(runtime, slot, args[1], a.helper)})
    elif c == 'aim':
        slot, target = int(args[0]), list(map(float, args[1:4]))
        record(aim(runtime, slot, target, a.helper, a.counts_per_radian or 600.0, record))
    elif c == 'approach':
        # Walk toward a world XZ point until the camera is within STOP metres.
        slot, target, stop = int(args[0]), list(map(float, args[1:4])), float(args[4])
        for step in range(30):
            status = request(runtime, slot, 0, runtime / 'aim.bmp')
            p = status['position']
            distance = math.hypot(target[0] - p[0], target[2] - p[2])
            record({'command': 'approach-step', 'slot': slot, 'position': p, 'distance': distance})
            if distance <= stop:
                break
            aim(runtime, slot, [target[0], p[1], target[2]], a.helper, a.counts_per_radian or 600.0, record)
            hold = int(max(100, min(1500, (distance - stop) / 4.0 * 1000)))
            drive(runtime, slot, f'key w {hold};wait 400', a.helper)
        record({'command': c, 'slot': slot, 'target': target, 'final_distance': distance})
    elif c == 'grab':
        # Compositor screenshot of the owned window (actual HUD and view).
        slot, name = int(args[0]), args[1]
        window = focus(runtime, slot, a.helper)
        time.sleep(0.5)
        info = next(w for w in niri_windows() if w['id'] == window)
        x, y = info['layout']['tile_pos_in_workspace_view']
        w, h = info['layout']['window_size']
        a.evidence.mkdir(parents=True, exist_ok=True)
        output = a.evidence / f'{name}.png'
        subprocess.run([GRIM, '-g', f'{int(x)},{int(y + h - 720)} {int(w)}x720', str(output)], check=True)
        record({'command': c, 'slot': slot, 'name': name, 'file': str(output),
                'status': request(runtime, slot, 0, runtime / 'aim.bmp')})
    elif c == 'roster':
        # Client N -> UDP port (owned prefix) -> server roster slot -> team (slot % 2).
        import re
        sockets = subprocess.check_output(['ss', '-uapn'], text=True)
        admitted = dict(re.findall(r'Peer 127\.0\.0\.1:(\d+) admitted to roster slot (\d+)',
                                   (runtime / 'server.log').read_text()))
        result = {}
        for slot in (1, 2):
            prefix = str(slot_dir(runtime, slot) / 'wineprefix')
            for port, pids in re.findall(r':(\d+)\s+\S+\s+users:\((.*)\)', sockets):
                for pid in re.findall(r'pid=(\d+)', pids):
                    try:
                        env = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
                        exe = Path(f'/proc/{pid}/cmdline').read_bytes()
                    except OSError:
                        continue
                    if f'WINEPREFIX={prefix}'.encode() in env and b'survarium' in exe and port in admitted:
                        roster = int(admitted[port])
                        result[slot] = {'udp_port': int(port), 'roster_slot': roster, 'team': roster % 2}
        record({'command': c, 'clients': result})
    elif c == 'checkpoints':
        players, batteries = args[0], args[1]
        snapshots = a.evidence / 'checkpoints'
        if snapshots.exists():
            shutil.rmtree(snapshots)
        snapshots.mkdir(parents=True)
        data = (runtime / 'server-packets.log').read_bytes()
        frozen = a.evidence / 'server-packets.log'
        frozen.write_bytes(data[:data.rfind(b'\n') + 1])
        subprocess.run([PYTHON, str(HERE / 'extract_snapshots.py'), str(frozen), str(snapshots)], check=True)
        files = sorted(snapshots.glob('*.bin'), key=lambda p: (p.name.split('_')[1], int(p.name.split('_')[2])))
        out = subprocess.check_output([str(a.decoder), players, batteries, *map(str, files)], text=True)
        (a.evidence / 'decoded.jsonl').write_text(out)
        print(out[-4000:])
    else:
        raise SystemExit('unknown command')


if __name__ == '__main__':
    main()
