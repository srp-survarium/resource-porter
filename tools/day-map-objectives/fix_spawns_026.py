#!/usr/bin/env python3
"""Proposals for a map variant's flagged spawn points (audit_spawns_026.py's checks), offline.

Usage: fix_spawns_026.py ROOT PROJECT AUDIT_JSON OUT_JSON [--add-from PROJECT2 --add-team T --add-up-to N ...]
  [--min-mate 1.8] [--search 3.0]

For each flagged point of the audit, in order:
- under or over its floor surface only: the same x, z on the floor (+0.02 m);
- otherwise (stacked under 1.5 m, inside geometry, facing a wall, no floor): the nearest spot within
  --search, on a 0.25 m ring grid, that passes every check (floor within 1 m, standing capsule clear,
  nothing within 0.8 m ahead at eye height, outside border volumes, at least --min-mate from every other
  point of its team as already fixed, nearer its own team's centroid), keeping its facing.
--add-from: candidate points (another project's respawn points of --add-team, e.g. the native map of the
same location) added while the team has fewer than --add-up-to good points: each must pass the same checks
at its own place (snapped to the floor), else it is skipped. The new names are <team folder>/respawn_point_fix<k>.
OUT_JSON: edit-built-pack ops (respawn_move / respawn_add) with the reason for each, and the counts after.
"""
import argparse, collections, json, math, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
from fast_world import FastWorld
from audit_respawns import config
from audit_collision_region import intersects


def yaw_of(rot): return rot[1] if abs(rot[0]) < 1 else math.pi - rot[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('root', type=Path); ap.add_argument('project', type=Path); ap.add_argument('audit', type=Path); ap.add_argument('out', type=Path)
    ap.add_argument('--add-from', type=Path, action='append', default=[]); ap.add_argument('--add-team', type=int, action='append', default=[])
    ap.add_argument('--grow', type=int, default=0, help="add points round the team's own (2-3.5 m out, 8 ways, facing as the nearest) up to this many good")
    ap.add_argument('--add-up-to', type=int, default=0); ap.add_argument('--min-mate', type=float, default=1.8); ap.add_argument('--search', type=float, default=3.0)
    a = ap.parse_args()
    project = config(a.project.read_bytes()); audit = json.loads(a.audit.read_text())
    world = FastWorld(a.root, project)
    border = [p for n, o, p in world.parts if 'level_border' in n]
    pts = [dict(name=p['full_name'], team=p['team'], pos=list(p['position']), yaw=yaw_of(p['rotation'])) for p in project['server_objects']['respawn_points']]
    teams = collections.defaultdict(list)
    for p in pts: teams[p['team']].append(p['pos'])
    centre = {t: [sum(q[i] for q in v) / len(v) for i in range(3)] for t, v in teams.items()}

    def check(x, y, z, yaw, team, me):
        fl = world.floor(x, y, z, below=1.0, above=0.6)
        if not fl: return None, 'no floor'
        f = fl[0]
        if world.capsule_hits(x, f, z): return None, 'capsule'
        face = world.facing(x, f, z, yaw)
        if face and face[0] < 0.8: return None, 'faces a wall'
        if any(intersects(p['bounds'], [[x - .01, f + .5, z - .01], [x + .01, f + 1.5, z + .01]]) for p in border): return None, 'border'
        mates = [math.dist([x, f, z], q['pos']) for q in pts if q is not me and q['team'] == team]
        if mates and min(mates) < a.min_mate: return None, 'near a mate'
        other = min((math.dist([x, f, z], c), t) for t, c in centre.items())
        if other[1] != team: return None, 'other side'
        return round(f + 0.02, 3), ''
    ops, notes = [], []
    flagged = {r['name']: r for r in audit['points'] if r['problems']}
    for p in pts:
        r = flagged.get(p['name'])
        if not r: continue
        x, y, z = p['pos']
        surface_only = all(('below its floor surface' in q or 'floats' in q) for q in r['problems'])
        if surface_only:
            fl = world.floor(x, y, z, below=1.0, above=0.6)
            ny, why = check(x, fl[0] if fl else y, z, p['yaw'], p['team'], p)
            if ny is not None:
                p['pos'] = [x, ny, z]; ops.append(dict(op='respawn_move', full_name=p['name'], position=[round(x, 3), ny, round(z, 3)], yaw=round(p['yaw'], 4),
                                                       why='; '.join(r['problems']) + ' -> on its floor'))
                continue
        best = None
        steps = int(a.search / 0.25)
        for i in range(-steps, steps + 1):
            for j in range(-steps, steps + 1):
                d = math.hypot(i * 0.25, j * 0.25)
                if d == 0 or d > a.search or (best and d >= best[0]): continue
                ny, why = check(x + i * 0.25, y, z + j * 0.25, p['yaw'], p['team'], p)
                if ny is not None: best = (d, [round(x + i * 0.25, 3), ny, round(z + j * 0.25, 3)])
        if best:
            p['pos'] = best[1]
            ops.append(dict(op='respawn_move', full_name=p['name'], position=best[1], yaw=round(p['yaw'], 4), why='; '.join(r['problems']) + f' -> moved {best[0]:.2f} m'))
        else:
            notes.append(f"{p['name']}: no valid spot within {a.search} m ({'; '.join(r['problems'])})")
    # Good counts after the moves.
    def good(team): return sum(check(*q['pos'], q['yaw'], team, q)[0] is not None for q in pts if q['team'] == team)
    adds = collections.Counter()
    for src, team in zip(a.add_from, a.add_team):
        other = config(src.read_bytes())
        like = next(q['name'] for q in pts if q['team'] == team)
        folder = like.rsplit('/', 1)[0]
        for c in other['server_objects']['respawn_points']:
            if c['team'] != team or good(team) >= a.add_up_to: continue
            x, y, z = c['position']
            ny, why = check(x, y, z, yaw_of(c['rotation']), team, None)
            if ny is None: notes.append(f"native {c['full_name']} at {[round(v, 2) for v in c['position']]}: skipped ({why})"); continue
            adds[team] += 1
            name = f'{folder}/respawn_point_fix{adds[team]}'
            q = dict(name=name, team=team, pos=[round(x, 3), ny, round(z, 3)], yaw=yaw_of(c['rotation'])); pts.append(q)
            ops.append(dict(op='respawn_add', like=like, full_name=name, position=q['pos'], yaw=round(q['yaw'], 4), why=f"native {c['full_name']}"))
    for team in sorted(teams) if a.grow else []:
        folder = next(q['name'] for q in pts if q['team'] == team).rsplit('/', 1)[0]
        like = next(q['name'] for q in pts if q['team'] == team)
        base = [q for q in pts if q['team'] == team]
        k = 0
        for q in base:
            for dist in (2.0, 2.75, 3.5):
                for t in range(8):
                    if good(team) >= a.grow: break
                    ang = 2 * math.pi * t / 8
                    x, y, z = q['pos'][0] + math.cos(ang) * dist, q['pos'][1], q['pos'][2] + math.sin(ang) * dist
                    ny, why = check(x, y, z, q['yaw'], team, None)
                    if ny is None: continue
                    k += 1; name = f'{folder}/respawn_point_fix{k}'
                    n = dict(name=name, team=team, pos=[round(x, 3), ny, round(z, 3)], yaw=q['yaw']); pts.append(n)
                    ops.append(dict(op='respawn_add', like=like, full_name=name, position=n['pos'], yaw=round(q['yaw'], 4), why=f"{dist} m from {q['name']}"))
    after = {t: dict(points=sum(q['team'] == t for q in pts), good=good(t)) for t in sorted(teams)}
    a.out.write_text(json.dumps(dict(project=str(a.project), ops=ops, notes=notes, after=after), indent=1) + '\n')
    print(json.dumps(dict(after=after, moves=sum(o['op'] == 'respawn_move' for o in ops), adds=sum(o['op'] == 'respawn_add' for o in ops), notes=notes), indent=1))


if __name__ == '__main__':
    main()
