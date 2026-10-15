#!/usr/bin/env python3
"""Rebuild the respawn gallery pages from current audit flags and native checks.

Usage: respawn_gallery_index.py GALLERY_ROOT AUDIT_DIR VALIDATION_DIR
GALLERY_ROOT/<map>/gallery.json comes from gallery_respawns.py; AUDIT_DIR/<map>.json
from audit_respawns.py; VALIDATION_DIR holds validate_spawns/validate_spawn_single runs.
"""
import glob, html, json, math, os, sys
from pathlib import Path

root, audits, runs = map(Path, sys.argv[1:4])
EYE = 1.47
native = {}
for d in sorted(glob.glob(str(runs / '19[456]-*'))):
    name = os.path.basename(d)
    if 'stalled' in name or 'iso' in name or not os.path.exists(f'{d}/actions.jsonl'): continue
    parts = name.split('-'); m = parts[0]
    audit = json.loads((audits / f'{m}.json').read_text())['points']
    rows = [json.loads(l) for l in open(f'{d}/actions.jsonl')]
    samples = {}
    for r in rows:
        if r['command'] == 'status': samples.setdefault(str(r['slot']), []).append(r['status']['position'])
    if parts[1] == 'single':
        assign = {'1': audit[int(parts[2])]}
    else:
        roster = next(r for r in rows if r['command'] == 'roster')['clients']
        by_team = {1: audit[int(parts[1])], 0: audit[int(parts[2])]}
        assign = {slot: by_team[info['team']] for slot, info in roster.items()}
    for slot, pt in assign.items():
        s = samples.get(slot, [])
        if len(s) < 2: continue
        native[(m, pt['index'])] = dict(moved=round(math.hypot(s[-1][0] - pt['position'][0], s[-1][2] - pt['position'][2]), 2),
                                        feet=round(s[-1][1] - EYE, 2), authored=pt['position'][1], drift=round(math.dist(s[0], s[1]), 3), run=name)
summary = []
for gallery in sorted(root.glob('19[456]/gallery.json')):
    m = gallery.parent.name; g = json.loads(gallery.read_text())
    audit = {p['index']: p for p in json.loads((audits / f'{m}.json').read_text())['points']}
    cards, flagged = [], 0
    for r in g['points']:
        a = audit[r['index']]; nat = native.get((m, r['index']))
        flag = '<b class="bad">' + html.escape('; '.join(a['problems'])) + '</b>' if a['problems'] else '<span class="ok">audit ok</span>'
        flagged += bool(a['problems'])
        check = (f"<br>native spawn ({nat['run']}): moved {nat['moved']} m, feet ≈ {nat['feet']} (authored {nat['authored']:.2f}), drift {nat['drift']} m"
                 if nat else '')
        cards.append(f'''<figure><figcaption>#{r['index']} team {r['team']} — {html.escape(r['name'])}<br>
pos {[round(c, 2) for c in r['position']]} yaw {r['yaw']:.2f}<br>{flag}{check}</figcaption>
<img src="{r['shots']['eye']['file']}"><img src="{r['shots']['overview']['file']}"></figure>''')
    (gallery.parent / 'index.html').write_text(f'''<!doctype html><meta charset="utf-8"><title>{html.escape(g['title'])}</title>
<style>body{{font:14px sans-serif;background:#111;color:#ddd}}figure{{display:inline-block;margin:8px;vertical-align:top;max-width:970px}}
img{{width:480px;margin:2px}}.bad{{color:#f77}}.ok{{color:#7c7}}</style><h1>{html.escape(g['title'])}</h1>
<p>Static camera-override views per point: the spawn's eye along its facing, and from behind/above. No character input.
Flags: audit_respawns.py (exact capsule/support/facing against movement collision). Native checks: the point alone, a real character spawned, camera sampled twice 10 s apart.</p>{''.join(cards)}''')
    summary.append((m, g['title'], len(g['points']), flagged, sum(1 for k in native if k[0] == m)))
rows = ''.join(f'<li><a href="{m}/index.html">{html.escape(t)}</a>: {n} points, {f} flagged by the audit, {c} checked natively</li>' for m, t, n, f, c in summary)
(root / 'index.html').write_text(f'<!doctype html><meta charset="utf-8"><title>Respawn point galleries</title><style>body{{font:15px sans-serif;background:#111;color:#ddd}}a{{color:#9cf}}</style><h1>Respawn point galleries</h1><ul>{rows}</ul>')
print(json.dumps(summary))
