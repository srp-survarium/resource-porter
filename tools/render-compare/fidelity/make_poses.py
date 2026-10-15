#!/usr/bin/env python3
"""Build the fidelity pose sets: frozen reference poses, user-reported spots and
spawn-derived coverage (eye height, spawn facing; some pitched to the floor).

    make_poses.py --project MAP=CLIENT_PROJECT_FILE ... --base MAP=POSES.json ... --output DIR

Spawn points stand in open, walkable space, so their views rarely start inside
geometry. A farthest-point subset spreads them across the map.
"""
import argparse, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'chemical-plant'))
from audit_collision_region import config, rotation  # noqa: E402

COMMON = {'fov': 65, 'aspect': 1280/720, 'near': .05, 'far': 500, 'buffers': False}


def spawn_poses(project, count, floors, prefix):
    points = project['server_objects']['respawn_points']
    pts = [(p['position'], p.get('rotation', [0, 0, 0])) for p in points]
    chosen = [0]
    while len(chosen) < min(count, len(pts)):
        best = max(range(len(pts)), key=lambda i: min(math.dist(pts[i][0], pts[j][0]) for j in chosen))
        chosen.append(best)
    out = []
    for n, i in enumerate(chosen):
        pos, rot = pts[i]
        m = rotation(rot)
        f = [m[0][2], m[1][2], m[2][2]]
        h = math.hypot(f[0], f[2]) or 1
        fx, fz = f[0]/h, f[2]/h
        eye = [pos[0], pos[1]+1.65, pos[2]]
        out.append(dict(COMMON, name=f'{prefix}-spawn{n:02d}', position=eye, forward=[fx, -0.05, fz],
                        settle_ms=3000, kind='spawn'))
        if n < floors:
            c, s = math.cos(math.radians(40)), math.sin(math.radians(40))
            out.append(dict(COMMON, name=f'{prefix}-spawn{n:02d}-floor', position=eye,
                            forward=[fx*c, -s, fz*c], settle_ms=3000, kind='grazing-floor'))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--project', action='append', required=True)
    ap.add_argument('--base', action='append', default=[])
    ap.add_argument('--extra', action='append', default=[], help='MAP=POSES.json of user-reported spots')
    ap.add_argument('--spawns', type=int, default=12)
    ap.add_argument('--floors', type=int, default=4)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    pairs = lambda items: dict(x.split('=', 1) for x in items)
    bases, extras = pairs(a.base), pairs(a.extra)
    for name, path in pairs(a.project).items():
        project = config(Path(path).read_bytes())
        poses = []
        for source, kind in ((bases.get(name), 'frozen'), (extras.get(name), 'user-reported')):
            if source:
                for p in json.loads(Path(source).read_text()):
                    q = dict(COMMON, **{k: v for k, v in p.items() if k in ('name', 'position', 'forward', 'fov', 'settle_ms', 'aspect', 'near', 'far')})
                    q.update(buffers=False, kind=kind)
                    poses.append(q)
        poses += spawn_poses(project, a.spawns, a.floors, name)
        names = [p['name'] for p in poses]
        assert len(names) == len(set(names)), name
        (a.output/f'{name}.json').write_text(json.dumps(poses, indent=1)+'\n')
        print(name, len(poses), {k: sum(p['kind'] == k for p in poses) for k in ('frozen', 'user-reported', 'spawn', 'grazing-floor')})


if __name__ == '__main__':
    main()
