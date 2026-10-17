#!/usr/bin/env python3
"""Static load check of the v0.11 maps: one client, a private display, no input.

Usage: loadcheck_v011e.py PACKAGE OUT_DIR KEY [KEY ...] [--port N]

For each map key of the package (play-maps): starts it with the UI on,
requests actual-camera screenshots (mode 3, the HUD included) every few
seconds while it loads (the loading picture) and, once the gameplay camera
is live (SRP_REPORT_POSITION's player.json away from the latched loading
camera), three more 5 s apart (the HUD and its minimap); keeps the client
log; stops. Refuses while another Survarium client runs. Writes index.html.
"""
import json, math, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'day-map-objectives'))
from render_base_kits import bmp_to_png


def read(path):
    try: return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError): return None


def shot(control, status, path):
    request = int(time.time() * 1000) % 0xffffffff or 1
    tmp = Path(str(control) + '.tmp')
    tmp.write_text(f'{request} 3 0 0 0 0 0 1 65 1.7777777778 .05 500 0\n' + 'Z:' + str(path.resolve()).replace('/', '\\') + '\n')
    tmp.replace(control)
    end = time.monotonic() + 20
    while time.monotonic() < end:
        s = read(status)
        if s and s['captured_id'] == request:
            return True
        time.sleep(0.2)
    return False


def main():
    args = sys.argv[1:]
    port = 2278
    if '--port' in args:
        i = args.index('--port'); port = int(args[i + 1]); del args[i:i + 2]
    package, out, keys = Path(args[0]), Path(args[1]), args[2:]
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for key in keys:
        if subprocess.run(['/mnt/hdd/Projects/rev18-v011-maps/porter/tools/content/clients_running.sh']).returncode == 0:
            raise SystemExit('another Survarium client runs')
        runtime = Path(f'/tmp/rev18-loadcheck-{key}')
        subprocess.run([str(package / 'play-maps'), 'start', key, '--clients', '1', '--runtime', str(runtime), '--port', str(port), '--ui', 'on'], check=True)
        control = runtime / 'camera.txt'; status = Path(str(control) + '.status.json'); player = Path(str(control) + '.player.json')
        result = dict(key=key, loading=[], game=[], live_after_s=None, position=None)
        start = time.monotonic()
        try:
            while time.monotonic() - start < 420:
                r = read(player)
                if r and r.get('position') and math.dist(r['position'], [16, 10, 0]) > 1.0 and r.get('calls', 0) > 100:
                    result['live_after_s'] = round(time.monotonic() - start); result['position'] = r['position']; break
                if read(status) and len(result['loading']) < 12 and int(time.monotonic() - start) % 6 == 0:
                    bmp = out / f'{key}_loading_{len(result["loading"]):02d}.bmp'
                    if shot(control, status, bmp): result['loading'].append(bmp.name)
                time.sleep(1)
            if result['live_after_s'] is not None:
                time.sleep(8)
                for n in range(3):
                    bmp = out / f'{key}_game_{n}.bmp'
                    if shot(control, status, bmp): result['game'].append(bmp.name)
                    time.sleep(5)
        finally:
            subprocess.run([str(package / 'play-maps'), 'stop', '--runtime', str(runtime)])
            log = runtime / 'client.log'
            if log.exists(): (out / f'{key}_client.log').write_bytes(log.read_bytes())
        for group in ('loading', 'game'):
            pngs = []
            for name in result[group]:
                bmp = out / name
                if bmp.exists():
                    png = bmp.with_suffix('.png'); bmp_to_png(bmp, png); bmp.unlink(); pngs.append(png.name)
            result[group] = pngs
        rows.append(result)
        (out / 'results.json').write_text(json.dumps(rows, indent=1))
    html = ["<!doctype html><meta charset='utf-8'><title>v0.11 load check</title><style>body{font:14px sans-serif;background:#111;color:#ddd}img{width:480px;margin:4px}</style><h1>v0.11 maps: load check</h1>"]
    for r in rows:
        html.append(f"<h2>{r['key']}: gameplay camera after {r['live_after_s']} s at {r['position']}</h2>")
        html += [f"<img src='{n}' title='{n}'>" for n in r['loading'] + r['game']]
    (out / 'index.html').write_text('\n'.join(html))
    print(json.dumps(rows, indent=1))


if __name__ == '__main__':
    main()
