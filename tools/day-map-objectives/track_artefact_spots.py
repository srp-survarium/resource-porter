#!/usr/bin/env python3
"""Artefact spots for the growing track fog: on the track bed, inside the fog, clear and reachable.

Usage: track_artefact_spots.py COLLISION_ROOT PROJECT SECTION [SECTION ...] [--count N] [--apart M]
SECTION: Z0:Z1 (a z range of the track). PROJECT: a built client_project with the track fog.
A spot is a point of the track bed (terrain, moss, rail collision) with nothing solid 0.2..2.6 m over
it (not under or in a wagon or a pipe), a free standing capsule (r 0.35), within 2.5 m of a fog
emitter and inside the final damage boxes. Per section, N spots at least M m apart, spread sideways:
the k-th prefers the west track (x 12.9), then the east (18.6), then between them (15.75), and so on
(falling back to the other lanes), taking the nearest clear point within 1 m across. Prints the planner's --artefacts spot lists.
"""
import argparse, json, math, re, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('root', type=Path); ap.add_argument('project', type=Path); ap.add_argument('sections', nargs='+')
ap.add_argument('--count', type=int, default=4); ap.add_argument('--apart', type=float, default=7.0)
a = ap.parse_args()
proj = config(a.project.read_bytes())
objs = {o['full_name']: o for o in proj['client_objects'] if 'full_name' in o}
w = FastWorld(a.root, proj)
bed = FastWorld(a.root, parts=[q for q in w.parts if re.search(r'/(terrain\d*|moss|railroad_0[12])\.model/', '/' + q[0])])
emit, boxes = [], []
for n, o in objs.items():
    if n.startswith('logic/fill/track_anomaly/') and '/fog_zone' in n:
        emit += [[o['position'][i] + e['position'][i] for i in range(3)] for e in o['effect_zones']]
ctl = objs['logic/fill/track_anomaly/damage/controller']
last = objs[ctl['states'][-1]['groups'][0]['zones'][0]]
for g in last['collision_geometries']:
    geo = objs[g['name']]
    boxes += [([geo['position'][i] + m['position'][i] for i in range(3)], m['scale']) for m in geo['meshes']]

def spot(x, z):
    f = bed.floor(x, 2.4, z, below=1.5, above=0.7)
    if not f: return None
    g = f[0]
    if w.floor(x, g + 2.6, z, below=2.4, above=0.0): return None
    if w.capsule_hits(x, g, z, r=0.35): return None
    if min(math.hypot(e[0] - x, e[2] - z) for e in emit) > 2.5: return None
    if not any(all(abs(p - c[i]) <= s[i] for i, p in zip((0, 2), (x, z))) and c[1] - s[1] <= g + 0.3 and g + 1.7 <= c[1] + s[1] for c, s in boxes): return None
    return round(g, 3)

out = []
for sec in a.sections:
    z0, z1 = (float(v) for v in sec.split(':'))
    lanes = [12.9, 18.6, 15.75]
    picked, k, z = [], 0, z0
    while z <= z1 and len(picked) < a.count:
        if picked and z - picked[-1][2] < a.apart: z += 0.5; continue
        found = None
        for base in lanes[k % 3:] + lanes[:k % 3]:
            for dx in (0, 0.25, -0.25, 0.5, -0.5, 0.75, -0.75, 1.0, -1.0):
                g = spot(base + dx, z)
                if g is not None: found = (round(base + dx, 2), g, z); break
            if found: break
        if found: picked.append(found); k += 1; z += a.apart
        else: z += 0.5
    assert len(picked) == a.count, f'section {sec}: only {len(picked)} spots'
    out.append(','.join(f'{x}/{z}' for x, _, z in picked))
    print(f'section {sec}:', json.dumps(picked), file=sys.stderr)
print(';'.join(out))
