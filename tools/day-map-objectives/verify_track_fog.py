#!/usr/bin/env python3
"""Offline check of the growing track fog in a built 206/207 client_project (no game).

Usage: verify_track_fog.py COLLISION_ROOT PROJECT [--from-z Z] [--end-z Z] [--json OUT]
COLLISION_ROOT: a directory with models/ (the 206 collision: pack files over the stock models).
- fog: every corridor point (x 9.9..21.6, z --from..--end-z, 0.25 m) lies within 5 m (the rust_smoke
  spawn radius) of a cloud when fully grown (track clouds + the rust-fog field), neighbours overlap;
- damage: at every step, the boxes of its zone; at the last step every bed point's prone (0.3), crouched
  (1.1) and standing (1.7 m) face height is inside a box (or the native moss zone at the junction);
  every elevated floor in the corridor (full movement collision, > bed + 1 m) is outside at 1.1/1.7 m;
- lag: each step's boxes end at least --lag behind the newest cloud started by then (last step: the end);
- spawns, containers, crates: distances to the clouds and the boxes.
"""
import argparse, json, math, re, sys
from pathlib import Path
T = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(T/'day-map-objectives')); sys.path.insert(0, str(T/'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config

ROOT = 'logic/fill/track_anomaly/'
ap = argparse.ArgumentParser(); ap.add_argument('root', type=Path); ap.add_argument('project', type=Path)
ap.add_argument("--from-z", type=float, default=0.0); ap.add_argument('--end-z', type=float, default=131.9)
ap.add_argument('--x', default='9.9,21.6'); ap.add_argument('--lag', type=float, default=4.5)
ap.add_argument('--json', type=Path); ap.add_argument('--step', type=float, default=0.25)
a = ap.parse_args()
X0, X1 = (float(v) for v in a.x.split(','))
project = config(a.project.read_bytes())
objs = {o['full_name']: o for o in project['client_objects'] if 'full_name' in o}
track = {n: o for n, o in objs.items() if n.startswith(ROOT)}
out = dict(project=str(a.project), track_objects=len(track))
problems = []

def world_meshes(geom_name):
    g = objs[geom_name]
    assert g['rotation'] == [0, 0, 0] and g['scale'] == [1, 1, 1], geom_name
    res = []
    for m in g['meshes']:
        assert m['rotation'] == [0, 0, 0], geom_name
        c = [g['position'][i] + m['position'][i] for i in range(3)]
        res.append((m['type'], c, m['scale']))
    return res

def inside(mesh, p, pad=0.0):
    t, c, s = mesh
    d = [p[i] - c[i] for i in range(3)]
    if t == 1: return all(abs(d[i]) <= s[i] + pad + 1e-4 for i in range(3))
    if t == 2: return math.hypot(d[0], d[2]) <= s[0] + pad and abs(d[1]) <= s[1] + pad
    if t == 0: return math.sqrt(sum(v * v for v in d)) <= s[0] + pad
    raise ValueError(t)

# Fog stages: controller state 0 time, fog zone effect zones -> clouds.
stages = []
for n, o in track.items():
    if n.endswith('/controller') and '/stage_' in n:
        st = o['states']
        clouds = []
        for zn in st[1]['groups'][0]['zones']:
            fog = objs[zn]
            assert fog['idle_particle'] in ('fx/level_3_anomaly/rust_smoke', 'fx/ported/v011e/track_fog') and fog['max_hit'] == 0 and len(fog['effect_zones']) <= 6
            clouds += [[fog['position'][i] + e['position'][i] for i in range(3)] for e in fog['effect_zones']]
        stages.append(dict(name=n.split('/')[-2], at=st[0]['active_time_sec'], clouds=clouds))
stages.sort(key=lambda s: s['at'])
rust = [o['position'] for n, o in objs.items() if n.startswith('fx/fill_rust_fog/')]
all_clouds = [c for s in stages for c in s['clouds']] + rust
out['stages'] = dict(count=len(stages), first=(stages[0]['at'], stages[0]['clouds'][0][2]), last=(stages[-1]['at'], stages[-1]['clouds'][0][2]),
                     intervals=sorted({b['at'] - a_['at'] for a_, b in zip(stages, stages[1:])}),
                     spacing=sorted({round(b['clouds'][0][2] - a_['clouds'][0][2], 3) for a_, b in zip(stages, stages[1:])}))
R = 5.0
worst = (0, None); fog_gaps = []
nz = int(round((a.end_z - a.from_z) / a.step)); nx = int(round((X1 - X0) / a.step))
ZS = [min(a.from_z + j * a.step, a.end_z) for j in range(nz + 1)]; XS = [min(X0 + i * a.step, X1) for i in range(nx + 1)]
for j in range(nz + 1):
    z = ZS[j]
    for i in range(nx + 1):
        x = XS[i]
        d = min(math.hypot(c[0] - x, c[2] - z) for c in all_clouds)
        if d > worst[0]: worst = (d, (x, z))
        if d > R: fog_gaps.append((round(x, 2), round(z, 2), round(d, 2)))
out['fog'] = dict(points=(nz + 1) * (nx + 1), uncovered=len(fog_gaps), farthest_from_a_cloud=[round(worst[0], 2), worst[1]], gaps=fog_gaps[:10])
pairs = [(s['name'], t['name'], round(min(math.hypot(p[0] - q[0], p[2] - q[2]) for p in s['clouds'] for q in t['clouds']), 2)) for s, t in zip(stages, stages[1:])]
first = min(math.hypot(p[0] - q[0], p[2] - q[2]) for p in stages[0]['clouds'] for q in rust)
out['fog']['neighbour_cloud_distance_max'] = max(d for *_, d in pairs); out['fog']['stage1_to_rust_field'] = round(first, 2)
if fog_gaps: problems.append(f'{len(fog_gaps)} corridor points farther than {R} m from any cloud')
if out['fog']['neighbour_cloud_distance_max'] >= 2 * R or first >= 2 * R: problems.append('neighbouring clouds do not overlap')

# Damage chain.
ctl = track[ROOT + 'damage/controller']
steps = []
for k, st in enumerate(ctl['states']):
    zones = st['groups'][0]['zones']
    if k == 0: idle = st['active_time_sec']; continue
    zone = objs[zones[0]]
    meshes = [m for g in zone['collision_geometries'] for m in world_meshes(g['name'])]
    steps.append(dict(step=k, zones=zones, max_hit=zone['max_hit'], filter=zone['hit_parts_filter'], meshes=meshes,
                      geometries=len(zone['collision_geometries']), duration=st['active_time_sec'], exit=st['energy_on_exit']))
t = idle
for s in steps: s['at'] = t; t += s['duration']
for s in steps:
    zmax = max(c[2] + sc[2] for _, c, sc in s['meshes']); zmin = min(c[2] - sc[2] for _, c, sc in s['meshes'])
    newest = max(c[2] for st in stages if st['at'] <= s['at'] for c in st['clouds'])
    s['z'] = [round(zmin, 2), round(zmax, 2)]; s['newest_cloud_z'] = newest; s['behind'] = round(newest - zmax, 2)
    if s is not steps[-1] and s['behind'] < a.lag - 0.3: problems.append(f"step {s['step']} reaches {zmax} within {a.lag} m of the newest cloud {newest}")
out['damage_steps'] = [{k: v for k, v in s.items() if k != 'meshes'} for s in steps]

root = a.root
full = FastWorld(root, project)
bed_re = re.compile(r'/(terrain\d*|moss|railroad_0[12])\.model/')
bed = FastWorld(root, parts=[q for q in full.parts if bed_re.search('/' + q[0])])
moss = world_meshes('logic/anomaly/toxic_moss_collision')
last = steps[-1]['meshes']
miss, junction_by_moss, n_bed, hdr = [], 0, 0, {}
clear = []  # per bed point: box top above the bed
for j in range(nz + 1):
    z = ZS[j]
    for i in range(nx + 1):
        x = XS[i]
        f = bed.floor(x, 2.4, z, below=1.5, above=0.7)
        if not f: continue
        g = f[0]; n_bed += 1
        tops = [c[1] + sc[1] for _, c, sc in last if abs(x - c[0]) <= sc[0] + 1e-4 and abs(z - c[2]) <= sc[2] + 1e-4]
        bots = [c[1] - sc[1] for _, c, sc in last if abs(x - c[0]) <= sc[0] + 1e-4 and abs(z - c[2]) <= sc[2] + 1e-4]
        if tops: clear.append((max(tops) - g, g - min(bots)))
        else: clear.append((-9, -9))
        for h in (0.3, 1.1, 1.7):
            p = (x, g + h, z)
            if any(inside(m, p) for m in last): continue
            if any(inside(m, p) for m in moss): junction_by_moss += 1; continue
            miss.append((round(x, 2), round(z, 2), h, round(g, 2)))
out['damage_final'] = dict(bed_points=n_bed, missed=len(miss), missed_sample=miss[:12], covered_only_by_moss_zone=junction_by_moss,
                           box_top_above_bed=[round(min(c[0] for c in clear), 2), round(max(c[0] for c in clear), 2)],
                           box_top_above_bed_percentiles={q: round(sorted(c[0] for c in clear)[int(q / 100 * (len(clear) - 1))], 2) for q in (1, 10, 50, 90, 99)},
                           box_bottom_below_bed=[round(min(c[1] for c in clear), 2), round(max(c[1] for c in clear), 2)])
if miss: problems.append(f'{len(miss)} bed face points outside the final damage')
# Elevated floors in the corridor.
elev = {}
for j in range(0, nz + 1, 2):
    z = ZS[j]
    for i in range(0, nx + 1, 2):
        x = XS[i]
        b = bed.floor(x, 2.4, z, below=1.5, above=0.7)
        if not b: continue
        y = 14.0
        while y > b[0] + 1.0:
            f = full.floor(x, y, z, below=y - (b[0] + 1.0), above=0.0)
            if not f or f[0] <= b[0] + 1.0: break
            free = not full.capsule_hits(x, f[0], z, r=0.2)
            if free:
                hit = [h for h in (1.1, 1.7) if any(inside(m, (x, f[0] + h, z)) for m in last)]
                key = f[1].split('/')[-2]
                e = elev.setdefault(key, dict(points=0, hit=0, heights=[99, 0], above_bed=[99, 0]))
                e['points'] += 1; e['hit'] += bool(hit)
                if hit: e.setdefault('hits', []).append([round(x, 2), round(z, 2), round(f[0], 2), round(f[0] - b[0], 2), hit])
                e['heights'] = [round(min(e['heights'][0], f[0]), 2), round(max(e['heights'][1], f[0]), 2)]
                e['above_bed'] = [round(min(e['above_bed'][0], f[0] - b[0]), 2), round(max(e['above_bed'][1], f[0] - b[0]), 2)]
            y = f[0] - 0.3
out['elevated_floors'] = elev
# Neighbour boxes.
seg = sorted({(round(c[2] - sc[2], 3), round(c[2] + sc[2], 3)) for _, c, sc in last})
out['segments'] = dict(count=len(seg), overlaps=sorted({round(p[1] - q[0], 3) for p, q in zip(seg, seg[1:])}))
if any(p[1] < q[0] for p, q in zip(seg, seg[1:])): problems.append('segment gap')
# Spawns, containers, crates.
def dist_to_boxes(p):
    return min(math.sqrt(sum(max(abs(p[i] - c[i]) - sc[i], 0) ** 2 for i in (0, 2))) for _, c, sc in last)
things = [(f"respawn {r['full_name']} team {r['team']}", r['position']) for r in project['server_objects']['respawn_points']]
things += [(f"container {c['full_name']}", c['visual_geometry']['position']) for c in project['server_objects'].get('victory_items_containers', [])]
things += [(f'crate {n}', o['position']) for n, o in objs.items() if o.get('game_object_type') == 'resupply_zone']
near = sorted((round(min(dist_to_boxes(p), min(math.hypot(c[0] - p[0], c[2] - p[2]) for s in stages for c in s['clouds']) - R), 2), n, [round(v, 1) for v in p]) for n, p in things)
out['nearest_team_objects'] = near[:6]
if near[0][0] < 3.0: problems.append(f'{near[0]} is near the fog')
out['problems'] = problems
print(json.dumps(out, indent=1))
if a.json: a.json.write_text(json.dumps(out, indent=1) + '\n')
sys.exit(1 if problems else 0)
