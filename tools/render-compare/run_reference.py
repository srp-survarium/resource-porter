#!/usr/bin/env python3
"""Capture matching 0.21d references and 0.26g ports with fixed cameras.

    run_reference.py REFERENCE_ROOT --poses POSES.json --frames N --output DIR --wine WINE \
        --viewport VIEWPORT.exe [--texture-quality 2]

REFERENCE_ROOT is the prepared private reference (`prepare_reference.py`):
client, wineprefix, server-source and sessions.txt. The run refuses an active
prefix or a busy port, restores the profile, and stops only its own prefix's
Wine server and its own server/client process groups.
"""
import argparse, hashlib, importlib.util, json, os, signal, socket, subprocess, time
from pathlib import Path


def win(path):
    return 'Z:'+str(path).replace('/', '\\')


def database_identity(root):
    path = (root/'client/resources.db').resolve(strict=True)
    stat = path.stat()
    identity = {'path': str(path), 'size': stat.st_size, 'mtime_ns': stat.st_mtime_ns,
                'ctime_ns': stat.st_ctime_ns, 'inode': stat.st_ino}
    cache = root/'capture-database-identity.json'
    if cache.exists():
        cached = json.loads(cache.read_text())
        if all(cached.get(key) == value for key, value in identity.items()):
            return cached
    with path.open('rb') as stream:
        identity['sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
    after = path.stat()
    if (after.st_size, after.st_mtime_ns, after.st_ctime_ns) != (stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns):
        raise RuntimeError('database changed while hashing')
    cache.write_text(json.dumps(identity, indent=2)+'\n')
    return identity


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root', type=Path); ap.add_argument('--poses', type=Path, required=True)
    ap.add_argument('--frames', type=int, default=1); ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--wine', type=Path, required=True); ap.add_argument('--viewport', type=Path, required=True)
    ap.add_argument('--texture-quality', default='2')
    ap.add_argument('--audit-fog', action='store_true', help='record the active retail fog shader constants without replacing shaders')
    ap.add_argument('--audit-render-target', action='store_true', help='also read the floating-point color target before the audited draw')
    ap.add_argument('--audit-after-draw', action='store_true', help='observe immediately after the selected draw instead of before it')
    ap.add_argument('--trace-after-audit', action='store_true', help='record small render-target snapshots after subsequent draws in the audited frame')
    ap.add_argument('--fog-digest', help='observe only these DXBC digests (one or up to eight comma-separated hashes)')
    ap.add_argument('--xdotool', required=True, type=Path)
    ap.add_argument('--project', required=True, choices=['level_03/evening','level_03/rain','level_06/evening','level_06/rain'])
    ap.add_argument('--build', choices=['021d', '026g'], default='021d')
    ap.add_argument('--server', type=Path, help='0.26 match-server executable')
    ap.add_argument('--map', type=int, choices=[167,168,176,177])
    a = ap.parse_args()
    if (a.audit_render_target or a.audit_after_draw) and not a.audit_fog:
        ap.error('--audit-render-target and --audit-after-draw require --audit-fog')
    if a.trace_after_audit and not (a.audit_fog and a.audit_render_target and a.audit_after_draw):
        ap.error('--trace-after-audit requires --audit-fog, --audit-render-target and --audit-after-draw')
    digests = a.fog_digest.split(',') if a.fog_digest else []
    if digests and (not a.audit_fog or len(digests) > 8 or any(len(d) != 32 or any(c not in '0123456789abcdef' for c in d) for d in digests)):
        ap.error('--fog-digest requires --audit-fog and up to eight 32-character lowercase hexadecimal digests')
    if a.build == '026g' and (a.server is None or a.map is None):
        ap.error('0.26 requires --server and --map')
    expected_map = {'level_03/evening':167, 'level_03/rain':168,
                    'level_06/evening':176, 'level_06/rain':177}[a.project]
    if a.build == '026g' and a.map != expected_map:
        ap.error('map ID does not match the requested project/weather')
    r = a.root.resolve(); prefix = r/'wineprefix'
    database = database_identity(r)
    poses = json.loads(a.poses.read_text())
    assert all(p.get('buffers') is False for p in poses) and len({p['name'] for p in poses}) == len(poses)
    frames = []
    for pose in poses:
        frames.append(dict(pose, name=pose['name']+('~f0' if a.frames > 1 else '')))
        frames += [dict(pose, name=f"{pose['name']}~f{i}", settle_ms=400) for i in range(1, a.frames)]
    for proc in Path('/proc').iterdir():
        if proc.name.isdigit() and int(proc.name) != os.getpid():
            try:
                if ('WINEPREFIX='+str(prefix)).encode() in (proc/'environ').read_bytes().split(b'\0'):
                    raise SystemExit('reference prefix is already active')
            except (OSError, PermissionError):
                pass
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(('127.0.0.1', 2264))
    spec = importlib.util.spec_from_file_location('capture', Path(__file__).resolve().parent/'capture.py')
    capture = importlib.util.module_from_spec(spec); spec.loader.exec_module(capture)
    cfg = prefix/'drive_c/users/sheep/Documents/Survarium/user.cfg'
    original_cfg = cfg.read_bytes()
    lines = [l for l in original_cfg.decode('cp1251').splitlines() if not l.startswith(('r_texture_quality ', 'ui '))]
    a.output.mkdir(parents=True, exist_ok=False)
    control = a.output/'camera.txt'
    env = {k: v for k, v in os.environ.items() if not k.startswith(('SRP_', 'WINEPREFIX'))}
    env.update(WINEPREFIX=str(prefix), DISPLAY=':0', WINEDEBUG='-all',
               WINEDLLOVERRIDES='mscoree,mshtml,qwave=;winemenubuilder.exe=d;d3d11,dxgi=n,b',
               VK_ICD_FILENAMES='/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json',
               DXVK_FRAME_RATE='60', SRP_CAPTURE_BUILD=a.build, SRP_REFERENCE_PROJECT=a.project, SRP_CAPTURE_CONTROL=win(control))
    env.pop('VK_DRIVER_FILES', None)
    if a.audit_fog:
        env['SRP_CAPTURE_AUDIT_FOG'] = '1'
        if a.audit_render_target:
            env['SRP_CAPTURE_AUDIT_RT'] = '1'
        if a.audit_after_draw:
            env['SRP_CAPTURE_AUDIT_AFTER'] = '1'
        if a.trace_after_audit:
            env['SRP_CAPTURE_TRACE_DRAWS'] = '1'
        if a.fog_digest:
            env['SRP_CAPTURE_FOG_DIGEST'] = a.fog_digest
    processes, logs = [], []
    crash_root = r/'client/binaries/x86'
    existing_crashes = set(crash_root.glob('SRP-crash-*.dmp'))
    try:
        cfg.write_text('\n'.join(lines+[f'r_texture_quality {a.texture_quality}', 'ui off'])+'\n', encoding='cp1251')
        commands = [
                ('server', [str(r/'server-source/target/release/match-server-021d'), str(r/'sessions.txt'), '2264', '1'], a.output),
                ('client', [str(a.wine), win(r/'client/binaries/x86/survarium-capture.exe'), '-client=127.0.0.1:2264',
                            '-spectator', '-localization=english', '-no_splash_screen',
                            '-minimum_resources_memory_size=768'], r/'client/binaries/x86')]
        ready_marker = 'static setup sent'
        if a.build == '026g':
            commands = [
                ('server', [str(a.server.resolve()), '--bind', '127.0.0.1:2264', '--players', '1',
                            '--start-players', '1', '--map', str(a.map), '--snapshots', '--checkpoints',
                            '--recover', '--clock-sync'], a.output),
                ('client', [str(a.wine), win(r/'client/binaries/x86/survarium-capture.exe'),
                            '-server=127.0.0.1:2264', '-localization=english', '-splash_screen',
                            '-mt_allocator=768', '-hash_check_interval_in_ms=50'], r/'client/binaries/x86')]
            ready_marker = 'All startup states acknowledged'
        for label, cmd, cwd in commands:
            log = (a.output/(label+'.log')).open('w'); logs.append(log)
            processes.append(subprocess.Popen(cmd, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                                              stdin=subprocess.DEVNULL, start_new_session=True))
        (a.output/'launch.json').write_text(json.dumps({'pids': [p.pid for p in processes], 'prefix': str(prefix),
                                                         'texture_quality': a.texture_quality, 'project': a.project, 'build': a.build,
                                                         'commands': [cmd for _, cmd, _ in commands]}, indent=2)+'\n')
        status_path = Path(str(control)+'.status.json')
        deadline, ready_since, previous = time.monotonic()+240, None, None
        window_shown = False
        last_progress = last_restore = time.monotonic()
        while time.monotonic() < deadline:
            new_crashes = set(crash_root.glob('SRP-crash-*.dmp')) - existing_crashes
            if new_crashes:
                raise RuntimeError('client crash during readiness: '+', '.join(str(p) for p in sorted(new_crashes)))
            if any(p.poll() is not None for p in processes):
                raise RuntimeError('reference process exited before scene readiness')
            rendered = status_path.exists() and json.loads(status_path.read_text()).get('present_calls', 0) > 0
            if not window_shown or not rendered:
                found = subprocess.run([str(a.xdotool), 'search', '--name', '^Survarium'],
                                       capture_output=True, text=True)
                for window in found.stdout.splitlines():
                    owner = subprocess.run([str(a.xdotool), 'getwindowpid', window],
                                           capture_output=True, text=True)
                    if owner.returncode or not owner.stdout.strip().isdigit():
                        continue
                    try:
                        entries = (Path('/proc') / owner.stdout.strip() / 'environ').read_bytes().split(b'\0')
                    except OSError:
                        continue
                    if ('WINEPREFIX='+str(prefix)).encode() not in entries:
                        continue
                    mapped = subprocess.run([str(a.xdotool), 'windowmap', '--sync', window],
                                            capture_output=True, timeout=5)
                    focused = subprocess.run([str(a.xdotool), 'windowfocus', window], capture_output=True)
                    window_shown = mapped.returncode == focused.returncode == 0
                    break
            status = json.loads(status_path.read_text()) if status_path.exists() else {}
            if status.get('error'):
                raise RuntimeError(status['error'])
            log = (a.output/'server.log').read_text(errors='replace')
            # Loading and gameplay have different directors in 0.26. The first
            # request relatches the active director, so readiness must observe
            # all apply_camera calls, not the retired loading director.
            counters = (status.get('director_calls', 0), status.get('present_calls', 0))
            now = time.monotonic()
            if previous != counters:
                last_progress = now
            if ready_marker in log and now-last_progress > 3 and now-last_restore > 5:
                # Mapping an X window does not necessarily clear Wine's iconic
                # state. Restore through Win32 when loading has finished but
                # the renderer remains suspended.
                subprocess.run([str(a.wine), win(a.viewport), '--exe', 'survarium-capture.exe'],
                               env=env, check=False, timeout=15)
                last_restore = now
            if ready_marker in log and min(counters) > 200:
                if previous and all(x > y for x, y in zip(counters, previous)):
                    ready_since = ready_since or time.monotonic()
                    if time.monotonic()-ready_since > 10:
                        (a.output/'readiness.json').write_text(json.dumps(status, indent=2)+'\n'); break
                else:
                    ready_since = None
            previous = counters
            time.sleep(.5)
        else:
            raise TimeoutError('scene readiness not established')
        subprocess.run([str(a.wine), win(a.viewport), '--exe', 'survarium-capture.exe'], env=env, check=True, timeout=15)
        warm = dict(frames[0], name='readiness-warmup', settle_ms=8000)
        capture.capture(control, [warm], a.output/'warmup', timeout=60)
        skipped, groups = {}, {}
        for pose in frames:
            groups.setdefault(pose['name'].split('~')[0], []).append(pose)
        for base, group in groups.items():
            try:
                capture.capture(control, group, a.output/'poses', timeout=40)
            except (TimeoutError, RuntimeError) as error:
                skipped[base] = str(error)
        (a.output/'skipped.json').write_text(json.dumps(skipped, indent=2)+'\n')
        (a.output/'final-status.json').write_bytes(status_path.read_bytes())
        (a.output/'profile-during.cfg').write_bytes(cfg.read_bytes())
        artifacts = {str(p.relative_to(a.output)): hashlib.sha256(p.read_bytes()).hexdigest()
                     for p in a.output.rglob('*') if p.is_file() and p.suffix in ('.bmp', '.json', '.jsonl', '.cfg', '.bin')}
        missing_fog = [p.name for p in (a.output/'poses').glob('*.bmp')
                       if a.audit_fog and not Path(str(p)+'.fog.json').exists()]
        missing_targets = [p.name for p in (a.output/'poses').glob('*.bmp')
                           if a.audit_render_target and not Path(str(p)+('.post-draw.bin' if a.audit_after_draw else '.pre-draw.bin')).exists()]
        wrong_fog = [p.name for p in (a.output/'poses').glob('*.fog.json')
                     if a.fog_digest and json.loads(p.read_text())['shader_digest'] not in digests]
        missing_traces = [p.name for p in (a.output/'poses').glob('*.bmp')
                          if a.trace_after_audit and not Path(str(p)+'.trace.jsonl').exists()]
        (a.output/'capture-manifest.json').write_text(json.dumps({
            'build': a.build, 'project': a.project, 'ui_command': 'ui off',
            'database': database,
            'audit_fog': a.audit_fog, 'audit_render_target': a.audit_render_target, 'missing_fog_audits': missing_fog,
            'missing_render_targets': missing_targets,
            'audit_after_draw': a.audit_after_draw,
            'trace_after_audit': a.trace_after_audit,
            'missing_draw_traces': missing_traces,
            'requested_fog_digest': a.fog_digest, 'wrong_fog_audits': wrong_fog,
            'expected_frames': len(frames), 'captured_frames': len(list((a.output/'poses').glob('*.bmp'))),
            'skipped': skipped, 'artifacts': artifacts,
            'runtime_sha256': {name: hashlib.sha256((r/'client/binaries/x86'/name).read_bytes()).hexdigest()
                               for name in ('survarium-capture.exe', 'camera_hook.dll', 'd3d11.dll', 'dxgi.dll')},
        }, indent=2)+'\n')
        if missing_fog:
            raise RuntimeError('requested fog audits were not captured: '+', '.join(missing_fog))
        if missing_targets:
            raise RuntimeError('requested pre-draw targets were not captured: '+', '.join(missing_targets))
        if wrong_fog:
            raise RuntimeError('requested experimental fog shader was not active: '+', '.join(wrong_fog))
        if missing_traces:
            raise RuntimeError('requested draw traces were not captured: '+', '.join(missing_traces))
    finally:
        subprocess.run([str(a.wine.parent/'wineserver'), '-k'], env=env, timeout=15, check=False)
        subprocess.run([str(a.wine.parent/'wineserver'), '-w'], env=env, timeout=30, check=False)
        for p in processes:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGTERM)
                try: p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid, signal.SIGKILL); p.wait(timeout=5)
        for log in logs: log.close()
        cfg.write_bytes(original_cfg)
    print(json.dumps({'output': str(a.output), 'frames': len(frames)}))


if __name__ == '__main__':
    main()
