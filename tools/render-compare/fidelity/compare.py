#!/usr/bin/env python3
"""Perceptual fidelity comparison of original and ported frames, with a gallery.

    compare.py --map NAME=POSES.json:REFERENCE_DIR:PORT_DIR[:VARIANT_DIR] ... --output DIR [--manifest JSON]

Each directory holds `<pose>~f<k>.bmp|.png` frames with their capture records.
For every pose, all frames on both sides must report the requested camera
(position, forward, field of view; aspect from the request), or the pose is
rejected. The per-pixel median over the frames removes unsynchronised grain.
NVIDIA FLIP (LDR, 67 ppd) on the medians gives the per-pixel error map; full
frame, shadow (original luma < .18), sunlit (>= .35) and non-sky region means
are reported with RGB MAE and luma ratio. Sky (top-connected bright, blue
pixels in both frames) is flagged non-comparable: clouds are not phase
synchronised. Needs numpy, Pillow and flip_evaluator.
"""
import argparse
import html
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

import flip_evaluator as flip

LUMA = np.array([.2126, .7152, .0722])
TOL = 2e-3


def frames(directory, name):
    out = []
    for k in range(64):
        for suffix in ('.bmp', '.png'):
            path = directory/f'{name}~f{k}{suffix}'
            if path.exists():
                record = json.loads((directory/f'{name}~f{k}.json').read_text())
                out.append((np.asarray(Image.open(path).convert('RGB'), dtype=np.uint8), record))
                break
        else:
            break
    return out


def camera_ok(record, pose):
    """None when the capture used exactly the requested camera, else the reason."""
    status = record.get('status', {})
    if status.get('error'):
        return 'capture error'
    unit = lambda v: [x/math.sqrt(sum(y*y for y in v)) for x in v]
    if any(abs(x-y) > TOL for x, y in zip(status['position'], pose['position'])):
        return f"position {status['position']} != {pose['position']}"
    if any(abs(x-y) > TOL for x, y in zip(unit(status['forward']), unit(pose['forward']))):
        return f"forward {status['forward']} != {pose['forward']}"
    if abs(status['fov']-pose['fov']) > 1e-3:
        return f"fov {status['fov']} != {pose['fov']}"
    if (status.get('width'), status.get('height')) != (1280, 720):
        return f"size {status.get('width')}x{status.get('height')}"
    for field in ('position', 'forward', 'fov', 'aspect', 'near', 'far'):
        if record['pose'].get(field) != pose.get(field):
            return f'requested {field} differs from the pose set'
    return None


def sky_mask(a, b):
    def skyish(x):
        x = x.astype(float)/255
        return (x@LUMA > .30) & (x[..., 2] > x[..., 0]+.02)
    s = skyish(a) & skyish(b)
    mask = np.zeros_like(s)
    run = np.ones(s.shape[1], bool)
    for y in range(s.shape[0]):
        run &= s[y]
        mask[y] = run
        if not run.any():
            break
    return mask


def jpeg(array, path, quality=88):
    Image.fromarray(array).save(path, quality=quality, optimize=True)


def colour_map(err):
    stops = np.array([[0, 0, 4], [80, 18, 123], [182, 54, 121], [251, 136, 97], [252, 253, 191]], float)
    x = np.clip(err, 0, 1)*(len(stops)-1)
    i = np.minimum(x.astype(int), len(stops)-2)
    t = (x-i)[..., None]
    return (stops[i]*(1-t)+stops[i+1]*t).astype(np.uint8)


def stats(err, ref, port, mask):
    if not mask.any():
        return None
    r, p = ref.astype(float)/255, port.astype(float)/255
    return {'pixels': int(mask.sum()), 'flip': float(err[mask].mean()),
            'rgb_mae': float(np.abs(r-p)[mask].mean()),
            'luma_ratio': float((p@LUMA)[mask].mean()/max((r@LUMA)[mask].mean(), 1e-6))}


def suspect(row):
    s, l = row['regions'].get('shadow'), row['regions'].get('sunlit')
    notes = []
    if s and s['luma_ratio'] < .8:
        notes.append('shade too dark (ambient/AO)')
    if s and s['luma_ratio'] > 1.25:
        notes.append('shade too bright')
    if l and l['luma_ratio'] < .8:
        notes.append('sunlit too dark (sun/exposure/tone)')
    if l and l['luma_ratio'] > 1.25:
        notes.append('sunlit too bright')
    if row['sky_fraction'] > .25:
        notes.append('large sky share (clouds not synchronised)')
    if not notes:
        notes.append('local material/geometry differences; see heatmap tiles')
    return '; '.join(notes)


def tiles(err, valid, count=3, size=200):
    h, w = err.shape
    picks = []
    e = np.where(valid, err, 0)
    for _ in range(count):
        best, where = -1, None
        for y in range(0, h-size+1, size//2):
            for x in range(0, w-size+1, size//2):
                if any(abs(x-px) < size and abs(y-py) < size for px, py in picks):
                    continue
                v = e[y:y+size, x:x+size].mean()
                if v > best:
                    best, where = v, (x, y)
        if where:
            picks.append(where)
    return picks


PAGE = '''<!doctype html><html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{background:#111;color:#ddd;font:14px sans-serif;margin:16px}}a{{color:#8cf}}
table{{border-collapse:collapse}}td,th{{border:1px solid #333;padding:3px 6px;text-align:right}}
td.l,th.l{{text-align:left}}.flip{{position:relative;display:inline-block;cursor:pointer}}
.flip img{{display:block;max-width:100%}}.lab{{position:absolute;left:6px;top:6px;background:#000a;padding:2px 6px}}
.row img{{width:640px;margin-right:4px}}.crop img{{width:400px;image-rendering:pixelated}}
textarea{{width:640px;height:60px;background:#222;color:#ddd}}</style></head><body>{body}</body></html>'''


def view_page(out, map_name, row, have_variant):
    n = row['name']
    states = ['original', 'port'] + (['port (probe-specular look)'] if have_variant else [])
    files = [f'{n}.orig.jpg', f'{n}.port.jpg'] + ([f'{n}.variant.jpg'] if have_variant else [])
    js = json.dumps(files)
    checks = ['shine / gloss', 'brightness / ambient', 'colour', 'material / texture', 'shadow', 'sky / fog', 'geometry / missing object', 'other']
    body = [f'<p><a href="../index.html">index</a> · {html.escape(map_name)} · <b>{html.escape(n)}</b> ({row["kind"]})'
            f' · FLIP {row["regions"]["full"]["flip"]:.3f} · {html.escape(row["suspect"])}</p>',
            '<p>Click (or hold Space) to flip between states in place; hover shows the label.</p>',
            f'<div class="flip" id="f"><img id="fi" src="{files[0]}"><span class="lab" id="fl">{states[0]}</span></div>',
            '<div class="row"><img src="%s"><img src="%s"></div><p>original | port</p>' % (files[0], files[1]),
            f'<div class="row"><img src="{n}.flip.jpg"></div><p>FLIP error map (black = identical, yellow = most visible)</p>',
            '<div class="crop">'+''.join(f'<img src="{n}.crop{i}.orig.jpg"><img src="{n}.crop{i}.port.jpg"><br>' for i in range(len(row['tiles'])))+
            '</div><p>highest-error tiles: original | port (pixel zoom)</p>',
            '<table><tr><th class="l">region</th><th>pixels</th><th>FLIP</th><th>RGB MAE</th><th>luma ratio</th></tr>' +
            ''.join(f'<tr><td class="l">{k}</td><td>{v["pixels"]}</td><td>{v["flip"]:.4f}</td><td>{v["rgb_mae"]:.4f}</td><td>{v["luma_ratio"]:.3f}</td></tr>'
                    for k, v in row['regions'].items() if v) + '</table>',
            '<h3>Checklist</h3>' + ''.join(f'<label><input type="checkbox" data-k="{c}"> {c}</label><br>' for c in checks),
            '<textarea id="notes" placeholder="notes"></textarea><br><button id="exp">export all notes (JSON)</button>',
            f'''<script>
const files={js}, states={json.dumps(states)}; let i=0;
const fi=document.getElementById('fi'), fl=document.getElementById('fl');
function show(k){{i=k%files.length; fi.src=files[i]; fl.textContent=states[i];}}
document.getElementById('f').onclick=()=>show(i+1);
document.addEventListener('keydown',e=>{{if(e.code==='Space'){{e.preventDefault();show(1);}}}});
document.addEventListener('keyup',e=>{{if(e.code==='Space')show(0);}});
const key='fidelity:{map_name}:{n}';
function load(){{try{{return JSON.parse(localStorage.getItem(key)||'{{}}');}}catch(e){{return {{}};}}}}
function save(d){{try{{localStorage.setItem(key,JSON.stringify(d));}}catch(e){{}}}}
const d=load(); document.querySelectorAll('input[data-k]').forEach(c=>{{c.checked=!!(d.checks||{{}})[c.dataset.k];
 c.onchange=()=>{{const x=load(); x.checks=x.checks||{{}}; x.checks[c.dataset.k]=c.checked; save(x);}};}});
const t=document.getElementById('notes'); t.value=d.notes||''; t.oninput=()=>{{const x=load(); x.notes=t.value; save(x);}};
document.getElementById('exp').onclick=()=>{{const all={{}}; try{{for(let j=0;j<localStorage.length;j++){{const k=localStorage.key(j);
 if(k.startsWith('fidelity:'))all[k]=JSON.parse(localStorage.getItem(k));}}}}catch(e){{}}
 const a=document.createElement('a'); a.href=URL.createObjectURL(new Blob([JSON.stringify(all,null,1)],{{type:'application/json'}}));
 a.download='fidelity-notes.json'; a.click();}};
</script>''']
    (out/f'{n}.html').write_text(PAGE.format(title=f'{map_name} {n}', body='\n'.join(body)))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--map', action='append', required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--manifest', type=Path)
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    report = {'scope': __doc__.strip(), 'maps': {}}
    for item in a.map:
        name, spec = item.split('=', 1)
        parts = spec.split(':')
        poses = json.loads(Path(parts[0]).read_text())
        ref_dir, port_dir = Path(parts[1]), Path(parts[2])
        var_dir = Path(parts[3]) if len(parts) > 3 else None
        out = a.output/name
        out.mkdir(exist_ok=True)
        rows, rejected = [], {}
        for pose in poses:
            n = pose['name']
            sides = {'ref': frames(ref_dir, n), 'port': frames(port_dir, n)}
            if var_dir:
                sides['variant'] = frames(var_dir, n)
            problem = None
            for side, fr in sides.items():
                if not fr:
                    problem = f'{side}: no frames'
                for _, rec in fr:
                    problem = problem or (camera_ok(rec, pose) and f'{side}: {camera_ok(rec, pose)}')
            if problem:
                rejected[n] = problem
                continue
            med = {s: np.median(np.stack([f for f, _ in fr]), axis=0).astype(np.uint8) for s, fr in sides.items()}
            err = np.asarray(flip.evaluate(med['ref'].astype(np.float32)/255, med['port'].astype(np.float32)/255,
                                           'LDR', applyMagma=False)[0], dtype=np.float32)
            err = err.reshape(err.shape[0], err.shape[1], -1)[..., 0]
            sky = sky_mask(med['ref'], med['port'])
            luma = (med['ref'].astype(float)/255)@LUMA
            masks = {'full': np.ones_like(sky), 'non_sky': ~sky, 'shadow': (~sky) & (luma < .18), 'sunlit': (~sky) & (luma >= .35)}
            row = {'name': n, 'kind': pose.get('kind', ''), 'frames': {s: len(fr) for s, fr in sides.items()},
                   'sky_fraction': float(sky.mean()),
                   'regions': {k: stats(err, med['ref'], med['port'], m) for k, m in masks.items()}}
            if 'variant' in med:
                ev = np.asarray(flip.evaluate(med['ref'].astype(np.float32)/255, med['variant'].astype(np.float32)/255,
                                              'LDR', applyMagma=False)[0], dtype=np.float32)
                ev = ev.reshape(ev.shape[0], ev.shape[1], -1)[..., 0]
                row['variant_regions'] = {k: stats(ev, med['ref'], med['variant'], m) for k, m in masks.items()}
            row['suspect'] = suspect(row)
            row['tiles'] = tiles(err, ~sky)
            jpeg(med['ref'], out/f'{n}.orig.jpg'); jpeg(med['port'], out/f'{n}.port.jpg')
            if 'variant' in med:
                jpeg(med['variant'], out/f'{n}.variant.jpg')
            jpeg(colour_map(err), out/f'{n}.flip.jpg')
            for i, (x, y) in enumerate(row['tiles']):
                for s, tag in (('ref', 'orig'), ('port', 'port')):
                    jpeg(med[s][y:y+200, x:x+200], out/f'{n}.crop{i}.{tag}.jpg', 92)
            np.save(out/f'{n}.flip.npy', err.astype(np.float16))
            view_page(out, name, row, 'variant' in med)
            rows.append(row)
            print(name, n, f"flip {row['regions']['full']['flip']:.4f}", flush=True)
        report['maps'][name] = {'views': rows, 'rejected': rejected}
    (a.output/'report.json').write_text(json.dumps(report, indent=1)+'\n')
    manifest = json.loads(a.manifest.read_text()) if a.manifest else {}
    body = ['<h1>Survarium port fidelity</h1>',
            '<p>Original donor client vs 0.26 port, matched cameras, per-pixel median of the captured frames, NVIDIA FLIP (LDR). '
            'Open a view to flip original/port in place, compare side by side and see the error map and highest-error tiles. '
            'Notes and checklists are stored in this browser; export them from any view page.</p>']
    if manifest:
        body.append('<details><summary>manifest (hashes and settings)</summary><pre>'+html.escape(json.dumps(manifest, indent=1))+'</pre></details>')
    worst = []
    for name, m in report['maps'].items():
        rows = sorted(m['views'], key=lambda r: -r['regions']['full']['flip'])
        def mean(key, region):
            v = [r[key][region]['flip'] for r in rows if r.get(key, {}).get(region)]
            return sum(v)/len(v) if v else float('nan')
        body.append(f'<h2>{html.escape(name)}</h2><p>{len(rows)} views, {len(m["rejected"])} rejected. Mean FLIP: full {mean("regions","full"):.4f}, '
                    f'non-sky {mean("regions","non_sky"):.4f}, shadow {mean("regions","shadow"):.4f}, sunlit {mean("regions","sunlit"):.4f}'
                    + (f'; probe-specular look full {mean("variant_regions","full"):.4f}' if any("variant_regions" in r for r in rows) else '') + '</p>')
        if m['rejected']:
            body.append('<p>Rejected: '+html.escape('; '.join(f'{k}: {v}' for k, v in m['rejected'].items()))+'</p>')
        body.append('<table><tr><th class="l">view</th><th class="l">kind</th><th>FLIP</th><th>non-sky</th><th>shadow</th><th>sunlit</th>'
                    '<th>shadow luma</th><th>sunlit luma</th><th>sky</th><th class="l">suspected cause</th></tr>')
        for r in rows:
            g = lambda reg, k: (f"{r['regions'][reg][k]:.3f}" if r['regions'].get(reg) else '–')
            body.append(f'<tr><td class="l"><a href="{name}/{r["name"]}.html">{html.escape(r["name"])}</a></td><td class="l">{r["kind"]}</td>'
                        f'<td>{g("full","flip")}</td><td>{g("non_sky","flip")}</td><td>{g("shadow","flip")}</td><td>{g("sunlit","flip")}</td>'
                        f'<td>{g("shadow","luma_ratio")}</td><td>{g("sunlit","luma_ratio")}</td><td>{r["sky_fraction"]:.2f}</td>'
                        f'<td class="l">{html.escape(r["suspect"])}</td></tr>')
            worst.append((r['regions']['non_sky']['flip'] if r['regions'].get('non_sky') else 0, name, r))
        body.append('</table>')
    worst.sort(key=lambda w: -w[0])
    body.insert(2 + bool(manifest), '<h2>Worst views (non-sky FLIP)</h2><ol>' + ''.join(
        f'<li><a href="{m}/{r["name"]}.html">{html.escape(m)} / {html.escape(r["name"])}</a> {v:.4f}: {html.escape(r["suspect"])}</li>'
        for v, m, r in worst[:15]) + '</ol>')
    (a.output/'index.html').write_text(PAGE.format(title='Port fidelity', body='\n'.join(body)))
    print(json.dumps({m: {'views': len(v['views']), 'rejected': len(v['rejected'])} for m, v in report['maps'].items()}))


if __name__ == '__main__':
    main()
