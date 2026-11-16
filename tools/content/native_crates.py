#!/usr/bin/env python3
"""native_crates.py STOCK_DB ROOT OUT_DIR LEVEL:MAP_ID[:TDM_MAP_ID]...   ($PORTER: vostok-port; $TOOLS: resource-porter
a0b5933's tools, for the movement collision)

The ammo crates of a 0.21d map whose native 0.26 level has the same layout (the importer's layout offset is 0: 0.26
level_02, level_03 and level_05 match 0.21d exactly), as revision 20 gave Vostok Radar Station (maps-021d-7: "check
0.26 for where they put it"): each native resupply set of the same weather and mode (every client object under
logic/vip/<team>/resupply/, the item/quest/container_closed instances within 1.5 m of any of them),
renamed logic/fill/<team>/, at native's spot, raised or lowered as one onto 0.21d's floor under its lowest crate
(the ROOT bundle's movement collision; a step over 10 cm stops). Writes one gameplay-fill pack a project
(survarium_026::fill) and plan.json for examples/edit-built-pack.
"""
import hashlib, json, math, os, subprocess, sys, tempfile
from pathlib import Path
T = os.environ['TOOLS']
sys.path.insert(0, T + '/day-map-objectives'); sys.path.insert(0, T + '/chemical-plant')
from fast_world import FastWorld
from audit_respawns import config

db, root, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
STEP = 0.10

def native(path):
    with tempfile.TemporaryDirectory() as d:
        f = Path(d) / 'p'
        subprocess.run([os.environ['PORTER'], 'extract', '--db', db, '--path', path, '--output', str(f)], check=True,
                       capture_output=True)
        data = f.read_bytes()
    return hashlib.sha256(data).hexdigest(), config(data)

plan = {'projects': {}}
for arg in sys.argv[4:]:
    level, *ids = arg.split(':')
    worlds = {}
    for mode, map_id in zip((2, 4), ids):
        template = f'projects/{level}/day_{mode}/client_project'
        target = f'projects/v021d_{level}/day_{mode}/client_project'
        sha, proj = native(template)
        if 2 not in worlds:
            worlds[2] = FastWorld(root, config((root / f'projects/v021d_{level}/day_2/client_project').read_bytes()))
        world = worlds[2]
        sets = sorted({o['full_name'].split('resupply/')[0] for o in proj['client_objects']
                       if o.get('game_object_type') == 'resupply_zone' and '/resupply/' in o.get('full_name', '')})
        assert len(sets) == 2, (template, sets)
        groups = []
        for src in sets:
            objs = {o['full_name']: o for o in proj['client_objects'] if o.get('full_name', '').startswith(src + 'resupply/')}
            team = objs[src + 'resupply/resupply_zone']['team']
            crates = [v for v in proj['render_visuals'] if v['lib_name'] == 'item/quest/container_closed'
                      and any(math.dist(v['position'], o['position']) < 1.5 for o in objs.values())]
            assert crates, (template, src)
            low = min(crates, key=lambda v: v['position'][1])['position']
            floor = world.floor(low[0], low[1] + STEP, low[2], below=2 * STEP, above=0.0)
            assert floor is not None, (template, src, 'no 0.21d floor within 10 cm of the native crate')
            dy = round(floor[0] - low[1], 3)
            dst = src.replace('logic/vip/', 'logic/fill/')
            groups.append({
                'name': f'team {team} ammo crate where native {level} has it ({src}resupply: {len(objs)} objects, '
                        f'{len(crates)} container_closed), native rotation, {dy * 100:+.1f} cm onto 0.21d\'s floor',
                'client_object_prefixes': [src + 'resupply/'], 'models': ['item/quest/container_closed'],
                'anchor': [round(v, 3) for v in low], 'radius': 1.5, 'team': team, 'rename': {src: dst},
                'to': {'position': [round(low[0], 3), round(low[1] + dy, 3), round(low[2], 3)], 'yaw': 0.0, 'pitch': 0.0, 'roll': 0.0}})
            print(target, src, 'team', team, 'crates', len(crates), 'native', low, 'floor', floor[0], 'dy', dy, file=sys.stderr)
        stem = f'v021d-{level}-day_{mode}-resupply'
        (out / f'{stem}.json').write_text(json.dumps({
            'name': f'v021d {level} day_{mode} ammo resupply (native {level} day_{mode}\'s crates at its spots; the battery containers stay 0.21d\'s)',
            'map_id': int(map_id), 'template_project': template, 'template_sha256': sha, 'groups': groups}, indent=2) + '\n')
        plan['projects'][target] = [{'op': 'fill', 'pack': str(out / f'{stem}.json')}]
(out / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
