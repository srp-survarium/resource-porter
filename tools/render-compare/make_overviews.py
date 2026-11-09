#!/usr/bin/env python3
"""Make separated elevated views from the authored playable-area bounds.

These are candidate views to inspect before freezing, not collision-safe player
positions. Aerial positions intentionally expose broad areas in a single frame.
Input is the decoded source client project as JSON.
"""
import argparse
import json
import math
from pathlib import Path


def overview_poses(project, name):
    objects = project['server_objects']
    positions = [obj['position'] for group in ('respawn_points', 'victory_item_spawners')
                 for obj in objects[group]]
    lo = [min(p[i] for p in positions) for i in range(3)]
    hi = [max(p[i] for p in positions) for i in range(3)]
    cx, cz = (lo[0]+hi[0])/2, (lo[2]+hi[2])/2
    sx, sz = hi[0]-lo[0], hi[2]-lo[2]
    ground = (lo[1]+hi[1])/2
    poses = []
    # Each station is a single broad view; no rotations at a fixed station.
    stations = [('southwest', -.65, -.65), ('south', 0, -.75),
                ('southeast', .65, -.65), ('east', .75, 0),
                ('northeast', .65, .65), ('north', 0, .75),
                ('northwest', -.65, .65), ('west', -.75, 0)]
    for label, x, z in stations:
        position = [cx+x*sx, hi[1]+28, cz+z*sz]
        target = [cx, ground, cz]
        poses.append((label, position, target))
    for label, x, z in [('south-quarter', 0, -.30), ('east-quarter', .30, 0),
                        ('north-quarter', 0, .30), ('west-quarter', -.30, 0)]:
        position = [cx+x*sx, hi[1]+14, cz+z*sz]
        target = [cx-x*sx*.7, ground, cz-z*sz*.7]
        poses.append((label, position, target))
    result = []
    for label, position, target in poses:
        direction = [b-a for a, b in zip(position, target)]
        length = math.sqrt(sum(v*v for v in direction))
        result.append(dict(name=f'{name}-{label}', position=position,
                           forward=[v/length for v in direction], fov=75,
                           aspect=1280/720, near=.05, far=500, buffers=False,
                           settle_ms=5000, kind='aerial-overview'))
    # Closer views use separated authored objective locations, not a cluster of
    # team spawns. Nearby authored heights keep the camera above local structures
    # where possible; inspect every result for roofs or occlusion before freezing.
    objectives = [obj['position'] for obj in objects['victory_item_spawners']]
    selected = [min(range(len(objectives)), key=lambda i: objectives[i][2])]
    while len(selected) < min(10, len(objectives)):
        def separation(i):
            return min(math.hypot(objectives[i][0]-objectives[j][0],
                                  objectives[i][2]-objectives[j][2]) for j in selected)
        best = max(range(len(objectives)), key=separation)
        if separation(best) < 18:
            break
        selected.append(best)
    for number, index in enumerate(selected):
        anchor = objectives[index]
        nearby = [p[1] for p in objectives if math.hypot(p[0]-anchor[0], p[2]-anchor[2]) < 22]
        angle = number * 2.399963229728653
        position = [anchor[0]+18*math.sin(angle), max(nearby)+10, anchor[2]+18*math.cos(angle)]
        target = [anchor[0], anchor[1]+1.5, anchor[2]]
        direction = [b-a for a, b in zip(position, target)]
        length = math.sqrt(sum(v*v for v in direction))
        result.append(dict(name=f'{name}-close-{number:02}', position=position,
                           forward=[v/length for v in direction], fov=65,
                           aspect=1280/720, near=.05, far=500, buffers=False,
                           settle_ms=5000, kind='close-elevated', source_anchor=anchor))
    return result


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--project', required=True, type=Path)
    p.add_argument('--name', required=True)
    p.add_argument('--output', required=True, type=Path)
    a = p.parse_args()
    with a.output.open('x') as out:
        json.dump(overview_poses(json.loads(a.project.read_text()), a.name), out, indent=2)
        out.write('\n')
