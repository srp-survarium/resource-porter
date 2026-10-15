#!/usr/bin/env python3
"""Static, input-free respawn gallery: two camera-override views per point.

Usage: gallery_respawns.py AUDIT.json RUNTIME OUTPUT_DIR [--title T] [--only i,j,...]
Needs a running session of the map (one client is enough). View 1 is the
spawn's eye (support + 1.55 m) along its authored facing, pitched slightly
down. View 2 looks at the spawn from 3.5 m behind and 2.5 m above. The render
camera is overridden only for the capture; the character is not moved and no
input is sent. Writes PNGs, the camera receipts and index.html.
"""
import argparse
import html
import json
import math
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gameplay_ctl import request  # noqa: E402


def norm(v):
    length = math.sqrt(sum(c * c for c in v))
    return [c / length for c in v]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('audit', type=Path)
    ap.add_argument('runtime', type=Path)
    ap.add_argument('output', type=Path)
    ap.add_argument('--title', default='Respawn points')
    ap.add_argument('--only')
    ap.add_argument('--slot', type=int, default=1)
    a = ap.parse_args()
    audit = json.loads(a.audit.read_text())
    only = {int(i) for i in a.only.split(',')} if a.only else None
    a.output.mkdir(parents=True, exist_ok=True)
    rows = []
    for point in audit['points']:
        if only is not None and point['index'] not in only:
            continue
        x, y, z = point['position']
        feet = point['support']['height'] if point['support'] else y
        fx, fz = -math.sin(point['yaw']), math.cos(point['yaw'])
        views = {
            'eye': ([x, feet + 1.55, z], norm([fx, -0.15, fz])),
            'overview': ([x - 3.5 * fx, feet + 2.5, z - 3.5 * fz], norm([3.5 * fx, -1.5, 3.5 * fz])),
        }
        shots = {}
        for name, (position, forward) in views.items():
            stem = f"{point['index']:02d}-{name}"
            bmp = (a.output / f'{stem}.bmp').resolve()
            status = request(a.runtime.resolve(), a.slot, 2, bmp, position, forward, settle=900)
            png = bmp.with_suffix('.png')
            subprocess.run(['magick', str(bmp), '-resize', '50%', str(png)], check=True)
            bmp.unlink()
            shots[name] = dict(file=png.name, position=position, forward=forward, status=status)
        rows.append(dict(point, shots=shots))
        print(json.dumps({'index': point['index'], 'name': point['name'], 'problems': point['problems']}), flush=True)
    (a.output / 'gallery.json').write_text(json.dumps(dict(title=a.title, points=rows), indent=1) + '\n')
    cards = []
    for r in rows:
        flag = '<b class="bad">' + html.escape('; '.join(r['problems'])) + '</b>' if r['problems'] else '<span class="ok">audit ok</span>'
        cards.append(f'''<figure><figcaption>#{r['index']} team {r['team']} — {html.escape(r['name'])}<br>
pos {[round(c, 2) for c in r['position']]} yaw {r['yaw']:.2f}<br>{flag}</figcaption>
<img src="{r['shots']['eye']['file']}"><img src="{r['shots']['overview']['file']}"></figure>''')
    (a.output / 'index.html').write_text(f'''<!doctype html><meta charset="utf-8"><title>{html.escape(a.title)}</title>
<style>body{{font:14px sans-serif;background:#111;color:#ddd}}figure{{display:inline-block;margin:8px;vertical-align:top}}
img{{width:480px;display:block;margin-top:4px}}.bad{{color:#f77}}.ok{{color:#7c7}}</style>
<h1>{html.escape(a.title)}</h1><p>Static camera-override views: spawn eye along its facing, and from behind/above.
No character input. Flags come from audit_respawns.py.</p>{''.join(cards)}''')


if __name__ == '__main__':
    main()
