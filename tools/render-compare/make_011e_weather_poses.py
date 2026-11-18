#!/usr/bin/env python3
"""Generate a collision-checked, source-derived Radar weather comparison sweep.

Pass decoded original project leaves from audit_011e_weather.py, a geoq binary,
and the ordered native resource layers. This writes cameras only; no game starts.
Both weather collision worlds must accept every selected camera. The rejection
report keeps the source locations that cannot safely be captured.
"""
import argparse
from contextlib import ExitStack
import hashlib
import itertools
import json
import math
from pathlib import Path
import re
import subprocess


def objects(leaves, table):
    result = {}
    for key, raw in leaves.items():
        match = re.fullmatch(re.escape(table) + r'\[(\d+)\]\.(\w+)', key)
        if match:
            kind, value = raw.split(':', 1)
            if kind in ('string', 'float3', 'i32', 'f32', 'bool'):
                result.setdefault(int(match[1]), {})[match[2]] = json.loads(value)
    return [result[key] for key in sorted(result)]


class Geometry:
    directions = [(x, y, z) for x, y, z in itertools.product((-1, 0, 1), repeat=3)
                  if x or y or z]

    def __init__(self, binary, project, layers):
        self.process = subprocess.Popen([str(binary), project, *map(str, layers)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        text=True, bufsize=1)
        if self.process.stdout.readline().strip() != 'ready':
            self.close()
            raise RuntimeError('collision query failed: ' + project)

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=10)

    def ray(self, eye, direction, limit=300):
        self.process.stdin.write(' '.join(map(str, (*eye, *direction, limit))) + '\n')
        value = self.process.stdout.readline().strip()
        if not value:
            raise RuntimeError('collision query ended unexpectedly')
        return None if value == '-' else float(value)

    def free(self, eye):
        floor = self.ray(eye, (0, -1, 0), 12)
        if floor is None:
            return 'no floor within 12m'
        hits = [self.ray(eye, direction, 30) for direction in self.directions]
        if any(t is not None and t < .3 for t in hits):
            return 'surface within 0.3m'
        if sum(t is not None and t < 1.2 for t in hits) > len(hits) // 2:
            return 'enclosed in a tight collision shell'
        return None


def generate(args):
    if args.output.exists():
        raise ValueError('refusing to overwrite an existing sweep')
    sources = [json.loads(path.read_text()) for path in args.leaves]
    with ExitStack() as stack:
        worlds = []
        for project in (
            'v011w_level_03_evn/evening_2',
            'v011w_level_03_rain/rain_2'):
            world = Geometry(args.geoq, project, args.layer)
            stack.callback(world.close)
            worlds.append(world)
        accepted, rejected, seen = [], [], set()

        def add(region, label, eye, forward=None):
            eye = [round(x, 4) for x in eye]
            key = tuple(eye)
            if key in seen:
                return
            seen.add(key)
            reasons = [world.free(eye) for world in worlds]
            if any(reasons):
                rejected.append(dict(region=region, source=label, position=eye, reasons=reasons))
                return
            if forward is None:
                choices = [(math.sin(k * math.tau / 16), 0, math.cos(k * math.tau / 16))
                           for k in range(16)]
                forward = max(choices, key=lambda d: min(
                    world.ray(eye, d, 50) or 50 for world in worlds))
            length = math.sqrt(sum(x*x for x in forward))
            accepted.append(dict(region=region, rationale=label, position=eye,
                                 forward=[round(x / length, 7) for x in forward],
                                 fov=65, settle_ms=10000, buffers=False, split='audit'))

        spawns = objects(sources[0], 'respawn_points')
        for index, spawn in enumerate(spawns):
            x, y, z = spawn['position']
            yaw = spawn['rotation'][1]
            add('spawn', spawn.get('full_name', f'respawn_points[{index}]'),
                (x, y + 1.65, z), (math.sin(yaw), 0, math.cos(yaw)))
        for variant, leaves in zip(('evening', 'rain'), sources, strict=True):
            for obj in objects(leaves, 'objects'):
                kind = obj.get('game_object_type')
                if kind not in ('environment_probe', 'victory_items_container', 'volume_fog'):
                    continue
                x, y, z = obj['position']
                label = variant + ':' + obj['full_name']
                if kind == 'environment_probe':
                    hit = worlds[0].ray((x, y + .2, z), (0, -1, 0), 12)
                    if hit is not None:
                        add('interior', label, (x, y + .2 - hit + 1.65, z))
                elif kind == 'victory_items_container':
                    for dx, dz in ((3, 0), (-3, 0), (0, 3), (0, -3)):
                        add('mode_object', label, (x + dx, y + 1.7, z + dz), (-dx, -.7, -dz))
                else:
                    for dx, dz in ((0, 0), (20, 0), (-20, 0), (0, 20), (0, -20)):
                        hit = worlds[0].ray((x + dx, 60, z + dz), (0, -1, 0), 100)
                        if hit is not None:
                            add('fog', label, (x + dx, 60 - hit + 1.65, z + dz), (0, -.1, 1))
        xs = [spawn['position'][0] for spawn in spawns]
        zs = [spawn['position'][2] for spawn in spawns]
        grid = []
        for x in range(math.floor(min(xs) - 16), math.ceil(max(xs) + 16), 8):
            for z in range(math.floor(min(zs) - 16), math.ceil(max(zs) + 16), 8):
                top = worlds[0].ray((x, 80, z), (0, -1, 0), 120)
                if top is None:
                    continue
                y = 80 - top
                if y < -12 or y > 40:
                    continue
                below = worlds[0].ray((x, y - .1, z), (0, -1, 0), 12)
                region = 'roof' if below is not None and below > 2.2 else 'terrain'
                grid.append((x, y, z))
                add(region, '8m collision grid', (x, y + 1.65, z), (0, -.25, 1))
        if not grid:
            raise ValueError('no usable collision grid')
        # Eight spatially separate backdrop viewpoints around the playable area.
        centre = ((min(xs) + max(xs))/2, (min(zs) + max(zs))/2)
        for k in range(8):
            d = (math.sin(k * math.tau/8), math.cos(k * math.tau/8))
            x, y, z = max(grid, key=lambda p: (p[0]-centre[0])*d[0]+(p[2]-centre[1])*d[1])
            add('horizon', 'grid edge facing outward', (x, y+5, z), (d[0], .08, d[1]))
        # Keep all authored spawns, then spread each other category spatially.
        chosen = [p for p in accepted if p['region'] == 'spawn']
        quotas = dict(interior=30, mode_object=8, fog=5, roof=18, horizon=8)
        def distance(p):
            return min(sum((x-y)**2 for x, y in zip(p['position'], q['position'], strict=True))
                       for q in chosen) if chosen else 1
        for region, count in quotas.items():
            pool = [p for p in accepted if p['region'] == region and p not in chosen]
            for _ in range(min(count, len(pool))):
                pose = max(pool, key=distance)
                chosen.append(pose)
                pool.remove(pose)
        pool = [p for p in accepted if p not in chosen]
        while len(chosen) < args.count and pool:
            pose = max(pool, key=distance)
            chosen.append(pose)
            pool.remove(pose)
        if len(chosen) < args.count:
            raise ValueError(f'only {len(chosen)} safe positions; requested {args.count}')
        for index, pose in enumerate(chosen):
            pose['name'] = f"sweep-{index:03d}-{pose['region']}"
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(chosen, indent=2) + '\n')
        report = dict(poses=len(chosen), accepted_candidates=len(accepted), rejected=rejected,
                      source_leaves={str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                                     for p in args.leaves},
                      layers=list(map(str, args.layer)),
                      scope='Both port collision worlds checked; visual source/port audit still required.')
        args.output.with_suffix('.audit.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(poses=len(chosen), rejected=len(rejected), output=str(args.output))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--leaves', type=Path, nargs=2, required=True)
    parser.add_argument('--geoq', type=Path, required=True)
    parser.add_argument('--layer', type=Path, action='append', required=True)
    parser.add_argument('--count', type=int, default=128)
    parser.add_argument('--output', type=Path, required=True)
    generate(parser.parse_args())
