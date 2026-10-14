#!/usr/bin/env python3
"""Capture ten views per authored battery spawn and build an offline gallery."""
import argparse
import html
import importlib.util
import json
import math
from pathlib import Path
import subprocess


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def poses(spawn):
    center = list(spawn.get('runtime_position', spawn['position']))
    center[1] += .12
    offsets = [(1.5*math.sin(i*math.pi/4), 1.0, 1.5*math.cos(i*math.pi/4)) for i in range(8)]
    offsets += [(0, .5, .65), (0, 3, 4)]
    result = []
    for i, offset in enumerate(offsets, 1):
        position = [a+b for a, b in zip(center, offset)]
        direction = [a-b for a, b in zip(center, position)]
        length = math.sqrt(sum(v*v for v in direction))
        result.append(dict(name=f'view-{i:02}', position=position,
                           forward=[v/length for v in direction], buffers=False,
                           fov=65, settle_ms=900))
    return result


def index(output, manifest):
    sections = []
    count = 0
    for item in manifest['maps']:
        cards = []
        for spawn in item['spawns']:
            key = f"{item['key']}/battery-{spawn['index']:02}"
            directory = output/key
            views = []
            for i in range(1, 11):
                name = f'view-{i:02}'
                if not (directory/(name+'.png')).exists():
                    continue
                count += 1
                views.append(f'<a href="{key}/{name}.png" target="_blank"><img loading="lazy" src="{key}/{name}-thumb.webp" alt="{name}"><span>{name}</span></a>')
            position = ', '.join(f'{v:.3f}' for v in spawn['position'])
            label = html.escape(str(spawn.get('id', spawn['index'])))
            cards.append(f'<details id="{item["key"]}-{spawn["index"]}" open><summary>Battery spawn {spawn["index"]:02} · {label} · ({position}) · {len(views)}/10 views</summary><div class="views">'+''.join(views)+'</div></details>')
        sections.append(f'<section id="{item["key"]}"><h2>{html.escape(item["title"])}</h2>'+''.join(cards)+'</section>')
    nav = ' · '.join(f'<a href="#{item["key"]}">{html.escape(item["title"])}</a>' for item in manifest['maps'])
    document = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Battery location gallery</title><style>
body{font:16px system-ui;background:#15181d;color:#eee;margin:24px}a{color:#9cceff}nav{position:sticky;top:0;background:#15181d;padding:12px 0;z-index:1}details{margin:18px 0;border:1px solid #495361;border-radius:6px;padding:12px}summary{cursor:pointer;font-weight:600}.views{display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:10px;margin-top:12px}.views img{width:100%;display:block}.views a{text-decoration:none}.views span{display:block;padding:4px}section{scroll-margin-top:55px}</style>
<h1>Battery locations — Factory and Chemical Plant</h1>'''
    document += f'<p>{count} screenshots saved. Ten viewpoints per authored spawn. Click a thumbnail for its full 1280 × 720 PNG. Coordinates and camera metadata are preserved alongside each image.</p>'
    document += '<p>These are diagnostic camera views. A normal match activates only a subset of candidate spawns; the manifest records the capture setup. Some views may be obstructed by nearby geometry. This gallery does not certify player access or pickup.</p>'
    document += '<nav>'+nav+'</nav>'+''.join(sections)+'</html>'
    (output/'index.html').write_text(document)
    return count


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('manifest', type=Path)
    ap.add_argument('output', type=Path)
    ap.add_argument('--map')
    ap.add_argument('--control', type=Path)
    ap.add_argument('--limit', type=int)
    ap.add_argument('--batch', type=int)
    args = ap.parse_args()
    manifest = read(args.manifest)
    args.output.mkdir(parents=True, exist_ok=True)
    write(args.output/'manifest.json', manifest)
    if args.map:
        if not args.control:
            ap.error('--map requires --control')
        script = Path(__file__).parents[1]/'render-compare/capture.py'
        spec = importlib.util.spec_from_file_location('capture', script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        item = next(m for m in manifest['maps'] if m['key'] == args.map)
        done = 0
        for spawn in item['spawns']:
            if args.batch is not None and spawn.get('diagnostic_batch') != args.batch:
                continue
            directory = args.output/item['key']/f"battery-{spawn['index']:02}"
            directory.mkdir(parents=True, exist_ok=True)
            write(directory/'spawn.json', spawn)
            for pose in poses(spawn):
                base = directory/pose['name']
                png = base.with_suffix('.png')
                bmp = base.with_suffix('.bmp')
                thumb = directory/(pose['name']+'-thumb.webp')
                if not png.exists():
                    # A BMP without an acknowledged metadata record is incomplete.
                    if bmp.exists() and not base.with_suffix('.json').exists():
                        bmp.unlink()
                    if not bmp.exists():
                        module.capture(args.control, [pose], directory, timeout=35)
                    subprocess.run(['magick', str(bmp), str(png)], check=True)
                    if not png.is_file() or png.stat().st_size < 100:
                        raise RuntimeError('PNG conversion failed')
                    bmp.unlink()
                if not thumb.exists():
                    subprocess.run(['magick', str(png), '-resize', '320x180', '-quality', '78', str(thumb)], check=True)
                done += 1
                if args.limit and done >= args.limit:
                    print(json.dumps({'screenshots': index(args.output, manifest)}), flush=True)
                    return
            print(json.dumps({'map': item['key'], 'spawn': spawn['index'], 'screenshots': index(args.output, manifest)}), flush=True)
    print(json.dumps({'screenshots': index(args.output, manifest)}))


if __name__ == '__main__':
    main()
