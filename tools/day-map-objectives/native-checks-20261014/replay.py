#!/usr/bin/env python3
"""Reproduce this corridor checkpoint comparison; not a general player decoder."""
import json
from pathlib import Path
import struct

root = Path(__file__).resolve().parent
result = json.loads((root / 'chemical-corridor.json').read_text())
positions = []
for time in (70600, 75600):
    body = (root / f'chemical-corridor-{time}.bin').read_bytes()
    mask, length = struct.unpack_from('<IH', body)
    assert mask == 1 and length == 675, 'Different roster/profile state layout'
    state = body[6:6 + length]
    hits = []
    for offset in range(len(state) - 11):
        x, y, z = struct.unpack_from('<3f', state, offset)
        if -65 < x < -60 and 5 < y < 10 and -29 < z < -27:
            hits.append((offset, [x, y, z]))
    assert len(hits) == 1 and hits[0][0] == 591, 'Ambiguous position candidate'
    positions.append(hits[0][1])
delta = [b - a for a, b in zip(*positions)]
camera = result['camera_position_delta']
assert abs(delta[0] - camera[0]) < 1e-5
assert abs(delta[1] - camera[1]) < 1e-5
assert abs(delta[2] + camera[2]) < 1e-5  # Bullet Z sign
assert positions[0][0] < -62.643 and positions[1][0] > -62.401
print(json.dumps({'checkpoint_position_candidates': positions,
                  'delta': delta, 'matches_mode0_camera_delta': True,
                  'scope': 'Recorded single-player replay only; field interpretation inferred.'}))
