#!/usr/bin/env python3
"""Plan the 194 rust-fog gameplay-fill pack from native level_03 conventions.

Usage: plan_rust_fog.py LEVEL_03_PROJECT LEVEL_04_PROJECT BUNDLE_OVERLAY OUTPUT_PACK

Native 0.26 level_03 (Radar) draws its anomaly cloud with nine
fx/level_3_anomaly/rust_smoke particles (white lit smoke, 100 particles,
12/s, 15 s life, 2.0-3.2 m billboards, cylinder spawn radius 5 m, no sound)
and one rust_smoke_dark (nine wide box emitters) near the anomaly centre.
The clouds sit on or beside the spring's artefact spots (0.6-4.5 m) and in
every sphere of the outer gas damage zone; median height 0.84 m above the
nearest spot, the dark cloud 2.58 m. Those numbers are measured here from
the LEVEL_03 project, not assumed.

For 194 the native level_04 toxic-moss controllers, containers and damage
zone stay (the anomaly pack); this places clouds the same way:
- greedy cover: a cloud on an artefact spot until every spot is within the
  widest level_03 spot-to-cloud distance of a cloud;
- every primitive of the toxic damage zone holds at least one cloud;
- heights: the 194 movement-collision ground under the cloud plus the
  level_03 median offset; the dark cloud over the centroid of the spots.
"""
import json, math, statistics, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'chemical-plant'))
from audit_respawns import World, config
import hashlib

TEMPLATE = 'projects/level_03/day_2/client_project'

def objects(project):
    return {o['full_name']: o for o in project['client_objects'] if 'full_name' in o}

def spot(objs, container):
    g = objs[container['collision_geometries'][0]['name']]
    m = g['meshes'][0]
    return [g['position'][i] + m['position'][i] for i in range(3)]

def xz(a, b): return math.hypot(a[0]-b[0], a[2]-b[2])

def level_03_convention(objs):
    spring = objs['logic/anomaly/anomaly_rust/rust_anomaly_spring']
    spots = [spot(objs, objs[n]) for k, n in spring.items() if k == 'artefact_containers' for n in n]
    fog = {n: o for n, o in objs.items() if o.get('lib_name') == 'fx/level_3_anomaly/rust_smoke'}
    dark = [o for o in objs.values() if o.get('lib_name') == 'fx/level_3_anomaly/rust_smoke_dark']
    assert len(fog) == 9 and len(dark) == 1, (len(fog), len(dark))
    near = [min(xz(s, f['position']) for f in fog.values()) for s in spots]
    dy = []
    for f in fog.values():
        s = min(spots, key=lambda s: xz(s, f['position']))
        dy.append(f['position'][1] - s[1])
    d = dark[0]['position']; s = min(spots, key=lambda s: xz(s, d))
    return dict(cover=round(max(near), 2), height=round(statistics.median(dy), 2), dark_height=round(d[1] - s[1], 2),
                fog_template=sorted(fog)[0], fog_anchor=fog[sorted(fog)[0]]['position'],
                dark_template=dark[0]['full_name'], dark_anchor=d)

def inside(primitive, p):
    """Movement-free test of a zone mesh in world space (box type 1, cylinder type 2, sphere type 0)."""
    c, s, kind = primitive
    if kind == 1:
        return all(abs(p[i]-c[i]) <= s[i] for i in (0, 2))
    return xz(c, p) <= s[0]

def main():
    l03, l04, overlay, out = map(Path, sys.argv[1:5])
    l03_bytes = l03.read_bytes()
    convention = level_03_convention(objects(config(l03_bytes)))
    native = objects(config(l04.read_bytes()))
    containers = sorted((n for n in native if n.startswith('logic/anomaly/artefacts/toxic_moss_art_cont_')),
                        key=lambda n: int(n.rsplit('_', 1)[1]))
    spots = {n.rsplit('_', 1)[1]: spot(native, native[n]) for n in containers}
    zone = native['logic/anomaly/toxic_moss_collision']
    assert all(abs(m['rotation'][1]) < 1e-6 for m in zone['meshes'])
    primitives = [([zone['position'][i] + m['position'][i] for i in range(3)], m['scale'], m['type']) for m in zone['meshes']]
    world = World(overlay, config((overlay/'projects/v011e_level_04/day_2/client_project').read_bytes()))
    def ground(x, y, z):
        f = world.floor(x, y, z, below=6.0, above=6.0)
        assert f, f'no 194 ground under {x:.2f}, {z:.2f}'
        return f[0]
    clouds, uncovered = [], set(spots)
    while uncovered:
        best = max(sorted(uncovered, key=int), key=lambda k: sum(xz(spots[k], spots[u]) <= convention['cover'] for u in uncovered))
        covered = sorted((u for u in uncovered if xz(spots[best], spots[u]) <= convention['cover']), key=int)
        clouds.append(dict(at=spots[best], why=f'artefact spots {",".join(covered)}'))
        uncovered -= set(covered)
    for i, p in enumerate(primitives):
        if not any(inside(p, c['at']) for c in clouds):
            clouds.append(dict(at=p[0], why=f'damage-zone primitive {i}'))
    groups = []
    for i, c in enumerate(clouds, 1):
        x, _, z = c['at']; y = ground(x, c['at'][1], z) + convention['height']
        groups.append(dict(name=f'rust fog cloud {i:02d} ({c["why"]})', client_objects=[convention['fog_template']],
                           anchor=convention['fog_anchor'], to=dict(position=[round(x, 3), round(y, 3), round(z, 3)]),
                           rename={convention['fog_template']: f'fx/fill_rust_fog/cloud_{i:02d}'}))
    cx = statistics.fmean(s[0] for s in spots.values()); cz = statistics.fmean(s[2] for s in spots.values())
    cy = ground(cx, statistics.fmean(s[1] for s in spots.values()), cz) + convention['dark_height']
    groups.append(dict(name='rust fog dark core (centroid of the artefact spots)', client_objects=[convention['dark_template']],
                       anchor=convention['dark_anchor'], to=dict(position=[round(cx, 3), round(cy, 3), round(cz, 3)]),
                       rename={convention['dark_template']: 'fx/fill_rust_fog/dark_core'}))
    pack = dict(name='chemical-011e rust fog', map_id=194, template_project=TEMPLATE,
                template_sha256=hashlib.sha256(l03_bytes).hexdigest(), groups=groups)
    out.write_text(json.dumps(pack, indent=2) + '\n')
    print(json.dumps(dict(convention={k: v for k, v in convention.items() if not k.endswith('anchor')},
                          clouds=len(clouds), dark=1, spots=len(spots), zone_primitives=len(primitives)), indent=1))

if __name__ == '__main__':
    main()
