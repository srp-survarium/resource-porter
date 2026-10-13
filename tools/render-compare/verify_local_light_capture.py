#!/usr/bin/env python3
"""Verify bounded, single-frame native local-light capture coverage."""
import argparse
import json
import re
from pathlib import Path

def verify(report):
    coverage = json.loads(report.read_text())
    prefix = str(report).removesuffix('.local-light-counts.json')
    if prefix == str(report):
        raise ValueError('expected a local-light-counts.json report')
    count, cap = coverage['observed_draws'], coverage['capture_cap']
    if count < 0 or cap != 32 or sum(coverage['observed_by_family']) != count:
        raise ValueError('inconsistent local-light counts')
    if coverage['first_present'] != coverage['last_present'] or not coverage['single_frame']:
        raise ValueError('local-light capture spans multiple frames')
    if coverage['overflow'] != max(count - cap, 0):
        raise ValueError('inconsistent overflow')
    identities = []
    for size, family, digest in re.findall(r'\{(\d+),(\d+),\{([^}]+)\}\}', Path(__file__).with_name('local_light_index.h').read_text()):
        identities.append((int(size), int(family), bytes(int(v, 16) for v in digest.split(','))))
    draws = []
    for index in range(min(count, cap)):
        base = f'{prefix}.local-light-draw-{index}'
        state = json.loads(Path(base + '.state.json').read_text())
        geometry = json.loads(Path(base + '.geometry.json').read_text())
        code = Path(base + '.dxbc').read_bytes()
        size, family, digest = identities[state['identity_index']]
        if len(code) != size or code[:4] != b'DXBC' or code[4:20] != digest or state['family'] != family:
            raise ValueError('unexpected local-light shader: ' + base)
        if state['present'] != coverage['first_present'] or geometry['present'] != state['present']:
            raise ValueError('draw not from the reported frame: ' + base)
        for suffix in ('.vs.dxbc', '.vs.cb0.bin', '.cb0.bin', '.vertices.bin'):
            if not Path(base + suffix).stat().st_size:
                raise ValueError('empty draw artifact: ' + base + suffix)
        if geometry['indexed'] and not Path(base + '.indices.bin').stat().st_size:
            raise ValueError('missing indexed geometry: ' + base)
        for suffix in ('.targets.json', '.samplers.json'):
            json.loads(Path(base + suffix).read_text())
        for phase in ('before', 'output'):
            target = f'{prefix}.local-light-{index}-{phase}'
            views = json.loads(Path(target + '.targets.json').read_text())
            if not views or any(not v.get('written') for v in views):
                raise ValueError('incomplete light output readback: ' + target)
            for view in views:
                if not Path(target + f'.rt{view["slot"]:02}.bin').stat().st_size:
                    raise ValueError('empty light output: ' + target)
        draws.append({'index': index, 'family': family, 'digest': digest.hex()})
    return {'coverage': coverage, 'draws': draws, 'complete': count <= cap,
            'scope': 'Pinned native families; does not assert original lighting equivalence.'}

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports', type=Path, nargs='+')
    args = parser.parse_args()
    print(json.dumps([verify(report) for report in args.reports], indent=2))
