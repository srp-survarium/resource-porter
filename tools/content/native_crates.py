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

A team whose 0.21d spawns stand elsewhere than native's (their centre over MOVED m from native's: Rudnya's team 0,
27 m off) has its crates moved with them: across by the centres' difference, to the nearest spot (out to 10 m) with
0.21d's floor within 3 m of the native crate's height and the movement collision clear 1 m around. Where the
spawns match (School, the Fort, Rudnya's team 1) the crates stay at native's spot.
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
MOVED = 5.0

def free_spot(world, want, reach=10.0, step=0.5, clear=1.0):
    """The nearest point to `want` (across, on a `step` grid out to `reach`) with a floor within 3 m of its height
    and the movement collision clear of a standing body there and `clear` m around: (point, floor) or (None, None)."""
    rings = sorted(((dx * step, dz * step) for dx in range(-int(reach / step), int(reach / step) + 1)
                    for dz in range(-int(reach / step), int(reach / step) + 1)), key=lambda d: (math.hypot(*d), d))
    for dx, dz in rings:
        x, z = want[0] + dx, want[2] + dz
        floor = world.floor(x, want[1] + 3.0, z, below=6.0, above=0.0)
        if floor is None:
            continue
        if all(world.floor(x + ox, floor[0] + 0.3, z + oz, below=0.6, above=0.0) is not None
               and not world.capsule_hits(x + ox, floor[0], z + oz)
               for ox, oz in ((0, 0), (clear, 0), (-clear, 0), (0, clear), (0, -clear))):
            return [x, want[1], z], floor
    return None, None


def spawn_centres(proj):
    """{team: centre} of a project's respawn points."""
    pts = {}
    for o in proj['server_objects']['respawn_points']:
        pts.setdefault(o['team'], []).append(o['position'])
    return {t: [sum(p[i] for p in ps) / len(ps) for i in range(3)] for t, ps in pts.items()}

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
            donor = config((root / f'projects/v021d_{level}/day_2/client_project').read_bytes())
            worlds[2] = (FastWorld(root, donor), spawn_centres(donor))
        world, donor_centres = worlds[2]
        native_centres = spawn_centres(proj)
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
            shift = [d - n for d, n in zip(donor_centres[team], native_centres[team])]
            moved = math.hypot(shift[0], shift[2]) > MOVED
            if moved:
                at, floor = free_spot(world, [low[0] + shift[0], low[1], low[2] + shift[2]])
                assert at is not None, (template, src, 'no free 0.21d floor within 10 m of where the moved crate goes')
            else:
                at = low
                floor = world.floor(low[0], low[1] + STEP, low[2], below=2 * STEP, above=0.0)
                assert floor is not None, (template, src, 'no 0.21d floor within 10 cm of the native crate')
            dy = round(floor[0] - at[1], 3)
            dst = src.replace('logic/vip/', 'logic/fill/')
            where = (f'moved with 0.21d\'s team {team} spawns ({shift[0]:+.1f}, {shift[2]:+.1f} m)' if moved
                     else f'where native {level} has it')
            groups.append({
                'name': f'team {team} ammo crate {where} ({src}resupply: {len(objs)} objects, '
                        f'{len(crates)} container_closed), native rotation, {dy * 100:+.1f} cm onto 0.21d\'s floor',
                'client_object_prefixes': [src + 'resupply/'], 'models': ['item/quest/container_closed'],
                'anchor': [round(v, 3) for v in low], 'radius': 1.5, 'team': team, 'rename': {src: dst},
                'to': {'position': [round(at[0], 3), round(at[1] + dy, 3), round(at[2], 3)], 'yaw': 0.0, 'pitch': 0.0, 'roll': 0.0}})
            print(target, src, 'team', team, 'crates', len(crates), 'native', low, 'to', at, 'floor', floor[0], 'dy', dy,
                  file=sys.stderr)
        stem = f'v021d-{level}-day_{mode}-resupply'
        (out / f'{stem}.json').write_text(json.dumps({
            'name': f'v021d {level} day_{mode} ammo resupply (native {level} day_{mode}\'s crates at its spots; the battery containers stay 0.21d\'s)',
            'map_id': int(map_id), 'template_project': template, 'template_sha256': sha, 'groups': groups}, indent=2) + '\n')
        plan['projects'][target] = [{'op': 'fill', 'pack': str(out / f'{stem}.json')}]
(out / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
