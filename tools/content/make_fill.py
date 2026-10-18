#!/usr/bin/env python3
"""make_fill.py STOCK_DB SPEC_JSON OUT_DIR  (revision 20: SPEC_JSON = rev20_021d_7.crates.json, OUT_DIR = ports/gameplay-fill;
$PORTER: vostok-port, to extract the templates)

SPEC_JSON: {"<fill file stem>": {"name": ..., "map_id": N, "template": native project (default level_04/day_2),
            "crates": [{"team": 0|1, "native": "logic/vip/team1/" (default team1 for team 0, team2 for team 1),
                        "position": [x,y,z], "keep_rotation": true | "yaw": phi, "slope_x": gx, "slope_z": gz, "why": "..."}]}}
Writes one survarium_026::fill pack per stem: per crate one group of the template's native resupply set `native`
(zone, collision, ammo decal, the container_closed instance under the decal), renamed to logic/fill/teamN/, moved
rigidly so the crate stands at `position`: with keep_rotation as natively turned (a pure translation), else
with its local x along (cos phi, sin phi) in (x, z), tilted to the fitted floor plane (slopes gx along its local
x, gz along its local z): the group's turn is basis(native crate)^T * basis(target) as Euler XYZ
(skeleton_animation's convention)."""
import hashlib, json, math, os, subprocess, sys, tempfile
from pathlib import Path
import os
sys.path.insert(0, os.environ.get('TOOLS', '/mnt/hdd/Projects/release-20/crates-021d/tools-a0b5933/tools') + '/chemical-plant')
from audit_collision_region import config

def e2b(a):
    sx, cx = math.sin(a[0]), math.cos(a[0]); sy, cy = math.sin(a[1]), math.cos(a[1]); sz, cz = math.sin(a[2]), math.cos(a[2])
    return [[cy * cz, -cy * sz, sy], [sy * sx * cz + cx * sz, -sx * sy * sz + cx * cz, -sx * cy], [-cx * sy * cz + sx * sz, cx * sy * sz + sx * cz, cx * cy]]
def b2e(b):
    sy = max(-1.0, min(1.0, b[0][2])); y = math.asin(sy); cy = math.sqrt(1 - sy * sy)
    if cy < 1e-6: return [math.atan2(b[1][0], b[1][1]), y, 0.0]
    return [math.atan2(-b[1][2], b[2][2]), y, math.atan2(-b[0][1], b[0][0])]
def mul(a, b): return [[sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)] for r in range(3)]
def tr(m): return [[m[j][i] for j in range(3)] for i in range(3)]
def norm(v): n = math.sqrt(sum(x * x for x in v)); return [x / n for x in v]
def cross(a, b): return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]

def target(phi, gx, gz):
    """Rows: local x, y, z in world; row2 = row0 x row1 (as basis([0, phi, 0]))."""
    c, s = math.cos(phi), math.sin(phi)
    x = norm([c, gx, s]); z0 = [-s, gz, c]
    y = norm(cross(z0, x)); z = cross(x, y)
    return [x, y, z]

NATIVE = {0: 'logic/vip/team1/', 1: 'logic/vip/team2/'}
FILL = {0: 'logic/fill/team1/', 1: 'logic/fill/team2/'}
TEMPLATE = 'projects/level_04/day_2/client_project'

def read_template(db, path, cache={}):
    if path not in cache:
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / 'client_project'
            subprocess.run([os.environ.get('PORTER', '/tmp/r18/target/release/vostok-port'), 'extract', '--db', db, '--path', path,
                            '--output', str(out)], check=True, capture_output=True)
            data = out.read_bytes()
        cache[path] = (hashlib.sha256(data).hexdigest(), config(data))
    return cache[path]

def native_crate(proj, src, team):
    """The native crate of resupply set `src` (the container_closed instance under its ammo decal)."""
    objs = {o['full_name']: o for o in proj['client_objects'] if o.get('full_name', '').startswith(src + 'resupply/')}
    assert objs[src + 'resupply/resupply_zone']['team'] == team, (src, team)
    decal = objs[src + 'resupply/ammo_decal']['position']
    crates = [v for v in proj['render_visuals'] if v['lib_name'] == 'item/quest/container_closed']
    near = min(crates, key=lambda v: math.dist(v['position'], decal))
    assert math.dist(near['position'], decal) < 1.5
    return near['position'], near['rotation']

def main():
    db, spec, out = sys.argv[1], json.load(open(sys.argv[2])), Path(sys.argv[3])
    for stem, f in spec.items():
        template = f.get('template', TEMPLATE); sha, proj = read_template(db, template)
        groups = []
        for c in f['crates']:
            src, dst = c.get('native', NATIVE[c['team']]), FILL[c['team']]
            anchor, nrot = native_crate(proj, src, c['team'])
            if c.get('keep_rotation'):   # the native crate's own rotation: a pure translation
                turn = e2b([0.0, 0.0, 0.0])
            else:
                turn = mul(tr(e2b(nrot)), target(c['yaw'], c.get('slope_x', 0.0), c.get('slope_z', 0.0)))
            pitch, yaw, roll = b2e(turn)
            final = b2e(mul(e2b(nrot), turn))
            groups.append({
                'name': c['why'],
                'client_objects': [src + 'resupply/resupply_zone', src + 'resupply/resupply_collision', src + 'resupply/ammo_decal'],
                'models': ['item/quest/container_closed'],
                'anchor': [round(v, 3) for v in anchor], 'radius': 1.5, 'team': c['team'], 'rename': {src: dst},
                'to': {'position': c['position'], 'yaw': round(yaw, 4) + 0.0, 'pitch': round(pitch, 4) + 0.0, 'roll': round(roll, 4) + 0.0}})
            print(stem, 'team', c['team'], 'native', src, [round(v, 3) for v in anchor], '-> crate at', c['position'],
                  'final crate rotation', [round(v, 4) for v in final], file=sys.stderr)
        pack = {'name': f['name'], 'map_id': f['map_id'], 'template_project': template, 'template_sha256': sha, 'groups': groups}
        (out / f'{stem}.json').write_text(json.dumps(pack, indent=2) + '\n')

main()
