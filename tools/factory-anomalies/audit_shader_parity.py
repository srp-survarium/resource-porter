#!/usr/bin/env python3
"""Compare original020e archive shaders with previously observed021 shaders."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

FAMILIES = ('composition.ps', 'atmosphere.ps', 'atmosphere_clouds.ps', 'aberration.ps',
            'complex_post_process_blend.ps', 'apply_ambient_lights.ps',
            'environment_probe_lighting.ps', 'ssao_filter_upsample_temporal.ps', 'volume_fog.ps')


def dxbc(path):
    data = path.read_bytes()
    begin = data.find(b'DXBC')
    if begin < 0 or len(data)-begin < 32:
        raise ValueError('missing DXBC: '+str(path))
    size, count = struct.unpack_from('<II', data, begin+24)
    data = data[begin:begin+size]
    if len(data) != size or 32+4*count > size:
        raise ValueError('truncated DXBC: '+str(path))
    chunks = {}
    for offset in struct.unpack_from('<'+'I'*count, data, 32):
        if offset+8 > size:
            raise ValueError('bad chunk offset')
        length, = struct.unpack_from('<I', data, offset+4)
        if offset+8+length > size:
            raise ValueError('bad chunk length')
        chunks[data[offset:offset+4].decode('ascii')] = hashlib.sha256(data[offset+8:offset+8+length]).hexdigest()
    return dict(size=size, sha256=hashlib.sha256(data).hexdigest(), chunks=chunks)


def audit(source, later_audit, later_captures):
    records = []
    for family in FAMILIES:
        for path in sorted((source/family).glob('*')):
            records.append(dict(family=family, permutation=path.name, **dxbc(path)))
    observed = list(later_captures.glob('sky-000.bmp.*.dxbc'))
    observed += list(later_audit.glob('aberration-*.dxbc'))
    observed += list((later_audit/'ambient-shaders').glob('*.dxbc'))
    compared = []
    for path in sorted(observed):
        later = dxbc(path)
        program = later['chunks'].get('SHDR', later['chunks'].get('SHEX'))
        matches = [dict(family=r['family'], permutation=r['permutation'],
                        sha256=r['sha256'], byte_identical=r['sha256']==later['sha256'])
                   for r in records if r['chunks'].get('SHDR', r['chunks'].get('SHEX'))==program]
        compared.append(dict(observed=str(path), **later, matches=matches))
    return dict(scope='Offline byte/program identities only; matching shaders do not prove live constant bindings or visual parity.',
                source=str(source), source_records=records, later=compared)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'later_audit', 'later_captures', 'output'):
        p.add_argument(name, type=Path)
    a = p.parse_args()
    report = audit(a.source, a.later_audit, a.later_captures)
    a.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(dict(source_entries=len(report['source_records']),
                          later_entries=len(report['later']),
                          same_program=sum(bool(r['matches']) for r in report['later']))))
