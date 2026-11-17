#!/usr/bin/env python3
"""native_crates.py STOCK_DB ROOT OUT_DIR LEVEL:MAP_ID[:TDM_MAP_ID]...   ($PORTER: vostok-port; $TOOLS: resource-porter
a0b5933's tools, for the movement collision)

The ammo crates of a 0.21d map whose native 0.26 level has the same layout (the importer's layout offset is 0: 0.26
level_02, level_03 and level_05 match 0.21d exactly), as revision 20 gave Vostok Radar Station (maps-021d-7: "check
0.26 for where they put it"): each native resupply set of the same weather and mode (every client object under
logic/vip/<team>/resupply/, the item/quest/container_closed instances within 1.5 m of any of them), renamed
logic/fill/<team>/, at native's spot and heading, turned and moved as one so that its lowest crate rests on 0.21d's
collision:
- tilt: the bottom face's 5 x 5 samples each find the floor under them (ROOT's movement collision, the floor within
  0.5 m below to 0.3 m above the sample); the crate's bottom takes the plane a box set down there lies on (through
  three of those floors, none above it, the least room under it, at most 15 degrees), its heading kept;
- height: then the height where the highest floor meets its bottom (no sample under the ground, at least one on
  it);
- place: its native spot, or within 1 m (rings of 10 cm, 16 headings) the place where it stands flattest (the
  least room under its bottom; a place must gain 2 cm on a nearer one), with no crate of the group inside
  movement collision (6 cm above its bottom up to its top: off walls); no such place within 1 m stops.
Collision a model of the stock archive provides (not in ROOT) is extracted into ROOT first. Writes one
gameplay-fill pack a project (survarium_026::fill), plan.json for examples/edit-built-pack and crates.json (each
crate group's native place, its move and the floor gaps before and after).
"""
import hashlib, json, math, os, subprocess, sys, tempfile
from pathlib import Path
T = os.environ['TOOLS']
sys.path.insert(0, T + '/day-map-objectives'); sys.path.insert(0, T + '/chemical-plant')
from fast_world import FastWorld
from audit_respawns import config, tri_box
from audit_collision_region import rotation, transform

db, root, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
# The crate's collision box (native exported primitive of item/quest/container_closed): centre and half sizes.
C, H = [-0.002333194, 0.33881423, -0.003938198], [0.73424494, 0.33861992, 0.56657296]
REACH, RING = 1.0, 0.1


def provide(level):
    """Extract into ROOT the collision files of the stock models the project places and ROOT lacks."""
    proj = config((root / f'projects/v021d_{level}/day_2/client_project').read_bytes())
    listing = None
    for obj in proj['collision_objects']:
        path = obj['lib_name'].removeprefix('resources/').split('#')[0]
        if (root / path).exists():
            continue
        if listing is None:
            listing = set(subprocess.run([os.environ['PORTER'], 'list', '--db', db], check=True, capture_output=True,
                                         text=True).stdout.splitlines())
        for name in ('exported_primitives', 'vertices', 'indices'):
            if f'{path}/{name}' in listing:
                (root / path).mkdir(parents=True, exist_ok=True)
                subprocess.run([os.environ['PORTER'], 'extract', '--db', db, '--path', f'{path}/{name}',
                                '--output', str(root / path / name)], check=True, capture_output=True)


def mul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)] for r in range(3)]


def tr(m):
    return [[m[j][i] for j in range(3)] for i in range(3)]


def b2e(b):
    """Euler XYZ of a basis (rotation()'s convention, the engine's)."""
    sy = max(-1.0, min(1.0, b[0][2])); y = math.asin(sy); cy = math.sqrt(1 - sy * sy)
    if cy < 1e-6:
        return [math.atan2(b[1][0], b[1][1]), y, 0.0]
    return [math.atan2(-b[1][2], b[2][2]), y, math.atan2(-b[0][1], b[0][0])]


def level_on(phi, gx, gz):
    """A crate's basis (rows: local x, y, z) heading phi in plan, its bottom on a plane rising gx along local x
    and gz along local z."""
    def norm(v):
        n = math.sqrt(sum(c * c for c in v)); return [c / n for c in v]
    def cross(a, b):
        return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    c, s = math.cos(phi), math.sin(phi)
    x = norm([c, gx, s]); y = norm(cross([-s, gz, c], x))
    return [x, y, cross(x, y)]


def bottom(basis, t):
    """The bottom face's 5 x 5 samples: (local x, local z, world point)."""
    return [(H[0] * (i / 2 - 1), H[2] * (j / 2 - 1), transform([C[0] + H[0] * (i / 2 - 1), C[1] - H[1], C[2] + H[2] * (j / 2 - 1)], basis, t))
            for i in range(5) for j in range(5)]


def gaps(world, basis, t):
    """The bottom samples' heights over the floor under them (None: no floor)."""
    out = []
    for _, _, b in bottom(basis, t):
        f = world.floor(b[0], b[1], b[2], below=0.5, above=0.3)
        out.append(None if f is None else b[1] - f[0])
    return out


def inside(world, basis, t, r=0.04):
    """The movement collision inside a crate (from 6 cm above its bottom to its top)."""
    found = set()
    for i in range(5):
        for j in range(5):
            for k in range(3):
                q = transform([C[0] + H[0] * (i / 2 - 1) * 0.95, C[1] - H[1] + 0.06 + (2 * H[1] - 0.06) * k / 2,
                               C[2] + H[2] * (j / 2 - 1) * 0.95], basis, t)
                lo, hi = [v - r for v in q], [v + r for v in q]
                for n, o, p in world.near(lo, hi):
                    if p['kind'] == 'triangle' and tri_box(p['vertices'], lo, hi) or \
                            p['kind'] != 'triangle' and world.primitive_distance(o, p, *q) <= r:
                        found.add(n)
    return found


def posed(crates, anchor, turn, shift):
    """Each crate's (basis, position) after the group's turn about anchor and shift."""
    out = []
    for c in crates:
        local = [c['position'][i] - anchor[i] for i in range(3)]
        t = [sum(local[i] * turn[i][j] for i in range(3)) + anchor[j] + shift[j] for j in range(3)]
        out.append((mul(rotation(c['rotation']), turn), t))
    return out


def settle(floors, steepest=0.27):
    """(slope along local x, along local z, room under it) of the plane a crate's bottom rests on over the floors
    [(u, v, h)]: a plane through three of them with none above it (a box set down on the ground lies on its three
    highest supports) and the least room under it, no steeper than `steepest` (15 degrees); None if there is none."""
    best = None
    n = len(floors)
    for i in range(n):
        for j in range(i + 1, n):
            for k in range(j + 1, n):
                (u1, v1, h1), (u2, v2, h2), (u3, v3, h3) = floors[i], floors[j], floors[k]
                det = (u2 - u1) * (v3 - v1) - (u3 - u1) * (v2 - v1)
                if abs(det) < 1e-6:
                    continue
                gx = ((h2 - h1) * (v3 - v1) - (h3 - h1) * (v2 - v1)) / det
                gz = ((u2 - u1) * (h3 - h1) - (u3 - u1) * (h2 - h1)) / det
                if max(abs(gx), abs(gz)) > steepest:
                    continue
                a = h1 - gx * u1 - gz * v1
                room = [a + gx * u + gz * v - h for u, v, h in floors]
                if min(room) < -1e-4:
                    continue
                if best is None or max(room) < best[2] - 1e-6:
                    best = (gx, gz, max(room))
    return best


def rest(world, crates):
    """The group's turn and move: its lowest crate, its heading kept, set down on 0.21d's collision (settle: on its
    highest supports, nothing under the ground) at its native spot or, within REACH, the place where it stands
    flattest (the least room under it, nearest first among equals: within 2 cm), no crate of the group inside
    collision."""
    low = min(crates, key=lambda v: v['position'][1])
    anchor = low['position']
    native = rotation(low['rotation'])
    phi = math.atan2(native[0][2], native[0][0])
    places = [(0.0, 0.0)] + [(d * math.cos(a * math.pi / 8), d * math.sin(a * math.pi / 8))
                             for d in [RING * k for k in range(1, int(REACH / RING) + 1)] for a in range(16)]
    best = None
    for dx, dz in places:
        samples = bottom(native, [anchor[0] + dx, anchor[1], anchor[2] + dz])
        floors = [(u, v, world.floor(b[0], b[1], b[2], below=0.5, above=0.3)) for u, v, b in samples]
        if any(f is None for _, _, f in floors):
            continue
        plane = settle([(u, v, f[0]) for u, v, f in floors])
        if plane is None or best is not None and plane[2] >= best[0] - 0.02:
            continue
        turn = mul(tr(native), level_on(phi, plane[0], plane[1]))
        basis, t = posed([low], anchor, turn, (dx, 0.0, dz))[0]
        g = [v for v in gaps(world, basis, t) if v is not None]
        shift = (dx, -min(g), dz)
        if any(inside(world, b, q) for b, q in posed(crates, anchor, turn, shift)):
            continue
        best = (plane[2], turn, shift)
    if best is None:
        raise SystemExit(f'no place for the crates at {anchor} within {REACH} m')
    return best[1], best[2]


def native(path):
    with tempfile.TemporaryDirectory() as d:
        f = Path(d) / 'p'
        subprocess.run([os.environ['PORTER'], 'extract', '--db', db, '--path', path, '--output', str(f)], check=True,
                       capture_output=True)
        data = f.read_bytes()
    return hashlib.sha256(data).hexdigest(), config(data)

plan, report = {'projects': {}}, []
for arg in sys.argv[4:]:
    level, *ids = arg.split(':')
    provide(level)
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
            near = FastWorld(root, parts=world.near([low[0] - 4, low[1] - 3, low[2] - 4], [low[0] + 4, low[1] + 4, low[2] + 4]))
            lowest = min(crates, key=lambda v: v['position'][1])
            before = [v for v in gaps(near, rotation(lowest['rotation']), lowest['position']) if v is not None]
            walls = sorted(set().union(*(inside(near, rotation(c['rotation']), c['position']) for c in crates)))
            turn, shift = rest(near, crates)
            dx, dy, dz = (round(v, 3) for v in shift)
            pitch, yaw, roll = (round(v, 4) + 0.0 for v in b2e(turn))
            after = [v for v in gaps(near, *posed([lowest], lowest['position'], turn, shift)[0]) if v is not None]
            report.append({'project': target, 'set': src, 'team': team, 'crates': len(crates), 'native': [round(v, 3) for v in low],
                           'move': [dx, dy, dz], 'turn': [pitch, yaw, roll],
                           'before': {'sunk': round(-min(before), 3), 'float': round(max(before), 3), 'inside': walls},
                           'after': {'sunk': round(-min(after), 3), 'float': round(max(after), 3)}})
            dst = src.replace('logic/vip/', 'logic/fill/')
            groups.append({
                'name': f'team {team} ammo crate where native {level} has it ({src}resupply: {len(objs)} objects, '
                        f'{len(crates)} container_closed), its heading native, tilted to 0.21d\'s floor and resting on it '
                        f'(moved {dx:+.2f} {dy:+.3f} {dz:+.2f} m)',
                'client_object_prefixes': [src + 'resupply/'], 'models': ['item/quest/container_closed'],
                'anchor': [round(v, 3) for v in low], 'radius': 1.5, 'team': team, 'rename': {src: dst},
                'to': {'position': [round(low[0] + dx, 3), round(low[1] + dy, 3), round(low[2] + dz, 3)], 'yaw': yaw, 'pitch': pitch, 'roll': roll}})
            print(json.dumps(report[-1]), file=sys.stderr)
        stem = f'v021d-{level}-day_{mode}-resupply'
        (out / f'{stem}.json').write_text(json.dumps({
            'name': f'v021d {level} day_{mode} ammo resupply (native {level} day_{mode}\'s crates at its spots; the battery containers stay 0.21d\'s)',
            'map_id': int(map_id), 'template_project': template, 'template_sha256': sha, 'groups': groups}, indent=2) + '\n')
        plan['projects'][target] = [{'op': 'fill', 'pack': str(out / f'{stem}.json')}]
(out / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
(out / 'crates.json').write_text(json.dumps(report, indent=1) + '\n')
