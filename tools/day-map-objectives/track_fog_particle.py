#!/usr/bin/env python3
"""The track fog's particle: native fx/level_3_anomaly/rust_smoke with its alpha over life multiplied.

Usage: track_fog_particle.py RUST_SMOKE_LEAVES ALPHA_MULT PATH SOURCE_SHA256 [RESULT_SHA256]
RUST_SMOKE_LEAVES: `vostok-port dump-db-config --leaves` of particles/fx/level_3_anomaly/rust_smoke.particle.
Prints an edit-built-pack `add_files` entry: every ColorLifeMatrix colour's alpha (the emitter's and its
data_sources copy) times ALPHA_MULT, capped at 1; colour, size, count, rate, life and domain stay native.
"""
import json, re, sys
leaves, mult, path, source_sha = sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[4]
result_sha = sys.argv[5] if len(sys.argv) > 5 else '0' * 64
assert 1 < mult <= 6
values = {}
for line in open(leaves):
    key, _, value = line.rstrip('\n').partition('\t')
    if 'color_matrix' in key or 'ColorLifeMatrix' in key:
        m = re.fullmatch(r'float4:\[(.*)\]', value)
        if m and key.endswith('.color'):
            r, g, b, a = (float(v) for v in m.group(1).split(','))
            if a > 0: values[key] = [r, g, b, round(min(1.0, a * mult), 6)]
assert len(values) == 4, f'expected the two peak alphas of the emitter and its data source, found {len(values)}'
print(json.dumps(dict(path=path, from_db='particles/fx/level_3_anomaly/rust_smoke.particle', from_sha256=source_sha,
                      set=dict(sorted(values.items())), sha256=result_sha), indent=2))
