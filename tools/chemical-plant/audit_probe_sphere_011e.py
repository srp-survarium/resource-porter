#!/usr/bin/env python3
"""Audit frozen 0.11e sphere falloff bytecode and a native 0.26 probe capture.

The evidence directory contains probe-extracted.json and its extracted DXBC files.
This checks the falloff block, not complete shader equivalence or visual parity.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def chunks(code):
    count = struct.unpack_from('<I', code, 28)[0]
    return {code[o:o+4]: code[o+8:o+8+struct.unpack_from('<I', code, o+4)[0]]
            for o in struct.unpack_from('<'+'I'*count, code, 32)}


def instructions(code):
    data = chunks(code)[b'SHDR']
    words = struct.unpack('<'+'I'*(len(data)//4), data)
    out = []
    i = 2
    while i < len(words):
        op = words[i] & 2047
        size = words[i+1] if op == 53 else (words[i] >> 24) & 127
        if not size or i+size > len(words):
            raise ValueError('invalid SHDR instruction length')
        out.append((i, op, words[i:i+size]))
        i += size
    return out


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def source_block(code):
    ins = instructions(code)
    starts = [i for i, (_, op, words) in enumerate(ins)
              if op == 50 and bits(.98) in words]
    assert len(starts) == 1, 'expected one source .98 multiply-add'
    block = ins[starts[0]:starts[0]+7]
    assert [v[1] for v in block] == [50, 0, 14, 56, 50, 56, 50]
    assert block[3][2][0] & (1 << 13), 'falloff fraction must saturate'
    assert bits(-2) in block[4][2] and bits(3) in block[4][2]
    assert bits(1) in block[6][2]
    return [{'word': i, 'opcode': op, 'words': list(w)} for i, op, w in block]


def native_block(code):
    ins = instructions(code)
    for i in range(len(ins)-6):
        b = ins[i:i+7]
        if [v[1] for v in b] != [0, 0, 14, 56, 50, 56, 50]:
            continue
        if not (b[3][2][0] & (1 << 13)):
            continue
        if bits(-2) in b[4][2] and bits(3) in b[4][2] and bits(1) in b[6][2]:
            return [{'word': n, 'opcode': op, 'words': list(w)} for n, op, w in b]
    raise ValueError('native saturate/cubic falloff block not found')


def objects(value):
    if isinstance(value, dict):
        if 'probe_side_width' in value:
            yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)


def alpha(distance, inner, outer):
    t = min(1, max(0, (distance-inner)/(outer-inner)))
    return 1-t*t*(3-2*t)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--native-capture', type=Path, required=True,
                   help='capture prefix ending in .probe-draw, without .dxbc')
    p.add_argument('--native-project', type=Path, required=True)
    p.add_argument('--source-project', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    rows = json.loads((a.evidence/'probe-extracted.json').read_text())
    spheres = [r for r in rows if r['key'].replace('_', '')[-1] == '0']
    assert len(spheres) == 8
    for row in spheres:
        code = (a.evidence/(row['name']+'.dxbc')).read_bytes()
        assert hashlib.sha256(code).hexdigest() == row['sha256']
        row['falloff_block'] = source_block(code)
    native_path = Path(str(a.native_capture)+'.dxbc')
    native = native_path.read_bytes()
    assert hashlib.sha256(native).hexdigest() == '7d04e85246882cbdd517e7cb757602bcf6a9b21de3d938957fb46eee4237b661'
    cb0 = Path(str(a.native_capture)+'.cb0.bin').read_bytes()
    cb2 = Path(str(a.native_capture)+'.cb2.bin').read_bytes()
    location = struct.unpack_from('<3f', cb0, 176)
    def probe(path):
        return next(o for o in objects(json.loads(path.read_text()))
                    if o['geometry'] == 0 and max(abs(x-y) for x, y in zip(o['position'], location)) < .001)
    target = probe(a.native_project)
    source = probe(a.source_project)
    inner = struct.unpack_from('<f', cb0, 188)[0]
    outer = struct.unpack_from('<f', cb2, 16)[0]
    delta_inner = inner+.05-target['radius']+target['probe_side_width']
    delta_outer = outer/struct.unpack('<f', struct.pack('<f', .98))[0]-target['radius']
    assert 0 <= delta_inner < .1 and abs(delta_inner-delta_outer) < .00005
    source_inner = source['radius']-source['probe_side_width']-.05
    source_outer = .98*source['radius']
    report = {
        'scope': 'Sphere radial falloff only. Shadow/normal clipping, index masks, colour encoding, '
                 'probe order and complete lighting equivalence are not established here.',
        'source_sphere_variants': spheres,
        'native': {'shader_sha256': hashlib.sha256(native).hexdigest(),
                   'falloff_block': native_block(native), 'location': location,
                   'authored_radius': target['radius'], 'authored_width': target['probe_side_width'],
                   'captured_inner': inner, 'captured_outer': outer,
                   'delta_from_inner': delta_inner, 'delta_from_outer': delta_outer},
        'source': {'name': source['full_name'], 'radius': source['radius'],
                   'width': source['probe_side_width'], 'inner': source_inner, 'outer': source_outer,
                   'inverted_interval': source_inner > source_outer,
                   'samples': [{'distance': d, 'radial_alpha': alpha(d, source_inner, source_outer)}
                               for d in [0, 100, source_outer, (source_inner+source_outer)/2, source_inner, 350]]},
        'mapping': 'Preserve explicit source radius and probe_side_width. The source shader applies '
                   '.98 to radius, while native CPU applies .98 before upload. Native adds its existing '
                   'small positive perturbation to radius and inner radius; no compensation is introduced.'}
    a.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'source_sphere_variants': len(spheres), 'source': report['source'], 'native': report['native']}, indent=2))


if __name__ == '__main__':
    main()
