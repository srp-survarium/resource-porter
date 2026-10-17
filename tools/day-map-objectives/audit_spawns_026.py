#!/usr/bin/env python3
"""Offline audit of every 0.26g map variant's player spawn points (server_objects.respawn_points).

Usage: audit_spawns_026.py CONFIG_JSON OUT_DIR

CONFIG_JSON: {"porter": "...vostok-port", "stock_db": "...resources.db", "cache": "DIR",
              "variants": [{"label": "208", "files": "PACK/files" | null, "db": "DB the project is in" | null,
                            "project": "projects/v011e_level_03/day_2/client_project"}, ...]}
A variant's project and its collision models come from `files` (a pack's files) when there, else from
`stock_db` (the native maps, and every native model a port shares with the stock archive); those are
extracted once into `cache`. Each variant gets a merged collision root of symlinks.

Per point (audit_respawns.World, the movement collision):
- support: the floor under it within 1 m below / 0.4 m above; floating more than 0.3 m, below its surface
  more than 0.1 m, or no floor within 1 m (a fall: looks 6 m down for the drop);
- a standing capsule (r 0.35, 0.25..1.8 m) free of collision;
- what it faces at eye height within 2 m (under 0.8 m: faces a wall);
- inside a map-border volume (level_border collisions);
- inside or within 1 m of an anomaly or damage zone (client objects of type anomaly / *zone with a
  collision geometry: its meshes' extents);
- nearer than 1.5 m to another point of its team;
- nearer to the other team's centroid than to its own.
Writes OUT_DIR/<label>.json and a summary table (summary.json, summary.md). Candidates for review, not a
runtime proof.
"""
import collections, json, math, os, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent)); sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'chemical-plant'))
import audit_respawns
from audit_respawns import World, config, tri_box
from audit_collision_region import intersects, rotation, transform

_cfg = {}
def cached_config(data):
    k = hash(data)
    if k not in _cfg: _cfg[k] = config(data)
    return _cfg[k]
audit_respawns.config = cached_config


def collision_dirs(project):
    for obj in project['collision_objects']:
        yield obj['lib_name'].removeprefix('resources/').split('#')[0]


def merged_root(cfg, variant, project, out):
    """A directory where every collision folder the project names resolves: the pack's, else stock's."""
    files = Path(variant['files']) if variant.get('files') else None
    cache = Path(cfg['cache']); cache.mkdir(parents=True, exist_ok=True)
    root = out / 'roots' / variant['label']
    missing = []
    for d in sorted(set(collision_dirs(project))):
        if files and (files / d).is_dir(): continue
        if not (cache / 'overlay' / d).is_dir(): missing.append(d)
    if missing:
        for i in range(0, len(missing), 200):
            args = [cfg['porter'], 'extract-bundle', '--db', cfg['stock_db'], '--output', str(cache / f'chunk-{os.getpid()}-{i}')]
            for d in missing[i:i + 200]: args += ['--prefix', d + '/']
            if subprocess.run(args, capture_output=True).returncode != 0:      # a prefix stock lacks: one at a time
                for d in missing[i:i + 200]:
                    subprocess.run([cfg['porter'], 'extract-bundle', '--db', cfg['stock_db'], '--output', str(cache / f'chunk-{os.getpid()}-{i}'),
                                    '--prefix', d + '/'], capture_output=True)
            src = cache / f'chunk-{os.getpid()}-{i}' / 'overlay'
            if not src.is_dir(): continue
            for f in src.rglob('*'):
                if f.is_file():
                    t = cache / 'overlay' / f.relative_to(src); t.parent.mkdir(parents=True, exist_ok=True); f.replace(t)
    for d in sorted(set(collision_dirs(project))):
        target = files / d if files and (files / d).is_dir() else cache / 'overlay' / d
        link = root / d
        if link.is_symlink() or link.exists(): continue
        if not target.is_dir(): continue                       # no such folder anywhere: as the game, no collision
        link.parent.mkdir(parents=True, exist_ok=True); link.symlink_to(target)
    return root


def load_project(cfg, variant, out):
    if variant.get('files') and (Path(variant['files']) / variant['project']).exists():
        return config((Path(variant['files']) / variant['project']).read_bytes())
    tmp = out / 'projects' / (variant['label'] + '.client_project'); tmp.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([cfg['porter'], 'extract', '--db', variant.get('db') or cfg['stock_db'], '--path', variant['project'], '--output', str(tmp)],
                   check=True, capture_output=True)
    return config(tmp.read_bytes())


def zones(project):
    """Anomaly and damage zones: (name, centre, radius) from their collision geometries' meshes."""
    found = []
    def walk(objs):
        for o in objs or []:
            kind = str(o.get('game_object_type', '')); name = str(o.get('full_name', ''))
            if ('anomal' in kind or 'anomal' in name or 'damage' in kind or 'kill' in kind) and 'position' in o:
                r = 1.0
                for m in o.get('meshes', []) or []:
                    s = m.get('scale', [1, 1, 1]); r = max(r, math.hypot(*s[:3]) if m.get('type') == 1 else max(s))
                found.append((name, o['position'], r))
    walk(project.get('client_objects'))
    so = project.get('server_objects') or {}
    if isinstance(so, dict):
        for k, v in so.items():
            if isinstance(v, list): walk(v)
    return found


def audit(cfg, variant, out):
    project = load_project(cfg, variant, out)
    root = merged_root(cfg, variant, project, out)
    world = World(root, project)
    border = [p for n, o, p in world.parts if 'level_border' in n]
    points = project['server_objects']['respawn_points']
    zs = zones(project)
    teams = collections.defaultdict(list)
    for p in points: teams[p['team']].append(p['position'])
    centre = {t: [sum(q[i] for q in v) / len(v) for i in range(3)] for t, v in teams.items()}
    rows = []
    for i, sp in enumerate(points):
        x, y, z = sp['position']; rx, ry, rz = sp['rotation']
        yaw = ry if abs(rx) < 1 else math.pi - ry
        fl = world.floor(x, y, z)
        problems = []
        if fl is None:
            deep = world.floor(x, y, z, below=6.0, above=0.4)
            problems.append('no floor within 1 m' + (f' (drops {y - deep[0]:.1f} m)' if deep else ' nor 6 m below'))
            f = deep[0] if deep else y
        else:
            f = fl[0]
            if y - f > 0.3: problems.append(f'floats {y - f:.2f} m above its floor')
            elif f - y > 0.1: problems.append(f'{f - y:.2f} m below its floor surface')
        hits = world.capsule_hits(x, f, z)
        if hits: problems.append('inside ' + ', '.join(hits))
        face = world.facing(x, f, z, yaw)
        if face and face[0] < 0.8: problems.append(f'faces {face[1]} at {face[0]} m')
        if any(intersects(p['bounds'], [[x - .01, f + .5, z - .01], [x + .01, f + 1.5, z + .01]]) for p in border):
            problems.append('inside a map-border volume')
        for name, c, r in zs:
            d = math.dist([x, z], [c[0], c[2]])
            if d < r + 1.0 and abs(c[1] - f) < r + 2.0: problems.append(f'{d:.1f} m from zone {name} (r {r:.1f})')
        mates = [math.dist(sp['position'], q['position']) for j, q in enumerate(points) if j != i and q['team'] == sp['team']]
        near = min(mates) if mates else None
        if near is not None and near < 1.5: problems.append(f'{near:.2f} m from another team-{sp["team"]} point')
        other = min((math.dist(sp['position'], c), t) for t, c in centre.items())
        if other[1] != sp['team']: problems.append('nearer the other team')
        rows.append(dict(index=i, name=sp.get('full_name'), team=sp['team'], position=sp['position'], yaw=round(yaw, 3),
                         floor=None if fl is None else round(fl[0], 3), nearest_mate=None if near is None else round(near, 2),
                         problems=problems))
    by_team = {}
    for t in sorted(teams):
        mine = [r for r in rows if r['team'] == t]
        by_team[t] = dict(points=len(mine), good=sum(not r['problems'] for r in mine),
                          safe_group=sum('safe_respawn' in (r['name'] or '') for r in mine))
    result = dict(label=variant['label'], project=variant['project'], files=variant.get('files'), teams=by_team,
                  collision_parts=len(world.parts), zones=len(zs), points=rows)
    (out / f'{variant["label"]}.json').write_text(json.dumps(result, indent=1) + '\n')
    return result


def main():
    cfg = json.loads(Path(sys.argv[1]).read_text()); out = Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
    only = set(sys.argv[3:])
    results = []
    for v in cfg['variants']:
        if only and v['label'] not in only: continue
        r = audit(cfg, v, out); results.append(r)
        print(r['label'], json.dumps(r['teams']), 'parts', r['collision_parts'], 'zones', r['zones'], flush=True)
        for p in r['points']:
            if p['problems']: print('   ', p['team'], p['name'], [round(c, 2) for c in p['position']], '; '.join(p['problems']), flush=True)
    summary = [{k: r[k] for k in ('label', 'project', 'teams', 'collision_parts', 'zones')} for r in results]
    (out / 'summary.json').write_text(json.dumps(summary, indent=1) + '\n')


if __name__ == '__main__':
    main()
