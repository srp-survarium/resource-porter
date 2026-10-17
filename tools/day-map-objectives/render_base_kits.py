#!/usr/bin/env python3
"""Static fixed-camera renders of the 194 base kits (charger + ammo crate), no input.

Usage: render_base_kits.py PACKAGE KIT_JSON [KIT_JSON ...] --runtime DIR --port N --output DIR

Starts the package's chemical-011e-day session in its own runtime (refuses
when any other Survarium client is running), waits for the map to render,
then requests mode-2 camera overrides (no buffer readback, no input): per
kit a front view from the room side and two views along the wall, all aimed
at the kit's middle 0.8 m above the floor. Stops the session, converts the
BMPs to PNG (ImageMagick) and writes index.html. KIT_JSON is the output of
place_base_kit.py.
"""
import argparse, json, math, struct, subprocess, time, zlib
from pathlib import Path

def read(path):
    try: return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError): return None

def bmp_to_png(bmp, png):
    """A 24/32-bit uncompressed BMP (the hook's captures) as an RGB PNG."""
    d = bmp.read_bytes()
    offset, = struct.unpack_from('<I', d, 10)
    width, height, _, bits = struct.unpack_from('<iiHH', d, 18)
    step = bits // 8; stride = (width * step + 3) & ~3
    rows = []
    for y in range(abs(height)):
        src = d[offset + (abs(height) - 1 - y if height > 0 else y) * stride:][:width * step]
        row = bytearray(width * 3)
        row[0::3] = src[2::step]; row[1::3] = src[1::step]; row[2::3] = src[0::step]
        rows.append(b'\0' + bytes(row))
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    png.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, abs(height), 8, 2, 0, 0, 0))
                    + chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b''))


def poses(kit):
    u, n = kit['wall_direction'], kit['room_normal']
    a, b = kit['charger']['position'], kit['crate']['position']
    c = [(a[0]+b[0])/2, (a[1]+b[1])/2 + 0.8, (a[2]+b[2])/2]
    room = kit['walkway_left'] + 0.6                      # free distance in front of the kit
    front = max(1.6, min(3.2, room - 0.4))
    out = []
    views = {'front': (0.0, 1.0, front, 0.9, 0.0), 'along_wall_a': (1.0, 0.35, 4.0, 0.9, 0.0), 'along_wall_b': (-1.0, 0.35, 4.0, 0.9, 0.0),
             # Floor level, close: the eye 0.3 m above the floor, aimed at the kit's base, to show contact.
             'floor_contact': (0.25, 1.0, min(1.8, room - 0.3), -0.5, -0.6)}
    for name, (du, dn, dist, up, aim) in views.items():
        d = [u[0]*du + n[0]*dn, u[1]*du + n[1]*dn]; l = math.hypot(*d); d = [d[0]/l, d[1]/l]
        eye = [c[0] + d[0]*dist, c[1] + up, c[2] + d[1]*dist]
        target = [c[0], c[1] + aim, c[2]]
        f = [target[0]-eye[0], target[1]-eye[1], target[2]-eye[2]]; fl = math.sqrt(sum(v*v for v in f))
        out.append((f"{kit['label']}_{name}", eye, [v/fl for v in f]))
    return out

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('package', type=Path); ap.add_argument('kits', type=Path, nargs='+')
    ap.add_argument('--runtime', type=Path, required=True); ap.add_argument('--port', type=int, required=True)
    ap.add_argument('--output', type=Path, required=True); ap.add_argument('--load-timeout', type=int, default=420)
    a = ap.parse_args()
    if subprocess.run(['/mnt/hdd/Projects/rev18-v011-maps/porter/tools/content/clients_running.sh']).returncode == 0:
        raise SystemExit('another Survarium client is running; refusing to render')
    a.output.mkdir(parents=True, exist_ok=True)
    play = [str(a.package/'play-maps')]
    subprocess.run(play + ['start', 'chemical-011e-day', '--clients', '1', '--runtime', str(a.runtime), '--port', str(a.port), '--ui', 'off'], check=True)
    control = a.runtime/'camera.txt'; status = Path(str(control) + '.status.json')
    shots = []
    try:
        # The gameplay camera is live once the position report's busiest director
        # is away from the latched loading camera at (16, 10, 0) (SRP_REPORT_POSITION=1).
        player = Path(str(control) + '.player.json')
        deadline = time.monotonic() + a.load_timeout; stable = 0
        while time.monotonic() < deadline:
            r = read(player)
            live = bool(r and r.get('position') and math.dist(r['position'], [16, 10, 0]) > 1.0 and r.get('calls', 0) > 100)
            stable = stable + 1 if live else 0
            if stable >= 5: break
            time.sleep(1)
        else:
            raise SystemExit('the gameplay camera did not appear in time')
        time.sleep(20)                                    # streaming and LOD settle
        for kit_path in a.kits:
            for name, eye, forward in poses(json.loads(kit_path.read_text())):
                request = int(time.time()*1000) % 0xffffffff or 1
                bmp = (a.output/(name + '.bmp')).resolve()
                tmp = Path(str(control) + '.tmp')
                tmp.write_text(f'{request} 2 {eye[0]:.4f} {eye[1]:.4f} {eye[2]:.4f} {forward[0]:.5f} {forward[1]:.5f} {forward[2]:.5f} '
                               f'70 1.7777777778 .05 500 3000\n' + 'Z:' + str(bmp).replace('/', '\\') + '\n')
                tmp.replace(control)
                end = time.monotonic() + 40
                while time.monotonic() < end:
                    s = read(status)
                    if s and s['error']: raise SystemExit(s['error'])
                    if s and s['captured_id'] == request: break
                    time.sleep(0.2)
                else:
                    raise SystemExit(f'{name}: capture timeout')
                shots.append(dict(name=name, eye=[round(v, 3) for v in eye], forward=[round(v, 4) for v in forward]))
        control.write_text('1 0 0 0 0 0 0 1 70 1.7777777778 .05 500 0\nZ:\\nul\n')
    finally:
        subprocess.run(play + ['stop', '--runtime', str(a.runtime)])
    rows = []
    for shot in shots:
        bmp = a.output/(shot['name'] + '.bmp'); png = bmp.with_suffix('.png')
        bmp_to_png(bmp, png); bmp.unlink()
        rows.append(f"<figure><img src='{png.name}' width='640'><figcaption>{shot['name']}: eye {shot['eye']}, forward {shot['forward']}</figcaption></figure>")
    (a.output/'index.html').write_text("<!doctype html><meta charset='utf-8'><title>194 base kits</title>"
        "<style>body{font:14px sans-serif;background:#111;color:#ddd}figure{display:inline-block;margin:8px}</style>"
        "<h1>194 base kits: charger + ammo crate</h1>" + '\n'.join(rows))
    (a.output/'shots.json').write_text(json.dumps(shots, indent=1))
    print(a.output/'index.html')

if __name__ == '__main__':
    main()
