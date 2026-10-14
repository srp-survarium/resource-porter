#!/usr/bin/env python3
"""Check that early-to-0.26 vertex conversion preserves visible geometry."""
import argparse
import json
from pathlib import Path
import struct
from audit_collision_region import chunks


def positions(data, early):
    vertex = chunks(data)[3]
    if early:
        end = next(i for i in range(0, len(vertex), 8)
                   if vertex[i:i+8] == bytes([255, 0, 0, 0, 17, 0, 0, 0]))
        start = end+12
        count = struct.unpack_from('<I', vertex, end+8)[0]
        strides = (32, 36)
    else:
        start = 4
        count = struct.unpack_from('<I', vertex)[0]
        strides = (28, 32)
    if not count or (len(vertex)-start) % count: raise ValueError('invalid vertex count')
    stride = (len(vertex)-start)//count
    if stride not in strides: raise ValueError(f'unexpected vertex stride {stride}')
    return [vertex[i:i+12] for i in range(start, len(vertex), stride)]


def audit(source, target):
    records = []
    for p in sorted(target.rglob('converted_model')):
        relative = str(p.relative_to(target))
        original = source/relative.replace('ported/v011a/', '')
        a, b = positions(original.read_bytes(), True), positions(p.read_bytes(), False)
        records.append(dict(path=relative, original_vertices=len(a), target_vertices=len(b),
                            position_bytes_equal=a == b,
                            indices_equal=chunks(original.read_bytes())[4] == chunks(p.read_bytes())[4]))
    if not records: raise ValueError('no surfaces checked')
    return records


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('source', type=Path); ap.add_argument('target', type=Path)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args(); result = audit(a.source, a.target)
    a.output.write_text(json.dumps(result, indent=2)+'\n')
    ok = all(x['position_bytes_equal'] and x['indices_equal'] for x in result)
    print(json.dumps(dict(surfaces=len(result), vertices=sum(x['target_vertices'] for x in result), unchanged=ok)))
    raise SystemExit(0 if ok else 1)
