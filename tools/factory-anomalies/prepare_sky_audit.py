#!/usr/bin/env python3
"""Extract compact donor/native sky contracts for offline comparison.

No game launch or database mutation. The caller supplies the porter's runtime
environment and coordinates any later FXC disassembly separately.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess


def bytecode(data):
    start = data.index(b'DXBC')
    size = struct.unpack_from('<I', data, start+24)[0]
    code = data[start:start+size]
    if len(code) != size:
        raise ValueError('truncated shader container')
    return code


def reflection(code):
    offsets = struct.unpack_from('<'+'I'*struct.unpack_from('<I', code, 28)[0], code, 32)
    start = next(o for o in offsets if code[o:o+4] == b'RDEF')
    raw = code[start+8:start+8+struct.unpack_from('<I', code, start+4)[0]]
    def string(offset):
        return raw[offset:raw.index(0, offset)].decode()
    count, offset, bindings, binding_offset = struct.unpack_from('<4I', raw)
    resources, buffers = {}, {}
    for i in range(bindings):
        fields = struct.unpack_from('<8I', raw, binding_offset+32*i)
        resources[string(fields[0])] = dict(kind=fields[1], slot=fields[5], count=fields[6])
    for i in range(count):
        fields = struct.unpack_from('<6I', raw, offset+24*i)
        variables = {}
        for j in range(fields[1]):
            v = struct.unpack_from('<6I', raw, fields[2]+24*j)
            variables[string(v[0])] = dict(offset=v[1], size=v[2], used=bool(v[3]&2))
        buffers[string(fields[0])] = variables
    return dict(resources=resources, buffers=buffers)


def prepare(source, target, porter, output):
    output.mkdir(exist_ok=False)
    records = []
    def save(data, build, family, resource):
        code = bytecode(data)
        sha = hashlib.sha256(code).hexdigest()
        filename = build+'-'+family+'-'+sha[:12]+'.dxbc'
        (output/filename).write_bytes(code)
        records.append(dict(build=build, family=family, resource=resource,
                            size=len(code), sha256=sha, file=filename,
                            reflection=reflection(code)))
    families = ('atmosphere.ps', 'atmosphere_clouds.ps', 'atmosphere_clouds.vs', 'skylight.ps')
    for family in families:
        for path in sorted((source/family).iterdir()):
            save(path.read_bytes(), '020e', family, str(path))
    command = [str(porter)]
    for family in families:
        paths = subprocess.check_output(command+['list', '--db', str(target),
            '--prefix', 'shaders/sm_4_0/'+family+'/'], text=True).splitlines()
        for resource in paths:
            temporary = output/'entry.tmp'
            subprocess.run(command+['extract', '--db', str(target), '--path', resource,
                '--output', str(temporary)], check=True, capture_output=True)
            save(temporary.read_bytes(), '026g', family, resource)
            temporary.unlink()
    (output/'manifest.json').write_text(json.dumps(records, indent=2)+'\n')
    print(json.dumps(dict(entries=len(records), unique_shaders=len(list(output.glob('*.dxbc'))))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'target', 'porter', 'output'):
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    prepare(args.source, args.target, args.porter, args.output)
