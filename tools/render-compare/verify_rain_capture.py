#!/usr/bin/env python3
"""Check rain draw coverage and geometry bounds; this does not establish parity."""
import argparse
import json
import struct
from pathlib import Path


def verify(report):
    counts = json.loads(report.read_text())
    prefix = str(report).removesuffix('.rain-counts.json')
    if not counts['single_frame'] or counts['first_present'] != counts['last_present']:
        raise ValueError(f'{report}: capture spans multiple frames')
    result = []
    for family, count in zip(counts['families'], counts['observed_draws'], strict=True):
        if count > counts['capture_cap_per_family']:
            raise ValueError(f'{report}: {family} exceeds capture cap')
        for draw in range(count):
            suffix = f'-{draw}' if draw else ''
            base = f'{prefix}.{family}-draw{suffix}'
            geometry = json.loads(Path(base + '.geometry.json').read_text())
            if geometry['present'] != counts['first_present']:
                raise ValueError(f'{base}: mismatched frame serial')
            for extension in ['.dxbc', '.vs.dxbc']:
                if not Path(base + extension).stat().st_size:
                    raise ValueError(f'{base + extension}: empty file')
            vertices = Path(base + '.vertices.bin').stat().st_size
            if geometry['indexed']:
                width = {57: 2, 42: 4}[geometry['index_format']]
                data = Path(base + '.indices.bin').read_bytes()
                offset = geometry['index_offset'] + width * geometry['start']
                end = offset + width * geometry['count']
                if offset < 0 or end > len(data):
                    raise ValueError(f'{base}: index range exceeds captured buffer')
                indices = [item[0] + geometry['base_vertex'] for item in
                           struct.iter_unpack('<H' if width == 2 else '<I', data[offset:end])]
            else:
                indices = range(geometry['start'], geometry['start'] + geometry['count'])
            if not indices or min(indices) < 0 or geometry['stride'] <= 0:
                raise ValueError(f'{base}: invalid vertex range')
            if geometry['vertex_offset'] + (max(indices) + 1) * geometry['stride'] > vertices:
                raise ValueError(f'{base}: vertex range exceeds captured buffer')
            for stage in ['before', 'output']:
                target = f'{prefix}.{family}{suffix}-{stage}'
                metadata = json.loads(Path(target + '.targets.json').read_text())
                color = next((item for item in metadata if item['slot'] == 0), None)
                if not color or color.get('skipped') or not color.get('written'):
                    raise ValueError(f'{target}: color readback missing')
                if not Path(target + '.rt00.bin').stat().st_size:
                    raise ValueError(f'{target}: empty color readback')
            result.append({'family': family, 'draw': draw, 'vertices_referenced': len(indices)})
    return {'report': str(report), 'present': counts['first_present'],
            'observed_draws': counts['observed_draws'], 'verified_draws': result,
            'scope': 'Known Draw/DrawIndexed rain identities and captured geometry/output coverage, not visual parity.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports', nargs='+', type=Path)
    args = parser.parse_args()
    print(json.dumps([verify(report) for report in args.reports], indent=2))
